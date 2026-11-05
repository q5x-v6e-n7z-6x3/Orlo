//! The rows of a view with folds: buffer lines joined where folds hide their line breaks
//! (display lines), each fold drawn as a placeholder, unwrapped or wrapped by a `WrapMap` that
//! laid the display lines out, placeholders included.

use super::fold::Folds;
use crate::buffer::{DisplayPoint, DisplayRow, Document, Lines, Rows};
use crate::viewport::{char_width, col_after_at, has_tab, WrapMap};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// The char a fold's placeholder is laid out as, `PLACEHOLDER_COLS` of them: one column each,
/// no line break after it, drawn over as one marker.
pub const PLACEHOLDER: char = '\u{2026}';
pub const PLACEHOLDER_COLS: usize = 3;

/// A stretch of a display line: buffer text shown as it is, or a fold's hidden text shown as a
/// placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    Text(Range<usize>),
    Fold(Range<usize>),
}

impl Piece {
    /// Display chars: its text's, or a placeholder's.
    fn width(&self) -> usize {
        match self {
            Piece::Text(r) => r.len(),
            Piece::Fold(_) => PLACEHOLDER_COLS,
        }
    }
}

/// The text of a display line as wrapping, columns and painting take it: its visible chars,
/// each outermost fold `PLACEHOLDER_COLS` placeholder chars. It holds its pieces, not their
/// text: a frame over a 10 MB line with a fold reads only the chars it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayText {
    /// The pieces in order, each with the display char it starts at.
    pieces: Vec<(usize, Piece)>,
    /// Display chars in all.
    len: usize,
}

impl DisplayText {
    /// The display text of the buffer chars `range`, a display line (or the start of one).
    pub fn new(folds: &Folds, range: Range<usize>) -> Self {
        let mut out = Self { pieces: Vec::new(), len: 0 };
        let mut at = range.start;
        let text = |out: &mut Self, r: Range<usize>| {
            if !r.is_empty() {
                out.pieces.push((out.len, Piece::Text(r.clone())));
                out.len += r.len();
            }
        };
        for f in folds.top_in(range.clone()) {
            text(&mut out, at..f.start);
            out.pieces.push((out.len, Piece::Fold(f.clone())));
            out.len += PLACEHOLDER_COLS;
            at = f.end;
        }
        text(&mut out, at..range.end.max(at));
        out
    }

    /// The whole text, as wrapping lays it out.
    pub fn text(&self, s: &Document) -> String {
        let mut out = String::new();
        for (_, p) in &self.pieces {
            match p {
                Piece::Text(r) => out.extend(s.rope.slice(r.clone()).chunks()),
                Piece::Fold(_) => out.extend(std::iter::repeat_n(PLACEHOLDER, PLACEHOLDER_COLS)),
            }
        }
        out
    }

    /// The display chars from display char `from` on.
    fn chars<'a>(&'a self, s: &'a Document, from: usize) -> impl Iterator<Item = char> + 'a {
        self.spans(from..self.len).flat_map(move |(text, marks)| {
            // a piece's text, or its placeholder chars: one iterator type for both
            let (text, marks) = match text {
                Some(r) => (r, 0),
                None => (0..0, marks),
            };
            (s.rope.slice(text).chars()).chain(std::iter::repeat_n(PLACEHOLDER, marks))
        })
    }

    /// The pieces in display chars `range`, cut to it: each piece's buffer text, or `None` for
    /// placeholder chars, and how many display chars it is.
    fn spans(&self, range: Range<usize>) -> impl Iterator<Item = (Option<Range<usize>>, usize)> + '_ {
        let i = (self.pieces).partition_point(|(at, p)| at + p.width() <= range.start);
        (self.pieces[i..].iter()).take_while(move |(at, _)| *at < range.end).map(move |(at, p)| {
            let a = range.start.max(*at) - at;
            let b = range.end.min(at + p.width()) - at;
            match p {
                Piece::Text(r) => (Some(r.start + a..r.start + b), b - a),
                Piece::Fold(_) => (None, b - a),
            }
        })
    }

    /// The display column after display chars `range` drawn from column `col`: tabs to their
    /// stops, every other char a column. Text with no tab is counted, not read.
    fn col_after(&self, s: &Document, range: Range<usize>, col: usize) -> usize {
        self.spans(range).fold(col, |col, (text, n)| match text {
            Some(r) => col_after_at(s.rope.slice(r), s.tab_size, col),
            None => col + n,
        })
    }

    /// Of display chars `range` drawn from column `x0`, how many come before the one nearest
    /// display column `col` (inside a tab, its nearer edge). Text with no tab before the
    /// column is counted, not read.
    fn chars_to_col(&self, s: &Document, range: Range<usize>, x0: usize, col: usize) -> usize {
        let (col, mut x, mut i) = (col.max(x0), x0, 0);
        for (text, n) in self.spans(range) {
            let k = (col - x).min(n);
            match text.map(|r| s.rope.slice(r)) {
                Some(t) if has_tab(t.slice(..k)) => {
                    for ch in t.chars() {
                        let next = x + char_width(ch, x, s.tab_size);
                        if next > col {
                            return i + usize::from(next - col < col - x);
                        }
                        (i, x) = (i + 1, next);
                    }
                }
                _ if k < n => return i + k,
                _ => (i, x) = (i + n, x + n),
            }
        }
        i
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The piece display char `c` is in (the last for its end).
    fn piece_at(&self, c: usize) -> Option<&(usize, Piece)> {
        let i = self.pieces.partition_point(|(p, _)| *p <= c);
        self.pieces.get(i.checked_sub(1)?)
    }

    /// The display char `offset` is drawn before: hidden text at its fold's start, a fold's
    /// end past its placeholder.
    pub fn char_of(&self, offset: usize) -> usize {
        // the last piece starting at or before the offset
        let i = self.pieces.partition_point(|(_, p)| match p {
            Piece::Text(r) | Piece::Fold(r) => r.start <= offset,
        });
        let Some((c, p)) = i.checked_sub(1).map(|i| &self.pieces[i]) else {
            return 0;
        };
        match p {
            Piece::Text(r) => c + offset.min(r.end) - r.start,
            Piece::Fold(r) if offset < r.end => *c,
            Piece::Fold(_) => c + PLACEHOLDER_COLS,
        }
    }

    /// The offset display char `c` is at: inside a placeholder, the fold's start for its first
    /// half and its end for the rest.
    pub fn offset_of(&self, c: usize) -> usize {
        let Some((at, p)) = self.piece_at(c.min(self.len)) else {
            return 0;
        };
        match p {
            Piece::Text(r) => (r.start + c - at).min(r.end),
            Piece::Fold(r) if (c - at) * 2 < PLACEHOLDER_COLS => r.start,
            Piece::Fold(r) => r.end,
        }
    }

    /// The pieces of display chars `chars`, a row of the display line.
    pub fn pieces(&self, chars: Range<usize>) -> Vec<Piece> {
        let mut out = Vec::new();
        for (at, p) in &self.pieces {
            let (a, b) = (*at, at + p.width());
            if b <= chars.start || a >= chars.end {
                continue;
            }
            out.push(match p {
                Piece::Text(r) => {
                    let from = r.start + chars.start.saturating_sub(a);
                    let to = r.start + chars.end.min(b) - a;
                    Piece::Text(from..to)
                }
                Piece::Fold(r) => Piece::Fold(r.clone()),
            });
        }
        out
    }

    /// The display chars where placeholders start.
    pub fn placeholders(&self) -> impl Iterator<Item = usize> + '_ {
        (self.pieces.iter()).filter_map(|(at, p)| matches!(p, Piece::Fold(_)).then_some(*at))
    }
}

/// The buffer chars of the display line `line` is on: its first line's start to its last
/// line's end.
pub fn display_range(s: &Document, folds: &Folds, line: usize) -> Range<usize> {
    let (head, last) = folds.display_lines(line);
    s.line_start(head)..s.line_end(last)
}

/// What painting needs of a row: its display line's first buffer line, its buffer chars (the
/// hidden ones included), the column it is drawn from, whether it starts and ends its display
/// line, and its pieces.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub line: usize,
    pub chars: Range<usize>,
    pub x0: usize,
    pub first: bool,
    pub last: bool,
    pub pieces: Vec<Piece>,
}

/// A display line laid out as text: its first line and the text's version it was laid out
/// for, the text, and its buffer chars.
type LaidOut = ((usize, u64), Rc<DisplayText>, Range<usize>);

/// The rows a view shows below its filler rows: one per display line, or a wrapped view's rows
/// of its display lines.
pub struct Folded<'a> {
    wrap: Option<&'a WrapMap>,
    folds: &'a Folds,
    /// The display line last laid out as text: a frame asks for each of a wrapped display
    /// line's rows in turn.
    last: RefCell<Option<LaidOut>>,
}

impl<'a> Folded<'a> {
    pub fn new(wrap: Option<&'a WrapMap>, folds: &'a Folds) -> Self {
        Self { wrap, folds, last: RefCell::new(None) }
    }

    /// The rows without folds: `Lines` or the wrap map's own.
    fn plain(&self) -> &dyn Rows {
        match self.wrap {
            Some(w) => w,
            None => &Lines,
        }
    }

    /// Row `row`'s display line's first buffer line, and which of its rows it is.
    fn head(&self, s: &Document, row: usize) -> (usize, usize) {
        match self.wrap {
            Some(w) => w.line_of_row(DisplayRow(row)),
            None => (self.folds.line_of_display(row).min(s.line_count() - 1), 0),
        }
    }

    /// The display line headed by `head` as text, if it has folds.
    fn text(&self, s: &Document, head: usize) -> Option<(Rc<DisplayText>, Range<usize>)> {
        let key = (head, s.version);
        if let Some((k, dt, range)) = self.last.borrow().as_ref() {
            if *k == key {
                return Some((dt.clone(), range.clone()));
            }
        }
        let range = display_range(s, self.folds, head);
        self.folds.top_in(range.clone()).next()?;
        let dt = Rc::new(DisplayText::new(self.folds, range.clone()));
        *self.last.borrow_mut() = Some((key, dt.clone(), range.clone()));
        Some((dt, range))
    }

    /// The display chars of row `seg` of display text `dt`, and the column it is drawn from (a
    /// continuation row's indent is the display text's, as it was wrapped).
    fn row_chars(&self, s: &Document, head: usize, seg: usize, dt: &DisplayText) -> (Range<usize>, usize) {
        let len = dt.len();
        match self.wrap {
            Some(w) => {
                let b = w.breaks_of(head);
                let start = if seg == 0 { 0 } else { b[seg - 1].min(len) };
                let end = b.get(seg).map_or(len, |&e| e.min(len));
                let indent = match seg {
                    0 => 0,
                    _ => w.indent_of(dt.chars(s, 0)),
                };
                (start..end, indent)
            }
            None => (0..len, 0),
        }
    }

    /// The first row of the display line headed by `head`.
    fn head_row(&self, head: usize) -> usize {
        match self.wrap {
            Some(w) => w.first_row_of(head),
            None => self.folds.display_line(head),
        }
    }

    /// The existing fold/wrap row projection without measuring a source prefix's column.
    /// Decorations can then measure only the same clipped text that painting uses.
    pub fn row_for_offset(&self, s: &Document, offset: usize) -> DisplayRow {
        let offset = self.folds.hiding(offset).map_or(offset, |f| f.start).min(s.len());
        let line = s.rope.char_to_line(offset);
        let (head, _) = self.folds.display_lines(line);
        match self.text(s, head) {
            Some((dt, _)) => {
                let c = dt.char_of(offset);
                let seg = self.wrap.map_or(0, |w| w.breaks_of(head).partition_point(|&k| k < c));
                DisplayRow(self.head_row(head) + seg)
            }
            None => {
                self.wrap.map_or_else(|| DisplayRow(self.folds.display_line(line)), |w| w.row_for_offset(s, offset))
            }
        }
    }

    /// Row `row` as painting takes it.
    pub fn segment(&self, s: &Document, row: DisplayRow) -> Segment {
        let row = row.0.min(self.rows(s) - 1);
        let (head, seg) = self.head(s, row);
        let Some((dt, range)) = self.text(s, head) else {
            // no folds: a line, or a wrapped line's row, as it always was
            let (line, chars, x0) = match self.wrap {
                Some(w) => w.segment(s, DisplayRow(row)),
                None => (head, 0..s.line_end(head) - s.line_start(head), 0),
            };
            let ls = s.line_start(line);
            return Segment {
                line,
                first: chars.start == 0,
                last: ls + chars.end == s.line_end(line),
                chars: ls + chars.start..ls + chars.end,
                x0,
                pieces: vec![Piece::Text(ls + chars.start..ls + chars.end)],
            };
        };
        let (chars, x0) = self.row_chars(s, head, seg, &dt);
        Segment {
            line: head,
            first: chars.start == 0,
            last: chars.end == dt.len(),
            chars: match chars.end == dt.len() {
                true => dt.offset_of(chars.start)..range.end,
                false => dt.offset_of(chars.start)..dt.offset_of(chars.end),
            },
            x0,
            pieces: dt.pieces(chars),
        }
    }
}

impl Rows for Folded<'_> {
    fn row_of(&self, s: &Document, offset: usize) -> DisplayPoint {
        // hidden text is drawn at its fold's start
        let offset = self.folds.hiding(offset).map_or(offset, |f| f.start).min(s.len());
        let line = s.rope.char_to_line(offset);
        let (head, _) = self.folds.display_lines(line);
        let Some((dt, _)) = self.text(s, head) else {
            let p = self.plain().row_of(s, offset);
            return match self.wrap {
                Some(_) => p,
                // unwrapped, the line's row counts the lines joined above it
                None => DisplayPoint { row: DisplayRow(self.folds.display_line(line)), ..p },
            };
        };
        let c = dt.char_of(offset);
        let (seg, start) = match self.wrap {
            // an offset at a soft break is on the upper row
            Some(w) => {
                let b = w.breaks_of(head);
                let seg = b.partition_point(|&k| k < c);
                (seg, if seg == 0 { 0 } else { b[seg - 1] })
            }
            None => (0, 0),
        };
        let (chars, x0) = self.row_chars(s, head, seg, &dt);
        // the row's chars up to the offset's, from the column it is drawn at
        let before = chars.start..chars.start + (c - start).min(chars.len());
        DisplayPoint::new(self.head_row(head) + seg, dt.col_after(s, before, x0))
    }

    fn offset_at(&self, s: &Document, at: DisplayPoint) -> usize {
        let row = at.row.0.min(self.rows(s) - 1);
        let (head, seg) = self.head(s, row);
        let Some((dt, _)) = self.text(s, head) else {
            let p = DisplayPoint {
                row: DisplayRow(match self.wrap {
                    Some(_) => row,
                    None => head,
                }),
                ..at
            };
            return self.plain().offset_at(s, p);
        };
        let (chars, x0) = self.row_chars(s, head, seg, &dt);
        let i = dt.chars_to_col(s, chars.clone(), x0, at.col.0);
        s.snap_to_cluster(dt.offset_of(chars.start + i))
    }

    fn row_bounds(&self, s: &Document, row: DisplayRow) -> Option<Range<usize>> {
        Some(self.segment(s, row).chars)
    }

    fn rows(&self, s: &Document) -> usize {
        match self.wrap {
            Some(w) => w.rows(s),
            None => (s.line_count() - self.folds.joined_lines()).max(1),
        }
    }

    /// Out of hidden text: onward to its fold's end, or back to its start.
    fn skip(&self, _: &Document, offset: usize, forward: bool) -> usize {
        match self.folds.hiding(offset) {
            Some(f) if forward => f.end,
            Some(f) => f.start,
            None => offset,
        }
    }

    /// A line that heads its display line starts its first row; a joined one's filler rows go
    /// after the row it is drawn on.
    fn boundary_row(&self, s: &Document, line: usize) -> usize {
        match line < s.line_count() && !self.folds.is_joined(line) {
            true => self.head_row(line),
            false if line >= s.line_count() => self.rows(s),
            false => self.row_of(s, s.line_start(line)).row.0 + 1,
        }
    }

    /// A line's own first row; a joined line's is the row its start is drawn on.
    fn first_row(&self, s: &Document, line: usize) -> DisplayRow {
        if line >= s.line_count() {
            return DisplayRow(self.rows(s));
        }
        match self.folds.display_lines(line) {
            (head, _) if head == line => DisplayRow(self.head_row(head)),
            _ => self.row_of(s, s.line_start(line)).row,
        }
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::buffer::{Cells, EditorState, Motion, Selection};
    use crate::rendering::fold::tests::next;
    use crate::viewport::{char_at_col, expand_tabs, wrap_breaks};

    #[test]
    fn u07_metadata_row_projection_matches_columns_for_wraps_tabs_unicode_and_folds() {
        let s = EditorState::from_text("\t界e\u{301}👩\u{200d}💻 abc def\naa\nbb\ncc\n");
        for ranges in [Vec::new(), vec![3..6], vec![19..25]] {
            let f = folds(&s.doc, &ranges);
            for width in [None, Some(5), Some(12)] {
                let wrap = width.map(|w| WrapMap::new(&s.doc, w, s.doc.tab_size, &f));
                let rows = Folded::new(wrap.as_ref(), &f);
                for at in 0..=s.len() {
                    assert_eq!(
                        rows.row_for_offset(&s.doc, at),
                        rows.row_of(&s.doc, at).row,
                        "offset {at}, width {width:?}, folds {ranges:?}"
                    );
                }
            }
        }
        let large = EditorState::from_text(&"\t界e\u{301}😀".repeat(40_000));
        let f = Folds::default();
        let lazy = WrapMap::lazy(&large.doc, 80, large.doc.tab_size, &f);
        assert_eq!(lazy.row_for_offset(&large.doc, large.len()), DisplayRow(0));
        assert_eq!(Folded::new(Some(&lazy), &f).row_for_offset(&large.doc, large.len()), DisplayRow(0));
    }

    fn dp(row: usize, col: usize) -> DisplayPoint {
        DisplayPoint::new(row, col)
    }

    fn folds(s: &Document, ranges: &[Range<usize>]) -> Folds {
        let mut f = Folds::default();
        for r in ranges {
            f.fold(s, r.clone());
        }
        f
    }

    /// Where a caret at `at` goes by motion `m` over `rows`.
    fn go(s: &mut EditorState, rows: &Folded, at: usize, m: Motion) -> usize {
        s.set_cursor(at);
        s.move_cursors_in(m, false, rows, &Cells);
        s.cursor()
    }

    /// Each display line's text, and per display char (and past the last) the offset a click
    /// there gives and whether it is a placeholder's char after its first: built char by char,
    /// the slow way.
    fn model(s: &Document, f: &Folds) -> Vec<(String, Vec<(usize, bool)>)> {
        let text: Vec<char> = s.rope.chars().collect();
        let top: Vec<Range<usize>> = f.top_in(0..s.len() + 1).collect();
        let mut out = vec![(String::new(), Vec::new())];
        let mut i = 0;
        while i <= text.len() {
            let line = out.last_mut().unwrap();
            if let Some(t) = top.iter().find(|t| t.start == i) {
                // a placeholder: its first half at the fold's start, the rest at its end
                for k in 0..PLACEHOLDER_COLS {
                    line.0.push(PLACEHOLDER);
                    let at = if k * 2 < PLACEHOLDER_COLS { t.start } else { t.end };
                    line.1.push((at, k > 0));
                }
                i = t.end;
                continue;
            }
            line.1.push((i, false));
            match text.get(i) {
                Some('\n') => out.push((String::new(), Vec::new())),
                Some(c) => line.0.push(*c),
                None => {}
            }
            i += 1;
        }
        out
    }

    /// Every method of unwrapped folded rows against `model`.
    fn check(s: &Document, f: &Folds, what: &str) {
        let rows = Folded::new(None, f);
        let lines = model(s, f);
        assert_eq!(rows.rows(s), lines.len(), "{what}");
        for (r, (text, offsets)) in lines.iter().enumerate() {
            let bounds = rows.row_bounds(s, DisplayRow(r)).unwrap();
            let (first, last) = (offsets[0].0, offsets.last().unwrap().0);
            assert_eq!(bounds, first..last, "{what}: row {r}");
            let cols = expand_tabs(text, s.tab_size).1;
            for (c, &(o, inside)) in offsets.iter().enumerate() {
                let at = dp(r, cols[c]);
                assert_eq!(rows.offset_at(s, at), s.snap_to_cluster(o), "{what}: {at:?}");
                // a fold's start is drawn at its placeholder's first char, its end past it
                if !inside {
                    assert_eq!(rows.row_of(s, o), at, "{what}: offset {o}");
                }
            }
            // every column, inside a tab and past the row's end too: the nearer char edge
            for col in 0..cols[cols.len() - 1] + 3 {
                let o = offsets[char_at_col(&cols, col)].0;
                let at = dp(r, col);
                assert_eq!(rows.offset_at(s, at), s.snap_to_cluster(o), "{what}: {at:?}");
            }
        }
        // hidden offsets are drawn at their fold's start, and motions skip out of them
        for o in 0..=s.len() {
            if let Some(h) = f.hiding(o) {
                assert_eq!(rows.row_of(s, o), rows.row_of(s, h.start), "{what}: {o}");
                assert_eq!((rows.skip(s, o, true), rows.skip(s, o, false)), (h.end, h.start));
            }
        }
    }

    #[test]
    fn a_fold_inside_a_line_shows_a_placeholder_in_its_place() {
        // "let x = compute(a, b);": the arguments, 16..20, folded
        let s = EditorState::from_text("let x = compute(a, b);\nnext\n");
        let f = folds(&s.doc, &[16..20]);
        check(&s.doc, &f, "in a line");
        let rows = Folded::new(None, &f);
        assert_eq!(rows.rows(&s.doc), 3, "no line is joined");
        assert_eq!(rows.row_of(&s.doc, 16), dp(0, 16), "its start: before the placeholder");
        assert_eq!(rows.row_of(&s.doc, 18), dp(0, 16), "hidden: at its start");
        assert_eq!(rows.row_of(&s.doc, 20), dp(0, 19), "its end: after the placeholder");
        assert_eq!(rows.row_of(&s.doc, 22), dp(0, 21), "after it, shifted by what it hides");
        assert_eq!(rows.offset_at(&s.doc, dp(0, 17)), 16, "a click on its first half");
        assert_eq!(rows.offset_at(&s.doc, dp(0, 18)), 20, "and on the rest");
        // a motion steps over it as over one char
        let mut s = s;
        let mut go = |at, m| go(&mut s, &rows, at, m);
        assert_eq!(go(16, Motion::Right), 20);
        assert_eq!(go(20, Motion::Left), 16);
        assert_eq!(go(15, Motion::Right), 16);
        assert_eq!(go(25, Motion::Up), 2, "\"next\" (23..27) col 2, up: before the fold");
        let seg = rows.segment(&Document::from_text("let x = compute(a, b);\nnext\n"), DisplayRow(0));
        assert_eq!(seg.pieces, [Piece::Text(0..16), Piece::Fold(16..20), Piece::Text(20..22)]);
    }

    #[test]
    fn a_fold_across_line_breaks_joins_its_lines_into_one_display_line() {
        // "fn a() {" 0..8, "    body" 9..17, "}" 18, "next" 20..24: the body folded from
        // after the brace to before the closing one
        let s = EditorState::from_text("fn a() {\n    body\n}\nnext\n");
        let f = folds(&s.doc, &[8..18]);
        check(&s.doc, &f, "across lines");
        let rows = Folded::new(None, &f);
        assert_eq!(rows.rows(&s.doc), 3, "fn a() {{…}}, next and the empty last line");
        assert_eq!(rows.row_bounds(&s.doc, DisplayRow(0)), Some(0..19));
        assert_eq!(rows.row_of(&s.doc, 18), dp(0, 11), "the brace after the placeholder");
        assert_eq!(rows.first_row(&s.doc, 1), DisplayRow(0), "a joined line's start: the fold");
        assert_eq!(rows.first_row(&s.doc, 2), DisplayRow(0));
        assert_eq!(rows.first_row(&s.doc, 3), DisplayRow(1));
        let mut s = s;
        let mut go = |at, m| go(&mut s, &rows, at, m);
        assert_eq!(go(22, Motion::Up), 2, "\"next\" up to the folded line, same column");
        assert_eq!(go(2, Motion::Down), 22);
        assert_eq!(go(0, Motion::LineEnd), 19, "the display line's end, past the brace");
        assert_eq!(go(8, Motion::Right), 18);
    }

    #[test]
    fn nested_folds_show_the_outer_and_touching_folds_show_two_placeholders() {
        let s = EditorState::from_text("abcdefghijklmnop\n");
        // 4..6 inside 2..10, and 10..12 touching it
        let f = folds(&s.doc, &[2..10, 4..6, 10..12]);
        check(&s.doc, &f, "nested and touching");
        let rows = Folded::new(None, &f);
        let seg = rows.segment(&s.doc, DisplayRow(0));
        assert_eq!(seg.pieces, [Piece::Text(0..2), Piece::Fold(2..10), Piece::Fold(10..12), Piece::Text(12..16)]);
        assert_eq!(rows.row_of(&s.doc, 10), dp(0, 5), "between the two placeholders");
        assert_eq!(rows.row_of(&s.doc, 12), dp(0, 8));
    }

    #[test]
    fn random_folds_draw_as_the_slow_model_does() {
        let mut seed = 0xf01d_ab1e_5eed_0001;
        for round in 0..150 {
            let text: String = (0..1 + next(&mut seed) % 8)
                .map(|i| match next(&mut seed) % 4 {
                    0 => "\n".to_string(),
                    1 => format!("\t{i}\tx\n"),
                    _ => format!("{} word {i}\n", " ".repeat(next(&mut seed) % 4)),
                })
                .collect();
            let s = EditorState::from_text(&text);
            let mut f = Folds::default();
            for _ in 0..next(&mut seed) % 5 {
                let a = next(&mut seed) % s.len().max(1);
                f.fold(&s.doc, a..a + 1 + next(&mut seed) % 15);
            }
            check(&s.doc, &f, &format!("round {round}: {text:?} {:?}", f.chars()));
        }
    }

    /// Every row of a wrapped folded view: the rows cover the text once, in order, a
    /// placeholder never cut, each row no wider than the width.
    fn check_wrapped(s: &Document, f: &Folds, w: &WrapMap, width: usize, what: &str) {
        let rows = Folded::new(Some(w), f);
        let mut at = 0;
        for r in 0..rows.rows(s) {
            let seg = rows.segment(s, DisplayRow(r));
            assert!(seg.chars.start == at || seg.chars.start == at + 1, "{what}: row {r} {seg:?}");
            at = seg.chars.end;
            let cols: usize = seg.pieces.iter().map(Piece::width).sum();
            assert!(seg.x0 + cols <= width, "{what}: row {r} too wide: {seg:?}");
            for o in seg.chars.clone() {
                if f.hiding(o).is_none() && o > seg.chars.start {
                    assert_eq!(rows.row_of(s, o).row, DisplayRow(r), "{what}: offset {o}");
                }
            }
        }
        assert_eq!(at, s.len(), "{what}");
    }

    #[test]
    fn wrapping_lays_out_the_placeholder_and_never_cuts_it() {
        // a fold whose placeholder lands on the wrap width
        let s = EditorState::from_text(&format!("{} {}\nnext\n", "a".repeat(17), "b".repeat(30)));
        let f = folds(&s.doc, &[18..40]);
        let w = WrapMap::new(&s.doc, 20, 4, &f);
        check_wrapped(&s.doc, &f, &w, 20, "at the width");
        let rows = Folded::new(Some(&w), &f);
        // "aaaa… " breaks after the space: the placeholder and the rest go to the next row
        assert_eq!(rows.segment(&s.doc, DisplayRow(1)).pieces[0], Piece::Fold(18..40));
        let breaks = w.breaks_of(0).to_vec();
        let display = DisplayText::new(&f, 0..s.line_end(0)).text(&s.doc);
        assert_eq!(display, format!("{} {}{}", "a".repeat(17), "\u{2026}".repeat(3), "b".repeat(8)));
        assert_eq!(breaks, wrap_breaks(&display, 20, 4));
        // a run with no break cut inside a placeholder: the cut goes to the placeholder's start
        let s = EditorState::from_text(&"x".repeat(40));
        let f = folds(&s.doc, &[19..25]);
        let w = WrapMap::new(&s.doc, 20, 4, &f);
        check_wrapped(&s.doc, &f, &w, 20, "a run");
        assert_eq!(w.breaks_of(0)[0], 19);
        // the row after a cut moved back to a placeholder is wrapped from there, so it fits too
        let s = EditorState::from_text(&format!("{}{}", "x".repeat(18), "y".repeat(44)));
        let f = folds(&s.doc, &[18..22]);
        let w = WrapMap::new(&s.doc, 20, 4, &f);
        check_wrapped(&s.doc, &f, &w, 20, "after a moved cut");
        assert_eq!(w.breaks_of(0), [18, 38, 58]);
    }

    #[test]
    fn a_wrap_map_refolded_or_edited_matches_one_built_fresh() {
        let text: String =
            (0..80).map(|i| format!("{}line {i} {}\n", " ".repeat(i % 3), "word ".repeat(i % 9))).collect();
        let mut seed = 0xabcd_ef01_2345_6789;
        for round in 0..60 {
            let mut s = EditorState::from_text(&text);
            let mut f = Folds::default();
            let mut w = WrapMap::new(&s.doc, 24, 4, &f);
            for _ in 0..4 {
                let before = f.clone();
                match next(&mut seed) % 3 {
                    0 => {
                        let h = next(&mut seed) % s.line_count();
                        f.fold_lines(&s.doc, h, h + 1 + next(&mut seed) % 6);
                    }
                    1 => {
                        let a = next(&mut seed) % s.len();
                        f.fold(&s.doc, a..a + 1 + next(&mut seed) % 60);
                    }
                    _ => {
                        let o = next(&mut seed) % s.len();
                        f.unfold_at(&[o..o]);
                    }
                }
                w.refold(&s.doc, &f, &Folds::changed_lines(&before, &f, &s.doc));
                assert_eq!(w, WrapMap::new(&s.doc, 24, 4, &f), "round {round}: refold");
                check_wrapped(&s.doc, &f, &w, 24, &format!("round {round}"));
            }
            // edits: typed at carets, in folds and out of them
            for _ in 0..3 {
                let mut at: Vec<usize> = (0..1 + next(&mut seed) % 3).map(|_| next(&mut seed) % s.len()).collect();
                at.sort_unstable();
                at.dedup();
                s.set_selections(at.iter().map(|&o| Selection::caret(o)).collect());
                s.insert(["x", "\n", "ab cd ef"][next(&mut seed) % 3]);
                let changes = s.take_changes();
                f.map(&changes);
                w.update(&s.doc, &changes, &f);
                assert_eq!(w, WrapMap::new(&s.doc, 24, 4, &f), "round {round}: edit");
            }
        }
    }

    #[test]
    fn an_edit_opening_a_fold_then_adding_lines_inside_it_rewraps_each_line_once() {
        // the first "c" opens the fold over lines 0..=3; the second adds a line inside the
        // rows the first already re-wrapped
        let mut s = EditorState::from_text(&format!("H\nc\nc\n{}\nz\n", "q ".repeat(30)));
        let mut f = Folds::default();
        f.fold_lines(&s.doc, 0, 3);
        let mut w = WrapMap::new(&s.doc, 20, 4, &f);
        s.take_changes();
        s.replace_all("c", "x\ny", &crate::buffer::FindOpts::default());
        let changes = s.take_changes();
        f.map(&changes);
        w.update(&s.doc, &changes, &f);
        assert_eq!(w, WrapMap::new(&s.doc, 20, 4, &f));
    }

    #[test]
    fn carets_on_several_lines_step_over_their_folds_together() {
        // "ab(cd)ef" on three lines, each "(cd)" folded; then lines 1..=2 folded too
        let s = EditorState::from_text("ab(cd)ef\nab(cd)ef\nab(cd)ef\nz\n");
        let f = folds(&s.doc, &[2..6, 11..15, 20..24]);
        let rows = Folded::new(None, &f);
        let mut s = s;
        s.set_selections(vec![Selection::caret(2), Selection::caret(11), Selection::caret(20)]);
        s.move_cursors_in(Motion::Right, false, &rows, &Cells);
        let heads: Vec<usize> = s.view.selections.iter().map(|x| x.head).collect();
        assert_eq!(heads, [6, 15, 24], "each over its fold");
        s.move_cursors_in(Motion::Left, true, &rows, &Cells);
        let ranges: Vec<Range<usize>> = s.view.selections.iter().map(|x| x.range()).collect();
        assert_eq!(
            ranges,
            [2..6, 11..15, 24..24].map(|r| match r.start {
                24 => 20..24,
                _ => r,
            })
        );
        // with the lines after the first joined too: Down from it goes to "z"
        let mut f = f;
        f.fold(&s.doc, 8..26);
        let rows = Folded::new(None, &f);
        s.set_selections(vec![Selection::caret(1), Selection::caret(28)]);
        s.move_cursors_in(Motion::Down, false, &rows, &Cells);
        let heads: Vec<usize> = s.view.selections.iter().map(|x| x.head).collect();
        assert_eq!(heads, [28, 29], "line 0 down to \"z\"; \"z\" down to the end");
    }

    #[test]
    fn a_wrap_map_still_to_be_laid_out_joins_the_lines_folds_hide() {
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let s = EditorState::from_text(&text);
        let mut f = Folds::default();
        f.fold_lines(&s.doc, 10, 60);
        // word wrap turned on: every line one row until it is laid out, a hidden one none
        let w = WrapMap::lazy(&s.doc, 40, 4, &f);
        let rows = Folded::new(Some(&w), &f);
        assert_eq!(rows.rows(&s.doc), Folded::new(None, &f).rows(&s.doc));
        assert_eq!(rows.first_row(&s.doc, 61), DisplayRow(11));
        assert_eq!(rows.segment(&s.doc, DisplayRow(11)).line, 61);
    }
}
