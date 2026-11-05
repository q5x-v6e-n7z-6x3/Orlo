use super::*;

/// The folder of `editor`'s file.
fn folder_of(editor: &Entity<Editor>, cx: &App) -> Option<PathBuf> {
    Some(editor.read(cx).doc(cx).path.as_deref()?.parent()?.to_path_buf())
}

fn first_of_path<'a>(
    mut tabs: impl Iterator<Item = &'a Entity<Editor>>,
    path: &Path,
    cx: &App,
) -> Option<Entity<Editor>> {
    tabs.find(|e| e.read(cx).doc(cx).path.as_deref() == Some(path)).cloned()
}

/// One view of each document among `editors` that `keep` accepts; the others show the same one.
pub(crate) fn one_per_document(
    editors: impl IntoIterator<Item = Entity<Editor>>,
    cx: &App,
    keep: impl Fn(&Editor) -> bool,
) -> Vec<Entity<Editor>> {
    let mut seen = HashSet::new();
    (editors.into_iter())
        .filter(|e| {
            let v = e.read(cx);
            keep(v) && seen.insert(v.document.entity_id())
        })
        .collect()
}

impl Workspace {
    /// `None` when the active group is empty.
    pub fn active_editor(&self) -> Option<Entity<Editor>> {
        self.views.active_tab().cloned()
    }

    /// An open view, preferring the active group for pending navigation.
    pub(crate) fn editor_for_path(&self, path: &Path, cx: &App) -> Option<Entity<Editor>> {
        let tabs = self.views.groups[self.views.active].tabs.iter();
        first_of_path(tabs.chain(self.views.tabs()), path, cx)
    }

    /// The first tab of this window with `path` open.
    pub(crate) fn tab_for_path(&self, path: &Path, cx: &App) -> Option<Entity<Editor>> {
        first_of_path(self.views.tabs(), path, cx)
    }

    /// Run `f` on the active view; nothing happens when the active group is empty.
    pub(crate) fn with_active<R>(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Editor, &mut Context<Editor>) -> R,
    ) -> Option<R> {
        Some(self.active_editor()?.update(cx, f))
    }

    /// The active group's view, or the window itself when that group is empty, so workspace
    /// keys keep working.
    pub fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_restoring_active();
        self.git.selection_active = false;
        match self.active_editor() {
            Some(e) => e.read(cx).focus(window),
            None => window.focus(&self.focus_handle),
        }
    }

    pub(crate) fn add_tab(&mut self, state: EditorState, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.new_editor(state, cx);
        let path = editor.read(cx).doc(cx).path.as_ref();
        if let Some(saved) = self.closed.iter().rev().find(|v| Some(&v.path) == path) {
            editor.update(cx, |e, cx| {
                let folds = saved.folds.as_ref().map(|f| f.restore(e.doc(cx))).unwrap_or_default();
                e.set_folds_quietly(folds, cx);
            });
        }
        self.attach(editor, false, window, cx);
    }

    /// A view of a newly opened document, with the settings' tab size and the window's display.
    /// A file already open in a tab of any window is not opened twice: the view is another of
    /// the document there, keeping the selections and bookmarks `state` came with.
    pub(crate) fn new_editor(&self, mut state: EditorState, cx: &mut Context<Self>) -> Entity<Editor> {
        let open = (state.doc.path.as_deref())
            .filter(|_| !state.doc.scratch && !state.doc.unsaved())
            .and_then(|p| crate::documents::Documents::for_path(p, cx));
        let view = self.view.clone();
        if let Some(document) = open {
            let (_, own) = state.into_parts();
            let attached = document.read(cx).model.adopt_view(own);
            return self.view_of(document, attached, cx);
        }
        (state.doc.tab_size, state.doc.translate_tabs) = self.default_indent(cx);
        cx.new(|cx| {
            let mut e = Editor::new(state, false, cx);
            e.view = view;
            let warnings = self.prepare_editor(&mut e, cx);
            e.preference_warning = (!warnings.is_empty()).then(|| warnings.join("\n"));
            e
        })
    }

    /// A view of `document`, already open in some tab, with the view state `attached` made
    /// for it and this window's display.
    pub(crate) fn view_of(
        &self,
        document: Entity<OpenDocument>,
        attached: crate::editing::text::ViewModel,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let view = self.view.clone();
        cx.new(|cx| {
            let mut e = Editor::of_document(document, attached, false, cx);
            e.view = view;
            self.derive_modelines(&mut e, cx);
            e.view = self.view_for_editor(&e, cx);
            e
        })
    }

    /// Hook a view up to the window: redraws, its edits and results, focus.
    pub(crate) fn wire(&mut self, editor: &Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let editor = editor.clone();
        editor.update(cx, |e, cx| {
            e.close_completion(cx);
            e.close_definition_hover(cx);
        });
        let observe = cx.observe(&editor, |_, _, cx| cx.notify());
        self.watch_document(&editor, window, cx);
        let events = cx.subscribe_in(&editor, window, |ws, editor, event: &EditorEvent, window, cx| match event {
            EditorEvent::OpenResult(path, line, hit) => {
                let (path, hit) = (path.clone(), hit.clone());
                ws.open_result(editor.clone(), path, *line, hit, window, cx)
            }
            EditorEvent::HoverDefinition(word, name, version) => {
                ws.hover_definitions(editor.clone(), word.clone(), name.clone(), *version, window, cx)
            }
            EditorEvent::OpenDefinition(target, side) => ws.open_definition(target.clone(), *side, window, cx),
            EditorEvent::SpellingSuggestions(at) => ws.spelling_suggestions(editor.clone(), *at, window, cx),
            EditorEvent::ScanSpelling(forward) => ws.scan_spelling(editor.clone(), *forward, window, cx),
            EditorEvent::Status(text) => ws.status_message(text.clone(), cx),
            EditorEvent::Scrolled => ws.sync_scroll_from(editor, cx),
            EditorEvent::FoldsChanged => autosave(cx),
            EditorEvent::ShowChange(index) => ws.show_change_for(editor.clone(), Some(*index), window, cx),
            EditorEvent::SignatureInput(text, active, trigger) if ws.active_editor().as_ref() == Some(editor) => {
                ws.signature_input(text.clone(), active.clone(), *trigger, window, cx)
            }
            EditorEvent::ServerCompletions if ws.active_editor().as_ref() == Some(editor) => {
                ws.server_completions(true, window, cx)
            }
            EditorEvent::CommitServerItem(item, since) => ws.commit_server_item(*item, since.clone(), window, cx),
            EditorEvent::ChooseSnippet(paths, trigger) => ws.choose_snippet(paths.clone(), trigger.clone(), window, cx),
            _ => {}
        });
        // a click into any group's view makes that group the active one
        let weak = editor.downgrade();
        let handle = editor.read(cx).focus_handle(cx);
        let focus_sub = cx.on_focus(&handle, window, move |ws, window, cx| {
            ws.git.selection_active = false;
            #[cfg(test)]
            FOCUS_RUNS.set(FOCUS_RUNS.get() + 1);
            let Some(editor) = weak.upgrade() else { return };
            if let Some(dir) = folder_of(&editor, cx) {
                ws.last_dir = Some(dir);
            }
            let Some(at) = ws.views.find(&editor) else {
                return;
            };
            // a click into the text ends Goto Anything and the palette
            if ws.quick_panel.is_some() {
                ws.cancel_quick_panel(cx);
                cx.defer_in(window, |_, _, cx| cx.notify());
            }
            if at != (ws.views.active, ws.views.groups[ws.views.active].active) {
                ws.views.select(at.0, at.1);
                ws.invalidate_restoring_active();
                ws.refresh_matches(cx);
                // focus listeners run mid-draw, where gpui drops a notify; redraw after the frame
                cx.defer_in(window, |_, _, cx| cx.notify());
            }
            // Delimiter checks and language-server actions belong to the focused view.
            if ws.menu_delim != editor.read(cx).doc(cx).delimiter_override()
                || ws.menu_range_formatting != ws.has_range_formatting(cx)
                || ws.menu_language_navigation != ws.navigation_capabilities(cx)
            {
                ws.refresh_menus(cx);
            }
        });
        // a view that is not drawn (a tab left for another) cannot see it lost the focus
        let weak = editor.downgrade();
        let blur_sub = cx.on_blur(&handle, window, move |ws, window, cx| {
            // Opening guarded choices owns this blur; later context changes still cancel them.
            if !ws.quick_panel.as_ref().is_some_and(|p| {
                (p.kind == Kind::Spelling || p.language_navigation.is_some()) && p.input.read(cx).is_focused(window)
            }) {
                ws.invalidate_navigation();
            }
            weak.update(cx, |e, cx| {
                e.close_completion(cx);
                e.close_snippet_choices(cx);
            })
            .ok();
        });
        let subs = vec![observe, events, focus_sub, blur_sub];
        (self.focus_subs).insert(editor.entity_id(), subs);
        // the completion list offers the words of the other buffers open, in any window
        let ws = cx.weak_entity();
        editor.update(cx, |e, _| {
            e.completion_index = Some(self.index.downgrade());
            e.completion_recent = self.completion_recent.clone();
            e.open_views = Some(std::rc::Rc::new(move |cx: &App| {
                ws.upgrade().map_or_else(Vec::new, |ws| ws.read(cx).open_editors(cx))
            }));
        });
        if let Some(dir) = folder_of(&editor, cx) {
            self.last_dir = Some(dir);
        }
        self.index.update(cx, |i, cx| i.open_view(&editor, cx));
        cx.notify();
    }

    /// Wire a new view into the window and show it in the active group: appended, or right after
    /// the active tab when `next` (New View into File).
    pub(crate) fn attach(&mut self, editor: Entity<Editor>, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.wire(&editor, window, cx);
        if let Some(warning) = editor.update(cx, |e, _| e.preference_warning.take()) {
            self.status_message(warning, cx);
        }
        autosave(cx);
        if next {
            self.views.insert_next(editor);
        } else {
            self.views.add(editor);
        }
        self.focus_editor(window, cx);
        self.refresh_file_autosaves(cx);
        cx.notify();
    }

    /// Follow `editor`'s document in this window, once however many of its tabs show it: its
    /// path finds it from then on (`Documents`).
    fn watch_document(&mut self, editor: &Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let document = editor.read(cx).document.clone();
        crate::documents::Documents::shown(&document, cx);
        let id = document.read(cx).id();
        if self.doc_subs.contains_key(&id) {
            return;
        }
        let sub =
            cx.subscribe_in(&document, window, |ws, _, event: &DocumentEvent, _, cx| ws.document_changed(event, cx));
        self.doc_subs.insert(id, sub);
    }

    /// What this window does when a document it shows changes, whichever view or window made
    /// the change: the tabs' titles and dots redraw, and on a text change the navigation under
    /// way is dropped, the quick panel's symbol preview and the find panel's outlines stay on
    /// their text, an edited preview is kept and the session is written soon.
    fn document_changed(&mut self, event: &DocumentEvent, cx: &mut Context<Self>) {
        // the views draw the markers; the tabs and the session do not hold them
        if let DocumentEvent::Changes { .. } = event {
            return;
        }
        if let DocumentEvent::File { document, renamed: true } = event {
            for editor in self.views.tabs() {
                if editor.read(cx).doc(cx).id() == *document {
                    editor.update(cx, |e, cx| {
                        self.apply_editor_preferences(e, cx);
                        cx.notify();
                    });
                }
            }
        }
        if let DocumentEvent::Edited(change) = event {
            self.refresh_modelines(change.document, cx);
            self.invalidate_navigation();
            if let Some((view, ranges)) = (self.quick_panel.as_mut()).and_then(|p| p.symbol_preview.as_mut()) {
                if view.read(cx).doc(cx).id() == change.document {
                    crate::editing::text::map_ranges(ranges, &change.changes);
                }
            }
            self.promote_if_edited(change.document, cx);
            if self.find_panel().is_some() {
                self.refresh_matches_after_edit(change.document, &change.changes, cx);
            }
        }
        self.schedule_changed_file_autosave(event.document(), cx);
        autosave(cx);
        cx.notify();
    }

    /// A lone blank untitled tab in the active group gives way to a file being opened.
    pub(crate) fn drop_blank_untitled(&mut self, cx: &mut Context<Self>) {
        if self.keep_blank_for_preview {
            return;
        }
        let group = &self.views.groups[self.views.active];
        if let [only] = group.tabs.as_slice() {
            if !self.views.is_pinned(only) && Self::is_blank_untitled(only, cx) {
                let only = only.clone();
                self.remove_view(&only, cx);
            }
        }
    }

    /// Take a view out of its group, with its focus listener and its place in the index.
    pub(crate) fn remove_view(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        self.views.remove(editor);
        self.focus_subs.remove(&editor.entity_id());
        let document = editor.read(cx).document.clone();
        crate::documents::Documents::hidden(&document, cx);
        cx.defer(file_autosave::refresh_all);
        if !self.views.tabs().any(|t| t.read(cx).document == document) {
            self.doc_subs.remove(&document.read(cx).id());
        }
        self.index.update(cx, |i, cx| i.close_view(editor, cx));
        cx.notify();
    }

    /// Whether `f` is one of this window's views'.
    pub(crate) fn is_view_focus(&self, f: &FocusHandle, cx: &App) -> bool {
        self.views.tabs().any(|t| t.read(cx).focus_handle(cx) == *f)
    }

    pub(crate) fn is_blank_untitled(editor: &Entity<Editor>, cx: &App) -> bool {
        let (e, d) = (editor.read(cx), editor.read(cx).doc(cx));
        d.path.is_none() && !d.dirty && d.is_empty() && !e.results && e.label.is_none()
    }

    /// Sublime's `inactive_sheet_dimming`: views outside the active group get a lighter
    /// background. Only views whose state changes are touched.
    pub(crate) fn sync_dimming(&self, cx: &mut Context<Self>) {
        for (g, group) in self.views.groups.iter().enumerate() {
            let input = group.tabs.get(group.active);
            for t in &group.tabs {
                // tabs shown side by side without the input are dimmed too
                let dim = g != self.views.active || Some(t) != input;
                if t.read(cx).dimmed != dim {
                    t.update(cx, |e, cx| {
                        e.dimmed = dim;
                        cx.notify();
                    });
                }
            }
        }
    }
}
