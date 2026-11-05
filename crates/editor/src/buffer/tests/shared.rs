use super::*;

#[test]
fn adopting_a_view_preserves_highlights_and_clamps_them_to_the_new_document() {
    let mut source = EditorState::from_text("one\ntwo\nthree");
    source.add_highlights(0, [0..3, 4..7, 8..13]);
    source.add_highlights(2, [5..11]);
    let (_, view) = source.into_parts();
    let doc = Document::from_text("one\ntw");
    let adopted = doc.adopt_view(view);
    assert_eq!(adopted.highlights[0], vec![0..3, 4..6]);
    assert_eq!(adopted.highlights[2], vec![5..6]);
    assert!(adopted.highlights[1].is_empty());
}

#[test]
fn typing_in_one_view_shows_in_the_other_and_shifts_its_caret() {
    let mut v = views("abc");
    v.b().set_cursor(2);
    v.a().set_cursor(0);
    v.a().insert("XY");
    let b = v.b();
    assert_eq!(b.text(), "XYabc");
    assert_eq!(b.cursor(), 4);
    assert!(b.doc.dirty);
}

#[test]
fn deleting_around_a_follower_caret_moves_it_to_the_cut() {
    let mut v = views("abcdefg");
    v.b().set_cursor(3);
    v.a().set_selection(1, 5);
    v.a().backspace();
    assert_eq!(v.b().text(), "afg");
    assert_eq!(v.b().cursor(), 1);
}

#[test]
fn multi_cursor_changes_map_in_order() {
    let mut v = views("abcdefgh");
    v.a().view.selections = cs(&[0, 4]);
    v.b().view.selections = cs(&[2, 6]);
    v.a().insert("x");
    let b = v.b();
    assert_eq!(b.text(), "xabcdxefgh");
    assert_eq!(b.view.selections, cs(&[3, 8]));
}

#[test]
fn undo_in_the_other_view_undoes_the_shared_history() {
    let mut v = views("abc");
    v.a().set_cursor(3);
    v.a().insert("d");
    v.b().undo();
    assert_eq!((v.a().text(), v.b().text()), ("abc".into(), "abc".into()));
    v.a().redo();
    assert_eq!((v.a().text(), v.b().text()), ("abcd".into(), "abcd".into()));
}

#[test]
fn undo_in_the_other_view_keeps_its_own_selection() {
    let mut v = views("hello");
    v.b().set_cursor(5);
    v.a().set_cursor(0);
    v.a().insert("XX");
    assert_eq!(v.b().cursor(), 7);
    v.b().undo();
    assert_eq!(v.b().text(), "hello");
    assert_eq!(v.b().cursor(), 5, "b keeps its caret instead of jumping to a's");
    // the view that made the edit gets its own selection back
    v.a().redo();
    v.a().undo();
    assert_eq!(v.a().cursor(), 0);
}

#[test]
fn undo_in_one_view_does_not_move_the_carets_of_the_view_that_recorded_it() {
    let mut v = views("one two");
    v.a().set_cursor(3);
    v.a().insert("!");
    v.a().set_cursor(8);
    v.b().set_cursor(0);
    // b undoes a's step: b gets no selections of a's, and a's caret is only mapped
    v.b().undo();
    assert_eq!(v.b().text(), "one two");
    assert_eq!(v.b().cursor(), 0);
    assert_eq!(v.a().cursor(), 7, "a's caret stays on its char");
    // redo from b brings back a's text but leaves both sets of carets where they were
    v.b().redo();
    assert_eq!((v.a().cursor(), v.b().cursor()), (8, 0));
}

#[test]
fn saving_from_one_view_is_the_save_point_of_every_view() {
    let p = fresh_dir("orlo-buffer-files").join("orlo-shared-save.txt");
    let mut v = views("abc");
    v.a().insert("x");
    assert!(v.b().doc.dirty);
    v.a().save_to(p.clone()).unwrap();
    assert!(!v.b().doc.dirty);
    assert_eq!(v.b().doc.path.as_deref(), Some(p.as_path()));
    // the save point is the document's: undo in b is dirty again, and redo is clean
    v.b().undo();
    assert!(v.b().doc.dirty);
    v.b().redo();
    assert!(!v.b().doc.dirty);
    let _ = std::fs::remove_file(p);
}

#[test]
fn every_view_reads_the_file_state_of_its_one_document() {
    let mut v = views("x");
    let p = fresh_dir("orlo-buffer-files").join("orlo-follow-fields.txt");
    {
        let mut a = v.a();
        a.doc.scratch = true;
        a.save_to(p.clone()).unwrap();
        a.doc.read_only = true;
        a.doc.deleted = true;
        a.doc.watching = false;
        a.set_line_ending("\r\n");
        a.set_encoding(orlo_project::encoding::Encoding::Utf16Le, true);
        a.doc.tab_size = 2;
        a.doc.translate_tabs = false;
        a.doc.indent_override = [true, false];
        a.doc.line_ending_override = true;
    }
    let b = v.b();
    assert_eq!(b.doc.path.as_deref(), Some(p.as_path()));
    assert!(b.doc.read_only && b.doc.deleted && !b.doc.watching);
    assert_eq!((b.doc.line_ending, b.doc.bom), ("\r\n", true));
    assert_eq!(b.doc.encoding, orlo_project::encoding::Encoding::Utf16Le);
    assert_eq!((b.doc.tab_size, b.doc.translate_tabs), (2, false));
    assert_eq!(b.indent_width(), 2, "indenting in b uses the document's width");
    assert_eq!((b.doc.indent_override, b.doc.line_ending_override), ([true, false], true));
    // Find Results saved as a file in one view: typing in the other now dirties it
    let mut b = v.b();
    b.doc.read_only = false;
    b.insert("y");
    assert!(b.doc.dirty);
    let _ = std::fs::remove_file(p);
}

#[test]
fn a_second_view_shares_the_document_and_keeps_its_own_marks() {
    let mut v = views("one\ntwo\nthree");
    {
        let mut a = v.a();
        a.set_cursor(4);
        a.toggle_bookmark();
        a.set_mark();
        a.add_highlights(0, [0..3]);
    }
    // a third view starts where the first is, on the same document
    let mut c = v.a.duplicate(&v.doc);
    assert_eq!(c.selections, v.a.selections);
    assert_eq!(c.bookmarks, v.a.bookmarks);
    c.bookmarks.clear();
    c.highlights[0].clear();
    c.mark = None;
    let mut b = v.b();
    b.set_cursor(8);
    b.delete_to_line_edge(true);
    // the document's one text, its own selections, bookmarks, mark, highlights and kill text
    assert_eq!(v.a().text(), "one\ntwo\n");
    assert_eq!(v.a().cursor(), 4);
    assert_eq!(v.a().view.bookmarks, vec![4]);
    assert_eq!(v.a().view.mark, Some(4));
    assert_eq!(v.a().view.highlights[0], vec![0..3]);
    assert!(v.a().view.kill.is_empty(), "b's kill text is b's");
    assert!(c.bookmarks.is_empty() && c.highlights[0].is_empty() && c.mark.is_none());
    assert_ne!(v.a.view_id(), c.view_id());
}

#[test]
fn opening_another_view_adds_view_state_not_another_document() {
    let doc = Document::from_text("abc");
    let a = doc.attach_view();
    let b = a.duplicate(&doc);
    let c = doc.attach_view();
    assert_eq!(doc.views.borrow().len(), 3, "three views of one document");
    assert_ne!(a.view_id(), b.view_id());
    assert_ne!(b.view_id(), c.view_id());
    assert!([&a, &b, &c].iter().all(|v| v.revision() == doc.version));
}

#[test]
fn each_view_applies_each_change_once_and_records_its_version() {
    let mut v = views("abc");
    v.b().set_cursor(3);
    let before = v.doc.version;
    v.a().set_cursor(0);
    v.a().insert("x");
    let change = v.a().take_change().expect("a change");
    assert_eq!((change.before, change.after, change.origin), (before, v.doc.version, v.a.view_id()));
    assert_eq!(v.a.selections, vec![Selection::caret(1)]);
    assert_eq!(change.changes.len(), 1);
    assert_eq!(v.b.revision(), before, "b has not applied it yet");
    // applying again does nothing: the change is b's once
    let mut b = v.b();
    assert_eq!(b.cursor(), 4);
    assert!(b.catch_up().is_empty());
    assert_eq!(b.cursor(), 4);
    assert_eq!(b.view.revision(), b.doc.version);
    // every view has it: the document lets it go
    assert!(v.doc.log.borrow().is_empty());
}

#[test]
fn a_view_catching_up_on_two_changes_maps_through_both_as_one_sequence() {
    // one update can make several edits (a macro) before the other view hears of any
    let mut v = views(&"line\n".repeat(10));
    {
        let mut b = v.b();
        b.set_cursor(40);
        b.toggle_bookmark();
        b.set_cursor(50);
    }
    v.a().set_cursor(0);
    v.a().insert(&"x".repeat(20));
    v.a().take_change();
    v.a().set_selection(0, 60);
    v.a().backspace();
    let b = v.b();
    assert_eq!(b.text(), "line\nline\n");
    assert_eq!(b.cursor(), 10, "mapped through both, not clamped between them");
    assert_eq!(b.view.bookmarks, vec![0]);
}

#[test]
fn a_closed_view_does_not_hold_changes_back() {
    let mut v = views("abc");
    let gone = v.a.duplicate(&v.doc);
    v.a().insert("x");
    v.a().take_change();
    assert_eq!(v.doc.log.borrow().len(), 1, "b and the third view have yet to apply it");
    drop(gone);
    v.b();
    assert!(v.doc.log.borrow().is_empty());
}

#[test]
fn follow_maps_bookmarks_and_mark() {
    let mut v = views("a\nb\nc");
    v.b().set_cursor(4);
    v.b().toggle_bookmark();
    v.b().set_mark();
    v.a().set_cursor(0);
    v.a().insert("zz\n");
    assert_eq!(v.b().view.bookmarks, vec![7]);
    assert_eq!(v.b().view.mark, Some(7));
    assert!(v.a().view.bookmarks.is_empty(), "bookmarks are per view");
}

#[test]
fn follower_typing_starts_a_new_undo_step() {
    let mut v = views("abc");
    v.b().set_cursor(3);
    v.b().insert("x");
    v.a().set_cursor(0);
    v.a().insert("Q");
    v.b().insert("y");
    v.b().undo();
    assert_eq!(v.b().text(), "Qabcx");
}

#[test]
fn alternating_typing_between_views_makes_one_undo_step_per_turn() {
    let mut v = views("");
    // each turn clicks at the end and types two keys
    for (view, keys) in [(0, "ab"), (1, "cd"), (0, "ef"), (1, "gh")] {
        let mut e = if view == 0 { v.a() } else { v.b() };
        let end = e.len();
        e.set_cursor(end);
        for k in keys.chars() {
            let mut e = if view == 0 { v.a() } else { v.b() };
            e.type_text(&k.to_string());
        }
    }
    assert_eq!(v.a().text(), "abcdefgh");
    let mut texts = Vec::new();
    for _ in 0..4 {
        v.a().undo();
        texts.push(v.b().text());
    }
    assert_eq!(texts, ["abcdef", "abcd", "ab", ""]);
}

#[test]
fn moving_the_other_views_caret_leaves_this_views_typing_group_open() {
    let mut v = views("");
    v.a().type_text("a");
    v.b().set_cursor(0);
    v.a().type_text("b");
    v.a().undo();
    assert_eq!(v.a().text(), "", "one step: b only moved its caret");
}

#[test]
fn appending_resetting_or_reloading_closes_the_other_views_typing_group() {
    // Find Results streaming in through a while b types
    let mut v = views("");
    v.b().type_text("x");
    v.a().append("R");
    v.b().type_text("y");
    v.b().undo();
    assert_eq!(v.b().text(), "xR");
    // a panel field's new content
    let mut v = views("");
    v.b().type_text("x");
    v.a().reset_text("R");
    v.b().type_text("y");
    v.b().undo();
    assert_eq!(v.b().text(), "R");
    // a reload that finds the text b typed: an undo step comes back to the file's text
    let mut v = views("");
    v.b().type_text("x");
    reload(&mut v.a(), loaded("x"));
    v.b().type_text("y");
    v.b().undo();
    assert_eq!(v.b().text(), "x");
    assert!(!v.b().doc.dirty, "at the file's text");
}

#[test]
fn an_outline_does_not_grow_over_text_typed_at_either_end() {
    let ins = |start, inserted| change(start, 0, inserted);
    let mut r = vec![4..7, 10..13];
    map_ranges(&mut r, &[ins(4, 1)]);
    assert_eq!(r, vec![5..8, 11..14], "typed before the first");
    map_ranges(&mut r, &[ins(14, 2)]);
    assert_eq!(r, vec![5..8, 11..14], "typed after the second");
    map_ranges(&mut r, &[change(3, 3, 0)]);
    assert_eq!(r, vec![3..5, 8..11], "a deletion into the first");
}

#[test]
fn find_results_outlines_are_the_documents_in_every_view() {
    let mut v = views("a hit\nhit");
    v.a().doc.outlines = vec![2..5, 6..9];
    v.b().set_cursor(0);
    v.b().insert("> ");
    assert_eq!(v.a().doc.outlines, vec![4..7, 8..11], "mapped once, for both");
    v.a().undo();
    assert_eq!(v.b().doc.outlines, vec![2..5, 6..9]);
}

#[test]
fn indent_in_one_view_keeps_the_other_caret_on_its_char() {
    let mut v = views("foo bar");
    v.b().set_cursor(4);
    v.a().indent();
    assert_eq!(v.b().cursor(), 8);
    v.b().undo();
    assert_eq!(v.b().cursor(), 4, "undo of a's indent maps b's caret back");
}

#[test]
fn undo_of_multi_caret_typing_keeps_the_other_caret_mid_line() {
    let mut v = views("x\nmid\ny");
    v.a().view.selections = cs(&[0, 6]);
    v.a().insert("Z");
    v.b().set_cursor(4);
    v.a().undo();
    assert_eq!(v.b().text(), "x\nmid\ny");
    assert_eq!(v.b().cursor(), 3);
}

#[test]
fn bookmarks_in_the_other_view_survive_sort_lines() {
    let mut v = views("c\nb\na");
    {
        let mut b = v.b();
        b.view.selections = cs(&[0, 2, 4]);
        b.toggle_bookmark();
    }
    v.a().permute_lines(sort(true));
    assert_eq!(v.b().text(), "a\nb\nc");
    assert_eq!(v.b().view.bookmarks, vec![0, 2, 4]);
}

#[test]
fn a_bookmark_on_a_joined_line_moves_to_its_start_in_both_views() {
    let mut v = views("aaa\nbbb\nccc");
    for a in [true, false] {
        let mut s = if a { v.a() } else { v.b() };
        s.set_cursor(4);
        s.toggle_bookmark();
    }
    v.a().set_cursor(0);
    v.a().join_lines();
    assert_eq!(v.a().text(), "aaa bbb\nccc");
    assert_eq!(marked(&v.a()), ["aaa bbb"]);
    assert_eq!(marked(&v.b()), ["aaa bbb"]);
}

#[test]
fn bookmarks_stay_on_their_lines_through_indent_move_and_delete() {
    let mut v = views("l0\nl1\nl2\nl3");
    for a in [true, false] {
        let mut s = if a { v.a() } else { v.b() };
        s.set_cursor(6);
        s.toggle_bookmark();
    }
    let step = |v: &mut Views, line: &str| {
        assert_eq!(marked(&v.a()), [line], "own view");
        assert_eq!(marked(&v.b()), [line], "other view");
    };
    v.a().set_cursor(3); // l1, above the bookmark
    v.a().indent();
    step(&mut v, "l2");
    v.a().set_cursor(11); // the bookmarked line itself
    v.a().indent();
    step(&mut v, "    l2");
    v.a().move_line_up();
    step(&mut v, "    l2");
    assert_eq!(v.a().text(), "l0\n    l2\n    l1\nl3");
    v.a().move_line_down();
    step(&mut v, "    l2");
    v.a().move_line_down();
    step(&mut v, "    l2");
    assert_eq!(v.a().text(), "l0\n    l1\nl3\n    l2");
    v.a().set_cursor(0);
    v.a().delete_line();
    step(&mut v, "    l2");
    v.a().undo();
    step(&mut v, "    l2");
}

#[test]
fn reload_keeps_carets_and_bookmarks_on_their_lines_in_every_view() {
    let mut v = views("one\ntwo\nthree\nfour");
    v.a().set_cursor(5); // t|wo
    v.a().toggle_bookmark();
    v.b().set_cursor(16); // fo|ur
    v.b().toggle_bookmark();
    // the disk gained a line on top and an edit further down
    reload(&mut v.a(), loaded("zero\none\ntwo\nthree!\nfour"));
    let caret = |s: &ViewEdit| {
        let (l, c) = s.point(s.cursor());
        (s.line(l), c)
    };
    assert_eq!(caret(&v.a()), ("two".into(), 1));
    assert_eq!(marked(&v.a()), ["two"]);
    assert_eq!(caret(&v.b()), ("four".into(), 2));
    assert_eq!(marked(&v.b()), ["four"]);
    assert!(!v.b().doc.dirty, "the reload is the document's save point");
}

#[test]
fn wrap_lines_keeps_the_other_caret_on_its_word() {
    let mut v = views("aa bb cc dd ee");
    v.b().set_cursor(10); // between the d's
    v.a().wrap_lines(5, None);
    assert_eq!(v.b().text(), "aa bb\ncc dd\nee");
    assert_eq!(v.b().cursor(), 10);
    v.b().set_cursor(7); // between the c's
    v.a().undo();
    assert_eq!(v.b().cursor(), 7);
}
