use crate::buffer::*;

impl Document {
    pub fn ending_counts(&self) -> [usize; 3] {
        orlo_project::documents::endings::physical_counts(&self.rope, self.endings.as_ref(), self.line_ending)
    }

    pub fn next_ending(&self, ending: &str, cursor: usize) -> Option<usize> {
        let (mut first, mut next) = (None, None);
        orlo_project::documents::endings::visit_boundaries(
            &self.rope,
            self.endings.as_ref(),
            self.line_ending,
            |kind, at| {
                if kind == ending {
                    first.get_or_insert(at);
                    if at > cursor {
                        next.get_or_insert(at);
                    }
                }
            },
        );
        next.or(first)
    }

    /// Unsaved changes no undo can take back to the file's text (a buffer back from hot exit).
    pub fn forget_saved_point(&mut self) {
        self.dirty = !self.scratch;
        self.saved = None;
    }

    /// View > Line Endings. As in Sublime it is not an undo step, and no undo makes the document
    /// clean again, so the conversion is never lost by undoing other edits.
    pub fn set_encoding(&mut self, encoding: orlo_project::encoding::Encoding, bom: bool) {
        if self.edits_blocked() {
            return;
        }
        if (self.encoding, self.bom) != (encoding, bom) {
            self.encoding = encoding;
            self.bom = bom;
            self.forget_saved_point();
        }
    }

    pub fn set_line_ending(&mut self, ending: &'static str) {
        if self.edits_blocked() {
            return;
        }
        if ending == self.line_ending {
            return;
        }
        self.line_ending = ending;
        for snapshot in self.history.undo.iter_mut().chain(&mut self.history.redo) {
            if !snapshot.line_ending_override {
                snapshot.line_ending = ending;
            }
        }
        if !self.scratch {
            self.dirty = true;
            self.saved = None;
        }
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            rope: self.doc.rope.clone(),
            endings: self.doc.endings.clone(),
            line_ending: self.doc.line_ending,
            line_ending_override: self.doc.line_ending_override,
            selections: self.view.selections.clone(),
            primary: self.view.primary,
            outlines: self.doc.outlines.clone(),
            view_id: self.view.view_id,
            bookmarks: self.view.bookmarks.clone(),
            highlights: self.view.highlights.clone(),
            state: self.doc.state,
        }
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    // ---- undo ----

    pub fn undo(&mut self) {
        self.step_history(true);
    }

    pub fn redo(&mut self) {
        self.step_history(false);
    }

    /// Undo, or redo: the text as it stands goes on the other stack.
    fn step_history(&mut self, undo: bool) {
        if self.doc.edits_blocked() {
            return;
        }
        self.view.snippet = None;
        self.set_typing(false);
        self.map_outlines();
        let h = &mut self.doc.history;
        if let Some(snap) = (if undo { &mut h.undo } else { &mut h.redo }).pop() {
            let current = self.snapshot();
            let h = &mut self.doc.history;
            (if undo { &mut h.redo } else { &mut h.undo }).push(current);
            self.restore(snap);
        }
    }

    /// The view that recorded `snap` gets its selections back; another view of the buffer keeps
    /// its own, mapped through the change, as Sublime does.
    pub(crate) fn restore(&mut self, snap: Snapshot) {
        let changes = diff(&self.doc.rope, &snap.rope);
        let start = changes.first().map_or(self.len(), |c| c.start);
        let line = self.doc.rope.char_to_line(start.min(self.len()));
        self.map_outlines();
        self.doc.rope = snap.rope;
        self.doc.endings = snap.endings;
        self.doc.line_ending = snap.line_ending;
        self.doc.line_ending_override = snap.line_ending_override;
        self.view.metadata_changed |= changes.is_empty();
        self.view.changes.extend(&changes);
        // outlines belong to the buffer: they come back as they were, unmapped
        (self.doc.outlines, self.view.outlines_at) = (snap.outlines, self.view.changes.len());
        if snap.view_id == self.view.view_id {
            (self.view.selections, self.view.primary) = (snap.selections, snap.primary);
            self.map_marks(&changes);
            (self.view.bookmarks, self.view.highlights) = (snap.bookmarks, snap.highlights);
        } else {
            self.map_own(&changes);
        }
        self.view.pending = None;
        self.touch(line);
        // the text of that step: clean again if it is what the file holds
        self.doc.state = snap.state;
        self.doc.refresh_dirty();
        self.normalize();
    }

    /// Explicit conversion is one undoable shared metadata edit, even with identical source.
    pub fn convert_line_endings(&mut self, ending: &'static str) {
        if self.doc.edits_blocked() || !["\n", "\r\n", "\r"].contains(&ending) {
            return;
        }
        let converted = LineEndings::uniform(&self.doc.rope, ending);
        if self.doc.line_ending == ending
            && self.doc.endings.as_ref().is_none_or(|map| *map == converted)
            && !self.doc.rope.chars().any(|c| c == '\r')
        {
            if !self.doc.line_ending_override {
                // Preference provenance is undoable without changing what a save writes.
                // This shared transaction must also end another view's typing group.
                self.doc.typing.set(None);
                self.record();
                if let Some(snapshot) = self.view.pending.take() {
                    let history = &mut self.doc.history;
                    history.undo.push(snapshot);
                    history.redo.clear();
                }
                self.doc.line_ending_override = true;
                self.doc.version += 1;
                self.view.metadata_changed = true;
            }
            return;
        }
        self.set_typing(false);
        self.record();
        let version = self.doc.version;
        if self.doc.rope.chars().any(|c| c == '\r') {
            let (source, _, _) = decoded_file(&self.doc.rope.to_string());
            self.replace_ranges(vec![(0..self.len(), source.to_string())]);
        }
        self.doc.endings = Some(LineEndings::uniform(&self.doc.rope, ending));
        self.doc.line_ending = ending;
        self.doc.line_ending_override = true;
        if self.doc.version == version {
            self.view.metadata_changed = true;
            self.touch(0);
        }
    }

    /// Snapshot the state before an edit unless a typing group is open; `touch` pushes it.
    pub(crate) fn record(&mut self) {
        if self.doc.edits_blocked() {
            return;
        }
        if !self.typing() {
            self.map_outlines();
            self.view.pending = Some(self.snapshot());
        }
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// Monotonic selection generation: replacements, checkpoints, undo/redo and text edits.
    pub fn selection_generation(&self) -> u64 {
        self.view.sel_hist.generation
    }

    /// Exact view-local selections, primary and whole-word metadata for transient reversal.
    pub fn selection_state(&self) -> SelState {
        self.sel_now()
    }

    pub(crate) fn sel_now(&self) -> SelState {
        SelState {
            selections: self.view.selections.clone(),
            primary: self.primary(),
            whole_words: self.view.selections == self.view.quick_add_words,
        }
    }

    pub(crate) fn sel_unchanged(&self) -> bool {
        let seen = &self.view.sel_hist.seen;
        seen.primary == self.primary() && seen.selections == self.view.selections
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    /// Note the selections as they stand: a change since the last note pushes the state it
    /// replaced, and a text change here clears the history, which never changes text. The
    /// editor calls it before each command and after each frame; repeats cost a comparison.
    pub fn sel_checkpoint(&mut self) {
        let edited = std::mem::take(&mut self.view.sel_hist.edited);
        if !edited && self.sel_unchanged() {
            return;
        }
        let now = self.sel_now();
        let h = &mut self.view.sel_hist;
        h.generation = h.generation.wrapping_add(1);
        let old = std::mem::replace(&mut h.seen, now);
        h.redo.clear();
        match edited {
            true => h.undo.clear(),
            false => push_state(&mut h.undo, old),
        }
    }

    /// Restore a same-source snapshot exactly, recording it in ordinary selection history.
    pub fn restore_selection_state(&mut self, state: SelState) {
        self.set_typing(false);
        self.sel_checkpoint();
        (self.view.selections, self.view.primary) = (state.selections, state.primary);
        self.view.quick_add_words = if state.whole_words { self.view.selections.clone() } else { Vec::new() };
        self.sel_checkpoint();
    }

    /// Start the history here: nothing earlier can be undone (a tab opened with its saved
    /// selections).
    pub fn sel_reset(&mut self) {
        let generation = self.view.sel_hist.generation.wrapping_add(1);
        self.view.sel_hist = SelHistory::new(self.view.selections.clone(), self.primary());
        self.view.sel_hist.generation = generation;
    }

    /// Another view's edit: the states kept here follow it; a view that had not moved since
    /// its last checkpoint stays that way.
    pub(crate) fn sel_follow(&mut self, changes: &[Change], synced: bool) {
        let h = &mut self.view.sel_hist;
        for st in h.undo.iter_mut().chain(&mut h.redo) {
            map_sels(&mut st.selections, changes, false);
        }
        match synced {
            true => self.view.sel_hist.seen = self.sel_now(),
            false => map_sels(&mut self.view.sel_hist.seen.selections, changes, false),
        }
    }

    /// Selection > Undo Selection: the previous selections, exactly. False when there are none.
    pub fn undo_selection(&mut self) -> bool {
        self.step_selection(true)
    }

    /// Selection > Redo Selection: what Undo Selection took back.
    pub fn redo_selection(&mut self) -> bool {
        self.step_selection(false)
    }

    pub(crate) fn step_selection(&mut self, undo: bool) -> bool {
        self.sel_checkpoint();
        let h = &mut self.view.sel_hist;
        let Some(state) = (if undo { &mut h.undo } else { &mut h.redo }).pop() else {
            return false;
        };
        let now = self.sel_now();
        let h = &mut self.view.sel_hist;
        push_state(if undo { &mut h.redo } else { &mut h.undo }, now);
        self.set_typing(false);
        (self.view.selections, self.view.primary) = (state.selections, state.primary);
        self.all_off_cr_lf();
        self.normalize();
        self.view.quick_add_words = match state.whole_words {
            true => self.view.selections.clone(),
            false => Vec::new(),
        };
        self.view.sel_hist.seen = self.sel_now();
        self.view.sel_hist.generation = self.view.sel_hist.generation.wrapping_add(1);
        true
    }
}
