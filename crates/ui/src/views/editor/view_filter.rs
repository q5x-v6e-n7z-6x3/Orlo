//! Reversible, view-local source narrowing through the existing fold/row mapping.
use super::*;
use crate::editing::fold::Focus;
use orlo_editor::search::pattern::{Deadline, Pattern, SearchError};

pub(super) struct ViewFilter {
    pub prior: Folds,
    mode: Mode,
    focus: Focus,
    version: Option<u64>,
    /// Shown once already: the scans after an edit keep the lines the carets are on.
    published: bool,
}

enum Mode {
    Region,
    Matching { pattern: Arc<Pattern>, context: usize },
}

pub(super) struct FilterJob {
    stop: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for FilterJob {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn matching(rope: &ropey::Rope, pattern: &Pattern, context: usize, stop: &AtomicBool) -> Result<Focus, SearchError> {
    let deadline = Deadline::new(orlo_editor::search::pattern::BACKGROUND_SECONDS);
    Focus::matching_lines(rope, pattern, context, &|| stop.load(Ordering::Relaxed) || deadline.passed())
        .map_err(|error| deadline.explain(error))
}

impl Editor {
    #[cfg(test)]
    pub(crate) fn filter_pending(&self) -> bool {
        self.filter_job.is_some()
    }

    pub(crate) fn narrow_to_selection(&mut self, _: &NarrowToSelection, _: &mut Window, cx: &mut Context<Self>) {
        if !self.admit_view_filter(cx) {
            return;
        }
        self.catch_up(cx);
        let ranges: Vec<_> =
            self.state.selections.iter().filter(|s| s.start() < s.end()).map(Selection::range).collect();
        if ranges.is_empty() {
            return self.status("Select source text before narrowing", cx);
        }
        let focus = Focus::new(self.doc(cx).len(), ranges);
        self.start_view_filter(Mode::Region, focus, cx);
    }

    pub(crate) fn narrow_to_fold(&mut self, _: &NarrowToFold, _: &mut Window, cx: &mut Context<Self>) {
        if !self.admit_view_filter(cx) {
            return;
        }
        self.catch_up(cx);
        let doc = self.doc(cx);
        let active = self.view_filter.as_ref().map_or(&self.folds, |f| &f.prior).chars();
        let syntax = self.highlighter.fold_regions(&doc.rope);
        let mut ranges = Vec::new();
        for sel in &self.state.selections {
            let at = sel.head;
            let line = doc.point(at).0;
            let found = active
                .iter()
                .filter(|r| (r.start <= at && at <= r.end) || doc.point(r.start).0 == line)
                .min_by_key(|r| r.len())
                .cloned();
            let found = found.or_else(|| match &syntax {
                Some(regions) => regions.innermost(line),
                None => crate::editing::fold::innermost(doc, line).map(|(h, l)| doc.line_end(h)..doc.line_end(l)),
            });
            if let Some(r) = found {
                ranges.push(doc.line_start(doc.point(r.start).0)..r.end);
            }
        }
        if ranges.is_empty() {
            return self.status("No fold region at the caret", cx);
        }
        self.start_view_filter(Mode::Region, Focus::new(doc.len(), ranges), cx);
    }

    pub(crate) fn show_matching_lines(&mut self, a: &ShowMatchingLines, _: &mut Window, cx: &mut Context<Self>) {
        if !self.admit_view_filter(cx) {
            return;
        }
        let pattern = match Pattern::new(
            &a.pattern,
            &crate::editing::text::FindOpts { regex: true, case_sensitive: !a.ignore_case, ..Default::default() },
        ) {
            Ok(pattern) => Arc::new(pattern),
            Err(error) => return self.status(error, cx),
        };
        self.catch_up(cx);
        self.start_view_filter(Mode::Matching { pattern, context: a.context }, Focus::default(), cx);
    }

    pub(crate) fn clear_filter(&mut self, _: &ClearFilter, _: &mut Window, cx: &mut Context<Self>) {
        if self.admit_view_filter(cx) {
            self.catch_up(cx);
            self.clear_view_filter(cx);
            self.status("Filter cleared", cx);
        }
    }

    /// Native workspace admission runs before this borrow. Direct callers also preserve
    /// local marked ranges; delayed replies separately check shared views.
    fn admit_view_filter(&mut self, cx: &mut Context<Self>) -> bool {
        if self.single_line || self.aligned {
            self.status("Narrowing is available in document views", cx);
            return false;
        }
        if !self.marked.is_empty() {
            self.status("Finish or cancel text composition before changing the view filter", cx);
            return false;
        }
        true
    }

    fn filter_composing(&self, cx: &Context<Self>) -> bool {
        if !self.marked.is_empty() {
            return true;
        }
        let me = cx.entity();
        self.open_views.as_ref().is_some_and(|open| {
            open(cx).into_iter().filter(|other| *other != me).any(|other| {
                let other = other.read(cx);
                other.document == self.document && !other.marked.is_empty()
            })
        })
    }

    fn start_view_filter(&mut self, mode: Mode, focus: Focus, cx: &mut Context<Self>) {
        self.cancel_filter_job();
        self.fold_job = None;
        let prior = self.view_filter.take().map_or_else(|| self.folds.clone(), |f| f.prior);
        self.view_filter = Some(ViewFilter { prior, mode, focus, version: None, published: false });
        self.refresh_view_filter(cx);
    }

    pub(super) fn cancel_filter_job(&mut self) {
        self.filter_job = None;
        self.filter_epoch = self.filter_epoch.wrapping_add(1);
    }

    pub(super) fn clear_view_filter(&mut self, cx: &mut Context<Self>) {
        self.restore_view_filter(false, cx);
    }

    pub(super) fn reveal_view_filter(&mut self, cx: &mut Context<Self>) {
        self.restore_view_filter(true, cx);
    }

    fn restore_view_filter(&mut self, reveal: bool, cx: &mut Context<Self>) {
        self.cancel_filter_job();
        if let Some(mut filter) = self.view_filter.take() {
            if reveal {
                let ends: Vec<_> = self.state.selections.iter().flat_map(|s| [s.start(), s.end()]).collect();
                filter.prior.reveal(&ends);
            }
            self.replace_folds(cx, |folds, _| *folds = filter.prior);
            self.folds_changed(cx);
        }
    }

    /// Mapping is part of ordinary presentation, including changes published by other views.
    pub(super) fn map_view_filter(&mut self, changes: &[Change], cx: &App) {
        if changes.is_empty() || self.view_filter.is_none() {
            return;
        }
        self.cancel_filter_job();
        let len = self.doc(cx).len();
        let filter = self.view_filter.as_mut().unwrap();
        filter.prior.map(changes);
        filter.focus.map(len, changes);
        filter.version = None;
    }

    pub(super) fn filter_hides_selections(&self) -> bool {
        self.view_filter.as_ref().is_some_and(|f| {
            f.version.is_some()
                && self.filter_job.is_none()
                && self
                    .state
                    .selections
                    .iter()
                    .flat_map(|s| [s.anchor, s.head])
                    .any(|at| !f.focus.contains(at) && !(f.focus.is_empty() && at == 0))
        })
    }

    pub(super) fn refresh_view_filter(&mut self, cx: &mut Context<Self>) {
        if self.view_filter.is_some() && self.state.revision() != self.doc(cx).version {
            self.catch_up(cx);
        }
        if self.view_filter.is_some() && !self.marked.is_empty() {
            self.cancel_filter_job();
            self.view_filter.as_mut().unwrap().version = None;
            if !self.folds.is_empty() {
                self.replace_folds(cx, |folds, _| *folds = Folds::default());
                self.folds_changed(cx);
            }
            return;
        }
        let Some(filter) = &self.view_filter else { return };
        let version = self.doc(cx).version;
        if filter.version == Some(version) {
            return;
        }
        if matches!(filter.mode, Mode::Region) {
            let focus = filter.focus.clone();
            self.publish_filter_focus(focus, version, cx);
            return;
        }
        // Old regex exclusions describe old text. Show current source while recomputing or
        // composing; no late worker may move the marked endpoints under an input method.
        if !self.folds.is_empty() {
            self.replace_folds(cx, |folds, _| *folds = Folds::default());
            self.folds_changed(cx);
        }
        let filter = self.view_filter.as_mut().unwrap();
        let Mode::Matching { pattern, context } = &filter.mode else { return };
        let (pattern, context) = (pattern.clone(), *context);
        filter.version = Some(version);
        let rerun = filter.published;
        let rope = self.doc(cx).rope.clone();
        self.status("Finding matching lines…", cx);
        let document = self.document.clone();
        self.sel(cx).sel_checkpoint();
        let selection = self.read_state(cx).selection_state();
        let selection_generation = self.read_state(cx).selection_generation();
        let epoch = self.filter_epoch;
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let found = cx.background_spawn(async move { matching(&rope, &pattern, context, &cancelled) });
        let task = cx.spawn(async move |this, cx| {
            let result = found.await;
            this.update(cx, |e, cx| {
                if e.filter_epoch != epoch || e.document != document || e.doc(cx).version != version {
                    return;
                }
                e.filter_job = None;
                if e.filter_composing(cx) {
                    if let Some(filter) = e.view_filter.as_mut() {
                        filter.version = None;
                    }
                    return;
                }
                let current = e.read_state(cx);
                let moved =
                    current.selection_state() != selection || current.selection_generation() != selection_generation;
                if moved && !rerun {
                    e.reveal_view_filter(cx);
                    e.status("The selections changed; run Show Matching Lines again", cx);
                    return;
                }
                // after an edit, the line being typed on stays in view though it no longer
                // matches, and its caret stays put (as do carets moved during the scan)
                let result = match rerun {
                    true => result.map(|focus| {
                        let heads = e.state.selections.iter().flat_map(|s| [s.anchor, s.head]);
                        focus.with_lines(&e.doc(cx).rope, heads.collect::<Vec<_>>())
                    }),
                    false => result,
                };
                e.finish_filter_result(result, cx);
            })
            .ok();
        });
        self.filter_job = Some(FilterJob { stop, _task: task });
    }

    fn finish_filter_result(&mut self, result: Result<Focus, SearchError>, cx: &mut Context<Self>) {
        match result {
            Ok(focus) => {
                let empty = focus.is_empty();
                self.publish_filter_focus(focus, self.doc(cx).version, cx);
                self.status(
                    if empty {
                        "No matching lines — Clear Filter to show all text"
                    } else {
                        "Matching lines — Clear Filter to show all text"
                    },
                    cx,
                );
            }
            Err(error) => {
                self.clear_view_filter(cx);
                self.status(error.message(), cx);
            }
        }
    }

    fn publish_filter_focus(&mut self, focus: Focus, version: u64, cx: &mut Context<Self>) {
        let selections = self
            .state
            .selections
            .iter()
            .map(|s| {
                let anchor = focus.nearest(s.anchor);
                let head = focus.nearest(s.head);
                Selection {
                    anchor,
                    head,
                    anchor_virtual: if anchor == s.anchor { s.anchor_virtual } else { 0 },
                    head_virtual: if head == s.head { s.head_virtual } else { 0 },
                    goal: None,
                    ..*s
                }
            })
            .collect();
        self.sel(cx).reselect(selections);
        let folds = focus.exclusions(self.doc(cx));
        if let Some(filter) = self.view_filter.as_mut() {
            filter.focus = focus;
            filter.version = Some(version);
            filter.published = true;
        }
        self.replace_folds(cx, |f, _| *f = folds);
        self.folds_changed(cx);
        self.scroll_to_cursor(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};

    fn editor<'a>(text: &str, cx: &'a mut TestAppContext) -> (Entity<Editor>, &'a mut VisualTestContext) {
        super::super::test_editor(text, cx, |_, _| {})
    }

    fn settle(e: &Entity<Editor>, cx: &mut VisualTestContext) {
        let started = Instant::now();
        loop {
            cx.run_until_parked();
            if e.read_with(cx, |e, _| e.filter_job.is_none()) {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(15));
            std::thread::yield_now();
        }
    }

    fn run(e: &Entity<Editor>, cx: &mut VisualTestContext, action: &dyn gpui::Action) {
        super::super::run(e, cx, action);
        settle(e, cx);
    }

    #[gpui::test]
    fn n14_clear_preserves_nested_inline_folds_source_identity_save_and_real_redo(cx: &mut TestAppContext) {
        let text = "head\n  first\n    nested\n  last\ntail\n";
        let (e, cx) = editor(text, cx);
        e.update_in(cx, |e, w, cx| {
            let end = e.doc(cx).len();
            e.sel(cx).set_cursor(end);
            e.edit(cx, |s| s.insert("!"));
            e.perform_action(&Undo, w, cx);
            e.fold_chars_now([4..30, 12..23, 1..3], cx);
            e.sel(cx).set_selection(31, 34);
        });
        let before = e.read_with(cx, |e, cx| {
            (
                e.read_state(cx).buffer_id(),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.doc(cx).unsaved(),
                e.folds.clone(),
            )
        });
        run(&e, cx, &NarrowToSelection);
        run(&e, cx, &ShowMatchingLines { pattern: "nested".into(), context: 0, ignore_case: false });
        run(&e, cx, &ClearFilter);
        e.read_with(cx, |e, cx| {
            assert_eq!(e.text(cx), text);
            assert_eq!(
                (
                    e.read_state(cx).buffer_id(),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.doc(cx).unsaved(),
                    e.folds.clone()
                ),
                before
            );
            assert!(e.view_filter.is_none());
        });
        run(&e, cx, &Redo);
        e.read_with(cx, |e, cx| assert_eq!(e.text(cx), format!("{text}!")));
        run(&e, cx, &Undo);
        e.read_with(cx, |e, cx| assert_eq!(e.text(cx), text));
    }

    #[gpui::test]
    fn n14_region_edits_map_focus_and_saved_folds_and_persist_only_prior_state(cx: &mut TestAppContext) {
        let (e, cx) = editor("before\nfocus\ntail\nmore\n", cx);
        e.update(cx, |e, cx| {
            e.fold_chars_now([17..21, 18..20], cx);
            e.sel(cx).set_selection(7, 12);
        });
        run(&e, cx, &NarrowToSelection);
        e.update(cx, |e, cx| {
            e.sel(cx).set_cursor(7);
            e.edit(cx, |s| s.insert("雪"));
            e.sel(cx).set_cursor(13);
            e.edit(cx, |s| s.insert("!"));
        });
        cx.run_until_parked();
        e.read_with(cx, |e, cx| {
            let filter = e.view_filter.as_ref().unwrap();
            assert!(filter.focus.contains(7) && filter.focus.contains(14));
            assert_eq!(e.capture_folds(cx).unwrap().restore(e.doc(cx)), filter.prior);
            assert_eq!(filter.prior.chars(), [19..23, 20..22]);
            assert_ne!(e.folds, filter.prior);
            assert_eq!(e.text(cx), "before\n雪focus!\ntail\nmore\n");
        });
        run(&e, cx, &ClearFilter);
        e.read_with(cx, |e, _| assert_eq!(e.folds.chars(), [19..23, 20..22]));
        run(&e, cx, &Undo);
        run(&e, cx, &Redo);
        e.read_with(cx, |e, cx| assert!(e.text(cx).contains("雪focus!")));
    }

    #[gpui::test]
    fn n14_typing_on_a_line_that_stops_matching_keeps_the_caret(cx: &mut TestAppContext) {
        let (e, cx) = editor("hit\nhit two\nmiss\n", cx);
        run(&e, cx, &ShowMatchingLines { pattern: "hit".into(), context: 0, ignore_case: false });
        e.update(cx, |e, cx| {
            e.sel(cx).set_selection(4, 7);
            e.edit(cx, |s| s.insert("D"));
            e.reveal_selections(cx);
        });
        cx.run_until_parked();
        settle(&e, cx);
        e.update(cx, |e, cx| e.edit(cx, |s| s.insert("ONE")));
        e.read_with(cx, |e, cx| assert_eq!(e.text(cx), "hit\nDONE two\nmiss\n"));
        // a caret moved while the scan after an edit runs keeps the filter
        e.update(cx, |e, cx| {
            e.edit(cx, |s| s.insert("!"));
            e.reveal_selections(cx);
            e.sel(cx).set_cursor(1);
        });
        settle(&e, cx);
        e.read_with(cx, |e, _| assert!(e.view_filter.is_some()));
    }

    #[gpui::test]
    fn n14_matching_lines_recompute_appearing_disappearing_matches_context_and_empty(cx: &mut TestAppContext) {
        let (e, cx) = editor("first\nhit\nmiss\nlast\n", cx);
        let matching = ShowMatchingLines { pattern: "hit".into(), context: 0, ignore_case: false };
        run(&e, cx, &matching);
        e.update(cx, |e, cx| {
            let end = e.doc(cx).line_end(1);
            e.sel(cx).set_selection(6, end);
            e.edit(cx, |s| s.insert("gone"));
        });
        cx.run_until_parked();
        settle(&e, cx);
        // the edited line no longer matches, but holds the caret: it stays shown, caret and all
        e.read_with(cx, |e, cx| {
            let f = &e.view_filter.as_ref().unwrap().focus;
            assert!(f.contains(e.doc(cx).line_start(1)) && !f.contains(0));
            assert_eq!(e.state.cursor(), 10);
            assert_eq!(e.folds.chars()[0], 0..e.doc(cx).line_start(1));
        });
        // Source-coordinate edits may legitimately touch hidden text, without deleting others.
        e.update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(vec![(11..15, "hit".into())])));
        cx.run_until_parked();
        settle(&e, cx);
        e.read_with(cx, |e, cx| {
            let f = e.view_filter.as_ref().unwrap();
            assert!(f.focus.contains(e.doc(cx).line_start(2)));
            assert_eq!(e.text(cx), "first\ngone\nhit\nlast\n");
        });
        run(&e, cx, &ShowMatchingLines { context: 1, ..matching });
        e.read_with(cx, |e, cx| {
            let f = &e.view_filter.as_ref().unwrap().focus;
            assert!(!f.contains(0));
            assert!(f.contains(e.doc(cx).line_start(1)) && f.contains(e.doc(cx).line_start(3)));
        });
        run(&e, cx, &ClearFilter);
        e.update(cx, |e, cx| e.set_text("", cx));
        run(&e, cx, &ShowMatchingLines { pattern: "x".into(), context: 0, ignore_case: false });
        e.read_with(cx, |e, cx| {
            assert_eq!(e.text(cx), "");
            assert_eq!(e.state.cursor(), 0);
            assert!(e.folds.is_empty());
        });
    }

    #[gpui::test]
    fn n14_region_copy_wrap_tabs_unicode_virtual_and_hidden_navigation_use_source(cx: &mut TestAppContext) {
        let text = "before\n雪\tlong words for wrapping\nafter\n";
        let (e, cx) = super::super::test_editor(text, cx, |e, cx| {
            e.wrap_override = Some(true);
            e.wrap_width_override = Some(12);
            e.sel(cx).set_virtual_space(true);
            let mut selection = Selection::new(7, 31);
            selection.rectangular = true;
            e.sel(cx).set_selections(vec![selection]);
        });
        run(&e, cx, &NarrowToSelection);
        run(&e, cx, &Copy);
        e.read_with(cx, |e, cx| {
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), e.doc(cx).rope.slice(7..31).to_string());
            assert!(e.state.selections[0].rectangular);
            assert_eq!(e.state.selections[0].range(), 7..31);
            let rows = e.rows();
            for at in [7, 8, 9, 15, 25, 30] {
                let point = rows.row_of(e.doc(cx), at);
                assert_eq!(rows.offset_at(e.doc(cx), point), at);
            }
        });
        e.update(cx, |e, cx| e.goto_line(1, cx));
        cx.run_until_parked();
        e.read_with(cx, |e, cx| {
            assert!(e.view_filter.is_none());
            assert_eq!(e.state.cursor(), 0);
            assert_eq!(e.text(cx), text);
        });
    }

    #[gpui::test]
    fn n14_fold_region_fallback_and_empty_selection_failure_keep_original_folds(cx: &mut TestAppContext) {
        let (e, cx) = editor("a\n  b\n    c\ntail\n", cx);
        e.update(cx, |e, cx| e.sel(cx).set_cursor(4));
        run(&e, cx, &NarrowToFold);
        e.read_with(cx, |e, _| {
            let f = &e.view_filter.as_ref().unwrap().focus;
            assert!(f.contains(2) && f.contains(10) && !f.contains(0));
        });
        run(&e, cx, &ClearFilter);
        e.update(cx, |e, cx| {
            e.fold_chars_now([1..11, 5..11], cx);
            e.sel(cx).set_cursor(1);
        });
        run(&e, cx, &NarrowToFold);
        run(&e, cx, &ClearFilter);
        e.read_with(cx, |e, _| assert_eq!(e.folds.chars(), [1..11, 5..11]));
        e.update(cx, |e, cx| e.sel(cx).set_cursor(0));
        run(&e, cx, &NarrowToSelection);
        e.read_with(cx, |e, _| assert!(e.view_filter.is_none()));
        e.update(cx, |e, cx| e.sel(cx).set_selection(0, 1));
        run(&e, cx, &NarrowToSelection);
        run(&e, cx, &UnfoldAll);
        e.read_with(cx, |e, _| {
            assert!(e.view_filter.is_none());
            assert!(e.folds.is_empty());
        });
    }

    #[gpui::test]
    fn n14_async_clear_switch_and_changed_source_reject_late_regex_work(cx: &mut TestAppContext) {
        let (e, cx) = editor("before\nhit\nafter\n", cx);
        e.update_in(cx, |e, w, cx| {
            e.show_matching_lines(&ShowMatchingLines { pattern: "hit".into(), context: 0, ignore_case: false }, w, cx);
            assert!(e.filter_job.is_some());
            e.clear_filter(&ClearFilter, w, cx);
            e.sel(cx).set_selection(11, 16);
            e.narrow_to_selection(&NarrowToSelection, w, cx);
        });
        settle(&e, cx);
        e.read_with(cx, |e, _| {
            assert!(matches!(e.view_filter.as_ref().unwrap().mode, Mode::Region));
            assert!(e.folds.hiding(8).is_some());
        });
        e.update_in(cx, |e, w, cx| {
            e.show_matching_lines(&ShowMatchingLines { pattern: "hit".into(), context: 0, ignore_case: false }, w, cx);
            e.edit(cx, |s| s.replace_ranges(vec![(7..10, "miss".into())]));
            e.reveal_selections(cx);
        });
        settle(&e, cx);
        e.read_with(cx, |e, cx| {
            assert_eq!(e.text(cx), "before\nmiss\nafter\n");
            assert!(e.view_filter.as_ref().unwrap().focus.is_empty());
        });
    }

    #[gpui::test]
    fn n14_active_local_composition_suspends_exclusions_and_delayed_reply_without_moving_marked_text(
        cx: &mut TestAppContext,
    ) {
        let (e, cx) = editor("before\nhit\nafter\n", cx);
        run(&e, cx, &ShowMatchingLines { pattern: "hit".into(), context: 0, ignore_case: false });
        e.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(Some(7..10), "仮", Some(0..1), w, cx);
            let before = (e.marked.clone(), e.state.selections.clone(), e.text(cx));
            e.reveal_selections(cx);
            assert_eq!((e.marked.clone(), e.state.selections.clone(), e.text(cx)), before);
            assert!(e.folds.is_empty());
        });
        settle(&e, cx);
        let before = e.read_with(cx, |e, cx| (e.marked.clone(), e.state.selections.clone(), e.text(cx)));
        run(&e, cx, &ClearFilter);
        assert_eq!(e.read_with(cx, |e, cx| (e.marked.clone(), e.state.selections.clone(), e.text(cx))), before);
        e.update_in(cx, |e, w, cx| e.unmark_text(w, cx));
        cx.run_until_parked();
        settle(&e, cx);
        // nothing matches now: only the line holding the caret stays shown
        e.read_with(cx, |e, cx| {
            let f = &e.view_filter.as_ref().unwrap().focus;
            assert!(!f.contains(0) && f.contains(e.doc(cx).line_start(1)));
            assert!(!f.contains(e.doc(cx).line_start(2) + 1), "nor the line after it");
        });
        e.update_in(cx, |e, w, cx| {
            e.show_matching_lines(&ShowMatchingLines { pattern: "仮".into(), context: 0, ignore_case: false }, w, cx);
            assert!(e.filter_pending());
            e.replace_and_mark_text_in_range(Some(7..8), "仮", Some(0..1), w, cx);
        });
        let before = e.read_with(cx, |e, cx| (e.marked.clone(), e.state.selections.clone(), e.text(cx)));
        settle(&e, cx);
        assert_eq!(e.read_with(cx, |e, cx| (e.marked.clone(), e.state.selections.clone(), e.text(cx))), before);
        e.read_with(cx, |e, _| assert!(e.folds.is_empty()));
    }
}

#[cfg(test)]
mod virtual_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn n14_pure_virtual_selection_refuses_narrowing_without_fabricating_source_focus(cx: &mut TestAppContext) {
        let (e, cx) = super::super::test_editor("before\nhit\nafter", cx, |e, cx| {
            e.sel(cx).set_virtual_space(true);
            e.sel(cx).set_selections(vec![Selection::from_points((10, 2), (10, 5))]);
        });
        let before = e.read_with(cx, |e, cx| (e.text(cx), e.doc(cx).version, e.state.selections.clone()));
        super::super::run(&e, cx, &NarrowToSelection);
        e.read_with(cx, |e, cx| {
            assert!(e.view_filter.is_none(), "virtual columns cannot become a physical focus range");
            assert_eq!((e.text(cx), e.doc(cx).version, e.state.selections.clone()), before);
            assert!(e.folds.is_empty());
        });
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn n14_invalid_regex_preserves_active_filter_and_scan_failure_restores_saved_folds(cx: &mut TestAppContext) {
        let text = format!("head\n{}c\ntail\n", "a".repeat(40));
        let (e, cx) = super::super::test_editor(&text, cx, |e, cx| {
            e.fold_chars_now(std::iter::once(1..3), cx);
            e.sel(cx).set_selection(5, 46);
        });
        super::super::run(&e, cx, &NarrowToSelection);
        let before = e.read_with(cx, |e, cx| (e.folds.clone(), e.doc(cx).version, e.doc(cx).save_point(), e.text(cx)));
        super::super::run(&e, cx, &ShowMatchingLines { pattern: "[".into(), context: 0, ignore_case: false });
        assert_eq!(
            e.read_with(cx, |e, cx| (e.folds.clone(), e.doc(cx).version, e.doc(cx).save_point(), e.text(cx))),
            before
        );
        super::super::run(&e, cx, &ShowMatchingLines { pattern: r"^(a|a)*\1b".into(), context: 0, ignore_case: false });
        let started = Instant::now();
        loop {
            cx.run_until_parked();
            if e.read_with(cx, |e, _| !e.filter_pending()) {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(15));
            std::thread::yield_now();
        }
        e.read_with(cx, |e, cx| {
            assert!(e.view_filter.is_none());
            assert_eq!(e.folds.chars(), std::slice::from_ref(&(1..3)));
            assert_eq!(e.text(cx), text);
        });
    }
}

#[cfg(test)]
mod freshness_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn n14_pending_regex_rejects_away_back_selection_changes(cx: &mut TestAppContext) {
        let (e, cx) = super::super::test_editor("before\nhit\nafter\n", cx, |e, cx| e.sel(cx).set_selection(7, 10));
        e.update_in(cx, |e, w, cx| {
            e.show_matching_lines(&ShowMatchingLines { pattern: "hit".into(), context: 0, ignore_case: false }, w, cx);
            e.sel(cx).set_cursor(0);
            e.sel(cx).set_selection(7, 10);
        });
        let started = Instant::now();
        loop {
            cx.run_until_parked();
            if e.read_with(cx, |e, _| !e.filter_pending()) {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(15));
            std::thread::yield_now();
        }
        e.read_with(cx, |e, _| {
            assert!(e.view_filter.is_none(), "a delayed filter must not publish after selection moved away and back");
            assert_eq!(e.state.selections[0].range(), 7..10);
        });
    }

    #[gpui::test]
    fn n14_hidden_source_navigation_keeps_destination_inside_an_original_fold(cx: &mut TestAppContext) {
        let (e, cx) = super::super::test_editor("head\n  body\ntail\n", cx, |e, cx| {
            e.fold_chars_now(std::iter::once(4..11), cx);
            e.sel(cx).set_selection(12, 16);
        });
        super::super::run(&e, cx, &NarrowToSelection);
        e.update(cx, |e, cx| e.goto_line(2, cx));
        cx.run_until_parked();
        e.read_with(cx, |e, _| {
            assert_eq!(e.state.cursor(), 5, "clearing must preserve the actual source navigation destination");
            assert!(e.view_filter.is_none());
            assert!(e.folds.is_empty());
        });
    }
}
