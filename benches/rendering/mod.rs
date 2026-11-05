use super::*;
use crate::testing::block_on;

#[test]
#[ignore]
fn bench_highlight() {
    let s = EditorState::from_text(&rust_lines(100_000));
    let path = Path::new("x.rs");
    for (name, from, budget_ms) in
        [("highlight the first 60 lines", 0, 30.), ("highlight 60 lines at line 50,000", 50_000, 60.)]
    {
        time(name, 5, budget_ms, || {
            let mut h = Highlighter::new(Some(path));
            (from..from + 60).map(|l| h.line_runs(&s.doc.rope, l).len()).sum::<usize>()
        });
    }
    let mut h = Highlighter::new(Some(path));
    for l in 0..1_000 {
        h.line_runs(&s.doc.rope, l);
    }
    let mut rope = s.doc.rope.clone();
    // as a frame asks: within its budget, an edit's unchanged lines taken from the cache
    time("type a char, re-highlight the 60 lines below", 5, 3., || {
        rope.insert_char(rope.line_to_char(940) + 4, 'x');
        h.invalidate_from(940);
        h.begin_frame();
        (940..1_000).filter_map(|l| h.try_line_runs(&rope, l)).count()
    });
    time("jump 400 lines down, highlighting on the way", 5, 60., || {
        Highlighter::new(Some(path)).line_runs(&s.doc.rope, 400).len()
    });
}

#[test]
#[ignore]
fn bench_wrap() {
    let s = EditorState::from_text(&rust_lines(100_000));
    time("wrap 100k lines at 100 columns", 3, 600., || WrapMap::new(&s.doc, 100, 4, &Folds::NONE));
    let long = EditorState::from_text(&"word ".repeat(2_000_000));
    time("wrap one 10 MB line at 100 columns", 3, 600., || WrapMap::new(&long.doc, 100, 4, &Folds::NONE));
}

#[test]
#[ignore]
fn bench_wrap_update() {
    let mut s = EditorState::from_text(&rust_lines(100_000));
    let mut w = WrapMap::new(&s.doc, 100, 4, &Folds::NONE);
    s.set_selections(vec![Selection::caret(s.line_start(1)), Selection::caret(s.line_start(60_000))]);
    s.take_changes();
    time("rewrap after typing at lines 1 and 60,000", 5, 30., || {
        s.type_text("x");
        let changes = s.take_changes();
        w.update(&s.doc, &changes, &Folds::NONE);
    });
}

#[test]
#[ignore]
fn bench_wrap_update_1m_lines() {
    let mut s = EditorState::from_text(&rust_lines(1_200_000));
    let mut w = WrapMap::new(&s.doc, 100, 4, &Folds::NONE);
    s.set_selections(vec![Selection::caret(s.line_start(1)), Selection::caret(s.line_start(1_000_000))]);
    s.take_changes();
    // the edit's changes rewrapped, counting no more than a few thousand lines
    let rewrap = |s: &mut EditorState, w: &mut WrapMap| {
        let counted = || orlo_editor::display::COUNTED.get();
        let changes = s.take_changes();
        let before = counted();
        w.update(&s.doc, &changes, &Folds::NONE);
        let lines = counted() - before;
        assert!(lines <= 4 * 1024, "{lines} lines counted");
    };
    time("rewrap after typing at lines 1 and 1M, 1.2M lines", 5, 3., || {
        s.type_text("x");
        rewrap(&mut s, &mut w);
    });
    time("rewrap after a line break at line 1, 1.2M lines", 5, 3., || {
        s.set_selections(vec![Selection::caret(s.line_start(1))]);
        s.newline();
        rewrap(&mut s, &mut w);
    });
    assert_eq!(w, WrapMap::new(&s.doc, 100, 4, &Folds::NONE), "the same rows as a fresh wrap");
}

#[test]
#[ignore]
fn bench_highlight_frames() {
    // 60 lines of 15 KB each: minified code, generated tables
    let long: String = (0..60).map(|_| rust_lines(400).replace('\n', " ") + "\n").collect();
    let s = EditorState::from_text(&long);
    let path = Path::new("x.rs");
    // a frame colours what fits its 16 KB budget, one such line; the job colours the rest
    time("a frame of 50 lines of 15 KB (colours 1)", 3, 300., || {
        let mut h = Highlighter::new(Some(path));
        h.begin_frame();
        let n = (0..50).filter_map(|l| h.try_line_runs(&s.doc.rope, l)).count();
        assert_eq!(n, 1, "lines coloured in the frame");
    });
    time("the job colouring those 50 lines", 3, 3000., || {
        let job = Highlighter::new(Some(path)).job(0);
        let job = orlo_syntax::highlight::run_job(job, &s.doc.rope, &[0, 49]);
        assert!(job.cached_lines() >= 50, "{} lines coloured", job.cached_lines());
    });
    // a second view of a 100k-line file far down, then an edit at the top of the first: above
    // the second's cache, whose lines draw as they were until a job redoes them
    let s = EditorState::from_text(&rust_lines(100_000));
    let mut warm = Highlighter::new(Some(path));
    for l in 0..20_060 {
        warm.line_runs(&s.doc.rope, l);
    }
    let mut rope = s.doc.rope.clone();
    rope.insert_char(4, 'x');
    let typed = orlo_editor::text::Change { start: 4, removed: 0, inserted: 1, lines: (0, 0) };
    time("split view's frame below an edit above its cache (60 as they were)", 3, 300., || {
        let mut h = warm.clone();
        h.invalidate_from(0);
        h.text_changed(&[typed]);
        h.begin_frame();
        assert!(h.provisional());
        let n = (20_000..20_060).filter(|&l| h.try_line_runs(&rope, l).is_some()).count();
        assert_eq!(n, 60, "drawn from the cache as it was");
    });
    time("background job: 60 lines at line 50,000", 3, 120., || {
        let job = Highlighter::new(Some(path)).job(50_000);
        let job = orlo_syntax::highlight::run_job(job, &s.doc.rope, &[50_000, 50_060]);
        assert!(job.cached_lines() >= 61);
    });
    // a line too long to colour, and 40 below it: all in one frame
    let huge = format!("let x = \"{}\";\n{}", "a".repeat(10 << 20), rust_lines(40));
    let s = EditorState::from_text(&huge);
    time("a frame over a 10 MB line and 40 below (all 41)", 3, 8., || {
        let mut h = Highlighter::new(Some(path));
        h.begin_frame();
        let n = (0..41).filter(|&l| h.try_line_runs(&s.doc.rope, l).is_some()).count();
        assert_eq!(n, 41);
    });
    // typing in a 17 KB line: the 40 lines below keep their colours until the job lands
    let line = format!("let x = [{}];\n", "1, ".repeat(5_800));
    let text = format!("{line}{}", rust_lines(40));
    let mut rope = ropey::Rope::from_str(&text);
    let mut h = Highlighter::new(Some(path));
    for l in 0..41 {
        h.line_runs(&rope, l);
    }
    rope.insert_char(9, '9');
    h.invalidate_from(0);
    h.text_changed(&[orlo_editor::text::Change { start: 9, removed: 0, inserted: 1, lines: (0, 0) }]);
    time("typing in a 17 KB line: lines below drawn (all 40)", 3, 8., || {
        h.begin_frame();
        let n = (1..41).filter(|&l| h.try_line_runs(&rope, l).or_else(|| h.fallback_runs(&rope, l)).is_some()).count();
        assert_eq!(n, 40);
    });
    // 3 MB of lines below an edit: one frame hashes at most its 1 MB budget
    let plain = format!("{}\n", "x".repeat(1_023)).repeat(3_000);
    let mut rope = ropey::Rope::from_str(&plain);
    let mut warm = Highlighter::new(None);
    for l in 0..3_000 {
        warm.line_runs(&rope, l);
    }
    rope.insert_char(0, 'y');
    time("a frame walking 3 MB of unchanged lines (hashes <= 1 MB)", 3, 8., || {
        let mut h = warm.clone();
        h.invalidate_from(0);
        h.begin_frame();
        let before = orlo_syntax::highlight::HASHED.get();
        h.try_line_runs(&rope, 2_999);
        let hashed = orlo_syntax::highlight::HASHED.get() - before;
        assert!(hashed <= (1 << 20) + 1_024, "{hashed} bytes hashed");
    });
}

#[test]
#[ignore]
fn bench_warm_up_behind_a_job() {
    // a view's job over 20,000 lines is under way when a big file opens: its first lines
    // are coloured at once, not after the job
    let s = EditorState::from_text(&rust_lines(20_000));
    let path = Path::new("x.rs");
    let rope = s.doc.rope.clone();
    let mut job = Highlighter::new(Some(path)).job(0);
    job.cancel_on(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let long = orlo_syntax::highlight::on_worker(move || orlo_syntax::highlight::run_job(job, &rope, &[19_999]));
    warm_up(&s.doc.rope, "a big file's warm-up behind a 20k-line job");
    block_on(long);
    // the same behind a Goto Symbol parse of 20,000 lines
    let h = Highlighter::new(Some(path));
    let long = orlo_syntax::highlight::parse_on_worker(h.symbol_parse(), s.doc.rope.clone(), || false);
    warm_up(&s.doc.rope, "a big file's warm-up behind a symbol parse");
    block_on(long);
}

/// A big file's first lines coloured by an urgent job, under way 20 ms after another's.
fn warm_up(rope: &ropey::Rope, name: &str) {
    std::thread::sleep(Duration::from_millis(20));
    let asked = Instant::now();
    let rope = rope.clone();
    let warm = orlo_syntax::highlight::on_worker_urgent(move || {
        let h = Highlighter::new(Some(Path::new("x.rs")));
        orlo_syntax::highlight::run_job(h, &rope, &[50])
    });
    let waited = block_on(warm).map(|_| asked.elapsed());
    let ms = waited.unwrap_or_default().as_secs_f64() * 1000.;
    println!("bench {name:<40} {ms:>10.3} ms   (budget 50 ms)");
    assert!(ms < 50.);
}

#[test]
#[ignore]
fn bench_scroll_up_past_the_restart() {
    // scrolled up just past where the parse restarted, then back down: the lines below are
    // walked to, not parsed again
    // Both restart points begin a complete function. Embedded application source can shift
    // those fixed offsets into different enclosing scopes, where cache reuse is invalid.
    let function = concat!(
        "fn sum_bytes(input: &[u8]) -> usize {\n",
        "    input\n",
        "        .iter()\n",
        "        .map(|&byte| {\n",
        "            let value = usize::from(byte);\n",
        "            value + 1\n",
        "        })\n",
        "        .sum()\n",
        "}\n",
        "\n",
    );
    let s = EditorState::from_text(&function.repeat(1_000));
    let path = Path::new("x.rs");
    // the time includes setting it up: 200 lines parsed, and a job's 200
    time("back down after a job above the restart point", 3, 60., || {
        let mut h = Highlighter::new(Some(path));
        h.line_runs(&s.doc.rope, 5_000);
        let restart = 5_000 - 200;
        let job = orlo_syntax::highlight::run_job(h.job(restart - 10), &s.doc.rope, &[restart - 10]);
        h.adopt(job, &s.doc.rope);
        let parsed = h.parsed();
        h.begin_frame();
        let ready = h.try_line_runs(&s.doc.rope, 5_000);
        assert!(ready.is_some(), "{} lines parsed again; walking: {}", h.parsed() - parsed, h.walking());
        // the old cache began from a fresh parser state at its restart point: a few
        // lines differ in state until the two parses meet
        let again = h.parsed() - parsed;
        println!("      lines parsed again on the way back down: {again}");
        assert!(again <= 50, "{again} lines parsed again");
    });
}

#[test]
#[ignore]
fn bench_symbols_long_line() {
    let mut text = rust_lines(2_000);
    text.push_str(&format!("{}\n", "let x = 1; ".repeat(1_000_000)));
    text.push_str(&rust_lines(2_000));
    let s = EditorState::from_text(&text);
    let h = Highlighter::new(Some(Path::new("x.rs")));
    time("goto symbol, 4k lines and one 11 MB line", 1, 30_000., || h.symbols(&s.doc.rope).len());
}

/// Two views side by side, as a comparison shows them.
struct SideBySide {
    sides: [gpui::Entity<crate::editor::Editor>; 2],
    pair: Option<gpui::Entity<crate::editor::AlignedPair>>,
}

impl gpui::Render for SideBySide {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, prelude::*};
        let side = |i: usize| div().flex_1().min_w_0().h_full().child(self.sides[i].clone());
        div().size_full().flex().child(side(0)).child(side(1))
    }
}

/// A wheel scroll of `dy` pixels at `at`, and the frame it paints.
fn wheel(cx: &mut gpui::VisualTestContext, at: gpui::Point<gpui::Pixels>, dy: f32) {
    use gpui::{point, px, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};
    cx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
}

#[gpui::test]
#[ignore]
fn bench_scroll_a_comparison_of_100k_lines_and_1000_hunks(cx: &mut gpui::TestAppContext) {
    use crate::editing::display::align::Hunk;
    use crate::editor::{AlignedPair, ComparisonHighlights, Editor};
    use gpui::{point, px, size, AppContext as _};
    // 100,000 lines; every 100th changed on the new side, and every 200th followed there by an
    // added line: 1,000 hunks, 500 of them taller on the new side
    let lines = rust_lines(100_000);
    let (mut old, mut new) = (String::new(), String::new());
    let (mut oc, mut nc, mut ol, mut nl) = (0, 0, 0, 0);
    let mut hunks = Vec::new();
    for (i, l) in lines.lines().enumerate() {
        let n = l.chars().count() + 1;
        old.push_str(l);
        old.push('\n');
        if i % 100 != 50 {
            new.push_str(l);
            new.push('\n');
            (oc, nc, ol, nl) = (oc + n, nc + n, ol + 1, nl + 1);
            continue;
        }
        let changed = format!("{l} changed\n{}", if i % 200 == 50 { "added\n" } else { "" });
        new.push_str(&changed);
        let (m, k) = (changed.chars().count(), changed.matches('\n').count());
        hunks.push(Hunk { lines: [ol..ol + 1, nl..nl + k], chars: [oc..oc + n, nc..nc + m] });
        (oc, nc, ol, nl) = (oc + n, nc + m, ol + 1, nl + k);
    }
    assert_eq!(hunks.len(), 1_000);
    let marks = |side: usize| ComparisonHighlights {
        lines: hunks.iter().map(|h| h.chars[side].clone()).collect(),
        inline: Vec::new(),
        added: side == 1,
    };
    let texts = [old, new];
    let (view, cx) = cx.add_window_view(|_, cx| {
        let sides = std::array::from_fn(|i| {
            cx.new(|cx| {
                let mut s = EditorState::from_text(&texts[i]);
                s.doc.read_only = true;
                s.doc.path = Some(PathBuf::from("compared.rs"));
                let mut e = Editor::new(s, false, cx);
                e.input_enabled = false;
                e.view.minimap = false;
                e.wrap_override = Some(false);
                e.comparison = Some(marks(i));
                e
            })
        });
        SideBySide { sides, pair: None }
    });
    cx.simulate_resize(size(px(1400.), px(900.)));
    view.update(cx, |v, cx| {
        let sides = v.sides.clone();
        v.pair = Some(cx.new(|cx| AlignedPair::new(sides, hunks, cx)));
    });
    cx.run_until_parked();
    let sides = view.read_with(cx, |v, _| v.sides.clone());
    let rows = sides.each_ref().map(|e| e.read_with(cx, |e, cx| e.display_rows(cx)));
    assert_eq!(rows[0], rows[1], "aligned");
    // each frame scrolls 60 rows on: both views paint their rows, the new side following
    // (3.1-3.5 ms measured, and the same with a tenth of the lines and hunks)
    let at = point(px(300.), px(400.));
    time("scroll a 100k-line comparison, 1,000 hunks", 20, 10., || {
        wheel(cx, at, -900.);
    });
    let tops = sides.each_ref().map(|e| e.read_with(cx, |e, _| e.scroll_rows().0));
    assert!(tops[0] > 1_000. && tops[0] == tops[1], "{tops:?}");
}

#[gpui::test]
#[ignore]
fn bench_scroll_a_100k_line_file_with_1000_folds(cx: &mut gpui::TestAppContext) {
    use crate::editor::Editor;
    use gpui::{point, px, size};
    let text = rust_lines(100_000);
    let (e, cx) = cx.add_window_view(move |_, cx| {
        let mut e = Editor::new(EditorState::from_text(&text), false, cx);
        // as a field write: the highlighter picked at `new` stays
        e.document.update(cx, |d, _| d.model.path = Some(PathBuf::from("folded.rs")));
        e.wrap_override = Some(true);
        e
    });
    cx.simulate_resize(size(px(1000.), px(900.)));
    cx.run_until_parked();
    // 500 line folds of 20 lines and 500 folds inside a nonempty line in each block
    e.update(cx, |e, cx| {
        let mut f = Folds::default();
        for i in 0..500 {
            let header = i * 200;
            let doc = e.doc(cx);
            f.fold_lines(doc, header, header + 20);
            let line = (header + 100..header + 200)
                .find(|&line| doc.line_end(line) > doc.line_start(line) + 4)
                .expect("a nonempty inline fold in each block");
            let ls = doc.line_start(line);
            let n = doc.line_end(line) - ls;
            f.fold(doc, ls + 4..ls + 12.min(n));
        }
        e.set_folds_now(f, cx);
        cx.notify();
    });
    cx.run_until_parked();
    e.update(cx, |e, cx| e.settle_wrap(cx));
    cx.run_until_parked();
    let folds = e.read_with(cx, |e, _| e.folds().chars().len());
    assert!(folds >= 950, "{folds} folds");
    // each frame scrolls 60 rows on, over folded lines and placeholders, wrapped (2.4-2.5 ms
    // measured, and the same with a tenth of the lines and folds)
    let at = point(px(400.), px(400.));
    time("scroll a 100k-line file with 1,000 folds", 20, 8., || {
        wheel(cx, at, -900.);
    });
    let top = e.read_with(cx, |e, _| e.scroll_rows().0);
    assert!(top > 1_000., "{top}");
}
