use super::*;

#[test]
fn find_is_case_insensitive_and_wraps() {
    let s = st("Foo foo", 0);
    assert_eq!(s.find_all("FOO", &FindOpts::default()), vec![0..3, 4..7]);
    assert_eq!(s.find_next("foo", 5, &FindOpts::default()), Some(0..3));
    assert_eq!(s.find_prev("foo", 0, &FindOpts::default()), Some(4..7));
    assert!(s.find_all("", &FindOpts::default()).is_empty());
}

#[test]
fn replace_all_counts() {
    let mut s = st("a a", 0);
    assert_eq!(s.replace_all("a", "bb", &FindOpts::default()), 2);
    assert_eq!(s.text(), "bb bb");
}

#[test]
fn ascii_find_queries_match_unicode_lowercase_equivalents() {
    let text = "k K K Kelvin kelvin xKx";
    let chars: Vec<char> = text.chars().collect();
    for query in ["k", "K", "kelvin", "K"] {
        for case_sensitive in [false, true] {
            for whole_word in [false, true] {
                let opts = opts_for(case_sensitive, whole_word);
                let query_chars: Vec<char> = query.chars().collect();
                assert_eq!(find_in_str(text, query, &opts), find_in(&chars, &query_chars, &opts), "{query:?} {opts:?}");
            }
        }
    }
    assert_eq!(find_in_str("K", "k", &FindOpts::default()), vec![0..1]);
}

#[test]
fn find_in_str_finds_what_find_in_does_across_its_windows() {
    // Starts that fail, Unicode-to-ASCII case folding, words, and a match across the
    // 1 MB window edge.
    let unit = "sx Self sélf SÉLF ſelf Kelvin kelvin Kelvin self_self xSELFx ";
    let mut text = unit.repeat((1 << 20) / unit.len());
    text.truncate(text.floor_char_boundary((1 << 20) - 2));
    text.push_str(&unit.repeat(3));
    let chars: Vec<char> = text.chars().collect();
    for q in ["self", "SÉLF", "kelvin", "\u{212A}ELVIN", "sx s", "f "] {
        for (case_sensitive, whole_word) in [(false, false), (true, false), (false, true)] {
            let o = opts_for(case_sensitive, whole_word);
            let qc: Vec<char> = q.chars().collect();
            assert_eq!(find_in_str(&text, q, &o), find_in(&chars, &qc, &o), "{q} {o:?}");
        }
    }
}

#[test]
fn matches_after_an_edit_are_found_around_it_alone_and_equal_a_whole_search() {
    let text = "self.a self_b Self xséLF aaaa\n".repeat(2000);
    let queries = [("self", false, false), ("self", true, true), ("SÉlf", false, false), ("aa", false, false)];
    for (q, case_sensitive, whole_word) in queries {
        let o = opts_for(case_sensitive, whole_word);
        let mut s = EditorState::from_text(&text);
        s.find_all(q, &o);
        let edits = [(5, "self "), (3000, ""), (40_000, "aaa"), (100, "\n")];
        for (at, ins) in edits {
            match ins {
                "" => s.set_selection(at, at + 7),
                _ => s.set_cursor(at),
            }
            s.insert(ins);
            if at == 40_000 {
                s.take_changes();
            }
            let before = SCANNED.get();
            let got = s.find_all(q, &o);
            let scanned = SCANNED.get() - before;
            assert_eq!(got, find_in_str(&s.text(), q, &o), "{q} at {at}");
            let whole = scanned >= s.len();
            assert_eq!(whole, overlaps_itself(q, &o), "{q} at {at}: {scanned} bytes");
        }
    }
}

#[test]
fn keystrokes_after_a_find_leave_the_kept_matches_until_they_are_read() {
    let mut s = st(&"e ".repeat(50_000), 0);
    let o = FindOpts::default();
    assert_eq!(s.find_all("e", &o).len(), 50_000);
    let before = MAPPED.get();
    for _ in 0..10 {
        s.insert("x");
        s.take_changes();
    }
    assert_eq!(MAPPED.get(), before, "no keystroke touched the matches");
    assert_eq!(s.find_all("e", &o).len(), 50_000);
    assert!(MAPPED.get() - before <= 50_000, "brought up to date once, when read");
    // past 64 changes they are dropped: a whole search is cheaper than the catching up
    for _ in 0..70 {
        s.insert("x");
        s.take_changes();
    }
    assert!(s.view.found.borrow().is_none());
}

#[test]
fn find_all_is_kept_for_the_text_and_redone_after_an_edit() {
    let o = FindOpts::default();
    let mut s = st("a b a", 0);
    assert_eq!(s.find_all("a", &o), vec![0..1, 4..5]);
    assert_eq!(s.find_all("a", &o), vec![0..1, 4..5], "from the cache");
    s.insert("a");
    assert_eq!(s.find_all("a", &o), vec![0..1, 1..2, 5..6]);
    let whole = FindOpts { whole_word: true, ..o };
    assert_eq!(s.find_all("a", &whole), vec![5..6], "other options, another search");
}

#[test]
fn only_kelvin_sign_case_folds_onto_an_ascii_char() {
    // the candidate bytes for an ASCII first char rest on this
    let folding: Vec<char> = (0x80..=0x10FFFF)
        .filter_map(char::from_u32)
        // a match needs a lowercase that starts with an ASCII char: a cheap first cut
        .filter(|&c| c.to_lowercase().next().is_some_and(|l| l.is_ascii()))
        .filter(|&c| (0u8..128).any(|q| EditorState::eq_ignore_case(c, q as char)))
        .collect();
    assert_eq!(folding, ['\u{212A}']);
}

#[test]
fn an_ascii_query_skips_non_ascii_text_but_finds_kelvin() {
    let o = FindOpts::default();
    let hay = "中".repeat(1_000_000) + "Key \u{212A}EY";
    let t = std::time::Instant::now();
    assert_eq!(find_in_str(&hay, "key", &o), [1_000_000..1_000_003, 1_000_004..1_000_007]);
    assert!(t.elapsed() < std::time::Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(find_in_str("Ωmega ωMEGA", "ωmega", &o), [0..5, 6..11]);
}
