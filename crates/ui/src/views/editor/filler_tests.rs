//! Filler rows in a view: drawing, motion, clicks, copying, finding and the scroll range.

use super::*;
use crate::editing::text::{FindOpts, Motion};
use gpui::VisualTestContext;
use gpui::{point, Entity, Modifiers, TestAppContext};

/// "zero", "one", "two", "three" with 2 filler rows before "zero", 3 before "two" and 1 after
/// "three": rows F F zero one F F F two three F. Offsets: one 5..8, two 9..12, three 13..18.
const TEXT: &str = "zero\none\ntwo\nthree";

/// An editor alone in a 600 × 300 window, unwrapped, with `TEXT`'s filler rows, drawn once.
fn editor(cx: &mut TestAppContext) -> (Entity<Editor>, &mut VisualTestContext) {
    test_editor(TEXT, cx, |e, cx| {
        e.fillers = Fillers::new(e.doc(cx), [(0, 2), (2, 3), (4, 1)]);
    })
}

fn caret(e: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    e.read_with(cx, |e, _| e.state.cursor())
}

/// The middle of display row `row`, `x` across.
fn at_row(e: &Entity<Editor>, cx: &mut VisualTestContext, x: Pixels, row: f32) -> Point<Pixels> {
    e.read_with(cx, |e, _| {
        let top = e.layout.as_ref().unwrap().bounds.top();
        point(x, top + px(e.line_height() * (row + 0.5)))
    })
}

#[gpui::test]
fn filler_rows_draw_no_text_no_number_and_selections_skip_them(cx: &mut TestAppContext) {
    let (e, cx) = editor(cx);
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        let drawn: Vec<Option<&str>> = (l.rows.iter()).map(|r| r.as_ref().map(|(_, s)| s.text.as_ref())).collect();
        let (f, t) = (None, Some);
        let want = [f, f, t("zero"), t("one"), f, f, f, t("two"), t("three"), f];
        assert_eq!(drawn, want);
        assert_eq!(l.drawn.fillers, [0, 1, 4, 5, 6, 9]);
        // numbers on the text rows only, and they count lines, not rows
        assert_eq!(l.drawn.numbers, [(2, 1), (3, 2), (7, 3), (8, 4)]);
    });
    // a selection over all of it paints on the text rows only
    run(&e, cx, &SelectAll);
    e.read_with(cx, |e, _| {
        assert_eq!(e.layout.as_ref().unwrap().drawn.selected, [2, 3, 7, 8]);
    });
}

#[gpui::test]
fn up_and_down_step_over_filler_rows_in_a_view(cx: &mut TestAppContext) {
    let (e, cx) = editor(cx);
    let (down, up) = (Move::to(Motion::Down, false), Move::to(Motion::Up, false));
    e.update(cx, |e, cx| e.sel(cx).set_cursor(6));
    run(&e, cx, &down);
    assert_eq!(caret(&e, cx), 10, "from \"one\" over three filler rows to \"two\"");
    run(&e, cx, &up);
    assert_eq!(caret(&e, cx), 6);
    run(&e, cx, &up);
    run(&e, cx, &up);
    assert_eq!(caret(&e, cx), 0, "above \"zero\" there are filler rows only");
    e.update(cx, |e, cx| e.sel(cx).set_cursor(14));
    run(&e, cx, &down);
    assert_eq!(caret(&e, cx), 18, "below \"three\" likewise");
}

#[gpui::test]
fn a_click_on_a_filler_row_lands_at_the_end_of_the_line_above_it(cx: &mut TestAppContext) {
    let (e, cx) = editor(cx);
    let x = e.read_with(cx, |e, _| e.layout.as_ref().unwrap().text_left + px(20.));
    let at = at_row(&e, cx, x, 5.);
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(caret(&e, cx), 8, "the end of \"one\"");
    let at = at_row(&e, cx, x, 0.);
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(caret(&e, cx), 0, "no line above: the start");
    let at = at_row(&e, cx, x, 9.);
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(caret(&e, cx), 18, "after \"three\": its end");
}

#[gpui::test]
fn copy_and_find_never_take_in_filler_rows(cx: &mut TestAppContext) {
    let (e, cx) = editor(cx);
    // from "one" down over the filler rows into "two"
    e.update(cx, |e, cx| e.sel(cx).set_cursor(6));
    run(&e, cx, &Move::to(Motion::Down, true));
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0].range(), 6..10));
    run(&e, cx, &Copy);
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("ne\nt"), "the text of 6..10, no rows between");
    e.read_with(cx, |e, cx| {
        let found = e.read_state(cx).find_all("o", &FindOpts::default());
        let plain = EditorState::from_text(TEXT).find_all("o", &FindOpts::default());
        assert_eq!(found, plain);
        assert_eq!(e.text(cx), TEXT, "the text has no filler rows in it");
    });
}

#[gpui::test]
fn the_scroll_range_and_minimap_count_filler_rows(cx: &mut TestAppContext) {
    // 301 lines with the empty last one, and 100 filler rows after line 10
    let (e, cx) = test_editor(&"line\n".repeat(300), cx, |_, _| {});
    let before = e.read_with(cx, |e, cx| e.max_scroll(cx));
    e.update(cx, |e, cx| {
        let fillers = Fillers::new(e.doc(cx), [(10, 100)]);
        e.set_fillers(fillers, cx);
    });
    cx.run_until_parked();
    e.read_with(cx, |e, cx| {
        assert_eq!(e.max_scroll(cx), before + 100. * 15.);
        assert_eq!(e.rows().rows(e.doc(cx)), 401);
        // the minimap's 150 rows from the top take in the filler rows as rows
        assert_eq!(e.layout.as_ref().unwrap().drawn.minimap_rows, 151);
    });
}

#[gpui::test]
fn new_filler_rows_keep_the_text_on_top_where_it_is(cx: &mut TestAppContext) {
    let (e, cx) = test_editor(&"line\n".repeat(300), cx, |_, _| {});
    e.update(cx, |e, cx| {
        e.follow_scroll(Some((50, 0.)), None, cx);
        cx.notify();
    });
    cx.run_until_parked();
    e.update(cx, |e, cx| {
        let fillers = Fillers::new(e.doc(cx), [(10, 7), (60, 3)]);
        e.set_fillers(fillers, cx);
    });
    cx.run_until_parked();
    // line 50 is still on top: 7 filler rows above it now
    e.read_with(cx, |e, cx| {
        assert_eq!(e.scroll_position(cx).0, 50);
        assert_eq!(e.scroll_top, 57. * 15.);
    });
}
