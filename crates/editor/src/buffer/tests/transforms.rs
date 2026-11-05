use super::*;

// ---- Sublime's Edit / Selection / Goto commands ----

#[test]
fn convert_case_variants() {
    assert_eq!(convert_case("hello world foo", Case::Title), "Hello World Foo");
    assert_eq!(convert_case("qux", Case::Swap), "QUX");
    assert_eq!(convert_case("helloWorld", Case::Snake), "hello_world");
    assert_eq!(convert_case("foo_bar", Case::UpperCamel), "FooBar");
    assert_eq!(convert_case("HTTPServer-id", Case::Kebab), "http-server-id");
    assert_eq!(convert_case("Foo bar", Case::LowerCamel), "fooBar");
    let mut s = st("hello world", 2);
    s.convert_case(Case::Upper);
    assert_eq!(s.text(), "HELLO world");
    assert_eq!(s.cursor(), 2);
}

#[test]
fn transform_selections_converts_each_selection_in_one_undo_step() {
    use crate::editing::convert::{encode, Codec};
    let b64 = |t: &str| Ok(encode(Codec::Base64, t));
    let mut s = selecting("foo bar baz", &[(0, 3), (8, 11)]);
    s.transform_selections(false, b64).unwrap();
    assert_eq!(s.text(), "Zm9v bar YmF6");
    assert_eq!(s.view.selections, sels(&[(0, 4), (9, 13)]));
    s.undo();
    assert_eq!(s.text(), "foo bar baz");
    assert_eq!(s.view.selections, sels(&[(0, 3), (8, 11)]));
    s.redo();
    assert_eq!(s.text(), "Zm9v bar YmF6");
    // a caret among the selections converts nothing of its own
    let mut s = selecting("foo bar baz", &[(5, 5), (8, 11)]);
    s.transform_selections(false, b64).unwrap();
    assert_eq!(s.text(), "foo bar YmF6");
    assert_eq!(s.view.selections, sels(&[(5, 5), (8, 12)]));
}

#[test]
fn transform_document_replaces_everything_in_one_undo_step_keeping_the_caret() {
    let mut s = st("one\ntwo", 5);
    s.doc.dirty = false;
    assert_eq!(s.transform_document(|t| Ok(t.to_uppercase())), Ok(true));
    assert_eq!(s.text(), "ONE\nTWO");
    assert_eq!(s.view.selections, [Selection::caret(5)]);
    assert!(s.doc.dirty);
    s.undo();
    assert_eq!(s.text(), "one\ntwo");
    assert_eq!(s.view.selections, [Selection::caret(5)]);
    s.redo();
    assert_eq!(s.text(), "ONE\nTWO");
    // a selection is not what it works on, and the caret is kept inside a shorter text
    let mut s = selecting("abcdef", &[(1, 2)]);
    s.transform_document(|_| Ok("xy".into())).unwrap();
    assert_eq!(s.text(), "xy");
    assert_eq!(s.view.selections, [Selection::caret(2)]);
}

#[test]
fn transform_document_changing_nothing_or_failing_leaves_no_trace() {
    let mut s = st("same", 1);
    s.doc.dirty = false;
    assert_eq!(s.transform_document(|t| Ok(t.to_string())), Ok(false));
    assert!(!s.doc.dirty);
    assert_eq!(s.transform_document(|_| Err("boom".into())), Err("boom".into()));
    assert_eq!((s.text().as_str(), s.doc.dirty), ("same", false));
    s.undo();
    assert_eq!(s.text(), "same");
    // line breaks a step wrote as CRLF are the buffer's own LF
    assert_eq!(s.transform_document(|_| Ok("a\r\nb".into())), Ok(true));
    assert_eq!(s.text(), "a\nb");
}

#[test]
fn file_text_applies_the_line_ending_and_the_byte_order_mark() {
    assert_eq!(file_text("a\nb\n".into(), "\n", false), "a\nb\n");
    assert_eq!(file_text("a\nb\n".into(), "\r\n", false), "a\r\nb\r\n");
    assert_eq!(file_text("a\nb".into(), "\r", true), "\u{feff}a\rb");
    // a save writes exactly that
    let mut s = EditorState::from_text("x\ny");
    (s.doc.line_ending, s.doc.bom) = ("\r\n", true);
    assert_eq!(s.bytes_as_saved(None).unwrap(), file_text("x\ny".into(), "\r\n", true).into_bytes());
}

#[test]
fn transform_selections_without_a_selection_converts_the_whole_document() {
    use crate::editing::convert::{encode, Codec};
    let mut s = st("foo\nbar", 2);
    s.transform_selections(false, |t| Ok(encode(Codec::Base64, t))).unwrap();
    assert_eq!(s.text(), "Zm9vCmJhcg==");
    assert_eq!(s.view.selections, [Selection::new(0, 12)]);
    s.undo();
    assert_eq!(s.text(), "foo\nbar");
    assert_eq!(s.view.selections, [Selection::caret(2)]);
    s.redo();
    assert_eq!(s.text(), "Zm9vCmJhcg==");
    // decoded line breaks are the buffer's own
    let mut s = st("DQo=", 0);
    s.transform_selections(false, |t| crate::editing::convert::decode(Codec::Base64, t)).unwrap();
    assert_eq!(s.text(), "\n");
}

#[test]
fn a_transform_that_changes_nothing_keeps_the_selections() {
    // Straighten Quotes on text without curly quotes: the caret stays, not the document selected
    let straighten = |t: &str| Ok(crate::editing::transform::straighten_quotes(t));
    let mut s = st("plain \"text\"", 3);
    let version = s.doc.version;
    s.transform_selections(false, straighten).unwrap();
    assert_eq!(s.view.selections, [Selection::caret(3)]);
    assert_eq!((s.doc.version, s.doc.dirty), (version, false));
    // a reversed selection keeps its direction
    s.view.selections = vec![Selection::new(5, 1)];
    s.transform_selections(false, |t| Ok(t.to_string())).unwrap();
    assert_eq!(s.view.selections, [Selection::new(5, 1)]);
    // one that does change something still selects its result
    let mut s = st("“a”", 1);
    s.transform_selections(false, straighten).unwrap();
    assert_eq!((s.text().as_str(), s.view.selections.clone()), ("\"a\"", vec![Selection::new(0, 3)]));
}

#[test]
fn a_failed_conversion_changes_nothing_and_names_the_selection() {
    use crate::editing::convert::{decode, Codec};
    let mut s = selecting("Zm9v\nZm9v\n!!!!", &[(0, 4), (10, 14)]);
    let (version, dirty) = (s.doc.version, s.doc.dirty);
    let e = s.transform_selections(false, |t| decode(Codec::Base64, t)).unwrap_err();
    assert!(e.starts_with("Line 3: Invalid Base64"), "{e}");
    assert_eq!(s.text(), "Zm9v\nZm9v\n!!!!");
    assert_eq!(s.view.selections, sels(&[(0, 4), (10, 14)]));
    assert_eq!((s.doc.version, s.doc.dirty), (version, dirty));
    s.undo();
    assert_eq!(s.text(), "Zm9v\nZm9v\n!!!!", "no undo step was made");
    // the whole document, when nothing is selected
    let mut s = st("Zm9v\n/w==", 0);
    let e = s.transform_selections(false, |t| decode(Codec::Base64, t)).unwrap_err();
    assert_eq!(e, "Line 1: The decoded data is not UTF-8 text");
    assert_eq!(s.text(), "Zm9v\n/w==");
}

#[test]
fn transform_selections_by_lines_widens_each_selection_to_whole_lines() {
    let up = |t: &str| Ok(t.to_uppercase());
    let mut s = EditorState::from_text("ab\ncd\nef\ngh\nij");
    // lines 0-1, a caret on line 2 (left alone), lines 3-4
    s.view.selections = sels(&[(1, 4), (7, 7), (10, 13)]);
    let before = s.view.selections.clone();
    s.transform_selections(true, up).unwrap();
    assert_eq!(s.text(), "AB\nCD\nef\nGH\nIJ");
    assert_eq!(s.view.selections, sels(&[(0, 6), (9, 14)]));
    s.undo();
    assert_eq!(s.text(), "ab\ncd\nef\ngh\nij");
    assert_eq!(s.view.selections, before);
    s.redo();
    assert_eq!(s.text(), "AB\nCD\nef\nGH\nIJ");
}

#[test]
fn line_selections_that_share_or_touch_lines_are_one_block() {
    let wrap = |t: &str| Ok(format!("[{t}]"));
    let mut s = selecting("ab\ncd\nef", &[(0, 1), (3, 4)]);
    s.transform_selections(true, wrap).unwrap();
    assert_eq!(s.text(), "[ab\ncd\n]ef", "the block sees its own line break");
    // a selection ending at the start of a line stops short of it
    let mut s = selecting("ab\ncd", &[(0, 3)]);
    s.transform_selections(true, wrap).unwrap();
    assert_eq!(s.text(), "[ab\n]cd");
}

#[test]
fn line_transforms_of_the_whole_document_see_its_final_line_break() {
    let wrap = |t: &str| Ok(format!("<{t}>"));
    let mut s = st("a\nb\n", 0);
    s.transform_selections(true, wrap).unwrap();
    assert_eq!(s.text(), "<a\nb\n>");
    let mut s = st("a\nb", 0);
    s.transform_selections(true, wrap).unwrap();
    assert_eq!(s.text(), "<a\nb>");
    // a selection sees the break of its last line, unless that line has none
    let mut s = selecting("a\nb\n", &[(2, 3)]);
    s.transform_selections(true, wrap).unwrap();
    assert_eq!(s.text(), "a\n<b\n>");
    let mut s = selecting("a\nb", &[(2, 3)]);
    s.transform_selections(true, wrap).unwrap();
    assert_eq!(s.text(), "a\n<b>");
}

#[test]
fn a_line_block_emptied_takes_its_line_break_with_it() {
    let gone = |t: &str| {
        let mut s = EditorState::from_text(t);
        let at = t.find('X').unwrap();
        s.view.selections = vec![Selection::new(at, at + 1)];
        s.transform_selections(true, |_| Ok(String::new())).unwrap();
        let (text, selections) = (s.text(), s.view.selections.clone());
        s.undo();
        assert_eq!(s.text(), t, "undo");
        s.redo();
        assert_eq!(s.text(), text, "redo");
        (text, selections)
    };
    // in the middle, at the start, at the end, with and without a final line break
    assert_eq!(gone("a\nX\nc"), ("a\nc".into(), vec![Selection::caret(2)]));
    assert_eq!(gone("X\nb\nc"), ("b\nc".into(), vec![Selection::caret(0)]));
    assert_eq!(gone("a\nb\nX"), ("a\nb".into(), vec![Selection::caret(3)]));
    assert_eq!(gone("a\nX\n"), ("a\n".into(), vec![Selection::caret(2)]));
    assert_eq!(gone("X\n"), (String::new(), vec![Selection::caret(0)]));
    assert_eq!(gone("X"), (String::new(), vec![Selection::caret(0)]));
    // two blocks, each emptied
    let mut s = selecting("X\nb\nX\nd", &[(0, 1), (4, 5)]);
    s.transform_selections(true, |_| Ok(String::new())).unwrap();
    assert_eq!(s.text(), "b\nd");
    // not emptied: the break stays where it was
    let mut s = selecting("a\nb\nc", &[(2, 3)]);
    s.transform_selections(true, |t| Ok(t.to_uppercase())).unwrap();
    assert_eq!(s.text(), "a\nB\nc");
}

#[test]
fn a_failed_line_transform_changes_nothing_and_names_the_first_line() {
    let mut s = selecting("x\ny\nz", &[(3, 4)]);
    let e = s.transform_selections(true, |_| Err("boom".into()));
    assert_eq!(e, Err("Line 2: boom".to_string()));
    assert_eq!(s.text(), "x\ny\nz");
    assert_eq!(s.view.selections, [Selection::new(3, 4)]);
    s.undo();
    assert_eq!(s.text(), "x\ny\nz", "no undo step was made");
}

#[test]
fn transform_texts_reads_what_a_transformation_would_change() {
    let mut s = EditorState::from_text("ab\ncd\nef");
    assert_eq!(s.transform_texts(true), ["ab\ncd\nef"]);
    s.view.selections = sels(&[(1, 2), (4, 4), (6, 7)]);
    assert_eq!(s.transform_texts(false), ["b", "e"]);
    assert_eq!(s.transform_texts(true), ["ab\n", "ef"]);
    assert_eq!(s.text(), "ab\ncd\nef");
}

#[test]
fn a_document_hash_is_the_hash_of_the_file_a_save_writes() {
    use crate::editing::convert::{digest, digest_file};
    let dir = fresh_dir("orlo-hash-saved");
    let p = dir.join("crlf.txt");
    let mut s = EditorState::from_text("one\ntwo\n");
    (s.doc.line_ending, s.doc.bom) = ("\r\n", true);
    s.save_to(p.clone()).unwrap();
    let file = std::fs::read(&p).unwrap();
    assert_eq!(file, "\u{feff}one\r\ntwo\r\n".as_bytes());
    assert_eq!(s.bytes_as_saved(None).unwrap(), file);
    let on_disk = digest_file(&p).unwrap();
    for (h, want) in Hash::ALL.into_iter().zip(&on_disk) {
        assert_eq!(&s.hash_selections(h).unwrap(), std::slice::from_ref(want));
        assert_eq!(&digest(h, &file), want);
    }
    // a selection of the whole document hashes like the file, BOM included
    s.select_all();
    assert_eq!(s.bytes_as_saved(Some(0..s.len())).unwrap(), file);
    for (h, want) in Hash::ALL.into_iter().zip(&on_disk) {
        assert_eq!(&s.hash_selections(h).unwrap(), std::slice::from_ref(want));
    }
    // a partial selection takes the line ending but never the BOM
    assert_eq!(s.bytes_as_saved(Some(0..4)).unwrap(), b"one\r\n");
    assert_eq!(s.bytes_as_saved(Some(0..7)).unwrap(), b"one\r\ntwo");
    s.set_selection(0, 7);
    assert_eq!(s.hash_selections(Hash::Md5).unwrap(), [digest(Hash::Md5, b"one\r\ntwo")]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hashing_several_selections_gives_one_digest_each_in_document_order() {
    use crate::editing::convert::digest;
    let mut s = selecting("abc\n\nfoo", &[(5, 8), (4, 4), (0, 3)]);
    let md5 = |t: &[u8]| digest(Hash::Md5, t);
    assert_eq!(s.hash_selections(Hash::Md5).unwrap(), [md5(b"abc"), md5(b"foo")]);
    // no selection: the document, line breaks and all
    s.view.selections = vec![Selection::caret(1)];
    assert_eq!(s.hash_selections(Hash::Md5).unwrap(), [md5(b"abc\n\nfoo")]);
}

#[test]
fn transpose_keeps_crlf_and_unicode_graphemes_whole() {
    for (text, caret, expected, after) in [
        ("ab\r\ncd", 2, "a\r\nbcd", 4),
        ("ae\u{301}b", 3, "abe\u{301}", 4),
        ("a👩\u{200d}💻b", 4, "ab👩\u{200d}💻", 5),
        ("a🇨🇦b", 3, "ab🇨🇦", 4),
    ] {
        let mut s = st(text, caret);
        s.transpose();
        assert_eq!(s.text(), expected, "{text:?}");
        assert_eq!(s.cursor(), after);
        s.undo();
        assert_eq!(s.text(), text);
        assert_eq!(s.cursor(), caret);
        s.redo();
        assert_eq!(s.text(), expected);
        assert_eq!(s.cursor(), after);
    }
}

#[test]
fn transpose_swaps_around_caret_and_rotates_selections() {
    let mut s = st("abcd\n", 2);
    s.transpose();
    assert_eq!(s.text(), "acbd\n");
    assert_eq!(s.cursor(), 3);
    let mut s = st("abcd\n", 0);
    s.transpose();
    assert_eq!(s.text(), "abcd\n");
    let mut s = st("abcd\n", 4);
    s.transpose();
    assert_eq!(s.text(), "abc\nd");
    let mut s = selecting("one two", &[(0, 3), (4, 7)]);
    s.transpose();
    assert_eq!(s.text(), "two one");
}

#[test]
fn join_lines_collapses_next_indent_to_one_space() {
    let mut s = st("hello\n  bar\nqux", 2);
    s.join_lines();
    assert_eq!(s.text(), "hello bar\nqux");
    assert_eq!(s.cursor(), 2);
    let mut s = selecting("a\nb\nc\nd", &[(0, 5)]);
    s.join_lines();
    assert_eq!(s.text(), "a b c\nd");
}

#[test]
fn permute_lines_whole_buffer_when_nothing_selected() {
    let mut s = st("b\nA\nc\nb\n", 0);
    s.permute_lines(sort(false));
    assert_eq!(s.text(), "A\nb\nb\nc\n");
    s.permute_lines(Permute::Unique);
    assert_eq!(s.text(), "A\nb\nc\n");
    s.permute_lines(Permute::Reverse);
    assert_eq!(s.text(), "c\nb\nA\n");
    s.permute_lines(sort(true));
    assert_eq!(s.text(), "A\nb\nc\n");
    s.view.selections = vec![Selection::new(0, 4)];
    s.permute_lines(Permute::Reverse);
    assert_eq!(s.text(), "b\nA\nc\n");
    s.permute_lines(Permute::Shuffle);
    assert_eq!(s.line_count(), 4);
}

#[test]
fn permute_selections_sorts_and_unique_blanks_repeats() {
    let mut s = selecting("b a a", &[(0, 1), (2, 3), (4, 5)]);
    s.permute_selections(sort(true));
    assert_eq!(s.text(), "a a b");
    s.permute_selections(Permute::Unique);
    assert_eq!(s.text(), "a  b");
}

#[test]
fn delete_to_line_edges_and_yank() {
    let mut s = st("hello\nworld", 2);
    s.delete_to_line_edge(true);
    assert_eq!(s.text(), "he\nworld");
    s.delete_to_line_edge(true);
    assert_eq!(s.text(), "heworld");
    s.yank();
    assert_eq!(s.text(), "he\nworld");
    s.view.selections = vec![Selection::caret(5)];
    s.delete_to_line_edge(false);
    assert_eq!(s.text(), "he\nrld");
}

#[test]
fn mark_select_swap_delete() {
    let mut s = st("abcdef", 1);
    s.set_mark();
    s.view.selections = vec![Selection::caret(4)];
    s.swap_with_mark();
    assert_eq!(s.cursor(), 1);
    s.swap_with_mark();
    assert_eq!(s.cursor(), 4);
    s.select_to_mark();
    assert_eq!(s.view.selections, vec![Selection::new(1, 4)]);
    s.delete_to_mark();
    assert_eq!(s.text(), "aef");
    s.yank();
    assert_eq!(s.text(), "abcdef");
    s.clear_mark();
    assert_eq!(s.view.mark, None);
}

#[test]
fn expand_to_paragraph_indentation_and_line_upward() {
    let mut s = st("a\nb\n\nc", 2);
    s.expand_to_paragraph();
    assert_eq!(s.view.selections, vec![Selection::new(0, 4)]);
    let mut s = st("x\n  a\n\n  b\ny", 4);
    s.expand_to_indentation();
    assert_eq!(s.view.selections, vec![Selection::new(2, 11)]);
    let mut s = st("ab\ncd\nef", 4);
    s.select_line_upward();
    assert_eq!(s.view.selections, vec![Selection::new(6, 3)]);
    s.select_line_upward();
    assert_eq!(s.view.selections, vec![Selection::new(6, 0)]);
}

#[test]
fn enclosing_brackets_count_chars_and_stop_at_the_document_edges() {
    let s = EditorState::from_text("a(é😀[x]b)");
    assert_eq!(s.enclosing_brackets(5, 5), Some((4, 6)));
    assert_eq!(s.enclosing_brackets(4, 7), Some((1, 8)));
    assert_eq!(s.enclosing_brackets(0, 0), None);
    assert_eq!(s.enclosing_brackets(9, 9), None);
}

#[test]
fn brackets_expand_outward_and_jump_toggles() {
    let mut s = st("(a [b {c} d] e)", 7);
    s.expand_to_brackets();
    assert_eq!(s.view.selections, vec![Selection::new(7, 8)]);
    s.expand_to_brackets();
    assert_eq!(s.view.selections, vec![Selection::new(6, 9)]);
    s.expand_to_brackets();
    assert_eq!(s.view.selections, vec![Selection::new(4, 11)]);
    let mut s = st("foo(bar)", 4);
    s.jump_to_bracket();
    assert_eq!(s.cursor(), 7);
    s.jump_to_bracket();
    assert_eq!(s.cursor(), 4);
    s.view.selections = vec![Selection::caret(8)];
    s.jump_to_bracket();
    assert_eq!(s.cursor(), 4);
    s.view.selections = vec![Selection::caret(3)];
    s.jump_to_bracket();
    assert_eq!(s.cursor(), 7);
}

#[test]
fn find_under_from_a_caret_takes_the_word_then_whole_words_as_sublime_does() {
    // Sublime 4215
    let mut s = st("foo foobar foo", 1);
    s.find_under();
    assert_eq!(s.view.selections, vec![Selection::new(0, 3)], "the word under the caret");
    s.find_under();
    assert_eq!(s.view.selections, vec![Selection::new(11, 14)], "foobar skipped");
    s.find_under();
    assert_eq!(s.view.selections, vec![Selection::new(0, 3)], "round to the top");
    // from a selection: any occurrence
    let mut s = st("foo foobar foo", 0);
    s.set_selection(0, 3);
    s.find_under();
    assert_eq!(s.view.selections, vec![Selection::new(4, 7)]);
}

#[test]
fn backspace_in_leading_spaces_goes_to_the_previous_tab_stop() {
    // Sublime 4215: 6 spaces lose 2, 8 lose 4, 3 lose 3; elsewhere one char
    for (spaces, left) in [(6, 4), (8, 4), (3, 0), (1, 0)] {
        let text = format!("{}x", " ".repeat(spaces));
        let mut s = st(&text, spaces);
        s.backspace();
        assert_eq!(s.line(0), format!("{}x", " ".repeat(left)), "{spaces} spaces");
    }
    let mut s = st("a   b", 4);
    s.backspace();
    assert_eq!(s.text(), "a  b", "not leading: one char");
}

#[test]
fn select_lines_keeps_the_display_column_across_tabs() {
    // Sublime 4215: a tab counts its width, not one char
    let mut s = st("\tab\n0123456789", 2);
    s.select_lines(true, &Lines, &Cells);
    assert_eq!(s.view.selections.last().unwrap().head, 4 + 5, "after the tab and a: column 5");
}

#[test]
fn wrap_lines_reflows_at_width_keeping_indent() {
    let mut s = st("  one two three four\n  five\n\nsix", 3);
    s.wrap_lines(12, None);
    assert_eq!(s.text(), "  one two\n  three four\n  five\n\nsix");
}

#[test]
fn paste_and_indent_prefixes_following_lines() {
    let mut s = st("  bar", 2);
    s.paste_and_indent("x\n  y\n");
    assert_eq!(s.text(), "  x\n    y\nbar");
    let mut s = st("    q", 4);
    s.paste_and_indent("    a\n      b");
    assert_eq!(s.text(), "    a\n      bq");
}

#[test]
fn bookmarks_toggle_cycle_and_shift_with_typing() {
    let mut s = st("a\nb\nc", 2);
    s.toggle_bookmark();
    s.view.selections = vec![Selection::caret(4)];
    s.toggle_bookmark();
    assert_eq!(s.view.bookmarks, vec![2, 4]);
    s.view.selections = vec![Selection::caret(0)];
    s.next_bookmark(true);
    assert_eq!(s.cursor(), 2);
    s.next_bookmark(true);
    assert_eq!(s.cursor(), 4);
    s.next_bookmark(true);
    assert_eq!(s.cursor(), 2);
    s.next_bookmark(false);
    assert_eq!(s.cursor(), 4);
    s.view.selections = vec![Selection::caret(0)];
    s.insert("xx");
    assert_eq!(s.view.bookmarks, vec![4, 6]);
    s.select_all_bookmarks();
    assert_eq!(s.view.selections.len(), 2);
    s.view.selections = vec![Selection::caret(4)];
    s.toggle_bookmark();
    assert_eq!(s.view.bookmarks, vec![6]);
    s.clear_bookmarks();
    assert!(s.view.bookmarks.is_empty());
}

#[test]
fn hard_tabs_when_not_translating() {
    let mut s = st("x", 1);
    s.doc.translate_tabs = false;
    s.tab();
    assert_eq!(s.text(), "x\t");
    s.indent();
    assert_eq!(s.text(), "\tx\t");
    let mut s = st("{", 1);
    s.doc.translate_tabs = false;
    s.newline();
    assert_eq!(s.text(), "{\n\t");
}

#[test]
fn tab_size_two_fills_to_the_next_stop_of_two() {
    let mut s = st("a", 1);
    s.doc.tab_size = 2;
    s.tab();
    assert_eq!(s.text(), "a ");
    let mut s = st("ab", 2);
    s.doc.tab_size = 2;
    s.tab();
    assert_eq!(s.text(), "ab  ");
}

#[test]
fn tab_with_two_carets_on_a_line_puts_both_on_tab_stops() {
    let mut s = carets("abcdefgh", &[1, 2]);
    s.doc.tab_size = 4;
    s.tab();
    assert_eq!(s.text(), "a   b   cdefgh");
    assert_eq!(s.view.selections, cs(&[4, 8]));
    // a tab after the first caret shrinks to its stop
    let mut s = carets("a\tbcd", &[1, 4]);
    s.doc.tab_size = 4;
    s.tab();
    assert_eq!(s.text(), "a   \tbc  d");
}

#[test]
fn tab_fills_to_the_next_stop() {
    let mut s = st("abcd", 2);
    s.tab();
    assert_eq!(s.text(), "ab  cd");
}

#[test]
fn uri_component_converts_unicode_multiline_targets_in_one_undo_step() {
    use crate::editing::convert::{decode, encode, Codec};
    let input = "a b/é | +😀\n行 | end";
    let mut s = selecting(input, &[(0, 5), (7, 7), (8, 12)]);
    let before = s.view.selections.clone();
    s.transform_selections(false, |t| Ok(encode(Codec::UriComponent, t))).unwrap();
    let encoded = "a%20b%2F%C3%A9 | %2B%F0%9F%98%80%0A%E8%A1%8C | end";
    assert_eq!(s.text(), encoded);
    assert_eq!(s.view.selections, sels(&[(0, 14), (16, 16), (17, 44)]));
    s.undo();
    assert_eq!(s.text(), input);
    assert_eq!(s.view.selections, before);
    s.redo();
    assert_eq!(s.text(), encoded);
    s.transform_selections(false, |t| decode(Codec::UriComponent, t)).unwrap();
    assert_eq!(s.text(), input);
    s.undo();
    assert_eq!(s.text(), encoded);
    s.redo();
    assert_eq!(s.text(), input);
}

#[test]
fn uri_component_rejects_each_invalid_target_without_partial_edits_or_history() {
    use crate::editing::convert::{decode, Codec};
    for bad in ["%", "%GG", "%FF", "%C3"] {
        let input = format!("a%20b\n{bad}");
        let mut s = selecting(&input, &[(0, 5), (6, input.chars().count())]);
        let before = (s.view.selections.clone(), s.doc.version, s.doc.dirty, s.doc.history.undo.len());
        let error = s.transform_selections(false, |t| decode(Codec::UriComponent, t)).unwrap_err();
        assert!(error.starts_with("Line 2:"), "{error}");
        assert_eq!(s.text(), input);
        assert_eq!((s.view.selections.clone(), s.doc.version, s.doc.dirty, s.doc.history.undo.len()), before);
        s.undo();
        assert_eq!(s.text(), input);
    }
}

#[test]
fn uri_component_keeps_mixed_virtual_only_ranges_through_edit_undo_redo() {
    use crate::editing::convert::{encode, Codec};
    let mut s = selecting("a b/é\nend", &[(0, 5)]);
    s.view.virtual_space = true.into();
    s.view.selections.push(Selection::from_points((9, 1), (9, 4)));
    let before = s.view.selections.clone();
    s.transform_selections(false, |t| Ok(encode(Codec::UriComponent, t))).unwrap();
    assert_eq!(s.text(), "a%20b%2F%C3%A9\nend");
    let after = vec![Selection::new(0, 14), Selection::from_points((18, 1), (18, 4))];
    assert_eq!(s.view.selections, after);
    s.undo();
    assert_eq!(s.text(), "a b/é\nend");
    assert_eq!(s.view.selections, before);
    s.redo();
    assert_eq!(s.text(), "a%20b%2F%C3%A9\nend");
    assert_eq!(s.view.selections, after);
}

#[test]
fn html_entities_multiple_targets_keep_virtual_endpoints_and_undo_redo() {
    use crate::editing::convert::{decode, encode, Codec};
    let codec: Codec = serde_json::from_str("\"html_entities\"").unwrap();
    let input = "<& | é'\nend";
    let mut s = selecting(input, &[(0, 2), (3, 3), (5, 7)]);
    s.view.virtual_space = true.into();
    s.view.selections.push(Selection::from_points((11, 1), (11, 4)));
    let before = s.view.selections.clone();
    s.transform_selections(false, |t| Ok(encode(codec, t))).unwrap();
    let escaped = "&lt;&amp; | é&#39;\nend";
    let after = vec![
        Selection::new(0, 9),
        Selection::caret(10),
        Selection::new(12, 18),
        Selection::from_points((22, 1), (22, 4)),
    ];
    assert_eq!(s.text(), escaped);
    assert_eq!(s.view.selections, after);
    s.undo();
    assert_eq!(s.text(), input);
    assert_eq!(s.view.selections, before);
    s.redo();
    assert_eq!(s.text(), escaped);
    assert_eq!(s.view.selections, after);
    s.transform_selections(false, |t| decode(codec, t)).unwrap();
    assert_eq!(s.text(), input);
    s.undo();
    assert_eq!(s.text(), escaped);
    s.redo();
    assert_eq!(s.text(), input);
}

#[test]
fn html_entities_invalid_later_numeric_target_leaves_no_edit_or_history() {
    use crate::editing::convert::{decode, Codec};
    let codec: Codec = serde_json::from_str("\"html_entities\"").unwrap();
    for bad in ["&#", "&#65", "&#xGG;", "&#xD800;", "&#1114112;", "&#999999999999999999999;"] {
        let input = format!("&lt;\n{bad}");
        let mut s = selecting(&input, &[(0, 4), (5, input.chars().count())]);
        let before = (s.view.selections.clone(), s.doc.version, s.doc.dirty, s.doc.history.undo.len());
        let error = s.transform_selections(false, |t| decode(codec, t)).unwrap_err();
        assert!(error.starts_with("Line 2:"), "{error}");
        assert_eq!(s.text(), input);
        assert_eq!((s.view.selections.clone(), s.doc.version, s.doc.dirty, s.doc.history.undo.len()), before);
        s.undo();
        assert_eq!(s.text(), input);
    }
}

#[test]
fn n06_transpose_words_caret_boundaries_eof_and_missing_partners() {
    for (source, at, expected) in [
        ("alpha, beta gamma", 0, "beta, alpha gamma"),
        ("alpha, beta gamma", 2, "beta, alpha gamma"),
        ("alpha, beta gamma", 5, "beta, alpha gamma"),
        ("alpha, beta gamma", 6, "beta, alpha gamma"),
        ("alpha, beta gamma", 7, "alpha, gamma beta"),
        ("alpha, beta gamma", 17, "alpha, gamma beta"),
        ("alpha,beta", 5, "beta,alpha"),
        ("alpha beta  ", 11, "alpha beta  "),
        ("alpha beta  ", 12, "beta alpha  "),
        (" alpha beta", 0, " alpha beta"),
        ("alpha beta", 8, "alpha beta"),
        ("alpha", 5, "alpha"),
        ("alpha", 1, "alpha"),
        (" , \t", 4, " , \t"),
        ("", 0, ""),
        ("same same", 1, "same same"),
    ] {
        let mut s = st(source, at);
        let original = s.view.selections.clone();
        let before = (s.doc.version, s.doc.state, s.view.sel_hist.generation);
        assert_eq!(s.transpose_words(), source != expected, "{source:?} at {at}");
        assert_eq!(s.text(), expected, "{source:?} at {at}");
        if source == expected {
            assert_eq!((s.doc.version, s.doc.state, s.view.sel_hist.generation), before);
            assert_eq!(s.doc.history.undo.len(), 0);
            assert_eq!(s.view.selections, original);
        } else {
            assert_eq!(s.doc.history.undo.len(), 1);
            s.undo();
            assert_eq!(s.text(), source);
            assert_eq!(s.view.selections, original);
            s.redo();
            assert_eq!(s.text(), expected);
        }
    }
}

#[test]
fn n06_transpose_words_unicode_and_separator_source_bytes_are_never_replaced() {
    for (left, separator, right) in [
        ("cafe\u{301}", ", \t\n", "βήτα"),
        ("a\u{301}", "👩🏽\u{200d}💻\t\r\n  ", "東京"),
        ("foo_bar", "—,.;\t\n", "123"),
        ("é", "\r\n", "longer"),
    ] {
        let source = format!("{left}{separator}{right}");
        let mut s = st(&source, 1);
        let a = left.chars().count();
        let b = a + separator.chars().count();
        let edits = s.transpose_word_edits();
        assert_eq!(edits, [(0..a, right.into()), (b..source.chars().count(), left.into())]);
        for caret in a..b {
            let mut at_separator = st(&source, caret);
            assert!(at_separator.transpose_words(), "separator offset {caret}");
            assert_eq!(at_separator.text(), format!("{right}{separator}{left}"));
        }
        assert!(s.transpose_words());
        assert_eq!(s.text().as_bytes(), format!("{right}{separator}{left}").as_bytes());
        assert_eq!(s.doc.history.undo.len(), 1);
        s.undo();
        assert_eq!(s.text().as_bytes(), source.as_bytes());
    }
}

#[test]
fn n06_transpose_words_multicaret_overlap_rejects_all_before_history_or_selection_changes() {
    for carets in [[1, 6, 18], [1, 2, 18]] {
        let mut s = selecting("alpha beta gamma delta epsilon", &carets.map(|n| (n, n)));
        s.insert("x");
        s.undo();
        let before = (
            s.text(),
            s.view.selections.clone(),
            s.doc.version,
            s.doc.state,
            s.doc.history.undo.len(),
            s.doc.history.redo.len(),
            s.view.sel_hist.generation,
        );
        assert!(!s.transpose_words());
        assert_eq!(
            (
                s.text(),
                s.view.selections.clone(),
                s.doc.version,
                s.doc.state,
                s.doc.history.undo.len(),
                s.doc.history.redo.len(),
                s.view.sel_hist.generation
            ),
            before
        );
        s.redo();
        assert!(s.text().contains('x'), "rejection preserves the real redo branch");
    }
    let mut duplicate = st("alpha beta", 1);
    duplicate.set_selections(vec![Selection::caret(1), Selection::caret(1)]);
    assert_eq!(duplicate.view.selections.len(), 1, "existing selection normalization deduplicates exact carets");
    assert!(duplicate.transpose_words());
    assert_eq!(duplicate.text(), "beta alpha");
    let mut s = selecting("alpha beta gamma delta", &[(1, 1), (18, 12)]);
    s.view.primary = 1;
    s.view.selections[1].rectangular = true;
    s.view.selections[1].goal = Some(Goal::X(9.));
    let original = s.view.selections.clone();
    assert!(s.transpose_words());
    assert_eq!(s.text(), "beta alpha delta gamma");
    assert_eq!(s.view.primary, 1);
    assert!(s.view.selections[1].anchor > s.view.selections[1].head);
    assert!(s.view.selections[1].rectangular);
    assert_eq!(s.view.selections[1].goal, None);
    assert_eq!(s.doc.history.undo.len(), 1);
    s.undo();
    assert_eq!(s.text(), "alpha beta gamma delta");
    assert_eq!(s.view.selections, original);
    assert_eq!(s.view.primary, 1);
    s.redo();
    assert_eq!(s.text(), "beta alpha delta gamma");
}

#[test]
fn n06_transpose_words_virtual_no_padding_follow_readonly_and_shared_undo() {
    let mut s = st("alpha beta\nend", 10);
    s.set_virtual_space(true);
    s.view.selections = vec![Selection::from_points((14, 3), (14, 7)).in_rectangle()];
    let original = s.view.selections.clone();
    assert!(!s.transpose_words());
    assert_eq!(s.view.selections, original);
    assert_eq!(s.doc.history.undo.len(), 0);
    s.set_selections(vec![Selection::caret(2), original[0]]);
    let original = s.view.selections.clone();
    assert!(s.transpose_words());
    assert_eq!(s.text(), "beta alpha\nend");
    assert_eq!(s.view.selections[1], original[1], "untouched virtual EOL survives");
    s.undo();
    assert_eq!(s.view.selections, original);
    s.doc.set_following(true);
    assert!(!s.transpose_words());
    s.doc.set_following(false);
    s.doc.read_only = true;
    assert!(s.transpose_words());
    assert_eq!(s.text(), "beta alpha\nend");

    let mut v = views("alpha beta gamma delta");
    v.b().set_cursor(22);
    v.a().set_selections(vec![Selection::caret(1), Selection::caret(13)]);
    let original = v.a().view.selections.clone();
    assert!(v.a().transpose_words());
    assert_eq!(v.b().text(), "beta alpha delta gamma");
    assert_eq!(v.b().cursor(), 22);
    assert_eq!(v.doc.history.undo.len(), 1);
    v.a().undo();
    assert_eq!(v.b().text(), "alpha beta gamma delta");
    assert_eq!(v.a().view.selections, original);
    v.a().redo();
    assert_eq!(v.b().text(), "beta alpha delta gamma");
}

#[test]
fn n06_transpose_words_keeps_combining_separator_clusters_on_both_word_edges() {
    // Controls break clusters; standalone Alphabetic marks retain existing word classification.
    for (source, expected) in [
        ("alpha \u{345}beta", "beta \u{345}alpha"),
        ("alpha\t\u{345}beta", "\u{345}beta\talpha"),
        ("alpha\n\u{345}beta", "\u{345}beta\nalpha"),
        ("\u{345}beta alpha", "alpha \u{345}beta"),
    ] {
        let mut example = st(source, 1);
        assert!(example.transpose_words());
        assert_eq!(example.text(), expected);
        example.undo();
        assert_eq!(example.text(), source);
        example.redo();
        assert_eq!(example.text(), expected);
    }
    let (left, right) = ("cafe\u{301}", "βη\u{301}τα\u{301}");
    for separator in [
        " \u{301}",
        ",\u{301}",
        "\t\u{301}",
        "\n\u{301}",
        " \u{200d}",
        "—\u{301}  ",
        " \u{345}",
        ",\u{345}",
        "👩‍💻\u{345}",
        " \u{93e}",
    ] {
        let source = format!("{left}{separator}{right}");
        let expected = format!("{right}{separator}{left}");
        let a = left.chars().count();
        let b = a + separator.chars().count();
        for caret in (0..b).chain([source.chars().count()]) {
            let mut s = st(&source, caret);
            let original = s.view.selections.clone();
            assert!(s.transpose_words(), "{separator:?} at {caret}");
            assert_eq!(s.text().as_bytes(), expected.as_bytes(), "separator {separator:?} at {caret}");
            assert_eq!(s.doc.history.undo.len(), 1);
            s.undo();
            assert_eq!(s.text().as_bytes(), source.as_bytes());
            assert_eq!(s.view.selections, original);
            let edits = s.transpose_word_edits();
            assert_eq!(edits, [(0..a, right.into()), (b..source.chars().count(), left.into())]);
            assert!(edits
                .iter()
                .all(|(r, _)| s.snap_to_cluster(r.start) == r.start && s.snap_to_cluster(r.end) == r.end));
            s.redo();
            assert_eq!(s.text().as_bytes(), expected.as_bytes());
        }
    }
}

#[test]
fn n06_transpose_words_prepend_prefixes_keep_complete_word_bases() {
    for (source, expected) in [
        ("alpha \u{600}beta", "\u{600}beta alpha"),
        ("alpha \u{600}\u{600}beta", "\u{600}\u{600}beta alpha"),
        ("alpha \u{600}123", "\u{600}123 alpha"),
        ("alpha \u{600}\u{345}beta", "\u{600}\u{345}beta alpha"),
        ("alpha,\u{600}\u{345}beta", "\u{600}\u{345}beta,alpha"),
        ("alpha\u{600} \u{345}beta", "beta\u{600} \u{345}alpha"),
        ("alpha\u{600},\u{345}beta", "beta\u{600},\u{345}alpha"),
        ("alpha \u{600}👩‍💻\u{345}beta", "beta \u{600}👩‍💻\u{345}alpha"),
        ("alpha\u{600}\t\u{345}beta", "\u{345}beta\u{600}\talpha"),
    ] {
        let mut s = st(source, 1);
        let original = s.view.selections.clone();
        assert!(s.transpose_words(), "{source:?}");
        assert_eq!(s.text().as_bytes(), expected.as_bytes(), "{source:?}");
        assert_eq!(s.doc.history.undo.len(), 1);
        s.undo();
        assert_eq!(s.text().as_bytes(), source.as_bytes());
        assert_eq!(s.view.selections, original);
        s.redo();
        assert_eq!(s.text().as_bytes(), expected.as_bytes());
    }
    assert_eq!(
        st("alpha \u{600}beta", 1).transpose_word_edits(),
        [(0..5, "\u{600}beta".into()), (6..11, "alpha".into())]
    );
}

#[test]
fn n07_surrounds_change_delete_nth_pair_and_keep_exact_interior_bytes() {
    use std::num::NonZeroUsize;
    for (source, caret, target, replacement, count, expected) in [
        ("(value)", 3, Surround::Round, Some(Surround::Square), 1, "[value]"),
        ("{value}", 3, Surround::Curly, Some(Surround::Round), 1, "(value)"),
        ("[value]", 3, Surround::Square, Some(Surround::Curly), 1, "{value}"),
        ("[]", 1, Surround::Square, None, 1, ""),
        ("\"\"", 1, Surround::DoubleQuote, None, 1, ""),
        ("((value))", 4, Surround::Round, None, 2, "(value)"),
        ("{[(é\u{301}👩‍💻\r\n x)]}", 6, Surround::Round, Some(Surround::DoubleQuote), 1, "{[\"é\u{301}👩‍💻\r\n x\"]}"),
        (r#""a\"b\\""#, 3, Surround::DoubleQuote, Some(Surround::SingleQuote), 1, r#"'a\"b\\'"#),
        (r#"'a\'b'"#, 2, Surround::SingleQuote, None, 1, r#"a\'b"#),
        (r#"("[ignored]" + [real])"#, 17, Surround::Square, None, 1, r#"("[ignored]" + real)"#),
        (r#"(a\)b)"#, 3, Surround::Round, None, 1, r#"a\)b"#),
        ("'a\nb'", 3, Surround::SingleQuote, None, 1, "a\nb"),
    ] {
        let mut s = st(source, caret);
        let original = s.selection_state();
        let edits = s.surround_edits(target, replacement, NonZeroUsize::new(count).unwrap());
        assert_eq!(edits.len(), 2, "{source:?}");
        assert!(edits.iter().all(|(range, _)| range.end == range.start + 1));
        assert!(s.replace_ranges(edits));
        assert_eq!(s.text().as_bytes(), expected.as_bytes(), "{source:?}");
        assert_eq!(s.doc.history.undo.len(), 1);
        s.undo();
        assert_eq!(s.text().as_bytes(), source.as_bytes());
        assert!(s.selection_state() == original);
        s.redo();
        assert_eq!(s.text().as_bytes(), expected.as_bytes());
    }
}

#[test]
fn n07_surrounds_boundaries_and_selections_choose_complete_enclosing_pairs() {
    for (a, b) in [(0, 0), (1, 1), (3, 3), (4, 4), (5, 5), (1, 4), (4, 1), (0, 5), (0, 4), (1, 5)] {
        let mut s = selecting("(abc)", &[(a, b)]);
        let before = s.selection_state();
        let edits = s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN);
        assert!(s.replace_ranges(edits), "{a}..{b}");
        assert_eq!(s.text(), "abc");
        assert_eq!(s.view.selections, sels(&[(a.saturating_sub(1).min(3), b.saturating_sub(1).min(3))]));
        s.undo();
        assert!(s.selection_state() == before);
    }
    let s = st("(a)(b)", 3);
    assert_eq!(
        s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN),
        [(3..4, "".into()), (5..6, "".into())]
    );
}

#[test]
fn n07_surrounds_malformed_escaped_unmatched_and_no_change_preserve_history() {
    for (source, target, caret, count, replacement) in [
        ("(abc", Surround::Round, 2, 1, None),
        ("abc)", Surround::Round, 2, 1, None),
        ("([abc)]", Surround::Round, 3, 1, None),
        ("(abc)", Surround::Round, 2, 2, None),
        ("(abc)", Surround::Square, 2, 1, None),
        ("(abc)", Surround::Round, 2, 1, Some(Surround::Round)),
        (r#"\(abc\)"#, Surround::Round, 3, 1, None),
        (r#""a\""#, Surround::DoubleQuote, 2, 1, None),
        (r#""[abc]""#, Surround::Square, 3, 1, None),
        ("(a)(b) x", Surround::Round, 8, 1, None),
    ] {
        let mut s = st(source, caret.min(source.chars().count()));
        s.insert("z");
        s.undo();
        let before = (
            s.text(),
            s.view.selections.clone(),
            s.doc.version,
            s.doc.state,
            s.doc.history.undo.len(),
            s.doc.history.redo.len(),
            s.selection_generation(),
        );
        assert!(
            s.surround_edits(target, replacement, std::num::NonZeroUsize::new(count).unwrap()).is_empty(),
            "{source:?}"
        );
        assert!(!s.replace_ranges(Vec::new()));
        assert_eq!(
            (
                s.text(),
                s.view.selections.clone(),
                s.doc.version,
                s.doc.state,
                s.doc.history.undo.len(),
                s.doc.history.redo.len(),
                s.selection_generation()
            ),
            before
        );
        s.redo();
        assert!(s.text().contains('z'));
    }
}

#[test]
fn n07_surrounds_deduplicate_pairs_reject_nested_conflicts_and_map_reverse_origins() {
    let mut s = selecting("(αβ) (cafe\u{301})", &[(1, 1), (3, 2), (10, 7)]);
    s.view.primary = 2;
    s.view.selections[2].rectangular = true;
    s.view.selections[2].goal = Some(Goal::X(9.));
    let original = s.selection_state();
    let edits = s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN);
    assert_eq!(edits, [(0..1, "".into()), (3..4, "".into()), (5..6, "".into()), (11..12, "".into())]);
    assert!(s.replace_ranges(edits));
    assert_eq!(s.text(), "αβ cafe\u{301}");
    assert_eq!(s.view.primary, 2);
    assert!(s.view.selections[2].rectangular && s.view.selections[2].anchor > s.view.selections[2].head);
    s.undo();
    assert!(s.selection_state() == original);
    s.redo();
    assert_eq!(s.text(), "αβ cafe\u{301}");
    let s = selecting("((a)) (b)", &[(0, 0), (2, 2), (7, 7)]);
    assert!(s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN).is_empty());
    let mut s = selecting("(yes) no", &[(2, 2), (8, 8)]);
    assert!(s.replace_ranges(s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN)));
    assert_eq!(s.text(), "yes no");
}

#[test]
fn n07_surrounds_virtual_targets_follow_and_filesystem_readonly() {
    let mut s = st("(a)", 3);
    s.set_virtual_space(true);
    s.view.selections = vec![Selection::from_points((3, 2), (3, 4)).in_rectangle()];
    let original = s.view.selections.clone();
    assert!(s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN).is_empty());
    assert_eq!(s.view.selections, original);
    s.set_cursor(2);
    s.doc.set_following(true);
    assert!(s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN).is_empty());
    s.doc.set_following(false);
    s.doc.read_only = true;
    assert!(s.replace_ranges(s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN)));
    assert_eq!(s.text(), "a");
}

#[test]
fn n07_surrounds_physical_targets_map_unaffected_virtual_origin_without_padding() {
    let mut s = st("(a)\nend", 2);
    s.set_virtual_space(true);
    s.view.selections = vec![Selection::caret(2), Selection::from_points((7, 2), (7, 4)).in_rectangle()];
    s.view.primary = 1;
    let original = s.selection_state();
    assert!(s.replace_ranges(s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN)));
    assert_eq!(s.text(), "a\nend");
    assert_eq!(s.view.selections, [Selection::caret(1), Selection::from_points((5, 2), (5, 4)).in_rectangle()]);
    assert_eq!(s.view.primary, 1);
    s.undo();
    assert!(s.selection_state() == original);
    s.redo();
    assert_eq!(s.text(), "a\nend");
}

#[test]
fn n07_surrounds_contractions_and_unmatched_quotes_leave_brackets_reachable() {
    for source in ["don't (value)", "\" (value)", "' (value)", "don't \"quoted\" (value)"] {
        let mut s = st(source, source.chars().count() - 3);
        assert!(s.replace_ranges(s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::MIN)), "{source:?}");
        assert_eq!(s.text(), source.replace("(value)", "value"));
        s.undo();
        assert_eq!(s.text(), source);
    }
    let s = st("(a)(b)", 3);
    assert!(
        s.surround_edits(Surround::Round, None, std::num::NonZeroUsize::new(2).unwrap()).is_empty(),
        "the previous adjacent pair is not an outer pair"
    );
}

#[test]
fn n09_value_preparation_is_atomic_and_noops_preserve_full_state_and_real_redo() {
    use crate::columns::ValueStep;
    for text in ["009 é9", "009 170141183460469231731687303715884105727"] {
        let mut s = EditorState::from_text(text);
        s.set_cursor(s.len());
        s.insert("x");
        s.undo();
        s.set_selections(vec![Selection::new(3, 0), Selection::new(4, s.len())]);
        let before = (s.text(), s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point());
        assert!(s.adjust_value_edits(ValueStep(1)).is_err());
        assert!((s.text(), s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point()) == before);
        s.redo();
        assert_eq!(s.text(), format!("{text}x"));
    }
    let mut s = selecting("009 0xaB", &[(3, 0), (4, 8)]);
    s.set_virtual_space(true);
    s.view.selections.push(Selection::from_points((8, 2), (8, 5)));
    s.view.selections[0].rectangular = true;
    let original = s.selection_state();
    assert!(s.adjust_value_edits(ValueStep(0)).unwrap().is_empty());
    let edits = s.adjust_value_edits(ValueStep(1)).unwrap();
    assert_eq!(edits.len(), 2);
    assert!(s.replace_ranges(edits));
    assert_eq!(s.text(), "010 0xaC");
    s.undo();
    assert_eq!(s.text(), "009 0xaB");
    assert!(s.selection_state() == original);
    s.redo();
    assert_eq!(s.text(), "010 0xaC");
    s.undo();
    let before = (s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point());
    assert!(s.adjust_value_edits(ValueStep(0)).unwrap().is_empty());
    assert!((s.selection_state(), s.selection_generation(), s.doc.version, s.doc.save_point()) == before);
    s.redo();
    assert_eq!(s.text(), "010 0xaC");
    let s = carets("009", &[1, 3]);
    assert!(s.adjust_value_edits(ValueStep(1)).unwrap().is_empty());
    assert_eq!(s.text(), "009");
}

#[test]
fn n30_touching_line_selections_share_one_header_and_disjoint_blocks_stay_independent() {
    use crate::editing::transform::{sort_by_pattern, SortOpts};
    let before = "H\nv2-part10\nv2-part9\nGAP\nH2\nv3-part10\nv3-part9";
    let mut s = st(before, 0);
    s.view.selections = vec![
        Selection::new(1, s.line_start(2)),
        Selection::new(s.line_start(2) + 1, s.line_start(3)),
        Selection::caret(s.line_start(3) + 1),
        Selection::new(s.len(), s.line_start(4)),
    ];
    let selections = s.view.selections.clone();
    let opts: SortOpts =
        serde_json::from_value(serde_json::json!({"pattern":".*", "order":"natural", "keep_header":true})).unwrap();
    s.transform_selections(true, |text| sort_by_pattern(text, &opts)).unwrap();
    assert_eq!(s.text(), "H\nv2-part9\nv2-part10\nGAP\nH2\nv3-part9\nv3-part10");
    s.undo();
    assert_eq!(s.text(), before);
    assert_eq!(s.view.selections, selections);
}
