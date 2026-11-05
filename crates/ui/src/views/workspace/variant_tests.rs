//! Every variant of an action family (layout presets, indentation, highlight styles, extraction,
//! permutations), dispatched through the window as a key binding would.

use super::ui_tests::{find_text, settle, settle_until, temp, text, type_in, window, write_in};
use super::*;
use crate::searching::extract::What;
use crate::searching::results::SearchFlags;
use gpui::{Action, TestAppContext, VisualTestContext};

fn dispatch(ws: &Entity<Workspace>, cx: &mut VisualTestContext, action: impl Action) {
    ws.update_in(cx, |_, window, cx| window.dispatch_action(Box::new(action), cx));
    cx.run_until_parked();
}

#[gpui::test]
fn every_layout_preset_keeps_dirty_views_and_their_selections(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let originals = ws.update_in(cx, |ws, window, cx| {
        ws.set_layout(&SetLayout { preset: Preset::Grid4 }, window, cx);
        let mut originals = Vec::new();
        for (g, value) in ["alpha", "bravo", "charlie", "delta"].into_iter().enumerate() {
            ws.focus_group(&FocusGroup { group: g }, window, cx);
            ws.new_file(&NewFile, window, cx);
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.edit(cx, |s| s.insert(value));
                e.sel(cx).set_selection(g, g + 1);
            });
            originals.push((e, value, g..g + 1));
        }
        originals
    });
    let cases: [(Preset, &[[f32; 4]]); 7] = [
        (Preset::Grid4, &[[0., 0., 0.5, 0.5], [0.5, 0., 1., 0.5], [0., 0.5, 0.5, 1.], [0.5, 0.5, 1., 1.]]),
        (Preset::Columns(4), &[[0., 0., 0.25, 1.], [0.25, 0., 0.5, 1.], [0.5, 0., 0.75, 1.], [0.75, 0., 1., 1.]]),
        (Preset::Columns(3), &[[0., 0., 0.33, 1.], [0.33, 0., 0.66, 1.], [0.66, 0., 1., 1.]]),
        (Preset::Rows(3), &[[0., 0., 1., 0.33], [0., 0.33, 1., 0.66], [0., 0.66, 1., 1.]]),
        (Preset::Columns(2), &[[0., 0., 0.5, 1.], [0.5, 0., 1., 1.]]),
        (Preset::Rows(2), &[[0., 0., 1., 0.5], [0., 0.5, 1., 1.]]),
        (Preset::Single, &[[0., 0., 1., 1.]]),
    ];
    for (preset, cells) in cases {
        dispatch(&ws, cx, SetLayout { preset });
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.groups.len(), cells.len(), "{preset:?}");
            for (g, cell) in cells.iter().enumerate() {
                assert_eq!(&ws.views.layout.cell(g), cell, "{preset:?}, group {g}");
            }
            assert_eq!(ws.views.tabs().count(), 4, "{preset:?}");
            for (e, value, selection) in &originals {
                assert!(ws.views.find(e).is_some(), "{preset:?}: {value}");
                let s = e.read(cx).read_state(cx);
                assert_eq!(s.text(), *value);
                assert!(s.doc.dirty);
                assert_eq!(s.view.selections[0].range(), *selection);
            }
        });
    }
    ws.read_with(cx, |ws, _| assert_eq!(ws.active_editor().as_ref(), Some(&originals[3].0)));
}

#[gpui::test]
fn every_indentation_choice_changes_tab_insertion_in_all_shared_views(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "x");
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    for (size, spaces) in [(1, "x "), (2, "x "), (3, "x  "), (4, "x   "), (8, "x       "), (16, "x               ")] {
        for (to_spaces, expected) in [(true, spaces), (false, "x\t")] {
            ws.update(cx, |ws, cx| {
                ws.with_active(cx, |e, cx| {
                    e.edit(cx, |s| {
                        s.select_all();
                        s.insert("x");
                    });
                });
            });
            cx.run_until_parked();
            dispatch(&ws, cx, SetTabSize { size });
            dispatch(&ws, cx, SetIndentStyle { to_spaces });
            ws.read_with(cx, |ws, cx| {
                for e in ws.views.tabs() {
                    let s = e.read(cx).read_state(cx);
                    assert_eq!((s.doc.tab_size, s.doc.translate_tabs, s.text()), (size, to_spaces, "x".into()));
                }
            });
            dispatch(&ws, cx, editor::Tab);
            ws.read_with(cx, |ws, cx| {
                assert_eq!(ws.views.tabs().count(), 2);
                for e in ws.views.tabs() {
                    assert_eq!(e.read(cx).doc(cx).text(), expected, "width {size}, spaces {to_spaces}");
                }
            });
        }
    }
}

#[gpui::test]
fn horizontal_sync_preserves_vertical_positions_ignores_hidden_tabs_and_can_stop(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let (hidden, follower, source) = ws.update_in(cx, |ws, window, cx| {
        let mut editors = Vec::new();
        for i in 0..3 {
            if i > 0 {
                ws.new_file(&NewFile, window, cx);
            }
            ws.with_active(cx, |e, cx| {
                e.wrap_override = Some(false);
                e.edit(cx, |s| s.insert(&format!("{}\n", "x".repeat(200)).repeat(100)))
            });
            editors.push(ws.active_editor().unwrap());
        }
        ws.views.toggle_selected(0, 1);
        (editors[0].clone(), editors[1].clone(), editors[2].clone())
    });
    dispatch(&ws, cx, ToggleSyncScroll { vertical: false });
    ws.update(cx, |ws, cx| {
        hidden.update(cx, |e, cx| e.follow_scroll(Some((3, 0.)), Some(9.), cx));
        follower.update(cx, |e, cx| e.follow_scroll(Some((7, 0.)), None, cx));
        source.update(cx, |e, cx| e.follow_scroll(Some((20, 0.)), Some(41.), cx));
        ws.sync_scroll_from(&source, cx);
        assert_eq!(follower.read(cx).scroll_position(cx), (7, 0., 41.));
        assert_eq!(hidden.read(cx).scroll_position(cx), (3, 0., 9.));
        hidden.update(cx, |e, cx| e.follow_scroll(None, Some(80.), cx));
        ws.sync_scroll_from(&hidden, cx);
        assert_eq!(source.read(cx).scroll_position(cx), (20, 0., 41.));
    });
    cx.run_until_parked();
    ws.read_with(cx, |_, cx| assert_eq!(source.read(cx).scroll_position(cx), (20, 0., 41.)));
    dispatch(&ws, cx, ToggleSyncScroll { vertical: false });
    ws.update(cx, |ws, cx| {
        source.update(cx, |e, cx| e.follow_scroll(None, Some(73.), cx));
        ws.sync_scroll_from(&source, cx);
        assert_eq!(follower.read(cx).scroll_position(cx).2, 41.);
        assert_eq!(ws.sync_scroll, (false, false));
    });
}

#[gpui::test]
fn all_five_highlight_styles_match_select_navigate_copy_and_clear_independently(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "red blue red green");
    ws.update_in(cx, |ws, window, cx| find_text(ws, "red", false, window, cx));
    for style in 0..5 {
        dispatch(&ws, cx, HighlightMatches { style });
    }
    ws.update_in(cx, |ws, window, cx| ws.close_panel(&ClosePanel, window, cx));
    for style in 0..5 {
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_selection(4, 8)));
        dispatch(&ws, cx, editor::HighlightSelection { style });
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).state.highlights[style], vec![0..3, 4..8, 9..12])
        });
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(18)));
        for (forward, expected) in [(true, 0..3), (true, 4..8), (false, 0..3), (false, 9..12)] {
            dispatch(&ws, cx, editor::NextHighlight { style, forward });
            ws.read_with(cx, |ws, cx| {
                assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), expected)
            });
        }
        dispatch(&ws, cx, editor::CopyHighlighted { style });
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "red\nblue\nred\n");
        dispatch(&ws, cx, editor::ClearHighlights { style: Some(style) });
        ws.read_with(cx, |ws, cx| {
            let s = ws.active_editor().unwrap().read(cx).read_state(cx);
            for i in 0..5 {
                let expected = if i <= style { vec![] } else { vec![0..3, 9..12] };
                assert_eq!(s.view.highlights[i], expected, "clearing {style}, inspecting {i}");
            }
        });
    }
    for style in 0..5 {
        dispatch(&ws, cx, editor::HighlightSelection { style });
    }
    dispatch(&ws, cx, editor::ClearHighlights { style: None });
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).state.highlights.iter().all(Vec::is_empty)));
    // Empty styles must not replace the clipboard or move the selection.
    dispatch(&ws, cx, editor::CopyHighlighted { style: 4 });
    dispatch(&ws, cx, editor::NextHighlight { style: 4, forward: true });
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "red\nblue\nred\n");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), 9..12));
}

fn extraction_cases() -> [(What, bool, &'static str); 4] {
    [
        (What::Matches, false, "1\n2\n3\n"),
        (What::Matches, true, "{name}:1:3: 1\n{name}:1:7: 2\n{name}:3:3: 3\n"),
        (What::Lines, false, "a=1 b=2\nc=3\n"),
        (What::Lines, true, "{name}:1: a=1 b=2\n{name}:3: c=3\n"),
    ]
}

#[gpui::test]
fn remaining_document_extraction_options_have_exact_outputs_and_preserve_the_source(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a=1 b=2\nnone\nc=3\n");
    let source = ws.update_in(cx, |ws, window, cx| {
        let source = ws.active_editor().unwrap();
        find_text(ws, r"(?<==)\d", true, window, cx);
        source
    });
    for (what, locations, expected) in extraction_cases() {
        for to_clipboard in [true, false] {
            // These two routes already have exact assertions in search_tests.
            if (what == What::Matches && !locations && to_clipboard)
                || (what == What::Lines && locations && !to_clipboard)
            {
                continue;
            }
            ws.update_in(cx, |ws, window, cx| {
                let (g, i) = ws.views.find(&source).unwrap();
                ws.views.select(g, i);
                ws.focus_editor(window, cx);
            });
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into()));
            dispatch(&ws, cx, Extract { what, from: ExtractFrom::Document, to_clipboard, locations });
            let expected = expected.replace("{name}", "untitled");
            if to_clipboard {
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), expected);
                ws.read_with(cx, |ws, _| assert_eq!(ws.active_editor().as_ref(), Some(&source)));
            } else {
                assert_eq!(text(&ws, cx), expected);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "sentinel");
                ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).doc(cx).scratch));
            }
            source.read_with(cx, |e, cx| assert_eq!(e.doc(cx).text(), "a=1 b=2\nnone\nc=3\n"));
        }
    }
}

#[gpui::test]
fn remaining_result_extraction_options_have_exact_outputs_and_preserve_disk(cx: &mut TestAppContext) {
    let dir = temp("orlo-variant-extract-results");
    let path = write_in(&dir, "input.txt", "a=1 b=2\nnone\nc=3\n");
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.find_in_files(&FindInFiles, window, cx);
        ws.run_search(
            r"(?<==)\d",
            &dir.display().to_string(),
            None,
            FindOpts { regex: true, ..Default::default() },
            SearchFlags::default(),
            window,
            cx,
        )
        .unwrap();
    });
    settle_until(cx, |cx| {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done))
    });
    for (what, locations, expected) in extraction_cases() {
        for to_clipboard in [true, false] {
            // search_tests copies lines with locations from valid results; its plain-match copy
            // is only refused (stale results), so the one that goes through is checked here
            if what == What::Lines && locations && to_clipboard {
                continue;
            }
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into()));
            dispatch(&ws, cx, Extract { what, from: ExtractFrom::Results, to_clipboard, locations });
            settle(cx);
            let expected = expected.replace("{name}", &search::tilde(&path));
            if to_clipboard {
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), expected);
            } else {
                assert_eq!(text(&ws, cx), expected);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "sentinel");
                ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).doc(cx).scratch));
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "a=1 b=2\nnone\nc=3\n");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn smart_expansion_walks_words_and_nested_brackets_and_handles_boundaries(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let cases: [(&str, usize, &[std::ops::Range<usize>]); 4] = [
        ("fn(a [word])", 8, &[6..10, 5..11, 3..11, 2..12, 2..12]),
        ("...", 1, &[1..2, 1..2]),
        ("", 0, &[0..0, 0..0]),
        ("end", 3, &[0..3, 0..3]),
    ];
    for (input, caret, ranges) in cases {
        ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| {
                e.set_text(input, cx);
                e.sel(cx).set_cursor(caret);
            })
        });
        for expected in ranges {
            dispatch(&ws, cx, editor::ExpandSmart);
            ws.read_with(cx, |ws, cx| {
                let s = ws.active_editor().unwrap().read(cx).read_state(cx);
                assert_eq!(s.view.selections[0].range(), *expected, "{input:?}");
                assert_eq!(s.text(), input);
                assert!(!s.doc.dirty);
            });
        }
    }
}

#[gpui::test]
fn lower_case_maps_length_changing_unicode_selections_and_undo(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "A İ B");
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.sel(cx).set_selections(vec![
                crate::editing::text::Selection::new(0, 1),
                crate::editing::text::Selection::new(2, 3),
            ]);
        })
    });
    dispatch(&ws, cx, editor::ConvertCase { case: crate::editing::text::Case::Lower });
    assert_eq!(text(&ws, cx), "a i\u{307} B");
    ws.read_with(cx, |ws, cx| {
        let s = ws.active_editor().unwrap().read(cx).read_state(cx);
        assert_eq!(s.view.selections.iter().map(|s| s.range()).collect::<Vec<_>>(), [0..1, 2..4]);
    });
    dispatch(&ws, cx, editor::Undo);
    assert_eq!(text(&ws, cx), "A İ B");
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_editor().unwrap().read(cx).state.selections.iter().map(|s| s.range()).collect::<Vec<_>>(),
            [0..1, 2..3]
        )
    });
    dispatch(&ws, cx, editor::Redo);
    assert_eq!(text(&ws, cx), "a i\u{307} B");
}

#[gpui::test]
fn remaining_permutations_preserve_contents_gaps_and_undo(cx: &mut TestAppContext) {
    use crate::editing::text::{Permute, Selection};
    let (ws, cx) = window(cx);
    let input = "<bee | A | bee | cc>";
    for (op, expected) in [
        (Permute::Reverse, Some("<cc | bee | A | bee>")),
        (Permute::Sort { case_sensitive: false }, Some("<A | bee | bee | cc>")),
        (Permute::Shuffle, None),
    ] {
        ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| {
                e.set_text(input, cx);
                e.sel(cx).set_selections(vec![
                    Selection::new(1, 4),
                    Selection::new(7, 8),
                    Selection::new(11, 14),
                    Selection::new(17, 19),
                ]);
            })
        });
        dispatch(&ws, cx, editor::PermuteSelections { op });
        let output = text(&ws, cx);
        if let Some(expected) = expected {
            assert_eq!(output, expected, "{op:?}");
        }
        ws.read_with(cx, |ws, cx| {
            let s = ws.active_editor().unwrap().read(cx).read_state(cx);
            let selected: Vec<_> = s.view.selections.iter().map(|r| s.doc.rope.slice(r.range()).to_string()).collect();
            assert_eq!(output, format!("<{}>", selected.join(" | ")), "untouched separators and edges");
            let mut sorted = selected;
            sorted.sort();
            assert_eq!(sorted, ["A", "bee", "bee", "cc"], "duplicates and complete texts preserved");
        });
        if output != input {
            dispatch(&ws, cx, editor::Undo);
            assert_eq!(text(&ws, cx), input);
            dispatch(&ws, cx, editor::Redo);
            assert_eq!(text(&ws, cx), output);
        }
    }
    let input = "bee\nA\nbee\ncc";
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.set_text(input, cx)));
    dispatch(&ws, cx, editor::PermuteLines { op: Permute::Shuffle });
    let output = text(&ws, cx);
    let mut lines: Vec<_> = output.split('\n').collect();
    lines.sort();
    assert_eq!(lines, ["A", "bee", "bee", "cc"], "full line multiset preserved");
    if output != input {
        dispatch(&ws, cx, editor::Undo);
        assert_eq!(text(&ws, cx), input);
        dispatch(&ws, cx, editor::Redo);
        assert_eq!(text(&ws, cx), output);
    }
}

#[gpui::test]
fn hex_decode_action_preserves_gaps_and_restores_the_encoded_selection_on_undo(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "before C3 A9 after");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_selection(7, 12)));
    dispatch(&ws, cx, editor::Convert { codec: Codec::Hex, decode: true });
    assert_eq!(text(&ws, cx), "before é after");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), 7..8));
    dispatch(&ws, cx, editor::Undo);
    assert_eq!(text(&ws, cx), "before C3 A9 after");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), 7..12));
    dispatch(&ws, cx, editor::Redo);
    assert_eq!(text(&ws, cx), "before é after");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), 7..8));
}
