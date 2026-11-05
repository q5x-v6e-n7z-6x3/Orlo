use crate::buffer::*;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

/// Whether `c` joins the char before it into one cluster (a combining accent, a joiner, a
/// variation selector, an emoji's skin tone): it goes with a word that ends before it.
pub(crate) fn joins_previous(c: char) -> bool {
    let mut utf8 = [0; 4];
    c as u32 >= 0x300 && ["a", c.encode_utf8(&mut utf8)].concat().graphemes(true).count() == 1
}

/// Characters that can continue a selected word, shared with whole-word search.
pub(crate) fn word_continuation(c: char) -> bool {
    EditorState::is_word_char(c) || joins_previous(c)
}

/// The first byte in `range` where `needle` starts (overlapping ones too) and `ok` holds, the
/// rope read a block of chunks at a time rather than copied whole.
fn find_in_rope(rope: &Rope, needle: &str, range: Range<usize>, ok: &dyn Fn(usize) -> bool) -> Option<usize> {
    // twice the needle at least: each refill reads as much new text as the block keeps of the
    // old, or a long selection's Quick Add Next re-searches its own length a chunk at a time
    let block_len = (64 * 1024).max(2 * needle.len());
    let step = needle.chars().next()?.len_utf8();
    let (mut chunks, mut base, _, _) = rope.chunks_at_byte(range.start);
    let mut block = String::new();
    let mut at = 0;
    loop {
        let mut done = true;
        for c in chunks.by_ref() {
            block.push_str(c);
            if block.len() >= block_len {
                done = false;
                break;
            }
        }
        while let Some(p) = block[at..].find(needle) {
            let b = base + at + p;
            if b >= range.end {
                return None;
            }
            if b >= range.start && ok(b) {
                return Some(b);
            }
            at += p + step;
        }
        if done || base + block.len() >= range.end + needle.len() {
            return None;
        }
        // a match may start in the last needle.len() - 1 bytes and run into the next chunk
        let mut keep = block.len().saturating_sub(needle.len() - 1);
        while !block.is_char_boundary(keep) {
            keep -= 1;
        }
        at = at.max(keep) - keep;
        block.drain(..keep);
        base += keep;
    }
}

/// `sels` with `cut` taken out: a caret in it (its ends too) goes, a selection keeps what lies
/// outside it, each part facing as the selection did; a point inside one splits it in two.
fn subtract_selection(sels: &[Selection], cut: Selection) -> Vec<Selection> {
    let (start, end) = (cut.start_point(), cut.end_point());
    let mut out = Vec::new();
    for s in sels {
        if s.is_empty() {
            if !(start..=end).contains(&s.head_point()) {
                out.push(*s);
            }
            continue;
        }
        for (a, b) in [(s.start_point(), s.end_point().min(start)), (s.start_point().max(end), s.end_point())] {
            if a < b {
                let part = if s.head_point() >= s.anchor_point() {
                    Selection::from_points(a, b)
                } else {
                    Selection::from_points(b, a)
                };
                out.push(Selection { rectangular: s.rectangular, ..part });
            }
        }
    }
    out
}

/// `cut`, or a caret at `at` when nothing is left (Sublime leaves none; Orlo always has one).
fn or_caret(cut: Vec<Selection>, at: usize) -> Vec<Selection> {
    match cut.is_empty() {
        true => vec![Selection::caret(at)],
        false => cut,
    }
}

/// What a mouse drag selects from, kept for the whole drag (`EditorState::drag_from`).
#[derive(Clone, Debug, PartialEq)]
pub struct DragFrom {
    /// The selections the drag leaves as they are.
    pub others: Vec<Selection>,
    /// The press's offset, word or line: the selection always keeps all of it.
    pub anchor: Range<usize>,
    pub anchor_virtual: usize,
    pub unit: DragUnit,
    /// cmd-shift: what the drag covers comes out of `others` instead.
    pub subtract: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragUnit {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    SubwordLeft,
    SubwordRight,
    ParagraphPrevious,
    ParagraphNext,
    SentencePrevious,
    SentenceNext,
    /// cmd-left: a continuation row's start, else smart home.
    LineStart,
    /// cmd-right: the row's end, then the line's end.
    LineEnd,
    /// ctrl-a / ctrl-e: the logical line's start and end, rows or not.
    HardLineStart,
    HardLineEnd,
    DocStart,
    DocEnd,
    PageUp(usize),
    PageDown(usize),
}

/// A row of a view as drawn: wrapped rows, folded text and filler rows all count, so it is
/// never a buffer line (`Document::point` gives those).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayRow(pub usize);

/// A column of a display row: tabs run to their stops, and a continuation row's indent counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayCol(pub usize);

/// Where an offset is drawn, or a place on screen to look an offset up at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayPoint {
    pub row: DisplayRow,
    pub col: DisplayCol,
}

impl DisplayPoint {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row: DisplayRow(row), col: DisplayCol(col) }
    }
}

/// Display rows, for vertical and line motion: one per line, or a wrapped view's rows, less the
/// folded ones (`Folded`), with any filler rows (`Padded`). Columns are display columns (tabs
/// expanded, a continuation row's indent included).
pub trait Rows {
    /// Where `offset` is drawn, always on a text row; an offset at a soft break is on the
    /// upper row.
    fn row_of(&self, s: &Document, offset: usize) -> DisplayPoint;
    /// The offset nearest `at.col` on `at.row`, clamped to the row. A filler row has none: it
    /// gives the end of the nearest text row above it, or the document's start.
    fn offset_at(&self, s: &Document, at: DisplayPoint) -> usize;
    /// The row's chars, without the line break; `None` for a filler row, which has no text.
    fn row_bounds(&self, s: &Document, row: DisplayRow) -> Option<Range<usize>>;
    /// The rows drawn, filler rows included.
    fn rows(&self, s: &Document) -> usize;
    /// Where a horizontal motion that reached `offset` lands: out of folded text, onward
    /// (`forward`) or back (`Folded`); elsewhere `offset` itself.
    fn skip(&self, _: &Document, offset: usize, _forward: bool) -> usize {
        offset
    }
    /// The first row of `line`; `rows` past the last line.
    fn first_row(&self, s: &Document, line: usize) -> DisplayRow {
        match line < s.line_count() {
            true => self.row_of(s, s.line_start(line)).row,
            false => DisplayRow(self.rows(s)),
        }
    }
    /// The nearest text row to `row`: it, or the first after it (`down`) or before it that is
    /// not a filler row; `None` when there is none that way.
    fn text_row(&self, s: &Document, row: DisplayRow, _down: bool) -> Option<DisplayRow> {
        (row.0 < self.rows(s)).then_some(row)
    }
    /// The row filler rows before `line` go above: the first that starts at or after the
    /// line's start (a line folded away puts them after the row it is folded into), or past the
    /// last row for the line count.
    fn boundary_row(&self, s: &Document, line: usize) -> usize {
        if line >= s.line_count() {
            return self.rows(s);
        }
        let r = self.first_row(s, line);
        match self.row_bounds(s, r) {
            Some(bounds) if bounds.start < s.line_start(line) => r.0 + 1,
            _ => r.0,
        }
    }
}

/// A borrowed layer is one too, so a layer can sit on another it does not own.
impl<R: Rows + ?Sized> Rows for &R {
    fn row_of(&self, s: &Document, offset: usize) -> DisplayPoint {
        (**self).row_of(s, offset)
    }
    fn offset_at(&self, s: &Document, at: DisplayPoint) -> usize {
        (**self).offset_at(s, at)
    }
    fn row_bounds(&self, s: &Document, row: DisplayRow) -> Option<Range<usize>> {
        (**self).row_bounds(s, row)
    }
    fn rows(&self, s: &Document) -> usize {
        (**self).rows(s)
    }
    fn skip(&self, s: &Document, offset: usize, forward: bool) -> usize {
        (**self).skip(s, offset, forward)
    }
    fn first_row(&self, s: &Document, line: usize) -> DisplayRow {
        (**self).first_row(s, line)
    }
    fn text_row(&self, s: &Document, row: DisplayRow, down: bool) -> Option<DisplayRow> {
        (**self).text_row(s, row, down)
    }
    fn boundary_row(&self, s: &Document, line: usize) -> usize {
        (**self).boundary_row(s, line)
    }
}

/// How far across a row a display column is drawn, for the motions that keep to an x (Up,
/// Down, Page Up/Down, adding carets above and below). A view measures its shaped rows in the
/// unit it draws in; `Cells` counts a unit per column.
pub trait Measure {
    /// How far `at` is right of the text's left edge, a continuation row's indent included.
    fn x_of(&self, s: &Document, at: DisplayPoint) -> f32;
    /// The column of text row `row` drawn nearest `x`.
    fn col_at(&self, s: &Document, row: DisplayRow, x: f32) -> DisplayCol;
}

/// A unit per display column: how a view moves before it has shaped its rows.
pub struct Cells;

impl Measure for Cells {
    fn x_of(&self, _: &Document, at: DisplayPoint) -> f32 {
        at.col.0 as f32
    }

    fn col_at(&self, _: &Document, _: DisplayRow, x: f32) -> DisplayCol {
        DisplayCol(x.round().max(0.) as usize)
    }
}

/// One row per line, as an unwrapped view moves; columns are display columns.
pub struct Lines;

impl Rows for Lines {
    fn row_of(&self, s: &Document, offset: usize) -> DisplayPoint {
        let (line, col) = s.point(offset);
        let ls = s.line_start(line);
        DisplayPoint::new(line, col_after(s.rope.slice(ls..ls + col), s.tab_size))
    }

    fn offset_at(&self, s: &Document, at: DisplayPoint) -> usize {
        let line = at.row.0.min(s.line_count() - 1);
        let ls = s.line_start(line);
        let line_chars = s.rope.slice(ls..s.line_end(line));
        s.snap_to_cluster(ls + char_at(line_chars, s.tab_size, at.col.0))
    }

    fn row_bounds(&self, s: &Document, row: DisplayRow) -> Option<Range<usize>> {
        Some(s.line_start(row.0)..s.line_end(row.0))
    }

    fn rows(&self, s: &Document) -> usize {
        s.line_count()
    }

    fn first_row(&self, s: &Document, line: usize) -> DisplayRow {
        DisplayRow(line.min(s.line_count()))
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// The selections with the primary moved last (what a session stores).
    pub fn selections_primary_last(&self) -> Vec<Selection> {
        self.view.selections_primary_last()
    }

    pub fn cursor(&self) -> usize {
        self.view.cursor()
    }

    /// The first occurrence of `from`'s text after it, wrapping, that no selection starts at.
    pub(crate) fn next_occurrence(&self, from: Selection, whole_word: bool) -> Option<Range<usize>> {
        let rope = &self.doc.rope;
        let n = from.range().len();
        let len = rope.len_chars();
        if n == 0 || len < n {
            return None;
        }
        let needle = rope.slice(from.range()).to_string();
        let mut starts: Vec<usize> = self.view.selections.iter().map(|s| s.start()).collect();
        starts.sort_unstable();
        let edge =
            |i: usize| i == 0 || i >= len || !word_continuation(rope.char(i - 1)) || !word_continuation(rope.char(i));
        let ok = |b: usize| {
            let i = rope.byte_to_char(b);
            starts.binary_search(&i).is_err() && (!whole_word || edge(i) && edge(i + n))
        };
        let start = rope.char_to_byte(from.end().min(len - n + 1));
        let b = find_in_rope(rope, &needle, start..rope.len_bytes(), &ok)
            .or_else(|| find_in_rope(rope, &needle, 0..start, &ok))?;
        let i = rope.byte_to_char(b);
        Some(i..i + n)
    }

    pub fn primary(&self) -> usize {
        self.view.primary()
    }

    pub fn word_range_at(&self, offset: usize) -> Range<usize> {
        let len = self.len();
        let offset = offset.min(len);
        // the rope's char iterators, not a lookup a char: a 100 KB word under 1,000 carets;
        // the accents and joiners after a letter go with it
        let w = |c: &char| word_continuation(*c);
        let back = (self.doc.rope.chars_at(offset).reversed()).take_while(w).count();
        let ahead = self.doc.rope.chars_at(offset).take_while(w).count();
        let r = if back + ahead > 0 {
            offset - back..offset + ahead
        } else if offset < len {
            offset..offset + 1
        } else {
            offset..offset
        };
        // whole clusters: a word never ends on half an emoji, nor a symbol on half of one
        let end = match self.snap_to_cluster(r.end) {
            c if c < r.end => self.next_cluster(c),
            _ => r.end,
        };
        self.snap_to_cluster(r.start)..end
    }

    /// Where a mouse press starts a drag: `extend` (shift) keeps the primary's anchor; `add`
    /// (cmd) keeps the other selections, as Sublime does even when the press is on a caret;
    /// a double click takes the word and a triple click the line, and the drag then grows by
    /// words or lines (`drag_to`). Both (cmd-shift) take the char, word or line out of the
    /// selections instead, Sublime's subtractive drag (probed with `drag_text` off, as in Sublime
    /// a press inside a selection collapses it; Orlo's text drag takes unmodified presses only).
    pub fn drag_from(&self, offset: usize, clicks: usize, add: bool, extend: bool) -> DragFrom {
        self.drag_from_point((offset, 0), clicks, add, extend)
    }

    pub fn drag_from_point(&self, point: (usize, usize), clicks: usize, add: bool, extend: bool) -> DragFrom {
        let offset = point.0;
        let anchor_virtual = if clicks == 1 && self.view.virtual_space.allows(false) {
            if extend && !add {
                self.view.selections[self.primary()].anchor_virtual
            } else {
                point.1
            }
        } else {
            0
        };
        let offset = offset.min(self.len());
        let subtract = add && extend;
        let (unit, anchor) = match clicks {
            1 if extend && !subtract => {
                let p = self.view.selections[self.primary()];
                (DragUnit::Char, p.anchor..p.anchor)
            }
            // Sublime 4215: shift with a double or triple click extends by
            // nothing; every selection goes and a caret is left at the click
            _ if extend && !subtract => {
                return DragFrom {
                    others: Vec::new(),
                    anchor: offset..offset,
                    anchor_virtual: 0,
                    unit: DragUnit::Char,
                    subtract,
                }
            }
            1 => (DragUnit::Char, offset..offset),
            2 => (DragUnit::Word, self.word_range_at(offset)),
            _ => (DragUnit::Line, self.line_range_at(offset)),
        };
        let p = self.primary();
        let others = (self.view.selections.iter().enumerate())
            .filter(|&(i, s)| match (add, extend) {
                (true, true) => true,
                // the earlier clicks of this one left carets in its word or line
                (true, false) => {
                    !(s.is_empty()
                        && (anchor.start, anchor_virtual) <= s.head_point()
                        && s.head_point() <= (anchor.end, anchor_virtual))
                }
                (false, true) => i != p,
                (false, false) => false,
            })
            .map(|(_, s)| *s)
            .collect();
        DragFrom { others, anchor, anchor_virtual, unit, subtract }
    }

    /// Whether text dragged from the selections may drop at `offset`: not in or against one
    /// (carets are not dragged).
    pub fn can_drop(&self, offset: usize) -> bool {
        (self.view.selections.iter()).all(|s| s.is_empty() || offset < s.start() || offset > s.end())
    }

    /// The line `offset` is on, with its line break.
    pub fn line_range_at(&self, offset: usize) -> Range<usize> {
        let (l, _) = self.point(offset);
        self.line_start(l)..self.offset(l + 1, 0).max(self.line_end(l))
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// A display position, extending only beyond an actual line end on its final display row.
    pub fn selection_at(&self, at: DisplayPoint, rows: &dyn Rows) -> Selection {
        self.selection_at_origin(at, rows, false)
    }

    pub(crate) fn selection_at_origin(&self, at: DisplayPoint, rows: &dyn Rows, rectangular: bool) -> Selection {
        let offset = rows.offset_at(&self.doc, at);
        let end = rows.row_of(&self.doc, offset);
        let gap = if self.view.virtual_space.allows(rectangular)
            && end.row == at.row
            && offset == self.line_end(self.point(offset).0)
            && rows.row_bounds(&self.doc, at.row).is_some_and(|r| r.end == offset)
        {
            at.col.0.saturating_sub(end.col.0)
        } else {
            0
        };
        Selection { rectangular, ..Selection::from_points((offset, gap), (offset, gap)) }
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    pub fn set_virtual_space(&mut self, policy: impl Into<VirtualSpace>) {
        let policy = policy.into();
        if self.view.virtual_space != policy {
            self.set_typing(false);
            self.view.virtual_space = policy;
            self.view.sel_hist.generation = self.view.sel_hist.generation.wrapping_add(1);
            self.normalize();
        }
    }

    /// Replace every selection; the last one becomes the primary.
    pub fn set_selections(&mut self, sels: Vec<Selection>) {
        if self.view.selections != sels || self.view.primary != sels.len().saturating_sub(1) {
            self.view.sel_hist.generation = self.view.sel_hist.generation.wrapping_add(1);
        }
        self.view.selections = sels;
        self.view.primary = self.view.selections.len().saturating_sub(1);
        self.all_off_cr_lf();
        self.normalize();
    }

    pub fn select_all(&mut self) {
        self.set_typing(false);
        self.view.selections = vec![Selection::new(0, self.len())];
    }

    /// Expand touched half-open sentence intervals, retaining selection direction and origin.
    pub fn select_sentence(&mut self) {
        self.set_typing(false);
        let boundaries = self.sentence_boundaries();
        for i in 0..self.view.selections.len() {
            let sel = self.view.selections[i];
            let range = self.sentence_range(&boundaries, &sel);
            let expanded = if sel.head_point() < sel.anchor_point() {
                Selection::new(range.end, range.start)
            } else {
                Selection::new(range.start, range.end)
            };
            self.view.selections[i] = Selection { rectangular: sel.rectangular, ..expanded };
        }
        self.normalize();
    }

    /// Grow each selection to whole lines (including the line break); repeating grows by a line.
    pub fn select_line(&mut self) {
        self.set_typing(false);
        for i in 0..self.view.selections.len() {
            let sel = self.view.selections[i];
            let (l0, _) = self.point(sel.start());
            let (l1, _) = self.point(sel.end());
            let end = self.offset(l1 + 1, 0);
            self.view.selections[i] = Selection::new(self.line_start(l0), end);
        }
        self.normalize();
    }

    /// One selection per line covering that line's part of the original selection.
    pub fn split_into_lines(&mut self) {
        self.set_typing(false);
        let mut out = Vec::new();
        for sel in &self.view.selections {
            if sel.is_empty() {
                out.push(*sel);
                continue;
            }
            let (l0, _) = self.point(sel.start());
            let (l1, _) = self.point(sel.end());
            for l in l0..=l1 {
                let s = self.line_start(l).max(sel.start());
                let e = self.line_end(l).min(sel.end());
                if l == l1 && l > l0 && s == e {
                    continue;
                }
                out.push(Selection::new(s, e));
            }
        }
        self.view.selections = out;
        self.view.primary = self.view.selections.len().saturating_sub(1);
        self.normalize();
    }

    /// Sublime's quick-add-next: select the word under each caret, then add the next
    /// occurrence of the latest (primary) selection's text (wrapping, skipping already selected
    /// ones). Words found from a caret only match whole words; a selection made by hand matches
    /// anywhere.
    pub fn select_next_occurrence(&mut self) {
        self.set_typing(false);
        if self.view.selections.iter().all(|s| s.is_empty()) {
            for i in 0..self.view.selections.len() {
                let r = self.word_range_at(self.view.selections[i].head);
                self.view.selections[i] = Selection::new(r.start, r.end);
            }
            self.normalize();
            self.view.quick_add_words = self.view.selections.clone();
            return;
        }
        let whole_word = self.view.selections == self.view.quick_add_words;
        if let Some(r) = self.next_occurrence(self.view.selections[self.primary()], whole_word) {
            self.view.selections.push(Selection::new(r.start, r.end));
            self.view.primary = self.view.selections.len() - 1;
            self.normalize();
            if whole_word {
                self.view.quick_add_words = self.view.selections.clone();
            }
        }
    }

    /// Quick Skip Next: the latest selection gives way to the next occurrence of its text. With
    /// only carets it expands them to words, as quick-add-next does. False when there is no
    /// occurrence left to move to; the selections are then as they were.
    pub fn quick_skip_next(&mut self) -> bool {
        if self.view.selections.iter().all(|s| s.is_empty()) {
            self.select_next_occurrence();
            return true;
        }
        let whole_word = self.view.selections == self.view.quick_add_words;
        let p = self.primary();
        let Some(r) = self.next_occurrence(self.view.selections[p], whole_word) else {
            return false;
        };
        self.set_typing(false);
        self.view.selections[p] = Selection::new(r.start, r.end);
        self.normalize();
        if whole_word {
            self.view.quick_add_words = self.view.selections.clone();
        }
        true
    }

    pub fn single_selection(&mut self) {
        self.set_typing(false);
        self.view.selections.truncate(1);
    }

    pub fn set_cursor(&mut self, offset: usize) {
        self.set_typing(false);
        let selections = vec![Selection::caret(offset.min(self.len()))];
        if self.view.selections != selections {
            self.view.sel_hist.generation = self.view.sel_hist.generation.wrapping_add(1);
        }
        self.view.selections = selections;
    }

    /// A selection command: keep the primary through normalization and end the typing group.
    pub fn select_preserving_primary(&mut self, selections: Vec<Selection>) {
        self.set_typing(false);
        self.reselect(selections);
    }

    /// Replace the selections one for one, the primary staying the primary.
    pub fn reselect(&mut self, sels: Vec<Selection>) {
        if sels.len() == self.view.selections.len() {
            if self.view.selections != sels {
                self.view.sel_hist.generation = self.view.sel_hist.generation.wrapping_add(1);
            }
            self.view.selections = sels;
            self.normalize();
        } else {
            self.set_selections(sels);
        }
    }

    /// Another caret (tests stand in for a cmd-click with it).
    #[cfg(any(test, feature = "test-support"))]
    pub fn add_cursor(&mut self, offset: usize) {
        let mut sels = self.view.selections.clone();
        sels.push(Selection::caret(offset.min(self.len())));
        self.set_selections(sels);
    }

    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        self.set_typing(false);
        let selections = vec![Selection::new(anchor, head)];
        if self.view.selections != selections {
            self.view.sel_hist.generation = self.view.sel_hist.generation.wrapping_add(1);
        }
        self.view.selections = selections;
        self.normalize();
    }

    pub fn select_word_at(&mut self, offset: usize) {
        let r = self.word_range_at(offset);
        self.set_selection(r.start, r.end);
    }

    /// The selection a drag from `from` to `offset` makes: from the far side of the anchor to
    /// the char, word or line under the mouse, with the other selections kept as they were.
    pub fn drag_to(&mut self, from: &DragFrom, offset: usize) {
        self.drag_to_point(from, (offset, 0));
    }

    pub fn drag_to_point(&mut self, from: &DragFrom, point: (usize, usize)) {
        let offset = point.0;
        self.set_typing(false);
        let offset = offset.min(self.len());
        let target = match from.unit {
            DragUnit::Char => offset..offset,
            DragUnit::Word => self.word_range_at(offset),
            DragUnit::Line => self.line_range_at(offset),
        };
        let sel = if from.unit == DragUnit::Char {
            Selection::from_points((from.anchor.start, from.anchor_virtual), point)
        } else if target.start < from.anchor.start {
            Selection::new(from.anchor.end, target.start)
        } else {
            Selection::new(from.anchor.start, target.end.max(from.anchor.end))
        };
        let sels = match from.subtract {
            false => [from.others.as_slice(), &[sel]].concat(),
            // a caret at the press when nothing is left
            true => or_caret(subtract_selection(&from.others, sel), from.anchor.start),
        };
        self.set_selections(sels);
    }

    /// Option-drag: a box from display point `a` to `b`, a selection per text row (filler rows
    /// have none), the row under the mouse the primary.
    pub fn drag_box(
        &mut self,
        others: &[Selection],
        subtractive: bool,
        a: DisplayPoint,
        b: DisplayPoint,
        rows: &dyn Rows,
    ) {
        self.drag_box_measured(others, subtractive, (a.row, a.col.0 as f32), (b.row, b.col.0 as f32), rows, &Cells);
    }

    /// A rectangle's two horizontal goals in `measure` units. Resolve both on each row:
    /// character columns from differently shaped rows cannot be reused as x coordinates.
    pub fn drag_box_measured(
        &mut self,
        others: &[Selection],
        subtractive: bool,
        a: (DisplayRow, f32),
        b: (DisplayRow, f32),
        rows: &dyn Rows,
        measure: &dyn Measure,
    ) {
        self.set_typing(false);
        let mut boxed = Vec::new();
        let mut row = a.0;
        loop {
            if rows.row_bounds(&self.doc, row).is_some() {
                let at = |x| {
                    self.selection_at_origin(DisplayPoint { row, col: measure.col_at(&self.doc, row, x) }, rows, true)
                        .head_point()
                };
                boxed.push(Selection::from_points(at(a.1), at(b.1)).in_rectangle());
            }
            if row == b.0 {
                break;
            }
            row = DisplayRow(if b.0 > row { row.0 + 1 } else { row.0 - 1 });
        }
        // a box over filler rows alone: the caret goes where the press is
        if boxed.is_empty() {
            let at = DisplayPoint { row: a.0, col: measure.col_at(&self.doc, a.0, a.1) };
            boxed.push(Selection::caret(rows.offset_at(&self.doc, at)));
        }
        let sels = match subtractive {
            false => [others, &boxed].concat(),
            // alt-cmd-shift, Sublime's subtractive columns: the box comes out, row by row
            true => or_caret((boxed.iter()).fold(others.to_vec(), |s, r| subtract_selection(&s, *r)), boxed[0].anchor),
        };
        self.set_selections(sels);
    }

    pub fn select_ranges(&mut self, ranges: &[Range<usize>]) {
        if ranges.is_empty() {
            return;
        }
        self.set_typing(false);
        self.view.selections = (ranges.iter()).map(|r| Selection::new(r.start, r.end)).collect();
        self.view.primary = self.view.selections.len() - 1;
        self.normalize();
    }

    /// ctrl-shift-up/down: add a copy of every selection on the neighbouring text row of `rows`
    /// (a view's: folded lines and filler rows are stepped over), each end at the x `measure`
    /// draws it at.
    pub fn select_lines(&mut self, forward: bool, rows: &dyn Rows, measure: &dyn Measure) {
        self.set_typing(false);
        let n = rows.rows(&self.doc);
        let target = |r: DisplayRow| {
            let next = match forward {
                true => (r.0 + 1 < n).then_some(DisplayRow(r.0 + 1)),
                false => r.0.checked_sub(1).map(DisplayRow),
            };
            next.and_then(|r| rows.text_row(&self.doc, r, forward))
        };
        let mut added = Vec::new();
        let mut primary = self.view.primary;
        for (i, sel) in self.view.selections.iter().enumerate() {
            // each end where it is drawn, as Up and Down keep to it: a tab is its width (Sublime)
            let (mut pa, mut ph) = (rows.row_of(&self.doc, sel.anchor), rows.row_of(&self.doc, sel.head));
            pa.col.0 += sel.anchor_virtual;
            ph.col.0 += sel.head_virtual;
            let on = |p: DisplayPoint, row: DisplayRow| {
                let col = measure.col_at(&self.doc, row, measure.x_of(&self.doc, p));
                self.selection_at_origin(DisplayPoint { row, col }, rows, true).head_point()
            };
            if let (Some(a), Some(h)) = (target(pa.row), target(ph.row)) {
                // the primary's copy takes over, so the view follows the new carets
                if i == self.view.primary {
                    primary = self.view.selections.len() + added.len();
                }
                added.push(match sel.goal {
                    // a caret left at a line's end by cmd-right adds carets at line ends
                    Some(Goal::LineEnd) if sel.is_empty() => Selection {
                        goal: sel.goal,
                        // `target` gives text rows only
                        ..Selection::caret(rows.row_bounds(&self.doc, h).map_or(sel.head, |b| b.end))
                    },
                    _ => Selection::from_points(on(pa, a), on(ph, h)),
                });
            }
        }
        for sel in &mut added {
            sel.rectangular = true;
        }
        for sel in &mut self.view.selections {
            sel.rectangular = true;
        }
        self.view.primary = primary;
        self.view.selections.extend(added);
        self.normalize();
    }

    /// ctrl-cmd-g: select every occurrence of the word under the caret (or of the selection).
    pub fn select_all_occurrences(&mut self) {
        if self.view.selections.iter().all(|s| s.is_empty()) {
            self.select_next_occurrence();
        }
        let last = *self.view.selections.last().unwrap();
        let needle = self.doc.rope.slice(last.range()).to_string();
        let all = self.find_all(
            &needle,
            &FindOpts {
                case_sensitive: true,
                // words cmd-d found from a caret match whole words only, as in select_next_occurrence
                whole_word: self.view.selections == self.view.quick_add_words,
                ..FindOpts::default()
            },
        );
        self.select_ranges(&all);
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// Start a preview at virtual selections. Source remains until commit, as for overtype.
    pub fn compose_virtual(&mut self, text: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
        self.set_typing(false);
        self.record();
        self.set_typing(true);
        let mut padding = self.materialize_virtual();
        let ranges: Vec<_> = self.view.selections.iter().map(|s| s.start()..s.start()).collect();
        let placed = self.compose_tracking_padding(&ranges, text, false, &mut padding);
        (placed, padding)
    }

    /// Composition ranges and provisional pads follow the same logged edits as other ranges.
    pub fn compose_tracking_padding(
        &mut self,
        ranges: &[Range<usize>],
        text: &str,
        start: bool,
        padding: &mut Vec<Range<usize>>,
    ) -> Vec<Range<usize>> {
        // Preview insertions inside a shared pad split it, so cancellation never removes
        // a preview twice or mistakes inserted text for provisional whitespace.
        let cuts: Vec<_> = ranges.iter().map(|r| r.start).collect();
        *padding = padding
            .iter()
            .flat_map(|pad| {
                let mut start = pad.start;
                let mut pieces = Vec::new();
                for &cut in cuts.iter().filter(|&&cut| pad.start < cut && cut < pad.end) {
                    pieces.push(start..cut);
                    start = cut;
                }
                pieces.push(start..pad.end);
                pieces
            })
            .collect();
        let before = self.view.changes.len();
        let placed = self.compose(ranges, text, start);
        map_ranges(padding, &self.view.changes[before..]);
        placed
    }

    /// Remove a valid preview and its pads in the existing composition undo group.
    pub fn remove_composition(&mut self, marked: &[Range<usize>], padding: &[Range<usize>]) {
        if self.doc.edits_blocked() {
            return;
        }
        let mut ranges: Vec<_> = marked.iter().chain(padding).filter(|r| !r.is_empty()).cloned().collect();
        ranges.sort_by_key(|r| r.start);
        for r in ranges.into_iter().rev() {
            let line = self.doc.rope.char_to_line(r.start);
            self.splice(r, "");
            self.touch(line);
        }
    }

    /// IME and dead keys: `text` replaces each of `ranges` (one per selection, in order),
    /// which become the selections. The first call of a composition (`start`) makes the undo
    /// step; the rest, and the commit typed after them, join it.
    pub fn compose(&mut self, ranges: &[Range<usize>], text: &str, start: bool) -> Vec<Range<usize>> {
        if self.doc.edits_blocked() {
            return Vec::new();
        }
        if start {
            self.set_typing(false);
            self.record();
        }
        self.set_typing(true);
        let selections = ranges
            .iter()
            .enumerate()
            .map(|(i, r)| Selection {
                rectangular: self.view.selections.get(i).is_some_and(|sel| sel.rectangular),
                ..Selection::new(r.start, r.end)
            })
            .collect();
        self.reselect(selections);
        let n = text.chars().count();
        self.edit_with(|_, _, sel| (sel.range(), text.to_string(), Selection::new(sel.start(), sel.start() + n)));
        self.set_typing(true);
        self.view.selections.iter().map(|s| s.range()).collect()
    }

    /// Drag and drop: the non-empty selections, their texts joined by line breaks in
    /// document order, go to `drop` (or with `copy` are copied there), and the text put in is
    /// selected. One undo step. False, changing nothing, when there is none to drag or `drop` is
    /// in or against one.
    pub fn move_text(&mut self, drop: usize, copy: bool) -> bool {
        if self.doc.edits_blocked() {
            return false;
        }
        self.normalize();
        let drop = drop.min(self.len());
        let from: Vec<Range<usize>> =
            (self.view.selections.iter()).filter(|s| !s.is_empty()).map(Selection::range).collect();
        if from.is_empty() || !self.can_drop(drop) {
            return false;
        }
        let texts: Vec<String> = (from.iter()).map(|r| self.doc.rope.slice(r.clone()).to_string()).collect();
        let text = texts.join("\n");
        // last to first, so each range is still where the selection was
        let mut edits = vec![(drop..drop, text.as_str())];
        if !copy {
            edits.extend(from.iter().map(|r| (r.clone(), "")));
        }
        edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
        self.set_typing(false);
        self.record();
        // the carets stay on their text; the dragged selections give way to what is put in
        let mut sels: Vec<Selection> = (self.view.selections.iter()).filter(|s| s.is_empty()).copied().collect();
        let before = self.view.changes.len();
        for (r, t) in &edits {
            self.splice(r.clone(), t);
        }
        map_sels(&mut sels, &self.view.changes[before..], false);
        let first = edits.last().map_or(0, |e| e.0.start);
        self.touch(self.doc.rope.char_to_line(first.min(self.len())));
        // the cut text before the drop moves it back
        let cut: usize = match copy {
            true => 0,
            false => (from.iter().filter(|r| r.end < drop).map(|r| r.len())).sum(),
        };
        let start = drop - cut;
        sels.push(Selection::new(start, start + text.chars().count()));
        self.set_selections(sels);
        true
    }

    /// Delete the same whole intervals Select Sentence would cover, in one existing edit batch.
    pub fn delete_sentence(&mut self) {
        if self.doc.edits_blocked()
            || self.is_empty()
            || self.view.selections.iter().all(|s| s.has_virtual() && s.range().is_empty())
        {
            return;
        }
        let boundaries = self.sentence_boundaries();
        self.set_typing(false);
        self.record();
        self.edit_with(|s, _, sel| {
            if sel.has_virtual() && sel.range().is_empty() {
                return (sel.range(), String::new(), *sel);
            }
            let range = s.sentence_range(&boundaries, sel);
            let caret = Selection { rectangular: sel.rectangular, ..Selection::caret(range.start) };
            (range, String::new(), caret)
        });
    }

    /// Delete selected source text, or the adjacent identifier segment, in one undo step.
    pub fn delete_subword(&mut self, forward: bool) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        if !forward {
            for sel in &mut self.view.selections {
                if sel.is_empty() && sel.head_virtual > 0 {
                    sel.anchor_virtual = 0;
                }
            }
        }
        self.edit(|s, sel| {
            let range = match (sel.is_empty(), forward) {
                _ if sel.has_virtual() => sel.range(),
                (false, _) => sel.range(),
                (true, true) => s.snap_to_cluster(sel.head)..s.subword_right(sel.head),
                (true, false) => {
                    // Programmatic/shared positions may fall inside a cluster: take all of it.
                    let start = s.snap_to_cluster(sel.head);
                    let end = if start < sel.head { s.next_cluster(start) } else { sel.head };
                    s.subword_left(sel.head)..end
                }
            };
            (range, String::new())
        });
    }

    /// alt-backspace / alt-delete.
    pub fn delete_word(&mut self, forward: bool) {
        self.set_typing(false);
        self.record();
        if !forward {
            for sel in &mut self.view.selections {
                if sel.is_empty() && sel.head_virtual > 0 {
                    sel.anchor_virtual = 0;
                }
            }
        }
        // a selection goes as it is (Sublime 4215); a caret takes the word by it
        self.edit(|s, sel| {
            let r = match (sel.is_empty(), forward) {
                _ if sel.has_virtual() => sel.range(),
                (false, _) => sel.range(),
                (true, true) => sel.head..s.word_right(sel.head),
                (true, false) => s.word_left(sel.head)..sel.head,
            };
            (r, String::new())
        });
    }
}

impl Document {
    /// `offset`, or the start of the cluster it falls inside: Up, Down and clicks count
    /// columns in chars, and a caret must never split an emoji or an accented letter.
    pub fn snap_to_cluster(&self, offset: usize) -> usize {
        let offset = offset.min(self.len());
        if offset == self.len() {
            return offset;
        }
        // The previous boundary after including this scalar is its cluster's true start,
        // even when `offset` is a programmatic position deep inside an oversized cluster.
        self.prev_cluster(offset + 1)
    }

    /// The grapheme cluster boundary after `offset` (Sublime moves over an emoji, a flag or a
    /// letter with its accents in one step). The usual look-ahead is 64 chars; only an
    /// oversized cluster grows that window, so edits cannot remove part of such a cluster.
    pub fn next_cluster(&self, offset: usize) -> usize {
        let offset = offset.min(self.len());
        let mut width = 64;
        loop {
            let end = offset.saturating_add(width).min(self.len());
            let text = self.rope.slice(offset..end).to_string();
            let first = text.graphemes(true).next().map_or(0, |g| g.chars().count());
            if first < end - offset || end == self.len() {
                return (offset + first.max(1)).min(self.len());
            }
            width = width.saturating_mul(2);
        }
    }

    /// The complete grapheme cluster boundary before `offset`, reading rope chunks as needed.
    pub fn prev_cluster(&self, offset: usize) -> usize {
        let byte = self.rope.char_to_byte(offset.min(self.len()));
        let mut cursor = GraphemeCursor::new(byte, self.rope.len_bytes(), true);
        let (mut chunk, mut start, _, _) = self.rope.chunk_at_byte(byte.saturating_sub(1));
        loop {
            match cursor.prev_boundary(chunk, start) {
                Ok(Some(boundary)) => return self.rope.byte_to_char(boundary),
                Ok(None) => return 0,
                Err(GraphemeIncomplete::PrevChunk) => {
                    (chunk, start, _, _) = self.rope.chunk_at_byte(cursor.cur_cursor() - 1);
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let (context, from, _, _) = self.rope.chunk_at_byte(end - 1);
                    cursor.provide_context(&context[..end - from], from);
                }
                // Backward traversal always supplies a chunk covering the cursor, and
                // requested pre-context ends exactly where the segmenter asks for it.
                Err(GraphemeIncomplete::NextChunk | GraphemeIncomplete::InvalidOffset) => unreachable!(),
            }
        }
    }
}

impl ViewModel {
    /// The selections with the primary moved last (what a session stores).
    pub fn selections_primary_last(&self) -> Vec<Selection> {
        let mut v = self.selections.clone();
        let p = v.remove(self.primary.min(v.len() - 1));
        v.push(p);
        v
    }

    pub fn cursor(&self) -> usize {
        (self.selections.get(self.primary)).or(self.selections.last()).map_or(0, |s| s.head)
    }

    pub fn primary(&self) -> usize {
        self.primary.min(self.selections.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_add_with_a_long_selection_scans_linearly() {
        let text = format!("{}Z{}", "ab".repeat(200_000), "ab".repeat(2_000_000));
        let mut s = EditorState::from_text(&text);
        s.set_selection(0, 400_001);
        let t = std::time::Instant::now();
        s.select_next_occurrence();
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "{:?}", t.elapsed());
        assert_eq!(s.view.selections.len(), 1, "the selection occurs once");
        let rope = ropey::Rope::from_str(&text);
        let needle = "ab".repeat(40_000) + "Z";
        assert_eq!(find_in_rope(&rope, &needle, 0..rope.len_bytes(), &|_| true), Some(320_000));
    }

    #[test]
    fn a_word_takes_the_accents_that_follow_it() {
        // "cafe" + U+0301 (decomposed é) and a skin-toned thumb: no cluster is cut in two
        let s = EditorState::from_text("cafe\u{301} \u{1f44d}\u{1f3fd}");
        assert_eq!(s.word_range_at(2), 0..5, "the accent belongs to the word");
        assert_eq!(s.word_range_at(5), 0..5, "after the word: still it");
        assert_eq!(s.word_range_at(6), 6..8, "a lone symbol is its whole cluster");
        assert_eq!(s.word_range_at(7), 6..8, "inside a cluster: all of it");
    }

    #[test]
    fn clusters_inside_a_long_run_of_flags_pair_up_from_the_run_start() {
        // 40 flags are 80 regional indicators; the 64-char look-back starts mid-flag
        let s = EditorState::from_text(&"\u{1f1fa}\u{1f1f8}".repeat(40));
        for flag in 0..40 {
            let (a, b) = (flag * 2, flag * 2 + 2);
            assert_eq!(s.snap_to_cluster(a + 1), a, "inside flag {flag}");
            assert_eq!(s.next_cluster(a), b, "over flag {flag}");
            assert_eq!(s.prev_cluster(b), a, "back over flag {flag}");
        }
    }

    #[test]
    fn flag_pairing_survives_a_run_longer_than_the_look_back_limit() {
        for prefix in ["", "x", "ب", "e\u{301}"] {
            let text = format!("{prefix}{}", "\u{1f1fa}\u{1f1f8}".repeat(1_000));
            let s = EditorState::from_text(&text);
            for flag in [288, 289, 511, 512, 999] {
                let a = prefix.chars().count() + flag * 2;
                assert_eq!(s.snap_to_cluster(a + 1), a, "{prefix:?}, inside flag {flag}");
                assert_eq!(s.prev_cluster(a + 2), a, "{prefix:?}, back over flag {flag}");
                assert_eq!(s.next_cluster(a), a + 2, "{prefix:?}, over flag {flag}");
            }
        }
    }

    /// The char-by-char scan `next_occurrence` replaced, over a copy of the whole text.
    fn naive_next(s: &EditorState, from: Selection, whole_word: bool) -> Option<Range<usize>> {
        let needle: Vec<char> = s.doc.rope.slice(from.range()).chars().collect();
        let hay: Vec<char> = s.doc.rope.chars().collect();
        let n = needle.len();
        if n == 0 || hay.len() < n {
            return None;
        }
        let limit = hay.len() - n + 1;
        let taken = |i: usize| s.view.selections.iter().any(|s| s.start() == i);
        let edge = |i: usize| i == 0 || i >= hay.len() || !word_continuation(hay[i - 1]) || !word_continuation(hay[i]);
        let find = |mut range: Range<usize>| {
            range.find(|&i| hay[i..i + n] == needle[..] && !taken(i) && (!whole_word || edge(i) && edge(i + n)))
        };
        let start = from.end().min(limit);
        let i = find(start..limit).or_else(|| find(0..start))?;
        Some(i..i + n)
    }

    #[test]
    fn next_occurrence_finds_what_a_char_by_char_scan_does() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let alphabet = ["a", "b", "é", "😀", " ", "e\u{301}"];
        // across the 64 KB blocks the rope is read in: a match can straddle one
        let mut texts: Vec<String> =
            (0..3).map(|_| (0..20_000).map(|_| alphabet[rand(alphabet.len())]).collect()).collect();
        for at in 65_530..65_540 {
            texts.push(format!("{}é😀a{}", " ".repeat(at), " ".repeat(70_000)));
        }
        texts.push("aaaa".into());
        for text in &texts {
            let mut s = EditorState::from_text(text);
            let len = s.len();
            for _ in 0..20 {
                let a = rand(len);
                let b = (a + 1 + rand(6)).min(len);
                let from = Selection::new(a, b);
                s.view.selections = (0..rand(4)).map(|_| Selection::caret(rand(len))).collect();
                s.view.selections.push(from);
                for whole_word in [false, true] {
                    assert_eq!(
                        s.next_occurrence(from, whole_word),
                        naive_next(&s, from, whole_word),
                        "{from:?} whole word {whole_word}"
                    );
                }
            }
        }
        // overlapping occurrences count, and the search wraps
        let mut s = EditorState::from_text("aaaa");
        s.view.selections = vec![Selection::new(0, 2)];
        assert_eq!(s.next_occurrence(Selection::new(0, 2), false), Some(2..4));
        s.view.selections.push(Selection::new(2, 4));
        assert_eq!(s.next_occurrence(Selection::new(2, 4), false), Some(1..3));
    }

    #[test]
    fn cluster_navigation_clamps_offsets_past_the_document() {
        let s = EditorState::from_text("abc");
        assert_eq!(s.next_cluster(usize::MAX), s.len());
        assert_eq!(s.prev_cluster(usize::MAX), 2);
        assert_eq!(s.snap_to_cluster(usize::MAX), s.len());
    }
}

/// Regex operation on each complete selection, without changing source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionSetOp {
    Split,
    Keep,
    Remove,
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    /// Complement the physical union; carets and virtual space consume no source.
    pub fn invert_selections(&mut self) {
        let mut ranges: Vec<_> = self.view.selections.iter().map(Selection::range).collect();
        ranges.sort_by_key(|r| (r.start, r.end));
        let mut out = Vec::new();
        let mut end = 0;
        for r in ranges {
            if r.is_empty() {
                continue;
            }
            if end < r.start {
                out.push(Selection::new(end, r.start));
            }
            end = end.max(r.end);
        }
        if end < self.len() {
            out.push(Selection::new(end, self.len()));
        }
        self.apply_selection_set(out, 0);
    }

    /// Match independently against each complete selected string. Prepare every result before
    /// touching history, so syntax errors, timeouts and runaway patterns are atomic failures.
    pub fn regex_selections(&mut self, query: &str, op: SelectionSetOp, ignore_case: bool) -> Result<(), String> {
        let opts = FindOpts { regex: true, case_sensitive: !ignore_case, ..FindOpts::default() };
        let pattern = Pattern::new(query, &opts)?;
        let deadline = Deadline::new(crate::search::pattern::UI_SECONDS);
        let mut out = Vec::new();
        let mut primary = 0;
        for (i, sel) in self.view.selections.iter().enumerate() {
            let text = self.doc.rope.slice(sel.range()).to_string();
            if deadline.passed() {
                return Err(SearchError::TimedOut(crate::search::pattern::UI_SECONDS).message());
            }
            let before = out.len();
            match op {
                SelectionSetOp::Keep | SelectionSetOp::Remove => {
                    let matched = pattern
                        .captures_from(&text, 0..text.len(), &|| deadline.passed())
                        .map_err(|e| deadline.explain(e).message())?
                        .is_some();
                    if matched == (op == SelectionSetOp::Keep) {
                        out.push(*sel);
                    }
                }
                SelectionSetOp::Split => {
                    let found = crate::search::model::regex_matches(&text, &pattern, &|| deadline.passed())
                        .map_err(|e| deadline.explain(e).message())?;
                    if found.is_empty() {
                        out.push(*sel);
                    } else {
                        let count = text.chars().count();
                        let mut start = 0;
                        for m in found.iter().cloned().chain(std::iter::once(count..count)) {
                            if start < m.start {
                                let a = (sel.start() + start, if start == 0 { sel.start_point().1 } else { 0 });
                                let b = (sel.start() + m.start, if m.start == count { sel.end_point().1 } else { 0 });
                                let part = if sel.head_point() < sel.anchor_point() {
                                    Selection::from_points(b, a)
                                } else {
                                    Selection::from_points(a, b)
                                };
                                out.push(Selection { rectangular: sel.rectangular, ..part });
                            }
                            start = m.end;
                        }
                    }
                }
            }
            if deadline.passed() {
                return Err(SearchError::TimedOut(crate::search::pattern::UI_SECONDS).message());
            }
            if i <= self.primary() && out.len() > before {
                primary = out.len() - 1;
            }
        }
        // A half-CRLF endpoint cannot represent delimiter exclusion under the caret invariant.
        if op == SelectionSetOp::Split
            && out.iter().any(|s| self.off_cr_lf(s.anchor) != s.anchor || self.off_cr_lf(s.head) != s.head)
        {
            return Err("Cannot split between CR and LF (CRLF); match the complete line ending instead".into());
        }
        self.apply_selection_set(out, primary);
        Ok(())
    }

    fn apply_selection_set(&mut self, mut selections: Vec<Selection>, primary: usize) {
        if selections.is_empty() {
            selections.push(Selection::caret(self.cursor().min(self.len())));
        }
        let old = self.selection_state();
        self.view.selections = selections;
        self.view.primary = primary.min(self.view.selections.len() - 1);
        self.normalize();
        let next = self.sel_now();
        (self.view.selections, self.view.primary) = (old.selections.clone(), old.primary);
        if next != old {
            self.restore_selection_state(next);
        }
    }
}
