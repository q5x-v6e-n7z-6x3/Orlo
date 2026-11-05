//! Cursor-column backgrounds share the caret's shaped display geometry.
use super::*;
use gpui::{point, size, Bounds, Entity, Modifiers, TestAppContext, VisualTestContext};

pub(crate) fn bands(e: &Editor) -> Vec<(usize, Bounds<Pixels>, Hsla)> {
    e.layout.as_ref().unwrap().drawn.cursor_columns.clone()
}

fn redraw(e: &Entity<Editor>, cx: &mut VisualTestContext, change: impl FnOnce(&mut Editor, &mut Context<Editor>)) {
    e.update(cx, |e, cx| {
        change(e, cx);
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn u07_columns_follow_multiple_distinct_heads_and_are_disabled_by_default(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("abcdef\nabcdef\nabcdef\n", cx, |e, cx| {
        e.sel(cx).set_selections(vec![Selection::caret(2), Selection::caret(9), Selection::caret(18)]);
    });
    assert!(e.read_with(cx, |e, _| bands(e)).is_empty());
    redraw(&e, cx, |e, _| e.view.cursor_column_highlight = true);
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        for row in 0..4 {
            let row_bands: Vec<_> = bands(e).into_iter().filter(|b| b.0 == row).collect();
            assert_eq!(row_bands.len(), 2, "duplicate column two must paint once on row {row}");
            for (band, col) in row_bands.iter().zip([2., 4.]) {
                assert!((f32::from(band.1.left() - l.text_left - l.space_w * col)).abs() < 0.1);
                assert!((f32::from(band.1.size.width - l.space_w)).abs() < 0.1);
                assert_eq!(band.1.size.height, px(e.line_height()));
                assert!(band.2.a > 0. && band.2.a <= 0.08);
            }
        }
    });
    redraw(&e, cx, |e, _| e.view.cursor_column_highlight = false);
    assert!(e.read_with(cx, |e, _| bands(e)).is_empty());
    redraw(&e, cx, |e, cx| {
        e.view.cursor_column_highlight = true;
        e.sel(cx).set_cursor(0);
    });
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        assert_eq!(bands(e).len(), 4);
        assert!(bands(e).iter().all(|b| b.1.left() == l.text_left));
        assert_eq!(l.drawn.cursor_column_shapes, 0);
    });
    redraw(&e, cx, |e, _| e.single_line = true);
    assert!(e.read_with(cx, |e, _| bands(e)).is_empty());
}

#[gpui::test]
fn u07_tabs_wide_combining_emoji_and_virtual_heads_share_the_actual_caret_x(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("\t界e\u{301}👩\u{200d}💻tail\nshort\n", cx, |e, cx| {
        e.view.cursor_column_highlight = true;
        e.set_virtual_space(true, cx);
    });
    for (offset, gap) in [(1, 0), (2, 0), (4, 0), (7, 0), (11, 3)] {
        redraw(&e, cx, move |e, cx| {
            e.sel(cx).set_selections(vec![Selection::from_points((offset, gap), (offset, gap))]);
        });
        e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let x = l.drawn.carets[0].left();
            assert_eq!(bands(e).len(), 3);
            assert!(bands(e).iter().all(|b| (f32::from(b.1.left() - x)).abs() < 0.1), "offset {offset} gap {gap}");
            assert_eq!(l.drawn.cursor_column_shapes, 0, "visible shaping must be reused");
        });
    }
    // A reversed nonempty selection contributes its head, without changing the selection.
    redraw(&e, cx, |e, cx| e.sel(cx).set_selection(7, 1));
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.selections[0], Selection::new(7, 1));
        let x = e.char_bounds(1, cx).unwrap().left();
        assert!(bands(e).iter().all(|b| (f32::from(b.1.left() - x)).abs() < 0.1));
    });
}

#[gpui::test]
fn u07_wrap_fold_and_filler_rows_keep_their_existing_geometry(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("  abcd efgh ijkl\naa\nbb\ncc\ndd\n", cx, |e, cx| {
        e.view.cursor_column_highlight = true;
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(7);
        e.sel(cx).set_cursor(9);
        e.fold_chars_now(std::iter::once(18..24), cx);
        e.set_fillers(Fillers::new(e.doc(cx), [(1, 2)]), cx);
    });
    let check = |e: &Editor, cx: &App| {
        let l = e.layout.as_ref().unwrap();
        let x = l.drawn.carets[0].left();
        assert!(bands(e).iter().all(|b| (f32::from(b.1.left() - x)).abs() < 0.1));
        assert_eq!(bands(e).len(), l.rows.iter().filter(|r| r.is_some()).count());
        assert_eq!(l.drawn.fillers.len(), 2);
        assert!(bands(e).iter().all(|b| !l.drawn.fillers.contains(&b.0)));
        assert!(!l.drawn.markers.is_empty());
        for (i, row) in l.rows.iter().enumerate() {
            if row.is_some() {
                let at = point(
                    x,
                    l.bounds.top()
                        + px((i as f64 + l.first_row as f64 - l.scroll_top / e.line_height() as f64 + 0.5) as f32
                            * e.line_height()),
                );
                assert!(e.offset_for_position(at, cx).is_some());
            }
        }
    };
    e.read_with(cx, check);
    redraw(&e, cx, |e, cx| {
        e.sel(cx).set_cursor(24);
        e.fold_chars_now(std::iter::once(18..24), cx);
    });
    e.read_with(cx, check);
    redraw(&e, cx, |e, cx| {
        e.sel(cx).set_cursor(26);
        let mut f = Folds::default();
        f.fold_lines(e.doc(cx), 1, 3);
        e.set_folds_now(f, cx);
    });
    e.read_with(cx, check);
    cx.simulate_resize(size(px(320.), px(210.)));
    cx.run_until_parked();
    e.read_with(cx, check);
}

#[gpui::test]
fn u07_scroll_clips_bands_and_keeps_offscreen_caret_origins(cx: &mut TestAppContext) {
    let text: String = (0..150).map(|_| "abcdefghij\n").collect();
    let (e, cx) = test_editor(&text, cx, |e, cx| {
        e.view.cursor_column_highlight = true;
        e.sel(cx).set_selections(vec![Selection::caret(2), Selection::caret(7)]);
    });
    redraw(&e, cx, |e, _| {
        e.follow_caret = false;
        let space = e.layout.as_ref().unwrap().space_w;
        e.follow_scroll_rows((100., None, f32::from(space * 4.)));
    });
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        assert_eq!(l.first_row, 100);
        let first: Vec<_> = bands(e).into_iter().filter(|b| b.0 == 100).collect();
        assert_eq!(first.len(), 1, "column two is outside the left clip");
        assert!((f32::from(first[0].1.left() - l.text_left - l.space_w * 7.)).abs() < 0.1);
        assert!(bands(e).iter().all(|b| b.1.left() >= l.text_area.left() && b.1.right() <= l.text_area.right()));
        assert_eq!(l.drawn.cursor_column_shapes, 1, "one offscreen row for both carets");
    });
    redraw(&e, cx, |e, _| {
        let space = e.layout.as_ref().unwrap().space_w;
        // Part of column seven remains in the viewport's left edge.
        let margin = px(theme::TEXT_MARGIN);
        e.follow_scroll_rows((100., None, f32::from(space * 7.5 + margin)));
    });
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        let first: Vec<_> = bands(e).into_iter().filter(|b| b.0 == 100).collect();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].1.left(), l.text_area.left());
        assert!((f32::from(first[0].1.size.width - l.space_w / 2.)).abs() < 0.1);
    });
}

#[gpui::test]
fn u07_enabled_many_carets_and_offscreen_huge_rows_have_bounded_shape_work(cx: &mut TestAppContext) {
    let long = format!("{}\n", "\t界e\u{301}😀".repeat(40_000));
    let text = format!("{}{}{}", long, "abc\n".repeat(1_000), long);
    let tail = text.chars().count() - long.chars().count();
    let (e, cx) = test_editor(&text, cx, move |e, cx| {
        e.view.cursor_column_highlight = true;
        let mut carets: Vec<_> = (0..1_000).map(|i| Selection::caret(long.chars().count() + i * 4 + 2)).collect();
        carets.extend([
            Selection::caret(10),
            Selection::caret(20),
            Selection::caret(tail + 10),
            Selection::caret(tail + 20),
        ]);
        e.sel(cx).set_selections(carets);
        e.follow_caret = false;
        e.follow_scroll_rows((400., None, 0.));
    });
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        assert_eq!(l.first_row, 400);
        assert_eq!(e.state.selections.len(), 1_004);
        assert!(l.drawn.cursor_column_shapes <= 1_002, "one shaping per offscreen caret row");
        assert!(
            l.drawn.cursor_column_chars < 5_000,
            "huge rows are clipped, ordinary rows are tiny: {}",
            l.drawn.cursor_column_chars
        );
        let row = bands(e).into_iter().filter(|b| b.0 == 400).collect::<Vec<_>>();
        assert_eq!(row.len(), 3, "one repeated ordinary column plus two repeated huge-row columns");
        assert_eq!(bands(e).len(), row.len() * l.rows.len());
        eprintln!(
            "U07 bounded frame: {} carets, {} offscreen row shapes, {} shaped chars, {} visible bands",
            e.state.selections.len(),
            l.drawn.cursor_column_shapes,
            l.drawn.cursor_column_chars,
            bands(e).len()
        );
    });
    redraw(&e, cx, move |e, cx| {
        e.sel(cx).set_selections(vec![Selection::caret(5_000), Selection::caret(tail + 5_000)]);
        e.follow_caret = true;
    });
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        assert!(e.scroll_left > 0.);
        assert!(!l.drawn.carets.is_empty());
        let x = l.drawn.carets[0].left();
        assert!(!bands(e).is_empty());
        assert!(
            bands(e).iter().all(|b| f32::from(b.1.left() - x).abs() < 0.1),
            "clipped Unicode rows use the same shifted origin as real carets"
        );
        assert_eq!(l.drawn.cursor_column_shapes, 1, "reuse the visible long row, measure the other once");
        assert!(l.drawn.cursor_column_chars < 2_000);
    });
    redraw(&e, cx, |e, _| e.view.cursor_column_highlight = false);
    e.read_with(cx, |e, _| {
        let d = &e.layout.as_ref().unwrap().drawn;
        assert!(d.cursor_columns.is_empty());
        assert_eq!((d.cursor_column_shapes, d.cursor_column_chars), (0, 0));
    });
}

#[gpui::test]
fn u07_resize_rewraps_columns_and_font_changes_measure_the_new_frame(cx: &mut TestAppContext) {
    let (e, cx) = test_editor(&"  abc def ghi jkl mno pqr stu vwx yz".repeat(4), cx, |e, cx| {
        e.view.cursor_column_highlight = true;
        e.wrap_override = Some(true);
        e.sel(cx).set_cursor(35);
    });
    let before = e.read_with(cx, |e, cx| (e.display_rows(cx), e.layout.as_ref().unwrap().drawn.carets[0].left()));
    cx.simulate_resize(size(px(240.), px(300.)));
    cx.run_until_parked();
    e.read_with(cx, |e, cx| {
        let l = e.layout.as_ref().unwrap();
        assert!(e.display_rows(cx) > before.0);
        assert_ne!(l.drawn.carets[0].left(), before.1);
        assert!(bands(e).iter().all(|b| (f32::from(b.1.left() - l.drawn.carets[0].left())).abs() < 0.1));
    });
    redraw(&e, cx, |e, _| e.view.font_size = 18.);
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        assert_eq!(l.font_size, px(18.));
        assert!(bands(e).iter().all(|b| (f32::from(b.1.left() - l.drawn.carets[0].left())).abs() < 0.1));
    });
}

#[gpui::test]
fn u07_decoration_preserves_layers_clicks_copy_save_source_and_undo(cx: &mut TestAppContext) {
    let source = "\t界 café café\nsecond 😀 line\n";
    let (e, cx) = focused_editor(source, None, cx);
    let saved = super::super::workspace::ui_tests::temp("u07-bytes").join("source.txt");
    e.update(cx, |e, cx| {
        e.edit(cx, |s| s.insert("X"));
        e.sel(cx).set_selection(3, 7);
        e.view.rulers = vec![5];
        e.view.selection_occurrences = true;
        e.find_ranges = std::iter::once(8..12).collect();
        e.state.highlights[0] = std::iter::once(3..7).collect();
        e.view.inline_diagnostics = true;
        e.update_doc(cx, |d| d.path = Some(saved.clone()));
        e.language_version = Some((saved.clone(), e.doc(cx).version));
        e.install_diagnostics(
            vec![crate::languages::language::Diagnostic {
                range: 0..1,
                severity: 1,
                message: "message".into(),
                raw: serde_json::Value::Null,
            }],
            None,
            cx,
        );
        cx.notify();
    });
    cx.run_until_parked();
    let diagnostic_before = e.read_with(cx, |e, _| {
        let diagnostics = e.layout.as_ref().unwrap().drawn.inline_diagnostics.clone();
        assert!(!diagnostics.is_empty());
        diagnostics
    });
    let before = e.read_with(cx, |e, cx| {
        let l = e.layout.as_ref().unwrap();
        let at = point(l.text_left + l.space_w * 4., l.bounds.top() + px(e.line_height() * 1.5));
        (
            e.text(cx),
            e.doc(cx).version,
            e.state.selections.clone(),
            (0..e.doc(cx).len()).map(|at| e.char_bounds(at, cx)).collect::<Vec<_>>(),
            at,
            e.offset_for_position(at, cx),
            l.drawn.selected_bounds.clone(),
            l.drawn.occurrences.clone(),
            l.drawn.marks.clone(),
            l.drawn.carets.clone(),
            e.find_ranges.clone(),
            e.state.highlights.clone(),
        )
    });
    assert!(!before.6.is_empty(), "selection background must be present");
    assert_eq!(
        before.7.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
        std::iter::once(8..12).collect::<Vec<_>>(),
        "passive match must actually be painted before the toggle"
    );
    let print_before = e.read_with(cx, |e, cx| {
        crate::platform::printing::Snapshot::new(
            e.doc(cx).rope.clone(),
            e.state.selections.iter().map(|s| s.range()).collect(),
            "U07".into(),
        )
        .source(false, Default::default())
        .unwrap()
    });
    cx.dispatch_action(Copy);
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    redraw(&e, cx, |e, _| e.view.cursor_column_highlight = true);
    cx.dispatch_action(Copy);
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), copied);
    e.read_with(cx, |e, cx| {
        let l = e.layout.as_ref().unwrap();
        assert!(!bands(e).is_empty());
        assert_eq!(l.drawn.inline_diagnostics, diagnostic_before);
        assert_eq!(
            (e.text(cx), e.doc(cx).version, e.state.selections.clone()),
            (before.0.clone(), before.1, before.2.clone())
        );
        assert_eq!((0..e.doc(cx).len()).map(|at| e.char_bounds(at, cx)).collect::<Vec<_>>(), before.3);
        assert_eq!(e.offset_for_position(before.4, cx), before.5);
        assert_eq!(
            (
                l.drawn.selected_bounds.clone(),
                l.drawn.occurrences.clone(),
                l.drawn.marks.clone(),
                l.drawn.carets.clone()
            ),
            (before.6.clone(), before.7.clone(), before.8.clone(), before.9.clone())
        );
        assert_eq!((&e.find_ranges, &e.state.highlights), (&before.10, &before.11));
        // Print consumes only the source rope and source selections.
        let snapshot = crate::platform::printing::Snapshot::new(
            e.doc(cx).rope.clone(),
            e.state.selections.iter().map(|s| s.range()).collect(),
            "U07".into(),
        );
        assert_eq!(snapshot.source(false, Default::default()).unwrap(), print_before);
    });
    e.update(cx, |e, cx| e.edit(cx, |s| s.save_to(saved.clone()).unwrap()));
    assert_eq!(std::fs::read_to_string(saved).unwrap(), before.0);
    cx.simulate_click(before.4, Modifiers::none());
    e.read_with(cx, |e, _| assert_eq!(Some(e.state.cursor()), before.5));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), source);
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), before.0);
}
