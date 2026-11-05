//! Display columns: tabs expanded to tab stops, and soft wrap into display rows.

pub mod align;
pub mod filler;

use crate::buffer::{map_offset, Change, DisplayPoint, DisplayRow, Document, Rows};
use crate::rendering::fold::{Folds, PLACEHOLDER_COLS};
use crate::rendering::folded::{display_range, DisplayText};
use ropey::RopeSlice;
use std::borrow::Cow;
use std::cell::OnceCell;
use std::ops::Range;
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;

/// Columns `c` takes when it starts at display column `col`.
pub(crate) fn char_width(c: char, col: usize, tab_size: usize) -> usize {
    if c == '\t' {
        tab_size - col % tab_size
    } else {
        1
    }
}

/// Tabs expanded to the next tab stop; `cols[i]` is the display column of char `i` (len = chars + 1).
pub fn expand_tabs(line: &str, tab_size: usize) -> (Cow<'_, str>, Vec<usize>) {
    expand_tabs_at(line, tab_size, 0)
}

/// `expand_tabs` for text drawn from display column `x0` (a continuation row): tab stops count
/// from the line's left edge, `cols` from `x0`.
pub fn expand_tabs_at(line: &str, tab_size: usize, x0: usize) -> (Cow<'_, str>, Vec<usize>) {
    let mut cols = Vec::with_capacity(line.len() + 1);
    let mut col = x0;
    for c in line.chars() {
        cols.push(col - x0);
        col += char_width(c, col, tab_size);
    }
    cols.push(col - x0);
    if !line.contains('\t') {
        return (Cow::Borrowed(line), cols);
    }
    let mut out = String::with_capacity(line.len() + col);
    for (i, c) in line.chars().enumerate() {
        if c == '\t' {
            out.extend(std::iter::repeat_n(' ', cols[i + 1] - cols[i]));
        } else {
            out.push(c);
        }
    }
    (Cow::Owned(out), cols)
}

/// The char nearest display column `col`; a column inside a tab goes to the nearer edge.
pub fn char_at_col(cols: &[usize], col: usize) -> usize {
    let i = cols.partition_point(|&c| c <= col).saturating_sub(1);
    if i + 1 < cols.len() && cols[i + 1] - col < col - cols[i] {
        i + 1
    } else {
        i
    }
}

pub(crate) fn has_tab(chars: RopeSlice) -> bool {
    chars.chunks().any(|c| c.contains('\t'))
}

/// The display column after `chars`, a line from its start: a scan, and tabs expanded only if
/// it has any, so a caret on a 232k-char line costs no allocation.
pub fn col_after(chars: RopeSlice, tab_size: usize) -> usize {
    col_after_at(chars, tab_size, 0)
}

/// `col_after` for chars drawn from display column `col`.
pub(crate) fn col_after_at(chars: RopeSlice, tab_size: usize, col: usize) -> usize {
    if !has_tab(chars) {
        return col + chars.len_chars();
    }
    chars.chars().fold(col, |col, c| col + char_width(c, col, tab_size))
}

/// `char_at_col` on a line's chars, expanding tabs only up to `col`.
pub fn char_at(line: RopeSlice, tab_size: usize, col: usize) -> usize {
    let n = line.len_chars();
    if !has_tab(line.slice(..col.min(n))) {
        return col.min(n);
    }
    let mut x = 0;
    for (i, c) in line.chars().enumerate() {
        let next = x + char_width(c, x, tab_size);
        if next > col {
            return if next - col < col - x { i + 1 } else { i };
        }
        x = next;
    }
    n
}

/// Sublime's `wrap_width: 0`: the most columns n with (n + 1) em < the view's width, from the
/// first text column to the minimap (measured in Sublime 4215; `floor(width / em) - 1` is one
/// too many when the width is a multiple of em).
pub fn wrap_width(view: f32, em: f32) -> usize {
    ((view / em).ceil() as usize).saturating_sub(2).max(1)
}

/// East Asian wide chars and emoji draw about two columns of the editor font.
/// The main wide blocks, not all of Unicode's East Asian Width table; add ranges
/// if a script runs past the edge.
pub(crate) fn is_wide(c: char) -> bool {
    // most text is below the first wide block
    (c as u32) >= 0x1100
        && matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
}

/// Whether the grapheme cluster boundary between the neighbours `before` and `after` can be told
/// from the two alone: both are Latin, Han, kana or the like, which never join the char before
/// or after them (only a CR and a LF make one cluster). Emoji, accents, Hangul and the rest
/// need the chars around them too.
pub(crate) fn free_boundary(before: char, after: char) -> bool {
    let plain = |c: char| matches!(c as u32, 0..=0x2ff | 0x3000..=0x3029 | 0x3031..=0x3098 | 0x309b..=0x9fff);
    plain(before) && plain(after) && (before, after) != ('\r', '\n')
}

/// Columns `c` takes on screen when wrapping: wide chars count two, so a wrapped row of them
/// fits.
fn wrap_width_of(c: char, col: usize, tab_size: usize) -> usize {
    if is_wide(c) {
        2
    } else {
        char_width(c, col, tab_size)
    }
}

/// A continuation row's indent, capped so a deeply indented line still shows text.
fn wrap_indent(indent: usize, width: usize) -> usize {
    indent.min(width / 2)
}

/// Sublime breaks a line after whitespace and after `/ ( ) [ ] < >`, nowhere else.
fn breaks_after(c: char) -> bool {
    matches!(c, ' ' | '\t' | '/' | '(' | ')' | '[' | ']' | '<' | '>')
}

fn is_indent(c: &char) -> bool {
    matches!(c, ' ' | '\t')
}

/// Char indices where rows 2.. of `line` start when it is wrapped at `width` columns. A row
/// holds at most `width` columns including the whitespace after its last word; a run with no
/// break is cut at the width; continuation rows are indented like the line, and the indent
/// counts. A row always takes a character, so any input terminates.
/// Counts columns, so a proportional font or wide glyphs (CJK, emoji) can run past
/// the right edge; measure with the shaper if that matters.
pub fn wrap_breaks(line: &str, width: usize, tab_size: usize) -> Vec<usize> {
    wrap_breaks_keeping(line, width, tab_size, &[])
}

/// `wrap_breaks` that never cuts inside the `PLACEHOLDER_COLS` chars from each of `whole`
/// (sorted): a run cut there is cut at its start instead (past its end when it starts the row),
/// and the next row is wrapped from that cut.
pub fn wrap_breaks_keeping(line: &str, width: usize, tab_size: usize, whole: &[usize]) -> Vec<usize> {
    // an ASCII line (nearly every one) is walked as bytes: no char array of a 10 MB line
    match line.is_ascii() {
        true => breaks_in(line, line.as_bytes(), |b| b as char, width, tab_size, whole),
        false => {
            let chars: Vec<char> = line.chars().collect();
            breaks_in(line, &chars, |c| c, width, tab_size, whole)
        }
    }
}

/// `end` (a row's end in `chars`, past `min`) moved back to the start of the grapheme cluster
/// it falls inside, so that no row starts with an accent cut from its letter or half an emoji.
/// A cluster that fills the row (it starts before `min`) is cut at `end` like a run.
fn whole_clusters<T: Copy>(
    chars: &[T],
    ch: &impl Fn(T) -> char,
    line: &str,
    boundaries: &OnceCell<Vec<usize>>,
    min: usize,
    end: usize,
) -> usize {
    if end >= chars.len() || free_boundary(ch(chars[end - 1]), ch(chars[end])) {
        return end;
    }
    // Segment only once, and only when a wrap needs context; flags pair from the run's start.
    let boundaries = boundaries.get_or_init(|| {
        let mut at = 0;
        std::iter::once(0)
            .chain(line.graphemes(true).map(|g| {
                at += g.chars().count();
                at
            }))
            .collect()
    });
    let at = boundaries[boundaries.partition_point(|&at| at <= end) - 1];
    if at >= min {
        at
    } else {
        end
    }
}

fn breaks_in<T: Copy>(
    line: &str,
    chars: &[T],
    ch: impl Fn(T) -> char,
    width: usize,
    tab_size: usize,
    whole: &[usize],
) -> Vec<usize> {
    let lead = chars.iter().take_while(|&&c| is_indent(&ch(c))).count();
    // a line of whitespace alone has no indent: it is cut at the width like any run (Sublime
    // keeps it on one row and scrolls sideways, which a wrapped Orlo view does not do)
    let lead = if lead == chars.len() { 0 } else { lead };
    let lead_cols = chars[..lead].iter().fold(0, |col, &c| col + char_width(ch(c), col, tab_size));
    let indent = wrap_indent(lead_cols, width);
    let boundaries = OnceCell::new();
    let mut breaks = Vec::new();
    let mut start = 0;
    loop {
        let mut col = if start == 0 { 0 } else { indent };
        let (mut i, mut best) = (start, None);
        while i < chars.len() {
            col += wrap_width_of(ch(chars[i]), col, tab_size);
            if col > width {
                break;
            }
            i += 1;
            // the indent itself is never broken
            if i > lead && i < chars.len() && breaks_after(ch(chars[i - 1])) {
                best = Some(i);
            }
        }
        if i == chars.len() {
            return breaks;
        }
        // the first row keeps its indent and at least one character after it, unless the
        // indent alone is wider than the row: then it is cut like a run, and the text starts
        // on a row it fits
        let min = match start == 0 && lead_cols < width {
            true => lead + 1,
            false => start + 1,
        };
        let mut end = whole_clusters(chars, &ch, line, &boundaries, min, best.unwrap_or(i).max(min));
        let p = whole.partition_point(|&p| p < end);
        if let Some(&p) = p.checked_sub(1).map(|p| &whole[p]) {
            if end < p + PLACEHOLDER_COLS {
                end = if p > start { p } else { p + PLACEHOLDER_COLS };
            }
        }
        if end >= chars.len() {
            return breaks;
        }
        breaks.push(end);
        start = end;
    }
}

/// The text at the top of a view `top` rows down: the first visible row's first char, and how
/// far into that row the view is scrolled. A filler row on top keeps its place by the text row
/// below it, `into` counting back the rows up to the top, or past the last text row by that.
pub fn top_anchor(rows: &dyn Rows, s: &Document, top: f64) -> (usize, f32) {
    let top = top.max(0.);
    let row = DisplayRow((top as usize).min(rows.rows(s) - 1));
    match rows.row_bounds(s, row) {
        Some(b) => (b.start, top.fract() as f32),
        None => match (rows.text_row(s, row, true)).or_else(|| rows.text_row(s, row, false)) {
            Some(r) => {
                let start = rows.row_bounds(s, r).map_or(s.len(), |b| b.start);
                (start, (row.0 as f64 + top.fract() - r.0 as f64) as f32)
            }
            None => (s.len(), 0.),
        },
    }
}

/// The first visible row that puts `top_anchor`'s text back on top after the rows changed (a
/// re-wrap, word wrap on or off, a new font).
pub fn top_row(rows: &dyn Rows, s: &Document, (offset, into): (usize, f32)) -> f64 {
    let mut row = rows.row_of(s, offset.min(s.len())).row.0;
    // a continuation row's first char is drawn on the row above, but it opens this row
    let below = DisplayRow(row + 1);
    if row + 1 < rows.rows(s) && rows.row_bounds(s, below).is_some_and(|b| b.start == offset) {
        row += 1;
    }
    (row as f64 + into as f64).max(0.)
}

/// The span of the final text that `changes`, applied in order, may have touched.
fn touched(changes: &[Change]) -> Option<(usize, usize)> {
    let mut span: Option<(usize, usize)> = None;
    for c in changes {
        let (lo, hi) = span.map_or((c.start, c.start), |(lo, hi)| (map_offset(lo, &[*c]), map_offset(hi, &[*c])));
        span = Some((lo.min(c.start), hi.max(c.start + c.inserted)));
    }
    span
}

/// Lines a `WrapMap` chunk holds at most: an edit counts the rows of its chunk and the starts
/// of the chunks, not of every line below it (a wrapped log of millions of lines).
const CHUNK: usize = 1024;

/// The display rows of a wrapped view: where each line breaks, and each line's first row,
/// kept a chunk of lines at a time.
#[derive(Debug)]
pub struct WrapMap {
    width: usize,
    tab_size: usize,
    chunks: Vec<Chunk>,
    /// Per chunk, its first line and its first row.
    starts: Vec<(usize, usize)>,
    /// New whenever the rows change: what a side-by-side view compares to know.
    generation: u64,
}

/// The next `WrapMap::generation`, unique across maps: a map built afresh is a change too.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Debug)]
struct Chunk {
    /// Per line, the char indices (in the line) where its rows 2.. start; for a line that heads
    /// a display line with folds on it, indices in its display text (`DisplayText`).
    breaks: Vec<Vec<usize>>,
    /// Per line, joined onto the display line above it by a fold: no rows of its own.
    joined: Vec<bool>,
    /// How many of its lines are joined.
    joins: usize,
    /// Per line, its first row counted from the chunk's.
    first_row: Vec<usize>,
    /// Per line, still broken for an older width (or not at all): a resize re-wraps the lines
    /// on screen at once and the rest a slice per frame, so dragging a window edge over a 60k-line
    /// file does not re-wrap all of it every frame.
    stale: Vec<bool>,
    rows: usize,
}

impl Chunk {
    fn count(&mut self) {
        #[cfg(any(test, feature = "test-support"))]
        COUNTED.set(COUNTED.get() + self.breaks.len());
        self.first_row.clear();
        let (mut row, mut joins) = (0, 0);
        for (b, &joined) in self.breaks.iter().zip(&self.joined) {
            self.first_row.push(row);
            row += if joined { 0 } else { b.len() + 1 };
            joins += usize::from(joined);
        }
        (self.rows, self.joins) = (row, joins);
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Lines whose rows were counted on this thread: how much an update did.
    pub static COUNTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Lines in chunks of at most `CHUNK`, cut evenly (sizes one apart, so none under half of it
/// unless there is one chunk), their rows counted.
fn chunked(lines: Vec<(Vec<usize>, bool)>, stale: Vec<bool>) -> Vec<Chunk> {
    let k = lines.len().div_ceil(CHUNK).max(1);
    let (size, longer) = (lines.len() / k, lines.len() % k);
    let mut out = Vec::with_capacity(k);
    let (mut lines, mut stale) = (lines.into_iter(), stale.into_iter());
    loop {
        let size = size + usize::from(out.len() < longer);
        let (b, joined): (Vec<Vec<usize>>, Vec<bool>) = lines.by_ref().take(size).unzip();
        if b.is_empty() {
            return out;
        }
        let mut c = Chunk {
            stale: stale.by_ref().take(b.len()).collect(),
            breaks: b,
            joined,
            joins: 0,
            first_row: Vec::new(),
            rows: 0,
        };
        c.count();
        out.push(c);
    }
}

/// The same rows, however the lines fall into chunks.
#[cfg(any(test, feature = "test-support"))]
impl PartialEq for WrapMap {
    fn eq(&self, o: &Self) -> bool {
        fn lines(m: &WrapMap) -> impl Iterator<Item = ((&Vec<usize>, &bool), &bool)> {
            (m.chunks.iter()).flat_map(|c| c.breaks.iter().zip(&c.stale).zip(&c.joined))
        }
        (self.width, self.tab_size) == (o.width, o.tab_size) && lines(self).eq(lines(o))
    }
}

/// Line `line`'s entry in a wrap map: where the rows 2.. of the display line it heads start,
/// or that a fold joins it onto the line above (no rows of its own). A display line with folds
/// is wrapped as its display text, placeholders included; a placeholder is never cut, a break
/// that would fall inside one going to its start (or past it, when it starts the row).
fn wrap_line(s: &Document, folds: &Folds, line: usize, width: usize, tab_size: usize) -> (Vec<usize>, bool) {
    if folds.is_empty() {
        return (wrap_breaks(&s.line(line), width, tab_size), false);
    }
    if folds.is_joined(line) {
        return (Vec::new(), true);
    }
    let range = display_range(s, folds, line);
    if folds.top_in(range.clone()).next().is_none() {
        return (wrap_breaks(&s.line(line), width, tab_size), false);
    }
    let dt = DisplayText::new(folds, range);
    let whole: Vec<usize> = dt.placeholders().collect();
    (wrap_breaks_keeping(&dt.text(s), width, tab_size, &whole), false)
}

impl WrapMap {
    /// The rows of `s` wrapped at `width` columns; with `folds`, of its display lines.
    pub fn new(s: &Document, width: usize, tab_size: usize, folds: &Folds) -> Self {
        let n = s.line_count();
        let lines = (0..n).map(|l| wrap_line(s, folds, l, width, tab_size));
        Self::build(width, tab_size, chunked(lines.collect(), vec![false; n]))
    }

    /// Every line one row until `rewrap` or `settle` gets to it, and a line `folds` join to the
    /// one above none.
    pub fn lazy(s: &Document, width: usize, tab_size: usize, folds: &Folds) -> Self {
        let n = s.line_count();
        let mut lines = vec![(Vec::new(), false); n];
        for run in folds.hidden_lines(s) {
            lines[run].iter_mut().for_each(|l| l.1 = true);
        }
        Self::build(width, tab_size, chunked(lines, vec![true; n]))
    }

    fn build(width: usize, tab_size: usize, chunks: Vec<Chunk>) -> Self {
        let mut map = Self { width, tab_size, chunks, starts: Vec::new(), generation: 0 };
        map.recount(0);
        map
    }

    /// Wrapped afresh: the text changed past what an update can follow.
    fn rebuild(&mut self, s: &Document, folds: &Folds) {
        *self = Self::new(s, self.width, self.tab_size, folds);
    }

    /// The chunks' starts from chunk `c` on, after its rows (or its lines) changed.
    fn recount(&mut self, c: usize) {
        self.generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.starts.truncate(c);
        let (mut line, mut row) = self.end();
        for ch in &self.chunks[c..] {
            self.starts.push((line, row));
            (line, row) = (line + ch.breaks.len(), row + ch.rows);
        }
    }

    /// The chunk holding `line` and its index there.
    fn loc(&self, line: usize) -> (usize, usize) {
        let c = self.starts.partition_point(|s| s.0 <= line).saturating_sub(1);
        (c, line - self.starts[c].0)
    }

    /// Where line `line`'s rows 2.. start.
    pub(crate) fn breaks_of(&self, line: usize) -> &[usize] {
        let (c, i) = self.loc(line);
        &self.chunks[c].breaks[i]
    }

    /// Whether line `line` is joined onto the display line above it.
    fn joined_of(&self, line: usize) -> bool {
        let (c, i) = self.loc(line);
        self.chunks[c].joined[i]
    }

    pub(crate) fn first_row_of(&self, line: usize) -> usize {
        let (c, i) = self.loc(line);
        self.starts[c].1 + self.chunks[c].first_row[i]
    }

    /// Take a new width or tab size; each line keeps its rows until it is re-wrapped.
    pub fn resize(&mut self, width: usize, tab_size: usize) {
        (self.width, self.tab_size) = (width, tab_size);
        for c in &mut self.chunks {
            c.stale.fill(true);
        }
    }

    /// Re-wrap the stale lines among `lines`.
    pub fn rewrap(&mut self, s: &Document, lines: Range<usize>, folds: &Folds) {
        let end = lines.end.min(self.lines());
        self.rewrap_until(s, lines.start.min(end)..end, None, folds);
    }

    /// Re-wrap stale lines until `deadline` (at least one); true when none are left.
    pub fn settle(&mut self, s: &Document, deadline: Instant, folds: &Folds) -> bool {
        let first = self.chunks.iter().position(|c| c.stale.contains(&true));
        if let Some(c) = first {
            let from = self.starts[c].0;
            self.rewrap_until(s, from..self.lines(), Some(deadline), folds);
        }
        self.settled()
    }

    /// Re-wrap the stale lines in `lines`, stopping once `deadline` passes (after a line).
    fn rewrap_until(&mut self, s: &Document, lines: Range<usize>, deadline: Option<Instant>, folds: &Folds) {
        if lines.is_empty() {
            return;
        }
        let (first, _) = self.loc(lines.start);
        let mut touched = None;
        'chunks: for c in first..self.chunks.len() {
            let base = self.starts[c].0;
            let ch = &mut self.chunks[c];
            let from = lines.start.max(base) - base;
            let to = (lines.end - base).min(ch.breaks.len());
            let mut changed = false;
            for i in from..to {
                if std::mem::take(&mut ch.stale[i]) {
                    let (b, joined) = wrap_line(s, folds, base + i, self.width, self.tab_size);
                    (ch.breaks[i], ch.joined[i]) = (b, joined);
                    changed = true;
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        ch.count();
                        touched = touched.or(Some(c));
                        break 'chunks;
                    }
                }
            }
            if changed {
                ch.count();
                touched = touched.or(Some(c));
            }
            if base + ch.breaks.len() >= lines.end {
                break;
            }
        }
        if let Some(c) = touched {
            self.recount(c);
        }
    }

    /// Changes whenever the rows do, and differs between maps.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn lines(&self) -> usize {
        self.end().0
    }

    /// The line and the row after the last chunk with a start.
    fn end(&self) -> (usize, usize) {
        let last = self.starts.len().saturating_sub(1);
        self.starts.get(last).map_or((0, 0), |s| (s.0 + self.chunks[last].breaks.len(), s.1 + self.chunks[last].rows))
    }

    pub fn settled(&self) -> bool {
        !self.chunks.iter().any(|c| c.stale.contains(&true))
    }

    /// Made for this width and tab size, with a row list for every line of `s`.
    pub fn fits(&self, s: &Document, width: usize, tab_size: usize) -> bool {
        (self.width, self.tab_size) == (width, tab_size) && self.lines() == s.line_count()
    }

    /// Lines `a..=b` as they were, replaced by `fresh` (just wrapped): the chunks they were in
    /// are cut again and counted, then the chunks' starts.
    fn splice(&mut self, a: usize, b: usize, fresh: Vec<(Vec<usize>, bool)>) {
        let ((mut ca, mut ia), (mut cb, ib)) = (self.loc(a), self.loc(b));
        let mut to = self.starts[cb].0 - self.starts[ca].0 + ib;
        // left under half full, they take in a neighbour: deletions leave no trail of tiny chunks
        let end = self.starts.get(cb + 1).map_or(self.lines(), |s| s.0);
        if end - self.starts[ca].0 - (to + 1 - ia) + fresh.len() < CHUNK / 2 {
            if cb + 1 < self.chunks.len() {
                cb += 1;
            } else if ca > 0 {
                ca -= 1;
                let n = self.chunks[ca].breaks.len();
                (ia, to) = (ia + n, to + n);
            }
        }
        let (mut lines, mut stale) = (Vec::new(), Vec::new());
        for ch in self.chunks.drain(ca..=cb) {
            lines.extend(ch.breaks.into_iter().zip(ch.joined));
            stale.extend(ch.stale);
        }
        let n = fresh.len();
        lines.splice(ia..=to, fresh);
        stale.splice(ia..=to, std::iter::repeat_n(false, n));
        let new = chunked(lines, stale);
        self.chunks.splice(ca..ca, new);
        self.recount(ca);
    }

    /// Lines `first..=last` of the text as it is, which were lines `first..=old_last` of this
    /// map, grown to whole display lines as they were here and as `folds` has them now: an
    /// edit to any line of one re-wraps all of it, and a fold the edit opened lets its lines go.
    fn widen(
        &self,
        s: &Document,
        folds: &Folds,
        (mut first, mut last, mut old_last): (usize, usize, usize),
    ) -> (usize, usize, usize) {
        if folds.is_empty() && !self.any_joined() {
            return (first, last, old_last);
        }
        // the lines above the range are the same here and in the text, and so are the lines
        // after it, one for one
        while first > 0 && (self.joined_of(first) || folds.is_joined(first)) {
            first -= 1;
        }
        while old_last + 1 < self.lines()
            && last + 1 < s.line_count()
            && (self.joined_of(old_last + 1) || folds.is_joined(last + 1))
        {
            (last, old_last) = (last + 1, old_last + 1);
        }
        (first, last, old_last)
    }

    /// Lines `first..=last` of the text, which were lines `first..=old_last` here, widened to
    /// whole display lines (`widen`) and wrapped again; the last line wrapped.
    fn rewrap_lines(&mut self, s: &Document, folds: &Folds, lines: (usize, usize, usize)) -> usize {
        let (first, last, old_last) = self.widen(s, folds, lines);
        let fresh = (first..=last).map(|l| wrap_line(s, folds, l, self.width, self.tab_size)).collect();
        self.splice(first, old_last, fresh);
        last
    }

    /// Whether any line is joined onto another.
    fn any_joined(&self) -> bool {
        self.chunks.iter().any(|c| c.joins > 0)
    }

    /// Re-wrap the lines `changes` touched (their whole display lines); `s` and `folds` already
    /// have them.
    pub fn update(&mut self, s: &Document, changes: &[Change], folds: &Folds) {
        if self.update_each(s, changes, folds) {
            return;
        }
        let Some((lo, hi)) = touched(changes) else {
            return;
        };
        let len = s.len();
        let first = s.rope.char_to_line(lo.min(len));
        let last = s.rope.char_to_line(hi.min(len));
        let old_last = last as isize + self.lines() as isize - s.line_count() as isize;
        if old_last < first as isize || old_last as usize >= self.lines() {
            self.rebuild(s, folds);
            return;
        }
        self.rewrap_lines(s, folds, (first, last, old_last as usize));
    }

    /// Re-wrap the display lines over `lines` (sorted) after the folds changed: their rows,
    /// placeholders and joined lines.
    pub fn refold(&mut self, s: &Document, folds: &Folds, lines: &[Range<usize>]) {
        if self.lines() != s.line_count() {
            self.rebuild(s, folds);
            return;
        }
        // from the last range up, so the ones above keep their lines
        for r in lines.iter().rev() {
            let n = s.line_count();
            let (a, b) = (r.start.min(n - 1), r.end.saturating_sub(1).clamp(r.start, n - 1));
            self.rewrap_lines(s, folds, (a, b.max(a), b.max(a)));
        }
    }

    /// `update` for one ascending or descending run of changes (typing at many carets, Replace
    /// All, a line command): each change's own lines re-wrapped, so carets on lines 1 and
    /// 60,000 re-wrap two lines, not 60,000. False for anything else.
    fn update_each(&mut self, s: &Document, changes: &[Change], folds: &Folds) -> bool {
        let up = changes.windows(2).all(|w| w[1].start >= w[0].start + w[0].inserted);
        let down = changes.windows(2).all(|w| w[1].start + w[1].removed <= w[0].start);
        if changes.is_empty() || !(up || down) {
            return false;
        }
        // each change, lowest first, at its place in the text as it is now
        let placed: Vec<(usize, Change)> = match up {
            // later changes lie after an earlier one: its start is already where it ends up
            true => changes.iter().map(|c| (c.start, *c)).collect(),
            false => {
                let mut shift = 0isize;
                (changes.iter().rev())
                    .map(|c| {
                        let at = (c.start as isize + shift) as usize;
                        shift += c.inserted as isize - c.removed as isize;
                        (at, *c)
                    })
                    .collect()
            }
        };
        // the lines up to `done` already hold the text as it is now: an earlier change widened
        // to a fold it opened may have wrapped past this change's own lines
        let mut done = 0;
        for (at, c) in placed {
            let l = s.rope.char_to_line(at.min(s.len()));
            let gone = (l + c.lines.0).max(done);
            let new = gone - c.lines.0 + c.lines.1;
            if gone >= self.lines() || new >= s.line_count() {
                self.rebuild(s, folds);
                return true;
            }
            done = self.rewrap_lines(s, folds, (l, new, gone));
        }
        if self.lines() != s.line_count() {
            self.rebuild(s, folds);
        }
        true
    }

    /// (line, segment) of display row `row`.
    pub fn line_of_row(&self, row: DisplayRow) -> (usize, usize) {
        let row = row.0;
        let c = self.starts.partition_point(|s| s.1 <= row).saturating_sub(1);
        let ch = &self.chunks[c];
        let r = row - self.starts[c].1;
        let i = ch.first_row.partition_point(|&x| x <= r).saturating_sub(1);
        (self.starts[c].0 + i, (r - ch.first_row[i]).min(ch.breaks[i].len()))
    }

    /// Where continuation rows of display text `text` start, as `indent` for a line.
    pub(crate) fn indent_of(&self, text: impl IntoIterator<Item = char>) -> usize {
        let mut col = 0;
        for c in text {
            if !is_indent(&c) {
                return wrap_indent(col, self.width);
            }
            col += char_width(c, col, self.tab_size);
        }
        0
    }

    /// Where continuation rows start: the line's leading whitespace in columns (at most half
    /// the width), none for a line of whitespace alone.
    pub fn indent(&self, s: &Document, line: usize) -> usize {
        let mut col = 0;
        let mut chars = s.rope.line(line).chars().peekable();
        while let Some(c) = chars.next() {
            if !is_indent(&c) {
                // the line break, a stray CRLF's too, as `Document::line` cuts it for
                // `wrap_breaks`
                let end = c == '\n' || (c == '\r' && chars.peek() == Some(&'\n'));
                return if end { 0 } else { wrap_indent(col, self.width) };
            }
            col += char_width(c, col, self.tab_size);
        }
        0
    }

    /// Row `row`'s line, its chars within the line (no break), and the column it is drawn at.
    pub fn segment(&self, s: &Document, row: DisplayRow) -> (usize, Range<usize>, usize) {
        let (line, seg) = self.line_of_row(row);
        let b = self.breaks_of(line);
        let n = s.line_end(line) - s.line_start(line);
        let start = if seg == 0 { 0 } else { b[seg - 1].min(n) };
        let end = b.get(seg).map_or(n, |&e| e.min(n));
        let x0 = if seg == 0 { 0 } else { self.indent(s, line) };
        (line, start..end, x0)
    }

    /// The row containing an offset, without expanding or copying its text for a column.
    /// An offset at a soft break belongs to the upper row.
    pub fn row_for_offset(&self, s: &Document, offset: usize) -> DisplayRow {
        let (line, col) = s.point(offset);
        DisplayRow(self.first_row_of(line) + self.breaks_of(line).partition_point(|&k| k < col))
    }

    /// Row `row`'s text and its char columns from `x0`.
    fn row_cols(&self, s: &Document, row: DisplayRow) -> (usize, Vec<usize>, usize) {
        let (line, r, x0) = self.segment(s, row);
        let ls = s.line_start(line);
        let text = s.rope.slice(ls + r.start..ls + r.end).to_string();
        (ls + r.start, expand_tabs_at(&text, self.tab_size, x0).1, x0)
    }
}

impl Rows for WrapMap {
    fn row_of(&self, s: &Document, offset: usize) -> DisplayPoint {
        let row = self.row_for_offset(s, offset);
        let (start, cols, x0) = self.row_cols(s, row);
        let i = offset.min(s.len()).saturating_sub(start);
        DisplayPoint::new(row.0, x0 + cols[i.min(cols.len() - 1)])
    }

    fn offset_at(&self, s: &Document, at: DisplayPoint) -> usize {
        let row = DisplayRow(at.row.0.min(self.rows(s) - 1));
        let (start, cols, x0) = self.row_cols(s, row);
        s.snap_to_cluster(start + char_at_col(&cols, at.col.0.saturating_sub(x0)))
    }

    fn row_bounds(&self, s: &Document, row: DisplayRow) -> Option<Range<usize>> {
        let (line, r, _) = self.segment(s, row);
        let ls = s.line_start(line);
        Some(ls + r.start..ls + r.end)
    }

    fn rows(&self, _: &Document) -> usize {
        self.end().1.max(1)
    }

    fn first_row(&self, s: &Document, line: usize) -> DisplayRow {
        DisplayRow(match line < self.lines() {
            true => self.first_row_of(line),
            false => self.rows(s),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::buffer::{DisplayPoint, DisplayRow, EditorState, Rows};
    use std::time::Instant;

    fn dp(row: usize, col: usize) -> DisplayPoint {
        DisplayPoint::new(row, col)
    }

    fn wrap(s: &EditorState, width: usize) -> WrapMap {
        WrapMap::new(&s.doc, width, 4, &Folds::NONE)
    }

    /// `map` brought through `s`'s edits since its last `take_changes`.
    fn update(map: &mut WrapMap, s: &mut EditorState) {
        let changes = s.take_changes();
        map.update(&s.doc, &changes, &Folds::NONE);
    }

    /// Edits at several carets, each tried on its own.
    fn edits() -> Vec<fn(&mut EditorState)> {
        vec![
            |s| s.type_text("x"),
            |s| s.newline(),
            |s| s.backspace(),
            |s| s.insert("a\nb\nc"),
            |s| s.delete_line(),
            |s| s.duplicate_line(),
            |s| s.join_lines(),
        ]
    }

    #[test]
    fn a_wrap_map_updated_edit_by_edit_matches_one_built_fresh() {
        let line = |i: usize| format!("{i} {}\n", "word ".repeat(i % 40));
        let text: String = (0..300).map(line).collect();
        let mut edits = edits();
        edits.push(|s| {
            s.replace_all("word", "w\nw", &crate::buffer::FindOpts::default());
        });
        edits.push(|s| s.move_line_down());
        for (k, edit) in edits.into_iter().enumerate() {
            let mut s = EditorState::from_text(&text);
            let mut w = wrap(&s, 30);
            let carets = [3, 150, 299].map(|l| crate::buffer::Selection::caret(s.line_start(l) + 2));
            s.set_selections(carets.to_vec());
            s.take_changes();
            edit(&mut s);
            update(&mut w, &mut s);
            assert_eq!(w, wrap(&s, 30), "edit {k}");
        }
    }

    #[test]
    fn edits_across_chunks_keep_the_rows_of_a_fresh_wrap() {
        // three chunks and a bit; carets at their seams and both ends
        let line = |i: usize| format!("{i} {}\n", "word ".repeat(i % 13));
        let text: String = (0..3 * CHUNK + 5).map(line).collect();
        for (k, edit) in edits().into_iter().enumerate() {
            let mut s = EditorState::from_text(&text);
            let mut w = wrap(&s, 30);
            let seams = (w.starts.iter().skip(1)).flat_map(|&(l, _)| [l - 1, l]);
            let ends = [0, s.line_count() - 2];
            let carets: Vec<_> =
                (seams.chain(ends)).map(|l| crate::buffer::Selection::caret(s.line_start(l) + 1)).collect();
            assert_eq!(carets.len(), 8, "three seams");
            s.set_selections(carets);
            s.take_changes();
            edit(&mut s);
            update(&mut w, &mut s);
            assert_eq!(w, wrap(&s, 30), "edit {k}");
            // every row maps to its line and back, across the chunks' seams
            for row in (0..w.rows(&s.doc)).step_by(7) {
                let (line, seg) = w.line_of_row(DisplayRow(row));
                assert!(line < s.line_count());
                assert_eq!(w.first_row_of(line) + seg, row, "edit {k} row {row}");
            }
        }
        // a selection over two whole chunks deleted: they go, their neighbours join
        let mut s = EditorState::from_text(&text);
        let mut w = wrap(&s, 30);
        s.set_selection(s.line_start(10), s.line_start(2 * CHUNK + 10));
        s.take_changes();
        s.backspace();
        update(&mut w, &mut s);
        assert_eq!(w, wrap(&s, 30));
    }

    #[test]
    fn no_chunk_is_under_half_however_many_lines() {
        // 525,823 lines in chunks of ceil(n/k) left 511 for the last
        for n in [1_025, 2_049, 525_823, 1_000_000] {
            let sizes: Vec<usize> =
                chunked(vec![(Vec::new(), false); n], vec![false; n]).iter().map(|c| c.breaks.len()).collect();
            assert_eq!(sizes.iter().sum::<usize>(), n);
            let (lo, hi) = (sizes.iter().min().unwrap(), sizes.iter().max().unwrap());
            assert!(*lo >= CHUNK / 2 && *hi <= CHUNK && hi - lo <= 1, "{n}: {lo}..{hi}");
        }
        assert_eq!(chunked(Vec::new(), Vec::new()).len(), 0);
    }

    #[test]
    fn chunks_emptied_by_deletions_join_their_neighbours() {
        let text: String = (0..4 * CHUNK).map(|i| format!("{i} word word\n")).collect();
        let mut s = EditorState::from_text(&text);
        let mut w = wrap(&s, 30);
        // each chunk loses all but a few lines, one deletion at a time, from the last up
        for c in (0..4).rev() {
            s.set_selection(s.line_start(c * CHUNK + 3), s.line_start((c + 1) * CHUNK));
            s.take_changes();
            s.backspace();
            update(&mut w, &mut s);
            assert_eq!(w, wrap(&s, 30));
        }
        assert_eq!(w.chunks.len(), 1, "12 lines in one chunk");
        // and lines typed in again are cut into even chunks, none small
        s.set_cursor(s.len());
        s.insert(&text);
        update(&mut w, &mut s);
        let sizes: Vec<usize> = w.chunks.iter().map(|c| c.breaks.len()).collect();
        assert!(sizes.iter().all(|&n| n >= CHUNK / 2), "{sizes:?}");
        assert_eq!(w, wrap(&s, 30));
    }

    #[test]
    fn wrapped_rows_of_wide_chars_and_deep_indents_fit_the_width() {
        // 60 CJK chars draw 120 columns: rows of 20 at width 40
        let cjk = "漢".repeat(60);
        assert_eq!(wrap_breaks(&cjk, 40, 4), vec![20, 40]);
        // an indent past the width is cut like a run, and continuation rows start at half
        // the width: every row fits, and the text is on screen
        let deep = format!("{}{}", " ".repeat(50), "word ".repeat(10));
        let b = wrap_breaks(&deep, 40, 4);
        let chars: Vec<char> = deep.chars().collect();
        let mut starts = vec![0];
        starts.extend(&b);
        for (k, &a) in starts.iter().enumerate() {
            let end = starts.get(k + 1).copied().unwrap_or(chars.len());
            let x0 = if k == 0 { 0 } else { 20 };
            assert!(x0 + (end - a) <= 40, "row {k} runs past the edge");
        }
        let s = EditorState::from_text(&deep);
        assert_eq!(wrap(&s, 40).indent(&s.doc, 0), 20);
    }

    #[test]
    fn a_blank_line_ending_in_a_stray_crlf_wraps_inside_the_width() {
        // an LF file with one CRLF line: the CR stays in the buffer
        let s = EditorState::from_text(&format!("{}\r\nx", " ".repeat(30)));
        let map = wrap(&s, 10);
        for row in 0..map.rows(&s.doc) {
            let (_, r, x0) = map.segment(&s.doc, DisplayRow(row));
            assert!(x0 + r.len() <= 10, "row {row} runs past the edge: {x0} + {r:?}");
        }
    }

    /// Sublime's probe line at width 40: rows [0,36) [36,76) [76,111) [111,150) [150,190) …, 300 chars.
    pub(crate) fn probe_line() -> String {
        format!("{} {} {} {} {}", "a".repeat(35), "b".repeat(39), "c".repeat(34), "d".repeat(38), "e".repeat(150))
    }

    #[test]
    fn breaks_after_spaces_and_counts_the_trailing_space() {
        assert_eq!(wrap_breaks(&format!("{} bbbb cc", "a".repeat(35)), 40, 4), vec![36]);
        assert!(wrap_breaks(&format!("{} bbbb", "a".repeat(35)), 40, 4).is_empty());
        assert_eq!(wrap_breaks(&format!("{}  cc", "a".repeat(38)), 40, 4), vec![40], "two trailing spaces still fit");
        assert_eq!(wrap_breaks(&probe_line(), 40, 4), vec![36, 76, 111, 150, 190, 230, 270]);
    }

    #[test]
    fn breaks_after_slash_and_brackets_but_not_hyphen() {
        assert_eq!(wrap_breaks("aaaa/bbbb", 6, 4), vec![5]);
        assert_eq!(wrap_breaks("aaaa(bbbb", 6, 4), vec![5]);
        assert_eq!(wrap_breaks("aaaa>bbbb", 6, 4), vec![5]);
        assert_eq!(wrap_breaks("aaaa-bbbb", 6, 4), vec![6]);
        assert_eq!(wrap_breaks("aaaa.bbbb", 6, 4), vec![6]);
    }

    #[test]
    fn an_unbreakable_run_is_cut_at_the_width() {
        assert_eq!(wrap_breaks(&"x".repeat(300), 40, 4), vec![40, 80, 120, 160, 200, 240, 280]);
        assert_eq!(wrap_breaks(&format!("{} cc", "a".repeat(40)), 40, 4), vec![40]);
        assert_eq!(wrap_breaks(&format!("{} cc", "a".repeat(41)), 40, 4), vec![40]);
    }

    #[test]
    fn a_word_that_fits_on_the_next_row_moves_whole() {
        let line = format!("{} {}", "a".repeat(30), "b".repeat(20));
        assert_eq!(wrap_breaks(&line, 40, 4), vec![31]);
    }

    #[test]
    fn continuation_rows_keep_the_indent_and_it_counts() {
        let line = format!("{}{}", " ".repeat(8), "y".repeat(100));
        assert_eq!(wrap_breaks(&line, 40, 4), vec![40, 72, 104]);
    }

    #[test]
    fn indent_wider_than_the_row_continues_at_half_the_width() {
        let line = format!("{}abc", " ".repeat(45));
        // the indent is cut at the width, so "abc" shows on the next row, from column 20
        assert_eq!(wrap_breaks(&line, 40, 4), vec![40]);
    }

    #[test]
    fn a_whitespace_only_line_is_cut_at_the_width_from_column_0() {
        assert_eq!(wrap_breaks(&" ".repeat(100), 40, 4), vec![40, 80]);
        assert_eq!(wrap_breaks(&"\t".repeat(30), 40, 4), vec![10, 20]);
        let s = EditorState::from_text(&format!("{}\n    {}", " ".repeat(100), "x".repeat(50)));
        let map = wrap(&s, 40);
        assert_eq!(map.segment(&s.doc, DisplayRow(1)), (0, 40..80, 0), "no indent: it is all indent");
        assert_eq!(map.segment(&s.doc, DisplayRow(4)), (1, 40..54, 4), "the next line keeps its own");
    }

    #[test]
    fn spaces_past_the_width_start_the_next_row() {
        // Sublime 4215: rows fill with spaces to the width
        let a = |n: usize| "a".repeat(n);
        let cases = [
            (format!("{}    cc", a(38)), vec![40]),
            (format!("{}        cc", a(36)), vec![40]),
            (format!("{}{}bb", a(30), " ".repeat(20)), vec![40]),
            (format!("{}  cc", a(39)), vec![40]),
            (format!("{}    bbbb cc", a(35)), vec![39]),
            (format!("{}{}y", "x".repeat(45), " ".repeat(10)), vec![40]),
            (format!("  {}    cc", a(36)), vec![40]),
        ];
        for (line, rows) in cases {
            assert_eq!(wrap_breaks(&line, 40, 4), rows, "{line:?}");
        }
    }

    #[test]
    fn wrap_width_is_sublimes() {
        // (view width, em) -> chars per row, measured in Sublime 4215
        for (w, em, n) in [
            (1105., 7., 156),
            (637., 7., 89),
            (721., 7., 101),
            (966., 7., 136),
            (700., 7., 98),
            (701., 7., 99),
            (707., 7., 99),
            (842., 8., 104),
            (840., 9., 92),
        ] {
            assert_eq!(wrap_width(w, em), n, "{w} / {em}");
        }
        assert_eq!(wrap_width(5., 7.), 1, "never under one column");
    }

    #[test]
    fn tabs_count_to_their_stops() {
        let line = format!("\t{}", "x".repeat(40));
        assert_eq!(wrap_breaks(&line, 40, 4), vec![37]);
        assert_eq!(wrap_breaks(&line, 40, 8), vec![33]);
    }

    #[test]
    fn empty_blank_tiny_and_huge_lines_terminate() {
        assert!(wrap_breaks("", 40, 4).is_empty());
        assert_eq!(wrap_breaks("abcd", 1, 4), vec![1, 2, 3]);
        assert_eq!(wrap_breaks("abcd", 0, 4), vec![1, 2, 3]);
        assert_eq!(wrap_breaks("ab cd", 3, 4), vec![3]);
        let huge = "word ".repeat(46_400);
        assert_eq!(wrap_breaks(&huge, 100, 4).len(), 2319, "20 words a row");
    }

    #[test]
    fn a_wrapped_row_never_starts_inside_a_grapheme_cluster() {
        // a decomposed é, a ZWJ family, a skin-toned thumb and a flag, each laid across the
        // width at every offset
        let clusters =
            ["e\u{301}", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}", "\u{1f44d}\u{1f3fd}", "\u{1f1fa}\u{1f1f8}"];
        for cluster in clusters {
            for lead in 0..12 {
                let line = format!("{}{cluster} tail words", "x".repeat(lead));
                let chars: Vec<char> = line.chars().collect();
                for width in [10, 12, 14] {
                    let starts: Vec<usize> =
                        (line.grapheme_indices(true)).map(|(b, _)| line[..b].chars().count()).collect();
                    for k in wrap_breaks(&line, width, 4) {
                        assert!(
                            starts.contains(&k),
                            "{line:?} at {width}: a row starts inside a cluster at {k} of {}",
                            chars.len()
                        );
                    }
                }
            }
        }
        // a cluster wider than the row is still cut at the width, so every row fits
        let zalgo = format!("a{}b", "\u{301}".repeat(30));
        assert!(wrap_breaks(&zalgo, 8, 4).windows(2).all(|w| w[1] - w[0] <= 8));
    }

    #[test]
    fn wrapping_an_unbroken_non_latin_line_visits_chars_linearly() {
        for n in [8_000, 16_000, 32_000] {
            let chars = vec!['ب'; n];
            let visits = std::cell::Cell::new(0);
            let breaks = breaks_in(
                &chars.iter().collect::<String>(),
                &chars,
                |c| {
                    visits.set(visits.get() + 1);
                    c
                },
                80,
                4,
                &[],
            );
            assert_eq!(breaks, (80..chars.len()).step_by(80).collect::<Vec<_>>());
            assert!(visits.get() <= chars.len() * 8, "wrapping {} chars visited {} chars", chars.len(), visits.get());
        }
    }

    #[test]
    fn long_non_latin_runs_keep_whole_clusters_at_every_wrap() {
        for (unit, width, step) in [
            ("ب", 80, 80),
            ("가", 80, 40),
            ("e\u{301}", 79, 78),
            ("\u{1f1fa}\u{1f1f8}", 79, 78),
            ("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}", 80, 50),
        ] {
            let line = unit.repeat(4_000);
            let n = line.chars().count();
            assert_eq!(
                wrap_breaks(&line, width, 4),
                (step..n).step_by(step).collect::<Vec<_>>(),
                "unit {unit:?} at width {width}"
            );
        }
        // The byte fast path still treats CRLF as one cluster when it fits on a row.
        assert_eq!(wrap_breaks("ab\r\ncd", 3, 4), vec![2, 5]);
    }

    #[test]
    fn caret_at_a_break_is_on_the_upper_row() {
        let s = EditorState::from_text(&probe_line());
        let map = wrap(&s, 40);
        assert_eq!(map.row_of(&s.doc, 36), dp(0, 36));
        assert_eq!(map.row_of(&s.doc, 37), dp(1, 1));
        assert_eq!(map.row_bounds(&s.doc, DisplayRow(1)), Some(36..76));
        assert_eq!(map.offset_at(&s.doc, dp(1, 0)), 36, "a click at the row start");
        assert_eq!(map.rows(&s.doc), 8);
    }

    #[test]
    fn continuation_rows_count_the_indent_in_their_columns() {
        let s = EditorState::from_text(&format!("{}{}\nnext", " ".repeat(8), "y".repeat(100)));
        let map = wrap(&s, 40);
        assert_eq!(map.line_of_row(DisplayRow(2)), (0, 2));
        assert_eq!(map.line_of_row(DisplayRow(4)), (1, 0));
        assert_eq!(map.indent(&s.doc, 0), 8);
        assert_eq!(map.row_of(&s.doc, 41), dp(1, 9));
        assert_eq!(map.offset_at(&s.doc, dp(1, 3)), 40, "inside the indent: the row start");
        assert_eq!(map.offset_at(&s.doc, dp(1, 20)), 52);
        assert_eq!(map.row_of(&s.doc, 110), dp(4, 1));
    }

    #[test]
    fn the_text_on_top_stays_on_top_when_the_rows_change() {
        use crate::buffer::Lines;
        let s = EditorState::from_text(&format!("{}\nshort\nthird\n", probe_line()));
        let (w40, w20) = (wrap(&s, 40), wrap(&s, 20));
        // wrapped at 40, line 0 has 8 rows: line 1 is row 8
        let top = top_anchor(&w40, &s.doc, 8.5);
        assert_eq!(top, (301, 0.5));
        assert_eq!(top_row(&Lines, &s.doc, top), 1.5, "wrap turned off");
        assert_eq!(top_row(&w40, &s.doc, top_anchor(&Lines, &s.doc, 1.25)), 8.25, "wrap turned on");
        // row 1 at width 40 starts at the break 36, which opens row 2 at width 20
        assert_eq!(top_anchor(&w40, &s.doc, 1.), (36, 0.));
        assert_eq!(w20.row_bounds(&s.doc, DisplayRow(2)).unwrap().start, 36);
        assert_eq!(top_row(&w20, &s.doc, (36, 0.)), 2., "a continuation row's start opens it");
        assert_eq!(top_row(&Lines, &s.doc, (36, 0.)), 0., "unwrapped: the line holding it");
        assert_eq!(top_anchor(&w40, &s.doc, 99.), (s.len(), 0.), "past the end: the last row");
    }

    #[test]
    fn filler_rows_on_top_keep_their_place_when_the_rows_change() {
        use crate::buffer::Lines;
        use crate::viewport::filler::{Fillers, Padded};
        let s = EditorState::from_text(&format!("{}\nshort\nthird", probe_line()));
        let w40 = wrap(&s, 40);
        // five filler rows above the first line, three after the last
        let fillers = Fillers::new(&s.doc, [(0, 5), (3, 3)]);
        let (lines, wrapped) = (Padded::new(Lines, &fillers), Padded::new(&w40, &fillers));
        for top in [0., 2.5, 4.] {
            let anchor = top_anchor(&lines, &s.doc, top);
            assert_eq!(anchor.0, 0, "the first line below them");
            assert_eq!(top_row(&wrapped, &s.doc, anchor), top, "wrap turned on");
        }
        // below the last line: as far past it
        let third = s.doc.line_start(2);
        assert_eq!(top_anchor(&lines, &s.doc, 9.), (third, 2.));
        let row = wrapped.first_row(&s.doc, 2).0 as f64;
        assert_eq!(top_row(&wrapped, &s.doc, (third, 2.)), row + 2.);
    }

    #[test]
    fn rows_stay_inside_the_text_for_rows_a_frame_old() {
        use crate::buffer::Lines;
        // a painted frame can name rows that an edit since then removed
        let mut s = EditorState::from_text(&format!("{}\n{}", probe_line(), probe_line()));
        let mut map = wrap(&s, 40);
        let old_rows = map.rows(&s.doc);
        s.select_all();
        s.insert("ab");
        update(&mut map, &mut s);
        for row in 0..old_rows + 3 {
            for rows in [&map as &dyn Rows, &Lines] {
                let b = rows.row_bounds(&s.doc, DisplayRow(row)).unwrap();
                assert!(b.start <= b.end && b.end <= s.len(), "row {row}: {b:?}");
                assert!(rows.offset_at(&s.doc, dp(row, 999)) <= s.len());
            }
        }
    }

    #[test]
    fn update_matches_a_rebuild_after_edits() {
        let text = format!("{}\nshort\n\t{}\n{}", probe_line(), "x".repeat(90), "word ".repeat(30));
        let mut s = EditorState::from_text(&text);
        let mut map = wrap(&s, 40);
        let edits: [(usize, &str); 6] = [
            (10, "\n\n"),
            (0, "zz "),
            (305, "a much longer insertion that wraps itself "),
            (400, "\n"),
            (3, ""),
            (50, "q\nr\ns"),
        ];
        for (at, text) in edits {
            let at = at.min(s.len());
            s.set_selection(at, (at + 2).min(s.len()));
            s.insert(text);
            update(&mut map, &mut s);
            assert_eq!(map, wrap(&s, 40), "after inserting {text:?} at {at}");
        }
        s.select_all();
        s.insert("one\ntwo");
        update(&mut map, &mut s);
        assert_eq!(map, wrap(&s, 40));
        s.undo();
        update(&mut map, &mut s);
        assert_eq!(map, wrap(&s, 40), "undo logs its changes too");
    }

    #[test]
    fn line_columns_match_a_full_expansion_with_and_without_tabs() {
        for line in ["plain text here", "a\tbc\t\td  e\t", "\t\tlead", ""] {
            let r = ropey::Rope::from_str(line);
            let (_, cols) = expand_tabs(line, 4);
            for (i, &col) in cols.iter().enumerate() {
                assert_eq!(col_after(r.slice(..i), 4), col, "{line:?} char {i}");
            }
            for col in 0..cols.last().unwrap() + 3 {
                assert_eq!(char_at(r.slice(..), 4, col), char_at_col(&cols, col), "{line:?} col {col}");
            }
        }
    }

    #[test]
    fn a_resize_rewraps_near_lines_now_and_the_rest_when_settled() {
        let text: Vec<String> = (0..9)
            .map(|i| match i % 3 {
                0 => probe_line(),
                1 => "short".into(),
                _ => format!("\t{}", "x".repeat(90)),
            })
            .collect();
        let s = EditorState::from_text(&text.join("\n"));
        let mut map = wrap(&s, 40);
        let (old, fresh) = (wrap(&s, 40), wrap(&s, 20));
        map.resize(20, 4);
        assert!(map.fits(&s.doc, 20, 4) && !map.settled());
        map.rewrap(&s.doc, 3..5, &Folds::NONE);
        for l in 3..5 {
            assert_eq!(map.breaks_of(l), fresh.breaks_of(l), "the lines on screen at once");
        }
        assert_eq!(map.breaks_of(0), old.breaks_of(0), "the rest keeps its rows meanwhile");
        // meanwhile the rows still cover the text, each char once
        let mut at = 0;
        for row in 0..map.rows(&s.doc) {
            let b = map.row_bounds(&s.doc, DisplayRow(row)).unwrap();
            assert!(b.start == at || b.start == at + 1, "row {row} starts at {b:?}");
            at = b.end;
        }
        assert_eq!(at, s.len());
        while !map.settle(&s.doc, Instant::now(), &Folds::NONE) {}
        assert_eq!(map, fresh);
    }

    #[test]
    fn a_lazy_map_settles_to_a_full_build_across_edits() {
        let mut s = EditorState::from_text(&vec![probe_line(); 4].join("\n"));
        let mut map = WrapMap::lazy(&s.doc, 40, 4, &Folds::NONE);
        assert_eq!(map.rows(&s.doc), 4, "one row a line until wrapped");
        s.set_cursor(5);
        s.insert("new\nlines ");
        update(&mut map, &mut s);
        while !map.settle(&s.doc, Instant::now(), &Folds::NONE) {}
        assert_eq!(map, wrap(&s, 40));
    }

    #[test]
    fn expand_tabs_fills_to_stops() {
        let (t, cols) = expand_tabs("a\tbc\td", 4);
        assert_eq!(t, "a   bc  d");
        assert_eq!(cols, vec![0, 1, 4, 5, 6, 8, 9]);
    }

    #[test]
    fn no_tab_borrows() {
        assert!(matches!(expand_tabs("abc", 4).0, Cow::Borrowed(_)));
    }

    #[test]
    fn a_column_inside_a_tab_rounds_to_the_nearer_char() {
        let (_, cols) = expand_tabs("a\tb", 4); // a at 0, tab 1..4, b at 4
        assert_eq!([0, 1, 2, 3, 4, 5, 9].map(|c| char_at_col(&cols, c)), [0, 1, 1, 2, 2, 3, 3]);
    }
}
