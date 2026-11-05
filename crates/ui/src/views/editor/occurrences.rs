//! Per-view passive matches; Find and manual highlight state are never borrowed mutably.
use super::*;
use crate::editing::text::{find_in_rope_until, FindOpts};
use orlo_editor::search::pattern::{Deadline, SearchError, BACKGROUND_SECONDS};

/// Bound automatic query copying to at most 4 KiB of UTF-8, before starting a worker.
const MAX_QUERY_CHARS: usize = 1024;

pub(super) struct Occurrences {
    source: Rc<Source>,
    pub ranges: Vec<Range<usize>>,
    job: Option<HtmlJob>,
}

struct Source {
    document: Entity<OpenDocument>,
    rope: ropey::Rope,
    version: u64,
    selection: Selection,
    generation: u64,
}

impl Source {
    fn current(&self, e: &Editor, cx: &App) -> bool {
        e.occurrence_selection(cx) == Some(self.selection)
            && self.document == e.document
            && self.version == e.doc(cx).version
            && self.rope.is_instance(&e.doc(cx).rope)
            && self.generation == e.read_state(cx).selection_generation()
    }
}

fn matching(rope: &ropey::Rope, selection: Range<usize>, stop: &AtomicBool) -> Result<Vec<Range<usize>>, SearchError> {
    let deadline = Deadline::new(BACKGROUND_SECONDS);
    let query = rope.slice(selection.clone()).to_string();
    let opts = FindOpts { case_sensitive: true, ..Default::default() };
    let mut ranges =
        find_in_rope_until(rope, &query, &opts, None, &|| stop.load(Ordering::Relaxed) || deadline.passed())
            .map_err(|error| deadline.explain(error))?;
    ranges.retain(|r| *r != selection);
    Ok(ranges)
}

impl Editor {
    #[cfg(test)]
    pub(crate) fn passive_occurrence_ranges(&self) -> &[Range<usize>] {
        self.occurrences.as_ref().map_or(&[], |o| o.ranges.as_slice())
    }

    fn occurrence_selection(&self, cx: &App) -> Option<Selection> {
        if !self.view.selection_occurrences || self.single_line || !self.marked.is_empty() {
            return None;
        }
        let [selection] = self.state.selections.as_slice() else { return None };
        let doc = self.doc(cx);
        (self.state.revision() == doc.version
            && selection.start() < selection.end()
            && selection.range().len() <= MAX_QUERY_CHARS
            && !selection.has_virtual()
            && doc.point(selection.start()).0 == doc.point(selection.end()).0)
            .then_some(*selection)
    }

    pub(crate) fn clear_stale_occurrences(&mut self, cx: &App) {
        if self.occurrences.as_ref().is_some_and(|kept| !kept.source.current(self, cx)) {
            self.occurrences = None;
        }
    }

    pub(super) fn refresh_occurrences(&mut self, cx: &mut Context<Self>) {
        self.clear_stale_occurrences(cx);
        if self.occurrences.is_some() {
            return;
        }
        let Some(selection) = self.occurrence_selection(cx) else { return };
        let source = Rc::new(Source {
            document: self.document.clone(),
            rope: self.doc(cx).rope.clone(),
            version: self.doc(cx).version,
            selection,
            generation: self.read_state(cx).selection_generation(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (rope, cancelled) = (source.rope.clone(), stop.clone());
        let found = cx.background_spawn(async move { matching(&rope, selection.range(), &cancelled) });
        let requested = source.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = found.await;
            this.update(cx, |e, cx| e.finish_occurrences(&requested, result, cx)).ok();
        });
        self.occurrences =
            Some(Occurrences { source, ranges: Vec::new(), job: Some(HtmlJob { _task: task, _stop: Cancel(stop) }) });
    }

    fn finish_occurrences(
        &mut self,
        source: &Rc<Source>,
        result: Result<Vec<Range<usize>>, SearchError>,
        cx: &mut Context<Self>,
    ) {
        if !self.occurrences.as_ref().is_some_and(|kept| Rc::ptr_eq(&kept.source, source)) {
            return;
        }
        if !source.current(self, cx) {
            self.occurrences = None;
        } else if let Some(kept) = &mut self.occurrences {
            // A failed bounded search stays empty until the source changes; no frame retry loop.
            kept.ranges = result.unwrap_or_default();
            kept.job = None;
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::single_range_in_vec_init)]
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};

    fn editor(cx: &mut TestAppContext) -> (Entity<Editor>, &mut VisualTestContext) {
        test_editor("é😀 é😀\né😀", cx, |e, cx| {
            e.view.selection_occurrences = true;
            e.sel(cx).set_selection(0, 2);
        })
    }

    fn ranges(e: &Entity<Editor>, cx: &VisualTestContext) -> Vec<Range<usize>> {
        e.read_with(cx, |e, _| e.occurrences.as_ref().map_or_else(Vec::new, |o| o.ranges.clone()))
    }

    // Hold a real background matcher result before delivering it through the production receiver.
    fn delayed(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (Rc<Source>, Vec<Range<usize>>) {
        let source = e.read_with(cx, |e, _| e.occurrences.as_ref().unwrap().source.clone());
        let (rope, selection) = (source.rope.clone(), source.selection.range());
        let work = cx.executor().spawn(async move { matching(&rope, selection, &AtomicBool::new(false)) });
        (source, cx.executor().block(work).unwrap())
    }

    #[test]
    fn passive_occurrences_reuse_literal_unicode_case_nonoverlap_and_cancellation_rules() {
        let stop = AtomicBool::new(false);
        assert_eq!(matching(&ropey::Rope::from_str("é😀 é😀 É😀 é😀"), 0..2, &stop), Ok(vec![3..5, 9..11]));
        assert_eq!(matching(&ropey::Rope::from_str("a.a aXa a.a"), 0..3, &stop), Ok(vec![8..11]));
        assert_eq!(matching(&ropey::Rope::from_str("aaaaa"), 0..2, &stop), Ok(vec![2..4]));
        stop.store(true, Ordering::Relaxed);
        assert_eq!(matching(&ropey::Rope::from_str("é😀 é😀"), 0..2, &stop), Err(SearchError::Cancelled));
    }

    #[gpui::test]
    fn passive_occurrences_query_admission_caps_scalars_before_copying_or_starting_a_job(cx: &mut TestAppContext) {
        let text = "😀".repeat(2048);
        let (e, cx) = test_editor(&text, cx, |e, cx| {
            e.view.selection_occurrences = true;
            e.sel(cx).set_selection(0, 1024);
        });
        assert_eq!(ranges(&e, cx), [1024..2048], "1,024 scalars, including four-byte characters, remain eligible");
        e.update(cx, |e, cx| {
            e.sel(cx).set_selection(0, 1025);
            e.refresh_occurrences(cx);
            assert!(e.occurrences.is_none(), "oversize selection clears the cache and starts no worker");
            assert_eq!(e.state.selections, [Selection::new(0, 1025)]);
            assert_eq!(e.doc(cx).version, 0);
            cx.notify();
        });
        cx.run_until_parked();
        e.read_with(cx, |e, cx| {
            assert!(e.occurrences.is_none());
            assert_eq!(e.text(cx), text);
        });
    }

    #[gpui::test]
    fn passive_occurrences_dense_huge_line_visits_only_horizontally_clipped_ranges(cx: &mut TestAppContext) {
        let text = "a ".repeat(200_000);
        let (e, cx) = test_editor(&text, cx, |e, cx| {
            e.view.selection_occurrences = true;
            e.view.minimap = false;
            e.sel(cx).set_selection(0, 1);
        });
        let (source, space) = e.read_with(cx, |e, _| {
            assert_eq!(e.passive_occurrence_ranges().len(), 199_999);
            (e.occurrences.as_ref().unwrap().source.clone(), f32::from(e.layout.as_ref().unwrap().space_w))
        });
        for column in [0., 200_000., 0.] {
            e.update(cx, |e, cx| {
                e.scroll_left = space * column;
                cx.notify();
            });
            cx.run_until_parked();
            e.read_with(cx, |e, cx| {
                let drawn = &e.layout.as_ref().unwrap().drawn;
                assert!(!drawn.occurrences.is_empty());
                assert!(
                    drawn.occurrence_visits < 1024,
                    "visited {} of 199,999 retained matches",
                    drawn.occurrence_visits
                );
                assert!(drawn.occurrences.len() < 1024);
                if column > 0. {
                    assert!(drawn.occurrences[0].0.start > 190_000, "the far horizontal viewport is being painted");
                }
                assert!(Rc::ptr_eq(&source, &e.occurrences.as_ref().unwrap().source));
                assert_eq!(e.display_rows(cx), 1);
                assert_eq!(e.doc(cx).version, 0);
            });
        }
    }

    #[gpui::test]
    fn passive_occurrences_clear_empty_multiple_multiline_virtual_disabled_and_field_views(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx);
        assert_eq!(ranges(&e, cx), [3..5, 6..8]);
        for selections in [
            vec![Selection::caret(0)],
            vec![Selection::new(0, 2), Selection::new(3, 5)],
            vec![Selection::new(0, 7)],
            vec![Selection::from_points((8, 1), (8, 3))],
        ] {
            e.update(cx, |e, cx| {
                e.sel(cx).set_virtual_space(true);
                e.sel(cx).set_selections(selections);
                e.clear_stale_occurrences(cx);
                assert!(e.occurrences.is_none(), "clear before another frame");
                cx.notify();
            });
            cx.run_until_parked();
            assert!(ranges(&e, cx).is_empty());
            e.update(cx, |e, cx| {
                e.sel(cx).set_selection(0, 2);
                cx.notify();
            });
            cx.run_until_parked();
            assert_eq!(ranges(&e, cx), [3..5, 6..8]);
        }
        for field in [false, true] {
            e.update(cx, |e, cx| {
                e.single_line = field;
                e.view.selection_occurrences = field;
                e.refresh_occurrences(cx);
                assert!(e.occurrences.is_none());
            });
        }
    }

    #[gpui::test]
    fn passive_occurrences_delayed_edit_result_cannot_reappear_or_change_undo(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx);
        let (source, result) = delayed(&e, cx);
        e.update(cx, |e, cx| {
            e.edit(cx, |s| s.replace_ranges(vec![(3..5, "other".into())]));
            assert!(e.occurrences.is_none());
            e.finish_occurrences(&source, Ok(result), cx);
            assert!(e.occurrences.is_none());
        });
        cx.run_until_parked();
        assert_eq!(ranges(&e, cx), [9..11]);
        run(&e, cx, &Undo);
        e.read_with(cx, |e, cx| assert_eq!(e.text(cx), "é😀 é😀\né😀"));
        assert_eq!(ranges(&e, cx), [3..5, 6..8]);
    }

    #[gpui::test]
    fn passive_occurrences_delayed_away_back_generation_and_disabled_policy_are_rejected(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx);
        let (source, result) = delayed(&e, cx);
        e.update(cx, |e, cx| {
            e.sel(cx).set_cursor(7);
            e.sel(cx).set_selection(0, 2);
            // Deliberately deliver before render/invalidation: exact offsets alone are insufficient.
            e.finish_occurrences(&source, Ok(result), cx);
            assert!(e.occurrences.is_none());
        });
        cx.run_until_parked();
        assert_eq!(ranges(&e, cx), [3..5, 6..8]);
        let (source, result) = delayed(&e, cx);
        e.update(cx, |e, cx| {
            e.view.selection_occurrences = false;
            e.clear_stale_occurrences(cx);
            e.view.selection_occurrences = true;
            e.refresh_occurrences(cx);
            e.finish_occurrences(&source, Ok(result), cx);
            assert!(e.occurrences.as_ref().unwrap().ranges.is_empty(), "old reply cannot replace the new request");
        });
        cx.run_until_parked();
        assert_eq!(ranges(&e, cx), [3..5, 6..8]);
    }

    #[gpui::test]
    fn passive_occurrences_shared_view_edits_and_native_ime_invalidate_delayed_results(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx);
        let peer = e.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
        let (source, result) = delayed(&e, cx);
        peer.update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(vec![(3..5, "no".into())])));
        e.update(cx, |e, cx| {
            e.finish_occurrences(&source, Ok(result), cx);
            assert_ne!(e.doc(cx).version, source.version);
            assert!(!e.passive_occurrence_ranges().contains(&(3..5)), "old peer text must never render");
        });
        cx.run_until_parked();
        assert_eq!(ranges(&e, cx), [6..8]);
        let (source, result) = delayed(&e, cx);
        e.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(Some(0..3), "é😀", Some(0..3), w, cx);
            let before = (e.marked.clone(), e.state.selections.clone(), e.text(cx));
            e.finish_occurrences(&source, Ok(result), cx);
            assert!(e.occurrences.is_none());
            assert_eq!((e.marked.clone(), e.state.selections.clone(), e.text(cx)), before);
        });
        cx.run_until_parked();
        assert!(ranges(&e, cx).is_empty());
    }

    #[gpui::test]
    fn passive_occurrences_are_per_view_and_work_in_read_only_and_viewer_documents(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx);
        let peer = e.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
        peer.update(cx, |e, cx| {
            e.sel(cx).set_selection(3, 5);
            e.refresh_occurrences(cx);
        });
        cx.run_until_parked();
        assert_eq!(ranges(&e, cx), [3..5, 6..8]);
        assert_eq!(ranges(&peer, cx), [0..2, 6..8]);
        peer.update(cx, |e, cx| {
            e.sel(cx).set_selection(1, 2);
            e.refresh_occurrences(cx);
        });
        cx.run_until_parked();
        assert_eq!(ranges(&peer, cx), [4..5, 7..8]);
        assert_eq!(ranges(&e, cx), [3..5, 6..8]);
        e.update(cx, |e, cx| {
            e.update_doc(cx, |d| d.read_only = true);
            e.document.update(cx, |d, cx| d.set_viewer_mode(true, cx));
            e.input_enabled = false;
        });
        cx.run_until_parked();
        assert_eq!(ranges(&e, cx), [3..5, 6..8]);
        let source = e.read_with(cx, |e, _| e.occurrences.as_ref().unwrap().source.clone());
        e.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        e.read_with(cx, |e, _| {
            assert!(
                Rc::ptr_eq(&source, &e.occurrences.as_ref().unwrap().source),
                "unchanged frames reuse the prepared cache"
            );
        });
    }

    #[gpui::test]
    fn passive_occurrences_cancel_pending_work_and_cache_bounded_failures_without_frame_retries(
        cx: &mut TestAppContext,
    ) {
        let (e, cx) = editor(cx);
        e.update(cx, |e, cx| {
            e.sel(cx).set_selection(3, 5);
            e.refresh_occurrences(cx);
            let kept = e.occurrences.as_ref().unwrap();
            let stop = kept.job.as_ref().unwrap()._stop.0.clone();
            assert!(!stop.load(Ordering::Relaxed));
            e.sel(cx).set_cursor(0);
            e.clear_stale_occurrences(cx);
            assert!(stop.load(Ordering::Relaxed), "dropping a superseded request signals the actual worker");
            e.sel(cx).set_selection(0, 2);
            e.refresh_occurrences(cx);
            let source = e.occurrences.as_ref().unwrap().source.clone();
            e.finish_occurrences(&source, Err(SearchError::TimedOut(BACKGROUND_SECONDS)), cx);
            e.refresh_occurrences(cx);
            let kept = e.occurrences.as_ref().unwrap();
            assert!(kept.ranges.is_empty() && kept.job.is_none());
            assert!(Rc::ptr_eq(&kept.source, &source));
        });
    }
}
