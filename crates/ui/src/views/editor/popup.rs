//! The caret-anchored popup, and the completion list drawn in it: typing a word or `ctrl-space`
//! opens the list, typing filters it, anything else closes it.

use super::*;
use crate::chrome::ui as kit;
use crate::languages::complete::{self, Candidate, Insert, Kind, Signals};
use crate::languages::highlight::Class;
use gpui::{anchored, deferred, point, AnyElement, Corner, FontWeight, ScrollWheelEvent, Size};
use ropey::Rope;
use std::collections::HashSet;
use std::path::Path;

/// Rows the list shows at once, and their height.
pub(super) const ROWS: usize = 12;
pub(super) const ROW_H: f32 = 22.;
/// The box's padding, and how far left of the text being completed the box starts so that the
/// labels line up with it (padding, border, a row's inset, a badge and the gap after it).
const PAD: f32 = 4.;
const INSET: f32 = PAD + 1. + 6. + 16. + 6.;
/// Lines either side of the caret whose words are on offer.
const NEAR_LINES: usize = 5_000;
/// The longest word, or part of a path, completed: a longer run offers nothing.
const MAX_PREFIX: usize = 100;

pub(crate) struct SignaturePending {
    pub context: Option<(Arc<serde_json::Value>, usize)>,
    origin: Option<SignatureOrigin>,
}

struct SignatureOrigin {
    source: DefinitionSource,
    viewport: Size<Pixels>,
    caret: usize,
    selection_generation: u64,
    selections: Vec<Selection>,
    primary: usize,
}

pub(crate) struct SignaturePopup {
    pub help: orlo_syntax::signature::SignatureHelp,
    source: DefinitionSource,
    viewport: Size<Pixels>,
    caret: usize,
    selection_generation: u64,
    selections: Vec<Selection>,
    primary: usize,
    cancelled: Arc<AtomicBool>,
}

impl Editor {
    pub(crate) fn signature_pending_context(context: Option<(Arc<serde_json::Value>, usize)>) -> SignaturePending {
        SignaturePending { context, origin: None }
    }

    pub(crate) fn begin_signature(&mut self, cancel: Arc<AtomicBool>, window: &Window, cx: &mut Context<Self>) {
        self.close_snippet_choices(cx);
        self.signature_help = None;
        let origin = SignatureOrigin {
            source: self.definition_source(cx),
            viewport: window.viewport_size(),
            caret: self.state.cursor(),
            selection_generation: self.read_state(cx).selection_generation(),
            selections: self.state.selections.clone(),
            primary: self.state.primary(),
        };
        self.signature_inflight = Some(cancel);
        self.signature_pending.get_or_insert_with(|| Self::signature_pending_context(None)).origin = Some(origin);
    }

    pub(crate) fn dismiss_signature(&mut self) {
        self.signature_help = None;
        self.signature_pending = None;
        if let Some(cancel) = self.signature_inflight.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn finish_signature(&mut self, cancel: &Arc<AtomicBool>) {
        if self.signature_inflight.as_ref().is_some_and(|current| Arc::ptr_eq(current, cancel)) {
            self.dismiss_signature();
        }
    }

    pub(crate) fn validate_signature_context(&mut self, window: &Window, cx: &App) {
        let invalid = self.signature_help.as_ref().is_some_and(|signature| {
            signature.cancelled.load(Ordering::Relaxed)
                || !self.signature_allowed(window)
                || signature.viewport != window.viewport_size()
                || signature.source != self.definition_source(cx)
                || signature.caret != self.state.cursor()
                || signature.selection_generation != self.read_state(cx).selection_generation()
                || signature.selections != self.state.selections
                || signature.primary != self.state.primary()
        }) || self.signature_inflight.as_ref().is_some_and(|cancel| cancel.load(Ordering::Relaxed))
            || self.signature_pending.as_ref().and_then(|p| p.origin.as_ref()).is_some_and(|origin| {
                !self.signature_allowed(window)
                    || origin.viewport != window.viewport_size()
                    || origin.source != self.definition_source(cx)
                    || origin.caret != self.state.cursor()
                    || origin.selection_generation != self.read_state(cx).selection_generation()
                    || origin.selections != self.state.selections
                    || origin.primary != self.state.primary()
            });
        if invalid {
            self.dismiss_signature();
        }
    }

    pub(crate) fn signature_allowed(&self, window: &Window) -> bool {
        self.marked.is_empty()
            && !self.state.selections.iter().any(Selection::has_virtual)
            && self.focus_handle.is_focused(window)
            && window.is_window_active()
    }

    pub(crate) fn show_signature(
        &mut self,
        help: orlo_syntax::signature::SignatureHelp,
        cancelled: Arc<AtomicBool>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.close_snippet_choices(cx);
        self.close_completion(cx);
        self.signature_pending = None;
        self.signature_help = Some(SignaturePopup {
            help,
            source: self.definition_source(cx),
            viewport: window.viewport_size(),
            caret: self.state.cursor(),
            selection_generation: self.read_state(cx).selection_generation(),
            selections: self.state.selections.clone(),
            primary: self.state.primary(),
            cancelled,
        });
        cx.notify();
    }

    pub(crate) fn render_signature(&mut self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        self.validate_signature_context(window, cx);
        let signature = self.signature_help.as_ref()?;
        let help = &signature.help.signatures[signature.help.active];
        let style = gpui::HighlightStyle {
            color: Some(ui(theme::ACCENT)),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let text =
            gpui::StyledText::new(help.label.clone()).with_highlights(help.parameter.clone().map(|r| (r, style)));
        let doc_lines = help.documentation.as_ref().map_or(0, |d| d.chars().count() / 60 + 1);
        let extra = usize::from(signature.help.signatures.len() > 1);
        let height = px(16. + 18. * (help.label.chars().count() / 60 + 1 + doc_lines + extra).min(8) as f32);
        let mut body = div().id("signature-help").max_w(px(560.)).max_h(px(144.)).overflow_y_scroll().p_2().child(text);
        if signature.help.signatures.len() > 1 {
            body = body.child(format!("{} / {}  (↑/↓)", signature.help.active + 1, signature.help.signatures.len()));
        }
        if let Some(doc) = &help.documentation {
            body = body.child(doc.clone());
        }
        self.popup(self.state.cursor(), height, px(8.), window, body, cx)
    }
}

#[derive(Clone)]
pub(crate) struct DefinitionPreview {
    pub place: crate::chrome::quick_panel::Place,
    pub path: String,
    pub key: crate::languages::index::EntryKey,
    pub symbol: crate::languages::index::IndexedSymbol,
}

pub(crate) struct DefinitionLine {
    target: usize,
    text: String,
    location: bool,
}

pub(crate) fn definition_lines(definitions: &[DefinitionPreview]) -> Vec<DefinitionLine> {
    let mut lines = Vec::new();
    for (target, d) in definitions.iter().enumerate() {
        let path = format!("{}:{}", d.path, d.symbol.line + 1);
        let doc = d.symbol.doc.as_deref().unwrap_or("No documentation comment found");
        for (text, location) in [(path.as_str(), true), (d.symbol.signature.as_str(), false), (doc, false)] {
            for line in text.lines() {
                let chars: Vec<_> = line.chars().collect();
                let mut start = 0;
                for end in
                    crate::editing::display::wrap_breaks(line, 60, 4).into_iter().chain(std::iter::once(chars.len()))
                {
                    lines.push(DefinitionLine { target, text: chars[start..end].iter().collect(), location });
                    start = end;
                }
            }
        }
        lines.push(DefinitionLine { target, text: String::new(), location: false });
    }
    lines
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DefinitionSource {
    buffer: u64,
    version: u64,
    path: Option<PathBuf>,
    context: CompletionContext,
}

pub(crate) struct DefinitionHover {
    pub source: DefinitionSource,
    pub word: Range<usize>,
    pub version: u64,
    pub requested: bool,
    pub definitions: Arc<Vec<DefinitionPreview>>,
    pub lines: Arc<Vec<DefinitionLine>>,
    pub viewport: Size<Pixels>,
    #[cfg(test)]
    pub rendered_rows: usize,
    pub over_word: bool,
    pub over_popup: bool,
    pub task: Option<Task<()>>,
    pub _keys: Subscription,
}

type CompletionContext = (String, Option<(gpui::EntityId, u64)>);

/// The completion list while it is open.
pub(crate) struct Completion {
    context: CompletionContext,
    /// Where the text being completed starts (chars), the word before the caret or the last part
    /// of a path in a string, and what is typed of it.
    start: usize,
    prefix: String,
    /// A path before the caret (`./src/ma`): where it starts, where its last part starts, and its
    /// folder (`./src/`).
    path: Option<(usize, usize, String)>,
    /// The path's last part is what is completed: the path is in a string.
    in_path: bool,
    /// The buffer version and the caret the list was last brought up to date with.
    version: u64,
    caret: usize,
    /// Comment, string or code where the text starts, once its line is coloured: the line just
    /// typed on has no colours until the next frame, and would read as code.
    class: Option<Class>,
    /// What is on offer, once gathered on another thread, and what the prefix matches of it,
    /// best first.
    all: Option<Vec<Candidate>>,
    signals: Signals,
    shown: Vec<Candidate>,
    /// The prefix `shown` was ranked for: typing on only narrows it.
    ranked: Option<String>,
    selected: usize,
    /// The first row in view, and the wheel's scroll toward the next row (pixels).
    first: usize,
    wheel: f32,
    /// The window's size when it opened: a resize closes it.
    viewport: Size<Pixels>,
    /// The words being gathered or the folder being read; dropped with the list.
    _task: Option<Task<()>>,
    /// The language server's items, once it answered an explicit request. Until then, or until
    /// the text changes, the list waits for them: it stays open, hidden while nothing matches.
    server: Vec<Candidate>,
    waiting: bool,
    /// The edits since the server's items came: committing one maps its edits through them.
    since: Vec<Change>,
    /// The snippets for the scope where the word starts, once its line is coloured.
    snippets: Vec<Candidate>,
}

impl Completion {
    fn visible(&self) -> bool {
        !self.shown.is_empty()
    }

    /// Where the class is read: at the path, else at the word.
    fn class_at(&self) -> usize {
        self.path.as_ref().map_or(self.start, |p| p.0)
    }

    /// Select row `i`, keeping it in view.
    fn select(&mut self, i: usize) {
        self.selected = i;
        self.first = self.first.clamp(i.saturating_sub(ROWS - 1), i);
    }

    /// Whether `ch` can be part of what is completed.
    fn takes(&self, ch: char) -> bool {
        match self.in_path {
            true => !(ch == '/' || ch.is_whitespace() || matches!(ch, '"' | '\'' | '`')),
            false => EditorState::is_word_char(ch),
        }
    }

    /// Edits move where the text being completed starts.
    pub(crate) fn map(&mut self, changes: &[Change]) {
        self.since.extend_from_slice(changes);
        self.start = crate::editing::text::map_offset(self.start, changes);
        if let Some(p) = &mut self.path {
            p.0 = crate::editing::text::map_offset(p.0, changes);
            p.1 = crate::editing::text::map_offset(p.1, changes);
        }
    }

    /// Rank what is on offer for the prefix, once it and the class are in (typing on narrows what
    /// was shown); false when that leaves nothing.
    fn rank(&mut self, s: &Document) -> bool {
        let (Some(all), Some(class)) = (&self.all, self.class) else {
            return true;
        };
        let narrows = (self.ranked.as_ref()).is_some_and(|p| self.prefix.starts_with(p.as_str()));
        // the server's first, then the snippets: a label offered twice is the first's
        let base = match narrows {
            true => std::mem::take(&mut self.shown),
            false => (self.server.iter().chain(&self.snippets).chain(all)).cloned().collect(),
        };
        let (before, receiver) = complete::receiver_before(s.rope.slice(..), self.start);
        let place = complete::Context {
            prefix: self.prefix.clone(),
            before,
            after_paren: self.caret < s.len() && s.rope.char(self.caret) == '(',
            class,
            receiver,
        };
        self.shown = complete::rank(base, &place, &self.signals);
        self.ranked = Some(self.prefix.clone());
        (self.selected, self.first) = (0, 0);
        !self.shown.is_empty()
    }
}

/// A path typed before `caret` (`./src/ma`, `~/Doc`, `/usr/lo`, `.gi`): where it starts, where
/// its last part starts, and its folder (`./src/`).
fn path_before(rope: &Rope, caret: usize) -> Option<(usize, usize, String)> {
    let n = (rope.chars_at(caret).reversed().take(1024))
        .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '`'))
        .count();
    let token = rope.slice(caret - n..caret).to_string();
    if n == 1024 || !(token.contains('/') || token.starts_with(['.', '~'])) {
        return None;
    }
    let folder = &token[..token.rfind('/').map_or(0, |i| i + 1)];
    Some((caret - n, caret - n + folder.chars().count(), folder.to_string()))
}

/// What the list's words are made of, off the UI thread.
struct WordJob {
    context: CompletionContext,
    rope: Rope,
    caret: usize,
    syntax: String,
    path: Option<PathBuf>,
    current: Option<crate::languages::index::EntryKey>,
    index: Option<crate::languages::index::SymbolSnapshot>,
    receiver: Option<String>,
    recent: Vec<String>,
    /// Other buffers of the syntax: their words read already (for their version), or their text.
    others: Vec<(Option<Arc<Vec<String>>>, Rope)>,
}

impl WordJob {
    /// Gather candidates and ranking signals; other views keep their versioned word lists.
    fn run(self) -> (Vec<Candidate>, Vec<Arc<Vec<String>>>, Signals) {
        let keywords = complete::keywords(&self.syntax);
        let mut all: Vec<Candidate> = keywords.iter().map(|k| Candidate::text(k.clone(), Kind::Keyword)).collect();
        let read: Vec<Arc<Vec<String>>> = self
            .others
            .into_iter()
            .map(|(words, rope)| words.unwrap_or_else(|| Arc::new(complete::words(&rope))))
            .collect();
        let near = complete::buffer_words(&self.rope, self.caret, NEAR_LINES);
        let mut signals = Signals {
            local: near.iter().map(|(w, d, n)| (w.clone(), (*d, *n))).collect(),
            recent: self.recent,
            ..Signals::default()
        };
        let at = self.rope.char_to_line(self.caret);
        let last = self.rope.len_lines() - 1;
        let lines = (0..=NEAR_LINES)
            .flat_map(|d| at.checked_sub(d).into_iter().chain((d > 0 && at + d <= last).then_some(at + d)));
        let mut members = complete::member_counts(&self.rope, lines, complete::HARVEST_BYTES, || false);
        let mut entries = self.index.map_or_else(Vec::new, |i| i.entries());
        entries.retain(|e| complete::same_syntax(&e.syntax, &self.syntax));
        let locality = |e: &crate::languages::index::SymbolEntry| {
            if self.current.as_ref() == Some(&e.key) || (self.path.is_some() && e.path == self.path) {
                0
            } else if self
                .path
                .as_deref()
                .and_then(Path::parent)
                .is_some_and(|dir| e.path.as_deref().and_then(Path::parent) == Some(dir))
            {
                1
            } else {
                2
            }
        };
        entries.sort_by(|a, b| locality(a).cmp(&locality(b)).then_with(|| a.path.cmp(&b.path)));
        for entry in entries {
            let place = locality(&entry);
            let detail = match &entry.path {
                Some(p) => p.display().to_string(),
                None if place == 0 => "This buffer".to_string(),
                None => "Untitled buffer".to_string(),
            };
            signals.locality.insert(detail.clone(), place);
            for symbol in entry.symbols.iter() {
                all.push(Candidate {
                    label: symbol.name.clone(),
                    kind: Kind::Symbol(symbol.kind.clone()),
                    detail: detail.clone(),
                    insert: Insert::Text(symbol.name.clone()),
                });
            }
            if place == 0 {
                continue;
            }
            if entry.syntax == "Plain Text" {
                all.extend(entry.idents.keys().map(|word| Candidate::text(word.to_string(), Kind::Word)));
            }
            for (word, n) in entry.idents.iter() {
                let count = signals.project.entry(word.clone()).or_default();
                *count = count.saturating_add(*n);
            }
            for (member, n) in entry.members.iter() {
                let count = members.entry(member.clone()).or_default();
                *count = count.saturating_add(*n);
            }
        }
        if let Some(receiver) = self.receiver {
            let prefix = format!("{receiver}.");
            signals.members = members
                .into_iter()
                .filter_map(|(name, count)| name.strip_prefix(&prefix).map(|s| (s.to_string(), count)))
                .collect();
        }
        let mut seen = HashSet::new();
        for w in near.into_iter().map(|w| w.0).chain(read.iter().flat_map(|w| w.iter().cloned())) {
            if seen.insert(w.clone()) {
                all.push(Candidate::text(w, Kind::Word));
            }
        }
        (all, read, signals)
    }
}

/// Other views read by a list, with buffer identities and versions to validate returned words.
type Readers = Vec<(WeakEntity<Editor>, u64, u64)>;

/// A kind's letter and colour (Goto Symbol's for a symbol).
fn badge(kind: &Kind) -> (char, u32) {
    match kind {
        Kind::Word => ('w', 0xa6acb9),
        Kind::Keyword => ('k', 0xc695c6),
        Kind::Path => ('p', 0x6699cc),
        Kind::Snippet => ('s', 0x99c794),
        Kind::Symbol(kind) => crate::languages::highlight::symbol_badge(kind),
    }
}

impl Editor {
    fn completion_context(&self, cx: &App) -> CompletionContext {
        let index = self
            .completion_index
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .map(|index| (index.entity_id(), index.read(cx).generation));
        (self.highlighter.syntax_name().to_string(), index)
    }

    /// Typed text: after a word char, with the word before the caret at least
    /// `auto_complete_min_chars` long, the list opens.
    pub(crate) fn typed(&mut self, text: &str, window: &Window, cx: &mut Context<Self>) {
        let word = text.chars().last().is_some_and(EditorState::is_word_char);
        if !word || !self.view.auto_complete || self.completion.is_some() {
            return;
        }
        if self.word_back(self.state.cursor(), cx) >= self.view.auto_complete_min_chars {
            self.open_completion(window, cx);
        }
    }

    /// The word chars before `caret`, counted up to one past MAX_PREFIX.
    fn word_back(&self, caret: usize, cx: &App) -> usize {
        let word = |c: &char| EditorState::is_word_char(*c);
        let back = self.doc(cx).rope.chars_at(caret).reversed();
        back.take(MAX_PREFIX + 1).take_while(word).count()
    }

    /// Why the list cannot open at the primary caret (LSP: Complete says it): a composition, a
    /// selection or a word too long to complete; `None` when it can.
    pub fn cannot_complete(&self, cx: &App) -> Option<String> {
        let sel = self.state.selections[self.state.primary()];
        let why = if !self.marked.is_empty() {
            "text is being composed".into()
        } else if sel.has_virtual() {
            "the caret is beyond the line end".into()
        } else if !sel.is_empty() {
            "text is selected".into()
        } else if self.word_back(sel.head, cx) > MAX_PREFIX {
            format!("the word before the caret is over {MAX_PREFIX} characters")
        } else {
            return None;
        };
        Some(format!("Completion unavailable: {why}"))
    }

    /// Open the list at the primary caret, on the word before it (empty for `ctrl-space`): what it
    /// offers is gathered on another thread.
    pub(crate) fn open_completion(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dismiss_signature();
        self.completion_epoch += 1;
        if self.single_line || self.cannot_complete(cx).is_some() {
            return;
        }
        self.close_snippet_choices(cx);
        let s = self.read_state(cx);
        let sel = s.view.selections[s.primary()];
        let (caret, start) = (sel.head, sel.head - self.word_back(sel.head, cx));
        self.completion = Some(Completion {
            context: self.completion_context(cx),
            start,
            prefix: s.doc.rope.slice(start..caret).to_string(),
            path: path_before(&s.doc.rope, caret),
            in_path: false,
            version: s.doc.version,
            caret,
            class: None,
            all: None,
            signals: Signals::default(),
            shown: Vec::new(),
            ranked: None,
            selected: 0,
            first: 0,
            wheel: 0.,
            viewport: window.viewport_size(),
            _task: None,
            server: Vec::new(),
            waiting: false,
            since: Vec::new(),
            snippets: Vec::new(),
        });
        self.gather_words(cx);
        self.resolve_class(window, cx);
        cx.notify();
    }

    /// The list is drawn, so its keys are bound (`completing`): not while composing, nor with its
    /// row off screen.
    pub(crate) fn completing(&self, cx: &App) -> bool {
        let shown = (self.completion.as_ref()).filter(|c| c.visible() && self.marked.is_empty());
        shown.is_some_and(|c| self.char_bounds(c.start, cx).is_some())
    }

    /// Bring the list up to the text and the caret: an edit that leaves a prefix to complete
    /// filters it (a path's folder is read again for a shorter part); a caret moved by anything
    /// else, a selection, or a prefix gone or emptied closes it.
    pub(crate) fn sync_completion(&mut self, cx: &mut Context<Self>) {
        if self.completion.as_ref().is_some_and(|c| c.context != self.completion_context(cx)) {
            self.close_completion(cx);
            return;
        }
        let Some(c) = &mut self.completion else {
            return;
        };
        let s = read_of(&self.document, &self.state, cx);
        let sel = s.view.selections[s.primary()];
        let (caret, same) = (sel.head, s.doc.version == c.version);
        let keep = sel.is_empty()
            && !sel.has_virtual()
            && match same {
                true => caret == c.caret,
                false => {
                    (c.start + 1..=c.start + MAX_PREFIX).contains(&caret)
                        && s.doc.rope.slice(c.start..caret).chars().all(|ch| c.takes(ch))
                }
            };
        if !keep {
            self.completion = None;
            return;
        }
        if same {
            return;
        }
        (c.version, c.caret) = (s.doc.version, caret);
        // the server answers for the text it was asked about
        let waited = std::mem::take(&mut c.waiting);
        let prefix = s.doc.rope.slice(c.start..caret).to_string();
        if prefix != c.prefix || waited {
            // the folder's entries were read for the part typed: a shorter one needs them again
            let shorter = c.in_path && !prefix.starts_with(c.prefix.as_str());
            c.prefix = prefix;
            match (c.path.as_ref()).filter(|_| shorter).map(|p| p.2.clone()) {
                Some(folder) => {
                    (c.all, c.ranked) = (None, None);
                    self.read_folder(&folder, cx);
                }
                None => self.rank_completion(cx),
            }
        }
    }

    /// Rank the list again; it closes when nothing is left to show and no server's items are to
    /// come.
    fn rank_completion(&mut self, cx: &App) {
        let doc = doc_of(&self.document, cx);
        if (self.completion.as_mut()).is_some_and(|c| !c.rank(doc) && !c.waiting) {
            self.completion = None;
        }
    }

    /// An explicit request (`ctrl-space`, LSP: Complete): the list, opened if it is not, waits for
    /// the language server's items. False when it cannot open.
    pub fn expect_server_items(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        self.sync_completion(cx);
        if self.completion.is_none() {
            self.open_completion(window, cx);
        }
        let Some(c) = &mut self.completion else {
            return false;
        };
        c.waiting = true;
        true
    }

    /// The language server's items for the open list, ranked with the rest; a commit maps
    /// an item's edits through the edits from now on.
    pub fn add_server_items(&mut self, items: Vec<Candidate>, cx: &mut Context<Self>) {
        let Some(c) = &mut self.completion else {
            return;
        };
        let on = c.shown.get(c.selected).map(|s| s.label.clone());
        (c.server, c.waiting, c.ranked) = (items, false, None);
        c.since.clear();
        self.rank_completion(cx);
        // the row the user is on stays selected, wherever the merge ranks it
        if let (Some(c), Some(on)) = (&mut self.completion, on) {
            if let Some(i) = c.shown.iter().position(|s| s.label == on) {
                c.select(i);
            }
        }
        cx.notify();
    }

    /// The language server's items are not coming (the request failed or was discarded): the
    /// list stops waiting, and closes if it has nothing to show, so no hidden list is left waiting.
    pub fn end_server_wait(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.completion.as_mut().filter(|c| c.waiting) else {
            return;
        };
        c.waiting = false;
        if !c.visible() {
            self.rank_completion(cx);
            cx.notify();
        }
    }

    /// Before a frame: focus loss, the window going inactive (gpui redraws it then) and a resize
    /// close the list; it catches up with the text, and with its line's colours.
    pub(crate) fn refresh_completion(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(c) = &self.completion else {
            return;
        };
        let focused = self.focus_handle.is_focused(window) && window.is_window_active();
        if !focused || window.viewport_size() != c.viewport {
            self.completion = None;
            return;
        }
        self.sync_completion(cx);
        self.resolve_class(window, cx);
    }

    /// The popup's owner closes it (a view losing the focus while it is not drawn, as a tab
    /// left for another).
    pub fn close_completion(&mut self, cx: &mut Context<Self>) {
        self.dismiss_signature();
        self.completion_epoch += 1;
        self.close_definition_hover(cx);
        if self.completion.take().is_some() {
            cx.notify();
        }
    }

    /// The list waits for its line's colours, and they are in: the next frame can show it.
    pub(crate) fn completion_coloured(&self, cx: &App) -> bool {
        let Some(c) = self.completion.as_ref().filter(|c| c.class.is_none()) else {
            return false;
        };
        let rope = &self.doc(cx).rope;
        self.highlighter.cached_scope_id(rope, c.class_at()).is_some()
    }

    /// The class where the text starts, from its line's colours once they are in, and the
    /// snippets for its scope. A path in a string is completed from its folder instead of the
    /// words.
    fn resolve_class(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(c) = self.completion.as_ref().filter(|c| c.class.is_none()) else {
            return;
        };
        let Some(id) = self.highlighter.cached_scope_id(&self.doc(cx).rope, c.class_at()) else {
            return;
        };
        let class = crate::languages::highlight::class(id);
        let path = c.path.clone().filter(|_| class == Class::String);
        let snippets = match path {
            Some(_) => Vec::new(),
            None => self.snippet_rows(id, window, cx),
        };
        let Some(c) = self.completion.as_mut() else {
            return;
        };
        (c.class, c.snippets) = (Some(class), snippets);
        match path {
            Some((_, part, folder)) => {
                c.prefix = doc_of(&self.document, cx).rope.slice(part..c.caret).to_string();
                (c.start, c.in_path, c.all, c.ranked) = (part, true, None, None);
                self.read_folder(&folder, cx);
            }
            None => self.rank_completion(cx),
        }
    }

    /// The keywords and words on offer, gathered on another thread: this buffer's near the caret
    /// and those of the other open buffers of its syntax, which their views keep for a version.
    fn gather_words(&mut self, cx: &mut Context<Self>) {
        let context = self.completion_context(cx);
        let task = cx.spawn(async move |this, cx| {
            let refresh = cx
                .update(|cx| {
                    let editor = this.upgrade()?;
                    let e = editor.read(cx);
                    let index = e.completion_index.as_ref()?.upgrade()?;
                    let source = (e.doc(cx).id(), e.doc(cx).version);
                    // an entry in this view's syntax is left to its debounce; with none, or one in
                    // another view's syntax, this view speaks for the buffer and it parses now
                    if index
                        .read(cx)
                        .buffer_entry(source.0)
                        .is_some_and(|entry| entry.syntax == e.highlighter.syntax_name())
                    {
                        return None;
                    }
                    Some((source, index.update(cx, |i, cx| i.ensure_fresh(&editor, cx))))
                })
                .ok()
                .flatten();
            let source = if let Some((source, refresh)) = refresh {
                refresh.await;
                Some(source)
            } else {
                None
            };
            let Ok(Some((job, views))) = this.update(cx, |e, cx| {
                if context != e.completion_context(cx) {
                    e.sync_completion(cx);
                    return None;
                }
                if source.is_some_and(|source| source != (e.doc(cx).id(), e.doc(cx).version)) {
                    e.close_completion(cx);
                    return None;
                }
                e.word_job(cx).filter(|(job, _)| job.context == context)
            }) else {
                return;
            };
            let context = job.context.clone();
            let (all, read, signals) = cx.background_spawn(async move { job.run() }).await;
            this.update(cx, |e, cx| {
                for ((view, buffer, version), words) in views.into_iter().zip(read) {
                    let keep = |v: &mut Editor, cx: &mut Context<Editor>| {
                        let doc = v.doc(cx);
                        if doc.id() == buffer && doc.version == version {
                            v.words_cache = Some((version, words));
                        }
                    };
                    view.update(cx, keep).ok();
                }
                e.publish_words(context, all, Some(signals), cx);
            })
            .ok();
        });
        if let Some(c) = &mut self.completion {
            c._task = Some(task);
        }
    }

    /// What is on offer for the list, gathered in `context` (with its ranking signals, or a
    /// folder's entries that keep the list's), unless the list or its context changed since.
    fn publish_words(
        &mut self,
        context: CompletionContext,
        all: Vec<Candidate>,
        signals: Option<Signals>,
        cx: &mut Context<Self>,
    ) {
        if context != self.completion_context(cx) {
            self.sync_completion(cx);
            return;
        }
        if let Some(c) = self.completion.as_mut().filter(|c| c.context == context) {
            c.all = Some(all);
            if let Some(signals) = signals {
                c.signals = signals;
            }
            self.rank_completion(cx);
            cx.notify();
        }
    }

    /// The job for the list's words, and the other views it reads words of, with their versions.
    fn word_job(&self, cx: &Context<Self>) -> Option<(WordJob, Readers)> {
        let c = self.completion.as_ref()?;
        let syntax = self.highlighter.syntax_name().to_string();
        let mut buffers = vec![self.doc(cx).id()];
        let (mut views, mut others) = (Vec::new(), Vec::new());
        let open = self.open_views.as_ref().map_or_else(Vec::new, |f| f(cx));
        // this view is the one being updated: it is not read
        for view in open.iter().filter(|v| v.entity_id() != cx.entity_id()) {
            let e = view.read(cx);
            let id = e.doc(cx).id();
            if e.single_line || buffers.contains(&id) || !complete::same_syntax(e.highlighter.syntax_name(), &syntax) {
                continue;
            }
            buffers.push(id);
            let version = e.doc(cx).version;
            let words = (e.words_cache.as_ref()).filter(|w| w.0 == version).map(|w| w.1.clone());
            views.push((view.downgrade(), id, version));
            others.push((words, e.doc(cx).rope.clone()));
        }
        let rope = self.doc(cx).rope.clone();
        let index = self.completion_index.as_ref().and_then(WeakEntity::upgrade);
        let current = index.as_ref().and_then(|i| i.read(cx).buffer_entry(self.doc(cx).id())).map(|e| e.key.clone());
        let job = WordJob {
            context: c.context.clone(),
            path: self.doc(cx).path.clone(),
            current,
            index: index.map(|i| i.read(cx).snapshot()),
            receiver: complete::receiver_before(rope.slice(..), c.start).1,
            recent: self.completion_recent.borrow().iter().cloned().collect(),
            rope,
            caret: c.caret,
            syntax,
            others,
        };
        Some((job, views))
    }

    /// The folder of a path being completed (relative to the file's folder, `~` the home folder),
    /// read on another thread: its entries are what is on offer.
    fn read_folder(&mut self, folder: &str, cx: &mut Context<Self>) {
        let base = self.doc(cx).path.as_deref().and_then(Path::parent);
        let dir = match folder {
            f if f.starts_with("~/") => Some(crate::searching::search::untilde(f)),
            f if f.starts_with('/') => Some(PathBuf::from(f)),
            f if f.starts_with('~') => None,
            f => base.map(|b| b.join(f)),
        };
        let Some(c) = &mut self.completion else {
            return;
        };
        let prefix = c.prefix.clone();
        let context = c.context.clone();
        c._task = Some(cx.spawn(async move |this, cx| {
            let entries = match dir {
                Some(dir) => {
                    let read = async move { complete::path_entries(&dir, &prefix) };
                    cx.background_spawn(read).await
                }
                None => Vec::new(),
            };
            this.update(cx, |e, cx| e.publish_words(context, entries, None, cx)).ok();
        }));
    }

    /// Commit row `row` (the selected one for `None`): its text replaces the prefix at every caret
    /// where that prefix is typed, as one undo step, and a folder opens the list on its
    /// entries. False when the list was not showing.
    fn commit_completion(&mut self, row: Option<usize>, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled {
            return false;
        }
        self.sync_completion(cx);
        let Some(mut c) = self.completion.take() else {
            return false;
        };
        let i = row.unwrap_or(c.selected);
        if i >= c.shown.len() {
            return false;
        }
        let picked = c.shown.swap_remove(i);
        let mut recent = self.completion_recent.borrow_mut();
        recent.push_front(picked.label.clone());
        recent.truncate(50);
        drop(recent);
        let text = match picked.insert {
            Insert::Text(text) => text,
            // the workspace resolves the server's item and applies it
            Insert::Server(item) => {
                cx.emit(EditorEvent::CommitServerItem(item, c.since));
                return true;
            }
            Insert::Snippet(path) => {
                self.commit_snippet(&path, &c.prefix, cx);
                return true;
            }
        };
        // a keyword's `#`, `@` or `$` typed before the word goes with it
        let rope = &self.doc(cx).rope;
        let sigil = (text.chars().next())
            .filter(|&ch| !EditorState::is_word_char(ch) && c.start > 0 && rope.char(c.start - 1) == ch);
        let prefix = match sigil {
            Some(ch) => format!("{ch}{}", c.prefix),
            None => c.prefix,
        };
        // a macro records the typed prefix, not this commit (the completion actions are
        // not captured); record it as text if macros that complete are wanted
        self.edit_text(cx, |s| s.complete(&prefix, &text));
        if picked.kind == Kind::Path && text.ends_with('/') {
            self.open_completion(window, cx);
        }
        true
    }

    /// Move the selection `by` rows, keeping it in view; false when the list was not showing.
    fn step_completion(&mut self, by: isize, cx: &mut Context<Self>) -> bool {
        self.sync_completion(cx);
        let Some(c) = self.completion.as_mut().filter(|c| c.visible()) else {
            return false;
        };
        let last = c.shown.len() as isize - 1;
        c.select((c.selected as isize + by).clamp(0, last) as usize);
        true
    }

    /// `ctrl-space`: open the list at once, with any prefix (one showing stays as it is), and have
    /// the workspace ask the language server; a list that typing opened takes its items in too.
    pub(crate) fn auto_complete(&mut self, _: &AutoComplete, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_completion(cx);
        if !(self.completion.as_ref()).is_some_and(Completion::visible) {
            self.open_completion(window, cx);
        }
        if self.completion.is_some() {
            cx.emit(EditorEvent::ServerCompletions);
        }
    }

    pub(crate) fn completion_next(&mut self, _: &CompletionNext, window: &mut Window, cx: &mut Context<Self>) {
        self.step_or_pass(1, window, cx);
    }

    pub(crate) fn completion_prev(&mut self, _: &CompletionPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.step_or_pass(-1, window, cx);
    }

    /// The list's keys go on to what they do without it when it has closed since the frame that
    /// bound them.
    fn step_or_pass(&mut self, by: isize, window: &Window, cx: &mut Context<Self>) {
        self.refresh_snippet_choices(window, cx);
        if let Some(choice) = &mut self.snippet_choice {
            let last = choice.values.len() - 1;
            choice.selected = choice.selected.saturating_add_signed(by).min(last);
            choice.first = choice.first.clamp(choice.selected.saturating_sub(ROWS - 1), choice.selected);
            cx.notify();
            return;
        }
        if let Some(signature) = self.signature_help.as_mut() {
            let last = signature.help.signatures.len().saturating_sub(1) as isize;
            signature.help.active = (signature.help.active as isize + by).clamp(0, last) as usize;
            cx.notify();
            return;
        }
        match self.step_completion(by, cx) {
            true => cx.notify(),
            false => cx.propagate(),
        }
    }

    pub(crate) fn completion_commit(&mut self, _: &CompletionCommit, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commit_snippet_choice(None, window, cx) && !self.commit_completion(None, window, cx) {
            cx.propagate();
        }
    }

    pub(crate) fn completion_close(&mut self, a: &CompletionClose, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_snippet_choices(window, cx);
        if self.snippet_choice.is_some() {
            self.close_snippet_choices(cx);
            if a.fall_through {
                cx.propagate();
            }
            return;
        }
        self.completion_epoch += 1;
        if let Some(cancel) = self.signature_inflight.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.sync_completion(cx);
        let shown = self.signature_help.take().is_some()
            | self.signature_pending.take().is_some()
            | self.completion.take().is_some_and(|c| c.visible());
        if !shown || a.fall_through {
            cx.propagate();
        }
        cx.notify();
    }

    /// The wheel or trackpad over the list moves the rows in view, not the selection.
    fn scroll_completion(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &mut self.completion else {
            return;
        };
        // a trackpad's small steps add up to rows
        c.wheel -= f32::from(e.delta.pixel_delta(px(ROW_H)).y);
        let rows = (c.wheel / ROW_H).trunc();
        c.wheel -= rows * ROW_H;
        let last = c.shown.len().saturating_sub(ROWS) as isize;
        c.first = (c.first as isize + rows as isize).clamp(0, last) as usize;
        cx.notify();
    }

    /// The list in its popup, while it has something to show and no composition is under way.
    pub(crate) fn render_completion(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let list = self.completion.as_ref().filter(|_| self.completing(cx))?;
        let end = (list.first + ROWS).min(list.shown.len());
        let rows = (list.first..end).map(|i| self.completion_row(list, i, cx));
        let height = px(ROW_H * (end - list.first) as f32 + 2. * (PAD + 1.));
        let body = div().min_w(px(160.)).max_w(px(560.)).flex().flex_col();
        let body = body.on_scroll_wheel(cx.listener(Self::scroll_completion));
        self.popup(list.start, height, px(INSET), window, body.children(rows), cx)
    }

    fn completion_row(&self, list: &Completion, i: usize, cx: &Context<Self>) -> impl IntoElement {
        let item = &list.shown[i];
        let selected = i == list.selected;
        let (letter, colour) = badge(&item.kind);
        let (label_fg, detail_fg) = match selected {
            true => (theme::QP_LABEL_SELECTED, theme::QP_PATH_SELECTED),
            false => (theme::QP_LABEL, theme::QP_PATH),
        };
        let badge = div()
            .flex_none()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(kit::RADIUS))
            .bg(c(colour).opacity(0.18))
            .text_size(px(10.))
            .font_weight(FontWeight::BOLD)
            .text_color(c(colour))
            .child(letter.to_string());
        let label = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .text_color(ui(label_fg))
            .child(crate::chrome::quick_panel::matched(&list.prefix, &item.label));
        div()
            .id(("completion", i))
            .h(px(ROW_H))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(kit::RADIUS))
            .whitespace_nowrap()
            .when(selected, |d| d.bg(ui(theme::QP_SELECTED_BG)))
            .when(!selected, |d| d.hover(|s| s.bg(ui(theme::ELEMENT_HOVER).opacity(0.6))))
            .child(badge)
            .child(label)
            .when(!item.detail.is_empty(), |d| {
                d.child(
                    // a server's long signature leaves the label its room
                    div()
                        .flex_none()
                        .max_w(px(280.))
                        .truncate()
                        .pl(px(12.))
                        .text_size(px(11.))
                        .text_color(ui(detail_fg))
                        .child(item.detail.clone()),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |e, _, window, cx| {
                    e.commit_completion(Some(i), window, cx);
                }),
            )
    }

    /// `content` in a box at char `at`'s row as last painted (the IME's bounds), starting
    /// `inset` left of the char: below the row, or above it when there is no room below for
    /// `height` (the box's), and kept in the window. The box takes the mouse for what it covers,
    /// the gutter's buttons and the fold markers too, and never the keyboard focus.
    pub(crate) fn popup(
        &self,
        at: usize,
        height: Pixels,
        inset: Pixels,
        window: &Window,
        content: impl IntoElement,
        cx: &App,
    ) -> Option<AnyElement> {
        let row = self.char_bounds(at, cx)?;
        let fits_below = row.bottom() + height <= window.viewport_size().height;
        let (corner, y) = match !fits_below && row.top() >= height {
            true => (Corner::BottomLeft, row.top()),
            false => (Corner::TopLeft, row.bottom()),
        };
        let body = div()
            .occlude()
            .debug_selector(|| "popup".into())
            .p(px(PAD))
            .rounded(px(kit::OVERLAY_RADIUS))
            .border_1()
            .border_color(ui(theme::MENU_BORDER))
            .bg(ui(theme::OVERLAY_BG))
            .text_color(ui(theme::UI_FG))
            .shadow_lg()
            .cursor(CursorStyle::Arrow)
            .child(content);
        let place = anchored().anchor(corner).position(point(row.left() - inset, y));
        Some(deferred(place.snap_to_window().child(body)).into_any_element())
    }

    pub(crate) fn definition_source(&self, cx: &App) -> DefinitionSource {
        DefinitionSource {
            buffer: self.doc(cx).id(),
            version: self.doc(cx).version,
            path: self.doc(cx).path.clone(),
            context: self.completion_context(cx),
        }
    }

    /// Definitions may show: the view has the keys in the active window, and the settings allow.
    pub(super) fn definitions_allowed(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.is_focused(window)
            && window.is_window_active()
            && (cx.try_global::<crate::config::settings::Settings>()).is_none_or(|s| s.show_definitions)
    }

    pub fn close_definition_hover(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.definition_hover.take() {
            let content = (h.definitions, h.lines);
            cx.background_spawn(async move { drop(content) }).detach();
            cx.notify();
        }
    }

    pub(crate) fn show_definitions(
        &mut self,
        word: Range<usize>,
        version: u64,
        definitions: Vec<DefinitionPreview>,
        lines: Vec<DefinitionLine>,
        cx: &mut Context<Self>,
    ) {
        let source = self.definition_source(cx);
        let Some(h) = self.definition_hover.as_mut().filter(|h| {
            h.source == source
                && h.requested
                && h.word == word
                && h.version == version
                && doc_of(&self.document, cx).version == version
        }) else {
            return;
        };
        if definitions.is_empty() {
            self.close_definition_hover(cx);
            return;
        }
        let old = (
            std::mem::replace(&mut h.definitions, Arc::new(definitions)),
            std::mem::replace(&mut h.lines, Arc::new(lines)),
        );
        cx.background_spawn(async move { drop(old) }).detach();
        h.task = None;
        self.completion = None;
        cx.notify();
    }

    pub(crate) fn render_definitions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let h = self.definition_hover.as_ref()?;
        if h.source != self.definition_source(cx)
            || h.version != self.doc(cx).version
            || h.viewport != window.viewport_size()
            || !self.definitions_allowed(window, cx)
        {
            self.close_definition_hover(cx);
            return None;
        }
        if h.definitions.is_empty() {
            return None;
        }
        let height = (h.lines.len() as f32 * 18.).min((f32::from(window.viewport_size().height) - 60.).max(18.));
        let rows = gpui::uniform_list(
            "definition-lines",
            h.lines.len(),
            cx.processor(|e, range: Range<usize>, _, cx| {
                let Some(h) = &mut e.definition_hover else {
                    return Vec::new();
                };
                #[cfg(test)]
                {
                    h.rendered_rows = range.len();
                }
                range
                    .map(|i| {
                        let line = &h.lines[i];
                        let target = h.definitions[line.target].clone();
                        div()
                            .id(("definition", i))
                            .h(px(18.))
                            .px(px(6.))
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .when(line.location, |d| d.text_color(ui(theme::TEXT_MUTED)))
                            .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)))
                            .child(line.text.clone())
                            .on_click(cx.listener(move |e, event: &gpui::ClickEvent, _, cx| {
                                e.close_definition_hover(cx);
                                cx.emit(EditorEvent::OpenDefinition(target.clone(), event.modifiers().platform));
                            }))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .h(px(height))
        .w_full();
        let body = div()
            .id("definition-hover")
            .w(px(500.))
            .max_h(px((f32::from(window.viewport_size().height) - 20.).max(40.)))
            .flex()
            .flex_col()
            .text_size(px(12.))
            .when(h.definitions.len() > 1, |d| {
                d.child(div().px(px(6.)).child(format!("{} definitions with this name", h.definitions.len())))
            })
            .child(rows)
            .on_hover(cx.listener(|e, over: &bool, _, cx| {
                if let Some(h) = &mut e.definition_hover {
                    h.over_popup = *over;
                    if !over && !h.over_word {
                        e.close_definition_hover(cx);
                    }
                }
            }))
            .on_scroll_wheel(cx.listener(|e, _: &ScrollWheelEvent, _, cx| {
                e.close_definition_hover(cx);
                cx.stop_propagation();
            }));
        self.popup(h.word.start, px(height + 26.), px(0.), window, body, cx)
    }

    #[cfg(test)]
    pub(crate) fn definition_rendered_rows(&self) -> usize {
        self.definition_hover.as_ref().map_or(0, |h| h.rendered_rows)
    }

    #[cfg(test)]
    pub(crate) fn definition_text(&self) -> Option<String> {
        let h = self.definition_hover.as_ref().filter(|h| !h.definitions.is_empty())?;
        Some(
            h.definitions
                .iter()
                .map(|d| {
                    format!(
                        "{}:{}\n{}\n{}",
                        d.path,
                        d.symbol.line + 1,
                        d.symbol.signature,
                        d.symbol.doc.as_deref().unwrap_or("No documentation comment found")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Tests: the list as shown, its labels and selected row; `None` while it is not showing.
    #[cfg(test)]
    pub fn completion_list(&self) -> Option<(Vec<String>, usize)> {
        let c = self.completion.as_ref().filter(|c| c.visible())?;
        Some((c.shown.iter().map(|c| c.label.clone()).collect(), c.selected))
    }

    /// Tests: the rows as shown; empty while the list is not showing.
    #[cfg(test)]
    pub fn completion_rows(&self) -> Vec<Candidate> {
        self.completion.as_ref().map_or_else(Vec::new, |c| c.shown.clone())
    }

    /// Tests: whether an open list (shown or not) waits for the language server's items; `None`
    /// when no list is open.
    #[cfg(test)]
    pub fn completion_waiting(&self) -> Option<bool> {
        self.completion.as_ref().map(|c| c.waiting)
    }

    /// Tests: this buffer's words as another view's list last read them.
    #[cfg(test)]
    pub fn words_read(&self) -> Option<Arc<Vec<String>>> {
        self.words_cache.as_ref().map(|w| w.1.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::super::focused_editor as editor;
    use super::*;
    use gpui::{size, Bounds, Modifiers, ScrollDelta, TestAppContext};
    use gpui::{Entity, EntityInputHandler, TouchPhase, VisualTestContext};

    fn list(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
        e.read_with(cx, |e, _| e.completion_list().map(|l| l.0))
    }

    fn words(w: &[&str]) -> Option<Vec<String>> {
        Some(w.iter().map(|w| w.to_string()).collect())
    }

    fn text(e: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
        e.read_with(cx, |e, cx| e.text(cx))
    }

    fn bounds(e: &Entity<Editor>, cx: &mut VisualTestContext, at: usize) -> Bounds<Pixels> {
        e.read_with(cx, |e, cx| e.char_bounds(at, cx).expect("on screen"))
    }

    #[gpui::test]
    fn navigation_caps_huge_identifiers_without_copying_the_line(cx: &mut TestAppContext) {
        let e = cx.new(|cx| Editor::new(EditorState::from_text(&"x".repeat(4_000_000)), false, cx));
        e.read_with(cx, |e, cx| assert!(e.navigation_word(2_000_000, cx).is_none()));
    }

    #[gpui::test]
    fn typing_opens_the_list_filters_it_and_a_non_word_char_closes_it(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha alpine beta\n", None, cx);
        cx.simulate_input("a");
        assert_eq!(list(&e, cx), None, "one letter is not enough");
        cx.simulate_input("l");
        assert_eq!(list(&e, cx), words(&["alpha", "alpine"]));
        cx.simulate_input("pi");
        assert_eq!(list(&e, cx), words(&["alpine"]));
        cx.simulate_keystrokes("backspace");
        assert_eq!(list(&e, cx), words(&["alpha", "alpine"]), "a backspace widens it");
        cx.simulate_input("-");
        assert_eq!(list(&e, cx), None);
        cx.simulate_input("be");
        assert_eq!(list(&e, cx), words(&["beta"]));
        cx.simulate_keystrokes("backspace backspace");
        assert_eq!(list(&e, cx), None, "the word backspaced away");
        cx.simulate_input("beta");
        assert_eq!(list(&e, cx), None, "only what is typed is left");
    }

    #[gpui::test]
    fn moving_the_caret_or_a_click_closes_the_list(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha\n", None, cx);
        cx.simulate_input("al");
        assert!(list(&e, cx).is_some());
        cx.simulate_keystrokes("left");
        assert_eq!(list(&e, cx), None);
        cx.simulate_keystrokes("right");
        assert_eq!(list(&e, cx), None, "it stays closed");
        cx.simulate_input("p");
        assert_eq!(list(&e, cx), words(&["alpha"]));
        let start = bounds(&e, cx, 0).center();
        cx.simulate_click(start, Modifiers::none());
        assert_eq!(list(&e, cx), None);
    }

    #[gpui::test]
    fn a_scroll_focus_loss_or_a_resize_closes_the_list(cx: &mut TestAppContext) {
        let (e, cx) = editor(&"alpha\n".repeat(40), None, cx);
        let open = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            cx.simulate_input("al");
            assert!(list(e, cx).is_some());
        };
        open(&e, cx);
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(200.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(30.))),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        assert_eq!(list(&e, cx), None, "scrolled");
        cx.simulate_keystrokes("backspace backspace");
        open(&e, cx);
        cx.update(|window, _| window.blur());
        cx.run_until_parked();
        assert_eq!(list(&e, cx), None, "focus went");
        e.update_in(cx, |e, window, _| e.focus(window));
        cx.simulate_keystrokes("backspace backspace");
        open(&e, cx);
        cx.simulate_resize(size(px(500.), px(300.)));
        assert_eq!(list(&e, cx), None, "the window was resized");
    }

    #[gpui::test]
    fn the_wheel_over_the_list_scrolls_its_rows_and_leaves_the_selection(cx: &mut TestAppContext) {
        // 20 words with `w`: 12 rows show at once
        let text: String = (0..20).map(|i| format!("w{i:02}x ")).collect();
        let (e, cx) = editor(&format!("{text}\n"), None, cx);
        cx.simulate_input("w");
        cx.simulate_keystrokes("ctrl-space");
        let window = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, _| {
                let c = e.completion.as_ref().expect("open");
                (c.first, c.selected, c.shown.len())
            })
        };
        assert_eq!(window(&e, cx), (0, 0, 20));
        let at = cx.debug_bounds("popup").expect("drawn").center();
        let wheel = |cx: &mut VisualTestContext, delta: ScrollDelta| {
            cx.simulate_event(ScrollWheelEvent {
                position: at,
                delta,
                modifiers: Modifiers::none(),
                touch_phase: TouchPhase::Moved,
            })
        };
        let lines = |n: f32| ScrollDelta::Lines(point(0., n));
        wheel(cx, lines(-3.));
        assert_eq!(window(&e, cx), (3, 0, 20), "three rows on, the selection kept");
        // a trackpad's small steps add up to a row
        let pixels = |y: f32| ScrollDelta::Pixels(point(px(0.), px(y)));
        wheel(cx, pixels(-15.));
        assert_eq!(window(&e, cx).0, 3);
        wheel(cx, pixels(-15.));
        assert_eq!(window(&e, cx).0, 4);
        // never past the last rows or above the first
        wheel(cx, lines(-50.));
        assert_eq!(window(&e, cx).0, 8);
        wheel(cx, lines(50.));
        assert_eq!(window(&e, cx).0, 0);
        // the keys bring the selected row back into view
        wheel(cx, lines(-6.));
        cx.simulate_keystrokes("down");
        assert_eq!(window(&e, cx), (1, 1, 20));
    }

    #[gpui::test]
    fn the_list_closes_when_its_window_stops_being_the_active_one(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha\n", None, cx);
        cx.simulate_input("al");
        assert!(list(&e, cx).is_some());
        cx.deactivate_window();
        cx.run_until_parked();
        assert_eq!(list(&e, cx), None, "another app or window took the keys");
    }

    #[gpui::test]
    fn the_popup_sits_below_the_caret_with_its_labels_under_the_word(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha\n", None, cx);
        cx.simulate_input("al");
        let (row, popup) = (bounds(&e, cx, 6), cx.debug_bounds("popup").expect("drawn"));
        assert!(popup.top() >= row.bottom(), "{popup:?} below {row:?}");
        assert!((popup.left() - (row.left() - px(INSET))).abs() < px(1.), "{popup:?}");
    }

    #[gpui::test]
    fn the_popup_goes_above_a_caret_on_the_last_row(cx: &mut TestAppContext) {
        // 20 lines fill the 20 rows
        let (e, cx) = editor(&"alpha\n".repeat(19), None, cx);
        cx.simulate_input("al");
        let (row, popup) = (bounds(&e, cx, 114), cx.debug_bounds("popup").expect("drawn"));
        assert!(popup.bottom() <= row.top() + px(1.), "{popup:?} above {row:?}");
    }

    #[gpui::test]
    fn the_popup_takes_the_clicks_on_it_and_a_click_on_a_row_commits_it(cx: &mut TestAppContext) {
        // the list covers line 1's fold button: a click there is the list's, not the fold's
        let (e, cx) = editor("|\nfn b() {\n    x\n    y\n}\nalpha alpine\n", None, cx);
        cx.simulate_input("al");
        assert_eq!(list(&e, cx), words(&["alpha", "alpine"]));
        let at = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let button = l.text_left - px(theme::TEXT_MARGIN + 10.);
            point(button, l.bounds.top() + px(e.line_height() * 1.5))
        });
        let popup = cx.debug_bounds("popup").unwrap();
        assert!(popup.contains(&at), "{at:?} in {popup:?}");
        cx.simulate_click(at, Modifiers::none());
        assert!(e.read_with(cx, |e, cx| e.hidden_lines(cx).is_empty()), "nothing folded");
        assert!(text(&e, cx).starts_with("alpha\nfn b()"), "{:?}", text(&e, cx));
        assert_eq!(e.read_with(cx, |e, _| e.state.cursor()), 5, "the caret is after it");
        assert_eq!(list(&e, cx), None);
    }

    #[gpui::test]
    fn composing_never_opens_the_list_and_tab_stays_the_input_methods(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha\n", None, cx);
        e.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "al", None, window, cx));
        cx.run_until_parked();
        assert_eq!(list(&e, cx), None, "not while composing");
        cx.simulate_keystrokes("tab");
        assert!(!text(&e, cx).contains("alpha\nalpha"), "{:?}", text(&e, cx));
    }

    #[gpui::test]
    fn a_composition_hides_the_open_list(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha\n", None, cx);
        cx.simulate_input("al");
        assert!(list(&e, cx).is_some());
        e.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "p", None, window, cx));
        cx.run_until_parked();
        assert_eq!(list(&e, cx), None);
        cx.simulate_keystrokes("tab");
        assert!(!text(&e, cx).contains("alpha\nalpha"), "{:?}", text(&e, cx));
    }

    #[gpui::test]
    fn commit_replaces_the_prefix_at_every_caret_that_has_it_in_one_undo_step(cx: &mut TestAppContext) {
        let (e, cx) = editor("foobar\nfo\nfo\nxfo", None, cx);
        // carets after "fo", "xfo" and "fo"; the primary (the last set) on line 2
        e.update(cx, |e, cx| {
            let sels = [9, 16, 12].map(Selection::caret).to_vec();
            e.edit(cx, |s| s.set_selections(sels));
        });
        cx.simulate_keystrokes("ctrl-space");
        assert_eq!(list(&e, cx), words(&["foobar", "xfo"]));
        cx.simulate_keystrokes("tab");
        let carets = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, _| (e.state.selections.iter()).map(|s| s.head).collect::<Vec<_>>())
        };
        assert_eq!(text(&e, cx), "foobar\nfoobar\nfoobar\nxfo");
        assert_eq!(carets(&e, cx), [13, 20, 24]);
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&e, cx), "foobar\nfo\nfo\nxfo");
        assert_eq!(carets(&e, cx), [9, 12, 16]);
        cx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(text(&e, cx), "foobar\nfoobar\nfoobar\nxfo");
        assert_eq!(carets(&e, cx), [13, 20, 24]);
    }

    #[gpui::test]
    fn a_comment_offers_words_but_no_keywords_once_its_line_is_coloured(cx: &mut TestAppContext) {
        let (e, cx) = editor("fn main() {}\n// ", Some(Path::new("/tmp/orlo-x.rs")), cx);
        cx.simulate_input("m");
        e.update_in(cx, |e, window, cx| {
            e.replace_text_in_range(None, "a", window, cx);
            // the line just typed on has no colours: it would read as code
            assert_eq!(e.completion.as_ref().expect("open").class, None);
        });
        cx.run_until_parked();
        assert_eq!(list(&e, cx), words(&["main"]));
        // in code the keywords are on offer
        cx.simulate_keystrokes("escape enter");
        cx.simulate_input("ma");
        let l = list(&e, cx).unwrap_or_default();
        assert!(l.contains(&"match".into()) && l.contains(&"main".into()), "{l:?}");
    }

    #[gpui::test]
    fn a_path_in_a_string_offers_its_folders_entries_and_a_folder_its_own(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("orlo-popup-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        std::fs::write(dir.join("main.rs"), "").unwrap();
        let (e, cx) = editor("fn main() {\n", Some(&dir.join("x.rs")), cx);
        cx.simulate_input("let p = \"./sr");
        assert_eq!(list(&e, cx), words(&["src/"]));
        cx.simulate_keystrokes("tab");
        assert!(text(&e, cx).ends_with("\"./src/\""), "{:?}", text(&e, cx));
        assert_eq!(list(&e, cx), words(&["lib.rs"]), "the folder's own entries");
        cx.simulate_keystrokes("enter");
        assert!(text(&e, cx).ends_with("\"./src/lib.rs\""), "{:?}", text(&e, cx));
        // in code a path offers words, not entries
        cx.simulate_keystrokes("cmd-down enter");
        cx.simulate_input("./sr");
        let l = list(&e, cx).unwrap_or_default();
        assert!(l.contains(&"src".into()) && !l.contains(&"src/".into()), "{l:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn a_backspace_in_a_path_reads_its_folder_again_for_the_shorter_part(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("orlo-popup-widen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        for file in ["main.rs", "Makefile", "mod.rs"] {
            std::fs::write(dir.join("src").join(file), "").unwrap();
        }
        let (e, cx) = editor("fn main() {\n", Some(&dir.join("x.rs")), cx);
        cx.simulate_input("let p = \"./src/ma");
        assert_eq!(list(&e, cx), words(&["main.rs", "Makefile"]));
        cx.simulate_keystrokes("backspace");
        assert_eq!(list(&e, cx), words(&["mod.rs", "main.rs", "Makefile"]));
        // typing on narrows what was read
        cx.simulate_input("o");
        assert_eq!(list(&e, cx), words(&["mod.rs"]));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The language server's item `label`, as the workspace offers it.
    fn server(label: &str, at: usize) -> Candidate {
        Candidate {
            label: label.into(),
            kind: Kind::Symbol("function".into()),
            detail: format!("fn {label}()"),
            insert: Insert::Server(at),
        }
    }

    #[gpui::test]
    fn the_list_waits_for_the_servers_items_and_ranks_them_with_the_words(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha alpine\nzq", None, cx);
        let ask = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.update_in(cx, |e, window, cx| assert!(e.expect_server_items(window, cx)));
            cx.run_until_parked();
        };
        // nothing here matches `zq`: the list waits for the server's items, hidden
        ask(&e, cx);
        assert!(e.read_with(cx, |e, _| e.completion.is_some()), "still open");
        assert_eq!(list(&e, cx), None);
        e.update(cx, |e, cx| e.add_server_items(vec![server("zqx", 0)], cx));
        assert_eq!(list(&e, cx), words(&["zqx"]));
        // an edit ends the wait: the answer would be for the text before it
        cx.simulate_keystrokes("escape");
        ask(&e, cx);
        cx.simulate_input("w");
        assert!(e.read_with(cx, |e, _| e.completion.is_none()), "nothing has `zqw`");
        // ranked with the words; a label both offer is the server's item
        cx.simulate_keystrokes("enter");
        cx.simulate_input("al");
        assert_eq!(list(&e, cx), words(&["alpha", "alpine"]));
        ask(&e, cx);
        let items = vec![server("alpha", 0), server("alpaca", 1)];
        e.update(cx, |e, cx| e.add_server_items(items, cx));
        let rows = e.read_with(cx, |e, _| e.completion_rows());
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        // the words are two lines away; a server item not in the text loses nothing for distance
        assert_eq!(labels, ["alpaca", "alpha", "alpine"]);
        assert_eq!(rows[1].insert, Insert::Server(0));
    }

    #[gpui::test]
    fn a_servers_item_committed_goes_to_the_workspace_with_the_edits_typed_since(cx: &mut TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        let (e, cx) = editor("pri", None, cx);
        let committed = Rc::new(RefCell::new(Vec::new()));
        let sink = committed.clone();
        let _sub = cx.update(|_, cx| {
            cx.subscribe(&e, move |_, event: &EditorEvent, _| {
                if let EditorEvent::CommitServerItem(at, since) = event {
                    sink.borrow_mut().push((*at, since.clone()));
                }
            })
        });
        // the server's items before the words: the list shows them once the words are in
        e.update_in(cx, |e, window, cx| {
            assert!(e.expect_server_items(window, cx));
            e.add_server_items(vec![server("print", 3)], cx);
        });
        cx.run_until_parked();
        assert_eq!(list(&e, cx), words(&["print"]));
        cx.simulate_input("n");
        assert_eq!(list(&e, cx), words(&["print"]), "typing filters them");
        cx.simulate_keystrokes("tab");
        assert_eq!(list(&e, cx), None);
        assert_eq!(text(&e, cx), "prin", "the workspace applies it");
        let typed = Change { start: 3, removed: 0, inserted: 1, lines: (0, 0) };
        assert_eq!(*committed.borrow(), [(3, vec![typed])]);
    }

    #[gpui::test]
    fn ctrl_space_over_a_showing_list_asks_again_and_a_merge_keeps_the_row_selected(cx: &mut TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let (e, cx) = editor("alpha alpine alto\n", None, cx);
        let asked = Rc::new(Cell::new(0));
        let n = asked.clone();
        let _sub = cx.update(|_, cx| {
            cx.subscribe(&e, move |_, event: &EditorEvent, _| {
                if matches!(event, EditorEvent::ServerCompletions) {
                    n.set(n.get() + 1);
                }
            })
        });
        let shown = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, _| e.completion_list()).expect("showing")
        };
        cx.simulate_input("al");
        cx.simulate_keystrokes("down");
        let (rows, at) = shown(&e, cx);
        assert_eq!((rows[at].as_str(), at), ("alpha", 1), "{rows:?}");
        cx.simulate_keystrokes("ctrl-space");
        assert_eq!(asked.get(), 1, "a showing list asks too");
        assert_eq!(shown(&e, cx), (rows, at), "and stays as it is");
        // the workspace's part: the items join the showing list
        e.update(cx, |e, cx| e.add_server_items(vec![server("alfa", 0)], cx));
        let (merged, now) = shown(&e, cx);
        assert_eq!(merged, ["alfa", "alto", "alpha", "alpine"]);
        assert_eq!(now, 2, "the row the user is on stays selected");
    }

    #[gpui::test]
    fn late_gather_cannot_publish_after_syntax_changes(cx: &mut TestAppContext) {
        let (editor, cx) = editor("alpha bravo\nal", Some(Path::new("/tmp/late.rs")), cx);
        editor.update_in(cx, |e, window, cx| {
            e.expect_server_items(window, cx);
        });
        cx.run_until_parked();
        let (context, all, signals) = gathered(&editor, cx);
        editor.update(cx, |e, cx| {
            e.highlighter = crate::languages::highlight::Highlighter::by_name("Python");
            e.publish_words(context, all, Some(signals), cx);
            assert!(e.completion.is_none(), "old gathered syntax must not survive publication");
        });
    }

    /// The list's words as its job gathers them: their context, the words and the signals.
    fn gathered(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (CompletionContext, Vec<Candidate>, Signals) {
        let (job, _) = e.update(cx, |e, cx| e.word_job(cx)).unwrap();
        let context = job.context.clone();
        let (all, _, signals) = job.run();
        (context, all, signals)
    }

    fn late_index_context(cx: &mut TestAppContext, replace_owner: bool) {
        let (editor, cx) = editor("alpha bravo\nal", Some(Path::new("/tmp/late-index.rs")), cx);
        cx.update(|_, cx| cx.set_global(crate::config::settings::Settings::default()));
        let index = cx.new(crate::languages::index::ProjectIndex::new);
        editor.update_in(cx, |e, window, cx| {
            e.completion_index = Some(index.downgrade());
            e.expect_server_items(window, cx);
        });
        cx.run_until_parked();
        let (context, all, signals) = gathered(&editor, cx);
        if replace_owner {
            let other = cx.new(crate::languages::index::ProjectIndex::new);
            editor.update(cx, |e, _| e.completion_index = Some(other.downgrade()));
            // `other` is held, so the new index is still there when the words are published
            editor.update(cx, |e, cx| {
                e.publish_words(context, all, Some(signals), cx);
                assert!(e.completion.is_none(), "old window's gathered context survived");
            });
        } else {
            index.update(cx, |i, cx| {
                i.generation += 1;
                cx.notify();
            });
            editor.update(cx, |e, cx| {
                e.publish_words(context, all, Some(signals), cx);
                assert!(e.completion.is_none(), "old index generation survived");
            });
        }
    }

    #[gpui::test]
    fn late_gather_cannot_publish_after_index_generation_changes(cx: &mut TestAppContext) {
        late_index_context(cx, false);
    }

    #[gpui::test]
    fn late_gather_cannot_publish_after_window_owner_changes(cx: &mut TestAppContext) {
        late_index_context(cx, true);
    }

    #[gpui::test]
    fn rejected_old_gather_keeps_a_new_popup_in_its_new_context(cx: &mut TestAppContext) {
        let (editor, cx) = editor("alpha bravo\nal", Some(Path::new("/tmp/new-context.rs")), cx);
        editor.update_in(cx, |e, window, cx| {
            e.expect_server_items(window, cx);
        });
        cx.run_until_parked();
        let (context, all, signals) = gathered(&editor, cx);
        editor.update_in(cx, |e, window, cx| {
            e.highlighter = crate::languages::highlight::Highlighter::by_name("Python");
            e.close_completion(cx);
            e.open_completion(window, cx);
            e.publish_words(context, all, Some(signals), cx);
            let completion = e.completion.as_ref().expect("new popup survives");
            assert!(completion.all.is_none(), "old data was published into the new popup");
            assert_eq!(completion.context.0, "Python");
        });
    }

    #[gpui::test]
    fn member_context_flows_from_buffer_to_popup(cx: &mut TestAppContext) {
        let (e, cx) = editor("obj.length(); let len = 1;\nobj.le|", Some(Path::new("/tmp/context.rs")), cx);
        cx.simulate_keystrokes("ctrl-space");
        let labels = list(&e, cx).expect("completion list");
        assert_eq!(labels.first().map(String::as_str), Some("length"), "{labels:?}");
    }

    #[gpui::test]
    fn recency_retains_50_commit_events_including_repeated_items(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha bravo\nal", None, cx);
        for n in 0..51 {
            e.update_in(cx, |e, window, cx| {
                e.set_text(if n == 0 { "alpha bravo\nal" } else { "alpha bravo\nbr" }, cx);
                sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).len());
                e.open_completion(window, cx);
            });
            cx.run_until_parked();
            assert!(e.update_in(cx, |e, window, cx| e.commit_completion(None, window, cx)));
        }
        e.read_with(cx, |e, _| {
            let recent = e.completion_recent.borrow();
            assert_eq!(recent.len(), 50);
            assert!(recent.iter().all(|s| s == "bravo"));
        });
    }

    #[gpui::test]
    fn call_context_is_read_at_the_caret(cx: &mut TestAppContext) {
        let (e, cx) = editor("|(", None, cx);
        e.update_in(cx, |e, window, cx| {
            e.expect_server_items(window, cx);
            e.add_server_items(
                vec![
                    Candidate::text("alpha".into(), Kind::Symbol("variable".into())),
                    Candidate::text("bravo".into(), Kind::Symbol("function".into())),
                ],
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(list(&e, cx), words(&["bravo", "alpha"]));
    }

    #[gpui::test]
    fn a_huge_buffers_words_are_gathered_off_the_ui_thread(cx: &mut TestAppContext) {
        // 4M chars; the caret on the first line
        let text: String = (0..500_000).map(|i| format!("w{i:06}\n")).collect();
        let (e, cx) = editor(&format!("|{text}"), None, cx);
        e.update_in(cx, |e, window, cx| {
            e.auto_complete(&AutoComplete, window, cx);
            let c = e.completion.as_ref().expect("open");
            assert!(c.all.is_none(), "nothing gathered on the UI thread");
        });
        cx.run_until_parked();
        let (l, _) = e.read_with(cx, |e, _| e.completion_list()).expect("shown");
        assert_eq!(l.len(), 5_000, "lines 1 to 5,000 (the caret's word is left out)");
    }

    #[gpui::test]
    fn replacing_another_buffers_text_refreshes_its_completion_words(cx: &mut TestAppContext) {
        let mut source_app = cx.clone();
        let (source, source_cx) = editor("oldword", None, &mut source_app);
        let (e, cx) = editor("ol", None, cx);
        let reader = source.clone();
        e.update(cx, |e, _| e.open_views = Some(std::rc::Rc::new(move |_| vec![reader.clone()])));
        cx.simulate_keystrokes("ctrl-space");
        assert_eq!(list(&e, cx), words(&["oldword"]));
        source.update(source_cx, |source, cx| source.set_text("newword", cx));
        e.update(cx, |e, cx| {
            e.set_text("ne", cx);
            e.sel(cx).set_cursor(2);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("ctrl-space");
        assert_eq!(list(&e, cx), words(&["newword"]));
    }

    #[gpui::test]
    fn navigation_hover_closes_on_resize_blur_syntax_and_root_changes(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_global(crate::config::settings::Settings::default()));
        let index = cx.new(crate::languages::index::ProjectIndex::new);
        let shared = index.clone();
        let (editor, cx) = cx.add_window_view(move |_, cx| {
            let mut e = Editor::new(EditorState::from_text("fn target() {}\n"), false, cx);
            e.highlighter = Highlighter::by_name("Rust");
            e.completion_index = Some(shared.downgrade());
            e
        });
        cx.simulate_resize(gpui::size(px(600.), px(300.)));
        cx.run_until_parked();
        editor.update_in(cx, |e, w, cx| {
            w.activate_window();
            e.focus(w);
            cx.notify();
        });
        cx.run_until_parked();
        editor.update_in(cx, |e, w, cx| {
            let p = e.char_bounds(4, cx).unwrap().center();
            e.hover_definition(Some(p), w, cx);
            assert!(e.definition_hover.is_some());
        });
        cx.simulate_resize(gpui::size(px(650.), px(300.)));
        cx.run_until_parked();
        assert!(editor.read_with(cx, |e, _| e.definition_hover.is_none()));
        editor.update_in(cx, |e, w, cx| {
            let p = e.char_bounds(4, cx).unwrap().center();
            e.hover_definition(Some(p), w, cx);
            assert!(e.definition_hover.is_some());
            e.highlighter = Highlighter::by_name("Plain Text");
            let _ = e.render_definitions(w, cx);
            assert!(e.definition_hover.is_none());
            e.highlighter = Highlighter::by_name("Rust");
            cx.notify();
        });
        cx.run_until_parked();
        editor.update_in(cx, |e, w, cx| {
            let p = e.char_bounds(4, cx).unwrap().center();
            e.hover_definition(Some(p), w, cx);
            assert!(e.definition_hover.is_some());
            index.update(cx, |index, _| index.generation += 1);
            let _ = e.render_definitions(w, cx);
            assert!(e.definition_hover.is_none());
            e.hover_definition(Some(p), w, cx);
            assert!(e.definition_hover.is_some());
            let other = cx.focus_handle();
            w.focus(&other);
            let _ = e.render_definitions(w, cx);
            assert!(e.definition_hover.is_none());
        });
    }
}
