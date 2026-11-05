use crate::buffer::*;

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    // ---- bookmarks from matches, and the bookmarked lines ----

    /// The bookmarked lines, in order.
    pub fn bookmarked_lines(&self) -> Vec<usize> {
        let mut l: Vec<usize> =
            (self.view.bookmarks.iter()).map(|&b| self.doc.rope.char_to_line(b.min(self.len()))).collect();
        l.dedup();
        l
    }

    /// The lines a match touches: from its first to its last, a match ending just after a line
    /// break not touching the next line; an empty match its own line.
    pub fn match_lines(&self, m: &Range<usize>) -> std::ops::RangeInclusive<usize> {
        let (a, b) = (self.point(m.start), self.point(m.end));
        let last = match b.1 == 0 && b.0 > a.0 {
            true => b.0 - 1,
            false => b.0,
        };
        a.0..=last
    }

    /// The bookmarked lines' text, each with a line break: what Copy Bookmarked Lines puts on
    /// the clipboard, as whole lines.
    pub fn bookmarked_text(&self) -> Option<String> {
        let lines = self.bookmarked_lines();
        (!lines.is_empty()).then(|| lines.iter().map(|&l| self.line(l) + "\n").collect())
    }

    /// The text of `style`'s ranges in document order, one a line.
    pub fn highlighted_text(&self, style: usize) -> Option<String> {
        let h = self.view.highlights.get(style).filter(|h| !h.is_empty())?;
        Some(h.iter().map(|r| self.doc.rope.slice(r.clone()).to_string() + "\n").collect())
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    // ---- Goto > Bookmarks ----

    /// Bookmark (or un-bookmark) every caret's line.
    pub fn toggle_bookmark(&mut self) {
        let lines: Vec<usize> = (self.view.selections.iter()).map(|s| self.line_start(self.point(s.head).0)).collect();
        for ls in lines {
            match self.view.bookmarks.binary_search(&ls) {
                Ok(i) => {
                    self.view.bookmarks.remove(i);
                }
                Err(i) => self.view.bookmarks.insert(i, ls),
            }
        }
    }

    pub fn clear_bookmarks(&mut self) {
        self.view.bookmarks.clear();
    }

    pub fn next_bookmark(&mut self, forward: bool) {
        let cur = self.cursor();
        let cur_line = self.line_start(self.point(cur).0);
        let marks = &self.view.bookmarks;
        let target = match forward {
            true => marks.iter().find(|&&b| b > cur_line).or(marks.first()),
            false => marks.iter().rev().find(|&&b| b < cur_line).or(marks.last()),
        };
        if let Some(&b) = target {
            self.set_cursor(b);
        }
    }

    pub(crate) fn set_bookmarked_lines(&mut self, lines: impl IntoIterator<Item = usize>) {
        let mut b: Vec<usize> = lines.into_iter().map(|l| self.line_start(l)).collect();
        b.sort_unstable();
        b.dedup();
        self.view.bookmarks = b;
    }

    /// Bookmark every line the matches touch, added to the bookmarks there are; how many lines
    /// were not bookmarked before.
    pub fn bookmark_matches(&mut self, matches: &[Range<usize>]) -> usize {
        let before = self.bookmarked_lines();
        let mut lines = before.clone();
        lines.extend(matches.iter().flat_map(|m| self.match_lines(m)));
        self.set_bookmarked_lines(lines);
        self.bookmarked_lines().len() - before.len()
    }

    /// Every line that is not bookmarked is, and every one that is is not.
    pub fn invert_bookmarks(&mut self) {
        let marked = self.bookmarked_lines();
        let all = (0..self.line_count()).filter(|l| marked.binary_search(l).is_err());
        self.set_bookmarked_lines(all);
    }

    // ---- highlight styles ----

    /// Mark `ranges` in `style`, merged with what it marks; how many ranges it marks now.
    pub fn add_highlights(&mut self, style: usize, ranges: impl IntoIterator<Item = Range<usize>>) -> usize {
        let Some(h) = self.view.highlights.get_mut(style) else {
            return 0;
        };
        h.extend(ranges.into_iter().filter(|r| !r.is_empty()));
        h.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(h.len());
        for r in h.drain(..) {
            match merged.last_mut() {
                Some(m) if r.start <= m.end => m.end = m.end.max(r.end),
                _ => merged.push(r),
            }
        }
        *h = merged;
        h.len()
    }

    /// One style's highlights gone, or every style's.
    pub fn clear_highlights(&mut self, style: Option<usize>) {
        match style {
            Some(s) => (self.view.highlights.get_mut(s).into_iter()).for_each(Vec::clear),
            None => self.view.highlights.iter_mut().for_each(Vec::clear),
        }
    }

    /// Select the next (or previous) range of `style` after the caret, round the ends.
    pub fn next_highlight(&mut self, style: usize, forward: bool) -> bool {
        let Some(h) = self.view.highlights.get(style).filter(|h| !h.is_empty()) else {
            return false;
        };
        let sel = self.view.selections[self.primary()];
        let r = match forward {
            true => h.iter().find(|r| r.start >= sel.end() && **r != sel.range()).or(h.first()),
            false => h.iter().rev().find(|r| r.start < sel.start()).or(h.last()),
        };
        let Some(r) = r.cloned() else { return false };
        self.set_selection(r.start, r.end);
        true
    }

    pub fn select_all_bookmarks(&mut self) {
        let sels: Vec<Selection> = (self.view.bookmarks.iter()).map(|&b| Selection::caret(b)).collect();
        if !sels.is_empty() {
            self.set_typing(false);
            self.view.selections = sels;
            self.view.primary = self.view.selections.len() - 1;
            self.normalize();
        }
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// Delete the bookmarked lines (`keep`: every other line), each with its line break; the
    /// last line takes the break before it. One undo step; one caret where the first went.
    pub fn delete_bookmarked_lines(&mut self, keep: bool) -> bool {
        let marked = self.bookmarked_lines();
        if marked.is_empty() {
            return false;
        }
        let n = self.line_count();
        // the empty end after a final line break is no line: deleting it would take that break
        let end = n > 1 && self.line_start(n - 1) == self.len();
        let gone: Vec<usize> = match keep {
            false => marked,
            true => (0..n).filter(|&l| !(end && l == n - 1) && marked.binary_search(&l).is_err()).collect(),
        };
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for l in gone {
            let r = match l + 1 < n {
                true => self.line_start(l)..self.line_start(l + 1),
                false if l > 0 => self.off_cr_lf(self.line_start(l) - 1)..self.len(),
                false => self.line_start(l)..self.len(),
            };
            match ranges.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => ranges.push(r),
            }
        }
        let first = ranges.first().map_or(0, |r| r.start);
        let edits = ranges.into_iter().map(|r| (r, String::new())).collect();
        if !self.replace_ranges(edits) {
            return false;
        }
        // the deleted lines' bookmarks go with them (mapping would move them onto neighbours);
        // kept lines are all bookmarked; undo brings the bookmarks back with the lines
        match keep {
            false => self.view.bookmarks.clear(),
            true => self.set_bookmarked_lines(0..self.line_count()),
        }
        self.set_cursor(first.min(self.len()));
        true
    }

    /// Each bookmarked line's text becomes `text` (less one final line break), or with as many
    /// lines as there are bookmarked lines, line i the i-th of them. One undo step.
    pub fn paste_to_bookmarked_lines(&mut self, text: &str) -> bool {
        let lines = self.bookmarked_lines();
        if lines.is_empty() {
            return false;
        }
        let text = text.strip_suffix('\n').unwrap_or(text);
        let parts: Vec<&str> = text.split('\n').collect();
        let each = parts.len() == lines.len() && lines.len() > 1;
        let edits = (lines.iter().enumerate())
            .map(|(i, &l)| {
                let t = if each { parts[i] } else { text };
                (self.line_start(l)..self.line_end(l), t.to_string())
            })
            .collect();
        self.replace_ranges(edits)
    }
}
