use super::*;

#[test]
#[ignore]
fn bench_typing() {
    let text = rust_lines(100_000);
    let mut s = EditorState::from_text(&text);
    let mid = s.line_start(50_000);
    s.set_selections(vec![Selection::caret(mid)]);
    time("type a char, 100k lines, 1 caret", 50, 1., || {
        s.type_text("x");
        s.backspace();
    });
    time("undo + redo, 100k lines, 1 caret", 20, 3., || {
        s.undo();
        s.redo();
    });
    let carets = (0..1_000).map(|l| Selection::caret(s.line_start(l * 100)));
    s.set_selections(carets.collect());
    time("type a char, 100k lines, 1,000 carets", 10, 20., || {
        s.type_text("x");
        s.backspace();
    });
    time("undo + redo, 1,000 carets", 10, 30., || {
        s.undo();
        s.redo();
    });
}

#[test]
#[ignore]
fn bench_find() {
    let s = EditorState::from_text(&rust_lines(100_000));
    let o = FindOpts::default();
    time("find all \"self\" in 100k lines", 3, 300., || {
        EditorState::from_text("").find_all("x", &o);
        s.find_all("sel", &o);
        s.find_all("self", &o).len()
    });
    time("find all, case-insensitive, 100k lines", 3, 300., || {
        s.find_all("SEL", &o);
        s.find_all("SELF", &o).len()
    });
    {
        let mut b = EditorState::load(&big_file()).unwrap();
        time("find all \"self\" in 100 MB", 1, 3000., || b.find_all("self", &o).len());
        time("find all \"self\" in 100 MB, as the query is typed again", 3, 1., || b.find_all("self", &o).len());
        // Find Next after an edit: the matches are brought up to it, not searched for again
        let mid = b.len() / 2;
        time("find next \"self\" after an edit, 100 MB", 3, 60., || {
            b.set_cursor(mid);
            b.insert("x");
            let before = orlo_editor::text::SCANNED.get();
            let r = b.find_next("self", mid, &o);
            let scanned = orlo_editor::text::SCANNED.get() - before;
            assert!(scanned < 1000, "searched {scanned} bytes, not around the edit alone");
            r
        });
        let fold = FindOpts { case_sensitive: false, ..o };
        time("find all \"SÉLF\", case-insensitive, 100 MB", 1, 3000., || b.find_all("SÉLF", &fold).len());
        // typing after finding "e" (about 10M matches): the matches wait, nothing a keystroke
        let n = b.find_all("e", &o).len();
        assert!(n > 5_000_000, "{n} matches");
        let at = b.len() / 3;
        time("a keystroke after a find of 10M matches, 100 MB", 3, 1., || {
            b.set_cursor(at);
            b.insert("x");
            let before = orlo_editor::text::MAPPED.get();
            b.take_changes();
            assert_eq!(orlo_editor::text::MAPPED.get(), before, "no match moved on a keystroke");
        });
    }
    let paths: Vec<String> = (0..20_000).map(|i| format!("src/module_{}/file_{i}.rs", i % 97)).collect();
    time("fuzzy filter 20k paths", 5, 60., || orlo_utils::fuzzy::filter("modfile12", &paths).len());
    time("empty fuzzy filter 20k paths", 5, 1., || orlo_utils::fuzzy::filter("", &paths).len());
    let h = Highlighter::new(Some(Path::new("x.rs")));
    time("goto symbol over 100k lines", 1, 3000., || h.symbols(&s.doc.rope).len());
}

/// One state of `text` for each of three runs, each set up by `prepare` before the timing.
fn fresh(text: &str, prepare: impl Fn(&mut EditorState)) -> Vec<EditorState> {
    let state = |_| {
        let mut s = EditorState::from_text(text);
        prepare(&mut s);
        s
    };
    (0..3).map(state).collect()
}

#[test]
#[ignore]
fn bench_large_edits() {
    let text = rust_lines(100_000);
    let mut states = fresh(&text, EditorState::select_all);
    time("select all and delete, 100k lines", 3, 300., || {
        let mut s = states.pop().unwrap();
        s.backspace();
        s.len()
    });
    // a paste over everything with a line more: the words do not line up from the first
    let more = format!("// one more\n{text}");
    let mut states = fresh(&text, EditorState::select_all);
    time("paste over 100k lines, a line more", 3, 300., || {
        let mut s = states.pop().unwrap();
        s.insert(&more);
        s.len()
    });
    // a caret in each of 100 long block comments: the markers are looked for around each
    let text = format!("/* {} */\n", "x".repeat(3000)).repeat(100);
    let mut states = fresh(&text, |s| {
        let at = (0..100).map(|i| Selection::caret(i * 3007 + 1500));
        s.set_selections(at.collect())
    });
    time("block comments off around 100 carets", 3, 60., || {
        let mut s = states.pop().unwrap();
        s.toggle_comment_with_block("/*", "*/");
        assert!(!s.text().contains("/*"));
        s.len()
    });
}

/// Peak memory is read from outside: `/usr/bin/time -l` on the test binary with
/// `bench_delete_all_100mb` (the load alone is `bench_load_100mb`).
#[test]
#[ignore]
fn bench_delete_all_100mb() {
    let mut s = EditorState::load(&big_file()).unwrap();
    s.select_all();
    time("select all and delete, 100 MB", 1, 3000., || {
        s.backspace();
        s.len()
    });
}

/// Undo of a whole-file Indent on 100 MB: the changes are found line by line, without a list
/// of every run of spaces and words on both sides.
#[test]
#[ignore]
fn bench_undo_indent_100mb() {
    let mut s = EditorState::load(&big_file()).unwrap();
    s.select_all();
    s.indent();
    let mut most = 0;
    time("undo a whole-file indent, 100 MB", 1, 1700., || {
        most = allocated_at_most(|| s.undo()).1;
    });
    println!("bench {:<40} {:>10} MB", "  allocated at most by that undo", most >> 20);
    // the two texts it compares and the changes it finds (430 MB); every run of both was 1.3 GB
    assert!(most < 640 << 20, "{} MB", most >> 20);
}

/// Upper Case with 1,000 carets in one 100 KB word: each asks for the same change, which is
/// made once, found by comparing it with the one distinct change of that word.
#[test]
#[ignore]
fn bench_upper_case_1000_carets_in_one_word() {
    let mut s = EditorState::from_text(&"a".repeat(100_000));
    s.set_selections((0..1_000).map(|k| Selection::caret(k * 100)).collect());
    time("upper case, 1,000 carets in a 100 KB word", 3, 800., || {
        s.convert_case(orlo_editor::text::Case::Upper);
        s.undo();
    });
}

/// A block comment toggled over all of 100 MB, with no comment around it: the text before the
/// selection is looked at for an opener, not the whole selection copied first.
#[test]
#[ignore]
fn bench_block_comment_over_100mb() {
    let mut s = EditorState::load(&big_file()).unwrap();
    s.select_all();
    let mut most = 0;
    time("toggle a block comment over 100 MB", 1, 550., || {
        most = allocated_at_most(|| s.toggle_comment_with_block("/*", "*/")).1;
    });
    println!("bench {:<40} {:>10} MB", "  allocated at most by that toggle", most >> 20);
}
