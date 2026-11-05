//! The Find / Replace panel's commands, and Find in Files: its panel, the background run
//! streaming into Find Results, and opening a result.

use crate::searching::pattern::{Deadline, Matcher, Pattern, Replacer, SearchError};
use crate::searching::results::{
    matching_lines, Body, Entry, FileResult, MatchingLines, ResultSet, SearchFlags, Source,
};
use crate::workspace::*;
use gpui::WeakEntity;
use orlo_editor::search::model::SyntaxMask;
use orlo_syntax::highlight::{Class, SearchClasses, SearchSource};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// What the background search sends: the header once the files are known, each file with a
/// result (its structured result, and its Find Results block with outlines and match count),
/// and the end (cancelled or not).
enum SearchMsg {
    Header(String),
    File(Box<FileResult>, Option<Box<MatchingLines>>, (String, Vec<Range<usize>>, usize)),
    Done(bool),
}

/// An open buffer's text as Find in Files reads it with Use Buffer on.
struct BufferText {
    path: PathBuf,
    rope: ropey::Rope,
    id: u64,
    version: u64,
}

/// A Find in Files run: cancel flag and progress counters shared with the background search,
/// the Find Results view it streams into, and the 100 ms drain.
pub(crate) struct Search {
    pub(crate) page: u64,
    request: u64,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    /// Files the walk has found so far, for the header of a run cancelled before it ends.
    listed: Arc<AtomicUsize>,
    /// The Find Results view it streams into; Replace in Files previews in the results
    /// browser only.
    pub(crate) results: Option<WeakEntity<Editor>>,
    rx: mpsc::Receiver<SearchMsg>,
    /// The header is in Find Results.
    header: bool,
    query: String,
    opts: FindOpts,
    matches: usize,
    files: usize,
    reads: std::collections::HashMap<PathBuf, Arc<pipelines::Read>>,
    _tasks: (Task<()>, Task<()>),
}

/// A find in a big buffer under way on another thread: (view, version, query, options, scope),
/// and its cancel flag.
pub(crate) type Searching = ((EntityId, u64, String, FindOpts, Option<Vec<Range<usize>>>), Arc<AtomicBool>);

/// Which search history a field keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    Find,
    Replace,
    Where,
}

/// A selection up to this many chars seeds the find field.
pub(crate) const SEED_MAX: usize = 16 << 10;

/// Buffers longer than this (chars) are searched on another thread as the query is typed.
pub(crate) const SEARCH_IN_BACKGROUND: usize = 4 << 20;

#[cfg(test)]
thread_local! {
    /// A test's `SEARCH_IN_BACKGROUND`, so a small buffer takes the big buffers' path.
    pub(crate) static SEARCH_BG: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn search_in_background() -> usize {
    #[cfg(test)]
    if let Some(n) = SEARCH_BG.get() {
        return n;
    }
    SEARCH_IN_BACKGROUND
}

/// The background loop only stops at the flag: a window closing (or a finished or replaced run)
/// must not leave a walk of `~/` going.
impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Workspace {
    forward_actions! {
    pub(crate) find(Find, window, cx) => open_find_panel(false, window, cx);
    pub(crate) replace(Replace, window, cx) => open_find_panel(true, window, cx);
    pub(crate) previous_find_query(PreviousFindQuery, window, cx) => step_find_history(true, window, cx);
    pub(crate) next_find_query(NextFindQuery, window, cx) => step_find_history(false, window, cx);
    pub(crate) toggle_find_regex(ToggleFindRegex, _, cx) => flip_opts(|o| o.regex ^= true, cx);
    pub(crate) toggle_find_case(ToggleFindCase, _, cx) => flip_opts(|o| o.case_sensitive ^= true, cx);
    pub(crate) toggle_find_word(ToggleFindWord, _, cx) => flip_opts(|o| o.whole_word ^= true, cx);
    pub(crate) toggle_show_context(ToggleShowContext, _, cx) => flip_flags(|f| f.context ^= true, "Show Context", |f| f.context, cx);
    pub(crate) toggle_use_buffer(ToggleUseBuffer, _, cx) => flip_flags(|f| f.use_buffer ^= true, "Use Buffer", |f| f.use_buffer, cx);
    pub(crate) toggle_use_gitignore(ToggleUseGitignore, _, cx) => flip_flags(|f| f.gitignore ^= true, "Use gitignore", |f| f.gitignore, cx,);
    pub(crate) find_next(FindNext, _, cx) => find_step(true, cx);
    pub(crate) find_prev(FindPrev, _, cx) => find_step(false, cx);
    /// Find: the panel closes, Find Results takes focus and fills in as the background search
    /// finds files.
    pub(crate) run_find_in_files(RunFindInFiles, window, cx) => start_find_in_files(false, window, cx);
    pub(crate) cancel_find_in_files(CancelFindInFiles, _, cx) => stop_search(cx);
    /// A double click in Find Results: the file in the results' group, right after the results
    /// tab (or its view already there), caret at the match (a header or context line: at the
    /// start of the line).
    pub(crate) next_result(NextResult, window, cx) => step_result(true, window, cx);
    pub(crate) prev_result(PrevResult, window, cx) => step_result(false, window, cx);
    /// Tab in the Find in Files panel: Find, Where, Replace and round; Shift-Tab back.
    pub(crate) focus_next_fif_field(find_in_files::FocusNext, window, cx) => step_fif_field(1, window, cx);
    pub(crate) focus_prev_fif_field(find_in_files::FocusPrev, window, cx) => step_fif_field(2, window, cx);
    }

    fn open_find_panel(&mut self, show_replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_panel(cx);
        // a preview gives its place back first, so the find starts where the user was
        self.cancel_quick_panel(cx);
        self.find_origin = self.caret_for_find(cx);
        let initial = self.selection_for_find(cx).unwrap_or_else(|| self.last_query.clone());
        let scope = self.selection_scope(cx);
        if let Some(p) = self.find_panel_mut() {
            p.show_replace = show_replace;
            if !p.in_selection {
                p.scope = scope;
            }
            let query = p.query.clone();
            self.find_quiet = true;
            query.update(cx, |e, cx| e.set_text(&initial, cx));
            query.read(cx).focus(window);
        } else {
            let mut panel = FindPanel::new(show_replace, &initial, &self.last_replace, self.last_find_opts, window, cx);
            panel.scope = scope;
            panel.preserve_case = self.last_preserve_case;
            self.show_panel(Panel::Find(panel), cx);
        }
        self.refresh_matches(cx);
        cx.notify();
    }

    /// The active view and where its last selection ends: Sublime's incremental find starts
    /// there.
    fn caret_for_find(&self, cx: &App) -> Option<(EntityId, usize)> {
        let e = self.active_editor()?;
        let at = e.read(cx).state.selections.last().map_or(0, |s| s.end());
        Some((e.entity_id(), at))
    }

    /// The find field is seeded with the selection, lines and all, unless it is longer
    /// than `SEED_MAX` chars.
    fn selection_for_find(&self, cx: &App) -> Option<String> {
        let editor = self.active_editor()?;
        let e = editor.read(cx);
        let sel = *e.state.selections.last()?;
        if sel.is_empty() || sel.range().len() > SEED_MAX {
            return None;
        }
        Some(e.doc(cx).rope.slice(sel.range()).to_string())
    }

    pub(crate) fn current_query(&self, cx: &App) -> String {
        self.find_panel().map_or_else(|| self.last_query.clone(), |p| p.query.read(cx).text(cx))
    }

    fn replacement(&self, cx: &App) -> String {
        self.find_panel().map_or_else(|| self.last_replace.clone(), |p| p.replace.read(cx).text(cx))
    }

    pub(crate) fn find_opts(&self) -> FindOpts {
        self.find_panel().map_or(self.last_find_opts, |p| p.opts)
    }

    fn preserve_case(&self) -> bool {
        self.find_panel().map_or(self.last_preserve_case, |p| p.preserve_case)
    }

    /// The query and replacement checked together, or why they cannot replace: said in the
    /// panel, and nothing is changed.
    fn replacer(&mut self, query: &str, rep: &str, cx: &mut Context<Self>) -> Option<Replacer> {
        let (opts, preserve) = (self.find_opts(), self.preserve_case());
        match Replacer::new(query, rep, &opts, preserve) {
            Ok(r) => Some(r),
            Err(e) => {
                match self.find_panel_mut() {
                    Some(p) => p.error = Some(e),
                    None => self.status_message(format!("Cannot replace: {e}"), cx),
                }
                cx.notify();
                None
            }
        }
    }

    fn selection_scope(&self, cx: &App) -> Option<(EntityId, Vec<Range<usize>>)> {
        let e = self.active_editor()?;
        let ranges = e.read(cx).state.selections.iter().filter(|s| !s.is_empty()).map(|s| s.range()).collect();
        Some((e.entity_id(), ranges))
    }

    fn step_find_history(&mut self, older: bool, window: &Window, cx: &mut Context<Self>) {
        let Some((field, which)) = self.history_field(window, cx) else {
            return;
        };
        // in a field of several lines Up and Down move between them; history is past the
        // first or last line
        if !field.read(cx).caret_at_edge(older, cx) {
            let motion = match older {
                true => crate::editing::text::Motion::Up,
                false => crate::editing::text::Motion::Down,
            };
            return field.update(cx, |e, cx| e.move_caret(motion, window, cx));
        }
        let current = field.read(cx).text(cx);
        if let Some(text) = self.history_mut(which).step(&current, older) {
            field.update(cx, |e, cx| {
                e.set_text(&text, cx);
                e.sel(cx).select_all();
            });
        }
    }

    /// The focused Find, Replace or Where field of the Find or Find in Files panel, and which
    /// history is its.
    pub(crate) fn history_field(&self, window: &Window, cx: &App) -> Option<(Entity<Editor>, History)> {
        let fields: Vec<(&Entity<Editor>, History)> = match &self.panel {
            Some(Panel::Find(p)) => vec![(&p.query, History::Find), (&p.replace, History::Replace)],
            Some(Panel::FindInFiles(p)) => {
                vec![(&p.query, History::Find), (&p.where_, History::Where), (&p.replace, History::Replace)]
            }
            _ => return None,
        };
        (fields.into_iter()).find(|(f, _)| f.read(cx).is_focused(window)).map(|(f, h)| (f.clone(), h))
    }

    pub(crate) fn history_mut(&mut self, which: History) -> &mut search::QueryHistory {
        match which {
            History::Find => &mut self.query_history,
            History::Replace => &mut self.replace_history,
            History::Where => &mut self.where_history,
        }
    }

    pub(crate) fn find_scope(&self) -> Option<&[Range<usize>]> {
        let p = self.find_panel().filter(|p| p.in_selection)?;
        let active = self.active_editor().map(|e| e.entity_id());
        Some(p.scope.as_ref().filter(|(view, _)| Some(*view) == active).map_or(&[], |(_, ranges)| ranges.as_slice()))
    }

    pub(crate) fn all_matches(&mut self, query: &str, opts: &FindOpts, cx: &mut Context<Self>) -> Vec<Range<usize>> {
        if self.syntax_filtered() {
            return match self.syntax_matches(query, opts, cx) {
                Ok(all) => all,
                Err(error) => {
                    if let Some(p) = self.find_panel_mut() {
                        p.error = Some(error);
                    }
                    Vec::new()
                }
            };
        }
        self.active_editor().map_or_else(Vec::new, |e| match self.find_scope() {
            Some(scope) => e.read(cx).read_state(cx).find_scoped(query, opts, scope),
            None => e.read(cx).read_state(cx).find_all(query, opts),
        })
    }

    pub(crate) fn toggle_preserve_case(&mut self, _: &TogglePreserveCase, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.find_panel_mut() {
            p.preserve_case ^= true;
        } else if let Some(p) = self.fif_panel_mut() {
            p.flags.preserve_case ^= true;
        } else if let Some(p) = self.playground_panel_mut() {
            p.preserve_case ^= true;
            self.playground_changed(cx);
        }
        cx.notify();
    }

    /// A Find in Files toggle from the menu or palette: the panel's, else the next run's.
    fn flip_flags(
        &mut self,
        flip: impl Fn(&mut SearchFlags),
        name: &str,
        on: impl Fn(&SearchFlags) -> bool,
        cx: &mut Context<Self>,
    ) {
        let flags = match self.fif_panel_mut() {
            Some(p) => {
                flip(&mut p.flags);
                p.flags
            }
            None => {
                flip(&mut self.fif_flags);
                self.fif_flags
            }
        };
        let state = if on(&flags) { "on" } else { "off" };
        self.status_message(format!("Find in Files: {name} {state}"), cx);
        cx.notify();
    }

    /// A toggle of the Find or Find in Files panel, from its key.
    fn flip_opts(&mut self, flip: impl Fn(&mut FindOpts), cx: &mut Context<Self>) {
        if let Some(p) = self.find_panel_mut() {
            flip(&mut p.opts);
            self.refresh_matches(cx);
            self.find_as_you_type(cx);
        } else if let Some(p) = self.fif_panel_mut() {
            flip(&mut p.opts);
            self.validate_fif(cx);
        } else if let Some(p) = self.playground_panel_mut() {
            flip(&mut p.opts);
            self.playground_changed(cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_find_selection(&mut self, _: &ToggleFindSelection, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.find_panel_mut() {
            p.toggle_in_selection();
        }
        self.refresh_matches(cx);
        self.find_as_you_type(cx);
        cx.notify();
    }

    /// Keep the Find panel's replacement and toggles before it goes.
    pub(crate) fn remember_find(&mut self, cx: &App) {
        if let Some(p) = self.find_panel() {
            let kept = (p.replace.read(cx).text(cx), p.opts, p.preserve_case);
            (self.last_replace, self.last_find_opts, self.last_preserve_case) = kept;
        }
    }

    /// The active view's matches, if they are ready: a buffer over `SEARCH_IN_BACKGROUND`
    /// chars is searched on another thread (100 MB took 136 ms), and the find panel's
    /// outlines catch up when it lands; the selection too, if the user typed that query
    /// (`find_pending`) and nothing changed since. A search for anything else is cancelled.
    fn matches_now(&mut self, query: &str, opts: &FindOpts, cx: &mut Context<Self>) -> Option<Vec<Range<usize>>> {
        let editor = self.active_editor()?;
        let classes = if self.syntax_filtered() {
            match self.syntax_classes(&editor, cx) {
                Some(classes) => Some(classes),
                None => return Some(Vec::new()),
            }
        } else {
            None
        };
        let (mask, exclude) = self.find_panel().map_or((SyntaxMask::Any, false), |p| (p.syntax, p.exclude_syntax));
        let eligible = |all: Vec<Range<usize>>| match &classes {
            None => Some(all),
            Some(classes) => mask.filter(exclude, &all, |start| {
                classes.at(start).map(|class| (class == Class::Comment, class == Class::String))
            }),
        };
        let s = editor.read(cx).read_state(cx);
        let scope = self.find_scope().map(<[_]>::to_vec);
        // lookaround and backreferences backtrack: never on the UI thread as the query is typed
        let backtracks = opts.regex && Pattern::new(query, opts).is_ok_and(|p| p.is_fancy());
        if s.len() <= search_in_background() && !backtracks {
            return eligible(match &scope {
                Some(scope) => s.find_scoped(query, opts, scope),
                None => s.find_all(query, opts),
            });
        }
        let cached = match &scope {
            Some(scope) => s.found_scoped(query, opts, Some(scope)),
            None => s.found(query, opts),
        };
        if let Some(all) = cached {
            return eligible(all);
        }
        let key = (editor.entity_id(), s.doc.version, query.to_string(), *opts, scope.clone());
        if self.searching.as_ref().is_some_and(|(k, _)| *k == key) {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some((_, old)) = self.searching.replace((key.clone(), cancel.clone())) {
            old.store(true, Ordering::Relaxed);
        }
        let (rope, q, o) = (s.doc.rope.clone(), query.to_string(), *opts);
        let work = cx.background_spawn(async move {
            let d = Deadline::new(crate::searching::pattern::BACKGROUND_SECONDS);
            let stop = || cancel.load(Ordering::Relaxed) || d.passed();
            crate::editing::text::find_in_rope_until(&rope, &q, &o, scope.as_deref(), &stop).map_err(|e| d.explain(e))
        });
        cx.spawn(async move |ws, cx| {
            let found = work.await;
            if found == Err(SearchError::Cancelled) {
                return;
            }
            ws.update(cx, |ws, cx| {
                if ws.searching.as_ref().is_some_and(|(k, _)| *k == key) {
                    ws.searching = None;
                }
                // for the query in the field now, in the view it was for
                let current = ws.find_panel().is_some()
                    && ws.current_query(cx) == key.2
                    && ws.find_opts() == key.3
                    && ws.find_scope() == key.4.as_deref()
                    && ws.active_editor().is_some_and(|e| e.entity_id() == key.0);
                // for text edited since: the search after the edits, debounced, is under way
                if !current || editor.read(cx).doc(cx).version != key.1 {
                    return;
                }
                editor.update(cx, |e, cx| match found {
                    Ok(all) => e.read_state(cx).remember_scoped_found(&key.2, &key.3, key.4.as_deref(), all),
                    Err(err) => e.read_state(cx).remember_search_error(&key.2, &key.3, key.4.as_deref(), err),
                });
                ws.refresh_matches(cx);
                if ws.find_pending.as_deref() == Some(key.2.as_str()) {
                    ws.find_as_you_type(cx);
                }
            })
            .ok();
        })
        .detach();
        None
    }

    /// `document`'s text changed with the find panel open: the outlines follow at once when the
    /// kept matches can be brought up to the edit, else once edits pause, so a big buffer is
    /// not searched again every keystroke; the old outlines stay on their text meanwhile. A
    /// typed query's selection waiting for a search is dropped: it would land on the new
    /// typing. Another buffer's edit (Find Results streaming in, a reload) changes nothing.
    pub(crate) fn refresh_matches_after_edit(&mut self, document: u64, changes: &[Change], cx: &mut Context<Self>) {
        let Some(active) = self.active_editor() else {
            return;
        };
        if active.read(cx).doc(cx).id() != document {
            return;
        }
        let scope =
            (self.find_panel_mut().and_then(|p| p.scope.as_mut())).filter(|(view, _)| *view == active.entity_id());
        for r in scope.into_iter().flat_map(|(_, ranges)| ranges) {
            for c in changes {
                let end = if r.end >= c.start + c.removed {
                    r.end - c.removed + c.inserted
                } else if r.end > c.start {
                    c.start + c.inserted
                } else {
                    r.end
                };
                r.start = crate::editing::text::map_offset(r.start, std::slice::from_ref(c));
                r.end = end.max(r.start);
            }
        }
        self.find_pending = None;
        // a search of the text as it was is wasted work now
        if let Some((_, cancel)) = self.searching.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if self.syntax_filtered() {
            if let Some(p) = self.find_panel_mut() {
                p.syntax_search = None;
                p.error = Some("Syntax search is preparing; matches are unavailable".into());
            }
            active.update(cx, |e, cx| {
                e.find_ranges.clear();
                cx.notify();
            });
            self.find_debounce = Some(cx.spawn(async move |ws, cx| {
                cx.background_executor().timer(Duration::from_millis(300)).await;
                ws.update(cx, |ws, cx| {
                    ws.find_debounce = None;
                    ws.refresh_matches(cx);
                    cx.notify();
                })
                .ok();
            }));
            return;
        }
        let (query, opts) = (self.current_query(cx), self.find_opts());
        let s = active.read(cx).read_state(cx);
        if s.len() <= search_in_background() || s.has_found(&query, &opts) {
            self.find_debounce = None;
            return self.refresh_matches(cx);
        }
        active.update(cx, |e, _| crate::editing::text::map_ranges(&mut e.find_ranges, changes));
        self.find_debounce = Some(cx.spawn(async move |ws, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            ws.update(cx, |ws, cx| ws.refresh_matches(cx)).ok();
        }));
    }

    pub fn refresh_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.current_query(cx);
        let active = self.active_editor().map(|e| e.entity_id());
        if self.find_panel().is_some_and(|p| p.scope.as_ref().map(|s| s.0) != active) {
            let scope = self.selection_scope(cx);
            if let Some(p) = self.find_panel_mut() {
                p.scope = scope;
                p.in_selection = false;
            }
        }
        let replacement = self.replacement(cx);
        if let Some(p) = self.find_panel_mut() {
            p.error = match (p.opts.regex, p.show_replace) {
                (false, _) => None,
                (true, false) => Pattern::new(&query, &p.opts).err(),
                (true, true) => Replacer::new(&query, &replacement, &p.opts, false).err(),
            };
        }
        self.last_query = query.clone();
        let outline = self.find_panel().is_some_and(|p| p.highlight) && !query.is_empty();
        let opts = self.find_opts();
        // `None`: a search is under way, and the active view keeps its outlines until it lands
        let mut found = match outline {
            true => self.matches_now(&query, &opts, cx),
            false => Some(Vec::new()),
        };
        if self.syntax_filtered() {
            if !outline {
                if let Some(editor) = self.active_editor() {
                    self.syntax_classes(&editor, cx);
                }
            }
            if found.is_none() {
                if let Some(p) = self.find_panel_mut() {
                    p.error = Some("Syntax search is preparing matches; matches are unavailable".into());
                }
                found = Some(Vec::new());
            }
        }
        // a search that gave up says so, rather than showing no matches
        let failed = (self.active_editor())
            .and_then(|e| e.read(cx).read_state(cx).search_error(&query, &opts, self.find_scope()));
        if let (Some(p), Some(e)) = (self.find_panel_mut(), failed) {
            p.error.get_or_insert(e.message());
        }
        // only the active view shows outlines; other groups' views stay visible, so clear them
        let active = self.active_editor();
        for t in self.views.tabs() {
            let mine = Some(t) == active.as_ref();
            t.update(cx, |e, cx| {
                // Find Results' outlines are its matches, the playground sample's the
                // playground's: not the find panel's
                if e.results || e.playground {
                    return;
                }
                if mine {
                    if let Some(found) = found.take() {
                        e.find_ranges = found;
                        cx.notify();
                    }
                } else if !e.find_ranges.is_empty() {
                    e.find_ranges.clear();
                    cx.notify();
                }
            });
        }
    }

    /// Sublime's incremental find: as the query changes, the first match from where the caret
    /// was when the panel opened is selected (and scrolled to).
    pub fn find_as_you_type(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.find_quiet) {
            return;
        }
        let query = self.current_query(cx);
        let Some(here) = self.caret_for_find(cx).filter(|_| !query.is_empty()) else {
            return;
        };
        // another tab since the panel opened: search from where its caret is
        let origin = match self.find_origin {
            Some((view, at)) if view == here.0 => at,
            _ => {
                self.find_origin = Some(here);
                here.1
            }
        };
        let opts = self.find_opts();
        let Some(all) = self.matches_now(&query, &opts, cx) else {
            // selected when the search lands, if the query is still this one
            self.find_pending = Some(query);
            return;
        };
        self.find_pending = self
            .find_panel()
            .filter(|p| p.syntax != SyntaxMask::Any)
            .and_then(|p| p.syntax_search.as_ref())
            .filter(|search| search.result.is_none())
            .map(|_| query.clone());
        // the first match from where the caret was, round to the top
        let next = all.iter().find(|r| r.start >= origin).or(all.first().filter(|_| opts.wrap));
        if let Some(r) = next.cloned() {
            self.with_active(cx, |e, cx| e.select_range(r, cx));
        }
    }

    fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let query = self.current_query(cx);
        if query.is_empty() {
            return;
        }
        self.last_query = query.clone();
        self.query_history.remember(&query);
        let opts = self.find_opts();
        let all = (self.syntax_filtered() || self.find_panel().is_some_and(|p| p.in_selection))
            .then(|| self.all_matches(&query, &opts, cx));
        self.with_active(cx, |e, cx| {
            let last = *e.state.selections.last().unwrap();
            let found = if let Some(all) = &all {
                if forward {
                    all.iter()
                        .find(|r| r.start >= last.end() && (!last.is_empty() || !r.is_empty() || r.start != last.head))
                        .or(all.first().filter(|_| opts.wrap))
                        .cloned()
                } else {
                    all.iter().rev().find(|r| r.start < last.start()).or(all.last().filter(|_| opts.wrap)).cloned()
                }
            } else if forward {
                let zero = opts.regex && last.is_empty() && e.read_state(cx).is_match(&query, &opts, &last.range());
                let from = last.end() + usize::from(zero);
                e.read_state(cx).find_next(&query, from, &opts)
            } else {
                e.read_state(cx).find_prev(&query, last.start(), &opts)
            };
            if let Some(r) = found {
                e.select_range(r, cx);
            }
        });
    }

    pub(crate) fn find_all(&mut self, _: &FindAll, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.current_query(cx);
        if query.is_empty() {
            return;
        }
        let opts = self.find_opts();
        self.query_history.remember(&query);
        let all = if self.syntax_filtered() {
            match self.search_matches(cx) {
                Ok(all) => all,
                Err(error) => {
                    if let Some(p) = self.find_panel_mut() {
                        p.error = Some(error);
                    }
                    return;
                }
            }
        } else {
            self.all_matches(&query, &opts, cx)
        };
        self.with_active(cx, |e, cx| {
            if !all.is_empty() {
                e.sel(cx).select_ranges(&all);
                e.scroll_to_cursor(cx);
                cx.notify();
            }
        });
        self.close_panel(&ClosePanel, window, cx);
    }

    pub(crate) fn replace_all(&mut self, _: &ReplaceAll, _: &mut Window, cx: &mut Context<Self>) {
        let (query, rep) = (self.current_query(cx), self.replacement(cx));
        if query.is_empty() {
            return;
        }
        let opts = self.find_opts();
        self.query_history.remember(&query);
        self.replace_history.remember(&rep);
        let Some(r) = self.replacer(&query, &rep, cx) else {
            return;
        };
        let all = self.all_matches(&query, &opts, cx);
        if self.syntax_filtered() && (all.is_empty() || !self.syntax_replace_admitted(cx)) {
            return;
        }
        self.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.replace_with(&r, &all);
            })
        });
        self.refresh_matches(cx);
    }

    /// Sublime's Replace Next: the last selection, if it is a match, is replaced (and only
    /// it); then the next match becomes the only selection.
    pub(crate) fn replace_one(&mut self, _: &find_panel::ReplaceOne, _: &mut Window, cx: &mut Context<Self>) {
        let (query, rep) = (self.current_query(cx), self.replacement(cx));
        if query.is_empty() {
            return;
        }
        let opts = self.find_opts();
        let Some(r) = self.replacer(&query, &rep, cx) else {
            return;
        };
        self.query_history.remember(&query);
        self.replace_history.remember(&rep);
        let all = (self.syntax_filtered() || self.find_panel().is_some_and(|p| p.in_selection))
            .then(|| self.all_matches(&query, &opts, cx));
        if self.syntax_filtered() && (all.as_ref().is_none_or(Vec::is_empty) || !self.syntax_replace_admitted(cx)) {
            return;
        }
        self.with_active(cx, |e, cx| {
            let last = *e.state.selections.last().unwrap();
            let matched = match &all {
                None => e.read_state(cx).is_match(&query, &opts, &last.range()),
                Some(all) => all.binary_search_by_key(&last.start(), |r| r.start).is_ok_and(|i| all[i] == last.range()),
            };
            if matched {
                e.edit(cx, |s| {
                    s.set_selection(last.anchor, last.head);
                    let tail = s.len() - last.end();
                    s.replace_with(&r, &[last.range()]);
                    s.set_cursor(s.len() - tail);
                });
            }
        });
        if self.find_scope().is_some() {
            // Changed maps the scope before advancing past a replacement that grew it.
            let this = cx.entity();
            cx.defer(move |cx| {
                this.update(cx, |ws, cx| {
                    ws.find_step(true, cx);
                    ws.refresh_matches(cx);
                });
            });
        } else {
            self.find_step(true, cx);
            self.refresh_matches(cx);
        }
    }

    pub(crate) fn use_selection_for_replace(
        &mut self,
        _: &UseSelectionForReplace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let (Some(text), Some(p)) = (self.selection_for_find(cx), self.find_panel()) {
            p.replace.update(cx, |e, cx| e.set_text(&text, cx));
        }
    }

    pub(crate) fn use_selection_for_find(&mut self, _: &UseSelectionForFind, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_for_find(cx) {
            self.last_query = text.clone();
            if let Some(query) = self.find_panel().map(|p| p.query.clone()) {
                self.find_quiet = true;
                self.find_origin = self.caret_for_find(cx);
                query.update(cx, |e, cx| e.set_text(&text, cx));
            }
            self.refresh_matches(cx);
        }
    }

    pub(crate) fn focus_next_field(&mut self, _: &find_panel::FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.find_panel() {
            let target = if p.show_replace && p.query.read(cx).is_focused(window) { &p.replace } else { &p.query };
            target.read(cx).focus(window);
        }
    }

    pub(crate) fn close_panel(&mut self, _: &ClosePanel, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_find(cx);
        self.remember_result_browser(cx);
        self.cancel_comparison();
        self.panel = None;
        self.refresh_menus(cx);
        for tab in self.views.tabs() {
            tab.update(cx, |e, cx| {
                // no outlines, and no matches kept for typing to carry along
                e.state.drop_found();
                if !e.results {
                    e.find_ranges.clear();
                    cx.notify();
                }
            });
        }
        self.focus_editor(window, cx);
        cx.notify();
    }

    /// Find > Find in Files… (⇧⌘F): the Find field takes a one-line selection, else the last
    /// query; Where keeps the last value in this window.
    pub(crate) fn find_in_files(&mut self, _: &FindInFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_panel(cx);
        self.cancel_quick_panel(cx);
        let seed = self.selection_for_find(cx).unwrap_or_else(|| self.last_fif_query.clone());
        if let Some(p) = self.fif_panel() {
            let query = p.query.clone();
            query.update(cx, |e, cx| e.set_text(&seed, cx));
            query.read(cx).focus(window);
        } else {
            let mut panel = FindInFilesPanel::new(&seed, &self.last_where, &self.last_fif_replace, window, cx);
            (panel.opts, panel.flags) = (self.fif_opts, self.fif_flags);
            self.show_panel(Panel::FindInFiles(panel), cx);
            // the Find panel's outlines go with it
            self.refresh_matches(cx);
        }
        cx.notify();
    }

    /// Replace…: the same search, each match with its replacement, previewed in the results
    /// browser; nothing changes until Apply.
    pub(crate) fn replace_in_files(&mut self, _: &ReplaceInFiles, window: &mut Window, cx: &mut Context<Self>) {
        // from the menu: the panel opens with the keys in Replace
        if self.fif_panel().is_none() {
            self.find_in_files(&FindInFiles, window, cx);
            if let Some(p) = self.fif_panel() {
                p.replace.read(cx).focus(window);
            }
            return;
        }
        self.start_find_in_files(true, window, cx);
    }

    fn start_find_in_files(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.validate_fif(cx);
        let Some(p) = self.fif_panel() else {
            return;
        };
        let (query, where_, rep) = (p.query.read(cx).text(cx), p.where_.read(cx).text(cx), p.replace.read(cx).text(cx));
        let (opts, flags) = (p.opts, p.flags);
        if query.is_empty() {
            return;
        }
        let replacement = replace.then_some(rep.as_str());
        if let Err(e) = self.run_search(&query, &where_, replacement, opts, flags, window, cx) {
            if let Some(p) = self.fif_panel_mut() {
                p.error = Some(e);
            }
            cx.notify();
        }
    }

    /// The results browser's Refresh: the result set's own search again, a fresh preview.
    pub(crate) fn refresh_results(&mut self, _: &RefreshResults, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = &self.result_set else { return };
        let (q, w, rep, o, f) = (r.query.clone(), r.where_.clone(), r.replacement.clone(), r.opts, r.flags);
        if let Err(e) = self.run_search_page((&q, &w, rep.as_deref(), o, f), true, window, cx) {
            self.status_message(e, cx);
        }
    }

    /// Start a run: everything is checked first (the pattern, the replacement, Where's globs
    /// and that it names something), so a problem is said before any file is read. Open
    /// buffers in every window are searched as they are in the editor when Use Buffer is on.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_search(
        &mut self,
        query: &str,
        where_: &str,
        replacement: Option<&str>,
        opts: FindOpts,
        flags: SearchFlags,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.run_search_page((query, where_, replacement, opts, flags), false, window, cx)
    }

    fn run_search_page(
        &mut self,
        input: (&str, &str, Option<&str>, FindOpts, SearchFlags),
        refresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let (query, where_, replacement, opts, flags) = input;
        if self.applying.is_some() {
            return Err("Replacements are being applied; wait for them or cancel".into());
        }
        let matcher = Matcher::new(query, &opts)?;
        let replacer = match replacement {
            Some(r) => Some(Replacer::new(query, r, &opts, flags.preserve_case)?),
            None => None,
        };
        let open: Vec<PathBuf> = (self.views.tabs()).filter_map(|t| t.read(cx).doc(cx).path.clone()).collect();
        let w = search::parse_where(where_);
        let previous_lines = if w.previous_lines {
            Some(
                (if refresh {
                    self.result_set.as_ref().and_then(|r| r.previous_lines.clone())
                } else {
                    self.previous_lines.clone()
                })
                .or_else(|| self.result_set.as_ref().and_then(|r| r.previous_lines.clone()))
                .map(Ok)
                .unwrap_or_else(|| {
                    self.result_set
                        .as_ref()
                        .ok_or_else(|| "Previous Matching Lines: there are no results in this window yet".to_string())?
                        .line_scope()
                })?,
            )
        } else {
            None
        };
        let previous = match &previous_lines {
            Some(lines) => Some(lines.iter().map(|s| s.route.clone()).collect()),
            None => self.result_set.as_ref().map(ResultSet::source_files),
        };
        let roots: Vec<PathBuf> = crate::config::project::present(&self.roots);
        let mut scope = search::resolve(&w, &roots, &open, previous.as_deref().unwrap_or(&[]));
        scope.gitignore = flags.gitignore;
        scope.root_excludes = (self.roots.iter())
            .filter(|r| !r.folder_exclude_patterns.is_empty() || !r.file_exclude_patterns.is_empty())
            .map(|r| {
                let (d, f) = (&r.folder_exclude_patterns, &r.file_exclude_patterns);
                (r.path.clone(), d.clone(), f.clone())
            })
            .collect();
        if refresh {
            if let Some(retained) = self.result_set.as_ref().and_then(|r| r.scope.clone()) {
                scope = retained;
            }
        }
        if let Some(why) = search::scope_problem(&w, &scope, previous.is_some()) {
            return Err(why);
        }
        let open_editors = self.open_editors(cx);
        let problems: std::collections::HashMap<_, _> = previous_lines
            .iter()
            .flat_map(|sources| sources.iter())
            .filter_map(|source| previous_line_problem(source, &open_editors, cx).map(|why| (source.path.clone(), why)))
            .collect();
        let reads: std::collections::HashMap<_, _> = previous_lines
            .iter()
            .flat_map(|sources| sources.iter())
            .filter(|s| matches!(s.source, Source::Disk { .. }))
            .map(|s| (s.path.clone(), pipelines::observe_read(&s.route, cx)))
            .collect();
        let mut hints: std::collections::HashMap<_, _> =
            self.closed.iter().filter_map(|v| Some((v.path.clone(), v.disk_encoding_hint?))).collect();
        hints.extend(self.open_editors(cx).into_iter().filter_map(|e| {
            let d = e.read(cx).doc(cx);
            Some((d.path.clone()?, d.disk_encoding))
        }));
        let mut buffers: Vec<BufferText> = Vec::new();
        if flags.use_buffer {
            for t in self.open_editors(cx) {
                let s = t.read(cx).read_state(cx);
                if let Some(path) = &s.doc.path {
                    if !buffers.iter().any(|b| b.path == *path) {
                        buffers.push(BufferText {
                            path: path.clone(),
                            rope: s.doc.rope.clone(),
                            id: s.buffer_id(),
                            version: s.doc.version,
                        });
                    }
                }
            }
        }
        let retire = if refresh { None } else { self.admit_result_page()? };
        (self.last_fif_query, self.last_where, self.fif_opts, self.fif_flags) =
            (query.to_string(), where_.to_string(), opts, flags);
        self.query_history.remember(query);
        self.where_history.remember(where_);
        if let Some(r) = replacement {
            self.last_fif_replace = r.to_string();
            self.replace_history.remember(r);
        }
        // a search still running ends first, marked cancelled
        self.stop_search(cx);
        if refresh {
            self.ensure_result_page();
        } else {
            self.create_result_page(retire, cx);
        }
        self.reset_result_review();
        self.result_pages.request += 1;
        let (page, request) = (self.result_pages.active, self.result_pages.request);
        if let Some(panel) = self.results_panel_mut() {
            panel.invalidate_open();
        }
        self.result_set = Some(ResultSet::new(query, replacement, opts, flags, where_));
        self.result_set.as_mut().unwrap().previous_lines = previous_lines.clone();
        self.result_set.as_mut().unwrap().scope = Some(scope.clone());
        if !refresh && previous_lines.is_some() {
            self.previous_lines = previous_lines.clone();
        }
        let results = match replacement {
            None => {
                self.close_panel(&ClosePanel, window, cx);
                Some(self.results_view(window, cx).downgrade())
            }
            Some(_) => {
                self.show_results_panel(window, cx);
                None
            }
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        // unknown until the walk is done
        let total = Arc::new(AtomicUsize::new(usize::MAX));
        let listed = Arc::new(AtomicUsize::new(0));
        let context = if flags.context { search::CONTEXT } else { 0 };
        let replacing = replacer.is_some();
        let work = cx.background_spawn({
            let (cancel, done, total, listed) = (cancel.clone(), done.clone(), total.clone(), listed.clone());
            let query = query.to_string();
            async move {
                let roots = search::typed_roots(&scope);
                let files = search::collect(&scope, &cancel, &listed);
                total.store(files.len(), Ordering::Relaxed);
                let _ = tx.send(SearchMsg::Header(search::header(files.len(), &query, &opts)));
                let stop = || cancel.load(Ordering::Relaxed);
                let search_text = |text: &str, binary| {
                    search::search_source(text, binary, &matcher, context, replacer.as_ref(), &stop)
                };
                for f in &files {
                    if stop() {
                        break;
                    }
                    let resolved = previous_lines
                        .as_ref()
                        .and_then(|sources| sources.iter().find(|s| s.route == *f))
                        .map_or_else(|| crate::editing::text::resolve(f), |s| s.path.clone());
                    let source_route = search::shown(f, &roots);
                    let open = buffers.iter().find(|b| b.path == resolved);
                    let mut captured = None;
                    let searched = if let Some(lines) = &previous_lines {
                        let source = lines.iter().find(|s| s.route == *f).unwrap();
                        let found = match problems.get(&source.path) {
                            Some(why) => Some((search::Found::Failed(stale_lines(why)), None)),
                            None => search_previous_lines(source, &matcher, context, replacer.as_ref(), &stop),
                        };
                        found.map(|(found, next)| {
                            captured = next;
                            (source.source.clone(), found)
                        })
                    } else {
                        match open {
                            Some(b) => {
                                let source = Source::Buffer { id: b.id, version: b.version };
                                let text = b.rope.to_string();
                                search_text(&text, false).map(|found| {
                                    captured = capture_lines(
                                        &source_route,
                                        &resolved,
                                        &source,
                                        &text,
                                        Some(b.rope.clone()),
                                        &found,
                                    );
                                    (source, found)
                                })
                            }
                            // stamped before the read: a write during it shows as a change
                            None => match (
                                crate::files::watch::stamp(f),
                                search::read_file_with_hint(f, hints.get(&resolved).copied()),
                            ) {
                                (stamp, Ok(t)) => {
                                    let found = t.search(&matcher, context, replacer.as_ref(), &stop);
                                    let source = Source::Disk {
                                        stamp,
                                        hash: t.hash,
                                        line_ending: t.line_ending,
                                        bom: t.bom,
                                        encoding: t.encoding,
                                        lossy: t.lossy,
                                        binary: t.binary,
                                    };
                                    found.map(|found| {
                                        captured =
                                            capture_lines(&source_route, &resolved, &source, &t.text, None, &found);
                                        (source, found)
                                    })
                                }
                                (stamp, Err(None)) => {
                                    let source = Source::Disk {
                                        stamp,
                                        hash: 0,
                                        line_ending: "\n",
                                        bom: false,
                                        encoding: Default::default(),
                                        lossy: false,
                                        binary: false,
                                    };
                                    Some((source, search::Found::TooLarge))
                                }
                                // unreadable (gone, permission denied): skipped
                                (_, Err(Some(_))) => None,
                            },
                        }
                    };
                    if let Some((source, found)) = searched {
                        let shown = search::shown(f, &roots);
                        // Replace in Files shows its results in the panel, not Find Results
                        let block = match replacing {
                            true => Default::default(),
                            false => search::file_block(&shown, &found),
                        };
                        let n = found.matches();
                        let result = file_result(resolved, shown, source, found, replacing);
                        let _ =
                            tx.send(SearchMsg::File(Box::new(result), captured.map(Box::new), (block.0, block.1, n)));
                    }
                    done.fetch_add(1, Ordering::Relaxed);
                }
                let _ = tx.send(SearchMsg::Done(stop()));
            }
        });
        let drain = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            if this.update(cx, |ws, cx| ws.drain_search_for(page, request, cx)).unwrap_or(true) {
                break;
            }
        });
        self.search = Some(Search {
            page,
            request,
            cancel,
            done,
            total,
            listed,
            results,
            rx,
            header: false,
            query: query.to_string(),
            opts,
            matches: 0,
            files: 0,
            reads,
            _tasks: (work, drain),
        });
        cx.notify();
        Ok(())
    }

    pub(crate) fn validate_fif(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.fif_panel_mut() {
            let query = p.query.read(cx).text(cx);
            let replace = p.replace.read(cx).text(cx);
            p.error = match p.opts.regex {
                false => None,
                // the replacement is checked once there is one, so a plain Find is not held up
                // by a half-typed one
                true if replace.is_empty() => Pattern::new(&query, &p.opts).err(),
                true => Pattern::new(&query, &p.opts)
                    .and_then(|_| Replacer::new(&query, &replace, &p.opts, false).map(|_| ()))
                    .err(),
            };
            cx.notify();
        }
    }

    /// Find Results, reused (and made active in its group) or opened at the end of the active
    /// group; a new run starts two lines below the last one, scrolled into view.
    fn results_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Editor> {
        let existing = self.views.tabs().find(|t| t.read(cx).results).cloned();
        let results = match existing.and_then(|e| Some((self.views.find(&e)?, e))) {
            Some(((g, i), e)) => {
                self.views.select(g, i);
                self.focus_editor(window, cx);
                e
            }
            None => {
                let mut state = EditorState::new();
                state.doc.scratch = true;
                let e = self.new_editor(state, cx);
                e.update(cx, |e, _| e.results = true);
                self.attach(e.clone(), false, window, cx);
                e
            }
        };
        results.update(cx, |e, cx| {
            if !e.doc(cx).is_empty() {
                e.append("\n\n", [], cx);
            }
            sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).len());
            e.scroll_to_cursor(cx);
        });
        results
    }

    /// Move what the search found so far into the result set and Find Results; true once it
    /// has ended.
    pub(crate) fn drain_search_for(&mut self, page: u64, request: u64, cx: &mut Context<Self>) -> bool {
        if !self.search.as_ref().is_some_and(|s| s.page == page && s.request == request) {
            return true;
        }
        self.drain_search(cx)
    }

    fn drain_search(&mut self, cx: &mut Context<Self>) -> bool {
        let page = self.search.as_ref().map(|s| s.page).unwrap_or(0);
        let restricted = self.result_page(page).and_then(|r| r.previous_lines.clone());
        let open_editors = if restricted.is_some() { self.open_editors(cx) } else { Vec::new() };
        let Some(s) = self.search.as_mut() else {
            return true;
        };
        let (mut text, mut outlines, mut chars) = (String::new(), Vec::new(), 0);
        let mut finished = None;
        let mut found = Vec::new();
        let mut captured = Vec::new();
        while let Ok(msg) = s.rx.try_recv() {
            let piece = match msg {
                SearchMsg::Header(h) => {
                    s.header = true;
                    h
                }
                SearchMsg::File(mut result, source, (mut block, mut ranges, mut n)) => {
                    let stale = restricted
                        .as_ref()
                        .and_then(|sources| sources.iter().find(|s| s.path == result.path))
                        .and_then(|source| {
                            previous_line_problem(source, &open_editors, cx).or_else(|| {
                                s.reads
                                    .get(&source.path)
                                    .filter(|read| read.changed())
                                    .map(|_| "an overlapping write invalidated its read".into())
                            })
                        });
                    if let Some(why) = stale {
                        let failure = search::Found::Failed(stale_lines(&why));
                        let b = search::file_block(&result.shown, &failure);
                        (block, ranges, n) = (b.0, b.1, 0);
                        result.body = Body::Failed(stale_lines(&why));
                        result.lines.clear();
                    } else if let Some(source) = source {
                        captured.push(*source);
                    }
                    found.push(*result);
                    outlines.extend(ranges.into_iter().map(|r| chars + r.start..chars + r.end));
                    s.matches += n;
                    s.files += usize::from(n > 0);
                    block
                }
                SearchMsg::Done(cancelled) => {
                    finished = Some(cancelled);
                    search::footer(s.matches, s.files, cancelled)
                }
            };
            chars += piece.chars().count();
            text += &piece;
            if finished.is_some() {
                break;
            }
        }
        // Find Results saved as a file in the meantime takes no more results
        let results = (s.results.as_ref()).map(|r| r.upgrade().filter(|r| r.read(cx).results));
        if let Some(None) = results {
            s.cancel.store(true, Ordering::Relaxed);
            finished = finished.or(Some(true));
        }
        if let Some(results) = results.flatten().filter(|_| !text.is_empty()) {
            results.update(cx, |e, cx| e.append(&text, outlines, cx));
        }
        let searched = s.done.load(Ordering::Relaxed);
        if let Some(set) = self.result_page_mut(page) {
            set.files.extend(found);
            set.matching_lines.extend(captured);
            set.searched = searched;
            if let Some(cancelled) = finished {
                (set.done, set.cancelled) = (true, cancelled);
            }
        }
        if finished.is_some() {
            self.search = None;
        }
        cx.notify();
        finished.is_some()
    }

    /// Stop the running search: what it found stays, and its block ends with `Cancelled!`.
    pub(crate) fn stop_search(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &self.search else {
            return;
        };
        s.cancel.store(true, Ordering::Relaxed);
        if self.drain_search(cx) {
            return;
        }
        if let Some(s) = self.search.take() {
            let tab = s.results.as_ref().and_then(|r| r.upgrade());
            if let Some(results) = tab.filter(|r| r.read(cx).results) {
                // cancelled while still listing files: the header counts those found so far
                let mut end = match s.header {
                    true => String::new(),
                    false => search::header(s.listed.load(Ordering::Relaxed), &s.query, &s.opts),
                };
                end += &search::footer(s.matches, s.files, true);
                results.update(cx, |e, cx| e.append(&end, [], cx));
            }
            if let Some(set) = self.result_page_mut(s.page) {
                (set.done, set.cancelled) = (true, true);
            }
        }
        cx.notify();
    }

    /// The status bar's "Find in Files: 1,234 of 14,451 files" while a search runs.
    pub(crate) fn search_progress(&self) -> Option<String> {
        let s = self.search.as_ref()?;
        let total = s.total.load(Ordering::Relaxed);
        let done = s.done.load(Ordering::Relaxed);
        Some(search::progress(done, (total != usize::MAX).then_some(total)))
    }

    fn step_result(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        // the results browser, while it is open, is what F4 walks
        if self.results_panel().is_some() {
            return self.step_browser(forward, window, cx);
        }
        let results = self
            .active_editor()
            .filter(|e| e.read(cx).results)
            .or_else(|| {
                self.last_results
                    .as_ref()
                    .and_then(|e| e.upgrade())
                    .filter(|e| e.read(cx).results && self.views.find(e).is_some())
            })
            .or_else(|| self.views.tabs().find(|e| e.read(cx).results).cloned());
        let Some(results) = results else { return };
        let e = results.read(cx);
        let selected = e.state.selections.last().unwrap().range();
        let all = &e.doc(cx).outlines;
        let range = if forward {
            all.iter().find(|r| r.start >= selected.end && **r != selected).or(all.first())
        } else {
            all.iter().rev().find(|r| r.start < selected.start).or(all.last())
        };
        let Some(range) = range.cloned() else { return };
        let line = e.doc(cx).point(range.start).0;
        let Some((path, line_number)) = search::result_target(e.doc(cx), line) else {
            return;
        };
        let hit = search::result_match(e.doc(cx), all, line, range.start);
        results.update(cx, |e, cx| e.select_range(range, cx));
        self.open_result(results, path, line_number, hit, window, cx);
    }

    pub(crate) fn open_result(
        &mut self,
        results: Entity<Editor>,
        path: PathBuf,
        line: usize,
        hit: Option<(usize, Option<String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((g, i)) = self.views.find(&results) else {
            return;
        };
        self.last_results = Some(results.downgrade());
        let path = crate::editing::text::resolve(&path);
        if pipelines::reading_blocked(&path, cx) {
            cx.spawn_in(window, async move |this, cx| {
                pipelines::wait_for_write(&path, cx).await;
                this.update_in(cx, |ws, window, cx| ws.open_result(results, path, line, hit, window, cx)).ok();
            })
            .detach();
            return;
        }
        let is_it = |t: &Entity<Editor>, cx: &App| t.read(cx).doc(cx).path.as_deref() == Some(path.as_path());
        self.views.select(g, i);
        let here = self.views.groups[g].tabs.iter().position(|t| is_it(t, cx));
        let editor = match here {
            Some(j) => {
                self.views.select(g, j);
                self.views.groups[g].tabs[j].clone()
            }
            None => {
                let elsewhere = self.views.tabs().find(|t| is_it(t, cx)).cloned();
                let big = std::fs::metadata(&path).is_ok_and(|m| m.len() > LOAD_IN_BACKGROUND);
                if elsewhere.is_none() && (big || self.loading.contains(&path)) {
                    // read in the background (or already being read): the jump waits for it
                    self.pending_jumps.insert(path.clone(), PendingJump { line, hit, selection: None, browser: None });
                    return self.read_in_background(path, window, cx);
                }
                let view = match elsewhere {
                    Some(other) => other.update(cx, |src, cx| cx.new(|cx| src.another_view(cx))),
                    None => match EditorState::load(&path) {
                        Ok(state) => {
                            push_recent(&mut cx.global_mut::<Recent>().files, path.clone());
                            self.refresh_menus(cx);
                            self.new_editor(state, cx)
                        }
                        Err(e) => {
                            let detail = format!("{}: {e}", path.display());
                            return self.alert(window, cx, "Could not open file", &detail);
                        }
                    },
                };
                self.attach(view.clone(), true, window, cx);
                view
            }
        };
        jump_to_result(&editor, line, &hit, cx);
        self.focus_editor(window, cx);
        self.refresh_matches(cx);
        cx.notify();
    }

    fn step_fif_field(&mut self, by: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.fif_panel() {
            let fields = p.fields();
            let at = fields.iter().position(|f| f.read(cx).is_focused(window));
            let next = at.map_or(0, |i| (i + by) % fields.len());
            fields[next].read(cx).focus(window);
        }
    }
}

/// A searched file's structured result. Every match starts included; one whose replacement
/// could not be made (the pattern no longer matched there) starts excluded.
fn file_result(path: PathBuf, shown: PathBuf, source: Source, found: search::Found, replacing: bool) -> FileResult {
    let (body, lines) = match found {
        search::Found::Lines { lines, matches, .. } => {
            let entries = (matches.into_iter())
                .map(|hit| Entry { included: !replacing || hit.replacement.is_some(), hit })
                .collect();
            (Body::Matches(entries), lines)
        }
        search::Found::Binary => (Body::Binary, Vec::new()),
        search::Found::TooLarge => (Body::TooLarge, Vec::new()),
        search::Found::Failed(why) => (Body::Failed(why), Vec::new()),
    };
    FileResult { path, shown, source, body, lines, included: true, collapsed: false, status: None }
}

fn stale_lines(why: &str) -> String {
    format!("Previous Matching Lines is stale: {why}; refresh the source search and choose its lines again")
}

fn capture_lines(
    route: &Path,
    path: &Path,
    source: &Source,
    text: &str,
    buffer: Option<ropey::Rope>,
    found: &search::Found,
) -> Option<MatchingLines> {
    let search::Found::Lines { matches, .. } = found else { return None };
    Some(MatchingLines {
        path: path.to_path_buf(),
        route: route.to_path_buf(),
        source: source.clone(),
        ranges: matching_lines(text, matches),
        buffer,
        file: pipelines::file_id(route),
    })
}

fn search_previous_lines(
    source: &MatchingLines,
    matcher: &Matcher,
    context: usize,
    replacer: Option<&Replacer>,
    stop: &dyn Fn() -> bool,
) -> Option<(search::Found, Option<MatchingLines>)> {
    let stale = |why| Some((search::Found::Failed(stale_lines(why)), None));
    let text = match &source.buffer {
        Some(rope) => rope.to_string(),
        None => {
            let Source::Disk { stamp, encoding, .. } = &source.source else { return stale("source unavailable") };
            if stamp.is_none() || crate::files::watch::stamp(&source.route) != *stamp {
                return stale("changed or missing on disk");
            }
            let Ok(read) = search::read_file_with_hint(&source.route, Some(*encoding)) else {
                return stale("cannot read the source");
            };
            let current = Source::Disk {
                stamp: crate::files::watch::stamp(&source.route),
                hash: read.hash,
                line_ending: read.line_ending,
                bom: read.bom,
                encoding: read.encoding,
                lossy: read.lossy,
                binary: read.binary,
            };
            if current != source.source || read.lossy || read.binary {
                return stale("disk bytes or decoded format changed");
            }
            read.text
        }
    };
    let found = search::search_source_scoped(&text, matcher, &source.ranges, context, replacer, stop)?;
    let captured = capture_lines(&source.route, &source.path, &source.source, &text, source.buffer.clone(), &found);
    Some((found, captured))
}

/// Exact source admission for retained line coordinates, also used immediately before Apply.
pub(crate) fn previous_line_problem(source: &MatchingLines, open: &[Entity<Editor>], cx: &App) -> Option<String> {
    if crate::editing::text::resolve(&source.route) != source.path || pipelines::file_id(&source.route) != source.file {
        return Some("the source path is missing or rebound".into());
    }
    if pipelines::reading_blocked(&source.route, cx) {
        return Some("another operation is writing the source".into());
    }
    let peers: Vec<_> = open
        .iter()
        .filter(|e| {
            e.read(cx).doc(cx).path.as_ref().is_some_and(|p| {
                crate::editing::text::resolve(p) == source.path
                    || source.file.is_some_and(|file| pipelines::file_id(p) == Some(file))
            })
        })
        .collect();
    match &source.source {
        Source::Buffer { id, version } => {
            let Some(e) = peers.iter().find(|e| e.read(cx).doc(cx).id() == *id) else {
                return Some("the source buffer was closed or rebound".into());
            };
            let doc = e.read(cx).doc(cx);
            if doc.path.as_ref() != Some(&source.path)
                || doc.version != *version
                || !source.buffer.as_ref().is_some_and(|rope| rope.is_instance(&doc.rope))
                || peers.iter().any(|e| e.read(cx).doc(cx).id() != *id)
            {
                return Some("the source buffer was edited, reinterpreted or opened through another alias".into());
            }
        }
        Source::Disk { stamp, encoding, bom, line_ending, .. } => {
            if stamp.is_none() || crate::files::watch::stamp(&source.route) != *stamp {
                return Some("the source changed or disappeared on disk".into());
            }
            if peers.iter().any(|e| {
                let doc = e.read(cx).doc(cx);
                doc.dirty
                    || doc.disk != *stamp
                    || doc.disk_encoding != *encoding
                    || doc.bom != *bom
                    || doc.line_ending != *line_ending
                    || !doc.watching
            }) {
                return Some("the source was edited or reinterpreted in an open buffer".into());
            }
        }
    }
    None
}

/// A Find Results line's match: its column and the text cut before it, if the line was cut.
pub(crate) type Hit = Option<(usize, Option<String>)>;

/// A Find Results line's place in its file: a match line at the match, a header or context
/// line at the line's start.
pub(crate) fn jump_to_result(editor: &Entity<Editor>, line: usize, hit: &Hit, cx: &mut App) {
    editor.update(cx, |e, cx| match hit {
        Some((col, cut)) => {
            let col = search::file_column(&e.doc(cx).line(line), *col, cut.as_deref());
            let at = e.doc(cx).offset(line, col);
            e.select_range(at..at, cx);
        }
        None => e.goto_line(line + 1, cx),
    });
}

/// One bounded classification per exact source; query/option changes reuse it.
pub(crate) struct SyntaxSearch {
    view: EntityId,
    document: u64,
    version: u64,
    rope: ropey::Rope,
    source: SearchSource,
    cancel: Arc<AtomicBool>,
    result: Option<Option<Arc<SearchClasses>>>,
}

impl Drop for SyntaxSearch {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl SyntaxSearch {
    fn current(&self, editor: &Entity<Editor>, cx: &App) -> bool {
        let e = editor.read(cx);
        let doc = e.doc(cx);
        self.view == editor.entity_id()
            && self.document == doc.id()
            && self.version == doc.version
            && self.rope.is_instance(&doc.rope)
            && e.highlighter.search_source(&doc.rope) == Some(self.source)
    }
}

impl Workspace {
    /// Search readiness is independent of the panel's replacement/admission error.
    pub(crate) fn syntax_matches(
        &mut self,
        query: &str,
        opts: &FindOpts,
        cx: &mut Context<Self>,
    ) -> Result<Vec<Range<usize>>, String> {
        let editor = self.active_editor().ok_or("Open a tab first")?;
        if self.syntax_classes(&editor, cx).is_none() {
            return Err(self
                .find_panel()
                .and_then(|p| p.error.clone())
                .unwrap_or_else(|| "Syntax search is unavailable".into()));
        }
        self.matches_now(query, opts, cx)
            .ok_or_else(|| "Syntax search is preparing matches; matches are unavailable".into())
    }

    fn syntax_filtered(&self) -> bool {
        self.find_panel().is_some_and(|p| p.syntax != SyntaxMask::Any)
    }

    fn syntax_classes(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) -> Option<Arc<SearchClasses>> {
        if self.find_debounce.is_some() {
            self.find_panel_mut()?.error = Some("Syntax search is preparing; matches are unavailable".into());
            return None;
        }
        if self.find_panel()?.syntax_search.as_ref().is_some_and(|search| search.current(editor, cx)) {
            let result = self.find_panel()?.syntax_search.as_ref()?.result.clone();
            return match result {
                Some(Some(classes)) => Some(classes),
                result => {
                    self.find_panel_mut()?.error = Some(if result.is_none() {
                        "Syntax search is preparing; matches are unavailable".into()
                    } else {
                        "Syntax search is unavailable for this document or grammar limit".into()
                    });
                    None
                }
            };
        }
        self.find_panel_mut()?.syntax_search = None;
        let e = editor.read(cx);
        let doc = e.doc(cx);
        let Some(source) = e.highlighter.search_source(&doc.rope) else {
            self.find_panel_mut()?.error =
                Some("Syntax search is unavailable for this language or document size".into());
            return None;
        };
        let rope = doc.rope.clone();
        let job = e.highlighter.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.find_panel_mut()?.syntax_search = Some(SyntaxSearch {
            view: editor.entity_id(),
            document: doc.id(),
            version: doc.version,
            rope: rope.clone(),
            source,
            cancel: cancel.clone(),
            result: None,
        });
        self.find_panel_mut()?.error = Some("Syntax search is preparing; matches are unavailable".into());
        let work_cancel = cancel.clone();
        let work = cx.background_spawn(async move {
            let deadline = Deadline::new(crate::searching::pattern::UI_SECONDS);
            job.search_classes(&rope, &|| work_cancel.load(Ordering::Relaxed) || deadline.passed()).map(Arc::new)
        });
        let editor = editor.clone();
        cx.spawn(async move |ws, cx| {
            let classes = work.await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            ws.update(cx, |ws, cx| {
                let valid = ws
                    .find_panel()
                    .and_then(|p| p.syntax_search.as_ref())
                    .is_some_and(|search| Arc::ptr_eq(&search.cancel, &cancel) && search.current(&editor, cx))
                    && ws.active_editor().as_ref() == Some(&editor);
                if !valid {
                    return;
                }
                ws.find_panel_mut().unwrap().syntax_search.as_mut().unwrap().result = Some(classes);
                ws.refresh_matches(cx);
                if ws.find_pending.as_deref() == Some(ws.current_query(cx).as_str()) {
                    ws.find_as_you_type(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    /// Preserve marked text/history on refusals, before borrowing the source for mutation.
    fn syntax_replace_admitted(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.active_editor() else {
            return false;
        };
        let e = editor.read(cx);
        let error = if !e.input_enabled || e.doc(cx).edits_blocked() {
            Some("This source is read-only".to_string())
        } else {
            self.selection_set_admission(&editor, cx).err()
        };
        if let Some(error) = error {
            if let Some(p) = self.find_panel_mut() {
                p.error = Some(error);
            }
            cx.notify();
            return false;
        }
        let current = self
            .find_panel()
            .and_then(|p| p.syntax_search.as_ref())
            .is_some_and(|search| search.current(&editor, cx) && matches!(search.result, Some(Some(_))));
        if current {
            editor.update(cx, |e, cx| e.catch_up(cx));
        }
        current
    }
}
