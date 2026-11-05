use super::*;
use crate::editing::text::Motion;
use gpui::{point, EntityInputHandler, Modifiers, MouseButton, TestAppContext};

fn caret(at: usize, gap: usize) -> Selection {
    Selection::from_points((at, gap), (at, gap))
}

#[gpui::test]
fn virtual_space_real_click_keyboard_and_ime_bounds_follow_shaped_text(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("\t😀e\u{301}|\nx", None, cx);
    let (base, space) = e.update(cx, |e, cx| {
        e.set_virtual_space(true, cx);
        (e.char_bounds(4, cx).unwrap(), e.layout.as_ref().unwrap().space_w)
    });
    let target = point(base.left() + space * 3., base.center().y);
    cx.simulate_click(target, Modifiers::none());
    cx.run_until_parked();
    e.update_in(cx, |e, window, cx| {
        assert_eq!(e.state.selections, vec![caret(4, 3)]);
        assert_eq!(e.text(cx), "\t😀e\u{301}\nx");
        let bounds = e.bounds_for_range(5..5, base, window, cx).unwrap();
        assert!((f32::from(bounds.left() - target.x)).abs() < 0.1);
        let drawn = e.layout.as_ref().unwrap().drawn.carets[0];
        assert!((f32::from(drawn.left() - target.x)).abs() < 0.1);
        assert!(e.cannot_complete(cx).unwrap().contains("beyond the line end"));
    });
    cx.simulate_keystrokes("left shift-right");
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0], Selection::from_points((4, 2), (4, 3))));
    cx.simulate_keystrokes("X");
    assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "\t😀e\u{301}  X\nx");
    cx.simulate_keystrokes("cmd-z");
    e.read_with(cx, |e, cx| {
        assert_eq!(e.text(cx), "\t😀e\u{301}\nx");
        assert_eq!(e.state.selections[0], Selection::from_points((4, 2), (4, 3)));
    });
}

#[gpui::test]
fn virtual_space_mouse_drag_rectangles_and_vertical_motion_use_measured_x(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("😀😀|\nx\nlonger", None, cx);
    e.update(cx, |e, cx| e.set_virtual_space(true, cx));
    cx.simulate_keystrokes("down");
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0].head_point(), (4, 3)));
    cx.simulate_keystrokes("up");
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0].head_point(), (2, 0)));
    let (a, b) = e.read_with(cx, |e, cx| {
        let l = e.layout.as_ref().unwrap();
        let end = e.char_bounds(2, cx).unwrap();
        (point(end.left() + l.space_w * 2., end.center().y), point(end.left() + l.space_w * 5., end.center().y))
    });
    cx.simulate_mouse_down(a, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(b, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(b, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    e.read_with(cx, |e, _| {
        assert_eq!(e.state.selections[0], Selection::from_points((2, 2), (2, 5)));
        assert!(!e.layout.as_ref().unwrap().drawn.selected.is_empty());
    });
    let lower = point(b.x, b.y + px(e.read_with(cx, |e, _| e.line_height())));
    let alt = Modifiers { alt: true, ..Modifiers::none() };
    cx.simulate_mouse_down(a, MouseButton::Left, alt);
    cx.simulate_mouse_move(lower, Some(MouseButton::Left), alt);
    cx.simulate_mouse_up(lower, MouseButton::Left, alt);
    e.read_with(cx, |e, cx| {
        assert_eq!(
            e.state.selections,
            vec![
                Selection::from_points((2, 2), (2, 5)).in_rectangle(),
                Selection::from_points((4, 5), (4, 8)).in_rectangle()
            ]
        );
        assert_eq!(e.text(cx), "😀😀\nx\nlonger");
    });
    cx.simulate_click(a, Modifiers::none());
    cx.simulate_click(b, Modifiers { platform: true, ..Modifiers::none() });
    e.read_with(cx, |e, _| assert_eq!(e.state.selections, vec![caret(2, 2), caret(2, 5)]));
    cx.simulate_keystrokes("z");
    e.read_with(cx, |e, cx| assert_eq!(e.text(cx), "😀😀  z   z\nx\nlonger"));
}

#[gpui::test]
fn virtual_space_native_composition_updates_cancel_commit_and_unmark(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("word\n😀", None, cx);
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(true, cx);
        for overtype in [false, true] {
            e.state.overtype = overtype;
            for cancel in [true, false] {
                let original = vec![Selection::from_points((4, 4), (4, 2)), caret(4, 7), caret(6, 3)];
                let mut primary_last = original.clone();
                primary_last.rotate_left(1);
                e.sel(cx).set_selections(primary_last);
                for preview in ["e\u{301}", "かな", "😀"] {
                    e.replace_and_mark_text_in_range(None, preview, Some(0..0), window, cx);
                    assert!(e.text(cx).starts_with("word  "));
                    assert_eq!(e.marked.len(), 3);
                }
                if cancel {
                    e.replace_and_mark_text_in_range(None, "", None, window, cx);
                } else {
                    e.replace_text_in_range(None, "", window, cx);
                }
                assert_eq!(e.text(cx), "word\n😀");
                assert_eq!(e.state.selections, original);
                assert_eq!(e.state.primary(), 0);
            }
            e.sel(cx).set_selections(vec![caret(4, 3), caret(6, 2)]);
            let original = e.state.selections.clone();
            e.replace_and_mark_text_in_range(None, "é", None, window, cx);
            e.unmark_text(window, cx);
            assert_eq!(e.text(cx), "word   é\n😀  é");
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "word\n😀");
            assert_eq!(e.state.selections, original);
            e.replace_and_mark_text_in_range(None, "e", None, window, cx);
            let marked = e.marked_text_range(window, cx);
            e.replace_text_in_range(marked, "\u{301}", window, cx);
            assert_eq!(e.text(cx), "word   \u{301}\n😀  \u{301}");
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "word\n😀");
        }
    });
}

#[gpui::test]
fn virtual_space_stale_composition_cannot_remove_shared_edits(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("word", None, cx);
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(true, cx);
        e.sel(cx).set_selections(vec![caret(4, 3)]);
        e.replace_and_mark_text_in_range(None, "é", None, window, cx);
    });
    let other = e.update(cx, |e, cx| {
        let document = e.document.clone();
        let view = e.state.duplicate(e.doc(cx));
        cx.new(|cx| Editor::of_document(document, view, false, cx))
    });
    other.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("!");
        })
    });
    e.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(None, "", window, cx);
        assert_eq!(e.text(cx), "!word   é");
        assert!(e.overtype_composition.is_none());
        assert!(e.marked.is_empty());
        e.sel(cx).set_selections(vec![caret(9, 2)]);
        e.replace_and_mark_text_in_range(None, "x", None, window, cx);
        e.sel(cx).set_cursor(0);
        e.replace_text_in_range(None, "Z", window, cx);
        assert_eq!(e.text(cx), "Z!word   é  x");
    });
}

#[gpui::test]
fn virtual_space_wrapped_rows_and_folds_keep_virtual_points_out_of_hidden_text(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("abcdefghij\nshort\ntail", cx, |e, cx| {
        e.set_virtual_space(true, cx);
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(4);
    });
    e.update(cx, |e, cx| {
        let rows = e.rows();
        let first = rows.row_bounds(e.doc(cx), DisplayRow(0)).unwrap();
        assert!(first.end < 10);
        let at = e.read_state(cx).selection_at(DisplayPoint::new(0, 100), &rows);
        assert_eq!(at.head_virtual, 0);
        let last = rows.row_of(e.doc(cx), 10);
        let at = e.read_state(cx).selection_at(DisplayPoint::new(last.row.0, last.col.0 + 3), &rows);
        assert_eq!(at, caret(10, 3));
        e.sel(cx).set_selections(vec![at]);
        e.fold_lines_now(0, 1, cx);
    });
    cx.run_until_parked();
    e.read_with(cx, |e, cx| {
        assert_eq!(e.text(cx), "abcdefghij\nshort\ntail");
        for sel in &e.state.selections {
            assert!(!e.folds.hides_any(&[sel.head]));
        }
    });
}

#[gpui::test]
fn virtual_space_immutable_and_single_line_views_keep_input_protection(cx: &mut TestAppContext) {
    let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("source"), true, cx));
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(true, cx);
        assert!(!e.state.virtual_space.enabled());
        e.single_line = false;
        e.input_enabled = false;
        e.set_virtual_space(true, cx);
        assert!(!e.state.virtual_space.enabled());
        e.replace_and_mark_text_in_range(None, "preview", None, window, cx);
        e.replace_text_in_range(None, "edit", window, cx);
        assert_eq!(e.text(cx), "source");
    });
}

#[gpui::test]
fn virtual_space_right_crosses_a_fold_marker_and_tracks_horizontal_scroll(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("head\ninside\ntail", None, cx);
    e.update(cx, |e, cx| {
        e.set_virtual_space(true, cx);
        e.fold_lines_now(0, 1, cx);
        e.sel(cx).set_cursor(4);
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("right");
    e.read_with(cx, |e, cx| {
        let sel = e.state.selections[0];
        assert_eq!(sel.head_virtual, 0);
        assert!(sel.head > 4);
        assert!(!e.folds.hides_any(&[sel.head]));
        assert_eq!(e.text(cx), "head\ninside\ntail");
    });
    e.update(cx, |e, cx| {
        e.sel(cx).set_selections(vec![caret(16, 200)]);
        e.scroll_to_cursor(cx);
        cx.notify();
    });
    cx.run_until_parked();
    e.read_with(cx, |e, cx| {
        assert!(e.scroll_left > 0.);
        let layout = e.layout.as_ref().unwrap();
        let caret = e.char_bounds(16, cx).unwrap();
        assert!(caret.left() >= layout.bounds.left());
        assert!(caret.left() < layout.bounds.right());
    });
}

#[gpui::test]
fn virtual_space_explicit_native_current_ranges_preserve_virtual_targets(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("😀word", None, cx);
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(true, cx);
        e.sel(cx).set_selections(vec![caret(5, 3)]);
        let native = e.selected_text_range(false, window, cx).unwrap().range;
        e.replace_text_in_range(Some(native.clone()), "X", window, cx);
        assert_eq!(e.text(cx), "😀word   X");
        e.edit(cx, |s| s.undo());
        e.replace_and_mark_text_in_range(Some(native), "é", None, window, cx);
        assert_eq!(e.text(cx), "😀word   é");
        e.replace_text_in_range(None, "", window, cx);
        assert_eq!(e.text(cx), "😀word");
        assert_eq!(e.state.selections, vec![caret(5, 3)]);
        e.replace_text_in_range(Some(0..2), "!", window, cx);
        assert_eq!(e.text(cx), "!word");
        assert!(e.state.selections.iter().all(|s| !s.has_virtual()));
    });
}

#[gpui::test]
fn virtual_space_far_beyond_a_clipped_long_line_stays_visible(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor(&"x".repeat(5000), None, cx);
    e.update(cx, |e, cx| {
        e.set_virtual_space(true, cx);
        e.sel(cx).set_selections(vec![caret(5000, 2000)]);
        e.scroll_to_cursor(cx);
        cx.notify();
    });
    cx.run_until_parked();
    e.read_with(cx, |e, cx| {
        let layout = e.layout.as_ref().unwrap();
        let caret = e.char_bounds(5000, cx).unwrap();
        assert!(caret.left() >= layout.bounds.left() && caret.left() < layout.bounds.right(), "{caret:?}");
        assert_eq!(e.text(cx).len(), 5000);
        assert!(!e.doc(cx).unsaved());
    });
}

#[gpui::test]
fn virtual_space_folding_a_header_clears_a_gap_that_now_overlaps_the_marker(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("head\ninside\ntail", None, cx);
    e.update(cx, |e, cx| {
        e.set_virtual_space(true, cx);
        e.sel(cx).set_selections(vec![caret(4, 3)]);
        e.change_folds(cx, |folds, doc| {
            folds.fold_lines(doc, 0, 1);
        });
        assert_eq!(e.state.selections, vec![Selection::caret(4)]);
        assert_eq!(e.text(cx), "head\ninside\ntail");
    });
}

#[gpui::test]
fn virtual_space_preserves_overtype_explicit_composition_target(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("abc😀tail", None, cx);
    e.update_in(cx, |e, window, cx| {
        e.state.overtype = true;
        for virtual_mode in [false, true] {
            e.set_virtual_space(virtual_mode, cx);
            let original = vec![caret(8, if virtual_mode { 3 } else { 0 })];
            e.sel(cx).set_selections(original.clone());
            e.replace_and_mark_text_in_range(Some(1..5), "é", None, window, cx);
            assert_eq!(e.text(cx), "aébc😀tail");
            e.replace_text_in_range(None, "é", window, cx);
            assert_eq!(e.text(cx), "aétail", "commit targets the explicitly named original source");
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "abc😀tail");
            assert_eq!(e.state.selections, original);
            e.replace_and_mark_text_in_range(Some(1..5), "é", None, window, cx);
            e.replace_text_in_range(None, "", window, cx);
            assert_eq!(e.text(cx), "abc😀tail");
            assert_eq!(e.state.selections, original);
        }
    });
}

#[gpui::test]
fn virtual_space_rectangle_edges_align_across_shaped_rows_in_both_directions(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("\t😀e\u{301}|\n\te\u{301}\nlonger", None, cx);
    e.update(cx, |e, cx| e.set_virtual_space(true, cx));
    let (a, b) = e.read_with(cx, |e, cx| {
        let end = e.char_bounds(4, cx).unwrap();
        let space = e.layout.as_ref().unwrap().space_w;
        (
            point(end.left() + space * 2., end.center().y),
            point(end.left() + space * 5., end.center().y + px(e.line_height())),
        )
    });
    let alt = Modifiers { alt: true, ..Modifiers::none() };
    for reverse in [false, true] {
        let (press, release) = if reverse { (b, a) } else { (a, b) };
        cx.simulate_mouse_down(press, MouseButton::Left, alt);
        cx.simulate_mouse_move(release, Some(MouseButton::Left), alt);
        cx.simulate_mouse_up(release, MouseButton::Left, alt);
        cx.run_until_parked();
        e.read_with(cx, |e, cx| {
            let endpoints = [((4, 2), (4, 5)), ((8, 4), (8, 7))];
            let expected: Vec<_> = endpoints
                .into_iter()
                .map(|(a, b)| {
                    (if reverse { Selection::from_points(b, a) } else { Selection::from_points(a, b) }).in_rectangle()
                })
                .collect();
            assert_eq!(e.state.selections, expected);
            assert_eq!(e.state.primary(), if reverse { 0 } else { 1 });
            assert_eq!(e.text(cx), "\t😀e\u{301}\n\te\u{301}\nlonger");
            let bounds = &e.layout.as_ref().unwrap().drawn.selected_bounds;
            assert_eq!(bounds.len(), 2);
            for (row, rect) in bounds {
                assert!(*row < 2);
                assert!(f32::from(rect.left() - a.x).abs() < 0.1, "left edge: {rect:?}");
                assert!(f32::from(rect.right() - b.x).abs() < 0.1, "right edge: {rect:?}");
            }
        });
    }
}

#[gpui::test]
fn virtual_space_rectangular_policy_real_mouse_and_keyboard_preserve_shaped_goals(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    let (e, cx) = focused_editor("\t界e\u{301}|\nx\nlonger", None, cx);
    e.update(cx, |e, cx| e.set_virtual_space(VirtualSpace::Rectangular, cx));
    let (a, b) = e.read_with(cx, |e, cx| {
        let end = e.char_bounds(4, cx).unwrap();
        let l = e.layout.as_ref().unwrap();
        let x = end.left() + l.space_w * 3.;
        (point(x, end.center().y), point(x, end.center().y + px(e.line_height())))
    });
    cx.simulate_click(a, Modifiers::none());
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0], Selection::caret(4)));
    let alt = Modifiers { alt: true, ..Modifiers::none() };
    cx.simulate_mouse_down(a, MouseButton::Left, alt);
    cx.simulate_mouse_move(b, Some(MouseButton::Left), alt);
    cx.simulate_mouse_up(b, MouseButton::Left, alt);
    cx.run_until_parked();
    let original = e.read_with(cx, |e, cx| {
        assert!(e.state.selections.iter().all(|sel| sel.rectangular && sel.has_virtual()));
        assert_eq!(e.state.selections[0].head_point(), (4, 3));
        assert_eq!(e.layout.as_ref().unwrap().drawn.carets.len(), 2);
        for drawn in &e.layout.as_ref().unwrap().drawn.carets {
            assert!(f32::from(drawn.left() - a.x).abs() < 0.1);
        }
        assert_eq!(e.text(cx), "\t界e\u{301}\nx\nlonger");
        assert!(!e.doc(cx).unsaved());
        e.state.selections.clone()
    });
    cx.simulate_keystrokes("X cmd-z");
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.selections, original);
        assert_eq!(e.text(cx), "\t界e\u{301}\nx\nlonger");
    });
    cx.simulate_keystrokes("left right delete");
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.selections, original);
        assert_eq!(e.text(cx), "\t界e\u{301}\nx\nlonger");
    });
    cx.simulate_click(a, Modifiers::none());
    cx.simulate_keystrokes("ctrl-shift-down right");
    e.read_with(cx, |e, _| {
        assert_eq!(e.state.selections.len(), 2);
        assert!(e.state.selections.iter().all(|sel| sel.rectangular && sel.head_virtual > 0));
    });
    cx.simulate_click(a, alt);
    e.read_with(cx, |e, _| {
        assert_eq!(e.state.selections, [caret(4, 3).in_rectangle()], "an Option-click already has rectangular origin");
    });
}

#[gpui::test]
fn virtual_space_rectangular_native_tabs_cancel_commit_recovery_and_mode_changes(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    let (e, cx) = focused_editor("a \t\n😀", None, cx);
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(VirtualSpace::Rectangular, cx);
        e.update_doc(cx, |doc| {
            doc.tab_size = 4;
            doc.translate_tabs = false;
        });
        for overtype in [false, true] {
            e.state.overtype = overtype;
            let original = vec![caret(3, 7).in_rectangle(), caret(5, 6).in_rectangle()];
            e.sel(cx).set_selections(original.clone());
            for text in ["e\u{301}", "かな", "𐐀"] {
                e.replace_and_mark_text_in_range(None, text, None, window, cx);
                assert!(e.text(cx).starts_with("a \t\t   "));
                let recovery = e.recovery_composition(cx).unwrap();
                let mut recovered = e.doc(cx).rope.clone();
                for range in recovery.removed.iter().rev() {
                    recovered.remove(range.clone());
                }
                assert_eq!(recovered.to_string(), "a \t\n😀");
                assert_eq!(recovery.selections, [(3, 3), (5, 5)]);
            }
            e.replace_text_in_range(None, "", window, cx);
            assert_eq!(e.text(cx), "a \t\n😀");
            assert_eq!(e.state.selections, original);
            e.replace_and_mark_text_in_range(None, "界", None, window, cx);
            e.unmark_text(window, cx);
            assert!(e.text(cx).starts_with("a \t\t   界"));
            e.edit(cx, |s| s.undo());
            assert_eq!(e.state.selections, original);
            assert_eq!(e.text(cx), "a \t\n😀");
        }
        e.replace_and_mark_text_in_range(None, "界", None, window, cx);
        let preview = e.text(cx);
        e.set_virtual_space(VirtualSpace::Off, cx);
        assert!(e.marked.is_empty() && e.overtype_composition.is_none());
        assert!(e.recovery_composition(cx).is_none());
        assert_eq!(e.text(cx), preview, "policy changes retain accepted ordinary preview semantics");
        e.replace_text_in_range(None, "", window, cx);
        assert_eq!(e.text(cx), preview, "late native cancellation cannot erase committed preview");
    });
}

#[gpui::test]
fn virtual_space_rectangular_soft_wraps_and_fold_markers_use_existing_rows(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    let (e, cx) = test_editor("界😀e\u{301}abcdefghij\nshort\ntail", cx, |e, cx| {
        e.set_virtual_space(VirtualSpace::Rectangular, cx);
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(4);
    });
    e.update_in(cx, |e, window, cx| {
        let rows = e.rows();
        let last = rows.row_of(e.doc(cx), 14);
        e.with_rows(window, cx, |s, rows, _| {
            s.drag_box(&[], false, DisplayPoint::new(0, 50), DisplayPoint::new(last.row.0, 50), rows);
        });
        assert_eq!(e.state.selections.first().unwrap().head_virtual, 0);
        assert!(e.state.selections.last().unwrap().head_virtual > 0);
        e.fold_lines_now(0, 1, cx);
        assert!(e.state.selections.iter().all(|sel| !e.folds.hides_any(&[sel.head])));
        assert_eq!(e.text(cx), "界😀e\u{301}abcdefghij\nshort\ntail");
    });
}

#[gpui::test]
fn virtual_space_origin_only_changes_invalidate_native_composition_targets(cx: &mut TestAppContext) {
    let (e, cx) = focused_editor("source", None, cx);
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(true, cx);
        e.state.overtype = true;
        e.sel(cx).set_selections(vec![caret(6, 5).in_rectangle()]);
        e.replace_and_mark_text_in_range(None, "界", None, window, cx);
        let preview = e.text(cx);
        assert!(e.recovery_composition(cx).is_some());
        let generation = e.read_state(cx).selection_generation();
        let changed = e.state.selections.iter().map(|sel| Selection { rectangular: false, ..*sel }).collect();
        e.sel(cx).reselect(changed);
        assert!(e.read_state(cx).selection_generation() > generation);
        assert!(e.recovery_composition(cx).is_none());
        e.replace_text_in_range(None, "", window, cx);
        assert_eq!(
            e.text(cx),
            preview,
            "the same physical offsets with a different origin cannot revive a native target"
        );
    });
}

#[gpui::test]
fn virtual_space_rectangular_native_read_only_working_copy_and_follow_keep_guards(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    let (e, cx) = focused_editor("source", None, cx);
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(VirtualSpace::Rectangular, cx);
        e.update_doc(cx, |doc| doc.read_only = true);
        e.sel(cx).set_selections(vec![caret(6, 3).in_rectangle()]);
        e.replace_text_in_range(None, "X", window, cx);
        assert_eq!(e.text(cx), "source   X");
        e.edit(cx, |s| {
            assert_eq!(s.doc.save().unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
            s.undo();
        });
        assert_eq!(e.text(cx), "source");
        let original = e.state.selections.clone();
        e.update_doc(cx, |doc| doc.set_following(true));
        e.replace_and_mark_text_in_range(None, "界", None, window, cx);
        e.replace_text_in_range(None, "X", window, cx);
        assert_eq!(e.text(cx), "source");
        assert_eq!(e.state.selections, original);
        assert!(e.marked.is_empty() && e.overtype_composition.is_none());
        e.update_doc(cx, |doc| doc.set_following(false));
    });
}

#[gpui::test]
fn virtual_space_hard_tabs_keep_the_rendered_target_after_folded_and_wrapped_prefixes(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    for wrapped in [false, true] {
        let source = if wrapped { "abcdefghij abc" } else { "head\nhidden\nx" };
        let (e, cx) = test_editor(source, cx, move |e, cx| {
            e.set_virtual_space(VirtualSpace::Rectangular, cx);
            e.update_doc(cx, |doc| {
                doc.tab_size = 4;
                doc.translate_tabs = false;
            });
            e.wrap_override = Some(wrapped);
            e.wrap_width_override = Some(10);
            if !wrapped {
                e.fold_chars_now(std::iter::once(4..12), cx);
            }
        });
        let (target, space, original) = e.update_in(cx, |e, window, cx| {
            let end = e.doc(cx).len();
            let at = e.rows().row_of(e.doc(cx), end);
            let physical = crate::editing::display::col_after(
                e.doc(cx).rope.slice(e.doc(cx).line_start(e.doc(cx).point(end).0)..end),
                4,
            );
            assert_ne!(at.col.0 % 4, physical % 4, "fixture has different physical/display tab stops");
            let end_bounds = e.char_bounds(end, cx).unwrap();
            let space = e.layout.as_ref().unwrap().space_w;
            let target = end_bounds.left() + space * 3.;
            e.with_rows(window, cx, |s, rows, _| {
                s.drag_box(
                    &[],
                    false,
                    DisplayPoint::new(at.row.0, at.col.0 + 3),
                    DisplayPoint::new(at.row.0, at.col.0 + 3),
                    rows,
                );
            });
            (target, space, e.state.selections.clone())
        });
        e.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "X", None, window, cx));
        cx.run_until_parked();
        e.read_with(cx, |e, cx| {
            let drawn = e.char_bounds(e.state.cursor(), cx).unwrap();
            assert!(
                f32::from(drawn.left() - target - space).abs() < 0.1,
                "preview target {target:?}, actual {drawn:?}, wrapped {wrapped}"
            );
        });
        e.update_in(cx, |e, window, cx| {
            e.replace_text_in_range(None, "", window, cx);
            assert_eq!(e.text(cx), source);
            assert_eq!(e.state.selections, original);
            e.replace_and_mark_text_in_range(None, "X", None, window, cx);
            e.unmark_text(window, cx);
        });
        cx.run_until_parked();
        e.update(cx, |e, cx| {
            let drawn = e.char_bounds(e.state.cursor(), cx).unwrap();
            assert!(
                f32::from(drawn.left() - target - space).abs() < 0.1,
                "commit target {target:?}, actual {drawn:?}, wrapped {wrapped}"
            );
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), source);
            assert_eq!(e.state.selections, original);
        });
    }
}

#[gpui::test]
fn virtual_space_mapped_tab_permission_refreshes_after_shared_edits(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    let (e, cx) = focused_editor("head\nhidden\nx", None, cx);
    e.update(cx, |e, cx| {
        e.set_virtual_space(VirtualSpace::Rectangular, cx);
        e.update_doc(cx, |doc| {
            doc.tab_size = 4;
            doc.translate_tabs = false;
        });
        e.fold_chars_now(std::iter::once(4..12), cx);
        e.sel(cx).set_selections(vec![caret(13, 3).in_rectangle()]);
    });
    let other = e.update(cx, |e, cx| {
        let document = e.document.clone();
        let state = e.state.duplicate(e.doc(cx));
        cx.new(|cx| Editor::of_document(document, state, false, cx))
    });
    e.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(None, "X", window, cx);
        assert_eq!(e.text(cx), "head\nhidden\nx   X");
        e.edit(cx, |s| s.undo());
    });
    other.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("!");
        })
    });
    e.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(None, "X", window, cx);
        assert_eq!(e.text(cx), "!head\nhidden\nx\tX", "new mapped/source tab stops agree");
        e.edit(cx, |s| s.undo());
    });
    other.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.set_selection(0, 1);
            s.insert("");
        })
    });
    e.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(None, "X", window, cx);
        assert_eq!(e.text(cx), "head\nhidden\nx   X", "a later mismatch cannot reuse prior permission");
        e.edit(cx, |s| s.undo());
        assert_eq!(e.state.selections, [caret(13, 3).in_rectangle()]);
    });
}

#[gpui::test]
fn virtual_space_rectangular_native_reconstruction_explicit_replacements_keep_each_origin(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    for overtype in [false, true] {
        let original = "a😀\na😀";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(original), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.set_virtual_space(VirtualSpace::Rectangular, cx);
            e.sel(cx).set_selections(vec![Selection::caret(2).in_rectangle(), Selection::caret(5)]);
            if overtype {
                e.perform_action(&ToggleOvertype, window, cx);
            }
            // UTF-16 4..7 is the second a😀; the same relative range reaches the first caret.
            e.replace_text_in_range(Some(4..7), "界", window, cx);
            assert_eq!(e.text(cx), "界\n界");
            e.perform_action(&Move::to(Motion::Right, false), window, cx);
            assert_eq!(e.state.selections.iter().map(Selection::head_point).collect::<Vec<_>>(), [(1, 1), (3, 0)]);
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

#[gpui::test]
fn virtual_space_rectangular_native_reconstruction_explicit_overtype_keeps_commit_and_cancel(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    for cancel in [false, true] {
        let original = "a😀";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(original), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.set_virtual_space(VirtualSpace::Rectangular, cx);
            e.sel(cx).set_selections(vec![Selection::caret(2).in_rectangle()]);
            let before = e.state.selections.clone();
            e.perform_action(&ToggleOvertype, window, cx);
            e.replace_and_mark_text_in_range(Some(1..3), "仮", None, window, cx);
            assert_eq!(e.text(cx), "a仮😀");
            if cancel {
                e.replace_and_mark_text_in_range(None, "", None, window, cx);
                assert_eq!(e.text(cx), original);
                assert_eq!(e.state.selections, before);
            } else {
                e.replace_text_in_range(None, "界", window, cx);
                assert_eq!(e.text(cx), "a界");
            }
            assert!(e.marked.is_empty());
            e.perform_action(&Move::to(Motion::Right, false), window, cx);
            assert_eq!(e.state.selections[0].head_point(), (2, 1));
            assert!(e.state.selections[0].rectangular);
            if !cancel {
                e.edit(cx, |s| s.undo());
                assert_eq!(e.text(cx), original);
                assert_eq!(e.state.selections, before);
                e.edit(cx, |s| s.redo());
                assert_eq!(e.text(cx), "a界");
            }
        });
    }
}

#[gpui::test]
fn virtual_space_rectangular_native_reconstruction_empty_marked_update_keeps_each_origin(cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    let original = "a\nb";
    let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(original), false, cx));
    e.update_in(cx, |e, window, cx| {
        e.set_virtual_space(VirtualSpace::Rectangular, cx);
        e.sel(cx).set_selections(vec![Selection::caret(1).in_rectangle(), Selection::caret(3)]);
        let before = e.state.selections.clone();
        e.replace_and_mark_text_in_range(None, "仮", None, window, cx);
        assert_eq!(e.text(cx), "a仮\nb仮");
        e.replace_and_mark_text_in_range(None, "", None, window, cx);
        assert_eq!(e.text(cx), original);
        assert!(e.marked.is_empty());
        assert_eq!(e.state.selections, before);
        e.perform_action(&Move::to(Motion::Right, false), window, cx);
        assert_eq!(e.state.selections.iter().map(Selection::head_point).collect::<Vec<_>>(), [(1, 1), (3, 0)]);
        assert!(e.state.selections[0].rectangular);
        assert!(!e.state.selections[1].rectangular);
        assert_eq!(e.text(cx), original);
    });
}

#[gpui::test]
fn virtual_space_follows_a_wrapped_continuation_row_sideways_from_its_indent(cx: &mut TestAppContext) {
    let text = format!("{}{}", " ".repeat(16), "word ".repeat(20));
    let len = text.chars().count();
    let (e, cx) = focused_editor(&text, None, cx);
    e.update(cx, |e, cx| {
        e.set_virtual_space(true, cx);
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(40);
        e.sel(cx).set_selections(vec![caret(len, 200)]);
        e.scroll_to_cursor(cx);
        cx.notify();
    });
    cx.run_until_parked();
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        let caret = l.drawn.carets[0];
        assert!(caret.left() >= l.bounds.left() && caret.left() < l.bounds.right(), "{caret:?} in {:?}", l.bounds);
    });
}
