use crate::buffer::*;

/// One shared EOL pad, with every original endpoint kept on a character boundary.
#[derive(Default)]
struct VirtualPad {
    columns: usize,
    text: String,
    offsets: std::collections::BTreeMap<usize, usize>,
}

/// Chars of a buffer read to pick its CSV delimiter: a huge file is never copied for it.
pub(crate) const DETECT_CHARS: usize = 1 << 20;

/// What a column command does, worked out without touching the buffer.
pub enum ColumnPlan {
    Select(Vec<Range<usize>>),
    Copy(String),
    Edit(csv::Edits),
    Nothing,
}

/// A column command's input: a copy of the text (a rope clone is cheap) and the primary
/// selection that picks the column and the records, as of one version of the buffer.
pub struct ColumnJob {
    op: ColumnOp,
    delim: Delim,
    rope: Rope,
    buffer: u64,
    version: u64,
    primary: Selection,
}

impl ColumnJob {
    /// The buffer is still the text and the primary selection the job was made from.
    pub fn is_current<D: DocRef, V: ViewRef>(&self, s: &Editing<D, V>) -> bool {
        s.buffer_id() == self.buffer
            && s.doc.version == self.version
            && s.view.selections.get(s.primary()) == Some(&self.primary)
    }

    /// Parse the whole document and plan the command. Used on the UI thread for a small
    /// document and on another for a big one.
    pub fn run(&self) -> Result<ColumnPlan, String> {
        let recs = csv::records(&self.rope.to_string(), self.delim)?;
        let p = self.primary;
        let across_lines = self.rope.char_to_line(p.start()) != self.rope.char_to_line(p.end());
        let scope = (!p.is_empty() && across_lines).then(|| p.range());
        let Some(t) = csv::target(&recs, p.head, scope) else {
            return Ok(ColumnPlan::Nothing);
        };
        let slice = |r: Range<usize>| self.rope.slice(r).to_string();
        Ok(match self.op {
            ColumnOp::Select => ColumnPlan::Select(csv::select(&recs, &t)),
            ColumnOp::Copy => ColumnPlan::Copy(csv::copy(&recs, &t, &slice)),
            ColumnOp::MoveLeft => ColumnPlan::Edit(csv::move_left(&recs, &t, self.delim, &slice)),
            ColumnOp::MoveRight => ColumnPlan::Edit(csv::move_right(&recs, &t, self.delim, &slice)),
            ColumnOp::Delete => ColumnPlan::Edit(csv::delete(&recs, &t, &slice)),
            ColumnOp::Clear => ColumnPlan::Edit(csv::clear(&recs, &t)),
        })
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    // ---- Edit > Columns ----

    /// The display column of char offset `at` (tabs to their stops, wide characters 2).
    // copies the line's prefix per caret, so carets on very long lines cost O(line); walk the rope's chars if that lags.
    pub(crate) fn display_col_at(&self, at: usize, tab_size: usize) -> usize {
        let start = self.line_start(self.point(at).0);
        let prefix = self.doc.rope.slice(start..at).to_string();
        columns::display_col(&prefix, at - start, tab_size)
    }

    /// The spaces each selection needs so its insertion lines up: a caret at a line end reaches
    /// the rightmost insertion point that is not at one.
    pub(crate) fn short_line_pads(&self, tab_size: usize) -> Vec<usize> {
        let places: Vec<(usize, bool)> = (self.view.selections.iter())
            .map(|s| {
                let at = s.start();
                let col = self.display_col_at(at, tab_size);
                (col, at == self.line_end(self.point(at).0))
            })
            .collect();
        let target = places.iter().filter(|p| !p.1).map(|p| p.0).max();
        (self.view.selections.iter().zip(&places))
            .map(|(s, &(col, at_end))| match target {
                Some(t) if s.is_empty() && at_end => t.saturating_sub(col),
                _ => 0,
            })
            .collect()
    }

    /// What a column command needs of this buffer, to be run here or on another thread.
    pub fn column_job(&self, op: ColumnOp) -> ColumnJob {
        ColumnJob {
            op,
            delim: self.delimiter(),
            rope: self.doc.rope.clone(),
            buffer: self.buffer_id(),
            version: self.doc.version,
            primary: self.view.selections[self.primary()],
        }
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// A mapped view may require spaces when its tab stops differ from the source line's.
    /// Applies only to virtual padding; callers reset this after their synchronous edit.
    pub fn set_virtual_padding_tabs(&mut self, allowed: bool) {
        self.view.virtual_padding_tabs = allowed;
    }

    /// Materialize the replacement starts, sharing one pad per line. The caller records its
    /// undo snapshot first; returned ranges let an IME remove its provisional padding.
    pub(crate) fn materialize_virtual(&mut self) -> Vec<Range<usize>> {
        self.materialize_virtual_where(|_| true)
    }

    pub(crate) fn materialize_virtual_where(&mut self, needs_text: impl Fn(usize) -> bool) -> Vec<Range<usize>> {
        if self.doc.edits_blocked() {
            return Vec::new();
        }
        if !self.view.selections.iter().any(Selection::has_virtual) {
            return Vec::new();
        }
        self.normalize();
        let mut pads = std::collections::BTreeMap::<usize, VirtualPad>::new();
        for (i, sel) in self.view.selections.iter().enumerate() {
            let (at, gap) = sel.start_point();
            if gap > 0 && needs_text(i) {
                let pad = pads.entry(at).or_default();
                pad.columns = pad.columns.max(gap);
            }
        }
        for sel in &self.view.selections {
            for (at, gap) in [sel.anchor_point(), sel.head_point()] {
                if let Some(pad) = pads.get_mut(&at) {
                    pad.offsets.insert(gap.min(pad.columns), 0);
                }
            }
        }
        for (&at, pad) in &mut pads {
            let col = col_after(self.doc.rope.slice(self.line_start(self.point(at).0)..at), self.doc.tab_size);
            let mut previous = 0;
            let mut chars = 0;
            for (&gap, offset) in &mut pad.offsets {
                // A tab may not jump over another caret, including an empty paste target.
                let mut column = col + previous;
                let target = col + gap;
                while column < target {
                    let next_tab = column + self.doc.tab_size - column % self.doc.tab_size;
                    if !self.doc.translate_tabs && self.view.virtual_padding_tabs && next_tab <= target {
                        pad.text.push('\t');
                        column = next_tab;
                    } else {
                        pad.text.push(' ');
                        column += 1;
                    }
                    chars += 1;
                }
                *offset = chars;
                previous = gap;
            }
        }
        let before = self.view.changes.len();
        for (&at, pad) in pads.iter().rev() {
            self.splice(at..at, &pad.text);
            self.touch(self.doc.rope.char_to_line(at));
        }
        let changes = self.view.changes[before..].to_vec();
        let original = self.view.selections.clone();
        for sel in &mut self.view.selections {
            *sel = Selection { rectangular: sel.rectangular, ..Selection::new(sel.anchor, sel.head) };
        }
        map_sels(&mut self.view.selections, &changes, false);
        for (i, (sel, original)) in self.view.selections.iter_mut().zip(original).enumerate() {
            let mapped = |at: usize, gap: usize| {
                pads.get(&at).map_or((0, 0), |pad| (pad.columns, pad.offsets[&gap.min(pad.columns)]))
            };
            let (anchor_pad, anchor_chars) = mapped(original.anchor, original.anchor_virtual);
            let (head_pad, head_chars) = mapped(original.head, original.head_virtual);
            sel.anchor += anchor_chars;
            sel.head += head_chars;
            if !needs_text(i) {
                sel.anchor_virtual = original.anchor_virtual.saturating_sub(anchor_pad);
                sel.head_virtual = original.head_virtual.saturating_sub(head_pad);
            }
        }
        let mut starts: Vec<_> = pads.keys().copied().collect();
        map_sorted(&mut starts, &changes);
        starts.into_iter().zip(pads.values()).map(|(start, pad)| start..start + pad.text.chars().count()).collect()
    }

    /// The Column Editor: `texts[i]` replaces selection `i` (document order) and leaves a caret
    /// after it. With `pad_short`, a caret at the end of a short line gets spaces first.
    pub fn insert_per_selection(&mut self, texts: &[String], pad_short: bool, tab_size: usize) {
        self.normalize();
        self.set_typing(false);
        self.record();
        if texts.iter().any(|text| !text.is_empty()) {
            self.materialize_virtual_where(|i| pad_short || texts.get(i).is_some_and(|text| !text.is_empty()));
        }
        let pads = match pad_short {
            true => self.short_line_pads(tab_size),
            false => vec![0; self.view.selections.len()],
        };
        let text_at = |i: usize| texts.get(i).map_or("", String::as_str);
        let nothing = (self.view.selections.iter().enumerate())
            .all(|(i, s)| s.is_empty() && pads[i] == 0 && text_at(i).is_empty());
        if nothing {
            return;
        }
        self.edit_with(|_, i, sel| {
            if pads[i] == 0 && text_at(i).is_empty() && sel.range().is_empty() {
                return (sel.range(), String::new(), Selection::from_points(sel.start_point(), sel.start_point()));
            }
            let text = format!("{}{}", " ".repeat(pads[i]), text_at(i));
            let caret = sel.start() + text.chars().count();
            (sel.range(), text, Selection::caret(caret))
        });
    }

    /// Spaces at each caret up to display column `col` (0-based), or the rightmost caret's
    /// column. Carets already past it, and selections, stay as they are.
    pub fn pad_to_column(&mut self, col: Option<usize>, tab_size: usize) {
        self.normalize();
        let cols: Vec<Option<usize>> = (self.view.selections.iter())
            .map(|s| s.is_empty().then(|| self.display_col_at(s.head, tab_size) + s.head_virtual))
            .collect();
        let Some(target) = col.or_else(|| cols.iter().flatten().max().copied()) else {
            return;
        };
        let pads: Vec<usize> = (cols.iter()).map(|c| c.map_or(0, |c| target.saturating_sub(c))).collect();
        if pads.iter().all(|&p| p == 0) {
            return;
        }
        self.set_typing(false);
        self.record();
        self.materialize_virtual_where(|i| pads[i] > 0);
        self.edit_with(|_, i, sel| match pads[i] {
            0 => (sel.head..sel.head, String::new(), *sel),
            n => (sel.head..sel.head, " ".repeat(n), Selection::caret(sel.head + n)),
        });
    }

    /// Replace each (sorted, disjoint) range with its text: one undo step, selections kept on
    /// their text. A replacement that changes nothing is left out; false when none is left.
    pub fn replace_ranges(&mut self, mut edits: Vec<(Range<usize>, String)>) -> bool {
        if self.doc.edits_blocked() {
            return false;
        }
        edits.retain(|(r, t)| self.doc.rope.slice(r.clone()) != t.as_str());
        edits.sort_by_key(|e| e.0.start);
        let Some(first) = edits.first().map(|e| e.0.start) else {
            return false;
        };
        self.set_typing(false);
        self.record();
        let n = self.view.changes.len();
        let aside = self.set_marks_aside();
        for (r, t) in edits.iter().rev() {
            self.splice(r.clone(), t);
        }
        self.restore_marks(aside);
        let changes = self.view.changes[n..].to_vec();
        self.map_selections(&changes, false);
        self.touch(self.doc.rope.char_to_line(first.min(self.len())));
        self.normalize();
        true
    }

    /// A language server item's commit: `replace_ranges`, then the primary caret after the
    /// text of the edit that held it (the item's), moved by the edits before that one.
    pub fn complete_ranges(&mut self, mut edits: Vec<(Range<usize>, String)>) -> bool {
        edits.sort_by_key(|e| e.0.start);
        let at = self.cursor();
        let grew = |(r, t): &(Range<usize>, String)| t.chars().count() as isize - r.len() as isize;
        let caret = (edits.iter()).position(|(r, _)| r.start <= at && at <= r.end).map(|k| {
            let end = edits[k].0.start as isize + edits[k].1.chars().count() as isize;
            (end + edits[..k].iter().map(grew).sum::<isize>()) as usize
        });
        let changed = self.replace_ranges(edits);
        if let Some(caret) = caret.filter(|_| changed) {
            let primary = self.primary();
            let rectangular = self.view.selections[primary].rectangular;
            self.view.selections[primary] = Selection { rectangular, ..Selection::caret(caret) };
            self.normalize();
        }
        changed
    }

    /// Carry out a plan made for this buffer as it is; Copy gives the text to copy.
    pub fn apply_column_plan(&mut self, plan: ColumnPlan) -> Option<String> {
        match plan {
            ColumnPlan::Select(ranges) => self.select_ranges(&ranges),
            ColumnPlan::Edit(edits) => {
                self.replace_ranges(edits);
            }
            ColumnPlan::Copy(text) => return Some(text),
            ColumnPlan::Nothing => {}
        }
        None
    }
}

impl Document {
    /// The delimiter the CSV commands use: Edit > Columns > Delimiter's choice, else what the
    /// start of the text and the file name say (kept for this version of the text).
    pub fn delimiter(&self) -> Delim {
        let mut meta = self.csv.borrow_mut();
        if let Some(d) = meta.forced {
            return d;
        }
        let ext = (self.path.as_deref()).and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase());
        if let Some((version, e, d)) = &meta.detected {
            if *version == self.version && *e == ext {
                return *d;
            }
        }
        let mut head = self.rope.slice(..self.len().min(DETECT_CHARS)).to_string();
        // a record cut by the limit would read as a short one
        if self.len() > DETECT_CHARS {
            head.truncate(head.rfind('\n').map_or(head.len(), |i| i + 1));
        }
        let d = csv::detect(&head, ext.as_deref());
        meta.detected = Some((self.version, ext, d));
        d
    }

    pub fn delimiter_override(&self) -> Option<Delim> {
        self.csv.borrow().forced
    }

    pub fn set_delimiter(&self, d: Option<Delim>) {
        self.csv.borrow_mut().forced = d;
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// Immutable preflight: match ordinal capture starts across the selected source lines.
    pub fn align_pattern_edits(&self, pattern: &str, capture: usize, ignore_case: bool) -> Result<csv::Edits, String> {
        let pattern =
            Pattern::new(pattern, &FindOpts { regex: true, case_sensitive: !ignore_case, ..Default::default() })?;
        if capture >= pattern.names().len() {
            return Err(format!("Capture index must be between 0 and {}", pattern.names().len() - 1));
        }
        let deadline = Deadline::new(crate::search::pattern::UI_SECONDS);
        let edits = self.align_pattern_plan(&pattern, capture, &deadline)?;
        if !edits.is_empty() {
            let mut candidate = EditorState::new();
            candidate.doc.rope = self.doc.rope.clone();
            candidate.doc.endings = self.doc.endings.clone();
            candidate.doc.line_ending = self.doc.line_ending;
            candidate.doc.tab_size = self.doc.tab_size;
            *candidate.view = self.view.duplicate(&candidate.doc);
            // Publication mirrors active snippet fields; preflight the same changes on detached state.
            candidate.view.snippet = self.view.snippet.clone();
            candidate.view.changes = self.view.changes.clone();
            candidate.replace_ranges(edits.clone());
            candidate.sync_snippet();
            if !candidate.align_pattern_plan(&pattern, capture, &deadline)?.is_empty() {
                return Err("The alignment changes its field columns after padding; adjust the pattern or finish the active snippet".into());
            }
        }
        Ok(edits)
    }

    fn align_pattern_plan(&self, pattern: &Pattern, capture: usize, deadline: &Deadline) -> Result<csv::Edits, String> {
        let ranges = self.alignment_ranges()?;
        if ranges.is_empty() && self.view.selections.iter().all(Selection::has_virtual) {
            return Ok(Vec::new());
        }
        let mut lines = std::collections::BTreeSet::new();
        if ranges.is_empty() {
            lines.extend(0..self.doc.rope.len_lines());
        } else {
            for r in ranges {
                lines.extend(self.point(r.start).0..=self.point(r.end - 1).0);
            }
        }
        let mut rows = Vec::new();
        let mut starts = Vec::new();
        for line in lines {
            let start = self.line_start(line);
            let text = self.doc.rope.slice(start..self.line_end(line)).to_string();
            let mut fields: Vec<Option<usize>> = Vec::new();
            let char_at: HashMap<_, _> = text
                .char_indices()
                .map(|(b, _)| b)
                .chain(std::iter::once(text.len()))
                .enumerate()
                .map(|(c, b)| (b, c))
                .collect();
            let boundaries: HashSet<_> =
                text.grapheme_indices(true).map(|(b, _)| b).chain(std::iter::once(text.len())).collect();
            let mut padding_runs = Vec::new();
            let mut run = None;
            for (byte, c) in text.char_indices().chain(std::iter::once((text.len(), 'x'))) {
                if matches!(c, ' ' | '\t') {
                    run.get_or_insert(byte);
                } else if let Some(start) = run.take() {
                    padding_runs.push(start..byte);
                }
            }
            let padding_run = |r: &Range<usize>| {
                let i = padding_runs.partition_point(|run| run.end < r.start);
                padding_runs.get(i).filter(|run| run.start <= r.start && r.end <= run.end)
            };
            let mut anchors: HashMap<usize, usize> = HashMap::new();
            let mut previous_end = 0;
            for matched in pattern.find_all(&text, &|| deadline.passed()).map_err(|e| deadline.explain(e).message())? {
                if deadline.passed() {
                    return Err(deadline.explain(SearchError::Cancelled).message());
                }
                let captured =
                    pattern.captures(&text, matched.clone()).ok_or("The pattern captures could not be read")?;
                let (anchor, padding) = match captured[capture].as_ref().filter(|r| !r.is_empty()) {
                    None => (None, padding_run(&matched)),
                    Some(r) => {
                        if r.start < previous_end {
                            return Err("Pattern captures overlap".into());
                        }
                        previous_end = r.end;
                        let leading = text[r.clone()].bytes().take_while(|c| matches!(c, b' ' | b'\t')).count();
                        let padding = (leading == r.len()).then(|| padding_run(r)).flatten();
                        let byte = padding.map_or(r.start + leading, |run| run.end);
                        for at in [r.start, r.end, byte] {
                            if !boundaries.contains(&at) {
                                return Err("Alignment cannot split a grapheme".into());
                            }
                        }
                        (Some(char_at[&byte]), padding)
                    }
                };
                // Padding and the following capture can name the same physical boundary.
                let boundary = anchor.or_else(|| padding.map(|run| char_at[&run.end]));
                if let Some(at) = boundary {
                    if let Some(&slot) = anchors.get(&at) {
                        if fields[slot].is_none() {
                            fields[slot] = anchor;
                        }
                        continue;
                    }
                    anchors.insert(at, fields.len());
                }
                fields.push(anchor);
            }
            starts.push(start);
            rows.push((text, fields));
        }
        let pads = columns::alignment_padding(&rows, self.doc.tab_size);
        if deadline.passed() {
            return Err(deadline.explain(SearchError::Cancelled).message());
        }
        Ok(rows
            .iter()
            .zip(starts)
            .zip(pads)
            .flat_map(|(((_, fields), start), pads)| {
                fields.iter().zip(pads).filter_map(move |(at, pad)| {
                    at.filter(|_| pad > 0).map(|at| (start + at..start + at, " ".repeat(pad)))
                })
            })
            .collect())
    }

    /// Each same-line selection is a field; grow its right edge inside the mapped range.
    pub fn align_selected_field_edits(&self) -> Result<csv::Edits, String> {
        let ranges = self.alignment_ranges()?;
        let mut lines = std::collections::BTreeMap::<usize, Vec<Range<usize>>>::new();
        for r in ranges {
            let line = self.point(r.start).0;
            if r.end > self.line_end(line) {
                return Err("Align Selected Fields requires same-line ranges".into());
            }
            lines.entry(line).or_default().push(r);
        }
        let rows: Vec<_> = lines
            .iter()
            .map(|(&line, ranges)| {
                let start = self.line_start(line);
                (
                    self.doc.rope.slice(start..self.line_end(line)).to_string(),
                    ranges.iter().map(|r| Some(r.end - start)).collect(),
                )
            })
            .collect();
        let pads = columns::alignment_padding(&rows, self.doc.tab_size);
        Ok(lines
            .values()
            .zip(pads)
            .flat_map(|(ranges, pads)| {
                ranges
                    .iter()
                    .zip(pads)
                    .filter(|(_, pad)| *pad > 0)
                    .map(|(r, pad)| (r.clone(), format!("{}{}", self.doc.rope.slice(r.clone()), " ".repeat(pad))))
            })
            .collect())
    }

    fn alignment_ranges(&self) -> Result<Vec<Range<usize>>, String> {
        if self.doc.edits_blocked() {
            return Err("Stop following before aligning fields".into());
        }
        let mut ranges: Vec<_> = self.view.selections.iter().map(Selection::range).filter(|r| !r.is_empty()).collect();
        ranges.sort_by_key(|r| (r.start, r.end));
        for (i, r) in ranges.iter().enumerate() {
            if r.end > self.len() {
                return Err("An alignment range is outside the document".into());
            }
            if i > 0 && ranges[i - 1].end > r.start {
                return Err("Alignment ranges overlap".into());
            }
            self.alignment_boundary(r.start)?;
            self.alignment_boundary(r.end)?;
        }
        Ok(ranges)
    }

    fn alignment_boundary(&self, at: usize) -> Result<(), String> {
        if at > 0 && at < self.len() && self.doc.rope.char(at - 1) == '\r' && self.doc.rope.char(at) == '\n' {
            return Err("Alignment cannot split CRLF".into());
        }
        let start = self.line_start(self.point(at).0);
        let text = self.doc.rope.line(self.point(at).0).to_string();
        let byte = text.char_indices().nth(at - start).map_or(text.len(), |(b, _)| b);
        if byte != text.len() && !text.grapheme_indices(true).any(|(b, _)| b == byte) {
            return Err("Alignment cannot split a grapheme".into());
        }
        Ok(())
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// Keep appended field padding inside its selection using the existing endpoint mapper.
    pub fn apply_selected_field_alignment(&mut self, edits: csv::Edits) -> bool {
        let mut original = self.view.selections.clone();
        let primary = self.view.primary;
        let before = self.view.changes.len();
        if !self.replace_ranges(edits) {
            return false;
        }
        let changes = &self.view.changes[before..];
        for selection in &mut original {
            let field = !selection.range().is_empty();
            map_sels(std::slice::from_mut(selection), changes, field);
        }
        self.view.selections = original;
        self.view.primary = primary;
        self.all_off_cr_lf();
        self.normalize();
        true
    }
}
