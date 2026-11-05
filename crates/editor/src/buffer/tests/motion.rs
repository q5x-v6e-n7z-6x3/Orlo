use super::*;

#[test]
fn left_on_selection_collapses_to_start() {
    let mut s = selecting("abc", &[(0, 2)]);
    s.move_cursors(Motion::Left, false);
    assert_eq!(s.view.selections, vec![Selection::caret(0)]);
}

#[test]
fn up_down_keep_the_display_column_across_tabs() {
    // the caret after "\t\tfoo" is at display column 11, not char column 5
    let mut s = st("abcdefghijklmnop\n\t\tfoo", 22);
    s.move_cursors(Motion::Up, false);
    assert_eq!(s.cursor(), 11);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), 22);
    let mut s = st("\t\tfoo\n0123456789abcdef", 5);
    s.move_cursors(Motion::PageDown(1), false);
    assert_eq!(s.cursor(), 6 + 11);
}

#[test]
fn down_moves_by_row_keeping_the_column() {
    let line = crate::viewport::tests::probe_line();
    let (mut s, map) = wrapped(&line, 5);
    assert_eq!(walk(&mut s, &map, Motion::Down, 3), [41, 81, 116]);
    let (mut s, map) = wrapped(&line, 35);
    assert_eq!(walk(&mut s, &map, Motion::Down, 2), [71, 111]);
    let (mut s, map) = wrapped(&line, 36);
    assert_eq!(walk(&mut s, &map, Motion::Down, 2), [72, 111], "from a break");
    let (mut s, map) = wrapped(&line, 0);
    assert_eq!(walk(&mut s, &map, Motion::Down, 3), [36, 76, 111], "a caret put at a row start moves on from that row");
}

#[test]
fn up_on_the_first_row_goes_to_zero() {
    let line = crate::viewport::tests::probe_line();
    let (mut s, map) = wrapped(&line, 41);
    assert_eq!(walk(&mut s, &map, Motion::Up, 2), [5, 0]);
    let (mut s, map) = wrapped(&line, 76);
    assert_eq!(walk(&mut s, &map, Motion::Up, 2), [36, 0]);
    let (mut s, map) = wrapped(&line, 77);
    assert_eq!(walk(&mut s, &map, Motion::Up, 2), [37, 1]);
}

#[test]
fn wrapped_rows_keep_the_display_column_across_tabs() {
    let text = format!("\t\tfoo\n{}", "x".repeat(100));
    let (mut s, map) = wrapped(&text, 5);
    assert_eq!(walk(&mut s, &map, Motion::Down, 2), [17, 57]);
    assert_eq!(walk(&mut s, &map, Motion::Up, 2), [17, 5]);
}

#[test]
fn bol_walks_rows_then_smart_home() {
    let line = crate::viewport::tests::probe_line();
    let (mut s, map) = wrapped(&line, 50);
    assert_eq!(walk(&mut s, &map, Motion::LineStart, 3), [36, 0, 0]);
    let line = format!("{}{} {} {}", " ".repeat(8), "x".repeat(20), "y".repeat(31), "z".repeat(10));
    let (mut s, map) = wrapped(&line, 61);
    assert_eq!(map.row_of(&s.doc, 61), DisplayPoint::new(1, 40));
    assert_eq!(walk(&mut s, &map, Motion::LineStart, 4), [29, 8, 0, 8]);
}

#[test]
fn eol_goes_to_the_row_end_then_the_line_end() {
    let line = crate::viewport::tests::probe_line();
    let (mut s, map) = wrapped(&line, 50);
    assert_eq!(walk(&mut s, &map, Motion::LineEnd, 3), [76, 300, 300]);
}

#[test]
fn hard_line_start_and_end_ignore_rows() {
    let line = crate::viewport::tests::probe_line();
    let (mut s, map) = wrapped(&format!("{line}\nnext"), 90);
    assert_eq!(walk(&mut s, &map, Motion::HardLineStart, 1), [0]);
    assert_eq!(walk(&mut s, &map, Motion::HardLineEnd, 1), [300]);
    let mut s = st("    ab\ncd", 5);
    s.move_cursors(Motion::HardLineStart, true);
    assert_eq!(s.view.selections, vec![Selection::new(5, 0)]);
    s.move_cursors(Motion::HardLineEnd, false);
    assert_eq!(s.cursor(), 6);
}

#[test]
fn after_cmd_right_carets_keep_to_line_ends() {
    // probed in Sublime 4 on "ab / abcdef / abc / abcdefghi"
    let text = "ab\nabcdef\nabc\nabcdefghi\n";
    let mut s = EditorState::from_text(text);
    s.move_cursors(Motion::LineEnd, false);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), 9, "end of abcdef");
    s.move_cursors(Motion::Down, false);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), s.line_end(3), "end of abcdefghi");
    let mut s = EditorState::from_text(text);
    s.move_cursors(Motion::LineEnd, false);
    s.select_lines(true, &Lines, &Cells);
    s.select_lines(true, &Lines, &Cells);
    assert_eq!(carets_of(&s), [2, 9, 13], "ends of the first three lines");
    // ctrl-e does not stick
    let mut s = EditorState::from_text(text);
    s.move_cursors(Motion::HardLineEnd, false);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), 5);
}

#[test]
fn up_down_keep_goal_column() {
    let mut s = st("abcdef\nab\nabcdef", 5);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), 9);
    s.move_cursors(Motion::Down, false);
    assert_eq!(s.cursor(), 15);
}

#[test]
fn word_right_and_left() {
    let mut s = st("foo  bar.baz", 0);
    s.move_cursors(Motion::WordRight, false);
    assert_eq!(s.cursor(), 3);
    s.move_cursors(Motion::WordRight, false);
    assert_eq!(s.cursor(), 8);
    s.move_cursors(Motion::WordRight, false);
    assert_eq!(s.cursor(), 9);
    s.move_cursors(Motion::WordLeft, false);
    assert_eq!(s.cursor(), 8);
    s.move_cursors(Motion::WordLeft, false);
    assert_eq!(s.cursor(), 5);
}

#[test]
fn smart_home_toggles() {
    let mut s = st("    x", 5);
    s.move_cursors(Motion::LineStart, false);
    assert_eq!(s.cursor(), 4);
    s.move_cursors(Motion::LineStart, false);
    assert_eq!(s.cursor(), 0);
}

#[test]
fn shift_motion_extends_selection() {
    let mut s = st("abc", 0);
    s.move_cursors(Motion::LineEnd, true);
    assert_eq!(s.view.selections[0].range(), 0..3);
}

#[test]
fn subword_boundaries_are_source_ordered_and_symmetric() {
    let right: Motion = serde_json::from_str("\"subword_right\"").unwrap();
    let left: Motion = serde_json::from_str("\"subword_left\"").unwrap();
    for (text, boundaries) in [
        ("camelCase", vec![0, 5, 9]),
        ("HTTPServer", vec![0, 4, 10]),
        ("snake_case", vec![0, 5, 6, 10]),
        ("item42Value", vec![0, 4, 6, 11]),
        ("a__  ...B", vec![0, 1, 3, 5, 8, 9]),
        ("écoleÉté", vec![0, 5, 8]),
        ("ΑΒΓΔelta", vec![0, 3, 8]),
        ("cafe\u{301}Value", vec![0, 5, 10]),
        ("漢字٤٢Value", vec![0, 2, 4, 9]),
        ("a👩‍💻!!B", vec![0, 1, 6, 7]),
    ] {
        let mut s = st(text, 0);
        for &at in &boundaries[1..] {
            s.move_cursors(right, false);
            assert_eq!(s.cursor(), at, "forward in {text:?}");
        }
        s.move_cursors(right, false);
        assert_eq!(s.cursor(), s.len());
        for &at in boundaries[..boundaries.len() - 1].iter().rev() {
            s.move_cursors(left, false);
            assert_eq!(s.cursor(), at, "backward in {text:?}");
        }
        s.move_cursors(left, false);
        assert_eq!(s.cursor(), 0);
        for offset in 0..=s.len() {
            s.set_cursor(offset);
            let at = s.cursor();
            s.move_cursors(right, false);
            assert_eq!(s.cursor(), boundaries.iter().copied().find(|&b| b > at).unwrap_or(s.len()));
            s.set_cursor(at);
            s.move_cursors(left, false);
            assert_eq!(s.cursor(), boundaries.iter().rev().copied().find(|&b| b < at).unwrap_or(0));
        }
    }
}

#[test]
fn subword_motion_from_inside_segments_selects_without_changing_text() {
    let mut s = st("HTTPServer item42Value", 2);
    s.move_cursors(Motion::SubwordRight, true);
    assert_eq!(s.view.selections, [Selection::new(2, 4)]);
    s.move_cursors(Motion::SubwordRight, true);
    assert_eq!(s.view.selections, [Selection::new(2, 10)]);
    s.move_cursors(Motion::SubwordLeft, true);
    assert_eq!(s.view.selections, [Selection::new(2, 4)]);
    s.move_cursors(Motion::SubwordLeft, false);
    assert_eq!(s.view.selections, [Selection::caret(0)]);
    s.set_selections(vec![Selection::caret(2), Selection::caret(17)]);
    s.move_cursors(Motion::SubwordRight, true);
    assert_eq!(s.view.selections, [Selection::new(2, 4), Selection::new(17, 22)]);
    assert_eq!(s.text(), "HTTPServer item42Value");
    assert!(!s.doc.dirty);
}

#[test]
fn subword_clusters_keep_oversized_accents_zwj_and_flag_runs_whole() {
    for (text, boundary) in [
        (format!("a{}B", "\u{301}".repeat(2048)), 2049),
        (format!("{}B", "🇺🇸".repeat(1000)), 2000),
        ("👩‍💻👍🏽!!B".to_string(), 7),
    ] {
        let mut s = st(&text, 0);
        s.move_cursors(Motion::SubwordRight, false);
        assert_eq!(s.cursor(), boundary);
        assert_eq!(s.snap_to_cluster(s.cursor()), boundary);
        s.move_cursors(Motion::SubwordLeft, false);
        assert_eq!(s.cursor(), 0);
        s.delete_subword(true);
        assert_eq!(s.text(), "B");
        s.undo();
        assert_eq!(s.text(), text);
        s.set_cursor(boundary);
        s.delete_subword(false);
        assert_eq!(s.text(), "B");
        s.undo();
        assert_eq!(s.text(), text);
    }
}

#[test]
fn subword_deletion_merges_overlaps_keeps_selections_and_undoes_atomically() {
    for (forward, carets, expected) in [(true, vec![1, 3, 7], "cCa"), (false, vec![2, 4, 8], "le")] {
        let mut s = st("camelCase", 0);
        s.set_selections(carets.into_iter().map(Selection::caret).collect());
        let original = s.view.selections.clone();
        s.delete_subword(forward);
        assert_eq!(s.text(), expected);
        assert_eq!(s.doc.history.undo.len(), 1);
        s.undo();
        assert_eq!(s.text(), "camelCase");
        assert_eq!(s.view.selections, original);
        s.redo();
        assert_eq!(s.text(), expected);
    }
    let mut s = selecting("camelCase", &[(2, 4), (5, 7)]);
    s.delete_subword(true);
    assert_eq!(s.text(), "calse", "selected text only");
    s.undo();
    assert_eq!(s.view.selections, sels(&[(2, 4), (5, 7)]));
    // One caret's target extends into another selection: delete the union once.
    let mut s = selecting("camelCase", &[(1, 1), (3, 7)]);
    s.delete_subword(true);
    assert_eq!(s.text(), "cse");
    s.undo();
    assert_eq!(s.text(), "camelCase");
}

#[test]
fn subword_deletion_at_ends_and_virtual_positions_never_pads_or_consumes_source() {
    for (text, at, forward) in [("", 0, false), ("", 0, true), ("a", 0, false), ("a", 1, true)] {
        let mut s = st(text, at);
        s.delete_subword(forward);
        assert_eq!(s.text(), text);
        assert!(s.doc.history.undo.is_empty());
        assert!(!s.doc.dirty);
    }
    let mut s = st("camelCase\nend", 0);
    s.set_virtual_space(true);
    let virtual_caret = Selection::from_points((9, 4), (9, 4));
    s.set_selections(vec![virtual_caret]);
    s.delete_subword(true);
    assert_eq!(s.view.selections, [virtual_caret]);
    s.delete_subword(false);
    assert_eq!(s.view.selections, [Selection::caret(9)]);
    s.set_selections(vec![Selection::from_points((9, 5), (9, 2))]);
    s.delete_subword(false);
    assert_eq!(s.view.selections, [Selection::from_points((9, 2), (9, 2))]);
    assert_eq!(s.text(), "camelCase\nend");
    assert!(s.doc.history.undo.is_empty());
    s.set_selections(vec![Selection::caret(1), virtual_caret]);
    s.delete_subword(true);
    assert_eq!(s.text(), "cCase\nend");
    assert_eq!(s.view.selections, [Selection::caret(1), Selection::from_points((5, 4), (5, 4))]);
    s.undo();
    assert_eq!(s.text(), "camelCase\nend");
    assert_eq!(s.view.selections, [Selection::caret(1), virtual_caret]);
    s.set_selections(vec![virtual_caret]);
    s.move_cursors(Motion::SubwordLeft, false);
    assert_eq!(s.view.selections, [Selection::caret(9)]);
    s.set_selections(vec![Selection::from_points((5, 0), (9, 4))]);
    s.delete_subword(false);
    assert_eq!(s.text(), "camel\nend");
}

#[test]
fn subword_shared_views_map_through_delete_undo_and_follow_lock() {
    let mut v = views("camelCase tail");
    v.b().set_cursor(14);
    v.a().set_cursor(5);
    v.a().delete_subword(true);
    assert_eq!(v.b().cursor(), 10);
    assert_eq!(v.b().text(), "camel tail");
    v.a().undo();
    assert_eq!(v.b().cursor(), 14);
    assert_eq!(v.a().cursor(), 5);
    v.a().redo();
    assert_eq!(v.b().text(), "camel tail");
    v.doc.set_following(true);
    let before = (v.doc.text(), v.doc.version, v.doc.history.undo.len());
    v.a().delete_subword(false);
    v.b().delete_subword(true);
    assert_eq!((v.doc.text(), v.doc.version, v.doc.history.undo.len()), before);
    v.doc.set_following(false);
    v.doc.read_only = true;
    v.a().delete_subword(false);
    assert_eq!(v.b().text(), " tail", "read-only working copy remains editable");
}

#[test]
fn subword_carets_inside_clusters_delete_the_whole_cluster_in_either_direction() {
    for (text, offsets, expected) in [
        ("A👩‍💻B".to_string(), vec![2, 3], "AB"),
        ("a\u{301}B".to_string(), vec![1], "B"),
        (format!("a{}B", "\u{301}".repeat(2048)), vec![1, 64, 2048], "B"),
    ] {
        for offset in offsets {
            for forward in [false, true] {
                let mut s = st(&text, offset);
                s.delete_subword(forward);
                assert_eq!(s.text(), expected, "at {offset}, forward {forward}");
                s.undo();
                assert_eq!(s.text(), text);
                assert_eq!(s.cursor(), offset);
            }
        }
    }
}

#[test]
fn prose_navigation_boundaries_are_symmetric_at_every_source_position() {
    let text = "  Dr. Li said (\"cafe\u{301}?\")]  👩‍💻 goes!\n\n \t\n尾 without stop";
    let sentence_starts = [0, 6, 27, 41, text.chars().count()];
    let paragraph_starts = [0, 41, text.chars().count()];
    let missing: Vec<_> = ["paragraph_previous", "paragraph_next", "sentence_previous", "sentence_next"]
        .into_iter()
        .filter(|name| serde_json::from_value::<Motion>(serde_json::json!(name)).is_err())
        .collect();
    assert!(missing.is_empty(), "missing prose motions {missing:?}");
    for (name, boundaries) in [("sentence", sentence_starts.as_slice()), ("paragraph", paragraph_starts.as_slice())] {
        for (direction, forward) in [("previous", false), ("next", true)] {
            let motion: Motion = serde_json::from_value(serde_json::json!(format!("{name}_{direction}"))).unwrap();
            for at in 0..=text.chars().count() {
                let expected = if forward {
                    boundaries.iter().copied().find(|&b| b > at).unwrap_or(text.chars().count())
                } else {
                    boundaries.iter().copied().rev().find(|&b| b < at).unwrap_or(0)
                };
                let mut s = st(text, at);
                let before = (s.doc.version, s.doc.state, s.doc.dirty, s.doc.history.undo.len());
                s.move_cursors(motion, false);
                assert_eq!(s.cursor(), expected, "{name} {direction} at {at}");
                assert_eq!(s.snap_to_cluster(s.cursor()), s.cursor());
                assert_eq!(s.text(), text);
                assert_eq!((s.doc.version, s.doc.state, s.doc.dirty, s.doc.history.undo.len()), before);
            }
        }
    }
}

#[test]
fn prose_sentence_selection_and_delete_cover_all_positions_and_undo_once() {
    let text = "  Dr. Li said (\"cafe\u{301}?\")]  👩‍💻 goes!\n\n \t\n尾 without stop";
    let starts = [0, 6, 27, 41, 55];
    let chars: Vec<_> = text.chars().collect();
    for at in 0..=chars.len() {
        let i = starts.partition_point(|&b| b <= at).saturating_sub(1).min(starts.len() - 2);
        let range = starts[i]..starts[i + 1];
        let mut s = st(text, at);
        s.select_sentence();
        assert_eq!(s.view.selections, [Selection::new(range.start, range.end)], "select at {at}");
        s.select_sentence();
        assert_eq!(s.view.selections, [Selection::new(range.start, range.end)], "repeat at {at}");
        assert!(s.doc.history.undo.is_empty());
        let mut s = st(text, at);
        s.delete_sentence();
        let expected: String = chars[..range.start].iter().chain(&chars[range.end..]).collect();
        assert_eq!(s.text(), expected, "delete at {at}");
        assert_eq!(s.doc.history.undo.len(), 1);
        s.undo();
        assert_eq!(s.text(), text);
        assert_eq!(s.view.selections, [Selection::caret(at)]);
        assert!(s.doc.history.undo.is_empty());
        s.redo();
        assert_eq!(s.text(), expected);
    }
}

#[test]
fn prose_unicode_quotes_suffix_clusters_and_whitespace_clusters_stay_whole() {
    for (text, boundaries) in [
        ("A. \u{301}B!\u{301}”\u{301})\u{301}  👩‍💻 tail", vec![0, 4, 13, 21]),
        ("“雪?!”』】  👩‍💻. \r\nCafe\u{301} no stop", vec![0, 9, 16, 29]),
        ("Dr. Li e.g. goes... next?yes! last", vec![0, 4, 12, 20, 30, 34]),
        ("A.\u{a0}\u{2003}B!\n", vec![0, 4, 7]),
    ] {
        let s = EditorState::from_text(text);
        assert_eq!(s.sentence_boundaries(), boundaries, "{text:?}");
        for &b in &boundaries {
            assert_eq!(s.snap_to_cluster(b), b, "boundary {b} in {text:?}");
        }
        for at in 0..=s.len() {
            let mut selected = st(text, at);
            selected.select_sentence();
            for sel in &selected.view.selections {
                assert_eq!(selected.snap_to_cluster(sel.start()), sel.start());
                assert_eq!(selected.snap_to_cluster(sel.end()), sel.end());
            }
        }
    }
}

#[test]
fn prose_empty_blank_runs_trailing_space_and_final_unpunctuated_endpoints() {
    for (text, paragraphs, sentences) in [
        ("", vec![0], vec![0]),
        (" \t\r\n\n", vec![0, 5], vec![0, 5]),
        ("\n \t\n  First\ncontinued\n \n\nSecond\n\n", vec![0, 4, 25, 33], vec![0, 33]),
        ("One.  \r\n", vec![0, 8], vec![0, 8]),
        ("One. Two no stop", vec![0, 16], vec![0, 5, 16]),
        ("One. Two. ", vec![0, 10], vec![0, 5, 10]),
    ] {
        let s = EditorState::from_text(text);
        assert_eq!(s.paragraph_boundaries(), paragraphs, "paragraphs {text:?}");
        assert_eq!(s.sentence_boundaries(), sentences, "sentences {text:?}");
        for (previous, next, boundaries) in [
            (Motion::ParagraphPrevious, Motion::ParagraphNext, paragraphs),
            (Motion::SentencePrevious, Motion::SentenceNext, sentences),
        ] {
            for at in 0..=s.len() {
                for (motion, forward) in [(previous, false), (next, true)] {
                    let want = if forward {
                        boundaries.iter().copied().find(|&b| b > at).unwrap_or(s.len())
                    } else {
                        boundaries.iter().copied().rev().find(|&b| b < at).unwrap_or(0)
                    };
                    let mut moved = st(text, at);
                    moved.move_cursors(motion, false);
                    assert_eq!(moved.cursor(), want, "{motion:?} at {at} in {text:?}");
                    assert_eq!(moved.snap_to_cluster(moved.cursor()), moved.cursor());
                }
            }
        }
    }
}

#[test]
fn prose_selection_half_open_backward_multicaret_primary_and_rectangular_origins() {
    let mut s = selecting("One. Two! Three tail", &[(9, 6), (11, 20)]);
    s.view.selections[0].rectangular = true;
    s.view.primary = 0;
    s.sel_reset();
    s.sel_checkpoint();
    s.select_sentence();
    assert_eq!(s.view.selections, [Selection::new(10, 5).in_rectangle(), Selection::new(10, 20)]);
    assert_eq!(s.primary(), 0);
    assert!(s.undo_selection());
    assert_eq!(s.view.selections, [Selection::new(9, 6).in_rectangle(), Selection::new(11, 20)]);
    assert_eq!(s.primary(), 0);
    for (anchor, head, want) in [(0, 5, (0, 5)), (5, 0, (5, 0)), (4, 10, (0, 10)), (10, 5, (10, 5))] {
        let mut s = selecting("One. Two! Three tail", &[(anchor, head)]);
        s.select_sentence();
        assert_eq!(s.view.selections, [Selection::new(want.0, want.1)]);
    }
}

#[test]
fn prose_motion_extends_anchor_clears_goals_and_virtual_head_without_padding() {
    let mut s = st("One. Two!\n\nThree tail", 1);
    s.set_virtual_space(true);
    let original = Selection { goal: Some(Goal::X(12.)), ..Selection::from_points((9, 3), (1, 0)).in_rectangle() };
    s.view.selections = vec![original];
    s.move_cursors(Motion::SentenceNext, true);
    assert_eq!(s.view.selections, [Selection::from_points((9, 3), (5, 0)).in_rectangle()]);
    s.move_cursors(Motion::SentencePrevious, true);
    assert_eq!(s.view.selections, [Selection::from_points((9, 3), (0, 0)).in_rectangle()]);
    s.move_cursors(Motion::ParagraphNext, false);
    assert_eq!(s.view.selections, [Selection::caret(11).in_rectangle()]);
    assert_eq!(s.text(), "One. Two!\n\nThree tail");
    assert!(s.doc.history.undo.is_empty());
    let virtual_caret = Selection::from_points((s.len(), 4), (s.len(), 4)).in_rectangle();
    s.set_selections(vec![virtual_caret]);
    assert_eq!(s.view.selections[0].head_virtual, 4);
    s.move_cursors(Motion::SentencePrevious, false);
    assert_eq!(s.view.selections, [Selection::caret(s.len()).in_rectangle()], "previous first drops virtual gap");
    s.move_cursors(Motion::SentencePrevious, false);
    assert_eq!(s.view.selections, [Selection::caret(11).in_rectangle()]);
}

#[test]
fn prose_delete_merges_duplicate_and_overlapping_intervals_and_keeps_undo_origins() {
    let mut overlapping = selecting("One. Two! Three tail", &[(1, 1), (9, 3)]);
    let original = overlapping.view.selections.clone();
    overlapping.delete_sentence();
    assert_eq!(overlapping.text(), "Three tail", "distinct overlapping sentence requests delete their union");
    assert_eq!(overlapping.doc.history.undo.len(), 1);
    overlapping.undo();
    assert_eq!(overlapping.text(), "One. Two! Three tail");
    assert_eq!(overlapping.view.selections, original);
    overlapping.redo();
    assert_eq!(overlapping.text(), "Three tail");
    let mut s = selecting("One. Two! Three tail\nend", &[(1, 1), (3, 3), (9, 6)]);
    s.set_virtual_space(true);
    s.view.selections[2].rectangular = true;
    s.view.selections.push(Selection::from_points((24, 4), (24, 4)).in_rectangle());
    s.view.primary = 2;
    let original = s.view.selections.clone();
    s.delete_sentence();
    assert_eq!(s.text(), "Three tail\nend");
    assert_eq!(s.view.selections[0], Selection::caret(0));
    assert_eq!(s.view.selections.last().unwrap().head_virtual, 4);
    assert_eq!(s.doc.history.undo.len(), 1);
    s.undo();
    assert_eq!(s.text(), "One. Two! Three tail\nend");
    assert_eq!(s.view.selections, original);
    assert_eq!(s.primary(), 2);
    s.redo();
    assert_eq!(s.text(), "Three tail\nend");
}

#[test]
fn prose_pure_virtual_and_empty_deletes_are_exact_noops_and_follow_remains_selectable() {
    for text in ["", "One. tail"] {
        let mut s = st(text, text.chars().count());
        s.set_virtual_space(true);
        if !text.is_empty() {
            s.view.selections = vec![Selection::from_points((s.len(), 4), (s.len(), 7)).in_rectangle()];
        }
        let before = (s.text(), s.view.selections.clone(), s.doc.version, s.doc.state, s.doc.history.undo.len());
        s.delete_sentence();
        assert_eq!((s.text(), s.view.selections.clone(), s.doc.version, s.doc.state, s.doc.history.undo.len()), before);
    }
    let mut s = st("One. Two!", 6);
    s.doc.set_following(true);
    let before = (s.text(), s.doc.version, s.doc.history.undo.len());
    s.delete_sentence();
    assert_eq!((s.text(), s.doc.version, s.doc.history.undo.len()), before);
    s.move_cursors(Motion::SentencePrevious, false);
    assert_eq!(s.cursor(), 5);
    s.select_sentence();
    assert_eq!(s.view.selections, [Selection::new(5, 9)]);
    s.doc.set_following(false);
    s.doc.read_only = true;
    s.delete_sentence();
    assert_eq!(s.text(), "One. ");
}

#[test]
fn prose_shared_delete_undo_redo_maps_sibling_view_and_typing_history() {
    let mut v = views("One. Two! Three tail");
    v.b().set_cursor(20);
    v.a().set_selections(vec![Selection::caret(1), Selection::caret(3), Selection::new(9, 6)]);
    let original = v.a().view.selections.clone();
    v.a().delete_sentence();
    assert_eq!(v.b().text(), "Three tail");
    assert_eq!(v.b().cursor(), 10);
    assert_eq!(v.doc.history.undo.len(), 1);
    v.a().undo();
    assert_eq!(v.b().text(), "One. Two! Three tail");
    assert_eq!(v.b().cursor(), 20);
    assert_eq!(v.a().view.selections, original);
    v.a().redo();
    assert_eq!(v.b().text(), "Three tail");
}

#[test]
fn prose_many_carets_use_one_boundary_table_and_paragraph_expansion_stays_compatible() {
    let text = "One. Two!\n\n".repeat(1000);
    let mut s = EditorState::from_text(&text);
    s.set_selections((0..1000).map(|i| Selection::caret(i * 11)).collect());
    s.move_cursors(Motion::SentenceNext, false);
    assert_eq!(s.view.selections.len(), 1000);
    assert!(s.view.selections.iter().enumerate().all(|(i, sel)| sel.head == i * 11 + 5));
    assert_eq!(s.text(), text);
    assert!(s.doc.history.undo.is_empty());
    let mut s = st("One\nmore\n\nTwo", 1);
    s.expand_to_paragraph();
    assert_eq!(s.view.selections, [Selection::new(0, 9)]);
    s.expand_to_paragraph();
    assert_eq!(s.view.selections, [Selection::new(0, 10)]);
}

#[test]
fn prose_oversized_combining_tails_after_whitespace_quotes_and_punctuation_stay_whole() {
    let marks = "\u{301}".repeat(2048);
    let text = format!("A.{marks}”{marks}){marks} {marks}👩‍💻 tail");
    let mut s = st(&text, 0);
    let start = text.chars().position(|c| c == '👩').unwrap();
    assert_eq!(s.sentence_boundaries(), [0, start, s.len()]);
    s.move_cursors(Motion::SentenceNext, false);
    assert_eq!(s.cursor(), start);
    assert_eq!(s.snap_to_cluster(s.cursor()), s.cursor());
    s.select_sentence();
    assert_eq!(sel_texts(&s), ["👩‍💻 tail"]);
    s.set_cursor(1);
    s.delete_sentence();
    assert_eq!(s.text(), "👩‍💻 tail");
    s.undo();
    assert_eq!(s.text(), text);
}

#[test]
fn word_motion_keeps_combining_marks_with_their_letter() {
    let mut s = st("cafe\u{301} x", 5);
    s.move_cursors(Motion::WordLeft, false);
    assert_eq!(s.cursor(), 0);
    let mut s = st("cafe\u{301} x", 0);
    s.move_cursors(Motion::WordRight, false);
    assert_eq!(s.cursor(), 5);
    // Thai: a consonant and its tone mark
    let mut s = st("\u{0e44}\u{0e21}\u{0e48} x", 3);
    s.move_cursors(Motion::WordLeft, false);
    assert_eq!(s.cursor(), 0);
    let mut s = st("a.\u{301}b", 0);
    s.move_cursors(Motion::WordRight, false);
    assert_eq!(s.cursor(), 1, "punctuation stays a word of its own");
    let mut s = st("cafe\u{301}", 5);
    s.delete_word(false);
    assert_eq!(s.text(), "", "the accent goes with its word");
}
