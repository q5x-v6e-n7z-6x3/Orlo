use super::*;

#[test]
fn duplicate_and_delete_line() {
    let mut s = st("a\nb", 0);
    s.duplicate_line();
    assert_eq!(s.text(), "a\na\nb");
    assert_eq!(s.cursor(), 2);
    s.delete_line();
    assert_eq!(s.text(), "a\nb");
}

#[test]
fn move_line_edges_are_noops_and_middle_moves() {
    let mut s = st("a\nb\nc", 0);
    s.move_line_up();
    assert_eq!(s.text(), "a\nb\nc");
    s.move_line_down();
    assert_eq!(s.text(), "b\na\nc");
    assert_eq!(s.cursor(), 2);
    s.view.selections = vec![Selection::caret(4)];
    s.move_line_down();
    assert_eq!(s.text(), "b\na\nc");
}

#[test]
fn indent_unindent_multi_line_selection() {
    let mut s = selecting("a\nb", &[(0, 3)]);
    s.tab();
    assert_eq!(s.text(), "    a\n    b");
    s.unindent();
    assert_eq!(s.text(), "a\nb");
}

#[test]
fn tab_inserts_spaces_on_caret() {
    let mut s = st("a", 1);
    s.tab();
    assert_eq!(s.text(), "a   ");
}

#[test]
fn toggle_comment_round_trip() {
    let mut s = selecting("  x\n\n  y", &[(0, 8)]);
    s.toggle_comment("//");
    assert_eq!(s.text(), "  // x\n\n  // y");
    s.toggle_comment("//");
    assert_eq!(s.text(), "  x\n\n  y");
}

#[test]
fn toggle_comment_only_treats_spaces_and_tabs_as_indent() {
    let mut s = st("\u{a0}// x", 0);
    s.toggle_comment("//");
    assert_eq!(s.text(), "// \u{a0}// x");
}

#[test]
fn copy_empty_selection_takes_line_and_paste_per_cursor() {
    let s = st("ab\ncd", 1);
    assert_eq!(s.copy_text(), ("ab\n".to_string(), true));
    let mut s = carets("\n", &[0, 1]);
    s.paste("x\ny", false);
    assert_eq!(s.text(), "x\ny");
}

#[test]
fn cut_whole_line() {
    let mut s = st("ab\ncd", 1);
    assert_eq!(s.cut(), ("ab\n".to_string(), true));
    assert_eq!(s.text(), "cd");
}

#[test]
fn independent_indentation_keeps_blank_lines_unicode_selections_and_undo() {
    let text = "x\n\n \n\t y\n　z";
    for (spaces, indented) in [(false, "\t x\n\n\t  \n\t\t  y\n\t 　z"), (true, "    x\n\n     \n        y\n    　z")]
    {
        for (indent, expected) in [(true, indented), (false, "x\n\n\ny\n　z")] {
            let mut s = EditorState::from_text(text);
            s.doc.tab_size = 3;
            s.doc.indent_size = Some(4);
            s.doc.translate_tabs = spaces;
            s.view.selections = vec![Selection::new(0, s.len())];
            let selected = s.view.selections.clone();
            if indent {
                s.indent();
            } else {
                s.unindent();
            }
            assert_eq!(s.text(), expected);
            s.undo();
            assert_eq!(s.text(), text);
            assert_eq!(s.view.selections, selected);
        }
    }
}
