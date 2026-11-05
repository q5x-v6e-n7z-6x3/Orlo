//! Enter in a language whose folder has `indents.scm` (the bundled Pony), by its current tree;
//! other languages keep their rules.
use super::*;
use gpui::{TestAppContext, VisualTestContext};
use std::path::Path;

fn parsed(e: &mut Editor, cx: &mut Context<Editor>) {
    let rope = e.doc(cx).rope.clone();
    let job = highlight::run_job(e.highlighter.job(0), &rope, &[0, rope.len_lines() - 1]);
    e.highlighter.adopt(job, &rope);
}

/// An editor of `text` as the file `path`, indenting by two spaces, its carets at each `|`.
fn editor<'a>(text: &str, path: &str, cx: &'a mut TestAppContext) -> (Entity<Editor>, &'a mut VisualTestContext) {
    let carets = text.split('|').map(|piece| piece.chars().count()).scan(0, |at, n| {
        *at += n;
        Some(*at)
    });
    let carets: Vec<usize> = carets.collect::<Vec<_>>().split_last().unwrap().1.to_vec();
    let (e, cx) = focused_editor(&text.replace('|', ""), Some(Path::new(path)), cx);
    e.update_in(cx, |e, _, cx| {
        e.update_doc(cx, |doc| {
            (doc.tab_size, doc.indent_size, doc.translate_tabs) = (2, None, true);
        });
        e.state.selections = carets.into_iter().map(Selection::caret).collect();
    });
    (e, cx)
}

#[gpui::test]
fn enter_indents_pony_by_its_tree_at_every_caret_and_undoes(cx: &mut TestAppContext) {
    let text = "actor Main\n  new create(env: Env) =>|\n    None\n\n  be go() =>\n    while true do\n      None\n    end|\n    None\n";
    let (e, cx) = editor(text, "/project/main.pony", cx);
    e.update_in(cx, |e, _, cx| {
        assert_eq!(e.highlighter.syntax_name(), "Pony");
        parsed(e, cx);
    });
    run(&e, cx, &Newline);
    let entered = "actor Main\n  new create(env: Env) =>\n    \n    None\n\n  be go() =>\n    while true do\n      None\n    end\n    \n    None\n";
    assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), entered);
    run(&e, cx, &Undo);
    assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), text.replace('|', ""));
}

#[gpui::test]
fn enter_in_pony_keeps_the_line_indent_until_the_tree_is_current(cx: &mut TestAppContext) {
    let (e, cx) = editor("actor Main\n  be go() =>\n    while true do|\n    end\n", "/project/main.pony", cx);
    e.update_in(cx, |e, window, cx| {
        parsed(e, cx);
        // typed and not parsed yet: the line's own indent, as in a language without indents.scm
        e.edit_text(cx, |s| {
            s.insert(" ");
            s.backspace();
        });
        assert!(e.perform_action(&Newline, window, cx));
        assert_eq!(e.text(cx), "actor Main\n  be go() =>\n    while true do\n    \n    end\n");
        parsed(e, cx);
        assert!(e.perform_action(&Newline, window, cx));
        assert_eq!(e.text(cx), "actor Main\n  be go() =>\n    while true do\n    \n      \n    end\n");
        // a block still being written at the end of the file is an error the tree places nowhere
        e.set_text("actor Main\n  new create(env: Env) =>", cx);
        e.state.selections = vec![Selection::caret(e.doc(cx).rope.len_chars())];
        parsed(e, cx);
        assert!(e.perform_action(&Newline, window, cx));
        assert_eq!(e.text(cx), "actor Main\n  new create(env: Env) =>\n  ");
    });
}

#[gpui::test]
fn enter_in_builtin_languages_keeps_their_rules(cx: &mut TestAppContext) {
    for (path, text, entered) in [
        ("/project/main.rs", "fn main() {|}\n", "fn main() {\n  \n}\n"),
        ("/project/main.py", "def f():|\n", "def f():\n  \n"),
        ("/project/main.go", "func f() {|\n", "func f() {\n  \n"),
    ] {
        let (e, cx) = editor(text, path, cx);
        e.update_in(cx, |e, window, cx| {
            parsed(e, cx);
            assert!(e.highlighter.indents(&e.doc(cx).rope).is_none(), "{path}: no indents.scm");
            assert!(e.perform_action(&Newline, window, cx));
            assert_eq!(e.text(cx), entered, "{path}");
        });
    }
}
