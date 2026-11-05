use super::*;

#[test]
fn block_comment_wraps_strips_and_opens_empty_pair() {
    let mut s = st("x", 0);
    s.toggle_block_comment("/*", "*/");
    assert_eq!(s.text(), "/**/x");
    assert_eq!(s.cursor(), 2);
    let mut s = selecting("ab", &[(0, 2)]);
    s.toggle_block_comment("/*", "*/");
    assert_eq!(s.text(), "/*ab*/");
    assert_eq!(s.view.selections, vec![Selection::new(2, 4)]);
    s.view.selections = vec![Selection::new(0, 6)];
    s.toggle_block_comment("/*", "*/");
    assert_eq!(s.text(), "ab");
}

#[test]
fn convert_indentation_both_ways() {
    let mut s = st("\tx\n    y\n  z", 0);
    s.convert_indentation(true);
    assert_eq!(s.text(), "    x\n    y\n  z");
    s.convert_indentation(false);
    assert_eq!(s.text(), "\tx\n\ty\n  z");
}

#[test]
fn insert_at_two_cursors_keeps_both_aligned() {
    let mut s = carets("ab\ncd", &[1, 4]);
    s.insert("X");
    assert_eq!(s.text(), "aXb\ncXd");
    assert_eq!(s.view.selections, cs(&[2, 6]));
}

#[test]
fn backspace_at_start_and_delete_at_end_are_noops() {
    let mut s = st("a", 0);
    s.backspace();
    assert_eq!(s.text(), "a");
    s.view.selections = vec![Selection::caret(1)];
    s.delete();
    assert_eq!(s.text(), "a");
}

#[test]
fn backspace_removes_selection_then_char() {
    let mut s = selecting("hello", &[(1, 4)]);
    s.backspace();
    assert_eq!(s.text(), "ho");
    s.backspace();
    assert_eq!(s.text(), "o");
}

#[test]
fn newline_copies_indent_and_opens_brace() {
    let mut s = st("    foo {", 9);
    s.newline();
    assert_eq!(s.text(), "    foo {\n        ");
}

#[test]
fn point_and_offset_round_trip_and_clamp() {
    let s = EditorState::from_text("ab\ncde");
    assert_eq!(s.point(4), (1, 1));
    assert_eq!(s.offset(1, 1), 4);
    assert_eq!(s.offset(1, 99), 6);
    assert_eq!(s.offset(9, 0), 6);
    assert_eq!(s.line(0), "ab");
    assert_eq!(s.line_end(0), 2);
}

#[test]
fn typing_coalesces_into_one_undo_step_but_newline_breaks_it() {
    let mut s = EditorState::new();
    s.insert("a");
    s.insert("b");
    s.newline();
    s.insert("c");
    s.undo();
    assert_eq!(s.text(), "ab\n");
    s.undo();
    assert_eq!(s.text(), "ab");
    s.undo();
    assert_eq!(s.text(), "");
    s.redo();
    assert_eq!(s.text(), "ab");
    assert_eq!(s.view.selections, vec![Selection::caret(2)]);
}

#[test]
fn arrows_and_delete_take_whole_clusters_and_backspace_one_code_point() {
    // probed in Sublime 4: a, a family emoji (7 code points), b, a flag, c, e + accent, d
    let text = "a👩\u{200d}👩\u{200d}👧\u{200d}👦b🇯🇵ce\u{301}d\n";
    let mut s = EditorState::from_text(text);
    s.set_cursor(0);
    let rows = Lines;
    let mut seen = Vec::new();
    for _ in 0..7 {
        s.move_cursors_in(Motion::Right, false, &rows, &Cells);
        seen.push(s.cursor());
    }
    assert_eq!(seen, [1, 8, 9, 11, 12, 14, 15]);
    s.move_cursors_in(Motion::Left, false, &rows, &Cells);
    assert_eq!(s.cursor(), 14, "back over d");
    s.move_cursors_in(Motion::Left, false, &rows, &Cells);
    assert_eq!(s.cursor(), 12, "back over e and its accent together");
    s.set_cursor(8);
    s.move_cursors_in(Motion::Left, false, &rows, &Cells);
    assert_eq!(s.cursor(), 1, "back over the whole family");
    s.set_cursor(14);
    s.backspace();
    assert_eq!(s.text(), "a👩\u{200d}👩\u{200d}👧\u{200d}👦b🇯🇵ced\n", "only the accent");
    s.set_cursor(1);
    s.delete();
    assert_eq!(s.text(), "ab🇯🇵ced\n", "the whole family");
}

#[test]
fn cmd_double_click_adds_the_word_and_keeps_the_other_carets() {
    let mut s = EditorState::from_text("one two three\nfour five\n");
    s.set_cursor(0);
    s.add_cursor(18);
    for clicks in [1, 2] {
        let cmd_click = s.drag_from(5, clicks, true, false);
        s.drag_to(&cmd_click, 5);
    }
    let got: Vec<_> = s.view.selections.iter().map(|x| x.range()).collect();
    assert_eq!(got, vec![0..0, 4..7, 18..18]);
    assert_eq!(s.cursor(), 7, "the word is the primary selection");
}

#[test]
fn cmd_shift_clicks_take_a_char_word_or_line_out_as_sublime_does() {
    // Sublime 4215 with `drag_text` off
    let text = "foo bar baz\nqux quux\nend\n";
    let run = |sels: &[(usize, usize)], at: usize, clicks: usize, to: Option<usize>| {
        let mut s = EditorState::from_text(text);
        s.set_selections(super::sels(sels));
        let from = s.drag_from(at, clicks, true, true);
        s.drag_to(&from, at);
        if let Some(to) = to {
            s.drag_to(&from, to);
        }
        ends_of(&s)
    };
    let words = [(0, 3), (4, 7), (8, 11)];
    assert_eq!(run(&words, 5, 2, None), [(0, 3), (8, 11)], "Q1");
    assert_eq!(run(&[(1, 1), (5, 5), (9, 9)], 5, 1, None), [(1, 1), (9, 9)], "Q2");
    assert_eq!(run(&[(0, 11)], 5, 2, None), [(0, 4), (7, 11)], "Q3");
    assert_eq!(run(&[(5, 5)], 5, 1, None), [(5, 5)], "Q4: one caret is left");
    assert_eq!(run(&[(1, 1), (9, 9)], 5, 2, None), [(1, 1), (9, 9)], "Q5");
    assert_eq!(run(&[(0, 12), (14, 14)], 5, 3, None), [(14, 14)], "Q6");
    let q7 = [(1, 1), (5, 5), (9, 9), (12, 15)];
    assert_eq!(run(&q7, 1, 1, Some(13)), [(13, 15)], "Q7");
    assert_eq!(run(&[(0, 11)], 5, 1, None), [(0, 5), (5, 11)], "Q8");
    assert_eq!(run(&[(0, 11), (14, 14)], 1, 2, None), [(3, 11), (14, 14)], "Q9");
    assert_eq!(run(&[(0, 3), (8, 11)], 9, 2, None), [(0, 3)], "Q10");
    assert_eq!(run(&[(11, 0)], 5, 2, None), [(4, 0), (11, 7)], "each part faces as before");
}

#[test]
fn normalize_merges_overlapping_selections() {
    let mut s = selecting("abcdef", &[(3, 5), (0, 4), (5, 5)]);
    s.normalize();
    assert_eq!(s.view.selections, vec![Selection::new(0, 5)]);
}

#[test]
fn dirty_line_tracks_lowest_edit() {
    let mut s = st("a\nb\nc", 4);
    s.insert("x");
    s.view.selections = vec![Selection::caret(0)];
    s.insert("y");
    assert_eq!(s.take_dirty_line(), Some(0));
    assert_eq!(s.take_dirty_line(), None);
}

#[test]
fn language_newline_ignores_multiline_string_delimiters() {
    use crate::commands::indentation::IndentLanguage;
    let mut s = st("value = (\"\"\"{", 0);
    s.set_cursor(s.len());
    s.newline_with_language(Some(IndentLanguage::Python));
    assert_eq!(s.text(), "value = (\"\"\"{\n");
}

#[test]
fn language_reindent_python_selection_and_undo() {
    use crate::commands::indentation::IndentLanguage;
    let original = "if ok:\nwork()\nelse:\n雪()\nafter()";
    let mut s = selecting(original, &[(0, original[..original.find("after").unwrap()].chars().count())]);
    assert!(s.reindent_selection(IndentLanguage::Python));
    assert_eq!(s.text(), "if ok:\n    work()\nelse:\n    雪()\nafter()");
    s.undo();
    assert_eq!(s.text(), original);
    s.redo();
    assert_eq!(s.text(), "if ok:\n    work()\nelse:\n    雪()\nafter()");
}

#[test]
fn language_reindent_braces_continuations_and_comments() {
    use crate::commands::indentation::IndentLanguage;
    for language in [IndentLanguage::Rust, IndentLanguage::JavaScript, IndentLanguage::TypeScript] {
        let original = "fn x() {\ncall(\narg,\n);\n/* {\nstill }\n*/\n} else {\ny();\n}";
        let mut s = selecting(original, &[(0, original.chars().count())]);
        assert!(s.reindent_selection(language));
        assert_eq!(s.text(), "fn x() {\n    call(\n        arg,\n    );\n    /* {\nstill }\n*/\n} else {\n    y();\n}");
        s.undo();
        assert_eq!(s.text(), original);
    }
}

#[test]
fn language_newline_keywords_and_tabs() {
    use crate::commands::indentation::IndentLanguage;
    for (input, expected) in
        [("if ok:", "if ok:\n    "), ("    return x", "    return x\n"), ("x = '# {'", "x = '# {'\n"), ("# {", "# {\n")]
    {
        let mut s = st(input, input.chars().count());
        s.newline_with_language(Some(IndentLanguage::Python));
        assert_eq!(s.text(), expected);
    }
    let mut s = st("if ok:", 6);
    s.doc.translate_tabs = false;
    s.newline_with_language(Some(IndentLanguage::Python));
    assert_eq!(s.text(), "if ok:\n\t");
}

#[test]
fn language_newline_rust_raw_strings_and_lifetimes() {
    use crate::commands::indentation::IndentLanguage;
    for (source, expected) in [("let s = r#\"{", "let s = r#\"{\n"), ("fn f<'a>() {", "fn f<'a>() {\n    ")] {
        let mut s = st(source, source.chars().count());
        s.newline_with_language(Some(IndentLanguage::Rust));
        assert_eq!(s.text(), expected);
    }
}

#[test]
fn language_reindent_overlaps_reversed_and_separate_tab_widths() {
    use crate::commands::indentation::IndentLanguage;
    let text = "if x:\n雪()\nif y:\nz()\nend";
    let mut s = selecting(text, &[(0, 15), (20, 6)]);
    s.doc.tab_size = 4;
    s.doc.indent_size = Some(2);
    s.doc.translate_tabs = false;
    assert!(s.reindent_selection(IndentLanguage::Python));
    assert_eq!(s.text(), "if x:\n  雪()\n  if y:\n\tz()\nend");
    s.undo();
    assert_eq!(s.text(), text);
}

#[test]
fn language_context_limit_declines_reindent_and_falls_back_on_newline() {
    use crate::commands::indentation::IndentLanguage;
    let text = format!("{}if x:", "x".repeat(65536));
    let mut s = st(&text, text.len());
    assert!(!s.reindent_selection(IndentLanguage::Python));
    assert_eq!(s.text(), text);
    s.newline_with_language(Some(IndentLanguage::Python));
    assert_eq!(s.text(), format!("{text}\n"));
}

#[test]
fn language_newline_closing_and_reopening_braces() {
    use crate::commands::indentation::IndentLanguage;
    for (source, expected) in [
        ("if x {\n    }", "if x {\n    }\n    "),
        ("if x {\n    work(); }", "if x {\n    work(); }\n"),
        ("if x {\n} else {", "if x {\n} else {\n    "),
    ] {
        let mut s = st(source, source.chars().count());
        s.newline_with_language(Some(IndentLanguage::JavaScript));
        assert_eq!(s.text(), expected);
    }
}

#[test]
fn reindent_remembers_block_before_multiline_comment() {
    use crate::commands::indentation::IndentLanguage;
    let text = "fn x() { /*\ncomment\n*/\ny();\n}";
    let mut s = selecting(text, &[(0, text.chars().count())]);
    assert!(s.reindent_selection(IndentLanguage::Rust));
    assert_eq!(s.text(), "fn x() { /*\ncomment\n*/\n    y();\n}");
}

#[test]
fn reindent_python_nested_dedent_and_crlf() {
    use crate::commands::indentation::IndentLanguage;
    let text = "if a:\r\nif b:\r\nreturn x\r\n  else:\r\npass\r\n";
    let mut s = selecting(text, &[(0, text.chars().count())]);
    assert!(s.reindent_selection(IndentLanguage::Python));
    assert_eq!(s.text(), "if a:\r\n    if b:\r\n        return x\r\n    else:\r\n        pass\r\n");
    s.undo();
    assert_eq!(s.text(), text);
}

#[test]
fn language_newline_does_not_split_pairs_inside_multiline_comments_or_strings() {
    use crate::commands::indentation::IndentLanguage;
    for (language, source) in [
        (IndentLanguage::Rust, "/* comment\n    {}"),
        (IndentLanguage::JavaScript, "`string\n    {}"),
        (IndentLanguage::TypeScript, "/* comment\n    {}"),
        (IndentLanguage::Python, "\"\"\"string\n    {}"),
    ] {
        let at = source.chars().count() - 1;
        let mut s = st(source, at);
        s.newline_with_language(Some(language));
        assert_eq!(s.text(), source.replacen("{}", "{\n    }", 1));
        s.undo();
        assert_eq!(s.text(), source);
    }
}

#[test]
fn rust_multiline_quoted_string_preserves_literal_and_following_code() {
    use crate::commands::indentation::IndentLanguage;
    let text = "fn x() {\nlet text = \"first\n  { literal\nlast\";\ncall();\n}";
    let mut s = selecting(text, &[(0, text.chars().count())]);
    assert!(s.reindent_selection(IndentLanguage::Rust));
    assert_eq!(s.text(), "fn x() {\n    let text = \"first\n  { literal\nlast\";\n    call();\n}");
    s.undo();
    assert_eq!(s.text(), text);
    let mut s = st("let text = \"first\n  {", 0);
    s.set_cursor(s.len());
    s.newline_with_language(Some(IndentLanguage::Rust));
    assert_eq!(s.text(), "let text = \"first\n  {\n  ");
}

#[test]
fn javascript_regex_does_not_create_continuation_and_division_remains_code() {
    use crate::commands::indentation::IndentLanguage;
    for language in [IndentLanguage::JavaScript, IndentLanguage::TypeScript] {
        for line in [r"const re = /\(/;", r"const re = /[(\/)]/;", "const x = value / 2;"] {
            let mut s = st(line, line.chars().count());
            s.newline_with_language(Some(language));
            assert_eq!(s.text(), format!("{line}\n"));
        }
        let text = "function f() {\nconst re = /\\(/;\nconst x = value / 2;\ncall();\n}";
        let mut s = selecting(text, &[(0, text.chars().count())]);
        assert!(s.reindent_selection(language));
        assert_eq!(s.text(), "function f() {\n    const re = /\\(/;\n    const x = value / 2;\n    call();\n}");
    }
}

#[test]
fn javascript_ambiguous_slash_declines_batch_without_history() {
    use crate::commands::indentation::IndentLanguage;
    let text = "if (ready) /[(]/.test(x);\nnext();";
    let mut s = selecting(text, &[(0, text.chars().count())]);
    assert!(!s.reindent_selection(IndentLanguage::JavaScript));
    assert_eq!(s.text(), text);
    s.undo();
    assert_eq!(s.text(), text);
}

#[test]
fn reindent_preserves_valid_outer_else_after_nested_exit() {
    use crate::commands::indentation::IndentLanguage;
    let text = "if outer:\n    if inner:\n        pass\nelse:\n    done()";
    let start = text.find("else:").unwrap();
    for (a, b) in [(0, text.len()), (start, text.len())] {
        let mut s = selecting(text, &[(a, b)]);
        assert!(s.reindent_selection(IndentLanguage::Python));
        assert_eq!(s.text(), text);
    }
}

#[test]
fn language_newline_preserves_aligned_closer_in_outer_block() {
    use crate::commands::indentation::IndentLanguage;
    for (language, text) in [
        (IndentLanguage::Python, "def f():\n    result = call(\n        value,\n    )"),
        (IndentLanguage::Rust, "fn f() {\n    if x {\n        work();\n    }"),
        (IndentLanguage::JavaScript, "function f() {\n    if (x) {\n        work();\n    }"),
        (IndentLanguage::TypeScript, "function f() {\n    if (x) {\n        work();\n    }"),
    ] {
        let mut s = st(text, text.chars().count());
        s.newline_with_language(Some(language));
        assert_eq!(s.text(), format!("{text}\n    "));
    }
}

#[test]
fn python_keyword_boundaries_exclude_combining_identifiers() {
    use crate::commands::indentation::IndentLanguage;
    for text in ["def f():\n    return\u{301} = 1\n    work()", "if x:\n    else\u{301} = 1\n    work()"] {
        let mut s = selecting(text, &[(0, text.chars().count())]);
        assert!(s.reindent_selection(IndentLanguage::Python));
        assert_eq!(s.text(), text);
        let at = text.find('\n').unwrap() + 1;
        let line_end = text[at..].find('\n').unwrap() + at;
        let mut s = st(text, text[..line_end].chars().count());
        s.newline_with_language(Some(IndentLanguage::Python));
        assert_eq!(s.text(), format!("{}\n    {}", &text[..line_end], &text[line_end..]));
    }
    let mut s = st("    return(1)", 13);
    s.newline_with_language(Some(IndentLanguage::Python));
    assert_eq!(s.text(), "    return(1)\n");
}

#[test]
fn tree_indents_enter_at_every_caret_in_the_views_unit_and_undoes_as_one() {
    // levels by where each break is: what a syntax tree's indents would say
    let levels = |at: usize, _: usize, _: bool| Some(if at < 6 { 1 } else { 3 });
    let original = "actor\r\nbe go\r\n";
    let mut s = carets(original, &[5, 12]);
    s.doc.tab_size = 4;
    s.doc.indent_size = Some(2);
    s.newline_indented(None, levels);
    assert_eq!(s.text(), "actor\n  \r\nbe go\n      \r\n");
    assert_eq!(carets_of(&s), vec![8, 22]);
    s.undo();
    assert_eq!(s.text(), original);
    let mut s = st("x", 1);
    s.doc.translate_tabs = false;
    s.doc.tab_size = 4;
    s.newline_indented(None, |_, _, _| Some(2));
    assert_eq!(s.text(), "x\n\t\t");
    // it wins over a language's own rule; without it that rule stands
    use crate::commands::indentation::IndentLanguage;
    let mut s = st("    foo {", 9);
    s.newline_indented(Some(IndentLanguage::Rust), |_, _, _| Some(0));
    assert_eq!(s.text(), "    foo {\n");
    let mut s = st("    foo {", 9);
    s.newline_indented(Some(IndentLanguage::Rust), |_, _, _| None);
    assert_eq!(s.text(), "    foo {\n        ");
}

#[test]
fn tree_indents_a_pushed_brace_and_the_selection_enter_replaces() {
    let seen = std::cell::RefCell::new(Vec::new());
    let levels = |start: usize, end: usize, blank: bool| {
        seen.borrow_mut().push((start, end, blank));
        Some(if blank { 2 } else { 1 })
    };
    let mut s = st("  f {}", 5);
    s.newline_indented(None, levels);
    assert_eq!(s.text(), "  f {\n        \n    }");
    assert_eq!(s.cursor(), 14);
    assert_eq!(seen.take(), [(5, 5, true), (5, 5, false)]);
    let mut s = selecting("ab cd", &[(1, 4)]);
    s.newline_indented(None, levels);
    assert_eq!(s.text(), "a\n    d");
    assert_eq!(seen.take(), [(1, 4, false)]);
}
