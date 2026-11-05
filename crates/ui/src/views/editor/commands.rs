//! The editor's action handlers: thin wrappers over `EditorState` commands.

use super::actions::editor_actions;
use super::*;
use crate::editing::clipboard_history::{ClipEntry, ClipboardHistory};
use crate::editing::convert;
use crate::editing::fold;
use crate::editing::html;
use crate::editing::stats;
use crate::editing::text::{ColumnJob, ColumnPlan, Motion, WRAP_WIDTH};
use crate::keybindings::InsertText;
use gpui::{prelude::*, ClipboardItem, Context, Div, PromptLevel, Window};

/// Clipboard metadata for text copied from empty selections, so any tab or window pastes it
/// back as whole lines; gpui drops it once another app replaces the text.
const WHOLE_LINES: &str = "orlo.whole-lines";

pub(crate) fn clipboard_item((text, whole_lines): (String, bool)) -> ClipboardItem {
    if whole_lines {
        ClipboardItem::new_string_with_metadata(text, WHOLE_LINES.into())
    } else {
        ClipboardItem::new_string(text)
    }
}

/// Copy and Cut: onto the system clipboard and into the history.
pub(crate) fn copy_to_clipboard(cx: &mut App, (text, whole_lines): (String, bool)) {
    cx.default_global::<ClipboardHistory>().record(&text, whole_lines);
    cx.write_to_clipboard(clipboard_item((text, whole_lines)));
}

/// AppKit's HTML importer (`NSAttributedString`) reads UTF-8 with no charset as Windows-1252,
/// so "é" pastes as "Ã©" into the apps that go through it; this tag says what it is.
const HTML_CHARSET: &str = "<meta charset=\"utf-8\">";

/// The newest rich copy in any window cancels an older view's pending copy too.
#[derive(Default)]
struct HtmlCopies(std::sync::Weak<AtomicBool>);

impl gpui::Global for HtmlCopies {}

#[cfg(not(test))]
fn clipboard_stamp(_: &mut App) -> Option<isize> {
    crate::app::macos::pasteboard_change_count(None)
}

#[cfg(test)]
fn clipboard_stamp(cx: &mut App) -> Option<ClipboardItem> {
    cx.read_from_clipboard()
}

/// Copy as HTML's one pasteboard write; false if the pasteboard took none of it.
#[cfg(not(test))]
fn write_rich(html: &str, text: &str) -> bool {
    crate::app::macos::write_html_and_text(None, html, text)
}

#[cfg(test)]
fn write_rich(html: &str, text: &str) -> bool {
    rich::write(html, text)
}

/// Stands in for the user's pasteboard in tests, which must never write it: what Copy as HTML
/// wrote on this thread, and whether the pasteboard is refusing.
#[cfg(test)]
pub(crate) mod rich {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static WRITES: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
        static REFUSING: Cell<bool> = const { Cell::new(false) };
    }

    pub fn write(html: &str, text: &str) -> bool {
        if REFUSING.get() {
            return false;
        }
        WRITES.with_borrow_mut(|w| w.push((html.into(), text.into())));
        true
    }

    /// The writes since the last call: (html, plain text).
    pub fn take() -> Vec<(String, String)> {
        WRITES.take()
    }

    pub fn refuse(yes: bool) {
        REFUSING.set(yes);
    }
}

// Direct editor commands share their typed handler with the dispatch table.
macro_rules! define_simple_actions {
    ($($ty:ident [$kind:ident, $policy:ident] => $handler:ident $(($mode:ident => $method:ident))?,)*) => {
        impl Editor {
            $($(fn $handler(&mut self, _: &$ty, _: &mut Window, cx: &mut Context<Self>) {
                self.$mode(cx, |s| s.$method());
            })?)*
        }
    };
}
editor_actions!(define_simple_actions);

impl Editor {
    /// Every action the editor handles, in one `.on_action` chain.
    pub(crate) fn bind_actions(&self, d: Div, cx: &mut Context<Self>) -> Div {
        macro_rules! bind_one {
            // These native actions need the workspace's all-window composition admission.
            ($d:ident, MoveToFold, $handler:ident) => {};
            ($d:ident, SelectFold, $handler:ident) => {};
            ($d:ident, NarrowToSelection, $handler:ident) => {};
            ($d:ident, NarrowToFold, $handler:ident) => {};
            ($d:ident, ShowMatchingLines, $handler:ident) => {};
            ($d:ident, ClearFilter, $handler:ident) => {};
            ($d:ident, AdjustValues, $handler:ident) => {};
            ($d:ident, InsertDateTime, $handler:ident) => {};
            ($d:ident, InsertUnicodeCharacter, $handler:ident) => {};
            ($d:ident, InsertLiteralNextKey, $handler:ident) => {};
            ($d:ident, InsertLiteralText, $handler:ident) => {};
            ($d:ident, AlignOnPattern, $handler:ident) => {};
            ($d:ident, AlignSelectedFields, $handler:ident) => {};
            ($d:ident, InvertSelections, $handler:ident) => {};
            ($d:ident, SplitSelectionsByRegex, $handler:ident) => {};
            ($d:ident, KeepSelectionsMatchingRegex, $handler:ident) => {};
            ($d:ident, RemoveMatchingSelections, $handler:ident) => {};
            ($d:ident, $ty:ident, $handler:ident) => {
                $d = $d.on_action(cx.listener(Self::$handler));
            };
        }
        macro_rules! bind {
            ($($ty:ident [$kind:ident, $policy:ident] => $handler:ident $(($mode:ident => $method:ident))?,)*) => {{
                let mut d=d;
                $(bind_one!(d,$ty,$handler);)*
                d
            }};
        }
        editor_actions!(bind)
    }

    /// The JSON API and macros use the same handlers, including before a new view's first frame.
    /// A batch runs no frame between its commands, so the reveal runs here around each one: a
    /// workspace command (Find Next) before it may have put a selection inside a fold, and this
    /// one may put one inside another.
    pub fn perform_action(&mut self, action: &dyn gpui::Action, window: &mut Window, cx: &mut Context<Self>) -> bool {
        macro_rules! dispatch {
            ($($ty:ident [$kind:ident, $policy:ident] => $handler:ident $(($mode:ident => $method:ident))?,)*) => { $(
                if let Some(a)=action.as_any().downcast_ref::<$ty>() {
                    self.reveal_selections(cx);
                    self.$handler(a,window,cx);
                    self.reveal_selections(cx);
                    return true;
                }
            )* };
        }
        editor_actions!(dispatch);
        false
    }

    pub fn handles_action(action: &dyn gpui::Action) -> bool {
        macro_rules! handles {
            ($($ty:ident [$kind:ident, $policy:ident] => $handler:ident $(($mode:ident => $method:ident))?,)*) => { $(
                if action.as_any().is::<$ty>() {return true;}
            )* };
        }
        editor_actions!(handles);
        false
    }

    pub fn set_virtual_space(&mut self, policy: impl Into<crate::config::settings::VirtualSpace>, cx: &App) {
        let policy = if !self.single_line && self.input_enabled { policy.into() } else { false.into() };
        if self.state.virtual_space != policy {
            self.marked.clear();
            self.overtype_composition = None;
            self.completion = None;
            self.completion_epoch += 1;
            self.sel(cx).set_virtual_space(policy);
            self.drag = None;
        }
    }

    fn toggle_overtype(&mut self, _: &ToggleOvertype, _: &mut Window, cx: &mut Context<Self>) {
        self.literal_next = None;
        if !self.single_line && self.input_enabled {
            self.edit(cx, |s| s.toggle_overtype());
        }
    }

    fn insert_text(&mut self, a: &crate::config::customization::InsertText, _: &mut Window, cx: &mut Context<Self>) {
        self.literal_next = None;
        self.insert_text_with_policy(&a.text, a.replace, false, cx);
    }

    pub(crate) fn insert_literal_text(&mut self, a: &InsertLiteralText, _: &mut Window, cx: &mut Context<Self>) {
        self.literal_next = None;
        if let Err(error) = self.character_input_admission(cx) {
            return self.status(error, cx);
        }
        self.insert_text_with_policy(&a.text, a.replace, true, cx);
    }

    fn insert_text_with_policy(
        &mut self,
        text: &str,
        replace: Option<[isize; 2]>,
        literal: bool,
        cx: &mut Context<Self>,
    ) {
        let literal_quote = text == "'" && !self.highlighter.pairs_single_quote();
        self.edit_text(cx, |s| {
            if let Some([start, end]) = replace {
                let len = s.len();
                let selections = s
                    .view
                    .selections
                    .iter()
                    .map(|selection| crate::editing::text::Selection {
                        rectangular: selection.rectangular,
                        ..crate::editing::text::Selection::new(
                            selection.head.saturating_add_signed(start).min(len),
                            selection.head.saturating_add_signed(end).min(len),
                        )
                    })
                    .collect();
                s.reselect(selections);
            }
            if literal {
                s.end_typing();
                s.type_literal(text);
                s.end_typing();
            } else if replace.is_some() && s.view.overtype {
                s.insert(text);
            } else if literal_quote {
                s.type_literal(text);
            } else {
                s.type_text(text);
            }
        });
    }

    fn field_newline(&mut self, _: &FieldNewline, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.edit_text(cx, |s| s.insert("\n"));
        }
    }
    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            let language =
                orlo_editor::commands::indentation::IndentLanguage::from_syntax(self.highlighter.syntax_name());
            let indents = self.highlighter.indents(&self.doc(cx).rope);
            self.edit_text(cx, |s| {
                s.newline_indented(language, |start, end, blank| {
                    indents.as_ref().and_then(|i| i.levels(start, end, blank))
                })
            });
        }
    }
    fn reindent_selection(&mut self, _: &ReindentSelection, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        let Some(language) =
            orlo_editor::commands::indentation::IndentLanguage::from_syntax(self.highlighter.syntax_name())
        else {
            return self.status("Reindent Selection is unavailable for this language", cx);
        };
        if !self.edit(cx, |s| s.reindent_selection(language)) {
            self.status("Reindent Selection context is unavailable or exceeds the limit", cx);
        }
    }
    /// Tab: a snippet's next field, else a snippet whose trigger is before every caret, else
    /// indentation.
    fn tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tab_snippet(window, cx) {
            self.edit_text(cx, |s| s.tab());
        }
    }

    fn select_line(&mut self, _: &SelectLine, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |s| s.select_line());
        self.lines_over_folds(cx);
    }

    fn single_selection(&mut self, _: &SingleSelection, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.edit(cx, |s| s.single_selection());
        }
    }

    /// The syntax's comment markers; for a file shown as Plain Text, those of the registered
    /// language it is detected as (one with no grammar of its own).
    fn comment_markers(&self, cx: &App) -> (Option<String>, Option<(String, String)>) {
        let (line, block) = (self.highlighter.comment_prefix(), self.highlighter.block_comment());
        if line.is_some()
            || block.is_some()
            || self.highlighter.language_override().is_some()
            || self.highlighter.syntax_name() != "Plain Text"
        {
            return (line.map(Into::into), block.map(|(open, close)| (open.into(), close.into())));
        }
        let registry = crate::languages::registry::registry();
        let doc = self.doc(cx);
        let language = match &self.modeline_language {
            Some(id) => registry.get(id),
            None => {
                let head = doc.rope.slice(..doc.rope.len_chars().min(crate::languages::registry::CONTENT_BYTES));
                registry.detect(doc.path.as_deref(), &head.to_string())
            }
        };
        let Some(comments) = language.map(|l| &l.manifest.comments) else {
            return (None, None);
        };
        (comments.line.clone(), comments.block.clone().map(|[open, close]| (open, close)))
    }

    fn toggle_comment(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        match self.comment_markers(cx) {
            (Some(prefix), _) => self.edit_text(cx, |s| s.toggle_comment(&prefix)),
            // CSS, HTML, XML, Markdown: Sublime falls back to the block pair
            (None, Some((open, close))) => self.edit_text(cx, |s| s.toggle_comment_with_block(&open, &close)),
            (None, None) => {}
        }
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.read_state(cx).copy_text();
        copy_to_clipboard(cx, text);
    }

    /// Edit > Copy as HTML: the selections, or with all of them empty the whole document,
    /// as coloured HTML and plain text in one pasteboard write. The lines are coloured and the
    /// HTML built on the highlighting thread (urgent), so a big document does not stall the
    /// editor; only the write is made here, once that is done. More than 16 MiB of text is
    /// refused with a word in the status bar, before anything is started.
    fn copy_as_html(&mut self, _: &CopyAsHtml, _: &mut Window, cx: &mut Context<Self>) {
        if self.single_line {
            return;
        }
        let s = self.read_state(cx);
        let mut ranges: Vec<_> = (s.view.selections.iter()).filter(|x| !x.is_empty()).map(|x| x.range()).collect();
        if ranges.is_empty() {
            if s.view.selections.iter().any(Selection::has_virtual) {
                return;
            }
            ranges.push(0..s.len());
        }
        // before any work: the page is several times the text, built, held and written whole
        if html::too_big(html::text_len(&s.doc.rope, &ranges)) {
            return self.status("Copy as HTML is limited to 16 MB of text", cx);
        }
        let (rope, colours) = (s.doc.rope.clone(), self.highlighter.clone());
        let (font, size, tab) = (self.view.font_face.to_string(), self.view.font_size, s.doc.tab_size);
        let (fg, bg) = (c(theme::EDITOR_FG), c(theme::EDITOR_BG));
        if ranges.iter().map(|r| r.len()).sum::<usize>() > stats::BIG {
            self.status("Working…", cx);
        }
        // a copy nothing has replaced keeps the highlighting thread until it is built
        // (urgent, and it gives the thread to no one); build it in stretches that let other work
        // in if that shows up.
        let stop = Arc::new(AtomicBool::new(false));
        let latest = &mut cx.default_global::<HtmlCopies>().0;
        if let Some(older) = latest.upgrade() {
            older.store(true, Ordering::Relaxed);
        }
        *latest = Arc::downgrade(&stop);
        let clipboard = clipboard_stamp(cx);
        let (pending, flag) = (stop.clone(), stop.clone());
        let built = highlight::on_worker_urgent(move || {
            let mut runs = highlight::LineRuns::new(colours, rope.clone(), flag.clone());
            let lines = html::lines(&rope, &ranges, &flag, |line| runs.line(line));
            let mut page = html::to_html(lines, fg, bg, &font, size, tab);
            page.insert_str(0, HTML_CHARSET);
            let mut text = String::new();
            for (i, r) in ranges.iter().enumerate() {
                if i > 0 {
                    text.push('\n');
                }
                text.extend(rope.slice(r.clone()).chunks());
            }
            (page, text)
        });
        let task = cx.spawn(async move |this, cx| {
            let built = built.await;
            this.update(cx, |e, cx| {
                let cancelled = pending.load(Ordering::Relaxed);
                e.html_job = None;
                if cancelled || clipboard_stamp(cx) != clipboard {
                    return;
                }
                match built {
                    Some((page, text)) => e.put_html(&page, text, cx),
                    None => e.status("Copy as HTML failed", cx),
                }
            })
            .ok();
        });
        // a copy in flight that a newer one replaces, or that the view closing drops, has its flag
        // set: the thread stops at its next line, or inside tree-sitter's parse or query
        self.html_job = Some(HtmlJob { _task: task, _stop: Cancel(stop) });
    }

    /// Copy as HTML's result: onto the pasteboard, and its text into the clipboard history. A
    /// pasteboard that takes no HTML gets the text alone, as Copy would put it.
    fn put_html(&mut self, page: &str, text: String, cx: &mut Context<Self>) {
        if write_rich(page, &text) {
            cx.default_global::<ClipboardHistory>().record(&text, false);
            self.status("Copied as HTML", cx);
        } else {
            copy_to_clipboard(cx, (text, false));
            self.status("Copied the text; the pasteboard took no HTML", cx);
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        self.marked.clear();
        let copied = self.change(cx, |s| s.cut());
        copy_to_clipboard(cx, copied);
        self.after_edit(cx);
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let Some(text) = item.text() else { return };
        let whole_lines = item.metadata().is_some_and(|m| m == WHOLE_LINES);
        self.paste_entry(&ClipEntry { text, whole_lines }, cx);
    }
    /// `paste`'s edit, for text that did not come from the system clipboard (Paste from History).
    pub fn paste_entry(&mut self, e: &ClipEntry, cx: &mut Context<Self>) {
        let whole_line = !self.single_line && e.whole_lines;
        // a one-line input takes the first line, as Sublime's palette does; a multiline field
        // all of it, its line breaks as `\n`
        let text = match (self.single_line, self.multiline) {
            (true, false) => e.text.lines().next().unwrap_or("").to_string(),
            (true, true) => e.text.replace("\r\n", "\n").replace('\r', "\n"),
            (false, _) => e.text.clone(),
        };
        // a whole-line copy's own line break is not part of a query
        let text = match self.single_line && e.whole_lines {
            true => text.strip_suffix('\n').unwrap_or(&text).to_string(),
            false => text,
        };
        let text = text.as_str();
        self.edit_text(cx, |s| s.paste(text, whole_line));
    }

    fn undo_selection(&mut self, _: &UndoSelection, _: &mut Window, cx: &mut Context<Self>) {
        if !self.edit(cx, |s| s.undo_selection()) {
            self.status("No selection to undo", cx);
        }
    }
    fn redo_selection(&mut self, _: &RedoSelection, _: &mut Window, cx: &mut Context<Self>) {
        if !self.edit(cx, |s| s.redo_selection()) {
            self.status("No selection to redo", cx);
        }
    }
    fn quick_skip_next(&mut self, _: &QuickSkipNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.edit(cx, |s| s.quick_skip_next()) {
            self.status("No more occurrences", cx);
        }
    }
    pub(crate) fn r#move(&mut self, a: &Move, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            a.motion,
            Motion::ParagraphPrevious | Motion::ParagraphNext | Motion::SentencePrevious | Motion::SentenceNext
        ) {
            self.catch_up(cx);
        }
        self.sel(cx).sel_checkpoint();
        // Like other selection commands, motion commits the marked text where it is.
        self.marked.clear();
        // a page is the rows in view, less one (Sublime's); the view moves with the caret
        let motion = match a.motion {
            Motion::PageUp(_) => Motion::PageUp(self.page_lines()),
            Motion::PageDown(_) => Motion::PageDown(self.page_lines()),
            m => m,
        };
        let page = match motion {
            Motion::PageUp(n) => Some(-(n as f64)),
            Motion::PageDown(n) => Some(n as f64),
            _ => None,
        };
        if let Some(rows) = page {
            let dy = rows * self.line_height() as f64;
            self.scroll_page_to((self.scroll_top + dy).clamp(0., self.max_scroll(cx)));
            self.scrolled(cx);
        }
        self.with_rows(window, cx, |s, rows, m| s.move_cursors_in(motion, a.select, rows, m));
        self.after_edit(cx);
    }
    fn select_lines(&mut self, a: &SelectLines, window: &mut Window, cx: &mut Context<Self>) {
        // `edit`'s steps, with the view's rows to hand
        self.marked.clear();
        self.sel(cx).sel_checkpoint();
        self.with_rows(window, cx, |s, rows, m| s.select_lines(a.forward, rows, m));
        self.after_edit(cx);
    }

    fn delete_subword(&mut self, a: &DeleteSubword, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(cx, |s| s.delete_subword(a.forward));
    }

    fn delete_word(&mut self, a: &DeleteWord, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(cx, |s| s.delete_word(a.forward));
    }

    fn select_line_upward(&mut self, _: &SelectLineUpward, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |s| s.select_line_upward());
        self.lines_over_folds(cx);
    }

    fn paste_and_indent(&mut self, _: &PasteAndIndent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit_text(cx, |s| s.paste_and_indent(&text));
        }
    }
    /// Resolve the physical address before restoring a narrowed presentation.
    pub(crate) fn goto_address(&mut self, line: usize, column: usize, cx: &mut Context<Self>) {
        self.catch_up(cx);
        let at = self.doc(cx).offset(line.saturating_sub(1), column.saturating_sub(1));
        self.jump_to(at, cx);
        self.reveal_selections(cx);
        self.scroll_to_cursor(cx);
    }

    /// Validate and reveal a physical bookmark after shared edits, before moving its caret.
    pub(crate) fn goto_bookmark(&mut self, document: u64, version: u64, line: usize, cx: &mut Context<Self>) -> bool {
        if !self.marked.is_empty() {
            return false;
        }
        self.catch_up(cx);
        if self.doc(cx).id() != document
            || self.doc(cx).version != version
            || !self.read_state(cx).bookmarked_lines().contains(&line)
        {
            return false;
        }
        self.reveal_selections(cx);
        let at = self.doc(cx).line_start(line);
        self.record_jump(self.state.cursor());
        self.edit(cx, |s| s.set_cursor(at));
        self.reveal_selections(cx);
        self.scroll_to_cursor(cx);
        true
    }

    fn toggle_bookmark(&mut self, _: &ToggleBookmark, _: &mut Window, cx: &mut Context<Self>) {
        self.sel(cx).toggle_bookmark();
        cx.notify();
    }
    fn next_bookmark(&mut self, a: &NextBookmark, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |s| s.next_bookmark(a.forward));
    }
    fn clear_bookmarks(&mut self, _: &ClearBookmarks, _: &mut Window, cx: &mut Context<Self>) {
        self.sel(cx).clear_bookmarks();
        cx.notify();
    }

    fn toggle_block_comment(&mut self, _: &ToggleBlockComment, _: &mut Window, cx: &mut Context<Self>) {
        if let (_, Some((open, close))) = self.comment_markers(cx) {
            self.edit_text(cx, |s| s.toggle_block_comment(&open, &close));
        }
    }
    fn delete_to_line_edge(&mut self, a: &DeleteToLineEdge, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(cx, |s| s.delete_to_line_edge(a.forward));
    }
    fn convert_case(&mut self, a: &ConvertCase, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(cx, |s| s.convert_case(a.case));
    }
    fn transform_text(&mut self, a: &TransformText, _: &mut Window, cx: &mut Context<Self>) {
        let op = a.op;
        // these cannot fail
        self.edit_text(cx, |s| drop(s.transform_selections(false, |t| Ok(op.apply(t)))));
    }
    fn convert(&mut self, a: &Convert, window: &mut Window, cx: &mut Context<Self>) {
        let (codec, decode) = (a.codec, a.decode);
        if matches!(codec, convert::Codec::RegexLiteral | convert::Codec::JsonString) {
            return self.convert_literal(
                |text| if decode { convert::decode(codec, text) } else { Ok(convert::encode(codec, text)) },
                window,
                cx,
            );
        }
        if matches!(codec, convert::Codec::UriComponent | convert::Codec::HtmlEntities) {
            self.catch_up(cx);
            if self.state.selections.iter().all(|s| s.range().is_empty()) {
                return;
            }
        }
        let mut done = Ok(());
        self.edit_text(cx, |s| {
            done = s.transform_selections(false, |t| match decode {
                true => convert::decode(codec, t),
                false => Ok(convert::encode(codec, t)),
            })
        });
        if let Err(e) = done {
            drop(window.prompt(PromptLevel::Critical, "Decode failed", Some(&e), &["OK"], cx));
        }
    }
    fn transpose_words(&mut self, _: &TransposeWords, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        self.catch_up(cx);
        let edits = self.read_state(cx).transpose_word_edits();
        if !edits.is_empty() {
            self.edit_text(cx, |state| {
                state.replace_ranges(edits);
            });
        }
    }

    fn change_surround(&mut self, a: &ChangeSurround, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_surround(a.target, Some(a.replacement), a.count, cx);
    }

    fn delete_surround(&mut self, a: &DeleteSurround, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_surround(a.target, None, a.count, cx);
    }

    fn edit_surround(
        &mut self,
        target: crate::editing::text::Surround,
        replacement: Option<crate::editing::text::Surround>,
        count: std::num::NonZeroUsize,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        self.catch_up(cx);
        let edits = self.read_state(cx).surround_edits(target, replacement, count);
        if !edits.is_empty() {
            self.edit_text(cx, |state| {
                state.replace_ranges(edits);
            });
        }
    }

    fn decode_text_escapes(&mut self, _: &DecodeTextEscapes, window: &mut Window, cx: &mut Context<Self>) {
        self.convert_literal(convert::decode_text_escapes, window, cx);
    }

    /// Literal operations validate every physical target before touching text or selection history.
    fn convert_literal(
        &mut self,
        convert: impl Fn(&str) -> Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        self.catch_up(cx);
        let state = self.read_state(cx);
        let edits: Result<Vec<_>, String> = state
            .view
            .selections
            .iter()
            .filter(|s| !s.range().is_empty())
            .map(|s| {
                let range = s.range();
                let old = state.doc.rope.slice(range.clone()).to_string();
                let new = convert(&old)
                    .map_err(|error| format!("Line {}: {error}", state.doc.rope.char_to_line(range.start) + 1))?
                    .replace("\r\n", "\n");
                Ok((range, new))
            })
            .collect();
        let edits = match edits {
            Ok(edits) => edits,
            Err(error) => {
                return drop(window.prompt(PromptLevel::Critical, "Decode failed", Some(&error), &["OK"], cx));
            }
        };
        if edits.iter().all(|(range, text)| state.doc.rope.slice(range.clone()) == text.as_str()) {
            return;
        }
        self.edit_text(cx, |state| {
            state.replace_ranges(edits);
        });
    }

    fn copy_hash(&mut self, a: &CopyHash, _: &mut Window, cx: &mut Context<Self>) {
        // hashes on the UI thread, so a multi-hundred-MB selection or document stalls it; hash on a background task past a size.
        let digests = match self.read_state(cx).hash_selections(a.hash) {
            Ok(digests) => digests,
            Err(error) => return self.status(error.to_string(), cx),
        };
        // a digest is not a Copy, so it stays out of the clipboard history
        cx.write_to_clipboard(ClipboardItem::new_string(digests.join("\n")));
        let name = a.hash.name();
        match digests.len() {
            1 => self.status(format!("{name} copied"), cx),
            n => self.status(format!("{name} of {n} selections copied"), cx),
        }
    }
    fn permute_lines(&mut self, a: &PermuteLines, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(cx, |s| s.permute_lines(a.op));
    }
    fn permute_selections(&mut self, a: &PermuteSelections, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(cx, |s| s.permute_selections(a.op));
    }
    /// Goto > Bookmarks: the bookmarked lines copied (as whole lines, into the clipboard
    /// history), cut, deleted, kept alone, pasted over, or the bookmarks inverted.
    fn bookmarked_lines(&mut self, a: &BookmarkedLines, _: &mut Window, cx: &mut Context<Self>) {
        if self.single_line {
            return;
        }
        if a.op != MarkedLines::Invert && self.state.bookmarks.is_empty() {
            return self.status("No bookmarks", cx);
        }
        if matches!(a.op, MarkedLines::Copy | MarkedLines::Cut) {
            if let Some(text) = self.read_state(cx).bookmarked_text() {
                copy_to_clipboard(cx, (text, true));
            }
        }
        match a.op {
            MarkedLines::Copy => {
                let n = self.read_state(cx).bookmarked_lines().len();
                let s = if n == 1 { "" } else { "s" };
                self.status(format!("Copied {n} bookmarked line{s}"), cx);
            }
            MarkedLines::Cut | MarkedLines::Delete | MarkedLines::KeepOnly => {
                let keep = a.op == MarkedLines::KeepOnly;
                self.edit_text(cx, |s| {
                    s.delete_bookmarked_lines(keep);
                });
            }
            MarkedLines::Paste => {
                let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
                    return self.status("The clipboard has no text", cx);
                };
                let text = text.replace("\r\n", "\n");
                self.edit_text(cx, |s| {
                    s.paste_to_bookmarked_lines(&text);
                });
            }
            MarkedLines::Invert => {
                self.sel(cx).invert_bookmarks();
                cx.notify();
            }
        }
    }

    fn highlight_selection(&mut self, a: &HighlightSelection, _: &mut Window, cx: &mut Context<Self>) {
        let ranges: Vec<_> = (self.state.selections.iter()).filter(|s| !s.is_empty()).map(|s| s.range()).collect();
        if ranges.is_empty() {
            return self.status("Select the text to highlight", cx);
        }
        self.sel(cx).add_highlights(a.style, ranges);
        cx.notify();
    }

    fn next_highlight(&mut self, a: &NextHighlight, _: &mut Window, cx: &mut Context<Self>) {
        self.sel(cx).sel_checkpoint();
        match self.sel(cx).next_highlight(a.style, a.forward) {
            true => {
                self.marked.clear();
                self.after_edit(cx);
            }
            false => self.status(format!("Nothing is highlighted in style {}", a.style + 1), cx),
        }
    }

    fn copy_highlighted(&mut self, a: &CopyHighlighted, _: &mut Window, cx: &mut Context<Self>) {
        match self.read_state(cx).highlighted_text(a.style) {
            Some(text) => copy_to_clipboard(cx, (text, false)),
            None => self.status(format!("Nothing is highlighted in style {}", a.style + 1), cx),
        }
    }

    fn clear_highlights(&mut self, a: &ClearHighlights, _: &mut Window, cx: &mut Context<Self>) {
        self.sel(cx).clear_highlights(a.style);
        cx.notify();
    }

    /// Edit > Columns. A big document is parsed on another thread (the buffer keeps its version
    /// and the caret its place, or the result is dropped); a small one at once.
    fn column_command(&mut self, a: &ColumnCommand, window: &mut Window, cx: &mut Context<Self>) {
        if self.single_line {
            return;
        }
        let job = self.read_state(cx).column_job(a.op);
        if self.doc(cx).len() <= stats::BIG {
            let plan = job.run();
            return self.finish_column(&job, plan, false, window, cx);
        }
        self.status("Working…", cx);
        let task = cx.background_spawn(async move {
            let plan = job.run();
            (job, plan)
        });
        self.column_job = Some(cx.spawn_in(window, async move |this, cx| {
            let (job, plan) = task.await;
            this.update_in(cx, |e, window, cx| {
                e.column_job = None;
                e.finish_column(&job, plan, true, window, cx)
            })
            .ok();
        }));
    }

    /// A column command's plan reaches the buffer if it is still the one the plan is for:
    /// Copy Column goes on the clipboard and into its history; a document that does not parse
    /// is said so and left as it is.
    fn finish_column(
        &mut self,
        job: &ColumnJob,
        plan: Result<ColumnPlan, String>,
        was_slow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !job.is_current(&self.read_state(cx)) {
            return self.status("The document changed; run the command again", cx);
        }
        let plan = match plan {
            Ok(plan) => plan,
            Err(e) => {
                let title = "Cannot edit columns";
                let prompt = window.prompt(PromptLevel::Critical, title, Some(&e), &["OK"], cx);
                return drop(prompt);
            }
        };
        let mut copied = None;
        if matches!(plan, ColumnPlan::Edit(_)) {
            self.edit_text(cx, |s| copied = s.apply_column_plan(plan));
        } else {
            self.edit(cx, |s| copied = s.apply_column_plan(plan));
        }
        match copied {
            Some(text) => {
                copy_to_clipboard(cx, (text, false));
                self.status("Column copied", cx);
            }
            None if was_slow => self.status("Done", cx),
            None => {}
        }
    }
    /// Width 0 is Sublime's default: the first ruler, else 78.
    fn wrap_lines(&mut self, a: &WrapLines, _: &mut Window, cx: &mut Context<Self>) {
        let width = match a.width {
            0 => self.view.rulers.first().copied().unwrap_or(WRAP_WIDTH),
            w => w,
        };
        let (comment, _) = self.comment_markers(cx);
        self.edit_text(cx, |s| s.wrap_lines(width, comment.as_deref()));
    }
    fn trim_trailing_whitespace(&mut self, _: &TrimTrailingWhitespace, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.edit_text(cx, |s| s.clean_whitespace(false));
        }
    }

    fn remove_blank_lines(&mut self, _: &RemoveBlankLines, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.edit_text(cx, |s| s.clean_whitespace(true));
        }
    }

    fn convert_indentation(&mut self, a: &ConvertIndentation, _: &mut Window, cx: &mut Context<Self>) {
        self.edit_text(cx, |s| s.convert_indentation(a.to_spaces));
    }
    fn jump_back(&mut self, _: &JumpBack, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.jumps.pop() {
            self.jumps_fwd.push(self.state.cursor());
            self.edit(cx, |s| s.set_cursor(p));
        }
    }
    fn jump_forward(&mut self, _: &JumpForward, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.jumps_fwd.pop() {
            self.jumps.push(self.state.cursor());
            self.edit(cx, |s| s.set_cursor(p));
        }
    }

    pub(crate) fn move_to_fold(&mut self, a: &MoveToFold, _: &mut Window, cx: &mut Context<Self>) {
        self.traverse_folds(Some(a.motion), a.select, cx);
    }

    pub(crate) fn select_fold(&mut self, _: &SelectFold, _: &mut Window, cx: &mut Context<Self>) {
        self.traverse_folds(None, false, cx);
    }

    fn traverse_folds(&mut self, motion: Option<fold::Motion>, extend: bool, cx: &mut Context<Self>) {
        if self.single_line || self.aligned {
            return self.status("Fold navigation is available in document views", cx);
        }
        if !self.marked.is_empty() {
            return self.status("Finish or cancel text composition before navigating folds", cx);
        }
        self.catch_up(cx);
        // Refresh existing filter focus before installing new source destinations.
        self.reveal_selections(cx);
        let doc = self.doc(cx);
        let mut ranges = self.view_filter.as_ref().map_or(&self.folds, |f| &f.prior).chars();
        match self.highlighter.fold_regions(&doc.rope) {
            Some(regions) => ranges.extend(regions.chars()),
            None => {
                ranges.extend(fold::regions(&doc.rope, doc.tab_size).into_iter().map(|(h, l)| line_fold(doc, h, l)))
            }
        }
        let traversal = fold::Traversal::new(doc, ranges);
        // Resolve source targets before filter restoration can clamp them to saved folds.
        let selections: Vec<_> = self
            .state
            .selections
            .iter()
            .map(|s| match motion {
                Some(motion) => match traversal.destination(s.head, motion) {
                    Some(at) if extend => Selection { head: at, head_virtual: 0, goal: None, ..*s },
                    Some(at) => Selection::caret(at),
                    None => *s,
                },
                None => match traversal.current(s.head) {
                    Some(r) if s.anchor_point() > s.head_point() => Selection::new(r.end, r.start),
                    Some(r) => Selection::new(r.start, r.end),
                    None => *s,
                },
            })
            .collect();
        if selections == self.state.selections {
            return self.status("No fold destination", cx);
        }
        self.edit(cx, |s| s.select_preserving_primary(selections));
        self.reveal_selections(cx);
        self.scroll_to_cursor(cx);
    }

    /// Edit > Code Folding > Fold: a selection inside a line hides just its text (Sublime); a
    /// multi-line selection its lines after its first (one ending at a line's start ends on the
    /// line before); a caret, the innermost region around it.
    fn fold(&mut self, _: &Fold, _: &mut Window, cx: &mut Context<Self>) {
        let s = self.read_state(cx);
        let in_line = |x: &&Selection| !x.is_empty() && s.point(x.start()).0 == s.point(x.end()).0;
        let exact: Vec<Range<usize>> = (s.view.selections.iter()).filter(in_line).map(|x| x.range()).collect();
        let lines: Vec<(usize, usize)> =
            (s.view.selections.iter()).filter(|x| !in_line(x)).map(|x| s.covered_lines(x)).collect();
        let carets: Vec<usize> = (lines.iter()).filter(|l| l.1 <= l.0).map(|l| l.0).collect();
        // a scan out from a caret reads its block: past a few carets, one pass over the
        // document reads each line once (scanning out from 2,000 carets in an 8,000-line block
        // takes seconds)
        let syntax = self.highlighter.fold_regions(&s.doc.rope);
        let mut around = match syntax {
            Some(regions) => carets.iter().map(|&line| regions.innermost(line)).collect::<Vec<_>>(),
            None => {
                let found = match carets.len() > 16 {
                    true => fold::innermost_each(&s.doc.rope, s.doc.tab_size, &carets),
                    false => carets.iter().map(|&l| fold::innermost(s.doc, l)).collect(),
                };
                found.into_iter().map(|r| r.map(|(h, l)| line_fold(s.doc, h, l))).collect()
            }
        }
        .into_iter();
        let found: Vec<Range<usize>> = lines
            .into_iter()
            .filter_map(|(l0, l1)| if l1 > l0 { Some(line_fold(s.doc, l0, l1)) } else { around.next().flatten() })
            .collect();
        self.change_folds(cx, |f, s| f.fold_many(s, found.into_iter().chain(exact)));
    }

    /// Unfold: the folds on the selections' lines.
    fn unfold(&mut self, _: &Unfold, _: &mut Window, cx: &mut Context<Self>) {
        let s = self.read_state(cx);
        let lines: Vec<Range<usize>> = (s.view.selections.iter())
            .map(|x| {
                let (l0, l1) = s.covered_lines(x);
                s.line_start(l0)..s.line_end(l1)
            })
            .collect();
        self.change_folds(cx, |f, _| {
            f.unfold_at(&lines);
        });
    }

    fn unfold_all(&mut self, _: &UnfoldAll, _: &mut Window, cx: &mut Context<Self>) {
        self.change_folds(cx, |f, _| *f = Folds::default());
    }

    /// Fold Level N, in place of the view's folds. A big document's regions are found on
    /// another thread (from a rope clone), and land if the text is still the one scanned.
    fn fold_level(&mut self, a: &FoldLevel, _: &mut Window, cx: &mut Context<Self>) {
        self.clear_view_filter(cx);
        let (rope, tab, level) = (self.doc(cx).rope.clone(), self.doc(cx).tab_size, a.level);
        if let Some(regions) = self.highlighter.fold_regions(&rope) {
            let found = regions.at_level(level);
            return self.change_folds(cx, |f, s| {
                *f = Folds::default();
                f.fold_many(s, found);
            });
        }
        if self.doc(cx).len() <= stats::BIG {
            return self.set_folds(fold::at_level(&rope, tab, level), cx);
        }
        self.status("Working…", cx);
        let version = self.doc(cx).version;
        let found = cx.background_spawn(async move { fold::at_level(&rope, tab, level) });
        self.fold_job = Some(cx.spawn(async move |this, cx| {
            let found = found.await;
            this.update(cx, |e, cx| {
                e.fold_job = None;
                if e.doc(cx).version != version {
                    return e.status("The document changed; run the command again", cx);
                }
                e.set_folds(found, cx);
                e.status("Done", cx);
            })
            .ok();
        }));
    }

    fn set_folds(&mut self, found: Vec<(usize, usize)>, cx: &mut Context<Self>) {
        self.change_folds(cx, |f, s| {
            *f = Folds::default();
            f.fold_many(s, found.into_iter().map(|(h, l)| line_fold(s, h, l)));
        });
    }

    /// A click on a fold's placeholder: open it; the folds inside it stay folded.
    pub(crate) fn open_fold(&mut self, a: &OpenFold, _: &mut Window, cx: &mut Context<Self>) {
        self.change_folds(cx, |f, _| {
            f.unfold_from(a.at);
        });
    }

    /// Shared candidate source for gutter controls and their action. A current empty index
    /// is authoritative; only unavailable syntax uses indentation.
    pub(crate) fn fold_region(&self, doc: &Document, line: usize) -> Option<Range<usize>> {
        match self.highlighter.fold_regions(&doc.rope) {
            Some(regions) => regions.at_header(line),
            None => fold::region(doc, line).map(|(h, l)| line_fold(doc, h, l)),
        }
    }

    pub(crate) fn fold_header(&self, doc: &Document, line: usize) -> bool {
        match self.highlighter.fold_regions(&doc.rope) {
            Some(regions) => regions.at_header(line).is_some(),
            None => fold::heads(doc, line),
        }
    }

    /// A gutter button: unfold the folds that hide `line`'s line break, else fold the region it
    /// heads (beside a fold that ends the line).
    pub(crate) fn toggle_fold_at(&mut self, a: &ToggleFoldAt, _: &mut Window, cx: &mut Context<Self>) {
        let line = a.line.min(self.doc(cx).line_count() - 1);
        let end = self.doc(cx).line_end(line);
        let region = self.fold_region(self.doc(cx), line);
        self.change_folds(cx, |f, s| {
            if !f.unfold_over(end) {
                if let Some(range) = region {
                    f.fold(s, range);
                }
            }
        });
    }
}

/// What folding lines `header + 1 ..= last` hides: from the header's line end to the last
/// line's (nothing when the last is not below the header).
fn line_fold(s: &Document, header: usize, last: usize) -> Range<usize> {
    let last = last.min(s.line_count() - 1);
    match header < last {
        true => s.line_end(header)..s.line_end(last),
        false => 0..0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext};

    #[gpui::test]
    fn virtual_space_explicit_macro_replacements_keep_origin_and_policy(cx: &mut TestAppContext) {
        use crate::config::settings::VirtualSpace;
        for policy in [VirtualSpace::Rectangular, VirtualSpace::All, VirtualSpace::Off] {
            for overtype in [false, true] {
                let original = "a😀\na😀";
                let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(original), false, cx));
                editor.update_in(cx, |e, window, cx| {
                    e.set_virtual_space(policy, cx);
                    e.sel(cx).set_selections(vec![Selection::caret(2).in_rectangle(), Selection::caret(5)]);
                    if overtype {
                        e.perform_action(&ToggleOvertype, window, cx);
                    }
                    e.perform_action(
                        &crate::config::customization::InsertText { text: "界".into(), replace: Some([-2, 0]) },
                        window,
                        cx,
                    );
                    assert_eq!(e.text(cx), "界\n界");
                    e.perform_action(&Move::to(Motion::Right, false), window, cx);
                    let expected = match policy {
                        VirtualSpace::Rectangular => [(1, 1), (3, 0)],
                        VirtualSpace::All => [(1, 1), (3, 1)],
                        VirtualSpace::Off => [(2, 0), (3, 0)],
                    };
                    assert_eq!(e.state.selections.iter().map(Selection::head_point).collect::<Vec<_>>(), expected);
                    assert!(e.state.selections[0].rectangular);
                    assert!(!e.state.selections[1].rectangular);
                    assert_eq!(e.text(cx), "界\n界");
                    e.edit(cx, |s| s.undo());
                    assert_eq!(e.text(cx), original);
                    e.edit(cx, |s| s.redo());
                    assert_eq!(e.text(cx), "界\n界");
                });
            }
        }
    }

    #[gpui::test]
    fn navigation_ends_composition_before_the_next_input(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::new(), false, cx));
        let mut outcomes = Vec::new();
        for navigation in ["jump", "line", "restore", "range", "highlight"] {
            let outcome = editor.update_in(cx, |e, window, cx| {
                e.set_text("ab\ncd", cx);
                e.edit(cx, |s| s.set_cursor(1));
                e.replace_and_mark_text_in_range(None, "k", None, window, cx);
                match navigation {
                    "jump" => e.jump_to(4, cx),
                    "line" => e.goto_line(2, cx),
                    "restore" => e.restore_view(vec![Selection::caret(4)], (0, 0.), cx),
                    "range" => e.select_range(4..6, cx),
                    "highlight" => {
                        e.sel(cx).add_highlights(0, std::iter::once(4..6));
                        e.next_highlight(&NextHighlight { style: 0, forward: true }, window, cx);
                    }
                    _ => unreachable!(),
                }
                let unmarked = e.marked.is_empty();
                e.replace_text_in_range(None, "x", window, cx);
                (navigation, unmarked, e.text(cx))
            });
            outcomes.push(outcome);
        }
        assert_eq!(
            outcomes,
            [
                ("jump", true, "akb\nxcd".into()),
                ("line", true, "akb\nxcd".into()),
                ("restore", true, "akb\nxcd".into()),
                ("range", true, "akb\nx".into()),
                ("highlight", true, "akb\nx".into()),
            ]
        );
    }

    #[gpui::test]
    fn moving_the_caret_ends_composition_before_the_next_input(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("ab"), false, cx));
        editor.update_in(cx, |e, window, cx| {
            e.sel(cx).set_cursor(1);
            e.replace_and_mark_text_in_range(None, "k", None, window, cx);
            e.r#move(&Move::to(Motion::Left, false), window, cx);
            assert!(e.marked.is_empty());
            e.replace_text_in_range(None, "x", window, cx);
            assert_eq!(e.text(cx), "axkb");
        });
    }

    #[gpui::test]
    fn cutting_composed_text_does_not_leave_a_stale_mark(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("ab"), false, cx));
        editor.update_in(cx, |e, window, cx| {
            e.sel(cx).set_cursor(1);
            e.replace_and_mark_text_in_range(None, "k", Some(0..1), window, cx);
            e.cut(&Cut, window, cx);
            assert!(e.marked.is_empty());
            e.replace_text_in_range(None, "x", window, cx);
            assert_eq!(e.text(cx), "axb");
        });
    }

    #[gpui::test]
    fn replacing_panel_text_discards_old_composition_ranges(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("long query"), true, cx));
        editor.update_in(cx, |e, window, cx| {
            sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).len());
            e.replace_and_mark_text_in_range(None, "k", None, window, cx);
            e.set_text("a", cx);
            assert!(e.marked.is_empty());
            e.replace_text_in_range(None, "x", window, cx);
            assert_eq!(e.text(cx), "x");
        });
    }

    #[gpui::test]
    fn long_single_line_input_scrolls_to_its_caret(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::new(), true, cx));
        cx.run_until_parked();
        editor.update_in(cx, |e, window, cx| {
            e.replace_text_in_range(None, &"x".repeat(2_000), window, cx);
        });
        cx.run_until_parked();
        editor.read_with(cx, |e, _| {
            assert!(e.scroll_left > 0., "the end of the input must be visible");
            assert_eq!(e.scroll_top, 0.);
        });
        editor.update(cx, |e, cx| e.set_text("x", cx));
        cx.run_until_parked();
        editor.read_with(cx, |e, _| assert_eq!(e.scroll_left, 0.));
        editor.update_in(cx, |e, window, cx| {
            e.replace_text_in_range(None, &"x".repeat(2_000), window, cx);
        });
        cx.run_until_parked();
        editor.update_in(cx, |e, window, cx| {
            e.r#move(&Move::to(Motion::LineStart, false), window, cx);
        });
        cx.run_until_parked();
        editor.read_with(cx, |e, _| assert_eq!(e.scroll_left, 0.));
    }

    #[gpui::test]
    fn copy_as_html_does_nothing_in_a_one_line_field(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("query"), true, cx));
        rich::take();
        editor.update_in(cx, |e, window, cx| {
            e.copy_as_html(&CopyAsHtml, window, cx);
            assert!(e.html_job.is_none(), "no job was started");
        });
        cx.run_until_parked();
        assert!(rich::take().is_empty());
    }

    #[gpui::test]
    fn a_pending_html_copy_preserves_a_newer_clipboard_write(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("older"), false, cx));
        rich::take();
        editor.update_in(cx, |e, window, cx| {
            e.copy_as_html(&CopyAsHtml, window, cx);
            cx.write_to_clipboard(ClipboardItem::new_string("newer".into()));
        });
        for _ in 0..5_000 {
            cx.run_until_parked();
            if !editor.read_with(cx, |e, _| e.html_job.is_some()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!editor.read_with(cx, |e, _| e.html_job.is_some()));
        assert!(rich::take().is_empty(), "the older HTML must not replace it");
        assert_eq!(cx.read_from_clipboard().and_then(|i| i.text()).as_deref(), Some("newer"));
    }

    #[gpui::test]
    fn a_newer_html_copy_in_another_view_supersedes_the_older_one(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("older"), false, cx));
        let other = editor.update(cx, |_, cx| cx.new(|cx| Editor::new(EditorState::from_text("newer"), false, cx)));
        rich::take();
        editor.update_in(cx, |e, window, cx| e.copy_as_html(&CopyAsHtml, window, cx));
        other.update_in(cx, |e, window, cx| e.copy_as_html(&CopyAsHtml, window, cx));
        for _ in 0..5_000 {
            cx.run_until_parked();
            if ![&editor, &other].iter().any(|e| e.read_with(cx, |e, _| e.html_job.is_some())) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let written = rich::take();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].1, "newer");
    }
}

impl Editor {
    pub(crate) fn invert_selections(&mut self, _: &InvertSelections, _: &mut Window, cx: &mut Context<Self>) {
        if !self.marked.is_empty() {
            return;
        }
        self.catch_up(cx);
        self.sel(cx).invert_selections();
        self.after_edit(cx);
    }

    pub(crate) fn split_selections_by_regex(
        &mut self,
        a: &SplitSelectionsByRegex,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.regex_selection_set(&a.pattern, a.ignore_case, crate::editing::text::SelectionSetOp::Split, cx);
    }
    pub(crate) fn keep_selections_matching_regex(
        &mut self,
        a: &KeepSelectionsMatchingRegex,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.regex_selection_set(&a.pattern, a.ignore_case, crate::editing::text::SelectionSetOp::Keep, cx);
    }
    pub(crate) fn remove_matching_selections(
        &mut self,
        a: &RemoveMatchingSelections,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.regex_selection_set(&a.pattern, a.ignore_case, crate::editing::text::SelectionSetOp::Remove, cx);
    }
    fn regex_selection_set(
        &mut self,
        pattern: &str,
        ignore_case: bool,
        op: crate::editing::text::SelectionSetOp,
        cx: &mut Context<Self>,
    ) {
        if !self.marked.is_empty() {
            return;
        }
        self.catch_up(cx);
        match self.sel(cx).regex_selections(pattern, op, ignore_case) {
            Ok(()) => self.after_edit(cx),
            Err(error) => self.status(error, cx),
        }
    }
}

impl Editor {
    pub(crate) fn adjust_values(&mut self, action: &AdjustValues, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        self.catch_up(cx);
        match self.read_state(cx).adjust_value_edits(action.step) {
            Ok(edits) if !edits.is_empty() => self.edit_text(cx, |s| {
                s.replace_ranges(edits);
            }),
            Ok(_) => {}
            Err(error) => self.status(error, cx),
        }
    }
}

impl Editor {
    pub(crate) fn align_on_pattern(&mut self, action: &AlignOnPattern, _: &mut Window, cx: &mut Context<Self>) {
        self.align_fields(Some(action), cx);
    }
    pub(crate) fn align_selected_fields(&mut self, _: &AlignSelectedFields, _: &mut Window, cx: &mut Context<Self>) {
        self.align_fields(None, cx);
    }
    fn align_fields(&mut self, pattern: Option<&AlignOnPattern>, cx: &mut Context<Self>) {
        if !self.input_enabled || self.doc(cx).edits_blocked() || !self.marked.is_empty() {
            return;
        }
        self.catch_up(cx);
        let edits = match pattern {
            Some(a) => self.read_state(cx).align_pattern_edits(&a.pattern, a.capture, a.ignore_case),
            None => self.read_state(cx).align_selected_field_edits(),
        };
        match edits {
            Ok(edits) if !edits.is_empty() => self.edit_text(cx, |s| {
                if pattern.is_some() {
                    s.replace_ranges(edits);
                } else {
                    s.apply_selected_field_alignment(edits);
                }
            }),
            Ok(_) => {}
            Err(error) => self.status(error, cx),
        }
    }
}
