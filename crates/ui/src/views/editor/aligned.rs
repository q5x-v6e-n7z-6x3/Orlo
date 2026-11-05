//! Two read-only views as one side-by-side diff (Compare Files, the Git panel): filler rows keep
//! each unchanged line level with its partner, the two scroll as one by display row, and a fold
//! on either side folds the matching text on the other.

use super::{ComparisonHighlights, Editor, EditorEvent};
use crate::editing::display::align::{self, Hunk};
use crate::editing::display::filler::Fillers;
use crate::editing::fold::Folds;
use crate::files::comparison::Pair;
use gpui::{Context, Entity, Subscription};
use std::sync::atomic::{AtomicBool, Ordering};

/// The hunks of a comparison, as alignment takes them.
pub fn hunks_of(pair: &Pair) -> Vec<Hunk> {
    (pair.hunks.iter())
        .map(|h| Hunk {
            lines: [h.lines.old_lines.clone(), h.lines.new_lines.clone()],
            chars: [h.lines.old_chars.clone(), h.lines.new_chars.clone()],
        })
        .collect()
}

/// Each side's red or green line and within-line backgrounds for a comparison; `None` once
/// `stop` is set.
pub fn highlights_of(pair: &Pair, stop: &AtomicBool) -> Option<[ComparisonHighlights; 2]> {
    let mut highlights =
        std::array::from_fn(|side| ComparisonHighlights { lines: Vec::new(), inline: Vec::new(), added: side == 1 });
    for (side, marks) in highlights.iter_mut().enumerate() {
        for h in &pair.hunks {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            marks.lines.push(match side {
                0 => h.lines.old_chars.clone(),
                _ => h.lines.new_chars.clone(),
            });
            marks.inline.extend(h.inline.iter().map(|inline| match side {
                0 => inline.old_chars.clone(),
                _ => inline.new_chars.clone(),
            }));
        }
    }
    Some(highlights)
}

/// Keeps two views (0 the old side, 1 the new) aligned for as long as it lives.
pub struct AlignedPair {
    viewers: [Entity<Editor>; 2],
    hunks: Vec<Hunk>,
    _subscriptions: Vec<Subscription>,
}

impl AlignedPair {
    pub fn new(viewers: [Entity<Editor>; 2], hunks: Vec<Hunk>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        for (side, viewer) in viewers.iter().enumerate() {
            viewer.update(cx, |e, _| e.aligned = true);
            subscriptions.push(cx.subscribe(viewer, move |pair, _, event, cx| match event {
                EditorEvent::RowsChanged => pair.realign(cx),
                EditorEvent::Scrolled => pair.sync_scroll(side, cx),
                EditorEvent::FoldsChanged => pair.mirror_folds(side, cx),
                _ => {}
            }));
        }
        let mut pair = Self { viewers, hunks, _subscriptions: subscriptions };
        pair.realign(cx);
        pair
    }

    pub fn viewers(&self) -> &[Entity<Editor>; 2] {
        &self.viewers
    }

    /// Place both sides' filler rows again from their rows as they are now. While a side's
    /// lines are still being wrapped at a new width, a slice a frame, its rows change every
    /// frame: the filler rows wait for it to finish, which tells the pair again.
    fn realign(&mut self, cx: &mut Context<Self>) {
        let fillers = {
            let [a, b] = [self.viewers[0].read(cx), self.viewers[1].read(cx)];
            if !(a.wrap_settled() && b.wrap_settled()) {
                return;
            }
            let (da, db) = (a.doc(cx), b.doc(cx));
            let lines = [da.line_count(), db.line_count()];
            let every_line = a.wrapped_now() || b.wrapped_now();
            let anchors = align::anchors(&self.hunks, lines, every_line);
            let (ra, rb) = (a.unpadded_rows(), b.unpadded_rows());
            let [fa, fb] = align::fillers(&anchors, [(&ra, da), (&rb, db)]);
            [Fillers::new(da, fa), Fillers::new(db, fb)]
        };
        for (viewer, f) in self.viewers.iter().zip(fillers) {
            viewer.update(cx, |e, cx| e.set_fillers(f, cx));
        }
        // a side that kept its own text on top through a new width may have moved: the new
        // side goes back to the old side's row
        self.sync_scroll(0, cx);
    }

    /// Side `side` scrolled: the other goes to the same display row. It is not told back.
    fn sync_scroll(&mut self, side: usize, cx: &mut Context<Self>) {
        let scroll = self.viewers[side].read(cx).scroll_rows();
        self.viewers[1 - side].update(cx, |e, cx| {
            e.follow_scroll_rows(scroll);
            cx.notify();
        });
        self.viewers[side].update(cx, |e, _| e.scroll_synced());
    }

    /// Side `side`'s folds changed: the other side folds the text they map to, in place of its
    /// own folds, and the filler rows are placed again.
    fn mirror_folds(&mut self, side: usize, cx: &mut Context<Self>) {
        let ranges = self.viewers[side].read(cx).folds().chars();
        let hunks = &self.hunks;
        let mapped: Vec<std::ops::Range<usize>> = {
            let docs = self.viewers.each_ref().map(|v| v.read(cx).doc(cx));
            (ranges.into_iter())
                .map(|r| {
                    let map = |o, after| align::map_offset(hunks, docs, side, o, after);
                    map(r.start, false)..map(r.end, true)
                })
                .collect()
        };
        self.viewers[1 - side].update(cx, |e, cx| {
            let mut folds = Folds::default();
            folds.fold_many(e.doc(cx), mapped);
            e.set_folds_quietly(folds, cx);
        });
        self.realign(cx);
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::editing::text::{EditorState, Rows};
    use crate::editor::{run, Fold, Unfold};
    use gpui::{
        div, point, prelude::*, px, size, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
        VisualTestContext,
    };

    /// Two views side by side, `widths` wide, as a comparison shows them.
    struct Panes {
        sides: [Entity<Editor>; 2],
        widths: [f32; 2],
        pair: Option<Entity<AlignedPair>>,
    }

    impl Render for Panes {
        fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
            let side = |i: usize| div().w(px(self.widths[i])).h_full().child(self.sides[i].clone());
            div().size_full().flex().child(side(0)).child(side(1))
        }
    }

    fn hunks(left: &str, right: &str) -> Vec<Hunk> {
        use crate::files::comparison::{compare, Format, Text};
        let text = |t: &str| Text {
            endings: None,
            rope: ropey::Rope::from_str(t),
            format: Format { line_ending: "\n", bom: false, encoding: Default::default() },
        };
        let stop = std::sync::atomic::AtomicBool::new(false);
        hunks_of(&compare(&text(left), &text(right), &stop).unwrap())
    }

    /// Two read-only views of `texts` in a 600 × 300 window, `widths` wide, wrapped or not,
    /// aligned and drawn.
    fn panes<'a>(
        texts: [&str; 2],
        widths: [f32; 2],
        wrap: [bool; 2],
        cx: &'a mut TestAppContext,
    ) -> (Entity<Panes>, [Entity<Editor>; 2], &'a mut VisualTestContext) {
        let texts = texts.map(str::to_string);
        let (panes, cx) = cx.add_window_view(move |_, cx| {
            let sides = std::array::from_fn(|i| {
                cx.new(|cx| {
                    let mut s = EditorState::from_text(&texts[i]);
                    s.doc.read_only = true;
                    let mut e = Editor::new(s, false, cx);
                    e.input_enabled = false;
                    e.view.minimap = false;
                    e.wrap_override = Some(wrap[i]);
                    e
                })
            });
            Panes { sides, widths, pair: None }
        });
        cx.simulate_resize(size(px(600.), px(300.)));
        let sides = panes.read_with(cx, |p, _| p.sides.clone());
        let hs = sides.each_ref().map(|e| e.read_with(cx, |e, cx| e.text(cx)));
        let hs = hunks(&hs[0], &hs[1]);
        panes.update(cx, |p, cx| {
            let sides = p.sides.clone();
            p.pair = Some(cx.new(|cx| AlignedPair::new(sides, hs, cx)));
        });
        cx.run_until_parked();
        (panes, sides, cx)
    }

    /// Alignment deliberately waits for both frame-sliced wrap maps to finish.
    fn wait_for_wraps(sides: &[Entity<Editor>; 2], cx: &mut VisualTestContext) {
        crate::workspace::ui_tests::settle_until(cx, |cx| {
            let settled = sides.iter().all(|e| e.read_with(cx, |e, _| e.wrap_settled()));
            if !settled {
                // GPUI's test platform has no display-link callback. Drive the next real
                // frame so request_animation_frame work gets its normal wrapping slice.
                cx.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear();
                });
            }
            settled
        });
    }

    /// Each side's top display row.
    fn tops(sides: &[Entity<Editor>; 2], cx: &mut VisualTestContext) -> [f64; 2] {
        sides.each_ref().map(|e| e.read_with(cx, |e, _| e.scroll_rows().0))
    }

    /// Each side's rows, and the display row of each line on it.
    fn rows(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (usize, Vec<usize>) {
        e.read_with(cx, |e, cx| {
            let (rows, doc) = (e.rows(), e.doc(cx));
            let lines = (0..doc.line_count()).map(|l| rows.first_row(doc, l).0);
            (rows.rows(doc), lines.collect())
        })
    }

    fn level(sides: &[Entity<Editor>; 2], pairs: &[(usize, usize)], cx: &mut VisualTestContext) {
        let (a, b) = (rows(&sides[0], cx), rows(&sides[1], cx));
        assert_eq!(a.0, b.0, "as many rows on both sides");
        for &(l, r) in pairs {
            assert_eq!(a.1[l], b.1[r], "line {l} and its partner {r}: {a:?} {b:?}");
        }
    }

    const OLD: &str = "a\nb\nc\nd\ne\nf\ng\n";
    const NEW: &str = "a\nB1\nB2\nB3\nc\nd\nf\ng\nh\n";
    /// The unchanged pairs of `OLD` and `NEW`.
    const SAME: [(usize, usize); 6] = [(0, 0), (2, 4), (3, 5), (5, 6), (6, 7), (7, 9)];

    #[gpui::test]
    fn a_pair_keeps_unchanged_lines_level(cx: &mut TestAppContext) {
        let (_panes, sides, cx) = panes([OLD, NEW], [300., 300.], [false, false], cx);
        level(&sides, &SAME, cx);
        // the old side's padding is drawn as filler rows
        sides[0].read_with(cx, |e, _| {
            assert_eq!(e.fillers().blocks(), [(2, 2), (7, 1)]);
            assert_eq!(e.layout.as_ref().unwrap().drawn.fillers, [2, 3, 9]);
        });
    }

    #[gpui::test]
    fn wrapped_sides_of_different_widths_stay_level(cx: &mut TestAppContext) {
        let long = "word ".repeat(30);
        let (old, new) = (format!("{long}\nb\n{long}\nd\n"), format!("{long}\nB\n{long}\nd\n"));
        // the old side wraps in a narrower pane than the new; the new does not wrap at all
        let (_panes, sides, cx) = panes([&old, &new], [250., 350.], [true, false], cx);
        level(&sides, &[(0, 0), (2, 2), (3, 3)], cx);
        let (a, b) = (rows(&sides[0], cx), rows(&sides[1], cx));
        assert!(a.1[1] > 1, "the long line wraps on the old side: {a:?}");
        assert_eq!(b.1[1], a.1[1], "the new side is padded after it");
        // both wrapped, each at its own width
        let (panes, sides, cx) = panes([&old, &new], [250., 350.], [true, true], cx);
        level(&sides, &[(0, 0), (2, 2), (3, 3)], cx);
        // a pane made narrower wraps more; the other side is padded to match
        panes.update(cx, |p, cx| {
            p.widths = [350., 200.];
            cx.notify();
        });
        cx.run_until_parked();
        level(&sides, &[(0, 0), (2, 2), (3, 3)], cx);
    }

    #[gpui::test]
    fn scrolling_either_side_takes_the_other_to_the_same_row_and_no_further(cx: &mut TestAppContext) {
        let old: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let new: String = (0..200)
            .map(|i| match i % 40 {
                7 => format!("line {i}\nadded\nadded\n"),
                _ => format!("line {i}\n"),
            })
            .collect();
        let (_panes, sides, cx) = panes([&old, &new], [300., 300.], [false, false], cx);
        let told = std::rc::Rc::new(std::cell::Cell::new([0; 2]));
        let _subs: Vec<_> = (0..2)
            .map(|i| {
                let told = told.clone();
                cx.update(|_, cx| {
                    cx.subscribe(&sides[i], move |_, e: &EditorEvent, _| {
                        if matches!(e, EditorEvent::Scrolled) {
                            let mut t = told.get();
                            t[i] += 1;
                            told.set(t);
                        }
                    })
                })
            })
            .collect();
        let wheel = |x: f32, dy: f32, cx: &mut VisualTestContext| {
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(x), px(100.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                modifiers: Modifiers::none(),
                touch_phase: TouchPhase::Moved,
            });
            cx.run_until_parked();
        };
        // the old side scrolls 100 rows' worth: past several hunks, so lines differ by now
        wheel(100., -1500., cx);
        let [a, b] = tops(&sides, cx);
        assert_eq!(a, 100.);
        assert_eq!(b, a, "the same display row, not the same line");
        assert_eq!(told.get(), [1, 0], "the new side does not tell it back");
        let top_lines = sides.each_ref().map(|e| e.read_with(cx, |e, cx| e.scroll_position(cx).0));
        assert_ne!(top_lines[0], top_lines[1], "the new side has more lines above");
        // and the other way round
        wheel(400., 750., cx);
        assert_eq!(tops(&sides, cx), [50., 50.]);
        assert_eq!(told.get(), [1, 1]);
    }

    #[gpui::test]
    fn filler_rows_above_the_first_line_open_both_sides_at_one_row(cx: &mut TestAppContext) {
        // the new side starts with 30 lines more: the old side is padded above its first line
        let old: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let new = format!("{}{old}", "added\n".repeat(30));
        let (_panes, sides, cx) = panes([&old, &new], [300., 300.], [false, false], cx);
        assert_eq!(tops(&sides, cx), [0., 0.], "both from the top: the padding shows beside the added lines");
        level(&sides, &[(0, 30), (100, 130)], cx);
        // and a width change that wraps one side keeps them at one row too
        sides[0].update(cx, |e, cx| {
            e.follow_scroll_rows((40., None, 0.));
            cx.notify();
        });
        sides[1].update(cx, |e, cx| {
            e.follow_scroll_rows((40., None, 0.));
            e.wrap_override = Some(true);
            e.wrap_width_override = Some(4);
            cx.notify();
        });
        wait_for_wraps(&sides, cx);
        let [a, b] = tops(&sides, cx);
        assert_eq!(a, b);
    }

    #[gpui::test]
    fn wrap_resize_waits_for_complete_maps_before_asserting_alignment(cx: &mut TestAppContext) {
        // Rewrapping is sliced at the production frame deadline. Enough lines make the
        // pending-map phase observable independently of machine load in the small fixture.
        let old: String = (0..100_000).map(|i| format!("line {i}\n")).collect();
        let new = format!("{}{old}", "added\n".repeat(30));
        let (_panes, sides, cx) = panes([&old, &new], [300., 300.], [false, false], cx);
        sides[0].update(cx, |e, cx| {
            e.follow_scroll_rows((40., None, 0.));
            cx.notify();
        });
        sides[1].update(cx, |e, cx| {
            e.follow_scroll_rows((40., None, 0.));
            e.wrap_override = Some(true);
            e.wrap_width_override = Some(4);
            cx.notify();
        });
        wait_for_wraps(&sides, cx);
        assert!(
            sides.iter().all(|e| e.read_with(cx, |e, _| e.wrap_settled())),
            "both maps must finish before checking comparison alignment"
        );
        let [a, b] = tops(&sides, cx);
        assert_eq!(a, b);
        level(&sides, &[(0, 30), (100, 130), (99_999, 100_029)], cx);
    }

    #[gpui::test]
    fn a_fold_on_either_side_folds_the_matching_text_on_the_other(cx: &mut TestAppContext) {
        let old = "fn a() {\n    one\n    two\n    three\n}\nend\n";
        let new = "fn a() {\n    one\n    TWO\n    2\n    three\n}\nend\n";
        let (_panes, sides, cx) = panes([old, new], [300., 300.], [false, false], cx);
        // Fold on the old side's header folds its block, and the new side's block with it
        sides[0].update(cx, |e, cx| e.sel(cx).set_cursor(2));
        run(&sides[0], cx, &Fold);
        let hidden = |cx: &mut VisualTestContext| sides.each_ref().map(|e| e.read_with(cx, |e, cx| e.hidden_lines(cx)));
        assert_eq!(hidden(cx), [vec![1..4], vec![1..5]]);
        level(&sides, &[(0, 0), (4, 5), (5, 6)], cx);
        assert_eq!(rows(&sides[0], cx).0, 4, "no filler rows needed: the hunk is folded");
        // Unfold on the new side opens both
        sides[1].update(cx, |e, cx| e.sel(cx).set_cursor(2));
        run(&sides[1], cx, &Unfold);
        assert!(hidden(cx).iter().all(Vec::is_empty), "both opened");
        level(&sides, &[(0, 0), (1, 1), (3, 4), (4, 5), (5, 6)], cx);
        // a fold inside an unchanged line, on the new side: the old side folds the same text
        let at = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, cx| {
                let doc = e.doc(cx);
                doc.line_start(doc.line_count() - 2)
            })
        };
        let end = at(&sides[1], cx);
        sides[1].update(cx, |e, cx| e.sel(cx).set_selection(end, end + 2));
        run(&sides[1], cx, &Fold);
        let folds = sides.each_ref().map(|e| e.read_with(cx, |e, _| e.folds().chars()));
        let old_end = at(&sides[0], cx);
        assert_eq!(folds, [vec![old_end..old_end + 2], vec![end..end + 2]]);
        level(&sides, &[(0, 0), (1, 1), (3, 4), (4, 5), (5, 6)], cx);
    }

    #[gpui::test]
    fn a_caret_that_scrolls_its_side_sideways_takes_the_other_along(cx: &mut TestAppContext) {
        let long = "word ".repeat(100);
        let (old, new) = (format!("a\n{long}\nb\n"), format!("A\n{long}\nb\n"));
        let (_panes, sides, cx) = panes([&old, &new], [300., 300.], [false, false], cx);
        sides[0].update(cx, |e, cx| e.sel(cx).set_cursor(2));
        let end = crate::editor::Move::to(crate::editing::text::Motion::LineEnd, false);
        run(&sides[0], cx, &end);
        let lefts = sides.each_ref().map(|e| e.read_with(cx, |e, _| e.scroll_rows().2));
        assert!(lefts[0] > 0., "the old side scrolls to its caret");
        assert_eq!(lefts[1], lefts[0]);
    }

    #[gpui::test]
    fn a_fold_edge_inside_a_difference_keeps_the_rows_below_level(cx: &mut TestAppContext) {
        let old = "fn a() {\n    one\n    two\n}\nend\n";
        let new = "fn a() {\n    one\n    TWO\n}\nend\n";
        let (_panes, sides, cx) = panes([old, new], [300., 300.], [false, false], cx);
        let folds = |cx: &mut VisualTestContext| sides.each_ref().map(|e| e.read_with(cx, |e, _| e.folds().chars()));
        // the block ends inside the changed line, before its line break: so does the new side's
        sides[0].update(cx, |e, cx| e.sel(cx).set_cursor(20));
        run(&sides[0], cx, &Fold);
        level(&sides, &[(0, 0), (3, 3), (4, 4)], cx);
        assert_eq!(folds(cx), [vec![8..24], vec![8..24]]);
        run(&sides[0], cx, &Unfold);
        // a fold inside the changed line folds as much of the other side's line, no more
        sides[1].update(cx, |e, cx| e.sel(cx).set_selection(18, 21));
        run(&sides[1], cx, &Fold);
        assert_eq!(folds(cx), [vec![18..21], vec![18..21]]);
        level(&sides, &[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)], cx);
        // a fold on the other side carries it back as it was, not as the whole difference
        sides[0].update(cx, |e, cx| e.sel(cx).set_selection(27, 29));
        run(&sides[0], cx, &Fold);
        assert_eq!(folds(cx), [vec![18..21, 27..29], vec![18..21, 27..29]]);
        level(&sides, &[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)], cx);
    }
}
