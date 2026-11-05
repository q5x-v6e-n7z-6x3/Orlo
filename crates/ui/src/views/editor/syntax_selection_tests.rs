//! Real action dispatch against the existing parser/highlighter and shared document owners.
#![allow(clippy::single_range_in_vec_init)]
use super::*;
use gpui::{TestAppContext, VisualTestContext};

fn parsed(e: &mut Editor, cx: &mut Context<Editor>) {
    let rope = e.doc(cx).rope.clone();
    let job = highlight::run_job(e.highlighter.job(0), &rope, &[0, rope.len_lines() - 1]);
    e.highlighter.adopt(job, &rope);
}

fn editor<'a>(text: &str, cx: &'a mut TestAppContext) -> (Entity<Editor>, &'a mut VisualTestContext) {
    test_editor(text, cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Rust");
        parsed(e, cx);
    })
}

fn selections(e: &Entity<Editor>, cx: &VisualTestContext) -> Vec<Selection> {
    e.read_with(cx, |e, _| e.state.selections.clone())
}

#[gpui::test]
fn syntax_selection_actions_expand_identifier_expression_statement_and_reverse(cx: &mut TestAppContext) {
    let text = "fn main() { café + 雪; }";
    let (e, cx) = editor(text, cx);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(13));
    let before = e.read_with(cx, |e, cx| (e.doc(cx).version, e.doc(cx).dirty, e.doc(cx).save_point()));
    for (start, end) in [(12, 16), (12, 20), (12, 21)] {
        run(&e, cx, &ExpandSyntaxSelection);
        assert_eq!(selections(&e, cx), [Selection::new(start, end)]);
    }
    for (start, end) in [(12, 20), (12, 16), (13, 13)] {
        run(&e, cx, &ShrinkSyntaxSelection);
        assert_eq!(selections(&e, cx), [Selection::new(start, end)]);
    }
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), text);
    assert_eq!(e.read_with(cx, |e, cx| (e.doc(cx).version, e.doc(cx).dirty, e.doc(cx).save_point())), before);
    run(&e, cx, &Undo);
    assert_eq!(e.read_with(cx, |e, cx| e.doc(cx).version), before.0, "no text undo entry");
}

#[gpui::test]
fn syntax_selection_overlap_shrink_preserves_reversal_virtual_goal_and_primary(cx: &mut TestAppContext) {
    let (e, cx) = editor("fn main() { café + 雪; }", cx);
    let original = e.update(cx, |e, cx| {
        e.state.virtual_space = true.into();
        let mut reverse = Selection::new(16, 12);
        reverse.goal = Some(crate::editing::text::Goal::X(23.));
        e.sel(cx).set_selections(vec![reverse, Selection::caret(19)]);
        e.read_state(cx).selection_state()
    });
    run(&e, cx, &ExpandSyntaxSelection);
    assert_eq!(selections(&e, cx).len(), 1, "overlapping node ranges normalize");
    run(&e, cx, &ShrinkSyntaxSelection);
    assert!(e.read_with(cx, |e, cx| e.read_state(cx).selection_state()) == original);
    e.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::from_points((22, 4), (22, 1))]));
    let original = selections(&e, cx);
    run(&e, cx, &ExpandSyntaxSelection);
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), original);
}

#[gpui::test]
fn syntax_selection_invalidates_after_caret_away_back_selection_undo_text_and_syntax_changes(cx: &mut TestAppContext) {
    let (e, cx) = editor("fn main() { café + 雪; }", cx);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(13));
    run(&e, cx, &ExpandSyntaxSelection);
    let expanded = selections(&e, cx);
    run(&e, cx, &Move::to(crate::editing::text::Motion::Right, false));
    e.update(cx, |e, cx| e.sel(cx).set_selections(expanded.clone()));
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), expanded, "away and back must invalidate");
    run(&e, cx, &ExpandSyntaxSelection);
    run(&e, cx, &UndoSelection);
    run(&e, cx, &RedoSelection);
    let now = selections(&e, cx);
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), now, "ordinary undo/redo is unrelated movement");
    run(&e, cx, &ExpandSyntaxSelection);
    e.update(cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Python");
        e.highlighter = Highlighter::by_name("Rust");
        parsed(e, cx);
    });
    let now = selections(&e, cx);
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), now, "syntax away and back must invalidate");
    run(&e, cx, &ExpandSyntaxSelection);
    e.update(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
    let now = selections(&e, cx);
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), now);
}

#[gpui::test]
fn syntax_selection_stale_worker_cannot_move_carets_or_revalidate_history(cx: &mut TestAppContext) {
    let (e, cx) = editor("fn main() { café + 雪; }", cx);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(13));
    run(&e, cx, &ExpandSyntaxSelection);
    e.update_in(cx, |e, window, cx| {
        let old_rope = e.doc(cx).rope.clone();
        let old_job = highlight::run_job(e.highlighter.job(0), &old_rope, &[0]);
        e.edit(cx, |s| s.insert("changed"));
        e.highlighter.adopt(old_job, &e.doc(cx).rope);
        let before = e.state.selections.clone();
        e.perform_action(&ExpandSyntaxSelection, window, cx);
        assert_eq!(e.state.selections, before);
        assert!(e.syntax_selection.is_none());
        e.sel(cx).set_cursor(0);
        // Worker completion publishes colors only, never an originating selection.
        parsed(e, cx);
        assert_eq!(e.state.selections, [Selection::caret(0)]);
        e.perform_action(&ShrinkSyntaxSelection, window, cx);
        assert_eq!(e.state.selections, [Selection::caret(0)]);
    });
}

#[gpui::test]
fn syntax_selection_unavailable_and_empty_are_unchanged_and_report_status(cx: &mut TestAppContext) {
    let (e, cx) = editor("", cx);
    let said = Rc::new(RefCell::new(Vec::new()));
    let sink = said.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&e, move |_, event: &EditorEvent, _| {
            if let EditorEvent::Status(s) = event {
                sink.borrow_mut().push(s.clone());
            }
        })
    });
    run(&e, cx, &ExpandSyntaxSelection);
    run(&e, cx, &ShrinkSyntaxSelection);
    e.update(cx, |e, _| e.highlighter = Highlighter::by_name("Plain Text"));
    run(&e, cx, &ExpandSyntaxSelection);
    assert_eq!(selections(&e, cx), [Selection::caret(0)]);
    assert!(said.borrow().iter().any(|s| s.contains("No larger")));
    assert!(said.borrow().iter().any(|s| s.contains("No syntax selection")));
    assert!(said.borrow().iter().any(|s| s.contains("unavailable")));
}

#[gpui::test]
fn syntax_selection_immutable_composition_fold_and_wrap_use_selection_command_policy(cx: &mut TestAppContext) {
    let (e, cx) = editor(
        "fn main() {
    café + 雪;
}
",
        cx,
    );
    e.update(cx, |e, cx| {
        e.input_enabled = false;
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(10);
        e.fold_lines(0, 2, cx);
        e.sel(cx).set_cursor(17);
        e.marked = vec![17..18];
    });
    run(&e, cx, &ExpandSyntaxSelection);
    assert_eq!(selections(&e, cx), [Selection::new(16, 20)]);
    e.read_with(cx, |e, cx| {
        assert!(e.marked.is_empty());
        assert!(e.hidden_lines(cx).is_empty());
        assert!(e.wrap.is_some());
        assert_eq!(
            e.text(cx),
            "fn main() {
    café + 雪;
}
"
        );
    });
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), [Selection::caret(17)]);
}

#[gpui::test]
fn syntax_selection_follow_and_read_only_sources_select_without_text_changes(cx: &mut TestAppContext) {
    let (e, cx) = editor("fn main() { café + 雪; }", cx);
    e.update(cx, |e, cx| {
        e.update_doc(cx, |d| {
            d.read_only = true;
            d.set_following(true);
        });
        e.catch_up(cx);
        parsed(e, cx);
        e.sel(cx).set_cursor(13);
    });
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    run(&e, cx, &ExpandSyntaxSelection);
    assert_eq!(selections(&e, cx), [Selection::new(12, 16)]);
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), [Selection::caret(13)]);
    e.read_with(cx, |e, cx| {
        assert_eq!(e.doc(cx).version, version);
        assert!(!e.doc(cx).dirty);
        assert_eq!(e.text(cx), "fn main() { café + 雪; }");
    });
}

#[gpui::test]
fn syntax_selection_overtaken_syntax_worker_cannot_restore_a_previous_parser_identity(cx: &mut TestAppContext) {
    let (e, cx) = editor("fn main() { café + 雪; }", cx);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(13));
    run(&e, cx, &ExpandSyntaxSelection);
    e.update_in(cx, |e, window, cx| {
        let rope = e.doc(cx).rope.clone();
        let old_job = highlight::run_job(e.highlighter.job(0), &rope, &[0]);
        e.highlighter = Highlighter::by_name("Python");
        e.highlighter = Highlighter::by_name("Rust");
        e.highlighter.adopt(old_job, &rope);
        e.perform_action(&ShrinkSyntaxSelection, window, cx);
        assert_eq!(e.state.selections, [Selection::new(12, 16)], "old work cannot resurrect a shrink stack");
    });
}

#[gpui::test]
fn syntax_selection_workspace_selection_away_back_without_a_frame_invalidates_shrink(cx: &mut TestAppContext) {
    let (e, cx) = editor("fn main() { café + 雪; }", cx);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(13));
    run(&e, cx, &ExpandSyntaxSelection);
    e.update_in(cx, |e, window, cx| {
        e.select_range(0..0, cx);
        e.select_range(12..16, cx);
        e.perform_action(&ShrinkSyntaxSelection, window, cx);
        assert_eq!(e.state.selections, [Selection::new(12, 16)]);
    });
}

#[gpui::test]
fn syntax_selection_multicarets_at_different_depths_expand_independently_and_reverse(cx: &mut TestAppContext) {
    let text = "fn main() { café + 雪; abc + def; }";
    let abc = text[..text.find("abc").unwrap()].chars().count();
    let (e, cx) = editor(text, cx);
    e.update(cx, |e, cx| {
        e.sel(cx).set_selections(vec![Selection::new(12, 16), Selection::caret(abc + 1)]);
    });
    let before = selections(&e, cx);
    run(&e, cx, &ExpandSyntaxSelection);
    assert_eq!(selections(&e, cx), [Selection::new(12, 20), Selection::new(abc, abc + 3)]);
    let first = selections(&e, cx);
    run(&e, cx, &ExpandSyntaxSelection);
    assert_eq!(selections(&e, cx), [Selection::new(12, 21), Selection::new(abc, abc + 9)]);
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), first);
    run(&e, cx, &ShrinkSyntaxSelection);
    assert_eq!(selections(&e, cx), before);
}

#[gpui::test]
fn syntax_selection_native_composition_commits_without_rewriting_or_adding_text_undo(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (e, cx) = editor("fn main() { ca + 雪; }", cx);
    e.update_in(cx, |e, window, cx| {
        e.sel(cx).set_cursor(14);
        e.replace_and_mark_text_in_range(None, "fé", Some(0..0), window, cx);
        assert!(!e.marked.is_empty());
        assert_eq!(e.text(cx), "fn main() { café + 雪; }");
        parsed(e, cx);
        let version = e.doc(cx).version;
        let before = e.state.selections.clone();
        e.perform_action(&ExpandSyntaxSelection, window, cx);
        assert_eq!(e.state.selections, [Selection::new(12, 16)]);
        assert!(e.marked_text_range(window, cx).is_none());
        assert_eq!(e.doc(cx).version, version);
        e.perform_action(&ShrinkSyntaxSelection, window, cx);
        assert_eq!(e.state.selections, before);
        assert_eq!(e.doc(cx).version, version);
        assert_eq!(e.text(cx), "fn main() { café + 雪; }");
        e.perform_action(&Undo, window, cx);
        assert_eq!(e.text(cx), "fn main() { ca + 雪; }", "one native preview undo, no selection undo entry");
    });
}

#[gpui::test]
fn syntax_selection_commands_close_the_previous_typing_group(cx: &mut TestAppContext) {
    let (e, cx) = editor("fn main() { ca + 雪; }", cx);
    e.update_in(cx, |e, window, cx| {
        e.sel(cx).set_cursor(14);
        e.edit(cx, |s| s.type_text("fé"));
        parsed(e, cx);
        e.perform_action(&ExpandSyntaxSelection, window, cx);
        e.perform_action(&ShrinkSyntaxSelection, window, cx);
        e.edit(cx, |s| s.type_text("X"));
        e.perform_action(&Undo, window, cx);
        assert_eq!(e.text(cx), "fn main() { café + 雪; }", "selection commands separate later typing");
    });
}
