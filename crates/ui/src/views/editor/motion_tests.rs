//! Up, Down, Page Up/Down and added carets keep to the x a caret is drawn at. gpui's test text
//! system lays a char out one em wide, and a char outside the basic plane (😀, 𝄞) two: a column
//! and an x part there.

use super::test_editor as editor;
use super::*;
use crate::editing::text::Motion;
use gpui::{Entity, TestAppContext, VisualTestContext};

fn set_carets(e: &Entity<Editor>, cx: &mut VisualTestContext, carets: &[usize]) {
    e.update(cx, |e, cx| {
        let sels = carets.iter().map(|&c| Selection::caret(c)).collect();
        e.sel(cx).set_selections(sels);
    });
    cx.run_until_parked();
}

fn press(e: &Entity<Editor>, cx: &mut VisualTestContext, motion: Motion, select: bool) {
    e.update_in(cx, |e, window, cx| {
        assert!(e.perform_action(&Move { motion, select }, window, cx));
    });
    cx.run_until_parked();
}

fn heads(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<usize> {
    e.read_with(cx, |e, _| e.state.selections.iter().map(|s| s.head).collect())
}

/// Where `offset` is drawn in the last frame, from the text's left edge.
fn x_of(e: &Entity<Editor>, cx: &mut VisualTestContext, offset: usize) -> Pixels {
    e.read_with(cx, |e, cx| {
        let left = e.layout.as_ref().unwrap().text_left;
        e.char_bounds(offset, cx).expect("on screen").left() - left
    })
}

/// One em, as the test text system lays a char out.
fn em(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Pixels {
    x_of(e, cx, 1) - x_of(e, cx, 0)
}

#[gpui::test]
fn up_and_down_keep_the_x_across_wide_chars(cx: &mut TestAppContext) {
    // after 😀😀 is x 4: the fifth column below, not the third; on "𝄞x" (x 2 and 3) the end
    let text = "😀😀ab\nabcdefgh\n𝄞x\nabcdefgh";
    let (e, cx) = editor(text, cx, |_, _| {});
    set_carets(&e, cx, &[2]);
    let mut seen = Vec::new();
    for motion in [Motion::Down, Motion::Down, Motion::Down, Motion::Up] {
        press(&e, cx, motion, false);
        seen.extend(heads(&e, cx));
    }
    assert_eq!(seen, [5 + 4, 14 + 2, 17 + 4, 14 + 2]);
    press(&e, cx, Motion::Up, false);
    press(&e, cx, Motion::Up, false);
    assert_eq!(heads(&e, cx), [2]);
}

#[gpui::test]
fn horizontal_scroll_does_not_move_the_x(cx: &mut TestAppContext) {
    let long = "a".repeat(300);
    let text = format!("😀😀ab{long}\nabcdefgh{long}\n𝄞x");
    let mut landed = Vec::new();
    for scroll in [0., 37.] {
        let mut app = cx.clone();
        let (e, cx) = editor(&text, &mut app, |_, _| {});
        e.update(cx, |e, cx| {
            e.scroll_left = scroll;
            cx.notify();
        });
        cx.run_until_parked();
        set_carets(&e, cx, &[2]);
        let mut seen = Vec::new();
        for _ in 0..2 {
            press(&e, cx, Motion::Down, false);
            seen.extend(heads(&e, cx));
        }
        landed.push(seen);
    }
    let line1 = 4 + 300 + 1;
    assert_eq!(landed[0], [line1 + 4, line1 + 8 + 300 + 1 + 2]);
    assert_eq!(landed[0], landed[1]);
}

#[gpui::test]
fn tabs_count_their_width_and_carets_land_on_clusters(app: &mut TestAppContext) {
    {
        // after "\t😀" is x 6 (a tab to column 4, then two)
        let (e, cx) = editor("\t😀x\nabcdefghij", app, |_, _| {});
        set_carets(&e, cx, &[2]);
        press(&e, cx, Motion::Down, false);
        assert_eq!(heads(&e, cx), [4 + 6]);
        press(&e, cx, Motion::Up, false);
        assert_eq!(heads(&e, cx), [2]);
    }
    // x 1 is between e and its accent: the caret goes to one side of the cluster
    let (e, cx) = editor("a\ne\u{301}e\u{301}", app, |_, _| {});
    set_carets(&e, cx, &[1]);
    press(&e, cx, Motion::Down, false);
    let h = heads(&e, cx)[0];
    assert!(h == 2 || h == 4, "{h} is inside e\u{301}");
}

#[gpui::test]
fn several_carets_and_extended_selections_keep_their_own_x(cx: &mut TestAppContext) {
    let (e, cx) = editor("😀😀😀ab\nabcdefghij", cx, |_, _| {});
    set_carets(&e, cx, &[1, 3]);
    press(&e, cx, Motion::Down, false);
    assert_eq!(heads(&e, cx), [6 + 2, 6 + 6]);
    set_carets(&e, cx, &[2]);
    press(&e, cx, Motion::Down, true);
    let range = e.read_with(cx, |e, _| e.state.selections[0].range());
    assert_eq!(range, 2..6 + 4);
}

#[gpui::test]
fn adding_a_caret_below_keeps_the_x(cx: &mut TestAppContext) {
    let (e, cx) = editor("😀😀ab\nabcdefgh", cx, |_, _| {});
    set_carets(&e, cx, &[2]);
    e.update_in(cx, |e, window, cx| {
        assert!(e.perform_action(&SelectLines { forward: true }, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(heads(&e, cx), [2, 5 + 4]);
}

#[gpui::test]
fn a_fold_placeholder_is_measured_as_drawn(cx: &mut TestAppContext) {
    // "ab0123456789cd" with 0123456789 folded shows "ab", the placeholder, "cd": after c is x 6
    let (e, cx) = editor("ab0123456789cd\n😀😀😀😀😀😀😀😀", cx, |e, cx| {
        let doc = e.document.read(cx);
        e.folds.fold(&doc.model, 2..12);
    });
    set_carets(&e, cx, &[13]);
    assert_eq!(x_of(&e, cx, 13), em(&e, cx) * 6.);
    press(&e, cx, Motion::Down, false);
    assert_eq!(heads(&e, cx), [15 + 3]);
    press(&e, cx, Motion::Up, false);
    assert_eq!(heads(&e, cx), [13]);
}

#[gpui::test]
fn filler_rows_are_stepped_over_at_the_x(cx: &mut TestAppContext) {
    let (e, cx) = editor("😀😀ab\nabcdefgh", cx, |e, cx| {
        e.fillers = Fillers::new(e.doc(cx), [(1, 2)]);
    });
    set_carets(&e, cx, &[2]);
    press(&e, cx, Motion::Down, false);
    assert_eq!(heads(&e, cx), [5 + 4]);
    press(&e, cx, Motion::Up, false);
    assert_eq!(heads(&e, cx), [2]);
}

#[gpui::test]
fn a_wrapped_rows_indent_counts_toward_its_x(cx: &mut TestAppContext) {
    // the second line wraps into rows indented 4: "    abcd ", then "efgh ", "ijkl ", ...
    let text = "😀😀😀ab\n    abcd efgh ijkl mnop qrst";
    let (e, cx) = editor(text, cx, |e, _| {
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(12);
    });
    let rows = e.read_with(cx, |e, cx| {
        let rows = e.rows();
        (0..rows.rows(e.doc(cx))).map(|r| rows.row_bounds(e.doc(cx), DisplayRow(r)).unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(rows[..3], [0..5, 6..15, 15..20]);
    // x 6, 7 and 8 on the continuation row are its columns 6, 7 and 8, its 3rd to 5th chars
    // (by columns, 3 to 5 are all inside its indent, at its start)
    for (from, want) in [(3, 15 + 2), (4, 15 + 3), (5, 15 + 4)] {
        set_carets(&e, cx, &[from]);
        press(&e, cx, Motion::Down, false);
        press(&e, cx, Motion::Down, false);
        assert_eq!(heads(&e, cx), [want], "from {from}");
        let (landed, aimed) = (x_of(&e, cx, want), x_of(&e, cx, from));
        assert!((landed - aimed).abs() < px(0.01), "{landed:?} for {aimed:?}");
    }
}

#[gpui::test]
fn page_down_measures_rows_not_drawn_yet(cx: &mut TestAppContext) {
    // the caret on the last drawn row; a page below is a row no frame has shaped
    let mut text = String::new();
    for i in 0..60 {
        text.push_str(if i % 2 == 0 { "😀😀ab\n" } else { "abcdefgh\n" });
    }
    let (e, cx) = editor(&text, cx, |_, _| {});
    let (first, drawn, page) = e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        (l.first_row, l.rows.len(), e.page_lines())
    });
    assert_eq!(first, 0);
    // an even line: after 😀😀 at x 4
    let row = (drawn - 1) & !1;
    let start = |line: usize| (line / 2) * 14 + (line % 2) * 5;
    set_carets(&e, cx, &[start(row) + 2]);
    let target = row + page;
    assert!(target >= drawn, "{target} is drawn");
    press(&e, cx, Motion::PageDown(0), false);
    let want = match target % 2 {
        // the same x on the same kind of line
        0 => start(target) + 2,
        _ => start(target) + 4,
    };
    assert_eq!(heads(&e, cx), [want]);
}

#[gpui::test]
fn subword_actions_skip_folds_cross_wraps_and_commit_native_composition(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (e, cx) = editor("camelCase HTTPServer", app, |e, cx| {
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(6);
        e.folds.fold(&e.document.read(cx).model, 2..8);
    });
    set_carets(&e, cx, &[0]);
    press(&e, cx, Motion::SubwordRight, false);
    assert_eq!(heads(&e, cx), [8], "skip hidden subword boundary");
    press(&e, cx, Motion::SubwordLeft, false);
    assert_eq!(heads(&e, cx), [2]);
    set_carets(&e, cx, &[10]);
    press(&e, cx, Motion::SubwordRight, true);
    assert_eq!(heads(&e, cx), [14], "source boundary across wrapped rows");
    e.update_in(cx, |e, window, cx| {
        e.sel(cx).set_cursor(20);
        e.replace_and_mark_text_in_range(None, "é", None, window, cx);
        assert!(!e.marked.is_empty());
        e.state.overtype = true;
        assert!(e.perform_action(&Move::to(Motion::SubwordLeft, false), window, cx));
        assert!(e.marked.is_empty(), "motion commits native preview");
        assert_eq!(e.text(cx), "camelCase HTTPServeré");
        assert!(e.state.overtype);
        assert!(e.perform_action(&DeleteSubword { forward: true }, window, cx));
        assert_eq!(e.text(cx), "camelCase HTTP");
        assert!(e.state.overtype, "explicit deletion preserves keyboard mode");
        assert!(e.perform_action(&Undo, window, cx));
        assert_eq!(e.text(cx), "camelCase HTTPServeré");
        let end = e.doc(cx).len();
        e.sel(cx).set_cursor(end);
        e.replace_and_mark_text_in_range(None, "中", None, window, cx);
        assert!(!e.marked.is_empty());
        assert!(e.perform_action(&DeleteSubword { forward: false }, window, cx));
        assert!(e.marked.is_empty(), "deletion commits native preview through edit_text");
        assert_eq!(e.text(cx), "camelCase HTTP");
        assert!(e.perform_action(&Undo, window, cx));
        assert_eq!(e.text(cx), "camelCase HTTPServeré中");
    });
}

#[gpui::test]
fn prose_source_motions_cross_wraps_skip_folds_and_commit_ime(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (e, cx) = editor("One. Two!\n\nThree tail", app, |e, cx| {
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(6);
        e.folds.fold(&e.document.read(cx).model, 2..7);
    });
    set_carets(&e, cx, &[0]);
    press(&e, cx, Motion::SentenceNext, false);
    assert_eq!(heads(&e, cx), [7], "hidden sentence boundary skips to fold end");
    press(&e, cx, Motion::SentencePrevious, false);
    assert_eq!(heads(&e, cx), [2], "reverse skips to fold start");
    press(&e, cx, Motion::ParagraphNext, true);
    assert_eq!(heads(&e, cx), [11], "paragraph is source based across wrapped rows");
    e.update_in(cx, |e, window, cx| {
        e.sel(cx).set_cursor(21);
        e.replace_and_mark_text_in_range(None, "中", None, window, cx);
        assert!(!e.marked.is_empty());
        assert!(e.perform_action(&Move::to(Motion::SentencePrevious, false), window, cx));
        assert!(e.marked.is_empty());
        assert_eq!(e.text(cx), "One. Two!\n\nThree tail中");
        assert_eq!(e.state.cursor(), 11);
        e.replace_and_mark_text_in_range(None, "é", None, window, cx);
        assert!(e.perform_action(&DeleteSentence, window, cx));
        assert!(e.marked.is_empty());
        assert_eq!(e.text(cx), "One. Two!\n\n");
        assert!(e.perform_action(&Undo, window, cx));
        assert_eq!(e.text(cx), "One. Two!\n\néThree tail中");
    });
}

#[gpui::test]
fn prose_viewer_follow_refuse_delete_before_ime_but_allow_selection(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for follow in [false, true] {
        let mut app = app.clone();
        let (e, cx) = editor("One. Two!", &mut app, |_, _| {});
        e.update_in(cx, |e, window, cx| {
            e.sel(cx).set_cursor(6);
            e.replace_and_mark_text_in_range(None, "中", None, window, cx);
            e.update_doc(cx, |d| d.set_following(follow));
            e.input_enabled = follow;
            let before = (e.text(cx), e.state.selections.clone(), e.doc(cx).version, e.marked.clone());
            assert!(!e.marked.is_empty());
            assert!(e.perform_action(&DeleteSentence, window, cx));
            assert_eq!((e.text(cx), e.state.selections.clone(), e.doc(cx).version, e.marked.clone()), before);
            assert!(e.perform_action(&Move::to(Motion::SentencePrevious, false), window, cx));
            assert_eq!(e.state.cursor(), 5);
            assert!(e.marked.is_empty(), "explicit motion commits preview in read-only views");
            assert!(e.perform_action(&SelectSentence, window, cx));
            assert_eq!(e.state.selections, [Selection::new(5, 10)]);
            assert_eq!(e.text(cx), before.0);
            assert_eq!(e.doc(cx).version, before.2);
        });
    }
    let (e, cx) = editor("One. Two!", app, |_, _| {});
    e.update_in(cx, |e, window, cx| {
        e.update_doc(cx, |d| d.read_only = true);
        e.sel(cx).set_cursor(6);
        assert!(e.perform_action(&DeleteSentence, window, cx));
        assert_eq!(e.text(cx), "One. ", "filesystem protected working copy remains editable");
    });
}

#[gpui::test]
fn n06_transpose_words_viewer_follow_overlap_missing_and_identical_keep_ime(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in ["viewer", "follow", "overlap", "missing", "identical", "virtual"] {
        let mut app = app.clone();
        let source = if mode == "identical" { "same same tail" } else { "alpha beta gamma" };
        let (e, cx) = editor(source, &mut app, |_, _| {});
        e.update_in(cx, |e, window, cx| {
            e.sel(cx).set_cursor(source.chars().count() - 1);
            e.replace_and_mark_text_in_range(None, "中", None, window, cx);
            match mode {
                "viewer" => e.input_enabled = false,
                "follow" => {
                    e.update_doc(cx, |d| d.set_following(true));
                }
                "overlap" => e.sel(cx).set_selections(vec![Selection::caret(1), Selection::caret(2)]),
                "identical" => e.sel(cx).set_cursor(1),
                "virtual" => {
                    e.sel(cx).set_virtual_space(true);
                    let end = e.doc(cx).len();
                    e.sel(cx).set_selections(vec![Selection::from_points((end, 2), (end, 4))]);
                }
                _ => {}
            }
            let before = (
                e.text(cx),
                e.state.selections.clone(),
                e.doc(cx).version,
                e.marked.clone(),
                e.read_state(cx).selection_generation(),
            );
            assert!(!e.marked.is_empty());
            assert!(e.perform_action(&TransposeWords, window, cx));
            assert_eq!(
                (
                    e.text(cx),
                    e.state.selections.clone(),
                    e.doc(cx).version,
                    e.marked.clone(),
                    e.read_state(cx).selection_generation()
                ),
                before,
                "{mode}"
            );
        });
    }
}

#[gpui::test]
fn n06_transpose_words_uses_source_across_wrap_folds_commits_ime_and_edits_readonly_buffer(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (e, cx) = editor("alpha, beta", app, |e, cx| {
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(4);
        e.folds.fold(&e.document.read(cx).model, 3..9);
    });
    e.update_in(cx, |e, window, cx| {
        e.sel(cx).set_cursor(1);
        e.replace_and_mark_text_in_range(None, "中", None, window, cx);
        e.update_doc(cx, |d| d.read_only = true);
        let original = e.state.selections.clone();
        assert!(!e.marked.is_empty());
        assert!(e.perform_action(&TransposeWords, window, cx));
        assert_eq!(e.text(cx), "beta, a中lpha");
        assert!(e.marked.is_empty());
        assert!(e.perform_action(&Undo, window, cx));
        assert_eq!(e.text(cx), "a中lpha, beta");
        assert_eq!(e.state.selections, original);
        assert!(e.perform_action(&Redo, window, cx));
        assert_eq!(e.text(cx), "beta, a中lpha");
    });
}

#[gpui::test]
fn n07_surrounds_refusals_and_noops_preserve_ime_and_full_source_state(app: &mut TestAppContext) {
    use crate::editing::text::Surround;
    use gpui::EntityInputHandler;
    for mode in ["viewer", "follow", "overlap", "missing", "identical", "virtual"] {
        let mut app = app.clone();
        let (e, cx) = editor("((value)) tail", &mut app, |_, _| {});
        e.update_in(cx, |e, window, cx| {
            let end = e.doc(cx).len();
            e.sel(cx).set_cursor(end);
            e.replace_and_mark_text_in_range(None, "中", None, window, cx);
            e.sel(cx).set_cursor(3);
            match mode {
                "viewer" => e.input_enabled = false,
                "follow" => {
                    e.update_doc(cx, |d| d.set_following(true));
                }
                "overlap" => e.sel(cx).set_selections(vec![Selection::caret(0), Selection::caret(3)]),
                "missing" => {
                    let at = e.doc(cx).len();
                    e.sel(cx).set_cursor(at);
                }
                "virtual" => {
                    e.sel(cx).set_virtual_space(true);
                    let at = e.doc(cx).len();
                    e.sel(cx).set_selections(vec![Selection::from_points((at, 2), (at, 4))]);
                }
                _ => {}
            }
            let before = (
                e.text(cx),
                e.state.selections.clone(),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.marked.clone(),
                e.read_state(cx).selection_generation(),
            );
            assert!(!e.marked.is_empty());
            assert!(e.perform_action(
                &ChangeSurround {
                    target: Surround::Round,
                    replacement: if mode == "identical" { Surround::Round } else { Surround::Square },
                    count: std::num::NonZeroUsize::MIN
                },
                window,
                cx
            ));
            assert_eq!(
                (
                    e.text(cx),
                    e.state.selections.clone(),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.marked.clone(),
                    e.read_state(cx).selection_generation()
                ),
                before,
                "{mode}"
            );
        });
    }
}

#[gpui::test]
fn n07_surrounds_use_source_with_folds_wrap_ime_and_filesystem_readonly(app: &mut TestAppContext) {
    use crate::editing::text::Surround;
    use gpui::EntityInputHandler;
    let (e, cx) = editor("(value)", app, |e, cx| {
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(3);
        e.folds.fold(&e.document.read(cx).model, 2..5);
    });
    e.update_in(cx, |e, window, cx| {
        e.sel(cx).set_cursor(1);
        e.replace_and_mark_text_in_range(None, "中", None, window, cx);
        e.update_doc(cx, |d| d.read_only = true);
        let original = e.state.selections.clone();
        assert!(e.perform_action(
            &ChangeSurround {
                target: Surround::Round,
                replacement: Surround::Square,
                count: std::num::NonZeroUsize::MIN
            },
            window,
            cx
        ));
        assert_eq!(e.text(cx), "[中value]");
        assert!(e.marked.is_empty());
        e.perform_action(&Undo, window, cx);
        assert_eq!(e.text(cx), "(中value)");
        assert_eq!(e.state.selections, original);
        e.perform_action(&Redo, window, cx);
        assert_eq!(e.text(cx), "[中value]");
    });
}

#[gpui::test]
fn n08_selection_actions_allow_viewer_follow_readonly_and_preserve_ime(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in ["viewer", "follow", "readonly", "ime"] {
        let mut app = app.clone();
        let (e, cx) = editor("a,b,c", &mut app, |_, _| {});
        e.update_in(cx, |e, w, cx| {
            e.sel(cx).set_selection(0, 5);
            match mode {
                "viewer" => e.input_enabled = false,
                "follow" => {
                    e.update_doc(cx, |d| d.set_following(true));
                }
                "readonly" => {
                    e.update_doc(cx, |d| d.read_only = true);
                }
                _ => e.replace_and_mark_text_in_range(None, "中", None, w, cx),
            }
            let before = (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.marked.clone());
            let original = e.state.selections.clone();
            for a in [
                Box::new(InvertSelections) as Box<dyn gpui::Action>,
                Box::new(SplitSelectionsByRegex { pattern: ",".into(), ignore_case: false }),
                Box::new(KeepSelectionsMatchingRegex { pattern: "a".into(), ignore_case: false }),
                Box::new(RemoveMatchingSelections { pattern: "c".into(), ignore_case: false }),
            ] {
                e.perform_action(&*a, w, cx);
                assert_eq!((e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.marked.clone()), before);
            }
            if mode == "ime" {
                assert_eq!(e.state.selections, original);
            } else {
                assert_ne!(e.state.selections, original);
            }
        });
    }
}

#[gpui::test]
fn n09_failures_and_noops_preserve_local_ime_identity_selections_and_history(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in ["viewer", "follow", "invalid", "overflow", "zero", "virtual"] {
        let mut app = app.clone();
        let (e, cx) = editor("009 bad", &mut app, |_, _| {});
        e.update_in(cx, |e, w, cx| {
            let end = e.doc(cx).len();
            e.sel(cx).set_cursor(end);
            e.replace_and_mark_text_in_range(None, "中", None, w, cx);
            e.sel(cx).set_selection(0, 3);
            let step = if mode == "zero" {
                0
            } else if mode == "overflow" {
                i128::MAX
            } else {
                1
            };
            match mode {
                "viewer" => e.input_enabled = false,
                "follow" => {
                    e.update_doc(cx, |d| d.set_following(true));
                }
                "invalid" => e.sel(cx).set_selections(vec![Selection::new(0, 3), Selection::new(4, 7)]),
                "virtual" => {
                    e.sel(cx).set_virtual_space(true);
                    let at = e.doc(cx).len();
                    e.sel(cx).set_selections(vec![Selection::from_points((at, 2), (at, 4))]);
                }
                _ => {}
            }
            let before = (
                e.text(cx),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.marked.clone(),
            );
            e.perform_action(&AdjustValues { step: crate::editing::columns::ValueStep(step) }, w, cx);
            assert!(
                (
                    e.text(cx),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.marked.clone()
                ) == before,
                "{mode}"
            );
        });
    }
}

#[gpui::test]
fn n09_success_commits_local_ime_preserves_virtual_provenance_and_ignores_overtype(app: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (e, cx) = editor("009", app, |_, _| {});
    e.update_in(cx, |e, w, cx| {
        e.sel(cx).set_cursor(3);
        e.replace_and_mark_text_in_range(None, "中", None, w, cx);
        e.sel(cx).set_virtual_space(true);
        let mut value = Selection::new(3, 0);
        value.rectangular = true;
        e.sel(cx).set_selections(vec![value, Selection::from_points((4, 2), (4, 5))]);
        e.update_doc(cx, |d| d.read_only = true);
        e.state.overtype = true;
        let original = e.read_state(cx).selection_state();
        e.perform_action(&AdjustValues { step: crate::editing::columns::ValueStep(1) }, w, cx);
        assert_eq!(e.text(cx), "010中");
        assert!(e.marked.is_empty());
        assert!(e.state.overtype);
        e.perform_action(&Undo, w, cx);
        assert_eq!(e.text(cx), "009中");
        assert!(e.read_state(cx).selection_state() == original);
        e.perform_action(&Redo, w, cx);
        assert_eq!(e.text(cx), "010中");
    });
}
