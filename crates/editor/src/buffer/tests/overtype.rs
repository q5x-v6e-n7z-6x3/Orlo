use super::*;

#[test]
fn overtype_is_opt_in_local_and_does_not_change_history() {
    let mut v = views("abcd");
    v.a().type_text("x");
    assert_eq!(v.a().text(), "xabcd");
    v.a().toggle_overtype();
    assert!(!v.b.overtype);
    assert!(!v.a.duplicate(&v.doc).overtype);
    v.a().type_text("y");
    assert_eq!(v.b().text(), "xybcd");
    v.a().undo();
    assert_eq!(v.b().text(), "xabcd");
    assert!(v.a.overtype);
    v.a().redo();
    assert_eq!(v.b().text(), "xybcd");
    v.a().toggle_overtype();
    v.a().undo();
    assert_eq!(v.a().text(), "xabcd");
}

#[test]
fn overtype_replaces_graphemes_at_multiple_carets_and_keeps_line_endings() {
    for ending in ["\n", "\r\n", "\r"] {
        let first = "e\u{301}👨‍👩‍👧‍👦";
        let text = format!("{first}{ending}👍🏽🇨🇦!");
        let mut s = carets(&text, &[0, first.chars().count() + ending.chars().count()]);
        s.toggle_overtype();
        s.type_text("😀éZ");
        assert_eq!(s.text(), format!("😀éZ{ending}😀éZ"));
        s.undo();
        assert_eq!(s.text(), text);
        s.redo();
        assert_eq!(s.text(), format!("😀éZ{ending}😀éZ"));
    }
}

#[test]
fn overtype_extends_at_line_end_and_keeps_selections_exact() {
    let mut s = selecting("ab\r\ncd", &[(1, 2), (4, 4)]);
    s.toggle_overtype();
    s.type_text("XYZ");
    assert_eq!(s.text(), "aXYZ\r\nXYZ");
    s.set_cursor(4);
    s.type_text("!");
    assert_eq!(s.text(), "aXYZ!\r\nXYZ");
    s.set_cursor(s.len());
    s.type_text("!");
    assert!(s.text().ends_with("XYZ!"));
}

#[test]
fn overtype_disables_pairs_but_insert_and_structural_text_keep_inserting() {
    let mut s = st("abc", 0);
    s.toggle_overtype();
    s.type_text("(");
    assert_eq!(s.text(), "(bc");
    s.set_cursor(0);
    s.type_literal("'");
    assert_eq!(s.text(), "'bc");
    s.set_cursor(0);
    s.insert("PASTE");
    assert_eq!(s.text(), "PASTE'bc");
    s.type_text("\t");
    s.newline();
    assert_eq!(s.text(), "PASTE\t\n'bc");
    s.type_text("x\ny");
    assert_eq!(s.text(), "PASTE\t\nx\ny'bc");
}

#[test]
fn overtype_overlapping_carets_uses_existing_edit_merge() {
    let mut s = carets("abcdef", &[0, 1]);
    s.toggle_overtype();
    s.type_text("XY");
    assert_eq!(s.text(), "XXYdef");
    s.undo();
    assert_eq!(s.text(), "abcdef");
}

#[test]
fn overtype_never_truncates_an_oversized_grapheme() {
    let text = format!("e{}x", "\u{301}".repeat(200));
    let mut s = st(&text, 0);
    s.toggle_overtype();
    s.type_text("a");
    assert_eq!(s.text(), "ax");
    s.undo();
    assert_eq!(s.text(), text);
}

#[test]
fn overtype_does_not_bypass_the_read_only_save_boundary() {
    let mut s = st("abc", 0);
    s.doc.read_only = true;
    s.toggle_overtype();
    s.type_text("x");
    assert_eq!(s.doc.save().unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
    assert!(s.doc.read_only);
}

#[test]
fn overtype_counts_clusters_in_each_carets_left_context() {
    for (source, at, typed, expected) in [
        ("abc", 1, "\u{301}", "a\u{301}bc"),
        ("abc", 1, "\u{301}X", "a\u{301}Xc"),
        ("👨bc", 1, "\u{200d}", "👨‍bc"),
        ("👨‍bc", 2, "👩", "👨‍👩bc"),
        ("🇨bc", 1, "🇦", "🇨🇦bc"),
        ("🇨🇦bc", 2, "🇫", "🇨🇦🇫c"),
    ] {
        let mut s = st(source, at);
        s.toggle_overtype();
        s.type_text(typed);
        assert_eq!(s.text(), expected, "{source:?} + {typed:?}");
        s.undo();
        assert_eq!(s.text(), source);
    }
    let mut s = carets("👨‍bc\nabc", &[2, 5]);
    s.toggle_overtype();
    s.type_text("👩");
    assert_eq!(s.text(), "👨‍👩bc\n👩bc");
}

#[test]
fn review_overtype_long_preceding_emoji_context_preserves_following_source() {
    // Cross several rope chunks; losing the Extended_Pictographic base changes GB11.
    for accents in [600, 2_000] {
        let original = format!("prefix 👩{}z", "\u{301}".repeat(accents));
        let at = 8 + accents;
        let expected = format!("prefix 👩{}\u{200d}💻z", "\u{301}".repeat(accents));
        let mut s = st(&original, at);
        s.toggle_overtype();
        s.type_text("\u{200d}💻");
        assert_eq!(s.text(), expected, "{accents} accents must not hide the preceding emoji base");
        s.undo();
        assert_eq!(s.text(), original);
        s.redo();
        assert_eq!(s.text(), expected);
    }
}

#[test]
fn review_previous_clusters_cross_rope_chunks_without_truncation() {
    let samples = [
        format!("{}尾", "ordinary text ".repeat(1_000)),
        format!("{}尾", "🇨".repeat(1_001)),
        format!("a👩{}\u{200d}💻z", "\u{301}".repeat(2_000)),
        format!("a{}z", "क्".repeat(1_000)),
    ];
    for text in samples {
        let s = EditorState::from_text(&text);
        let mut start = 0;
        for cluster in text.graphemes(true) {
            let end = start + cluster.chars().count();
            assert_eq!(s.prev_cluster(end), start, "previous cluster at {end}");
            for at in [start, start + (end - start) / 2, end - 1] {
                assert_eq!(s.snap_to_cluster(at), start, "cluster start at {at}");
            }
            start = end;
        }
    }
}

#[test]
fn review_overtype_interior_caret_snaps_to_the_complete_grapheme_start() {
    let original = format!("👩{}z", "\u{301}".repeat(600));
    let mut s = st(&original, 600);
    s.toggle_overtype();
    s.type_text("a");
    assert_eq!(s.text(), "az");
    s.undo();
    assert_eq!(s.text(), original);
    s.redo();
    assert_eq!(s.text(), "az");
}
