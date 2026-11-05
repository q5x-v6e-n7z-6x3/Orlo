use super::*;

fn virtual_caret(at: usize, spaces: usize) -> Selection {
    Selection::from_points((at, spaces), (at, spaces))
}

#[test]
fn virtual_space_is_opt_in_and_navigation_never_changes_bytes() {
    let mut s = EditorState::from_text("a\n\t😀e\u{301}\r\n");
    s.set_cursor(1);
    s.move_cursors(Motion::Right, false);
    assert_eq!(s.cursor(), 2);
    s.set_virtual_space(true);
    s.set_cursor(1);
    for _ in 0..3 {
        s.move_cursors(Motion::Right, false);
    }
    assert_eq!(s.view.selections, vec![virtual_caret(1, 3)]);
    s.move_cursors(Motion::Left, true);
    assert_eq!(s.view.selections[0].start_point(), (1, 2));
    assert_eq!(s.copy_text(), (String::new(), false));
    assert_eq!(s.text(), "a\n\t😀e\u{301}\r\n");
    assert!(!s.doc.dirty);
    s.set_virtual_space(false);
    assert_eq!(s.view.selections, vec![Selection::caret(1)]);
}

#[test]
fn virtual_space_typing_shared_line_padding_and_undo_restore_endpoints() {
    let mut s = EditorState::from_text("word\n😀");
    s.set_virtual_space(true);
    let original = vec![virtual_caret(4, 2), virtual_caret(4, 5), virtual_caret(6, 3)];
    s.set_selections(original.clone());
    s.type_text("X");
    assert_eq!(s.text(), "word  X   X\n😀   X");
    s.undo();
    assert_eq!(s.text(), "word\n😀");
    assert_eq!(s.view.selections, original);
    s.redo();
    assert_eq!(s.text(), "word  X   X\n😀   X");
}

#[test]
fn virtual_space_deletions_and_copy_do_not_consume_preceding_source() {
    let mut s = EditorState::from_text("source\nnext");
    s.set_virtual_space(true);
    s.set_selections(vec![virtual_caret(6, 3)]);
    s.backspace();
    assert_eq!(s.view.selections, vec![virtual_caret(6, 2)]);
    s.delete();
    assert_eq!(s.view.selections, vec![virtual_caret(6, 2)]);
    s.delete_word(true);
    assert_eq!(s.view.selections, vec![virtual_caret(6, 2)]);
    assert_eq!(s.cut(), (String::new(), false));
    s.delete_word(false);
    assert_eq!(s.view.selections, vec![Selection::caret(6)]);
    assert_eq!(s.text(), "source\nnext");
    assert!(!s.doc.dirty);
    s.set_selections(vec![Selection::from_points((6, 5), (6, 2))]);
    s.backspace();
    assert_eq!(s.view.selections, vec![virtual_caret(6, 2)]);
    s.set_selections(vec![Selection::from_points((3, 0), (6, 5))]);
    assert_eq!(s.copy_text(), ("rce".into(), false));
    s.insert("!");
    assert_eq!(s.text(), "sou!\nnext");
}

#[test]
fn virtual_space_vertical_tabs_graphemes_and_rectangles_use_existing_rows() {
    let mut s = EditorState::from_text("\tabc\nx\n\t👩\u{301}\u{200d}💻z");
    s.set_virtual_space(true);
    s.set_cursor(4);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.view.selections[0].head_point(), (6, 6));
    s.move_cursors(Motion::Up, false);
    assert_eq!(s.cursor(), 4);
    s.drag_box(&[], false, DisplayPoint::new(0, 8), DisplayPoint::new(2, 10), &Lines);
    assert_eq!(s.view.selections.len(), 3);
    assert!(s.view.selections.iter().all(Selection::has_virtual));
    let text = s.text();
    let original = s.view.selections.clone();
    s.insert_per_selection(&["A".into(), "B".into(), "C".into()], false, 4);
    s.undo();
    assert_eq!(s.text(), text);
    assert_eq!(s.view.selections, original);
}

#[test]
fn virtual_space_paste_tab_newline_and_overtype_have_one_undo_step() {
    for kind in 0..5 {
        let mut s = EditorState::from_text("e\u{301}\r\nnext");
        s.set_virtual_space(true);
        s.set_selections(vec![virtual_caret(2, 2)]);
        match kind {
            0 => s.paste("界", false),
            1 => s.tab(),
            2 => s.newline(),
            3 => {
                s.toggle_overtype();
                s.type_literal("\u{301}");
            }
            _ => s.type_text("'"),
        }
        assert!(s.text().starts_with("e\u{301}  "));
        assert!(s.text().ends_with("\r\nnext"));
        s.undo();
        assert_eq!(s.text(), "e\u{301}\r\nnext");
        assert_eq!(s.view.selections, vec![virtual_caret(2, 2)]);
    }
}

#[test]
fn virtual_space_shared_views_keep_unaffected_gaps_and_drop_stale_endpoints() {
    let mut v = views("first\nend");
    v.a().set_virtual_space(true);
    v.b().set_virtual_space(true);
    v.b().set_selections(vec![virtual_caret(9, 3)]);
    v.a().set_cursor(0);
    v.a().insert("X");
    assert_eq!(v.b().view.selections, vec![virtual_caret(10, 3)]);
    v.a().set_cursor(10);
    v.a().insert("!");
    assert_eq!(v.b().view.selections, vec![Selection::caret(10)]);
    v.b().type_text("B");
    assert_eq!(v.a().text(), "Xfirst\nendB!");
}

#[test]
fn virtual_space_completion_and_snippet_triggers_do_not_reach_behind_a_gap() {
    let mut s = EditorState::from_text("word");
    s.set_virtual_space(true);
    s.set_selections(vec![virtual_caret(4, 3)]);
    assert!(!s.trigger_before(4, "word"));
    s.complete("word", "replacement");
    assert_eq!(s.text(), "word");
    assert_eq!(s.view.selections, vec![virtual_caret(4, 3)]);
}

#[test]
fn virtual_space_preview_shared_pads_cancel_without_source_or_selection_loss() {
    let mut s = EditorState::from_text("word\n😀");
    s.set_virtual_space(true);
    let original = vec![virtual_caret(4, 2), virtual_caret(4, 5), virtual_caret(6, 3)];
    s.set_selections(original.clone());
    let (ranges, mut pads) = s.compose_virtual("é");
    let ranges = s.compose_tracking_padding(&ranges, "🙂🙂", false, &mut pads);
    s.remove_composition(&ranges, &pads);
    s.reselect(original.clone());
    assert_eq!(s.text(), "word\n😀");
    assert_eq!(s.view.selections, original);
    s.undo();
    assert_eq!(s.text(), "word\n😀");
    assert_eq!(s.view.selections, original);
}

#[test]
fn virtual_space_explicit_snippet_and_selection_history_keep_original_positions() {
    let mut s = EditorState::from_text("word");
    s.set_virtual_space(true);
    s.sel_checkpoint();
    s.set_selections(vec![virtual_caret(4, 3)]);
    s.sel_checkpoint();
    s.move_cursors(Motion::Right, true);
    s.sel_checkpoint();
    assert!(s.undo_selection());
    assert_eq!(s.view.selections, vec![virtual_caret(4, 3)]);
    let snippet = Snippet {
        trigger: String::new(),
        scope: String::new(),
        description: String::new(),
        body: snippet::parse("${1:hello} $0").unwrap(),
        path: PathBuf::from("/virtual.sublime-snippet"),
    };
    assert!(s.expand_snippet(&snippet, "", &|_, _| None));
    assert_eq!(s.text(), "word   hello ");
    s.undo();
    assert_eq!(s.text(), "word");
    assert_eq!(s.view.selections, vec![virtual_caret(4, 3)]);
}

#[test]
fn virtual_space_save_capture_never_writes_unmaterialized_padding() {
    let path = fresh_dir("orlo-virtual-space-save").join("virtual.txt");
    std::fs::write(&path, b"word\r\n").unwrap();
    let mut s = EditorState::load(&path).unwrap();
    s.set_virtual_space(true);
    s.set_selections(vec![virtual_caret(4, 3)]);
    assert!(!s.doc.unsaved());
    let req = s.doc.capture_save(path.clone(), None).unwrap();
    s.type_text("X");
    let written = req.write().unwrap();
    s.doc.finish_save(&req, &written);
    assert_eq!(std::fs::read(&path).unwrap(), b"word\r\n");
    assert!(s.doc.unsaved());
    s.undo();
    assert!(!s.doc.unsaved());
    assert_eq!(s.view.selections, vec![virtual_caret(4, 3)]);
    s.doc.read_only = true;
    assert!(s.doc.capture_save(path, None).is_err());
}

#[test]
fn virtual_space_empty_column_insertions_and_snippets_never_materialize_padding() {
    let mut s = EditorState::from_text("word\nx");
    s.set_virtual_space(true);
    s.set_selections(vec![virtual_caret(4, 3), virtual_caret(6, 10)]);
    s.insert_per_selection(&["A".into(), String::new()], false, 4);
    assert_eq!(s.text(), "word   A\nx");
    assert_eq!(s.view.selections[1], virtual_caret(10, 10));
    s.undo();
    let original = s.view.selections.clone();
    let empty = Snippet {
        trigger: String::new(),
        scope: String::new(),
        description: String::new(),
        body: snippet::parse("$TM_CURRENT_WORD").unwrap(),
        path: PathBuf::from("/virtual.sublime-snippet"),
    };
    assert!(!s.expand_snippet(&empty, "", &|_, _| None));
    assert_eq!(s.text(), "word\nx");
    assert_eq!(s.view.selections, original);
}

#[test]
fn virtual_space_pad_to_column_uses_virtual_targets_without_editing_carets_already_there() {
    let mut s = EditorState::from_text("x\nword");
    s.set_virtual_space(true);
    let original = vec![virtual_caret(1, 1), virtual_caret(6, 4)];
    s.set_selections(original.clone());
    s.pad_to_column(None, 4);
    assert_eq!(s.text(), "x       \nword");
    assert_eq!(s.view.selections[1], virtual_caret(13, 4));
    s.undo();
    assert_eq!(s.text(), "x\nword");
    assert_eq!(s.view.selections, original);
}

#[test]
fn virtual_space_whole_line_paste_pads_selections_and_keeps_caret_line_semantics() {
    let mut s = EditorState::from_text("word\nx");
    s.set_virtual_space(true);
    let original = vec![Selection::from_points((4, 5), (4, 2)), virtual_caret(6, 3)];
    s.set_selections(original.clone());
    s.paste("A\nB\n", true);
    assert_eq!(s.text(), "word  A\n\nB\nx");
    assert_eq!(s.view.selections[1], virtual_caret(12, 3));
    s.undo();
    assert_eq!(s.text(), "word\nx");
    assert_eq!(s.view.selections, original);
}

#[test]
fn virtual_space_preserves_default_vertical_goal_reset_after_shared_edits() {
    let mut v = views("abcde\nx\nabcde");
    assert!(!v.b.virtual_space.enabled());
    v.b().set_cursor(5);
    v.b().move_cursors(Motion::Down, false);
    assert_eq!(v.b.cursor(), 7);
    v.a().set_cursor(6);
    v.a().insert("zz");
    v.b().move_cursors(Motion::Down, false);
    assert_eq!(v.b.cursor(), 13, "after a shared edit, motion starts from the newly mapped caret x");
}

#[test]
fn virtual_space_paste_empty_text_keeps_the_virtual_caret() {
    let mut s = EditorState::from_text("word");
    s.set_virtual_space(true);
    let original = vec![virtual_caret(4, 3)];
    s.set_selections(original.clone());
    for whole_line in [false, true] {
        s.paste("", whole_line);
        assert_eq!(s.text(), "word");
        assert_eq!(s.view.selections, original);
        assert!(!s.doc.unsaved());
    }
}

#[test]
fn virtual_space_paste_whole_lines_preserves_default_goal_reset() {
    let mut s = EditorState::from_text("abcde\nx\nabcde");
    s.set_cursor(5);
    s.move_cursors(Motion::Down, false);
    assert!(s.view.selections[0].goal.is_some());
    s.paste("z\n", true);
    assert_eq!(s.view.selections[0].goal, None);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), 11);
}

#[test]
fn virtual_space_distributed_paste_leaves_empty_targets_unmaterialized() {
    let mut s = EditorState::from_text("a\nb");
    s.set_virtual_space(true);
    let original = vec![virtual_caret(1, 2), virtual_caret(3, 2)];
    s.set_selections(original.clone());
    s.paste("X\n", false);
    assert_eq!(s.text(), "a  X\nb");
    assert_eq!(s.view.selections, vec![Selection::caret(4), virtual_caret(6, 2)]);
    s.undo();
    assert_eq!(s.text(), "a\nb");
    assert_eq!(s.view.selections, original);
}

#[test]
fn virtual_space_distributed_paste_maps_empty_targets_through_shared_padding() {
    for (gaps, clipboard, expected, selections) in [
        ([1, 3, 6], "\nX\n", "a   X", vec![Selection::caret(2), Selection::caret(5), virtual_caret(5, 3)]),
        ([2, 5, 9], "X\n\nY", "a  X       Y", vec![Selection::caret(4), Selection::caret(7), Selection::caret(12)]),
    ] {
        let mut s = EditorState::from_text("a");
        s.set_virtual_space(true);
        let original: Vec<_> = gaps.into_iter().map(|gap| virtual_caret(1, gap)).collect();
        s.set_selections(original.clone());
        s.paste(clipboard, false);
        assert_eq!(s.text(), expected);
        assert_eq!(s.view.selections, selections);
        s.undo();
        assert_eq!(s.text(), "a");
        assert_eq!(s.view.selections, original);
        s.redo();
        assert_eq!(s.text(), expected);
        assert_eq!(s.view.selections, selections);
    }
}

#[test]
fn virtual_space_materialization_uses_indent_policy_without_rewriting_explicit_padding() {
    for spaces in [true, false] {
        let mut s = EditorState::from_text("a \t");
        s.doc.tab_size = 4;
        s.doc.translate_tabs = spaces;
        s.set_virtual_space(true);
        let original = vec![virtual_caret(3, 7)];
        s.set_selections(original.clone());
        s.type_literal("X");
        assert_eq!(s.text(), if spaces { "a \t       X" } else { "a \t\t   X" });
        s.undo();
        assert_eq!(s.text(), "a \t");
        assert_eq!(s.view.selections, original);
    }
}

#[test]
fn virtual_space_rectangular_policy_limits_entry_and_survives_motion_and_undo() {
    let (settings, _) = orlo_config::settings::parse(r#"{"virtual_space":"rectangular"}"#).unwrap();
    let mut s = EditorState::from_text("long\nx");
    s.set_virtual_space(settings.virtual_space);
    s.drag_box(&[], false, DisplayPoint::new(0, 6), DisplayPoint::new(1, 6), &Lines);
    assert_eq!(s.view.selections.iter().map(Selection::head_point).collect::<Vec<_>>(), [(4, 2), (6, 5)]);
    let original = s.view.selections.clone();
    s.type_literal("X");
    assert_eq!(s.text(), "long  X\nx     X");
    s.undo();
    assert_eq!(s.view.selections, original);
    s.move_cursors(Motion::Right, false);
    assert_eq!(s.view.selections.iter().map(Selection::head_point).collect::<Vec<_>>(), [(4, 3), (6, 6)]);
    s.set_cursor(4);
    s.move_cursors(Motion::Right, false);
    assert_eq!(s.cursor(), 5, "ordinary Right crosses EOL");
    assert!(s.view.selections.iter().all(|sel| !sel.has_virtual()));
    s.set_cursor(4);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.view.selections[0].head_point(), (6, 0));
}

#[test]
fn virtual_space_policy_cleanup_tracks_mixed_origins_and_history_without_editing_source() {
    let mut s = EditorState::from_text("a\nb");
    s.set_virtual_space(VirtualSpace::All);
    let boxed = virtual_caret(1, 3).in_rectangle();
    s.set_selections(vec![boxed, virtual_caret(3, 4)]);
    let generation = s.selection_generation();
    s.set_virtual_space(VirtualSpace::Rectangular);
    assert!(s.selection_generation() > generation);
    assert_eq!(s.view.selections, [boxed, Selection::caret(3)]);
    assert!(!s.doc.unsaved());
    s.type_literal("X");
    s.set_virtual_space(VirtualSpace::Off);
    s.undo();
    assert_eq!(s.text(), "a\nb");
    assert!(s.view.selections.iter().all(|sel| !sel.has_virtual()));
    s.set_virtual_space(VirtualSpace::Rectangular);
    s.set_cursor(1);
    s.select_lines(true, &Lines, &Cells);
    assert!(s.view.selections.iter().all(|sel| sel.rectangular));
    s.move_cursors(Motion::Right, false);
    assert!(s.view.selections.iter().all(|sel| sel.head_virtual == 1));
    s.sel_checkpoint();
    let before = s.selection_generation();
    let mut ordinary = s.view.selections.clone();
    for sel in &mut ordinary {
        sel.rectangular = false;
    }
    s.reselect(ordinary);
    assert!(s.selection_generation() > before, "same physical offsets with a new origin invalidate pending work");
    assert!(s.view.selections.iter().all(|sel| !sel.has_virtual()));
}

#[test]
fn virtual_space_hard_tabs_share_padding_without_skipping_empty_or_overlapping_targets() {
    let mut s = EditorState::from_text("a");
    s.doc.translate_tabs = false;
    s.doc.tab_size = 4;
    s.set_virtual_space(true);
    let original = vec![virtual_caret(1, 1), virtual_caret(1, 5), virtual_caret(1, 7)];
    s.set_selections(original.clone());
    s.paste("X\n\nY", false);
    assert_eq!(s.text(), "a X\t  \tY");
    assert_eq!(s.view.selections, [Selection::caret(3), Selection::caret(6), Selection::caret(8)]);
    s.undo();
    assert_eq!(s.text(), "a");
    assert_eq!(s.view.selections, original);
    s.set_selections(vec![Selection::from_points((1, 1), (1, 6)), Selection::from_points((1, 3), (1, 8))]);
    let merged = s.view.selections.clone();
    assert_eq!(merged, [Selection::from_points((1, 1), (1, 8))]);
    s.type_literal("Z");
    assert_eq!(s.text(), "a Z");
    assert_eq!(s.view.selections, [Selection::caret(3)]);
    s.undo();
    assert_eq!(s.text(), "a");
    assert_eq!(s.view.selections, merged);
    s.redo();
    assert_eq!(s.text(), "a Z");
    assert_eq!(s.view.selections, [Selection::caret(3)]);
}

#[test]
fn virtual_space_rectangular_shared_views_preserve_origin_and_clear_stale_gaps() {
    let mut v = views("first\nend");
    v.a().set_virtual_space(VirtualSpace::Rectangular);
    v.b().set_virtual_space(VirtualSpace::Rectangular);
    v.b().drag_box(&[], false, DisplayPoint::new(1, 6), DisplayPoint::new(1, 6), &Lines);
    let original = v.b.selections.clone();
    let duplicate = v.b.duplicate(&v.doc);
    assert_eq!(duplicate.selections, original);
    v.a().set_cursor(0);
    v.a().insert("X");
    assert_eq!(v.b().view.selections, [virtual_caret(10, 3).in_rectangle()]);
    v.a().set_cursor(10);
    v.a().insert("!");
    assert_eq!(v.b().view.selections[0].head_virtual, 0);
    assert!(v.b().view.selections[0].rectangular);
    v.b().set_virtual_space(VirtualSpace::Off);
    assert_eq!(v.a().view.virtual_space, VirtualSpace::Rectangular);
}

#[test]
fn virtual_space_mixed_origin_merges_do_not_promote_ordinary_virtual_endpoints() {
    let mut s = EditorState::from_text("a");
    s.set_virtual_space(VirtualSpace::All);
    s.set_selections(vec![Selection::from_points((0, 0), (1, 5)), virtual_caret(1, 3).in_rectangle()]);
    assert_eq!(s.view.selections.len(), 1);
    s.set_virtual_space(VirtualSpace::Rectangular);
    assert_eq!(s.view.selections, [Selection::new(0, 1)]);
    assert_eq!(s.text(), "a");
    assert!(!s.doc.unsaved());
    let mut s = EditorState::from_text("a\nb");
    s.set_virtual_space(VirtualSpace::Rectangular);
    s.set_selections(vec![
        Selection::caret(0),
        Selection::caret(2),
        Selection::from_points((2, 0), (3, 5)).in_rectangle(),
    ]);
    assert_eq!(s.view.selections, [Selection::caret(0), Selection::new(2, 3)]);
    assert_eq!(s.primary(), 1, "clamping a mixed merge keeps its primary group");
}
