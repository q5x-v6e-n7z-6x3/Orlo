//! Code folding: a view's folds, which hide any range of text behind a placeholder, and how
//! they join the lines whose line breaks they hide into display lines.
//!
//! A fold hides the chars of its range. An offset strictly inside one is hidden and drawn at
//! the fold's start; an offset at its start is drawn before the placeholder, one at its end
//! after it. Folds nest: an inner fold stays when the outer one opens. A fold over part of
//! another takes it in, the two becoming one over both; folds that only touch stay two. An edit
//! moves a fold with its text when it lies wholly before it (text typed at its start goes
//! before it) or wholly after it (text typed at its end goes after it); an edit that takes
//! text from inside it or puts text inside it opens it.

use crate::buffer::{Change, Document};
use ropey::{Rope, RopeSlice};
use std::cmp::Reverse;
use std::ops::Range;

pub use super::folded::{Folded, Piece, PLACEHOLDER, PLACEHOLDER_COLS};

/// A fold: its hidden chars, and the lines whose line break before them it hides (from the
/// line after its start's to the line its end is on, none for a fold inside a line), kept in
/// step with the chars so no frame looks them up.
type Fold = (Range<usize>, Range<usize>);

/// Discovered multiline regions for one immutable rope snapshot. Unlike active `Folds`,
/// these are candidates shared by commands and gutter hit testing.
#[derive(Clone, Debug, Default)]
pub struct Regions {
    entries: Vec<Region>,
}

#[derive(Clone, Debug)]
struct Region {
    chars: Range<usize>,
    header: usize,
    last: usize,
    parent: Option<usize>,
    level: usize,
}

impl Regions {
    /// Canonicalize syntax candidates: one outermost region per header, no partial overlaps.
    /// All offsets are chars in `rope`; an invalid or single-line candidate is ignored.
    pub fn new(rope: &Rope, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        let mut ranges: Vec<_> = ranges.into_iter().filter(|r| r.start < r.end && r.end <= rope.len_chars()).collect();
        ranges.sort_unstable_by_key(|r| (r.start, Reverse(r.end)));
        let mut entries: Vec<Region> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for chars in ranges {
            let header = rope.char_to_line(chars.start);
            let last = rope.char_to_line(chars.end - 1);
            if header >= last || entries.last().is_some_and(|r| r.header == header) {
                continue;
            }
            while open.last().is_some_and(|&i| entries[i].chars.end <= chars.start) {
                open.pop();
            }
            let parent = open.last().copied();
            if parent.is_some_and(|i| chars.end > entries[i].chars.end) {
                continue;
            }
            let level = parent.map_or(1, |i| entries[i].level + 1);
            open.push(entries.len());
            entries.push(Region { chars, header, last, parent, level });
        }
        Self { entries }
    }

    /// Every current discovered range, without requiring a collapsed fold.
    pub fn chars(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.entries.iter().map(|r| r.chars.clone())
    }

    pub fn at_header(&self, line: usize) -> Option<Range<usize>> {
        let i = self.entries.binary_search_by_key(&line, |r| r.header).ok()?;
        Some(self.entries[i].chars.clone())
    }

    pub fn innermost(&self, line: usize) -> Option<Range<usize>> {
        let mut i = self.entries.partition_point(|r| r.header <= line).checked_sub(1)?;
        loop {
            let r = &self.entries[i];
            if line <= r.last {
                return Some(r.chars.clone());
            }
            i = r.parent?;
        }
    }

    pub fn at_level(&self, level: usize) -> Vec<Range<usize>> {
        self.entries.iter().filter(|r| r.level == level).map(|r| r.chars.clone()).collect()
    }
}

/// Source-ordered fold navigation; selection extension belongs to the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Next,
    Previous,
    Parent,
}

/// Valid ordinary/discovered fold ranges, including visible multiline headers and exact
/// inline selections. Built on command entry, never from a filter's exclusion placeholders.
pub struct Traversal {
    ranges: Vec<Candidate>,
    len: usize,
}

struct Candidate {
    chars: Range<usize>,
    header: usize,
}

impl Traversal {
    pub fn new(doc: &Document, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        let mut ranges: Vec<_> = ranges
            .into_iter()
            .filter(|r| r.start < r.end && r.end <= doc.len())
            .map(|chars| {
                let line = doc.point(chars.start).0;
                let header = if line < doc.point(chars.end).0 { doc.line_start(line) } else { chars.start };
                Candidate { chars, header }
            })
            .collect();
        ranges.sort_unstable_by_key(|r| (r.header, r.chars.start, Reverse(r.chars.end)));
        ranges.dedup_by(|a, b| a.chars == b.chars);
        Self { ranges, len: doc.len() }
    }

    fn current_fold(&self, at: usize) -> Option<&Candidate> {
        self.ranges
            .iter()
            .filter(|r| r.header <= at && (at < r.chars.end || at == self.len && r.chars.end == at))
            .min_by_key(|r| r.chars.len())
    }

    pub fn current(&self, at: usize) -> Option<Range<usize>> {
        self.current_fold(at).map(|r| r.header..r.chars.end)
    }

    pub fn destination(&self, at: usize, motion: Motion) -> Option<usize> {
        match motion {
            Motion::Next => self.ranges.iter().find(|r| r.header > at).map(|r| r.header),
            Motion::Previous => self.ranges.iter().rev().find(|r| r.header < at).map(|r| r.header),
            Motion::Parent => {
                let current = &self.current_fold(at)?.chars;
                // Containment uses raw ranges: distinct folds may share a header and end.
                self.ranges
                    .iter()
                    .filter(|r| r.chars.start <= current.start && current.end <= r.chars.end && r.chars != *current)
                    .min_by_key(|r| r.chars.len())
                    .map(|r| r.header)
            }
        }
    }
}

/// A view's folds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folds {
    /// Every fold, sorted by start, an outer fold before those inside it: folds nest or lie
    /// apart (touching or not), none overlaps another in part.
    ranges: Vec<Fold>,
    /// The outermost folds: what is hidden, each behind a placeholder. Sorted and apart.
    top: Vec<Fold>,
    /// The lines joined onto the display line above them (their line break before them is
    /// hidden), runs that touch merged. Sorted and apart.
    joined: Vec<Range<usize>>,
    /// The joined lines through each run of `joined`.
    joined_total: Vec<usize>,
}

/// Visible physical source ranges for transient narrowing. Exclusions use ordinary folds;
/// text, history and the row/source coordinate mapping remain the document's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Focus {
    ranges: Vec<Range<usize>>,
}

impl Focus {
    pub fn new(len: usize, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        let mut ranges: Vec<_> = ranges.into_iter().map(|r| r.start.min(len)..r.end.min(len)).collect();
        ranges.retain(|r| r.start <= r.end);
        ranges.sort_unstable_by_key(|r| (r.start, r.end));
        let mut out: Vec<Range<usize>> = Vec::new();
        for r in ranges {
            match out.last_mut() {
                Some(p) if r.start <= p.end => p.end = p.end.max(r.end),
                _ => out.push(r),
            }
        }
        Self { ranges: out }
    }

    /// Include inserted/replacement text at either edge, as well as edits in the interior.
    pub fn map(&mut self, len: usize, changes: &[Change]) {
        let mut starts: Vec<_> = self.ranges.iter().map(|r| r.start).collect();
        let mut ends: Vec<_> = self.ranges.iter().map(|r| r.end).collect();
        crate::buffer::map_sorted(&mut starts, changes);
        crate::buffer::map_sorted_starts(&mut ends, changes);
        *self = Self::new(len, starts.into_iter().zip(ends).map(|(a, b)| a..b.max(a)));
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// With the whole lines holding the offsets `at` too, as `matching_lines` gives lines.
    pub fn with_lines(self, rope: &Rope, at: impl IntoIterator<Item = usize>) -> Self {
        let line = |at: usize| {
            let l = rope.char_to_line(at.min(rope.len_chars()));
            rope.line_to_char(l)..rope.line_to_char((l + 1).min(rope.len_lines()))
        };
        Self::new(rope.len_chars(), self.ranges.into_iter().chain(at.into_iter().map(line)))
    }

    pub fn contains(&self, at: usize) -> bool {
        let i = self.ranges.partition_point(|r| r.end < at);
        self.ranges.get(i).is_some_and(|r| r.start <= at)
    }

    /// A bounded source caret even when nothing matches. Equal distances prefer the left.
    pub fn nearest(&self, at: usize) -> usize {
        let i = self.ranges.partition_point(|r| r.end < at);
        match self.ranges.get(i) {
            Some(r) if r.start <= at => at,
            Some(r) => match i.checked_sub(1).map(|j| self.ranges[j].end) {
                Some(left) if at - left <= r.start - at => left,
                _ => r.start,
            },
            None => self.ranges.last().map_or(0, |r| r.end),
        }
    }

    pub fn exclusions(&self, doc: &Document) -> Folds {
        let mut end = 0;
        let mut hidden = Vec::new();
        for r in &self.ranges {
            if end < r.start {
                hidden.push(end..r.start);
            }
            end = r.end;
        }
        if end < doc.len() {
            hidden.push(end..doc.len());
        }
        let mut folds = Folds::default();
        folds.fold_many(doc, hidden);
        folds
    }

    /// Match the complete source with the existing bounded regex engine. A match ending
    /// at a line start belongs to the preceding line; a zero-width match belongs there.
    pub fn matching_lines(
        rope: &Rope,
        pattern: &crate::search::pattern::Pattern,
        context: usize,
        stop: &dyn Fn() -> bool,
    ) -> Result<Self, crate::search::pattern::SearchError> {
        let text = rope.to_string();
        let matches = crate::search::model::regex_matches(&text, pattern, stop)?;
        let ranges = matches.into_iter().map(|m| {
            let first = rope.char_to_line(m.start).saturating_sub(context);
            let last = rope.char_to_line(if m.is_empty() { m.end } else { m.end - 1 });
            let after = last.saturating_add(context).saturating_add(1).min(rope.len_lines());
            rope.line_to_char(first)..rope.line_to_char(after)
        });
        Ok(Self::new(rope.len_chars(), ranges))
    }
}

/// Whether fold `f` is over part of `r` only: they overlap, and neither holds the other.
fn in_part(f: &Range<usize>, r: &Range<usize>) -> bool {
    let inside = r.start <= f.start && f.end <= r.end;
    let around = f.start <= r.start && r.end <= f.end;
    f.start < r.end && r.start < f.end && !inside && !around
}

impl Folds {
    /// No folds, for a wrap map of plain lines.
    pub const NONE: Folds = Folds { ranges: Vec::new(), top: Vec::new(), joined: Vec::new(), joined_total: Vec::new() };

    /// The lines `range` joins, in `s` as it is.
    fn lines_of(s: &Document, range: &Range<usize>) -> Range<usize> {
        let line = |o: usize| s.rope.char_to_line(o.min(s.len()));
        line(range.start) + 1..line(range.end) + 1
    }

    /// The outermost folds and the joined lines, after the folds changed.
    fn recount(&mut self) {
        #[cfg(any(test, feature = "test-support"))]
        RECOUNTED.set(RECOUNTED.get() + 1);
        self.top.clear();
        for f in &self.ranges {
            match self.top.last() {
                Some(t) if f.0.start < t.0.end => {}
                _ => self.top.push(f.clone()),
            }
        }
        self.joined.clear();
        for (_, l) in &self.top {
            match self.joined.last_mut() {
                Some(p) if l.start <= p.end => p.end = p.end.max(l.end),
                _ if !l.is_empty() => self.joined.push(l.clone()),
                _ => {}
            }
        }
        self.joined_total = (self.joined.iter())
            .scan(0, |total, j| {
                *total += j.len();
                Some(*total)
            })
            .collect();
    }

    /// Where fold `r` is in `ranges`, or would go.
    fn find(&self, r: &Range<usize>) -> Result<usize, usize> {
        let key = |(f, _): &Fold| (f.start, Reverse(f.end));
        (self.ranges).binary_search_by_key(&(r.start, Reverse(r.end)), key)
    }

    /// Keep the folds `keep` takes, and recount when that unfolded any; whether it did.
    fn retain(&mut self, keep: impl FnMut(&Fold) -> bool) -> bool {
        let n = self.ranges.len();
        self.ranges.retain(keep);
        let unfolded = self.ranges.len() < n;
        if unfolded {
            self.recount();
        }
        unfolded
    }

    /// Hide `range` behind a placeholder; whether that changed anything. A fold over part of
    /// another takes it in: they become one fold, over both.
    pub fn fold(&mut self, s: &Document, range: Range<usize>) -> bool {
        let mut r = range.start.min(s.len())..range.end.min(s.len());
        if r.is_empty() {
            return false;
        }
        // the folds it overlaps in part widen it, until it overlaps none: each pass takes in
        // those over part of it as it was when the pass began
        let mut merged = false;
        loop {
            let (n, was) = (self.ranges.len(), r.clone());
            self.ranges.retain(|(f, _)| {
                let part = in_part(f, &was);
                if part {
                    r = r.start.min(f.start)..r.end.max(f.end);
                }
                !part
            });
            if self.ranges.len() == n {
                break;
            }
            merged = true;
        }
        let added = match self.find(&r) {
            Ok(_) => false,
            Err(i) => {
                self.ranges.insert(i, (r.clone(), Self::lines_of(s, &r)));
                true
            }
        };
        self.recount();
        added || merged
    }

    /// Hide lines `header + 1 ..= last`: from the header's line end to the last line's end.
    pub fn fold_lines(&mut self, s: &Document, header: usize, last: usize) {
        let last = last.min(s.line_count() - 1);
        if header < last {
            self.fold(s, s.line_end(header)..s.line_end(last));
        }
    }

    /// Unfold the folds any of `ranges` (sorted) touches, their ends included; whether there
    /// were any.
    pub fn unfold_at(&mut self, ranges: &[Range<usize>]) -> bool {
        self.retain(|(f, _)| {
            // the first range not ending before the fold touches it, unless it starts past it
            let i = ranges.partition_point(|r| r.end < f.start);
            ranges.get(i).is_none_or(|r| r.start > f.end)
        })
    }

    /// Unfold the folds that hide the char at `at` (a header's line break), the folds inside
    /// them that do not hide it staying; whether there were any.
    pub fn unfold_over(&mut self, at: usize) -> bool {
        self.retain(|(f, _)| !(f.start <= at && at < f.end))
    }

    /// Unfold the outermost fold that starts at `start`, the folds inside it staying; whether
    /// there was one.
    pub fn unfold_from(&mut self, start: usize) -> bool {
        let i = self.ranges.partition_point(|(f, _)| f.start < start);
        let found = self.ranges.get(i).is_some_and(|(f, _)| f.start == start);
        if found {
            self.ranges.remove(i);
            self.recount();
        }
        found
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Number of active ranges, including nested folds, without copying them.
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Each fold's hidden text in chars, in order.
    pub fn chars(&self) -> Vec<Range<usize>> {
        self.ranges.iter().map(|(c, _)| c.clone()).collect()
    }

    /// Whether a fold hides the line break after `header` (and so the lines after it).
    pub fn is_folded(&self, header: usize) -> bool {
        let i = self.joined.partition_point(|j| j.start <= header);
        self.joined.get(i).is_some_and(|j| j.start == header + 1)
    }

    /// Follow edits already made to the text: an edit wholly before a fold moves it, one wholly
    /// after it (from its end on) leaves it, and one that reaches inside it opens it.
    pub fn map(&mut self, changes: &[Change]) {
        if self.ranges.is_empty() {
            return;
        }
        // one sweep a run of changes (ascending, in the coordinates before it): thousands of
        // carets typing over thousands of folds
        for run in crate::buffer::change_runs(changes) {
            // the changes wholly before this fold, and the chars and lines they added
            let (mut k, mut chars, mut lines) = (0, 0isize, 0isize);
            self.ranges.retain_mut(|(f, l)| {
                #[cfg(any(test, feature = "test-support"))]
                VISITED.set(VISITED.get() + 1);
                while let Some(c) = run.get(k).filter(|c| c.start + c.removed <= f.start) {
                    chars += c.inserted as isize - c.removed as isize;
                    lines += c.lines.1 as isize - c.lines.0 as isize;
                    k += 1;
                }
                // the next one reaches inside it unless it starts at its end or past it
                if run.get(k).is_some_and(|c| c.start < f.end) {
                    return false;
                }
                let by = |x: usize, d: isize| x.saturating_add_signed(d);
                *f = by(f.start, chars)..by(f.end, chars);
                *l = by(l.start, lines)..by(l.end, lines);
                true
            });
        }
        self.recount();
    }

    /// The lines joined onto the display line above them, sorted, touching runs merged.
    pub fn hidden_lines(&self, s: &Document) -> Vec<Range<usize>> {
        let n = s.line_count();
        (self.joined.iter()).map(|j| j.start.min(n)..j.end.min(n)).filter(|j| !j.is_empty()).collect()
    }

    /// `offsets` (sorted) strictly inside a fold's hidden text go to its start (the outermost
    /// fold's, for nested ones); a fold's end and everything outside stay.
    pub fn out_of_hidden(&self, offsets: &mut [usize]) {
        // the later folds first: an offset moved to a fold's start inside another fold moves on
        for (f, _) in self.ranges.iter().rev() {
            let a = offsets.partition_point(|&o| o <= f.start);
            let b = offsets.partition_point(|&o| o < f.end).max(a);
            offsets[a..b].fill(f.start);
        }
    }

    /// Whether any of `offsets` is strictly inside a fold's hidden text: what `reveal` would
    /// open, asked without changing anything.
    pub fn hides_any(&self, offsets: &[usize]) -> bool {
        !self.top.is_empty() && offsets.iter().any(|&o| self.hiding(o).is_some())
    }

    /// Fold every one of `ranges` in one pass, as `fold` would one by one with the folds there
    /// are, all of them by start and the outer first: tens of thousands of them (Fold Level,
    /// another view's folds carried over) cost a sort, not a pass each.
    pub fn fold_many(&mut self, s: &Document, ranges: impl IntoIterator<Item = Range<usize>>) {
        let len = s.len();
        let mut all: Vec<Range<usize>> = self.ranges.iter().map(|(c, _)| c.clone()).collect();
        let new = ranges.into_iter().map(|r| r.start.min(len)..r.end.min(len));
        all.extend(new.filter(|r| !r.is_empty()));
        all.sort_unstable_by_key(|r| (r.start, Reverse(r.end)));
        all.dedup();
        // a sweep with the folds still open, outermost first: a fold that runs on past the
        // innermost open one is over part of it, and they become one, which in turn takes in
        // the open ones around it that end before it does
        let mut out: Vec<Option<Range<usize>>> = Vec::with_capacity(all.len());
        let mut open: Vec<usize> = Vec::new();
        let end = |out: &[Option<Range<usize>>], i: usize| out[i].as_ref().map_or(0, |r| r.end);
        for r in all {
            while open.last().is_some_and(|&i| end(&out, i) <= r.start) {
                open.pop();
            }
            match open.last().copied() {
                Some(mut t) if r.end > end(&out, t) => {
                    let start = out[t].as_ref().map_or(r.start, |o| o.start);
                    out[t] = Some(start..r.end);
                    while let Some(&p) = open.len().checked_sub(2).map(|i| &open[i]) {
                        if end(&out, p) >= r.end {
                            break;
                        }
                        let start = out[p].as_ref().map_or(r.start, |o| o.start);
                        (out[p], out[t]) = (Some(start..r.end), None);
                        open.pop();
                        t = p;
                    }
                }
                _ => {
                    out.push(Some(r));
                    open.push(out.len() - 1);
                }
            }
        }
        // an inner fold grown to its parent's very range is that fold twice
        let mut out: Vec<Range<usize>> = out.into_iter().flatten().collect();
        out.sort_unstable_by_key(|r| (r.start, Reverse(r.end)));
        out.dedup();
        self.ranges = out.into_iter().map(|r| (r.clone(), Self::lines_of(s, &r))).collect();
        self.recount();
    }

    /// Unfold the folds with any of `offsets` strictly inside their hidden text; whether there
    /// were any.
    pub fn reveal(&mut self, offsets: &[usize]) -> bool {
        if self.ranges.is_empty() {
            return false;
        }
        let mut offsets = offsets.to_vec();
        offsets.sort_unstable();
        self.retain(|(f, _)| {
            let i = offsets.partition_point(|&o| o <= f.start);
            offsets.get(i).is_none_or(|&o| o >= f.end)
        })
    }

    /// The outermost fold `offset` is strictly inside: what hides it.
    pub fn hiding(&self, offset: usize) -> Option<Range<usize>> {
        let i = self.top.partition_point(|(f, _)| f.start < offset);
        let (f, _) = self.top.get(i.checked_sub(1)?)?;
        (offset < f.end).then(|| f.clone())
    }

    /// The outermost folds that start in `range`, in order: those a display line's text shows
    /// as placeholders.
    pub fn top_in(&self, range: Range<usize>) -> impl Iterator<Item = Range<usize>> + '_ {
        let i = self.top.partition_point(|(f, _)| f.start < range.start);
        (self.top[i..].iter()).take_while(move |(f, _)| f.start < range.end).map(|(f, _)| f.clone())
    }

    /// Whether a fold hides the line break before `line`, joining it onto the line above.
    pub fn is_joined(&self, line: usize) -> bool {
        let i = self.joined.partition_point(|j| j.end <= line);
        self.joined.get(i).is_some_and(|j| j.start <= line)
    }

    /// The first and last buffer lines of the display line `line` is on.
    pub fn display_lines(&self, line: usize) -> (usize, usize) {
        let i = self.joined.partition_point(|j| j.end <= line);
        match self.joined.get(i) {
            // joined: the run's head is the line above it
            Some(j) if j.start <= line => (j.start - 1, j.end - 1),
            // a head: the run after it, if it starts below it
            Some(j) if j.start == line + 1 => (line, j.end - 1),
            _ => (line, line),
        }
    }

    /// The display line of buffer line `line`, as a row of an unwrapped view: the lines above
    /// it less those joined onto others.
    pub fn display_line(&self, line: usize) -> usize {
        let (head, _) = self.display_lines(line);
        let i = self.joined.partition_point(|j| j.end <= head);
        head - i.checked_sub(1).map_or(0, |i| self.joined_total[i])
    }

    /// The first buffer line of display line `row` (`display_line`'s inverse).
    pub fn line_of_display(&self, row: usize) -> usize {
        // the line after each run of joined lines is on the row its end gives, less the lines
        // joined so far: those rows rise with the runs
        let (mut lo, mut hi) = (0, self.joined.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.joined[mid].end - self.joined_total[mid] <= row {
                true => lo = mid + 1,
                false => hi = mid,
            }
        }
        row + lo.checked_sub(1).map_or(0, |i| self.joined_total[i])
    }

    /// The lines whose display lines a change from `old` to `new` folds may have changed: each
    /// fold in one but not the other, from its first line through its last.
    pub fn changed_lines(old: &Folds, new: &Folds, s: &Document) -> Vec<Range<usize>> {
        let line = |o: usize| s.rope.char_to_line(o.min(s.len()));
        let mut out: Vec<Range<usize>> = Vec::new();
        for (a, b) in [(old, new), (new, old)] {
            for (f, _) in &a.ranges {
                if b.find(f).is_err() {
                    out.push(line(f.start)..line(f.end) + 1);
                }
            }
        }
        out.sort_unstable_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for r in out {
            match merged.last_mut() {
                Some(p) if r.start <= p.end => p.end = p.end.max(r.end),
                _ => merged.push(r),
            }
        }
        merged
    }

    /// All the lines joined onto others: an unwrapped view has as many rows fewer.
    pub fn joined_lines(&self) -> usize {
        self.joined_total.last().copied().unwrap_or(0)
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Folds `Folds::map` visited on this thread: once a run of changes, not once a change.
    static VISITED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Times the outermost folds and joined lines were worked out again on this thread.
    static RECOUNTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Lines `indent` measured on this thread: how far the region finders read.
    pub static MEASURED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The leading white space of `line` in columns, tabs to their stops; `None` for a blank line.
pub fn indent(line: RopeSlice, tab_size: usize) -> Option<usize> {
    #[cfg(any(test, feature = "test-support"))]
    MEASURED.set(MEASURED.get() + 1);
    let mut col = 0;
    for c in line.chars() {
        if !c.is_whitespace() {
            return Some(col);
        }
        if c == '\n' {
            break;
        }
        col += if c == '\t' { tab_size - col % tab_size } else { 1 };
    }
    None
}

/// Indentation regions as (header, last line), sorted, inner ones nested in outer ones: a
/// header is followed by a non-blank line indented more, and its block runs to the last
/// non-blank line before one indented the same or less. A whole-document pass.
pub fn regions(rope: &Rope, tab_size: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    // the headers whose blocks are open, outermost first: (indent, line)
    let mut open: Vec<(usize, usize)> = Vec::new();
    // the last non-blank line so far, and its indent
    let mut prev: Option<(usize, usize)> = None;
    for (l, line) in rope.lines().enumerate() {
        let Some(i) = indent(line, tab_size) else {
            continue;
        };
        if let Some((p, pi)) = prev {
            while let Some(&(_, h)) = open.last().filter(|&&(oi, _)| oi >= i) {
                open.pop();
                out.push((h, p));
            }
            if pi < i {
                open.push((pi, p));
            }
        }
        prev = Some((l, i));
    }
    if let Some((p, _)) = prev {
        out.extend(open.into_iter().map(|(_, h)| (h, p)));
    }
    out.sort_unstable();
    out
}

/// Fold Level `level`: the regions whose header is indented `level - 1` tab stops.
pub fn at_level(rope: &Rope, tab_size: usize, level: usize) -> Vec<(usize, usize)> {
    let mut found = regions(rope, tab_size);
    found.retain(|&(h, _)| indent(rope.line(h), tab_size).unwrap_or(0) / tab_size + 1 == level);
    found
}

/// Whether `line` heads a region: the next non-blank line is indented more. Looks
/// at most 100 lines down for it, as the indent guides do; past a longer blank run the gutter
/// shows no button (Fold still finds the region).
pub fn heads(s: &Document, line: usize) -> bool {
    let tab = s.tab_size;
    let Some(h) = indent(s.rope.line(line), tab) else {
        return false;
    };
    (s.rope.lines_at(line + 1).take(100)).filter_map(|l| indent(l, tab)).next().is_some_and(|i| i > h)
}

/// The region `header` heads, as (header, last line): through the last non-blank line before
/// one indented the same or less. Scans its block, not the document.
pub fn region(s: &Document, header: usize) -> Option<(usize, usize)> {
    let tab = s.tab_size;
    let h = indent(s.rope.line(header), tab)?;
    let mut last = None;
    for (l, line) in (header + 1..).zip(s.rope.lines_at(header + 1)) {
        match indent(line, tab) {
            Some(i) if i <= h => break,
            Some(_) => last = Some(l),
            None => {}
        }
    }
    last.map(|last| (header, last))
}

/// The innermost region holding `line` or headed by it, scanning out from `line`: its header
/// is the nearest line above indented less than it (than the next non-blank line, for a blank
/// one).
pub fn innermost(s: &Document, line: usize) -> Option<(usize, usize)> {
    if let Some(r) = region(s, line) {
        return Some(r);
    }
    let tab = s.tab_size;
    let within = match indent(s.rope.line(line), tab) {
        Some(i) => i,
        None => s.rope.lines_at(line + 1).find_map(|l| indent(l, tab))?,
    };
    let above = (0..line).rev().zip(s.rope.lines_at(line).reversed());
    let (header, _) = above.filter_map(|(l, text)| Some((l, indent(text, tab)?))).find(|&(_, i)| i < within)?;
    region(s, header)
}

/// `innermost` at each of `lines` (ascending), from one pass over the document: for many carets,
/// where scanning out from each would read a long block once a caret.
pub fn innermost_each(rope: &Rope, tab_size: usize, lines: &[usize]) -> Vec<Option<(usize, usize)>> {
    let mut all = regions(rope, tab_size).into_iter().peekable();
    // the regions headed at or above the line, latest header on top; one that ended above
    // the line holds none of the lines after it either
    let mut open: Vec<(usize, usize)> = Vec::new();
    (lines.iter())
        .map(|&l| {
            open.extend(std::iter::from_fn(|| all.next_if(|r| r.0 <= l)));
            while open.pop_if(|r| r.1 < l).is_some() {}
            open.last().copied()
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn folding_inside_a_nested_fold_up_to_its_parent_keeps_one_fold() {
        let s = EditorState::from_text(&"x\n".repeat(12));
        let le = |l| s.doc.line_end(l);
        let mut f = Folds::default();
        f.fold(&s.doc, le(2)..le(10));
        f.fold(&s.doc, le(2)..le(5));
        f.fold_many(&s.doc, [le(4)..le(10)]);
        assert_eq!(f.chars(), vec![le(2)..le(10)]);
    }
    use crate::buffer::{EditorState, Selection};

    fn folded(s: &Document, folds: &[(usize, usize)]) -> Folds {
        let mut f = Folds::default();
        for &(header, last) in folds {
            f.fold_lines(s, header, last);
        }
        f
    }

    /// The folds' hidden text, after checking that they are sorted, nest or lie apart, keep
    /// their lines in step and that the outermost ones and the joined lines agree with them.
    fn chars(f: &Folds, s: &Document) -> Vec<Range<usize>> {
        let line = |o: usize| s.rope.char_to_line(o);
        for (i, (c, l)) in f.ranges.iter().enumerate() {
            assert!(!c.is_empty() && c.end <= s.len(), "{c:?}");
            assert_eq!(*l, line(c.start) + 1..line(c.end) + 1, "the lines of {c:?}");
            for (d, _) in &f.ranges[i + 1..] {
                let (apart, inside) = (d.start >= c.end, d.end <= c.end);
                assert!(d.start >= c.start && (apart || inside), "{c:?} then {d:?}");
            }
        }
        let mut want = f.clone();
        want.recount();
        assert_eq!((&f.top, &f.joined), (&want.top, &want.joined));
        f.ranges.iter().map(|(c, _)| c.clone()).collect()
    }

    #[test]
    fn syntax_regions_normalize_headers_crossings_and_input_order() {
        let rope = Rope::from_str("outer\ninner\nbody\nend\nnext\ntail\n");
        let mut candidates = vec![5..15, 5..21, 11..16, 11..15, 16..28, 25..31, 90..100, 5..21];
        for _ in 0..candidates.len() {
            for ranges in [candidates.clone(), candidates.iter().rev().cloned().collect()] {
                let regions = Regions::new(&rope, ranges);
                // One outermost region per header, with nested headers retained. The
                // crossing 16..28 is rejected, and duplicate 5..21 adds no level.
                assert_eq!(regions.at_header(0), Some(5..21));
                assert_eq!(regions.at_header(2), None);
                assert_eq!(regions.at_level(1), [5..21, 25..31]);
                assert_eq!(regions.at_level(2), [11..16]);
                assert!(regions.at_level(3).is_empty());
                // The outer ends at the next line's start; that line is outside it.
                assert_eq!(regions.innermost(3), Some(5..21));
                assert_eq!(regions.innermost(4), Some(25..31));
                assert_eq!(regions.innermost(6), None);
            }
            candidates.rotate_left(1);
        }
    }

    #[test]
    fn regions_are_indented_blocks_less_their_trailing_blank_lines() {
        // the closing brace is as indented as the header: it ends the block, outside it
        let s =
            EditorState::from_text("fn a() {\n    let x = 1;\n\n    if x {\n        y();\n    }\n\n}\n\nfn b() {}\n");
        assert_eq!(regions(&s.doc.rope, 4), [(0, 5), (3, 4)]);
        // a blank line after the header is skipped; the end of the file closes every block
        let s = EditorState::from_text("a:\n\n\tb\n\t\tc\n\n");
        assert_eq!(regions(&s.doc.rope, 4), [(0, 3), (2, 3)]);
        // a line indented less than the block but more than its header stays in it
        let s = EditorState::from_text("a\n    b\n  c\nd\n");
        assert_eq!(regions(&s.doc.rope, 4), [(0, 2)]);
        // tabs count to their stops
        let s = EditorState::from_text("x\n  y\n\tz\n");
        assert_eq!(regions(&s.doc.rope, 2), [(0, 2)]);
        assert_eq!(regions(&s.doc.rope, 4), [(0, 2), (1, 2)]);
        // three levels
        let s = EditorState::from_text("a\n b\n  c\n   d\n  e\nf\n");
        assert_eq!(regions(&s.doc.rope, 4), [(0, 4), (1, 4), (2, 3)]);
        assert!(regions(&Document::from_text("flat\ntext\n\n").rope, 4).is_empty());
        let mut s = EditorState::from_text("\tx\n\ty\n");
        s.doc.tab_size = 4;
        assert_eq!(indent(s.doc.rope.line(0), 4), Some(4));
        assert_eq!(indent(s.doc.rope.line(2), 4), None, "the empty last line is blank");
    }

    /// xorshift: this crate has no random number generator.
    pub(crate) fn next(seed: &mut u64) -> usize {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 33) as usize
    }

    #[test]
    fn folds_nest_take_in_those_they_overlap_in_part_and_stay_two_when_they_touch() {
        // "abcdefghij\nklmnop\n"
        let s = EditorState::from_text("abcdefghij\nklmnop\n");
        let mut f = Folds::default();
        assert!(f.fold(&s.doc, 2..8));
        assert!(f.fold(&s.doc, 4..6), "a fold inside another");
        assert_eq!(chars(&f, &s.doc), [2..8, 4..6]);
        assert_eq!(f.top_in(0..s.len()).collect::<Vec<_>>(), [2..8], "the outer one shows");
        assert!(!f.fold(&s.doc, 2..8), "the same fold again changes nothing");
        assert!(!f.fold(&s.doc, 3..3), "nothing to hide");
        // over part of 2..8: one fold over both; the one inside stays inside
        assert!(f.fold(&s.doc, 6..14));
        assert_eq!(chars(&f, &s.doc), [2..14, 4..6]);
        // touching: two folds, two placeholders
        assert!(f.fold(&s.doc, 14..16));
        assert_eq!(chars(&f, &s.doc), [2..14, 4..6, 14..16]);
        assert_eq!(f.top_in(0..s.len()).collect::<Vec<_>>(), [2..14, 14..16]);
        // what is hidden, and where
        assert_eq!(f.hiding(5), Some(2..14));
        assert_eq!((f.hiding(2), f.hiding(14), f.hiding(15)), (None, None, Some(14..16)));
        // 2..14 hides the line break at 10: line 1 is joined onto line 0
        assert!(f.is_joined(1) && f.is_folded(0) && !f.is_folded(1));
        assert_eq!(f.display_lines(1), (0, 1));
        assert_eq!(f.display_lines(2), (2, 2));
        assert_eq!(f.hidden_lines(&s.doc), [1..2]);
        // opening the outer fold shows the inner one's placeholder
        assert!(f.unfold_from(2));
        assert_eq!(chars(&f, &s.doc), [4..6, 14..16]);
        assert!(!f.is_joined(1));
    }

    #[test]
    fn display_lines_and_their_rows_count_the_lines_joined_above() {
        // ten lines; folds join 2..=4 onto 1 and 7..=8 onto 6, and one stays inside line 9
        let s = EditorState::from_text(&"line\n".repeat(10));
        let mut f = Folds::default();
        f.fold_lines(&s.doc, 1, 4);
        f.fold(&s.doc, s.line_start(6) + 2..s.line_start(8) + 1);
        f.fold(&s.doc, s.line_start(9) + 1..s.line_start(9) + 3);
        assert_eq!(f.hidden_lines(&s.doc), [2..5, 7..9]);
        assert_eq!(f.joined_lines(), 5);
        let rows: Vec<usize> = (0..s.line_count()).map(|l| f.display_line(l)).collect();
        assert_eq!(rows, [0, 1, 1, 1, 1, 2, 3, 3, 3, 4, 5]);
        let heads: Vec<usize> = (0..6).map(|r| f.line_of_display(r)).collect();
        assert_eq!(heads, [0, 1, 5, 6, 9, 10]);
        assert_eq!(f.display_lines(3), (1, 4));
        assert_eq!(f.display_lines(6), (6, 8));
    }

    #[test]
    fn edits_reaching_inside_a_fold_open_it_and_others_move_it() {
        // "head" hides 4..16 (the break after it to the end of "  two"); "tail" is 17..21
        let text = "head\n  one\n  two\ntail\n";
        // (replaced, typed, the fold after, after undo); an opened fold stays open
        let cases = [
            (0..0, "x", Some(5..17), Some(4..16)),
            (0..2, "", Some(2..14), Some(4..16)),
            // typed at its start, before the hidden text: the fold moves on
            (4..4, "!", Some(5..17), Some(4..16)),
            // the undo of this one takes the next line break, the fold's first char
            (4..4, "\n", Some(5..17), None),
            (8..8, "x", None, None),
            (4..5, "", None, None),
            // typed at its end, after the hidden text: the fold stays, the text shows after it
            (16..16, "x", Some(4..16), Some(4..16)),
            // the line after joined on: it shows after the placeholder
            (16..17, "", Some(4..16), Some(4..16)),
            // over its end: it opens
            (15..17, "", None, None),
            (17..17, "x", Some(4..16), Some(4..16)),
            (18..21, "", Some(4..16), Some(4..16)),
        ];
        for (at, typed, kept, undone) in cases {
            let mut s = EditorState::from_text(text);
            let mut folds = folded(&s.doc, &[(0, 2)]);
            assert_eq!(chars(&folds, &s.doc), [4..16]);
            s.set_selection(at.start, at.end);
            s.insert(typed);
            folds.map(&s.take_changes());
            assert_eq!(chars(&folds, &s.doc), Vec::from_iter(kept), "{typed:?} at {at:?}");
            s.undo();
            folds.map(&s.take_changes());
            let undone = Vec::from_iter(undone);
            assert_eq!(chars(&folds, &s.doc), undone, "undo of {typed:?} at {at:?}");
        }
        // a fold inside a line: typed at either edge it stays, inside it opens
        let s0 = "abcdefgh";
        let cases = [(2, Some(3..6)), (5, Some(2..5)), (6, Some(2..5)), (3, None), (4, None)];
        for (at, kept) in cases {
            let mut s = EditorState::from_text(s0);
            let mut f = Folds::default();
            f.fold(&s.doc, 2..5);
            s.set_cursor(at);
            s.insert("x");
            f.map(&s.take_changes());
            assert_eq!(chars(&f, &s.doc), Vec::from_iter(kept), "typed at {at}");
        }
        // carets before and after folds type together; one inside opens only its fold
        let mut s = EditorState::from_text("a\n b\n c\nd\n e\nf\n");
        let mut folds = folded(&s.doc, &[(0, 2), (3, 4)]);
        let (a, d) = (folds.ranges[0].0.clone(), folds.ranges[1].0.clone());
        s.set_selections(vec![Selection::caret(0), Selection::caret(9)]);
        s.insert("x");
        folds.map(&s.take_changes());
        let both = [a.start + 1..a.end + 1, d.start + 2..d.end + 2];
        assert_eq!(chars(&folds, &s.doc), both);
        // 13 is inside the second fold (11..14), not at its end
        s.set_selections(vec![Selection::caret(1), Selection::caret(13)]);
        s.insert("y");
        folds.map(&s.take_changes());
        assert_eq!(chars(&folds, &s.doc), [a.start + 2..a.end + 2]);
        // nested folds keep the inner one when the outer opens
        let s = EditorState::from_text("a\n b\n  c\n  d\n e\n");
        let mut folds = folded(&s.doc, &[(1, 3), (0, 4)]);
        assert_eq!(folds.hidden_lines(&s.doc), [1..5]);
        assert!(!folds.unfold_at(&[0..0]), "the header's text touches none");
        assert!(folds.unfold_at(&[1..1]));
        assert_eq!(folds.hidden_lines(&s.doc), [2..4]);
        assert!(!folds.unfold_at(&[1..1]));
        assert!(folds.reveal(&[s.line_start(2) + 1]));
        assert!(folds.hidden_lines(&s.doc).is_empty());
    }

    #[test]
    fn folds_kept_through_random_edits_stay_sorted_nested_or_apart() {
        let text: String = (0..30).map(|i| format!("{}l{i}\n", " ".repeat(i % 4))).collect();
        let pieces = ["", "x", "\n", "ab\ncd", "\n\n"];
        let mut seed = 0x2545_f491_4f6c_dd1d;
        for _ in 0..200 {
            let mut s = EditorState::from_text(&text);
            let mut folds = Folds::default();
            for _ in 0..5 {
                // lines, and ranges anywhere
                match next(&mut seed) % 2 {
                    0 => {
                        let header = next(&mut seed) % s.line_count();
                        folds.fold_lines(&s.doc, header, header + 1 + next(&mut seed) % 5);
                    }
                    _ => {
                        let a = next(&mut seed) % s.len();
                        folds.fold(&s.doc, a..a + 1 + next(&mut seed) % 30);
                    }
                }
                chars(&folds, &s.doc);
            }
            for _ in 0..6 {
                let a = next(&mut seed) % (s.len() + 1);
                let b = (a + next(&mut seed) % 4).min(s.len());
                s.set_selection(a, b);
                s.insert(pieces[next(&mut seed) % pieces.len()]);
                folds.map(&s.take_changes());
                chars(&folds, &s.doc);
            }
        }
    }

    /// The regions found at a line, by scanning out from it, against the whole-document pass.
    fn check_finders(text: &str, tab: usize) {
        let mut s = EditorState::from_text(text);
        s.doc.tab_size = tab;
        let all = regions(&s.doc.rope, tab);
        let lines: Vec<usize> = (0..s.line_count()).collect();
        for (l, each) in innermost_each(&s.doc.rope, tab, &lines).into_iter().enumerate() {
            let headed = all.iter().find(|r| r.0 == l).copied();
            assert_eq!(region(&s.doc, l), headed, "{text:?} at {tab}: line {l}");
            assert_eq!(heads(&s.doc, l), headed.is_some(), "{text:?} at {tab}: line {l}");
            // nested regions: the innermost holding a line has the latest header
            let around = (all.iter().filter(|r| r.0 <= l && l <= r.1)).max_by_key(|r| r.0).copied();
            assert_eq!(innermost(&s.doc, l), around, "{text:?} at {tab}: line {l}");
            assert_eq!(each, around, "{text:?} at {tab}: line {l}, in one pass");
        }
        // carets on some lines only, two to a line
        let some: Vec<usize> = (1..s.line_count()).step_by(3).flat_map(|l| [l, l]).collect();
        let one_by_one: Vec<_> = some.iter().map(|&l| innermost(&s.doc, l)).collect();
        assert_eq!(innermost_each(&s.doc.rope, tab, &some), one_by_one, "{text:?} at {tab}");
        for level in 1..=4 {
            let mut want = all.clone();
            want.retain(|&(h, _)| indent(s.doc.rope.line(h), tab).unwrap() / tab + 1 == level);
            assert_eq!(at_level(&s.doc.rope, tab, level), want, "{text:?} level {level}");
        }
    }

    #[test]
    fn regions_found_at_a_line_agree_with_the_whole_document() {
        for text in [
            "fn a() {\n    let x = 1;\n\n    if x {\n        y();\n    }\n\n}\n\nfn b() {}\n",
            "a:\n\n\tb\n\t\tc\n\n",
            "a\n    b\n  c\nd\n",
            "x\n  y\n\tz\n",
            "a\n b\n  c\n   d\n  e\nf\n",
            "\n\n  lead\nflat\n",
            "",
        ] {
            check_finders(text, 4);
            check_finders(text, 2);
        }
        let mut seed = 0x1234_5678_9abc_def1;
        for _ in 0..60 {
            let text: String = (0..30)
                .map(|_| match next(&mut seed) % 7 {
                    0 => "\n".to_string(),
                    1 => "\t\tt\n".to_string(),
                    n => format!("{}x\n", " ".repeat(2 * (n - 2))),
                })
                .collect();
            check_finders(&text, 4);
            check_finders(&text, 2);
        }
    }

    #[test]
    fn unfold_at_takes_sorted_ranges_and_is_folded_names_headers() {
        // folds 1..4, 6..9 and 11..14, headed by lines 0, 2 and 4
        let s = EditorState::from_text("a\n b\nc\n d\ne\n f\n");
        let mut folds = folded(&s.doc, &[(0, 1), (2, 3), (4, 5)]);
        assert!((0..6).all(|l| folds.is_folded(l) == [0, 2, 4].contains(&l)));
        assert!(folds.unfold_at(&[0..1, 9..9]), "touching ends count");
        assert_eq!(chars(&folds, &s.doc), [11..14]);
        assert!(!folds.unfold_at(&[15..15]));
        assert!(!folds.is_empty());
        assert!(folds.unfold_at(&[0..s.len()]));
        assert!(folds.is_empty());
    }

    #[test]
    fn revealing_or_unfolding_nothing_costs_no_recount() {
        // the view asks every frame; with thousands of folds that must not rebuild them
        let s = EditorState::from_text(&"head\n  body\n".repeat(2_000));
        let mut f = Folds::default();
        f.fold_many(&s.doc, (0..2_000).map(|i| s.line_end(2 * i)..s.line_end(2 * i + 1)));
        let before = RECOUNTED.get();
        let shown = [0, s.line_start(2), s.len()];
        assert!(!f.hides_any(&shown));
        assert!(!f.reveal(&shown));
        assert!(!f.unfold_at(&[0..0]));
        assert_eq!(RECOUNTED.get(), before);
        assert!(f.hides_any(&[s.line_start(1) + 1]));
    }

    #[test]
    fn many_folds_at_once_fold_as_one_by_one_and_in_one_pass() {
        let text: String = (0..60).map(|i| format!("{}l{i}\n", " ".repeat(i % 4))).collect();
        let s = EditorState::from_text(&text);
        let mut seed = 0x1357_9bdf_2468_ace0;
        for round in 0..200 {
            let ranges: Vec<Range<usize>> = (0..next(&mut seed) % 12)
                .map(|_| {
                    let a = next(&mut seed) % s.len();
                    a..a + next(&mut seed) % 40
                })
                .collect();
            let mut start = Folds::default();
            for _ in 0..next(&mut seed) % 4 {
                let a = next(&mut seed) % s.len();
                start.fold(&s.doc, a..a + 1 + next(&mut seed) % 30);
            }
            // one by one, in order (folding depends on it: a fold taken into another is gone
            // when a later one comes), as the pass takes them: the folds there were and the new
            // ones together, by start, the outer first
            let mut sorted: Vec<Range<usize>> = start.chars();
            sorted.extend(ranges.iter().cloned());
            sorted.sort_by_key(|r| (r.start, Reverse(r.end)));
            let mut one = Folds::default();
            for r in &sorted {
                one.fold(&s.doc, r.clone());
            }
            let mut many = start.clone();
            many.fold_many(&s.doc, ranges.iter().cloned());
            assert_eq!(chars(&many, &s.doc), chars(&one, &s.doc), "round {round}: {ranges:?}");
        }
        // Fold Level's tens of thousands of folds: one pass, one recount
        let s = EditorState::from_text(&"head\n  body\n".repeat(50_000));
        let mut f = Folds::default();
        let before = RECOUNTED.get();
        f.fold_many(&s.doc, (0..50_000).map(|i| s.line_end(2 * i)..s.line_end(2 * i + 1)));
        assert_eq!(RECOUNTED.get() - before, 1);
        assert_eq!(f.chars().len(), 50_000);
    }

    /// Before the sweep: each change visits every fold.
    fn map_one_by_one(f: &mut Folds, changes: &[Change]) {
        for c in changes {
            let end = c.start + c.removed;
            let moved = |x: usize| x + c.inserted - c.removed;
            let lines = |l: usize| (l + c.lines.1).saturating_sub(c.lines.0);
            f.ranges.retain_mut(|(f, l)| {
                if end <= f.start {
                    *f = moved(f.start)..moved(f.end);
                    *l = lines(l.start)..lines(l.end);
                    return true;
                }
                c.start >= f.end
            });
        }
        f.recount();
    }

    #[test]
    fn folds_follow_runs_of_changes_as_they_would_change_by_change() {
        let text: String = (0..40).map(|i| format!("{}l{i}\n", " ".repeat(i % 4))).collect();
        let pieces = ["", "x", "\n", "ab\ncd"];
        let mut seed = 0x0bad_cafe_f00d_d00d;
        for round in 0..150 {
            let mut s = EditorState::from_text(&text);
            let mut folds = Folds::default();
            for _ in 0..8 {
                let header = next(&mut seed) % s.line_count();
                folds.fold_lines(&s.doc, header, header + 1 + next(&mut seed) % 4);
            }
            for _ in 0..4 {
                let piece = pieces[next(&mut seed) % pieces.len()];
                let mut at: Vec<usize> =
                    (0..1 + next(&mut seed) % 6).map(|_| next(&mut seed) % (s.len() + 1)).collect();
                at.sort_unstable();
                at.dedup();
                match next(&mut seed) % 3 {
                    // typing at carets: an ascending run
                    0 => {
                        s.set_selections(at.iter().map(|&o| Selection::caret(o)).collect());
                        s.insert(piece);
                    }
                    // replacing ranges, applied last to first: a descending run
                    1 => {
                        let ends = at.windows(2).map(|w| w[0]..(w[0] + 2).min(w[1]));
                        let edits = ends.map(|r| (r, piece.to_string())).collect();
                        s.replace_ranges(edits);
                    }
                    _ => {
                        s.undo();
                    }
                }
                let changes = s.take_changes();
                let mut want = folds.clone();
                map_one_by_one(&mut want, &changes);
                folds.map(&changes);
                assert_eq!(folds, want, "round {round}: {changes:?}");
                chars(&folds, &s.doc);
            }
        }
    }

    #[test]
    fn folds_follow_thousands_of_changes_in_one_sweep() {
        // Fold Level's thousands of folds, then a keystroke at Find All's thousands of carets
        let s = EditorState::from_text(&"head\n  body\n".repeat(4_000));
        let mut folds = Folds::default();
        for h in (0..8_000).step_by(2) {
            folds.fold_lines(&s.doc, h, h + 1);
        }
        let changes: Vec<Change> =
            (0..2_000).map(|i| Change { start: s.line_start(i * 4) + i, inserted: 1, ..Change::default() }).collect();
        let mut want = folds.clone();
        map_one_by_one(&mut want, &changes);
        let before = VISITED.get();
        folds.map(&changes);
        assert_eq!(folds, want);
        // the changes are one run: each fold is visited once, not once a change
        assert_eq!(VISITED.get() - before, 4_000);
    }
}

#[cfg(test)]
mod n14_tests {
    use super::*;
    use crate::buffer::EditorState;
    use crate::search::{
        model::FindOpts,
        pattern::{Pattern, SearchError},
    };

    fn pattern(text: &str) -> Pattern {
        Pattern::new(text, &FindOpts { regex: true, case_sensitive: true, ..Default::default() }).unwrap()
    }

    #[test]
    fn n14_focus_uses_source_union_and_existing_exclusion_folds() {
        let doc = Document::from_text("before\n雪\tkeep\nafter\n");
        let focus = Focus::new(doc.len(), [8..11, 7..9, 10..13]);
        assert_eq!(focus.ranges, std::slice::from_ref(&(7..13)));
        let folds = focus.exclusions(&doc);
        assert_eq!(folds.chars(), [0..7, 13..doc.len()]);
        assert_eq!(doc.rope.to_string(), "before\n雪\tkeep\nafter\n");
        assert_eq!((focus.nearest(0), focus.nearest(8), focus.nearest(doc.len())), (7, 8, 13));
        let split = Focus::new(doc.len(), [2..4, 8..10]);
        assert_eq!((split.nearest(6), split.nearest(7)), (4, 8));
        assert!(Focus::default().exclusions(&Document::from_text("")).is_empty());
    }

    #[test]
    fn n14_focus_tracks_edits_at_both_edges_interior_and_full_replacement() {
        let mut s = EditorState::from_text("preFOCUSpost");
        let mut focus = Focus::new(s.len(), std::iter::once(3..8));
        for (range, text, expected) in
            [(3..3, "雪", 3..9), (9..9, "!", 3..10), (5..8, "abcde", 3..12), (3..12, "x", 3..4)]
        {
            s.set_selection(range.start, range.end);
            s.insert(text);
            focus.map(s.len(), &s.take_changes());
            assert_eq!(focus.ranges, [expected]);
        }
        assert_eq!(s.doc.rope.to_string(), "prexpost");
        s.set_selection(0, 3);
        s.insert("");
        focus.map(s.len(), &s.take_changes());
        assert_eq!(focus.ranges, std::slice::from_ref(&(0..1)));
    }

    #[test]
    fn n14_matching_lines_keeps_full_regex_context_and_clips_merges_context() {
        let doc = Document::from_text("pre\r\n雪 hit\r\nnext hit\r\ntail\r\n");
        let focus = Focus::matching_lines(&doc.rope, &pattern(r"(?<=雪 )hit\r\nnext"), 0, &|| false).unwrap();
        assert_eq!(focus.ranges, std::slice::from_ref(&(doc.line_start(1)..doc.line_start(3))));
        let context = Focus::matching_lines(&doc.rope, &pattern("hit"), 1, &|| false).unwrap();
        assert_eq!(context.ranges, std::slice::from_ref(&(0..doc.line_start(4))));
        let all = Focus::matching_lines(&doc.rope, &pattern("hit"), usize::MAX, &|| false).unwrap();
        assert_eq!(all.ranges, std::slice::from_ref(&(0..doc.len())));
        let boundary = Focus::matching_lines(&doc.rope, &pattern("pre\r\n"), 0, &|| false).unwrap();
        assert_eq!(
            boundary.ranges,
            std::slice::from_ref(&(0..doc.line_start(1))),
            "half-open match excludes the next line"
        );
    }

    #[test]
    fn n14_no_match_zero_width_empty_and_cancel_keep_source_unchanged() {
        let doc = Document::from_text("a\nb\n");
        let none = Focus::matching_lines(&doc.rope, &pattern("missing"), 0, &|| false).unwrap();
        assert!(none.is_empty());
        assert_eq!(none.nearest(100), 0);
        assert_eq!(none.exclusions(&doc).chars(), std::slice::from_ref(&(0..doc.len())));
        let eof = Focus::matching_lines(&doc.rope, &pattern(r"\z"), 0, &|| false).unwrap();
        assert_eq!(eof.ranges, std::slice::from_ref(&(doc.len()..doc.len())));
        assert!(eof.contains(doc.len()));
        let empty = Focus::matching_lines(&Rope::new(), &pattern(""), 0, &|| false).unwrap();
        assert_eq!(empty.ranges, std::slice::from_ref(&(0..0)));
        assert_eq!(Focus::matching_lines(&doc.rope, &pattern("a"), 0, &|| true), Err(SearchError::Cancelled));
        assert_eq!(doc.rope.to_string(), "a\nb\n");
    }
}

#[cfg(test)]
mod traversal_tests {
    use super::*;

    #[test]
    fn n15_source_order_strict_parent_inline_duplicates_and_boundaries() {
        let d = Document::from_text("root\n  child\n    雪😀\n  tail\nnext\n  end");
        let first = d.line_end(0)..d.line_end(3);
        let child = d.line_end(1)..d.line_end(2);
        let sibling = d.line_end(4)..d.len();
        let inline = d.line_start(2) + 4..d.line_end(2);
        let t = Traversal::new(&d, [first.clone(), child.clone(), sibling, inline.clone(), child, 9..9, 0..999]);
        assert_eq!(t.destination(0, Motion::Next), Some(d.line_start(1)), "next fold must skip current header");
        assert_eq!(t.destination(d.len(), Motion::Next), None);
        assert_eq!(t.destination(0, Motion::Previous), None);
        assert_eq!(t.destination(d.len(), Motion::Previous), Some(d.line_start(4)));
        assert_eq!(t.current(inline.start), Some(inline.clone()));
        assert_eq!(t.destination(inline.start, Motion::Parent), Some(d.line_start(1)));
        assert_eq!(t.destination(d.line_start(1), Motion::Parent), Some(0));
        assert_eq!(t.destination(0, Motion::Parent), None);
        assert_eq!(t.current(first.end), None, "non-EOF ends are outside");
        assert_eq!(t.current(d.len()), Some(d.line_start(4)..d.len()));
    }
}

#[cfg(test)]
mod traversal_boundary_tests {
    use super::*;

    #[test]
    fn n15_same_header_parent_touching_and_empty_source() {
        let d = Document::from_text("abcdef");
        let t = Traversal::new(&d, [0..6, 0..4, 0..4, 1..3, 4..6]);
        assert_eq!(t.current(0), Some(0..4));
        assert_eq!(t.destination(0, Motion::Parent), Some(0));
        assert_eq!(t.destination(1, Motion::Parent), Some(0));
        assert_eq!(t.destination(0, Motion::Next), Some(1));
        assert_eq!(t.destination(4, Motion::Previous), Some(1));
        assert_eq!(t.current(4), Some(4..6));
        let newline = Document::from_text("abc\nrest");
        let t = Traversal::new(&newline, std::iter::once(3..4));
        assert_eq!(t.current(0), Some(0..4), "a hidden newline includes its visible header");
        let shared_header = Document::from_text("abc\nrest");
        let t = Traversal::new(&shared_header, [1..8, 2..8]);
        assert_eq!(
            t.destination(0, Motion::Parent),
            Some(0),
            "distinct raw ranges sharing header/end retain their strict parent"
        );
        let empty = Traversal::new(&Document::from_text(""), std::iter::once(0..1));
        assert_eq!(empty.current(0), None);
        assert_eq!(empty.destination(0, Motion::Next), None);
        assert_eq!(empty.destination(0, Motion::Parent), None);
    }
}
