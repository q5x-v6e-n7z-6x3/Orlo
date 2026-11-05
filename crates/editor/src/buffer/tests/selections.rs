use super::*;

#[test]
fn quick_add_next_selects_word_then_occurrences_and_wraps() {
    let mut s = st("foo bar foo foo", 9);
    s.select_next_occurrence();
    assert_eq!(s.view.selections, vec![Selection::new(8, 11)]);
    s.select_next_occurrence();
    assert_eq!(s.view.selections, sels(&[(8, 11), (12, 15)]));
    s.select_next_occurrence();
    assert_eq!(s.view.selections, sels(&[(0, 3), (8, 11), (12, 15)]));
    s.select_next_occurrence();
    assert_eq!(s.view.selections.len(), 3);
}

#[test]
fn undo_selection_restores_ranges_direction_and_primary() {
    let mut s = st("one two three", 0);
    s.sel_checkpoint();
    // the second is reversed (anchor after head) and the primary
    s.set_selections(sels(&[(0, 3), (7, 4)]));
    s.sel_checkpoint();
    s.set_cursor(13);
    s.sel_checkpoint();
    assert!(s.undo_selection());
    assert_eq!(s.view.selections, sels(&[(0, 3), (7, 4)]));
    assert_eq!(s.primary(), 1);
    assert!(s.undo_selection());
    assert_eq!(s.view.selections, vec![Selection::caret(0)]);
    assert!(s.redo_selection());
    assert_eq!((s.view.selections.len(), s.primary()), (2, 1));
    assert!(s.redo_selection());
    assert_eq!(s.view.selections, vec![Selection::caret(13)]);
    assert!(!s.redo_selection());
}

#[test]
fn selection_undo_and_redo_keep_quick_add_whole_words() {
    let mut s = EditorState::from_text("one stone one one");
    s.sel_reset();
    s.select_next_occurrence();
    s.sel_checkpoint();
    s.select_next_occurrence();
    s.sel_checkpoint();
    assert!(s.undo_selection());
    assert!(s.redo_selection());
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, sels(&[(0, 3), (14, 17)]));
    assert!(s.undo_selection());
    assert!(s.undo_selection());
    s.select_next_occurrence();
    assert_eq!(s.view.selections, sels(&[(0, 3), (10, 13)]));
}

#[test]
fn an_empty_selection_history_changes_nothing() {
    let mut s = st("abc", 1);
    s.insert("x");
    s.sel_checkpoint();
    let undo_len = s.doc.history.undo.len();
    let (text, version, sels) = (s.text(), s.doc.version, s.view.selections.clone());
    assert!(!s.undo_selection());
    assert!(!s.redo_selection());
    assert_eq!((s.text(), s.doc.version, s.view.selections.clone()), (text, version, sels));
    assert!(s.doc.dirty);
    assert_eq!(s.doc.history.undo.len(), undo_len);
}

#[test]
fn an_edit_clears_both_selection_stacks() {
    let mut s = st("abc def", 0);
    s.set_selection(0, 3);
    s.sel_checkpoint();
    s.set_cursor(5);
    s.sel_checkpoint();
    assert!(s.undo_selection());
    assert_eq!(sel_depth(&s), (1, 1));
    s.insert("x");
    s.sel_checkpoint();
    assert_eq!(sel_depth(&s), (0, 0));
    // a text undo is an edit too
    s.set_selection(0, 2);
    s.sel_checkpoint();
    s.set_cursor(0);
    s.sel_checkpoint();
    s.undo();
    s.sel_checkpoint();
    assert_eq!(sel_depth(&s), (0, 0));
}

#[test]
fn another_views_edit_maps_the_stored_selections() {
    let mut v = views("abc def");
    let mut b = v.b();
    b.set_selection(4, 7);
    b.sel_checkpoint();
    b.set_cursor(0);
    b.sel_checkpoint();
    v.a().set_cursor(0);
    v.a().insert("XY");
    let mut b = v.b();
    b.sel_checkpoint();
    assert_eq!(sel_depth(&b), (2, 0), "the stacks stay, and the follow is no new state");
    assert!(b.undo_selection());
    assert_eq!(b.view.selections, vec![Selection::new(6, 9)]);
    assert!(b.redo_selection());
}

#[test]
fn a_new_selection_after_undo_selection_clears_redo() {
    let mut s = st("abc def", 0);
    s.set_selection(0, 3);
    assert!(s.undo_selection());
    assert_eq!(sel_depth(&s), (0, 1));
    s.set_cursor(5);
    s.sel_checkpoint();
    assert_eq!(sel_depth(&s), (1, 0));
    assert!(!s.redo_selection());
    assert_eq!(s.view.selections, vec![Selection::caret(5)]);
}

#[test]
fn selection_history_skips_repeats_and_keeps_128() {
    let mut s = st(&"x".repeat(300), 0);
    for _ in 0..3 {
        s.sel_checkpoint();
    }
    assert_eq!(sel_depth(&s), (0, 0));
    for i in 1..200 {
        s.set_cursor(i);
        s.sel_checkpoint();
        s.sel_checkpoint();
    }
    assert_eq!(sel_depth(&s), (128, 0));
    for _ in 0..128 {
        assert!(s.undo_selection());
    }
    assert!(!s.undo_selection());
    // the oldest were dropped: the 128th state back is caret 71 (199 - 128)
    assert_eq!(s.view.selections, vec![Selection::caret(71)]);
}

#[test]
fn quick_skip_next_replaces_the_latest_with_the_next_occurrence() {
    let mut s = st("ab ab ab", 0);
    s.select_next_occurrence();
    s.select_next_occurrence();
    assert_eq!(s.view.selections, sels(&[(0, 2), (3, 5)]));
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, sels(&[(0, 2), (6, 8)]));
    assert_eq!(s.primary(), 1);
    // cmd-d goes on from the one the skip chose
    s.select_next_occurrence();
    assert_eq!(s.view.selections.len(), 3);
}

#[test]
fn quick_skip_next_wraps_to_the_first_unselected() {
    let mut s = st("ab ab ab", 0);
    s.set_selections(sels(&[(3, 5), (6, 8)]));
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, sels(&[(0, 2), (3, 5)]));
    assert_eq!(s.primary(), 0);
    // and cmd-d continues from there, not from the rightmost selection
    s.select_next_occurrence();
    assert_eq!(s.view.selections.last(), Some(&Selection::new(6, 8)));
}

#[test]
fn quick_skip_next_passes_occurrences_already_selected() {
    let mut s = st("ab ab ab ab", 0);
    s.set_selections(sels(&[(3, 5), (0, 2)]));
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, sels(&[(3, 5), (6, 8)]));
}

#[test]
fn quick_skip_next_with_every_occurrence_selected_changes_nothing() {
    let mut s = st("ab ab", 0);
    s.set_selections(sels(&[(0, 2), (3, 5)]));
    let before = (s.view.selections.clone(), s.primary());
    assert!(!s.quick_skip_next());
    assert_eq!((s.view.selections.clone(), s.primary()), before);
}

#[test]
fn quick_skip_next_moves_one_selection_and_stays_when_it_is_alone() {
    let mut s = st("ab ab", 0);
    s.set_selection(0, 2);
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, vec![Selection::new(3, 5)]);
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, vec![Selection::new(0, 2)]);
    let mut one = st("ab cd", 0);
    one.set_selection(0, 2);
    assert!(!one.quick_skip_next());
    assert_eq!(one.view.selections, vec![Selection::new(0, 2)]);
}

#[test]
fn quick_skip_next_from_carets_selects_their_words_and_keeps_whole_words() {
    let mut s = st("foo foobar foo foo", 1);
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, vec![Selection::new(0, 3)]);
    // whole words only, as after cmd-d from a caret; the skip keeps it so
    assert!(s.quick_skip_next());
    assert_eq!(s.view.selections, vec![Selection::new(11, 14)]);
    s.select_next_occurrence();
    s.select_next_occurrence();
    assert_eq!(s.view.selections, sels(&[(0, 3), (11, 14), (15, 18)]));
}

#[test]
fn undo_selection_after_a_skip_brings_back_the_set_before_it() {
    let mut s = st("ab ab ab", 0);
    s.select_next_occurrence();
    s.select_next_occurrence();
    s.sel_checkpoint();
    assert!(s.quick_skip_next());
    assert!(s.undo_selection());
    assert_eq!(s.view.selections, sels(&[(0, 2), (3, 5)]));
    assert_eq!(s.primary(), 1);
    assert!(s.redo_selection());
    assert_eq!(s.view.selections, sels(&[(0, 2), (6, 8)]));
}

#[test]
fn split_into_lines_makes_a_selection_per_line() {
    let mut s = selecting("ab\ncd\nef", &[(0, 7)]);
    s.split_into_lines();
    assert_eq!(s.view.selections, sels(&[(0, 2), (3, 5), (6, 7)]));
}

#[test]
fn select_line_grows_to_full_lines() {
    let mut s = st("ab\ncd\n", 1);
    s.select_line();
    assert_eq!(s.view.selections, vec![Selection::new(0, 3)]);
}

#[test]
fn cmd_click_on_a_caret_keeps_it_and_single_selection_keeps_first() {
    // Sublime 4215: cmd-click never removes a caret
    let mut s = st("abc", 0);
    for _ in 0..2 {
        let cmd_click = s.drag_from(2, 1, true, false);
        s.drag_to(&cmd_click, 2);
        assert_eq!(s.view.selections, cs(&[0, 2]));
    }
    s.single_selection();
    assert_eq!(s.view.selections, vec![Selection::caret(0)]);
}

#[test]
fn double_click_selects_word() {
    let mut s = st("hello world", 0);
    s.select_word_at(7);
    assert_eq!(s.view.selections, vec![Selection::new(6, 11)]);
}

#[test]
fn syntax_selection_checkpoint_generation_detects_away_back_and_exact_snapshot_reversal() {
    let mut s = st("one two", 1);
    s.sel_checkpoint();
    let original = s.selection_state();
    let first = s.selection_generation();
    s.sel_checkpoint();
    assert_eq!(s.selection_generation(), first);
    s.set_cursor(2);
    s.sel_checkpoint();
    s.set_cursor(1);
    s.sel_checkpoint();
    assert!(s.selection_generation() > first);
    let next = s.selection_generation();
    s.restore_selection_state(original.clone());
    assert!(s.selection_state() == original);
    s.set_selections(sels(&[(3, 0), (4, 7)]));
    s.sel_checkpoint();
    let exact = s.selection_state();
    s.reselect(sels(&[(0, 7)]));
    s.restore_selection_state(exact.clone());
    assert!(s.selection_state() == exact);
    assert!(s.selection_generation() > next);
    let before = s.selection_generation();
    assert!(s.undo_selection());
    assert!(s.selection_generation() > before);
    let before = s.selection_generation();
    s.sel_reset();
    assert!(s.selection_generation() > before);
    assert!(!s.doc.dirty);
    assert!(s.doc.history.undo.is_empty());
}

#[test]
fn syntax_selection_generation_detects_replacements_away_back_between_checkpoints() {
    let mut s = st("one two", 1);
    s.sel_checkpoint();
    for operation in 0..4 {
        let before = s.selection_generation();
        match operation {
            0 => {
                s.set_cursor(2);
                s.set_cursor(1);
            }
            1 => {
                s.set_selection(0, 3);
                s.set_selection(1, 1);
            }
            2 => {
                s.set_selections(sels(&[(0, 3)]));
                s.set_selections(sels(&[(1, 1)]));
            }
            _ => {
                s.reselect(sels(&[(0, 3)]));
                s.reselect(sels(&[(1, 1)]));
            }
        }
        assert!(s.selection_generation() > before);
        assert_eq!(s.view.selections, [Selection::caret(1)]);
    }
}

#[test]
fn n08_invert_is_document_complement_without_source_or_text_history_change() {
    let mut s = selecting("abcd", &[(2, 1)]);
    let before = (
        s.text(),
        s.doc.version,
        s.doc.state,
        s.doc.save_point(),
        s.doc.dirty,
        s.doc.history.undo.len(),
        s.doc.history.redo.len(),
    );
    let original = s.selection_state();
    s.invert_selections();
    assert_eq!(s.view.selections, sels(&[(0, 1), (2, 4)]));
    assert_eq!(
        (
            s.text(),
            s.doc.version,
            s.doc.state,
            s.doc.save_point(),
            s.doc.dirty,
            s.doc.history.undo.len(),
            s.doc.history.redo.len()
        ),
        before
    );
    assert!(s.undo_selection());
    assert!(s.selection_state() == original);
    assert!(s.redo_selection());
    assert_eq!(s.view.selections, sels(&[(0, 1), (2, 4)]));
}

#[test]
fn n08_split_excludes_delimiters_and_keeps_reverse_direction() {
    let mut s = selecting("a,二,😀", &[(5, 0)]);
    s.regex_selections(",", SelectionSetOp::Split, false).unwrap();
    assert_eq!(s.view.selections, sels(&[(1, 0), (3, 2), (5, 4)]));
    assert_eq!(s.primary(), 2);
    assert_eq!(s.text(), "a,二,😀");
}

#[test]
fn n08_keep_tests_each_complete_selected_string_and_retains_identity() {
    let mut s = selecting("cat dog", &[(3, 0), (4, 7)]);
    s.view.primary = 0;
    s.view.selections[0].goal = Some(Goal::X(8.));
    s.view.selections[0].rectangular = true;
    let keep = s.view.selections[0];
    s.regex_selections("^cat$", SelectionSetOp::Keep, false).unwrap();
    assert_eq!(s.view.selections, [keep]);
    assert_eq!(s.primary(), 0);
}

#[test]
fn n08_remove_tests_complete_selected_ranges() {
    let mut s = selecting("cat dog", &[(0, 3), (7, 4)]);
    let keep = s.view.selections[1];
    s.regex_selections("cat", SelectionSetOp::Remove, false).unwrap();
    assert_eq!(s.view.selections, [keep]);
}

#[test]
fn n08_empty_zero_width_unicode_and_virtual_selection_policy() {
    for (query, expected) in
        [("", vec![(0, 1), (1, 2), (2, 3)]), ("(?=二)", vec![(0, 1), (1, 3)]), ("^|$", vec![(0, 3)])]
    {
        let mut s = selecting("a二😀", &[(0, 3)]);
        s.regex_selections(query, SelectionSetOp::Split, false).unwrap();
        assert_eq!(s.view.selections, sels(&expected), "{query}");
    }
    let mut combining = selecting("e\u{301}😀", &[(0, 3)]);
    combining.regex_selections("", SelectionSetOp::Split, false).unwrap();
    assert_eq!(combining.view.selections, sels(&[(0, 1), (1, 2), (2, 3)]));
    let mut s = selecting(",,", &[(2, 0)]);
    s.regex_selections(",", SelectionSetOp::Split, false).unwrap();
    assert_eq!(s.view.selections, [Selection::caret(0)]);
    let mut s = st("", 0);
    s.invert_selections();
    assert_eq!(s.view.selections, [Selection::caret(0)]);
    let mut s = st("a,b", 3);
    s.set_virtual_space(true);
    s.view.selections = vec![Selection::from_points((0, 0), (3, 4)).in_rectangle()];
    s.regex_selections(",", SelectionSetOp::Split, false).unwrap();
    assert_eq!(
        s.view.selections,
        [Selection::new(0, 1).in_rectangle(), Selection::from_points((2, 0), (3, 4)).in_rectangle()]
    );
    s.view.selections = vec![Selection::from_points((3, 4), (3, 2)).in_rectangle()];
    let original = s.view.selections[0];
    s.regex_selections("^$", SelectionSetOp::Keep, false).unwrap();
    assert_eq!(s.view.selections, [original]);
    s.invert_selections();
    assert_eq!(s.view.selections, [Selection::new(0, 3)]);
    assert!(s.undo_selection());
    assert_eq!(s.view.selections, [original]);
}

#[test]
fn n08_overlap_union_predicates_context_and_primary_are_deterministic() {
    let mut s = st("abcdef", 0);
    s.view.selections = sels(&[(3, 1), (2, 5), (6, 6)]);
    s.invert_selections();
    assert_eq!(s.view.selections, sels(&[(0, 1), (5, 6)]));
    let mut s = selecting("cat dog fox", &[(0, 3), (7, 4), (8, 11)]);
    s.view.primary = 1;
    s.regex_selections("dog", SelectionSetOp::Remove, false).unwrap();
    assert_eq!(s.view.selections, sels(&[(0, 3), (8, 11)]));
    assert_eq!(s.primary(), 0);
    s.regex_selections("(?<!x)^fox$", SelectionSetOp::Keep, false).unwrap();
    assert_eq!(s.view.selections, [Selection::new(8, 11)]);
    let mut s = selecting("cat", &[(1, 2)]);
    s.regex_selections("(?<=c)a", SelectionSetOp::Keep, false).unwrap();
    assert_eq!(s.view.selections, [Selection::caret(2)], "lookaround cannot see outside selected string");
}

#[test]
fn n08_errors_noops_and_success_preserve_source_redo_and_saved_identity() {
    let mut s = st("Cat dog", 0);
    s.insert("x");
    s.undo();
    s.set_selections(sels(&[(0, 3), (4, 7)]));
    s.sel_checkpoint();
    let before = (
        s.text(),
        s.doc.version,
        s.doc.state,
        s.doc.save_point(),
        s.doc.dirty,
        s.doc.history.undo.len(),
        s.doc.history.redo.len(),
    );
    let selection = s.selection_state();
    let generation = s.selection_generation();
    for q in ["[", r"\Gfoo", r"\K"] {
        assert!(s.regex_selections(q, SelectionSetOp::Keep, false).is_err());
        assert!(s.selection_state() == selection);
        assert_eq!(s.selection_generation(), generation);
    }
    s.regex_selections("nomatch", SelectionSetOp::Remove, false).unwrap();
    assert!(s.selection_state() == selection);
    assert_eq!(s.selection_generation(), generation);
    s.regex_selections("cat", SelectionSetOp::Keep, true).unwrap();
    assert_eq!(s.view.selections, [Selection::new(0, 3)]);
    assert_eq!(
        (
            s.text(),
            s.doc.version,
            s.doc.state,
            s.doc.save_point(),
            s.doc.dirty,
            s.doc.history.undo.len(),
            s.doc.history.redo.len()
        ),
        before
    );
    s.redo();
    assert_eq!(s.text(), "xCat dog");
    let mut s = selecting(&"a".repeat(100), &[(0, 100)]);
    let original = s.selection_state();
    assert!(s.regex_selections(r"^(a|a)*\1b", SelectionSetOp::Remove, false).unwrap_err().contains("backtracked"));
    assert!(s.selection_state() == original);
}

#[test]
fn n08_follow_and_readonly_allow_source_immutable_selection_metadata() {
    let mut s = selecting("abc", &[(1, 2)]);
    s.doc.set_following(true);
    let before = (s.text(), s.doc.version, s.doc.save_point());
    s.invert_selections();
    assert_eq!(s.view.selections, sels(&[(0, 1), (2, 3)]));
    s.regex_selections("a", SelectionSetOp::Keep, false).unwrap();
    assert_eq!(s.view.selections, [Selection::new(0, 1)]);
    assert_eq!((s.text(), s.doc.version, s.doc.save_point()), before);
    s.doc.set_following(false);
    s.doc.read_only = true;
    s.invert_selections();
    assert_eq!(s.view.selections, [Selection::new(1, 3)]);
}

#[test]
fn n08_split_rejects_crlf_endpoints_atomically_before_history_mutation() {
    for query in [r",|\r", "", r"(?=\n)"] {
        let mut s = st("a,b x\r\ny", 8);
        s.insert("z");
        s.undo();
        s.set_selections(sels(&[(0, 3), (8, 4)]));
        s.view.primary = 1;
        s.view.selections[0].goal = Some(Goal::X(7.));
        s.view.selections[0].rectangular = true;
        s.sel_checkpoint();
        let original = s.selection_state();
        let before = (
            s.text(),
            s.doc.version,
            s.doc.state,
            s.doc.save_point(),
            s.doc.dirty,
            s.doc.history.undo.len(),
            s.doc.history.redo.len(),
            s.selection_generation(),
            sel_depth(&s),
        );
        let error = s.regex_selections(query, SelectionSetOp::Split, false).unwrap_err();
        assert!(error.contains("CRLF"), "{query}: {error}");
        assert!(s.selection_state() == original, "no unrelated target or goal changes");
        assert_eq!(
            (
                s.text(),
                s.doc.version,
                s.doc.state,
                s.doc.save_point(),
                s.doc.dirty,
                s.doc.history.undo.len(),
                s.doc.history.redo.len(),
                s.selection_generation(),
                sel_depth(&s)
            ),
            before
        );
        s.redo();
        assert_eq!(s.text(), "a,b x\r\nyz");
    }
}

#[test]
fn n08_split_whole_crlf_and_unicode_delimiters_remains_selectable_and_undoable() {
    let mut s = selecting("a\r\n二,😀", &[(6, 0)]);
    let original = s.selection_state();
    let before = (s.text(), s.doc.version, s.doc.save_point());
    s.regex_selections(r"\r\n|,", SelectionSetOp::Split, false).unwrap();
    assert_eq!(s.view.selections, sels(&[(1, 0), (4, 3), (6, 5)]));
    assert_eq!((s.text(), s.doc.version, s.doc.save_point()), before);
    assert!(s.undo_selection());
    assert!(s.selection_state() == original);
    assert!(s.redo_selection());
    assert_eq!(s.view.selections, sels(&[(1, 0), (4, 3), (6, 5)]));
}
