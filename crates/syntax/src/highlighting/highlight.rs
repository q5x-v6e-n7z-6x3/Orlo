//! Incremental tree-sitter colors on the worker, with Sublime grammars as a fallback.

use crate::index::{Docs, IndexedSymbol};
use crate::manifest::Source;
use crate::registry::{LanguageDef, Origin};
use gpui::{Hsla, Rgba};
use orlo_config::selector::Selector;
use orlo_editor::text::Change;
use ropey::Rope;
use std::collections::{HashMap, VecDeque};
use std::hash::Hasher;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};
use syntect::highlighting::{
    Color, FontStyle, HighlightIterator, HighlightState, Highlighter as SynHighlighter, ScopeSelectors, Style,
    StyleModifier, Theme, ThemeItem, ThemeSet, ThemeSettings,
};
use syntect::parsing::syntax_definition::{ContextReference, Pattern};
use syntect::parsing::{ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxDefinition, SyntaxReference, SyntaxSet};

struct Assets {
    syntaxes: SyntaxSet,
    bundled: usize,
    /// `None` keeps Mariana's prepared highlighter shared, including its selector ordering.
    theme: Option<&'static Theme>,
    highlighter: &'static SynHighlighter<'static>,
}

pub(crate) fn bundled_syntaxes() -> SyntaxSet {
    syntect::dumps::from_uncompressed_data(include_bytes!(concat!(env!("OUT_DIR"), "/syntaxes.packdump")))
        .expect("bundled syntaxes must be valid")
}

fn assets_slot() -> &'static RwLock<Arc<Assets>> {
    static ASSETS: OnceLock<RwLock<Arc<Assets>>> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let syntaxes = bundled_syntaxes();
        RwLock::new(Arc::new(Assets {
            bundled: syntaxes.syntaxes().len(),
            syntaxes,
            theme: None,
            highlighter: syn_highlighter(),
        }))
    })
}

fn assets() -> Arc<Assets> {
    assets_slot().read().unwrap_or_else(|e| e.into_inner()).clone()
}

impl Assets {
    fn theme(&self) -> &Theme {
        self.theme.unwrap_or_else(theme)
    }

    fn background(&self) -> Color {
        self.theme().settings.background.unwrap_or(color(0x303841))
    }

    fn load(support: &Path, selected: &str) -> (Self, Vec<String>) {
        let (files, mut errors) = customization_files(support);
        let mut definitions = Vec::new();
        let mut chosen = builtin_themes().themes.get(selected).cloned();
        let builtin = selected == "Mariana" || chosen.is_some();
        for path in files {
            match path.extension().and_then(|e| e.to_str()) {
                Some("sublime-syntax") => {
                    let parsed = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|text| {
                        SyntaxDefinition::load_from_str(&text, true, path.file_stem().and_then(|s| s.to_str()))
                            .map_err(|e| e.to_string())
                    });
                    match parsed {
                        Ok(syntax) => definitions.push(syntax),
                        Err(error) => errors.push(format!("{}: {error}", path.display())),
                    }
                }
                Some("tmTheme") => match ThemeSet::get_theme(&path) {
                    Ok(theme) => {
                        if asset_matches(support, &path, selected)
                            || !builtin && theme.name.as_deref() == Some(selected)
                        {
                            chosen = Some(theme);
                        }
                    }
                    Err(error) => errors.push(format!("{}: {error}", path.display())),
                },
                _ => {}
            }
        }
        if chosen.is_none() && selected != "Mariana" {
            errors.push(format!("Color scheme {selected:?} could not be loaded; using Mariana."));
        }
        if let Some(theme) = chosen.as_mut() {
            theme.settings.foreground.get_or_insert(color(0xd8dee9));
            theme.settings.background.get_or_insert(color(0x303841));
        }
        let syntaxes = bundled_syntaxes();
        let bundled = syntaxes.syntaxes().len();
        let syntaxes = if definitions.is_empty() {
            syntaxes
        } else {
            match linked(syntaxes, definitions) {
                Ok(syntaxes) => syntaxes,
                Err(unresolved) => {
                    errors
                        .push("Custom syntaxes contain unresolved context references; using built-in syntaxes.".into());
                    errors.extend(unresolved);
                    // unresolved dependencies reject the custom batch; isolate package groups if needed.
                    bundled_syntaxes()
                }
            }
        };
        // each language folder's syntax was checked alone, so one broken folder drops alone
        let mut languages = Vec::new();
        for def in crate::registry::registry().languages() {
            match language_syntax(def) {
                Some(Ok(definition)) => languages.push(definition),
                Some(Err(error)) => errors.push(error),
                None => {}
            }
        }
        let syntaxes = match languages.is_empty() {
            true => syntaxes,
            false => linked(syntaxes, languages).unwrap_or_else(|unresolved| {
                // each linked alone, so not seen: keep the bundled syntaxes working regardless
                errors.extend(unresolved);
                bundled_syntaxes()
            }),
        };
        let (theme, highlighter) = match chosen {
            Some(theme) => {
                let (theme, highlighter) = prepared(theme);
                (Some(theme), highlighter)
            }
            None => (None, syn_highlighter()),
        };
        (Self { syntaxes, bundled, theme, highlighter }, errors)
    }

    /// The syntax a language's manifest names: one of syntect's (`builtin:<name>`), or its
    /// folder's, if that loaded.
    fn language_syntax(&self, def: &LanguageDef) -> Option<usize> {
        let ss = self.syntaxes.syntaxes();
        match Source::of(def.manifest.syntax.as_deref()?) {
            Source::Builtin(name) => ss[..self.bundled].iter().position(|s| s.name == name),
            Source::File(_) => {
                let custom = ss.iter().enumerate().skip(self.bundled);
                custom
                    .map(|(i, s)| (i, s.variables.get(LANGUAGE_VARIABLE)))
                    .find(|(_, id)| *id == Some(&def.manifest.id))
                    .map(|(i, _)| i)
            }
        }
    }
}

/// Marks a syntax a language folder brought, by its language's id.
const LANGUAGE_VARIABLE: &str = "orlo.language";

/// `definitions` added to `syntaxes` and linked, or every reference among them that resolves
/// nowhere.
fn linked(syntaxes: SyntaxSet, definitions: Vec<SyntaxDefinition>) -> Result<SyntaxSet, Vec<String>> {
    let from = syntaxes.syntaxes().len();
    let mut builder = syntaxes.into_builder();
    for definition in definitions {
        builder.add(definition);
    }
    let linked = builder.build();
    let mut unresolved = linked.find_unlinked_contexts();
    // syntect's checker covers push/set references but omits include patterns.
    let builder = linked.into_builder();
    for syntax in builder.syntaxes().iter().skip(from) {
        for pattern in syntax.contexts.values().flat_map(|c| &c.patterns) {
            match pattern {
                Pattern::Include(ContextReference::Direct { .. }) => {}
                Pattern::Include(reference) => {
                    unresolved
                        .insert(format!("Syntax {:?} has unresolved context reference {reference:?}", syntax.name));
                }
                _ => {}
            }
        }
    }
    match unresolved.is_empty() {
        true => Ok(builder.build()),
        false => Err(unresolved.into_iter().collect()),
    }
}

fn bundled_once() -> &'static SyntaxSet {
    static BUNDLED: OnceLock<SyntaxSet> = OnceLock::new();
    BUNDLED.get_or_init(bundled_syntaxes)
}

/// A language folder's own `.sublime-syntax`, checked against the bundled syntaxes alone; `None`
/// when its manifest names none. It is hidden, and claims no files: the manifest detects them.
fn language_syntax(def: &LanguageDef) -> Option<Result<SyntaxDefinition, String>> {
    let Source::File(path) = Source::of(def.manifest.syntax.as_deref()?) else {
        return None;
    };
    let file = def.folder.join(path);
    let definition = def.read(path, crate::manifest::SYNTAX_BYTES).and_then(|bytes| {
        let text = String::from_utf8(bytes).map_err(|_| format!("{}: is not UTF-8", file.display()))?;
        let stem = Path::new(path).file_stem().and_then(|s| s.to_str());
        let mut definition =
            SyntaxDefinition::load_from_str(&text, true, stem).map_err(|e| format!("{}: {e}", file.display()))?;
        definition.variables.insert(LANGUAGE_VARIABLE.into(), def.manifest.id.clone());
        definition.hidden = true;
        definition.file_extensions.clear();
        definition.first_line_match = None;
        linked(bundled_once().clone(), vec![definition.clone()])
            .map_err(|unresolved| format!("{}: {}", file.display(), unresolved.join("; ")))?;
        Ok(definition)
    });
    Some(definition)
}

/// A language folder's syntax, as loading it would report it.
pub(crate) fn check_language_syntax(def: &LanguageDef) -> Result<(), String> {
    language_syntax(def).transpose().map(drop)
}

fn builtin_themes() -> &'static ThemeSet {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    THEMES.get_or_init(ThemeSet::load_defaults)
}

pub use orlo_config::themes::{asset_matches, customization_files};

pub fn color_schemes(support: &Path) -> Vec<String> {
    let mut names: Vec<_> = builtin_themes().themes.keys().cloned().collect();
    names.push("Mariana".into());
    names.extend(
        customization_files(support)
            .0
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "tmTheme"))
            .filter_map(|p| p.strip_prefix(support).ok().map(|p| p.to_string_lossy().into_owned())),
    );
    names.sort_by_key(|s| s.to_lowercase());
    names.dedup();
    names
}

/// Replace one owned generation; jobs and symbol parses retain the generation they began on.
/// Call each open highlighter's `reload` after this, then rebuild the syntax menu.
pub fn reload_customizations(support: &Path, color_scheme: &str, appearance: &str) -> Vec<String> {
    // the languages first: their folders' syntaxes join the new assets
    let languages = crate::registry::reload(support);
    let (loaded, mut errors) = Assets::load(support, color_scheme);
    orlo_config::themes::apply_scheme(&loaded.theme().settings);
    errors.extend(orlo_config::themes::load_appearance(support, appearance));
    errors.extend(languages);
    *assets_slot().write().unwrap_or_else(|e| e.into_inner()) = Arc::new(loaded);
    errors
}

#[cfg(test)]
fn syntax_set() -> &'static SyntaxSet {
    static S: OnceLock<SyntaxSet> = OnceLock::new();
    S.get_or_init(bundled_syntaxes)
}

fn theme() -> &'static Theme {
    static T: OnceLock<Theme> = OnceLock::new();
    T.get_or_init(mariana)
}

/// Built once: making one sorts the theme's selectors.
fn syn_highlighter() -> &'static SynHighlighter<'static> {
    static H: OnceLock<SynHighlighter<'static>> = OnceLock::new();
    H.get_or_init(|| SynHighlighter::new(theme()))
}

/// A chosen scheme and its highlighter, built once: a highlighter borrows its theme, so each
/// distinct scheme loaded is kept until quit.
fn prepared(theme: Theme) -> (&'static Theme, &'static SynHighlighter<'static>) {
    type Prepared = (&'static Theme, &'static SynHighlighter<'static>);
    static PREPARED: Mutex<Vec<Prepared>> = Mutex::new(Vec::new());
    let mut prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&found) = prepared.iter().find(|(t, _)| **t == theme) {
        return found;
    }
    let theme: &'static Theme = Box::leak(Box::new(theme));
    let found = (theme, &*Box::leak(Box::new(SynHighlighter::new(theme))));
    prepared.push(found);
    found
}

/// Language-server semantic tokens share the active syntax theme.
pub fn scope_style(scope: &str) -> (Hsla, Look) {
    style_for_scope(&assets(), "", scope)
}

fn style_for_scope(assets: &Assets, base: &str, scope: &str) -> (Hsla, Look) {
    let scopes: Vec<_> =
        [base, scope].into_iter().filter(|s| !s.is_empty()).filter_map(|s| Scope::new(s).ok()).collect();
    styled(assets.highlighter.style_for_stack(&scopes), assets.background())
}

/// `style`'s colour and look: its background only where it is not the theme's `background`.
fn styled(style: Style, background: Color) -> (Hsla, Look) {
    let look = Look {
        font_style: style.font_style,
        background: (style.background != background).then(|| to_hsla(style.background)),
    };
    (to_hsla(style.foreground), look)
}

fn hash_str<'a>(pieces: impl IntoIterator<Item = &'a str>) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    for p in pieces {
        h.write(p.as_bytes());
    }
    h.finish()
}

fn crlf(line: ropey::RopeSlice) -> bool {
    let n = line.len_chars();
    n >= 2 && line.char(n - 1) == '\n' && line.char(n - 2) == '\r'
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Bytes hashed to tell lines unchanged on this thread: how much a frame read.
    pub static HASHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn color(hex: u32) -> Color {
    Color { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 255 }
}

fn to_hsla(c: Color) -> Hsla {
    Rgba { r: c.r as f32 / 255., g: c.g as f32 / 255., b: c.b as f32 / 255., a: c.a as f32 / 255. }.into()
}

/// Sublime's Mariana colour scheme: its rules in order (later ones win ties, as in Sublime),
/// with parenthesised selectors spelled out for syntect.
fn mariana() -> Theme {
    let (blue, blue2, blue5, blue6) = (0x6699cc, 0x596673, 0x5fb4b4, 0xa6acb9);
    let (green, orange, orange2, pink) = (0x99c794, 0xf9ae58, 0xee932b, 0xc695c6);
    let (red, red2, white, white2, white3) = (0xec5f66, 0xf97b58, 0xffffff, 0xf7f7f7, 0xd8dee9);
    let (i, b, u) = (FontStyle::ITALIC, FontStyle::BOLD, FontStyle::UNDERLINE);
    let none = FontStyle::empty();
    let item = |sel: &str, fg: Option<u32>, style: FontStyle, bg: Option<Color>| ThemeItem {
        scope: ScopeSelectors::from_str(sel).expect("valid scope selector"),
        style: StyleModifier {
            foreground: fg.map(color),
            background: bg,
            font_style: (!style.is_empty()).then_some(style),
        },
    };
    let fg = |sel: &str, hex: u32| item(sel, Some(hex), none, None);
    let fgs = |sel: &str, hex: u32, style| item(sel, Some(hex), style, None);
    let st = |sel: &str, style| item(sel, None, style, None);
    // blue2 at the alphas markup.raw uses
    let raw = |a: u8| Some(Color { a, ..color(blue2) });
    Theme {
        name: Some("Mariana".into()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(color(white3)),
            background: Some(color(0x303841)),
            ..Default::default()
        },
        scopes: vec![
            fg("comment, punctuation.definition.comment", blue6),
            fg("string", green),
            fg(
                "meta.mapping.key string - meta.mapping.key meta meta, \
                 meta.mapping.key meta.mapping.key string - meta.mapping.key meta.mapping.key meta meta",
                blue5,
            ),
            fg("punctuation.definition - punctuation.definition.numeric.base", blue5),
            fg("constant.numeric", orange),
            fgs("storage.type.numeric", pink, i),
            fgs("constant.language", red, i),
            fg("constant.character, constant.other", pink),
            fg("variable.member", red),
            fg("keyword - keyword.operator, keyword.operator.word", pink),
            fg("keyword.operator", red2),
            fg("punctuation.separator, punctuation.terminator", blue6),
            fg("punctuation.section", white),
            fg("punctuation.accessor", blue6),
            fg("punctuation.definition.annotation", blue5),
            fg(
                "variable.other.dollar.only.js, variable.other.object.dollar.only.js, \
                 variable.type.dollar.only.js, support.class.dollar.only.js",
                blue5,
            ),
            fg("storage", red),
            fgs("storage.type", pink, i),
            fg("entity.name.function", blue5),
            fg("entity.name - entity.name.section - entity.name.tag - entity.name.label", orange),
            fgs("entity.other.inherited-class", blue5, i),
            fg("variable.parameter", orange),
            fgs("variable.language", red, i),
            fg("entity.name.tag", red),
            fg("entity.other.attribute-name", pink),
            fg("variable.function, variable.annotation", blue),
            fgs("support.function, support.macro", blue, i),
            fgs("support.constant", pink, i),
            fgs("support.type, support.class", blue, i),
            item("invalid", Some(white2), none, Some(color(red))),
            item("invalid.deprecated", Some(white2), none, Some(color(orange2))),
            fg("entity.name.tag.yaml", blue5),
            fg("source.yaml string.unquoted", white3),
            st("markup.heading", b),
            fg("markup.heading punctuation.definition.heading", red2),
            fg("markup.heading.1 punctuation.definition.heading", red),
            fg("string.other.link, markup.underline.link", blue),
            st("markup.bold", b),
            st("markup.italic", i),
            st("markup.underline", u),
            st("markup.italic markup.bold, markup.bold markup.italic", b | i),
            st("markup.underline markup.bold, markup.bold markup.underline", b | u),
            st("markup.underline markup.italic, markup.italic markup.underline", i | u),
            st(
                "markup.bold markup.italic markup.underline, markup.bold markup.underline markup.italic, \
                 markup.italic markup.bold markup.underline, markup.italic markup.underline markup.bold, \
                 markup.underline markup.bold markup.italic, markup.underline markup.italic markup.bold",
                b | i | u,
            ),
            fg("punctuation.definition.thematic-break", orange),
            fg("markup.list.numbered.bullet", green),
            fg("markup.quote punctuation.definition.blockquote, markup.list punctuation.definition.list_item", orange),
            item("markup.raw", None, none, raw(97)),
            item("markup.raw.inline", None, none, raw(128)),
            fg("text punctuation.definition.italic, text punctuation.definition.bold", pink),
            fg("meta.diff, meta.diff.header", pink),
            fg("markup.deleted", red),
            fg("markup.inserted", green),
            fg("markup.changed", orange),
            fg("support.type.property-name", white3),
            fg("constant.numeric.line-number.match", red),
            fg("message.error", red),
            fg("message.warning", orange2),
            fg("message.info", blue),
        ],
    }
}

/// A coloured piece of a line: its length in bytes, colour, and Mariana's font style and
/// background (`None`: the editor's).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub font_style: FontStyle,
    pub background: Option<Hsla>,
}

impl Look {
    pub fn italic(self) -> bool {
        self.font_style.contains(FontStyle::ITALIC)
    }
    pub fn bold(self) -> bool {
        self.font_style.contains(FontStyle::BOLD)
    }
    pub fn underline(self) -> bool {
        self.font_style.contains(FontStyle::UNDERLINE)
    }
}

pub type Run = (usize, Hsla, Look);

/// An interned scope stack (`scope_stack`): the scopes a piece of a line is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeId(u32);

/// What a scope stack is to completion: a comment, a string, or code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Comment,
    String,
    Code,
}

/// Every scope stack coloured so far, once each, for the whole process.
#[derive(Default)]
struct Interner {
    ids: HashMap<Arc<[Scope]>, ScopeId>,
    stacks: Vec<Arc<[Scope]>>,
    scratch: Vec<Scope>,
}

impl Interner {
    /// `id` of `stack` with `outer` in place of its outermost scope, when there is one.
    fn id_under(&mut self, stack: &[Scope], outer: Option<Scope>) -> ScopeId {
        match (outer, stack.split_first()) {
            (Some(outer), Some((_, inner))) => {
                let mut scratch = std::mem::take(&mut self.scratch);
                scratch.clear();
                scratch.push(outer);
                scratch.extend_from_slice(inner);
                let id = self.id(&scratch);
                self.scratch = scratch;
                id
            }
            _ => self.id(stack),
        }
    }

    fn id(&mut self, stack: &[Scope]) -> ScopeId {
        if let Some(&id) = self.ids.get(stack) {
            return id;
        }
        let id = ScopeId(self.stacks.len() as u32);
        let stack: Arc<[Scope]> = stack.into();
        self.stacks.push(stack.clone());
        self.ids.insert(stack, id);
        id
    }
}

// stacks are kept until quit, so a file nested thousands deep leaves megabytes of
// them; give each file its own interner if that shows up.
fn interner() -> std::sync::MutexGuard<'static, Interner> {
    static INTERNER: OnceLock<Mutex<Interner>> = OnceLock::new();
    INTERNER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The id of the stack of the scopes named, outermost first (empty or invalid names left out).
pub fn scope_id(names: &[&str]) -> ScopeId {
    let stack: Vec<_> = (names.iter().filter(|n| !n.is_empty())).filter_map(|n| Scope::new(n).ok()).collect();
    interner().id(&stack)
}

/// The scopes of stack `id`, outermost first.
pub fn scope_stack(id: ScopeId) -> Arc<[Scope]> {
    interner().stacks[id.0 as usize].clone()
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Selector matches worked out on this thread, not answered from the cache.
    pub static MATCHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether stack `id` matches the scope selector (Sublime's, see `selector`: `string, comment -
/// comment.block`, `(a | b) & c`, `-(constant, entity)`); an empty or invalid selector matches
/// nothing (`Selector::parse` says what is wrong with one). A selector is parsed once, and each
/// answer is kept, per stack and selector.
pub fn matches(id: ScopeId, selector: &str) -> bool {
    type Answers = HashMap<String, (Option<Selector>, HashMap<ScopeId, bool>)>;
    static ANSWERS: OnceLock<Mutex<Answers>> = OnceLock::new();
    let mut answers = (ANSWERS.get_or_init(Default::default).lock()).unwrap_or_else(|e| e.into_inner());
    if !answers.contains_key(selector) {
        let parsed = Selector::parse(selector).ok();
        answers.insert(selector.to_string(), (parsed, HashMap::new()));
    }
    let (parsed, known) = answers.get_mut(selector).expect("inserted above");
    *known.entry(id).or_insert_with(|| {
        #[cfg(any(test, feature = "test-support"))]
        MATCHED.set(MATCHED.get() + 1);
        (parsed.as_ref()).is_some_and(|s| s.matches(&scope_stack(id)))
    })
}

/// A comment where the stack has a `comment` scope, else a string where it has a `string` one.
pub fn class(id: ScopeId) -> Class {
    if matches(id, "comment") {
        Class::Comment
    } else if matches(id, "string") {
        Class::String
    } else {
        Class::Code
    }
}

/// Tests: a definition's line, name and kind ("function", "struct" ...).
#[cfg(any(test, feature = "test-support"))]
pub type Symbol = (usize, String, String);

type Work = Box<dyn FnOnce() + Send>;

/// Urgent work queued on the highlighting thread: a job running yields to it at a line.
static URGENT: AtomicUsize = AtomicUsize::new(0);

/// Whether a job should stop where it is and give the thread to urgent work.
fn urgent_waiting() -> bool {
    URGENT.load(Ordering::Relaxed) > 0
}

/// Highlighting jobs and symbol parses run here, one at a time on one thread: syntect's
/// regexes keep match caches (megabytes a grammar) for each thread that runs them, so jobs
/// spread over a thread pool kept multiplying them, and a lock to take turns left the pool's
/// threads waiting. `None` when the work panicked.
pub fn on_worker<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> impl std::future::Future<Output = Option<R>> {
    queue(false, f)
}

/// `on_worker`, ahead of the queue: a big file's first lines as it opens. A job running stops
/// at its next line (what it parsed is kept) so this runs at once.
pub fn on_worker_urgent<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> impl std::future::Future<Output = Option<R>> {
    queue(true, f)
}

fn queue<R: Send + 'static>(
    urgent: bool,
    f: impl FnOnce() -> R + Send + 'static,
) -> impl std::future::Future<Output = Option<R>> {
    static WORKER: OnceLock<Mutex<std::sync::mpsc::Sender<(bool, Work)>>> = OnceLock::new();
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<(bool, Work)>();
        std::thread::Builder::new()
            .name("orlo-highlight".into())
            .spawn(move || {
                // [normal, urgent], each in the order queued; urgent work runs first
                let mut queues: [VecDeque<Work>; 2] = Default::default();
                loop {
                    if queues.iter().all(VecDeque::is_empty) {
                        match rx.recv() {
                            Ok((urgent, w)) => queues[urgent as usize].push_back(w),
                            Err(_) => return,
                        }
                    }
                    for (urgent, w) in rx.try_iter() {
                        queues[urgent as usize].push_back(w);
                    }
                    let work = match queues[1].pop_front() {
                        Some(w) => {
                            URGENT.fetch_sub(1, Ordering::Relaxed);
                            w
                        }
                        None => queues[0].pop_front().expect("one is waiting"),
                    };
                    work();
                }
            })
            .expect("start the highlighting thread");
        Mutex::new(tx)
    });
    let slot = Slot::shared();
    let to = slot.clone();
    let work: Work = Box::new(move || {
        Slot::fill(&to, std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok());
    });
    if urgent {
        URGENT.fetch_add(1, Ordering::Relaxed);
    }
    let sent = tx.lock().unwrap_or_else(|e| e.into_inner()).send((urgent, work)).is_ok();
    if !sent {
        if urgent {
            URGENT.fetch_sub(1, Ordering::Relaxed);
        }
        Slot::fill(&slot, None);
    }
    receive(slot)
}

/// A worker's one answer, which `receive` waits for.
pub(crate) struct Slot<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

impl<T> Slot<T> {
    pub(crate) fn shared() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self { value: None, waker: None }))
    }

    pub(crate) fn fill(slot: &Mutex<Self>, value: T) {
        let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.value = Some(value);
        if let Some(waker) = slot.waker.take() {
            waker.wake();
        }
    }
}

pub(crate) fn receive<T>(slot: Arc<Mutex<Slot<T>>>) -> impl std::future::Future<Output = T> {
    std::future::poll_fn(move |cx| {
        let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
        match slot.value.take() {
            Some(value) => Poll::Ready(value),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    })
}

// Share captured states across entries; a header is never copied once per cached line.
// Not syntect's native backend: its pointer equality on captured states breaks this sharing
// (one captured-delimiter fixture took about 399 MiB on it, 0.6 MiB here).
const SYMBOL_PARSE_STATES: usize = 256;
const SYMBOL_STATE_BYTES: usize = 16 << 10;
const SYMBOL_TRANSITION_SLOTS: u64 = 16_384;
const SYMBOL_TRANSITION_BYTES: usize = 4 << 20;
const SYMBOL_TRANSITION_LINE: usize = 1_024;
const SYMBOL_OPERATIONS: usize = 1_024;

type EncodedOps = (Vec<(u16, u16)>, Vec<ScopeStackOp>);

struct SymbolTransition {
    text: String,
    before: usize,
    after: usize,
    ops: Vec<(u16, u16)>,
}

struct SymbolStateSize(usize);

impl std::fmt::Write for SymbolStateSize {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if text.len() > SYMBOL_STATE_BYTES - self.0 {
            return Err(std::fmt::Error);
        }
        self.0 += text.len();
        Ok(())
    }
}

#[derive(Default)]
struct SymbolTransitions {
    states: Vec<ParseState>,
    // `resume` resets this when adopting a checkpoint's parser state.
    current: Option<usize>,
    entries: HashMap<u64, SymbolTransition>,
    operations: Vec<ScopeStackOp>,
    bytes: usize,
    disabled: bool,
    #[cfg(any(test, feature = "test-support"))]
    reused: usize,
}

impl SymbolTransitions {
    /// Offsets fit the admitted line bound; operation parameters retain their full types.
    /// New catalog values are committed only with the complete, budgeted transition.
    fn encode_ops(&self, ops: &[(usize, ScopeStackOp)]) -> Option<EncodedOps> {
        let mut encoded = Vec::with_capacity(ops.len());
        let mut added = Vec::new();
        for (at, op) in ops {
            let index = if let Some(i) = self.operations.iter().position(|old| old == op) {
                i
            } else if let Some(i) = added.iter().position(|old| old == op) {
                self.operations.len() + i
            } else {
                let i = self.operations.len() + added.len();
                if i >= SYMBOL_OPERATIONS {
                    return None;
                }
                added.push(op.clone());
                i
            };
            encoded.push((u16::try_from(*at).ok()?, u16::try_from(index).ok()?));
        }
        Some((encoded, added))
    }

    fn state(&mut self, parse: &ParseState) -> Option<usize> {
        if self.disabled {
            return None;
        }
        if let Some(i) = self.current.filter(|&i| self.states[i] == *parse) {
            return Some(i);
        }
        if let Some(i) = self.states.iter().position(|s| s == parse) {
            return Some(i);
        }
        if self.states.len() == SYMBOL_PARSE_STATES {
            return None;
        }
        // Syntect's derived Debug includes owned captures; bound it before cloning a state.
        use std::fmt::Write as _;
        if write!(&mut SymbolStateSize(0), "{parse:?}").is_err() {
            *self = Self { disabled: true, ..Self::default() };
            return None;
        }
        self.states.push(parse.clone());
        Some(self.states.len() - 1)
    }

    fn line(&mut self, parse: &mut ParseState, text: &str, ss: &SyntaxSet) -> Vec<(usize, ScopeStackOp)> {
        if self.disabled || text.len() > SYMBOL_TRANSITION_LINE {
            self.current = None;
            return parse.parse_line(text, ss).unwrap_or_default();
        }
        let before = self.current.or_else(|| self.state(parse));
        // Reducing the key to a slot makes unrelated repeated lines evict one another.
        // Keep its full hash, with exact text/state guards and bounded admission below.
        let key = before.map(|i| hash_str([text]) ^ (i as u64).wrapping_mul(0x9e3779b97f4a7c15));
        if let Some(c) =
            key.and_then(|key| self.entries.get(&key)).filter(|c| Some(c.before) == before && c.text == text)
        {
            if c.before != c.after {
                *parse = self.states[c.after].clone();
            }
            self.current = Some(c.after);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.reused += 1;
            }
            return c.ops.iter().map(|&(at, op)| (usize::from(at), self.operations[usize::from(op)].clone())).collect();
        }
        let ops = parse.parse_line(text, ss).unwrap_or_default();
        let after = self.state(parse);
        self.current = after;
        if let (Some(key), Some(before), Some(after)) = (key, before, after) {
            let size = |text: &str, ops: &[_]| text.len() + std::mem::size_of_val(ops);
            let old = self.entries.get(&key).map_or(0, |c| size(&c.text, &c.ops));
            let bytes = self.bytes - old + text.len() + ops.len() * std::mem::size_of::<(u16, u16)>();
            if bytes <= SYMBOL_TRANSITION_BYTES
                && (self.entries.len() < SYMBOL_TRANSITION_SLOTS as usize || self.entries.contains_key(&key))
            {
                if let Some((encoded, added)) = self.encode_ops(&ops) {
                    let bytes = bytes + std::mem::size_of_val(added.as_slice());
                    if bytes <= SYMBOL_TRANSITION_BYTES {
                        self.operations.extend(added);
                        self.entries
                            .insert(key, SymbolTransition { text: text.to_string(), before, after, ops: encoded });
                        self.bytes = bytes;
                    }
                }
            }
        }
        ops
    }
}

/// A symbol parse under way (the project index), to go on from where it stopped.
pub struct SymbolParse {
    reuse: SymbolTransitions,
    assets: Arc<Assets>,
    syntax: usize,
    parse: ParseState,
    stack: ScopeStack,
    /// Python's docs follow its definitions, in docstrings.
    docstrings: bool,
    line: usize,
    pub found: Vec<IndexedSymbol>,
    docs: Docs,
    /// An open buffer's parse keeps checkpoints (`resume`): the parse before's up to where this
    /// one began, then its own.
    kept: Option<Vec<Checkpoint>>,
    /// The parse before's checkpoints past the edit, at the lines they are at now: reaching one
    /// in the state it holds, this parse is done, the definitions after it standing.
    ahead: VecDeque<Checkpoint>,
    /// What `found` stands for among the parse before's definitions (`finish`).
    splice: Splice,
    /// A language without a regex grammar finds its definitions by its `tags.scm` instead.
    tags: Option<crate::treesitter::Tags>,
    /// Tests: the lines it went through.
    #[cfg(any(test, feature = "test-support"))]
    parsed: usize,
}

/// An open buffer's parse keeps its state this many lines apart.
const CHECKPOINT_LINES: usize = 1_024;

/// A symbol parse's state before a line, for a later parse of the text after an edit to start
/// from, or to stop at (`SymbolParse::resume`).
#[derive(Clone)]
pub struct Checkpoint {
    line: usize,
    parse: ParseState,
    stack: ScopeStack,
    docs: Docs,
    /// The completed definitions above it, then any still waiting for a docstring.
    found: usize,
    pending: Vec<IndexedSymbol>,
}

/// What an open buffer's symbol parse leaves for the next one: the text it parsed and its
/// checkpoints, in the grammar they belong to.
pub struct Checkpoints {
    assets: Arc<Assets>,
    syntax: usize,
    text: Rope,
    list: Vec<Checkpoint>,
    /// Tests: the lines the parse that left them went through.
    #[cfg(any(test, feature = "test-support"))]
    pub parsed: usize,
}

#[cfg(any(test, feature = "test-support"))]
impl Checkpoints {
    pub fn lines(&self) -> Vec<usize> {
        self.list.iter().map(|c| c.line).collect()
    }
}

/// A parse's definitions stand for the parse before's `from..to` (`None`: to the end); those
/// after them move by `delta` lines.
#[derive(Clone, Copy, Default)]
struct Splice {
    from: usize,
    to: Option<usize>,
    delta: isize,
}

/// Syntaxes whose definitions take the string under them as their doc.
const DOCSTRING_SYNTAXES: [&str; 1] = ["Python"];

/// What a symbol parse looks for in a line's scopes.
struct SymbolScopes {
    /// Definitions: Sublime's Goto Symbol scopes, tags excluded.
    name: ScopeSelectors,
    comment: ScopeSelectors,
    /// Rust attributes, Python decorators, Java annotations.
    attribute: ScopeSelectors,
    docstring: ScopeSelectors,
    /// A definition's header going on to another line (Python's `def f(` … `):`).
    header: ScopeSelectors,
}

fn symbol_scopes() -> &'static SymbolScopes {
    static SCOPES: OnceLock<SymbolScopes> = OnceLock::new();
    SCOPES.get_or_init(|| {
        let sel = |s: &str| ScopeSelectors::from_str(s).expect("valid scope selector");
        SymbolScopes {
            name: sel("entity.name - entity.name.tag"),
            comment: sel("comment"),
            attribute: sel("meta.annotation"),
            docstring: sel("string, comment.block.documentation"),
            header: sel("meta.function, meta.class"),
        }
    })
}

/// How long a symbol parse holds the highlighting thread before what is queued behind it (a
/// view's colours) gets a turn.
const PARSE_SLICE: Duration = Duration::from_millis(20);

impl SymbolParse {
    /// An open buffer's parse of `text`, keeping checkpoints. With `last` (the parse of the
    /// text before, if in this grammar) it starts at the checkpoint above the first line that
    /// changed, and stops at one past the edit whose state it reaches again.
    pub fn resume(mut self, last: Option<&Checkpoints>, text: &Rope) -> Self {
        // a tags parse starts over: it keeps no checkpoints
        if self.tags.is_some() {
            return self;
        }
        let first = self.checkpoint_here();
        self.kept = Some(vec![first]);
        let Some(last) = last.filter(|l| Arc::ptr_eq(&l.assets, &self.assets) && l.syntax == self.syntax) else {
            return self;
        };
        let old = &last.text;
        let same = same_start(old, text);
        let tail = same_end(old, text, old.len_bytes().min(text.len_bytes()) - same);
        let edited = old.byte_to_line(same);
        // the last at or above the first line changed: the first one is at line 0
        let at = last.list.partition_point(|c| c.line <= edited) - 1;
        let delta = text.len_lines() as isize - old.len_lines() as isize;
        // past the edit: the line break above each is in the unchanged end of the text
        let unchanged = old.len_bytes() - tail;
        self.ahead = (last.list[at + 1..].iter())
            .filter(|c| old.line_to_byte(c.line) > unchanged)
            .map(|c| Checkpoint { line: c.line.saturating_add_signed(delta), ..c.clone() })
            .collect();
        let start = &last.list[at];
        (self.parse, self.stack) = (start.parse.clone(), start.stack.clone());
        self.reuse.current = None;
        (self.docs, self.line) = (start.docs.clone(), start.line);
        self.found = start.pending.clone();
        self.splice = Splice { from: start.found, to: None, delta };
        self.kept = Some(last.list[..=at].to_vec());
        self
    }

    /// The state before line `self.line`.
    fn checkpoint_here(&self) -> Checkpoint {
        let from = *self.docs.pending().first().unwrap_or(&self.found.len());
        Checkpoint {
            line: self.line,
            parse: self.parse.clone(),
            stack: self.stack.clone(),
            docs: self.docs.checkpoint(),
            found: self.splice.from + from,
            pending: self.found[from..].to_vec(),
        }
    }

    /// Before line `self.line` of an open buffer's parse: true at a checkpoint of the parse
    /// before, past the edit, in the state it holds (what follows is as it was). Else one is
    /// kept every 1,024 lines, including within a definition's docstring.
    fn at_checkpoint(&mut self) -> bool {
        let Some(kept) = &mut self.kept else {
            return false;
        };
        while let Some(c) = self.ahead.front().filter(|c| c.line <= self.line) {
            let same = c.line == self.line
                && (c.parse == self.parse && c.stack == self.stack && c.docs == self.docs.checkpoint());
            if same {
                // The rest of a pending docstring is unchanged too.
                for (&i, old) in self.docs.pending().iter().zip(&c.pending) {
                    self.found[i].doc = old.doc.clone();
                }
                let to = c.found + c.pending.len();
                let moved = (self.splice.from + self.found.len()) as isize - to as isize;
                self.splice.to = Some(to);
                kept.extend(self.ahead.drain(..).map(|mut c| {
                    c.found = c.found.saturating_add_signed(moved);
                    c
                }));
                return true;
            }
            self.ahead.pop_front();
        }
        let last = kept.last().map_or(0, |c| c.line);
        if self.line >= last + CHECKPOINT_LINES {
            let c = self.checkpoint_here();
            self.kept.get_or_insert_default().push(c);
        }
        false
    }

    /// Done: `symbols`, the parse before's, takes this one's definitions in place of those
    /// it parsed again, the ones after them moved by the lines the edit added or took away.
    /// What it leaves for the next parse of `text` (an open buffer's).
    pub fn finish(mut self, symbols: &mut Vec<IndexedSymbol>, text: Rope) -> Option<Checkpoints> {
        let Splice { from, to, delta } = self.splice;
        // in range even for symbols out of step with the checkpoints
        let from = from.min(symbols.len());
        let to = to.unwrap_or(symbols.len()).clamp(from, symbols.len());
        let n = self.found.len();
        symbols.splice(from..to, std::mem::take(&mut self.found));
        for s in &mut symbols[from + n..] {
            s.line = s.line.saturating_add_signed(delta);
        }
        let mut list = self.kept?;
        for c in &mut list {
            let end = c.found + c.pending.len();
            c.pending.clone_from_slice(&symbols[c.found..end]);
        }
        Some(Checkpoints {
            assets: self.assets,
            syntax: self.syntax,
            text,
            list,
            #[cfg(any(test, feature = "test-support"))]
            parsed: self.parsed,
        })
    }

    /// Tests: whether it stopped at a checkpoint past the edit, before the end.
    #[cfg(any(test, feature = "test-support"))]
    pub fn stopped_early(&self) -> bool {
        self.splice.to.is_some()
    }

    /// Parse on: true at the end, false when `pause` says so at a line. Lines too long to
    /// colour are not parsed for symbols either.
    pub fn step(&mut self, rope: &Rope, pause: impl Fn() -> bool) -> bool {
        use crate::index::Line;
        if let Some(tags) = &mut self.tags {
            return tags.step(rope, pause, &mut self.found);
        }
        let assets = self.assets.clone();
        let (ss, sel) = (&assets.syntaxes, symbol_scopes());
        let end = rope.len_lines();
        let mut lines = rope.lines_at(self.line.min(end));
        while self.line < end {
            if pause() {
                return false;
            }
            if self.at_checkpoint() {
                return true;
            }
            let Some(line) = lines.next() else { break };
            let i = self.line;
            self.line += 1;
            #[cfg(any(test, feature = "test-support"))]
            {
                self.parsed += 1;
            }
            if line.len_bytes() > MAX_HIGHLIGHT {
                let new = self.found.len();
                self.docs.line(Line::Code, "", self.docstrings, false, &mut self.found, new);
                continue;
            }
            let mut text = line.to_string();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            let mut ops = self.reuse.line(&mut self.parse, &text, ss);
            ops.push((text.len(), ScopeStackOp::Noop));
            let new = self.found.len();
            let mut pos = 0;
            let (mut name, mut kind, mut at) = (String::new(), String::new(), 0);
            // the scopes of the line's first text, and whether all of its text is a comment
            let (mut first, mut comment) = (None, true);
            for (end, op) in &ops {
                let stack = self.stack.as_slice();
                if !text[pos..*end].trim().is_empty() {
                    let is = |s: &ScopeSelectors| s.does_match(stack).is_some();
                    first.get_or_insert_with(|| (is(&sel.attribute), is(&sel.docstring), is(&sel.header)));
                    comment = comment && is(&sel.comment);
                }
                if sel.name.does_match(stack).is_some() {
                    if name.is_empty() {
                        kind = symbol_kind_of(&self.stack);
                        at = pos;
                    }
                    name.push_str(&text[pos..*end]);
                } else {
                    if !name.trim().is_empty() {
                        let k = std::mem::take(&mut kind);
                        self.found.push(definition(i, &text, at, &name, k));
                    }
                    name.clear();
                }
                pos = *end;
                self.stack.apply(op).ok();
            }
            if !name.trim().is_empty() {
                self.found.push(definition(i, &text, at, &name, kind));
            }
            let open = self.docstrings && sel.docstring.does_match(self.stack.as_slice()).is_some();
            let class = match first {
                None => Line::Blank,
                Some((_, true, _)) if self.docstrings => Line::Docstring,
                _ if comment => Line::Comment,
                Some((true, _, _)) => Line::Attribute,
                Some((_, _, true)) if self.docstrings => Line::Header,
                _ => Line::Code,
            };
            self.docs.line(class, text.trim_end(), self.docstrings, open, &mut self.found, new);
        }
        true
    }
}

/// How many bytes `a` and `b` begin with alike. Chunks they share (a rope and its copy share
/// every leaf an edit did not touch) are passed over unread.
fn same_start(a: &Rope, b: &Rope) -> usize {
    orlo_utils::text::common_bytes(a.chunks(), b.chunks(), |c| c.next(), false)
}

/// How many bytes `a` and `b` end with alike, at most `max` (as `same_start`).
fn same_end(a: &Rope, b: &Rope, max: usize) -> usize {
    orlo_utils::text::common_bytes_up_to(
        a.chunks_at_byte(a.len_bytes()).0,
        b.chunks_at_byte(b.len_bytes()).0,
        |c| c.prev(),
        true,
        max,
    )
}

/// The index of `syntax` in `ss`.
fn index_of(ss: &SyntaxSet, syntax: &SyntaxReference) -> usize {
    (ss.syntaxes().iter()).position(|s| std::ptr::eq(s, syntax)).unwrap()
}

/// The definition `name`, which starts at byte `at` of line `line`'s `text`.
fn definition(line: usize, text: &str, at: usize, name: &str, kind: String) -> IndexedSymbol {
    let lead = name.len() - name.trim_start().len();
    IndexedSymbol {
        name: name.trim().to_string(),
        kind,
        line,
        col: text[..at + lead].chars().count(),
        signature: crate::index::signature(text),
        doc: None,
    }
}

/// A symbol parse on the highlighting thread a stretch at a time: urgent work (a big file's
/// first colours) takes the thread at a line, and the parse goes on after it, as it does after
/// each `PARSE_SLICE` for the work queued behind it. The parse, done; `None` once `stale`.
pub fn parse_on_worker<S: Fn() -> bool + Clone + Send + 'static>(
    parse: SymbolParse,
    rope: Rope,
    stale: S,
) -> impl std::future::Future<Output = Option<SymbolParse>> {
    let step = |mut p: SymbolParse, rope: Rope, stale: S| {
        on_worker(move || {
            let start = Instant::now();
            let done = p.step(&rope, || urgent_waiting() || stale() || start.elapsed() > PARSE_SLICE);
            (p, done)
        })
    };
    let first = step(parse, rope.clone(), stale.clone());
    async move {
        let mut next = first;
        loop {
            let (p, done) = next.await?;
            if stale() {
                return None;
            }
            if done {
                return Some(p);
            }
            next = step(p, rope.clone(), stale.clone());
        }
    }
}

/// Run a `job` up to each of `lines` (on the worker: see `on_worker`).
pub fn run_job(mut job: Highlighter, rope: &Rope, lines: &[usize]) -> Highlighter {
    #[cfg(any(test, feature = "test-support"))]
    assert!(!job.panics, "a test's job that panics");
    for &line in lines {
        job.line_runs(rope, line);
    }
    if let (Some(&from), Some(&to)) = (lines.iter().min(), lines.iter().max()) {
        job.prepare_tree(rope, from, to, None);
    }
    job
}

/// What follows `entity.name.` in the innermost such scope: "function", "impl", "struct" ...
fn symbol_kind_of(stack: &ScopeStack) -> String {
    stack
        .as_slice()
        .iter()
        .rev()
        .find_map(|s| {
            let s = s.build_string();
            s.strip_prefix("entity.name.").map(|k| k.split('.').next().unwrap_or_default().to_string())
        })
        .unwrap_or_default()
}

/// Sublime's badge for a symbol kind in Goto Symbol: its letter and Mariana colour (red for
/// functions, blue for modules, orange for constants, purple for types and the rest).
pub fn symbol_badge(kind: &str) -> (char, u32) {
    match kind {
        "function" | "method" => ('f', 0xec5f66),
        "namespace" | "module" | "package" => ('n', 0x6699cc),
        "constant" | "variable" => ('c', 0xf9ae58),
        "section" | "heading" | "label" => ('#', 0xfac761),
        k => (k.chars().next().unwrap_or('s'), 0xc695c6),
    }
}

#[derive(Clone)]
struct LineState {
    /// The parser and highlighter state after the line, shared with the line above when
    /// equal: a full copy per line came to 6.5 KB a line.
    state: Arc<(ParseState, HighlightState)>,
    runs: Vec<Run>,
    /// `(byte_len, id)` of the scope stacks over the line without its break (`scope_at`);
    /// shared, as walking unchanged lines copies their entries.
    scopes: Arc<[(usize, ScopeId)]>,
    /// The line's `line_key`, so an edit above can tell the line is unchanged.
    text: u64,
    len: usize,
}

impl LineState {
    /// Whether it was made from line `i` as it is now: the length, then the hash, which
    /// counts toward the frame's hashing budget, `hashed`.
    fn same_text(&self, rope: &Rope, i: usize, hashed: &mut usize) -> bool {
        let line = rope.line(i);
        let len = line.len_bytes();
        if len != self.len {
            return false;
        }
        if len > MAX_HIGHLIGHT {
            return crlf(line) as u64 == self.text;
        }
        *hashed += len;
        #[cfg(any(test, feature = "test-support"))]
        HASHED.set(HASHED.get() + len);
        hash_str(line.chunks()) == self.text
    }
}

/// `(byte_len, id)` of the stacks a line's parse `ops` take `stack` through over its first
/// `len` bytes (the text without its break), each with `outer` as its outermost scope if given.
fn scope_runs(
    mut stack: ScopeStack,
    ops: &[(usize, ScopeStackOp)],
    len: usize,
    outer: Option<Scope>,
) -> Arc<[(usize, ScopeId)]> {
    let mut interner = interner();
    let (mut runs, mut pos): (Vec<(usize, ScopeId)>, _) = (Vec::new(), 0);
    let noop = ScopeStackOp::Noop;
    for (at, op) in (ops.iter().map(|(at, op)| (*at, op))).chain([(len, &noop)]) {
        let at = at.min(len);
        if at > pos {
            let id = interner.id_under(stack.as_slice(), outer);
            match runs.last_mut() {
                Some(last) if last.1 == id => last.0 += at - pos,
                _ => runs.push((at - pos, id)),
            }
            pos = at;
        }
        stack.apply(op).ok();
    }
    runs.into()
}

#[derive(Clone)]
pub struct Highlighter {
    explicit: Option<String>,
    search_identity: u64,
    assets: Arc<Assets>,
    syntax: usize,
    tree: Option<crate::treesitter::State>,
    /// The registered language this buffer was found to be, or chosen as.
    language: Option<Arc<LanguageDef>>,
    /// Whether the syntax is the language's own; not when a loose `Syntaxes/` grammar claims a
    /// bundled language's files.
    named: bool,
    /// Entry `i` holds the parser state after line `start + i` and that line's coloured runs.
    cache: Vec<LineState>,
    start: usize,
    /// The entries an edit cut off the cache, from line `stale_from` of the buffer as it was
    /// when it had `stale_lines` lines: a line whose text and starting state match its entry
    /// takes the entry instead of being parsed again, so typing re-parses a line or two.
    stale: Vec<LineState>,
    stale_from: usize,
    stale_lines: usize,
    /// The buffer's line count at the last `line_runs`.
    lines: usize,
    /// How far past the cache `line_runs` parses on rather than restarting above the line.
    lookahead: usize,
    /// A `job` that carries on from its source's last cached line.
    seeded: bool,
    /// Lines parsed so far, and the count at which this frame's budget runs out.
    parsed: usize,
    budget: usize,
    /// The lowest line an edit invalidated since the last `job` started: what the job
    /// parsed above it still holds when it lands (`adopt`).
    edited: Option<usize>,
    /// What this frame has parsed, in bytes, and when its time is up (`begin_frame`).
    frame_bytes: usize,
    deadline: Option<Instant>,
    /// An edit above the cache left its lines in place: they draw while their text is the
    /// same, until a job parses them again (`adopt`).
    provisional: bool,
    /// Where the cache's first line starts (chars) in the text as last seen, and the lines
    /// edits above it have added or removed since: the cache moves by those, not by edits
    /// below it (`text_changed`).
    start_char: usize,
    shift: isize,
    /// This frame stopped walking unchanged lines at its time or hashing budget: the next
    /// frame walks on, so no job is needed.
    walking: bool,
    /// What this frame has hashed, in bytes.
    frame_hashed: usize,
    /// A job's: set when the view it is for closes; the job stops at its next line.
    cancel: Option<Arc<AtomicBool>>,
    /// Tests: this highlighter's jobs panic.
    #[cfg(any(test, feature = "test-support"))]
    pub panics: bool,
    /// Tests: told each line a copy's `LineRuns` is asked for.
    #[cfg(any(test, feature = "test-support"))]
    pub probe: Option<Arc<dyn Fn(usize) + Send + Sync>>,
}

/// A jump further than this past the cache restarts the parser `LOOKBACK` lines above the
/// target instead of parsing everything in between (5s for 60k lines).
const LOOKAHEAD: usize = 500;
const LOOKBACK: usize = 200;
/// A frame parses at most this many lines past the cache (about 3 ms); further lines are
/// parsed by a `job` on another thread, which carries on from the cache up to `CATCH_UP`
/// lines (about a second) before it restarts above the line instead.
const MAX_GAP: usize = 50;
/// A frame also stops parsing past this many bytes (long lines: 50 lines of 15 KB took
/// 181 ms in one frame); a line longer than that always goes to the job. Bytes, not time:
/// a grammar compiles its patterns on first use, and a deadline would leave a new file's
/// first frame plain.
const FRAME_BYTES: usize = 16 * 1024;
/// Walking the lines an edit left unchanged hashes at most this many bytes a frame (about
/// 0.7 ms), and stops after `FRAME_TIME`.
const HASH_BYTES: usize = 1 << 20;
/// Walking the lines an edit left unchanged stops after this long.
const FRAME_TIME: Duration = match cfg!(debug_assertions) {
    true => Duration::from_millis(100),
    false => Duration::from_millis(4),
};
const CATCH_UP: usize = 20_000;
/// The cache keeps at most this many lines (about 1.6 KB each); past it the oldest half goes,
/// as if a jump had restarted the parser below them.
const MAX_CACHE: usize = 20_000;
/// Lines longer than this many bytes are not parsed (VS Code's `maxTokenizationLineLength`).
pub(crate) const MAX_HIGHLIGHT: usize = 20_000;

/// Line comment markers by syntax name (bundled or installed).
const LINE_COMMENTS: &[(&[&str], &str)] = &[
    (
        &[
            "Rust",
            "C",
            "C++",
            "HolyC",
            "C#",
            "Java",
            "JavaScript",
            "TypeScript",
            "TypeScript (TSX)",
            "Kotlin",
            "Swift",
            "Go",
            "Scala",
            "Objective-C",
            "Objective-C++",
            "D",
            "PHP",
            "Groovy",
            "JSON",
        ],
        "//",
    ),
    (
        &[
            "Python",
            "Ruby",
            "Bourne Again Shell (bash)",
            "Shell-Unix-Generic",
            "YAML",
            "Makefile",
            "Perl",
            "R",
            "Tcl",
            "HCL",
        ],
        "#",
    ),
    (&["Lua", "SQL", "Haskell"], "--"),
    (&["FreeBASIC"], "'"),
];

/// Block comment pairs where they are not C's.
const BLOCK_COMMENTS: &[(&[&str], (&str, &str))] = &[
    (&["HTML", "XML", "Markdown"], ("<!-- ", " -->")),
    (&["Lua"], ("--[[", "]]")),
    (&["CSS"], ("/*", "*/")),
    (&["HCL"], ("/*", "*/")),
    (&["FreeBASIC"], ("/'", "'/")),
];

fn table_comment_prefix(name: &str) -> Option<&'static str> {
    LINE_COMMENTS.iter().find(|(n, _)| n.contains(&name)).map(|c| c.1)
}

fn table_block_comment(name: &str) -> Option<(&'static str, &'static str)> {
    BLOCK_COMMENTS
        .iter()
        .find(|(n, _)| n.contains(&name))
        .map(|c| c.1)
        .or_else(|| table_comment_prefix(name).filter(|p| *p == "//").map(|_| ("/*", "*/")))
}

type CommentMarkers = (Option<&'static str>, Option<(&'static str, &'static str)>);

/// The markers of the language registered as `name`, if its manifest declares any; the tables
/// cover the rest.
fn registered_comments(name: &str) -> Option<CommentMarkers> {
    let registry = crate::registry::registry();
    let comments = &registry.named(name)?.manifest.comments;
    if comments.line.is_none() && comments.block.is_none() {
        return None;
    }
    Some((
        comments.line.as_deref().map(intern),
        comments.block.as_ref().map(|[open, close]| (intern(open), intern(close))),
    ))
}

/// A manifest's marker for as long as Orlo runs; each distinct one is kept once, however often
/// languages reload.
fn intern(marker: &str) -> &'static str {
    static MARKERS: std::sync::Mutex<std::collections::BTreeSet<&'static str>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    let mut markers = MARKERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(kept) = markers.get(marker) {
        return kept;
    }
    let kept: &'static str = Box::leak(marker.into());
    markers.insert(kept);
    kept
}

impl Highlighter {
    pub fn new(path: Option<&Path>) -> Self {
        Self::detect(path, "")
    }

    /// The registered language of `path` and `head` (the start of the text, up to
    /// `registry::CONTENT_BYTES`; its first line is enough without `files.content` patterns),
    /// else the syntax for `path` by its extension or name, else by its first line (a shebang:
    /// `#!/usr/bin/env python3` is Python, as in Sublime).
    pub fn detect(path: Option<&Path>, head: &str) -> Self {
        Self::detect_with_assets(path, head, assets())
    }

    fn detect_with_assets(path: Option<&Path>, head: &str, assets: Arc<Assets>) -> Self {
        let ss = &assets.syntaxes;
        let by = |part: Option<&str>| part.and_then(|p| ss.find_syntax_by_extension(p));
        let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str());
        // Syntect matches extensions without case; HolyC's .HH must not claim C++'s .hh.
        // An installed header grammar still takes precedence over this bundled alias.
        let custom_header = ext == Some("HH")
            && ss.syntaxes()[assets.bundled..]
                .iter()
                .any(|s| s.file_extensions.iter().any(|e| e.eq_ignore_ascii_case("HH")));
        // so must the registry, which matches extensions without case too
        let registry = crate::registry::registry();
        let holyc_header = ext == Some("HH") && !custom_header;
        if let Some(language) = registry.detect(path, head).filter(|_| !holyc_header) {
            if let Some(highlighter) = Self::registered(assets.clone(), &registry, language, path) {
                return highlighter;
            }
        }
        let first_line = &head[..head.find('\n').map_or(head.len(), |i| i + 1)];
        // a pipeline is JSON with comments, a snippet XML
        let ext = match ext {
            Some("HH") if !custom_header => Some("hc"),
            Some("orlo-pipeline") => Some("json"),
            Some(orlo_editor::snippet::EXT) => Some("xml"),
            ext => ext,
        };
        let language = path.and_then(crate::language::Language::detect);
        let syntax = by(ext)
            .or_else(|| by(path.and_then(|p| p.file_name()).and_then(|n| n.to_str())))
            .or_else(|| language.and_then(|l| ss.find_syntax_by_name(crate::treesitter::name(l))))
            .or_else(|| language.and_then(|l| ss.find_syntax_by_name(crate::treesitter::fallback_name(l))))
            .or_else(|| ss.find_syntax_by_first_line(first_line))
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let index = index_of(ss, syntax);
        Self::with_syntax(assets, index, language.map(crate::treesitter::Grammar::builtin))
    }

    /// A registered language's highlighter: its syntax (the bundled one it names, or its
    /// folder's), and its tree when it has a grammar. `None` when its grammar failed in this
    /// generation, or its folder's syntax did not load and it has no grammar: the file is
    /// detected as if the folder were not there.
    fn registered(
        assets: Arc<Assets>,
        registry: &Arc<crate::registry::Registry>,
        language: &Arc<LanguageDef>,
        path: Option<&Path>,
    ) -> Option<Self> {
        let grammar = crate::treesitter::Grammar::of(registry, language);
        if grammar.as_ref().is_some_and(|g| g.failed()) {
            return None;
        }
        let ss = &assets.syntaxes;
        let by = |part: Option<&std::ffi::OsStr>| part.and_then(|p| ss.find_syntax_by_extension(p.to_str()?));
        // a loose `Syntaxes/` grammar for a bundled language's files still replaces its colours
        let loose = (language.origin == Origin::Bundled)
            .then(|| by(path.and_then(Path::extension)).or_else(|| by(path.and_then(Path::file_name))))
            .flatten()
            .map(|syntax| index_of(ss, syntax))
            .filter(|&i| i >= assets.bundled && !ss.syntaxes()[i].variables.contains_key(LANGUAGE_VARIABLE));
        let mut highlighter = match (loose, assets.language_syntax(language)) {
            (Some(syntax), _) => Self::with_syntax(assets, syntax, None),
            (None, syntax) => {
                if syntax.is_none() && grammar.is_none() && language.manifest.syntax.is_some() {
                    return None;
                }
                let plain = || index_of(ss, ss.find_syntax_plain_text());
                let mut highlighter = Self::with_syntax(assets.clone(), syntax.unwrap_or_else(plain), None);
                highlighter.tree = grammar.map(crate::treesitter::State::with_grammar);
                highlighter.named = true;
                highlighter
            }
        };
        highlighter.language = Some(language.clone());
        Some(highlighter)
    }

    /// A bundled syntax gets the tree of `grammar`, else of the language it is named for.
    fn with_syntax(assets: Arc<Assets>, syntax: usize, grammar: Option<Arc<crate::treesitter::Grammar>>) -> Self {
        let tree = (syntax < assets.bundled)
            .then(|| {
                let named = || crate::treesitter::named(&assets.syntaxes.syntaxes()[syntax].name);
                grammar.or_else(|| named().map(crate::treesitter::Grammar::builtin))
            })
            .flatten()
            .map(crate::treesitter::State::with_grammar);
        Self {
            explicit: None,
            search_identity: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, Ordering::Relaxed)
            },
            assets,
            syntax,
            tree,
            language: None,
            named: false,
            cache: Vec::new(),
            start: 0,
            stale: Vec::new(),
            stale_from: 0,
            stale_lines: 0,
            lines: 0,
            lookahead: LOOKAHEAD,
            seeded: false,
            parsed: 0,
            budget: usize::MAX,
            edited: None,
            frame_bytes: 0,
            deadline: None,
            provisional: false,
            start_char: 0,
            shift: 0,
            walking: false,
            frame_hashed: 0,
            cancel: None,
            #[cfg(any(test, feature = "test-support"))]
            panics: false,
            #[cfg(any(test, feature = "test-support"))]
            probe: None,
        }
    }

    /// A frame starts: from here `ready` lets it parse `MAX_GAP` lines (lines an edit left
    /// unchanged come back from the cache and do not count).
    pub fn begin_frame(&mut self) {
        self.budget = self.parsed + MAX_GAP;
        (self.frame_bytes, self.frame_hashed, self.walking) = (0, 0, false);
        self.deadline = Some(Instant::now() + FRAME_TIME);
    }

    /// The last frame stopped on its way over unchanged lines: draw another, no job.
    pub fn walking(&self) -> bool {
        self.walking
    }

    /// Colours for line `i` to draw while it waits for a job: those it had before an edit cut
    /// it from the cache, if its text is the same. `None` for a line whose text changed.
    pub fn fallback_runs(&mut self, rope: &Rope, i: usize) -> Option<Vec<Run>> {
        let old = (i + self.stale_lines).checked_sub(rope.len_lines())?;
        let entry = self.stale.get(old.checked_sub(self.stale_from)?)?;
        (entry.same_text(rope, i, &mut self.frame_hashed)).then(|| entry.runs.clone())
    }

    /// The text changed by `changes`, as the editor logged them: the line count follows, and
    /// a cache below the edits moves by the line breaks they added or removed above it.
    pub fn text_changed(&mut self, changes: &[Change]) {
        if let Some(tree) = self.tree.as_mut() {
            tree.text_changed(changes);
        }
        for c in changes {
            let moved = c.lines.1 as isize - c.lines.0 as isize;
            self.lines = (self.lines as isize + moved).max(0) as usize;
            // an edit at the cache's own lines cut it (`invalidate_from`): nothing to move
            if self.cache.is_empty() {
                continue;
            }
            if c.start + c.removed <= self.start_char {
                self.start_char = self.start_char + c.inserted - c.removed;
                self.shift += moved;
            } else if c.start < self.start_char {
                // into the cache's first line: where it went is not known
                self.cache.clear();
                (self.provisional, self.shift) = (false, 0);
            }
        }
    }

    /// Note where the cache starts in `rope`, for `text_changed`.
    fn anchor(&mut self, rope: &Rope) {
        self.start_char = rope.line_to_char(self.start.min(rope.len_lines()));
    }

    fn out_of_time(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    /// Provisional colors draw while a background job catches up with edits or tree queries.
    pub fn provisional(&self) -> bool {
        self.provisional || self.tree.as_ref().is_some_and(|t| t.requested)
    }

    /// `line_runs` within what is left of the frame's budget: walks to the line taking what
    /// an edit left unchanged (free) and parsing the rest while the budget lasts. `None` when
    /// the line is above the cache, far past it, or the budget runs out on the way: the line
    /// draws plain while a `job` catches up.
    pub fn try_line_runs(&mut self, rope: &Rope, line: usize) -> Option<Vec<Run>> {
        if let Some(tree) = self.tree.as_mut() {
            if let Some(runs) = tree.runs(rope, line) {
                return Some(runs);
            }
            tree.request(rope, line);
        }
        self.settle_shift(rope);
        if self.provisional {
            // only the lines it has, and only while their text is the same
            let entry = self.cache.get(line.checked_sub(self.start)?)?;
            return (entry.same_text(rope, line, &mut self.frame_hashed)).then(|| entry.runs.clone());
        }
        if self.cache.is_empty() && self.stale.is_empty() && self.start > 0 {
            // emptied above where it was: start over above this line, as a jump does
            self.start = line.saturating_sub(LOOKBACK);
        }
        self.anchor(rope);
        let end = self.start + self.cache.len();
        // out of the budget's reach, unless an edit's unchanged lines can be walked to free
        let far = line >= end + self.budget.saturating_sub(self.parsed);
        if line < self.start || far && self.stale.is_empty() {
            return None;
        }
        let mut walked = 0;
        let last = line.min(rope.len_lines() - 1);
        while self.start + self.cache.len() <= last {
            let i = self.start + self.cache.len();
            match self.reuse(rope, i) {
                Some(entry) => {
                    self.cache.push(entry);
                    walked += 1;
                    if self.frame_hashed > HASH_BYTES || walked % 16 == 0 && self.out_of_time() {
                        self.walking = true;
                        return None;
                    }
                }
                None if self.parsed >= self.budget => return None,
                None => {
                    // a line too long to colour costs nothing; a long one, or the frame's
                    // bytes spent: the job colours it
                    let len = rope.line(i).len_bytes();
                    if len <= MAX_HIGHLIGHT {
                        if self.frame_bytes + len > FRAME_BYTES {
                            return None;
                        }
                        self.frame_bytes += len;
                    }
                    self.parse_next(rope);
                }
            }
        }
        Some(self.line_runs(rope, line))
    }

    /// Move the cache by the lines edits above it added or removed.
    fn settle_shift(&mut self, rope: &Rope) {
        if self.shift != 0 {
            self.start = (self.start as isize + self.shift).max(0) as usize;
            self.shift = 0;
        }
        self.lines = rope.len_lines();
    }

    /// A highlighter to run on another thread up to `line`: it carries on from the last
    /// cached line when that is near enough (so colours stay right), else restarts above it.
    pub fn job(&mut self, line: usize) -> Highlighter {
        self.edited = None;
        let mut h = Self::with_syntax(self.assets.clone(), self.syntax, None);
        h.tree = self.tree.clone();
        (h.language, h.named) = (self.language.clone(), self.named);
        #[cfg(any(test, feature = "test-support"))]
        {
            h.panics = self.panics;
        }
        let end = self.start + self.cache.len();
        // a provisional cache's states may be wrong: the job starts afresh above the line
        match self.cache.last().filter(|_| !self.provisional && line >= end && line - end <= CATCH_UP) {
            Some(last) => {
                h.cache.push(last.clone());
                (h.start, h.seeded) = (end - 1, true);
            }
            // restart above the line once; the job never restarts again however far it goes
            None => h.start = line.saturating_sub(LOOKBACK),
        }
        h.lookahead = usize::MAX;
        h
    }

    /// Stop this job at its next line once `cancel` is set (the view it is for closed).
    pub fn cancel_on(&mut self, cancel: Arc<AtomicBool>) {
        self.cancel = Some(cancel);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cached_lines(&self) -> usize {
        self.cache.len()
    }

    /// Lines parsed so far (not taken unchanged from before an edit).
    #[cfg(any(test, feature = "test-support"))]
    pub fn parsed(&self) -> usize {
        self.parsed
    }

    /// Take what a `job` parsed for the same text (`rope` as it is now): appended when it
    /// carried on from this cache's end, else in place of the cache.
    pub fn adopt(&mut self, mut job: Highlighter, rope: &Rope) {
        if !Arc::ptr_eq(&job.assets, &self.assets) || job.syntax != self.syntax {
            return;
        }
        if let (Some(current), Some(tree)) = (self.tree.as_mut(), job.tree.take()) {
            current.adopt(tree);
        }
        // the text changed while the job ran: its lines above the change still hold, and
        // those below draw while typing goes on (`fallback_runs`), until a job matches
        if let Some(l) = self.edited {
            let below = (job.cache).split_off(l.saturating_sub(job.start).min(job.cache.len()));
            if !below.is_empty() {
                let from = job.start + job.cache.len();
                (self.stale, self.stale_from, self.stale_lines) = (below, from, job.lines);
            }
        }
        if job.cache.is_empty() {
            return;
        }
        let end = self.start + self.cache.len();
        // carried on from a line this cache still has (it may have grown since)
        if job.seeded && !self.provisional && (self.start..end).contains(&job.start) {
            self.cache.truncate(job.start + 1 - self.start);
            self.cache.extend(job.cache.into_iter().skip(1));
            return;
        }
        let old = std::mem::replace(&mut self.cache, job.cache);
        // a restart above the cache: its lines below wait to be walked to, not parsed again
        if !self.provisional && self.edited.is_none() && self.start > job.start && !old.is_empty() {
            (self.stale, self.stale_from, self.stale_lines) = (old, self.start, self.lines);
        } else if self.edited.is_none() {
            self.stale.clear();
        }
        self.start = job.start;
        // where the cache starts in the text as it is now, not as the job saw it
        self.anchor(rope);
        (self.provisional, self.shift) = (false, 0);
    }

    /// View > Syntax: a fresh highlighter for the language with this id or name, else the
    /// named syntax (plain text if unknown).
    pub fn by_name(name: &str) -> Self {
        let registry = crate::registry::registry();
        let assets = assets();
        let language = registry.get(name).or_else(|| registry.named(name));
        if let Some(mut highlighter) = language.and_then(|l| Self::registered(assets.clone(), &registry, l, None)) {
            let by_id = registry.get(name).is_some();
            highlighter.explicit = Some(if by_id { name.into() } else { highlighter.syntax_name().into() });
            return highlighter;
        }
        let language = registry.get(name);
        let syntax_name = language.map_or(name, |l| l.name());
        let ss = &assets.syntaxes;
        let syntax = ss
            .find_syntax_by_name(syntax_name)
            .or_else(|| {
                crate::treesitter::named(syntax_name)
                    .and_then(|l| ss.find_syntax_by_name(crate::treesitter::fallback_name(l)))
            })
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let index = index_of(ss, syntax);
        let grammar = crate::treesitter::named(syntax_name).map(crate::treesitter::Grammar::builtin);
        let mut highlighter = Self::with_syntax(assets, index, grammar);
        highlighter.explicit = Some(if language.is_some() { name.into() } else { highlighter.syntax_name().into() });
        highlighter
    }

    /// A registry-admitted file declaration uses the existing grammar without becoming a
    /// deliberate View > Syntax choice. Unknown registry grammars retain the plain fallback.
    pub fn declared(name: &str) -> Self {
        let mut highlighter = Self::by_name(name);
        highlighter.explicit = None;
        highlighter
    }

    /// Keep the selected language while dropping parser and color caches from old assets.
    pub fn reload(&mut self) {
        let explicit = self.explicit.clone();
        let detected = self.language.as_ref().filter(|_| self.named).map(|l| l.id().to_string());
        *self = Self::by_name(explicit.as_deref().or(detected.as_deref()).unwrap_or_else(|| self.syntax_name()));
        self.explicit = explicit;
    }

    /// The registered language this buffer is, if any: its comments, indent and server.
    pub fn language(&self) -> Option<&Arc<LanguageDef>> {
        self.language.as_ref()
    }

    /// A deliberate View > Syntax selection, retained through customization reloads.
    pub fn language_override(&self) -> Option<&str> {
        self.explicit.as_deref()
    }

    fn syntax(&self) -> &SyntaxReference {
        &self.assets.syntaxes.syntaxes()[self.syntax]
    }

    /// Every syntax the View > Syntax menu can switch to: the registered languages, and the
    /// syntaxes and built-in parsers no language claims.
    pub fn syntax_names() -> Vec<String> {
        let registry = crate::registry::registry();
        let mut v: Vec<String> =
            assets().syntaxes.syntaxes().iter().filter(|s| !s.hidden).map(|s| s.name.clone()).collect();
        v.extend(registry.languages().iter().filter(|l| !l.manifest.hidden).map(|l| l.name().to_string()));
        v.extend(
            (crate::language::Language::ALL.iter())
                .filter(|l| registry.get(l.registry_id()).is_none())
                .map(|&l| crate::treesitter::name(l).to_string()),
        );
        v.sort_by_key(|n| n.to_lowercase());
        v.dedup();
        v
    }

    pub fn syntax_name(&self) -> &str {
        self.identity().map_or(&self.syntax().name, |(name, _)| name)
    }

    /// The name and base scope of the language the buffer is coloured as, where that is not
    /// simply its syntect syntax: its tree's, else its registered language's.
    fn identity(&self) -> Option<(&str, &str)> {
        let tree = self.tree.as_ref().map(|t| (t.grammar.name(), t.grammar.scope()));
        // a language with neither a syntax nor a grammar (a server alone) is plain text, named so
        let language = self.language.as_ref().filter(|l| self.named && l.manifest.syntax.is_some());
        let language = || language.map(|l| (l.name(), l.manifest.scope.as_str()));
        tree.or_else(language)
    }

    /// What a syntect stack's outermost scope is read as: the language's scope where its lines
    /// go through another grammar (TypeScript's through JavaScript's), so a `source.ts`
    /// selector matches before the tree colours a line as after.
    fn outer_scope(&self) -> Option<Scope> {
        let (name, scope) = self.identity()?;
        // a name compared a line, not a scope built a line
        (self.syntax().name != name).then(|| Scope::new(scope).ok()).flatten()
    }

    /// The syntax's base scope ("source.rust", "text.plain"), which `word_wrap: "auto"` reads.
    pub fn scope(&self) -> String {
        self.identity().map_or_else(|| self.syntax().scope.build_string(), |(_, scope)| scope.to_string())
    }

    /// The scope stack of the char at `offset` (chars; at a line's end, of its last char) as
    /// its line was coloured: the syntax's top scope on a line with no colours cached (nothing
    /// is parsed here).
    #[cfg(any(test, feature = "test-support"))]
    pub fn scope_at(&self, rope: &Rope, offset: usize) -> Arc<[Scope]> {
        scope_stack(self.scope_id_at(rope, offset))
    }

    /// `scope_at`'s stack by id, for `matches` and `class`: a snippet's Tab trigger, which cannot
    /// wait for a line just typed on to be coloured (completion, which can, uses
    /// `cached_scope_id`).
    pub fn scope_id_at(&self, rope: &Rope, offset: usize) -> ScopeId {
        (self.cached_scope_id(rope, offset)).unwrap_or_else(|| scope_id(&[self.scope().as_str()]))
    }

    /// `scope_id_at` where the line's colours are cached for this text, else `None`: a line just
    /// typed on has none until the next frame colours it, and would read as the top scope.
    pub fn cached_scope_id(&self, rope: &Rope, offset: usize) -> Option<ScopeId> {
        let offset = offset.min(rope.len_chars());
        let line = rope.char_to_line(offset);
        let byte = rope.char_to_byte(offset) - rope.line_to_byte(line);
        let runs = self.cached_scope_runs(rope, line)?;
        let mut end = 0;
        let at = runs.iter().find(|r| {
            end += r.0;
            byte < end
        });
        // an empty line is coloured too: the top scope
        Some(at.or(runs.last()).map_or_else(|| scope_id(&[self.scope().as_str()]), |r| r.1))
    }

    /// The cached line's byte-length/scope runs, without parsing or copying its text.
    pub(crate) fn cached_scope_runs(&self, rope: &Rope, line: usize) -> Option<&[(usize, ScopeId)]> {
        self.tree.as_ref().and_then(|t| t.scopes(rope, line)).or_else(|| {
            let entry = self.cache.get(line.checked_sub(self.start)?)?;
            (!self.provisional).then_some(&entry.scopes[..])
        })
    }

    /// Current worker-discovered regions; absent while parsing or for fallback grammars.
    pub fn fold_regions(&self, rope: &Rope) -> Option<&orlo_editor::fold::Regions> {
        self.tree.as_ref()?.fold_regions(rope)
    }

    /// What Enter indents by, from the language's `indents.scm`: `None` without one, or until the
    /// tree is parsed for `rope`.
    pub fn indents(&self, rope: &Rope) -> Option<crate::treesitter::Indents> {
        self.tree.as_ref()?.indents(rope)
    }

    /// A parser identity/revision for transient syntax selection history; absent without a bundled parser.
    pub fn syntax_selection_source(&self, rope: &Rope) -> Option<(u64, u64)> {
        self.tree.as_ref()?.source_key(rope)
    }

    /// None means not current; Some(None) means there is no larger named source range.
    pub fn enclosing_named_range(&self, rope: &Rope, range: Range<usize>) -> Option<Option<Range<usize>>> {
        self.tree.as_ref()?.enclosing_named_range(rope, range)
    }

    /// Apostrophes start Rust lifetimes and some languages' comments (FreeBASIC's), so they
    /// type plainly.
    pub fn pairs_single_quote(&self) -> bool {
        self.syntax_name() != "Rust" && self.comment_prefix() != Some("'")
    }

    /// The block comment pair, with the spaces Sublime puts inside (HTML's, not CSS's): the
    /// registered language's, else the table's.
    pub fn block_comment(&self) -> Option<(&'static str, &'static str)> {
        let name = self.syntax_name();
        registered_comments(name).map_or_else(|| table_block_comment(name), |c| c.1)
    }

    /// `(line, name, kind)` of every definition, by Sublime's `entity.name` scopes (tags
    /// excluded); the kind is the scope's next part ("function", "impl", "struct", ...). Parsed
    /// on the spot: the app parses on the highlighting thread (`parse_on_worker`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn symbols(&self, rope: &Rope) -> Vec<Symbol> {
        self.symbols_until(rope, || false)
    }

    /// `symbols`, given up (with what it has) once `stop` says so.
    #[cfg(any(test, feature = "test-support"))]
    pub fn symbols_until(&self, rope: &Rope, stop: impl Fn() -> bool) -> Vec<Symbol> {
        let mut p = self.symbol_parse();
        p.step(rope, stop);
        (p.found.into_iter()).map(|s| (s.line, s.name, s.kind)).collect()
    }

    /// A symbol parse from the first line, run by `SymbolParse::step`.
    pub fn symbol_parse(&self) -> SymbolParse {
        SymbolParse {
            reuse: SymbolTransitions::default(),
            assets: self.assets.clone(),
            syntax: self.syntax,
            parse: ParseState::new(self.syntax()),
            stack: ScopeStack::new(),
            docstrings: DOCSTRING_SYNTAXES.contains(&self.syntax().name.as_str()),
            line: 0,
            found: Vec::new(),
            docs: Docs::default(),
            kept: None,
            ahead: VecDeque::new(),
            splice: Splice::default(),
            tags: self.tags(),
            #[cfg(any(test, feature = "test-support"))]
            parsed: 0,
        }
    }

    /// A tags parse, for a language folder with `tags.scm` and a tree but no regex grammar.
    fn tags(&self) -> Option<crate::treesitter::Tags> {
        let grammar = &self.tree.as_ref()?.grammar;
        grammar.language()?.manifest.tree_sitter.as_ref()?.tags.as_ref()?;
        let plain = std::ptr::eq(self.syntax(), self.assets.syntaxes.find_syntax_plain_text());
        plain.then(|| crate::treesitter::Tags::new(grammar.clone()))
    }

    /// The line comment marker: the registered language's, else the table's.
    pub fn comment_prefix(&self) -> Option<&'static str> {
        let name = self.syntax_name();
        registered_comments(name).map_or_else(|| table_comment_prefix(name), |c| c.0)
    }

    pub fn invalidate_from(&mut self, line: usize) {
        if let Some(tree) = self.tree.as_mut() {
            tree.invalidate();
        }
        self.edited = Some(self.edited.map_or(line, |l| l.min(line)));
        // above the cache (another view's edit, far up): its lines stay, provisionally
        if line < self.start && !self.cache.is_empty() && (self.stale.is_empty() || self.provisional) {
            self.provisional = true;
            return;
        }
        if self.provisional {
            self.cache.clear();
            (self.provisional, self.shift) = (false, 0);
        }
        let end = self.start + self.cache.len();
        match line.checked_sub(self.start) {
            Some(n) if n < self.cache.len() => {
                let mut cut = self.cache.split_off(n);
                // a second edit above the first: the lines between keep their numbers
                if !self.stale.is_empty() && self.stale_from == end {
                    cut.append(&mut self.stale);
                } else {
                    // a frame walked the cache on into the stale entries since the last edit:
                    // the ones past its end still lie below it, so keep them (reuse checks
                    // each one's text and starting state, so a misplaced one is only not used)
                    let walked = (end + self.stale_lines).checked_sub(self.lines + self.stale_from);
                    if let Some(k) = walked.filter(|&k| k < self.stale.len()) {
                        cut.extend(self.stale.drain(k..));
                    }
                    self.stale_lines = self.lines;
                }
                (self.stale, self.stale_from) = (cut, line);
            }
            Some(_) => {}
            None => {
                self.cache.clear();
                self.stale.clear();
            }
        }
    }

    /// Line `i`'s entry from before the edit, if its text and the state it starts in are
    /// unchanged; `cache` holds the lines above it.
    fn reuse(&mut self, rope: &Rope, i: usize) -> Option<LineState> {
        let prev = self.cache.last()?;
        let old = (i + self.stale_lines).checked_sub(rope.len_lines())?;
        let k = old.checked_sub(self.stale_from).filter(|&k| k >= 1)?;
        let (before, entry) = (self.stale.get(k - 1)?, self.stale.get(k)?);
        let same_state = Arc::ptr_eq(&before.state, &prev.state) || before.state == prev.state;
        (same_state && entry.same_text(rope, i, &mut self.frame_hashed)).then(|| entry.clone())
    }

    /// `(byte_len, colour)` runs covering the line without its line break.
    /// Parse the line after the cache and push it.
    fn parse_next(&mut self, rope: &Rope) {
        let assets = self.assets.clone();
        let ss = &assets.syntaxes;
        let highlighter = assets.highlighter;
        let foreground = assets.theme().settings.foreground;
        let default_fg = to_hsla(foreground.unwrap_or(color(0xd8dee9)));
        let default_bg = assets.background();
        let i = self.start + self.cache.len();
        self.parsed += 1;
        let (mut parse, mut hstate) = match self.cache.last() {
            Some(prev) => prev.state.as_ref().clone(),
            None => (ParseState::new(self.syntax()), HighlightState::new(highlighter, ScopeStack::new())),
        };
        let line = rope.line(i);
        let len = line.len_bytes();
        // a line too long to colour is drawn in the colour of the scope it starts
        // in, and the parse carries on past it, so a minified file costs no 600 ms per
        // keystroke; a string or comment such a line opens or closes is missed (pinned by a
        // test); Sublime colours them, slowly. Its text is not copied.
        if len > MAX_HIGHLIGHT {
            let content = len - if crlf(line) { 2 } else { usize::from(line.char(line.len_chars() - 1) == '\n') };
            let piece = HighlightIterator::new(&mut hstate, &[], "\n", highlighter).next();
            let (fg, look) = piece.map_or((default_fg, Look::default()), |(style, _)| styled(style, default_bg));
            let stack = match hstate.path.as_slice() {
                // the first line: its syntax's scope is pushed as it is parsed
                [] => std::slice::from_ref(&self.syntax().scope),
                stack => stack,
            };
            let scope = interner().id_under(stack, self.outer_scope());
            let state = match self.cache.last() {
                Some(prev) => prev.state.clone(),
                None => Arc::new((parse, hstate)),
            };
            self.cache.push(LineState {
                state,
                runs: vec![(content, fg, look)],
                scopes: [(content, scope)].into(),
                text: crlf(line) as u64,
                len,
            });
            return;
        }
        let mut parse_text = line.to_string();
        let text = hash_str([parse_text.as_str()]);
        let content = parse_text.strip_suffix('\n').map_or(parse_text.as_str(), |t| t.strip_suffix('\r').unwrap_or(t));
        let content_len = content.len();
        // the "newlines" syntax set wants every line to end in '\n'
        if !parse_text.ends_with('\n') {
            parse_text.push('\n');
        }
        let ops = parse.parse_line(&parse_text, ss).unwrap_or_default();
        let scopes = scope_runs(hstate.path.clone(), &ops, content_len, self.outer_scope());
        let mut runs: Vec<Run> = Vec::new();
        let mut pos = 0;
        for (style, piece) in HighlightIterator::new(&mut hstate, &ops, &parse_text, highlighter) {
            let len = (pos + piece.len()).min(content_len).saturating_sub(pos);
            pos += piece.len();
            if len == 0 {
                continue;
            }
            let (c, look) = styled(style, default_bg);
            match runs.last_mut() {
                Some(last) if (last.1, last.2) == (c, look) => last.0 += len,
                _ => runs.push((len, c, look)),
            }
        }
        if runs.is_empty() && content_len > 0 {
            runs.push((content_len, default_fg, Look::default()));
        }
        // code mostly moves between a few states (a block, a call spread over lines): reuse
        // one of the last few seen
        let seen = self.cache.iter().rev().take(64).map(|e| &e.state);
        let recent = seen.fold(Vec::<&Arc<_>>::new(), |mut v, a| {
            if v.len() < 8 && v.last().is_none_or(|l| !Arc::ptr_eq(l, a)) {
                v.push(a);
            }
            v
        });
        let state = (recent.into_iter())
            .find(|a| a.0 == parse && a.1 == hstate)
            .cloned()
            .unwrap_or_else(|| Arc::new((parse, hstate)));
        runs.shrink_to_fit();
        self.cache.push(LineState { state, runs, scopes, text, len });
    }

    /// Tree-sitter's colours for lines `from..=to` (none for a syntax without a tree), found on
    /// the worker; whether `from` has them now. A job gives the thread up to urgent work and
    /// stops for a closed view; a copy (`Some(stop)`) is urgent itself, and stops only when
    /// `stop` is set.
    fn prepare_tree(&mut self, rope: &Rope, from: usize, to: usize, copy: Option<&AtomicBool>) -> bool {
        let Some(tree) = self.tree.as_mut() else {
            return false;
        };
        let assets = &self.assets;
        let cancel = &self.cancel;
        let grammar = tree.grammar.clone();
        let base = grammar.scope();
        let worker = std::thread::current().name() == Some("orlo-highlight");
        tree.prepare(
            rope,
            from..to.saturating_add(1),
            copy.is_none().then_some(Duration::from_millis(10)),
            || match copy {
                Some(stop) => stop.load(Ordering::Relaxed),
                None => cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) || worker && urgent_waiting(),
            },
            |scope| style_for_scope(assets, base, scope),
        );
        tree.queried(rope, from)
    }

    pub fn line_runs(&mut self, rope: &Rope, line: usize) -> Vec<Run> {
        if let Some(runs) = self.tree.as_ref().and_then(|t| t.runs(rope, line)) {
            return runs;
        }
        self.settle_shift(rope);
        let end = self.start + self.cache.len();
        let empty = self.cache.is_empty() && self.stale.is_empty() && self.start > 0;
        if self.provisional || empty || line < self.start || line > end.saturating_add(self.lookahead) {
            // a block comment or string opened above the restart point is missed;
            // parse on a background thread if that shows up in practice
            let old = std::mem::take(&mut self.cache);
            // above the cache: its lines wait below to be walked to, not parsed again
            if line < self.start && !self.provisional && !old.is_empty() {
                (self.stale, self.stale_from) = (old, self.start);
                self.stale_lines = rope.len_lines();
            } else {
                self.stale.clear();
            }
            (self.provisional, self.shift) = (false, 0);
            self.start = line.saturating_sub(LOOKBACK);
        }
        self.anchor(rope);
        let last = line.min(rope.len_lines() - 1);
        while self.start + self.cache.len() <= last {
            // a job for a closed view, or urgent work waiting: what is parsed so far is kept
            let cancel = self.cancel.as_ref();
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed) || urgent_waiting()) {
                break;
            }
            let i = self.start + self.cache.len();
            match self.reuse(rope, i) {
                Some(entry) => self.cache.push(entry),
                None => self.parse_next(rope),
            }
        }
        let runs = self.cache.get(line - self.start).map(|l| l.runs.clone()).unwrap_or_default();
        while self.cache.len() > MAX_CACHE && line >= self.start + MAX_CACHE / 2 {
            self.cache.drain(..MAX_CACHE / 2);
            self.start += MAX_CACHE / 2;
            self.stale.clear();
            self.anchor(rope);
        }
        runs
    }
}

/// Copy as HTML's colours: the runs of any line of `rope` from `job`, run on the worker (see
/// `on_worker`). Cheap in ascending order: tree-sitter's colours are found a stretch of lines at
/// a time, syntect's one line after another. A line over the colouring limit has no runs (the
/// default foreground), and nor has any line once `stop` is set.
// a first line far from what the view has parsed restarts 200 lines above itself (as a
// jump does), so a comment or string opened further up is missed; start from the top if that
// shows up in practice.
pub struct LineRuns {
    job: Highlighter,
    rope: Rope,
    /// The lines tree-sitter has coloured.
    ready: std::ops::Range<usize>,
    /// Set when nobody wants the colours any more.
    stop: Arc<AtomicBool>,
}

impl LineRuns {
    pub fn new(job: Highlighter, rope: Rope, stop: Arc<AtomicBool>) -> Self {
        Self { job, rope, ready: 0..0, stop }
    }

    pub fn line(&mut self, line: usize) -> Vec<Run> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(probe) = &self.job.probe {
            probe(line);
        }
        if self.stop.load(Ordering::Relaxed) || self.rope.line(line).len_bytes() > MAX_HIGHLIGHT {
            return Vec::new();
        }
        if !self.ready.contains(&line) {
            // half the cache's cap: the parse of these lines stays inside it
            let to = (line + MAX_CACHE / 2).min(self.rope.len_lines());
            // a copy is urgent itself and gives the thread to no one: were urgent work to stop
            // the tree, the rest would come from syntect, 8 times slower and not as the view
            // draws it
            if self.job.prepare_tree(&self.rope, line, to - 1, Some(&self.stop)) {
                self.ready = line..to;
            } else if self.stop.load(Ordering::Relaxed) {
                return Vec::new();
            }
        }
        self.job.line_runs(&self.rope, line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::block_on as futures_lite_block_on;

    #[test]
    fn syntax_selection_adopts_independently_detected_warm_tree_without_replacing_source_identity() {
        let rope = Rope::from_str("fn main() { café + 雪; }");
        let path = Some(Path::new("warm.rs"));
        let mut receiver = Highlighter::detect(path, "");
        let identity = receiver.syntax_selection_source(&rope);
        let warm = run_job(Highlighter::detect(path, ""), &rope, &[50]);
        assert_ne!(warm.syntax_selection_source(&rope), identity);
        receiver.adopt(warm, &rope);
        assert_eq!(receiver.enclosing_named_range(&rope, 13..13), Some(Some(12..16)));
        assert!(receiver.fold_regions(&rope).is_some(), "warm tree/folds reused without reparsing");
        assert_eq!(receiver.syntax_selection_source(&rope), identity);
    }

    #[test]
    fn syntax_selection_warmup_yields_to_urgent_work_and_normal_retry_prepares_current_tree() {
        let rope = Rope::from_str("fn main() { café + 雪; }");
        let path = Some(Path::new("warm.rs"));
        let mut receiver = Highlighter::detect(path, "");
        let identity = receiver.syntax_selection_source(&rope);
        assert!(identity.is_some());
        let warm_rope = rope.clone();
        let (warm, other) = futures_lite_block_on(on_worker_urgent(move || {
            // Enqueue eagerly from this running job, so another urgent request is known
            // to be waiting throughout this file-open-style warmup.
            let other = on_worker_urgent(|| ());
            let warm = run_job(Highlighter::detect(path, ""), &warm_rope, &[50]);
            assert!(urgent_waiting());
            assert_eq!(warm.enclosing_named_range(&warm_rope, 13..13), None);
            assert!(warm.fold_regions(&warm_rope).is_none());
            (warm, other)
        }))
        .unwrap();
        futures_lite_block_on(other).unwrap();
        receiver.adopt(warm, &rope);
        assert_eq!(receiver.enclosing_named_range(&rope, 13..13), None);
        assert!(receiver.fold_regions(&rope).is_none());
        assert_eq!(receiver.syntax_selection_source(&rope), identity);

        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let (job, text) = (receiver.job(0), rope.clone());
            let retry = futures_lite_block_on(on_worker(move || run_job(job, &text, &[0]))).unwrap();
            receiver.adopt(retry, &rope);
            if receiver.enclosing_named_range(&rope, 13..13).is_some() && receiver.fold_regions(&rope).is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "normal cooperative retry never prepared the current tree");
        }
        assert_eq!(receiver.enclosing_named_range(&rope, 13..13), Some(Some(12..16)));
        assert!(receiver.fold_regions(&rope).is_some());
        assert_eq!(receiver.syntax_selection_source(&rope), identity);
    }

    fn file(name: &str) -> Highlighter {
        Highlighter::new(Some(Path::new(name)))
    }

    fn syntect_only(mut h: Highlighter) -> Highlighter {
        h.tree = None;
        h
    }

    fn colour_all(h: &mut Highlighter, rope: &Rope) {
        for line in 0..rope.len_lines() {
            h.line_runs(rope, line);
        }
    }

    /// Whether the stack at char `at` has `scope`.
    fn has_scope(h: &Highlighter, rope: &Rope, at: usize, scope: &str) -> bool {
        names(&h.scope_at(rope, at)).iter().any(|s| s == scope)
    }

    /// `token`'s first char of `text` has `scope`, and is coloured as `scope` is.
    #[track_caller]
    fn assert_styled(h: &mut Highlighter, rope: &Rope, text: &str, token: &str, scope: &str) {
        let byte = text.find(token).unwrap();
        let at = rope.byte_to_char(byte);
        assert!(has_scope(h, rope, at, scope), "{token}");
        let line = rope.char_to_line(at);
        let runs = h.line_runs(rope, line);
        let column = byte - rope.line_to_byte(line);
        assert_eq!(foreground_at(&runs, column), scope_style(scope).0, "{token}");
    }

    fn wait_for(flag: &AtomicBool) {
        while !flag.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// What the tree colours for all of `rope`, a Rust file, at once.
    fn rust_tree(rope: &Rope) -> crate::treesitter::State {
        let (assets, language) = (assets(), crate::language::Language::Rust);
        let base = crate::treesitter::scope(language);
        let mut tree = crate::treesitter::State::new(language);
        let lines = 0..rope.len_lines();
        tree.prepare(rope, lines, None, || false, |s| style_for_scope(&assets, base, s));
        tree
    }

    #[track_caller]
    fn foreground_at(runs: &[Run], byte: usize) -> Hsla {
        let mut end = 0;
        runs.iter()
            .find(|r| {
                end += r.0;
                byte < end
            })
            .unwrap()
            .1
    }

    #[test]
    fn freebasic_detects_sources_headers_and_comment_commands() {
        for extension in ["bas", "BAS", "bi", "BI"] {
            let path = format!("example.{extension}");
            let h = file(&path);
            assert_eq!(h.syntax_name(), "FreeBASIC", "{extension}");
            assert_eq!(h.scope(), "source.freebasic");
            assert_eq!(h.comment_prefix(), Some("'"));
            assert_eq!(h.block_comment(), Some(("/'", "'/")));
            assert!(!h.pairs_single_quote());
            assert!(crate::language::Language::detect(Path::new(&path)).is_none());
        }
        assert_eq!(Highlighter::by_name("FreeBASIC").syntax_name(), "FreeBASIC");
        assert!(Highlighter::syntax_names().iter().any(|n| n == "FreeBASIC"));
    }

    #[test]
    fn freebasic_highlights_mixed_case_keywords_strings_and_numeric_literals() {
        let text = concat!(
            "#InClUdE Once \"crt/stdio.bi\"\n",
            "FuNcTiOn Twice(ByVal n As Integer) As Integer\n",
            "  DiM text As String = \"Hello 雪 \"\"quoted\"\"\"\n",
            "  Dim escaped As String = !\"line\\n\\\"tail\"\n",
            "  Dim raw As String = $\"C:\\temp\\folder\"\n",
            "  Dim count As ULongInt = &hFFull + &b1010 + &o77 + &77\n",
            "  Dim value As Double = .5D+2 + 42ULL + 3.1!\n",
            "  If True AndAlso n > 0 Then Return n * 2\n",
            "End Function\n",
            "' apostrophe comment\n",
            "rEm comment /' ignored block opener\n",
            "Print text\n",
            "Print \"unterminated\n",
            "Dim visible As Integer\n",
        );
        let rope = Rope::from_str(text);
        let mut h = Highlighter::by_name("FreeBASIC");
        colour_all(&mut h, &rope);
        for (token, scope) in [
            ("#InClUdE", "keyword.control.import.freebasic"),
            ("FuNcTiOn", "keyword.control.freebasic"),
            ("Twice", "entity.name.function.freebasic"),
            ("DiM", "storage.modifier.freebasic"),
            ("ByVal", "storage.modifier.freebasic"),
            ("Integer", "storage.type.freebasic"),
            ("ULongInt", "storage.type.freebasic"),
            ("True", "constant.language.freebasic"),
            ("AndAlso", "keyword.operator.freebasic"),
            ("Hello 雪", "string.quoted.double.freebasic"),
            ("tail", "string.quoted.double.freebasic"),
            ("\\n", "constant.character.escape.freebasic"),
            ("\"\"", "constant.character.escape.freebasic"),
            ("C:\\temp\\folder", "string.quoted.double.freebasic"),
            ("&hFFull", "constant.numeric.freebasic"),
            ("&b1010", "constant.numeric.freebasic"),
            ("&o77", "constant.numeric.freebasic"),
            ("&77", "constant.numeric.freebasic"),
            (".5D+2", "constant.numeric.freebasic"),
            ("42ULL", "constant.numeric.freebasic"),
            ("3.1!", "constant.numeric.freebasic"),
            ("apostrophe comment", "comment.line.apostrophe.freebasic"),
            ("ignored block opener", "comment.line.rem.freebasic"),
            ("Print text", "support.function.freebasic"),
            ("Dim visible", "storage.modifier.freebasic"),
        ] {
            assert_styled(&mut h, &rope, text, token, scope);
            if scope == "constant.numeric.freebasic" {
                let last = rope.byte_to_char(text.find(token).unwrap()) + token.chars().count() - 1;
                assert!(has_scope(&h, &rope, last, scope), "{token}: suffix");
            }
        }
    }

    #[test]
    fn freebasic_nested_comments_survive_incremental_edits() {
        let mut rope = Rope::from_str("/' outer\n/' nested '/\nstill outer\n'/ Dim value As Integer\n");
        let mut h = Highlighter::by_name("FreeBASIC");
        h.line_runs(&rope, 3);
        let at = rope.line_to_char(2);
        assert!(has_scope(&h, &rope, at, "comment.block.freebasic"));
        let code = rope.line_to_char(3) + 3;
        assert!(has_scope(&h, &rope, code, "storage.modifier.freebasic"));

        let edit = change(&mut rope, at, 0, "'/ ");
        h.invalidate_from(2);
        h.text_changed(&[edit]);
        let mut fresh = Highlighter::by_name("FreeBASIC");
        for line in 0..rope.len_lines() {
            assert_eq!(h.line_runs(&rope, line), fresh.line_runs(&rope, line));
        }
        assert_eq!(class(h.scope_id_at(&rope, at + 3)), Class::Code);
        assert_eq!(class(h.scope_id_at(&rope, code + 3)), Class::Comment);
    }

    #[test]
    fn freebasic_continuations_ignore_trailing_comment_openers() {
        for (prefix, separator) in [("Print ", " "), ("Print ", ""), ("Print 1", "")] {
            let text = format!("{prefix}_{separator}/' ignored opener\n    \"hello\"\nDim saved_value As Integer\nPrint \"_\", saved_value\n");
            let rope = Rope::from_str(&text);
            let mut h = Highlighter::by_name("FreeBASIC");
            h.line_runs(&rope, rope.len_lines() - 1);
            for (token, scope) in [
                ("ignored opener", "comment.line.continuation.freebasic"),
                ("hello", "string.quoted.double.freebasic"),
                ("Dim", "storage.modifier.freebasic"),
                ("saved_value", "variable.other.freebasic"),
                ("_\"", "string.quoted.double.freebasic"),
            ] {
                let at = rope.byte_to_char(text.find(token).unwrap());
                assert!(has_scope(&h, &rope, at, scope), "{token}, separator {separator:?}");
            }
        }
    }

    #[test]
    fn freebasic_escaped_macro_underscores_keep_block_comments() {
        for escaped in ["##_", "_##"] {
            let text =
                format!("#macro demo()\nPrint {escaped} /' actual comment\nstill a comment '/\n\"hello\"\n#endmacro\n");
            let rope = Rope::from_str(&text);
            let mut h = Highlighter::by_name("FreeBASIC");
            h.line_runs(&rope, rope.len_lines() - 1);
            for token in ["actual comment", "still a comment"] {
                let at = rope.byte_to_char(text.find(token).unwrap());
                assert!(has_scope(&h, &rope, at, "comment.block.freebasic"), "{token}, {escaped}");
            }
            let at = rope.byte_to_char(text.find("hello").unwrap());
            assert_eq!(class(h.scope_id_at(&rope, at)), Class::String);
        }
    }

    #[test]
    fn freebasic_colors_complete_binary_and_octal_string_escapes() {
        let text = "Print !\"\\&o101 \\&b01000001 \\&h41\"\n";
        let rope = Rope::from_str(text);
        let mut h = Highlighter::by_name("FreeBASIC");
        h.line_runs(&rope, 0);
        for token in ["\\&o101", "\\&b01000001", "\\&h41"] {
            let at = rope.byte_to_char(text.find(token).unwrap());
            for char in at..at + token.len() {
                let scope = "constant.character.escape.freebasic";
                assert!(has_scope(&h, &rope, char, scope), "{token}");
            }
        }
    }

    #[test]
    fn holyc_detects_sources_and_headers_without_claiming_cpp_headers() {
        for extension in ["HC", "hc", "HH"] {
            let path = format!("Test.{extension}");
            let h = file(&path);
            assert_eq!(h.syntax_name(), "HolyC", "{extension}");
            assert_eq!(h.scope(), "source.holyc");
            assert_eq!(h.comment_prefix(), Some("//"));
            assert_eq!(h.block_comment(), Some(("/*", "*/")));
            assert!(crate::language::Language::detect(Path::new(&path)).is_none());
        }
        assert_eq!(file("Test.hh").syntax_name(), "C++");
        assert_eq!(Highlighter::by_name("HolyC").syntax_name(), "HolyC");
        assert!(Highlighter::syntax_names().iter().any(|n| n == "HolyC"));
    }

    #[test]
    fn holyc_highlights_its_types_keywords_literals_and_multiline_comments() {
        let text = concat!(
            "#include \"KernelA.HH\"\n",
            "#exe { Print(\"compile\"); }\n",
            "public U0 Demo(I64 count=0x2A, U8 *name=lastclass)\n",
            "{\n",
            "  Bool enabled=TRUE; F64 value=/* adjacent comment */.5e+2;\n",
            "  if (count) no_warn value`2;\n",
            "  \"Hello 雪\\n\"; 'ABC'; // return U64\n",
            "  /* I32\n",
            "     still a comment */ return;\n",
            "}\n",
        );
        let rope = Rope::from_str(text);
        let mut h = Highlighter::by_name("HolyC");
        colour_all(&mut h, &rope);
        for (token, scope) in [
            ("#include", "keyword.control.import.holyc"),
            ("#exe", "keyword.control.import.holyc"),
            ("public", "storage.modifier.holyc"),
            ("U0", "storage.type.holyc"),
            ("I64", "storage.type.holyc"),
            ("U8", "storage.type.holyc"),
            ("Bool", "storage.type.holyc"),
            ("F64", "storage.type.holyc"),
            ("Demo", "entity.name.function.holyc"),
            ("lastclass", "keyword.operator.holyc"),
            ("TRUE", "constant.language.holyc"),
            ("0x2A", "constant.numeric.holyc"),
            (".5e+2", "constant.numeric.holyc"),
            ("if", "keyword.control.holyc"),
            ("no_warn", "keyword.control.holyc"),
            ("`", "keyword.operator.holyc"),
            ("adjacent comment", "comment.block.holyc"),
            ("Hello 雪", "string.quoted.double.holyc"),
            ("\\n", "constant.character.escape.holyc"),
            ("ABC", "string.quoted.single.holyc"),
            ("return U64", "comment.line.double-slash.holyc"),
            ("still a comment", "comment.block.holyc"),
        ] {
            assert_styled(&mut h, &rope, text, token, scope);
        }
        let at = rope.byte_to_char(text.rfind("return").unwrap());
        assert!(has_scope(&h, &rope, at, "keyword.control.holyc"));
    }

    #[test]
    fn holyc_incremental_comments_match_fresh_highlighting() {
        let mut rope = Rope::from_str("/* comment\nU64 value=42;\n");
        let mut h = Highlighter::by_name("HolyC");
        h.line_runs(&rope, 1);
        let edit = change(&mut rope, 2, 0, " */");
        h.invalidate_from(0);
        h.text_changed(&[edit]);
        let mut fresh = Highlighter::by_name("HolyC");
        for line in 0..rope.len_lines() {
            assert_eq!(h.line_runs(&rope, line), fresh.line_runs(&rope, line));
        }
        let at = rope.line_to_char(1);
        assert!(has_scope(&h, &rope, at, "storage.type.holyc"));
    }

    #[test]
    fn tree_sitter_detects_all_five_language_families_and_module_extensions() {
        for (extension, expected) in [
            ("rs", "Rust"),
            ("go", "Go"),
            ("py", "Python"),
            ("js", "JavaScript"),
            ("jsx", "JavaScript"),
            ("mjs", "JavaScript"),
            ("cjs", "JavaScript"),
            ("ts", "TypeScript"),
            ("mts", "TypeScript"),
            ("cts", "TypeScript"),
            ("tsx", "TypeScript (TSX)"),
        ] {
            let path = format!("code.{extension}");
            assert_eq!(file(&path).syntax_name(), expected, "{path}");
        }
    }

    #[test]
    fn popular_languages_use_bundled_trees_and_style_their_tokens() {
        for (extension, name, text, token, scope) in [
            ("c", "C", "int main(void) { return 12; }\n", "return", "keyword"),
            ("cpp", "C++", "class Café { int value = 12; };\n", "class", "keyword"),
            ("cs", "C#", "class Café { int value = 12; }\n", "class", "keyword"),
            ("java", "Java", "class Café { int value = 12; }\n", "class", "keyword"),
            ("php", "PHP", "<?php function café() { return 12; }\n", "function", "keyword"),
            ("rb", "Ruby", "def café\n  '雪'\nend\n", "def", "keyword"),
            ("swift", "Swift", "func café() -> Int { return 12 }\n", "func", "keyword"),
            ("kt", "Kotlin", "fun café(): Int { return 12 }\n", "fun", "keyword"),
            ("kt", "Kotlin", "val enabled = true\n", "true", "constant.language"),
            ("kt", "Kotlin", "fun run() { while (true) { break } }\n", "break", "keyword"),
            ("sh", "Bourne Again Shell (bash)", "if true; then echo '雪'; fi\n", "if", "keyword"),
            ("hcl", "HCL", "value = 12\n", "12", "constant.numeric"),
            ("tf", "HCL", "locals { value = 12 }\n", "12", "constant.numeric"),
            ("tfvars", "HCL", "value = 12\n", "12", "constant.numeric"),
        ] {
            let path = format!("main.{extension}");
            let rope = Rope::from_str(text);
            let h = file(&path);
            assert!(h.tree.is_some(), "{extension}: missing bundled tree");
            assert_eq!(h.syntax_name(), name);
            let h = run_job(h, &rope, &[0, rope.len_lines() - 1]);
            let tree = h.tree.as_ref().unwrap();
            assert!(!tree.requested, "{extension}: highlight query completed");
            let at = text.find(token).unwrap();
            let line = rope.byte_to_line(at);
            let runs = tree.runs(&rope, line).unwrap();
            assert_eq!(
                foreground_at(&runs, at - rope.line_to_byte(line)),
                scope_style(scope).0,
                "{extension}: {token}"
            );
            assert_eq!(Highlighter::by_name(name).syntax_name(), name);
            assert!(Highlighter::syntax_names().iter().any(|n| n == name));
        }
    }

    #[test]
    fn bundled_languages_keep_their_comment_commands() {
        for (extension, line, block) in [
            ("rs", "//", true),
            ("go", "//", true),
            ("js", "//", true),
            ("ts", "//", true),
            ("tsx", "//", true),
            ("py", "#", false),
            ("c", "//", true),
            ("cpp", "//", true),
            ("cs", "//", true),
            ("java", "//", true),
            ("php", "//", true),
            ("rb", "#", false),
            ("swift", "//", true),
            ("kt", "//", true),
            ("sh", "#", false),
            ("hcl", "#", true),
            ("tf", "#", true),
        ] {
            let h = file(&format!("main.{extension}"));
            assert_eq!(h.comment_prefix(), Some(line), "{extension}");
            assert_eq!(h.block_comment(), block.then_some(("/*", "*/")), "{extension}");
        }
    }

    #[test]
    fn php_templates_keep_embedded_html_styles_and_scopes() {
        let rope = Rope::from_str(
            "<h1 class=\"x\">Hi</h1>\n<?php echo 1; ?>\n<script>const value = '雪';</script>\n<p><?= 'hello' ?></p>\n",
        );
        let mut h = run_job(file("page.phtml"), &rope, &[0, 3]);
        let mut fallback = syntect_only(file("page.phtml"));
        for line in [0, 2, 3] {
            let expected = fallback.line_runs(&rope, line);
            assert_eq!(h.try_line_runs(&rope, line), Some(expected), "line {line}");
            assert_eq!(h.cached_scope_runs(&rope, line), fallback.cached_scope_runs(&rope, line));
            assert!(!h.provisional(), "embedded text does not queue another tree query");
        }
        let tree = h.tree.as_mut().unwrap();
        assert!(tree.runs(&rope, 1).is_some(), "PHP code still uses the tree");
        tree.request(&rope, 4);
        tree.request(&rope, 0);
        assert!(tree.requested, "a cached template line keeps another line's pending request");
        let stop = going();
        let mut copy = LineRuns::new(file("page.phtml"), rope.clone(), stop);
        assert_eq!(copy.line(0), fallback.line_runs(&rope, 0));
        assert!(!copy.ready.is_empty(), "a queried fallback line counts as prepared");
    }

    #[test]
    fn tree_sitter_worker_keeps_multiline_context_far_above_the_view() {
        let rope = Rope::from_str(&format!("/*\n{}*/\n", "fn hidden() {}\n".repeat(3_000)));
        let mut h = file("code.rs");
        let job = run_job(h.job(2_000), &rope, &[2_000, 2_040]);
        h.adopt(job, &rope);
        let runs = h.line_runs(&rope, 2_020);
        assert_eq!(runs.len(), 1, "a full-tree parse sees the opening comment");
        assert_eq!(runs[0].1, to_hsla(color(0xa6acb9)));
    }

    #[test]
    fn tree_sitter_queries_color_all_languages_and_jsx_typescript_syntax() {
        for (extension, text, word, expected) in [
            ("rs", "fn café() { let x = 12; }\n", "fn", "keyword"),
            ("go", "package main\nfunc café() { println(12) }\n", "package", "keyword"),
            ("py", "def café():\n    return '雪'\n", "def", "keyword"),
            ("mjs", "export function café() { return '雪'; }\n", "export", "keyword"),
            ("cjs", "const café = require('雪');\n", "const", "keyword"),
            ("jsx", "const App = () => <div title='雪'>hello</div>;\n", "div", "entity.name.tag"),
            ("mts", "interface Café { value: number }\n", "number", "support.type"),
            ("cts", "type Café = number;\n", "type", "keyword"),
            ("tsx", "const App = (p: string) => <div>{p}</div>;\n", "div", "entity.name.tag"),
        ] {
            let rope = Rope::from_str(text);
            let path = format!("code.{extension}");
            let h = file(&path);
            let mut h = run_job(h, &rope, &[0, rope.len_lines() - 1]);
            assert!(!h.tree.as_ref().unwrap().requested, "{extension} query completed");
            let start = text.find(word).unwrap();
            let line = rope.byte_to_line(start);
            let byte = start - rope.line_to_byte(line);
            let runs = h.try_line_runs(&rope, line).unwrap();
            assert_eq!(runs.iter().map(|r| r.0).sum::<usize>(), text.lines().nth(line).unwrap().len());
            assert_eq!(foreground_at(&runs, byte), scope_style(expected).0, "{extension}: {word}");
        }
    }

    #[test]
    fn tree_sitter_updates_multiline_unicode_edits_and_rejects_stale_jobs() {
        for (extension, original, insertion) in [
            ("rs", "fn café() {}\nfn next() {}\n", "/* 雪\n"),
            ("go", "package main\nfunc café() {}\n", "/* 雪\n"),
            ("py", "x = '雪'\ndef café(): pass\n", "'''雪\n"),
            ("js", "const café = '雪';\nconst x = 1;\n", "/* 雪\n"),
            ("ts", "const café: string = '雪';\nlet x = 1;\n", "/* 雪\n"),
            ("c", "int café(void) { return 12; }\n", "/* 雪\n"),
            ("cpp", "class Café { int value = 12; };\n", "/* 雪\n"),
            ("cs", "class Café { int value = 12; }\n", "/* 雪\n"),
            ("java", "class Café { int value = 12; }\n", "/* 雪\n"),
            ("php", "<?php function café() { return 12; }\n", "<?php /* 雪\n"),
            ("rb", "def café\n  '雪'\nend\n", "=begin 雪\n"),
            ("swift", "func café() -> Int { return 12 }\n", "/* 雪\n"),
            ("kt", "fun café(): Int { return 12 }\n", "/* 雪\n"),
            ("sh", "echo '雪'\necho next\n", "cat <<'END'\n"),
            ("hcl", "value = \"雪\"\ncount = 12\n", "/* 雪\n"),
            ("tsx", "const App = () => <div>雪</div>;\nlet x: number;\n", "/* 雪\n"),
        ] {
            let path = format!("code.{extension}");
            let mut rope = Rope::from_str(original);
            let mut h = run_job(file(&path), &rope, &[0, 1]);
            let c = change(&mut rope, 0, 0, insertion);
            h.invalidate_from(0);
            h.text_changed(&[c]);
            let old_job = run_job(h.job(0), &rope, &[0, rope.len_lines() - 1]);
            let c = change(&mut rope, 2, 0, "🦀\n");
            h.invalidate_from(0);
            h.text_changed(&[c]);
            h.adopt(old_job, &rope);
            assert!(h.tree.as_ref().unwrap().requested, "stale tree was rejected");
            let job = run_job(h.job(0), &rope, &[0, rope.len_lines() - 1]);
            h.adopt(job, &rope);
            let mut fresh = run_job(file(&path), &rope, &[0, rope.len_lines() - 1]);
            for line in 0..rope.len_lines() {
                assert_eq!(h.line_runs(&rope, line), fresh.line_runs(&rope, line), "{extension} line {line}");
            }
        }
    }

    #[test]
    fn custom_syntax_and_scheme_load_and_bad_files_leave_defaults_available() {
        let dir = std::env::temp_dir().join(format!("orlo-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |path: &str, text: &str| {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("Packages/Example/package.json", r#"{"name":"Example"}"#);
        let definition = "name: Example\nfile_extensions: [orloexample]\nscope: source.orloexample\ncontexts:\n  main:\n    - match: '\\bhello\\b'\n      scope: keyword.control.orloexample\n";
        write("Packages/Example/Example.sublime-syntax", definition);
        let staged = definition.replace("Example", "Staged");
        write("Packages/.install-stage/Staged.sublime-syntax", &staged);
        write("Syntaxes/Broken.sublime-syntax", "not: [yaml");
        write("Packages/Example/Broken.tmTheme", "not a property list");
        write(
            "Packages/Example/Example.tmTheme",
            r##"<?xml version="1.0"?><plist version="1.0"><dict><key>name</key><string>Example</string><key>settings</key><array><dict><key>settings</key><dict><key>background</key><string>#102030</string><key>foreground</key><string>#e0d0c0</string><key>caret</key><string>#ff0000</string><key>selection</key><string>#405060</string></dict></dict><dict><key>scope</key><string>keyword.control</string><key>settings</key><dict><key>foreground</key><string>#abcdef</string></dict></dict></array></dict></plist>"##,
        );
        let (assets, errors) = Assets::load(&dir, "Example.tmTheme");
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("Broken.sublime-syntax")));
        assert!(errors.iter().any(|e| e.contains("Broken.tmTheme")));
        assert!(assets.syntaxes.find_syntax_by_name("Rust").is_some());
        assert!(assets.syntaxes.find_syntax_by_name("Staged").is_none());
        let syntax = assets.syntaxes.find_syntax_by_extension("orloexample");
        let index = index_of(&assets.syntaxes, syntax.unwrap());
        let mut h = Highlighter::with_syntax(Arc::new(assets), index, None);
        assert_eq!(h.syntax_name(), "Example");
        let runs = h.line_runs(&Rope::from_str("hello world"), 0);
        assert_eq!(runs[0].1, to_hsla(color(0xabcdef)));
        assert_eq!(runs.last().unwrap().1, to_hsla(color(0xe0d0c0)));
        assert_eq!(h.assets.theme.as_ref().unwrap().settings.caret, Some(color(0xff0000)));
        let (fallback, errors) = Assets::load(&dir, "Missing.tmTheme");
        assert_eq!(fallback.theme().name.as_deref(), Some("Mariana"));
        assert!(errors.iter().any(|e| e.contains("Missing.tmTheme")));
        orlo_config::automation::set_enabled(&dir.join("Packages"), "Example", false).unwrap();
        let (disabled, errors) = Assets::load(&dir, "Mariana");
        assert_eq!(errors.len(), 1, "disabled package errors are ignored: {errors:?}");
        assert!(disabled.syntaxes.find_syntax_by_name("Example").is_none());
        assert!(!color_schemes(&dir).iter().any(|p| p.ends_with("Example.tmTheme")));
        orlo_config::automation::set_enabled(&dir.join("Packages"), "Example", true).unwrap();
        let broken = definition.replace("\\bhello\\b", "[");
        write("Packages/Example/Example.sublime-syntax", &broken);
        let (reloaded, errors) = Assets::load(&dir, "Mariana");
        assert!(reloaded.syntaxes.find_syntax_by_name("Example").is_none());
        assert!(reloaded.syntaxes.find_syntax_by_name("Rust").is_some());
        assert!(errors.iter().any(|e| e.contains("Example.sublime-syntax")));
        h.invalidate_from(0);
        assert_eq!(
            h.line_runs(&Rope::from_str("hello again"), 0)[0].1,
            to_hsla(color(0xabcdef)),
            "an outstanding old generation remains valid"
        );
        write(
            "Packages/Example/Example.sublime-syntax",
            "name: Example\nscope: source.orloexample\ncontexts:\n  main:\n    - include: scope:source.orlo.missing\n",
        );
        let (unlinked, errors) = Assets::load(&dir, "Mariana");
        assert!(unlinked.syntaxes.find_syntax_by_name("Rust").is_some());
        assert!(unlinked.syntaxes.find_syntax_by_name("HolyC").is_some());
        assert!(unlinked.syntaxes.find_syntax_by_name("FreeBASIC").is_some());
        assert!(unlinked.syntaxes.find_syntax_by_name("Example").is_none());
        assert!(errors.iter().any(|e| e.contains("unresolved context")));
        let scheme = "Packages/Example/Example.tmTheme";
        let source = std::fs::read_to_string(dir.join(scheme)).unwrap();
        write(scheme, &source.replace("<string>Example</string>", "<string>Mariana</string>"));
        let (default, _) = Assets::load(&dir, "Mariana");
        assert!(default.theme.is_none(), "an installed theme cannot replace the built-in default by its display name");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn installed_syntaxes_override_added_bundled_languages() {
        let dir = std::env::temp_dir().join(format!("orlo-language-overrides-{}", std::process::id()));
        let syntaxes = dir.join("Syntaxes");
        std::fs::create_dir_all(&syntaxes).unwrap();
        let languages = [
            ("C", "c"),
            ("C++", "cpp"),
            ("HolyC", "hc"),
            ("Custom Header", "HH"),
            ("FreeBASIC", "bas"),
            ("Custom BASIC Header", "bi"),
            ("C#", "cs"),
            ("Java", "java"),
            ("PHP", "php"),
            ("Ruby", "rb"),
            ("Swift", "swift"),
            ("Kotlin", "kt"),
            ("Bourne Again Shell (bash)", "sh"),
            ("HCL", "hcl"),
        ];
        for (name, ext) in languages {
            std::fs::write(syntaxes.join(format!("{ext}.sublime-syntax")), format!(
                "name: '{name}'\nfile_extensions: [{ext}]\nscope: source.custom.{ext}\ncontexts:\n  main:\n    - match: hello\n      scope: keyword.control.custom\n"
            )).unwrap();
        }
        let (assets, errors) = Assets::load(&dir, "Mariana");
        assert!(errors.is_empty(), "{errors:?}");
        let assets = Arc::new(assets);
        for (name, ext) in languages {
            let path = format!("Test.{ext}");
            let h = Highlighter::detect_with_assets(Some(Path::new(&path)), "", assets.clone());
            assert_eq!(h.syntax_name(), name);
            assert_eq!(h.scope(), format!("source.custom.{ext}"));
            assert!(h.tree.is_none(), "{name}: installed grammar takes precedence");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn jobs_from_an_old_asset_generation_cannot_replace_new_colors() {
        let plain = |assets: &Arc<Assets>| {
            let mut syntaxes = assets.syntaxes.syntaxes().iter();
            let index = syntaxes.position(|s| s.name == "Plain Text").unwrap();
            Highlighter::with_syntax(assets.clone(), index, None)
        };
        let generation = || {
            Arc::new(Assets {
                syntaxes: SyntaxSet::load_defaults_newlines(),
                bundled: 0,
                theme: None,
                highlighter: syn_highlighter(),
            })
        };
        let a = generation();
        let rope = Rope::from_str("text");
        let mut old = plain(&a);
        let mut new = plain(&generation());
        old.line_runs(&rope, 0);
        new.adopt(old, &rope);
        assert_eq!(new.cached_lines(), 0);
        assert_eq!(Arc::strong_count(&a), 1, "discarded jobs release their syntax set");
        let current = Arc::downgrade(&new.assets);
        let mut symbols = new.symbol_parse();
        drop(new);
        assert!(current.upgrade().is_some(), "a running symbol parse owns its generation");
        assert!(symbols.step(&rope, || false));
        drop(symbols);
        assert!(current.upgrade().is_none(), "finished work releases the old generation");
    }

    #[test]
    fn every_syntax_in_the_comment_tables_exists_and_shebangs_pick_a_syntax() {
        let ss = syntax_set();
        let names = LINE_COMMENTS.iter().flat_map(|c| c.0).chain(BLOCK_COMMENTS.iter().flat_map(|c| c.0));
        for n in names {
            assert!(
                ss.find_syntax_by_name(n).is_some() || crate::treesitter::named(n).is_some(),
                "no syntax named {n}"
            );
        }
        let syntax = |p: &str, first: &str| Highlighter::detect(Some(Path::new(p)), first).syntax_name().to_string();
        assert_eq!(syntax("run", "#!/usr/bin/env python3"), "Python");
        assert_eq!(syntax("deploy", "#!/bin/sh"), "Bourne Again Shell (bash)");
        assert_eq!(syntax("serve", "#!/usr/bin/env node"), "JavaScript");
        assert_eq!(syntax("x.rs", "#!/bin/sh"), "Rust", "the extension wins");
        assert_eq!(syntax("notes", "hello"), "Plain Text");
        // JSON (and the settings file) comment with //, as Sublime's do
        assert_eq!(file("a.json").comment_prefix(), Some("//"));
        assert_eq!(file("Preferences.sublime-settings").comment_prefix(), Some("//"));
        // a pipeline is JSON with comments: its template is enabled by toggling them
        let pipeline = file("tidy.orlo-pipeline");
        assert_eq!(pipeline.syntax_name(), "JSON");
        assert_eq!(pipeline.comment_prefix(), Some("//"));
        // New Snippet's file is XML
        let snippet = file("hello.sublime-snippet");
        assert_eq!(snippet.syntax_name(), "XML");
    }

    #[test]
    fn comment_markers_are_unchanged_for_every_syntax_with_the_bundled_languages() {
        let registry = crate::registry::registry();
        for name in Highlighter::syntax_names() {
            let h = Highlighter::by_name(&name);
            let syntax = h.syntax_name();
            // a language bundled as data alone had no table entry: its manifest supplies them
            if let Some(added) =
                registry.named(syntax).filter(|l| crate::language::Language::from_registry_id(l.id()).is_none())
            {
                let comments = &added.manifest.comments;
                assert_eq!(h.comment_prefix(), comments.line.as_deref(), "{name}");
                assert_eq!(h.block_comment(), comments.block.as_ref().map(|[a, b]| (a.as_str(), b.as_str())), "{name}");
                continue;
            }
            assert_eq!(h.comment_prefix(), table_comment_prefix(syntax), "{name}");
            assert_eq!(h.block_comment(), table_block_comment(syntax), "{name}");
            assert_eq!(h.pairs_single_quote(), !matches!(syntax, "Rust" | "FreeBASIC"), "{name}");
        }
    }

    #[test]
    fn a_registered_language_supplies_its_comment_markers_before_the_tables() {
        let support = std::env::temp_dir().join(format!("orlo-comment-languages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&support);
        struct Restore(std::path::PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
                crate::registry::reload(&self.0);
            }
        }
        let _restore = Restore(support.clone());
        for (id, name, comments) in [
            ("rust", "Rust", r##"{"line":"#","block":["{-","-}"]}"##),
            ("python", "Python", r#"{"line":"'"}"#),
            ("go", "Go", "{}"),
            ("javascript", "JavaScript", r#"{"block":["(*","*)"]}"#),
        ] {
            let folder = support.join("Languages").join(id);
            std::fs::create_dir_all(&folder).unwrap();
            let manifest = format!(
                r#"{{"schema":1,"id":"{id}","name":"{name}","scope":"source.{id}","syntax":"builtin:{name}",
                "tree_sitter":{{"grammar":"builtin:{id}","highlights":["builtin:{id}/highlights"]}},
                "comments":{comments}}}"#
            );
            std::fs::write(folder.join("language.json"), manifest).unwrap();
        }
        assert_eq!(crate::registry::reload(&support), Vec::<String>::new());
        let rust = Highlighter::by_name("Rust");
        assert_eq!((rust.comment_prefix(), rust.block_comment()), (Some("#"), Some(("{-", "-}"))));
        assert!(!rust.pairs_single_quote(), "lifetimes");
        let python = Highlighter::by_name("Python");
        assert_eq!((python.comment_prefix(), python.block_comment()), (Some("'"), None));
        assert!(!python.pairs_single_quote(), "an apostrophe that starts a comment types plainly");
        let go = Highlighter::by_name("Go");
        assert_eq!((go.comment_prefix(), go.block_comment()), (Some("//"), Some(("/*", "*/"))), "no markers: tables");
        let javascript = Highlighter::by_name("JavaScript");
        assert_eq!((javascript.comment_prefix(), javascript.block_comment()), (None, Some(("(*", "*)"))));
        assert!(javascript.pairs_single_quote());
        // the tables still serve syntaxes no language registers
        assert_eq!(Highlighter::by_name("Lua").block_comment(), Some(("--[[", "]]")));
    }

    #[test]
    fn detects_rust_and_colors_keyword() {
        let mut h = rs();
        assert_eq!(h.syntax_name(), "Rust");
        assert_eq!(h.comment_prefix(), Some("//"));
        let rope = Rope::from_str("fn main() {}\n");
        let runs = h.line_runs(&rope, 0);
        assert_eq!(runs.iter().map(|r| r.0).sum::<usize>(), 12);
        assert_ne!(runs[0].1, runs[1].1);
    }

    #[test]
    fn symbols_lists_definitions_and_syntax_can_be_switched() {
        let h = rs();
        let rope = Rope::from_str("fn main() {}\nstruct Foo;\nlet x = 1;\n");
        let got: Vec<_> = h.symbols(&rope).into_iter().map(|(l, n, k)| (l, n, symbol_badge(&k).0)).collect();
        assert_eq!(got, vec![(0, "main".to_string(), 'f'), (1, "Foo".to_string(), 's')]);
        assert_eq!(h.block_comment(), Some(("/*", "*/")));
        let h = Highlighter::by_name("Python");
        assert_eq!(h.syntax_name(), "Python");
        assert_eq!(h.block_comment(), None);
        assert!(Highlighter::syntax_names().iter().any(|n| n == "Rust"));
    }

    #[test]
    fn symbol_reuse_keeps_token_dense_lines_within_the_existing_payload_budget() {
        let ss = SyntaxSet::load_defaults_newlines();
        let syntax = ss.find_syntax_by_name("Rust").unwrap();
        let (mut parse, mut normal) = (ParseState::new(syntax), ParseState::new(syntax));
        let mut reuse = SymbolTransitions::default();
        let lines: Vec<_> = (0..1536).map(|i| format!("let value_{i} = [{}];\n", "a + b, ".repeat(40))).collect();
        for line in &lines {
            assert!(line.len() <= SYMBOL_TRANSITION_LINE);
            assert_eq!(reuse.line(&mut parse, line, &ss), normal.parse_line(line, &ss).unwrap());
            assert_eq!(parse, normal);
        }
        let before = reuse.reused;
        for line in &lines[1532..] {
            assert_eq!(reuse.line(&mut parse, line, &ss), normal.parse_line(line, &ss).unwrap());
            assert_eq!(parse, normal);
        }
        assert_eq!(reuse.reused - before, 4, "dense scope operations must not crowd later lines out of the cache");
        assert!(reuse.entries.len() <= SYMBOL_TRANSITION_SLOTS as usize);
        assert!(reuse.bytes <= SYMBOL_TRANSITION_BYTES);
        assert!(reuse.states.len() <= SYMBOL_PARSE_STATES);
        assert!(reuse.operations.len() <= SYMBOL_OPERATIONS);
        let payload: usize =
            reuse.entries.values().map(|c| c.text.len() + std::mem::size_of_val(c.ops.as_slice())).sum();
        assert_eq!(reuse.bytes, payload + std::mem::size_of_val(reuse.operations.as_slice()));
    }

    #[test]
    fn symbol_operation_encoding_preserves_variants_parameters_and_catalog_bounds() {
        use syntect::parsing::ClearAmount;
        let mut reuse = SymbolTransitions::default();
        let ops = vec![
            (0, ScopeStackOp::Push(Scope::new("entity.name.function").unwrap())),
            (1, ScopeStackOp::Pop(usize::MAX)),
            (2, ScopeStackOp::Clear(ClearAmount::TopN(usize::MAX))),
            (3, ScopeStackOp::Clear(ClearAmount::All)),
            (4, ScopeStackOp::Restore),
            (SYMBOL_TRANSITION_LINE, ScopeStackOp::Noop),
        ];
        let (encoded, added) = reuse.encode_ops(&ops).unwrap();
        assert!(reuse.operations.is_empty(), "encoding alone does not publish catalog additions");
        reuse.operations.extend(added);
        let decoded: Vec<_> =
            encoded.iter().map(|&(at, op)| (usize::from(at), reuse.operations[usize::from(op)].clone())).collect();
        assert_eq!(decoded, ops);
        assert!(reuse.encode_ops(&ops).unwrap().1.is_empty(), "identical operations share catalog values");
        let unchanged = reuse.operations.clone();
        assert!(reuse.encode_ops(&[(usize::from(u16::MAX) + 1, ScopeStackOp::Noop)]).is_none());
        assert_eq!(reuse.operations, unchanged);
        reuse.operations = (0..SYMBOL_OPERATIONS).map(ScopeStackOp::Pop).collect();
        let unchanged = reuse.operations.clone();
        assert!(reuse.encode_ops(&[(0, ScopeStackOp::Restore)]).is_none());
        assert_eq!(reuse.operations, unchanged, "catalog overflow declines atomically");
        assert!(
            reuse.encode_ops(&[(0, ScopeStackOp::Pop(0))]).unwrap().1.is_empty(),
            "full catalogs still encode known operations"
        );
    }

    #[test]
    fn symbol_reuse_preserves_exact_line_operations_and_state() {
        let ss = SyntaxSet::load_defaults_newlines();
        for (syntax, text) in [
        ("Rust", "\nfn repeated() {}\nfn repeated() {}\n/*\nfn repeated() {}\n*/\nfn repeated() {}\nlet s = r###\"\nfn repeated() {}\n\"#;\nfn repeated() {}\n\"###;\nfn repeated() {}\n"),
        ("Python", "def repeated(\n    value,\n):\n    \"\"\"Docs.\n    def hidden(): pass\n    \"\"\"\n    pass\n\ndef repeated(\n    value,\n):\n    \"\"\"Docs.\n    def hidden(): pass\n    \"\"\"\n    pass\n"),
        ("Bourne Again Shell (bash)", "cat <<'FIRST'\nbody\nSECOND\nFIRST\ncat <<'SECOND'\nbody\nFIRST\nSECOND\n"),
    ] {
        let syntax = ss.find_syntax_by_name(syntax).unwrap();
        let (mut cached, mut normal) = (ParseState::new(syntax), ParseState::new(syntax));
        let mut reuse = SymbolTransitions::default();
        for line in text.split_inclusive('\n') {
            assert_eq!(reuse.line(&mut cached, line, &ss), normal.parse_line(line, &ss).unwrap());
            assert_eq!(cached, normal);
        }
    }
    }

    #[test]
    fn symbol_reuse_keeps_colliding_lines_with_exact_parser_state_and_bounds() {
        let mut slots = HashMap::new();
        let (a, b) = (0..=SYMBOL_TRANSITION_SLOTS)
            .find_map(|i| {
                let line = format!("let value_{i} = {i};\n");
                let slot = hash_str([line.as_str()]) % SYMBOL_TRANSITION_SLOTS;
                slots.insert(slot, line.clone()).map(|previous| (previous, line))
            })
            .expect("more lines than slots must collide");
        let ss = SyntaxSet::load_defaults_newlines();
        let mut parse = ParseState::new(ss.find_syntax_by_name("Rust").unwrap());
        let mut normal = parse.clone();
        let mut reuse = SymbolTransitions::default();
        assert_eq!(reuse.line(&mut parse, "let warm = 0;\n", &ss), normal.parse_line("let warm = 0;\n", &ss).unwrap());
        for line in [&a, &b].into_iter().cycle().take(20) {
            assert_eq!(reuse.line(&mut parse, line, &ss), normal.parse_line(line, &ss).unwrap());
            assert_eq!(parse, normal);
        }
        assert_eq!(reuse.reused, 18, "both repeated lines should avoid parsing again");
        assert!(reuse.entries.len() <= SYMBOL_TRANSITION_SLOTS as usize);
        assert!(reuse.bytes <= SYMBOL_TRANSITION_BYTES);
        assert!(reuse.states.len() <= SYMBOL_PARSE_STATES);
    }

    #[test]
    fn symbol_reuse_shares_captured_states_and_stays_bounded() {
        let ss = SyntaxSet::load_defaults_newlines();
        let mut parse = ParseState::new(ss.find_syntax_by_name("Bourne Again Shell (bash)").unwrap());
        let header = format!("{}cat <<'DELIMITER'\n", " ".repeat(1_000));
        let mut reuse = SymbolTransitions::default();
        reuse.line(&mut parse, &header, &ss);
        let body = "x".repeat(1_015);
        for i in 0..6_000 {
            assert!(reuse.line(&mut parse, &format!("{i:05} {body}\n"), &ss).is_empty());
        }
        assert_eq!(reuse.states.len(), 2, "the initial state and one captured header, shared by all transitions");
        assert!(reuse.entries.len() <= SYMBOL_TRANSITION_SLOTS as usize);
        assert!(reuse.bytes <= SYMBOL_TRANSITION_BYTES);
        assert!(reuse.bytes >= SYMBOL_TRANSITION_BYTES - 1024, "exercise the payload budget");
        for i in 0..SYMBOL_PARSE_STATES + 10 {
            reuse.line(&mut parse, "DELIMITER\n", &ss);
            reuse.line(&mut parse, &format!("cat <<'D{i}'\n"), &ss);
            reuse.line(&mut parse, "body\n", &ss);
            reuse.line(&mut parse, &format!("D{i}\n"), &ss);
        }
        assert!(reuse.states.len() <= SYMBOL_PARSE_STATES);
    }

    #[test]
    fn symbol_reuse_releases_oversized_captured_states() {
        let definition = SyntaxDefinition::load_from_str(
            r"
name: Nested Captures
scope: source.nested
contexts:
  main:
    - match: '^<<([A-Z]+)'
      push: nested
  nested:
    - match: '^<<([A-Z]+)'
      push: nested
    - match: '^\1$'
      pop: true
",
            true,
            None,
        )
        .unwrap();
        let mut builder = syntect::parsing::SyntaxSetBuilder::new();
        builder.add(definition);
        let ss = builder.build();
        let syntax = &ss.syntaxes()[0];
        let (mut cached, mut normal) = (ParseState::new(syntax), ParseState::new(syntax));
        let mut reuse = SymbolTransitions::default();
        let header = format!("<<D {}\n", "x".repeat(990));
        for _ in 0..40 {
            assert_eq!(reuse.line(&mut cached, &header, &ss), normal.parse_line(&header, &ss).unwrap());
            assert_eq!(cached, normal);
        }
        assert!(reuse.states.is_empty(), "oversized nested captures release shared states");
        assert!(reuse.entries.is_empty());
        assert_eq!(reuse.bytes, 0);
        for _ in 0..40 {
            assert_eq!(reuse.line(&mut cached, "D\n", &ss), normal.parse_line("D\n", &ss).unwrap());
            assert_eq!(cached, normal);
        }
        assert!(reuse.states.is_empty(), "the cache stays disabled after the state shrinks");
    }

    #[test]
    fn symbol_reuse_preserves_docs_pauses_and_checkpoints_after_edits() {
        let h = Highlighter::by_name("Python");
        let block = "def repeated(\n    value,\n):\n    \"\"\"Docs.\n    def hidden(): pass\n    \"\"\"\n    pass\n\n";
        let mut rope = Rope::from_str(&block.repeat(400));
        let run = |rope: &Rope, last: Option<&Checkpoints>, reuse: bool| {
            let mut p = h.symbol_parse().resume(last, rope);
            loop {
                if !reuse {
                    p.reuse = SymbolTransitions::default();
                }
                let lines = std::cell::Cell::new(0);
                if p.step(rope, || {
                    let n = lines.get();
                    lines.set(n + 1);
                    n >= if reuse { 17 } else { 1 }
                }) {
                    return p;
                }
            }
        };
        let normal = run(&rope, None, false);
        let cached = run(&rope, None, true);
        assert_eq!(cached.found, normal.found);
        assert_eq!(cached.parse, normal.parse);
        assert_eq!(cached.stack, normal.stack);
        assert!(cached.docs == normal.docs);
        let before = normal.kept.as_ref().unwrap();
        let after = cached.kept.as_ref().unwrap();
        assert_eq!(after.len(), before.len());
        for (a, b) in after.iter().zip(before) {
            assert_eq!((a.line, a.found), (b.line, b.found));
            assert_eq!(a.parse, b.parse);
            assert_eq!(a.stack, b.stack);
            assert!(a.docs == b.docs);
        }
        assert!(cached.reuse.reused > 0);
        let mut symbols = Vec::new();
        let last = cached.finish(&mut symbols, rope.clone()).unwrap();
        assert_eq!(symbols.len(), 400);
        assert!(symbols.iter().all(|s| s.doc.as_deref() == Some("Docs.\ndef hidden(): pass")));
        rope.insert(rope.line_to_char(4), "    edited doc line\n");
        let mut warm = h.symbol_parse();
        assert!(warm.step(&Rope::from_str("\"\"\"open\n"), || false));
        assert!(warm.reuse.current.is_some());
        let mut changed = warm.resume(Some(&last), &rope);
        assert!(changed.reuse.current.is_none(), "the checkpoint replaced the parser");
        assert!(changed.step(&rope, || false));
        assert!(changed.stopped_early());
        changed.finish(&mut symbols, rope.clone()).unwrap();
        assert_eq!(symbols, run(&rope, None, false).found);
    }

    #[test]
    fn rust_types_single_quotes_unpaired_for_lifetimes() {
        assert!(!rs().pairs_single_quote());
        assert!(file("x.py").pairs_single_quote());
    }

    #[test]
    fn unknown_extension_is_plain_text_single_run() {
        let mut h = file("x.unknownext");
        assert_eq!(h.syntax_name(), "Plain Text");
        let rope = Rope::from_str("hello");
        assert_eq!(h.line_runs(&rope, 0).len(), 1);
    }

    #[test]
    fn a_huge_line_is_left_plain_and_the_next_still_colours() {
        let mut h = rs();
        let huge = "let x = 1; ".repeat(3_000);
        let rope = Rope::from_str(&format!("fn a() {{}}\n{huge}\nfn b() {{}}\n"));
        let plain = h.line_runs(&rope, 1);
        assert_eq!(plain.len(), 1, "one run: not parsed");
        assert_eq!(plain[0].0, huge.len());
        let next = h.line_runs(&rope, 2);
        assert_eq!(next, h.line_runs(&Rope::from_str("fn b() {}\n"), 0));
        assert!(next.len() > 1);
    }

    #[test]
    fn a_comment_opened_or_closed_on_a_huge_line_is_missed() {
        // deliberate: skipping the parse keeps typing in a minified line fast, at the cost of
        // the scope that line opens or closes (README, Known limits)
        let huge = "let x = 1; ".repeat(3_000);
        let colours = |text: &str, line| {
            let mut h = rs();
            h.line_runs(&Rope::from_str(text), line)
        };
        let code = colours("fn b() {}\n", 0);
        let comment = colours("/* fn b() {}\n", 0)[0].1;
        assert_eq!(
            colours(&format!("{huge}/* open\nfn b() {{}}\n"), 1),
            code,
            "opened there: the next line stays code"
        );
        let after = colours(&format!("/* start\n{huge} */\nfn b() {{}}\n"), 2);
        assert!(after.iter().all(|r| r.1 == comment), "closed there: the next line stays comment");
    }

    #[test]
    fn an_edit_reparses_only_until_the_state_matches_again() {
        let text: String = (0..300).map(|i| format!("fn f{i}() {{ let x = {i}; }}\n")).collect();
        let mut rope = Rope::from_str(&text);
        let mut h = rs();
        let all = |h: &mut Highlighter, r: &Rope| (0..r.len_lines()).map(|l| h.line_runs(r, l)).collect::<Vec<_>>();
        all(&mut h, &rope);
        let fresh = |r: &Rope| all(&mut rs(), r);
        // typing inside line 5, then a line inserted at line 10 and one removed at line 20
        for (at, insert, remove) in [(5, "y", 0), (10, "let z = 1;\n", 0), (20, "", 1)] {
            let start = rope.line_to_char(at) + 3;
            rope.remove(start..start + remove);
            rope.insert(start, insert);
            let parsed = h.parsed;
            h.invalidate_from(at);
            assert_eq!(all(&mut h, &rope), fresh(&rope), "same colours as a fresh parse");
            assert!(h.parsed - parsed <= 3, "{} lines parsed", h.parsed - parsed);
        }
        // a comment opened above recolours everything after it
        rope.insert(rope.line_to_char(30), "/*\n");
        h.invalidate_from(30);
        assert_eq!(all(&mut h, &rope), fresh(&rope));
        assert!(h.parsed > 250, "every line after the comment opens is parsed again");
    }

    #[test]
    fn the_cache_drops_its_oldest_half_past_its_limit() {
        let text = "let x = 1;\n".repeat(MAX_CACHE + 10);
        let rope = Rope::from_str(&text);
        let mut h = rs();
        let first = h.line_runs(&rope, 0);
        for l in 0..rope.len_lines() {
            h.line_runs(&rope, l);
        }
        assert!(h.cache.len() <= MAX_CACHE, "{}", h.cache.len());
        assert_eq!(h.line_runs(&rope, 0), first, "line 0 parses again");
    }

    #[test]
    fn a_job_seeded_from_the_cache_keeps_a_comment_open_far_down_and_one_above_restarts() {
        // a comment opened at the top must still colour line 3,000 once the job is adopted
        let text = format!("/*\n{}*/\nfn x() {{}}\n", "a b c\n".repeat(3_000));
        let rope = Rope::from_str(&text);
        let mut h = rs();
        h.line_runs(&rope, 10);
        h.begin_frame();
        assert!(h.try_line_runs(&rope, 50).is_some() && h.try_line_runs(&rope, 3_000).is_none());
        let mut job = h.job(3_000);
        job.line_runs(&rope, 3_000);
        h.adopt(job, &rope);
        assert!(h.try_line_runs(&rope, 3_000).is_some());
        // the reference parses every line from the top, never restarting
        let mut whole = rs();
        let all: Vec<_> = (0..rope.len_lines()).map(|l| whole.line_runs(&rope, l)).collect();
        let fresh = |l: usize| all[l].clone();
        for l in [5, 2_999, 3_000, 3_001, 3_002] {
            assert_eq!(h.line_runs(&rope, l), fresh(l), "line {l}");
        }
        // above the cache: a restart, adopted in place of it
        let mut up = h.job(1);
        up.line_runs(&rope, 1);
        h.adopt(up, &rope);
        assert!(h.try_line_runs(&rope, 1).is_some());
        assert_eq!(h.line_runs(&rope, 1), fresh(1));
    }

    #[test]
    fn a_frame_parses_its_budget_and_no_more() {
        let rope = Rope::from_str(&"let x = 1;\n".repeat(3_000));
        let mut h = rs();
        h.begin_frame();
        let n = (0..1_000).take_while(|&l| h.try_line_runs(&rope, l).is_some()).count();
        assert_eq!(n, MAX_GAP);
        // an edit far above the view: the unchanged lines below come back free
        for l in 0..2_000 {
            h.line_runs(&rope, l);
        }
        let mut rope = rope;
        rope.insert(rope.line_to_char(10) + 4, "y");
        h.invalidate_from(10);
        h.begin_frame();
        assert!(h.try_line_runs(&rope, 1_500).is_some(), "walked by reuse, not parsed");
    }

    #[test]
    fn a_job_over_a_long_span_restarts_once_and_is_adopted() {
        // a range wider than LOOKAHEAD restarts once and keeps its first line
        let rope = Rope::from_str(&"let x = 1;\n".repeat(60_000));
        let mut h = rs();
        h.line_runs(&rope, 10);
        let (from, to) = (40_000, 40_800);
        let job = h.job(from);
        h.adopt(run_job(job, &rope, &[from, to]), &rope);
        h.begin_frame();
        assert!(h.try_line_runs(&rope, from).is_some() && h.try_line_runs(&rope, to).is_some());
    }

    #[test]
    fn cache_resumes_after_invalidate() {
        let mut h = rs();
        let rope = Rope::from_str("/* a\nb */ fn\n");
        let before = h.line_runs(&rope, 1).clone();
        h.invalidate_from(0);
        assert_eq!(h.line_runs(&rope, 1), before);
    }

    #[test]
    fn a_job_that_lands_after_an_edit_keeps_its_lines_above_the_edit() {
        let rope = Rope::from_str(&"let x = 1;\n".repeat(5_000));
        // above its lines it is taken as is; below, they wait as stale and come back by reuse
        // where the text is unchanged
        for (edit_at, kept) in [(4_500, true), (100, true)] {
            let mut h = rs();
            h.line_runs(&rope, 10);
            let job = h.job(3_000);
            let job = run_job(job, &rope, &[3_000, 3_060]);
            // typed at `edit_at` while the job ran
            h.invalidate_from(edit_at);
            h.adopt(job, &rope);
            h.begin_frame();
            assert_eq!(h.try_line_runs(&rope, 3_030).is_some(), kept, "edit at {edit_at}");
        }
    }

    #[test]
    fn symbols_stop_when_told_and_skip_lines_too_long_to_colour() {
        let mut text = "fn a() {}\n".to_string();
        text.push_str(&format!("{}\n", "let x = 1; ".repeat(3_000)));
        text.push_str(&"fn b() {}\n".repeat(2_000));
        let rope = Rope::from_str(&text);
        let h = rs();
        let all = h.symbols(&rope);
        assert_eq!(all.first().map(|s| s.1.as_str()), Some("a"));
        assert_eq!(all.len(), 2_001, "the long line costs nothing and hides nothing");
        let some = h.symbols_until(&rope, || true);
        assert!(some.is_empty());
    }

    #[test]
    fn a_frame_leaves_long_lines_to_the_job() {
        let long = format!("let x = [{}];\n", "1, ".repeat(3_400));
        let rope = Rope::from_str(&format!("{long}{long}let y = 1;\n"));
        let mut h = rs();
        h.begin_frame();
        assert!(h.try_line_runs(&rope, 0).is_some(), "one 10 KB line fits a frame");
        assert!(h.try_line_runs(&rope, 1).is_none(), "a second does not: the job's");
        let huge = format!("let x = [{}];\n", "1, ".repeat(6_000));
        let mut h = rs();
        h.begin_frame();
        assert!(h.try_line_runs(&Rope::from_str(&huge), 0).is_none(), "18 KB: never in a frame");
    }

    #[test]
    fn an_edit_far_above_leaves_the_cache_drawing_until_a_job_redoes_it() {
        let text = "let x = 1;\n".repeat(5_000);
        let mut rope = Rope::from_str(&text);
        let mut h = rs();
        h.line_runs(&rope, 3_000);
        let before = h.line_runs(&rope, 3_000);
        // another view types a line break at the top
        let c = change(&mut rope, 0, 0, "\n");
        h.invalidate_from(0);
        h.text_changed(&[c]);
        h.begin_frame();
        assert!(h.provisional());
        assert_eq!(h.try_line_runs(&rope, 3_001), Some(before), "moved down with its text");
        for _ in 0..100 {
            let job = h.job(3_001);
            h.adopt(run_job(job, &rope, &[3_001, 3_060]), &rope);
            h.begin_frame();
            assert!(h.try_line_runs(&rope, 3_030).is_some());
            if !h.provisional() {
                break;
            }
        }
        assert!(!h.provisional());
        let mut exact = LineRuns::new(rs(), rope.clone(), going());
        assert_eq!(h.try_line_runs(&rope, 3_030), Some(exact.line(3_030)));
    }

    #[test]
    fn the_cache_stays_under_its_cap_however_far_a_job_goes() {
        // one trim a call left 2 × MAX_CACHE + 10 lines at MAX_CACHE + 10
        let rope = Rope::from_str(&"x\n".repeat(2 * MAX_CACHE + 12));
        // a job never restarts above a far line: it walks all of them
        let job = rs().job(0);
        let job = run_job(job, &rope, &[0, 2 * MAX_CACHE + 10]);
        assert!(job.cache.len() <= MAX_CACHE, "{}", job.cache.len());
    }

    #[test]
    fn a_cache_a_job_trimmed_moves_down_with_a_line_break_above_it() {
        let text: String = (0..MAX_CACHE + 100).map(|i| format!("let x{i} = 1;\n")).collect();
        let mut rope = Rope::from_str(&text);
        let mut h = rs();
        // the job walks past the cap and drops the cache's oldest half on its last line
        let job = run_job(h.job(0), &rope, &[0, MAX_CACHE + 5]);
        h.adopt(job, &rope);
        let line = MAX_CACHE;
        let before = h.cache[line - h.start].runs.clone();
        // another view types a line break far above the cache
        let at = rope.line_to_char(100);
        let c = change(&mut rope, at, 0, "\n");
        h.invalidate_from(100);
        h.text_changed(&[c]);
        h.begin_frame();
        assert_eq!(h.try_line_runs(&rope, line + 1), Some(before));
    }

    fn rs() -> Highlighter {
        file("x.rs")
    }

    #[test]
    fn a_second_keystroke_keeps_the_cache_below_the_walked_lines() {
        let mut rope = Rope::from_str(&"let x = 1;\n".repeat(2_000));
        let mut h = rs();
        (0..2_000).for_each(|l| drop(h.line_runs(&rope, l)));
        for _ in 0..2 {
            rope.insert(rope.line_to_char(10) + 4, "y");
            h.invalidate_from(10);
            h.begin_frame();
            (0..60).for_each(|l| drop(h.try_line_runs(&rope, l)));
        }
        h.begin_frame();
        assert!(h.try_line_runs(&rope, 1_500).is_some(), "walked to by reusing the lines kept");
    }

    /// A change as the editor logs it: `removed` chars at `start` replaced by `text`.
    fn change(rope: &mut Rope, start: usize, removed: usize, text: &str) -> Change {
        let gone = rope.slice(start..start + removed).chars().filter(|&c| c == '\n').count();
        rope.remove(start..start + removed);
        rope.insert(start, text);
        Change { start, removed, inserted: text.chars().count(), lines: (gone, text.matches('\n').count()) }
    }

    #[test]
    fn a_line_too_long_to_colour_costs_a_frame_nothing_and_hides_nothing_below() {
        let huge = format!("let x = \"{}\";\n", "a".repeat(30_000));
        let rope = Rope::from_str(&format!("{huge}{}", "let y = 1;\n".repeat(40)));
        let mut h = rs();
        h.begin_frame();
        let n = (0..41).filter(|&l| h.try_line_runs(&rope, l).is_some()).count();
        assert_eq!(n, 41);
        // in the colour of the scope it starts in, every byte of it
        assert_eq!(h.line_runs(&rope, 0).iter().map(|r| r.0).sum::<usize>(), huge.len() - 1);
    }

    #[test]
    fn typing_in_a_long_line_keeps_the_colours_below_until_the_job_lands() {
        // 17 KB: coloured, but too long for a frame
        let long = format!("let x = [{}];\n", "1, ".repeat(5_800));
        let mut rope = Rope::from_str(&format!("let a = 1;\n{long}{}", "let s = \"s\";\n".repeat(40)));
        let mut h = rs();
        let below: Vec<_> = (0..42).map(|l| h.line_runs(&rope, l)).collect();
        let at = rope.line_to_char(1) + 9;
        let c = change(&mut rope, at, 0, "9");
        h.invalidate_from(1);
        h.text_changed(&[c]);
        h.begin_frame();
        assert!(h.try_line_runs(&rope, 1).is_none(), "the long line is the job's");
        let drawn =
            |h: &mut Highlighter, rope: &Rope, l: usize| h.try_line_runs(rope, l).or_else(|| h.fallback_runs(rope, l));
        for (l, runs) in below.iter().enumerate().skip(2) {
            assert_eq!(drawn(&mut h, &rope, l).as_ref(), Some(runs), "line {l}");
        }
        // the job lands while typing goes on: what it found below still draws
        let job = run_job(h.job(1), &rope, &[1, 41]);
        let c = change(&mut rope, at, 0, "9");
        h.invalidate_from(1);
        h.text_changed(&[c]);
        h.adopt(job, &rope);
        h.begin_frame();
        for (l, runs) in below.iter().enumerate().skip(2) {
            assert_eq!(drawn(&mut h, &rope, l).as_ref(), Some(runs), "line {l}");
        }
    }

    #[test]
    fn a_frame_hashes_unchanged_lines_within_its_budget() {
        // 3 MB of 1 KB lines, plain text so the warm-up is quick
        let rope = Rope::from_str(&format!("{}\n", "x".repeat(1_023)).repeat(3_000));
        let mut h = Highlighter::new(None);
        for l in 0..3_000 {
            h.line_runs(&rope, l);
        }
        let mut rope = rope;
        let c = change(&mut rope, 0, 0, "y");
        h.invalidate_from(0);
        h.text_changed(&[c]);
        h.begin_frame();
        let before = HASHED.get();
        let got = h.try_line_runs(&rope, 2_999);
        let hashed = HASHED.get() - before;
        assert!(got.is_none() && h.walking(), "walks on next frame");
        assert!(hashed <= HASH_BYTES + 1_024, "{hashed} bytes hashed in one frame");
        // a few frames later it is there, unparsed
        let parsed = h.parsed;
        let frames = (0..10)
            .take_while(|_| {
                h.begin_frame();
                h.try_line_runs(&rope, 2_999).is_none()
            })
            .count();
        assert!(frames < 4 && h.parsed - parsed <= 1, "{frames} frames");
    }

    #[test]
    fn edits_above_the_cache_move_it_by_their_own_lines_alone() {
        // every line its own text, so a cache off by a line draws nothing
        let text: String = (0..5_000).map(|i| format!("let x{i} = 1;\n")).collect();
        let mut rope = Rope::from_str(&text);
        let mut h = rs();
        h.line_runs(&rope, 3_000);
        let before = h.line_runs(&rope, 3_000);
        // two edits above with a frame between, then two in one edit: one above and one
        // below the cache
        let mut line = 3_000;
        for (k, edits) in [vec![(0, "\n")], vec![(0, "\n")], vec![(0, "\n"), (4_900, "\n\n")]].into_iter().enumerate() {
            let mut changes = Vec::new();
            for (l, text) in edits {
                let at = rope.line_to_char(l + changes.len());
                changes.push(change(&mut rope, at, 0, text));
            }
            h.invalidate_from(0);
            h.text_changed(&changes);
            line += 1;
            h.begin_frame();
            assert_eq!(h.try_line_runs(&rope, line).as_ref(), Some(&before), "edit {k}");
        }
    }

    #[test]
    fn scrolling_up_past_the_restart_point_keeps_the_lines_below() {
        let rope = Rope::from_str(&"let x = 1;\n".repeat(5_000));
        let mut h = rs();
        h.line_runs(&rope, 3_000);
        // scrolled up just above where the parse restarted: a job above it
        let restart = h.start;
        let job = run_job(h.job(restart - 10), &rope, &[restart - 10, restart - 5]);
        h.adopt(job, &rope);
        let parsed = h.parsed;
        h.begin_frame();
        assert!(h.try_line_runs(&rope, 3_000).is_some(), "walked to, not parsed");
        assert!(h.parsed - parsed <= 6, "{} parsed", h.parsed - parsed);
        // the same through a synchronous restart
        let mut h = rs();
        h.line_runs(&rope, 3_000);
        let restart = h.start;
        h.line_runs(&rope, restart - 10);
        let parsed = h.parsed;
        h.line_runs(&rope, 3_000);
        assert!(h.parsed - parsed <= 12, "{} parsed", h.parsed - parsed);
    }

    #[test]
    fn a_job_for_a_closed_view_stops() {
        let rope = Rope::from_str(&"let x = 1;\n".repeat(5_000));
        let cancel = Arc::new(AtomicBool::new(true));
        let mut job = rs().job(4_000);
        job.cancel_on(cancel);
        let job = run_job(job, &rope, &[4_000, 4_060]);
        assert_eq!(job.parsed, 0);
    }

    #[test]
    fn urgent_work_runs_before_a_long_job_finishes() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let note = |o: &Arc<Mutex<Vec<&'static str>>>, what| o.lock().unwrap().push(what);
        // the job is running before the urgent work is queued, whatever else the worker has
        let (started, queued) = (going(), going());
        let (s, q, o) = (started.clone(), queued.clone(), order.clone());
        let long = on_worker(move || {
            s.store(true, Ordering::Relaxed);
            wait_for(&q);
            // a job's lines, each asking whether to give way (the minute only stops a hang)
            let t = Instant::now();
            while !urgent_waiting() && t.elapsed() < Duration::from_secs(60) {
                std::thread::sleep(Duration::from_millis(1));
            }
            note(&o, if urgent_waiting() { "long gave way" } else { "long ran out" });
        });
        wait_for(&started);
        let o = order.clone();
        let normal = on_worker(move || note(&o, "normal"));
        let o = order.clone();
        let urgent = on_worker_urgent(move || note(&o, "urgent"));
        queued.store(true, Ordering::Relaxed);
        futures_lite_block_on(long).unwrap();
        futures_lite_block_on(urgent).unwrap();
        futures_lite_block_on(normal).unwrap();
        assert_eq!(*order.lock().unwrap(), ["long gave way", "urgent", "normal"]);
    }

    #[test]
    fn urgent_work_runs_before_a_goto_symbol_parse_finishes() {
        use std::future::Future as _;
        let rope = Rope::from_str(crate::fixtures::EDITING_MODEL);
        let h = rs();
        let order = Arc::new(Mutex::new(Vec::new()));
        let (started, queued) = (going(), going());
        let (s, q, o) = (started.clone(), queued.clone(), order.clone());
        // asked at each line of the parse: notes that it ran, and holds it at its first line
        // until the urgent work is queued
        let stale = move || {
            if std::thread::current().name() == Some("orlo-highlight") {
                s.store(true, Ordering::Relaxed);
                wait_for(&q);
                let mut o = o.lock().unwrap();
                if o.last() != Some(&"parse") {
                    o.push("parse");
                }
            }
            false
        };
        let mut parse = std::pin::pin!(parse_on_worker(h.symbol_parse(), rope.clone(), stale));
        let mut cx = std::task::Context::from_waker(Waker::noop());
        // a pause for another test's urgent work wants the next stretch queued: polled for it
        while !started.load(Ordering::Relaxed) {
            assert!(parse.as_mut().poll(&mut cx).is_pending(), "parsed unseen");
            std::thread::sleep(Duration::from_millis(1));
        }
        let o = order.clone();
        let urgent = on_worker_urgent(move || o.lock().unwrap().push("urgent"));
        queued.store(true, Ordering::Relaxed);
        futures_lite_block_on(urgent).unwrap();
        let all = futures_lite_block_on(parse).unwrap().found;
        assert_eq!(*order.lock().unwrap(), ["parse", "urgent", "parse"]);
        // taken up again where it stopped
        let all: Vec<Symbol> = all.into_iter().map(|s| (s.line, s.name, s.kind)).collect();
        assert_eq!(all, h.symbols(&rope));
    }

    #[test]
    fn files_without_an_extension_are_named_by_their_name_or_first_line() {
        let named = |n: &str| file(n).syntax_name().to_string();
        assert_eq!(named(".bashrc"), "Bourne Again Shell (bash)");
        assert_eq!(named(".bash_profile"), "Bourne Again Shell (bash)");
        assert_eq!(named("Makefile"), "Makefile");
        assert_eq!(named("Gemfile"), "Ruby");
        assert_eq!(named("x.mk"), "Makefile");
        // the 2020 grammars syntect ships do not list .zshrc or Dockerfile
        assert_eq!(named(".zshrc"), "Plain Text");
        let first = |n: &str, line: &str| Highlighter::detect(Some(Path::new(n)), line).syntax_name().to_string();
        assert_eq!(first("run", "#!/usr/bin/env ruby"), "Ruby");
        assert_eq!(first(".zshrc", "#!/bin/zsh"), "Bourne Again Shell (bash)");
    }

    #[test]
    fn html_line_runs_are_what_a_highlighter_gives_line_after_line() {
        // a comment across lines in the syntect fallback
        let text = "/* a\n comment */\nint main() { return 0; }\n// done\n".repeat(40);
        let rope = Rope::from_str(&text);
        let c = || syntect_only(file("main.c"));
        let (mut want, mut got) = (c(), LineRuns::new(c(), rope.clone(), going()));
        for line in 0..rope.len_lines() {
            assert_eq!(got.line(line), want.line_runs(&rope, line), "line {line}");
        }
    }

    #[test]
    fn html_line_runs_take_tree_sitter_colours_a_stretch_at_a_time() {
        let text = "fn f() { 1 }\n".repeat(MAX_CACHE / 2 + 500);
        let rope = Rope::from_str(&text);
        let mut got = LineRuns::new(rs(), rope.clone(), going());
        let tree = rust_tree(&rope);
        // and what syntect would, for a line to tell them apart by
        let mut syntect = syntect_only(rs());
        // the last line of the first stretch, the first and last of the second
        for line in [0, MAX_CACHE / 2 - 1, MAX_CACHE / 2, MAX_CACHE / 2 + 499] {
            let want = tree.runs(&rope, line).expect("the tree coloured it");
            assert_ne!(want, syntect.line_runs(&rope, line), "line {line}");
            assert_eq!(got.line(line), want, "line {line}");
        }
    }

    #[test]
    fn html_line_runs_leave_a_line_too_long_to_colour_in_the_default_foreground() {
        let long = format!("let s = \"{}\";", "x".repeat(MAX_HIGHLIGHT));
        let rope = Rope::from_str(&format!("{long}\nfn b() {{}}\n"));
        let mut got = LineRuns::new(rs(), rope, going());
        assert!(got.line(0).is_empty(), "no runs: the default foreground");
        assert!(got.line(1).len() > 1, "the next line is coloured");
    }

    #[test]
    fn html_line_runs_walk_past_the_cache_cap_without_parsing_a_line_twice() {
        let rope = Rope::from_str(&"x\n".repeat(MAX_CACHE + MAX_CACHE / 2));
        let mut got = LineRuns::new(Highlighter::new(None), rope.clone(), going());
        for line in 0..rope.len_lines() {
            got.line(line);
        }
        assert_eq!(got.job.parsed(), rope.len_lines(), "each line, once");
        assert!(got.job.cached_lines() <= MAX_CACHE);
    }

    /// A flag for a `LineRuns` that nobody has told to stop.
    fn going() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn html_line_runs_keep_the_tree_colours_while_other_urgent_work_waits_behind_them() {
        let text = "fn f() { 1 }\n".repeat(MAX_CACHE / 2 + 500);
        let rope = Rope::from_str(&text);
        // what the tree colours, and what syntect would give instead
        let tree = rust_tree(&rope);
        let mut syntect = syntect_only(rs());
        // hold the thread, queue the copy, then another urgent job behind it
        struct Gate(Arc<AtomicBool>);
        impl Drop for Gate {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let (open, running) = (going(), going());
        let gate = Gate(open.clone());
        let (o, r) = (open, running.clone());
        drop(on_worker(move || {
            r.store(true, Ordering::Relaxed);
            wait_for(&o);
        }));
        wait_for(&running);
        let (job, copied) = (rs(), rope.clone());
        let copy = on_worker_urgent(move || {
            let before = urgent_waiting();
            let mut got = LineRuns::new(job, copied.clone(), going());
            let runs: Vec<_> = (0..copied.len_lines()).map(|l| got.line(l)).collect();
            (before, urgent_waiting(), runs)
        });
        let other = on_worker_urgent(|| ());
        drop(gate);
        let (before, after, runs) = futures_lite_block_on(copy).unwrap();
        futures_lite_block_on(other).unwrap();
        assert!(before && after, "the other job waited behind the copy all along");
        // the whole copy has the tree's colours, the second stretch too
        for (line, got) in runs.iter().enumerate() {
            assert_eq!(Some(got), tree.runs(&rope, line).as_ref(), "line {line}");
        }
        for line in [0, MAX_CACHE / 2 - 1, MAX_CACHE / 2, MAX_CACHE / 2 + 499] {
            assert_ne!(runs[line], syntect.line_runs(&rope, line), "line {line}");
        }
    }

    #[test]
    fn html_line_runs_fall_back_to_syntect_line_by_line_where_there_is_no_tree() {
        // over tree-sitter's size limit a Rust file is syntect's, as it is in the view
        let rope = Rope::from_str(&"fn f() { 1 }\n".repeat(16 * 1024 * 1024 / 13 + 1000));
        assert!(rope.len_bytes() > 16 * 1024 * 1024);
        let mut got = LineRuns::new(rs(), rope.clone(), going());
        let mut syntect = rs();
        for line in [0, 1, 2, 500] {
            assert_eq!(got.line(line), syntect.line_runs(&rope, line), "line {line}");
        }
        // nothing was coloured by a tree, so no stretch counts as done
        assert!(got.ready.is_empty());
    }

    #[test]
    fn html_line_runs_do_no_work_once_told_to_stop() {
        let stop = going();
        let rope = Rope::from_str(&"int a;\n".repeat(100));
        let mut got = LineRuns::new(syntect_only(file("a.c")), rope, stop.clone());
        got.line(0);
        got.line(1);
        let parsed = got.job.parsed();
        assert!(parsed >= 2, "{parsed}");
        stop.store(true, Ordering::Relaxed);
        assert!(got.line(2).is_empty() && got.line(3).is_empty());
        assert_eq!(got.job.parsed(), parsed, "nothing parsed after the stop");
        // a tree language: not even the colours it has are given, and none are looked for
        let rope = Rope::from_str(&"fn f() { 1 }\n".repeat(50));
        let stop = going();
        let mut got = LineRuns::new(rs(), rope.clone(), stop.clone());
        assert!(!got.line(0).is_empty() && !got.line(1).is_empty());
        stop.store(true, Ordering::Relaxed);
        assert!(got.line(2).is_empty());
        let mut got = LineRuns::new(rs(), rope.clone(), stop);
        assert!(got.line(0).is_empty());
        assert_eq!(got.job.parsed(), 0);
        let tree = got.job.tree.as_ref().unwrap();
        assert!(tree.runs(&rope, 0).is_none(), "the tree was not asked");
    }

    #[test]
    fn a_copys_tree_colours_stop_for_its_own_flag_alone() {
        let rope = Rope::from_str(&"fn f() { 1 }\n".repeat(50));
        let stop = AtomicBool::new(true);
        let mut h = rs();
        assert!(!h.prepare_tree(&rope, 0, 49, Some(&stop)), "told to stop");
        assert!(h.tree.as_ref().unwrap().runs(&rope, 0).is_none());
        stop.store(false, Ordering::Relaxed);
        assert!(h.prepare_tree(&rope, 0, 49, Some(&stop)));
        assert!(h.tree.as_ref().unwrap().runs(&rope, 49).is_some());
        // no tree, no colours
        let mut plain = Highlighter::new(None);
        assert!(!plain.prepare_tree(&rope, 0, 49, Some(&stop)));
    }

    /// A stack's scope names, outermost first.
    #[test]
    fn a_registered_wasm_language_is_detected_named_coloured_and_listed() {
        let support = crate::treesitter::testing::Support::new("highlight-json");
        support.copy("json-wasm", "json-wasm");
        assert_eq!(crate::registry::reload(&support.0), Vec::<String>::new());
        let text = "{\"a\": [12, true]}\n";
        let rope = Rope::from_str(text);
        let h = file("data.jsonw");
        assert_eq!((h.syntax_name(), h.scope().as_str()), ("JSON (wasm)", "source.json-wasm"));
        assert_eq!(h.language().unwrap().id(), "json-wasm");
        assert!(h.tree.is_some() && h.language_override().is_none());
        assert_eq!(names(&h.scope_at(&rope, 0)), ["source.json-wasm"], "named before it is coloured");
        let mut h = run_job(h, &rope, &[0]);
        let runs = h.try_line_runs(&rope, 0).unwrap();
        assert_eq!(foreground_at(&runs, text.find("12").unwrap()), scope_style("constant.numeric").0);
        assert_eq!(foreground_at(&runs, text.find("true").unwrap()), scope_style("constant.language").0);
        assert!(has_scope(&h, &rope, text.find("12").unwrap(), "constant.numeric"));
        assert!(Highlighter::syntax_names().iter().any(|n| n == "JSON (wasm)"));
        for (name, explicit) in [("JSON (wasm)", "JSON (wasm)"), ("json-wasm", "json-wasm")] {
            let chosen = Highlighter::by_name(name);
            assert!(chosen.tree.is_some(), "{name}");
            assert_eq!((chosen.syntax_name(), chosen.language_override()), ("JSON (wasm)", Some(explicit)));
        }
        h.reload();
        assert_eq!(h.syntax_name(), "JSON (wasm)");
        assert!(h.tree.is_some() && h.language_override().is_none(), "a detected language stays detected");
        // a language with a server alone is plain text, and keeps its identity through a reload
        let server = r#"{"schema":1,"id":"served","name":"Served","scope":"source.served","files":{"extensions":["srv"]},"lsp":{"language_id":"served"}}"#;
        support.write("Languages/served/language.json", server);
        assert_eq!(crate::registry::reload(&support.0), Vec::<String>::new());
        let mut served = file("x.srv");
        served.reload();
        assert_eq!((served.syntax_name(), served.language().map(|l| l.id())), ("Plain Text", Some("served")));
    }

    #[test]
    fn tree_sitter_wins_over_the_folders_regex_grammar_and_a_broken_one_drops_alone() {
        let support = crate::treesitter::testing::Support::new("highlight-folder-syntax");
        support.copy("json-wasm", "json-wasm");
        let numbers = "contexts:\n  main:\n    - match: '[0-9]+'\n      scope: keyword.control.n\n";
        support
            .write("Languages/json-wasm/JSONW.sublime-syntax", format!("name: JSONW\nscope: source.jsonw\n{numbers}"));
        support.manifest("json-wasm", |m| m["syntax"] = serde_json::json!("JSONW.sublime-syntax"));
        for id in ["good", "bad"] {
            let manifest = format!(
                r#"{{"schema":1,"id":"{id}","name":"Lang {id}","scope":"source.{id}","files":{{"extensions":["{id}"]}},"syntax":"{id}.sublime-syntax"}}"#
            );
            support.write(&format!("Languages/{id}/language.json"), manifest);
        }
        support.write(
            "Languages/good/good.sublime-syntax",
            format!("name: Good\nfile_extensions: [txt]\nscope: source.good\n{numbers}"),
        );
        let unlinked = "name: Bad\nscope: source.bad\ncontexts:\n  main:\n    - include: scope:source.missing\n";
        let bad = support.write("Languages/bad/bad.sublime-syntax", unlinked);
        support.write(
            "Syntaxes/Loose.sublime-syntax",
            format!("name: Loose\nfile_extensions: [loose]\nscope: source.loose\n{numbers}"),
        );
        assert_eq!(crate::registry::reload(&support.0), Vec::<String>::new());
        let (assets, errors) = Assets::load(&support.0, "Mariana");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with(&format!("{}: ", bad.display())) && errors[0].contains("source.missing"),
            "{errors:?}"
        );
        let assets = Arc::new(assets);
        let detect = |path: &str| Highlighter::detect_with_assets(Some(Path::new(path)), "", assets.clone());
        let (text, keyword) = ("[12]\n", scope_style("keyword.control").0);
        let rope = Rope::from_str(text);
        let mut h = run_job(detect("x.jsonw"), &rope, &[0]);
        assert_eq!((h.syntax().name.as_str(), h.syntax_name()), ("JSONW", "JSON (wasm)"));
        assert_eq!(foreground_at(&h.try_line_runs(&rope, 0).unwrap(), 1), scope_style("constant.numeric").0);
        let mut regex = syntect_only(detect("x.jsonw"));
        assert_eq!(foreground_at(&regex.line_runs(&rope, 0), 1), keyword, "the folder's syntax is the fallback");
        let mut good = detect("x.good");
        assert_eq!((good.syntax_name(), good.scope().as_str()), ("Lang good", "source.good"));
        assert_eq!(foreground_at(&good.line_runs(&rope, 0), 1), keyword);
        let bad = detect("x.bad");
        assert!(bad.language().is_none() && bad.syntax_name() == "Plain Text", "as if the folder were not there");
        let mut loose = detect("x.loose");
        assert_eq!(foreground_at(&loose.line_runs(&rope, 0), 1), keyword, "loose syntaxes still load");
        // the manifest detects a folder's files: its syntax claims none and is not in the menu
        assert_eq!(detect("x.txt").syntax_name(), "Plain Text");
        let folder_syntaxes =
            assets.syntaxes.syntaxes().iter().filter(|s| ["JSONW", "Good"].contains(&s.name.as_str()));
        assert!(folder_syntaxes.clone().count() == 2 && folder_syntaxes.clone().all(|s| s.hidden));
        assert!(folder_syntaxes.clone().all(|s| s.file_extensions.is_empty() && s.first_line_match.is_none()));
    }

    #[test]
    fn a_grammar_that_fails_leaves_plain_text_and_detection_skips_it_until_a_reload() {
        let support = crate::treesitter::testing::Support::new("highlight-broken");
        let folder = support.copy("json-wasm", "json-wasm");
        let grammar = std::fs::read(folder.join("grammar.wasm")).unwrap();
        support.write("Languages/json-wasm/grammar.wasm", &grammar[..100]);
        assert_eq!(crate::registry::reload(&support.0), Vec::<String>::new());
        let rope = Rope::from_str("[12]\n");
        let h = file("x.jsonw");
        assert!(h.tree.is_some(), "compiled by the first job, not on detection");
        let mut h = run_job(h, &rope, &[0]);
        assert!(!h.provisional());
        let mut plain = Highlighter::by_name("Plain Text");
        assert_eq!(h.try_line_runs(&rope, 0), Some(plain.line_runs(&rope, 0)));
        assert_eq!(crate::treesitter::reported(&folder.display().to_string()).len(), 1);
        assert!(h.clone().search_classes(&rope, &|| false).is_some(), "search falls back to the syntax");
        let again = file("x.jsonw");
        assert!(again.language().is_none() && again.tree.is_none());
        assert_eq!(again.syntax_name(), "Plain Text");
        assert!(Highlighter::by_name("JSON (wasm)").tree.is_none());
        support.write("Languages/json-wasm/grammar.wasm", &grammar);
        assert_eq!(crate::registry::reload(&support.0), Vec::<String>::new());
        let mut fixed = run_job(file("x.jsonw"), &rope, &[0]);
        assert_eq!(foreground_at(&fixed.try_line_runs(&rope, 0).unwrap(), 1), scope_style("constant.numeric").0);
        assert_eq!(crate::treesitter::reported(&folder.display().to_string()).len(), 1, "reported once");
    }

    #[test]
    fn a_loose_syntax_replaces_a_bundled_languages_colours_but_not_a_user_folders() {
        let support = crate::treesitter::testing::Support::new("highlight-loose");
        let folder = support.copy("json-wasm", "otest");
        support.write(
            "Syntaxes/Loose.sublime-syntax",
            "name: Loose\nfile_extensions: [jsonw]\nscope: source.loose\ncontexts:\n  main:\n    - match: x\n      scope: keyword.x\n",
        );
        let (assets, errors) = Assets::load(&support.0, "Mariana");
        assert!(errors.is_empty(), "{errors:?}");
        let assets = Arc::new(assets);
        for (origin, name, scope, tree) in
            [(Origin::Bundled, "Loose", "source.loose", false), (Origin::User, "JSON (wasm)", "source.json-wasm", true)]
        {
            let (registry, errors) = crate::registry::Registry::build([(origin, folder.clone())]);
            assert!(errors.is_empty(), "{errors:?}");
            let registry = Arc::new(registry);
            let path = Some(Path::new("x.jsonw"));
            let h = Highlighter::registered(assets.clone(), &registry, registry.get("otest").unwrap(), path).unwrap();
            assert_eq!((h.syntax_name(), h.scope().as_str(), h.tree.is_some()), (name, scope, tree), "{origin:?}");
            assert_eq!(h.language().unwrap().id(), "otest", "comments and LSP still come from the manifest");
        }
    }

    fn names(stack: &[Scope]) -> Vec<String> {
        stack.iter().map(|s| s.build_string()).collect()
    }

    #[test]
    fn scope_at_tells_a_rust_string_comment_and_code_apart() {
        let text = "let s = \"雪 in a string\"; // a comment\n\nfn f() {}\n";
        let rope = Rope::from_str(text);
        let at = |needle: &str| rope.byte_to_char(text.find(needle).unwrap());
        // syntect's scopes, as a frame parses them, and tree-sitter's once a job has run
        let mut syntect = rs();
        colour_all(&mut syntect, &rope);
        let tree = run_job(rs(), &rope, &[0, rope.len_lines() - 1]);
        let ts = tree.tree.as_ref().unwrap();
        for l in 0..rope.len_lines() {
            let len = text.lines().nth(l).map_or(0, str::len);
            let total = |runs: &[(usize, ScopeId)]| runs.iter().map(|r| r.0).sum::<usize>();
            assert_eq!(total(&syntect.cache[l].scopes), len, "line {l}");
            assert_eq!(total(ts.scopes(&rope, l).unwrap()), len, "line {l}");
        }
        for h in [&syntect, &tree] {
            let class_at = |needle| class(h.scope_id_at(&rope, at(needle)));
            assert_eq!(class_at("in a"), Class::String);
            assert_eq!(class_at("a comment"), Class::Comment);
            assert_eq!(class_at("let"), Class::Code);
            assert_eq!(class_at("fn"), Class::Code);
            for needle in ["in a", "a comment", "let", "fn"] {
                let stack = names(&h.scope_at(&rope, at(needle)));
                assert_eq!(stack[0], "source.rust", "{needle}");
                assert_eq!(h.scope_at(&rope, at(needle)), scope_stack(h.scope_id_at(&rope, at(needle))));
            }
            // a line's end is its last char's; an empty line is the syntax's
            assert_eq!(class(h.scope_id_at(&rope, at("\n\n"))), Class::Comment);
            assert_eq!(names(&h.scope_at(&rope, at("\n\n") + 1)), ["source.rust"]);
        }
        // tree-sitter's stack is the base scope and the capture's
        assert_eq!(names(&tree.scope_at(&rope, at("in a"))), ["source.rust", "string"]);
        assert_eq!(names(&tree.scope_at(&rope, at("a comment"))), ["source.rust", "comment"]);
        assert_eq!(names(&tree.scope_at(&rope, at(" = "))), ["source.rust"]);
        // syntect's is the grammar's own
        let string = names(&syntect.scope_at(&rope, at("in a")));
        assert!(string.contains(&"string.quoted.double.rust".to_string()), "{string:?}");
    }

    #[test]
    fn scope_at_a_line_without_colours_is_the_syntaxs_top_scope() {
        let rope = Rope::from_str("// a comment\r\nfn f() {}");
        // nothing parsed: no line has colours yet
        assert_eq!(names(&rs().scope_at(&rope, 5)), ["source.rust"]);
        assert_eq!(names(&Highlighter::new(None).scope_at(&rope, 5)), ["text.plain"]);
        let mut h = rs();
        h.line_runs(&rope, 0);
        // the CR of a CRLF ending, and past the end of the text
        assert_eq!(class(h.scope_id_at(&rope, 12)), Class::Comment);
        assert_eq!(names(&h.scope_at(&rope, 1_000)), ["source.rust"]);
        h.line_runs(&rope, 1);
        assert_eq!(class(h.scope_id_at(&rope, 1_000)), Class::Code);
    }

    #[test]
    fn scope_at_does_not_trust_a_cache_left_provisional_by_an_edit_above_it() {
        let mut rope = Rope::from_str(&"// c\n".repeat(5_000));
        let mut h = rs();
        h.line_runs(&rope, 3_000);
        let class_at = |h: &Highlighter, rope: &Rope, l: usize| class(h.scope_id_at(rope, rope.line_to_char(l) + 3));
        assert_eq!(class_at(&h, &rope, 3_000), Class::Comment);
        // another view types a line break at the top: the lines draw as they were, unparsed
        let c = change(&mut rope, 0, 0, "\n");
        h.invalidate_from(0);
        h.text_changed(&[c]);
        assert!(h.provisional);
        assert_eq!(class_at(&h, &rope, 3_001), Class::Code);
        let job = h.job(3_001);
        h.adopt(run_job(job, &rope, &[3_001, 3_060]), &rope);
        assert_eq!(class_at(&h, &rope, 3_001), Class::Comment);
    }

    #[test]
    fn a_line_too_long_to_colour_is_one_run_in_the_scope_it_starts_in() {
        let huge = "x ".repeat(MAX_HIGHLIGHT);
        let rope = Rope::from_str(&format!("{huge}\n/* open\n{huge}\n*/\n"));
        let mut h = rs();
        colour_all(&mut h, &rope);
        let line = |l: usize| rope.line_to_char(l);
        // the first line starts before anything is pushed: in the syntax's scope
        assert_eq!(names(&h.scope_at(&rope, line(0) + 7)), ["source.rust"]);
        // in the comment opened above it, every byte of it
        let id = h.scope_id_at(&rope, line(2) + 7);
        assert_eq!(class(id), Class::Comment);
        assert_eq!(*h.cache[2].scopes, [(huge.len(), id)]);
    }

    #[test]
    fn selector_answers_are_kept_per_stack_and_selector() {
        let string = scope_id(&["source.rust", "string.quoted.double.rust"]);
        let code = scope_id(&["source.rust"]);
        assert_eq!(string, scope_id(&["source.rust", "string.quoted.double.rust"]), "one id a stack");
        assert_ne!(string, code);
        assert_eq!(names(&scope_stack(string)), ["source.rust", "string.quoted.double.rust"]);
        // a selector no other test asks, so this thread's count is all its matching
        let sel = "source.rust string - comment.orlo-cache-test";
        let before = MATCHED.get();
        assert!(matches(string, sel));
        assert!(matches(string, sel));
        assert_eq!(MATCHED.get() - before, 1, "the second answer was kept");
        assert!(!matches(code, sel));
        assert_eq!(MATCHED.get() - before, 2, "another stack is matched once too");
        // Sublime's syntax: alternatives and exclusions; empty and invalid match nothing
        assert!(matches(string, "comment, string"));
        assert!(!matches(string, "string - source"));
        assert!(!matches(code, ""));
        assert!(!matches(string, "a.b.c.d.e.f.g.h.i"));
    }

    #[test]
    fn matches_reads_sublimes_selectors_with_groups_and_negations() {
        let comment = scope_id(&["source.rust", "comment.line.double-slash.rust"]);
        let keyword = scope_id(&["source.rust", "keyword.control.rust"]);
        let entity = scope_id(&["source.rust", "entity.name.function.rust"]);
        let skip = "-(constant, entity, keyword)";
        assert!(matches(comment, skip));
        assert!(!matches(keyword, skip));
        assert!(!matches(entity, skip));
        let and = "(keyword | storage) & source.rust";
        assert!(matches(keyword, and));
        assert!(!matches(comment, and));
        assert!(matches(comment, "(source.c | source.rust) - string"));
        assert!(matches(scope_id(&["source.c++"]), "(source.c | source.c++) - comment"));
        // a selector that does not parse matches nothing
        assert!(!matches(comment, "(comment"));
        assert!(!matches(comment, "comment -"));
        assert!(!matches(comment, "comment @"));
        assert!(!matches(comment, "   "));
    }

    #[test]
    fn typescript_stacks_start_with_its_own_scope_before_the_tree_colours_them_and_after() {
        // no syntect grammar: the JavaScript one parses, but its stacks say TypeScript's scope
        let text = "// note\nlet s = \"x\";\n";
        let rope = Rope::from_str(text);
        let at = |needle: &str| rope.byte_to_char(text.find(needle).unwrap());
        for (name, scope) in [("x.ts", "source.ts"), ("x.tsx", "source.tsx")] {
            let mut syntect = file(name);
            colour_all(&mut syntect, &rope);
            let tree = run_job(file(name), &rope, &[0, 2]);
            for (h, how) in [(&syntect, "syntect"), (&tree, "tree")] {
                for needle in ["note", "\"x\"", "let"] {
                    let id = h.cached_scope_id(&rope, at(needle)).expect("coloured");
                    assert_eq!(names(&scope_stack(id))[0], scope, "{name} {how} {needle}");
                    assert!(matches(id, scope) && !matches(id, "source.js"), "{name} {how}");
                }
                assert_eq!(class(h.scope_id_at(&rope, at("note"))), Class::Comment);
            }
            // syntect's own scopes inside it are kept
            let string = names(&syntect.scope_at(&rope, at("\"x\"") + 1));
            assert!(string.iter().any(|s| s.starts_with("string.")), "{string:?}");
        }
        // JavaScript itself is left as it is
        let mut js = file("x.js");
        js.line_runs(&rope, 0);
        assert_eq!(names(&js.scope_at(&rope, at("note")))[0], "source.js");
    }

    #[test]
    fn a_comment_anywhere_in_the_stack_wins_over_a_string() {
        let class_of = |names: &[&str]| class(scope_id(names));
        assert_eq!(class_of(&["source.rust", "comment.block", "string"]), Class::Comment);
        assert_eq!(class_of(&["source.js", "string.template", "comment.line"]), Class::Comment);
        assert_eq!(class_of(&["source.rust", "string.quoted"]), Class::String);
        assert_eq!(class_of(&["source.rust", "keyword.control"]), Class::Code);
        assert_eq!(class_of(&["source.rust", "strings.quoted"]), Class::Code);
    }

    const PONY_DEFINITIONS: [(usize, &str, &str); 21] = [
        (7, "Number", "class"),
        (9, "Main", "class"),
        (13, "create", "method"),
        (21, "tick", "method"),
        (36, "_report", "method"),
        (44, "Counter", "class"),
        (48, "create", "method"),
        (52, "increment", "method"),
        (55, "total", "method"),
        (57, "Maths", "class"),
        (58, "half", "method"),
        (60, "parse", "method"),
        (63, "Named", "interface"),
        (64, "name", "method"),
        (66, "Greeter", "interface"),
        (67, "greet", "method"),
        (69, "Point", "class"),
        (73, "apply", "method"),
        (75, "clone", "method"),
        (78, "is_origin", "method"),
        (80, "repeated", "method"),
    ];

    fn pony_sample() -> String {
        let registry = crate::registry::registry();
        String::from_utf8(registry.get("pony").unwrap().read("samples/sample.pony", 1 << 20).unwrap()).unwrap()
    }

    #[test]
    fn a_language_without_a_regex_grammar_lists_its_tags_scm_definitions() {
        let text = pony_sample();
        let rope = Rope::from_str(&text);
        let h = file("sample.pony");
        assert_eq!((h.syntax().name.as_str(), h.syntax_name()), ("Plain Text", "Pony"));
        let want: Vec<Symbol> =
            PONY_DEFINITIONS.iter().map(|&(line, name, kind)| (line, name.into(), kind.into())).collect();
        assert_eq!(h.symbols(&rope), want);
        let mut p = h.symbol_parse();
        assert!(p.step(&rope, || false));
        let main = &p.found[1];
        assert_eq!((main.col, main.signature.as_str(), main.doc.as_ref()), (6, "actor Main", None));
        let report = &p.found[4];
        assert_eq!((report.col, report.signature.as_str()), (10, "fun ref _report(total: Number) =>"));
        // an edit parses again from the start, in place of what was found
        let mut symbols = p.found.clone();
        let edited = Rope::from_str(&text.replace("actor Main", "actor Mine"));
        let mut again = h.symbol_parse().resume(None, &edited);
        assert!(again.step(&edited, || false));
        assert!(again.finish(&mut symbols, edited).is_none(), "no checkpoints");
        assert_eq!((symbols.len(), symbols[1].name.as_str()), (21, "Mine"));
        // a builtin language has no tags parse
        assert!(file("main.rs").symbol_parse().tags.is_none());
    }

    #[test]
    fn a_tags_parse_pauses_and_goes_on_across_stretches() {
        let copies = 30;
        let text = pony_sample().repeat(copies);
        let rope = Rope::from_str(&text);
        let h = file("big.pony");
        let all = h.symbols(&rope);
        assert_eq!(all.len(), 21 * copies);
        assert_eq!(all[21 * (copies - 1) + 20], (84 * (copies - 1) + 80, "repeated".into(), "method".into()));
        let mut p = h.symbol_parse();
        let (calls, mut steps) = (std::cell::Cell::new(0), 0);
        // a pause every few checks, in the parse and in the query
        while !p.step(&rope, || {
            calls.set(calls.get() + 1);
            calls.get() % 25 == 0
        }) {
            steps += 1;
            assert!(steps < 10_000);
        }
        assert!(steps > 2, "{steps}");
        let found: Vec<Symbol> = p.found.into_iter().map(|s| (s.line, s.name, s.kind)).collect();
        assert_eq!(found, all);
        assert!(h.symbols_until(&rope, || true).is_empty(), "stopped before it began");
    }

    #[test]
    fn a_regex_grammar_keeps_its_symbols_over_tags_scm() {
        let support = crate::treesitter::testing::Support::new("highlight-tags");
        support.copy("json-wasm", "json-wasm");
        support.write(
            "Languages/json-wasm/queries/tags.scm",
            "(pair key: (string (string_content) @name)) @definition.field\n",
        );
        let keys =
            "contexts:\n  main:\n    - match: '\"(\\w+)\"'\n      captures:\n        1: entity.name.function.k\n";
        support.write("Languages/json-wasm/JSONW.sublime-syntax", format!("name: JSONW\nscope: source.jsonw\n{keys}"));
        support.manifest("json-wasm", |m| m["tree_sitter"]["tags"] = serde_json::json!("queries/tags.scm"));
        let rope = Rope::from_str("{\"a\": 1,\n \"bc\": [2]}\n");
        let symbols = |support: &crate::treesitter::testing::Support| {
            assert_eq!(crate::registry::reload(&support.0), Vec::<String>::new());
            let (assets, errors) = Assets::load(&support.0, "Mariana");
            assert!(errors.is_empty(), "{errors:?}");
            Highlighter::detect_with_assets(Some(Path::new("x.jsonw")), "", Arc::new(assets)).symbols(&rope)
        };
        let field = |line, name: &str| (line, name.to_string(), "field".to_string());
        assert_eq!(symbols(&support), [field(0, "a"), field(1, "bc")], "tags alone");
        support.manifest("json-wasm", |m| m["syntax"] = serde_json::json!("JSONW.sublime-syntax"));
        let function = |line, name: &str| (line, name.to_string(), "function".to_string());
        assert_eq!(symbols(&support), [function(0, "a"), function(1, "bc")], "the regex grammar's");
    }
}

/// Exact grammar owner/revision, stable across cloned background jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchSource(u64, Option<(u64, u64)>);

/// Disjoint character runs, including the EOF position, from one exact source snapshot.
pub struct SearchClasses {
    ends: Vec<(usize, Class)>,
}

impl SearchClasses {
    pub fn at(&self, start: usize) -> Option<Class> {
        self.ends.get(self.ends.partition_point(|(end, _)| *end <= start)).map(|(_, class)| *class)
    }
}

impl Highlighter {
    /// No approximate/plain-text classification may authorize a syntax-filtered operation.
    pub fn search_source(&self, rope: &Rope) -> Option<SearchSource> {
        (rope.len_bytes() <= 16 * 1024 * 1024 && self.syntax_name() != "Plain Text")
            .then(|| SearchSource(self.search_identity, self.tree.as_ref().and_then(|t| t.source_key(rope))))
    }

    /// Background-only exact scopes. Parser colors win; embedded/fallback lines use the same
    /// grammar, walked from the beginning so a distant opening delimiter cannot be missed.
    pub fn search_classes(self, rope: &Rope, stop: &dyn Fn() -> bool) -> Option<SearchClasses> {
        self.search_source(rope)?;
        let mut job = Self::with_syntax(self.assets.clone(), self.syntax, None);
        job.tree = self.tree.as_ref().map(|t| crate::treesitter::State::with_grammar(t.grammar.clone()));
        let mut parse = ParseState::new(self.syntax());
        let mut stack = ScopeStack::new();
        let mut lexical_line = 0;
        let mut ends: Vec<(usize, Class)> = Vec::new();
        fn push(ends: &mut Vec<(usize, Class)>, end: usize, class: Class) -> Option<()> {
            if let Some(last) = ends.last_mut().filter(|last| last.1 == class) {
                last.0 = end;
            } else {
                if ends.len() >= 1_000_000 {
                    return None;
                }
                ends.push((end, class));
            }
            Some(())
        }
        let mut chars = 0;
        for line in 0..rope.len_lines() {
            if stop() || rope.line(line).len_bytes() > MAX_HIGHLIGHT {
                return None;
            }
            if line % (MAX_CACHE / 2) == 0 {
                if let Some(tree) = job.tree.as_mut() {
                    let assets = &job.assets;
                    let grammar = tree.grammar.clone();
                    tree.prepare(rope, line..(line + MAX_CACHE / 2).min(rope.len_lines()), None, stop, |scope| {
                        style_for_scope(assets, grammar.scope(), scope)
                    });
                    if grammar.failed() {
                        // the syntect walk below classifies every line instead
                        job.tree = None;
                    } else if !tree.queried(rope, line) {
                        return None;
                    }
                }
            }
            let fallback_scopes;
            let scopes = match job.tree.as_ref().and_then(|tree| tree.scopes(rope, line)) {
                Some(scopes) => scopes,
                None => {
                    // The normal paint cache can restart/approximate; eligibility cannot.
                    let mut current = Arc::from([]);
                    for previous in lexical_line..=line {
                        let source = rope.line(previous);
                        if stop() || source.len_bytes() > MAX_HIGHLIGHT {
                            return None;
                        }
                        let mut text = source.to_string();
                        let len =
                            text.strip_suffix('\n').map_or(text.len(), |s| s.strip_suffix('\r').unwrap_or(s).len());
                        if !text.ends_with('\n') {
                            text.push('\n');
                        }
                        let ops = parse.parse_line(&text, &self.assets.syntaxes).ok()?;
                        if previous == line {
                            current = scope_runs(stack.clone(), &ops, len, self.outer_scope());
                        }
                        for (_, op) in &ops {
                            stack.apply(op).ok()?;
                        }
                    }
                    lexical_line = line + 1;
                    fallback_scopes = current;
                    &fallback_scopes
                }
            };
            let text = rope.line(line);
            let mut byte = 0;
            for &(len, scope) in scopes {
                let end = byte + len;
                chars += text.byte_slice(byte..end).len_chars();
                push(&mut ends, chars, class(scope))?;
                byte = end;
            }
            // Newline/EOF follow cached_scope_id's last-character/empty-line convention.
            let last = scopes.last().map_or(Class::Code, |(_, scope)| class(*scope));
            chars += text.len_chars() - text.byte_slice(..byte).len_chars();
            push(&mut ends, chars, last)?;
            if line + 1 == rope.len_lines() {
                push(&mut ends, chars + 1, last)?;
            }
        }
        if stop() || ends.len() > 1_000_000 {
            return None;
        }
        Some(SearchClasses { ends })
    }
}

#[cfg(test)]
mod syntax_search_tests {
    use super::*;
    fn classes(name: &str, text: &str) -> (Rope, SearchClasses) {
        let rope = Rope::from_str(text);
        let found = Highlighter::by_name(name).search_classes(&rope, &|| false).expect("exact grammar scopes");
        (rope, found)
    }

    #[test]
    fn n11_rust_code_normal_raw_multiline_comments_and_unicode_use_current_scopes() {
        let text = "let code = \"雪 normal\"; let raw = r###\"raw\nstring\"###; /* block\ncomment */ // line\n";
        let (rope, found) = classes("Rust", text);
        for (needle, expected) in [
            ("code", Class::Code),
            ("雪", Class::String),
            ("raw\n", Class::String),
            ("string", Class::String),
            ("block", Class::Comment),
            ("comment", Class::Comment),
            ("line", Class::Comment),
        ] {
            assert_eq!(found.at(rope.byte_to_char(text.find(needle).unwrap())), Some(expected), "{needle}");
        }
        assert_eq!(found.at(rope.len_chars()), Some(Class::Code), "empty final line");
    }

    #[test]
    fn n11_template_interpolation_expression_uses_winning_parser_capture() {
        let text = "let code = `literal ${expression} tail`; // note";
        let (rope, found) = classes("JavaScript", text);
        for (needle, expected) in [
            ("code", Class::Code),
            ("literal", Class::String),
            ("expression", Class::Code),
            ("tail", Class::String),
            ("note", Class::Comment),
        ] {
            assert_eq!(found.at(rope.byte_to_char(text.find(needle).unwrap())), Some(expected), "{needle}");
        }
    }

    #[test]
    fn n11_fallback_grammar_walks_from_start_and_classifies_newlines_like_cached_scopes() {
        let text = format!("/* open\n{}still comment\n*/\na {{ content: \"string\"; }}", "middle\n".repeat(1200));
        let (rope, found) = classes("CSS", &text);
        for (needle, expected) in [("still", Class::Comment), ("a {", Class::Code), ("string", Class::String)] {
            assert_eq!(found.at(rope.byte_to_char(text.find(needle).unwrap())), Some(expected));
        }
        assert_eq!(found.at(7), Some(Class::Comment), "newline inherits preceding character");
    }

    #[test]
    fn n11_unsupported_oversized_long_line_cancel_and_new_source_never_reuse_guesses() {
        let rope = Rope::from_str("// comment");
        assert!(Highlighter::new(None).search_source(&rope).is_none());
        assert!(Highlighter::new(None).search_classes(&rope, &|| false).is_none());
        let rust = Highlighter::by_name("Rust");
        assert_eq!(rust.search_source(&rope), rust.clone().search_source(&rope));
        assert_ne!(rust.search_source(&rope), Highlighter::by_name("Rust").search_source(&rope));
        assert!(rust.clone().search_classes(&rope, &|| true).is_none());
        assert!(rust.clone().search_classes(&Rope::from_str(&"x".repeat(MAX_HIGHLIGHT + 1)), &|| false).is_none());
        assert!(rust.clone().search_source(&Rope::from_str(&"x".repeat(16 * 1024 * 1024 + 1))).is_none());
        assert_eq!(rust.clone().search_classes(&rope, &|| false).unwrap().at(3), Some(Class::Comment));
        assert_eq!(rust.search_classes(&Rope::from_str("let value = 1;"), &|| false).unwrap().at(3), Some(Class::Code));
    }
}
