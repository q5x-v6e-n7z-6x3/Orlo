use super::*;

#[test]
fn diff_finds_the_same_middle_as_a_char_by_char_scan() {
    // chunk edges and multi-byte chars: the common bytes must end on whole chars
    let base = "é漢😀ab\n".repeat(3_000);
    let cases = [
        (base.clone(), base.clone()),
        (base.clone(), format!("{base}x")),
        (format!("x{base}"), base.clone()),
        (base.clone(), base.replacen('漢', "字", 1)),
        (base.clone(), base.replacen('😀', "é", 1)),
        (base.clone(), format!("{}é{}", &base[..9_000], &base[9_000..])),
        ("é".into(), "è".into()),
        ("aé".into(), "a©".into()),
        ("".into(), "a".into()),
    ];
    for (a, b) in cases {
        let (ra, rb) = (Rope::from_str(&a), Rope::from_str(&b));
        // the middle by a plain char by char scan from both ends
        let (la, lb) = (ra.len_chars(), rb.len_chars());
        let max = la.min(lb);
        let (mut ia, mut ib) = (ra.chars(), rb.chars());
        let mut start = 0;
        while start < max && ia.next() == ib.next() {
            start += 1;
        }
        let (mut pa, mut pb) = (ra.chars_at(la), rb.chars_at(lb));
        let mut end = 0;
        while end < max - start && pa.prev() == pb.prev() {
            end += 1;
        }
        let mut want = Vec::new();
        let (old, new) = (ra.slice(start..la - end), rb.slice(start..lb - end));
        minimal(start, &old.to_string(), &new.to_string(), &mut want);
        assert_eq!(diff(&ra, &rb), want, "{} vs {} chars", la, lb);
    }
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn an_outline_keeps_only_its_own_surviving_text() {
    let mut r = vec![4..7];
    map_ranges(&mut r, &[change(3, 2, 5)]);
    assert_eq!(r, vec![8..10], "3..5 replaced by xxxxx: the x's stay outside");
}

#[test]
fn undo_and_redo_bring_back_find_results_outlines() {
    let text = "/p/a.rs:\n    3: let foo = 1;\n    9: foo();\n";
    let at = |w: &str| text.find(w).unwrap();
    let mut s = EditorState::from_text(text);
    s.doc.scratch = true;
    let first = at("foo")..at("foo") + 3;
    let second = at("foo()")..at("foo()") + 3;
    s.doc.outlines = vec![first.clone(), second.clone()];
    // delete the first match's line, then type before the second
    s.set_selection(at("    3:"), at("    9:"));
    s.insert("");
    s.take_changes();
    let moved = second.start - 20..second.end - 20;
    assert_eq!(s.doc.outlines, vec![moved.clone()], "the edit drops the first");
    s.set_cursor(moved.start);
    s.type_text("x");
    s.take_changes();
    assert_eq!(s.doc.outlines, vec![moved.start + 1..moved.end + 1]);
    s.undo();
    s.take_changes();
    assert_eq!(s.doc.outlines, vec![moved.clone()]);
    s.undo();
    s.take_changes();
    assert_eq!(s.doc.outlines, vec![first.clone(), second.clone()], "undo brings it back");
    s.redo();
    s.take_changes();
    assert_eq!(s.doc.outlines, vec![moved], "redo drops it again");
}

#[test]
fn map_ranges_maps_thousands_of_outlines_through_thousands_of_changes_in_one_sweep() {
    // Find Results with 20k outlines, then a Find All of 5k matches and a keystroke
    let mut ranges: Vec<Range<usize>> = (0..20_000).map(|i| i * 10..i * 10 + 3).collect();
    let changes: Vec<Change> = (0..5_000).map(|i| change(i * 40 + 5 + i, 0, 1)).collect();
    let naive: Vec<Range<usize>> = ranges
        .iter()
        .map(|r| {
            let first = map_offset(r.start + 1, &changes) - 1;
            map_offset(r.start, &changes).max(first)..map_offset(r.end, &changes)
        })
        .collect();
    let t = std::time::Instant::now();
    map_ranges(&mut ranges, &changes);
    let took = t.elapsed();
    assert_eq!(ranges, naive);
    assert!(took.as_millis() < 100, "{took:?}");
}

#[test]
fn map_sorted_matches_map_offset_one_by_one() {
    let mut rnd = lcg(7);
    for _ in 0..3000 {
        // runs as the editor logs them: ascending, each after the text the previous one
        // inserted (typing, diff), and descending (replace all, line commands)
        let mut changes = Vec::new();
        for _ in 0..1 + rnd(3) {
            let mut p = rnd(40);
            if rnd(2) == 0 {
                for _ in 0..rnd(6) {
                    let (start, removed, inserted) = (p + rnd(3), rnd(3), rnd(3));
                    changes.push(change(start, removed, inserted));
                    p = start + inserted;
                }
            } else {
                p += 20;
                for _ in 0..rnd(6) {
                    let end = p - rnd(3).min(p);
                    let removed = rnd(3).min(end);
                    let start = end - removed;
                    changes.push(change(start, removed, rnd(3)));
                    p = start;
                }
            }
        }
        let mut xs: Vec<usize> = (0..rnd(12)).map(|_| rnd(60)).collect();
        xs.sort();
        let one_by_one: Vec<usize> = xs.iter().map(|&x| map_offset(x, &changes)).collect();
        let starts: Vec<usize> = xs.iter().map(|&x| map_start(x, &changes)).collect();
        let mut ys = xs.clone();
        map_sorted(&mut xs, &changes);
        assert_eq!(xs, one_by_one, "{changes:?}");
        // starts are exact within one run, as the editor logs most edits (map_sorted_starts)
        let up = |a: &Change, b: &Change| b.start >= a.start + a.inserted;
        let down = |a: &Change, b: &Change| b.start + b.removed <= a.start && (b.removed > 0 || b.start < a.start);
        let pairs = || changes.windows(2);
        if pairs().all(|w| up(&w[0], &w[1])) || pairs().all(|w| down(&w[0], &w[1])) {
            map_sorted_starts(&mut ys, &changes);
            assert_eq!(ys, starts, "starts: {changes:?}");
        }
    }
}

#[test]
fn logged_changes_keep_every_untouched_char_in_place() {
    let mut rnd = lcg(11);
    let words = ["a", "bb", "ccc", "  d", "e e", ""];
    for _ in 0..2000 {
        let mut lines: Vec<String> = (0..rnd(8)).map(|_| words[rnd(6)].to_string()).collect();
        let old = lines.join("\n");
        for _ in 0..1 + rnd(4) {
            let i = rnd(lines.len() + 1);
            match rnd(3) {
                0 => lines.insert(i, words[rnd(6)].to_string()),
                1 if i < lines.len() => drop(lines.remove(i)),
                _ if i < lines.len() => lines[i].push('!'),
                _ => {}
            }
        }
        let new = lines.join("\n");
        let mut changes = Vec::new();
        minimal(0, &old, &new, &mut changes);
        // replay: kept chars carry their value, inserted ones are unknown
        let mut text: Vec<Option<char>> = old.chars().map(Some).collect();
        for c in &changes {
            text.splice(c.start..c.start + c.removed, std::iter::repeat_n(None, c.inserted));
        }
        let want: Vec<char> = new.chars().collect();
        assert_eq!(text.len(), want.len(), "{old:?} → {new:?}: {changes:?}");
        for (got, want) in text.iter().zip(&want) {
            assert!(got.is_none_or(|g| g == *want), "{old:?} → {new:?}: {changes:?}");
        }
    }
}

#[test]
fn diff_is_the_minimal_middle() {
    assert_eq!(diff(&Rope::from("abcXdef"), &Rope::from("abcYYdef")), vec![change(3, 1, 2)]);
    assert_eq!(diff(&Rope::from("aaa"), &Rope::from("aaaa")), vec![change(3, 0, 1)]);
}
