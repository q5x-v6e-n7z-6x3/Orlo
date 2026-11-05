use super::*;

#[test]
fn quick_add_uses_whole_word_edges_after_combining_marks() {
    let mut s = EditorState::from_text("cafe\u{301} cafe\u{301}s cafe\u{301}");
    s.select_next_occurrence();
    assert_eq!(s.view.selections, [Selection::new(0, 5)]);
    s.select_next_occurrence();
    assert_eq!(s.view.selections, sels(&[(0, 5), (13, 18)]));
    for case_sensitive in [false, true] {
        let opts = opts_for(case_sensitive, true);
        let expected = vec![0..5, 13..18];
        let chars: Vec<_> = s.text().chars().collect();
        let query: Vec<_> = "cafe\u{301}".chars().collect();
        assert_eq!(find_in(&chars, &query, &opts), expected);
        assert_eq!(s.find_all("cafe\u{301}", &opts), expected);
        // Repeating the query exercises the cached result; a scope ending before the
        // suffix must still observe the word continuation outside that scope.
        assert_eq!(s.find_all("cafe\u{301}", &opts), expected);
        assert_eq!(s.find_scoped("cafe\u{301}", &opts, &[0..11, 13..18]), expected);
        assert_eq!(s.find_all("cafe\u{301}", &FindOpts { regex: true, ..opts }), expected);
    }
}

// ---- edge cases: undo steps, carets, line endings and overlapping edits ----

#[test]
fn select_all_then_type_is_its_own_undo_step() {
    let mut s = EditorState::new();
    s.insert("a");
    s.insert("b");
    s.select_all();
    s.insert("c");
    s.undo();
    assert_eq!(s.text(), "ab");
}

#[test]
fn shift_extend_then_type_is_its_own_undo_step() {
    let mut s = st("ab", 2);
    s.insert("c");
    let shift_click = s.drag_from(0, 1, false, true);
    s.drag_to(&shift_click, 0);
    s.insert("x");
    s.undo();
    assert_eq!(s.text(), "abc");
}

#[test]
fn adjacent_selections_stay_separate() {
    let mut s = EditorState::from_text("))");
    s.select_ranges(&[0..1, 1..2]);
    assert_eq!(s.view.selections.len(), 2);
    s.insert("]");
    assert_eq!(s.text(), "]]");
}

#[test]
fn duplicate_line_with_two_carets_moves_both_onto_their_copies() {
    let mut s = carets("ab\ncd", &[0, 3]);
    s.duplicate_line();
    assert_eq!(s.text(), "ab\nab\ncd\ncd");
    assert_eq!(s.view.selections, cs(&[3, 9]));
}

#[test]
fn duplicate_selection_moves_selection_onto_copy() {
    let mut s = selecting("abcd", &[(0, 1), (2, 3)]);
    s.duplicate_line();
    assert_eq!(s.text(), "aabccd");
    assert_eq!(s.view.selections, sels(&[(1, 2), (4, 5)]));
}

#[test]
fn newline_before_indent_does_not_double_it() {
    let mut s = st("    foo", 0);
    s.newline();
    assert_eq!(s.text(), "\n    foo");
}

#[test]
fn insert_at_two_cursors_on_the_same_line() {
    let mut s = carets("abcd", &[1, 3]);
    s.insert("X");
    assert_eq!(s.text(), "aXbcXd");
    assert_eq!(s.view.selections, cs(&[2, 5]));
}

#[test]
fn typing_an_opener_inserts_the_pair_and_a_closer_steps_over() {
    let mut s = st("", 0);
    s.type_text("(");
    assert_eq!(s.text(), "()");
    assert_eq!(s.view.selections, vec![Selection::caret(1)]);
    s.type_text(")");
    assert_eq!(s.text(), "()");
    assert_eq!(s.view.selections, vec![Selection::caret(2)]);
}

#[test]
fn no_pair_before_a_word_or_for_a_quote_after_a_word() {
    let mut s = st("x", 0);
    s.type_text("(");
    assert_eq!(s.text(), "(x");
    let mut s = st("don", 3);
    s.type_text("'");
    assert_eq!(s.text(), "don'");
}

#[test]
fn opener_with_a_selection_wraps_it() {
    let mut s = selecting("ab", &[(0, 2)]);
    s.type_text("[");
    assert_eq!(s.text(), "[ab]");
    assert_eq!(s.view.selections, vec![Selection::new(1, 3)]);
}

#[test]
fn backspace_between_a_pair_removes_both() {
    let mut s = st("{}", 1);
    s.backspace();
    assert_eq!(s.text(), "");
}

#[test]
fn enter_between_braces_opens_an_indented_block() {
    let mut s = st("  {}", 3);
    s.newline();
    assert_eq!(s.text(), "  {\n      \n  }");
    assert_eq!(s.view.selections, vec![Selection::caret(10)]);
}

#[test]
fn select_lines_adds_a_caret_on_the_neighbouring_line() {
    let mut s = st("abc\ndef\ng", 1);
    s.select_lines(false, &Lines, &Cells);
    assert_eq!(s.view.selections, vec![Selection::caret(1).in_rectangle()]);
    s.select_lines(true, &Lines, &Cells);
    assert_eq!(s.view.selections, cs(&[1, 5]).into_iter().map(Selection::in_rectangle).collect::<Vec<_>>());
    s.select_lines(true, &Lines, &Cells);
    assert_eq!(s.view.selections, cs(&[1, 5, 9]).into_iter().map(Selection::in_rectangle).collect::<Vec<_>>());
}

#[test]
fn select_all_occurrences_takes_every_match_of_the_word() {
    let mut s = st("foo bar foo", 1);
    s.select_all_occurrences();
    assert_eq!(s.view.selections, sels(&[(0, 3), (8, 11)]));
}

#[test]
fn delete_word_stops_at_the_boundary_and_is_a_noop_at_the_edge() {
    let mut s = st("foo bar", 7);
    s.delete_word(false);
    assert_eq!(s.text(), "foo ");
    let mut s = st("foo bar", 0);
    s.delete_word(false);
    assert_eq!(s.text(), "foo bar");
    s.delete_word(true);
    assert_eq!(s.text(), " bar");
}

#[test]
fn find_options_case_whole_word_and_no_wrap() {
    let s = st("Foo foobar foo", 0);
    let f = |case_sensitive, whole_word| s.find_all("foo", &opts_for(case_sensitive, whole_word));
    assert_eq!(f(false, false).len(), 3);
    assert_eq!(f(true, false).len(), 2);
    assert_eq!(f(true, true), vec![11..14]);
    let no_wrap = FindOpts { case_sensitive: true, whole_word: true, wrap: false, ..FindOpts::default() };
    assert_eq!(s.find_next("foo", 12, &no_wrap), None);
    assert_eq!(s.find_next("foo", 12, &FindOpts { wrap: true, ..no_wrap }), Some(11..14));
}

#[test]
fn lone_cr_in_a_unix_file_survives_save() {
    // one stray carriage return inside line 2 must not turn every line ending into CR
    let dir = fresh_dir("orlo-lone-cr");
    let p = dir.join("a.txt");
    std::fs::write(&p, "a\nb\rc\nd\n").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"a\nb\rc\nd\n");
}

#[test]
fn mixed_line_endings_keep_majority_preference_until_explicit_conversion() {
    let p = fresh_dir("orlo-buffer-files").join("orlo_mixed_endings.txt");
    for (raw, saved) in [("a\r\nb\r\nc\n", "a\r\nb\r\nc\r\n"), ("a\rb\r\nc\r", "a\rb\rc\r")] {
        std::fs::write(&p, raw).unwrap();
        let mut s = EditorState::load(&p).unwrap();
        assert_eq!(s.text(), "a\nb\nc\n", "{raw:?}");
        s.save().unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), raw);
        // The old majority observation still describes arbitrary-text conversion.
        assert_eq!(file_text(normalized(raw).0.into_owned(), s.doc.line_ending, false), saved);
        let ending = s.doc.line_ending;
        s.convert_line_endings(ending);
        s.save().unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), saved);
        s.undo();
        assert_eq!(s.doc.bytes_as_saved(None).unwrap(), raw.as_bytes());
    }
}

#[test]
fn unicode_separators_and_form_feed_are_not_line_breaks() {
    // Sublime, git and compilers count 3 lines here; ropey's default features count 5
    let s = EditorState::from_text("var s = \"a\u{2028}b\";\nint x;\u{000C}\nend");
    assert_eq!(s.line_count(), 3);
}

#[test]
fn paste_is_its_own_undo_step() {
    let mut s = st("", 0);
    s.type_text("a");
    s.type_text("b");
    s.paste("XYZ", false);
    s.undo();
    assert_eq!(s.text(), "ab");
    s.redo();
    s.type_text("c");
    s.undo();
    assert_eq!(s.text(), "abXYZ");
}

#[test]
fn copy_two_empty_carets_has_no_blank_line_between() {
    let s = carets("one\ntwo\n", &[1, 5]);
    assert_eq!(s.copy_text().0, "one\ntwo\n");
}

#[test]
fn whole_line_copy_pastes_above_the_caret_line() {
    let mut s = st("one\ntwo\nthree", 1);
    let (clip, whole_line) = s.copy_text();
    s.view.selections = vec![Selection::caret(10)]; // th|ree
    s.paste(&clip, whole_line);
    assert_eq!(s.text(), "one\ntwo\none\nthree");
}

#[test]
fn whole_line_copy_from_two_carets_pastes_one_line_per_caret() {
    let mut s = carets("one\ntwo\nA\nB", &[1, 5]);
    let (clip, whole_line) = s.copy_text();
    s.view.selections = cs(&[9, 11]);
    s.paste(&clip, whole_line);
    assert_eq!(s.text(), "one\ntwo\none\nA\ntwo\nB");
}

#[test]
fn cmd_d_from_a_caret_matches_whole_words_only() {
    // expanding from an empty caret makes quick-add-next whole-word: "foobar" is skipped
    let mut s = st("foo foobar foo", 1);
    s.select_next_occurrence();
    s.select_next_occurrence();
    let starts: Vec<usize> = s.view.selections.iter().map(|x| x.start()).collect();
    assert_eq!(starts, vec![0, 11], "selected {:?}", sel_texts(&s));
}

#[test]
fn cmd_d_from_a_selection_still_matches_substrings() {
    // with a real selection to start from, Sublime matches anywhere
    let mut s = selecting("foo foobar", &[(0, 3)]);
    s.select_next_occurrence();
    assert_eq!(s.view.selections.len(), 2);
}

#[test]
fn drag_after_cmd_click_extends_the_new_caret() {
    // caret on line 3; cmd-click on line 1, then drag: the new caret grows, not the old one
    let mut s = st("aaaa\nbbbb\ncccc", 12);
    let cmd_press = s.drag_from(1, 1, true, false);
    s.drag_to(&cmd_press, 1);
    s.drag_to(&cmd_press, 3); // what Editor::on_mouse_move does while dragging
    let pairs = ends_of(&s);
    assert!(pairs.contains(&(1, 3)) && pairs.contains(&(12, 12)), "{pairs:?}");
}

#[test]
fn cursor_is_the_newest_selection_after_cmd_d_wraps() {
    // the status bar and scrolling follow `cursor()`; after wrapping it must be the new match
    let mut s = st("x\nfoo\nx", 6);
    s.select_next_occurrence();
    s.select_next_occurrence();
    assert_eq!(s.point(s.cursor()).0, 0);
}

#[test]
fn select_lines_upward_makes_the_new_caret_primary() {
    let mut s = st("a\nb\nc", 4);
    s.select_lines(false, &Lines, &Cells);
    assert_eq!(s.point(s.cursor()).0, 1);
    s.select_lines(false, &Lines, &Cells);
    assert_eq!(s.point(s.cursor()).0, 0);
}

#[test]
fn undo_back_to_saved_text_is_clean() {
    let mut s = st("hello", 5);
    s.doc.dirty = false; // as just loaded or saved
    s.type_text("!");
    s.undo();
    assert_eq!(s.text(), "hello");
    assert!(!s.doc.dirty);
}

#[test]
fn save_point_survives_undo_and_redo_but_not_a_new_branch() {
    let p = fresh_dir("orlo-buffer-files").join("orlo_save_point.txt");
    let mut s = st("", 0);
    s.type_text("a");
    s.save_to(p).unwrap();
    s.type_text("b");
    s.undo();
    assert_eq!(s.text(), "a");
    assert!(!s.doc.dirty);
    s.redo();
    assert!(s.doc.dirty);
    s.undo();
    s.undo();
    s.type_text("x");
    s.undo();
    s.redo();
    assert!(s.doc.dirty, "the saved text is gone from the redo stack");
}

#[test]
fn crlf_in_an_lf_majority_file_loads_as_lf() {
    let p = fresh_dir("orlo-buffer-files").join("orlo_lf_majority.txt");
    std::fs::write(&p, "a\r\nb\nc\nd\n").unwrap();
    let load = || EditorState::load(&p).unwrap();
    let mut s = load();
    s.set_cursor(s.line_end(0));
    s.move_cursors(Motion::Right, false);
    s.type_text("X");
    assert_eq!((s.line(0), s.line(1)), ("a".into(), "Xb".into()));
    let mut s = load();
    s.set_cursor(s.line_start(1));
    s.backspace();
    assert_eq!(s.line(0), "ab");
    let mut s = load();
    s.set_cursor(s.line_end(0));
    s.delete();
    assert_eq!(s.line(0), "ab");
    load().save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"a\r\nb\nc\nd\n");
    let mut converted = load();
    converted.convert_line_endings("\n");
    converted.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"a\nb\nc\nd\n");
    converted.undo();
    assert_eq!(converted.doc.bytes_as_saved(None).unwrap(), b"a\r\nb\nc\nd\n");
}

#[test]
fn select_all_occurrences_from_a_caret_matches_whole_words_only() {
    let mut s = st("foo foobar foo", 1);
    s.select_all_occurrences();
    assert_eq!(s.view.selections, sels(&[(0, 3), (11, 14)]));
}

#[test]
fn edits_that_change_nothing_are_not_dirty_and_not_undo_steps() {
    let mut s = st("ab", 0);
    s.backspace();
    s.set_cursor(2);
    s.delete();
    s.insert("");
    assert!(!s.doc.dirty);
    assert!(s.doc.history.undo.is_empty());
    s.type_text("c");
    s.undo();
    s.set_cursor(0);
    s.backspace();
    s.redo();
    assert_eq!(s.text(), "abc", "a no-op edit keeps the redo stack");
}

#[test]
fn shift_double_and_triple_clicks_leave_a_caret_at_the_click_as_sublime_does() {
    // Sublime 4215: (selections before, clicks, offset clicked) -> caret
    let text = "alpha beta gamma\nsecond line here\nthird\n";
    let cases = [
        (vec![Selection::caret(2)], 2, 13),
        (vec![Selection::caret(14)], 2, 2),
        (vec![Selection::caret(2)], 3, 24),
        (vec![Selection::new(6, 8)], 2, 13),
        (vec![Selection::new(8, 6)], 2, 13),
        (cs(&[2, 20]), 2, 13),
    ];
    for (sels, clicks, at) in cases {
        let mut s = st(text, 0);
        s.set_selections(sels.clone());
        let from = s.drag_from(at, clicks, false, true);
        s.drag_to(&from, at);
        assert_eq!(s.view.selections, [Selection::caret(at)], "{sels:?} x{clicks} at {at}");
    }
    // a single shift-click extends the newest caret and keeps the others
    let mut s = st(text, 0);
    s.set_selections(cs(&[2, 20]));
    let from = s.drag_from(13, 1, false, true);
    s.drag_to(&from, 13);
    assert_eq!(s.view.selections, sels(&[(2, 2), (20, 13)]));
}

#[test]
fn delete_word_with_a_selection_deletes_only_the_selection() {
    // Sublime 4215
    for (sel, forward) in [((6, 9), false), ((6, 9), true), ((9, 6), false)] {
        let mut s = selecting("hello world foo", &[sel]);
        s.delete_word(forward);
        assert_eq!((s.text().as_str(), s.cursor()), ("hello ld foo", 6));
    }
    let mut s = st("hello world foo", 8);
    s.delete_word(false);
    assert_eq!(s.text(), "hello rld foo", "a caret still takes the word part");
}

#[test]
fn convert_indentation_leaves_unicode_spaces_alone() {
    let mut s = st("\u{3000}a\n\u{a0}b\n\t c\n", 0);
    s.convert_indentation(true);
    assert_eq!(s.text(), "\u{3000}a\n\u{a0}b\n     c\n");
}

#[test]
fn wrap_paragraph_keeps_a_caret_on_a_word_that_moves_to_the_next_line() {
    // two lines before and after: "ccc" moves down
    let mut s = st("aaa bbb ccc\nd", 9);
    s.wrap_lines(7, None);
    assert_eq!(s.text(), "aaa bbb\nccc d");
    assert_eq!(s.point(s.cursor()), (1, 1), "still between the c's");
}

#[test]
fn ctrl_k_at_two_carets_and_its_undo_keep_another_views_caret_on_its_line() {
    // ctrl-k's edits are deletions, and the undo pairs lines
    let mut v = views("aa bb\ncc dd");
    v.b().set_cursor(10);
    v.a().view.selections = cs(&[2, 8]);
    v.a().delete_to_line_edge(true);
    let line = |v: &mut Views| {
        let b = v.b();
        b.point(b.cursor()).0
    };
    assert_eq!((v.a().text().as_str(), line(&mut v)), ("aa\ncc", 1));
    v.a().undo();
    assert_eq!((v.a().text().as_str(), line(&mut v)), ("aa bb\ncc dd", 1));
}

#[test]
fn undo_follows_the_caret_that_was_primary() {
    let mut s = carets("hello world", &[5, 8]);
    s.view.primary = 1;
    // the two cuts merge into one selection, then undo brings both back
    s.delete_to_line_edge(true);
    s.undo();
    assert_eq!((s.view.selections.len(), s.cursor()), (2, 8));
    s.redo();
    s.undo();
    assert_eq!(s.cursor(), 8);
}

#[test]
fn backspace_at_a_line_start_removes_a_stray_crlf_whole() {
    let mut s = st("ab\r\ncd", 4);
    s.backspace();
    assert_eq!(s.text(), "abcd");
    assert_eq!(s.cursor(), 2);
    s.undo();
    assert_eq!(s.text(), "ab\r\ncd");
    assert_eq!(s.cursor(), 4);

    // Ordinary combining marks still delete one code point at a time.
    let mut s = st("e\u{301}", 2);
    s.backspace();
    assert_eq!(s.text(), "e");
}

#[test]
fn ctrl_k_at_a_line_end_takes_a_stray_crlf_whole() {
    // an LF file with one CRLF line keeps its CR in the buffer
    let mut s = st("ab\r\ncd", 2);
    s.delete_to_line_edge(true);
    assert_eq!(s.text(), "abcd");
    let mut s = st("ab\r\ncd", 4);
    s.delete_to_line_edge(false);
    assert_eq!(s.text(), "abcd");
}

#[test]
fn transpose_takes_adjacent_carets_left_to_right_as_sublime_does() {
    // Sublime 4215, without its stray extra caret at 0
    for (text, at, want, after) in [
        ("abcd", &[1, 2][..], "bacd", &[2][..]),
        ("abcde", &[1, 2, 3], "badce", &[2, 4]),
        ("abcd", &[1, 3], "badc", &[2, 4]),
        ("abcdef", &[1, 2, 4], "bacedf", &[2, 5]),
    ] {
        let mut s = carets(text, at);
        s.transpose();
        assert_eq!((s.text().as_str(), &carets_of(&s)[..]), (want, after), "{text} {at:?}");
    }
}

#[test]
fn a_replacement_overlapping_a_lengthening_edit_is_left_out() {
    // CSS: a selection over two lines and a caret on the second; the caret's line comment
    // would overwrite part of the selection's, which made that text longer: it is left
    // out, and nothing the first wrote is lost or doubled
    let mut s = selecting("a\nb c\n", &[(0, 3), (4, 4)]);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!(s.text(), "/*a\nb*/ c\n");
    assert_eq!(s.view.selections.len(), 2);
}

#[test]
fn edit_with_asks_each_selection_once_whatever_overlaps() {
    // every closure that keeps its own state, with selections whose edits overlap
    let mut s = selecting("ab cd ef", &[(0, 2), (3, 5), (6, 8)]);
    s.transpose();
    assert_eq!(s.text(), "ef ab cd");
    let mut s = selecting("b\na\nc", &[(0, 3), (4, 4)]);
    s.permute_selections(Permute::Reverse);
    let mut s = selecting("abcdefgh", &[(1, 1), (2, 2), (4, 6)]);
    s.tab();
    assert_eq!(s.text(), "a   b   cd  gh");
    // counted directly: edits that overlap, repeat (made once) and come after another
    let mut s = selecting("hello world", &[(4, 7), (8, 8), (10, 10)]);
    let mut asked = vec![0; 3];
    s.edit_with(|s, i, sel| {
        asked[i] += 1;
        let r = if sel.is_empty() { 6..11 } else { sel.range() };
        let upper = s.doc.rope.slice(r.clone()).to_string().to_uppercase();
        (r, upper, *sel)
    });
    assert_eq!(asked, [1, 1, 1]);
    assert_eq!(s.text(), "hellO WORLD");
}

#[test]
fn a_whole_line_paste_into_overlapping_selections_does_not_panic() {
    // the clipboard holds two whole lines; a caret's line start falls inside the selection
    let mut s = selecting("xx\nyyyy", &[(1, 5), (6, 6)]);
    s.paste("a\nb\n", true);
    // as Sublime makes them one after another: the selection's line, then the caret's
    assert_eq!(s.text(), "xa\nb\nyy");
    // the selection's caret after its line; the caret's where it was, between the y's
    assert_eq!(s.view.selections, cs(&[3, 6]));
}

#[test]
fn an_upper_case_made_after_an_overlapping_one_keeps_the_selections_inside_it() {
    // the caret's word overlaps the first selection: its edit is made after, on the
    // edited text, and the selection inside that word stays where it is
    let mut s = selecting("hello world", &[(4, 7), (8, 8), (9, 10)]);
    s.convert_case(Case::Upper);
    assert_eq!(s.text(), "hellO WORLD");
    assert_eq!(s.view.selections, sels(&[(4, 7), (8, 8), (9, 10)]));
    // two carets asking for that one later edit: each keeps its place
    let mut s = selecting("hello world", &[(4, 7), (8, 8), (10, 10)]);
    s.convert_case(Case::Upper);
    assert_eq!(s.text(), "hellO WORLD");
    assert_eq!(s.view.selections, sels(&[(4, 7), (8, 8), (10, 10)]));
}

#[test]
fn a_selection_beside_a_carets_word_keeps_its_place_through_a_case_change() {
    // Sublime 4215 swaps the overlap twice ("HEllO"); Orlo asks each selection once, so the
    // selection's edit is the word's own text again
    let mut s = selecting("hello", &[(0, 0), (2, 4)]);
    s.convert_case(Case::Swap);
    assert_eq!(s.text(), "HELLO");
    assert_eq!(s.view.selections, sels(&[(0, 0), (2, 4)]));
    let mut s = selecting("hello world", &[(1, 1), (3, 8)]);
    s.convert_case(Case::Upper);
    assert_eq!(s.text(), "HELLO WOrld");
    assert_eq!(s.view.selections, sels(&[(1, 1), (3, 8)]));
}

#[test]
fn two_carets_on_a_line_duplicate_it_twice_as_in_sublime() {
    // Sublime 4215: insertions are not one change made twice
    let mut s = carets("ab\ncd\n", &[0, 1]);
    s.duplicate_line();
    assert_eq!(s.text(), "ab\nab\nab\ncd\n");
    let mut s = carets("xy\n", &[0, 1]);
    s.paste("aa\n", true);
    assert_eq!(s.text(), "aa\naa\nxy\n");
}

#[test]
fn two_carets_asking_for_the_same_change_make_it_once() {
    let mut s = carets("hello world", &[1, 3]);
    s.convert_case(Case::Upper);
    assert_eq!(s.text(), "HELLO world");
    assert_eq!(s.view.selections, cs(&[1, 3]));
    // CSS has only block comments: both carets wrap the one line
    let mut s = carets("foo bar\nbaz", &[1, 5]);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!(s.text(), "/*foo bar*/\nbaz");
    assert_eq!(s.view.selections, cs(&[3, 7]));
}

#[test]
fn case_conversion_of_shared_words_preserves_each_target_and_undo() {
    for (text, at, case, expected, after) in [
        ("straße FooBar", vec![0, 2, 4, 8, 10], Case::Upper, "STRASSE FOOBAR", vec![0, 2, 4, 9, 11]),
        ("!? abCD", vec![0, 1, 3, 5], Case::Snake, " ab_cd", vec![0, 1, 3]),
    ] {
        let mut s = carets(text, &at);
        s.convert_case(case);
        assert_eq!(s.text(), expected);
        assert_eq!(carets_of(&s), after);
        s.undo();
        assert_eq!(s.text(), text);
        assert_eq!(carets_of(&s), at);
        s.redo();
        assert_eq!(s.text(), expected);
        assert_eq!(carets_of(&s), after);
    }
}

#[test]
fn multi_caret_edits_that_overlap_merge_in_the_original_text() {
    // ctrl-k: Sublime extends both carets to the line end, the regions merge, one cut
    let mut s = carets("hello world", &[5, 8]);
    s.delete_to_line_edge(true);
    assert_eq!(s.text(), "hello");
    assert_eq!(s.view.kill, " world");
    let mut s = carets("abcde", &[1, 3]);
    s.delete_to_line_edge(true);
    assert_eq!(s.text(), "a");
    let mut s = carets("x()", &[2, 3]);
    s.backspace();
    assert_eq!(s.text(), "x");
    // cmd-enter: both carets go to the line end first, so one line opens
    let mut s = carets("ab\ncd", &[0, 1]);
    s.insert_line_after();
    assert_eq!(s.text(), "ab\n\ncd");
    assert_eq!(s.view.selections, vec![Selection::caret(3)]);
    let mut s = carets("ab\ncd", &[1, 2]);
    s.insert_line_before();
    assert_eq!(s.text(), "\nab\ncd");
    assert_eq!(s.view.selections, vec![Selection::caret(0)]);
}

#[test]
fn every_multi_caret_command_keeps_carets_after_their_own_edit() {
    let mut s = carets("a b c", &[1, 3, 5]);
    s.type_text("(");
    assert_eq!(s.text(), "a() b() c()");
    assert_eq!(carets_of(&s), [2, 6, 10]);
    s.backspace();
    assert_eq!(s.text(), "a b c");
    assert_eq!(carets_of(&s), [1, 3, 5]);
    s.newline();
    assert_eq!(s.text(), "a\n b\n c\n");
    assert_eq!(carets_of(&s), [2, 5, 8]);
    let mut s = carets("ab\ncd", &[1, 4]);
    s.duplicate_line();
    assert_eq!(s.text(), "ab\nab\ncd\ncd");
    assert_eq!(carets_of(&s), [4, 10]);
    let mut s = carets("ab cd", &[0, 3]);
    s.paste("1\n2", false);
    assert_eq!(s.text(), "1ab 2cd");
    assert_eq!(carets_of(&s), [1, 5]);
    let mut s = carets("ab cd", &[1, 4]);
    s.delete();
    assert_eq!(s.text(), "a c");
    let mut s = carets("ab cd", &[2, 5]);
    s.delete_to_line_edge(false);
    assert_eq!(s.text(), "", "the second cut reaches back over the first");
    let mut s = carets("ab\ncd", &[1, 4]);
    s.tab();
    assert_eq!(s.text(), "a   b\nc   d");
    let mut s = carets("foo bar", &[0, 4]);
    s.view.selections = sels(&[(0, 3), (4, 7)]);
    s.toggle_block_comment("/*", "*/");
    assert_eq!(s.text(), "/*foo*/ /*bar*/");
}

#[test]
fn no_caret_is_left_between_a_cr_and_its_line_feed() {
    let mut s = st("10%\r20%\nx", 0);
    s.move_cursors(Motion::LineEnd, false);
    for _ in 0..3 {
        s.backspace();
    }
    assert_eq!(s.text(), "10%\r\nx");
    assert_eq!(s.cursor(), 3, "before the CR, where the line visibly ends");
    s.tab();
    let wm = crate::viewport::WrapMap::new(&s.doc, 10, 4, &crate::fold::Folds::NONE);
    let _ = Rows::row_of(&wm, &s.doc, 4);
}

#[test]
fn line_commands_carry_a_cr_at_a_line_end_with_its_line() {
    let mut s = st("a\r\nb\nc", 3);
    s.move_line_up();
    assert_eq!(s.text(), "b\na\r\nc");
    assert_eq!(s.cursor(), 0);
    s.move_line_down();
    assert_eq!(s.text(), "a\r\nb\nc");
    let mut s = st("a\nb\r\nc", 5);
    s.move_line_up();
    assert_eq!(s.text(), "a\nc\nb\r");
    s.move_line_up();
    assert_eq!(s.text(), "c\na\nb\r");
    let mut s = st("x\r\ny", 0);
    s.move_line_down();
    assert_eq!(s.text(), "y\nx\r");
    let mut s = st("a\r\nb", 0);
    s.duplicate_line();
    assert_eq!(s.text(), "a\r\na\r\nb");
    let mut s = st("c\r\nb\na", 0);
    s.permute_lines(sort(true));
    assert_eq!(s.text(), "a\nb\nc\r");
}

#[test]
fn a_byte_order_mark_stays_at_the_start_of_the_file() {
    let dir = fresh_dir("orlo-bom");
    let p = dir.join("bom.txt");
    std::fs::write(&p, "\u{feff}b\r\na\r\n").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    assert_eq!(s.text(), "b\na\n");
    s.insert("x");
    s.permute_lines(sort(true));
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), "\u{feff}a\r\nxb\r\n".as_bytes());
    // and another view saves it the same way
    let mut v = two_views(s);
    v.b().insert("y");
    v.b().save().unwrap();
    assert!(std::fs::read(&p).unwrap().starts_with("\u{feff}".as_bytes()));
}

#[test]
fn undo_never_hides_a_line_ending_change() {
    let mut s = st("a\nb", 0);
    s.set_line_ending("\r\n");
    assert!(s.doc.dirty);
    s.insert("x");
    s.undo();
    assert!(s.doc.dirty, "the CRLF conversion is still unsaved");
    s.undo();
    assert!(s.doc.dirty);
    let mut results = st("x", 0);
    results.doc.scratch = true;
    results.set_line_ending("\r\n");
    assert!(!results.doc.dirty, "Find Results never gets dirty");
}

#[test]
fn up_down_and_clicks_never_land_inside_a_cluster() {
    // 7 chars, one cluster
    let family = "👩\u{200d}👩\u{200d}👧\u{200d}👦";
    let mut s = st(&format!("abcd\n{family}x\ne\u{301}e\u{301}"), 3);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), 5, "the family's start, not 3 chars into it");
    s.set_cursor(3);
    s.move_cursors(Motion::Down, false);
    s.move_cursors(Motion::Down, false);
    assert!([16, 18].contains(&s.cursor()), "{}", s.cursor());
    assert_eq!(Lines.offset_at(&s.doc, DisplayPoint::new(2, 1)), 14);
    let wm = crate::viewport::WrapMap::new(&s.doc, 80, 4, &crate::fold::Folds::NONE);
    assert_eq!(wm.offset_at(&s.doc, DisplayPoint::new(1, 3)), 5);
}

#[test]
fn brackets_and_quotes_pair_at_the_end_of_a_line_with_lines_below() {
    let mut s = st("a\nb", 1);
    s.type_text("(");
    assert_eq!(s.text(), "a()\nb");
    let mut s = st("a \nb", 2);
    s.type_text("\"");
    assert_eq!(s.text(), "a \"\"\nb");
    // a quote right after a word char is not paired (Sublime)
    let mut s = st("a\nb", 1);
    s.type_text("\"");
    assert_eq!(s.text(), "a\"\nb");
}

#[test]
fn the_end_of_line_column_outlives_up_on_the_first_line_as_in_sublime() {
    // Sublime 4215: cmd-right, Up to offset 0, then select_lines keeps to line ends
    let mut s = st("ab\nabcd\nabcdef", 5);
    s.move_cursors(Motion::LineEnd, false);
    s.move_cursors(Motion::Up, false);
    s.move_cursors(Motion::Up, false);
    assert_eq!(s.cursor(), 0);
    s.select_lines(true, &Lines, &Cells);
    assert_eq!(s.view.selections.last().unwrap().head, 7);
}

#[test]
fn wrap_paragraph_refills_each_paragraph_on_its_own_keeping_prefixes() {
    // a selection over two paragraphs: each refilled, the blank line kept, the third left
    let mut s = selecting("a b c d\ne f\n\ng h i\nj\n\nk l m n o p\n", &[(2, 12)]);
    s.wrap_lines(5, None);
    assert_eq!(s.text(), "a b c\nd e f\n\ng h i\nj\n\nk l m n o p\n");
    // comment lines keep their marker and indent; the code around is never joined in
    let mut s = st("fn x() {}\n    // aa bb cc\n    // dd\nfn y() {}\n", 15);
    s.wrap_lines(12, Some("//"));
    assert_eq!(s.text(), "fn x() {}\n    // aa bb\n    // cc dd\nfn y() {}\n");
    // every line takes the first line's indent; a word too long stands alone
    let mut s = st("  aa bb\n    cc dddddddddddd ee\n", 0);
    s.wrap_lines(8, None);
    assert_eq!(s.text(), "  aa bb\n  cc\n  dddddddddddd\n  ee\n");
    // a caret on a blank line wraps nothing, and wrapped text is no undo step
    let mut s = st("a b c\n\nd e f\n", 6);
    s.wrap_lines(3, None);
    assert_eq!(s.text(), "a b c\n\nd e f\n");
    let mut s = st("ab\ncd\n", 0);
    s.wrap_lines(2, None);
    assert!(!s.doc.dirty && s.doc.history.undo.is_empty());
}

#[test]
fn expand_to_paragraph_takes_one_unit_at_a_time_as_in_sublime() {
    let text = "a\nb\n\nc\nd\n";
    let mut s = st(text, 1);
    s.expand_to_paragraph();
    assert_eq!(s.view.selections, vec![Selection::new(0, 4)], "the paragraph and its last break");
    s.expand_to_paragraph();
    assert_eq!(s.view.selections, vec![Selection::new(0, 5)], "then the blank line");
    let mut s = st(text, 4);
    s.expand_to_paragraph();
    assert_eq!(s.view.selections, vec![Selection::new(4, 5)], "a blank line alone");
    s.expand_to_paragraph();
    assert_eq!(s.view.selections, vec![Selection::new(4, 9)], "then the next paragraph");
}

#[test]
fn carets_follow_their_text_through_replace_all_unique_and_indentation() {
    let mut s = st("foo bar foo", 11);
    s.replace_all("foo", "x", &FindOpts::default());
    assert_eq!((s.text().as_str(), s.cursor()), ("x bar x", 7));
    let mut s = st("a\na\nb", 5);
    s.permute_lines(Permute::Unique);
    assert_eq!((s.text().as_str(), s.cursor()), ("a\nb", 3));
    let mut s = st("\tx\n\ty", 5);
    s.convert_indentation(true);
    assert_eq!((s.text().as_str(), s.cursor()), ("    x\n    y", 11));
    s.undo();
    assert_eq!(s.view.selections, vec![Selection::caret(5)], "undo brings back the caret, not all");
    // nothing to convert, sort or replace: not dirty, no undo step (Sublime 4215 marks
    // these dirty)
    let mut s = st("a\nb", 0);
    s.convert_indentation(true);
    s.permute_lines(sort(true));
    s.replace_all("a", "a", &FindOpts::default());
    assert!(!s.doc.dirty && s.doc.history.undo.is_empty());
}

#[test]
fn toggle_comment_marks_a_selection_at_its_smallest_indent_as_sublime_does() {
    let text = "def f():\n    a\n        b\n    c\nz\n";
    let commented = "def f():\n    # a\n    #     b\n    # c\nz\n";
    let mut s = selecting(text, &[(9, 31)]);
    s.toggle_comment("#");
    assert_eq!((s.text().as_str(), s.view.selections[0]), (commented, Selection::new(9, 37)));
    s.toggle_comment("#");
    assert_eq!((s.text().as_str(), s.view.selections[0]), (text, Selection::new(9, 31)));
    // a selection starting at the marker column goes past the marker, and comes back
    s.view.selections = vec![Selection::new(13, 30)];
    s.toggle_comment("#");
    assert_eq!(s.view.selections[0], Selection::new(15, 36));
    s.toggle_comment("#");
    assert_eq!(s.view.selections[0], Selection::new(13, 30));
    // a caret uses its own line's indent; one before the marker stays
    let mut s = st(text, 23);
    s.toggle_comment("#");
    assert_eq!((s.line(2), s.cursor()), ("        # b".into(), 25));
    s.toggle_comment("#");
    assert_eq!(s.cursor(), 23);
    let mut s = st(text, 15);
    s.toggle_comment("#");
    assert_eq!(s.cursor(), 15);
    let mut s = st(text, 11);
    s.add_cursor(24);
    s.toggle_comment("#");
    assert_eq!(s.text(), "def f():\n    # a\n        # b\n    c\nz\n");
    // blank lines are skipped, tabs keep their place, removal takes one space
    let mut s = st("    a\n\n        b\n    c\n", 0);
    s.select_all();
    s.toggle_comment("#");
    assert_eq!(s.text(), "    # a\n\n    #     b\n    # c\n");
    let mut s = st("\ta\n\t\tb\n\tc\n", 0);
    s.select_all();
    s.toggle_comment("#");
    assert_eq!(s.text(), "\t# a\n\t# \tb\n\t# c\n");
    let mut s = st("    #a\n    #  b\n", 0);
    s.select_all();
    s.toggle_comment("#");
    assert_eq!(s.text(), "    a\n     b\n");
}

#[test]
fn two_carets_on_one_line_toggle_its_bookmark_twice_as_in_sublime() {
    let mut s = st("ab\ncd", 0);
    s.add_cursor(1);
    s.toggle_bookmark();
    assert!(s.view.bookmarks.is_empty(), "Sublime 4215 leaves it unmarked too");
}

#[test]
fn commands_that_change_nothing_leave_the_buffer_clean_and_keep_redo() {
    // unlike Sublime 4215, which marks these dirty
    type Command = fn(&mut EditorState);
    let noops: Vec<(&str, Command)> = vec![
        ("FOO", |s| s.convert_case(Case::Upper)),
        ("a\nb", |s| s.permute_lines(sort(true))),
        ("a\nb", |s| s.permute_lines(Permute::Unique)),
        ("", |s| s.delete_line()),
        ("ab", |s| s.wrap_lines(78, None)),
        ("    x", |s| s.convert_indentation(true)),
        ("x", |s| {
            s.replace_all("x", "x", &FindOpts::default());
        }),
        ("a", |s| s.unindent()),
    ];
    for (text, f) in noops {
        let mut s = st(text, 0);
        s.select_all();
        s.insert(text);
        s.save_to(fresh_dir("orlo-buffer-files").join("orlo-noop.txt")).unwrap();
        s.insert("z");
        s.undo();
        assert!(!s.doc.dirty);
        s.select_all();
        f(&mut s);
        assert!(!s.doc.dirty, "{text:?} became dirty");
        s.redo();
        assert!(s.text().contains('z'), "{text:?}: redo was cleared");
    }
}

#[test]
fn copy_and_cut_with_a_caret_beside_a_selection_take_only_the_selection() {
    let text = "line one\nline two\nline three\n";
    let mut s = st(text, 2);
    s.view.selections.push(Selection::new(14, 17));
    assert_eq!(s.copy_text(), ("two".into(), false));
    assert_eq!(s.cut().0, "two");
    assert_eq!(s.text(), "line one\nline \nline three\n");
    assert_eq!(s.view.selections, cs(&[2, 14]));
    // all carets: their lines, each once
    let mut s = st(text, 2);
    s.add_cursor(4);
    assert_eq!(s.copy_text(), ("line one\n".into(), true));
    s.cut();
    assert_eq!(s.text(), "line two\nline three\n");
    let mut s = st(text, 2);
    s.add_cursor(21);
    assert_eq!(s.copy_text().0, "line one\nline three\n");
}

#[test]
fn drags_keep_their_anchor_and_grow_by_the_unit_they_started_with() {
    let text = "alpha beta gamma\ndelta eps\nzeta\n";
    // a drag backward across an earlier selection keeps its own anchor
    let mut s = st(text, 0);
    s.set_selections(vec![Selection::new(6, 10)]);
    let cmd_press = s.drag_from(14, 1, true, false);
    s.drag_to(&cmd_press, 2);
    s.drag_to(&cmd_press, 15);
    assert_eq!(s.view.selections, sels(&[(6, 10), (14, 15)]));
    // double-click drag: whole words, the first word kept either way
    let mut s = st(text, 0);
    let dbl = s.drag_from(2, 2, false, false);
    s.drag_to(&dbl, 13);
    assert_eq!(s.view.selections, vec![Selection::new(0, 16)]);
    let dbl = s.drag_from(13, 2, false, false);
    s.drag_to(&dbl, 2);
    assert_eq!(s.view.selections, vec![Selection::new(16, 0)]);
    // triple-click drag: whole lines, and cmd keeps the other carets
    let mut s = st(text, 28);
    let triple = s.drag_from(2, 3, true, false);
    s.drag_to(&triple, 20);
    assert_eq!(s.view.selections, sels(&[(0, 27), (28, 28)]));
    // option-drag: a box, a selection per row
    let mut s = st(text, 0);
    s.drag_box(&[], false, DisplayPoint::new(0, 2), DisplayPoint::new(2, 4), &Lines);
    let got: Vec<_> = s.view.selections.iter().map(|x| s.doc.rope.slice(x.range()).to_string()).collect();
    assert_eq!(got, ["ph", "lt", "ta"]);
    assert_eq!(s.cursor(), 31, "the row under the mouse leads");
}

#[test]
fn alt_cmd_shift_drag_takes_a_box_out_as_sublime_does() {
    // Sublime's mousemap: alt+shift+super, drag_select by columns, subtractive
    let mut s = st("abcdef\nabcdef\nabcdef\n", 0);
    s.set_selections(sels(&[(0, 6), (10, 10), (14, 20)]));
    let from = s.drag_from(2, 1, true, true);
    assert!(from.subtract);
    s.drag_box(&from.others, from.subtract, DisplayPoint::new(0, 2), DisplayPoint::new(2, 4), &Lines);
    assert_eq!(s.view.selections, sels(&[(0, 2), (4, 6), (14, 16), (18, 20)]));
    // nothing left: a caret at the press
    let mut s = st("abcdef\nabcdef\n", 0);
    s.set_selections(vec![Selection::new(2, 4)]);
    let from = s.drag_from(1, 1, true, true);
    s.drag_box(&from.others, from.subtract, DisplayPoint::new(0, 1), DisplayPoint::new(1, 5), &Lines);
    assert_eq!(s.view.selections, [Selection::caret(1)]);
}

#[test]
fn block_toggle_comment_is_a_toggle_for_selections_as_in_sublime() {
    // Sublime 4215, CSS: (selection, text and selection after one toggle)
    let text = "foo bar\nbaz\n";
    let wrapped = "/*foo bar*/\nbaz\n";
    for ((a, h), (a1, h1)) in [((0, 3), (2, 5)), ((0, 8), (2, 12)), ((0, 7), (2, 9))] {
        let mut s = selecting(text, &[(a, h)]);
        s.toggle_comment_with_block("/*", "*/");
        assert_eq!((s.text().as_str(), s.view.selections[0]), (wrapped, Selection::new(a1, h1)));
        s.toggle_comment_with_block("/*", "*/");
        assert_eq!((s.text().as_str(), s.view.selections[0]), (text, Selection::new(a, h)));
    }
    let mut s = st("  foo\n", 3);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!((s.text().as_str(), s.cursor()), ("  /*foo*/\n", 5), "after the indent");
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!((s.text().as_str(), s.cursor()), ("  foo\n", 3));
    // just inside the markers, or anywhere in the comment: it comes off
    let mut s = selecting("/*foo*/ bar\n", &[(2, 5)]);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!((s.text().as_str(), s.view.selections[0]), ("foo bar\n", Selection::new(0, 3)));
    let mut s = st("a /* b */ c\n", 6);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!((s.text().as_str(), s.cursor()), ("a  b  c\n", 4));
    let mut s = st("<!-- x -->\n", 6);
    s.toggle_comment_with_block("<!-- ", " -->");
    assert_eq!((s.text().as_str(), s.cursor()), ("x\n", 1));
}

#[test]
fn toggle_comment_wraps_lines_in_block_pairs_where_there_is_no_line_comment() {
    // Sublime 4215
    let css = "body { color: red; }\np { margin: 0; }\n";
    let mut s = st(css, 2);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!(s.text(), "/*body { color: red; }*/\np { margin: 0; }\n");
    assert_eq!(s.cursor(), 4);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!((s.text().as_str(), s.cursor()), (css, 2));
    let mut s = selecting(css, &[(0, 21)]);
    s.toggle_comment_with_block("/*", "*/");
    assert_eq!(s.text(), "/*body { color: red; }*/\np { margin: 0; }\n", "the line break stays out");
    let html = "<div>hi</div>\n<p>x</p>\n";
    let mut s = st(html, 2);
    s.toggle_comment_with_block("<!-- ", " -->");
    assert_eq!((s.line(0), s.cursor()), ("<!-- <div>hi</div> -->".into(), 7));
    s.toggle_comment_with_block("<!-- ", " -->");
    assert_eq!((s.text().as_str(), s.cursor()), (html, 2));
}
