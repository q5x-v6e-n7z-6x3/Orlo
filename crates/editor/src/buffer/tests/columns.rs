use super::*;

#[test]
fn column_moves_and_deletions_keep_a_literal_cr_in_the_remaining_cell() {
    let text = "a\r,b\nc,d\n";
    let mut s = EditorState::from_text(text);
    column_command(&mut s, ColumnOp::MoveRight).unwrap();
    assert_eq!(s.text(), "b,\"a\r\"\nd,c\n");
    s.undo();
    assert_eq!(s.text(), text);
    s.set_cursor(3);
    column_command(&mut s, ColumnOp::Delete).unwrap();
    assert_eq!(s.text(), "\"a\r\"\nc\n");
    s.undo();
    assert_eq!(s.text(), text);
}

#[test]
fn a_column_plan_does_not_apply_to_a_replacement_buffer_at_the_same_version() {
    let mut s = EditorState::from_text("a,b\nc,d");
    s.set_cursor(2);
    let job = s.column_job(ColumnOp::Delete);
    assert!(job.is_current(&s));
    let mut replacement = EditorState::from_text("longer,other\nnew,content");
    replacement.set_cursor(2);
    assert_eq!(s.doc.version, replacement.doc.version);
    assert!(!job.is_current(&replacement));
}

#[test]
fn column_editor_text_goes_in_at_every_caret_as_one_undo_step() {
    let mut s = carets("ab\ncd\nef", &[1, 4, 7]);
    s.insert_per_selection(&strings(&["X", "X", "X"]), true, 4);
    assert_eq!(s.text(), "aXb\ncXd\neXf");
    assert_eq!(carets_of(&s), [2, 6, 10]);
    let carets = |at: &[usize]| at.iter().map(|&o| Selection::caret(o)).collect::<Vec<_>>();
    assert_undo_redo(&mut s, ("ab\ncd\nef", carets(&[1, 4, 7])), ("aXb\ncXd\neXf", carets(&[2, 6, 10])));
    // one step: a single undo took all three
    s.undo();
    assert!(!s.doc.dirty);
}

#[test]
fn column_editor_replaces_a_selection_and_numbers_each_in_document_order() {
    let mut s = selecting("a\nb\nc", &[(4, 5), (0, 1), (2, 3)]);
    s.insert_per_selection(&strings(&["1", "2", "3"]), true, 4);
    assert_eq!(s.text(), "1\n2\n3");
    assert_eq!(carets_of(&s), [1, 3, 5]);
}

#[test]
fn pad_short_lines_lines_the_text_up_at_the_rightmost_caret_that_is_not_at_a_line_end() {
    let at = [3, 9, 11, 15];
    let mut s = carets("abcdef\nab\na\nabc", &at);
    s.insert_per_selection(&strings(&["X"; 4]), true, 4);
    assert_eq!(s.text(), "abcXdef\nab X\na  X\nabcX");
    assert_eq!(carets_of(&s), [4, 12, 17, 22]);
    // off: the carets stay where they were
    let mut s = carets("abcdef\nab\na\nabc", &at);
    s.insert_per_selection(&strings(&["X"; 4]), false, 4);
    assert_eq!(s.text(), "abcXdef\nabX\naX\nabcX");
    // every caret at a line end: nothing to line up with
    let mut s = carets("ab\ncde", &[2, 6]);
    s.insert_per_selection(&strings(&["X", "X"]), true, 4);
    assert_eq!(s.text(), "abX\ncdeX");
}

#[test]
fn padding_measures_tabs_and_wide_characters_in_display_columns() {
    // after a tab (column 4) against a short line
    let mut s = carets("\tx\nab", &[1, 5]);
    s.insert_per_selection(&strings(&["X", "X"]), true, 4);
    assert_eq!(s.text(), "\tXx\nab  X");
    let mut s = carets("日本x\na", &[2, 5]);
    s.insert_per_selection(&strings(&["X", "X"]), true, 4);
    assert_eq!(s.text(), "日本Xx\na   X");
}

#[test]
fn nothing_to_insert_is_not_an_edit() {
    let mut s = carets("ab", &[1]);
    s.insert_per_selection(&strings(&[""]), true, 4);
    assert!(!s.doc.dirty);
    assert_eq!(s.text(), "ab");
}

#[test]
fn pad_to_column_brings_every_caret_to_the_rightmost_one() {
    let mut s = carets("abcd\nab\nabcdef", &[4, 7, 10]);
    s.pad_to_column(None, 4);
    assert_eq!(s.text(), "abcd\nab  \nab  cdef");
    assert_eq!(carets_of(&s), [4, 9, 14]);
    let carets = |at: &[usize]| at.iter().map(|&o| Selection::caret(o)).collect::<Vec<_>>();
    assert_undo_redo(&mut s, ("abcd\nab\nabcdef", carets(&[4, 7, 10])), ("abcd\nab  \nab  cdef", carets(&[4, 9, 14])));
}

#[test]
fn pad_to_an_explicit_column_leaves_carets_past_it_alone_and_measures_tabs() {
    let mut s = carets("abcd\nab\nabcdef", &[4, 7, 10]);
    s.pad_to_column(Some(6), 4);
    assert_eq!(s.text(), "abcd  \nab    \nab    cdef");
    let mut s = carets("\tab\nx", &[3, 5]);
    s.pad_to_column(None, 4);
    assert_eq!(s.text(), "\tab\nx     ");
    // already there: no edit, no undo step
    let mut s = carets("abcd\nab", &[4, 7]);
    s.pad_to_column(Some(1), 4);
    assert!(!s.doc.dirty);
    assert_eq!(s.text(), "abcd\nab");
}

#[test]
fn replace_ranges_is_one_undo_step_and_skips_what_would_not_change() {
    let mut s = carets("hello world", &[11]);
    let edits = vec![(0..5, "HELLO".to_string()), (6..11, "w".to_string())];
    assert!(s.replace_ranges(edits));
    assert_eq!(s.text(), "HELLO w");
    assert_eq!(carets_of(&s), [7]);
    s.undo();
    assert_eq!(s.text(), "hello world");
    assert!(!s.doc.dirty);
    s.redo();
    assert_eq!(s.text(), "HELLO w");
    // the same text again is no change
    let mut s = carets("same", &[0]);
    assert!(!s.replace_ranges(vec![(0..4, "same".to_string())]));
    assert!(!s.doc.dirty);
    assert!(!s.replace_ranges(Vec::new()));
}

#[test]
fn complete_ranges_puts_the_caret_after_the_items_text_whatever_the_other_edits_do() {
    let at = |n: usize| vec![Selection::caret(n)];
    let edit = |r: Range<usize>, t: &str| (r, t.to_string());
    // `pa` grows to `path`: the caret ends the word, not the shared `pa`, and the import
    // below leaves it be
    let mut s = carets("os.pa\nx\n", &[5]);
    assert!(s.complete_ranges(vec![edit(3..5, "path"), edit(6..6, "import\n")]));
    assert_eq!(s.text(), "os.path\nimport\nx\n");
    assert_undo_redo(&mut s, ("os.pa\nx\n", at(5)), ("os.path\nimport\nx\n", at(7)));
    // an import above moves it down with the word; the import's own end is not the caret
    let mut s = carets("os.pa\n", &[5]);
    assert!(s.complete_ranges(vec![edit(3..5, "path"), edit(0..0, "import os\n")]));
    assert_eq!((s.text().as_str(), carets_of(&s)), ("import os\nos.path\n", vec![17]));
    assert_undo_redo(&mut s, ("os.pa\n", at(5)), ("import os\nos.path\n", at(17)));
    // an edit that replaces the whole word, with the caret in it (`pa|rdir` to `path`)
    let mut s = carets("os.pardir\n", &[5]);
    assert!(s.complete_ranges(vec![edit(3..9, "path")]));
    assert_eq!((s.text().as_str(), carets_of(&s)), ("os.path\n", vec![7]));
    // only the primary caret follows the item; the other keeps its text
    let mut s = carets("a\nos.pa", &[1, 7]);
    s.view.primary = 1;
    assert!(s.complete_ranges(vec![edit(5..7, "path")]));
    assert_eq!((s.text().as_str(), carets_of(&s)), ("a\nos.path", vec![1, 9]));
    // an item that changes nothing leaves the caret and the undo stack alone
    let mut s = carets("os.path", &[7]);
    assert!(!s.complete_ranges(vec![edit(3..7, "path")]));
    assert_eq!((carets_of(&s), s.doc.dirty), (vec![7], false));
}

#[test]
fn column_commands_select_copy_move_delete_and_clear_with_undo_and_redo() {
    let text = "a,b,c\n1,2,3\nx\n";
    let at = |s: &mut EditorState| s.view.selections = vec![Selection::caret(2)];
    let mut s = st(text, 2);
    assert_eq!(column_command(&mut s, ColumnOp::Select), Ok(None));
    assert_eq!(s.view.selections, sels(&[(2, 3), (8, 9)]));
    assert_eq!(s.text(), text);
    at(&mut s);
    assert_eq!(column_command(&mut s, ColumnOp::Copy), Ok(Some("b\n2\n".to_string())));
    assert!(!s.doc.dirty, "copying changes nothing");
    for (op, want) in [
        (ColumnOp::MoveRight, "a,c,b\n1,3,2\nx\n"),
        (ColumnOp::MoveLeft, "b,a,c\n2,1,3\nx\n"),
        (ColumnOp::Delete, "a,c\n1,3\nx\n"),
        (ColumnOp::Clear, "a,,c\n1,,3\nx\n"),
    ] {
        let mut s = st(text, 2);
        assert_eq!(column_command(&mut s, op), Ok(None));
        assert_eq!(s.text(), want, "{op:?}");
        assert!(s.doc.dirty);
        s.undo();
        assert_eq!((s.text().as_str(), carets_of(&s)), (text, vec![2]), "undo {op:?}");
        s.redo();
        assert_eq!(s.text(), want, "redo {op:?}");
    }
}

#[test]
fn a_selection_across_lines_limits_the_records_and_one_line_does_not() {
    let mut s = selecting("a,b\n1,2\n3,4\n", &[(2, 6)]);
    column_command(&mut s, ColumnOp::Delete).unwrap();
    assert_eq!(s.text(), "a\n1\n3,4\n");
    let mut s = selecting("a,b\n1,2\n", &[(0, 1)]);
    column_command(&mut s, ColumnOp::Delete).unwrap();
    assert_eq!(s.text(), "b\n2\n");
}

#[test]
fn an_unclosed_quote_fails_every_command_and_changes_nothing() {
    let text = "a,b\n\"oops,1\nc,d\n";
    for op in
        [ColumnOp::Select, ColumnOp::Copy, ColumnOp::MoveLeft, ColumnOp::MoveRight, ColumnOp::Delete, ColumnOp::Clear]
    {
        let mut s = st(text, 2);
        let (version, sels) = (s.doc.version, s.view.selections.clone());
        assert_eq!(
            column_command(&mut s, op),
            Err("Unclosed quote in the record starting on line 2".to_string()),
            "{op:?}"
        );
        assert_eq!((s.text().as_str(), s.doc.version, s.view.selections.clone()), (text, version, sels));
        assert!(!s.doc.dirty);
    }
}

#[test]
fn a_command_on_text_without_columns_uses_comma_and_an_empty_document_does_nothing() {
    let mut s = st("one\ntwo\n", 1);
    assert_eq!(column_command(&mut s, ColumnOp::Delete), Ok(None));
    assert_eq!(s.text(), "\n\n");
    let mut s = st("", 0);
    for op in [ColumnOp::Select, ColumnOp::Copy, ColumnOp::Delete] {
        assert_eq!(column_command(&mut s, op), Ok(None));
    }
    assert!(!s.doc.dirty);
}

#[test]
fn the_delimiter_is_detected_per_version_and_an_override_wins_in_every_view() {
    let mut v = views("a,b\n1,2\n");
    let mut s = v.a();
    assert_eq!(s.delimiter(), Delim::Comma);
    // the same document, edited: detection runs again
    s.view.selections = vec![Selection::new(0, s.len())];
    s.insert("a;b\n1;2\n");
    assert_eq!(s.delimiter(), Delim::Semicolon);
    s.set_delimiter(Some(Delim::Pipe));
    assert_eq!(v.b().delimiter_override(), Some(Delim::Pipe));
    assert_eq!(v.b().delimiter(), Delim::Pipe);
    // under that delimiter every record is one field: nothing moves
    let mut s = v.a();
    s.view.selections = vec![Selection::caret(2)];
    assert_eq!(column_command(&mut s, ColumnOp::MoveRight), Ok(None));
    assert_eq!(s.text(), "a;b\n1;2\n");
    s.set_delimiter(None);
    assert_eq!(v.b().delimiter(), Delim::Semicolon);
    let mut s = v.a();
    assert_eq!(column_command(&mut s, ColumnOp::MoveLeft), Ok(None));
    assert_eq!(s.text(), "b;a\n2;1\n");
}

#[test]
fn the_file_extension_picks_tab_or_comma_for_the_delimiter() {
    let mut s = st("a,b\n1,2\n", 0);
    s.doc.path = Some(PathBuf::from("data.tsv"));
    assert_eq!(s.delimiter(), Delim::Tab);
    s.doc.path = Some(PathBuf::from("data.csv"));
    assert_eq!(s.delimiter(), Delim::Comma);
}

#[test]
fn detection_on_a_huge_buffer_reads_only_the_first_records() {
    let mut text = "a,b,c\n".repeat(csv::DETECT_RECORDS);
    text.push_str(&"x|y|z\n".repeat(700_000));
    assert!(text.len() > 4_000_000);
    let s = st(&text, 0);
    assert_eq!(s.delimiter(), Delim::Comma);
    // one enormous line: still answers without reading it all
    let one_line = "a,b".repeat(2_000_000);
    assert_eq!(st(&one_line, 0).delimiter(), Delim::Comma);
}

#[test]
fn a_crlf_file_with_a_bom_saves_both_after_a_column_move() {
    let p = fresh_dir("orlo-buffer-files").join(format!("orlo_columns_{}.csv", std::process::id()));
    std::fs::write(&p, b"\xef\xbb\xbfa,\"b\r\nc\",d\r\n1,2,3\r\n").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    assert_eq!((s.doc.line_ending, s.doc.bom), ("\r\n", true));
    assert_eq!(s.text(), "a,\"b\nc\",d\n1,2,3\n");
    s.view.selections = vec![Selection::caret(0)];
    assert_eq!(column_command(&mut s, ColumnOp::MoveRight), Ok(None));
    s.save().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"\xef\xbb\xbf\"b\r\nc\",a,d\r\n2,1,3\r\n");
    s.undo();
    s.redo();
    s.save().unwrap();
    assert_eq!((s.doc.line_ending, s.doc.bom), ("\r\n", true));
    let _ = std::fs::remove_file(p);
}

#[test]
fn n10_pattern_aligns_delimiters_capture_and_two_columns_idempotently_in_one_undo() {
    for (source, pattern, capture, expected) in [
        ("a=1\nlong=2\nunmatched\n", "=", 0, "a   =1\nlong=2\nunmatched\n"),
        ("a=1\nlong=2\nunmatched", r"^(\w+)(=)", 2, "a   =1\nlong=2\nunmatched"),
        ("a=1 x=2\nlong=3 longer=4", "=", 0, "a   =1 x     =2\nlong=3 longer=4"),
        ("a=1\nlong=2", r"\s*=\s*", 0, "a   =1\nlong=2"),
        ("a=1\nlong=2", r"(\s*=\s*)", 1, "a   =1\nlong=2"),
    ] {
        let mut s = EditorState::from_text(source);
        let original = s.selection_state();
        let edits = s.align_pattern_edits(pattern, capture, false).unwrap();
        assert!(s.replace_ranges(edits), "{pattern}");
        assert_eq!(s.text(), expected, "{pattern}");
        assert!(s.align_pattern_edits(pattern, capture, false).unwrap().is_empty(), "{pattern}");
        s.undo();
        assert_eq!(s.text(), source);
        assert!(s.selection_state() == original);
        s.redo();
        assert_eq!(s.text(), expected);
    }
}

#[test]
fn n10_selected_fields_align_two_widths_with_tab_remeasurement_and_preserve_selection_origins() {
    let source = "a|x\t!\n日本|e\u{301}z!";
    let mut s = selecting(source, &[(1, 0), (2, 4), (6, 8), (9, 12)]);
    s.doc.tab_size = 4;
    s.set_virtual_space(true);
    s.view.selections[0].rectangular = true;
    s.view.selections.push(Selection::from_points((13, 2), (13, 5)).in_rectangle());
    s.view.primary = 2;
    let original = s.selection_state();
    let edits = s.align_selected_field_edits().unwrap();
    assert!(s.apply_selected_field_alignment(edits));
    assert_eq!(s.text(), "a   |x\t!\n日本|e\u{301}z !");
    assert!(
        s.align_selected_field_edits().unwrap().is_empty(),
        "{:?} {:?}",
        s.view.selections,
        s.align_selected_field_edits()
    );
    assert!(s.view.selections[0].rectangular);
    assert_eq!(s.view.primary, 2);
    assert!(s.view.selections.last().unwrap().has_virtual());
    s.undo();
    assert_eq!(s.text(), source);
    assert!(s.selection_state() == original);
    s.redo();
    assert_eq!(s.text(), "a   |x\t!\n日本|e\u{301}z !");
}

#[test]
fn n10_pattern_keeps_missing_capture_ordinals_and_whitespace_matches_stable() {
    for (source, pattern, capture, expected) in [
        ("q =x\nlong=x =y", r"q|=( *\w)", 1, "q =     x\nlong=x =y"),
        ("a =x\nlong  =y", r"([ \t]+)=", 1, "a     =x\nlong  =y"),
        ("a=1\nlong=2", r"()=", 1, "a=1\nlong=2"),
        ("a=1\nlong=2", r"x(=)?|=", 1, "a=1\nlong=2"),
    ] {
        let mut s = EditorState::from_text(source);
        let edits = s.align_pattern_edits(pattern, capture, false).unwrap();
        s.replace_ranges(edits);
        assert_eq!(s.text(), expected, "{pattern}");
        assert!(s.align_pattern_edits(pattern, capture, false).unwrap().is_empty(), "{pattern}");
    }
}

#[test]
fn n10_pattern_uses_source_display_columns_unicode_and_recomputed_tabs() {
    for (source, expected) in [
        ("日=1\ne\u{301}=2\n😀=3\n\t=4\nplain", "日  =1\ne\u{301}   =2\n😀  =3\n\t=4\nplain"),
        ("a=x\t=1\nlong=y\tz=2", "a   =x\t =1\nlong=y\tz=2"),
        ("a=1\r\nlong=2\r\n", "a   =1\r\nlong=2\r\n"),
    ] {
        let mut s = EditorState::from_text(source);
        s.doc.tab_size = 4;
        s.replace_ranges(s.align_pattern_edits("=", 0, false).unwrap());
        assert_eq!(s.text(), expected);
        assert!(s.align_pattern_edits("=", 0, false).unwrap().is_empty());
        s.undo();
        assert_eq!(s.text(), source);
    }
}

#[test]
fn n10_pattern_scopes_physical_lines_once_and_virtual_only_never_falls_back() {
    let source = "a=1\nlong=2\nx=3\nlengthy=4";
    let mut s = selecting(source, &[(0, 2), (2, 11)]);
    s.replace_ranges(s.align_pattern_edits("=", 0, false).unwrap());
    assert_eq!(s.text(), "a   =1\nlong=2\nx=3\nlengthy=4");
    s.undo();
    s.set_virtual_space(true);
    s.view.selections = vec![Selection::from_points((s.len(), 2), (s.len(), 5)).in_rectangle()];
    let before = (s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point());
    assert!(s.align_pattern_edits("=", 0, false).unwrap().is_empty());
    assert!(s.align_selected_field_edits().unwrap().is_empty());
    assert!((s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point()) == before);
    s.view.selections = vec![Selection::caret(0)];
    assert!(!s.align_pattern_edits("=", 0, false).unwrap().is_empty());
}

#[test]
fn n10_preflight_refuses_every_invalid_target_atomically_with_full_state_and_real_redo() {
    for source in ["a=1\ne\u{301}=2", "a=1\r\nlong=2"] {
        let mut s = EditorState::from_text(source);
        s.set_cursor(s.len());
        s.insert("!");
        s.undo();
        let bad = if source.contains('\r') { 4 } else { 5 };
        s.view.selections = vec![Selection::new(0, 1).in_rectangle(), Selection::new(bad, bad + 1)];
        s.view.selections[0].goal = Some(Goal::X(13.0));
        let before = (
            s.text(),
            s.selection_state(),
            s.selection_generation(),
            s.doc.version,
            s.doc.save_point(),
            s.doc.unsaved(),
        );
        assert!(s.align_selected_field_edits().is_err());
        assert!(s.align_pattern_edits("=", 0, false).is_err());
        assert!(
            (
                s.text(),
                s.selection_state(),
                s.selection_generation(),
                s.doc.version,
                s.doc.save_point(),
                s.doc.unsaved()
            ) == before
        );
        s.redo();
        assert_eq!(s.text(), format!("{source}!"));
    }
    let mut s = EditorState::from_text("a=1\nlong=2");
    for (pattern, capture) in [("[", 0), ("=", 1)] {
        assert!(s.align_pattern_edits(pattern, capture, false).is_err());
    }
    for ranges in
        [vec![Selection::new(0, 2), Selection::new(1, 3)], vec![Selection::new(0, 5)], vec![Selection::new(0, 99)]]
    {
        s.view.selections = ranges;
        assert!(s.align_selected_field_edits().is_err());
    }
    s.view.selections = vec![Selection::new(0, 1), Selection::new(4, 8)];
    s.doc.set_following(true);
    assert!(s.align_selected_field_edits().is_err());
    assert!(s.align_pattern_edits("=", 0, false).is_err());
    s.doc.set_following(false);
    s.doc.read_only = true;
    assert!(s.apply_selected_field_alignment(s.align_selected_field_edits().unwrap()));
    assert_eq!(s.text(), "a   =1\nlong=2");
}

#[test]
fn n10_pattern_capture_boundaries_never_split_a_grapheme_or_supplementary_cluster() {
    for (source, pattern) in [("a=1\ne\u{301}=2", r"\u{301}"), ("x=1\n👨‍👩‍👧=2", "👩")] {
        let s = EditorState::from_text(source);
        assert!(s.align_pattern_edits(pattern, 0, false).unwrap_err().contains("grapheme"));
        assert_eq!(s.text(), source);
    }
}

#[test]
fn n10_whitespace_only_literal_capture_is_idempotent_when_padding_outgrows_the_match() {
    for pattern in [r"a( )", r"( )"] {
        let mut s = EditorState::from_text("aa =x\na =y");
        s.replace_ranges(s.align_pattern_edits(pattern, 1, false).unwrap());
        assert_eq!(s.text(), "aa =x\na  =y");
        assert!(s.align_pattern_edits(pattern, 1, false).unwrap().is_empty(), "{pattern}");
    }
}

#[test]
fn n10_selected_adjacent_fields_keep_pads_origins_and_noop_real_redo() {
    let mut s = selecting("ab!\nlongx!", &[(1, 0), (2, 1), (4, 8), (8, 9)]);
    s.view.selections[0].rectangular = true;
    s.view.selections[0].goal = Some(Goal::X(2.0));
    let original = s.selection_state();
    assert!(s.apply_selected_field_alignment(s.align_selected_field_edits().unwrap()));
    assert_eq!(s.text(), "a   b!\nlongx!");
    assert_eq!(s.view.selections[0].range(), 0..4);
    assert_eq!(s.view.selections[0].head, 0);
    assert!(s.view.selections[0].rectangular);
    assert!(s.view.selections.iter().all(|s| s.goal.is_none()));
    assert!(s.align_selected_field_edits().unwrap().is_empty());
    s.undo();
    assert!(s.selection_state() == original);
    assert_eq!(s.text(), "ab!\nlongx!");
    s.set_selections(vec![Selection::caret(0)]);
    let before = (s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point());
    assert!(!s.apply_selected_field_alignment(s.align_selected_field_edits().unwrap()));
    assert!((s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point()) == before);
    s.redo();
    assert_eq!(s.text(), "a   b!\nlongx!");
}

#[test]
fn n10_padding_run_missing_captures_keep_normalized_field_slots_after_insertion() {
    let mut s = EditorState::from_text("a =x =z\nbbb =y =w");
    s.replace_ranges(s.align_pattern_edits(r"(=)| ", 1, false).unwrap());
    assert_eq!(s.text(), "a   =x =z\nbbb =y =w");
    assert!(s.align_pattern_edits(r"(=)| ", 1, false).unwrap().is_empty());
}

#[test]
fn n10_broad_captures_treat_new_horizontal_padding_as_padding_on_second_run() {
    let mut s = EditorState::from_text("日aa\nxaa");
    s.replace_ranges(s.align_pattern_edits(r"(.)", 1, false).unwrap());
    assert_eq!(s.text(), "日aa\nx aa");
    assert!(s.align_pattern_edits(r"(.)", 1, false).unwrap().is_empty());
}

#[test]
fn n10_new_missing_capture_padding_coalesces_with_the_following_physical_boundary() {
    let mut s = EditorState::from_text("日==\nx==");
    s.replace_ranges(s.align_pattern_edits(r"(=)|.", 1, false).unwrap());
    assert_eq!(s.text(), "日==\nx ==");
    assert!(s.align_pattern_edits(r"(=)|.", 1, false).unwrap().is_empty());
}

#[test]
fn n10_pattern_topology_changes_refuse_before_any_edit_and_preserve_real_redo() {
    let source = "a= a=\nxa= a=";
    let mut s = EditorState::from_text(source);
    s.set_cursor(s.len());
    s.insert("!");
    s.undo();
    let before =
        (s.text(), s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point(), s.doc.unsaved());
    assert!(s.align_pattern_edits(r"a(=)", 1, false).unwrap_err().contains("field columns"));
    assert!(
        (s.text(), s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point(), s.doc.unsaved())
            == before
    );
    s.redo();
    assert_eq!(s.text(), format!("{source}!"));
}

#[test]
fn n10_topology_refusal_accepts_the_corresponding_whitespace_tolerant_capture() {
    let mut s = EditorState::from_text("a= a=\nxa= a=");
    s.replace_ranges(s.align_pattern_edits(r"a\s*(=)", 1, false).unwrap());
    assert_eq!(s.text(), "a = a=\nxa= a=");
    assert!(s.align_pattern_edits(r"a\s*(=)", 1, false).unwrap().is_empty());
}

#[test]
fn n10_review_ordinary_caret_retains_document_scope_beside_a_virtual_only_selection() {
    let mut s = EditorState::from_text("a=1\nlong=2");
    s.set_virtual_space(true);
    s.set_selections(vec![Selection::caret(0), Selection::from_points((10, 2), (10, 5)).in_rectangle()]);
    let original = s.selection_state();
    assert!(s.replace_ranges(s.align_pattern_edits("=", 0, false).unwrap()));
    assert_eq!(s.text(), "a   =1\nlong=2");
    assert!(s.view.selections[1].rectangular);
    assert_eq!((s.view.selections[1].anchor_virtual, s.view.selections[1].head_virtual), (2, 5));
    assert!(s.align_pattern_edits("=", 0, false).unwrap().is_empty());
    s.undo();
    assert_eq!(s.text(), "a=1\nlong=2");
    assert!(s.selection_state() == original);
    s.redo();
    assert_eq!(s.text(), "a   =1\nlong=2");
}
