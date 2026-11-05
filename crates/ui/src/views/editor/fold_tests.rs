//! Folds in a view: the commands, the gutter's buttons, the marker, reveal and drawing.
#![allow(clippy::single_range_in_vec_init)]

use super::*;
use crate::editing::text::Motion;
use gpui::{point, size, Entity, Modifiers, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

fn folds(s: &crate::editing::text::Document, header: usize, last: usize) -> Folds {
    let mut f = Folds::default();
    f.fold_lines(s, header, last);
    f
}

/// An editor alone in a 600 × 300 window, unwrapped, drawn once; gpui's test text system lays
/// text out monospace.
fn editor<'a>(text: &str, cx: &'a mut TestAppContext) -> (Entity<Editor>, &'a mut VisualTestContext) {
    test_editor(text, cx, |_, _| {})
}

/// Fold `chars` in place of the view's folds, then a frame.
fn fold_chars<const N: usize>(e: &Entity<Editor>, cx: &mut VisualTestContext, chars: [Range<usize>; N]) {
    e.update(cx, |e, cx| {
        e.fold_chars_now(chars, cx);
        cx.notify();
    });
    cx.run_until_parked();
}

/// What `FoldLevel 1` puts on the status bar meanwhile, as it is said.
fn statuses(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (Rc<RefCell<Vec<String>>>, gpui::Subscription) {
    let said = Rc::new(RefCell::new(Vec::new()));
    let sink = said.clone();
    let sub = cx.update(|_, cx| {
        cx.subscribe(e, move |_, event: &EditorEvent, _| {
            if let EditorEvent::Status(s) = event {
                sink.borrow_mut().push(s.clone());
            }
        })
    });
    (said, sub)
}

/// Fold Level 1, as its key runs it.
fn level_1(e: &Entity<Editor>, cx: &mut VisualTestContext) {
    e.update_in(cx, |e, window, cx| e.perform_action(&FoldLevel { level: 1 }, window, cx));
}

/// The lines this view's folds hide.
fn hidden(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<Range<usize>> {
    e.read_with(cx, |e, cx| e.folds.hidden_lines(e.doc(cx)))
}

fn caret(e: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    e.read_with(cx, |e, _| e.state.cursor())
}

/// Where row `row` of the drawn frame is, `x` across.
fn at_row(e: &Entity<Editor>, cx: &mut VisualTestContext, x: Pixels, row: f32) -> Point<Pixels> {
    e.read_with(cx, |e, _| {
        let top = e.layout.as_ref().unwrap().bounds.top();
        point(x, top + px(e.line_height() * (row + 0.5)))
    })
}

/// (the fold buttons' x, the text's left edge, a space's width)
fn columns(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (Pixels, Pixels, Pixels) {
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        // centred 10 left of the gutter's edge, which is the text margin left of the text
        let button = l.text_left - px(theme::TEXT_MARGIN + 10.);
        (button, l.text_left, l.space_w)
    })
}

/// Complete the normal worker job deterministically, retaining the explicit syntax choice.
fn parse_syntax(e: &mut Editor, cx: &mut Context<Editor>) {
    let rope = e.doc(cx).rope.clone();
    let job = highlight::run_job(e.highlighter.job(0), &rope, &[0, rope.len_lines() - 1]);
    e.highlighter.adopt(job, &rope);
    assert!(e.highlighter.fold_regions(&rope).is_some());
}

#[gpui::test]
fn syntax_folds_commands_gutter_nested_levels_and_trailing_code(cx: &mut TestAppContext) {
    let text = "fn café() {\nif true {\n雪();\n} next();\n} tail();\n";
    let (e, cx) = test_editor(text, cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Rust");
        e.view.fade_fold_buttons = false;
        parse_syntax(e, cx);
    });
    e.read_with(cx, |e, _| {
        assert_eq!(e.layout.as_ref().unwrap().drawn.fold_buttons, [(0, false), (1, false)]);
        assert_eq!(e.highlighter.language_override(), Some("Rust"));
    });
    let inner = e.read_with(cx, |e, cx| e.fold_region(e.doc(cx), 1).unwrap());
    let outer = e.read_with(cx, |e, cx| e.fold_region(e.doc(cx), 0).unwrap());
    let (button, _, _) = columns(&e, cx);
    let at = at_row(&e, cx, button, 1.);
    cx.simulate_click(at, Modifiers::none());
    assert!(drawn_rows(&e, cx).0.iter().any(|row| row.ends_with(" next();")));
    e.read_with(cx, |e, cx| {
        assert_eq!(e.folds.chars(), vec![inner.clone()]);
        assert_eq!(e.doc(cx).rope.slice(inner.end..e.doc(cx).line_end(3)).to_string(), " next();");
    });
    cx.simulate_click(at, Modifiers::none());
    assert!(hidden(&e, cx).is_empty());
    e.update(cx, |e, cx| {
        let at = e.doc(cx).line_start(2);
        e.sel(cx).set_cursor(at);
    });
    run(&e, cx, &Fold);
    e.read_with(cx, |e, _| assert_eq!(e.folds.chars(), vec![inner.clone()]));
    run(&e, cx, &FoldLevel { level: 1 });
    e.read_with(cx, |e, cx| {
        assert_eq!(e.folds.chars(), vec![outer.clone()]);
        assert_eq!(e.doc(cx).rope.slice(outer.end..).to_string(), " tail();\n");
    });
    run(&e, cx, &FoldLevel { level: 2 });
    e.read_with(cx, |e, _| assert_eq!(e.folds.chars(), vec![inner]));
}

#[gpui::test]
fn syntax_folds_preserve_manual_ranges_read_only_and_fallback(cx: &mut TestAppContext) {
    let text = "/* café\n雪\n*/ suffix\n";
    let (e, cx) = test_editor(text, cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Rust");
        parse_syntax(e, cx);
        e.document.update(cx, |d, _| d.model.read_only = true);
    });
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(0));
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [1..3]);
    run(&e, cx, &UnfoldAll);
    e.update(cx, |e, cx| e.sel(cx).set_selection(3, 7));
    run(&e, cx, &Fold);
    e.read_with(cx, |e, cx| {
        assert_eq!(e.folds.chars(), vec![3..7]);
        assert_eq!(e.doc(cx).version, version);
        assert_eq!(e.doc(cx).rope.to_string(), text);
    });
    run(&e, cx, &UnfoldAll);
    e.update(cx, |e, cx| {
        let end = e.doc(cx).line_start(2);
        e.sel(cx).set_selection(0, end);
    });
    run(&e, cx, &Fold);
    e.read_with(cx, |e, cx| assert_eq!(e.folds.chars(), vec![e.doc(cx).line_end(0)..e.doc(cx).line_end(1)]));
    e.update(cx, |e, cx| {
        e.set_text("header\n  body\ntail\n", cx);
        e.highlighter = Highlighter::by_name("Plain Text");
        e.sel(cx).set_cursor(0);
    });
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [1..2]);
}

#[gpui::test]
fn syntax_folds_stale_worker_shared_edits_undo_redo_and_pending_fallback(cx: &mut TestAppContext) {
    let text = "fn café() {\n雪();\n}\n";
    let (e, cx) = test_editor(text, cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Rust");
        parse_syntax(e, cx);
    });
    let b = e.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
    let old = e.update(cx, |e, cx| {
        let rope = e.doc(cx).rope.clone();
        highlight::run_job(e.highlighter.job(0), &rope, &[0, 2])
    });
    e.update(cx, |e, cx| e.sel(cx).set_cursor(0));
    run(&e, cx, &Fold);
    b.update(cx, |b, cx| {
        b.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("// 🦀\n");
        })
    });
    cx.run_until_parked();
    e.update(cx, |e, cx| {
        let rope = e.doc(cx).rope.clone();
        // Invalidate again to model a result arriving before the current worker finishes.
        e.highlighter.invalidate_from(0);
        e.highlighter.adopt(old, &rope);
        assert!(e.highlighter.fold_regions(&rope).is_none());
        assert!(e.fold_region(e.doc(cx), 0).is_none());
        parse_syntax(e, cx);
        assert!(e.fold_region(e.doc(cx), 1).is_some());
    });
    b.read_with(cx, |b, _| assert!(b.folds.is_empty(), "folds are view-local"));
    run(&e, cx, &Undo);
    e.update(cx, |e, cx| {
        parse_syntax(e, cx);
        assert!(e.fold_region(e.doc(cx), 0).is_some());
    });
    run(&e, cx, &Redo);
    e.update(cx, |e, cx| {
        parse_syntax(e, cx);
        assert!(e.fold_region(e.doc(cx), 1).is_some());
    });
    e.update_in(cx, |e, window, cx| {
        e.set_text("pending\n  body\ntail\n", cx);
        e.highlighter = Highlighter::by_name("Rust");
        assert!(e.highlighter.fold_regions(&e.doc(cx).rope).is_none());
        e.sel(cx).set_cursor(0);
        e.perform_action(&Fold, window, cx);
        assert_eq!(e.folds.hidden_lines(e.doc(cx)), [1..2]);
    });
}

#[gpui::test]
fn edits_here_and_in_another_view_map_each_views_own_folds(cx: &mut TestAppContext) {
    let text = "head\n  one\n  two\ntail\n  three\nend\n";
    let (a, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(text), false, cx));
    let b = a.update(cx, |a, cx| cx.new(|cx| a.another_view(cx)));
    a.update(cx, |a, cx| a.set_folds_now(folds(a.doc(cx), 0, 2), cx));
    b.update(cx, |b, cx| b.set_folds_now(folds(b.doc(cx), 3, 4), cx));
    // typed above both: each view's fold moves with its text
    a.update(cx, |a, cx| {
        a.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("x");
        });
        assert_eq!(a.folds, folds(a.doc(cx), 0, 2));
    });
    // the other view applies the document's change when it is delivered
    let follow = |cx: &mut gpui::VisualTestContext| cx.run_until_parked();
    follow(cx);
    b.read_with(cx, |b, cx| assert_eq!(b.folds, folds(b.doc(cx), 3, 4)));
    // Down from the header steps over this view's fold
    a.update_in(cx, |a, window, cx| {
        a.sel(cx).set_cursor(1);
        a.r#move(&Move::to(Motion::Down, false), window, cx);
        assert_eq!(a.state.cursor(), a.doc(cx).line_start(3) + 1);
    });
    // typed inside the other view's fold: it opens there, and this one stays
    a.update(cx, |a, cx| {
        let at = a.doc(cx).line_start(4) + 2;
        a.edit(cx, |s| {
            s.set_cursor(at);
            s.insert("y");
        });
        assert_eq!(a.folds, folds(a.doc(cx), 0, 2));
    });
    follow(cx);
    b.read_with(cx, |b, _| assert_eq!(b.folds, Folds::default()));
    // new text drops every fold; Find Results streaming in after a fold that ends the text
    // goes after it, and the fold stays (text typed at a fold's end is never hidden)
    a.update(cx, |a, cx| {
        a.set_text("head\n  one", cx);
        assert_eq!(a.folds, Folds::default());
        a.set_folds_now(folds(a.doc(cx), 0, 1), cx);
        a.append("\nmore", std::iter::empty(), cx);
        assert_eq!(a.folds, folds(a.doc(cx), 0, 1));
        assert_eq!(a.hidden_lines(cx), [1..2]);
    });
}

#[gpui::test]
fn fold_unfold_fold_level_and_unfold_all(cx: &mut TestAppContext) {
    // regions: fn a 0..=4 holding the if at 2..=3, fn b 6..=7
    let text = "fn a() {\n    let x = 1;\n    if x {\n        y();\n    }\n}\nfn b() {\n    z();\n}\n";
    let (e, cx) = editor(text, cx);
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    let to = |e: &Entity<Editor>, cx: &mut VisualTestContext, line: usize, col: usize| {
        e.update(cx, |e, cx| {
            let at = e.doc(cx).line_start(line) + col;
            e.sel(cx).set_cursor(at)
        })
    };
    // a caret: the innermost region around it, and the caret goes to the header's end
    to(&e, cx, 3, 8);
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [3..4]);
    e.read_with(cx, |e, cx| assert_eq!(e.state.cursor(), e.doc(cx).line_end(2)));
    // on the header line: the region it heads, folded already
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [3..4]);
    to(&e, cx, 2, 0);
    run(&e, cx, &Unfold);
    assert!(hidden(&e, cx).is_empty(), "the folds on the selections' lines");
    // a multi-line selection: its lines after the first; ending at a line's start, the line
    // before is its last
    e.update(cx, |e, cx| {
        let (a, b) = (e.doc(cx).line_start(6), e.doc(cx).line_start(9));
        e.sel(cx).set_selection(a, b)
    });
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [7..9]);
    e.read_with(cx, |e, cx| {
        let s = e.read_state(cx);
        assert_eq!(s.view.selections[0].range(), s.line_start(6)..s.line_start(9));
    });
    // ending at the end of its last line, the fold's end: the selection keeps all it had
    run(&e, cx, &UnfoldAll);
    let fn_b = e.update(cx, |e, cx| {
        let (a, b) = (e.doc(cx).line_start(6), e.doc(cx).line_end(8));
        e.sel(cx).set_selection(a, b);
        a..b
    });
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [7..9]);
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0].range(), fn_b));
    run(&e, cx, &Copy);
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("fn b() {\n    z();\n}"));
    // Fold Level N: the regions headed at indent level N - 1, in place of the folds there were
    run(&e, cx, &FoldLevel { level: 1 });
    assert_eq!(hidden(&e, cx), [1..5, 7..8]);
    run(&e, cx, &FoldLevel { level: 2 });
    assert_eq!(hidden(&e, cx), [3..4]);
    // Unfold at two carets opens both their folds
    run(&e, cx, &FoldLevel { level: 1 });
    e.update(cx, |e, cx| {
        let s = e.read_state(cx);
        let sels = vec![Selection::caret(s.line_end(0)), Selection::caret(s.line_start(6))];
        e.sel(cx).set_selections(sels)
    });
    run(&e, cx, &Unfold);
    assert!(hidden(&e, cx).is_empty());
    run(&e, cx, &FoldLevel { level: 2 });
    run(&e, cx, &UnfoldAll);
    assert!(hidden(&e, cx).is_empty());
    // folding is not an edit
    e.read_with(cx, |e, cx| {
        assert_eq!((e.doc(cx).text(), e.doc(cx).version), (text.to_string(), version));
    });
}

#[gpui::test]
fn fold_at_thousands_of_carets_reads_each_line_once(cx: &mut TestAppContext) {
    // 2,000 carets down an 8,000-line block, a selection over lines 7999-8001 and a caret in
    // the block at its end (8001-8002): scanning out from each caret would read the block
    // once a caret
    let body = "    x();\n".repeat(8000);
    let text = format!("fn a() {{\n{body}    if x {{\n        y();\n    }}\n}}\n");
    let (e, cx) = editor(&text, cx);
    let read = e.update_in(cx, |e, window, cx| {
        let s = e.read_state(cx);
        let at = |l: usize| s.line_start(l) + 2;
        let mut sels: Vec<Selection> = (1..8000).step_by(4).map(|l| Selection::caret(at(l))).collect();
        sels.push(Selection::new(s.line_start(7999), at(8001)));
        sels.push(Selection::caret(at(8002)));
        e.sel(cx).set_selections(sels);
        crate::editing::fold::MEASURED.set(0);
        assert!(e.perform_action(&Fold, window, cx));
        crate::editing::fold::MEASURED.get()
    });
    let lines = e.read_with(cx, |e, cx| {
        // what a scan out from each caret finds
        let mut want = Folds::default();
        for (header, last) in [(0, 8003), (7999, 8001), (8001, 8002)] {
            want.fold_lines(e.doc(cx), header, last);
        }
        assert_eq!(e.folds, want);
        e.doc(cx).line_count()
    });
    assert!(read <= lines, "{read} lines read, of {lines}");
}

#[gpui::test]
fn a_selection_over_a_fold_keeps_it_and_copy_and_cut_take_the_hidden_text(cx: &mut TestAppContext) {
    // "head" hides 4..16; "tail" starts at 17
    let text = "head\n  one\n  two\ntail\n";
    let (e, cx) = editor(text, cx);
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [1..3]);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(2));
    run(&e, cx, &Move::to(Motion::Down, true));
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0].range(), 2..19));
    assert_eq!(hidden(&e, cx), [1..3], "still folded, a frame later");
    let clipboard = |cx: &mut VisualTestContext| cx.read_from_clipboard().and_then(|c| c.text());
    run(&e, cx, &Copy);
    assert_eq!(clipboard(cx).as_deref(), Some("ad\n  one\n  two\nta"));
    run(&e, cx, &Cut);
    assert_eq!(clipboard(cx).as_deref(), Some("ad\n  one\n  two\nta"));
    e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).text(), "heil\n"));
    run(&e, cx, &Undo);
    e.read_with(cx, |e, cx| {
        assert_eq!(e.doc(cx).text(), text);
        assert_eq!(e.state.selections[0].range(), 2..19);
    });
    run(&e, cx, &Redo);
    e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).text(), "heil\n"));
}

#[gpui::test]
fn undo_and_redo_that_put_a_caret_in_a_fold_open_it(cx: &mut TestAppContext) {
    // "a" heads lines 1..=2, hidden as 1..9; offset 5 is the end of "  b"; "d" is at 10
    let (e, cx) = editor("a\n  b\n  c\nd\n", cx);
    // an edit below the block, made with the caret in the block
    let edit = |e: &Entity<Editor>, cx: &mut VisualTestContext, with: &'static str| {
        e.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(5);
                s.replace_ranges(vec![(10..11, with.to_string())]);
            })
        })
    };
    let state =
        |e: &Entity<Editor>, cx: &mut VisualTestContext| e.read_with(cx, |e, cx| (e.doc(cx).text(), e.state.cursor()));
    // Undo leaves the caret in the block, Fold takes it to the header, Redo brings it back
    edit(&e, cx, "D");
    run(&e, cx, &Undo);
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [1..3]);
    assert_eq!(caret(&e, cx), 1);
    run(&e, cx, &Redo);
    assert_eq!(state(&e, cx), ("a\n  b\n  c\nD\n".to_string(), 5));
    assert!(hidden(&e, cx).is_empty(), "the redo's caret opened the fold");
    // the edit's own caret was in the block, folded since: Undo brings it back
    edit(&e, cx, "d");
    e.update(cx, |e, cx| e.sel(cx).set_cursor(0));
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [1..3]);
    run(&e, cx, &Undo);
    assert_eq!(state(&e, cx), ("a\n  b\n  c\nD\n".to_string(), 5));
    assert!(hidden(&e, cx).is_empty(), "the undo's caret opened the fold");
}

#[gpui::test]
fn the_minimap_and_the_scroll_range_count_the_rows_shown(cx: &mut TestAppContext) {
    // 301 lines with the empty last one; the minimap fits 150 rows of 2 pt
    let (e, cx) = editor(&"line\n".repeat(300), cx);
    let minimap_rows = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
        e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.minimap_rows)
    };
    assert_eq!(minimap_rows(&e, cx), 151);
    e.update(cx, |e, cx| {
        e.fold_lines_now(0, 250, cx);
        cx.notify()
    });
    cx.run_until_parked();
    // the header, lines 251..=299 and the empty last line
    assert_eq!(minimap_rows(&e, cx), 51);
    e.read_with(cx, |e, cx| assert_eq!(e.max_scroll(cx), 51. * 15. - 300.));
}

#[gpui::test]
fn composing_at_a_folded_headers_end_keeps_the_fold_and_the_ime_on_its_row(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    // "a" and "c" head folds: the rows show a, c, e and the empty last line
    let (e, cx) = editor("a\n  b\nc\n  d\ne\n", cx);
    e.update(cx, |e, cx| {
        let mut f = folds(e.doc(cx), 0, 1);
        f.fold_lines(e.doc(cx), 2, 3);
        e.set_folds_now(f, cx);
        let at = e.doc(cx).line_end(2);
        e.sel(cx).set_cursor(at);
        cx.notify();
    });
    cx.run_until_parked();
    let (bounds, lh) = e.read_with(cx, |e, _| (e.layout.as_ref().unwrap().bounds, e.line_height()));
    let ime = e.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, "k", None, window, cx);
        let marked = e.marked_text_range(window, cx).unwrap();
        e.bounds_for_range(marked, bounds, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(ime.map(|b| b.origin.y), Some(bounds.top() + px(lh)));
    assert_eq!(hidden(&e, cx), [1..2, 3..4], "typed before the hidden text");
    e.read_with(cx, |e, cx| {
        assert_eq!(e.doc(cx).text(), "a\n  b\nck\n  d\ne\n");
        let rows = &e.layout.as_ref().unwrap().rows;
        assert_eq!(
            rows[1].as_ref().unwrap().1.text.as_ref(),
            "ck   ",
            "the marked text on the header's row, then its placeholder's blank columns"
        );
    });
}

#[gpui::test]
fn the_fold_buttons_and_the_marker_toggle_folds_without_moving_the_caret(cx: &mut TestAppContext) {
    // "head" heads lines 1..=2; "tail" heads nothing
    let (e, cx) = editor("head\n  one\n  two\ntail\n", cx);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(19));
    let (button, text_left, space_w) = columns(&e, cx);
    let head = at_row(&e, cx, button, 0.);
    cx.simulate_click(head, Modifiers::none());
    assert_eq!(hidden(&e, cx), [1..3]);
    assert_eq!(caret(&e, cx), 19, "the click did not move the caret");
    cx.simulate_click(head, Modifiers::none());
    assert!(hidden(&e, cx).is_empty());
    // the marker drawn after a folded header's text unfolds it too
    cx.simulate_click(head, Modifiers::none());
    assert_eq!(hidden(&e, cx), [1..3]);
    let marker = at_row(&e, cx, text_left + space_w * 4. + px(8.), 0.);
    cx.simulate_click(marker, Modifiers::none());
    assert!(hidden(&e, cx).is_empty());
    assert_eq!(caret(&e, cx), 19);
    // a line that heads nothing has no button: a click there puts the caret on it as usual
    let plain = at_row(&e, cx, button, 1.);
    cx.simulate_click(plain, Modifiers::none());
    assert_eq!(caret(&e, cx), 5);
}

#[gpui::test]
fn fold_buttons_show_folded_lines_and_wait_for_the_pointer_over_the_gutter(cx: &mut TestAppContext) {
    // "head" heads a region, "tail" none, "x" is folded
    let (e, cx) = editor("head\n  one\ntail\nx\n  y\n", cx);
    e.update(cx, |e, cx| {
        e.fold_lines_now(3, 4, cx);
        cx.notify()
    });
    cx.run_until_parked();
    let buttons = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
        e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.fold_buttons.clone())
    };
    assert_eq!(buttons(&e, cx), [(3, true)], "▸ always, ▾ under the pointer only");
    let (button, text_left, _) = columns(&e, cx);
    let gutter = at_row(&e, cx, button - px(20.), 2.);
    cx.simulate_mouse_move(gutter, None, Modifiers::none());
    assert_eq!(buttons(&e, cx), [(0, false), (3, true)]);
    let text = at_row(&e, cx, text_left + px(30.), 2.);
    cx.simulate_mouse_move(text, None, Modifiers::none());
    assert_eq!(buttons(&e, cx), [(3, true)]);
    e.update(cx, |e, cx| {
        e.view.fade_fold_buttons = false;
        cx.notify()
    });
    cx.run_until_parked();
    assert_eq!(buttons(&e, cx), [(0, false), (3, true)]);
    // fold_buttons off: none drawn, and a click there folds nothing
    e.update(cx, |e, cx| {
        e.view.fold_buttons = false;
        cx.notify()
    });
    cx.run_until_parked();
    assert!(buttons(&e, cx).is_empty());
    let head = at_row(&e, cx, button, 0.);
    cx.simulate_click(head, Modifiers::none());
    assert_eq!(hidden(&e, cx), [4..5]);
}

#[gpui::test]
fn line_selections_on_a_folded_header_take_in_the_fold(cx: &mut TestAppContext) {
    // "head" hides 4..16; "tail" is line 3, at 17..21
    let (e, cx) = editor("head\n  one\n  two\ntail\n", cx);
    e.update(cx, |e, cx| {
        e.fold_lines_now(0, 2, cx);
        e.sel(cx).set_cursor(2);
        cx.notify()
    });
    cx.run_until_parked();
    let selection = |e: &Entity<Editor>, cx: &mut VisualTestContext| e.read_with(cx, |e, _| e.state.selections.clone());
    run(&e, cx, &SelectLine);
    assert_eq!(selection(&e, cx), [Selection::new(0, 17)]);
    assert_eq!(hidden(&e, cx), [1..3]);
    // a triple click on the header's row
    let (_, text_left, _) = columns(&e, cx);
    let at = at_row(&e, cx, text_left + px(2.), 0.);
    let m = Modifiers::none();
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: m,
        button: MouseButton::Left,
        click_count: 3,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent { position: at, modifiers: m, button: MouseButton::Left, click_count: 3 });
    assert_eq!(selection(&e, cx), [Selection::new(0, 17)]);
    assert_eq!(hidden(&e, cx), [1..3]);
    // upward from the line below, it grows over the fold to its header
    e.update(cx, |e, cx| e.sel(cx).set_cursor(17));
    run(&e, cx, &SelectLineUpward);
    run(&e, cx, &SelectLineUpward);
    assert_eq!(selection(&e, cx), [Selection::new(22, 0)]);
    assert_eq!(hidden(&e, cx), [1..3]);
}

#[gpui::test]
fn a_caret_at_a_folds_end_is_drawn_after_its_marker(cx: &mut TestAppContext) {
    // a fold to the end of the text, where Cmd-Down puts the caret
    let (e, cx) = editor("head\n  one\n  two", cx);
    e.update(cx, |e, cx| {
        e.fold_lines_now(0, 2, cx);
        cx.notify()
    });
    cx.run_until_parked();
    run(&e, cx, &Move::to(Motion::DocEnd, false));
    assert_eq!(hidden(&e, cx), [1..3]);
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.cursor(), e.doc(cx).len());
        let l = e.layout.as_ref().unwrap();
        let carets = &l.drawn.carets;
        assert_eq!(carets.len(), 1, "{carets:?}");
        // on the header's row, past "head" and the marker after it
        assert!(carets[0].left() > l.text_left + l.space_w * 4. + px(12.), "{carets:?}");
        assert!(carets[0].top() < l.bounds.top() + px(e.line_height()), "{carets:?}");
    });
}

#[gpui::test]
fn fold_level_on_a_huge_buffer_finds_its_regions_off_the_ui_thread(cx: &mut TestAppContext) {
    // over 4M chars in 4,003 lines: one region, then long flat lines
    let text = format!("a\n  b\n{}", format!("{}\n", "x".repeat(999)).repeat(4_001));
    assert!(text.chars().count() > crate::editing::stats::BIG);
    let (e, cx) = editor(&text, cx);
    let (said, _sub) = statuses(&e, cx);
    level_1(&e, cx);
    assert!(hidden(&e, cx).is_empty(), "nothing found on this thread");
    cx.run_until_parked();
    assert_eq!(hidden(&e, cx), [1..2]);
    assert_eq!(*said.borrow(), ["Working…", "Done"]);
    // the text changed meanwhile: the regions found are another text's
    run(&e, cx, &UnfoldAll);
    level_1(&e, cx);
    e.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("z");
        })
    });
    cx.run_until_parked();
    assert!(hidden(&e, cx).is_empty());
    assert_eq!(said.borrow().last().map(String::as_str), Some("The document changed; run the command again"));
}

#[gpui::test]
fn a_reload_keeps_the_folds_outside_the_text_it_changed(cx: &mut TestAppContext) {
    let (e, cx) = editor("a\n  b\nc\n  d\n", cx);
    e.update(cx, |e, cx| {
        let mut f = folds(e.doc(cx), 0, 1);
        f.fold_lines(e.doc(cx), 2, 3);
        e.set_folds_now(f, cx);
        let loaded = Loaded {
            endings: None,
            encoding: Default::default(),
            rope: ropey::Rope::from_str("a\n  b\nc\n  D\n"),
            line_ending: "\n",
            read_only: false,
            disk: None,
            bom: false,
        };
        let changes = loaded.diff(&e.doc(cx).rope);
        e.reload(loaded, &changes, cx);
    });
    assert_eq!(hidden(&e, cx), [1..2], "the fold over the changed line opened");
}

#[gpui::test]
fn a_whole_line_selection_folds_and_unfolds_by_its_own_line(cx: &mut TestAppContext) {
    // "a:" heads line 1, "c:" heads line 3; offset 4 is in "  b"
    let (e, cx) = editor("a:\n  b\nc:\n  d\n", cx);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(4));
    run(&e, cx, &SelectLine);
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [1..2], "the region around line 1, not the one line 2 heads");
    run(&e, cx, &UnfoldAll);
    e.update(cx, |e, cx| {
        e.fold_lines_now(2, 3, cx);
        e.sel(cx).set_cursor(4);
        cx.notify()
    });
    run(&e, cx, &SelectLine);
    run(&e, cx, &Unfold);
    assert_eq!(hidden(&e, cx), [3..4], "the fold the next line heads stays");
}

#[gpui::test]
fn a_fold_command_supersedes_a_fold_level_still_working(cx: &mut TestAppContext) {
    // over 4M chars: "a" heads lines 1..=2 and "  b" line 2, then long flat lines
    let text = format!("a\n  b\n    c\n{}", format!("{}\n", "x".repeat(999)).repeat(4_001));
    let (e, cx) = editor(&text, cx);
    let (said, _sub) = statuses(&e, cx);
    // Fold at a caret in "  b" while Fold All works: the Fold's fold stays
    level_1(&e, cx);
    e.update_in(cx, |e, window, cx| {
        let at = e.doc(cx).line_start(1);
        e.sel(cx).set_cursor(at);
        assert!(e.perform_action(&Fold, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(hidden(&e, cx), [2..3], "not Fold All's 1..3");
    // and Unfold All
    level_1(&e, cx);
    run(&e, cx, &UnfoldAll);
    assert!(hidden(&e, cx).is_empty());
    assert_eq!(*said.borrow(), ["Working…", "Working…"], "no late Done or warning");
}

/// The text each row of the last frame shaped (a placeholder: its blank columns), and the
/// markers drawn.
fn drawn_rows(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (Vec<String>, usize) {
    e.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        let rows = (l.rows.iter().flatten()).map(|r| r.1.text.to_string());
        (rows.collect(), l.drawn.markers.len())
    })
}

fn chars(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<Range<usize>> {
    e.read_with(cx, |e, _| e.folds.chars())
}

#[gpui::test]
fn fold_on_a_selection_inside_a_line_hides_just_that_text(cx: &mut TestAppContext) {
    // "let x = compute(a, b);": the arguments are 16..20
    let text = "let x = compute(a, b);\nnext\n";
    let (e, cx) = editor(text, cx);
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    e.update(cx, |e, cx| e.sel(cx).set_selection(16, 20));
    run(&e, cx, &Fold);
    assert_eq!(chars(&e, cx), [16..20]);
    assert!(hidden(&e, cx).is_empty(), "no line is hidden");
    let (rows, markers) = drawn_rows(&e, cx);
    assert_eq!(rows[0], "let x = compute(   );", "three blank columns under the marker");
    assert_eq!(markers, 1);
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.selections[0].range(), 16..20, "the selection keeps the fold");
        assert_eq!((e.text(cx), e.doc(cx).version), (text.to_string(), version));
    });
    // a click on the marker opens it, and the caret stays where it is
    e.update(cx, |e, cx| e.sel(cx).set_cursor(25));
    let (_, text_left, space_w) = columns(&e, cx);
    let marker = at_row(&e, cx, text_left + space_w * 17.5, 0.);
    cx.simulate_click(marker, Modifiers::none());
    assert!(chars(&e, cx).is_empty());
    assert_eq!(caret(&e, cx), 25);
    // Fold on a selection over two lines still folds the lines after the first
    e.update(cx, |e, cx| e.sel(cx).set_selection(2, 26));
    run(&e, cx, &Fold);
    assert_eq!(hidden(&e, cx), [1..2]);
}

#[gpui::test]
fn a_fold_across_line_breaks_draws_its_last_lines_rest_after_the_placeholder(cx: &mut TestAppContext) {
    // "fn a() {" 0..8, "    body" 9..17, "}" 18, "next" 20..24: the body, brace to brace
    let (e, cx) = editor("fn a() {\n    body\n}\nnext\n", cx);
    fold_chars(&e, cx, [8..18]);
    let (rows, markers) = drawn_rows(&e, cx);
    assert_eq!(rows, ["fn a() {   }", "next", ""]);
    assert_eq!(markers, 1);
    e.read_with(cx, |e, _| {
        let numbers = &e.layout.as_ref().unwrap().drawn.numbers;
        assert_eq!(numbers, &[(0, 1), (1, 4), (2, 5)], "a display line shows its first line's");
    });
    // Down from "fn" goes to "next"; Right over the placeholder, to the closing brace
    e.update(cx, |e, cx| e.sel(cx).set_cursor(1));
    run(&e, cx, &Move::to(Motion::Down, false));
    assert_eq!(caret(&e, cx), 21);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(8));
    run(&e, cx, &Move::to(Motion::Right, false));
    assert_eq!(caret(&e, cx), 18);
    assert_eq!(hidden(&e, cx), [1..3], "a motion never opens it");
}

#[gpui::test]
fn nested_and_touching_folds_draw_a_marker_for_each_one_shown(cx: &mut TestAppContext) {
    let (e, cx) = editor("abcdefghijklmnop\n", cx);
    fold_chars(&e, cx, [2..10, 4..6, 10..12]);
    let (rows, markers) = drawn_rows(&e, cx);
    assert_eq!(rows[0], "ab      mnop", "the outer fold and the one touching it");
    assert_eq!(markers, 2);
    // opening the outer one shows the one inside it
    let (_, text_left, space_w) = columns(&e, cx);
    let outer = at_row(&e, cx, text_left + space_w * 3.5, 0.);
    cx.simulate_click(outer, Modifiers::none());
    assert_eq!(chars(&e, cx), [4..6, 10..12]);
    let (rows, markers) = drawn_rows(&e, cx);
    assert_eq!(rows[0], "abcd   ghij   mnop", "4..6 shows; 6..10 is ghij");
    assert_eq!(markers, 2);
}

#[gpui::test]
fn typing_at_a_folds_edges_keeps_it_and_inside_it_opens_it(cx: &mut TestAppContext) {
    let (e, cx) = editor("let x = compute(a, b);\n", cx);
    let type_at = |e: &Entity<Editor>, cx: &mut VisualTestContext, at: usize, t: &'static str| {
        e.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(at);
                s.insert(t);
            })
        });
        cx.run_until_parked();
    };
    fold_chars(&e, cx, [16..20]);
    type_at(&e, cx, 20, "!");
    assert_eq!(chars(&e, cx), [16..20], "typed at its end: after it");
    type_at(&e, cx, 16, "?");
    assert_eq!(chars(&e, cx), [17..21], "typed at its start: before it, and it moves on");
    // an edit inside it (a replace in hidden text) opens it
    e.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.replace_ranges(vec![(18..19, "z".into())]);
        })
    });
    cx.run_until_parked();
    assert!(chars(&e, cx).is_empty());
    e.read_with(cx, |e, cx| assert_eq!(e.text(cx), "let x = compute(?az b!);\n"));
}

#[gpui::test]
fn a_wrapped_line_lays_out_its_placeholder_with_its_text(cx: &mut TestAppContext) {
    let text = format!("{} {}\nnext\n", "word ".repeat(20).trim_end(), "tail ".repeat(10));
    let text_ = text.clone();
    let (e, cx) = cx.add_window_view(move |_, cx| {
        let mut e = Editor::new(EditorState::from_text(&text_), false, cx);
        e.wrap_override = Some(true);
        e.view.minimap = false;
        e
    });
    cx.simulate_resize(size(px(300.), px(300.)));
    cx.run_until_parked();
    let rows_before = e.read_with(cx, |e, cx| e.display_rows(cx));
    // fold "word word … word" bar the first: 20 words of 5 become a 3-column placeholder
    fold_chars(&e, cx, [5..99]);
    let (rows, markers) = drawn_rows(&e, cx);
    assert!(e.read_with(cx, |e, cx| e.display_rows(cx)) < rows_before);
    assert_eq!(markers, 1);
    // the placeholder is on one row, whole, right after "word "
    let first: String = rows.iter().take_while(|r| !r.starts_with("next")).cloned().collect();
    assert!(first.starts_with("word    "), "{rows:?}");
    e.read_with(cx, |e, _| {
        let m = &e.layout.as_ref().unwrap().drawn.markers[0].1;
        let space_w = e.layout.as_ref().unwrap().space_w;
        assert!((m.size.width - space_w * 3.).abs() < px(0.5), "{m:?}");
    });
}

#[gpui::test]
fn carets_on_several_folded_lines_step_over_their_folds_together(cx: &mut TestAppContext) {
    // "ab(cd)ef" three times, each "(cd)" folded: 2..6, 11..15, 20..24
    let (e, cx) = editor("ab(cd)ef\nab(cd)ef\nab(cd)ef\n", cx);
    fold_chars(&e, cx, [2..6, 11..15, 20..24]);
    let carets = [2, 11, 20].map(Selection::caret).to_vec();
    e.update(cx, |e, cx| e.sel(cx).set_selections(carets));
    cx.run_until_parked();
    run(&e, cx, &Move::to(Motion::Right, true));
    e.read_with(cx, |e, _| {
        let ranges: Vec<Range<usize>> = e.state.selections.iter().map(|s| s.range()).collect();
        assert_eq!(ranges, [2..6, 11..15, 20..24], "each selects its fold whole");
    });
    assert_eq!(chars(&e, cx), [2..6, 11..15, 20..24], "and keeps it");
    run(&e, cx, &Copy);
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("(cd)\n(cd)\n(cd)"), "the hidden text is copied");
}

#[gpui::test]
fn a_find_landing_in_folded_text_opens_its_fold(cx: &mut TestAppContext) {
    let (e, cx) = editor("let x = compute(alpha, b);\n", cx);
    fold_chars(&e, cx, [16..24]);
    // what Find Next does with a match: select it
    e.update(cx, |e, cx| e.select_range(16..21, cx));
    cx.run_until_parked();
    assert!(chars(&e, cx).is_empty(), "the match is shown");
    e.read_with(cx, |e, _| assert_eq!(e.state.selections[0].range(), 16..21));
}

#[gpui::test]
fn home_after_a_fold_goes_to_its_display_lines_start_and_keeps_it(cx: &mut TestAppContext) {
    // "  head {" folded from its end to the end of the text, 8..24
    let (e, cx) = editor("  head {\n    one\n    two", cx);
    e.update(cx, |e, cx| {
        e.set_folds_now(folds(e.doc(cx), 0, 2), cx);
        cx.notify();
    });
    cx.run_until_parked();
    let to = |motion| Move::to(motion, false);
    run(&e, cx, &to(Motion::DocEnd));
    assert_eq!(caret(&e, cx), 24);
    // Home: the header's indentation, then its start
    run(&e, cx, &to(Motion::LineStart));
    assert_eq!((caret(&e, cx), hidden(&e, cx)), (2, vec![1..3]));
    run(&e, cx, &to(Motion::LineStart));
    assert_eq!((caret(&e, cx), hidden(&e, cx)), (0, vec![1..3]));
    // ctrl-a: the header's start
    run(&e, cx, &to(Motion::DocEnd));
    run(&e, cx, &to(Motion::HardLineStart));
    assert_eq!((caret(&e, cx), hidden(&e, cx)), (0, vec![1..3]));
}

#[gpui::test]
fn the_fold_button_beside_a_header_ending_in_a_fold_folds_its_block(cx: &mut TestAppContext) {
    // "fn a(x, y) {" with "x, y) {" folded, 5..12: the block below is not
    let (e, cx) = editor("fn a(x, y) {\n    one\n    two\n}\n", cx);
    fold_chars(&e, cx, [5..12]);
    let (button, _, _) = columns(&e, cx);
    let head = at_row(&e, cx, button, 0.);
    cx.simulate_click(head, Modifiers::none());
    assert_eq!(chars(&e, cx), [5..12, 12..28], "folded beside the fold in the line");
    assert_eq!(hidden(&e, cx), [1..3]);
    // ▸ opens the block's fold, and the one in the line stays
    cx.simulate_click(head, Modifiers::none());
    assert_eq!(chars(&e, cx), [5..12]);
    // a fold from inside the line over its line break is the block's: ▸ opens it
    fold_chars(&e, cx, [8..20]);
    assert_eq!(hidden(&e, cx), [1..2]);
    cx.simulate_click(head, Modifiers::none());
    assert!(chars(&e, cx).is_empty());
}

#[gpui::test]
fn a_caret_after_a_placeholder_highlights_its_display_lines_gutter(cx: &mut TestAppContext) {
    let (e, cx) = editor("head\n  one\n  two\ntail\n", cx);
    e.update(cx, |e, cx| {
        e.set_folds_now(folds(e.doc(cx), 0, 2), cx);
        // after the placeholder, on line 2 joined to the header's display line
        e.sel(cx).set_cursor(16);
        cx.notify();
    });
    cx.run_until_parked();
    let current = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
        e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.current.clone())
    };
    assert_eq!(current(&e, cx), [0]);
    assert_eq!(hidden(&e, cx), [1..3]);
    // on the line after it: its own row
    e.update(cx, |e, cx| {
        e.sel(cx).set_cursor(18);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(current(&e, cx), [1]);
    // the last display line, folded to the end of the text
    let (e, cx) = editor("head\n  one\n  two", cx);
    e.update(cx, |e, cx| {
        e.set_folds_now(folds(e.doc(cx), 0, 2), cx);
        e.sel(cx).set_cursor(16);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(current(&e, cx), [0]);
}

#[gpui::test]
fn python_newline_after_typing_block_colon(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("if ready", cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Python");
        e.sel(cx).set_cursor(8);
    });
    e.update(cx, |e, cx| e.edit_text(cx, |s| s.type_text(":")));
    e.update_in(cx, |e, window, cx| e.perform_action(&Newline, window, cx));
    e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).rope.to_string(), "if ready:\n    "));
}

#[gpui::test]
fn reindent_action_uses_active_language_and_undo(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("if ok:\nwork()", cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Python");
        e.sel(cx).select_all();
    });
    e.update_in(cx, |e, window, cx| e.perform_action(&ReindentSelection, window, cx));
    e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).rope.to_string(), "if ok:\n    work()"));
    e.update_in(cx, |e, window, cx| e.perform_action(&Undo, window, cx));
    e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).rope.to_string(), "if ok:\nwork()"));
}

#[gpui::test]
fn reindent_refuses_immutable_follow_and_unsupported_sources(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("if ok:\nwork()", cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Python");
        e.sel(cx).select_all();
        e.input_enabled = false;
    });
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    e.update_in(cx, |e, window, cx| e.perform_action(&ReindentSelection, window, cx));
    assert_eq!(e.read_with(cx, |e, cx| e.doc(cx).version), version);
    e.update(cx, |e, cx| {
        e.input_enabled = true;
        e.update_doc(cx, |d| d.set_following(true));
    });
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    e.update_in(cx, |e, window, cx| e.perform_action(&ReindentSelection, window, cx));
    assert_eq!(e.read_with(cx, |e, cx| e.doc(cx).version), version);
    e.update(cx, |e, cx| {
        e.update_doc(cx, |d| d.set_following(false));
        e.highlighter = Highlighter::by_name("Plain Text");
    });
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    e.update_in(cx, |e, window, cx| e.perform_action(&ReindentSelection, window, cx));
    e.read_with(cx, |e, cx| {
        assert_eq!(e.doc(cx).rope.to_string(), "if ok:\nwork()");
        assert_eq!(e.doc(cx).version, version);
    });
}

#[gpui::test]
fn python_newline_after_native_composition_is_undoable(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (e, cx) = test_editor("if ready", cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Python");
        e.sel(cx).set_cursor(8);
    });
    e.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, ":", None, window, cx);
        e.replace_text_in_range(None, ":", window, cx);
        e.perform_action(&Newline, window, cx);
    });
    e.read_with(cx, |e, cx| {
        assert_eq!(e.text(cx), "if ready:\n    ");
        assert!(e.marked.is_empty());
    });
    e.update_in(cx, |e, window, cx| e.perform_action(&Undo, window, cx));
    e.read_with(cx, |e, cx| assert_eq!(e.text(cx), "if ready:"));
}

#[gpui::test]
fn rust_newline_inside_multiline_string_preserves_literal_indent(cx: &mut TestAppContext) {
    let text = "let text = \"first\n  {";
    let (e, cx) = test_editor(text, cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Rust");
        e.sel(cx).set_cursor(21);
    });
    e.update_in(cx, |e, window, cx| e.perform_action(&Newline, window, cx));
    e.read_with(cx, |e, cx| assert_eq!(e.text(cx), "let text = \"first\n  {\n  "));
}

#[gpui::test]
fn javascript_newline_after_regex_uses_real_action(cx: &mut TestAppContext) {
    let text = r"const re = /\(/;";
    let (e, cx) = test_editor(text, cx, |e, cx| {
        e.highlighter = Highlighter::by_name("JavaScript");
        e.sel(cx).set_cursor(16);
    });
    e.update_in(cx, |e, window, cx| e.perform_action(&Newline, window, cx));
    e.read_with(cx, |e, cx| assert_eq!(e.text(cx), "const re = /\\(/;\n"));
}

#[gpui::test]
fn python_newline_after_aligned_closer_keeps_outer_indent(cx: &mut TestAppContext) {
    let text = "def f():\n    result = call(\n        value,\n    )";
    let end = text.chars().count();
    let (e, cx) = test_editor(text, cx, move |e, cx| {
        e.highlighter = Highlighter::by_name("Python");
        e.sel(cx).set_cursor(end);
    });
    e.update_in(cx, |e, window, cx| e.perform_action(&Newline, window, cx));
    e.read_with(cx, |e, cx| assert_eq!(e.text(cx), format!("{text}\n    ")));
}

#[gpui::test]
fn n14_matching_lines_really_hides_nonmatching_source_without_rewriting_it(cx: &mut TestAppContext) {
    let text = "before\n雪 hit\nafter\n";
    let (e, cx) = editor(text, cx);
    run(&e, cx, &ShowMatchingLines { pattern: "hit".into(), context: 0, ignore_case: false });
    n14_settle_filter(&e, cx);
    e.read_with(cx, |e, cx| {
        assert_eq!(e.text(cx), text);
        assert!(e.folds.hiding(2).is_some(), "the nonmatching prefix must be hidden");
        assert!(e.folds.hiding(15).is_some(), "the nonmatching suffix must be hidden");
        assert!(e.folds.hiding(9).is_none(), "matching Unicode source stays visible");
    });
}

fn n14_settle_filter(e: &Entity<Editor>, cx: &mut VisualTestContext) {
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        if e.read_with(cx, |e, _| e.filter_job.is_none()) {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(15));
        std::thread::yield_now();
    }
}

use crate::editing::fold;

#[gpui::test]
fn n15_plain_indent_navigation_selection_undo_source_redo_and_boundaries(cx: &mut TestAppContext) {
    let text = "root\n\tchild\n\t\t雪😀\n\ttail\nnext\n\tend";
    let (e, cx) = test_editor(text, cx, |e, _| {
        e.highlighter = Highlighter::by_name("Plain Text");
        e.wrap_override = Some(true);
        e.wrap_width_override = Some(8);
    });
    e.update_in(cx, |e, w, cx| {
        let end = e.doc(cx).len();
        e.sel(cx).set_cursor(end);
        e.edit(cx, |s| s.insert("!"));
        e.perform_action(&Undo, w, cx);
        e.sel(cx).set_cursor(0);
    });
    let before = e.read_with(cx, |e, cx| (e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved()));
    run(&e, cx, &MoveToFold { motion: fold::Motion::Previous, select: false });
    assert_eq!(caret(&e, cx), 0);
    run(&e, cx, &MoveToFold { motion: fold::Motion::Next, select: false });
    assert_eq!(caret(&e, cx), 5);
    run(&e, cx, &MoveToFold { motion: fold::Motion::Parent, select: true });
    e.read_with(cx, |e, _| assert_eq!(e.state.selections, [Selection::new(5, 0)]));
    run(&e, cx, &UndoSelection);
    assert_eq!(caret(&e, cx), 5);
    run(&e, cx, &RedoSelection);
    e.read_with(cx, |e, _| assert_eq!(e.state.selections, [Selection::new(5, 0)]));
    run(&e, cx, &SelectFold);
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.selections, [Selection::new(e.doc(cx).line_end(3), 0)]);
        assert!(e.folds.is_empty());
        assert_eq!(e.text(cx), text);
        assert_eq!((e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved()), before);
    });
    run(&e, cx, &Redo);
    e.read_with(cx, |e, cx| assert_eq!(e.text(cx), format!("{text}!")));
    e.update(cx, |e, cx| {
        let end = e.doc(cx).len();
        e.sel(cx).set_cursor(end);
    });
    let end = caret(&e, cx);
    run(&e, cx, &MoveToFold { motion: fold::Motion::Next, select: true });
    assert_eq!(caret(&e, cx), end);
    run(&e, cx, &MoveToFold { motion: fold::Motion::Previous, select: false });
    let next = e.read_with(cx, |e, cx| e.doc(cx).line_start(4));
    assert_eq!(caret(&e, cx), next);
}

#[gpui::test]
fn n15_collapsed_nested_and_inline_targets_reveal_only_hiding_folds(cx: &mut TestAppContext) {
    let (e, cx) = editor("head\n  inner\n    body\ntail abcdef\n", cx);
    let (outer, inner, inline) = e.read_with(cx, |e, cx| {
        let d = e.doc(cx);
        (d.line_end(0)..d.line_end(2), d.line_end(1)..d.line_end(2), d.line_start(3) + 5..d.line_start(3) + 8)
    });
    fold_chars(&e, cx, [outer.clone(), inner.clone(), inline.clone()]);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(0));
    run(&e, cx, &MoveToFold { motion: fold::Motion::Next, select: false });
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.cursor(), e.doc(cx).line_start(1));
        assert_eq!(e.folds.chars(), [inner.clone(), inline.clone()]);
    });
    run(&e, cx, &MoveToFold { motion: fold::Motion::Next, select: false });
    assert_eq!(caret(&e, cx), inline.start);
    e.read_with(cx, |e, _| assert_eq!(e.folds.chars(), [inner.clone(), inline.clone()]));
    run(&e, cx, &SelectFold);
    e.read_with(cx, |e, _| {
        assert_eq!(e.state.selections, [Selection::new(inline.start, inline.end)]);
        assert_eq!(e.folds.chars(), [inner, inline]);
    });
}

#[gpui::test]
fn n15_current_syntax_empty_index_manual_and_stale_indent_fallback(cx: &mut TestAppContext) {
    let (e, cx) = test_editor("fn café() {\nif true {\n雪();\n}\n}\nfn sibling() {\nx();\n}\n", cx, |e, cx| {
        e.highlighter = Highlighter::by_name("Rust");
        parse_syntax(e, cx);
    });
    run(&e, cx, &MoveToFold { motion: fold::Motion::Next, select: false });
    assert_eq!(caret(&e, cx), 12);
    run(&e, cx, &MoveToFold { motion: fold::Motion::Parent, select: false });
    assert_eq!(caret(&e, cx), 0);
    e.update(cx, |e, cx| {
        e.set_text("alpha\n  beta\nend\n", cx);
        parse_syntax(e, cx);
        assert!(e.highlighter.fold_regions(&e.doc(cx).rope).unwrap().chars().next().is_none());
        e.sel(cx).set_cursor(0);
    });
    run(&e, cx, &SelectFold);
    assert_eq!(caret(&e, cx), 0, "valid empty syntax index is authoritative");
    fold_chars(&e, cx, [1..4]);
    e.update(cx, |e, cx| e.sel(cx).set_cursor(0));
    run(&e, cx, &MoveToFold { motion: fold::Motion::Next, select: false });
    assert_eq!(caret(&e, cx), 1, "manual range survives empty syntax candidates");
    e.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("new\n  nested\n");
        });
        assert!(e.highlighter.fold_regions(&e.doc(cx).rope).is_none());
        e.sel(cx).set_cursor(0);
    });
    run(&e, cx, &SelectFold);
    e.read_with(cx, |e, cx| assert_eq!(e.state.selections[0].range(), 0..e.doc(cx).line_end(1)));
}

#[gpui::test]
fn n15_multicarets_primary_virtual_anchors_and_shared_view_independence(cx: &mut TestAppContext) {
    let text = "a\n  b\nc\n  d\ne\n  f\n";
    let (e, cx) = editor(text, cx);
    let peer = e.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
    let ends = e.read_with(cx, |e, cx| (e.doc(cx).line_end(0), e.doc(cx).line_end(2), e.doc(cx).line_start(4)));
    e.update(cx, |e, cx| {
        e.set_virtual_space(crate::config::settings::VirtualSpace::All, cx);
        e.sel(cx).set_selections(vec![Selection::from_points((ends.0, 3), (ends.0, 3)), Selection::caret(ends.1)]);
    });
    let primary = e.read_with(cx, |e, cx| e.read_state(cx).primary());
    run(&e, cx, &MoveToFold { motion: fold::Motion::Next, select: true });
    e.read_with(cx, |e, cx| {
        assert_eq!(e.state.selections[0].anchor_point(), (ends.0, 3));
        assert_eq!(e.state.selections[0].head_point(), (e.doc(cx).line_start(2), 0));
        assert_eq!(e.state.selections[1].head, ends.2);
        assert_eq!(e.read_state(cx).primary(), primary);
        assert_eq!(e.text(cx), text);
    });
    peer.read_with(cx, |e, cx| assert_eq!(e.text(cx), text));
    peer.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("雪\n");
        });
    });
    e.update(cx, |e, cx| e.catch_up(cx));
    run(&e, cx, &MoveToFold { motion: fold::Motion::Previous, select: false });
    e.read_with(cx, |e, cx| {
        assert_eq!(
            e.state.selections.iter().map(|s| s.head).collect::<Vec<_>>(),
            [e.doc(cx).line_start(1), e.doc(cx).line_start(3)]
        );
        assert_eq!(e.text(cx), format!("雪\n{text}"));
    });
}

#[gpui::test]
fn n15_filter_saved_folds_reveal_exact_target_and_keep_visible_filter(cx: &mut TestAppContext) {
    let (e, cx) = editor("head\n  inner\n    body\ntail\n", cx);
    e.update_in(cx, |e, w, cx| {
        e.fold_chars_now([4..21, 12..21], cx);
        e.sel(cx).set_selection(0, 4);
        e.perform_action(&NarrowToSelection, w, cx);
        assert!(e.view_filter.is_some());
        e.perform_action(&MoveToFold { motion: fold::Motion::Parent, select: false }, w, cx);
        assert!(e.view_filter.is_some(), "no destination retains filter");
        e.perform_action(&MoveToFold { motion: fold::Motion::Next, select: false }, w, cx);
        assert_eq!(e.state.cursor(), 5);
        assert!(e.view_filter.is_none(), "hidden target reveals original source");
        assert_eq!(e.folds.chars(), vec![12..21], "open only the saved outer fold hiding target");
        assert_eq!(e.text(cx), "head\n  inner\n    body\ntail\n");
    });
    e.update_in(cx, |e, w, cx| {
        e.sel(cx).set_selection(0, 21);
        e.perform_action(&NarrowToSelection, w, cx);
        e.sel(cx).set_cursor(0);
        e.perform_action(&MoveToFold { motion: fold::Motion::Next, select: false }, w, cx);
        assert_eq!(e.state.cursor(), 5);
        assert!(e.view_filter.is_some(), "visible destination keeps the view filter");
        assert_eq!(e.capture_folds(cx).unwrap().restore(e.doc(cx)).chars(), vec![12..21]);
    });
}

#[gpui::test]
fn n15_empty_dummy_filter_fields_aligned_and_local_composition_refuse(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (e, cx) = editor("abcdef\n", cx);
    e.update_in(cx, |e, w, cx| {
        e.sel(cx).set_selection(2, 4);
        e.perform_action(&NarrowToSelection, w, cx);
        let folds = e.folds.clone();
        e.perform_action(&MoveToFold { motion: fold::Motion::Next, select: false }, w, cx);
        assert_eq!(e.folds, folds, "dummy exclusions are not ordinary candidates");
        assert!(e.view_filter.is_some());
        e.perform_action(&ClearFilter, w, cx);
        e.fold_chars_now([1..3], cx);
        e.sel(cx).set_cursor(0);
        for field in [true, false] {
            e.single_line = field;
            e.aligned = !field;
            e.perform_action(&MoveToFold { motion: fold::Motion::Next, select: false }, w, cx);
            e.perform_action(&SelectFold, w, cx);
            assert_eq!(e.state.cursor(), 0);
            assert_eq!(e.folds.chars(), vec![1..3]);
        }
        e.single_line = false;
        e.aligned = false;
        e.replace_and_mark_text_in_range(Some(0..1), "a", Some(0..1), w, cx);
        let before = (e.marked.clone(), e.read_state(cx).selection_state(), e.folds.clone(), e.text(cx));
        e.move_to_fold(&MoveToFold { motion: fold::Motion::Next, select: false }, w, cx);
        e.select_fold(&SelectFold, w, cx);
        assert!((e.marked.clone(), e.read_state(cx).selection_state(), e.folds.clone(), e.text(cx)) == before);
        e.unmark_text(w, cx);
        e.set_text("", cx);
        e.perform_action(&SelectFold, w, cx);
        e.perform_action(&MoveToFold { motion: fold::Motion::Parent, select: true }, w, cx);
        assert_eq!(e.state.selections, [Selection::caret(0)]);
    });
}
