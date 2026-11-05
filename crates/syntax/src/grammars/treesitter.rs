//! Incremental parsers, bundled or a language folder's WebAssembly, queried only for the lines a
//! background job needs.

use crate::highlight::{Run, ScopeId};
use crate::index::IndexedSymbol;
use crate::language::Language;
use crate::manifest::Source;
use crate::registry::{LanguageDef, Registry};
use orlo_editor::text::{ByteOffset, Change, CharOffset};
use ropey::Rope;
use std::collections::BTreeMap;
use std::ops::{ControlFlow, Range};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tree_sitter::{
    InputEdit, ParseOptions, Parser, Point, Query, QueryCursor, QueryCursorOptions, StreamingIterator, Tree,
};

// larger documents keep syntect's bounded caches; raise after measuring tree memory.
const MAX_PARSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_COLOR_LINES: usize = 20_000;
/// Whether built-in queries' capture names take the prefix fallback too (`capture_scope`): a
/// test shows every one maps to the scope it had without it.
const BUILTIN_PREFIXES: bool = false;

#[path = "runtime.rs"]
mod runtime;
#[cfg(test)]
pub(crate) use runtime::{reported, testing};

/// Grammars compiled into Orlo, as a language manifest names them (`builtin:rust`).
pub const BUILTIN_GRAMMARS: [&str; 16] = [
    "rust",
    "go",
    "python",
    "javascript",
    "typescript",
    "tsx",
    "c",
    "cpp",
    "csharp",
    "java",
    "php",
    "ruby",
    "swift",
    "kotlin",
    "bash",
    "hcl",
];

/// Highlight queries compiled into Orlo, as a manifest names them (`builtin:javascript/jsx`).
pub const BUILTIN_QUERIES: [&str; 16] = [
    "rust/highlights",
    "go/highlights",
    "python/highlights",
    "javascript/highlights",
    "javascript/jsx",
    "typescript/highlights",
    "c/highlights",
    "cpp/highlights",
    "csharp/highlights",
    "java/highlights",
    "php/highlights",
    "ruby/highlights",
    "swift/highlights",
    "kotlin/highlights",
    "bash/highlights",
    "hcl/highlights",
];

pub fn named(syntax: &str) -> Option<Language> {
    Language::ALL.iter().copied().find(|&language| name(language) == syntax)
}

/// TypeScript shares the bundled JavaScript fallback; other missing syntaxes stay plain.
pub fn fallback_name(language: Language) -> &'static str {
    match language {
        Language::TypeScript | Language::Tsx => "JavaScript",
        _ => name(language),
    }
}

/// A parser compiled into Orlo, by the name a manifest gives it (`builtin:rust`).
fn builtin_language(name: &str) -> Option<tree_sitter::Language> {
    Some(match name {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        "bash" => tree_sitter_bash::LANGUAGE.into(),
        "hcl" => tree_sitter_hcl::LANGUAGE.into(),
        _ => return None,
    })
}

/// A highlight query compiled into Orlo, by the name a manifest gives it (`builtin:rust/highlights`).
fn builtin_query(name: &str) -> Option<&'static str> {
    Some(match name {
        "rust/highlights" => tree_sitter_rust::HIGHLIGHTS_QUERY,
        "go/highlights" => tree_sitter_go::HIGHLIGHTS_QUERY,
        "python/highlights" => tree_sitter_python::HIGHLIGHTS_QUERY,
        "javascript/highlights" => tree_sitter_javascript::HIGHLIGHT_QUERY,
        "javascript/jsx" => tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        "typescript/highlights" => tree_sitter_typescript::HIGHLIGHTS_QUERY,
        "c/highlights" => tree_sitter_c::HIGHLIGHT_QUERY,
        "cpp/highlights" => tree_sitter_cpp::HIGHLIGHT_QUERY,
        "csharp/highlights" => tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
        "java/highlights" => tree_sitter_java::HIGHLIGHTS_QUERY,
        "php/highlights" => tree_sitter_php::HIGHLIGHTS_QUERY,
        "ruby/highlights" => tree_sitter_ruby::HIGHLIGHTS_QUERY,
        "swift/highlights" => tree_sitter_swift::HIGHLIGHTS_QUERY,
        "kotlin/highlights" => include_str!("queries/kotlin.scm"),
        "bash/highlights" => tree_sitter_bash::HIGHLIGHT_QUERY,
        "hcl/highlights" => include_str!("queries/hcl.scm"),
        _ => return None,
    })
}

/// A parser and the queries it runs, compiled once: per process for a built-in language, per
/// registry generation for a language folder's.
pub struct Compiled {
    language: tree_sitter::Language,
    query: Query,
    /// Each highlight capture's scope.
    scopes: Vec<&'static str>,
    folds: Option<Query>,
    indents: Option<Query>,
    tags: Option<Query>,
}

impl Compiled {
    fn new(language: tree_sitter::Language, query: Query, prefixes: bool) -> Self {
        let scopes = query.capture_names().iter().map(|c| capture_scope(c, prefixes)).collect();
        Self { language, query, scopes, folds: None, indents: None, tags: None }
    }

    /// `indents.scm`, which Enter runs (`Indents`).
    pub fn indents(&self) -> Option<&Query> {
        self.indents.as_ref()
    }

    /// `tags.scm`, which a symbol parse runs (`Tags`).
    pub fn tags(&self) -> Option<&Query> {
        self.tags.as_ref()
    }
}

/// The parser a `State` runs: one compiled into Orlo, or a language folder's, compiled on first
/// use (on the highlight worker) and dropped for the generation if it fails.
pub struct Grammar {
    name: String,
    scope: String,
    /// The built-in language whose fold rules (and PHP's template lines) apply.
    builtin: Option<Language>,
    source: Kind,
    failed: AtomicBool,
}

enum Kind {
    Builtin(Language),
    Folder(Arc<LanguageDef>, OnceLock<Option<Arc<Compiled>>>),
}

impl Grammar {
    /// The built-in language's grammar, shared by every buffer of it.
    pub fn builtin(language: Language) -> Arc<Self> {
        static GRAMMARS: OnceLock<Vec<Arc<Grammar>>> = OnceLock::new();
        let grammars = GRAMMARS.get_or_init(|| {
            (Language::ALL.iter())
                .map(|&l| {
                    let (name, scope) = (name(l).into(), scope(l).into());
                    Arc::new(Self { name, scope, builtin: Some(l), source: Kind::Builtin(l), failed: false.into() })
                })
                .collect()
        });
        grammars[Language::ALL.iter().position(|&l| l == language).unwrap()].clone()
    }

    /// `def`'s grammar in the current registry generation, if its manifest has `tree_sitter`;
    /// nothing is compiled until a parse needs it.
    pub fn of(registry: &Arc<Registry>, def: &Arc<LanguageDef>) -> Option<Arc<Self>> {
        def.manifest.tree_sitter.as_ref()?;
        runtime::generation(registry, def, || {
            let builtin = match Source::of(&def.manifest.tree_sitter.as_ref()?.grammar) {
                Source::Builtin(grammar) => Some(
                    Language::from_registry_id(def.id())
                        .filter(|&l| grammar_name(l) == grammar)
                        .unwrap_or_else(|| Language::ALL.into_iter().find(|&l| grammar_name(l) == grammar).unwrap()),
                ),
                Source::File(_) => None,
            };
            Some(Self {
                name: def.name().into(),
                scope: def.manifest.scope.clone(),
                builtin,
                source: Kind::Folder(def.clone(), OnceLock::new()),
                failed: false.into(),
            })
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The language folder it came from; `None` for a built-in language.
    pub fn language(&self) -> Option<&Arc<LanguageDef>> {
        match &self.source {
            Kind::Builtin(_) => None,
            Kind::Folder(def, _) => Some(def),
        }
    }

    /// Where an error about it points: the grammar file, else the manifest.
    fn label(&self) -> String {
        match &self.source {
            Kind::Builtin(_) => self.name.clone(),
            Kind::Folder(def, _) => {
                let grammar = def.manifest.tree_sitter.as_ref().map(|ts| Source::of(&ts.grammar));
                let file = match grammar {
                    Some(Source::File(path)) => path,
                    _ => crate::manifest::FILE,
                };
                def.folder.join(file).display().to_string()
            }
        }
    }

    /// It failed to compile or to parse: reported once, and left alone for this generation.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Compiled now if it was not yet: call it on the highlight worker.
    pub fn compiled(&self) -> Option<&Compiled> {
        match &self.source {
            Kind::Builtin(language) => Some(compiled(*language)),
            Kind::Folder(def, compiled) => compiled
                .get_or_init(|| runtime::compile(def, self.builtin).map_err(|errors| self.fail(errors)).ok())
                .as_deref()
                .filter(|_| !self.failed()),
        }
    }

    fn fail(&self, errors: Vec<String>) {
        if !self.failed.swap(true, Ordering::Relaxed) {
            runtime::report(errors);
        }
    }
}

/// Grammar failures since the last call, one message each, for the customization alert.
pub fn take_failures() -> Vec<String> {
    runtime::take_failures()
}

/// Load `def`'s grammar and compile every query it names, and check its `.sublime-syntax` on its
/// own: what activating it would report, without keeping anything.
pub fn activate(def: &LanguageDef) -> Result<(), Vec<String>> {
    let mut errors = crate::highlight::check_language_syntax(def).err().into_iter().collect::<Vec<_>>();
    if def.manifest.tree_sitter.is_some() {
        errors.extend(runtime::compile(def, None).err().into_iter().flatten());
    }
    errors.is_empty().then_some(()).ok_or(errors)
}

// Each family keeps one cache and one definition of its syntax name, scope and queries.
macro_rules! grammars {
    ($($family:pat => ($name:literal, $scope:literal, $grammar:literal, $query:expr),)*) => {
        pub fn name(language: Language) -> &'static str {
            match language { $($family => $name,)* }
        }
        pub fn scope(language: Language) -> &'static str {
            match language { $($family => $scope,)* }
        }
        /// The built-in parser it uses (`builtin:<name>`).
        fn grammar_name(language: Language) -> &'static str {
            match language { $($family => $grammar,)* }
        }
        /// Its highlight query, as composed from the built-in ones.
        fn query_text(language: Language) -> String {
            match language { $($family => $query.into(),)* }
        }
        fn compiled(language: Language) -> &'static Arc<Compiled> {
            match language { $($family => {
                static GRAMMAR: OnceLock<Arc<Compiled>> = OnceLock::new();
                GRAMMAR.get_or_init(|| {
                    let grammar = builtin_language($grammar).expect("bundled parser");
                    let query = Query::new(&grammar, &query_text(language))
                        .expect("bundled highlight query matches its grammar");
                    Arc::new(Compiled::new(grammar, query, BUILTIN_PREFIXES))
                })
            },)* }
        }
    };
}

grammars! {
    Language::Rust => ("Rust", "source.rust", "rust",
        format!("(identifier) @variable\n{}", tree_sitter_rust::HIGHLIGHTS_QUERY)),
    Language::Go => ("Go", "source.go", "go", tree_sitter_go::HIGHLIGHTS_QUERY),
    Language::Python => ("Python", "source.python", "python", tree_sitter_python::HIGHLIGHTS_QUERY),
    Language::JavaScript | Language::Jsx => ("JavaScript", "source.js", "javascript",
        format!("{}\n{}", tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY)),
    Language::TypeScript => ("TypeScript", "source.ts", "typescript",
        format!("{}\n{}", tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY)),
    Language::Tsx => ("TypeScript (TSX)", "source.tsx", "tsx",
        format!("{}\n{}\n{}", tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY)),
    Language::C => ("C", "source.c", "c", tree_sitter_c::HIGHLIGHT_QUERY),
    Language::Cpp => ("C++", "source.c++", "cpp",
        format!("{}\n{}", tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY)),
    Language::CSharp => ("C#", "source.cs", "csharp", tree_sitter_c_sharp::HIGHLIGHTS_QUERY),
    Language::Java => ("Java", "source.java", "java", tree_sitter_java::HIGHLIGHTS_QUERY),
    Language::Php => ("PHP", "source.php", "php",
        format!("{}\n(text) @orlo.embedded", tree_sitter_php::HIGHLIGHTS_QUERY)),
    Language::Ruby => ("Ruby", "source.ruby", "ruby", tree_sitter_ruby::HIGHLIGHTS_QUERY),
    Language::Swift => ("Swift", "source.swift", "swift", tree_sitter_swift::HIGHLIGHTS_QUERY),
    Language::Kotlin => ("Kotlin", "source.kotlin", "kotlin", include_str!("queries/kotlin.scm")),
    Language::Bash => ("Bourne Again Shell (bash)", "source.shell", "bash", tree_sitter_bash::HIGHLIGHT_QUERY),
    Language::Hcl | Language::Terraform | Language::TerraformVars | Language::TerraformStack
        | Language::TerraformDeploy | Language::TerraformSearch | Language::TerraformPolicy
        | Language::TerraformPolicyTest => ("HCL", "source.hcl", "hcl", include_str!("queries/hcl.scm")),
}

/// A tree-sitter capture name as the Sublime scope the colour schemes style. With `prefixes`, a
/// name the table lacks takes its longest listed prefix's scope (`keyword.function` as
/// `keyword`), for the longer names nvim-treesitter's queries use.
fn capture_scope(capture: &str, prefixes: bool) -> &'static str {
    let renamed = prefixes.then(|| renamed(capture)).flatten();
    let capture = renamed.as_deref().unwrap_or(capture);
    let prefix = prefixes
        .then(|| {
            let mut name = capture;
            std::iter::from_fn(move || {
                name = name.rsplit_once('.')?.0;
                Some(name)
            })
            .find_map(listed_scope)
        })
        .flatten();
    listed_scope(capture).or(prefix).unwrap_or_else(|| match capture.split('.').next().unwrap_or(capture) {
        "keyword" | "conditional" | "repeat" | "exception" | "include" => "keyword.control",
        "string" | "character" => "string",
        "comment" => "comment",
        "number" | "float" => "constant.numeric",
        "boolean" => "constant.language",
        "function" | "method" => "entity.name.function",
        "type" => "entity.name.type",
        "variable" => "variable.other",
        _ => "",
    })
}

/// A capture name nvim-treesitter has since renamed (Nim's queries use `@field` and
/// `@parameter`), as its current one: `field.x` is `variable.member.x`.
fn renamed(capture: &str) -> Option<String> {
    const RENAMED: [(&str, &str); 8] = [
        ("field", "variable.member"),
        ("parameter", "variable.parameter"),
        ("namespace", "module"),
        ("method", "function.method"),
        ("storageclass", "keyword.modifier"),
        ("type.qualifier", "keyword.modifier"),
        ("symbol", "string.special.symbol"),
        ("preproc", "keyword.directive"),
    ];
    RENAMED.iter().find_map(|(old, new)| {
        let rest = capture.strip_prefix(old)?;
        (rest.is_empty() || rest.starts_with('.')).then(|| format!("{new}{rest}"))
    })
}

fn listed_scope(capture: &str) -> Option<&'static str> {
    Some(match capture {
        "comment" => "comment",
        "comment.documentation" => "comment.documentation",
        "string" => "string",
        "string.special" => "string.special",
        "escape" | "string.escape" => "constant.character.escape",
        "number" => "constant.numeric",
        "constant.builtin" => "constant.language",
        "constant" => "constant.other",
        "keyword" => "keyword.control",
        "operator" | "keyword.operator" => "keyword.operator",
        "function" | "function.method" => "entity.name.function",
        "function.builtin" | "function.method.builtin" => "support.function",
        "function.macro" | "constant.macro" => "support.macro",
        "type" | "constructor" => "entity.name.type",
        "type.builtin" => "support.type",
        "property" | "property.definition" | "variable.member" => "variable.member",
        "module" | "module.builtin" => "entity.name.namespace",
        "variable.builtin" => "variable.language",
        "variable.parameter" => "variable.parameter",
        "variable" => "variable.other",
        "attribute" => "entity.other.attribute-name",
        "tag" => "entity.name.tag",
        "label" => "entity.name.label",
        "punctuation.bracket" => "punctuation.section",
        "punctuation.delimiter" | "delimiter" => "punctuation.separator",
        "punctuation.special" => "punctuation.definition",
        _ => return None,
    })
}

#[derive(Clone)]
struct Snapshot {
    tree: Tree,
    rope: Rope,
    folds: Option<Arc<orlo_editor::fold::Regions>>,
}

// Cancelled parsers resume only on the same rope; cloned jobs share that progress.
struct Pending {
    rope: Rope,
    /// `None` once a parse failed: its store is not reused.
    parser: Option<Parser>,
    old: Option<Tree>,
    result: Option<Tree>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(parser) = self.parser.take() {
            runtime::give_back(parser);
        }
    }
}

/// A line's colours and its `(byte_len, id)` scope stacks.
type Line = (Vec<Run>, Arc<[(usize, ScopeId)]>);

#[derive(Clone)]
pub struct State {
    pub grammar: Arc<Grammar>,
    identity: u64,
    snapshot: Option<Snapshot>,
    pending: Option<Arc<Mutex<Pending>>>,
    changes: Vec<Change>,
    pub revision: u64,
    dirty: bool,
    pub requested: bool,
    // None marks a queried template line whose embedded language stays with syntect.
    colors: Arc<BTreeMap<usize, Option<Line>>>,
}

impl State {
    pub fn new(language: Language) -> Self {
        Self::with_grammar(Grammar::builtin(language))
    }

    pub fn with_grammar(grammar: Arc<Grammar>) -> Self {
        Self {
            grammar,
            identity: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            snapshot: None,
            pending: None,
            changes: Vec::new(),
            revision: 0,
            dirty: true,
            requested: true,
            colors: Arc::new(BTreeMap::new()),
        }
    }

    /// Parse no more: the document is too large, or the grammar failed.
    fn give_up(&mut self) {
        self.snapshot = None;
        self.pending = None;
        self.changes.clear();
        self.colors = Arc::new(BTreeMap::new());
        self.requested = false;
    }

    pub fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        self.pending = None;
        self.requested = true;
        self.colors = Arc::new(BTreeMap::new());
    }

    pub fn text_changed(&mut self, changes: &[Change]) {
        if self.snapshot.is_some() {
            self.changes.extend_from_slice(changes);
        }
    }

    pub fn request(&mut self, rope: &Rope, line: usize) {
        if !self.queried(rope, line) {
            self.requested = rope.len_bytes() <= MAX_PARSE_BYTES && !self.grammar.failed();
        }
    }

    pub fn queried(&self, rope: &Rope, line: usize) -> bool {
        !self.dirty
            && self.snapshot.as_ref().is_some_and(|s| s.rope.is_instance(rope))
            && self.colors.contains_key(&line)
    }

    fn line(&self, rope: &Rope, line: usize) -> Option<&Line> {
        (!self.dirty && self.snapshot.as_ref()?.rope.is_instance(rope))
            .then(|| self.colors.get(&line))
            .flatten()
            .and_then(Option::as_ref)
    }

    /// Never expose parser coordinates for another text instance or an invalidated tree.
    pub fn fold_regions(&self, rope: &Rope) -> Option<&orlo_editor::fold::Regions> {
        let snapshot = self.snapshot.as_ref()?;
        (!self.dirty && snapshot.rope.is_instance(rope)).then_some(snapshot.folds.as_deref()).flatten()
    }

    /// Current named source range strictly enclosing the selected character range.
    pub fn enclosing_named_range(&self, rope: &Rope, range: Range<usize>) -> Option<Option<Range<usize>>> {
        let snapshot = self.snapshot.as_ref()?;
        if self.dirty || !snapshot.rope.is_instance(rope) {
            return None;
        }
        let text = &snapshot.rope;
        if range.start > range.end || range.end > text.len_chars() || text.len_chars() == 0 {
            return Some(None);
        }
        // Half-open selection; a caret takes the character to its right, or the last at EOF.
        let start = if range.is_empty() { range.start.min(text.len_chars() - 1) } else { range.start };
        let end = if range.is_empty() { start + 1 } else { range.end };
        let mut node =
            snapshot.tree.root_node().descendant_for_byte_range(text.char_to_byte(start), text.char_to_byte(end));
        while let Some(current) = node {
            let found = text.byte_to_char(current.start_byte())..text.byte_to_char(current.end_byte());
            if current.is_named()
                && !current.is_missing()
                && found.start <= range.start
                && found.end >= range.end
                && found.len() > range.len()
            {
                return Some(Some(found));
            }
            node = current.parent();
        }
        Some(None)
    }

    /// What Enter indents by: `None` without `indents.scm`, or while the tree is not `rope`'s.
    pub fn indents(&self, rope: &Rope) -> Option<Indents> {
        let snapshot = self.snapshot.as_ref().filter(|s| !self.dirty && s.rope.is_instance(rope))?;
        self.grammar.compiled()?.indents.as_ref()?;
        Some(Indents { grammar: self.grammar.clone(), tree: snapshot.tree.clone(), rope: rope.clone() })
    }

    /// Reuse compatible prepared caches without importing another owner's source identity.
    pub fn adopt(&mut self, mut prepared: Self) {
        if Arc::ptr_eq(&self.grammar, &prepared.grammar) && self.revision == prepared.revision {
            prepared.identity = self.identity;
            *self = prepared;
        }
    }

    /// Stable across cloned highlight jobs, fresh after a syntax switch or invalidation.
    pub fn source_key(&self, rope: &Rope) -> Option<(u64, u64)> {
        (rope.len_bytes() <= MAX_PARSE_BYTES).then_some((self.identity, self.revision))
    }

    pub fn runs(&self, rope: &Rope, line: usize) -> Option<Vec<Run>> {
        self.line(rope, line).map(|l| l.0.clone())
    }

    /// The line's scope stacks: the base scope and the capture's, as its colours have them.
    pub fn scopes(&self, rope: &Rope, line: usize) -> Option<&[(usize, ScopeId)]> {
        self.line(rope, line).map(|l| &l.1[..])
    }

    /// No document copy: the parser reads rope chunks, and edits reuse the preceding tree.
    pub fn prepare(
        &mut self,
        rope: &Rope,
        lines: Range<usize>,
        parse_slice: Option<Duration>,
        stop: impl Fn() -> bool,
        style: impl Fn(&str) -> (gpui::Hsla, crate::highlight::Look),
    ) {
        if rope.len_bytes() > MAX_PARSE_BYTES || self.grammar.failed() {
            self.give_up();
            return;
        }
        if stop() || lines.is_empty() {
            return;
        }
        let handle = self.grammar.clone();
        let Some(grammar) = handle.compiled() else {
            self.give_up();
            return;
        };
        if self.dirty || self.snapshot.as_ref().is_none_or(|s| !s.rope.is_instance(rope)) {
            if self.pending.as_ref().is_none_or(|p| !p.lock().unwrap_or_else(|e| e.into_inner()).rope.is_instance(rope))
            {
                let mut old = self.snapshot.as_ref().map(|s| s.tree.clone());
                if let (Some(tree), Some(snapshot)) = (old.as_mut(), self.snapshot.as_ref()) {
                    match edit(&snapshot.rope, rope, &self.changes) {
                        Some(edit) => tree.edit(&edit),
                        None if !snapshot.rope.is_instance(rope) => old = None,
                        None => {}
                    }
                }
                let parser = match runtime::parser(&grammar.language) {
                    Ok(parser) => parser,
                    Err(error) => {
                        self.grammar.fail(vec![format!("{}: {error}", self.grammar.label())]);
                        self.give_up();
                        return;
                    }
                };
                let pending = Pending { rope: rope.clone(), parser: Some(parser), old, result: None };
                self.pending = Some(Arc::new(Mutex::new(pending)));
            }
            let tree = {
                let pending = self.pending.as_ref().unwrap();
                let mut pending = pending.lock().unwrap_or_else(|e| e.into_inner());
                if pending.result.is_none() {
                    let started = Instant::now();
                    // a parse also returns None when the grammar traps: only a stop asked for is a pause
                    let asked = std::cell::Cell::new(false);
                    let mut progress = |_: &tree_sitter::ParseState| {
                        asked.set(stop() || parse_slice.is_some_and(|slice| started.elapsed() >= slice));
                        flow(asked.get())
                    };
                    let Pending { parser: Some(parser), old, .. } = &mut *pending else {
                        return;
                    };
                    let result = parser.parse_with_options(
                        &mut |byte, _| {
                            if byte >= rope.len_bytes() {
                                return &[][..];
                            }
                            let (chunk, start, _, _) = rope.chunk_at_byte(byte);
                            &chunk.as_bytes()[byte - start..]
                        },
                        old.as_ref(),
                        Some(ParseOptions::new().progress_callback(&mut progress)),
                    );
                    if result.is_none() && !asked.get() {
                        pending.parser = None;
                        drop(pending);
                        let error = format!("{}: the grammar failed while parsing", self.grammar.label());
                        self.grammar.fail(vec![error]);
                        self.give_up();
                        return;
                    }
                    pending.result = result;
                }
                pending.result.clone()
            };
            let Some(tree) = tree else {
                return;
            };
            self.pending = None;
            self.snapshot = Some(Snapshot { tree, rope: rope.clone(), folds: None });
            self.changes.clear();
            self.colors = Arc::new(BTreeMap::new());
            self.dirty = false;
        }
        let to = lines.end.min(rope.len_lines());
        let from = lines.start.max(to.saturating_sub(MAX_COLOR_LINES)).min(rope.len_lines() - 1);
        let bytes = rope.line_to_byte(from)..rope.line_to_byte(to);
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(bytes.clone());
        let mut spans = vec![Vec::new(); to - from];
        let mut progress = |_: &tree_sitter::QueryCursorState| flow(stop());
        let mut captures = cursor.captures_with_options(
            &grammar.query,
            self.snapshot.as_ref().unwrap().tree.root_node(),
            |node: tree_sitter::Node| rope.byte_slice(node.byte_range()).chunks().map(str::as_bytes),
            QueryCursorOptions::new().progress_callback(&mut progress),
        );
        while let Some((m, index)) = captures.next() {
            if stop() {
                return;
            }
            let c = m.captures()[*index];
            let start = c.node.start_byte().max(bytes.start);
            let end = c.node.end_byte().min(bytes.end);
            if start >= end {
                continue;
            }
            for line in rope.byte_to_line(start)..=rope.byte_to_line(end - 1).min(to - 1) {
                let line_start = rope.line_to_byte(line);
                let len = content_len(rope.line(line));
                if len > crate::highlight::MAX_HIGHLIGHT {
                    continue;
                }
                let start = start.saturating_sub(line_start).min(len);
                let end = end.saturating_sub(line_start).min(len);
                if start < end {
                    spans[line - from].push((start, end, c.index as usize, m.pattern_index));
                }
            }
        }
        let palette: Vec<_> = grammar.scopes.iter().map(|s| style(s)).collect();
        let default = style("");
        // a stack has the winning capture's scope alone (an escape in a string is
        // code); stack every capture in force if a consumer needs more
        let base = self.grammar.scope();
        let scope_ids: Vec<_> = grammar.scopes.iter().map(|s| crate::highlight::scope_id(&[base, s])).collect();
        let top = crate::highlight::scope_id(&[base]);
        let colors = Arc::make_mut(&mut self.colors);
        if colors.len() + to - from > MAX_COLOR_LINES {
            colors.clear();
        }
        for (offset, spans) in spans.into_iter().enumerate() {
            if stop() {
                return;
            }
            if self.grammar.builtin == Some(Language::Php)
                && spans.iter().any(|span| grammar.query.capture_names()[span.2] == "orlo.embedded")
            {
                colors.insert(from + offset, None);
                continue;
            }
            let len = content_len(rope.line(from + offset));
            let mut events = vec![(0, false, usize::MAX), (len, false, usize::MAX)];
            for (id, &(start, end, _, _)) in spans.iter().enumerate() {
                events.extend([(start, true, id), (end, false, id)]);
            }
            events.sort_unstable();
            let mut active = std::collections::BTreeSet::new();
            let mut runs: Vec<Run> = Vec::new();
            let mut scopes: Vec<(usize, ScopeId)> = Vec::new();
            let mut previous = 0;
            for (position, start, id) in events {
                let capture = active.first().map(|&(_, _, capture, _)| capture);
                let (fg, look) = capture.map_or(default, |c| palette[c]);
                let scope = capture.map_or(top, |c| scope_ids[c]);
                let len = position - previous;
                if len > 0 {
                    match runs.last_mut() {
                        Some(last) if (last.1, last.2) == (fg, look) => last.0 += len,
                        _ => runs.push((len, fg, look)),
                    }
                    match scopes.last_mut() {
                        Some(last) if last.1 == scope => last.0 += len,
                        _ => scopes.push((len, scope)),
                    }
                }
                previous = position;
                if id == usize::MAX {
                    continue;
                }
                let span = spans[id];
                let key = (span.1 - span.0, std::cmp::Reverse(span.3), span.2, id);
                if start {
                    active.insert(key);
                } else {
                    active.remove(&key);
                }
            }
            colors.insert(from + offset, Some((runs, scopes.into())));
        }
        // folds walk the whole tree: after the visible lines' colours, so a job stopped in the
        // walk (the next keystroke) has still coloured them, and the next one walks again
        let snapshot = self.snapshot.as_mut().unwrap();
        // a language folder without folds.scm keeps indentation folding
        if snapshot.folds.is_none() && (grammar.folds.is_some() || self.grammar.builtin.is_some()) {
            let folds = match (&grammar.folds, self.grammar.builtin) {
                (Some(query), _) => query_folds(&snapshot.tree, rope, query, &stop),
                (None, language) => discover_folds(&snapshot.tree, rope, language.unwrap(), &stop),
            };
            let Some(folds) = folds else {
                return;
            };
            snapshot.folds = Some(Arc::new(folds));
        }
        self.requested = false;
    }
}

/// A tree current for its text and the language's `indents.scm`: nvim-treesitter's
/// `@indent.begin` (with `indent.immediate` and `indent.start_at_same_line`), `@indent.end`,
/// `@indent.branch` and `@indent.dedent`, as Neovim indents a new line. Its other captures are
/// ignored.
pub struct Indents {
    grammar: Arc<Grammar>,
    tree: Tree,
    rope: Rope,
}

#[derive(Clone, Copy, Default)]
struct Indent {
    begin: bool,
    end: bool,
    branch: bool,
    dedent: bool,
    immediate: bool,
    same_line: bool,
}

/// How far up Enter looks past blank lines for the line its new one follows.
const INDENT_LOOKBACK: usize = 1_000;

impl Indents {
    /// The levels to indent the line a line break at `at` starts (chars), the text from `rest`
    /// moving down onto it; `blank`: the line stays empty whatever follows (a brace Enter pushes
    /// down further). Rows are those of the text after the break, which the tree has not seen.
    /// `None` where the tree has the line in an error outside any construct it knows (a block
    /// still being written at the end of a file).
    pub fn levels(&self, at: usize, rest: usize, blank: bool) -> Option<usize> {
        let query = self.grammar.compiled()?.indents.as_ref()?;
        let rope = &self.rope;
        let (at, rest) = (at.min(rope.len_chars()), rest.clamp(at, rope.len_chars()));
        let at_byte = rope.char_to_byte(at);
        let rest_line = rope.char_to_line(rest);
        let rest_end = rope.line_to_byte(rest_line) + content_len(rope.line(rest_line));
        let rest_byte = rope.char_to_byte(rest);
        let first = (!blank)
            .then(|| rope.byte_slice(rest_byte..rest_end.max(rest_byte)).to_string())
            .and_then(|text| text.find(|c: char| !c.is_whitespace()).map(|i| rest_byte + i));
        // from here on the text is on the new line
        let split = first.unwrap_or(at_byte);
        let new_row = rope.byte_to_line(at_byte) + 1;
        let row = |byte: usize, end: bool| {
            if byte < at_byte || (end && byte == at_byte) {
                rope.byte_to_line(byte)
            } else if byte > split || (!end && byte == split) {
                rope.byte_to_line(byte) - rope.byte_to_line(split) + new_row
            } else {
                new_row - 1
            }
        };
        let root = self.tree.root_node();
        let at_point = |byte: usize| root.descendant_for_byte_range(byte, byte);
        // the smallest node across the new, empty line
        let across = || {
            let mut node = at_point(at_byte)?;
            while node.start_byte() >= at_byte {
                node = node.parent()?;
            }
            Some(node)
        };
        let line_text = |line: usize, to: usize| {
            let start = rope.line_to_byte(line);
            let text = rope.byte_slice(start..to.max(start)).to_string();
            let first = text.find(|c: char| !c.is_whitespace()).map(|i| start + i);
            let last = text.char_indices().rev().find(|(_, c)| !c.is_whitespace()).map(|(i, _)| start + i);
            first.zip(last)
        };
        // the line the new one follows: its text before the break, else the last above with text
        let above = (first.is_none()).then(|| {
            let line = rope.byte_to_line(at_byte);
            std::iter::once((line, at_byte))
                .chain((line.saturating_sub(INDENT_LOOKBACK)..line).rev().map(|l| {
                    let start = rope.line_to_byte(l);
                    (l, start + content_len(rope.line(l)))
                }))
                .find_map(|(line, to)| line_text(line, to).map(|text| (line, text)))
        });
        let from =
            above.flatten().map_or(rope.line_to_byte(rope.byte_to_line(at_byte)), |(line, _)| rope.line_to_byte(line));
        let flags = indent_flags(query, root, rope, from..(split.max(at_byte) + 1).min(rope.len_bytes()));
        let flag = |node: &tree_sitter::Node| flags.get(&node.id()).copied().unwrap_or_default();
        let start = match above {
            Some(Some((line, (first, last)))) => {
                let mut node = at_point(last);
                // a trailing comment is passed over for the code before it
                if let Some(comment) = node.filter(|n| n.kind().contains("comment")) {
                    if at_point(first).is_some_and(|n| n.id() != comment.id()) {
                        let before = line_text(line, comment.start_byte()).map(|(_, last)| at_point(last));
                        node = before.flatten().or(node);
                    }
                }
                node.filter(|n| !flag(n).end).or_else(across)
            }
            Some(None) => across(),
            None => at_point(split),
        };
        let lost = |node: tree_sitter::Node| node.is_error() && node.parent().is_none_or(|p| p.parent().is_none());
        if std::iter::successors(start, |n| n.parent()).any(lost) {
            return None;
        }
        let mut levels = 0isize;
        let mut processed = std::collections::HashSet::new();
        let mut node = start;
        while let Some(n) = node {
            let f = flag(&n);
            let (start_row, end_row) = (row(n.start_byte(), false), row(n.end_byte(), true));
            let open = !processed.contains(&start_row);
            let mut done = false;
            if open && ((f.branch && start_row == new_row) || (f.dedent && start_row != new_row)) {
                levels -= 1;
                done = true;
            }
            let in_error = open && n.parent().is_some_and(|p| p.has_error());
            if open
                && f.begin
                && (start_row != end_row || in_error || f.immediate)
                && (start_row != new_row || f.same_line)
            {
                levels += 1;
                done = true;
            }
            if done {
                processed.insert(start_row);
            }
            node = n.parent();
        }
        Some(levels.max(0) as usize)
    }
}

/// A language folder's `tags.scm` over a text, a stretch at a time: each `@definition.<kind>`
/// match's `@name`, as a symbol parse finds definitions.
pub struct Tags {
    grammar: Arc<Grammar>,
    /// A parse that stopped, to go on with the same text.
    parser: Option<Parser>,
    tree: Option<Tree>,
    /// The line the query goes on from.
    line: usize,
}

/// A tags query stops at most this many lines apart (a stopped stretch is queried again).
const TAG_LINES: usize = 256;

impl Tags {
    pub fn new(grammar: Arc<Grammar>) -> Self {
        Self { grammar, parser: None, tree: None, line: 0 }
    }

    /// Go on: true at the end (or with no tags to find), false when `pause` says so at a stretch
    /// of the parse or the query. Definitions go to `found` in order.
    pub fn step(&mut self, rope: &Rope, pause: impl Fn() -> bool, found: &mut Vec<IndexedSymbol>) -> bool {
        if rope.len_bytes() > MAX_PARSE_BYTES || self.grammar.failed() {
            return true;
        }
        let handle = self.grammar.clone();
        let Some((compiled, query)) = handle.compiled().and_then(|c| Some((c, c.tags.as_ref()?))) else {
            return true;
        };
        if self.tree.is_none() {
            let mut parser = match self.parser.take().map_or_else(|| runtime::parser(&compiled.language), Ok) {
                Ok(parser) => parser,
                Err(error) => {
                    self.grammar.fail(vec![format!("{}: {error}", self.grammar.label())]);
                    return true;
                }
            };
            let asked = std::cell::Cell::new(false);
            let mut progress = |_: &tree_sitter::ParseState| {
                asked.set(pause());
                flow(asked.get())
            };
            let tree = parser.parse_with_options(
                &mut |byte, _| match byte < rope.len_bytes() {
                    true => {
                        let (chunk, start, _, _) = rope.chunk_at_byte(byte);
                        &chunk.as_bytes()[byte - start..]
                    }
                    false => &[][..],
                },
                None,
                Some(ParseOptions::new().progress_callback(&mut progress)),
            );
            match tree {
                Some(tree) => {
                    runtime::give_back(parser);
                    self.tree = Some(tree);
                }
                None if asked.get() => {
                    self.parser = Some(parser);
                    return false;
                }
                // a trapped parser is not pooled
                None => {
                    self.grammar.fail(vec![format!("{}: the grammar failed while parsing", self.grammar.label())]);
                    return true;
                }
            }
        }
        let tree = self.tree.as_ref().unwrap();
        let names = query.capture_names();
        let name = query.capture_index_for_name("name");
        while self.line < rope.len_lines() {
            if pause() {
                return false;
            }
            let to = (self.line + TAG_LINES).min(rope.len_lines());
            let bytes = rope.line_to_byte(self.line)..rope.line_to_byte(to);
            let mut cursor = QueryCursor::new();
            cursor.set_byte_range(bytes.clone());
            let asked = std::cell::Cell::new(false);
            let mut progress = |_: &tree_sitter::QueryCursorState| {
                asked.set(pause());
                flow(asked.get())
            };
            let mut matches = cursor.matches_with_options(
                query,
                tree.root_node(),
                |node: tree_sitter::Node| rope.byte_slice(node.byte_range()).chunks().map(str::as_bytes),
                QueryCursorOptions::new().progress_callback(&mut progress),
            );
            let mut stretch = Vec::new();
            while let Some(m) = matches.next() {
                let kind = m.captures().iter().find_map(|c| names[c.index as usize].strip_prefix("definition."));
                let node = m.captures().iter().find(|c| Some(c.index) == name).map(|c| c.node);
                let (Some(kind), Some(node)) = (kind, node) else {
                    continue;
                };
                // a definition is the stretch's its name starts in
                if !bytes.contains(&node.start_byte()) {
                    continue;
                }
                let text = rope.byte_slice(node.byte_range()).to_string();
                let text = text.lines().next().unwrap_or_default();
                let lead = text.len() - text.trim_start().len();
                if text.trim().is_empty() {
                    continue;
                }
                let line = rope.byte_to_line(node.start_byte());
                let start = rope.line_to_byte(line);
                stretch.push(IndexedSymbol {
                    name: text.trim().to_string(),
                    kind: kind.split('.').next().unwrap_or(kind).to_string(),
                    line,
                    col: rope.byte_slice(start..node.start_byte() + lead).len_chars(),
                    signature: crate::index::signature(&rope.line(line).to_string()),
                    doc: None,
                });
            }
            drop(matches);
            // a stretch stopped part way is queried again
            if asked.get() {
                return false;
            }
            stretch.sort_by_key(|s| (s.line, s.col));
            stretch.dedup_by(|a, b| (a.line, a.col, &a.name) == (b.line, b.col, &b.name));
            found.extend(stretch);
            self.line = to;
        }
        true
    }
}

impl Drop for Tags {
    fn drop(&mut self) {
        if let Some(parser) = self.parser.take() {
            runtime::give_back(parser);
        }
    }
}

/// The `indents.scm` captures of the nodes in `bytes` and those around them.
fn indent_flags(
    query: &Query,
    root: tree_sitter::Node,
    rope: &Rope,
    bytes: Range<usize>,
) -> std::collections::HashMap<usize, Indent> {
    let names = query.capture_names();
    let mut flags = std::collections::HashMap::<usize, Indent>::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(bytes);
    let mut matches = cursor
        .matches(query, root, |node: tree_sitter::Node| rope.byte_slice(node.byte_range()).chunks().map(str::as_bytes));
    while let Some(m) = matches.next() {
        let set = |key: &str| query.property_settings(m.pattern_index).iter().any(|p| &*p.key == key);
        for c in m.captures() {
            let f = flags.entry(c.node.id()).or_default();
            match names[c.index as usize] {
                "indent.begin" => {
                    f.begin = true;
                    f.immediate |= set("indent.immediate");
                    f.same_line |= set("indent.start_at_same_line");
                }
                "indent.end" => f.end = true,
                "indent.branch" => f.branch = true,
                "indent.dedent" => f.dedent = true,
                _ => {}
            }
        }
    }
    flags
}

/// Walk only multiline subtrees on the existing parser worker. Delimiters are grammar
/// tokens, so braces in strings and comments cannot manufacture code blocks. Keyword-based
/// languages also name their complete blocks; Python's body starts below its header.
fn discover_folds(
    tree: &Tree,
    rope: &Rope,
    language: Language,
    stop: &impl Fn() -> bool,
) -> Option<orlo_editor::fold::Regions> {
    let mut ranges = Vec::new();
    let mut cursor = tree.walk();
    loop {
        if stop() {
            return None;
        }
        let node = cursor.node();
        let multiline = node.start_position().row < node.end_position().row;
        if multiline && node.is_named() && !node.is_error() && !node.is_missing() {
            let kind = node.kind();
            let first = node.child(0).map(|n| n.kind());
            let last = node.child(node.child_count().saturating_sub(1)).map(|n| n.kind());
            let delimited = matches!((first, last), (Some("{"), Some("}")) | (Some("["), Some("]")));
            let keyword = match language {
                Language::Python => {
                    node.child_by_field_name("body").is_some()
                        || matches!(
                            kind,
                            "if_statement"
                                | "elif_clause"
                                | "else_clause"
                                | "except_clause"
                                | "finally_clause"
                                | "case_clause"
                        )
                }
                Language::Hcl => kind == "block" && last == Some("block_end"),
                Language::Ruby => matches!(
                    kind,
                    "method"
                        | "singleton_method"
                        | "class"
                        | "module"
                        | "do_block"
                        | "if"
                        | "unless"
                        | "case"
                        | "while"
                        | "until"
                        | "for"
                        | "begin"
                ),
                Language::Bash => matches!(
                    kind,
                    "if_statement" | "case_statement" | "for_statement" | "while_statement" | "function_definition"
                ),
                _ => false,
            };
            if delimited || keyword || kind.contains("comment") || matches!(kind, "jsx_element" | "jsx_fragment") {
                let header = node.start_position().row;
                let line = rope.line(header);
                let start = rope.line_to_byte(header) + content_len(line);
                let end = node.end_byte();
                if start < end {
                    ranges.push(rope.byte_to_char(start)..rope.byte_to_char(end));
                }
            }
        }
        if multiline && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Some(orlo_editor::fold::Regions::new(rope, ranges));
            }
        }
    }
}

/// A language folder's `folds.scm`: each multiline `@fold` folds below its first line.
fn query_folds(
    tree: &Tree,
    rope: &Rope,
    query: &Query,
    stop: &impl Fn() -> bool,
) -> Option<orlo_editor::fold::Regions> {
    let mut ranges = Vec::new();
    if let Some(fold) = query.capture_index_for_name("fold") {
        let mut cursor = QueryCursor::new();
        let mut progress = |_: &tree_sitter::QueryCursorState| flow(stop());
        let mut captures = cursor.captures_with_options(
            query,
            tree.root_node(),
            |node: tree_sitter::Node| rope.byte_slice(node.byte_range()).chunks().map(str::as_bytes),
            QueryCursorOptions::new().progress_callback(&mut progress),
        );
        while let Some((m, index)) = captures.next() {
            let c = m.captures()[*index];
            if c.index != fold || c.node.start_position().row >= c.node.end_position().row {
                continue;
            }
            let header = c.node.start_position().row;
            let start = rope.line_to_byte(header) + content_len(rope.line(header));
            if start < c.node.end_byte() {
                ranges.push(rope.byte_to_char(start)..rope.byte_to_char(c.node.end_byte()));
            }
        }
    }
    (!stop()).then(|| orlo_editor::fold::Regions::new(rope, ranges))
}

/// Stops tree-sitter's parse or query when `stop`.
fn flow(stop: bool) -> ControlFlow<()> {
    match stop {
        true => ControlFlow::Break(()),
        false => ControlFlow::Continue(()),
    }
}

fn content_len(line: ropey::RopeSlice) -> usize {
    let chars = line.len_chars();
    line.len_bytes()
        - if chars > 0 && line.char(chars - 1) == '\n' {
            1 + usize::from(chars > 1 && line.char(chars - 2) == '\r')
        } else {
            0
        }
}

// Tree-sitter reads the snapshot's UTF-8 bytes: these convert a char offset in that snapshot.

/// `offset`'s byte in `rope`, as tree-sitter counts its input.
fn byte(rope: &Rope, offset: CharOffset) -> ByteOffset {
    ByteOffset(rope.char_to_byte(offset.0))
}

/// `offset` as a tree-sitter point in `rope`: its row, and a column in bytes (not chars) from
/// the row's start.
fn point(rope: &Rope, offset: CharOffset) -> Point {
    let ByteOffset(byte) = byte(rope, offset);
    let row = rope.byte_to_line(byte);
    Point::new(row, byte - rope.line_to_byte(row))
}

/// Collapse sequential character edits into one replacement, preserving the unchanged tree.
fn edit(old: &Rope, new: &Rope, changes: &[Change]) -> Option<InputEdit> {
    let first = changes.first()?;
    let (mut start, mut end, mut new_end) = (first.start, first.start + first.removed, first.start + first.inserted);
    let mut delta = first.inserted as isize - first.removed as isize;
    for c in changes.iter().skip(1) {
        if c.start + c.removed > new_end {
            end = (c.start + c.removed).checked_add_signed(-delta)?;
        }
        start = start.min(c.start);
        delta += c.inserted as isize - c.removed as isize;
        new_end = end.checked_add_signed(delta)?;
    }
    let end_new = end.checked_add_signed(delta)?;
    if start > end
        || end > old.len_chars()
        || end_new > new.len_chars()
        || start > end_new
        || old.len_chars().checked_add_signed(delta)? != new.len_chars()
    {
        return None;
    }
    let (start, end, end_new) = (CharOffset(start), CharOffset(end), CharOffset(end_new));
    Some(InputEdit {
        start_byte: byte(old, start).0,
        old_end_byte: byte(old, end).0,
        new_end_byte: byte(new, end_new).0,
        start_position: point(old, start),
        old_end_position: point(old, end),
        new_end_position: point(new, end_new),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Colours `lines` of `rope` with the active theme, never stopping.
    fn prepare(state: &mut State, rope: &Rope, lines: Range<usize>) {
        state.prepare(rope, lines, None, || false, crate::highlight::scope_style);
    }

    #[test]
    fn a_job_stopped_while_walking_folds_has_coloured_its_lines() {
        let mut rope = Rope::from_str(&"fn f() {\n    1;\n}\n".repeat(20_000));
        let mut state = State::new(Language::Rust);
        prepare(&mut state, &rope, 0..1);
        let at = rope.line_to_char(30_000) + 4;
        rope.insert_char(at, 'x');
        state.invalidate();
        state.text_changed(&[Change { start: at, removed: 0, inserted: 1, lines: (0, 0) }]);
        // the next keystroke arrives while the job walks the whole tree for folds
        let calls = std::cell::Cell::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 5_000
        };
        state.prepare(&rope, 30_000..30_001, None, stop, crate::highlight::scope_style);
        assert!(state.runs(&rope, 30_000).is_some(), "stopped after {} checks", calls.get());
    }

    #[test]
    fn syntax_selection_boundaries_empty_equal_wrappers_unicode_and_malformed() {
        for text in ["fn main() { e\u{301} + 雪; }", "fn main() { café + 雪; ", "fn main() { café + 雪; } trailing"]
        {
            let rope = Rope::from_str(text);
            let mut state = State::new(Language::Rust);
            prepare(&mut state, &rope, 0..1);
            let at = 12;
            let found = state.enclosing_named_range(&rope, at..at).unwrap().unwrap();
            assert_eq!(found.start, at);
            assert_eq!(rope.slice(found.clone()).to_string(), if text.contains('é') { "café" } else { "e\u{301}" });
            let larger = state.enclosing_named_range(&rope, found.clone()).unwrap().unwrap();
            assert!(larger.len() > found.len());
            // EOF uses the final character, but still encloses the actual EOF caret.
            let eof = state.enclosing_named_range(&rope, rope.len_chars()..rope.len_chars()).unwrap().unwrap();
            assert_eq!(eof.end, rope.len_chars());
            assert!(!eof.is_empty());
        }
        let rope = Rope::from_str("foo");
        let mut state = State::new(Language::Python);
        prepare(&mut state, &rope, 0..1);
        assert_eq!(state.enclosing_named_range(&rope, 0..3), Some(None), "equal expression/module wrappers skipped");
        assert_eq!(state.enclosing_named_range(&rope, 0..0), Some(Some(0..3)), "right character at start");
        assert_eq!(state.enclosing_named_range(&rope, 3..3), Some(Some(0..3)), "left character at EOF");
        let empty = Rope::new();
        prepare(&mut state, &empty, 0..1);
        assert_eq!(state.enclosing_named_range(&empty, 0..0), Some(None));
    }

    #[test]
    fn syntax_selection_rejects_dirty_foreign_cancelled_and_oversized_sources() {
        let rope = Rope::from_str("fn main() { café + 雪; }");
        let mut state = State::new(Language::Rust);
        assert_eq!(state.enclosing_named_range(&rope, 13..13), None);
        state.prepare(&rope, 0..1, None, || true, crate::highlight::scope_style);
        assert_eq!(state.enclosing_named_range(&rope, 13..13), None, "cancelled before parse");
        prepare(&mut state, &rope, 0..1);
        assert!(state.enclosing_named_range(&rope, 13..13).is_some());
        assert_eq!(state.enclosing_named_range(&Rope::from_str(&rope.to_string()), 13..13), None);
        state.invalidate();
        assert_eq!(state.enclosing_named_range(&rope, 13..13), None);
        let mut changed = rope.clone();
        changed.insert(0, &"// new\n".repeat(10_000));
        state.prepare(&changed, 0..1, Some(Duration::ZERO), || false, crate::highlight::scope_style);
        assert_eq!(state.enclosing_named_range(&changed, 13..13), None, "sliced parse pending");
        prepare(&mut state, &changed, 0..1);
        assert!(state.enclosing_named_range(&changed, 70_013..70_013).is_some());
        assert_eq!(state.enclosing_named_range(&rope, 13..13), None);
        let huge = Rope::from_str(&" ".repeat(MAX_PARSE_BYTES + 1));
        assert_eq!(state.source_key(&huge), None);
        prepare(&mut state, &huge, 0..1);
        assert_eq!(state.enclosing_named_range(&huge, 0..0), None);
    }

    #[test]
    fn syntax_selection_named_nesting_is_strict_and_uses_character_offsets() {
        let rope = Rope::from_str("fn main() { café + 雪; }");
        let mut state = State::new(Language::Rust);
        prepare(&mut state, &rope, 0..1);
        assert_eq!(state.enclosing_named_range(&rope, 13..13), Some(Some(12..16)));
        assert_eq!(state.enclosing_named_range(&rope, 12..16), Some(Some(12..20)));
        assert_eq!(state.enclosing_named_range(&rope, 12..20), Some(Some(12..21)));
    }

    #[test]
    fn syntax_folds_use_braces_comments_markup_and_unicode_node_ends() {
        for (language, text, header, last, suffix) in [
            (Language::Rust, "fn café() {\n雪();\n} next();\n", 0, 2, " next();\n"),
            (Language::Rust, "fn café() {\r\n雪();\r\n} next();\r\n", 0, 2, " next();\r\n"),
            (Language::Go, "package main\nfunc café() {\nprintln(1)\n}\n", 1, 3, "\n"),
            (Language::JavaScript, "const café = {\n雪: 1\n}; after();\n", 0, 2, "; after();\n"),
            (Language::C, "void café() {\nreturn;\n}\n", 0, 2, "\n"),
            (Language::Cpp, "class Café {\nint value;\n};\n", 0, 2, ";\n"),
            (Language::CSharp, "class Café {\nint value;\n}\n", 0, 2, "\n"),
            (Language::Java, "class Café {\nint value;\n}\n", 0, 2, "\n"),
            (Language::Php, "<?php function café() {\nreturn 1;\n}\n", 0, 2, "\n"),
            (Language::Swift, "func café() {\nprint(1)\n}\n", 0, 2, "\n"),
            (Language::Kotlin, "fun café() {\nprintln(1)\n}\n", 0, 2, "\n"),
            (Language::Hcl, "resource \"雪\" \"x\" {\nvalue = 1\n}\n", 0, 2, "\n"),
            (Language::TypeScript, "interface Café {\nvalue: number\n}\n", 0, 2, "\n"),
            (Language::Python, "def café():\n    雪()\nnext()\n", 0, 1, "\nnext()\n"),
            (Language::Ruby, "def café\n雪()\nend\n", 0, 2, "\n"),
            (Language::Bash, "if true; then\necho 雪\nfi\n", 0, 2, "\n"),
            (Language::Rust, "/* café\n雪\n*/ next();\n", 0, 2, " next();\n"),
            (Language::Jsx, "const café = <div>\n雪\n</div>; next();\n", 0, 2, "; next();\n"),
            (Language::Tsx, "const café = <>\n雪\n</>; next();\n", 0, 2, "; next();\n"),
        ] {
            let rope = Rope::from_str(text);
            let mut state = State::new(language);
            prepare(&mut state, &rope, 0..rope.len_lines());
            let regions = state.fold_regions(&rope).unwrap();
            let range = regions.at_header(header).unwrap_or_else(|| {
                panic!("{language:?}: {}", state.snapshot.as_ref().unwrap().tree.root_node().to_sexp())
            });
            assert_eq!(rope.char_to_line(range.start), header);
            assert_eq!(rope.char_to_line(range.end), last);
            assert_eq!(rope.slice(range.end..).to_string(), suffix, "{language:?}");
            assert_eq!(regions.innermost(header), Some(range.clone()));
            assert_eq!(regions.at_level(1), vec![range]);
        }
    }

    #[test]
    fn syntax_folds_keep_complete_siblings_before_malformed_or_incomplete_code() {
        for (language, text, suffix) in [
            (
                Language::Rust,
                "fn café() {\n雪();\n} tail();\nfn broken() {\nlet =\n",
                " tail();\nfn broken() {\nlet =\n",
            ),
            (Language::Rust, "/* café\n雪\n*/ tail();\nfn broken( {\n", " tail();\nfn broken( {\n"),
            (
                Language::Jsx,
                "const café = <div>\n雪\n</div>; tail();\nconst broken = <span>\n",
                "; tail();\nconst broken = <span>\n",
            ),
        ] {
            let rope = Rope::from_str(text);
            let mut state = State::new(language);
            prepare(&mut state, &rope, 0..rope.len_lines());
            assert!(state.snapshot.as_ref().unwrap().tree.root_node().has_error(), "{language:?}");
            let regions = state.fold_regions(&rope).unwrap();
            let complete = regions.at_header(0).expect("complete structure survives later parse errors");
            assert_eq!(rope.slice(complete.end..).to_string(), suffix, "{language:?}");
            assert_eq!(regions.innermost(1), Some(complete), "{language:?}");
            // Recovery can retain incomplete structures, but every candidate belongs to
            // this current source and has a visible header and at least one hidden line.
            for level in 1..=rope.len_lines() {
                for range in regions.at_level(level) {
                    assert!(range.start < range.end && range.end <= rope.len_chars(), "{language:?}: {range:?}");
                    assert!(rope.char_to_line(range.start) < rope.char_to_line(range.end - 1));
                }
            }
        }
    }

    #[test]
    fn syntax_folds_python_conditional_and_handler_headers() {
        let rope = Rope::from_str("if ready:\n    first()\nelif next:\n    second()\nelse:\n    third()\ntry:\n    work()\nexcept Error:\n    recover()\nfinally:\n    finish()\n");
        let mut state = State::new(Language::Python);
        prepare(&mut state, &rope, 0..rope.len_lines());
        let regions = state.fold_regions(&rope).unwrap();
        for header in [0, 2, 4, 6, 8, 10] {
            assert!(
                regions.at_header(header).is_some(),
                "header {header}: {}",
                state.snapshot.as_ref().unwrap().tree.root_node().to_sexp()
            );
        }
    }

    #[test]
    fn syntax_folds_nest_ignore_string_braces_and_reject_stale_or_cancelled_text() {
        let mut rope = Rope::from_str("fn café() {\nif true {\n雪();\n}\n}\n");
        let mut state = State::new(Language::Rust);
        prepare(&mut state, &rope, 0..rope.len_lines());
        let regions = state.fold_regions(&rope).unwrap();
        let inner = regions.at_header(1).unwrap();
        assert_eq!(regions.at_level(2), vec![inner.clone()]);
        assert_eq!(regions.innermost(2), Some(inner));
        let cached = state.snapshot.as_ref().unwrap().folds.as_ref().unwrap().clone();
        prepare(&mut state, &rope, 1..2);
        assert!(
            Arc::ptr_eq(&cached, state.snapshot.as_ref().unwrap().folds.as_ref().unwrap()),
            "coloring another viewport reuses discovery"
        );
        let stale = state.clone();
        rope.insert(0, "// 🦀\n");
        assert!(stale.fold_regions(&rope).is_none(), "rope identity protects even without invalidation");
        state.invalidate();
        state.text_changed(&[Change { start: 0, removed: 0, inserted: 5, lines: (0, 1) }]);
        assert!(state.fold_regions(&rope).is_none());
        state.prepare(&rope, 0..1, None, || true, crate::highlight::scope_style);
        assert!(state.fold_regions(&rope).is_none());
        prepare(&mut state, &rope, 0..rope.len_lines());
        assert!(state.fold_regions(&rope).unwrap().at_header(2).is_some());
        let mut fresh = State::new(Language::Rust);
        prepare(&mut fresh, &rope, 0..rope.len_lines());
        assert_eq!(state.fold_regions(&rope).unwrap().at_level(2), fresh.fold_regions(&rope).unwrap().at_level(2));
        assert!(discover_folds(&state.snapshot.as_ref().unwrap().tree, &rope, Language::Rust, &|| true).is_none());
        let rope = Rope::from_str("let text = r#\"{\n    fake\n}\"#;\n");
        prepare(&mut state, &rope, 0..rope.len_lines());
        assert!(state.fold_regions(&rope).unwrap().at_level(1).is_empty(), "string contents are not blocks");
    }

    #[test]
    fn bundled_grammars_parse_representative_unicode_documents() {
        for (language, text) in [
            (Language::Rust, "fn café() { let value = \"雪\"; }\n"),
            (Language::Go, "package main\nfunc café() { println(\"雪\") }\n"),
            (Language::Python, "def café():\n    return '雪'\n"),
            (Language::JavaScript, "function café() { return '雪'; }\n"),
            (Language::Jsx, "const App = () => <div>雪</div>;\n"),
            (Language::TypeScript, "const café: string = '雪';\n"),
            (Language::Tsx, "const App = (p: string) => <div>雪</div>;\n"),
            (Language::C, "int café(void) { return 12; }\n"),
            (Language::Cpp, "class Café { int value = 12; };\n"),
            (Language::CSharp, "class Café { string value = \"雪\"; }\n"),
            (Language::Java, "class Café { String value = \"雪\"; }\n"),
            (Language::Php, "<?php function café() { return '雪'; }\n"),
            (Language::Ruby, "def café\n  '雪'\nend\n"),
            (Language::Swift, "func café() -> String { return \"雪\" }\n"),
            (Language::Kotlin, "fun café(): String { return \"雪\" }\n"),
            (Language::Bash, "if true; then echo '雪'; fi\n"),
            (Language::Hcl, "value = \"雪\"\n"),
        ] {
            let rope = Rope::from_str(text);
            let mut state = State::new(language);
            prepare(&mut state, &rope, 0..rope.len_lines());
            let tree = &state.snapshot.as_ref().expect("parsed tree").tree;
            assert!(!tree.root_node().has_error(), "{language:?}: {}", tree.root_node().to_sexp());
        }
    }

    #[test]
    fn interrupted_parses_resume_across_cloned_jobs_and_keep_full_context() {
        let text =
            format!("/*\n{}*/\n{}", "fn hidden() {}\n".repeat(3_000), "fn visible() { let x = 1; }\n".repeat(3_000));
        let rope = Rope::from_str(&text);
        let mut state = State::new(Language::Rust);
        for _ in 0..500 {
            let mut job = state.clone();
            let parsing = job.snapshot.is_none();
            let calls = std::cell::Cell::new(0);
            job.prepare(
                &rope,
                2_000..2_001,
                None,
                || {
                    calls.set(calls.get() + 1);
                    parsing && calls.get() > 3
                },
                crate::highlight::scope_style,
            );
            state = job;
            if state.runs(&rope, 2_000).is_some() {
                break;
            }
        }
        let runs = state.runs(&rope, 2_000).expect("bounded jobs eventually finish");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].1, crate::highlight::scope_style("comment").0);
        assert!(!state.requested);
    }

    #[test]
    fn interrupted_parse_is_discarded_for_replacement_text_and_edits() {
        let rope = Rope::from_str(&"fn old() { let x = 1; }\n".repeat(3_000));
        let mut state = State::new(Language::Rust);
        let calls = std::cell::Cell::new(0);
        state.prepare(
            &rope,
            0..1,
            None,
            || {
                calls.set(calls.get() + 1);
                calls.get() > 3
            },
            crate::highlight::scope_style,
        );
        assert!(state.snapshot.is_none());
        let pending = state.pending.as_ref().unwrap().clone();
        let replacement = Rope::from_str("// replacement 🦀\n");
        prepare(&mut state, &replacement, 0..1);
        assert!(state.snapshot.as_ref().unwrap().rope.is_instance(&replacement));
        assert_eq!(state.runs(&replacement, 0).unwrap().len(), 1);
        assert!(pending.lock().unwrap().result.is_none());
        state.pending = Some(pending);
        state.invalidate();
        assert!(state.pending.is_none(), "an edit cannot resume the old parser");
    }

    #[test]
    fn giant_documents_keep_syntect_without_allocating_a_syntax_tree() {
        let rope = Rope::from_str(&format!("//{}", "x".repeat(16 * 1024 * 1024)));
        let mut state = State::new(Language::Rust);
        prepare(&mut state, &rope, 0..1);
        assert!(state.snapshot.is_none(), "giant files retain the bounded syntect path");
        assert!(state.fold_regions(&rope).is_none(), "giant files retain indentation folding");
        assert!(!state.requested, "the paint loop does not queue the same unsupported parse");
    }

    #[test]
    fn incremental_reparse_reuses_untouched_functions() {
        let mut rope = Rope::from_str("fn first() { 1; }\nfn untouched() { 2; }\n");
        let mut state = State::new(Language::Rust);
        prepare(&mut state, &rope, 0..2);
        let second = |tree: &Tree| tree.root_node().named_child(1).unwrap().id();
        let old = state.snapshot.as_ref().unwrap().tree.clone();
        let untouched = second(&old);
        let at = rope.to_string().find('1').unwrap();
        rope.insert(at, "2");
        state.invalidate();
        state.text_changed(&[Change { start: at, removed: 0, inserted: 1, lines: (0, 0) }]);
        prepare(&mut state, &rope, 0..2);
        assert_eq!(second(&state.snapshot.as_ref().unwrap().tree), untouched);
    }

    #[test]
    fn color_cache_is_bounded_and_cancellation_leaves_a_retry_pending() {
        let rope = Rope::from_str(&"let x = 1;\n".repeat(MAX_COLOR_LINES + 10));
        let mut state = State::new(Language::Rust);
        state.prepare(&rope, 0..rope.len_lines(), None, || true, crate::highlight::scope_style);
        assert!(state.snapshot.is_none() && state.requested);
        prepare(&mut state, &rope, 0..rope.len_lines());
        assert_eq!(state.colors.len(), MAX_COLOR_LINES);
        assert!(!state.requested);
    }

    #[test]
    fn points_count_bytes_from_the_row_start_for_every_kind_of_char() {
        for text in [
            "plain ascii\nsecond\n",
            "café naïve\nÅngström",
            // combining marks are chars of their own, two bytes each here
            "e\u{301}cole n\u{303}o\n",
            // outside the basic plane: four bytes
            "😀 grin 𝄞 clef\n🦀\n",
            "\tindent\t\tcolumn\n\t\n",
            "日本語\tテキスト\n한국어 中文\n",
            // tree-sitter rows end at \n: a \r is the last byte of its row
            "crlf\r\nline\r\n",
            "\n\nempty lines around\n\n",
            "",
        ] {
            let rope = Rope::from_str(text);
            let chars: Vec<char> = text.chars().collect();
            for o in 0..=chars.len() {
                let before: String = chars[..o].iter().collect();
                let row_start = before.rfind('\n').map_or(0, |i| i + 1);
                let want = Point::new(before.matches('\n').count(), before.len() - row_start);
                assert_eq!(byte(&rope, CharOffset(o)), ByteOffset(before.len()), "{text:?} {o}");
                assert_eq!(point(&rope, CharOffset(o)), want, "{text:?} {o}");
            }
        }
    }

    #[test]
    fn sequential_unicode_edits_preserve_the_unchanged_tree_ranges() {
        let old = Rope::from_str("αβ\ne\u{301}\tcafé 🦀 𝄞\r\nlast\n");
        for first in 0..old.len_chars() {
            for second in 0..old.len_chars() {
                let mut new = old.clone();
                let mut changes = Vec::new();
                for (start, removed, text) in [(first, 1, "雪\n"), (second, 2, "😀\t")] {
                    new.remove(start..start + removed);
                    new.insert(start, text);
                    changes.push(Change { start, removed, inserted: text.chars().count(), lines: (0, 0) });
                }
                let edit = edit(&old, &new, &changes).unwrap();
                assert_eq!(old.byte_slice(..edit.start_byte), new.byte_slice(..edit.start_byte));
                assert_eq!(old.byte_slice(edit.old_end_byte..), new.byte_slice(edit.new_end_byte..));
                let at = |rope: &Rope, byte: usize| point(rope, CharOffset(rope.byte_to_char(byte)));
                assert_eq!(edit.start_position, at(&old, edit.start_byte));
                assert_eq!(edit.old_end_position, at(&old, edit.old_end_byte));
                assert_eq!(edit.new_end_position, at(&new, edit.new_end_byte));
            }
        }
    }

    #[test]
    fn builtin_capture_scopes_ignore_the_prefix_fallback_that_would_change_ruby() {
        let mut changed = Vec::new();
        for language in Language::ALL {
            let compiled = compiled(language);
            for (capture, scope) in compiled.query.capture_names().iter().zip(&compiled.scopes) {
                assert_eq!(*scope, capture_scope(capture, false), "{language:?} {capture}");
                if capture_scope(capture, true) != capture_scope(capture, false) {
                    changed.push(format!("{language:?} {capture}"));
                }
            }
        }
        changed.sort();
        changed.dedup();
        assert_eq!(changed, ["Ruby string.special.regex", "Ruby string.special.symbol"]);
        const { assert!(!BUILTIN_PREFIXES, "only runtime grammars take the prefix fallback") };
        assert_eq!(capture_scope("keyword.function", true), "keyword.control");
        assert_eq!(capture_scope("punctuation.bracket.round", true), "punctuation.section");
        assert_eq!(capture_scope("punctuation.bracket.round", false), "");
        assert_eq!(capture_scope("variable.parameter.builtin", true), "variable.parameter");
        assert_eq!(capture_scope("nonsense.thing", true), "");
        // names nvim-treesitter has since renamed, for runtime grammars only
        for (old, scope) in [
            ("field", "variable.member"),
            ("parameter", "variable.parameter"),
            ("parameter.reference", "variable.parameter"),
            ("namespace", "entity.name.namespace"),
            ("method.call", "entity.name.function"),
            ("type.qualifier", "keyword.control"),
            ("symbol", "string.special"),
        ] {
            assert_eq!(capture_scope(old, true), scope, "{old}");
        }
        assert_eq!((capture_scope("field", false), capture_scope("fieldname", true)), ("", ""));
    }

    #[test]
    fn every_capture_of_the_bundled_runtime_grammars_has_a_scope() {
        let registry = crate::registry::registry();
        for id in ["pony", "nim"] {
            let grammar = Grammar::of(&registry, registry.get(id).unwrap()).unwrap();
            let compiled = grammar.compiled().unwrap_or_else(|| panic!("{id} compiles"));
            let unstyled: Vec<_> = (compiled.query.capture_names().iter().zip(&compiled.scopes))
                .filter(|(capture, scope)| scope.is_empty() && !capture.starts_with('_'))
                .map(|(capture, _)| *capture)
                .collect();
            assert!(unstyled.is_empty(), "{id}: {unstyled:?}");
        }
        let nim = Grammar::of(&registry, registry.get("nim").unwrap()).unwrap();
        let compiled = nim.compiled().unwrap();
        let scope = |name| compiled.scopes[compiled.query.capture_index_for_name(name).unwrap() as usize];
        assert_eq!((scope("field"), scope("parameter")), ("variable.member", "variable.parameter"));
    }

    #[test]
    fn every_builtin_a_manifest_can_name_is_compiled_in() {
        for name in BUILTIN_GRAMMARS {
            assert!(builtin_language(name).is_some(), "{name}");
            assert!(Language::ALL.iter().any(|&l| grammar_name(l) == name), "{name}");
        }
        for name in BUILTIN_QUERIES {
            let query = builtin_query(name).unwrap_or_else(|| panic!("{name}"));
            let grammar = builtin_language(name.split('/').next().unwrap()).unwrap();
            Query::new(&grammar, query).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for language in Language::ALL {
            assert!(BUILTIN_GRAMMARS.contains(&grammar_name(language)), "{language:?}");
            let grammar = Grammar::builtin(language);
            assert!(Arc::ptr_eq(&grammar, &Grammar::builtin(language)), "shared by every buffer");
            assert_eq!((grammar.name(), grammar.scope()), (name(language), scope(language)));
            assert!(grammar.language().is_none() && !grammar.failed());
        }
    }

    /// `text` parsed with the bundled Pony grammar, `|` marking each caret (taken out).
    pub(super) fn pony(text: &str) -> (State, Rope, Vec<usize>) {
        let registry = crate::registry::registry();
        let grammar = Grammar::of(&registry, registry.get("pony").unwrap()).unwrap();
        let carets = text.split('|').scan(0, |at, piece| {
            *at += piece.chars().count();
            Some(*at)
        });
        let carets: Vec<usize> = carets.collect::<Vec<_>>().split_last().unwrap().1.to_vec();
        let rope = Rope::from_str(&text.replace('|', ""));
        let mut state = State::with_grammar(grammar);
        prepare(&mut state, &rope, 0..rope.len_lines());
        (state, rope, carets)
    }

    #[test]
    fn pony_indents_scm_indents_the_line_enter_starts() {
        for (text, levels) in [
            ("actor Main\n  let x: U32 = 0|\n\n  be go() =>\n    x\n", 1),
            ("actor Main\n  new create(env: Env) =>|\n    None\n", 2),
            ("actor Main\n  new create(env: Env) =>|   \n    None\n", 2),
            ("actor Main\n  be go() =>\n    while true do|\n      None\n    end\n", 3),
            ("actor Main\n  be go() =>\n    while true do\n      None\n    end|\n    None\n", 2),
            // the text after the caret starts the line: `end` closes the block
            ("actor Main\n  be go() =>\n    while true do\n      None |end\n    None\n", 2),
            // a trailing comment is passed over; blank lines are looked past
            ("actor Main\n  be go() => // start|\n    None\n", 2),
            ("actor Main\n  new create() =>\n\n|\n    None\n", 2),
            ("actor Main\n  let s: String = \"café 雪\"|\n  be go() => None\n", 1),
            ("actor Main\r\n  let s: String = \"café 雪\"|\r\n  be go() =>|\r\n    None\r\n", 1),
            ("primitive P\n  fun f(): String =>\n    \"雪\"|\n  fun g() => None\n", 2),
        ] {
            let (state, rope, carets) = pony(text);
            let indents = state.indents(&rope).expect("current tree and indents.scm");
            let got = carets.iter().map(|&at| indents.levels(at, at, false).unwrap()).collect::<Vec<_>>();
            let want = if text.contains("\r\n") { vec![levels, 2] } else { vec![levels] };
            assert_eq!(got, want, "{text:?}");
        }
        // a bracket pushed down: the empty line in it, then its own line (one level a row)
        let (state, rope, carets) = pony("primitive P\n  fun f(): Array[U8] => [|]\n");
        let indents = state.indents(&rope).unwrap();
        assert_eq!(
            (indents.levels(carets[0], carets[0], true), indents.levels(carets[0], carets[0], false)),
            (Some(2), Some(1))
        );
        // code still being written at the end of the file is an error the tree places nowhere
        for text in ["actor Main\n  new create(env: Env) =>|\n", "actor Main\n  be go() =>\n    while true do\n    |\n"]
        {
            let (state, rope, carets) = pony(text);
            assert_eq!(state.indents(&rope).unwrap().levels(carets[0], carets[0], false), None, "{text:?}");
        }
        let (state, rope, carets) = pony("actor Main\n  new create() =>\n    if x then|\n  be go() => None\n");
        assert_eq!(state.indents(&rope).unwrap().levels(carets[0], carets[0], false), Some(3), "an error in a method");
        // a selection Enter replaces: the text after it moves down
        let (state, rope, carets) = pony("actor Main\n  be go() =>\n    while true do\n      None|  x  |end\n");
        assert_eq!(state.indents(&rope).unwrap().levels(carets[0], carets[1], false), Some(2));
    }

    #[test]
    fn enter_indents_only_from_a_current_tree_with_indents_scm() {
        let (mut state, rope, _) = pony("actor Main\n  be go() => None\n");
        assert!(state.indents(&rope).is_some());
        assert!(state.indents(&Rope::from_str(&rope.to_string())).is_none(), "another text");
        let mut edited = rope.clone();
        edited.insert(0, "\n");
        assert!(state.clone().indents(&edited).is_none(), "an edit the tree has not seen");
        state.invalidate();
        assert!(state.indents(&rope).is_none(), "an invalidated tree");
        let rope = Rope::from_str("fn main() {\n}\n");
        let mut rust = State::new(Language::Rust);
        prepare(&mut rust, &rope, 0..rope.len_lines());
        assert!(rust.fold_regions(&rope).is_some() && rust.indents(&rope).is_none(), "no indents.scm");
        assert!(State::new(Language::Rust).indents(&rope).is_none(), "not parsed");
    }
}
