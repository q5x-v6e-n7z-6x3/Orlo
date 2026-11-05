use super::*;

pub(crate) fn indentation(columns: usize, tab_size: usize, spaces: bool) -> String {
    if spaces {
        " ".repeat(columns)
    } else {
        format!("{}{}", "\t".repeat(columns / tab_size), " ".repeat(columns % tab_size))
    }
}

impl Document {
    pub fn new() -> Self {
        Self::from_source("")
    }

    /// Fresh runtime source; legacy from_text deliberately retains single-ending semantics.
    pub fn from_source(text: &str) -> Self {
        let mut doc = Self::from_text(text);
        doc.endings = Some(LineEndings::uniform(&doc.rope, doc.line_ending));
        doc.mark_saved();
        doc
    }

    pub fn from_text(text: &str) -> Self {
        let mut doc = Self {
            rope: Rope::from_str(text),
            endings: None,
            version: 0,
            path: None,
            dirty: false,
            read_only: false,
            following: false,
            viewer_mode: false,
            line_ending: "\n",
            bom: false,
            encoding: Default::default(),
            disk_encoding: Default::default(),
            history_enabled: false,
            disk: None,
            deleted: false,
            watching: true,
            tab_size: TAB_WIDTH,
            indent_size: None,
            translate_tabs: true,
            indent_override: [false; 2],
            line_ending_override: false,
            inferred_indentation: None,
            scratch: false,
            outlines: Vec::new(),
            history: History::new(),
            state: 0,
            saved: None,
            csv: RefCell::default(),
            id: NEXT_BUFFER.fetch_add(1, Ordering::Relaxed),
            typing: std::cell::Cell::new(None),
            log: RefCell::default(),
            views: RefCell::default(),
        };
        // new text is its own save point: a new file, or one about to be read into it
        doc.state = next_state();
        doc.saved = Some(doc.save_point());
        doc
    }

    /// Stable for as long as the document is open, and never reused: a file closed and opened
    /// again is another document.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_following(&self) -> bool {
        self.following
    }

    /// Mode changes invalidate pending source captures even when no bytes changed.
    pub fn set_following(&mut self, following: bool) -> Option<Rc<TextChange>> {
        if self.following == following {
            return None;
        }
        self.following = following;
        self.mode_changed()
    }

    pub fn viewer_mode(&self) -> bool {
        self.viewer_mode
    }

    /// User mutation admission; disk overwrite protection intentionally remains separate.
    pub fn edits_blocked(&self) -> bool {
        self.following || self.viewer_mode
    }

    /// Preserve text/saved identity and history while invalidating every pending text capture.
    pub fn set_viewer_mode(&mut self, enabled: bool) -> Option<Rc<TextChange>> {
        if self.viewer_mode == enabled {
            return None;
        }
        self.viewer_mode = enabled;
        self.mode_changed()
    }

    fn mode_changed(&mut self) -> Option<Rc<TextChange>> {
        self.typing.set(None);
        let before = self.version;
        self.version += 1;
        let change =
            Rc::new(TextChange { document: self.id, before, after: self.version, changes: Vec::new(), origin: 0 });
        self.publish(change.clone());
        Some(change)
    }

    /// How many views of this document are open.
    pub fn view_count(&self) -> usize {
        self.live_revisions().len()
    }

    /// A new view of this document, its caret at the start.
    pub fn attach_view(&self) -> ViewModel {
        let view = ViewModel::new(self.version);
        self.views.borrow_mut().push(Rc::downgrade(&view.revision));
        view
    }

    /// A view of this document taking `view`'s selections, bookmarks, mark and highlights,
    /// made for another copy of its text (a file opened again while open elsewhere): they are
    /// clamped to this text.
    pub fn adopt_view(&self, view: ViewModel) -> ViewModel {
        let len = self.len();
        let mut v = self.attach_view();
        v.selections = (view.selections.iter()).map(|s| Selection::new(s.anchor.min(len), s.head.min(len))).collect();
        v.primary = view.primary.min(v.selections.len().saturating_sub(1));
        v.bookmarks = (view.bookmarks.iter()).map(|&b| self.line_start(self.point(b).0)).collect();
        v.bookmarks.dedup();
        v.mark = view.mark.map(|m| m.min(len));
        v.highlights = view.highlights.map(|ranges| {
            ranges
                .into_iter()
                .filter_map(|r| {
                    let end = r.end.min(len);
                    (r.start < end).then_some(r.start..end)
                })
                .collect()
        });
        v.sel_hist = SelHistory::new(v.selections.clone(), v.primary);
        v
    }

    /// Closing it would lose text: unsaved edits, or a file deleted on disk (Sublime asks
    /// about both).
    pub fn unsaved(&self) -> bool {
        self.dirty || self.deleted
    }

    /// The text as last read or saved, while it is still to hand: the document itself when
    /// clean, else the undo or redo step that holds it.
    pub fn saved_text(&self) -> Option<Rope> {
        if !self.dirty {
            return Some(self.rope.clone());
        }
        let state = self.saved?.state;
        let h = &self.history;
        (h.undo.iter().chain(&h.redo)).find(|s| s.state == state).map(|s| s.rope.clone())
    }

    /// The current text, for converting positions taken from this version.
    pub fn coords(&self) -> Coords<'_> {
        Coords::new(&self.rope, self.version)
    }

    /// Keep `change` for the views that have not applied it, and drop what every view has.
    pub(crate) fn publish(&self, change: Rc<TextChange>) {
        if self.live_revisions().into_iter().any(|r| r < change.after) {
            self.log.borrow_mut().push_back(change);
        }
        self.trim_log();
    }

    /// The versions the views attached to this document are at; a closed view's is gone.
    fn live_revisions(&self) -> Vec<u64> {
        let mut views = self.views.borrow_mut();
        views.retain(|v| v.strong_count() > 0);
        views.iter().filter_map(|v| Some(v.upgrade()?.get())).collect()
    }

    /// Changes every view has applied are let go.
    pub(crate) fn trim_log(&self) {
        let oldest = self.live_revisions().into_iter().min().unwrap_or(u64::MAX);
        let mut log = self.log.borrow_mut();
        while log.front().is_some_and(|c| c.after <= oldest) {
            log.pop_front();
        }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Line text without its trailing line break.
    pub fn line(&self, line: usize) -> String {
        let line = line.min(self.line_count() - 1);
        let mut text = self.rope.line(line).to_string();
        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }
        text
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.line_count() - 1))
    }

    /// Offset of the line break (or the end of the text on the last line).
    pub fn line_end(&self, line: usize) -> usize {
        self.coords().point_to_char(Point::new(line.min(self.line_count() - 1), usize::MAX)).0
    }

    /// Where the line's `\n` is (or the end of the text): a CR before it is content, which
    /// commands that move whole lines must carry along.
    pub(crate) fn line_end_raw(&self, line: usize) -> usize {
        match line + 1 < self.line_count() {
            true => self.rope.line_to_char(line + 1) - 1,
            false => self.len(),
        }
    }

    pub(crate) fn line_raw(&self, line: usize) -> String {
        let line = line.min(self.line_count() - 1);
        self.rope.slice(self.line_start(line)..self.line_end_raw(line)).to_string()
    }

    pub fn point(&self, offset: usize) -> (usize, usize) {
        let point = self.coords().char_to_point(CharOffset(offset));
        (point.line, point.column)
    }

    /// Clamps `col` to the line; a line past the end yields the end of the text.
    pub fn offset(&self, line: usize, col: usize) -> usize {
        self.coords().point_to_char(Point::new(line, col)).0
    }

    pub fn char_to_utf16(&self, offset: usize) -> usize {
        self.coords().char_to_utf16(CharOffset(offset)).0
    }

    pub fn utf16_to_char(&self, offset: usize) -> usize {
        self.coords().utf16_to_char(Utf16Offset(offset)).0
    }

    pub fn indent_width(&self) -> usize {
        self.indent_size.unwrap_or(self.tab_size).max(1)
    }

    pub(crate) fn indentation(&self, columns: usize) -> String {
        indentation(columns, self.tab_size, self.translate_tabs)
    }

    /// One level of indentation, with any spaces needed after hard tabs.
    pub(crate) fn indent_unit(&self) -> String {
        self.indentation(self.indent_width())
    }

    pub(crate) fn indent_of(&self, line: usize) -> String {
        self.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect()
    }

    /// `o`, or before the CR when it sits between a CR and its line feed: the CR draws as part
    /// of the line break, so no caret goes between them. Only a deletion (or text ending in a
    /// CR) can leave one there, so only those paths ask.
    pub(crate) fn off_cr_lf(&self, o: usize) -> usize {
        let o = o.min(self.len());
        match o > 0 && o < self.len() && self.rope.char(o) == '\n' && self.rope.char(o - 1) == '\r' {
            true => o - 1,
            false => o,
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewModel {
    fn new(revision: u64) -> Self {
        Self {
            overtype: false,
            virtual_space: VirtualSpace::Off,
            virtual_padding_tabs: true,
            selections: vec![Selection::caret(0)],
            primary: 0,
            bookmarks: Vec::new(),
            mark: None,
            highlights: Default::default(),
            kill: String::new(),
            dirty_from_line: None,
            quick_add_words: Vec::new(),
            quick_find_word: None,
            sel_hist: SelHistory::new(vec![Selection::caret(0)], 0),
            view_id: NEXT_VIEW.fetch_add(1, Ordering::Relaxed),
            changes: Vec::new(),
            metadata_changed: false,
            pending: None,
            found: RefCell::new(None),
            outlines_at: 0,
            snippet: None,
            revision: Rc::new(std::cell::Cell::new(revision)),
        }
    }

    /// Another view of the same document where this one is: its own copy of the selections,
    /// bookmarks, mark and highlights, at the same version.
    pub fn duplicate(&self, doc: &Document) -> Self {
        let mut view = doc.attach_view();
        view.revision.set(self.revision.get());
        view.virtual_space = self.virtual_space;
        view.selections = self.selections.clone();
        view.primary = self.primary;
        view.bookmarks = self.bookmarks.clone();
        view.mark = self.mark;
        view.highlights = self.highlights.clone();
        view.sel_hist = SelHistory::new(self.selections.clone(), self.primary);
        view
    }

    /// This view's number, unique in this run.
    pub fn view_id(&self) -> u64 {
        self.view_id
    }

    /// The document version this view's positions belong to.
    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub fn take_dirty_line(&mut self) -> Option<usize> {
        self.dirty_from_line.take()
    }

    /// Stop keeping the last find's matches (the find panel closed).
    pub fn drop_found(&self) {
        *self.found.borrow_mut() = None;
    }
}

impl EditorState {
    pub fn new() -> Self {
        Self::from_document(Document::new())
    }

    pub fn from_source(text: &str) -> Self {
        Self::from_document(Document::from_source(text))
    }

    pub fn from_text(text: &str) -> Self {
        Self::from_document(Document::from_text(text))
    }

    /// `doc` with a view of its own.
    pub fn from_document(doc: Document) -> Self {
        let view = doc.attach_view();
        Self { doc: Box::new(doc), view: Box::new(view) }
    }

    /// The document and its view, apart: a GPUI document entity takes the first.
    pub fn into_parts(self) -> (Document, ViewModel) {
        (*self.doc, *self.view)
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// This view's number, unique in this run.
    pub fn view_id(&self) -> u64 {
        self.view.view_id
    }

    /// The document's id: the same in every view of it, distinct from a file reopened.
    pub fn buffer_id(&self) -> u64 {
        self.doc.id
    }

    /// Note `edits` (the changes since the kept matches were last brought up to date, from
    /// `f.4` on) for when they are read; past 64 they are dropped for a whole search.
    pub(crate) fn defer_found(&self, edits: &[Change]) {
        let mut found = self.view.found.borrow_mut();
        if let Some(f) = &mut *found {
            f.5.extend_from_slice(edits.get(f.4..).unwrap_or(&[]));
            f.4 = 0;
            if f.5.len() > 64 {
                *found = None;
            }
        }
    }

    /// Stop keeping the last find's matches (the find panel closed).
    pub fn drop_found(&self) {
        self.view.drop_found();
    }

    /// End coalescing around an explicit insertion without changing text or selections.
    pub fn end_typing(&self) {
        self.set_typing(false);
    }

    /// Whether this view's typing group is open: its next keystroke joins the last undo step.
    pub(crate) fn typing(&self) -> bool {
        self.doc.typing.get() == Some(self.view.view_id)
    }

    /// Open this view's typing group, or close it; another view's is left as it is.
    pub(crate) fn set_typing(&self, on: bool) {
        match on {
            true if !self.doc.edits_blocked() => self.doc.typing.set(Some(self.view.view_id)),
            true => {}
            false if self.typing() => self.doc.typing.set(None),
            false => {}
        }
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    /// Apply, oldest first, the changes other views made to the document since this view's
    /// positions' version; each is applied once. The changes, for the view's presentation.
    pub fn catch_up(&mut self) -> Vec<Rc<TextChange>> {
        let at = self.view.revision.get();
        if at >= self.doc.version {
            return Vec::new();
        }
        let todo: Vec<Rc<TextChange>> = (self.doc.log.borrow().iter()).filter(|c| c.after > at).cloned().collect();
        // one sequence, mapped against the text they lead to: an intermediate text is gone, and
        // a mark mapped through one change alone may lie past the end of this one
        let me = self.view.view_id;
        let changes: Vec<Change> =
            (todo.iter()).filter(|c| c.origin != me).flat_map(|c| c.changes.iter().copied()).collect();
        if !changes.is_empty() {
            self.follow_change(&changes);
        }
        if let Some(last) = todo.last() {
            self.view.revision.set(last.after);
        }
        // a view attached while the change was made has no positions in the text before it;
        // its own edits not yet published are its own (`take_change`)
        if self.view.changes.is_empty() {
            self.view.revision.set(self.doc.version);
        }
        self.doc.trim_log();
        todo
    }

    /// Keep this view's selections, bookmarks and mark on their characters across another
    /// view's change, already made to the text.
    fn follow_change(&mut self, changes: &[Change]) {
        // another view's edit lies in none of this view's fields, so its snippet session ends
        self.view.snippet = None;
        // this view's own edits first, then the other view's, both left for when read
        self.defer_found(&self.view.changes);
        if let Some(f) = &mut *self.view.found.borrow_mut() {
            f.4 = self.view.changes.len();
        }
        let synced = self.sel_unchanged();
        self.map_own(changes);
        self.sel_follow(changes, synced);
        // the kept matches were for the text before the other view's edit
        let mut found = self.view.found.borrow_mut();
        if let Some(f) = &mut *found {
            f.5.extend_from_slice(changes);
            if f.5.len() > 64 {
                *found = None;
            }
        }
        drop(found);
        self.view.pending = None;
        self.view.quick_add_words.clear();
        if let Some(start) = changes.iter().map(|c| c.start).min() {
            let line = self.doc.rope.char_to_line(start.min(self.len()));
            self.view.dirty_from_line = Some(self.view.dirty_from_line.map_or(line, |l| l.min(line)));
        }
        self.normalize();
    }

    /// Keep this view's selections, bookmarks and mark on their characters across a text
    /// change already made (reload, another view's edit, undo of another view's edit).
    pub(crate) fn map_own(&mut self, changes: &[Change]) {
        self.map_selections(changes, false);
        self.map_marks(changes);
        self.normalize();
    }

    /// Keep the selections on their characters across `changes`, already made to the rope;
    /// with `after`, an offset where text was inserted goes after it.
    pub(crate) fn map_selections(&mut self, changes: &[Change], after: bool) {
        map_sels(&mut self.view.selections, changes, after);
        self.all_off_cr_lf();
    }

    /// Map the bookmarks and mark through `changes`, already applied to the rope. A bookmark
    /// marks a line: lines inserted at its start go above it, and it ends on its line's start.
    pub(crate) fn map_marks(&mut self, changes: &[Change]) {
        let rope = &self.doc.rope;
        map_bookmarks(&mut self.view.bookmarks, changes);
        for b in &mut self.view.bookmarks {
            *b = rope.line_to_char(rope.char_to_line(*b));
        }
        self.view.bookmarks.dedup();
        self.view.mark = self.view.mark.map(|m| map_offset(m, changes));
        for h in self.view.highlights.iter_mut().filter(|h| !h.is_empty()) {
            map_ranges(h, changes);
        }
    }

    /// The marks set aside while a command makes many splices, which then each skip them;
    /// `restore_marks` maps them over all the command's changes at once. Set aside after the
    /// undo snapshot, which keeps them. Per splice, every bookmark and highlight was remapped:
    /// Delete Bookmarked Lines on 20k lines took minutes.
    pub(crate) fn set_marks_aside(&mut self) -> MarksAside {
        MarksAside {
            bookmarks: std::mem::take(&mut self.view.bookmarks),
            highlights: std::mem::take(&mut self.view.highlights),
            mark: self.view.mark.take(),
            from: self.view.changes.len(),
        }
    }

    pub(crate) fn restore_marks(&mut self, aside: MarksAside) {
        let MarksAside { bookmarks, highlights, mark, from } = aside;
        let none = bookmarks.is_empty() && mark.is_none() && highlights.iter().all(Vec::is_empty);
        (self.view.bookmarks, self.view.highlights, self.view.mark) = (bookmarks, highlights, mark);
        // most views mark nothing: a whole-file indent's million changes need no copy then
        if !none {
            let changes = self.view.changes.get(from..).unwrap_or_default().to_vec();
            self.map_marks(&changes);
        }
    }

    pub fn take_dirty_line(&mut self) -> Option<usize> {
        self.view.take_dirty_line()
    }

    pub(crate) fn all_off_cr_lf(&mut self) {
        for i in 0..self.view.selections.len() {
            let s = self.view.selections[i];
            self.view.selections[i] =
                Selection { anchor: self.off_cr_lf(s.anchor), head: self.off_cr_lf(s.head), goal: None, ..s };
        }
    }

    /// Sort by start, merge overlapping selections (and carets touching one), never leave the list empty.
    pub(crate) fn normalize(&mut self) {
        let len = self.len();
        if self.view.selections.is_empty() {
            self.view.selections.push(Selection::caret(0));
        }
        let virtual_space = self.view.virtual_space;
        for s in &mut self.view.selections {
            s.anchor = s.anchor.min(len);
            s.head = s.head.min(len);
            if s.anchor_virtual != 0
                && (!virtual_space.allows(s.rectangular) || s.anchor != self.doc.line_end(self.doc.point(s.anchor).0))
            {
                s.anchor_virtual = 0;
            }
            if s.head_virtual != 0
                && (!virtual_space.allows(s.rectangular) || s.head != self.doc.line_end(self.doc.point(s.head).0))
            {
                s.head_virtual = 0;
            }
        }
        let p = self.view.selections[self.view.primary.min(self.view.selections.len() - 1)];
        self.view.selections.sort_by_key(|s| (s.start_point(), s.end_point()));
        let mut merged: Vec<Selection> = Vec::with_capacity(self.view.selections.len());
        for s in self.view.selections.drain(..) {
            if let Some(last) = merged.last_mut() {
                let touching_caret = s.start_point() == last.end_point() && (s.is_empty() || last.is_empty());
                if s.start_point() < last.end_point() || touching_caret {
                    let (start, end) = (last.start_point(), last.end_point().max(s.end_point()));
                    let rectangular = last.rectangular && s.rectangular;
                    *last = if last.head_point() >= last.anchor_point() {
                        Selection::from_points(start, end)
                    } else {
                        Selection::from_points(end, start)
                    };
                    last.rectangular = rectangular;
                    if !virtual_space.allows(rectangular) {
                        last.anchor_virtual = 0;
                        last.head_virtual = 0;
                    }
                    continue;
                }
            }
            merged.push(s);
        }
        self.view.selections = merged;
        // the primary is whichever selection now covers it
        self.view.primary = (self.view.selections.iter())
            .position(|s| s.start_point() <= p.start_point() && p.end_point() <= s.end_point())
            // A mixed-origin merge can clamp the primary's virtual endpoint.
            .or_else(|| self.view.selections.iter().position(|s| s.start() <= p.start() && p.end() <= s.end()))
            .unwrap_or(0);
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// The edits made since the last call, now published to the document's other views: this
    /// view's positions now belong to the document's current version.
    pub fn take_changes(&mut self) -> Vec<Change> {
        self.take_change().map_or_else(Vec::new, |c| c.changes.clone())
    }

    /// `take_changes`, as the change every view applies.
    pub fn take_change(&mut self) -> Option<Rc<TextChange>> {
        // a snippet's mirrors go out with the edit that changed their field
        self.sync_snippet();
        if let Some(s) = &mut self.view.snippet {
            s.at = 0;
        }
        self.map_outlines();
        self.view.outlines_at = 0;
        let changes = std::mem::take(&mut self.view.changes);
        self.defer_found(&changes);
        // a view that changed nothing has nothing to publish, though it may be behind
        if changes.is_empty() && !self.view.metadata_changed {
            return None;
        }
        self.view.metadata_changed = false;
        let (before, after) = (self.view.revision.get(), self.doc.version);
        self.view.revision.set(after);
        let change = Rc::new(TextChange { document: self.doc.id, before, after, changes, origin: self.view.view_id });
        self.doc.publish(change.clone());
        Some(change)
    }

    /// Bring the outlines up to the edits logged since they were last mapped.
    pub(crate) fn map_outlines(&mut self) {
        if self.view.outlines_at < self.view.changes.len() && !self.doc.outlines.is_empty() {
            map_ranges(&mut self.doc.outlines, &self.view.changes[self.view.outlines_at..]);
        }
        self.view.outlines_at = self.view.changes.len();
    }

    /// Swap in text read from disk (File > Revert, a reload) as one undoable step that ends clean.
    /// `changes` is `loaded.diff(&self.doc.rope)`, worked out off the UI thread against this text.
    pub fn reload_from(&mut self, loaded: Loaded, changes: &[Change]) {
        if self.doc.edits_blocked() {
            return;
        }
        self.view.snippet = None;
        // any view's typing, though the text read may be the same
        self.doc.typing.set(None);
        self.record();
        let metadata_changed = self.doc.endings != loaded.endings
            || self.doc.line_ending != loaded.line_ending
            || self.doc.bom != loaded.bom
            || self.doc.encoding != loaded.encoding;
        if let Some(first) = changes.first() {
            let line = self.doc.rope.char_to_line(first.start.min(self.len()));
            self.doc.rope = loaded.rope;
            self.view.changes.extend(changes);
            self.map_own(changes);
            self.touch(line);
        } else if metadata_changed {
            self.view.metadata_changed = true;
            self.touch(0);
        }
        self.view.pending = None;
        (self.doc.disk, self.doc.deleted) = (loaded.disk, false);
        self.doc.endings = loaded.endings;
        self.doc.line_ending = loaded.line_ending;
        self.doc.bom = loaded.bom;
        self.doc.encoding = loaded.encoding;
        self.doc.disk_encoding = loaded.encoding;
        self.doc.read_only = loaded.read_only;
        // what the file holds now, as it was read
        self.doc.saved = Some(self.doc.save_point());
        self.doc.dirty = false;
    }

    /// A trusted follow snapshot; no user undo point is retained for incoming log data.
    pub fn follow_from(&mut self, loaded: Loaded, changes: &[Change]) {
        if !self.doc.is_following() || self.doc.viewer_mode() {
            return;
        }
        // Only this trusted ingestion path bypasses the transient lock. Reuse reload's
        // position mapping and metadata without retaining a fresh full rope each poll.
        self.doc.following = false;
        let version = self.doc.version;
        self.reload_from(loaded, changes);
        self.doc.following = true;
        if self.doc.version != version {
            self.doc.history = History::new();
        }
    }

    /// Every edit goes through here (or `restore`/`reload_from`) and is logged for the
    /// document's other views; this view's bookmarks and mark follow it, its selections are the
    /// caller's.
    pub(crate) fn splice(&mut self, range: Range<usize>, text: &str) {
        if self.doc.edits_blocked() {
            return;
        }
        let n = self.view.changes.len();
        if text.is_empty() {
            // a deletion is one change: no copy of what goes (copied and split, select all +
            // delete on 100 MB peaks at 500 MB)
            if !range.is_empty() {
                self.view.changes.push(Change {
                    start: range.start,
                    removed: range.len(),
                    inserted: 0,
                    lines: (self.doc.rope.slice(range.clone()).len_lines() - 1, 0),
                });
            }
        } else {
            let old = self.doc.rope.slice(range.clone()).to_string();
            minimal(range.start, &old, text, &mut self.view.changes);
        }
        if self.view.changes.len() > n {
            let before = self
                .doc
                .endings
                .as_ref()
                .filter(|_| self.view.changes[n..].iter().any(|c| c.lines != (0, 0)))
                .map(|_| self.doc.rope.clone());
            self.doc.rope.remove(range.clone());
            self.doc.rope.insert(range.start, text);
            if let Some(before) = before {
                let doc = &mut *self.doc;
                if let Some(endings) = &mut doc.endings {
                    endings.apply_changes(&before, &doc.rope, &self.view.changes[n..], doc.line_ending);
                }
            }
            let new = self.view.changes[n..].to_vec();
            self.map_marks(&new);
        }
    }

    /// Mark a real change: the pending snapshot becomes an undo step and redo is cleared.
    pub(crate) fn touch(&mut self, line: usize) {
        if self.doc.edits_blocked() {
            return;
        }
        if let Some(snap) = self.view.pending.take() {
            let h = &mut self.doc.history;
            h.undo.push(snap);
            h.redo.clear();
        }
        // a new text: no undo step holds it yet, and it is not the file's
        self.doc.state = next_state();
        // another view's change sits between a view's typing and its next keystroke
        if !self.typing() {
            self.doc.typing.set(None);
        }
        self.view.dirty_from_line = Some(self.view.dirty_from_line.map_or(line, |l| l.min(line)));
        self.doc.version += 1;
        self.doc.dirty = !self.doc.scratch;
        self.view.sel_hist.edited = true;
    }

    /// Replace all the text, as a panel field's new content: no undo step and no history left,
    /// no typing group, and everything selected (so typing replaces it, as in Sublime's panels).
    pub fn reset_text(&mut self, text: &str) {
        if self.doc.edits_blocked() {
            return;
        }
        self.view.snippet = None;
        self.doc.typing.set(None);
        self.view.pending = None;
        let (len, n) = (self.len(), self.view.changes.len());
        self.splice(0..len, text);
        // the same text again is still news to whoever listens to the field
        if self.view.changes.len() == n {
            self.view.changes.push(Change::default());
        }
        self.doc.history = History::new();
        self.doc.state = next_state();
        self.doc.saved = Some(self.doc.save_point());
        self.doc.dirty = false;
        self.view.dirty_from_line = Some(0);
        self.doc.version += 1;
        self.select_all();
    }

    /// `append`, with the matches in `text` outlined (Find Results streaming in): `outlines` are
    /// offsets in `text`, added once the outlines so far are mapped through the append.
    pub fn append_outlined(&mut self, text: &str, outlines: impl IntoIterator<Item = Range<usize>>) {
        if self.doc.edits_blocked() {
            return;
        }
        let base = self.len();
        self.append(text);
        self.map_outlines();
        (self.doc.outlines).extend(outlines.into_iter().map(|r| base + r.start..base + r.end));
    }

    /// Text added at the end (Find Results streaming in): logged for other views, but not an
    /// undo step, and the history goes, so an undo never takes results back out.
    pub fn append(&mut self, text: &str) {
        if self.doc.edits_blocked() {
            return;
        }
        let end = self.len();
        let line = self.doc.rope.char_to_line(end);
        // any view's typing: its next keystroke starts a step after the appended text
        self.doc.typing.set(None);
        self.view.pending = None;
        self.splice(end..end, text);
        self.doc.history = History::new();
        self.doc.state = next_state();
        // a results tab or console saved as a file has unsaved text again
        self.doc.refresh_dirty();
        self.view.dirty_from_line = Some(self.view.dirty_from_line.map_or(line, |l| l.min(line)));
        self.doc.version += 1;
    }

    /// Replace every selection in order; the caret lands after the inserted text.
    pub(crate) fn edit(&mut self, mut f: impl FnMut(&Self, &Selection) -> (Range<usize>, String)) {
        self.edit_with(|s, _, sel| {
            let (range, text) = f(s, sel);
            let n = text.chars().count();
            let caret = if text.is_empty() && range.is_empty() && sel.has_virtual() {
                Selection::from_points(sel.start_point(), sel.start_point())
            } else {
                Selection::caret(range.start + n)
            };
            (range, text, caret)
        });
    }

    /// Replace every selection at once, as Sublime does. `f` is called once for each selection,
    /// with its index in `selections`, in the text as it was, and returns the range to replace,
    /// the replacement text and the selection to keep (in coordinates after its own replacement
    /// alone). Carets asking for the same change of some text (Upper Case twice in one word)
    /// make it once. A deletion overlapping an earlier edit merges into it. A replacement
    /// overlapping one is made after it, on the edited text, where an earlier edit of the same
    /// length leaves its place exact; over text an earlier edit lengthened or shortened it is
    /// left out rather than overwrite that edit.
    pub(crate) fn edit_with(
        &mut self,
        mut f: impl FnMut(&Self, usize, &Selection) -> (Range<usize>, String, Selection),
    ) {
        if self.doc.edits_blocked() {
            return;
        }
        self.normalize();
        // every call counted, wherever it is made: stateful closures (Transpose) need one each
        #[cfg(debug_assertions)]
        let (selections, mut calls) = (self.view.selections.len(), 0);
        #[cfg(debug_assertions)]
        let mut f = |s: &Self, i, sel: &Selection| {
            calls += 1;
            f(s, i, sel)
        };
        let edits: Vec<_> = (self.view.selections.iter().enumerate()).map(|(i, sel)| f(self, i, sel)).collect();
        let mut order: Vec<usize> = (0..edits.len()).collect();
        order.sort_by_key(|&i| (edits[i].0.start, edits[i].0.end));
        // (range, text) per merged group; each edit's group and where its text starts in it
        let mut groups: Vec<(Range<usize>, String)> = Vec::new();
        let mut place = vec![(0, 0); edits.len()];
        let mut later: Vec<usize> = Vec::new();
        // an edit of some text the same as an earlier one: that one's index. Sorted by range, the
        // edits of one range sit together: each is compared with that run's distinct edits
        let mut same: Vec<Option<usize>> = vec![None; edits.len()];
        let mut distinct: Vec<usize> = Vec::new();
        for &i in &order {
            let (r, t, _) = &edits[i];
            // an insertion is never one change made twice
            if r.is_empty() || distinct.first().is_some_and(|&j| edits[j].0 != *r) {
                distinct.clear();
            }
            if let Some(&first) = distinct.iter().find(|&&j| edits[j].1 == *t) {
                same[i] = Some(first);
                continue;
            }
            if !r.is_empty() {
                distinct.push(i);
            }
            let last = groups.len().wrapping_sub(1);
            match groups.last_mut() {
                Some((g, _)) if r.start < g.end && !t.is_empty() => later.push(i),
                Some((g, text)) if r.start < g.end => {
                    g.end = g.end.max(r.end);
                    place[i] = (last, text.chars().count());
                    text.push_str(t);
                }
                _ => {
                    place[i] = (groups.len(), 0);
                    groups.push((r.clone(), t.clone()));
                }
            }
        }
        // applied in order, each shifted by the ones before it
        let aside = self.set_marks_aside();
        let mut applied: Vec<Change> = Vec::with_capacity(groups.len());
        let mut delta = 0isize;
        for (r, text) in &groups {
            let start = (r.start as isize + delta) as usize;
            let n = self.view.changes.len();
            let line = self.doc.rope.char_to_line(start);
            self.splice(start..start + r.len(), text);
            if self.view.changes.len() > n {
                self.touch(line);
            }
            let inserted = text.chars().count();
            applied.push(Change { start, removed: r.len(), inserted, ..Change::default() });
            delta += inserted as isize - r.len() as isize;
        }
        // each kept selection (an edit made later is placed below), and whether it may not
        // sit inside a CRLF
        let mut out: Vec<(Selection, bool)> = (edits.iter().enumerate())
            .map(|(i, (r, t, sel))| {
                let (r, t, sel) = match same[i] {
                    Some(first) => (&edits[first].0, &edits[first].1, sel),
                    None => (r, t, sel),
                };
                let n = t.chars().count();
                let (g, at) = place[same[i].unwrap_or(i)];
                let map = |p: usize| {
                    if (r.start..=r.start + n).contains(&p) {
                        applied.get(g).map_or(p, |c| c.start + at + p - r.start)
                    } else {
                        let p = if p < r.start { p } else { p - n + r.len() };
                        map_offset_sorted(p, &groups, &applied)
                    }
                };
                let off = t.is_empty() || t.ends_with('\r');
                (Selection { anchor: map(sel.anchor), head: map(sel.head), goal: None, ..*sel }, off)
            })
            .collect();
        // the copies of each edit made later (`same`), placed with it
        let mut copies: Vec<Vec<usize>> = Vec::new();
        if !later.is_empty() {
            copies = vec![Vec::new(); edits.len()];
            for (j, first) in same.iter().enumerate() {
                if let Some(first) = first {
                    copies[*first].push(j);
                }
            }
        }
        let mut since: Vec<Change> = Vec::new();
        for i in later {
            let (r, t, _) = &edits[i];
            let onto = |p: usize| map_within(map_offset_sorted(p, &groups, &applied), &since);
            // over text an earlier edit lengthened or shortened, no place is exact
            let shifted = (groups.iter().zip(&applied))
                .any(|((g, _), c)| g.start < r.end && r.start < g.end && c.inserted != g.len());
            if shifted && !r.is_empty() {
                for j in std::iter::once(i).chain(copies[i].iter().copied()) {
                    let sel = self.view.selections[j];
                    out[j] = (Selection::new(onto(sel.anchor), onto(sel.head)), false);
                }
                continue;
            }
            let (start, end) = (onto(r.start), onto(r.end).max(onto(r.start)));
            let n = t.chars().count();
            // after this edit's own replacement too
            let keep_at = |p: usize| match p {
                p if (r.start..=r.start + n).contains(&p) => start + p - r.start,
                p if p < r.start => onto(p),
                p => onto(p - n + r.len()) - (end - start) + n,
            };
            let kept = |j: usize| {
                let keep = edits[j].2;
                Selection::new(keep_at(keep.anchor), keep_at(keep.head))
            };
            let kept: Vec<_> = std::iter::once(i).chain(copies[i].iter().copied()).map(|j| (j, kept(j))).collect();
            let before = self.view.changes.len();
            let line = self.doc.rope.char_to_line(start.min(self.len()));
            self.splice(start..end, t);
            if self.view.changes.len() > before {
                self.touch(line);
            }
            let new = self.view.changes[before..].to_vec();
            for (s, _) in &mut out {
                *s = Selection::new(map_within(s.anchor, &new), map_within(s.head, &new));
            }
            for (j, sel) in kept {
                out[j] = (sel, t.is_empty() || t.ends_with('\r'));
            }
            since.extend(new);
        }
        self.restore_marks(aside);
        self.view.selections = out
            .into_iter()
            .enumerate()
            .map(|(i, (mut s, off))| {
                s.rectangular = self.view.selections[i].rectangular;
                match off {
                    true => {
                        Selection { anchor: self.off_cr_lf(s.anchor), head: self.off_cr_lf(s.head), goal: None, ..s }
                    }
                    false => s,
                }
            })
            .collect();
        self.normalize();
        #[cfg(debug_assertions)]
        assert_eq!(calls, selections, "f once a selection");
    }
}
