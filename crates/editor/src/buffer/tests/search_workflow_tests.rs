use super::*;
use crate::search::pattern::{Replacer, SearchError};

#[test]
fn scoped_whole_words_retry_after_a_rejected_overlapping_candidate() {
    for (body, start, expected) in
        [("cafe\u{301}foo foo foo", 5, 9..16), ("xfoo foo foo", 1, 5..12), ("foo foo foo", 0, 0..7)]
    {
        let s = EditorState::from_text(body);
        let expected = [expected];
        for case_sensitive in [false, true] {
            for regex in [false, true] {
                let opts = FindOpts { whole_word: true, case_sensitive, regex, ..Default::default() };
                assert_eq!(s.find_all("foo foo", &opts), expected);
                for scope in [0..s.len(), start..s.len()] {
                    assert_eq!(
                        s.find_scoped("foo foo", &opts, &[scope]),
                        expected,
                        "{body:?}, case={case_sensitive}, regex={regex}"
                    );
                }
            }
        }
    }
}

fn rx(case_sensitive: bool) -> FindOpts {
    FindOpts { regex: true, case_sensitive, ..FindOpts::default() }
}

#[test]
fn lookarounds_find_and_replace_in_one_undo_step() {
    let mut s = EditorState::from_text("price $10, AU$20, $30\n");
    let o = rx(true);
    assert_eq!(s.find_all(r"(?<!AU)\$(\d+)", &o), vec![6..9, 18..21]);
    assert_eq!(s.replace_all(r"(?<!AU)\$(\d+)", "${1} USD", &o), 2);
    assert_eq!(s.text(), "price 10 USD, AU$20, 30 USD\n");
    s.undo();
    assert_eq!(s.text(), "price $10, AU$20, $30\n");
}

#[test]
fn backreferences_and_scopes_keep_context_outside_the_selection() {
    let s = EditorState::from_text("the the cat cat");
    let o = rx(true);
    assert_eq!(s.find_all(r"\b(\w+) \1\b", &o), vec![0..7, 8..15]);
    // the selection "e cat" (6..11): a lookbehind sees the "th" before it
    let scope = [6..11];
    assert_eq!(s.find_scoped(r"(?<=the )cat", &o, &scope), vec![8..11]);
    assert!(s.find_scoped(r"cat(?= the)", &o, &scope).is_empty());
    assert_eq!(s.find_scoped(r"cat(?= cat)", &o, &scope), vec![8..11]);
}

#[test]
fn a_runaway_pattern_reports_an_error_instead_of_no_matches() {
    let mut s = EditorState::from_text(&format!("{}c", "a".repeat(40)));
    let o = rx(true);
    let q = r"^(a|a)*\1b";
    assert!(s.find_all(q, &o).is_empty());
    assert_eq!(s.search_error(q, &o, None), Some(SearchError::Limit { line: 1 }));
    assert_eq!(s.replace_all(q, "x", &o), 0);
    assert!(!s.doc.dirty, "nothing replaced");
    // an edit makes the error stale; another query has none
    s.set_cursor(0);
    s.insert("z");
    assert_eq!(s.search_error(q, &o, None), None);
    assert!(s.find_all("z", &o).len() == 1 && s.search_error("z", &o, None).is_none());
}

#[test]
fn matches_cross_lines_with_literal_and_escaped_newlines() {
    let s = EditorState::from_text("ab\ncd\n\nef");
    assert_eq!(s.find_all("b\nc", &FindOpts::default()), vec![1..4]);
    let o = rx(true);
    assert_eq!(s.find_all(r"b\nc", &o), vec![1..4]);
    // `^`/`$` are line anchors; `.` stops at a line break unless (?s)
    assert_eq!(s.find_all(r"^$", &o), vec![6..6]);
    assert_eq!(s.find_all(r"d.e", &o), Vec::<Range<usize>>::new());
    assert_eq!(s.find_all(r"(?s)d.*e", &o), vec![4..8]);
    assert_eq!(s.find_all(r"\A\w", &o), vec![0..1]);
    assert_eq!(s.find_all(r"\w\z", &o), vec![8..9]);
    // zero-length matches at line boundaries
    assert_eq!(s.find_all(r"$", &o), vec![2..2, 5..5, 6..6, 9..9]);
    assert_eq!(s.find_all(r"(?=\n)", &o), vec![2..2, 5..5, 6..6]);
}

#[test]
fn unicode_offsets_and_length_changing_replacements() {
    let mut s = EditorState::from_text("é😀 straße e\u{301}x");
    let o = rx(true);
    assert_eq!(s.find_all(r"stra(?=ße)", &o), vec![3..7]);
    assert_eq!(s.find_all(r"e\p{M}(?=x)", &o), vec![10..12]);
    let r = Replacer::new(r"(\w+)", r"\U$1", &o, false).unwrap();
    let all = s.find_all(r"(\w+)", &o);
    assert_eq!(s.replace_with(&r, &all), 3);
    assert_eq!(s.text(), "É😀 STRASSE E\u{301}X");
    s.undo();
    assert_eq!(s.text(), "é😀 straße e\u{301}x");
}

#[test]
fn replace_counts_only_the_matches_it_replaced() {
    let mut s = EditorState::from_text("a b a");
    let o = rx(true);
    let r = Replacer::new("a", "x", &o, false).unwrap();
    assert_eq!(s.replace_with(&r, &[0..1, 2..3, 4..5]), 2, "b is stale");
    assert_eq!(s.text(), "x b x");
}

#[test]
fn preserve_case_replacement_through_the_buffer() {
    let mut s = EditorState::from_text("foo Foo FOO fOo");
    let o = FindOpts::default();
    let r = Replacer::new("foo", "bar", &o, true).unwrap();
    let all = s.find_all("foo", &o);
    s.replace_with(&r, &all);
    assert_eq!(s.text(), "bar Bar BAR bar");
}

#[test]
fn line_break_normalisation_retains_text_rules_and_disk_boundaries() {
    let dir = fresh_dir("orlo-normalised");
    for (i, text) in
        ["a\nb\n", "a\r\nb\r\n", "a\rb\r", "a\r\nb\nc\r\n", "a\rb\nc\n", "a\rb\r\nc\r", "lone\rcr\nin lf\n", "", "\r\n"]
            .iter()
            .enumerate()
    {
        let p = dir.join(format!("{i}.txt"));
        std::fs::write(&p, text).unwrap();
        let l = Loaded::read(&p).unwrap();
        let (n, ending) = normalized(text);
        // Keep the old arbitrary-text observation, including minority literal CR.
        let legacy = if *text == "a\rb\nc\n" {
            "a\rb\nc\n".to_string()
        } else if *text == "lone\rcr\nin lf\n" {
            "lone\rcr\nin lf\n".to_string()
        } else {
            l.rope.to_string()
        };
        assert_eq!(n.as_ref(), legacy, "{text:?}");
        assert_eq!(ending, l.line_ending);
        let source = crate::search::files::read_file(&p).unwrap();
        assert_eq!(source.text, l.rope.to_string());
        assert_eq!(l.endings.as_ref().unwrap().text(&l.rope, 0..l.rope.len_chars()).unwrap(), *text);
    }
}

#[test]
fn scoped_zero_length_matches_stay_inside_the_selection() {
    let s = EditorState::from_text("a\nb\nc");
    let o = rx(true);
    assert_eq!(s.find_scoped("^", &o, &[2..3]), vec![2..2]);
    assert_eq!(s.find_scoped(r"(?<=\n)", &o, &[2..3]), vec![2..2]);
    assert_eq!(s.find_scoped("$", &o, &[2..3]), vec![3..3]);
}

#[test]
fn n11_masks_include_exclude_and_any_bypasses_classification() {
    use crate::search::model::SyntaxMask::{self, *};
    let found = [0..3, 4..8, 9..13];
    let classes = |start| {
        Some(match start {
            0 => (false, false),
            4 => (true, false),
            _ => (false, true),
        })
    };
    for (mask, include, exclude) in [
        (Comments, vec![4..8], vec![0..3, 9..13]),
        (Strings, vec![9..13], vec![0..3, 4..8]),
        (CommentsOrStrings, vec![4..8, 9..13], vec![0..3]),
    ] {
        assert_eq!(mask.filter(false, &found, classes), Some(include));
        assert_eq!(mask.filter(true, &found, classes), Some(exclude));
        assert_eq!(mask.filter(true, &found, |_| None), None);
    }
    assert_eq!(Any.filter(true, &found, |_| panic!("Any must not classify")), Some(found.to_vec()));
    assert_eq!(SyntaxMask::default(), Any);
}

#[test]
fn n11_start_only_eligibility_composes_with_engine_scope_and_undo_redo() {
    use crate::search::model::SyntaxMask;
    let original = "hit HIT hitx // hit\n\"hit\" hit";
    let mut s = EditorState::from_text(original);
    let opts = FindOpts { regex: true, whole_word: true, case_sensitive: false, ..Default::default() };
    let all = s.find_scoped("hit", &opts, &[4..26]);
    assert_eq!(all, [4..7, 16..19, 21..24]);
    let eligible = SyntaxMask::CommentsOrStrings.filter(false, &all, |start| Some((start == 16, start == 21))).unwrap();
    let replacer = Replacer::new("hit", "done", &opts, false).unwrap();
    assert_eq!(s.replace_with(&replacer, &eligible), 2);
    assert_eq!(s.text(), "hit HIT hitx // done\n\"done\" hit");
    s.undo();
    assert_eq!(s.text(), original);
    s.redo();
    assert_eq!(s.text(), "hit HIT hitx // done\n\"done\" hit");
    let crosses = [16..26, 21..21];
    assert_eq!(
        SyntaxMask::Comments.filter(false, &crosses, |start| Some((start == 16, start == 21))),
        Some(vec![16..26])
    );
}
