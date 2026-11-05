//! Gutter labels use physical source lines without changing the display/hit pipeline.

use super::*;
use crate::config::settings::LineNumberMode::{Absolute, Relative};
use gpui::{point, Entity, Modifiers, TestAppContext, VisualTestContext};

pub(crate) fn gutter_labels(e: &Editor) -> Vec<(usize, SharedString)> {
    e.layout.as_ref().unwrap().drawn.number_labels.clone()
}

fn numbers(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<(usize, usize)> {
    e.read_with(cx, |e, _| {
        let drawn = &e.layout.as_ref().unwrap().drawn;
        let shaped: Vec<_> = drawn.number_labels.iter().map(|(r, text)| (*r, text.parse().unwrap())).collect();
        assert_eq!(shaped, drawn.numbers, "inspect the actual shaped labels supplied to paint");
        shaped
    })
}

#[gpui::test]
fn u06_relative_gutter_uses_only_the_primary_physical_head_at_both_document_ends(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("a\nb\nc\nd\ne\n", cx, |e, cx| {
        e.view.line_number_mode = Relative;
        // Normalization sorts carets; the last supplied caret remains primary, in the middle.
        e.sel(cx).set_selections(vec![Selection::caret(0), Selection::caret(8), Selection::caret(4)]);
    });
    assert_eq!(numbers(&e, cx), [(0, 2), (1, 1), (2, 3), (3, 1), (4, 2), (5, 3)]);
    e.update(cx, |e, cx| {
        e.sel(cx).set_selection(10, 4);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(numbers(&e, cx), [(0, 2), (1, 1), (2, 3), (3, 1), (4, 2), (5, 3)], "head, not anchor");
    for (caret, expected) in [
        (0, vec![(0, 1), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]),
        (10, vec![(0, 5), (1, 4), (2, 3), (3, 2), (4, 1), (5, 6)]),
    ] {
        e.update(cx, |e, cx| {
            e.sel(cx).set_cursor(caret);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(numbers(&e, cx), expected);
    }
}

#[gpui::test]
fn u06_wrap_continuations_and_fillers_have_no_numbers(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("abcd efgh ijkl\nb\nc\n", cx, |e, cx| {
        e.view.line_number_mode = Relative;
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(5);
        e.sel(cx).set_cursor(15);
        e.set_fillers(Fillers::new(e.doc(cx), [(1, 2)]), cx);
    });
    assert_eq!(numbers(&e, cx), [(0, 1), (5, 2), (6, 1), (7, 2)]);
    e.read_with(cx, |e, _| assert_eq!(e.layout.as_ref().unwrap().drawn.fillers, [3, 4]));
    let at = e.read_with(cx, |e, cx| {
        let l = e.layout.as_ref().unwrap();
        let at = point(l.text_left, l.bounds.top() + px(e.line_height() * 6.5));
        assert_eq!(e.offset_for_position(at, cx), Some(17), "wrapped/padded source hit");
        at
    });
    cx.simulate_click(at, Modifiers::none());
    e.read_with(cx, |e, _| assert_eq!(e.state.cursor(), 17));
}

#[gpui::test]
fn u06_folds_count_hidden_physical_lines_and_inline_tail_caret_keeps_its_source_origin(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("aa\nbb\ncc\ndd\nee\n", cx, |e, cx| {
        e.view.line_number_mode = Relative;
        e.sel(cx).set_cursor(9);
        let mut folds = Folds::default();
        folds.fold_lines(e.doc(cx), 0, 2);
        e.set_folds_now(folds, cx);
    });
    assert_eq!(numbers(&e, cx), [(0, 3), (1, 4), (2, 1), (3, 2)]);
    e.update(cx, |e, cx| {
        e.fold_chars_now(std::iter::once(2..9), cx);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(numbers(&e, cx), [(0, 3), (1, 1), (2, 2)], "joined header is not the primary physical line");
    e.read_with(cx, |e, cx| {
        assert_eq!(e.read_state(cx).point(e.state.cursor()).0, 3);
        assert_eq!(e.text(cx), "aa\nbb\ncc\ndd\nee\n");
    });
    let at = e.read_with(cx, |e, cx| {
        let l = e.layout.as_ref().unwrap();
        let at = point(l.text_left + l.space_w * 5., l.bounds.top() + px(e.line_height() * 0.5));
        assert_eq!(e.offset_for_position(at, cx), Some(9), "inline fold tail retains source hit");
        at
    });
    cx.simulate_click(at, Modifiers::none());
    e.read_with(cx, |e, _| assert_eq!(e.state.cursor(), 9));
}

#[gpui::test]
fn u06_mode_switch_preserves_layout_clicks_source_line_navigation_and_undo(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("aa\nbb\ncc\ndd\nee\n", cx, |e, cx| {
        e.sel(cx).set_cursor(6);
        e.edit(cx, |s| s.insert("X"));
    });
    let before = e.read_with(cx, |e, cx| {
        let layout = e.layout.as_ref().unwrap();
        let at = point(layout.text_left + layout.space_w, layout.bounds.top() + px(e.line_height() * 3.5));
        (
            e.text(cx),
            e.doc(cx).version,
            e.state.selections.clone(),
            layout.text_left,
            layout.text_width,
            e.scroll_rows().0,
            at,
            e.offset_for_position(at, cx),
        )
    });
    assert_eq!(numbers(&e, cx), [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6)]);
    e.update(cx, |e, cx| {
        e.view.line_number_mode = Relative;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(numbers(&e, cx), [(0, 2), (1, 1), (2, 3), (3, 1), (4, 2), (5, 3)]);
    e.read_with(cx, |e, cx| {
        let layout = e.layout.as_ref().unwrap();
        assert_eq!(
            (e.text(cx), e.doc(cx).version, e.state.selections.clone()),
            (before.0.clone(), before.1, before.2.clone())
        );
        assert_eq!((layout.text_left, layout.text_width, e.scroll_rows().0), (before.3, before.4, before.5));
        assert_eq!(e.offset_for_position(before.6, cx), before.7);
    });
    cx.simulate_click(before.6, Modifiers::none());
    e.read_with(cx, |e, _| assert_eq!(Some(e.state.cursor()), before.7));
    e.update(cx, |e, cx| {
        e.goto_line(5, cx);
        assert_eq!(e.read_state(cx).point(e.state.cursor()).0, 4, "Go to Line stays one-based source navigation");
        e.edit(cx, |s| s.undo());
        assert_eq!(e.text(cx), "aa\nbb\ncc\ndd\nee\n");
        e.edit(cx, |s| s.redo());
        assert_eq!(e.text(cx), before.0);
        e.view.line_number_mode = Absolute;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(numbers(&e, cx), [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6)]);
}

#[gpui::test]
fn u06_scrolled_gutter_retains_an_offscreen_primary_origin_and_visibility(cx: &mut TestAppContext) {
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let (e, cx) = test_editor(&text, cx, |e, cx| {
        e.view.line_number_mode = Relative;
        e.sel(cx).set_cursor(0);
    });
    e.update(cx, |e, cx| {
        e.follow_caret = false;
        e.follow_scroll_rows((50., None, 0.));
        cx.notify();
    });
    cx.run_until_parked();
    let shown = numbers(&e, cx);
    assert_eq!(&shown[..3], &[(50, 50), (51, 51), (52, 52)]);
    e.update(cx, |e, cx| {
        e.view.gutter = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(numbers(&e, cx).is_empty());
    e.update(cx, |e, cx| {
        e.view.gutter = true;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(numbers(&e, cx), shown);
}
