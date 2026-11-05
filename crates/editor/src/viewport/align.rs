//! Side-by-side diffs: the filler rows that keep two views level, so that each unchanged line
//! sits on the same display row as its partner, and offsets carried from one side to the other.

use crate::buffer::{Document, Rows};
use std::ops::Range;

/// A difference between the two sides (0 the old, 1 the new): its lines and its chars on each,
/// half-open; the chars take in its lines' breaks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    pub lines: [Range<usize>; 2],
    pub chars: [Range<usize>; 2],
}

/// The line pairs that must sit on one row: each hunk's start and end and both sides' ends,
/// and with `every_line` each unchanged pair between them too. Unwrapped, unfolded or folded
/// alike on both sides, an unchanged line takes one row on each side, so the hunks' edges are
/// enough; wrapped, a line can take more rows on one side than on the other.
pub fn anchors(hunks: &[Hunk], lines: [usize; 2], every_line: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    // the unchanged pairs from `at` up to left line `to`: as many on the right
    let pairs = |out: &mut Vec<(usize, usize)>, at: (usize, usize), n: usize| {
        if every_line {
            out.extend((0..n).map(|i| (at.0 + i, at.1 + i)));
        }
    };
    let mut at = (0, 0);
    for h in hunks {
        pairs(&mut out, at, h.lines[0].start.saturating_sub(at.0));
        out.push((h.lines[0].start, h.lines[1].start));
        out.push((h.lines[0].end, h.lines[1].end));
        at = (h.lines[0].end, h.lines[1].end);
    }
    let rest = lines[0].saturating_sub(at.0).min(lines[1].saturating_sub(at.1));
    pairs(&mut out, at, rest);
    out.push((lines[0], lines[1]));
    out.dedup();
    out
}

/// The filler rows that put each anchor's two lines on one row, as (line, rows) blocks for
/// each side: the shorter side of a hunk is padded at the hunk's end, and both sides end
/// with as many rows. `sides` are each side's rows below its filler rows. A line a fold joins
/// to the display line above is on the row its start is drawn on, and filler rows put before
/// it go below that row, so the rows after it are level with the other side's.
pub fn fillers(anchors: &[(usize, usize)], sides: [(&dyn Rows, &Document); 2]) -> [Vec<(usize, usize)>; 2] {
    let mut out = [Vec::new(), Vec::new()];
    // the filler rows put in so far on each side
    let mut added = [0, 0];
    for &(l, r) in anchors {
        let lines = [l, r];
        let row = |i: usize| sides[i].0.first_row(sides[i].1, lines[i]).0 + added[i];
        let rows = [row(0), row(1)];
        if rows[0] != rows[1] {
            let short = usize::from(rows[1] < rows[0]);
            let n = rows[1 - short] - rows[short];
            out[short].push((lines[short], n));
            added[short] += n;
        }
    }
    out
}

/// `offset` on side `from` of `sides` carried to the other side as a fold's start, or with
/// `after` its end, so that the two folds keep both sides level. Text outside the hunks is the
/// same on both, char for char, and a hunk's end goes to the other side's end. Inside a hunk
/// an offset keeps its line in the hunk and its column, within the other side's lines and
/// their text, so it never passes the last one's line break. A hunk with no lines on one side
/// has only its place there: from the other side, a start inside the hunk goes to the place
/// and an end to before the line break above it; the place itself goes to the other side's
/// start, or with `after` to its end.
pub fn map_offset(hunks: &[Hunk], sides: [&Document; 2], from: usize, offset: usize, after: bool) -> usize {
    let to = 1 - from;
    // the last hunk starting at or before the offset
    let i = hunks.partition_point(|h| h.chars[from].start <= offset);
    let Some(h) = i.checked_sub(1).map(|j| &hunks[j]) else {
        return offset;
    };
    let (here, lines, there) = (&h.chars[from], &h.lines[to], &h.chars[to]);
    match offset {
        // the place of a hunk with no text here
        o if o == here.end && here.is_empty() && !after => there.start,
        // at its end or after it
        o if o >= here.end => o - here.end + there.end,
        _ if lines.is_empty() && after => sides[to].off_cr_lf(there.start.saturating_sub(1)),
        _ if lines.is_empty() => there.start,
        o => {
            let (line, col) = sides[from].point(o);
            let line = (lines.start + line - h.lines[from].start).min(lines.end - 1);
            (sides[to].line_start(line) + col).min(sides[to].line_end(line))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::EditorState;
    use crate::fold::{Folded, Folds};
    use crate::viewport::filler::{Fillers, Padded};
    use crate::viewport::WrapMap;
    use orlo_project::comparison::{compare, Format, Text};
    use std::sync::atomic::AtomicBool;

    /// xorshift: this crate has no random number generator.
    fn next(seed: &mut u64) -> usize {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 33) as usize
    }

    /// The hunks the comparison panel gets for two texts.
    fn hunks(left: &str, right: &str) -> Vec<Hunk> {
        let text = |t: &str| Text {
            endings: None,
            rope: ropey::Rope::from_str(t),
            format: Format { line_ending: "\n", bom: false, encoding: Default::default() },
        };
        let pair = compare(&text(left), &text(right), &AtomicBool::new(false)).unwrap();
        (pair.hunks.iter())
            .map(|h| Hunk {
                lines: [h.lines.old_lines.clone(), h.lines.new_lines.clone()],
                chars: [h.lines.old_chars.clone(), h.lines.new_chars.clone()],
            })
            .collect()
    }

    /// The unchanged line pairs, from the hunks: the lines between them match one for one.
    fn unchanged(hunks: &[Hunk], lines: [usize; 2]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut at = (0, 0);
        for h in hunks.iter().chain([&Hunk { lines: [lines[0]..lines[0], lines[1]..lines[1]], ..Hunk::default() }]) {
            let n = (h.lines[0].start - at.0).min(h.lines[1].start - at.1);
            out.extend((0..n).map(|i| (at.0 + i, at.1 + i)));
            at = (h.lines[0].end, h.lines[1].end);
        }
        out
    }

    struct Side {
        s: EditorState,
        wrap: Option<WrapMap>,
        folds: Folds,
    }

    impl Side {
        fn new(text: &str, width: Option<usize>) -> Self {
            let s = EditorState::from_text(text);
            let wrap = width.map(|w| WrapMap::new(&s.doc, w, 4, &Folds::NONE));
            Self { s, wrap, folds: Folds::default() }
        }
    }

    /// Align two sides as the panel does, then check that both have as many rows and that
    /// every unchanged pair shown on both sides is on one row; the filler blocks for each.
    fn align(left: &Side, right: &Side, hunks: &[Hunk]) -> [Vec<(usize, usize)>; 2] {
        align_lines(left, right, hunks, true)
    }

    /// `align`, checking the lines a fold joins to the display line above only if `joined`.
    fn align_lines(left: &Side, right: &Side, hunks: &[Hunk], joined: bool) -> [Vec<(usize, usize)>; 2] {
        let lines = [left.s.line_count(), right.s.line_count()];
        let wraps = left.wrap.is_some() || right.wrap.is_some();
        let fl = Folded::new(left.wrap.as_ref(), &left.folds);
        let fr = Folded::new(right.wrap.as_ref(), &right.folds);
        let blocks = fillers(&anchors(hunks, lines, wraps), [(&fl, &left.s.doc), (&fr, &right.s.doc)]);
        let (bl, br) = (
            Fillers::new(&left.s.doc, blocks[0].iter().copied()),
            Fillers::new(&right.s.doc, blocks[1].iter().copied()),
        );
        let (pl, pr) = (Padded::new(&fl, &bl), Padded::new(&fr, &br));
        assert_eq!(pl.rows(&left.s.doc), pr.rows(&right.s.doc), "as many rows: {blocks:?}");
        // a line folded away has its start inside the fold: a motion there skips out of it
        let shown = |p: &Padded<&Folded>, side: &Side, l: usize| {
            let s = &side.s.doc;
            p.inner().skip(s, s.line_start(l), true) == s.line_start(l) && (joined || !side.folds.is_joined(l))
        };
        for (l, r) in unchanged(hunks, lines) {
            if l < lines[0] && shown(&pl, left, l) && shown(&pr, right, r) {
                let (a, b) = (pl.first_row(&left.s.doc, l), pr.first_row(&right.s.doc, r));
                assert_eq!(a, b, "line {l} and its partner {r}: {blocks:?}");
            }
        }
        blocks
    }

    /// `align` for two unwrapped, unfolded texts.
    fn align_texts(l: &str, r: &str) -> [Vec<(usize, usize)>; 2] {
        align(&Side::new(l, None), &Side::new(r, None), &hunks(l, r))
    }

    #[test]
    fn added_removed_and_changed_lines_pad_the_shorter_side_at_the_hunks_end() {
        // two lines added after "a": the left gets two rows before "b"
        let (l, r) = ("a\nb\nc\n", "a\nx\ny\nb\nc\n");
        assert_eq!(align_texts(l, r), [vec![(1, 2)], vec![]]);
        // removed: the right is padded where the lines were
        assert_eq!(align_texts(r, l), [vec![], vec![(1, 2)]]);
        // two lines changed into four: the left, shorter, gets two rows after them
        let (l, r) = ("a\nb\nc\nd\n", "a\nB\nC\nD\nE\nd\n");
        assert_eq!(align_texts(l, r), [vec![(3, 2)], vec![]]);
        // several hunks, longer on either side
        let (l, r) = ("a\nb\nc\nd\ne\nf\ng\n", "a\nB1\nB2\nB3\nc\nd\nf\ng\nh\n");
        assert_eq!(align_texts(l, r), [vec![(2, 2), (7, 1)], vec![(6, 1)]]);
    }

    #[test]
    fn changes_at_the_very_start_and_end_pad_before_the_first_line_and_after_the_last() {
        let (l, r) = ("b\nc\n", "x\ny\nb\nc\n");
        assert_eq!(align_texts(l, r), [vec![(0, 2)], vec![]], "before the first line");
        let (l, r) = ("a\nb\n", "a\nb\nc\nd\n");
        assert_eq!(align_texts(l, r), [vec![(2, 2)], vec![]], "after the last line with text");
        // no final line break on one side: its last line differs, the other has one more
        let (l, r) = ("a\nb", "a\nb\n");
        assert_eq!(align_texts(l, r), [vec![(2, 1)], vec![]]);
    }

    #[test]
    fn empty_and_identical_files_need_no_more_rows_than_they_have() {
        for (l, r) in [("", ""), ("same\ntext\n", "same\ntext\n"), ("x", "x")] {
            assert_eq!(align_texts(l, r), [vec![], vec![]], "{l:?}");
        }
        // against an empty file, the other side's lines are all a hunk
        let (l, r) = ("", "one\ntwo\n");
        assert_eq!(align_texts(l, r), [vec![(0, 2)], vec![]]);
        assert_eq!(align_texts(r, l), [vec![], vec![(0, 2)]]);
    }

    #[test]
    fn a_line_wrapped_on_one_side_only_pads_the_other_after_it() {
        let long = "word ".repeat(12);
        let (l, r) = (format!("a\n{long}\nb\nc\n"), format!("a\n{long}\nb\nC\n"));
        let hs = hunks(&l, &r);
        let wrapped = |wl, wr| align(&Side::new(&l, wl), &Side::new(&r, wr), &hs);
        // wrapped at 20 columns on the left only: its 60 chars take 3 rows there
        assert_eq!(wrapped(Some(20), None), [vec![], vec![(2, 2)]]);
        // both wrapped, at different widths (panes of different widths)
        assert_eq!(wrapped(Some(20), Some(31)), [vec![], vec![(2, 1)]]);
        assert_eq!(wrapped(Some(31), Some(20)), [vec![(2, 1)], vec![]]);
        // inside a hunk too: the changed block's rows, not its lines, are matched
        let (l, r) = (format!("a\n{long}\nz\n"), "a\nshort\nz\n".to_string());
        let blocks = align(&Side::new(&l, Some(20)), &Side::new(&r, Some(20)), &hunks(&l, &r));
        assert_eq!(blocks, [vec![], vec![(2, 2)]]);
    }

    #[test]
    fn matched_folds_keep_both_sides_level() {
        let (l, r) = (
            "fn a() {\n    one\n    two\n    three\n}\nend\n",
            "fn a() {\n    one\n    TWO\n    2\n    three\n}\nend\n",
        );
        let hs = hunks(l, r);
        let (mut left, mut right) = (Side::new(l, None), Side::new(r, None));
        // the left folds its block; the right folds the range the left's maps to
        left.folds.fold_lines(&left.s.doc, 0, 3);
        let docs: [&Document; 2] = [&left.s.doc, &right.s.doc];
        let start = map_offset(&hs, docs, 0, left.s.line_end(0), false);
        let end = map_offset(&hs, docs, 0, left.s.line_end(3), true);
        assert_eq!((start, end), (right.s.line_end(0), right.s.line_end(4)));
        right.folds.fold_lines(&right.s.doc, 0, 4);
        let blocks = align(&left, &right, &hs);
        assert_eq!(blocks, [vec![], vec![]], "the hunk is folded away on both sides");
    }

    #[test]
    fn offsets_carry_char_for_char_outside_hunks_and_by_line_and_column_inside() {
        let (l, r) = ("a\nold\nb\n", "a\nnew\nnewer\nb\n");
        let hs = hunks(l, r);
        let (l, r) = (EditorState::from_text(l), EditorState::from_text(r));
        let docs: [&Document; 2] = [&l.doc, &r.doc];
        let map = |from, o, after| map_offset(&hs, docs, from, o, after);
        assert_eq!(hs[0].chars, [2..6, 2..12]);
        for o in 0..=2 {
            assert_eq!(map(0, o, false), o, "before the hunk");
        }
        assert_eq!(map(0, 3, false), 3, "inside: the same line and column");
        assert_eq!(map(0, 5, true), 5, "a line's end, not its line break");
        assert_eq!(map(1, 9, true), 5, "past the other's lines: its last line's end");
        assert_eq!(map(1, 11, false), 5, "past that line's text: its end");
        assert_eq!(map(1, 7, true), 3, "the other's last line, at the same column");
        assert_eq!(map(0, 6, false), 12, "its end: the other's");
        assert_eq!(map(0, 8, true), 14, "after it: shifted");
        assert_eq!(map(1, 14, false), 8, "and back");
        // a hunk with no lines on one side: its place goes to the other's start or end, and the
        // other's offsets inside it to the place, or for an end before the line break above it
        let (l, r) = ("a\nb\n", "a\nx\nb\n");
        let hs = hunks(l, r);
        let (l, r) = (EditorState::from_text(l), EditorState::from_text(r));
        let docs: [&Document; 2] = [&l.doc, &r.doc];
        let map = |from, o, after| map_offset(&hs, docs, from, o, after);
        assert_eq!(map(0, 2, false), 2);
        assert_eq!(map(0, 2, true), 4);
        assert_eq!(map(1, 2, false), 2);
        assert_eq!(map(1, 3, false), 2);
        assert_eq!(map(1, 2, true), 1);
        assert_eq!(map(1, 3, true), 1);
        assert_eq!(map(1, 4, true), 2, "its end: the place");
    }

    #[test]
    fn mapped_fold_ends_stay_before_the_whole_crlf_break() {
        for newline in ["\n", "\r\n"] {
            let left = format!("a{newline}x{newline}b{newline}");
            let right = format!("a{newline}b{newline}");
            let hs = hunks(&left, &right);
            let left = EditorState::from_text(&left);
            let right = EditorState::from_text(&right);
            let docs: [&Document; 2] = [&left.doc, &right.doc];
            let inside = left.line_start(1) + 1;
            assert_eq!(
                map_offset(&hs, docs, 0, inside, true),
                right.line_end(0),
                "an end in removed text stops before {newline:?}"
            );
        }
    }

    /// A random text and another made from it by adding, removing and changing lines.
    fn random_pair(seed: &mut u64) -> (String, String) {
        let words = ["a", "bb", "word", "longer words", "x y z w v u t", ""];
        let lines: Vec<String> = (0..1 + next(seed) % 12)
            .map(|_| (0..next(seed) % 4).map(|_| words[next(seed) % words.len()]).collect::<Vec<_>>().join(" "))
            .collect();
        let mut other = lines.clone();
        for _ in 0..next(seed) % 4 {
            let i = next(seed) % (other.len() + 1);
            match next(seed) % 3 {
                0 => other.insert(i, words[next(seed) % words.len()].repeat(3)),
                1 if i < other.len() => {
                    other.remove(i);
                }
                _ if i < other.len() => other[i].push_str(" changed"),
                _ => {}
            }
        }
        (lines.join("\n") + "\n", other.join("\n"))
    }

    /// No wrapping, or a random width.
    fn random_width(seed: &mut u64) -> Option<usize> {
        match next(seed) % 3 {
            0 => None,
            _ => Some(4 + next(seed) % 20),
        }
    }

    #[test]
    fn random_pairs_wrapped_at_random_widths_stay_level() {
        let mut seed = 0xa11e_9e5d_0dd5_eed5;
        for round in 0..150 {
            let (l, r) = random_pair(&mut seed);
            let (wl, wr) = (random_width(&mut seed), random_width(&mut seed));
            let hs = hunks(&l, &r);
            let (left, right) = (Side::new(&l, wl), Side::new(&r, wr));
            let blocks = align(&left, &right, &hs);
            // the padding goes at hunks' edges, between unchanged lines or at the ends
            let n = [left.s.line_count(), right.s.line_count()];
            let ends: Vec<(usize, usize)> = unchanged(&hs, n)
                .into_iter()
                .chain(hs.iter().map(|h| (h.lines[0].end, h.lines[1].end)))
                .chain(hs.iter().map(|h| (h.lines[0].start, h.lines[1].start)))
                .chain([(n[0], n[1])])
                .collect();
            for (side, b) in blocks.iter().enumerate() {
                for &(line, _) in b {
                    let at = |e: &(usize, usize)| if side == 0 { e.0 } else { e.1 };
                    assert!(ends.iter().any(|e| at(e) == line), "round {round}: {line}");
                }
            }
        }
    }

    /// Fold on `to` the text `from`'s folds map to, as a pair does; `side` is `from`'s.
    fn carry(from: &Side, to: &mut Side, side: usize, hunks: &[Hunk]) {
        let mut docs: [&Document; 2] = [&from.s.doc, &to.s.doc];
        docs.swap(0, side);
        let map = |o, after| map_offset(hunks, docs, side, o, after);
        let mapped = (from.folds.chars().into_iter()).map(|f| map(f.start, false)..map(f.end, true));
        to.folds.fold_many(&to.s.doc, mapped);
    }

    #[test]
    fn folds_at_a_hunk_with_no_lines_on_one_side_keep_both_level() {
        // the left's "x" and "y" are not on the right
        let (l, r) = ("a\nb\nx\ny\nc\nd\n", "a\nb\nc\nd\n");
        let hs = hunks(l, r);
        let cases: [(usize, Range<usize>); 4] = [
            // ending at the hunk's start: "b" and "x" joined
            (0, 3..4),
            // from the hunk's start to past its last line break: "c" joined to "x"
            (0, 4..9),
            // from inside the hunk to past it
            (0, 7..9),
            // from the right's place for the hunk, joining "d" to "c"
            (1, 4..7),
        ];
        for (side, fold) in cases {
            let mut sides = [Side::new(l, None), Side::new(r, None)];
            let [left, right] = &mut sides;
            let (from, to) = match side {
                0 => (left, right),
                _ => (right, left),
            };
            from.folds.fold(&from.s.doc, fold.clone());
            carry(from, to, side, &hs);
            let [left, right] = &sides;
            align(left, right, &hs);
        }
    }

    #[test]
    fn random_folds_carried_to_the_other_side_keep_both_level() {
        let mut seed = 0xf01d_ed9e_5eed_1234;
        for _ in 0..1000 {
            let (l, r) = random_pair(&mut seed);
            let hs = hunks(&l, &r);
            let (mut left, mut right) = (Side::new(&l, None), Side::new(&r, None));
            let len = left.s.len();
            for _ in 0..1 + next(&mut seed) % 3 {
                let a = next(&mut seed) % (len + 1);
                let b = a + next(&mut seed) % (len - a + 1);
                left.folds.fold(&left.s.doc, a..b);
            }
            carry(&left, &mut right, 0, &hs);
            // wrapped too, at random widths, each side's folds laid out
            for side in [&mut left, &mut right] {
                let w = random_width(&mut seed);
                side.wrap = w.map(|w| WrapMap::new(&side.s.doc, w, 4, &side.folds));
            }
            // a line joined to a display line above sits where that line's text takes it, which
            // differs when the display line wraps at another width or starts at another line of a
            // hunk: the lines below it are level
            align_lines(&left, &right, &hs, false);
        }
    }
}
