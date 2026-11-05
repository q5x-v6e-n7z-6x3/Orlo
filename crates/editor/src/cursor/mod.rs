use crate::buffer::*;

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    pub fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    pub(crate) fn char_class(c: char) -> u8 {
        if c.is_whitespace() {
            0
        } else if Self::is_word_char(c) {
            1
        } else {
            2
        }
    }

    // Classification uses the cluster's leading scalar; its combining/ZWJ tail stays whole.
    fn subword_class(&self, at: usize) -> u8 {
        let c = self.doc.rope.char(at);
        if c.is_whitespace() {
            0
        } else if c == '_' {
            1
        } else if c.is_numeric() {
            2
        } else if c.is_lowercase() {
            3
        } else if c.is_uppercase() {
            4
        } else if c.is_alphabetic() {
            5
        } else {
            6
        }
    }

    /// The same source-order boundary predicate serves both directions, including HTTP|Server.
    fn is_subword_boundary(&self, at: usize) -> bool {
        if at == 0 || at == self.len() {
            return true;
        }
        let previous = self.subword_class(self.prev_cluster(at));
        let current = self.subword_class(at);
        let group = |class| if (3..=5).contains(&class) { 3 } else { class };
        if group(previous) != group(current) || (previous == 3 && current == 4) {
            return true;
        }
        let next = self.next_cluster(at);
        previous == 4 && current == 4 && next < self.len() && self.subword_class(next) == 3
    }

    pub(crate) fn subword_right(&self, pos: usize) -> usize {
        let mut at = self.snap_to_cluster(pos);
        while at < self.len() {
            at = self.next_cluster(at);
            if self.is_subword_boundary(at) {
                break;
            }
        }
        at
    }

    pub(crate) fn subword_left(&self, pos: usize) -> usize {
        let mut at = self.snap_to_cluster(pos);
        if at < pos.min(self.len()) && self.is_subword_boundary(at) {
            return at;
        }
        while at > 0 {
            at = self.prev_cluster(at);
            if self.is_subword_boundary(at) {
                break;
            }
        }
        at
    }

    /// Source paragraph starts, with document endpoints; blank runs have no inner stops.
    pub(crate) fn paragraph_boundaries(&self) -> Vec<usize> {
        let mut boundaries = vec![0];
        let (mut blank_before, mut start) = (true, 0);
        for line in self.doc.rope.lines() {
            let blank = line.chars().all(char::is_whitespace);
            if !blank && blank_before && start > 0 {
                boundaries.push(start);
            }
            blank_before = blank;
            start += line.len_chars();
        }
        if boundaries.last() != Some(&self.len()) {
            boundaries.push(self.len());
        }
        boundaries
    }

    /// Punctuation heuristic, shared by motion, selection and deletion; separators belong left.
    pub(crate) fn sentence_boundaries(&self) -> Vec<usize> {
        let mut boundaries = vec![0];
        let (mut terminal, mut separator, mut skip_until) = (false, false, 0);
        // ASCII quotes plus Unicode closing punctuation (Pe/Pf), including stacked suffixes.
        let closer = |c| {
            "\"')]}»༻༽᚜’”›⁆⁾₎⌉⌋〉❩❫❭❯❱❳❵⟆⟧⟩⟫⟭⟯⦄⦆⦈⦊⦌⦎⦐⦒⦔⦖⦘⧙⧛⧽⸃⸅⸊⸍⸝⸡⸣⸥⸧⸩⹖⹘⹚⹜〉》」』】〕〗〙〛〞〟﴾︘︶︸︺︼︾﹀﹂﹄﹈﹚﹜﹞）］｝｠｣".contains(c)
        };
        for (at, c) in self.doc.rope.chars().enumerate() {
            if at < skip_until {
                continue;
            }
            if separator {
                if c.is_whitespace() {
                    continue;
                }
                let start = self.snap_to_cluster(at);
                if start < at {
                    skip_until = self.next_cluster(start);
                    continue;
                }
                boundaries.push(at);
                separator = false;
            } else if terminal {
                if c.is_whitespace() {
                    separator = true;
                    continue;
                }
                if closer(c) {
                    continue;
                }
                let start = self.snap_to_cluster(at);
                if start < at {
                    // Skip an oversized combining tail once, without rescanning each prefix.
                    skip_until = self.next_cluster(start);
                    continue;
                }
            }
            terminal = matches!(c, '.' | '?' | '!');
        }
        if boundaries.last() != Some(&self.len()) {
            boundaries.push(self.len());
        }
        boundaries
    }

    pub(crate) fn sentence_range(&self, boundaries: &[usize], sel: &Selection) -> Range<usize> {
        if boundaries.len() < 2 {
            return 0..0;
        }
        let interval = |at| boundaries.partition_point(|&b| b <= at).saturating_sub(1).min(boundaries.len() - 2);
        let start = interval(sel.start());
        let end = interval(if sel.range().is_empty() { sel.head } else { sel.end() - 1 });
        boundaries[start]..boundaries[end + 1]
    }

    pub(crate) fn word_right(&self, mut pos: usize) -> usize {
        let len = self.len();
        while pos < len && self.doc.rope.char(pos).is_whitespace() {
            pos += 1;
        }
        if pos >= len {
            return len;
        }
        // a combining mark (an accent, a tone mark, a virama) goes with its letter
        let class = Self::char_class(self.doc.rope.char(pos));
        let goes_on = |c| Self::char_class(c) == class || crate::selection::joins_previous(c);
        while pos < len && goes_on(self.doc.rope.char(pos)) {
            pos += 1;
        }
        pos
    }

    pub(crate) fn word_left(&self, mut pos: usize) -> usize {
        while pos > 0 && self.doc.rope.char(pos - 1).is_whitespace() {
            pos -= 1;
        }
        if pos == 0 {
            return 0;
        }
        // the class of the letter under any combining marks, which go with it
        let joins = crate::selection::joins_previous;
        let base = self.doc.rope.chars_at(pos).reversed().find(|&c| !joins(c));
        let class = base.map_or(2, Self::char_class);
        while pos > 0 && (Self::char_class(self.doc.rope.char(pos - 1)) == class || joins(self.doc.rope.char(pos - 1)))
        {
            pos -= 1;
        }
        pos
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn move_cursors(&mut self, motion: Motion, select: bool) {
        self.move_cursors_in(motion, select, &Lines, &Cells);
    }

    /// Cursor motion over display rows: a view's `Folded` rows (over `Lines`, or a `WrapMap`
    /// when wrapped). Vertical motion keeps to the x `measure` draws a caret at.
    pub fn move_cursors_in(&mut self, motion: Motion, select: bool, rows: &dyn Rows, measure: &dyn Measure) {
        let prose = match motion {
            Motion::ParagraphPrevious | Motion::ParagraphNext => Some(self.paragraph_boundaries()),
            Motion::SentencePrevious | Motion::SentenceNext => Some(self.sentence_boundaries()),
            _ => None,
        };
        self.set_typing(false);
        let vertical = matches!(motion, Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_));
        for i in 0..self.view.selections.len() {
            let sel = self.view.selections[i];
            let head = sel.head;
            let mut goal = if vertical { sel.goal } else { None };
            let mut virtual_head = 0;
            let new_head = match motion {
                Motion::Left if !select && !sel.is_empty() => {
                    virtual_head = sel.start_point().1;
                    sel.start()
                }
                Motion::Left if sel.head_virtual > 0 => {
                    virtual_head = sel.head_virtual - 1;
                    head
                }
                Motion::Left => self.prev_cluster(head),
                Motion::Right if !select && !sel.is_empty() => {
                    virtual_head = sel.end_point().1;
                    sel.end()
                }
                Motion::Right
                    if self.view.virtual_space.allows(sel.rectangular)
                        && head == self.line_end(self.point(head).0)
                        && rows
                            .row_bounds(&self.doc, rows.row_of(&self.doc, head).row)
                            .is_some_and(|r| r.end == head) =>
                {
                    virtual_head = sel.head_virtual.saturating_add(1);
                    head
                }
                Motion::Right => self.next_cluster(head),
                Motion::Up | Motion::PageUp(_) | Motion::Down | Motion::PageDown(_) => {
                    let mut at = rows.row_of(&self.doc, head);
                    at.col.0 = at.col.0.saturating_add(sel.head_virtual);
                    let mut row = at.row.0;
                    // where the goal is on text row `r`: the column drawn nearest its x
                    let on = |goal: Goal, r: DisplayRow| {
                        let col = match goal {
                            Goal::X(x) => measure.col_at(&self.doc, r, x),
                            Goal::LineEnd => DisplayCol(usize::MAX),
                        };
                        if goal == Goal::LineEnd {
                            Selection::caret(rows.offset_at(&self.doc, DisplayPoint { row: r, col }))
                        } else {
                            self.selection_at_origin(DisplayPoint { row: r, col }, rows, sel.rectangular)
                        }
                    };
                    // a caret a vertical move put at a row start is drawn at the end of the row
                    // above; it moves on from the row it was put on (a filler row has no start)
                    if let Some(g) = goal {
                        let below = DisplayRow(row + 1);
                        if row + 1 < rows.rows(&self.doc)
                            && rows.row_bounds(&self.doc, below).is_some()
                            && on(g, below).head_point() == sel.head_point()
                        {
                            row += 1;
                        }
                    }
                    let g = goal.unwrap_or_else(|| Goal::X(measure.x_of(&self.doc, at)));
                    goal = Some(g);
                    let n = match motion {
                        Motion::PageUp(n) | Motion::PageDown(n) => n,
                        _ => 1,
                    };
                    // filler rows are stepped over, on in the direction of the motion
                    let up = matches!(motion, Motion::Up | Motion::PageUp(_));
                    let target = match up {
                        true if row == 0 => None,
                        true => rows.text_row(&self.doc, DisplayRow(row.saturating_sub(n)), false),
                        false if row + n >= rows.rows(&self.doc) => None,
                        false => rows.text_row(&self.doc, DisplayRow(row + n), true),
                    };
                    match (target, up) {
                        (Some(t), _) => {
                            let target = on(g, t);
                            virtual_head = target.head_virtual;
                            target.head
                        }
                        (None, true) => 0,
                        (None, false) => self.len(),
                    }
                }
                Motion::WordLeft | Motion::SubwordLeft | Motion::ParagraphPrevious | Motion::SentencePrevious
                    if sel.head_virtual > 0 =>
                {
                    head
                }
                Motion::WordLeft => self.word_left(head),
                Motion::WordRight => self.word_right(head),
                Motion::SubwordLeft => self.subword_left(head),
                Motion::SubwordRight => self.subword_right(head),
                Motion::ParagraphPrevious | Motion::SentencePrevious | Motion::ParagraphNext | Motion::SentenceNext => {
                    let boundaries = prose.as_ref().unwrap();
                    if matches!(motion, Motion::ParagraphNext | Motion::SentenceNext) {
                        let i = boundaries.partition_point(|&b| b <= head);
                        boundaries.get(i).copied().unwrap_or(self.len())
                    } else {
                        let i = boundaries.partition_point(|&b| b < head);
                        boundaries[i.saturating_sub(1)]
                    }
                }
                Motion::LineStart => {
                    let row = rows.row_of(&self.doc, head).row;
                    let start = rows.row_bounds(&self.doc, row).map_or(head, |b| b.start);
                    // a row that starts a display line goes by the indentation of its first
                    // line, which a line joined to it by a fold does not have
                    let (line, col) = self.point(start);
                    if col > 0 {
                        start
                    } else {
                        let first = self.line(line).chars().take_while(|c| c.is_whitespace()).count();
                        let indent = rows.skip(&self.doc, start + first, false);
                        if head == indent {
                            start
                        } else {
                            indent
                        }
                    }
                }
                Motion::LineEnd => {
                    // Sublime's move_to eol: later vertical moves and added carets keep to the
                    // ends of lines (ctrl-e, its hardeol, does not)
                    goal = Some(Goal::LineEnd);
                    let row = rows.row_of(&self.doc, head).row;
                    let end = rows.row_bounds(&self.doc, row).map_or(head, |b| b.end);
                    if head == end {
                        self.line_end(self.point(head).0)
                    } else {
                        end
                    }
                }
                Motion::HardLineStart => {
                    // a line joined to the display line above starts inside that line's row:
                    // on to the start of the line that row starts in
                    let mut start = self.line_start(self.point(head).0);
                    loop {
                        let row = rows.row_of(&self.doc, start).row;
                        match rows.row_bounds(&self.doc, row) {
                            Some(b) if b.start < start => {
                                start = self.line_start(self.point(b.start).0);
                            }
                            _ => break start,
                        }
                    }
                }
                Motion::HardLineEnd => self.line_end(self.point(head).0),
                Motion::DocStart => 0,
                Motion::DocEnd => self.len(),
            };
            let new_head = match motion {
                Motion::Left
                | Motion::WordLeft
                | Motion::SubwordLeft
                | Motion::ParagraphPrevious
                | Motion::SentencePrevious => rows.skip(&self.doc, new_head, false),
                Motion::Right
                | Motion::WordRight
                | Motion::SubwordRight
                | Motion::ParagraphNext
                | Motion::SentenceNext => rows.skip(&self.doc, new_head, true),
                _ => new_head,
            };
            self.view.selections[i] = Selection {
                anchor: if select { sel.anchor } else { new_head },
                head: new_head,
                anchor_virtual: if select { sel.anchor_virtual } else { virtual_head },
                head_virtual: virtual_head,
                goal,
                rectangular: sel.rectangular,
            };
        }
        self.normalize();
    }
}
