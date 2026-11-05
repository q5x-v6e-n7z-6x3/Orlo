use super::*;

fn opts() -> FindOpts {
    FindOpts { regex: true, ..FindOpts::default() }
}

#[test]
fn whole_word_regex_retries_after_a_rejected_overlapping_candidate() {
    let o = FindOpts { whole_word: true, ..opts() };
    let s = EditorState::from_text("xfoo bar");
    assert_eq!(s.find_all("foo.*bar|bar", &o), [5..8]);
}

#[test]
fn whole_word_regex_can_choose_a_shorter_match_at_the_same_start() {
    let o = FindOpts { whole_word: true, ..opts() };
    for (text, query, expected) in [
        ("foo bar brrz", "foo.*r", 0..7),
        ("bar xz", "bar.*x|bar", 0..3),
        ("a++b", r"\+\+", 1..3),
        ("bar xz", "(?x) b ar # trailing comment", 0..3),
    ] {
        let s = EditorState::from_text(text);
        assert_eq!(s.find_all(query, &o), [expected], "{query}");
    }
    let marked = EditorState::from_text("e\u{301}");
    assert!(marked.find_all("e", &o).is_empty());
    assert_eq!(marked.find_all(r"e\p{M}*", &o), [0..2]);
    assert!(marked.find_scoped("e", &o, &[0..1]).is_empty());
    let mut s = EditorState::from_text("bar xz");
    assert_eq!(s.replace_all("(bar.*x|bar)", "[$1]", &o), 1);
    assert_eq!(s.text(), "[bar] xz");
    s.undo();
    assert_eq!(s.text(), "bar xz");
}

#[test]
fn regex_search_returns_character_offsets_and_honors_case() {
    let s = EditorState::from_text("é😀 id12 ID٣\nid4");
    assert_eq!(s.find_all(r"id(\d+)", &opts()), vec![3..7, 8..11, 12..15]);
    let sensitive = FindOpts { case_sensitive: true, ..opts() };
    assert_eq!(s.find_all(r"id\d+", &sensitive), vec![3..7, 12..15]);
}

#[test]
fn regex_replacement_expands_captures_in_one_undo_step() {
    let mut s = EditorState::from_text("é12 abc34");
    assert_eq!(s.replace_all(r"(?<word>\p{L}+)(\d+)", "$2:${word}", &opts()), 2);
    assert_eq!(s.text(), "12:é 34:abc");
    s.undo();
    assert_eq!(s.text(), "é12 abc34");
    s.redo();
    assert_eq!(s.text(), "12:é 34:abc");
}

#[test]
fn regex_zero_width_replacements_keep_the_original_line_context() {
    let mut s = EditorState::from_text("a\nb");
    assert_eq!(s.replace_all("^", "_", &opts()), 2);
    assert_eq!(s.text(), "_a\n_b");
    s.undo();
    assert_eq!(s.text(), "a\nb");
}

#[test]
fn invalid_regex_leaves_the_buffer_untouched() {
    let mut s = EditorState::from_text("draft");
    assert!(opts().pattern("[").is_err());
    assert!(s.find_all("[", &opts()).is_empty());
    assert_eq!(s.replace_all("[", "lost", &opts()), 0);
    assert_eq!(s.text(), "draft");
    assert!(!s.doc.dirty);
}

#[test]
fn regex_matches_are_recomputed_after_an_edit() {
    let mut s = EditorState::from_text("a x z");
    assert_eq!(s.find_all("a.*z", &opts()), vec![0..5]);
    s.set_cursor(2);
    s.insert("z");
    assert_eq!(s.find_all("a.*z", &opts()), vec![0..6]);
}

#[test]
fn plain_replacement_keeps_dollar_signs_literal() {
    let mut s = EditorState::from_text("a a");
    assert_eq!(s.replace_all("a", "$1", &FindOpts::default()), 2);
    assert_eq!(s.text(), "$1 $1");
}

#[test]
fn whitespace_cleanup_trims_only_selected_lines_and_preserves_carets() {
    let mut s = EditorState::from_text("é  \nkeep\t \nlast  ");
    s.set_selection(4, 8);
    s.clean_whitespace(false);
    assert_eq!(s.text(), "é  \nkeep\nlast  ");
    assert_eq!(s.view.selections, vec![Selection::new(4, 8)]);
    s.undo();
    assert_eq!(s.text(), "é  \nkeep\t \nlast  ");
}

#[test]
fn whitespace_cleanup_removes_blank_lines_and_preserves_unicode_spaces_and_bookmarks() {
    let mut s = EditorState::from_text("a\n \t\n\u{a0}\n\nend\n");
    s.set_cursor(8);
    s.toggle_bookmark();
    s.clean_whitespace(true);
    assert_eq!(s.text(), "a\n\u{a0}\nend\n");
    assert_eq!(s.view.bookmarks, vec![4]);
    s.undo();
    assert_eq!(s.text(), "a\n \t\n\u{a0}\n\nend\n");
    s.redo();
    assert_eq!(s.text(), "a\n\u{a0}\nend\n");
}

#[test]
fn whitespace_cleanup_leaves_clean_text_and_unicode_whitespace_untouched() {
    let mut s = EditorState::from_text("a\u{a0}\n\u{3000}\n");
    s.clean_whitespace(false);
    assert!(!s.doc.dirty);
    assert_eq!(s.text(), "a\u{a0}\n\u{3000}\n");
}

fn cleanup(trim: Trim, blank: bool, newline: bool) -> Cleanup {
    Cleanup { trim, remove_trailing_blank_lines: blank, ensure_newline: newline, remove_final_newline: false }
}

#[test]
fn save_cleanup_options_alone_and_together_on_the_edge_texts() {
    // per text: the result for each (trim, blank lines, newline) combination, trim being bit 0
    // and the final newline bit 2; an empty text stays empty, only blank lines become empty
    let table: [(&str, [&str; 8]); 5] = [
        ("", [""; 8]),
        ("\n\n", ["\n\n", "\n\n", "", "", "\n\n", "\n\n", "", ""]),
        ("a  \n\n\n", ["a  \n\n\n", "a\n\n\n", "a  \n", "a\n", "a  \n\n\n", "a\n\n\n", "a  \n", "a\n"]),
        ("a", ["a", "a", "a", "a", "a\n", "a\n", "a\n", "a\n"]),
        ("a \t\nb  ", ["a \t\nb  ", "a\nb", "a \t\nb  ", "a\nb", "a \t\nb  \n", "a\nb\n", "a \t\nb  \n", "a\nb\n"]),
    ];
    for (input, results) in table {
        for (i, want) in results.iter().enumerate() {
            let trim = if i & 1 == 1 { Trim::All } else { Trim::None };
            let c = cleanup(trim, i & 2 != 0, i & 4 != 0);
            let mut s = EditorState::from_text(input);
            let changed = s.cleanup_for_save(&c);
            assert_eq!((s.text().as_str(), changed), (*want, *want != input), "{input:?} {c:?}");
            assert_eq!(s.doc.dirty, changed, "{input:?} {c:?}: dirty only when changed");
            if changed {
                s.undo();
                assert_eq!(s.text(), input, "{input:?} {c:?}: one undo step");
                s.redo();
                assert_eq!(s.text(), *want, "{input:?} {c:?}: redo");
            } else {
                s.undo();
                assert_eq!(s.text(), input, "{input:?} {c:?}: no step added");
            }
        }
    }
    // white-space-only last lines count as blank, other white space does not
    let all = cleanup(Trim::None, true, false);
    let mut s = EditorState::from_text("a\n \t\n\u{a0}\n  ");
    assert!(s.cleanup_for_save(&all));
    assert_eq!(s.text(), "a\n \t\n\u{a0}\n");
    let mut s = EditorState::from_text(" \t");
    assert!(s.cleanup_for_save(&cleanup(Trim::All, false, true)));
    assert_eq!(s.text(), "", "trimmed to nothing, no newline added");
}

#[test]
fn save_cleanup_keeps_carets_bookmarks_and_the_undo_step_whole() {
    let mut s = selecting("a  \nb  \n\n", &[(0, 2), (7, 7)]);
    s.view.bookmarks = vec![4];
    assert!(s.cleanup_for_save(&cleanup(Trim::All, true, true)));
    assert_eq!(s.text(), "a\nb\n");
    assert_eq!(s.view.selections, sels(&[(0, 1), (3, 3)]));
    assert_eq!(s.view.bookmarks, vec![2]);
    s.undo();
    assert_eq!(s.text(), "a  \nb  \n\n");
    assert_eq!(s.view.selections, sels(&[(0, 2), (7, 7)]));
}

#[test]
fn not_on_caret_spares_the_lines_a_caret_is_in_or_at_the_end_of() {
    let not_on_caret = cleanup(Trim::NotOnCaret, false, false);
    let run = |sels: Vec<Selection>| {
        let mut s = EditorState::from_text("a  \nb  \nc  ");
        s.view.selections = sels;
        s.cleanup_for_save(&not_on_caret);
        (s.text(), s.view.selections)
    };
    // at the end of the white space, inside it, and right after the text before it
    for at in [3, 2, 1] {
        assert_eq!(run(vec![Selection::caret(at)]).0, "a  \nb\nc", "caret {at}");
    }
    assert_eq!(run(vec![Selection::caret(0)]).0, "a\nb\nc", "before the text");
    // the end of the last line, with another caret: only the middle line is trimmed
    let (text, sels) = run(cs(&[3, 11]));
    assert_eq!(text, "a  \nb\nc  ");
    assert_eq!(sels, cs(&[3, 9]));
    // a selection counts by its head
    assert_eq!(run(vec![Selection::new(0, 3)]).0, "a  \nb\nc");
    assert_eq!(run(vec![Selection::new(3, 0)]).0, "a\nb\nc");
}

#[test]
fn not_on_caret_with_tens_of_thousands_of_carets_is_not_quadratic() {
    // Split into Lines on a big file: a caret in the white space of every other line
    let lines = 60_000;
    let mut s = EditorState::from_text(&"a \n".repeat(lines));
    s.view.selections = (0..lines).step_by(2).map(|l| Selection::caret(l * 3 + 2)).collect();
    assert!(s.cleanup_for_save(&cleanup(Trim::NotOnCaret, false, false)));
    let want: String = (0..lines).map(|l| if l % 2 == 0 { "a \n" } else { "a\n" }).collect();
    assert_eq!(s.text(), want);
}

#[test]
fn a_cleanup_with_nothing_to_clean_leaves_the_typing_group_open() {
    for c in [Cleanup::default(), cleanup(Trim::All, true, false)] {
        let mut s = EditorState::new();
        s.insert("a");
        s.insert("b");
        assert!(!s.cleanup_for_save(&c));
        s.insert("c");
        s.undo();
        assert_eq!(s.text(), "", "{c:?}: typing before and after is one step");
        s.redo();
        assert_eq!(s.text(), "abc");
    }
    // a cleanup that does something ends the group, then is a step of its own
    let mut s = EditorState::new();
    s.insert("ab ");
    assert!(s.cleanup_for_save(&cleanup(Trim::All, false, false)));
    s.insert("c");
    s.undo();
    assert_eq!(s.text(), "ab");
    s.undo();
    assert_eq!(s.text(), "ab ");
    s.undo();
    assert_eq!(s.text(), "");
}

#[test]
fn save_cleanup_off_leaves_markdown_hard_breaks() {
    let mut s = EditorState::from_text("line  \nnext  \n");
    assert!(!s.cleanup_for_save(&Cleanup::default()));
    // only the final newline asked for, and there is one
    assert!(!s.cleanup_for_save(&cleanup(Trim::None, false, true)));
    assert_eq!((s.text().as_str(), s.doc.dirty), ("line  \nnext  \n", false));
}

#[test]
fn zero_width_results_survive_streamed_appends() {
    let mut s = EditorState::from_text("prefix");
    s.doc.outlines = std::iter::once(3..3).collect();
    s.append(" suffix");
    s.take_changes();
    assert_eq!(s.doc.outlines, [3..3]);
}

#[test]
fn scoped_regex_bounds_greedy_captures_without_inventing_word_or_line_boundaries() {
    let mut s = EditorState::from_text("zaaaa!");
    let all = s.find_scoped("(a+)", &opts(), &[2..4]);
    assert_eq!(all, [2..4]);
    assert!(s.find_scoped(r"^a|a$|\ba", &opts(), &[2..4]).is_empty());
    s.replace_matches("(a+)", "[$1]", &opts(), &all);
    assert_eq!(s.text(), "za[aa]a!");
}

#[test]
fn scoped_literal_cache_distinguishes_different_selections() {
    let s = EditorState::from_text("aaa");
    assert_eq!(s.find_scoped("aa", &FindOpts::default(), &[1..3]), [1..3]);
    assert_eq!(s.find_scoped("aa", &FindOpts::default(), &[0..2]), [0..2]);
    assert_eq!(s.find_all("aa", &FindOpts::default()), [0..2]);
}
