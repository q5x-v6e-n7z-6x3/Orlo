//! The editor view: a gpui entity holding one view's state (`ViewModel`) of a shared document
//! (`OpenDocument`), plus a custom element that paints only the visible lines.

mod actions;
pub(crate) use actions::editor_actions;
mod aligned;
mod commands;
#[cfg(test)]
mod cursor_column_tests;
#[cfg(test)]
pub(crate) use cursor_column_tests::bands as cursor_column_bands;
mod element;
#[cfg(test)]
pub use element::BOOKMARK_DOTS;
mod diagnostics;
#[cfg(test)]
mod enter_indent_tests;
#[cfg(test)]
mod filler_tests;
#[cfg(test)]
mod fold_tests;
#[cfg(test)]
mod gutter_tests;
#[cfg(test)]
pub(crate) use gutter_tests::gutter_labels;
mod input;
pub(crate) use input::unicode_scalar;
mod measure;
#[cfg(test)]
mod motion_tests;
mod mouse;
mod occurrences;
mod popup;
mod snippets;
mod spelling;
mod syntax_selection;
#[cfg(test)]
mod syntax_selection_tests;
mod view_filter;
#[cfg(test)]
mod virtual_space_tests;
pub(crate) use spelling::SpellingSource;

pub use actions::*;
pub use aligned::{highlights_of, hunks_of, AlignedPair};
#[cfg(test)]
pub(crate) use commands::rich;
pub(crate) use commands::{clipboard_item, copy_to_clipboard};
use element::{EditorElement, Layout};
pub(crate) use popup::{definition_lines, DefinitionPreview, DefinitionSource};
#[cfg(test)]
pub(crate) use snippets::tests::fixed_clock;
pub(crate) use snippets::SnippetVariables;
pub use snippets::{refresh_snippets, snippet_title, SnippetStore};

use crate::chrome::theme::{self, c, ui};
use crate::config::settings::{Cleanup, WhiteSpace, WordWrap};
use crate::documents::{DocumentEvent, Documents, OpenDocument};
use crate::editing::display::filler::{Fillers, Padded};
use crate::editing::display::{top_anchor, WrapMap};
use crate::editing::fold::{Folded, Folds};
use crate::editing::text::{
    Change, DisplayCol, DisplayPoint, DisplayRow, Document, Editing, EditorState, Loaded, Rows, Selection, TextChange,
    ViewEdit, ViewModel, ViewRead, ViewSelect,
};
use crate::languages::highlight::{self, Highlighter};
use gpui::{
    div, prelude::*, px, App, Context, CursorStyle, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement,
    MouseButton, Pixels, Point, Render, SharedString, Subscription, Task, WeakEntity, Window,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Window-wide display settings: the settings file plus the window's menu overrides; the
/// workspace stamps them on every tab.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewSettings {
    pub font_face: SharedString,
    pub font_size: f32,
    pub minimap: bool,
    pub rulers: Vec<usize>,
    pub word_wrap: WordWrap,
    /// The settings' (or project's) `wrap_width`: 0 wraps at the view's width.
    pub wrap_width: usize,
    /// Page Up/Down scroll smoothly: `animation_enabled` and not Reduce Motion.
    pub animate: bool,
    /// Line numbers (and the bookmark dots beside them); distraction-free mode can hide them.
    pub gutter: bool,
    pub line_number_mode: crate::config::settings::LineNumberMode,
    pub cursor_column_highlight: bool,
    /// The text column centred in the view at the wrap width (distraction-free mode).
    pub centered: bool,
    /// The gutter's fold buttons, and whether ▾ waits for the pointer over the gutter.
    pub fold_buttons: bool,
    pub fade_fold_buttons: bool,
    /// Where white space is marked.
    pub white_space: WhiteSpace,
    pub unicode_warnings: bool,
    pub selection_occurrences: bool,
    pub inline_diagnostics: bool,
    pub inline_diagnostics_severity: crate::config::settings::DiagnosticSeverity,
    pub unicode_warning_characters: crate::config::settings::unicode::WarningCharacters,
    pub unicode_warning_ranges: crate::config::settings::unicode::WarningRanges,
    /// A press on selected text drags it.
    pub drag_text: bool,
    /// Typing a word opens the completion list, once it is this many letters long.
    pub auto_complete: bool,
    pub auto_complete_min_chars: usize,
    pub spell_check: bool,
    pub spelling_selector: String,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            font_face: theme::EDITOR_FONT.into(),
            font_size: theme::EDITOR_FONT_SIZE,
            minimap: true,
            rulers: Vec::new(),
            word_wrap: WordWrap::Auto,
            wrap_width: 0,
            animate: false,
            gutter: true,
            line_number_mode: crate::config::settings::LineNumberMode::Absolute,
            cursor_column_highlight: false,
            centered: false,
            fold_buttons: true,
            fade_fold_buttons: true,
            white_space: WhiteSpace::SELECTION,
            unicode_warnings: false,
            selection_occurrences: false,
            inline_diagnostics: false,
            inline_diagnostics_severity: crate::config::settings::DiagnosticSeverity::Warning,
            unicode_warning_characters: crate::config::settings::unicode::default_characters(),
            unicode_warning_ranges: Default::default(),
            drag_text: true,
            auto_complete: true,
            auto_complete_min_chars: 2,
            spell_check: false,
            spelling_selector: crate::languages::spell::DEFAULT_SELECTOR.into(),
        }
    }
}

/// A mouse drag under way: what it selects from, an option-drag's display corner and
/// pixel x goal, and where the mouse last was (autoscroll past an edge selects to it).
pub(crate) struct Drag {
    from: crate::editing::text::DragFrom,
    corner: Option<(DisplayPoint, f32)>,
    pos: Point<Pixels>,
    /// The press was on selected text, which a drag moves instead of selecting from.
    text: Option<TextDrag>,
}

/// A press on selected text: the selections stay as they are until the button comes up, a
/// click then (the press's caret), or a drop once the pointer has gone 4 px.
pub(crate) struct TextDrag {
    press: Point<Pixels>,
    moved: bool,
    /// Escape cancels the drag before a key binding sees it; dropped with the drag.
    _escape: Subscription,
}

pub(crate) enum EditorEvent {
    /// A double click in Find Results: the file and 0-based line it points at, and on a match
    /// line where the match sits (`search::result_match`).
    OpenResult(PathBuf, usize, Option<(usize, Option<String>)>),
    /// A short outcome for the status bar ("No more occurrences").
    Status(String),
    /// A gutter change marker was clicked; the workspace opens this hunk's diff.
    ShowChange(usize),
    /// The user scrolled the view (wheel, page, minimap, scroll commands), not a follower's
    /// sync: synchronized groups follow. An aligned view also tells of a scroll that keeps its
    /// caret in view.
    Scrolled,
    /// An aligned view's rows changed (its text, wrapping, width or folds): its pair places
    /// the filler rows again.
    RowsChanged,
    /// An aligned view's folds changed by a command, a click or text revealed in one: its pair
    /// folds the matching text on the other side.
    FoldsChanged,
    /// `ctrl-space` with the completion list open (or just opened): the workspace adds the
    /// language server's items, quietly.
    ServerCompletions,
    SignatureInput(String, Option<(Arc<serde_json::Value>, usize)>, bool),
    /// The language server's item at this index was committed from the list: the workspace
    /// applies it, through the edits made since the items came.
    CommitServerItem(usize, Vec<Change>),
    /// Tab found several snippets for the trigger before the carets: the workspace asks which
    /// (their files), and the choice replaces the trigger.
    ChooseSnippet(Vec<PathBuf>, String),
    HoverDefinition(Range<usize>, String, u64),
    OpenDefinition(DefinitionPreview, bool),
    ScanSpelling(bool),
    SpellingSuggestions(usize),
}

/// Comparison snapshots carry immutable, sorted character ranges prepared on the worker.
pub struct ComparisonHighlights {
    pub lines: Vec<Range<usize>>,
    pub inline: Vec<Range<usize>>,
    pub added: bool,
}

pub struct Editor {
    /// User text mutations; separate from a lossy buffer's save protection.
    pub(crate) input_enabled: bool,
    pub comparison: Option<ComparisonHighlights>,
    /// The document this view shows: its text, undo history and file, shared with every
    /// other view of it.
    pub document: Entity<OpenDocument>,
    /// This view's own selections, bookmarks and marks, at the version of the text they belong
    /// to.
    pub state: ViewModel,
    /// The document's changes, which this view follows once each.
    _document: Subscription,
    /// Hunks against the workspace's saved or Git index baseline, computed off the UI thread.
    pub changes: Vec<crate::integrations::changes::Hunk>,
    /// The buffer version these hunks describe; painting hides them as soon as text changes.
    pub changes_version: u64,
    pub preference_warning: Option<String>,
    pub highlighter: Highlighter,
    syntax_selection: Option<syntax_selection::History>,
    pub language_version: Option<(PathBuf, u64)>,
    pub diagnostics: Vec<crate::languages::language::Diagnostic>,
    inline_diagnostics: Option<Rc<diagnostics::InlineDiagnostics>>,
    pub semantic_spans: Vec<crate::languages::language::SemanticSpan>,
    pub lsp_status: String,
    pub single_line: bool,
    /// A panel field that keeps line breaks (Find, Replace): pasted and typed newlines stay,
    /// `ctrl-enter` inserts one, and it scrolls to its caret.
    pub multiline: bool,
    /// Quick panel inputs use the system font; everything else the editor font.
    pub ui_font: bool,
    pub view: ViewSettings,
    /// Last bounded declarative scan; workspace preferences own enablement/application.
    pub(crate) modelines: crate::config::settings::modelines::Properties,
    pub(crate) modelines_enabled: bool,
    /// Canonical registry/syntax name from enabled declarations, never a manual override.
    pub(crate) modeline_language: Option<String>,
    /// Drawn on the inactive-group background; the workspace sets it.
    pub dimmed: bool,
    /// Find Results: no line numbers, and a double click opens the result.
    pub results: bool,
    /// The regex playground's sample: its outlines are the playground's matches.
    pub playground: bool,
    /// The document copied into this playground sample; a closed source is not retained.
    pub playground_source: Option<WeakEntity<Editor>>,
    /// A scratch tab's title ("Preview: …"), until it is saved as a file.
    pub label: Option<String>,
    pub find_ranges: Vec<Range<usize>>,
    occurrences: Option<occurrences::Occurrences>,
    /// View > Word Wrap in this view, over the settings file's `word_wrap`.
    pub wrap_override: Option<bool>,
    pub spell_check_override: Option<bool>,
    spell_cache: crate::languages::spell::Cache,
    spell_job: Option<HtmlJob>,
    spell_revision: u64,
    spell_stop: Option<(Arc<AtomicBool>, SpellingSource)>,
    #[cfg(test)]
    pub(crate) spell_checker: Option<Arc<dyn crate::languages::spell::Checker + Send + Sync>>,
    /// View > Word Wrap Column in this view, over `wrap_width`: 0 is the view's width.
    pub wrap_width_override: Option<usize>,
    /// The display rows while wrapped; prepaint builds it at the view's width.
    wrap: Option<WrapMap>,
    /// This view's folds: edits map them; sessions persist validated exact ranges.
    folds: Folds,
    view_filter: Option<view_filter::ViewFilter>,
    filter_job: Option<view_filter::FilterJob>,
    filter_epoch: u64,
    session_fold_cache: std::cell::RefCell<orlo_workspace::sessions::folds::FoldCache>,
    /// Rows with no text between this view's lines (a side-by-side diff's padding); set from
    /// outside, never saved, copied or searched.
    fillers: Fillers,
    /// One side of a side-by-side diff (`AlignedPair`): it tells its pair when its rows, its
    /// folds or its scroll change.
    pub(crate) aligned: bool,
    /// What the rows were last drawn from (text, folds, wrapping), as an aligned view told its
    /// pair.
    rows_key: Option<(u64, u64, Option<u64>)>,
    /// Changes whenever this view's folds do.
    folds_generation: u64,
    /// The scroll (top, left) an aligned view last told its pair of, or was given by it.
    synced_scroll: (f64, f32),
    /// The text to keep on top through a change of font, width or wrapping, or a restored
    /// scroll; the next paint scrolls to it.
    top_anchor: Option<(usize, f32)>,
    /// Goto > Jump Back / Jump Forward: caret positions left by goto commands.
    jumps: Vec<usize>,
    jumps_fwd: Vec<usize>,
    /// Pixels from the top of the first row: f64, as f32 loses whole pixels past a million rows.
    /// While Page Up/Down animates it is where the view is going; `drawn_scroll` is where it is.
    scroll_top: f64,
    follow_log_tail: bool,
    /// Explicit selection generation of the auto-tail caret held while scrolled away.
    follow_log_paused_caret: Option<u64>,
    /// Page Up/Down's animation: the drawn scroll it started from, and when.
    scroll_anim: Option<(f64, Instant)>,
    scroll_left: f32,
    /// Set after an edit or goto: the next paint keeps the caret in view with that frame's
    /// rows, and scrolls sideways to it.
    follow_caret: bool,
    focus_handle: FocusHandle,
    layout: Option<Layout>,
    pub(crate) drag: Option<Drag>,
    autoscrolling: bool,
    /// IME or dead-key text being composed: a range per caret, in selection order.
    pub(crate) marked: Vec<Range<usize>>,
    overtype_composition: Option<input::OvertypeComposition>,
    literal_next: Option<input::LiteralNext>,
    /// Where the pointer is over this view, for what hover shows: the minimap's view box, the
    /// fold buttons.
    pub(crate) hover: Option<Point<Pixels>>,
    /// Set while the overlay scroll bar shows; the task hides it, and dropping it cancels.
    scroll_bar: Option<Task<()>>,
    /// Highlighting lines too far from the cache for a frame, on another thread.
    highlight_job: Option<Task<()>>,
    /// A column command on a big document, parsing on another thread; a newer one replaces it.
    column_job: Option<Task<()>>,
    /// Fold Level on a big document, finding the regions on another thread; likewise.
    fold_job: Option<Task<()>>,
    /// Copy as HTML, being built on the highlighting thread; a newer one replaces it.
    html_job: Option<HtmlJob>,
    /// After a job panicked: no other until then, and the wait doubles each time.
    highlight_backoff: Option<(std::time::Instant, std::time::Duration)>,
    /// The completion list, while it is open.
    pub(crate) signature_help: Option<popup::SignaturePopup>,
    pub(crate) signature_pending: Option<popup::SignaturePending>,
    pub(crate) signature_inflight: Option<Arc<AtomicBool>>,
    pub(crate) signature_triggers: (Vec<String>, Vec<String>),
    completion: Option<popup::Completion>,
    snippet_choice: Option<snippets::SnippetChoice>,
    pub(crate) completion_epoch: u64,
    pub(crate) completion_index: Option<WeakEntity<crate::languages::index::ProjectIndex>>,
    pub(crate) completion_recent: Rc<RefCell<VecDeque<String>>>,
    definition_hover: Option<popup::DefinitionHover>,
    /// Every view open in any window, for the other buffers' words on the list; the workspace
    /// sets it.
    pub open_views: Option<OpenViews>,
    /// This buffer's words as of a version, as another view's list last read them.
    words_cache: Option<(u64, Arc<Vec<String>>)>,
}

/// Sets its flag when dropped: a job on the highlighting thread is told to stop when the view that
/// wanted it is done with it (closed, or asking again).
pub(crate) struct Cancel(pub(crate) Arc<AtomicBool>);

impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A Copy as HTML under way: the task that waits for the highlighting thread's page, and the flag
/// that tells the thread to stop when this is dropped (a newer copy replaces it, the view closes).
struct HtmlJob {
    _task: Task<()>,
    _stop: Cancel,
}

/// The views open in every window, as the workspace lists them.
pub type OpenViews = Rc<dyn Fn(&App) -> Vec<Entity<Editor>>>;

/// How long a Page Up/Down glides.
pub const SCROLL_ANIM: Duration = Duration::from_millis(120);

/// The drawn scroll `elapsed` into a glide from `from` to `to`: ease-out cubic, `to` at the end.
pub fn scroll_at(from: f64, to: f64, elapsed: Duration) -> f64 {
    let t = (elapsed.as_secs_f64() / SCROLL_ANIM.as_secs_f64()).min(1.);
    from + (to - from) * (1. - (1. - t).powi(3))
}

/// The rows a view shows: its display lines (lines joined where folds hide their breaks, each
/// fold a placeholder), wrapped or one a row, with its filler rows. Built from the fields so a
/// caller can hold them while it moves the state's carets.
fn view_rows<'a>(wrap: Option<&'a WrapMap>, folds: &'a Folds, fillers: &'a Fillers) -> Padded<'a, Folded<'a>> {
    Padded::new(Folded::new(wrap, folds), fillers)
}

/// `document`'s model: `Editor::doc` borrowing only the editor's document field, for code that
/// changes the editor's other fields meanwhile.
pub(crate) fn doc_of<'a>(document: &Entity<OpenDocument>, cx: &'a App) -> &'a Document {
    &document.read(cx).model
}

/// `Editor::read_state` borrowing only the editor's document and view state.
pub(crate) fn read_of<'a>(document: &Entity<OpenDocument>, state: &'a ViewModel, cx: &'a App) -> ViewRead<'a> {
    Editing { doc: doc_of(document, cx), view: state }
}

/// `Editor::sel` borrowing only the editor's document and view state.
pub(crate) fn sel_of<'a>(document: &Entity<OpenDocument>, state: &'a mut ViewModel, cx: &'a App) -> ViewSelect<'a> {
    Editing { doc: doc_of(document, cx), view: state }
}

fn byte_of(text: &str, col: usize) -> usize {
    text.char_indices().nth(col).map_or(text.len(), |(b, _)| b)
}

/// Enough of the first line for a shebang, without copying a 10 MB one.
pub fn first_line(rope: &ropey::Rope) -> String {
    let line = rope.line(0);
    line.slice(..line.len_chars().min(200)).to_string()
}

/// The start of the text that language detection reads (`files.content` patterns).
pub fn head(rope: &ropey::Rope) -> String {
    let chars = rope.byte_to_char(rope.len_bytes().min(orlo_syntax::registry::CONTENT_BYTES));
    rope.slice(..chars).to_string()
}

impl Editor {
    /// The route used by settings/LSP; only Highlighter's explicit choice is manual state.
    pub(crate) fn language_choice(&self) -> Option<&str> {
        self.highlighter.language_override().or(self.modeline_language.as_deref())
    }

    /// A view of a document of its own (a new or opened file, a panel field).
    pub fn new(state: EditorState, single_line: bool, cx: &mut Context<Self>) -> Self {
        let (doc, view) = state.into_parts();
        let document = Documents::open(doc, cx);
        Self::of_document(document, view, single_line, cx)
    }

    /// A view of `document` with the view state `view`, made for it (`Document::attach_view`).
    pub fn of_document(
        document: Entity<OpenDocument>,
        mut view: ViewModel,
        single_line: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        sel_of(&document, &mut view, cx).sel_reset();
        let doc = doc_of(&document, cx);
        let changes_version = doc.version;
        let highlighter = match single_line {
            true => Highlighter::new(None),
            false => Highlighter::detect(doc.path.as_deref(), &head(&doc.rope)),
        };
        let subscription = cx.subscribe(&document, |e, _, ev, cx| e.on_document(ev, cx));
        Self {
            document,
            state: view,
            _document: subscription,
            comparison: None,
            changes: Vec::new(),
            changes_version,
            preference_warning: None,
            highlighter,
            syntax_selection: None,
            language_version: None,
            diagnostics: Vec::new(),
            inline_diagnostics: None,
            semantic_spans: Vec::new(),
            lsp_status: String::new(),
            single_line,
            input_enabled: true,
            multiline: false,
            ui_font: false,
            view: ViewSettings::default(),
            modelines: Default::default(),
            modelines_enabled: false,
            modeline_language: None,
            dimmed: false,
            results: false,
            playground: false,
            playground_source: None,
            label: None,
            find_ranges: Vec::new(),
            occurrences: None,
            wrap_override: None,
            spell_check_override: None,
            spell_cache: Default::default(),
            spell_job: None,
            spell_revision: 0,
            spell_stop: None,
            #[cfg(test)]
            spell_checker: None,
            wrap_width_override: None,
            wrap: None,
            folds: Folds::default(),
            view_filter: None,
            filter_job: None,
            filter_epoch: 0,
            session_fold_cache: Default::default(),
            fillers: Fillers::default(),
            aligned: false,
            rows_key: None,
            folds_generation: 0,
            synced_scroll: (0., 0.),
            top_anchor: None,
            jumps: Vec::new(),
            jumps_fwd: Vec::new(),
            scroll_top: 0.,
            follow_log_tail: false,
            follow_log_paused_caret: None,
            scroll_anim: None,
            scroll_left: 0.,
            follow_caret: false,
            focus_handle: cx.focus_handle(),
            layout: None,
            drag: None,
            autoscrolling: false,
            marked: Vec::new(),
            overtype_composition: None,
            literal_next: None,
            hover: None,
            scroll_bar: None,
            highlight_job: None,
            column_job: None,
            fold_job: None,
            html_job: None,
            highlight_backoff: None,
            signature_help: None,
            signature_pending: None,
            signature_inflight: None,
            signature_triggers: Default::default(),
            completion: None,
            snippet_choice: None,
            completion_epoch: 0,
            completion_index: None,
            completion_recent: Default::default(),
            definition_hover: None,
            open_views: None,
            words_cache: None,
        }
    }

    /// Navigation ignores identifiers over 256 chars; widen if real names need it.
    pub(crate) fn navigation_word(&self, at: usize, cx: &App) -> Option<(Range<usize>, String)> {
        let s = self.read_state(cx);
        let at = at.min(s.len());
        let word = |chars: ropey::iter::Chars| (chars.take(257)).take_while(|c| EditorState::is_word_char(*c)).count();
        let back = word(s.doc.rope.chars_at(at).reversed());
        let ahead = word(s.doc.rope.chars_at(at));
        let range = at - back..at + ahead;
        if range.is_empty() || range.len() > 256 {
            return None;
        }
        let name = s.doc.rope.slice(range.clone()).to_string();
        name.starts_with(|c: char| c == '_' || c.is_alphabetic()).then_some((range, name))
    }

    pub fn title(&self, cx: &App) -> String {
        if self.results {
            return "Find Results".into();
        }
        if let Some(label) = &self.label {
            return label.clone();
        }
        (self.doc(cx).path.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into())
    }

    pub fn text(&self, cx: &App) -> String {
        self.doc(cx).text()
    }

    pub fn language_current(&self, cx: &App) -> bool {
        self.language_version
            .as_ref()
            .is_some_and(|(path, version)| self.doc(cx).path.as_ref() == Some(path) && self.doc(cx).version == *version)
    }

    /// Panels keep Sublime's fixed 12px; the editor zooms with cmd-+/-.
    fn font_size(&self) -> f32 {
        if self.single_line {
            theme::EDITOR_FONT_SIZE
        } else {
            self.view.font_size
        }
    }

    /// Menlo 12 gives Sublime's 15px rows; other sizes keep that ratio.
    pub fn line_height(&self) -> f32 {
        (self.font_size() * 1.25).round()
    }

    /// Colour the lines `from..=to` on the highlighting thread (they draw plain meanwhile);
    /// an edit meanwhile keeps the result above it. One job at a time.
    pub(crate) fn highlight_in_background(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let waiting = (self.highlight_backoff).is_some_and(|(until, _)| Instant::now() < until);
        if self.highlight_job.is_some() || waiting {
            return;
        }
        let mut job = self.highlighter.job(from);
        // dropped with the task, when this view closes: the job stops at its next line
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        job.cancel_on(cancel.0.clone());
        let rope = self.doc(cx).rope.clone();
        self.highlight_job = Some(cx.spawn(async move |this, cx| {
            let _cancel = cancel;
            let job = highlight::on_worker(move || highlight::run_job(job, &rope, &[from, to])).await;
            this.update(cx, |e, cx| {
                match job {
                    Some(job) => {
                        e.highlighter.adopt(job, &e.doc(cx).rope);
                        e.highlight_backoff = None;
                    }
                    // a job that panics would every frame: wait 1 s, then 2, 4 ... a minute
                    None => {
                        let wait = e
                            .highlight_backoff
                            .map_or(Duration::from_secs(1), |b| (b.1 * 2).min(Duration::from_secs(60)));
                        e.highlight_backoff = Some((Instant::now() + wait, wait));
                    }
                }
                e.highlight_job = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Tests: fold `header..=last` in this view.
    #[cfg(test)]
    pub fn fold_lines(&mut self, header: usize, last: usize, cx: &mut Context<Self>) {
        self.change_folds(cx, |f, d| f.fold_lines(d, header, last));
    }

    /// Tests: the lines this view's folds hide.
    #[cfg(test)]
    pub fn hidden_lines(&self, cx: &App) -> Vec<Range<usize>> {
        self.folds.hidden_lines(self.doc(cx))
    }

    /// Tests: put in these folds as a fold command does, the wrapped rows laid out again.
    #[cfg(test)]
    pub(crate) fn set_folds_now(&mut self, folds: Folds, cx: &App) {
        let before = std::mem::replace(&mut self.folds, folds);
        self.refold(&before, cx);
    }

    /// Tests: fold `chars` in place of this view's folds, as a fold command does.
    #[cfg(test)]
    pub(crate) fn fold_chars_now(&mut self, chars: impl IntoIterator<Item = Range<usize>>, cx: &App) {
        let mut folds = Folds::default();
        for r in chars {
            folds.fold(self.doc(cx), r);
        }
        self.set_folds_now(folds, cx);
    }

    /// Tests: fold lines `header + 1 ..= last` as a fold command does.
    #[cfg(test)]
    pub(crate) fn fold_lines_now(&mut self, header: usize, last: usize, cx: &App) {
        let mut folds = self.folds.clone();
        folds.fold_lines(self.doc(cx), header, last);
        self.set_folds_now(folds, cx);
    }

    /// Tests: wrap every line still waiting to be wrapped at the view's width, as the frames
    /// would a slice at a time.
    #[cfg(test)]
    pub(crate) fn settle_wrap(&mut self, cx: &App) {
        if let Some(w) = &mut self.wrap {
            while !w.settle(doc_of(&self.document, cx), Instant::now() + Duration::from_secs(1), &self.folds) {}
        }
    }

    /// Tests: the display row of the primary caret.
    #[cfg(test)]
    pub fn caret_row(&self, cx: &App) -> usize {
        self.rows().row_of(self.doc(cx), self.state.cursor()).row.0
    }

    /// Tests: the rows drawn, filler rows included.
    #[cfg(test)]
    pub fn display_rows(&self, cx: &App) -> usize {
        self.rows().rows(self.doc(cx))
    }

    /// Tests: where the last frame drew the view, its text's left edge and a space's width.
    #[cfg(test)]
    pub fn drawn_frame(&self) -> Option<(gpui::Bounds<Pixels>, Pixels, Pixels)> {
        let l = self.layout.as_ref()?;
        Some((l.bounds, l.text_left, l.space_w))
    }

    /// Comparison backgrounds emitted by the last frame: chars, rectangle, color, inline.
    #[cfg(test)]
    pub fn comparison_backgrounds(&self) -> Vec<(Range<usize>, gpui::Bounds<Pixels>, Hsla, bool)> {
        self.layout.as_ref().map_or_else(Vec::new, |l| l.drawn.comparison.clone())
    }

    #[cfg(test)]
    pub fn comparison_range_visits(&self) -> usize {
        self.layout.as_ref().map_or(0, |l| l.drawn.comparison_visits)
    }

    /// Tests: a Copy as HTML is being built.
    #[cfg(test)]
    pub fn html_copying(&self) -> bool {
        self.html_job.is_some()
    }

    /// Tests: a highlighting job is under way, and whether jobs wait after a panic.
    #[cfg(test)]
    pub fn highlighting(&self) -> (bool, bool) {
        (self.highlight_job.is_some(), self.highlight_backoff.is_some())
    }

    pub fn minimap_line_height(&self) -> f32 {
        self.line_height() * theme::MINIMAP_LINE
    }

    /// Take text read from disk (File > Revert, an outside change) as one undoable step;
    /// carets stay on their text and the view does not scroll.
    pub fn reload(&mut self, loaded: Loaded, changes: &[Change], cx: &mut Context<Self>) {
        let scroll = (self.scroll_top, self.scroll_left);
        self.edit(cx, |s| s.reload_from(loaded, changes));
        self.document.update(cx, |d, _| d.file_autosave_loaded());
        (self.scroll_top, self.scroll_left) = scroll;
        self.follow_caret = false;
    }

    /// Another view of this document with the same selections, scroll, syntax and display
    /// settings: it adds view state and a subscription, not another document.
    pub fn another_view(&self, cx: &mut Context<Editor>) -> Self {
        let view = self.state.duplicate(self.doc(cx));
        let mut e = Editor::of_document(self.document.clone(), view, self.single_line, cx);
        e.input_enabled = self.input_enabled;
        // its colours too: the new view opens on the same lines
        e.highlighter = self.highlighter.clone();
        e.changes = self.changes.clone();
        e.changes_version = self.changes_version;
        e.language_version = self.language_version.clone();
        e.diagnostics = self.diagnostics.clone();
        e.inline_diagnostics = self.inline_diagnostics.clone();
        e.semantic_spans = self.semantic_spans.clone();
        e.lsp_status = self.lsp_status.clone();
        e.view = self.view.clone();
        e.modelines = self.modelines.clone();
        e.modelines_enabled = self.modelines_enabled;
        e.modeline_language = self.modeline_language.clone();
        e.wrap_override = self.wrap_override;
        e.spell_check_override = self.spell_check_override;
        #[cfg(test)]
        {
            e.spell_checker = self.spell_checker.clone();
        }
        e.wrap_width_override = self.wrap_width_override;
        // a view into Find Results is one too: its title and double click (the document has
        // the outlines)
        e.results = self.results;
        e.playground = self.playground;
        e.playground_source = self.playground_source.clone();
        e.label = self.label.clone();
        e.scroll_left = self.scroll_left;
        // scroll_top counts this view's display rows; the new view has none yet, so it opens
        // at the same text instead
        e.top_anchor = Some(self.top(cx));
        e
    }

    /// `f` after each change to the text of `editor`'s document, as a panel field is typed in.
    pub(crate) fn on_text_change<T: 'static>(
        editor: &Entity<Editor>,
        cx: &mut Context<T>,
        mut f: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Subscription {
        let doc = editor.read(cx).document.clone();
        cx.subscribe(&doc, move |this, _, ev: &DocumentEvent, cx| {
            if matches!(ev, DocumentEvent::Edited(_)) {
                f(this, cx)
            }
        })
    }

    /// `on_text_change` with the window.
    pub(crate) fn on_text_change_in<T: 'static>(
        editor: &Entity<Editor>,
        window: &Window,
        cx: &mut Context<T>,
        mut f: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let doc = editor.read(cx).document.clone();
        cx.subscribe_in(&doc, window, move |this, _, ev: &DocumentEvent, window, cx| {
            if matches!(ev, DocumentEvent::Edited(_)) {
                f(this, window, cx)
            }
        })
    }

    /// Whether `other` is a view of the same document.
    pub fn same_document(&self, other: &Editor) -> bool {
        self.document == other.document
    }

    /// The document this view shows.
    pub fn doc<'a>(&self, cx: &'a App) -> &'a Document {
        doc_of(&self.document, cx)
    }

    /// This view and its document, to read.
    pub fn read_state<'a>(&'a self, cx: &'a App) -> ViewRead<'a> {
        read_of(&self.document, &self.state, cx)
    }

    /// This view's selections, to change in its document's text as it stands.
    pub fn sel<'a>(&'a mut self, cx: &'a App) -> ViewSelect<'a> {
        sel_of(&self.document, &mut self.state, cx)
    }

    /// `f` on this view's selections with its rows and how they are drawn, for the motions that
    /// keep to an x.
    fn with_rows<R>(
        &mut self,
        window: &Window,
        cx: &App,
        f: impl FnOnce(&mut ViewSelect, &Padded<Folded>, &measure::Shaped) -> R,
    ) -> R {
        let rows = view_rows(self.wrap.as_ref(), &self.folds, &self.fillers);
        let measure = measure::Shaped::new(&rows, self.wrap.is_some(), self.layout.as_ref(), window);
        f(&mut sel_of(&self.document, &mut self.state, cx), &rows, &measure)
    }

    /// Change how the document is written or its file (not its text): every view of it, and
    /// the windows showing it, hear of it.
    pub fn update_doc<R>(&mut self, cx: &mut App, f: impl FnOnce(&mut Document) -> R) -> R {
        self.document.update(cx, |d, cx| d.update_file(f, cx))
    }

    /// The document changed: another view's edit is applied here once, at the version it
    /// came from, and the drawing follows.
    fn on_document(&mut self, ev: &DocumentEvent, cx: &mut Context<Self>) {
        match ev {
            DocumentEvent::Edited(_) => self.catch_up(cx),
            DocumentEvent::File { renamed, .. } => {
                // Save As in another view: the new extension may pick another syntax
                if *renamed {
                    self.on_path_changed(cx);
                }
                cx.notify();
            }
            DocumentEvent::Saved { .. } => cx.notify(),
            // found by whichever window looked; drawn while this view shows that version
            DocumentEvent::Changes { version, hunks, .. } => {
                self.changes = hunks.to_vec();
                self.changes_version = *version;
                cx.notify();
            }
        }
    }

    /// Apply the other views' edits this view has not, keeping its carets, folds, wrapping and
    /// jumps on their text, and re-highlight from the first changed line. Emits nothing.
    pub(crate) fn catch_up(&mut self, cx: &mut Context<Self>) {
        if !self.doc(cx).is_following() {
            self.follow_log_paused_caret = None;
            let caught = self.sel(cx).catch_up();
            self.followed(&caught, cx);
            return;
        }
        let tail = self.follow_log_tail
            && self.state.selections.len() == 1
            && self.state.selections[0].is_empty()
            && !self.state.selections[0].has_virtual();
        let cursor = self.state.cursor();
        let caught = self.sel(cx).catch_up();
        let delta: isize =
            caught.iter().flat_map(|c| &c.changes).map(|c| c.inserted as isize - c.removed as isize).sum();
        let old_end = self.doc(cx).len() as isize - delta;
        self.followed(&caught, cx);
        if tail && !caught.is_empty() && cursor as isize == old_end {
            self.follow_log_end(cx);
        }
    }

    pub(crate) fn follow_log_end(&mut self, cx: &mut Context<Self>) {
        self.follow_log_paused_caret = None;
        let end = self.doc(cx).len();
        self.sel(cx).set_cursor(end);
        self.follow_log_tail = true;
        self.scroll_to_cursor(cx);
        cx.notify();
    }

    /// What another view's edits change in this one's drawing.
    fn followed(&mut self, caught: &[std::rc::Rc<TextChange>], cx: &mut Context<Self>) {
        self.clear_stale_occurrences(cx);
        let me = self.state.view_id();
        let changes: Vec<Change> =
            (caught.iter()).filter(|c| c.origin != me).flat_map(|c| c.changes.iter().copied()).collect();
        if caught.is_empty() {
            return;
        }
        self.invalidate_spelling();
        // another view's edit ends a composition here; its ranges would point at other text
        self.marked.clear();
        self.completion = None;
        self.close_definition_hover(cx);
        self.map_jumps(&changes);
        self.map_folds(&changes, cx);
        let doc = &self.document.read(cx).model;
        if let Some(w) = &mut self.wrap {
            w.update(doc, &changes, &self.folds);
        }
        self.fillers.map(&changes);
        if let Some(line) = self.state.take_dirty_line() {
            self.highlighter.invalidate_from(line);
        }
        self.highlighter.text_changed(&changes);
        cx.notify();
    }

    /// The text on top: the first visible row's first char and how far into that row (what a
    /// session stores).
    pub fn top(&self, cx: &App) -> (usize, f32) {
        self.top_anchor
            .unwrap_or_else(|| top_anchor(&self.rows(), self.doc(cx), self.scroll_top / self.line_height() as f64))
    }

    /// Keep the text now on top there through a change of font, width or wrapping.
    pub fn hold_top(&mut self, cx: &App) {
        self.top_anchor = Some(self.top(cx));
    }

    /// Put `top` on top at the next paint, which clamps it to a file that may have shrunk.
    pub fn set_top(&mut self, top: (usize, f32)) {
        self.top_anchor = Some(top);
    }

    /// Word wrap as this view shows it: View > Word Wrap's choice, else the setting, where
    /// "auto" wraps everything but source code.
    pub fn wraps(&self) -> bool {
        !self.single_line && self.wrap_override.unwrap_or_else(|| self.view.word_wrap.wraps(&self.highlighter.scope()))
    }

    /// Where the view is drawn: `scroll_top`, or on the way to it while a page animates.
    pub fn drawn_scroll(&self) -> f64 {
        match self.scroll_anim {
            Some((from, t0)) => scroll_at(from, self.scroll_top, t0.elapsed()),
            None => self.scroll_top,
        }
    }

    /// A page scroll to `to`: animated from where the view is drawn now (so a second Page Down
    /// mid-way carries on from there), or at once.
    fn scroll_page_to(&mut self, to: f64) {
        let from = self.drawn_scroll();
        self.scroll_top = to;
        self.scroll_anim = (self.view.animate && from != to).then(|| (from, Instant::now()));
    }

    /// The scroll position other views may follow: the top line and how far into it,
    /// and the sideways scroll.
    pub fn scroll_position(&self, cx: &App) -> (usize, f32, f32) {
        let (top, into) = self.top(cx);
        (self.doc(cx).point(top).0, into, self.scroll_left)
    }

    /// Follow another view's scroll: its top line (clamped to this shorter or longer text) and
    /// its sideways scroll. Emits nothing, so followers never echo.
    pub fn follow_scroll(&mut self, line: Option<(usize, f32)>, left: Option<f32>, cx: &App) {
        if let Some((line, into)) = line {
            let line = line.min(self.doc(cx).line_count() - 1);
            self.scroll_anim = None;
            self.set_top((self.doc(cx).line_start(line), into));
        }
        if let Some(left) = left {
            self.scroll_left = left.max(0.);
        }
    }

    /// Direct scrolling (wheel, trackpad, scroll bar) ends a page animation where it is drawn,
    /// so the view does not jump and nothing smooths the trackpad's own momentum.
    fn stop_scroll_anim(&mut self) {
        if self.scroll_anim.is_some() {
            self.scroll_top = self.drawn_scroll();
            self.scroll_anim = None;
        }
    }

    /// The most columns a wrapped row holds, 0 for the view's width: the view's Word Wrap
    /// Column, else the window's `wrap_width`.
    pub fn wrap_cols(&self) -> usize {
        self.wrap_width_override.unwrap_or(self.view.wrap_width)
    }

    /// The display rows motion, clicks, scrolling and painting go by.
    fn rows(&self) -> Padded<'_, Folded<'_>> {
        view_rows(self.wrap.as_ref(), &self.folds, &self.fillers)
    }

    /// This view's filler rows.
    pub fn fillers(&self) -> &Fillers {
        &self.fillers
    }

    /// The rows below the filler rows: what an aligned pair measures to place them.
    pub(crate) fn unpadded_rows(&self) -> Folded<'_> {
        Folded::new(self.wrap.as_ref(), &self.folds)
    }

    /// Whether this view's rows are wrapped now (a wrapped view builds its map at its first
    /// paint).
    pub(crate) fn wrapped_now(&self) -> bool {
        self.wrap.is_some()
    }

    /// Whether every line is wrapped at the view's width yet (after a resize the frames wrap
    /// the rest a slice at a time).
    pub(crate) fn wrap_settled(&self) -> bool {
        self.wrap.as_ref().is_none_or(WrapMap::settled)
    }

    /// Session capture only: fingerprint changed text once, never during rendering.
    pub(crate) fn capture_folds(&self, cx: &App) -> Option<orlo_workspace::sessions::folds::FoldState> {
        let doc = self.doc(cx);
        // A sibling's edit notification may still be queued: old offsets must never be
        // fingerprinted as belonging to the new text.
        if self.state.revision() != doc.version {
            return None;
        }
        self.session_fold_cache.borrow_mut().capture(doc, self.view_filter.as_ref().map_or(&self.folds, |f| &f.prior))
    }

    pub fn folds(&self) -> &Folds {
        &self.folds
    }

    /// Folds an aligned pair carries over from the other side: as a fold command puts them in,
    /// but the pair is not told of them again.
    pub(crate) fn set_folds_quietly(&mut self, folds: Folds, cx: &mut Context<Self>) {
        self.clear_view_filter(cx);
        self.replace_folds(cx, |f, _| *f = folds);
    }

    /// What the rows are drawn from: the text, the folds and the wrapping.
    fn rows_key_now(&self, cx: &App) -> (u64, u64, Option<u64>) {
        let wrap = self.wrap.as_ref().map(WrapMap::generation);
        (self.doc(cx).version, self.folds_generation, wrap)
    }

    /// The scroll as rows rather than pixels, for a pair whose line height may differ: the
    /// top row, a Page Up/Down glide's start row and when it started, and the sideways scroll.
    pub(crate) fn scroll_rows(&self) -> (f64, Option<(f64, Instant)>, f32) {
        let lh = self.line_height() as f64;
        let anim = self.scroll_anim.map(|(from, t0)| (from / lh, t0));
        (self.scroll_top / lh, anim, self.scroll_left)
    }

    /// Take the other side's scroll, to the same display row, gliding as it glides. Emits
    /// nothing, so the other side is not told back.
    pub(crate) fn follow_scroll_rows(&mut self, (top, anim, left): (f64, Option<(f64, Instant)>, f32)) {
        let lh = self.line_height() as f64;
        self.top_anchor = None;
        self.scroll_top = top * lh;
        self.scroll_anim = anim.map(|(from, t0)| (from * lh, t0));
        self.scroll_left = left;
        self.synced_scroll = (self.scroll_top, self.scroll_left);
    }

    /// The scroll as it is now has been told to the pair.
    pub(crate) fn scroll_synced(&mut self) {
        self.synced_scroll = (self.scroll_top, self.scroll_left);
    }

    /// Put filler rows between this view's lines, keeping the text on top where it is; an
    /// aligned view keeps its row instead, which its pair holds level with the other side's.
    pub fn set_fillers(&mut self, fillers: Fillers, cx: &mut Context<Self>) {
        if fillers != self.fillers {
            if !self.aligned {
                self.hold_top(cx);
            }
            self.fillers = fillers;
            cx.notify();
        }
    }

    /// Open the folds a selection end landed strictly inside (Find, Goto, Undo Selection, undo
    /// and redo, another view's edit), keeping the text on top where it is. Motions and the
    /// fold commands never leave one there. Runs every frame, before layout, and before and after
    /// each command `perform_action` runs (a macro or an extension runs no frame between them).
    fn reveal_selections(&mut self, cx: &mut Context<Self>) {
        self.refresh_view_filter(cx);
        if self.view_filter.is_some() && !self.marked.is_empty() {
            return;
        }
        if self.filter_hides_selections() {
            self.reveal_view_filter(cx);
            self.status("Filter cleared to reveal the source selection", cx);
        }
        if self.folds.is_empty() {
            return;
        }
        let ends: Vec<usize> = (self.state.selections.iter()).flat_map(|s| [s.start(), s.end()]).collect();
        // every frame asks: only a selection end in folded text costs more than a lookup
        if !self.folds.hides_any(&ends) {
            return;
        }
        if self.view_filter.is_some() {
            self.reveal_view_filter(cx);
            self.status("Filter cleared to reveal the source selection", cx);
        }
        let top = self.top(cx);
        let before = self.folds.clone();
        if self.folds.reveal(&ends) {
            self.top_anchor = Some(top);
            self.refold(&before, cx);
            self.folds_changed(cx);
        }
    }

    /// The folds changed from `before`: the wrapped rows of the display lines that changed
    /// are laid out again.
    fn refold(&mut self, before: &Folds, cx: &App) {
        self.folds_generation += 1;
        if let Some(w) = &mut self.wrap {
            let doc = doc_of(&self.document, cx);
            let lines = Folds::changed_lines(before, &self.folds, doc);
            w.refold(doc, &self.folds, &lines);
        }
    }

    /// The folds follow edits to the text.
    fn map_folds(&mut self, changes: &[Change], cx: &App) {
        self.map_view_filter(changes, cx);
        if !self.folds.is_empty() {
            self.folds.map(changes);
            self.folds_generation += 1;
        }
    }

    /// Tell aligned views and workspace persistence that this view's folds changed.
    fn folds_changed(&self, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::FoldsChanged);
    }

    /// Change this view's folds, keeping the text on top where it is; a Fold Level still
    /// working on another thread gives way. Selection ends now strictly inside hidden text go
    /// back to their header's end (left there, the next frame would open the fold); one at a
    /// fold's end stays, so a selection over the fold keeps it all.
    fn change_folds(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Folds, &Document)) {
        if self.single_line {
            return;
        }
        self.clear_view_filter(cx);
        self.replace_folds(cx, f);
        self.folds_changed(cx);
    }

    /// `change_folds` without telling an aligned pair.
    fn replace_folds(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Folds, &Document)) {
        self.fold_job = None;
        self.hold_top(cx);
        let before = self.folds.clone();
        f(&mut self.folds, &self.document.read(cx).model);
        self.refold(&before, cx);
        let sels = &self.state.selections;
        // sorted and disjoint: their starts and ends are one sorted list
        let mut ends: Vec<usize> = sels.iter().flat_map(|s| [s.start(), s.end()]).collect();
        self.folds.out_of_hidden(&mut ends);
        let rows = self.rows();
        let keep_gap = |at: usize, before: usize, gap: usize| {
            if gap > 0
                && at == before
                && rows.row_bounds(self.doc(cx), rows.row_of(self.doc(cx), at).row).is_some_and(|r| r.end == at)
            {
                gap
            } else {
                0
            }
        };
        let sels = (sels.iter().zip(ends.chunks(2)))
            .map(|(s, e)| {
                let (anchor, head) = match s.head < s.anchor {
                    true => (e[1], e[0]),
                    false => (e[0], e[1]),
                };
                Selection {
                    anchor,
                    head,
                    anchor_virtual: keep_gap(anchor, s.anchor, s.anchor_virtual),
                    head_virtual: keep_gap(head, s.head, s.head_virtual),
                    ..*s
                }
            })
            .collect();
        self.sel(cx).reselect(sels);
        cx.notify();
    }

    /// Line selections (Expand Selection to Line, a triple click) take in a fold they reach
    /// instead of ending inside it, which would open it: an end goes past the fold to the end
    /// of the line it ends on, its line break included; a start to its header's line.
    pub(crate) fn lines_over_folds(&mut self, cx: &App) {
        if self.folds.is_empty() {
            return;
        }
        let rows = view_rows(self.wrap.as_ref(), &self.folds, &self.fillers);
        let s = self.read_state(cx);
        let sels = (s.view.selections.iter())
            .map(|x| {
                let start = match rows.skip(s.doc, x.start(), false) {
                    o if o == x.start() => o,
                    header => s.line_start(s.point(header).0),
                };
                let end = match rows.skip(s.doc, x.end(), true) {
                    o if o == x.end() => o,
                    past => s.line_range_at(past).end,
                };
                match x.head < x.anchor {
                    true => Selection::new(end, start),
                    false => Selection::new(start, end),
                }
            })
            .collect();
        self.sel(cx).reselect(sels);
    }

    /// Move the caret for a goto command and remember where it came from.
    pub fn jump_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.marked.clear();
        self.record_jump(self.state.cursor());
        self.sel(cx).set_cursor(offset);
        self.after_edit(cx);
    }

    pub fn record_jump(&mut self, from: usize) {
        self.jumps.push(from);
        self.jumps_fwd.clear();
    }

    /// Find Results streaming in: text at the end with its matches outlined; the caret and the
    /// scroll stay where they are.
    pub fn append(&mut self, text: &str, outlines: impl IntoIterator<Item = Range<usize>>, cx: &mut Context<Self>) {
        self.catch_up(cx);
        let doc = self.document.clone();
        let (_, caught, change) =
            doc.update(cx, |d, cx| d.edit(&mut self.state, |s| s.append_outlined(text, outlines), cx));
        self.followed(&caught, cx);
        self.present(&change.map_or_else(Vec::new, |c| c.changes.clone()), cx);
        cx.notify();
    }

    /// Replace the content and select all of it (so typing replaces it, like Sublime's panels).
    /// The document stays the same one, with no undo history.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.occurrences = None;
        self.words_cache = None;
        self.invalidate_spelling();
        // These ranges belong to the old text, not the replacement panel text.
        self.marked.clear();
        self.completion = None;
        self.close_definition_hover(cx);
        let doc = self.document.clone();
        doc.update(cx, |d, cx| d.edit(&mut self.state, |s| s.reset_text(text), cx));
        self.wrap = None;
        self.view_filter = None;
        self.cancel_filter_job();
        self.folds = Folds::default();
        self.folds_generation += 1;
        self.fillers = Fillers::default();
        self.fold_job = None;
        self.changes.clear();
        self.comparison = None;
        self.follow_caret = true;
        self.highlighter.invalidate_from(0);
        cx.notify();
    }

    pub fn status(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::Status(text.into()));
    }

    /// 1-based, clamped.
    pub fn goto_line(&mut self, line: usize, cx: &mut Context<Self>) {
        self.marked.clear();
        let l = line.clamp(1, self.doc(cx).line_count()) - 1;
        sel_of(&self.document, &mut self.state, cx).set_cursor(doc_of(&self.document, cx).line_start(l));
        self.after_edit(cx);
    }

    /// Selections and scroll as they were before a preview.
    pub fn restore_view(&mut self, sels: Vec<Selection>, top: (usize, f32), cx: &mut Context<Self>) {
        self.marked.clear();
        self.sel(cx).set_selections(sels);
        self.after_edit(cx);
        self.set_top(top);
        self.follow_caret = false;
    }

    pub fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.marked.clear();
        self.sel(cx).set_selection(range.start, range.end);
        self.after_edit(cx);
    }

    pub fn on_path_changed(&mut self, cx: &App) {
        self.invalidate_spelling();
        self.changes.clear();
        self.highlighter = Highlighter::detect(self.doc(cx).path.as_deref(), &head(&self.doc(cx).rope));
        self.modeline_language = None;
        // Find Results saved as a file is an ordinary file from then on
        self.results = false;
        self.label = None;
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    pub fn scroll_to_cursor(&mut self, cx: &App) {
        self.follow_caret = true;
        self.follow_log_tail = self.state.selections.len() == 1
            && self.state.selections[0].is_empty()
            && self.state.cursor() == self.doc(cx).len();
        let Some(layout) = &self.layout else { return };
        let lh = self.line_height() as f64;
        let height = f32::from(layout.bounds.size.height) as f64;
        let y = self.rows().row_of(self.doc(cx), self.state.cursor()).row.0 as f64 * lh;
        if y < self.scroll_top {
            self.scroll_top = y;
        } else if y + lh > self.scroll_top + height {
            self.scroll_top = y + lh - height;
        }
        self.scroll_top = self.scroll_top.max(0.);
    }

    /// Bring highlighting, wrapping and jumps up to `changes`, this view's own edits.
    fn present(&mut self, changes: &[Change], cx: &App) {
        self.clear_stale_occurrences(cx);
        let Some(line) = self.state.take_dirty_line() else {
            return;
        };
        self.invalidate_spelling();
        self.highlighter.invalidate_from(line);
        self.highlighter.text_changed(changes);
        self.map_folds(changes, cx);
        if let Some(w) = &mut self.wrap {
            w.update(&self.document.read(cx).model, changes, &self.folds);
        }
        self.fillers.map(changes);
        self.map_jumps(changes);
        if let Some(c) = &mut self.completion {
            c.map(changes);
        }
    }

    /// After a command: scroll to the caret and refresh the completion list. Its text change,
    /// if any, went through `change`, which published it and brought the drawing up to it.
    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.clear_stale_occurrences(cx);
        self.dismiss_signature();
        self.completion_epoch += 1;
        self.cancel_spelling_action();
        if self.single_line {
            if !self.multiline {
                self.scroll_top = 0.;
            }
            // a panel field still scrolls sideways to its caret when the query is wide
            self.follow_caret = true;
        } else {
            self.scroll_to_cursor(cx);
        }
        self.sync_completion(cx);
        cx.notify();
    }

    /// The user scrolled the view (not a follower's sync): the popup closes and synchronized
    /// groups follow.
    fn scrolled(&mut self, cx: &mut Context<Self>) {
        self.close_snippet_choices(cx);
        let bottom = self.scroll_top + 1. >= self.max_scroll(cx);
        if self.doc(cx).is_following() {
            self.sel(cx).sel_checkpoint();
            let generation = self.read_state(cx).selection_generation();
            if self.follow_log_paused_caret.is_some_and(|held| held != generation) {
                self.follow_log_paused_caret = None;
            }
            if !bottom
                && self.follow_log_tail
                && self.state.selections.len() == 1
                && self.state.selections[0].is_empty()
                && !self.state.selections[0].has_virtual()
                && self.state.cursor() == self.doc(cx).len()
            {
                self.follow_log_paused_caret = Some(generation);
            }
            if bottom && self.follow_log_paused_caret.take() == Some(generation) {
                // Ingestion preserves this generation; explicit review expires it.
                self.follow_log_end(cx);
            }
        }
        self.follow_log_tail = bottom;
        self.dismiss_signature();
        self.completion_epoch += 1;
        self.completion = None;
        self.close_definition_hover(cx);
        cx.emit(EditorEvent::Scrolled);
    }

    /// Jump Back and Forward land on the same text after edits.
    fn map_jumps(&mut self, changes: &[Change]) {
        for j in self.jumps.iter_mut().chain(&mut self.jumps_fwd) {
            *j = crate::editing::text::map_offset(*j, changes);
        }
    }

    /// Mariana's background, or the dimmed one in an inactive group.
    pub(crate) fn background(&self) -> Hsla {
        match self.dimmed {
            true => ui(theme::INACTIVE_EDITOR_BG),
            false => c(theme::EDITOR_BG),
        }
    }

    /// One row, without scrolling: a field that cannot hold line breaks.
    pub(crate) fn fixed_rows(&self) -> bool {
        self.single_line && !self.multiline
    }

    /// The caret's line is the first (`up`) or the last: Up/Down in a field recall history
    /// there and move between lines elsewhere.
    pub fn caret_at_edge(&self, up: bool, cx: &App) -> bool {
        let line = self.doc(cx).point(self.state.cursor()).0;
        match up {
            true => line == 0,
            false => line + 1 >= self.doc(cx).line_count(),
        }
    }

    /// A motion from a workspace command (Up/Down in a multiline field).
    pub fn move_caret(&mut self, motion: crate::editing::text::Motion, window: &Window, cx: &mut Context<Self>) {
        self.sel(cx).sel_checkpoint();
        self.marked.clear();
        self.with_rows(window, cx, |s, rows, m| s.move_cursors_in(motion, false, rows, m));
        self.after_edit(cx);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// User text input and mutating commands, excluding immutable snapshot viewers.
    fn edit_text(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ViewEdit)) {
        if self.input_enabled && !self.doc(cx).edits_blocked() {
            self.edit(cx, f);
        }
    }

    /// A command's edit through the document, then highlighting, scrolling and the other views
    /// brought up to it; it ends any IME composition (the marked text stays as typed).
    pub fn edit<R>(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ViewEdit) -> R) -> R {
        self.marked.clear();
        let r = self.change(cx, |s| {
            // the selections as they stood before this command are a state Undo Selection can
            // reach
            s.sel_checkpoint();
            f(s)
        });
        self.after_edit(cx);
        r
    }

    fn virtual_padding_tabs(&self, cx: &App) -> bool {
        let doc = self.doc(cx);
        if !self.state.virtual_space.enabled() || doc.translate_tabs {
            return true;
        }
        if !self.state.selections.iter().any(Selection::has_virtual) {
            // Native replay restores the original virtual targets inside the edit closure.
            return self.wrap.is_none() && self.folds.is_empty();
        }
        let rows = self.rows();
        self.state.selections.iter().all(|sel| {
            [sel.anchor_point(), sel.head_point()].into_iter().all(|(at, gap)| {
                gap == 0 || {
                    let physical = crate::editing::display::col_after(
                        doc.rope.slice(doc.line_start(doc.point(at).0)..at),
                        doc.tab_size,
                    );
                    physical % doc.tab_size == rows.row_of(doc, at).col.0 % doc.tab_size
                }
            })
        })
    }

    /// `f` through the document: this view first applies what other views changed, then its own
    /// change is published to them and its drawing brought up to it. Scrolling and the
    /// completion list are the caller's (`after_edit`).
    pub(crate) fn change<R>(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ViewEdit) -> R) -> R {
        // other views' edits first, drawn against the text they led to
        self.catch_up(cx);
        let tabs = self.virtual_padding_tabs(cx);
        let doc = self.document.clone();
        let (r, caught, change) = doc.update(cx, |d, cx| {
            d.edit(
                &mut self.state,
                |s| {
                    s.set_virtual_padding_tabs(tabs);
                    let result = f(s);
                    s.set_virtual_padding_tabs(true);
                    result
                },
                cx,
            )
        });
        self.followed(&caught, cx);
        if let Some(c) = change {
            self.present(&c.changes, cx);
        }
        r
    }

    /// Save-time cleanup in this view, through the document so every other view follows; the
    /// view stays where the user left it. Whether it changed the text.
    pub fn clean_for_save(&mut self, c: &Cleanup, cx: &mut Context<Self>) -> bool {
        self.catch_up(cx);
        let doc = self.document.clone();
        let (cleaned, caught, change) = doc.update(cx, |d, cx| {
            d.edit(
                &mut self.state,
                |s| {
                    s.sel_checkpoint();
                    s.cleanup_for_save(c)
                },
                cx,
            )
        });
        self.followed(&caught, cx);
        if let Some(c) = change {
            // the text under a composition just changed
            self.marked.clear();
            self.present(&c.changes, cx);
            cx.notify();
        }
        cleaned
    }
}

impl EventEmitter<EditorEvent> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // catches the changes workspace commands made (find, goto); a drag counts once, at its press
        if self.drag.is_none() {
            self.sel(cx).sel_checkpoint();
        }
        self.reveal_selections(cx);
        self.refresh_completion(window, cx);
        self.validate_signature_context(window, cx);
        self.sel(cx).check_snippet();
        self.refresh_snippet_choices(window, cx);
        self.refresh_occurrences(cx);
        let (font, size, fg, bg) = match (self.single_line, self.ui_font) {
            (false, _) => (self.view.font_face.clone(), self.view.font_size, c(theme::EDITOR_FG), self.background()),
            (true, false) => {
                (theme::EDITOR_FONT.into(), theme::EDITOR_FONT_SIZE, ui(theme::WIDGET_FG), ui(theme::WIDGET_BG))
            }
            // the palette's input lies on the palette itself, as Zed's does
            (true, true) => (theme::UI_FONT.into(), 13., ui(theme::WIDGET_FG), gpui::transparent_black()),
        };
        let mut context = gpui::KeyContext::new_with_defaults();
        context.add("Editor");
        context.set("syntax", self.highlighter.scope().replace('.', "-"));
        let flags = [
            (self.single_line, "single_line"),
            (self.multiline, "multiline"),
            (self.label.as_deref() == Some("Shell Worksheet"), "worksheet"),
            (self.state.selections.iter().any(|s| !s.is_empty()), "selection"),
            (self.completing(cx), "completing"),
            (self.snippet_choice.is_some(), "snippet_choice"),
            (
                (self.signature_help.is_some() || self.signature_pending.is_some()) && self.marked.is_empty(),
                "signature",
            ),
            // a composition's keys are the input method's
            (self.state.in_snippet() && self.marked.is_empty(), "snippet"),
        ];
        for (_, flag) in flags.into_iter().filter(|f| f.0) {
            context.add(flag);
        }
        let d = div()
            .size_full()
            // the find field and the palette: line commands are not bound there
            .key_context(context)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .bg(bg)
            .font_family(font)
            .text_size(px(size))
            .line_height(px(self.line_height()))
            .text_color(fg);
        let d = if self.single_line { d } else { crate::config::customization::capture(d) };
        self.bind_actions(d, cx)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_spelling_right_click))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up_out))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(EditorElement { editor: cx.entity() })
            .children(self.render_completion(window, cx))
            .children(self.render_snippet_choices(window, cx))
            .children(self.render_definitions(window, cx))
            .children(self.render_signature(window, cx))
    }
}

/// Tests: an editor of `text` alone in a 600 × 300 window, unwrapped unless `setup` wraps it,
/// drawn once.
#[cfg(test)]
pub(crate) fn test_editor<'a>(
    text: &str,
    cx: &'a mut gpui::TestAppContext,
    setup: impl FnOnce(&mut Editor, &mut Context<Editor>) + 'static,
) -> (Entity<Editor>, &'a mut gpui::VisualTestContext) {
    let text = text.to_string();
    let (e, cx) = cx.add_window_view(move |_, cx| {
        let mut e = Editor::new(EditorState::from_text(&text), false, cx);
        e.wrap_override = Some(false);
        setup(&mut e, cx);
        e
    });
    cx.simulate_resize(gpui::size(px(600.), px(300.)));
    cx.run_until_parked();
    (e, cx)
}

/// Tests: an editor of the file `path` alone in the active 600 × 300 window (20 rows) with the
/// editor's keys, focused, its caret at `|` (else at the end).
#[cfg(test)]
pub(crate) fn focused_editor<'a>(
    text: &str,
    path: Option<&std::path::Path>,
    cx: &'a mut gpui::TestAppContext,
) -> (Entity<Editor>, &'a mut gpui::VisualTestContext) {
    cx.update(|cx| cx.bind_keys(key_bindings()));
    let caret = text.find('|');
    let (text, path) = (text.replace('|', ""), path.map(std::path::Path::to_path_buf));
    let (e, cx) = cx.add_window_view(move |_, cx| {
        let mut s = EditorState::from_text(&text);
        s.set_cursor(caret.map_or(s.len(), |b| text[..b].chars().count()));
        s.doc.path = path;
        let mut e = Editor::new(s, false, cx);
        e.wrap_override = Some(false);
        e
    });
    cx.simulate_resize(gpui::size(px(600.), px(300.)));
    e.update_in(cx, |e, window, _| {
        window.activate_window();
        e.focus(window);
    });
    cx.run_until_parked();
    (e, cx)
}

/// Tests: a command as its key runs it, then a frame.
#[cfg(test)]
pub(crate) fn run(e: &Entity<Editor>, cx: &mut gpui::VisualTestContext, action: &dyn gpui::Action) {
    e.update_in(cx, |e, window, cx| assert!(e.perform_action(action, window, cx)));
    cx.run_until_parked();
}

#[cfg(test)]
mod wrap_column_tests {
    use super::*;
    use gpui::{size, TestAppContext};

    /// The rows the view shows and the widest one in display columns.
    fn rows(e: &Editor, cx: &App) -> (usize, usize) {
        let (w, s) = (e.wrap.as_ref().expect("wrapped"), e.doc(cx));
        let n = w.rows(s);
        let widest = (0..n).map(|r| w.row_of(s, w.row_bounds(s, DisplayRow(r)).unwrap().end).col.0).max().unwrap_or(0);
        (n, widest)
    }

    #[gpui::test]
    fn replacing_content_discards_wrapped_rows_before_navigation_or_repaint(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(EditorState::from_text("old"), false, cx);
            e.wrap_override = Some(true);
            e.wrap_width_override = Some(8);
            e
        });
        cx.run_until_parked();
        for replacement in ["first\na long new line with several wrapped rows\n", "x\ny\n"] {
            editor.update(cx, |e, cx| {
                e.set_text(replacement, cx);
                // Commands can navigate before the next prepaint rebuilds the rows.
                e.jump_to(e.doc(cx).line_start(1), cx);
            });
            cx.run_until_parked();
            editor.read_with(cx, |e, cx| {
                assert_eq!(e.wrap.as_ref().unwrap().lines(), e.doc(cx).line_count());
                assert_eq!(e.doc(cx).point(e.state.cursor()).0, 1);
            });
        }
    }

    #[gpui::test]
    fn add_next_and_previous_line_step_by_the_rows_of_a_wrapped_view(cx: &mut TestAppContext) {
        // at 8 columns the first line is three rows, from 0, 5 and 10
        let (editor, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(EditorState::from_text("aaaa bbbb cccc\nnext"), false, cx);
            e.wrap_override = Some(true);
            e.wrap_width_override = Some(8);
            e
        });
        cx.run_until_parked();
        editor.update_in(cx, |e, window, cx| {
            let w = e.wrap.as_ref().expect("wrapped");
            let starts: Vec<usize> =
                (0..w.rows(e.doc(cx))).map(|r| w.row_bounds(e.doc(cx), DisplayRow(r)).unwrap().start).collect();
            assert_eq!(starts, [0, 5, 10, 15]);
            let carets = |e: &Editor| -> Vec<usize> { e.state.selections.iter().map(|s| s.head).collect() };
            // from the middle row: the rows below and above, in the same line
            for (forward, want) in [(true, [6, 11]), (false, [1, 6])] {
                e.sel(cx).set_cursor(6);
                assert!(e.perform_action(&SelectLines { forward }, window, cx));
                assert_eq!(carets(e), want, "forward: {forward}");
            }
        });
    }

    #[gpui::test]
    fn a_wrap_column_caps_rows_by_display_columns_and_narrow_panes_still_fit(cx: &mut TestAppContext) {
        // tabs advance to stops, CJK takes two columns
        let line = "\tword 中文中文 wide\tthen more words and more words and more words";
        let text = format!("{line}\n").repeat(40);
        let (editor, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(EditorState::from_text(&text), false, cx);
            e.wrap_override = Some(true);
            e
        });
        cx.simulate_resize(size(px(900.), px(400.)));
        cx.run_until_parked();
        let (wide_rows, _) = editor.read_with(cx, rows);
        editor.update(cx, |e, cx| {
            e.wrap_width_override = Some(20);
            cx.notify();
        });
        cx.run_until_parked();
        let (n, widest) = editor.read_with(cx, rows);
        assert!(widest <= 20, "a row of {widest} columns");
        assert!(n > wide_rows, "more rows at 20 columns: {n} vs {wide_rows}");
        // the setting: a view's choice wins over it, 0 is the view's width
        editor.update(cx, |e, cx| {
            e.wrap_width_override = None;
            e.view.wrap_width = 30;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(editor.read_with(cx, rows).1 <= 30);
        // a pane narrower than the column wraps at the pane
        cx.simulate_resize(size(px(200.), px(400.)));
        cx.run_until_parked();
        let (_, narrow) = editor.read_with(cx, rows);
        assert!(narrow < 30, "the pane, {narrow} columns, is narrower than 30");
    }

    #[gpui::test]
    fn the_text_on_top_stays_on_top_when_the_wrap_column_changes(cx: &mut TestAppContext) {
        let text = "a long line of words that wraps at any narrow column we pick\n".repeat(200);
        let (editor, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(EditorState::from_text(&text), false, cx);
            e.wrap_override = Some(true);
            e
        });
        cx.simulate_resize(size(px(700.), px(300.)));
        cx.run_until_parked();
        let line_120 = editor.read_with(cx, |e, cx| e.doc(cx).line_start(120));
        editor.update(cx, |e, cx| {
            e.set_top((line_120, 0.));
            cx.notify();
        });
        cx.run_until_parked();
        for cols in [12, 40, 0] {
            editor.update(cx, |e, cx| {
                e.hold_top(cx);
                e.wrap_width_override = Some(cols);
                cx.notify();
            });
            cx.run_until_parked();
            let top = editor.read_with(cx, |e, cx| e.doc(cx).point(e.top(cx).0).0);
            assert_eq!(top, 120, "at {cols} columns");
        }
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use crate::editing::text::Motion;
    use gpui::{point, size, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase};

    #[test]
    fn a_glide_eases_out_and_ends_on_its_target() {
        assert_eq!(scroll_at(10., 110., Duration::ZERO), 10.);
        assert_eq!(scroll_at(10., 110., SCROLL_ANIM), 110.);
        assert_eq!(scroll_at(10., 110., SCROLL_ANIM * 5), 110., "clamped in time");
        let (a, b) = (scroll_at(0., 100., SCROLL_ANIM / 4), scroll_at(0., 100., SCROLL_ANIM / 2));
        assert!(0. < a && a < b && b < 100., "{a} {b}");
        assert!(a > 25., "eases out: more than a quarter in the first quarter");
    }

    fn page(e: &mut Editor, down: bool, window: &mut Window, cx: &mut Context<Editor>) {
        let motion = match down {
            true => Motion::PageDown(0),
            false => Motion::PageUp(0),
        };
        e.r#move(&Move::to(motion, false), window, cx);
    }

    #[gpui::test]
    fn page_down_glides_retargets_and_a_wheel_scroll_stops_it_where_it_is(cx: &mut TestAppContext) {
        let text = "line\n".repeat(2_000);
        let (editor, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(EditorState::from_text(&text), false, cx);
            e.view.animate = true;
            e
        });
        cx.simulate_resize(size(px(600.), px(300.)));
        cx.run_until_parked();
        editor.update_in(cx, |e, window, cx| {
            page(e, true, window, cx);
            let one = e.scroll_top;
            assert!(one > 0. && e.scroll_anim.is_some(), "going to {one}");
            assert!(e.drawn_scroll() < one, "on its way");
            // a second Page Down mid-way carries on from where the view is drawn
            let drawn = e.drawn_scroll();
            page(e, true, window, cx);
            assert_eq!(e.scroll_top, 2. * one);
            assert!((e.scroll_anim.unwrap().0 - drawn).abs() < one, "from the drawn place");
            // the wheel takes over where the view is, with no glide on top of it
            let at = e.drawn_scroll();
            let wheel = ScrollWheelEvent {
                position: point(px(100.), px(100.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
                modifiers: Default::default(),
                touch_phase: TouchPhase::Moved,
            };
            e.on_scroll(&wheel, window, cx);
            assert!(e.scroll_anim.is_none());
            assert!((e.scroll_top - (at + 30.)).abs() < 1., "{} vs {at}", e.scroll_top);
        });
        // painting goes on while it glides, and clicks follow the drawn rows
        editor.update_in(cx, |e, window, cx| page(e, true, window, cx));
        cx.run_until_parked();
        editor.read_with(cx, |e, _| {
            let drawn = e.layout.as_ref().unwrap().scroll_top;
            assert!(drawn <= e.scroll_top);
        });
    }

    #[gpui::test]
    fn no_glide_at_the_ends_or_with_animation_off(cx: &mut TestAppContext) {
        let text = "line\n".repeat(300);
        let (editor, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(EditorState::from_text(&text), false, cx);
            e.view.animate = true;
            e
        });
        cx.simulate_resize(size(px(600.), px(300.)));
        cx.run_until_parked();
        editor.update_in(cx, |e, window, cx| {
            page(e, false, window, cx);
            assert_eq!(e.scroll_top, 0.);
            assert!(e.scroll_anim.is_none(), "already at the top");
            for _ in 0..40 {
                page(e, true, window, cx);
            }
            assert_eq!(e.scroll_top, e.max_scroll(cx), "clamped at the end");
            e.scroll_anim = None;
            page(e, true, window, cx);
            assert!(e.scroll_anim.is_none(), "at the end: nothing to glide");
            e.view.animate = false;
            page(e, false, window, cx);
            assert!(e.scroll_anim.is_none() && e.scroll_top < e.max_scroll(cx));
        });
    }
}
