use crate::buffer::*;

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    // ---- snippets ----

    /// `trigger` ends the line's text before `caret` (it may be punctuation, as `$$` or `\begin`);
    /// one that starts with a word char must not follow another (or it starts the line).
    pub fn trigger_before(&self, caret: usize, trigger: &str) -> bool {
        let from = self.view.selections.partition_point(|s| s.end() < caret);
        if self.view.selections[from..]
            .iter()
            .take_while(|s| s.start() <= caret)
            .any(|s| s.head == caret && s.head_virtual > 0)
        {
            return false;
        }
        let n = trigger.chars().count();
        let line = self.line_start(self.point(caret).0);
        if n == 0 || caret < line + n {
            return false;
        }
        let at = caret - n;
        let glued =
            trigger.starts_with(Self::is_word_char) && at > line && Self::is_word_char(self.doc.rope.char(at - 1));
        !glued && self.doc.rope.slice(at..caret) == trigger
    }

    /// The variables the state knows at `sel` (snippet text going in at `at`), from the text as
    /// it is before the expansion.
    pub(crate) fn variable(&self, name: &str, sel: &Selection, at: usize) -> Option<String> {
        let (line, col) = self.point(at);
        let path = self.doc.path.as_deref();
        Some(match name {
            "SELECTION" | "TM_SELECTED_TEXT" => self.doc.rope.slice(sel.range()).to_string(),
            "TM_CURRENT_LINE" => self.line(line),
            "TM_CURRENT_WORD" if sel.has_virtual() => String::new(),
            "TM_CURRENT_WORD" => (self.doc.rope.slice(self.word_range_at(sel.head))).to_string(),
            "TM_FILENAME" => path?.file_name()?.to_string_lossy().into_owned(),
            "TM_FILEPATH" => path?.to_string_lossy().into_owned(),
            "TM_DIRECTORY" => path?.parent()?.to_string_lossy().into_owned(),
            // Sublime's: the column, from 0, and the line, from 1
            "TM_LINE_INDEX" => (col + sel.start_point().1).to_string(),
            "TM_LINE_NUMBER" => (line + 1).to_string(),
            "TM_SOFT_TABS" => if self.doc.translate_tabs { "YES" } else { "NO" }.into(),
            "TM_TAB_SIZE" => self.doc.tab_size.to_string(),
            _ => return None,
        })
    }

    pub fn in_snippet(&self) -> bool {
        self.view.in_snippet()
    }

    pub fn snippet_choices(&self) -> Option<&std::sync::Arc<[String]>> {
        self.view.snippet.as_ref()?.choices()
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    /// Escape: the session ends; the selections stay.
    pub fn end_snippet(&mut self) {
        self.view.snippet = None;
    }

    /// Dismiss only the choice list; ordinary field navigation remains available.
    pub fn close_snippet_choices(&mut self) {
        if let Some(s) = &mut self.view.snippet {
            s.close_choices();
        }
    }

    /// Place only a snippet's own fields; ordinary source range selection stays ordinary.
    fn select_snippet(&mut self, session: &Session) {
        let selections = session.selections();
        if selections.is_empty() {
            return;
        }
        self.set_typing(false);
        self.view.primary = selections.len() - 1;
        self.view.selections = selections;
        self.normalize();
    }

    /// Before a frame: a click is no command, so the session ends here when it left no selection
    /// in the field (the edits are the next command's to bring in).
    pub fn check_snippet(&mut self) {
        let Some(s) = &self.view.snippet else {
            return;
        };
        if s.at == self.view.changes.len()
            && !self.view.selections.iter().any(|x| !x.has_virtual() && s.holds(x.range()))
        {
            self.view.snippet = None;
        }
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// Literal timestamp at every selection, as one independent Undo step.
    pub fn insert_date_time(
        &mut self,
        format: &str,
        utc: bool,
        time: &orlo_utils::date_time::Snapshot,
    ) -> Result<bool, String> {
        if self.doc.read_only || self.doc.edits_blocked() {
            return Err("Document is read only".into());
        }
        let text = if utc { time.utc } else { time.local }.format(format)?;
        let version = self.doc.version;
        self.set_typing(false);
        self.insert(&text);
        self.set_typing(false);
        Ok(self.doc.version != version)
    }

    /// Replace the active choice field at every insertion caret, reusing mirror sync and Undo.
    pub fn choose_snippet(&mut self, index: usize) -> bool {
        if self.doc.read_only || self.doc.edits_blocked() {
            return false;
        }
        self.sync_snippet();
        let Some(s) = &self.view.snippet else { return false };
        if self.view.selections.iter().any(|x| x.has_virtual() || !s.holds(x.range())) {
            return false;
        }
        let Some(edits) = s.choice_edits(index) else { return false };
        self.replace_ranges(edits);
        self.sync_snippet();
        if let Some(mut s) = self.view.snippet.take() {
            s.close_choices();
            self.select_snippet(&s);
            self.view.snippet = Some(s);
        }
        true
    }

    /// Primary-caret server snippet plus additional edits, validated before one undoable edit.
    pub fn complete_snippet_ranges(
        &mut self,
        primary: Range<usize>,
        source: &str,
        additional: Vec<(Range<usize>, String)>,
    ) -> Result<bool, String> {
        self.complete_snippet_ranges_with(primary, source, additional, &|_| None)
    }

    /// Server snippet expansion with one caller-captured clock/clipboard/project snapshot.
    pub fn complete_snippet_ranges_with(
        &mut self,
        primary: Range<usize>,
        source: &str,
        additional: Vec<(Range<usize>, String)>,
        vars: &dyn Fn(&str) -> Option<String>,
    ) -> Result<bool, String> {
        if self.doc.read_only || self.doc.edits_blocked() {
            return Err("Document is read only".into());
        }
        let body = snippet::parse_lsp(source)?;
        let mut ranges: Vec<_> = additional.iter().map(|(r, _)| r.clone()).chain([primary.clone()]).collect();
        if ranges.iter().any(|r| r.start > r.end || r.end > self.len()) {
            return Err("Invalid completion range".into());
        }
        ranges.sort_by_key(|r| (r.start, r.end));
        if ranges.windows(2).any(|r| r[0].end > r[1].start || r[0].start == r[1].start) {
            return Err("Overlapping completion edits".into());
        }
        let at = primary.start;
        let rectangular = self.view.selections[self.primary()].rectangular;
        let selection = Selection::new(primary.start, primary.end);
        let var = |name: &str| self.variable(name, &selection, at).or_else(|| vars(name));
        let indent: String =
            self.doc.rope.line(self.point(at).0).chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
        let tab = if self.doc.translate_tabs { self.indent_unit() } else { "\t".into() };
        let expansion = snippet::expand(&body, &var, &indent, &tab);
        let moved = additional
            .iter()
            .filter(|(r, _)| r.end <= primary.start)
            .map(|(r, text)| text.chars().count() as isize - r.len() as isize)
            .sum::<isize>();
        let start = primary.start.saturating_add_signed(moved);
        let mut edits = additional;
        edits.push((primary, expansion.text.clone()));
        let changed = self.replace_ranges(edits);
        let session = Session::with_origins(&[(start, expansion)], &[rectangular], self.view.changes.len());
        self.select_snippet(&session);
        self.view.snippet = (!session.at_end()).then_some(session);
        Ok(changed)
    }

    /// Completion's commit: `text` in place of `prefix` before every caret where `prefix`
    /// is all of the word before it, the caret after it; one undo step.
    pub fn complete(&mut self, prefix: &str, text: &str) {
        let (n, m) = (prefix.chars().count(), text.chars().count());
        self.set_typing(false);
        self.record();
        self.edit_with(|s, _, sel| {
            let at = sel.head;
            let whole = sel.is_empty()
                && !sel.has_virtual()
                && at >= n
                && s.doc.rope.slice(at - n..at) == prefix
                && (at == n || !Self::is_word_char(s.doc.rope.char(at - n - 1)));
            match whole {
                true => (at - n..at, text.to_string(), Selection::caret(at - n + m)),
                false => (at..at, String::new(), *sel),
            }
        });
    }

    /// `s` expanded at every caret with `trigger` before it, in place of it (one undo step);
    /// an empty `trigger` expands at every selection, in place of it. A caret's variables are its
    /// own: the state's (`SELECTION`, its line, its word, the file, the indentation), else
    /// `vars(name, caret)`. The first field is then selected and its session starts, replacing
    /// any other; false when no caret had the trigger.
    pub fn expand_snippet(&mut self, s: &Snippet, trigger: &str, vars: &dyn Fn(&str, usize) -> Option<String>) -> bool {
        if self.doc.edits_blocked() {
            return false;
        }
        self.normalize();
        let virtual_insert = trigger.is_empty() && self.view.selections.iter().any(Selection::has_virtual);
        let n = trigger.chars().count();
        let tab = match self.doc.translate_tabs {
            true => self.indent_unit(),
            false => "\t".to_string(),
        };
        let mut edits: Vec<(Range<usize>, snippet::Expansion)> = Vec::new();
        let mut origins = Vec::new();
        for (i, sel) in self.view.selections.iter().enumerate() {
            let range = match n {
                0 => sel.range(),
                _ if sel.is_empty() && self.trigger_before(sel.head, trigger) => sel.head - n..sel.head,
                _ => continue,
            };
            // two carets' triggers would overlap
            if edits.last().is_some_and(|(r, _)| r.end > range.start) {
                return false;
            }
            let line = self.point(range.start).0;
            let indent: String = (self.doc.rope.line(line).chars()).take_while(|c| *c == ' ' || *c == '\t').collect();
            let at = range.start;
            let var = |name: &str| self.variable(name, sel, at).or_else(|| vars(name, i));
            edits.push((range, snippet::expand(&s.body, &var, &indent, &tab)));
            origins.push(sel.rectangular);
        }
        if edits.is_empty() {
            return false;
        }
        if virtual_insert && edits.iter().all(|(r, e)| r.is_empty() && e.text.is_empty()) {
            return false;
        }
        self.set_typing(false);
        self.record();
        if virtual_insert {
            self.materialize_virtual_where(|i| !edits[i].1.text.is_empty());
            for ((range, _), sel) in edits.iter_mut().zip(&self.view.selections) {
                *range = sel.range();
            }
        }
        let logged = self.view.changes.len();
        for (r, e) in edits.iter().rev() {
            self.splice(r.clone(), &e.text);
        }
        if self.view.changes.len() > logged {
            self.touch(self.doc.rope.char_to_line(edits[0].0.start));
        }
        // where each caret's text starts now
        let mut moved = 0isize;
        let expansions: Vec<(usize, snippet::Expansion)> = (edits.into_iter())
            .map(|(r, e)| {
                let start = r.start.saturating_add_signed(moved);
                moved += e.text.chars().count() as isize - r.len() as isize;
                (start, e)
            })
            .collect();
        let session = Session::with_origins(&expansions, &origins, self.view.changes.len());
        self.select_snippet(&session);
        self.view.snippet = (!session.at_end()).then_some(session);
        true
    }

    /// Tab (`forward`) and Shift-Tab in a session: the next or previous field still there is
    /// selected; at `$0` or the end of the expansion the session ends. False with no session.
    pub fn snippet_next(&mut self, forward: bool) -> bool {
        if self.view.selections.iter().any(Selection::has_virtual) {
            self.view.snippet = None;
            return false;
        }
        self.sync_snippet();
        let Some(mut s) = self.view.snippet.take() else {
            return false;
        };
        s.step(forward);
        self.select_snippet(&s);
        if !s.at_end() {
            self.view.snippet = Some(s);
        }
        true
    }

    /// After every command: the session's places follow the edits made since, and its mirrors
    /// their primaries, in the same undo step. An edit outside the field being typed in ends the
    /// session, as do an edit in a plain mirror whose primary had none (the text stays as
    /// typed), no selection left in that field and that field gone.
    pub fn sync_snippet(&mut self) {
        if self.doc.edits_blocked() {
            return;
        }
        let Some(mut s) = self.view.snippet.take() else {
            return;
        };
        let Some(new) = self.view.changes.get(s.at..) else {
            return;
        };
        if !new.is_empty() {
            s.close_choices();
        }
        let Some(took) = new.iter().map(|c| s.map(c)).collect::<Option<Vec<_>>>() else {
            return;
        };
        if s.typed_alone(&took) {
            return;
        }
        let logged = self.view.changes.len();
        // each selection end in the field, with the place that holds it (once a mirror changes)
        let mut ends: Vec<[Option<Spot>; 2]> = Vec::new();
        for i in s.mirrors() {
            if let Some(want) = s.stale(i, &self.doc.rope) {
                if ends.is_empty() {
                    ends = (self.view.selections.iter()).map(|x| [s.holder(x.anchor), s.holder(x.head)]).collect();
                }
                let r = s.range(i);
                let line = self.doc.rope.char_to_line(r.start);
                self.splice(r, &want);
                self.touch(line);
                s.replaced(i, want.chars().count());
            }
        }
        if !ends.is_empty() {
            // an end in the field goes with its place, never to the far side of a mirror beside
            // it; the session moved the places
            let synced = self.view.changes[logged..].to_vec();
            map_sels(&mut self.view.selections, &synced, false);
            for (sel, [anchor, head]) in self.view.selections.iter_mut().zip(ends) {
                if let Some(at) = anchor {
                    sel.anchor = s.held(at);
                }
                if let Some(at) = head {
                    sel.head = s.held(at);
                }
            }
            self.normalize();
        }
        s.at = self.view.changes.len();
        if s.alive() && self.view.selections.iter().any(|x| s.holds(x.range())) {
            self.view.snippet = Some(s);
        }
    }
}

impl ViewModel {
    pub fn in_snippet(&self) -> bool {
        self.snippet.is_some()
    }
}
