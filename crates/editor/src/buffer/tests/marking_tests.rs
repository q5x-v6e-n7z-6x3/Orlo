use super::*;

fn lines(s: &EditorState) -> Vec<usize> {
    s.bookmarked_lines()
}

#[test]
fn deleting_a_final_crlf_line_removes_its_whole_preceding_line_break() {
    for text in ["one\r\ntwo", "one\r\n"] {
        for bookmarked in [false, true] {
            let mut s = EditorState::from_text(text);
            s.set_cursor(s.len());
            if bookmarked {
                s.toggle_bookmark();
                assert!(s.delete_bookmarked_lines(false));
            } else {
                s.delete_line();
            }
            assert_eq!(s.text(), "one", "{text:?}, bookmarked: {bookmarked}");
            s.undo();
            assert_eq!(s.text(), text);
        }
    }
}

#[test]
fn emptying_a_final_crlf_line_by_transformation_removes_the_whole_break() {
    let mut s = EditorState::from_text("one\r\ntwo");
    s.set_selection(5, 8);
    s.transform_selections(true, |_| Ok(String::new())).unwrap();
    assert_eq!(s.text(), "one");
    s.undo();
    assert_eq!(s.text(), "one\r\ntwo");
}

#[test]
fn removing_final_newlines_takes_whole_crlf_pairs_and_keeps_literal_cr() {
    for text in ["one\r\n", "one\r\n\r\n", "one\r\r\n"] {
        let mut s = EditorState::from_text(text);
        assert!(s.cleanup_for_save(&Cleanup { remove_final_newline: true, ..Cleanup::default() }));
        let expected = if text == "one\r\r\n" { "one\r" } else { "one" };
        assert_eq!(s.text(), expected, "{text:?}");
        s.undo();
        assert_eq!(s.text(), text);
    }
}

#[test]
fn highlight_navigation_after_collapsing_multiple_selections_uses_the_remaining_one() {
    let collapse: [fn(&mut EditorState); 3] =
        [EditorState::select_all, EditorState::single_selection, |s| s.set_cursor(0)];
    for collapse in collapse {
        let mut s = EditorState::from_text("one two three");
        s.add_highlights(0, [4..7]);
        s.set_selections(cs(&[0, 8, 13]));
        collapse(&mut s);
        assert!(s.next_highlight(0, true));
        assert_eq!(s.view.selections, [Selection::new(4, 7)]);
    }
}

#[test]
fn matches_bookmark_the_lines_they_touch_once_added_to_the_others() {
    let mut s = EditorState::from_text("a x\nb\nx x\nc\nd\n");
    s.set_cursor(s.line_start(3));
    s.toggle_bookmark();
    let o = FindOpts::default();
    let found = s.find_all("x", &o);
    assert_eq!(s.bookmark_matches(&found), 2, "lines 0 and 2; 2's two matches once");
    assert_eq!(lines(&s), [0, 2, 3], "added to the manual one");
    // a match over lines touches each; one ending after a break not the next line
    let mut s = EditorState::from_text("ab\ncd\nef\n");
    assert_eq!(s.bookmark_matches(&[1..4]), 2);
    assert_eq!(lines(&s), [0, 1]);
    s.clear_bookmarks();
    s.bookmark_matches(&[1..3]);
    assert_eq!(lines(&s), [0], "ends just after the break");
    s.clear_bookmarks();
    s.bookmark_matches(&[9..9]);
    assert_eq!(lines(&s), [3], "an empty match at the end: the final empty line");
}

#[test]
fn bookmarked_lines_copy_delete_keep_and_invert_in_one_undo_step() {
    let text = "one\ntwo\nthree\nfour";
    let mut s = EditorState::from_text(text);
    s.bookmark_matches(&[4..5, 14..15]);
    assert_eq!(lines(&s), [1, 3]);
    assert_eq!(s.bookmarked_text().as_deref(), Some("two\nfour\n"));
    assert!(s.delete_bookmarked_lines(false));
    assert_eq!(s.text(), "one\nthree", "the last line takes the break before it");
    assert_eq!(s.view.selections, vec![Selection::caret(4)]);
    assert!(lines(&s).is_empty(), "no bookmark moves onto a neighbour");
    s.undo();
    assert_eq!(s.text(), text);
    assert_eq!(lines(&s), [1, 3], "undo brings them back with the lines");
    assert!(s.delete_bookmarked_lines(true));
    assert_eq!(s.text(), "two\nfour");
    assert_eq!(lines(&s), [0, 1], "the kept lines stay bookmarked");
    s.undo();
    s.invert_bookmarks();
    assert_eq!(lines(&s), [0, 2]);
    // a final empty line, and the only line
    let mut s = EditorState::from_text("a\nb\n");
    s.bookmark_matches(&[4..4]);
    assert!(s.delete_bookmarked_lines(false));
    assert_eq!(s.text(), "a\nb");
    let mut s = EditorState::from_text("only");
    s.bookmark_matches(&[0..1]);
    assert!(s.delete_bookmarked_lines(false));
    assert_eq!(s.text(), "");
    assert!(!EditorState::from_text("x").delete_bookmarked_lines(false), "no bookmarks");
}

#[test]
fn pasting_over_bookmarked_lines_takes_a_line_each_when_the_counts_match() {
    let mut s = EditorState::from_text("a\nb\nc\nd\n");
    s.bookmark_matches(&[0..1, 4..5]);
    assert!(s.paste_to_bookmarked_lines("X\nY\n"));
    assert_eq!(s.text(), "X\nb\nY\nd\n");
    assert_eq!(lines(&s), [0, 2], "the bookmarks stay");
    s.undo();
    assert!(s.paste_to_bookmarked_lines("Z"));
    assert_eq!(s.text(), "Z\nb\nZ\nd\n");
}

#[test]
fn highlights_follow_edits_undo_and_other_views() {
    let mut s = EditorState::from_text("foo bar foo baz");
    assert_eq!(s.add_highlights(0, [0..3, 8..11, 9..10]), 2, "merged");
    s.add_highlights(1, [4..7]);
    s.set_cursor(0);
    s.insert("> ");
    assert_eq!(s.view.highlights[0], vec![2..5, 10..13]);
    // deleting a highlighted word drops its highlight; undo brings the text back only
    s.set_selection(10, 13);
    s.insert("");
    assert_eq!(s.view.highlights[0], vec![2..5]);
    s.undo();
    assert_eq!(s.text(), "> foo bar foo baz");
    assert_eq!(s.view.highlights[0], vec![2..5, 10..13], "undo brings the highlight back too");
    s.redo();
    assert_eq!(s.view.highlights[0], vec![2..5]);
    s.undo();
    assert_eq!(s.highlighted_text(1).as_deref(), Some("bar\n"));
    s.clear_highlights(Some(0));
    s.add_highlights(0, [2..5]);
    s.set_cursor(0);
    assert!(s.next_highlight(0, true));
    assert_eq!(s.view.selections[0].range(), 2..5);
    assert!(s.next_highlight(0, false), "round to the last");
    // a second view starts with them and keeps its own; the first's edits map both
    s.take_changes();
    let mut v = two_views(s);
    v.b().clear_highlights(Some(1));
    let mut s = v.a();
    assert_eq!(s.view.highlights[1], vec![6..9]);
    s.set_cursor(0);
    s.insert("!");
    assert_eq!(v.b().view.highlights[0], vec![3..6], "the other view's follow the edit");
    assert!(v.b().view.highlights[1].is_empty());
    let mut s = v.a();
    s.clear_highlights(None);
    assert!(s.view.highlights.iter().all(Vec::is_empty));
}

#[test]
fn a_keystroke_at_thousands_of_carets_maps_marks_once() {
    let n = 2_000;
    let mut s = EditorState::from_text(&"ab\n".repeat(n));
    s.add_highlights(0, (0..n).map(|i| i * 3..i * 3 + 2));
    s.set_bookmarked_lines(0..n);
    s.view.selections = cs(&(0..n).map(|i| i * 3 + 1).collect::<Vec<_>>());
    s.insert("x");
    // each line is now "axb": a highlight takes in text typed inside it, bookmarks keep lines
    assert_eq!(s.view.highlights[0][..2], [0..3, 4..7]);
    assert_eq!(s.view.bookmarks[..3], [0, 4, 8]);
    assert_eq!(lines(&s).len(), n);
}

#[test]
fn deleting_twenty_thousand_bookmarked_lines_maps_marks_once() {
    let mut s = EditorState::from_text(&"x\ny\n".repeat(20_000));
    let found = s.find_all("x", &FindOpts::default());
    assert_eq!(s.bookmark_matches(&found), 20_000);
    s.add_highlights(1, (0..20_000).map(|i| i * 4 + 2..i * 4 + 3));
    let t = std::time::Instant::now();
    assert!(s.delete_bookmarked_lines(false));
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(s.text(), "y\n".repeat(20_000));
    assert_eq!(s.view.highlights[1][..2], [0..1, 2..3], "each y keeps its highlight");
}

#[test]
fn deleting_unbookmarked_lines_keeps_the_final_line_break() {
    let mut s = EditorState::from_text("one\ntwo\n");
    s.bookmark_matches(&[4..5]);
    assert!(s.delete_bookmarked_lines(true));
    assert_eq!(s.text(), "two\n");
    let mut s = EditorState::from_text("one\ntwo\nthree\n");
    s.bookmark_matches(&[0..1]);
    assert!(s.delete_bookmarked_lines(true));
    assert_eq!(s.text(), "one\n");
}
