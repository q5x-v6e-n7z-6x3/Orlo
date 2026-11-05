//! Filler rows: rows with no text between a view's lines, as the shorter side of a side-by-side
//! diff is padded so that its unchanged lines stay level with the other side's.

use crate::buffer::{Change, DisplayPoint, DisplayRow, Document, Rows};
use std::ops::Range;

/// A view's filler rows, in blocks at line boundaries: sorted, with running totals, so a row
/// is found by binary search. They change no text: never saved, copied, searched or numbered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fillers {
    blocks: Vec<Block>,
    /// The rows of `blocks[..=i]`.
    totals: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Block {
    /// The rows go before this line; the line count puts them after the last line.
    line: usize,
    /// `line`'s start in chars, kept in step with the line through edits (the text's length
    /// for a block after the last line).
    at: usize,
    rows: usize,
    /// After the last line: every edit is before it, and none takes it away.
    end: bool,
}

impl Fillers {
    /// `rows` filler rows before each `line` given; a line given twice takes both counts.
    pub fn new(s: &Document, blocks: impl IntoIterator<Item = (usize, usize)>) -> Self {
        let mut list: Vec<(usize, usize)> = blocks.into_iter().filter(|b| b.1 > 0).collect();
        list.sort_unstable();
        let n = s.line_count();
        let mut out = Self::default();
        for (line, rows) in list {
            let line = line.min(n);
            match out.blocks.last_mut() {
                Some(b) if b.line == line => b.rows += rows,
                _ => out.blocks.push(Block { line, at: s.offset(line, 0), rows, end: line == n }),
            }
        }
        out.count();
        out
    }

    fn count(&mut self) {
        let mut total = 0;
        self.totals = (self.blocks.iter())
            .map(|b| {
                total += b.rows;
                total
            })
            .collect();
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// All the filler rows.
    pub fn total(&self) -> usize {
        self.totals.last().copied().unwrap_or(0)
    }

    /// (line, rows), sorted: the rows before each line.
    pub fn blocks(&self) -> Vec<(usize, usize)> {
        self.blocks.iter().map(|b| (b.line, b.rows)).collect()
    }

    /// The rows of the blocks before block `i`.
    fn before(&self, i: usize) -> usize {
        i.checked_sub(1).map_or(0, |j| self.totals[j])
    }

    /// Follow edits already made to the text: a block moves with the line it is before; an
    /// edit that takes that line's start away (it joins the line above) drops the block. Text
    /// typed at a line's start goes after its block.
    pub fn map(&mut self, changes: &[Change]) {
        if self.blocks.is_empty() {
            return;
        }
        for c in changes {
            let end = c.start + c.removed;
            let chars = c.inserted as isize - c.removed as isize;
            let lines = c.lines.1 as isize - c.lines.0 as isize;
            self.blocks.retain_mut(|b| {
                // the line break before the block's line went with the edit
                if c.start < b.at && b.at <= end && !b.end {
                    return false;
                }
                if end < b.at || (c.removed > 0 && end == b.at) || b.end {
                    b.at = b.at.saturating_add_signed(chars);
                    b.line = b.line.saturating_add_signed(lines);
                }
                true
            });
        }
        self.count();
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Blocks whose row was worked out on this thread: what a lookup costs.
    pub static RESOLVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The rows a view shows: `inner`'s, with `fillers` between them.
pub struct Padded<'a, R> {
    inner: R,
    fillers: &'a Fillers,
}

impl<'a, R: Rows> Padded<'a, R> {
    pub fn new(inner: R, fillers: &'a Fillers) -> Self {
        Self { inner, fillers }
    }

    /// The rows below the padding.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// The inner row block `i` is drawn above (`boundary_row`).
    fn block_row(&self, s: &Document, i: usize) -> usize {
        #[cfg(any(test, feature = "test-support"))]
        RESOLVED.set(RESOLVED.get() + 1);
        self.inner.boundary_row(s, self.fillers.blocks[i].line)
    }

    /// The display row block `i` starts at.
    fn block_start(&self, s: &Document, i: usize) -> usize {
        self.block_row(s, i) + self.fillers.before(i)
    }

    /// The last block that starts at or before display row `row`, with its start.
    fn block_at(&self, s: &Document, row: usize) -> Option<(usize, usize)> {
        let (mut lo, mut hi) = (0, self.fillers.blocks.len());
        let mut found = None;
        // block starts rise strictly: each block has rows
        while lo < hi {
            let mid = (lo + hi) / 2;
            let start = self.block_start(s, mid);
            if start <= row {
                found = Some((mid, start));
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        found
    }

    /// The inner row drawn as `row`; `None` for a filler row.
    pub fn inner_row(&self, s: &Document, row: DisplayRow) -> Option<DisplayRow> {
        if self.fillers.is_empty() {
            return Some(row);
        }
        match self.block_at(s, row.0) {
            None => Some(row),
            Some((i, start)) if row.0 < start + self.fillers.blocks[i].rows => None,
            Some((i, _)) => Some(DisplayRow(row.0 - self.fillers.totals[i])),
        }
    }

    /// Where inner row `row` is drawn.
    pub fn shown_row(&self, s: &Document, row: DisplayRow) -> DisplayRow {
        if self.fillers.is_empty() {
            return row;
        }
        // the blocks drawn above it: those whose row is at or before it
        let (mut lo, mut hi) = (0, self.fillers.blocks.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.block_row(s, mid) <= row.0 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        DisplayRow(row.0 + self.fillers.before(lo))
    }
}

impl<R: Rows> Rows for Padded<'_, R> {
    fn row_of(&self, s: &Document, offset: usize) -> DisplayPoint {
        let p = self.inner.row_of(s, offset);
        DisplayPoint { row: self.shown_row(s, p.row), ..p }
    }

    fn offset_at(&self, s: &Document, at: DisplayPoint) -> usize {
        if let Some(row) = self.inner_row(s, at.row) {
            return self.inner.offset_at(s, DisplayPoint { row, ..at });
        }
        // a filler row: the end of the text row above its block, or the start
        let (i, _) = self.block_at(s, at.row.0).expect("a filler row is in a block");
        match self.block_row(s, i).checked_sub(1) {
            Some(above) => (self.inner.row_bounds(s, DisplayRow(above))).map_or(0, |b| b.end),
            None => 0,
        }
    }

    fn row_bounds(&self, s: &Document, row: DisplayRow) -> Option<Range<usize>> {
        let row = self.inner_row(s, row)?;
        self.inner.row_bounds(s, row)
    }

    fn rows(&self, s: &Document) -> usize {
        self.inner.rows(s) + self.fillers.total()
    }

    fn skip(&self, s: &Document, offset: usize, forward: bool) -> usize {
        self.inner.skip(s, offset, forward)
    }

    fn first_row(&self, s: &Document, line: usize) -> DisplayRow {
        match line < s.line_count() {
            true => self.shown_row(s, self.inner.first_row(s, line)),
            false => DisplayRow(self.rows(s)),
        }
    }

    fn text_row(&self, s: &Document, row: DisplayRow, down: bool) -> Option<DisplayRow> {
        let n = self.rows(s);
        let mut row = row.0;
        // blocks drawn above the same inner row run on from one another
        loop {
            if row >= n {
                return None;
            }
            let Some((i, start)) = self.block_at(s, row) else {
                return Some(DisplayRow(row));
            };
            let end = start + self.fillers.blocks[i].rows;
            if row >= end {
                return Some(DisplayRow(row));
            }
            row = match down {
                true => end,
                false => start.checked_sub(1)?,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Cells, EditorState, Lines, Motion, Selection};
    use crate::fold::{Folded, Folds};
    use crate::viewport::WrapMap;

    /// xorshift: this crate has no random number generator.
    fn next(seed: &mut u64) -> usize {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 33) as usize
    }

    fn dp(row: usize, col: usize) -> DisplayPoint {
        DisplayPoint::new(row, col)
    }

    /// The rows drawn, worked out the slow way: each inner row, with a block's filler rows
    /// before the first inner row that starts at or after its line's start.
    fn model(inner: &dyn Rows, s: &Document, blocks: &[(usize, usize)]) -> Vec<Option<usize>> {
        let n = inner.rows(s);
        let mut out = Vec::new();
        let mut pending: Vec<(usize, usize)> = blocks.to_vec();
        pending.sort();
        let starts = |line: usize| match line < s.line_count() {
            true => s.line_start(line),
            false => usize::MAX,
        };
        for r in 0..n {
            let start = inner.row_bounds(s, DisplayRow(r)).unwrap().start;
            // a hidden line's block waits for the next row; a line's own starts at its first
            while pending.first().is_some_and(|&(l, _)| starts(l) <= start) {
                let (_, k) = pending.remove(0);
                out.extend(std::iter::repeat_n(None, k));
            }
            out.push(Some(r));
        }
        for (_, k) in pending {
            out.extend(std::iter::repeat_n(None, k));
        }
        out
    }

    /// Every `Rows` method of `padded` against `model`.
    fn check<R: Rows>(padded: &Padded<R>, s: &Document, model: &[Option<usize>], what: &str) {
        let inner = padded.inner();
        assert_eq!(padded.rows(s), model.len(), "{what}: rows");
        for (d, &m) in model.iter().enumerate() {
            let row = DisplayRow(d);
            assert_eq!(padded.inner_row(s, row), m.map(DisplayRow), "{what}: row {d}");
            let bounds = m.and_then(|r| inner.row_bounds(s, DisplayRow(r)));
            assert_eq!(padded.row_bounds(s, row), bounds, "{what}: bounds of {d}");
            // the text row above a filler row: a click there lands at its end
            let above = model[..d].iter().rev().find_map(|m| *m);
            for col in [0, 2, 7, 999] {
                let want = match m {
                    Some(r) => inner.offset_at(s, dp(r, col)),
                    None => above.map_or(0, |r| inner.row_bounds(s, DisplayRow(r)).unwrap().end),
                };
                assert_eq!(padded.offset_at(s, dp(d, col)), want, "{what}: {d}, {col}");
            }
            let down = (d..model.len()).find(|&e| model[e].is_some());
            let up = (0..=d).rev().find(|&e| model[e].is_some());
            assert_eq!(padded.text_row(s, row, true), down.map(DisplayRow));
            assert_eq!(padded.text_row(s, row, false), up.map(DisplayRow));
            if let Some(r) = m {
                assert_eq!(padded.shown_row(s, DisplayRow(r)), row, "{what}: shown {r}");
            }
        }
        for o in 0..=s.len() {
            let p = inner.row_of(s, o);
            let d = model.iter().position(|&m| m == Some(p.row.0)).unwrap();
            assert_eq!(padded.row_of(s, o), DisplayPoint { row: DisplayRow(d), ..p });
            assert_eq!(padded.skip(s, o, true), inner.skip(s, o, true));
            assert_eq!(padded.skip(s, o, false), inner.skip(s, o, false));
        }
        for line in 0..=s.line_count() {
            let want = match line < s.line_count() {
                true => {
                    let r = inner.first_row(s, line).0;
                    model.iter().position(|&m| m == Some(r)).unwrap()
                }
                false => model.len(),
            };
            assert_eq!(padded.first_row(s, line), DisplayRow(want), "{what}: line {line}");
        }
    }

    /// `check` for `blocks` over `inner`.
    fn check_blocks(inner: &dyn Rows, s: &Document, blocks: &[(usize, usize)], what: &str) {
        let fillers = Fillers::new(s, blocks.iter().copied());
        check(&Padded::new(inner, &fillers), s, &model(inner, s, blocks), what);
    }

    const TEXT: &str = "zero\n  one\n  two\nthree\n\tfour\nfive is a longer line of words\nsix";

    #[test]
    fn filler_rows_at_the_start_middle_and_end_answer_every_rows_method() {
        let s = EditorState::from_text(TEXT);
        let n = s.line_count();
        for blocks in [
            vec![],
            vec![(0, 2)],
            vec![(3, 1)],
            vec![(n, 3)],
            vec![(0, 1), (1, 2), (2, 1), (n, 2)],
            vec![(4, 2), (4, 1), (6, 3)],
            vec![(n - 1, 1), (n, 1)],
        ] {
            check_blocks(&Lines, &s.doc, &blocks, &format!("{blocks:?}"));
        }
    }

    #[test]
    fn filler_rows_beside_wrapped_rows_go_after_a_lines_last_row() {
        let s = EditorState::from_text(TEXT);
        let w = WrapMap::new(&s.doc, 8, 4, &Folds::NONE);
        // line 5 wraps to several rows: a block before line 6 follows all of them
        assert!(w.first_row(&s.doc, 6).0 - w.first_row(&s.doc, 5).0 > 2);
        for blocks in [vec![(6, 2)], vec![(5, 1), (6, 1)], vec![(0, 1), (7, 4)]] {
            check_blocks(&w, &s.doc, &blocks, &format!("{blocks:?}"));
        }
    }

    #[test]
    fn filler_rows_beside_folds_go_after_the_folded_row() {
        let s = EditorState::from_text(TEXT);
        let mut folds = Folds::default();
        folds.fold_lines(&s.doc, 0, 2);
        let folded = Folded::new(None, &folds);
        // lines 1 and 2 are folded into row 0: their blocks go after it, with line 3's
        for blocks in [vec![(1, 1)], vec![(2, 2), (3, 1)], vec![(0, 1), (1, 1)]] {
            check_blocks(&folded, &s.doc, &blocks, &format!("{blocks:?}"));
        }
        let fillers = Fillers::new(&s.doc, [(1, 1)]);
        let padded = Padded::new(&folded, &fillers);
        assert_eq!(padded.inner_row(&s.doc, DisplayRow(1)), None, "after the fold's row");
        assert_eq!(padded.row_of(&s.doc, s.line_start(3)).row, DisplayRow(2));
    }

    #[test]
    fn random_fillers_over_folds_and_wrapping_agree_with_the_slow_model() {
        let mut seed = 0x5eed_f111_e7ab_1e5d;
        for round in 0..60 {
            let text: String = (0..12)
                .map(|i| match next(&mut seed) % 4 {
                    0 => "\n".to_string(),
                    1 => format!("{}w{i} {}\n", " ".repeat(i % 3), "word ".repeat(i % 5)),
                    2 => format!("\tl{i}\n"),
                    _ => format!("l{i}\n"),
                })
                .collect();
            let s = EditorState::from_text(&text);
            let n = s.line_count();
            let mut folds = Folds::default();
            for _ in 0..next(&mut seed) % 3 {
                match next(&mut seed) % 2 {
                    0 => {
                        let h = next(&mut seed) % n;
                        folds.fold_lines(&s.doc, h, h + 1 + next(&mut seed) % 3);
                    }
                    _ => {
                        let a = next(&mut seed) % s.len().max(1);
                        folds.fold(&s.doc, a..a + 1 + next(&mut seed) % 12);
                    }
                }
            }
            let wrap = WrapMap::new(&s.doc, 6 + next(&mut seed) % 10, 4, &folds);
            let folded = Folded::new((round % 2 == 1).then_some(&wrap), &folds);
            let blocks: Vec<(usize, usize)> =
                (0..next(&mut seed) % 6).map(|_| (next(&mut seed) % (n + 1), 1 + next(&mut seed) % 3)).collect();
            let what = format!("round {round}: {blocks:?} over {text:?}");
            check_blocks(&folded, &s.doc, &blocks, &what);
        }
    }

    #[test]
    fn up_and_down_step_over_filler_rows_and_never_rest_on_one() {
        // rows: 2 fillers, zero, one, 3 fillers, two, three, 1 filler
        let text = "zero\none\ntwo\nthree";
        let s0 = EditorState::from_text(text);
        let fillers = Fillers::new(&s0.doc, [(0, 2), (2, 3), (4, 1)]);
        let padded = Padded::new(&Lines, &fillers);
        let mut s = EditorState::from_text(text);
        let mut go = |at: usize, motion: Motion| {
            s.set_cursor(at);
            s.move_cursors_in(motion, false, &padded, &Cells);
            s.cursor()
        };
        // "one" is 5..8, "two" 9..12, "three" 13..18
        assert_eq!(go(6, Motion::Down), 10, "over the three filler rows, same column");
        assert_eq!(go(10, Motion::Up), 6);
        assert_eq!(go(1, Motion::Up), 0, "only fillers above: the document's start");
        assert_eq!(go(15, Motion::Down), 18, "only fillers below: the document's end");
        assert_eq!(go(1, Motion::PageDown(6)), 14, "a page landing on a text row");
        assert_eq!(go(1, Motion::PageDown(2)), 10, "a page ending on fillers goes on");
        assert_eq!(go(15, Motion::PageUp(2)), 7, "back past them");
        // every place a caret can be is on a text row
        for o in 0..=s0.len() {
            let row = padded.row_of(&s0.doc, o).row;
            assert!(padded.row_bounds(&s0.doc, row).is_some(), "offset {o}");
        }
        // Add Next Line steps over them too
        let mut s = EditorState::from_text(text);
        s.set_cursor(6);
        s.select_lines(true, &padded, &Cells);
        assert_eq!(s.view.selections, [Selection::caret(6).in_rectangle(), Selection::caret(10).in_rectangle()]);
        s.set_cursor(10);
        s.select_lines(false, &padded, &Cells);
        assert_eq!(s.view.selections, [Selection::caret(6).in_rectangle(), Selection::caret(10).in_rectangle()]);
    }

    #[test]
    fn a_click_on_a_filler_row_lands_at_the_end_of_the_line_above() {
        let s = EditorState::from_text("zero\none\ntwo\nthree");
        let fillers = Fillers::new(&s.doc, [(0, 2), (2, 3), (4, 1)]);
        let padded = Padded::new(&Lines, &fillers);
        assert_eq!(padded.offset_at(&s.doc, dp(0, 3)), 0, "no line above: the start");
        assert_eq!(padded.offset_at(&s.doc, dp(4, 0)), 8, "the end of \"one\"");
        assert_eq!(padded.offset_at(&s.doc, dp(6, 99)), 8);
        assert_eq!(padded.offset_at(&s.doc, dp(9, 1)), s.len(), "after the last line");
        // a box drawn over filler rows selects on the text rows only
        let mut s2 = EditorState::from_text("zero\none\ntwo\nthree");
        s2.drag_box(&[], false, dp(3, 1), dp(7, 2), &padded);
        let ranges: Vec<_> = s2.view.selections.iter().map(|x| x.range()).collect();
        assert_eq!(ranges, [6..7, 10..11]);
    }

    #[test]
    fn fillers_merge_by_line_and_follow_edits() {
        let mut s = EditorState::from_text("a\nb\nc\nd\n");
        let mut f = Fillers::new(&s.doc, [(2, 1), (0, 0), (2, 2), (4, 1), (1, 1)]);
        assert_eq!(f.blocks(), [(1, 1), (2, 3), (4, 1)]);
        assert_eq!(f.total(), 5);
        assert!(!f.is_empty() && Fillers::new(&s.doc, [(1, 0)]).is_empty());
        // a line typed in above moves them down
        s.set_cursor(0);
        s.insert("x\n");
        f.map(&s.take_changes());
        assert_eq!(f.blocks(), [(2, 1), (3, 3), (5, 1)]);
        // joining "b" onto the line above takes its start: its block goes
        s.set_selection(s.line_end(1), s.line_start(2));
        s.insert("");
        f.map(&s.take_changes());
        assert_eq!(f.blocks(), [(2, 3), (4, 1)]);
        // typing within a line moves nothing
        s.set_cursor(s.line_start(2) + 1);
        s.insert("yy");
        f.map(&s.take_changes());
        assert_eq!(f.blocks(), [(2, 3), (4, 1)]);
        // undo puts the lines back, and the blocks with them, but not the one dropped
        s.undo();
        s.undo();
        s.undo();
        f.map(&s.take_changes());
        assert_eq!(f.blocks(), [(2, 3), (4, 1)]);
        assert_eq!(s.text(), "a\nb\nc\nd\n");
    }

    #[test]
    fn rows_after_the_last_line_stay_through_edits_to_it() {
        let mut s = EditorState::from_text("a\nb\nlast");
        let mut f = Fillers::new(&s.doc, [(1, 1), (3, 2)]);
        // the last line's text goes, and a line is typed in above
        s.set_selection(s.line_start(2), s.len());
        s.insert("");
        f.map(&s.take_changes());
        assert_eq!(f.blocks(), [(1, 1), (3, 2)]);
        s.set_cursor(0);
        s.insert("x\n");
        f.map(&s.take_changes());
        assert_eq!(f.blocks(), [(2, 1), (4, 2)], "still after the last line");
        assert_eq!(s.line_count(), 4);
    }

    #[test]
    fn a_lookup_reads_a_logarithm_of_the_blocks() {
        // 100,000 lines and 10,000 blocks: one row looked up resolves at most ~2 log2 blocks
        let s = EditorState::from_text(&"line\n".repeat(100_000));
        let fillers = Fillers::new(&s.doc, (0..10_000).map(|i| (i * 10, 1 + i % 3)));
        let padded = Padded::new(&Lines, &fillers);
        let before = RESOLVED.get();
        let row = padded.row_of(&s.doc, s.line_start(55_555)).row;
        assert_eq!(padded.inner_row(&s.doc, row), Some(DisplayRow(55_555)));
        assert!(RESOLVED.get() - before <= 2 * 15, "{}", RESOLVED.get() - before);
        // and no fillers cost nothing at all
        let none = Fillers::default();
        let plain = Padded::new(&Lines, &none);
        let before = RESOLVED.get();
        assert_eq!(plain.row_of(&s.doc, 77).row, Lines.row_of(&s.doc, 77).row);
        assert_eq!(RESOLVED.get(), before);
    }
}
