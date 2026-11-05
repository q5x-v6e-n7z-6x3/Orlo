//! The results browser along the bottom of the window, and Replace in Files' Apply. The rows
//! come from the window's structured result set (`crate::searching::results`): files grouped
//! and collapsible, each match with its location, its line, and for Replace in Files what
//! replaces it; checkboxes say what Apply changes, the filter only what is listed.

use crate::chrome::find_panel::dispatching;
use crate::files::watch::stamp;
use crate::searching::results::{escape, Body, ResultSet, Row, Source, Status};
use crate::workspace::*;
use gpui::{uniform_list, ScrollStrategy, SharedString, UniformListScrollHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

actions!(
    results_panel,
    [
        SelectNextRow,
        SelectPrevRow,
        ToggleRow,
        OpenRow,
        CollapseRow,
        ExpandRow,
        FocusFilter,
        FocusList,
        IncludeAll,
        ExcludeAll,
        ExpandAll,
        CollapseAll,
    ]
);

/// Bound after the editor's bindings so they win inside the filter field.
pub fn key_bindings() -> Vec<KeyBinding> {
    let list = Some("ResultsList");
    let filter = Some("ResultsPanel > Editor");
    vec![
        KeyBinding::new("up", SelectPrevRow, list),
        KeyBinding::new("down", SelectNextRow, list),
        KeyBinding::new("space", ToggleRow, list),
        KeyBinding::new("enter", OpenRow, list),
        KeyBinding::new("left", CollapseRow, list),
        KeyBinding::new("right", ExpandRow, list),
        KeyBinding::new("tab", FocusFilter, list),
        KeyBinding::new("cmd-f", FocusFilter, list),
        KeyBinding::new("escape", ClosePanel, list),
        KeyBinding::new("up", SelectPrevRow, filter),
        KeyBinding::new("down", SelectNextRow, filter),
        KeyBinding::new("enter", OpenRow, filter),
        KeyBinding::new("tab", FocusList, filter),
        KeyBinding::new("escape", ClosePanel, filter),
    ]
}

/// Rows a list shows at once; the panel is this tall plus its header.
const ROW_H: f32 = 18.;

pub struct ResultsPanel {
    pub filter: Entity<Editor>,
    /// The highlighted row, an index into `rows`.
    pub selected: usize,
    pub focus: FocusHandle,
    pub(crate) navigation: u64,
    scroll: UniformListScrollHandle,
    /// The rows as last listed, with the filter they were listed for.
    rows: Vec<Row>,
    /// A note under the counts: a stale location opened, the outcome of Apply.
    pub note: Option<String>,
}

/// Inactive structured pages; the selected page stays in Workspace.result_set for its callers.
#[derive(Default)]
pub(crate) struct ResultPages {
    pub(crate) active: u64,
    next: u64,
    pub(crate) request: u64,
    pub(crate) inactive: Vec<ResultPage>,
    browser: ResultBrowser,
}

pub(crate) struct ResultPage {
    pub(crate) id: u64,
    pub(crate) set: ResultSet,
    browser: ResultBrowser,
}

#[derive(Default)]
struct ResultBrowser {
    filter: Option<Entity<Editor>>,
    selected: usize,
    note: Option<String>,
}

/// Replace in Files applying: its cancel flag and the task doing it.
pub(crate) struct Applying {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

/// What Apply does with one file, decided on the UI thread.
enum Route {
    Done(Status),
    /// A closed file to write in the background, with the lease that keeps it from opening.
    Disk(DiskJob, Arc<()>),
}

/// A closed file's replacements, checked against the file before anything is written.
struct DiskJob {
    path: PathBuf,
    stamp: Option<crate::files::watch::Stamp>,
    hash: u64,
    line_ending: &'static str,
    bom: bool,
    encoding: crate::editing::encoding::Encoding,
    /// (bytes in the text as searched, replacement), sorted.
    edits: Vec<(std::ops::Range<usize>, String)>,
}

#[cfg(test)]
thread_local! {
    static BEFORE_PUBLISH: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

/// The write itself: the file must still be the bytes the preview read; its permissions,
/// byte order mark and line endings stay.
#[cfg(test)]
fn write_disk(job: &DiskJob) -> Status {
    write_disk_with_history(job, false)
}

fn write_disk_with_history(job: &DiskJob, history: bool) -> Status {
    let meta = match std::fs::metadata(&job.path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Status::Stale("it no longer exists".into()),
        Err(e) => return Status::Failed(e.to_string()),
    };
    if stamp(&job.path) != job.stamp {
        return Status::Stale("changed on disk since the preview".into());
    }
    let bytes = match search::read_bytes(&job.path) {
        Ok(b) => b,
        Err(None) => return Status::Skipped("file too large".into()),
        Err(Some(e)) => return Status::Failed(e),
    };
    if search::hash(&bytes) != job.hash {
        return Status::Stale("changed on disk since the preview".into());
    }
    let decoded = crate::editing::encoding::decode_owned(bytes, Some(job.encoding));
    if decoded.lossy || decoded.text.contains('\0') {
        return Status::Skipped("malformed or binary text".into());
    }
    let (mut rope, _, mut endings) = crate::editing::text::decoded_file(&decoded.text);
    let old = rope.to_string();
    let mut new = old.clone();
    for (r, t) in job.edits.iter().rev() {
        if new.get(r.clone()).is_none() {
            return Status::Stale("its text no longer has the matches".into());
        }
        let range = rope.byte_to_char(r.start)..rope.byte_to_char(r.end);
        endings.splice(&rope, range.clone(), t, job.line_ending);
        rope.remove(range.clone());
        rope.insert(range.start, t);
        new.replace_range(r.clone(), t);
    }
    if new == old {
        return Status::Unchanged;
    }
    // the safe save replaces the file by renaming over it, which a read-only mode would not stop
    if meta.permissions().readonly() {
        return Status::Failed("the file is read-only".into());
    }
    let out = match crate::editing::encoding::encode_owned(
        match endings.text(&rope, 0..rope.len_chars()) {
            Ok(text) => text,
            Err(e) => return Status::Failed(e.to_string()),
        },
        decoded.encoding,
        job.bom,
    ) {
        Ok(bytes) => bytes,
        Err(e) => return Status::Failed(e.to_string()),
    };
    #[cfg(test)]
    if let Some(change) = BEFORE_PUBLISH.take() {
        change(&job.path);
    }
    if stamp(&job.path) != job.stamp {
        return Status::Stale("changed on disk while replacements were built".into());
    }
    if let Err(e) = crate::editing::text::capture_history(&job.path, decoded.encoding, history) {
        return Status::Failed(e.to_string());
    }
    if stamp(&job.path) != job.stamp || search::read_bytes(&job.path).map(|b| search::hash(&b)).ok() != Some(job.hash) {
        return Status::Stale("changed while capturing history".into());
    }
    let write = |f: &mut std::fs::File| std::io::Write::write_all(f, &out);
    match crate::editing::text::write_safely(&job.path, &write) {
        Ok(()) => Status::Changed,
        Err((e, true)) => Status::Failed(format!("write failed, the file may be partly written: {e}")),
        Err((e, false)) => Status::Failed(e.to_string()),
    }
}

fn result_filter(cx: &mut Context<Workspace>) -> Entity<Editor> {
    let filter = cx.new(|cx| Editor::new(EditorState::new(), true, cx));
    let id = filter.entity_id();
    Editor::on_text_change(&filter, cx, move |ws, cx| {
        if let Some(p) = ws.results_panel_mut().filter(|p| p.filter.entity_id() == id) {
            p.invalidate_open();
            p.selected = 0;
            cx.notify();
        }
    })
    .detach();
    filter
}

impl ResultsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let filter = result_filter(cx);
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            filter,
            selected: 0,
            focus,
            navigation: 0,
            scroll: UniformListScrollHandle::new(),
            rows: Vec::new(),
            note: None,
        }
    }

    pub(crate) fn invalidate_open(&mut self) {
        self.navigation = self.navigation.wrapping_add(1);
    }
}

/// Apply's edits made in an open buffer, one undo step left unsaved.
fn replace_in_buffer(editor: &Entity<Editor>, edits: Vec<(search::Hit, String)>, cx: &mut App) -> Status {
    let ranges: Vec<_> = (edits.into_iter()).map(|(h, t)| (h.chars, t)).collect();
    let mut changed = false;
    editor.update(cx, |e, cx| e.edit(cx, |s| changed = s.replace_ranges(ranges)));
    match changed {
        true => Status::ChangedInBuffer,
        false => Status::Unchanged,
    }
}

/// `line` with `range` (chars) marked: (before, the match, after), cut around the match.
fn excerpt(line: &str, range: std::ops::Range<usize>) -> (String, String, String) {
    let (shown, kept) = search::cut(line, std::slice::from_ref(&range));
    let r = kept.first().cloned().unwrap_or(0..0);
    let chars: Vec<char> = shown.chars().collect();
    let take = |r: std::ops::Range<usize>| chars[r.start.min(chars.len())..r.end.min(chars.len())].iter().collect();
    (take(0..r.start), take(r.clone()), take(r.end..chars.len()))
}

impl Workspace {
    pub(crate) fn ensure_result_page(&mut self) {
        if self.result_pages.active == 0 && self.result_set.is_some() {
            self.result_pages.next += 1;
            self.result_pages.active = self.result_pages.next;
        }
    }

    pub(crate) fn result_page(&self, id: u64) -> Option<&ResultSet> {
        if self.result_pages.active == id {
            self.result_set.as_ref()
        } else {
            self.result_pages.inactive.iter().find(|p| p.id == id).map(|p| &p.set)
        }
    }

    pub(crate) fn result_page_mut(&mut self, id: u64) -> Option<&mut ResultSet> {
        if self.result_pages.active == id {
            self.result_set.as_mut()
        } else {
            self.result_pages.inactive.iter_mut().find(|p| p.id == id).map(|p| &mut p.set)
        }
    }

    pub(crate) fn remember_result_browser(&mut self, _: &App) {
        if self.result_set.is_none() {
            return;
        }
        if let Some(p) = self.results_panel() {
            self.result_pages.browser =
                ResultBrowser { filter: Some(p.filter.clone()), selected: p.selected, note: p.note.clone() };
        }
    }

    fn restore_result_browser(&mut self, cx: &mut Context<Self>) {
        if self.results_panel().is_none() {
            return;
        }
        let filter = self.result_pages.browser.filter.clone().unwrap_or_else(|| result_filter(cx));
        self.result_pages.browser.filter = Some(filter.clone());
        let (selected, note) = (self.result_pages.browser.selected, self.result_pages.browser.note.clone());
        if let Some(p) = self.results_panel_mut() {
            p.filter = filter;
            p.invalidate_open();
            p.rows.clear();
            p.selected = selected;
            p.note = note;
            p.scroll.scroll_to_item(selected, ScrollStrategy::Center);
        }
    }

    /// Admission precedes cancellation: an active/running page is never an eviction victim.
    pub(crate) fn admit_result_page(&self) -> Result<Option<u64>, String> {
        if self.result_pages.inactive.len() + usize::from(self.result_set.is_some()) < 8 {
            return Ok(None);
        }
        self.result_pages
            .inactive
            .iter()
            .filter(|p| p.set.done)
            .map(|p| p.id)
            .min()
            .map(Some)
            .ok_or_else(|| "All eight search pages are active or running; close a page before searching again".into())
    }

    pub(crate) fn create_result_page(&mut self, retire: Option<u64>, cx: &mut Context<Self>) {
        self.ensure_result_page();
        self.remember_result_browser(cx);
        if let Some(id) = retire {
            self.result_pages.inactive.retain(|p| p.id != id);
        }
        let browser = std::mem::take(&mut self.result_pages.browser);
        if let Some(set) = self.result_set.take() {
            self.result_pages.inactive.push(ResultPage { id: self.result_pages.active, set, browser });
        }
        self.result_pages.next += 1;
        self.result_pages.active = self.result_pages.next;
        self.restore_result_browser(cx);
    }

    pub(crate) fn reset_result_review(&mut self) {
        self.result_pages.browser.note = None;
        if let Some(p) = self.results_panel_mut() {
            p.note = None;
        }
    }

    pub(crate) fn select_result_page(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying.is_some() {
            return self.status_message("Replacements are being applied; wait for them or cancel", cx);
        }
        if id == self.result_pages.active {
            return;
        }
        let Some(at) = self.result_pages.inactive.iter().position(|p| p.id == id) else { return };
        self.remember_result_browser(cx);
        let page = self.result_pages.inactive.remove(at);
        if let Some(set) = self.result_set.replace(page.set) {
            self.result_pages.inactive.push(ResultPage {
                id: self.result_pages.active,
                set,
                browser: std::mem::take(&mut self.result_pages.browser),
            });
        }
        self.result_pages.active = id;
        self.result_pages.browser = page.browser;
        self.restore_result_browser(cx);
        self.show_results_panel(window, cx);
    }

    pub(crate) fn close_result_page(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying.is_some() {
            return self.status_message("Replacements are being applied; wait for them or cancel", cx);
        }
        if self.result_page(id).is_none() {
            return;
        }
        if self.search.as_ref().is_some_and(|s| s.page == id) {
            self.stop_search(cx);
        }
        if self.result_pages.active != id {
            self.result_pages.inactive.retain(|p| p.id != id);
        } else {
            let next = self.result_pages.inactive.iter().map(|p| p.id).max();
            self.result_set = None;
            self.result_pages.active = 0;
            self.result_pages.browser = ResultBrowser::default();
            if let Some(id) = next {
                self.select_result_page(id, window, cx);
            } else {
                self.close_panel(&ClosePanel, window, cx);
            }
        }
        cx.notify();
    }

    pub fn results_panel(&self) -> Option<&ResultsPanel> {
        match &self.panel {
            Some(Panel::Results(p)) => Some(p),
            _ => None,
        }
    }

    pub fn results_panel_mut(&mut self) -> Option<&mut ResultsPanel> {
        match &mut self.panel {
            Some(Panel::Results(p)) => Some(p),
            _ => None,
        }
    }

    /// The browser for the window's result set; it keeps the keys.
    pub(crate) fn show_results_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_result_page();
        self.reveal_panel(cx);
        if self.results_panel().is_some() {
            self.focus_results(window);
        } else {
            let p = ResultsPanel::new(window, cx);
            self.show_panel(Panel::Results(p), cx);
            self.restore_result_browser(cx);
        }
        cx.notify();
    }

    /// Find > Search Results: the last Find in Files results in the browser.
    pub(crate) fn show_results(&mut self, _: &ShowResults, window: &mut Window, cx: &mut Context<Self>) {
        if self.result_set.is_none() {
            return self.status_message("No Find in Files results in this window yet", cx);
        }
        self.show_results_panel(window, cx);
    }

    /// The rows for the filter as typed, listed again whenever the panel draws (results
    /// streaming in, a checkbox, a collapse).
    pub(crate) fn results_rows(&mut self, cx: &App) -> Vec<Row> {
        let Some(p) = self.results_panel() else {
            return Vec::new();
        };
        let filter = p.filter.read(cx).text(cx);
        let rows = (self.result_set.as_ref()).map_or_else(Vec::new, |r| r.rows(&filter));
        if let Some(p) = self.results_panel_mut() {
            p.rows = rows.clone();
            p.selected = p.selected.min(rows.len().saturating_sub(1));
        }
        rows
    }

    fn selected_row(&self) -> Option<Row> {
        let p = self.results_panel()?;
        p.rows.get(p.selected).copied()
    }

    fn select_row(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.results_panel_mut() {
            if p.selected != i {
                p.invalidate_open();
            }
            p.selected = i;
            p.scroll.scroll_to_item(i, ScrollStrategy::Center);
        }
        cx.notify();
    }

    pub(crate) fn select_next_row(&mut self, _: &SelectNextRow, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.results_rows(cx).len();
        let next = self.results_panel().map_or(0, |p| (p.selected + 1).min(n.saturating_sub(1)));
        self.select_row(next, cx);
    }

    pub(crate) fn select_prev_row(&mut self, _: &SelectPrevRow, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.results_panel().map_or(0, |p| p.selected.saturating_sub(1));
        self.select_row(prev, cx);
    }

    pub(crate) fn toggle_row(&mut self, _: &ToggleRow, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        self.toggle_inclusion(row, cx);
    }

    fn toggle_inclusion(&mut self, row: Row, cx: &mut Context<Self>) {
        if self.applying.is_some() {
            return;
        }
        if let Some(set) = &mut self.result_set {
            match row {
                Row::File(i) => set.toggle_file(i),
                Row::Match(i, j) => set.toggle_match(i, j),
                Row::Context(..) => {}
            }
        }
        cx.notify();
    }

    fn collapse(&mut self, i: usize, collapsed: Option<bool>, cx: &mut Context<Self>) {
        if let Some(f) = self.result_set.as_mut().and_then(|r| r.files.get_mut(i)) {
            f.collapsed = collapsed.unwrap_or(!f.collapsed);
        }
        let rows = self.results_rows(cx);
        if let Some(at) = rows.iter().position(|r| *r == Row::File(i)) {
            self.select_row(at, cx);
        }
        cx.notify();
    }

    pub(crate) fn collapse_row(&mut self, _: &CollapseRow, _: &mut Window, cx: &mut Context<Self>) {
        let i = match self.selected_row() {
            Some(Row::File(i) | Row::Match(i, _) | Row::Context(i, _)) => i,
            None => return,
        };
        self.collapse(i, Some(true), cx);
    }

    pub(crate) fn expand_row(&mut self, _: &ExpandRow, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(Row::File(i)) = self.selected_row() {
            self.collapse(i, Some(false), cx);
        }
    }

    forward_actions! {
    pub(crate) include_all(IncludeAll, _, cx) => include_every_row(true, cx);
    pub(crate) exclude_all(ExcludeAll, _, cx) => include_every_row(false, cx);
    pub(crate) expand_all(ExpandAll, _, cx) => collapse_every_file(false, cx);
    pub(crate) collapse_all(CollapseAll, _, cx) => collapse_every_file(true, cx);
    pub(crate) focus_list(FocusList, window, _) => focus_results(window);
    }

    fn include_every_row(&mut self, included: bool, cx: &mut Context<Self>) {
        if self.applying.is_none() {
            if let Some(r) = self.result_set.as_mut() {
                r.set_all(included);
            }
        }
        cx.notify();
    }

    fn collapse_every_file(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        for f in self.result_set.iter_mut().flat_map(|r| &mut r.files) {
            f.collapsed = collapsed;
        }
        cx.notify();
    }

    /// The keys back in the browser, so F4 and the arrows go on.
    fn focus_results(&self, window: &mut Window) {
        if let Some(p) = self.results_panel() {
            window.focus(&p.focus.clone());
        }
    }

    pub(crate) fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.results_panel() {
            p.filter.read(cx).focus(window);
        }
    }

    pub(crate) fn open_row(&mut self, _: &OpenRow, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_row() {
            Some(Row::File(i)) => self.open_hit(i, None, window, cx),
            Some(Row::Match(i, j)) => self.open_hit(i, Some(j), window, cx),
            Some(Row::Context(i, l)) => self.open_line(i, l, window, cx),
            None => {}
        }
    }

    /// F4 / Shift-F4 with the browser open: the next or previous match row, opened (wraps).
    pub(crate) fn step_browser(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.results_rows(cx);
        let at = self.results_panel().map_or(0, |p| p.selected);
        let is_match = |r: &Row| matches!(r, Row::Match(..));
        let n = rows.len();
        let next = (1..=n)
            .map(|k| match forward {
                true => (at + k) % n.max(1),
                false => (at + n - k % n.max(1)) % n.max(1),
            })
            .find(|&i| rows.get(i).is_some_and(is_match));
        if let Some(i) = next {
            self.select_row(i, cx);
            if let Row::Match(f, j) = rows[i] {
                self.open_hit(f, Some(j), window, cx);
            }
        }
    }

    /// Whether file `i` still has the text its results were found in; why not, if not.
    fn source_now(&self, i: usize, cx: &App) -> Option<String> {
        let f = self.result_set.as_ref()?.files.get(i)?;
        let open =
            (self.open_editors(cx).into_iter()).find(|e| e.read(cx).doc(cx).path.as_deref() == Some(f.path.as_path()));
        match (&f.source, open) {
            (Source::Buffer { id, version }, Some(e))
                if e.read(cx).doc(cx).id() == *id && e.read(cx).doc(cx).version == *version =>
            {
                None
            }
            (Source::Buffer { .. }, Some(_)) => Some("edited since the search".into()),
            (Source::Buffer { .. }, None) if !f.path.exists() => Some("it no longer exists".into()),
            (Source::Buffer { .. }, None) => Some("closed since the search".into()),
            (Source::Disk { .. }, _) if !f.path.exists() => Some("it no longer exists".into()),
            (Source::Disk { stamp: s, .. }, _) if stamp(&f.path) != *s => {
                Some("changed on disk since the search".into())
            }
            (Source::Disk { encoding, .. }, Some(e)) if e.read(cx).doc(cx).disk_encoding != *encoding => {
                Some("reinterpreted since the search".into())
            }
            (Source::Disk { .. }, Some(e)) if e.read(cx).doc(cx).dirty => {
                Some("open with unsaved changes since the search".into())
            }
            _ => None,
        }
    }

    /// Open file `i` at match `j` (its first line without one), the whole match selected. A
    /// file changed since the search opens at the location as recorded, and the panel says the
    /// location may be stale; one that is gone is only reported.
    fn open_hit(&mut self, i: usize, j: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let browser = self.begin_browser_jump();
        let Some(f) = self.result_set.as_ref().and_then(|r| r.files.get(i)) else {
            return;
        };
        let (path, hit, source_stamp) = (
            f.path.clone(),
            j.and_then(|j| f.entries().get(j)).map(|e| e.hit.clone()),
            match &f.source {
                Source::Disk { stamp, .. } => *stamp,
                _ => None,
            },
        );
        let stale = self.source_now(i, cx);
        if let Some(p) = self.results_panel_mut() {
            p.note = stale.as_ref().map(|why| {
                format!("{}: {why}; the location may have moved. Refresh to search again.", search::tilde(&path))
            });
        }
        if !path.exists() {
            return cx.notify();
        }
        let line = hit.as_ref().map_or(0, |h| h.start.0);
        let col = hit.as_ref().map_or(0, |h| h.start.1);
        self.last_results = None;
        self.open_path_here(path.clone(), window, cx);
        let editor = (self.active_editor()).filter(|e| e.read(cx).doc(cx).path.as_deref() == Some(path.as_path()));
        match editor {
            Some(e) => e.update(cx, |e, cx| {
                let range = match (&hit, stale.is_none()) {
                    (Some(h), true) if h.chars.end <= e.doc(cx).len() => h.chars.clone(),
                    _ => {
                        let l = line.min(e.doc(cx).line_count().saturating_sub(1));
                        let at = e.doc(cx).offset(l, col);
                        at..at
                    }
                };
                e.select_range(range, cx);
            }),
            // read in the background: the jump waits for it
            None => {
                let selection = hit
                    .as_ref()
                    .filter(|_| stale.is_none())
                    .and_then(|h| source_stamp.map(|known| (h.chars.clone(), known)));
                self.pending_jumps.insert(path, PendingJump { line, hit: Some((col, None)), selection, browser });
            }
        }
        self.focus_results(window);
        cx.notify();
    }

    fn open_line(&mut self, i: usize, l: usize, window: &mut Window, cx: &mut Context<Self>) {
        let browser = self.begin_browser_jump();
        let Some(f) = self.result_set.as_ref().and_then(|r| r.files.get(i)) else {
            return;
        };
        let path = f.path.clone();
        if !path.exists() {
            return;
        }
        self.open_path_here(path.clone(), window, cx);
        match self.active_editor().filter(|e| e.read(cx).doc(cx).path.as_ref() == Some(&path)) {
            Some(editor) => editor.update(cx, |e, cx| e.goto_line(l + 1, cx)),
            None => {
                self.pending_jumps.insert(path, PendingJump { line: l, hit: None, selection: None, browser });
            }
        }
        self.focus_results(window);
    }

    fn begin_browser_jump(&mut self) -> Option<BrowserJump> {
        let origin = self.active_editor().map(|e| e.entity_id());
        let panel = self.results_panel_mut()?;
        panel.invalidate_open();
        Some(BrowserJump { origin, focus: panel.focus.clone(), navigation: panel.navigation })
    }

    /// Apply: every included replacement of a file whose text is still the one previewed, one
    /// undo step per open buffer (left unsaved) or a safe write per closed file; anything else
    /// is reported, nothing guessed. Cancel stops before the next file.
    pub(crate) fn apply_replacements(&mut self, _: &ApplyReplacements, window: &mut Window, cx: &mut Context<Self>) {
        let Some(set) = &self.result_set else { return };
        if set.replacement.is_none() || self.applying.is_some() {
            return;
        }
        if !set.done {
            return self.status_message("Wait for the search to finish, or cancel it", cx);
        }
        if set.counts().selected == 0 {
            return self.status_message("No replacements are selected", cx);
        }
        if set.files.iter().any(|f| f.status.is_some()) {
            return self.status_message("These results were applied: Refresh to preview again", cx);
        }
        let n = set.files.len();
        self.ensure_result_page();
        let page = self.result_pages.active;
        let history = cx.global::<Settings>().file_history;
        let cancel = Arc::new(AtomicBool::new(false));
        let task = cx.spawn_in(window, {
            let cancel = cancel.clone();
            async move |this, cx| {
                let mut cancelled = false;
                for i in 0..n {
                    if !this.read_with(cx, |ws, _| ws.result_pages.active == page).unwrap_or(false) {
                        return;
                    }
                    if cancel.load(Ordering::Relaxed) {
                        cancelled = true;
                    }
                    let route = match cancelled {
                        true => Some(Route::Done(Status::NotReached)),
                        false => this.update(cx, |ws, cx| ws.route_apply(i, cx)).ok(),
                    };
                    let status = match route {
                        None => return,
                        Some(Route::Done(s)) => s,
                        Some(Route::Disk(job, lease)) => {
                            cx.background_spawn(async move {
                                let _lease = lease;
                                write_disk_with_history(&job, history)
                            })
                            .await
                        }
                    };
                    let set_status = this.update(cx, |ws, cx| {
                        if ws.result_pages.active != page {
                            return;
                        }
                        if let Some(f) = ws.result_set.as_mut().and_then(|r| r.files.get_mut(i)) {
                            f.status = Some(status);
                        }
                        cx.notify();
                    });
                    if set_status.is_err() {
                        return;
                    }
                }
                this.update_in(cx, |ws, window, cx| {
                    if ws.result_pages.active == page {
                        ws.finish_apply(cancelled, window, cx);
                    }
                })
                .ok();
            }
        });
        self.applying = Some(Applying { cancel, _task: task });
        cx.notify();
    }

    pub(crate) fn cancel_replacements(&mut self, _: &CancelReplacements, _: &mut Window, cx: &mut Context<Self>) {
        match &self.applying {
            Some(a) => a.cancel.store(true, Ordering::Relaxed),
            None => self.status_message("No replacements are being applied", cx),
        }
    }

    fn finish_apply(&mut self, cancelled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.applying = None;
        let Some(set) = &self.result_set else { return };
        let report = set.report(cancelled);
        let count = |f: fn(&Status) -> bool| (set.files.iter()).filter(|r| r.status.as_ref().is_some_and(f)).count();
        let summary = format!(
            "Replace in Files: {} changed on disk, {} in open buffers, {} stale, {} skipped, {} failed{}",
            count(|s| *s == Status::Changed),
            count(|s| *s == Status::ChangedInBuffer),
            count(|s| matches!(s, Status::Stale(_))),
            count(|s| matches!(s, Status::Skipped(_))),
            count(|s| matches!(s, Status::Failed(_))),
            if cancelled { " (cancelled)" } else { "" },
        );
        if let Some(p) = self.results_panel_mut() {
            p.note = Some(summary.clone());
        }
        self.scratch_tab(&report, "Replace Report".into(), window, cx);
        self.status_message(summary, cx);
        // the browser stays in front of the report
        self.focus_results(window);
    }

    /// One file of Apply, decided against the buffers and files as they are now.
    fn route_apply(&mut self, i: usize, cx: &mut Context<Self>) -> Route {
        if let Some(source) = self
            .result_set
            .as_ref()
            .and_then(|r| r.previous_lines.as_ref())
            .and_then(|sources| sources.iter().find(|s| self.result_set.as_ref().unwrap().files[i].path == s.path))
        {
            let open = self.open_editors(cx);
            if let Some(why) = super::find::previous_line_problem(source, &open, cx) {
                return Route::Done(Status::Stale(format!(
                    "{why}; refresh the source search and choose its lines again"
                )));
            }
            for editor in &open {
                if editor.read(cx).doc(cx).path.as_ref().is_some_and(|p| {
                    crate::editing::text::resolve(p) == source.path
                        || source.file.is_some_and(|file| pipelines::file_id(p) == Some(file))
                }) {
                    if !editor.read(cx).input_enabled || editor.read(cx).doc(cx).edits_blocked() {
                        return Route::Done(Status::Skipped("This source is read-only".into()));
                    }
                    if let Err(why) = self.selection_set_admission(editor, cx) {
                        return Route::Done(Status::Skipped(why));
                    }
                }
            }
        }
        // moved or renamed in Orlo since the preview: its path names nothing now
        if self.moved_since_preview(i) {
            let why = "moved or renamed since the preview; Refresh";
            return Route::Done(Status::Stale(why.into()));
        }
        let Some(set) = &self.result_set else {
            return Route::Done(Status::NotReached);
        };
        let use_buffer = set.flags.use_buffer;
        let f = &set.files[i];
        let edits: Vec<_> = (f.selected()).filter_map(|e| Some((e.hit.clone(), e.hit.replacement.clone()?))).collect();
        if edits.is_empty() {
            return Route::Done(match &f.body {
                Body::Matches(_) => Status::Excluded,
                Body::Binary => Status::Skipped("binary file".into()),
                Body::TooLarge => Status::Skipped("over 100 MB".into()),
                Body::Failed(why) => Status::Skipped(why.clone()),
            });
        }
        let path = f.path.clone();
        let source = f.source.clone();
        let mut buffers: Vec<Entity<Editor>> = Vec::new();
        for e in self.open_editors(cx) {
            let v = e.read(cx);
            // Shared protection also covers earlier disk previews and another window's aliases.
            if v.doc(cx).edits_blocked()
                && v.doc(cx).path.as_ref().is_some_and(|open| {
                    open == &path
                        || crate::editing::text::resolve(open) == crate::editing::text::resolve(&path)
                        || pipelines::file_id(open).zip(pipelines::file_id(&path)).is_some_and(|(a, b)| a == b)
                })
            {
                return Route::Done(Status::Skipped(
                    if v.doc(cx).viewer_mode() {
                        "Viewer mode; turn it off before replacing this file"
                    } else {
                        "Following log; use Stop Following Log before replacing this file"
                    }
                    .into(),
                ));
            }
            if v.doc(cx).path.as_ref() == Some(&path) && !buffers.iter().any(|b| b.read(cx).same_document(v)) {
                buffers.push(e);
            }
        }
        if pipelines::writing(&path, cx) || self.loading.contains(&path) {
            return Route::Done(Status::Skipped("another operation is writing or reading it".into()));
        }
        match source {
            Source::Buffer { id, version } => {
                let Some(e) = buffers.first() else {
                    return Route::Done(Status::Stale("closed since the preview".into()));
                };
                if e.read(cx).doc(cx).id() != id || e.read(cx).doc(cx).version != version {
                    return Route::Done(Status::Stale("edited since the preview".into()));
                }
                if e.read(cx).doc(cx).read_only {
                    return Route::Done(Status::Skipped("malformed text; reopen with the correct encoding".into()));
                }
                Route::Done(replace_in_buffer(e, edits, cx))
            }
            Source::Disk { stamp, hash, line_ending, bom, encoding, lossy, binary } => {
                if lossy {
                    return Route::Done(Status::Skipped("malformed text; reopen with the correct encoding".into()));
                }
                if binary {
                    return Route::Done(Status::Skipped("binary file".into()));
                }
                if !buffers.is_empty() {
                    // opened since the preview: still the previewed text only while every view
                    // of it is unedited and was read from the file as it was then
                    let s = |e: &Entity<Editor>| e.read(cx).doc(cx);
                    let as_read = buffers.iter().all(|e| {
                        !s(e).dirty
                            && s(e).disk == stamp
                            && s(e).disk_encoding == encoding
                            && s(e).bom == bom
                            && s(e).line_ending == line_ending
                    });
                    let ignored = buffers.iter().any(|e| !s(e).watching);
                    if use_buffer {
                        if !as_read {
                            let why = "edited in Orlo since the preview";
                            return Route::Done(Status::Stale(why.into()));
                        }
                        return Route::Done(replace_in_buffer(&buffers[0], edits, cx));
                    }
                    if !as_read {
                        return Route::Done(Status::Skipped(
                            "open in Orlo with unsaved changes; save or revert it (or turn on \
                             Use Buffer), then Refresh"
                                .into(),
                        ));
                    }
                    if ignored {
                        return Route::Done(Status::Skipped(
                            "open in Orlo with outside changes ignored; close it or turn on Use \
                             Buffer, then Refresh"
                                .into(),
                        ));
                    }
                    // clean and watched: the write reloads it, as any outside change does
                }
                let job = DiskJob {
                    path: path.clone(),
                    stamp,
                    hash,
                    line_ending,
                    bom,
                    encoding,
                    edits: edits.into_iter().map(|(h, t)| (h.bytes, t)).collect(),
                };
                Route::Disk(job, pipelines::lease(&path, cx))
            }
        }
    }

    pub(crate) fn render_results_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_result_page();
        // Inactive headers inspect at most 19 query scalars and never visit their hit sets.
        let mut pages: Vec<_> = self
            .result_pages
            .inactive
            .iter()
            .map(|p| (p.id, p.set.query.as_str()))
            .chain(self.result_set.iter().map(|set| (self.result_pages.active, set.query.as_str())))
            .map(|(id, query)| {
                let mut chars = query.chars();
                let mut label: String = chars.by_ref().take(18).collect();
                if chars.next().is_some() {
                    label.push('…');
                }
                (id, escape(&label))
            })
            .collect();
        pages.sort_by_key(|p| p.0);
        let active_page = self.result_pages.active;
        let rows = self.results_rows(cx);
        let Some(p) = self.results_panel() else {
            return div().into_any_element();
        };
        let (filter, focus, scroll, selected, note) =
            (p.filter.clone(), p.focus.clone(), p.scroll.clone(), p.selected, p.note.clone());
        let set = self.result_set.as_ref();
        let replacing = set.is_some_and(|r| r.replacement.is_some());
        let title = set.map_or(String::new(), |r| match &r.replacement {
            Some(rep) => format!("Replace “{}” with “{}”", escape(&r.query), escape(rep)),
            None => format!("Find “{}”", escape(&r.query)),
        });
        let summary = set.map_or(String::new(), ResultSet::summary);
        let applying = self.applying.is_some();
        let n = rows.len();
        let list = uniform_list(
            "results-rows",
            n,
            cx.processor(move |ws, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|i| {
                        let row = *ws.results_panel()?.rows.get(i)?;
                        Some(ws.result_row(i, row, i == selected, cx))
                    })
                    .collect()
            }),
        )
        .size_full()
        .track_scroll(scroll);
        let filter_empty = filter.read(cx).doc(cx).is_empty();
        div()
            .key_context("ResultsPanel")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(ui(theme::PANEL_BG))
            .border_t_1()
            .border_color(ui(theme::BORDER))
            .text_color(ui(theme::UI_FG))
            .child(div().flex().flex_wrap().gap(px(4.)).px(px(10.)).py(px(3.)).children(pages.into_iter().map(
                |(id, query)| {
                    let label = format!("{} {}", if id == active_page { "●" } else { "○" }, query);
                    div()
                        .flex()
                        .items_center()
                        .child(
                            kit::ghost_button(SharedString::from(format!("result-page-{id}")), label)
                                .debug_selector(move || format!("result-page-{id}"))
                                .tooltip(kit::tooltip(query.into()))
                                .on_click(cx.listener(move |ws, _, window, cx| ws.select_result_page(id, window, cx))),
                        )
                        .child(
                            kit::icon_button(
                                SharedString::from(format!("close-result-page-{id}")),
                                Icon::Close,
                                "Close Search Page",
                            )
                            .debug_selector(move || format!("close-result-page-{id}"))
                            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                            .on_click(cx.listener(move |ws, _, window, cx| ws.close_result_page(id, window, cx))),
                        )
                },
            )))
            .child(
                kit::panel_header()
                    .child(icon(Icon::Search, 14.).text_color(ui(theme::ICON_MUTED)))
                    .child(div().flex_shrink_0().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
                    .child(
                        div()
                            .id("results-summary")
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(11.))
                            .text_color(ui(theme::TEXT_MUTED))
                            .child(summary),
                    )
                    .when(replacing, |d| {
                        d.child(dispatching(kit::ghost_button("results-include", "Select All"), IncludeAll))
                            .child(dispatching(kit::ghost_button("results-exclude", "Select None"), ExcludeAll))
                    })
                    .child(dispatching(kit::icon_button("results-expand", Icon::ChevronDown, "Expand All"), ExpandAll))
                    .child(dispatching(
                        kit::icon_button("results-collapse", Icon::Collapse, "Collapse All"),
                        CollapseAll,
                    ))
                    .child(dispatching(kit::icon_button("results-refresh", Icon::Refresh, "Refresh"), RefreshResults))
                    .when(replacing && !applying, |d| {
                        d.child(dispatching(kit::button("results-apply", "Apply").ml(px(4.)), ApplyReplacements))
                    })
                    .when(applying, |d| {
                        d.child(dispatching(kit::button("results-cancel", "Cancel").ml(px(4.)), CancelReplacements))
                    })
                    .child(
                        kit::icon_button("close-results", Icon::Close, "Close  ⎋")
                            .ml(px(4.))
                            .on_click(|_: &ClickEvent, window, cx| window.dispatch_action(Box::new(ClosePanel), cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .px(px(10.))
                    .py(px(8.))
                    .child(
                        kit::field()
                            .relative()
                            .h(px(26.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(icon(Icon::Search, 12.).ml(px(7.)).text_color(ui(theme::ICON_MUTED)))
                            .child(div().flex_1().h_full().py(px(4.)).px(px(6.)).child(filter))
                            .when(filter_empty, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left(px(25.))
                                        .top(px(4.))
                                        .font_family(theme::EDITOR_FONT)
                                        .text_size(px(theme::EDITOR_FONT_SIZE))
                                        .text_color(ui(theme::PLACEHOLDER_FG))
                                        .child("Filter results"),
                                )
                            }),
                    )
                    .when_some(note, |d, note| {
                        d.child(
                            div().id("results-note").text_size(px(11.)).text_color(ui(theme::TEXT_MUTED)).child(note),
                        )
                    })
                    .child(
                        kit::field()
                            .key_context("ResultsList")
                            .track_focus(&focus)
                            .flex_1()
                            .min_h(px(ROW_H))
                            .py(px(2.))
                            .overflow_hidden()
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    /// One row of the browser.
    fn result_row(&self, i: usize, row: Row, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(set) = &self.result_set else {
            return div().into_any_element();
        };
        let replacing = set.replacement.is_some();
        let base = div()
            .id(("result-row", i))
            .h(px(ROW_H))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(4.))
            .font_family(theme::EDITOR_FONT)
            .text_size(px(theme::EDITOR_FONT_SIZE))
            .text_color(ui(theme::WIDGET_FG))
            .when(selected, |d| d.bg(ui(theme::SIDEBAR_SELECTED_BG)))
            .on_click(cx.listener(move |ws, e: &ClickEvent, window, cx| {
                ws.select_row(i, cx);
                if e.click_count() > 1 {
                    ws.open_row(&OpenRow, window, cx);
                }
            }));
        let check = |on: bool, partial: bool| {
            div()
                .id(("result-check", i))
                .size(px(13.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .cursor_pointer()
                .map(|d| match on {
                    true => d
                        .bg(ui(theme::ACCENT))
                        .border_color(ui(theme::ACCENT))
                        .child(icon(if partial { Icon::Minus } else { Icon::Check }, 11.).text_color(gpui::white())),
                    false => d.border_color(ui(theme::ICON_MUTED)),
                })
                .on_click(cx.listener(move |ws, _: &ClickEvent, _, cx| {
                    ws.select_row(i, cx);
                    ws.toggle_inclusion(row, cx);
                    cx.stop_propagation();
                }))
        };
        let dim = ui(theme::PLACEHOLDER_FG);
        match row {
            Row::File(f) => {
                let file = &set.files[f];
                let (n, s) = (file.entries().len(), file.selected().count());
                let what = match &file.body {
                    Body::Matches(_) => format!("{n} match{}", if n == 1 { "" } else { "es" }),
                    Body::Binary => "binary file".into(),
                    Body::TooLarge => "too large, skipped".into(),
                    Body::Failed(why) => why.clone(),
                };
                // what went wrong in the error colour, what went as asked dimmed
                let status = (file.status.as_ref()).map(|s| {
                    let bad = matches!(s, Status::Stale(_) | Status::Skipped(_) | Status::Failed(_));
                    (s.label(), bad)
                });
                base.child(
                    div()
                        .id(("result-fold", i))
                        .w(px(12.))
                        .flex_shrink_0()
                        .cursor_pointer()
                        .child(
                            icon(
                                match file.collapsed {
                                    true => Icon::ChevronRight,
                                    false => Icon::ChevronDown,
                                },
                                11.,
                            )
                            .text_color(ui(theme::ICON_MUTED)),
                        )
                        .on_click(cx.listener(move |ws, _: &ClickEvent, _, cx| {
                            ws.collapse(f, None, cx);
                            cx.stop_propagation();
                        })),
                )
                .when(replacing && n > 0, |d| d.child(check(s > 0, s > 0 && s < n)))
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(search::tilde(&file.shown)))
                .child(div().text_color(dim).child(what))
                .when_some(status, |d, (s, bad)| {
                    let fg = if bad { ui(theme::ERROR_FG) } else { dim };
                    d.child(div().text_color(fg).child(s))
                })
                .into_any_element()
            }
            Row::Match(f, j) => {
                let file = &set.files[f];
                let e = &file.entries()[j];
                let h = &e.hit;
                let line = file.line(h.start.0).unwrap_or("");
                let end = match h.end.0 == h.start.0 {
                    true => h.end.1,
                    false => line.chars().count(),
                };
                let (before, matched, after) = excerpt(line, h.start.1..end);
                let more = h.end.0 > h.start.0;
                let hl = |t: String, bg: u32| {
                    div()
                        .bg(crate::chrome::theme::c(bg))
                        .text_color(crate::chrome::theme::c(theme::FIND_CURRENT_FG))
                        .child(t)
                };
                base.child(div().w(px(12.)))
                    .when(replacing, |d| d.child(check(file.included && e.included, false)))
                    .child(div().text_color(dim).child(format!("{:>5}:{:<3}", h.start.0 + 1, h.start.1 + 1)))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .child(before)
                            .child(hl(matched, theme::FIND_CURRENT_BG))
                            .child(if more { format!("{after}…") } else { after.clone() }),
                    )
                    .when_some(h.replacement.clone().filter(|_| replacing), |d, rep| {
                        let first = rep.split('\n').next().unwrap_or("").to_string();
                        let more = rep.contains('\n');
                        d.child(div().text_color(dim).child("→")).child(
                            div()
                                .flex()
                                .flex_row()
                                .child(hl(
                                    format!("{first}{}", if more { "⏎…" } else { "" }),
                                    theme::REPLACE_PREVIEW_BG,
                                ))
                                .child(if h.end.0 == h.start.0 { after } else { String::new() }),
                        )
                    })
                    .into_any_element()
            }
            Row::Context(f, l) => {
                let line = set.files[f].line(l).unwrap_or("");
                let (shown, _) = search::cut(line, &[]);
                base.text_color(dim)
                    .child(div().w(px(if replacing { 32. } else { 12. })))
                    .child(format!("{:>5}    {shown}", l + 1))
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file read from disk, shown under its own path, with every result included.
    fn disk_result(
        path: &Path,
        stamp: Option<crate::files::watch::Stamp>,
        body: Body,
    ) -> crate::searching::results::FileResult {
        crate::searching::results::FileResult {
            path: path.to_path_buf(),
            shown: path.to_path_buf(),
            source: Source::Disk {
                stamp,
                hash: 0,
                line_ending: "\n",
                bom: false,
                encoding: Default::default(),
                lossy: false,
                binary: false,
            },
            body,
            lines: Vec::new(),
            included: true,
            collapsed: false,
            status: None,
        }
    }

    /// `disk_result` with one match, "line" on line 1000 of a `write_large` file.
    fn match_at_line_1000(
        path: &Path,
        stamp: Option<crate::files::watch::Stamp>,
    ) -> crate::searching::results::FileResult {
        let entry = crate::searching::results::Entry {
            hit: crate::searching::search::Hit {
                chars: 5000..5004,
                bytes: 5000..5004,
                start: (1000, 0),
                end: (1000, 4),
                replacement: None,
            },
            included: true,
        };
        crate::searching::results::FileResult {
            lines: vec![(1000, "line".into())],
            ..disk_result(path, stamp, Body::Matches(vec![entry]))
        }
    }

    /// Results for `query` with a file row, no matches, for each of `paths`.
    fn file_rows(query: &str, paths: [&Path; 2]) -> ResultSet {
        let mut set = ResultSet::new(query, None, FindOpts::default(), Default::default(), "");
        for path in paths {
            (set.files).push(disk_result(path, stamp(path), Body::Matches(Vec::new())));
        }
        set
    }

    /// A file too large to open on the UI thread.
    fn write_large(path: &Path) {
        std::fs::write(path, "line\n".repeat((LOAD_IN_BACKGROUND / 5 + 1) as usize)).unwrap();
    }

    /// A job for `path` as it is now, with plain line endings and no byte order mark.
    fn disk_job(path: &Path, hash: u64, edits: Vec<(std::ops::Range<usize>, String)>) -> DiskJob {
        DiskJob {
            path: path.to_path_buf(),
            stamp: stamp(path),
            hash,
            line_ending: "\n",
            bom: false,
            encoding: Default::default(),
            edits,
        }
    }

    /// `path` opened in a tab of its own behind `original`, the browser keeping the keys.
    fn open_behind(
        ws: &mut Workspace,
        original: &Entity<Editor>,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        ws.add_tab(EditorState::load(path).unwrap(), window, cx);
        let (group, tab) = ws.views.find(original).unwrap();
        ws.views.select(group, tab);
        window.focus(&ws.results_panel().unwrap().focus.clone());
    }

    fn filter_results(ws: &Workspace, text: &str, cx: &mut Context<Workspace>) {
        let filter = ws.results_panel().unwrap().filter.clone();
        filter.update(cx, |e, cx| e.set_text(text, cx));
    }

    /// `lease` given up, and the clock past the wait for it.
    fn release<T>(lease: T, vcx: &gpui::VisualTestContext) {
        drop(lease);
        vcx.executor().advance_clock(std::time::Duration::from_millis(100));
    }

    #[test]
    fn legacy_replacement_uses_preview_codec_and_refuses_loss_before_history() {
        use crate::editing::encoding::Encoding;
        let dir = crate::workspace::ui_tests::temp("f01-replace-codec");
        let path = dir.join("text.txt");
        std::fs::write(&path, b"caf\xe9\r\n").unwrap();
        let read = search::read_file_with_hint(&path, Some(Encoding::Windows1252)).unwrap();
        let mut job = DiskJob {
            encoding: read.encoding,
            line_ending: read.line_ending,
            bom: read.bom,
            ..disk_job(&path, read.hash, vec![(0..5, "🦀".into())])
        };
        let history = dir.join("history");
        let result = crate::files::file_history::with_root(history.clone(), || write_disk_with_history(&job, true));
        assert!(matches!(result, Status::Failed(message) if message.contains("'🦀' (U+1F980) at line 1, column 1")));
        assert_eq!(std::fs::read(&path).unwrap(), b"caf\xe9\r\n");
        assert!(!history.exists());
        job.edits = vec![(0..5, "été".into())];
        assert_eq!(write_disk(&job), Status::Changed);
        assert_eq!(std::fs::read(&path).unwrap(), b"\xe9t\xe9\r\n");
    }

    #[test]
    fn n25_closed_replacement_preserves_mixed_boundaries_and_insertions_use_preference() {
        let p = std::env::temp_dir().join(format!("orlo-n25-replace-{}", std::process::id()));
        for encoding in [
            crate::editing::encoding::Encoding::Utf8,
            crate::editing::encoding::Encoding::Utf16Be,
            crate::editing::encoding::Encoding::ShiftJis,
        ] {
            let raw = crate::editing::encoding::encode("中\r\n文\n中\r文", encoding, false).unwrap();
            std::fs::write(&p, &raw).unwrap();
            let read = search::read_file_with_hint(&p, Some(encoding)).unwrap();
            let mut job = disk_job(&p, read.hash, vec![(4..7, "文\n中".into())]);
            job.line_ending = read.line_ending;
            job.encoding = read.encoding;
            job.bom = read.bom;
            assert_eq!(write_disk(&job), Status::Changed);
            assert_eq!(
                std::fs::read(&p).unwrap(),
                crate::editing::encoding::encode("中\r\n文\n中\n中\r文", encoding, false).unwrap()
            );
        }
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn replace_in_files_preserves_utf16_and_line_endings() {
        let p = std::env::temp_dir().join(format!("orlo-encoding-replace-{}", std::process::id()));
        for encoding in [
            crate::editing::encoding::Encoding::Utf16Le,
            crate::editing::encoding::Encoding::Utf32Le,
            crate::editing::encoding::Encoding::Utf32Be,
        ] {
            std::fs::write(&p, crate::editing::encoding::encode("one 🦀\r\n", encoding, true).unwrap()).unwrap();
            let read = search::read_file(&p).unwrap();
            let job = DiskJob {
                line_ending: read.line_ending,
                bom: read.bom,
                encoding: read.encoding,
                ..disk_job(&p, read.hash, vec![(0..3, "two".into())])
            };
            assert_eq!(write_disk(&job), Status::Changed);
            assert_eq!(
                std::fs::read(&p).unwrap(),
                crate::editing::encoding::encode("two 🦀\r\n", encoding, true).unwrap()
            );
            std::fs::remove_file(&p).unwrap();
        }
    }

    #[test]
    fn a_disk_write_checks_the_file_is_still_what_was_previewed() {
        let dir = std::env::temp_dir().join("orlo-replace-disk");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "\u{feff}one foo\r\ntwo foo\r\n").unwrap();
        let read = search::read_file(&p).unwrap();
        assert_eq!(read.text, "one foo\ntwo foo\n");
        let job = |hash| DiskJob {
            line_ending: read.line_ending,
            bom: read.bom,
            encoding: read.encoding,
            ..disk_job(&p, hash, vec![(4..7, "bar".into()), (12..15, "baz".into())])
        };
        assert_eq!(write_disk(&job(read.hash ^ 1)), Status::Stale("changed on disk since the preview".into()));
        assert_eq!(write_disk(&job(read.hash)), Status::Changed);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "\u{feff}one bar\r\ntwo baz\r\n");
        // the same job again: the file is no longer the previewed bytes
        let again = DiskJob { stamp: stamp(&p), ..job(read.hash) };
        assert!(matches!(write_disk(&again), Status::Stale(_)));
        std::fs::remove_file(&p).unwrap();
        assert_eq!(write_disk(&job(read.hash)), Status::Stale("it no longer exists".into()));
    }

    #[test]
    fn a_read_only_file_fails_and_keeps_its_text() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("orlo-replace-ro");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("ro.txt");
        std::fs::write(&p, "foo\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        let read = search::read_file(&p).unwrap();
        let job = disk_job(&p, read.hash, vec![(0..3, "bar".into())]);
        assert_eq!(write_disk(&job), Status::Failed("the file is read-only".into()));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "foo\n");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[test]
    fn disk_apply_refuses_a_nonregular_file() {
        use std::io::Write;
        use std::os::unix::ffi::OsStrExt;
        let dir = crate::workspace::ui_tests::temp("orlo-apply-fifo");
        let path = dir.join("pipe");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // a read that blocks gets the writer's bytes and ends, so this test cannot hang
        let mut writer = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            writer.write_all(b"foo").unwrap();
        });
        let job = disk_job(&path, search::hash(b"foo"), Vec::new());
        let status = write_disk(&job);
        writer.join().unwrap();
        assert_eq!(status, Status::Failed("Not a regular file".into()));
    }

    #[test]
    fn disk_apply_keeps_edits_made_while_replacements_are_built() {
        let dir = crate::workspace::ui_tests::temp("orlo-apply-external-write");
        let path = dir.join("input.txt");
        for replace in [false, true] {
            std::fs::write(&path, "foo").unwrap();
            let job = disk_job(&path, search::hash(b"foo"), vec![(0..3, "bar".into())]);
            BEFORE_PUBLISH.set(Some(if replace {
                |path| {
                    let other = path.with_extension("replacement");
                    std::fs::write(&other, "external").unwrap();
                    std::fs::rename(other, path).unwrap();
                }
            } else {
                |path| std::fs::write(path, "external").unwrap()
            }));
            assert!(matches!(write_disk(&job), Status::Stale(_)));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        }
    }

    #[test]
    fn excerpts_mark_the_match_and_cut_long_lines() {
        assert_eq!(excerpt("a foo b", 2..5), ("a ".into(), "foo".into(), " b".into()));
        let long = format!("{}foo{}", "x".repeat(3000), "y".repeat(3000));
        let (b, m, a) = excerpt(&long, 3000..3003);
        assert_eq!(m, "foo");
        assert!(b.starts_with('…') && a.ends_with('…'));
    }
    #[gpui::test]
    fn context_row_waits_for_its_background_file(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::{settle_until, temp, window};
        let d = temp("orlo-context-row").canonicalize().unwrap();
        let path = d.join("large.txt");
        write_large(&path);
        let (ws, vcx) = window(cx);
        let original = ws.update_in(vcx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("one\ntwo\nthree")));
            let original = ws.active_editor().unwrap();
            original.update(cx, |e, cx| e.sel(cx).set_cursor(1));
            let mut set = ResultSet::new("line", None, FindOpts::default(), Default::default(), "");
            set.files.push(disk_result(&path, stamp(&path), Body::Matches(Vec::new())));
            ws.result_set = Some(set);
            ws.show_results_panel(window, cx);
            ws.open_line(0, 1000, window, cx);
            assert_eq!(original.read(cx).state.cursor(), 1, "the unrelated open tab stays put");
            original
        });
        settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
        ws.read_with(vcx, |ws, cx| {
            let file = ws.views.tabs().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&path)).unwrap();
            assert_eq!(file.read(cx).doc(cx).point(file.read(cx).state.cursor()).0, 1000);
            assert_eq!(original.read(cx).state.cursor(), 1);
        });
        ws.update_in(vcx, |ws, window, cx| {
            assert_eq!(
                ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref(),
                Some(&path),
                "the requested source is visible after its background read"
            );
            assert!(ws.results_panel().unwrap().focus.is_focused(window), "the results browser keeps the keys");
        });
    }

    #[gpui::test]
    fn async_match_opens_with_the_whole_match_selected(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::{settle_until, temp, window};
        for change in ["fresh", "before", "during"] {
            let stale = change == "before";
            let d = temp(&format!("orlo-async-match-{change}")).canonicalize().unwrap();
            let path = d.join("large.txt");
            write_large(&path);
            let known = stamp(&path);
            if stale {
                let changed = "changed\n".to_string() + &std::fs::read_to_string(&path).unwrap();
                std::fs::write(&path, changed).unwrap();
            }
            let (ws, vcx) = window(cx);
            let lease = ws.update_in(vcx, |ws, window, cx| {
                let lease = (change == "during").then(|| pipelines::lease(&path, cx));
                let mut set = ResultSet::new("line", None, FindOpts::default(), Default::default(), "");
                set.files.push(match_at_line_1000(&path, known));
                ws.result_set = Some(set);
                ws.show_results_panel(window, cx);
                ws.open_hit(0, Some(0), window, cx);
                lease
            });
            if change == "during" {
                let changed = "changed\n".to_string() + &std::fs::read_to_string(&path).unwrap();
                std::fs::write(&path, changed).unwrap();
                release(lease, vcx);
            }
            settle_until(vcx, |cx| {
                ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref() == Some(&path))
            });
            ws.update_in(vcx, |ws, window, cx| {
                let editor = ws.active_editor().unwrap();
                let state = editor.read(cx).read_state(cx);
                assert_eq!(state.doc.path.as_ref(), Some(&path));
                if change != "fresh" {
                    let at = state.offset(1000, 0);
                    assert_eq!(state.view.selections[0].range(), at..at);
                    if stale {
                        assert!(ws.results_panel().unwrap().note.as_ref().unwrap().contains("changed on disk"));
                    }
                } else {
                    assert_eq!(state.view.selections[0].range(), 5000..5004);
                    assert_eq!(state.doc.rope.slice(state.view.selections[0].range()).to_string(), "line");
                }
                assert!(ws.results_panel().unwrap().focus.is_focused(window));
            });
        }
    }

    #[gpui::test]
    fn async_browser_opens_respect_later_navigation(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::{settle_until, temp, window};
        for mode in
            ["active", "row", "target", "filter", "search", "same", "lease", "existing", "blank-row", "blank-filter"]
        {
            let dir = temp(&format!("orlo-browser-later-{mode}")).canonicalize().unwrap();
            let large = dir.join("large.txt");
            let other = dir.join("other.txt");
            let body = "line\n".repeat((LOAD_IN_BACKGROUND / 5 + 1) as usize);
            std::fs::write(&large, &body).unwrap();
            std::fs::write(&other, if mode == "lease" { &body } else { "other\n" }).unwrap();
            let (ws, vcx) = window(cx);
            let (expected, lease) = ws.update_in(vcx, |ws, window, cx| {
                if !mode.starts_with("blank-") {
                    ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("keep this document")));
                }
                ws.result_set = Some(file_rows("line", [&large, &other]));
                ws.show_results_panel(window, cx);
                assert_eq!(ws.results_rows(cx), [Row::File(0), Row::File(1)]);
                ws.open_line(0, 1000, window, cx);
                let mut lease = None;
                match mode {
                    "active" => ws.open_path_here(other.clone(), window, cx),
                    "row" | "blank-row" => ws.select_row(1, cx),
                    "target" => ws.open_hit(1, None, window, cx),
                    "filter" | "blank-filter" => filter_results(ws, "other", cx),
                    "search" => {
                        ws.run_search(
                            "absent",
                            other.to_str().unwrap(),
                            Some("replacement"),
                            FindOpts::default(),
                            Default::default(),
                            window,
                            cx,
                        )
                        .unwrap();
                    }
                    "same" => ws.open_line(0, 1001, window, cx),
                    "existing" => {
                        let original = ws.active_editor().unwrap();
                        open_behind(ws, &original, &large, window, cx);
                    }
                    "lease" => {
                        lease = Some(pipelines::lease(&other, cx));
                        // a second open takes over from the first, though the selected row
                        // never changed
                        ws.open_line(1, 1002, window, cx);
                    }
                    _ => unreachable!(),
                }
                (ws.active_editor().unwrap(), lease)
            });
            settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
            ws.update_in(vcx, |ws, window, cx| {
                let active = ws.active_editor().unwrap();
                if matches!(mode, "same" | "existing") {
                    let state = active.read(cx).read_state(cx);
                    assert_eq!(state.doc.path.as_ref(), Some(&large));
                    assert_eq!(state.point(state.cursor()).0, if mode == "same" { 1001 } else { 1000 });
                } else {
                    assert_eq!(active, expected, "late read must not take over: {mode}");
                }
                if mode != "active" {
                    assert!(ws.results_panel().unwrap().focus.is_focused(window));
                }
            });
            if mode == "lease" {
                release(lease, vcx);
                settle_until(vcx, |cx| {
                    ws.read_with(cx, |ws, cx| {
                        ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref() == Some(&other)
                    })
                });
                ws.update_in(vcx, |ws, window, cx| {
                    let active = ws.active_editor().unwrap();
                    let state = active.read(cx).read_state(cx);
                    assert_eq!(state.point(state.cursor()).0, 1002);
                    assert!(ws.results_panel().unwrap().focus.is_focused(window));
                });
            }
        }
    }

    #[gpui::test]
    fn leased_small_browser_opens_respect_navigation_and_focus(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::{settle_until, temp, window};
        for mode in ["current", "row", "filter", "active", "existing", "same"] {
            let dir = temp(&format!("orlo-small-browser-{mode}")).canonicalize().unwrap();
            let path = dir.join("small.txt");
            let other = dir.join("other.txt");
            std::fs::write(&path, "first\nsecond\nthird\n").unwrap();
            std::fs::write(&other, "other\n").unwrap();
            let (ws, vcx) = window(cx);
            let (original, lease) = ws.update_in(vcx, |ws, window, cx| {
                ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("keep this document")));
                let original = ws.active_editor().unwrap();
                let lease = pipelines::lease(&path, cx);
                ws.result_set = Some(file_rows("second", [&path, &other]));
                ws.show_results_panel(window, cx);
                assert_eq!(ws.results_rows(cx), [Row::File(0), Row::File(1)]);
                ws.open_line(0, 1, window, cx);
                match mode {
                    "row" => ws.select_row(1, cx),
                    "filter" => filter_results(ws, "other", cx),
                    "active" => ws.new_file(&NewFile, window, cx),
                    "same" => ws.open_line(0, 2, window, cx),
                    "existing" => open_behind(ws, &original, &path, window, cx),
                    _ => {}
                }
                (original, lease)
            });
            let expected = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
            release(lease, vcx);
            settle_until(vcx, |cx| {
                ws.read_with(cx, |ws, cx| ws.views.tabs().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&path)))
            });
            ws.update_in(vcx, |ws, window, cx| {
                let active = ws.active_editor().unwrap();
                if matches!(mode, "current" | "existing" | "same") {
                    assert_eq!(active.read(cx).doc(cx).path.as_ref(), Some(&path));
                    assert_eq!(
                        active.read(cx).doc(cx).point(active.read(cx).state.cursor()).0,
                        if mode == "same" { 2 } else { 1 }
                    );
                } else {
                    assert_eq!(active, expected, "late leased open must stay behind: {mode}");
                }
                assert_eq!(original.read(cx).doc(cx).text(), "keep this document");
                if mode != "active" {
                    assert!(ws.results_panel().unwrap().focus.is_focused(window), "{mode}");
                }
            });
        }
    }

    #[gpui::test]
    fn browser_jumps_wait_for_session_restoration(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::{settle_until, temp, window};
        for mode in ["context", "match", "row", "filter", "active", "blank-row", "existing", "multi-view"] {
            let dir = temp(&format!("orlo-restoring-browser-{mode}")).canonicalize().unwrap();
            let path = dir.join("large.txt");
            let other = dir.join("other.txt");
            write_large(&path);
            std::fs::write(&other, "other\n").unwrap();
            let (ws, vcx) = window(cx);
            let expected = ws.update_in(vcx, |ws, window, cx| {
                if mode == "multi-view" {
                    ws.set_layout(&SetLayout { preset: Preset::Columns(2) }, window, cx);
                    ws.focus_group(&FocusGroup { group: 1 }, window, cx);
                    ws.new_file(&NewFile, window, cx);
                }
                if mode != "blank-row" {
                    ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("keep this document")));
                }
                let view = ViewState { path: path.clone(), ..Default::default() };
                let mut restoring = vec![(0, 0, true, view.clone())];
                if mode == "multi-view" {
                    restoring.push((1, 0, true, view));
                }
                ws.restore_later(restoring, window, cx);
                let mut set = ResultSet::new("line", None, FindOpts::default(), Default::default(), "");
                set.files.push(match_at_line_1000(&path, stamp(&path)));
                (set.files).push(disk_result(&other, stamp(&other), Body::Matches(Vec::new())));
                ws.result_set = Some(set);
                ws.show_results_panel(window, cx);
                let rows = ws.results_rows(cx);
                if mode == "match" {
                    ws.open_hit(0, Some(0), window, cx);
                } else {
                    ws.open_line(0, 1000, window, cx);
                }
                match mode {
                    "row" | "blank-row" => ws.select_row(rows.len() - 1, cx),
                    "filter" => filter_results(ws, "other", cx),
                    "active" => ws.new_file(&NewFile, window, cx),
                    "existing" => {
                        let original = ws.active_editor().unwrap();
                        open_behind(ws, &original, &path, window, cx);
                    }
                    _ => {}
                }
                ws.active_editor().unwrap()
            });
            settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
            ws.update_in(vcx, |ws, window, cx| {
                let file = if mode == "multi-view" {
                    &ws.views.groups[1].tabs[0]
                } else {
                    ws.views.tabs().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&path)).unwrap()
                };
                let state = file.read(cx).read_state(cx);
                if mode == "match" {
                    assert_eq!(state.view.selections[0].range(), 5000..5004);
                } else {
                    assert_eq!(state.point(state.cursor()).0, 1000, "{mode}");
                }
                if matches!(mode, "context" | "match" | "existing" | "multi-view") {
                    assert_eq!(ws.active_editor().as_ref(), Some(file));
                    if mode == "multi-view" {
                        assert_eq!(ws.views.active, 1);
                        assert_eq!(ws.views.groups.len(), 2);
                        assert_eq!(ws.views.groups[0].tabs[0].read(cx).state.cursor(), 0);
                    }
                } else {
                    assert_eq!(ws.active_editor(), Some(expected), "{mode}");
                }
                assert!(!ws.pending_jumps.contains_key(&path));
                if mode != "active" {
                    assert!(ws.results_panel().unwrap().focus.is_focused(window), "{mode}");
                }
            });
        }
    }
}
