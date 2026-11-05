//! File actions: new, open, save, close, revert, recent files, folders, quit and their prompts.

use super::*;
use gpui::EntityInputHandler;
use std::collections::VecDeque;
use std::os::unix::fs::MetadataExt;

/// Reopen Closed File's history, oldest dropped first.
const MAX_CLOSED: usize = 50;

impl Workspace {
    pub(crate) fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        // the new tab takes the keys from Goto Anything or the palette (its focus would too)
        self.cancel_quick_panel(cx);
        // a fresh window's hidden blank tab gives way, so one untitled tab shows, not two
        self.drop_blank_untitled(cx);
        self.add_tab(EditorState::new(), window, cx);
    }

    forward_actions! {
    pub(crate) open(Open, window, cx) => prompt_paths(picker(true, true, true), window, cx, Self::open_chosen);
    pub(crate) add_folder(AddFolder, window, cx) => prompt_paths(picker(false, true, false), window, cx, Self::open_chosen);
    pub(crate) hash_file(HashFile, window, cx) => prompt_paths(picker(true, false, true), window, cx, Self::hash_paths);
    pub(crate) save(Save, window, cx) => save_active(false, window, cx);
    pub(crate) save_as(SaveAs, window, cx) => save_active(true, window, cx);
    pub(crate) close_window(CloseWindow, window, cx) => confirm_close(window, cx);
    }

    /// `prompt_paths` for one file or folder: `chosen` gets the first path picked.
    pub(crate) fn prompt_path(
        &mut self,
        options: PathPromptOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
        chosen: impl FnOnce(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.prompt_paths(options, window, cx, |ws, paths, window, cx| {
            if let Some(path) = paths.into_iter().next() {
                chosen(ws, path, window, cx);
            }
        });
    }

    fn open_chosen(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        for path in paths {
            self.open_path(path, window, cx);
        }
    }

    /// The files' bytes on disk, hashed off the UI thread, then shown in an alert.
    pub(crate) fn hash_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            (paths.into_iter()).map(|p| (convert::digest_file(&p), p)).collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let hashed = task.await;
            this.update_in(cx, |ws, window, cx| ws.show_hashes(hashed, window, cx)).ok();
        })
        .detach();
    }

    /// Every digest of every file, with a Copy button for `algorithm  digest  path` lines.
    fn show_hashes(
        &mut self,
        hashed: Vec<(std::io::Result<[String; 4]>, PathBuf)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mut blocks, mut lines) = (Vec::new(), Vec::new());
        for (digests, path) in &hashed {
            let path = path.display();
            match digests {
                Ok(digests) => {
                    let mut block = vec![path.to_string()];
                    for (h, d) in Hash::ALL.iter().zip(digests) {
                        block.push(format!("{}  {d}", h.name()));
                        lines.push(format!("{}  {d}  {path}", h.name()));
                    }
                    blocks.push(block.join("\n"));
                }
                Err(e) => blocks.push(format!("{path}\nCould not read: {e}")),
            }
        }
        let message = if hashed.len() == 1 { "Hash File" } else { "Hash Files" };
        let buttons: &[&str] = if lines.is_empty() { &["OK"] } else { &["Copy", "Close"] };
        let answer = window.prompt(PromptLevel::Info, message, Some(&blocks.join("\n\n")), buttons, cx);
        if lines.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            if let Ok(0) = answer.await {
                // a digest is not a Copy: it stays out of the clipboard history
                this.update(cx, |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")))).ok();
            }
        })
        .detach();
    }

    fn save_active(&mut self, ask_path: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.save_editor(editor, ask_path, After::Nothing, window, cx);
        }
    }

    pub(crate) fn save_without_formatting(
        &mut self,
        _: &SaveWithoutFormatting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor() {
            self.save_editor(editor, false, After::WithoutFormatting, window, cx);
        }
    }

    pub(crate) fn save_copy_as(&mut self, _: &SaveCopyAs, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.save_editor(editor, true, After::Copy, window, cx);
        }
    }

    fn write_copy(&mut self, editor: Entity<Editor>, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let document = editor.read(cx).document.clone();
        let handle = self.window.downcast::<Workspace>();
        let completed = move |outcome: &crate::documents::SaveOutcome, cx: &mut App| {
            if let Some(handle) = handle {
                handle.update(cx, |ws, window, cx| ws.copied(outcome, window, cx)).ok();
            }
        };
        if let Some(outcome) = OpenDocument::save_copy(&document, path, completed, cx) {
            self.copied(&outcome, window, cx);
        }
    }

    fn copied(&mut self, outcome: &crate::documents::SaveOutcome, window: &mut Window, cx: &mut Context<Self>) {
        match outcome {
            Ok(_) => self.status_message("Copy saved", cx),
            Err(error) => self.alert(window, cx, "Could not save copy", error),
        }
    }

    pub(crate) fn choosing_save_for(&self, document: &Entity<OpenDocument>, cx: &App) -> bool {
        self.saves
            .current
            .iter()
            .chain(self.saves.waiting.iter())
            .any(|(editor, _, _)| editor.read(cx).document == *document)
    }

    /// Files deleted on disk need Save As: their sheets come one after another.
    /// Untitled tabs are skipped; Sublime would prompt for each.
    pub(crate) fn save_all(&mut self, _: &SaveAll, window: &mut Window, cx: &mut Context<Self>) {
        // a Save As panel is up (perhaps Save All's own): Save All does nothing more
        if self.saves.busy() {
            return;
        }
        let named: Vec<_> = (self.views.tabs()).filter(|t| t.read(cx).doc(cx).path.is_some()).cloned().collect();
        let mut sheets: Vec<Entity<Editor>> = Vec::new();
        // each document once, however many views of it there are: its save is under way, not
        // landed, when the next view comes up
        let mut saved = HashSet::new();
        for editor in named {
            let e = editor.read(cx);
            if !e.doc(cx).unsaved() || !saved.insert(e.document.entity_id()) {
                continue;
            }
            if e.doc(cx).deleted {
                sheets.push(editor);
            } else {
                self.save_editor(editor, false, After::Nothing, window, cx);
            }
        }
        sheets.reverse();
        self.save_sheets = sheets;
        self.save_next(window, cx);
    }

    /// Save All's next Save As sheet, after the previous one was saved or cancelled.
    fn save_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(editor) = self.save_sheets.pop() {
            if editor.read(cx).doc(cx).unsaved() && self.views.find(&editor).is_some() {
                return self.save_editor(editor, false, After::SaveNext, window, cx);
            }
        }
    }

    fn save_editor(
        &mut self,
        editor: Entity<Editor>,
        ask_path: bool,
        after: After,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let e = editor.read(cx);
        if e.doc(cx).is_following() {
            self.status_message("Stop following before saving", cx);
            if after == After::SaveNext {
                self.save_next(window, cx);
            }
            return;
        }
        let (path, read_only, title) = (e.doc(cx).path.clone(), e.doc(cx).read_only, e.title(cx));
        let deleted = e.doc(cx).deleted && path.is_some();
        // a deleted file goes to Save As below, lossy or not: there is no original to protect
        if read_only && !ask_path && !deleted {
            self.alert(
                window,
                cx,
                "Cannot save",
                &format!(
                    "{title} could not be decoded without loss and was opened read-only. Use Save As to write a copy."
                ),
            );
            return;
        }
        // a file deleted on disk is not silently recreated: Save As, in its old folder
        if let Some(path) = path.as_deref().filter(|_| !ask_path && !deleted) {
            // the document's own save under way is not another writer: this one follows it
            let own = editor.read(cx).document.read(cx).saving();
            if pipelines::writing(path, cx) && !own {
                self.status_message("The file is being written; save again when the operation finishes", cx);
                if after == After::SaveNext {
                    self.save_next(window, cx);
                }
                return;
            }
            return self.write_editor(editor, None, None, after, window, cx);
        }
        // gpui's Save panel is not modal: the window keeps taking keys while it is up, so a
        // request waits for it to close (`next_save`) instead of opening a second one
        if !self.saves.request((editor.clone(), ask_path, after)) {
            return;
        }
        // the file's own folder, as Sublime; an untitled buffer's is the last file's
        let dir = (path.as_deref().and_then(Path::parent))
            .map(Path::to_path_buf)
            .or_else(|| self.last_dir.clone())
            .or_else(|| self.roots.first().map(|r| r.path.clone()))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let rx = cx.prompt_for_new_path(&dir, Some(&title));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else {
                this.update_in(cx, |ws, window, cx| {
                    ws.saves.closed();
                    if after == After::Copy {
                        ws.status_message("Save a Copy As canceled", cx);
                    }
                    if after == After::SaveNext {
                        ws.save_next(window, cx);
                    }
                    ws.next_save(window, cx);
                })
                .ok();
                return;
            };
            if after == After::Copy {
                this.update_in(cx, |ws, window, cx| {
                    ws.saves.closed();
                    ws.write_copy(editor, path, window, cx);
                    ws.next_save(window, cx);
                })
                .ok();
                return;
            }
            let path = crate::editing::text::resolve(&path);
            pipelines::wait_for_write(&path, cx).await;
            this.update_in(cx, |ws, window, cx| {
                ws.saves.closed();
                let resolved = crate::editing::text::resolve(&path);
                let identity = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino()));
                let target = identity(&resolved);
                let previous_encoding = ws.open_editors(cx).iter().find_map(|e| {
                    let s = e.read(cx).doc(cx);
                    (s.path.as_deref())
                        .is_some_and(|p| p == resolved || (target.is_some() && identity(p) == target))
                        .then_some(s.disk_encoding)
                });
                // one document per file, however the panel spelled the folder
                ws.write_editor(editor, Some(resolved), previous_encoding, after, window, cx);
                ws.next_save(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Write `editor`'s document off the UI thread (`OpenDocument::save`), to `path` for a
    /// Save As, tidied just before its text is taken; once it lands, what follows a save: the
    /// new file's preferences after a Save As, the index, the settings, pipeline, project and
    /// EditorConfig files, then `after`.
    fn write_editor(
        &mut self,
        editor: Entity<Editor>,
        path: Option<PathBuf>,
        previous: Option<crate::editing::encoding::Encoding>,
        after: After,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let own = editor.read(cx).doc(cx).path.as_ref();
        let save_as = path.as_ref().is_some_and(|p| own != Some(p));
        let Some(handle) = self.window.downcast::<Workspace>() else {
            return;
        };
        let document = editor.read(cx).document.clone();
        let tidy = self.cleanup_for_save(&editor, path.as_deref(), cx);
        let prepare: crate::documents::save::Prepare = if after == After::WithoutFormatting {
            Box::new(move |cx| {
                tidy(cx);
                None
            })
        } else {
            Box::new(self.format_for_save(&editor, path.as_deref(), tidy, cx))
        };
        let later = editor.clone();
        let landed = move |outcome: &crate::documents::SaveOutcome, cx: &mut App| {
            let outcome = outcome.clone();
            handle.update(cx, |ws, window, cx| ws.saved(later, outcome, save_as, after, window, cx)).ok();
        };
        let outcome = if after == After::WithoutFormatting {
            OpenDocument::save_without_formatting(&document, path, previous, prepare, landed, cx)
        } else {
            OpenDocument::save(&document, path, previous, prepare, landed, cx)
        };
        if let Some(outcome) = outcome {
            self.saved(editor, outcome, save_as, after, window, cx);
        }
    }

    pub(crate) fn saved(
        &mut self,
        editor: Entity<Editor>,
        outcome: crate::documents::SaveOutcome,
        save_as: bool,
        after: After,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let written = match outcome {
            Ok(written) => written,
            // every view has the stamp a failed save leaves: it is the document's
            Err(e) => return self.alert(window, cx, "Could not save file", &e),
        };
        if save_as {
            editor.update(cx, |e, cx| {
                self.apply_editor_preferences(e, cx);
                cx.notify();
            });
        }
        self.index.update(cx, |i, cx| i.saved(&editor, cx));
        // the text written: typing since is not the file's
        self.apply_if_settings(&editor, &written, window, cx);
        self.check_if_pipeline(&editor, &written, window, cx);
        self.apply_if_project(&editor, &written, window, cx);
        self.customization_saved(&editor, window, cx);
        self.apply_if_editorconfig(&editor, cx);
        // edited while it was written: a close asks again rather than lose the edit
        let changed = editor.read(cx).doc(cx).unsaved();
        match after {
            After::Close | After::CloseAll | After::CloseQueue if changed => {
                self.request_close(editor, after, window, cx)
            }
            After::SaveAllThenClose if changed => self.confirm_close(window, cx),
            _ => self.after_save(editor, after, window, cx),
        }
    }

    /// The save-time cleanup of `editor`'s document for a write to `target` (its own file when
    /// `None`), with the settings as they are now. It runs just before the text is taken: for a
    /// save that waits its turn, later.
    pub(crate) fn cleanup_for_save(
        &mut self,
        editor: &Entity<Editor>,
        target: Option<&Path>,
        cx: &mut Context<Self>,
    ) -> impl FnOnce(&mut App) + 'static {
        let e = editor.read(cx);
        let path = target.or(e.doc(cx).path.as_deref());
        let (cleanup, policy, warnings) = self.cleanup_for_path(e, path, cx);
        if !warnings.is_empty() {
            self.status_message(warnings.join("; "), cx);
        }
        let editor = editor.clone();
        // through the document: every view and window follows the cleanup (`document_changed`)
        move |cx: &mut App| {
            editor.update(cx, |e, cx| {
                if e.doc(cx).edits_blocked() {
                    return;
                }
                e.update_doc(cx, |d| {
                    if let Some(ending) = policy.line_ending.filter(|_| !d.line_ending_override) {
                        d.set_line_ending(ending);
                    }
                    if let Some(bom) = policy.bom.filter(|&bom| bom != d.bom) {
                        d.bom = bom;
                        d.forget_saved_point();
                    }
                });
                e.clean_for_save(&cleanup, cx);
            });
            autosave(cx);
        }
    }

    fn apply_if_editorconfig(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        if (editor.read(cx).doc(cx).path.as_deref()).and_then(Path::file_name).is_some_and(|n| n == ".editorconfig") {
            self.apply_view(cx);
        }
    }

    /// Run the Save As requests that waited for a panel to close, until one opens a panel.
    fn next_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some((editor, ask_path, after)) = self.saves.next() {
            if self.views.find(&editor).is_some() {
                self.save_editor(editor, ask_path, after, window, cx);
                continue;
            }
            // closed while it waited: a quit or Save All carries on with the rest
            match after {
                After::SaveAllThenClose => self.save_all_then_close(window, cx),
                After::SaveNext => self.save_next(window, cx),
                _ => {}
            }
        }
    }

    /// A saved settings file, as `written`, applies to every window at once.
    fn apply_if_settings(
        &mut self,
        editor: &Entity<Editor>,
        written: &ropey::Rope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let e = editor.read(cx);
        if e.doc(cx).path.as_deref().is_some_and(settings::is_path) {
            let (text, disk) = (written.to_string(), e.doc(cx).disk);
            // the watcher must not take Orlo's own save for an outside edit
            cx.global_mut::<SettingsDisk>().0 = disk;
            self.apply_settings(&text, window, cx);
        }
    }

    /// A saved pipeline file, as `written`, is checked, and its problems shown with their
    /// lines. It stays saved: the file is the user's.
    fn check_if_pipeline(
        &mut self,
        editor: &Entity<Editor>,
        written: &ropey::Rope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let e = editor.read(cx);
        if e.doc(cx).path.as_deref().is_some_and(pipeline::is_path) {
            if let Err(errors) = pipeline::parse(&written.to_string()) {
                let detail = pipeline::problems(&errors);
                self.alert(window, cx, "Pipeline has problems", &detail);
            }
        }
    }

    /// Make `text` every window's settings; a broken file keeps the current ones and says why.
    pub(crate) fn apply_settings(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match settings::parse(text) {
            Ok((mut s, warning)) => {
                let current = cx.global::<Settings>();
                let appearance_changed = current.color_scheme != s.color_scheme || current.theme != s.theme;
                let previous = cx.global::<Settings>().font_face.clone();
                let font = check_font(&mut s, &previous, cx);
                let both: Vec<String> = font.into_iter().chain(warning).collect();
                if !both.is_empty() {
                    self.alert(window, cx, "Could not apply settings", &both.join("\n\n"));
                }
                cx.set_global(s);
                if appearance_changed {
                    self.reload_customizations(&ReloadCustomizations, window, cx);
                }
            }
            Err(e) => self.alert(window, cx, "Could not apply settings", &e),
        }
    }

    /// Orlo > Settings… > Settings: the user file in the active group. A missing file opens as
    /// the template and is written only by the first save, so closing it never prompts.
    pub(crate) fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.open_or_template(settings::path(), settings::TEMPLATE, window, cx);
    }

    fn after_save(&mut self, editor: Entity<Editor>, after: After, window: &mut Window, cx: &mut Context<Self>) {
        autosave(cx);
        match after {
            After::Nothing | After::Copy | After::WithoutFormatting => cx.notify(),
            After::Close => self.close_editor(&editor, window, cx),
            After::CloseAll => {
                // after "Don't Save" the buffer is still dirty: its other views go too, so it
                // is asked about once
                let views: Vec<_> = self
                    .views
                    .tabs()
                    .filter(|t| !self.views.is_pinned(t) && t.read(cx).same_document(editor.read(cx)))
                    .cloned()
                    .collect();
                for v in views {
                    self.close_editor(&v, window, cx);
                }
                self.close_all(&CloseAll, window, cx);
            }
            After::SaveAllThenClose => self.save_all_then_close(window, cx),
            After::SaveNext => self.save_next(window, cx),
            After::CloseQueue => {
                self.close_editor(&editor, window, cx);
                self.close_next_queued(window, cx);
            }
        }
    }

    pub(crate) fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        // cmd-w in an empty group does nothing, as in Sublime
        if let Some(editor) = self.active_editor() {
            self.request_close(editor, After::Close, window, cx);
        }
    }

    /// Clean tabs go at once; each unsaved one gets Sublime's prompt in turn. Pinned tabs stay:
    /// close them on their own.
    pub(crate) fn close_all(&mut self, _: &CloseAll, window: &mut Window, cx: &mut Context<Self>) {
        let clean: Vec<_> =
            self.views.tabs().filter(|t| !t.read(cx).doc(cx).unsaved() && !self.views.is_pinned(t)).cloned().collect();
        for t in clean {
            self.close_editor(&t, window, cx);
        }
        let dirty = self.views.tabs().find(|t| t.read(cx).doc(cx).unsaved() && !self.views.is_pinned(t)).cloned();
        if let Some(dirty) = dirty {
            self.request_close(dirty, After::CloseAll, window, cx);
        }
    }

    /// Sublime's "Save changes?" prompt for dirty tabs; a file deleted on disk, clean or not,
    /// gets its "was deleted" prompt, whose Save As… opens the Save As sheet.
    pub(crate) fn request_close(
        &mut self,
        editor: Entity<Editor>,
        after: After,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (dirty, deleted, title) = {
            let e = editor.read(cx);
            (e.doc(cx).dirty, e.doc(cx).deleted, e.title(cx))
        };
        let rx = if deleted {
            window.prompt(
                PromptLevel::Warning,
                &format!("The file {title} was deleted. Do you want to save it?"),
                Some("Your changes will be lost if you don't save them."),
                &["Save As…", "Don't Save", "Cancel"],
                cx,
            )
        } else if dirty {
            window.prompt(
                PromptLevel::Warning,
                &format!("Save changes to {title}?"),
                None,
                &["Save", "Don't Save", "Cancel"],
                cx,
            )
        } else {
            return self.after_save(editor, after, window, cx);
        };
        let document = editor.read(cx).document.clone();
        document.update(cx, |d, _| {
            d.file_autosave.cancel();
            d.file_autosave.suspensions += 1;
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = rx.await;
            document
                .update(cx, |d, _| d.file_autosave.suspensions = d.file_autosave.suspensions.saturating_sub(1))
                .ok();
            match answer {
                Ok(0) => this.update_in(cx, |ws, window, cx| ws.save_editor(editor, false, after, window, cx)).ok(),
                Ok(1) => this.update_in(cx, |ws, window, cx| ws.discard_editor(editor, after, window, cx)).ok(),
                _ => None,
            };
            cx.update(|_, cx| cx.defer(file_autosave::refresh_all)).ok();
        })
        .detach();
    }

    fn discard_editor(&mut self, editor: Entity<Editor>, after: After, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = match after {
            After::CloseAll => self
                .views
                .tabs()
                .filter(|t| !self.views.is_pinned(t) && t.read(cx).same_document(editor.read(cx)))
                .cloned()
                .collect(),
            After::Close | After::CloseQueue => vec![editor.clone()],
            _ => return self.after_save(editor, after, window, cx),
        };
        self.rescue_then(tabs, move |ws, window, cx| ws.after_save(editor, after, window, cx), window, cx);
    }

    fn discard_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving_before_close(cx) {
            return self.close_after(Documents::when_all_saved, Self::confirm_close, cx);
        }
        let tabs = self.views.tabs().cloned().collect();
        self.rescue_then(tabs, Self::finish, window, cx);
    }

    pub(crate) fn close_editor(&mut self, editor: &Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        if self.views.find(editor).is_none() {
            return;
        }
        // Native unmark commits overtype targets/virtual padding before this view disappears.
        // A surviving sibling must never inherit an orphaned provisional IME rope.
        editor.update(cx, |e, cx| {
            if !e.marked.is_empty() {
                e.unmark_text(window, cx);
            }
        });
        // the panel's preview and list were about the tabs as they were
        self.cancel_quick_panel(cx);
        // closing Find Results stops the search streaming into it, unless another view of it
        // stays open: the search streams into that one instead
        if self.search.as_ref().is_some_and(|s| s.results.as_ref() == Some(&editor.downgrade())) {
            let view = editor.read(cx);
            let other =
                self.views.tabs().find(|t| *t != editor && t.read(cx).same_document(view)).map(|t| t.downgrade());
            match (other, &mut self.search) {
                (Some(o), Some(s)) => s.results = Some(o),
                _ => self.stop_search(cx),
            }
        }
        if let Some(p) = editor.read(cx).doc(cx).path.clone() {
            let e = editor.read(cx);
            self.closed.push(ViewState {
                path: p,
                folds: e.capture_folds(cx),
                disk_encoding_hint: Some(e.doc(cx).disk_encoding),
                ..Default::default()
            });
            // Older entries keep their paths for Reopen Closed, but no fold metadata.
            let older = self.closed.len().saturating_sub(session::folds::MAX_CLOSED_VIEWS);
            if let Some(view) = older.checked_sub(1).and_then(|i| self.closed.get_mut(i)) {
                *view = ViewState { path: std::mem::take(&mut view.path), ..Default::default() };
            }
            if self.closed.len() > MAX_CLOSED {
                self.closed.remove(0);
            }
            self.refresh_menus(cx);
        }
        // the group may become empty: Sublime keeps it, empty and focused
        self.remove_view(editor, cx);
        tabs::refresh_tab_lists(cx);
        autosave(cx);
        self.focus_editor(window, cx);
        cx.notify();
    }

    pub(crate) fn revert(&mut self, _: &Revert, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let failed = |ws: &mut Self, e: std::io::Error, window: &mut Window, cx: &mut Context<Self>| {
            ws.alert(window, cx, "Could not revert file", &e.to_string())
        };
        self.reload_editor(editor, failed, window, cx);
    }

    /// File: New View into File: a second view of the active buffer, right after it.
    pub(crate) fn new_view_into_file(&mut self, _: &NewViewIntoFile, window: &mut Window, cx: &mut Context<Self>) {
        // a tab's context menu names the tab; it goes in its own group, right after it
        if let Some(active) = self.tab_target() {
            if let Some((g, i)) = self.views.find(&active) {
                self.views.select(g, i);
            }
            let view = active.update(cx, |src, cx| cx.new(|cx| src.another_view(cx)));
            self.attach(view, true, window, cx);
        }
    }

    pub(crate) fn reopen_closed_file(&mut self, _: &ReopenClosedFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.closed.pop() else { return };
        if Documents::for_path(&view.path, cx).is_some() {
            let mut state = EditorState::new();
            state.doc.path = Some(view.path.clone());
            let editor = self.new_editor(state, cx);
            sessions::restore_closed_folds(&editor, &view, cx);
            self.attach(editor, false, window, cx);
        } else if !pipelines::reading_blocked(&view.path, cx)
            && std::fs::metadata(&view.path).is_ok_and(|m| m.is_file() && m.len() <= LOAD_IN_BACKGROUND)
        {
            match Loaded::read_with_hint(&view.path, view.disk_encoding_hint)
                .map(|l| EditorState::from_loaded(&view.path, l))
            {
                Ok(state) => {
                    let editor = self.new_editor(state, cx);
                    sessions::restore_closed_folds(&editor, &view, cx);
                    self.drop_blank_untitled(cx);
                    self.attach(editor, false, window, cx);
                }
                Err(error) => self.opened(view.path, Err(error), window, cx),
            }
        } else if std::fs::metadata(&view.path).is_ok_and(|m| m.is_file() && m.len() <= ASK_ABOVE) {
            self.reopen_later(view, window, cx);
        } else {
            // Keep ordinary Open's missing-file behavior, errors and very-large-file prompt.
            self.open_path(view.path, window, cx);
        }
        autosave(cx);
    }

    pub(crate) fn open_recent(&mut self, a: &OpenRecent, window: &mut Window, cx: &mut Context<Self>) {
        let r = cx.global::<Recent>();
        let list = if a.folder { &r.folders } else { &r.files };
        if let Some(p) = list.get(a.index).cloned() {
            self.open_path(p, window, cx);
        }
    }

    /// File > Session Could Not Be Read…: the launch's alert again, while the session file
    /// still cannot be read.
    pub(crate) fn session_problem(&mut self, _: &SessionProblem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = cx.try_global::<SessionBlocked>().and_then(|b| b.0.clone()) {
            unreadable_session(&session::path(), &e, window, cx);
        }
    }

    pub(crate) fn clear_recent(&mut self, _: &ClearRecent, _: &mut Window, cx: &mut Context<Self>) {
        *cx.global_mut::<Recent>() = Recent::default();
        self.refresh_menus(cx);
    }

    pub(crate) fn new_window(&mut self, _: &NewWindow, _: &mut Window, cx: &mut Context<Self>) {
        cx.defer(|cx| {
            open_window(None, cx);
        });
    }

    pub(crate) fn refresh_folders(&mut self, _: &RefreshFolders, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.refresh();
        self.poll_disk(window, cx);
    }

    /// Sublime's hot exit: no prompts; the session keeps every unsaved buffer and the next
    /// launch brings them back (`super::quit`; main.rs saves too as the Dock quits it).
    pub(crate) fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        // after this update, when the window can be read through its handle
        cx.defer(quit);
    }

    /// Closing the window would lose text: unsaved edits or a file deleted on disk.
    pub(crate) fn has_dirty(&self, cx: &App) -> bool {
        self.views.tabs().any(|t| t.read(cx).doc(cx).unsaved())
            || self.restoring.iter().any(|view| view.view.unsaved.is_some())
    }

    /// A document of this window's being saved, if one is.
    pub(crate) fn saving(&self, cx: &App) -> Option<Entity<OpenDocument>> {
        (self.views.tabs()).map(|t| t.read(cx).document.clone()).find(|d| d.read(cx).saving())
    }

    /// Whether closing this window waits for a save: one of its documents', or, as the last
    /// window (closing it quits Orlo), any document's, a closed tab's included.
    pub(crate) fn saving_before_close(&self, cx: &App) -> bool {
        self.saving(cx).is_some() || (cx.windows().len() <= 1 && Documents::saving(cx))
    }

    /// "Saving…" until `wait` calls back, then `then`; asked to close again meanwhile, the
    /// window waits once.
    fn close_after(
        &mut self,
        wait: impl FnOnce(Box<dyn FnOnce(&mut App)>, &mut App),
        then: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        self.status_message("Saving…", cx);
        self.closing = true;
        let handle = self.window.downcast::<Workspace>();
        let again = move |cx: &mut App| {
            if let Some(h) = handle {
                h.update(cx, |ws, window, cx| {
                    ws.closing = false;
                    then(ws, window, cx)
                })
                .ok();
            }
        };
        wait(Box::new(again), cx);
    }

    /// Sublime's "Save all changes?" prompt before closing a window with unsaved tabs.
    pub(crate) fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // the close button again while it waits: one wait, one prompt
        if self.closing {
            return;
        }
        // A named recovery deferred behind a writer is text too, even before its tab exists.
        if self.restoring.iter().any(|view| view.view.unsaved.is_some()) {
            let workspace = cx.entity().downgrade();
            return self.close_after(
                move |done, cx| {
                    cx.spawn(async move |cx| {
                        while cx
                            .update(|cx| {
                                workspace.upgrade().is_some_and(|ws| {
                                    ws.read(cx).restoring.iter().any(|view| view.view.unsaved.is_some())
                                })
                            })
                            .unwrap_or(false)
                        {
                            cx.background_executor().timer(std::time::Duration::from_millis(10)).await;
                        }
                        cx.update(done).ok();
                    })
                    .detach();
                },
                Self::confirm_close,
                cx,
            );
        }
        // a save under way lands first: then the window closes, or asks about what is unsaved
        if let Some(document) = self.saving(cx) {
            let wait = move |f, cx: &mut App| OpenDocument::when_saved(&document, f, cx);
            return self.close_after(wait, Self::confirm_close, cx);
        }
        if !self.has_dirty(cx) {
            self.finish(window, cx);
            return;
        }
        let rx = window.prompt(
            PromptLevel::Warning,
            "Save all changes before closing?",
            None,
            &["Save All", "Discard All", "Cancel"],
            cx,
        );
        let documents: Vec<_> = editors::one_per_document(self.views.tabs().cloned(), cx, |_| true)
            .iter()
            .map(|e| e.read(cx).document.clone())
            .collect();
        for document in &documents {
            document.update(cx, |d, _| {
                d.file_autosave.cancel();
                d.file_autosave.suspensions += 1;
            });
        }
        cx.spawn_in(window, async move |this, cx| {
            let answer = rx.await;
            for document in documents {
                document
                    .update(cx, |d, _| d.file_autosave.suspensions = d.file_autosave.suspensions.saturating_sub(1))
                    .ok();
            }
            match answer {
                Ok(0) => {
                    this.update_in(cx, |ws, window, cx| ws.save_all_then_close(window, cx)).ok();
                }
                Ok(1) => {
                    this.update_in(cx, |ws, window, cx| ws.discard_window(window, cx)).ok();
                }
                _ => {}
            }
            cx.update(|_, cx| cx.defer(file_autosave::refresh_all)).ok();
        })
        .detach();
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // the last window quits Orlo as it goes, which would cut off a save under way (a closed
        // tab's, or one Save All started): it goes once every save has landed
        if cx.windows().len() <= 1 && Documents::saving(cx) {
            return self.close_after(Documents::when_all_saved, Self::finish, cx);
        }
        // what was not saved was discarded: the workspace keeps the files and the layout
        if self.project.is_some() {
            if let Err(e) = self.save_workspace(window, false, cx) {
                eprintln!("Orlo: could not save the workspace: {e}");
            }
        }
        self.save_if_last(window, cx);
        // Visible ownership ends when the window closes, even if an outstanding operation
        // retains its Workspace. Reuse removal so shared follow workers stop at the last tab;
        // the release hook sees an empty list and cannot decrement ownership twice.
        let tabs: Vec<_> = self.views.tabs().cloned().collect();
        for editor in tabs {
            editor.update(cx, |e, cx| {
                if !e.marked.is_empty() {
                    e.unmark_text(window, cx);
                }
            });
            self.remove_view(&editor, cx);
        }
        self.close_terminals(cx);
        window.remove_window();
    }

    /// Saves unsaved tabs (dirty, or deleted on disk) one at a time (each may prompt for a
    /// path), then closes the window.
    fn save_all_then_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.views.tabs().find(|t| t.read(cx).doc(cx).unsaved()).cloned();
        if let Some(editor) = dirty {
            return self.save_editor(editor, false, After::SaveAllThenClose, window, cx);
        }
        self.finish(window, cx);
    }
}

/// A path panel's options: files, folders or both, one or `multiple`.
pub(crate) fn picker(files: bool, directories: bool, multiple: bool) -> PathPromptOptions {
    PathPromptOptions { files, directories, multiple, prompt: None }
}

/// Save panels one at a time: gpui's is not modal, so a request made while one is up waits for
/// it to close, and each waits once.
pub(crate) struct SaveQueue<T> {
    current: Option<T>,
    waiting: VecDeque<T>,
}

impl<T> Default for SaveQueue<T> {
    fn default() -> Self {
        Self { current: None, waiting: VecDeque::new() }
    }
}

impl<T: PartialEq> SaveQueue<T> {
    /// True when `req` may open its panel now; otherwise it waits, unless it is the one up or
    /// waiting already.
    pub(crate) fn request(&mut self, req: T) -> bool {
        if self.current.is_none() {
            self.current = Some(req);
            return true;
        }
        if self.current.as_ref() != Some(&req) && !self.waiting.contains(&req) {
            self.waiting.push_back(req);
        }
        false
    }

    pub(crate) fn closed(&mut self) {
        self.current = None;
    }

    /// The next waiting request, once no panel is up.
    pub(crate) fn next(&mut self) -> Option<T> {
        match self.current {
            Some(_) => None,
            None => self.waiting.pop_front(),
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.current.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{settle_until, temp, type_in, window};

    #[gpui::test]
    fn discarded_untitled_confirmation_persists_unicode(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "救助 🦀 e\u{301}\n");
        ws.update_in(cx, |ws, window, cx| ws.close_tab(&CloseTab, window, cx));
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.views.tabs().next().is_none()));
        let path = session::support_dir().join("DiscardedUntitled");
        assert!(path.is_dir(), "confirmed discard must retain a private rescue");
    }

    #[gpui::test]
    fn opening_a_file_keeps_a_pinned_blank_tab(cx: &mut gpui::TestAppContext) {
        let path = temp("orlo-open-pinned-blank").join("file.txt");
        std::fs::write(&path, "opened").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            let pinned = ws.active_editor().unwrap();
            ws.views.set_pinned(&pinned, true);
            ws.open_path(path, window, cx);
            assert_eq!(ws.views.tabs().count(), 2);
            assert!(ws.views.find(&pinned).is_some());
            assert!(ws.views.is_pinned(&pinned));
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "opened");
        });
    }

    #[gpui::test]
    fn adopting_a_view_keeps_a_pinned_blank_tab(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            let pinned = ws.active_editor().unwrap();
            ws.views.set_pinned(&pinned, true);
            let incoming = ws.new_editor(EditorState::from_text("incoming"), cx);
            ws.adopt_views(vec![(incoming.clone(), false)], 0, 1, Some(incoming), window, cx);
            assert_eq!(ws.views.tabs().count(), 2);
            assert!(ws.views.find(&pinned).is_some());
            assert!(ws.views.is_pinned(&pinned));
        });
    }

    #[gpui::test]
    fn the_closed_file_history_is_capped(cx: &mut gpui::TestAppContext) {
        let dir = temp("orlo-closed-history").canonicalize().unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            for i in 0..MAX_CLOSED + 5 {
                let path = dir.join(format!("{i}.txt"));
                std::fs::write(&path, "").unwrap();
                ws.open_path(path, window, cx);
                ws.close_editor(&ws.active_editor().unwrap(), window, cx);
            }
            assert_eq!(ws.closed.len(), MAX_CLOSED);
            assert_eq!(ws.closed.first().map(|v| &v.path), Some(&dir.join("5.txt")));
            assert_eq!(ws.closed.last().map(|v| &v.path), Some(&dir.join(format!("{}.txt", MAX_CLOSED + 4))));
        });
    }

    #[gpui::test]
    fn close_all_keeps_a_pinned_view_of_an_unsaved_shared_document(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "keep this draft");
        let pinned = ws.update_in(cx, |ws, window, cx| {
            let pinned = ws.active_editor().unwrap();
            ws.views.set_pinned(&pinned, true);
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            assert_eq!(ws.views.tabs().count(), 2);
            ws.close_all(&CloseAll, window, cx);
            pinned
        });
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        assert!(session::discarded::Store::current().list().unwrap().1.is_empty());
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 1);
            assert!(ws.views.is_pinned(&pinned));
            assert!(ws.views.find(&pinned).is_some());
            assert_eq!(pinned.read(cx).text(cx), "keep this draft");
            assert!(pinned.read(cx).doc(cx).dirty);
        });
    }

    #[gpui::test]
    fn save_as_uses_an_open_destinations_bomless_disk_encoding(cx: &mut gpui::TestAppContext) {
        use crate::editing::encoding::Encoding;
        let (ws, cx) = window(cx);
        let dir = temp("g66-save-as").canonicalize().unwrap();
        let target = dir.join("target.txt");
        let source = dir.join("source.txt");
        let old = crate::editing::encoding::encode("old café 🦀\n", Encoding::Utf16Le, false).unwrap();
        std::fs::write(&target, &old).unwrap();
        std::fs::write(&source, "replacement\n").unwrap();
        let history = dir.join("history");
        crate::files::file_history::with_root(history.clone(), || {
            ws.update_in(cx, |ws, window, cx| {
                let state =
                    EditorState::from_loaded(&target, Loaded::read_encoding(&target, Some(Encoding::Utf16Le)).unwrap());
                let e = ws.new_editor(state, cx);
                ws.attach(e, false, window, cx);
                ws.open_path(source.clone(), window, cx);
                ws.active_editor().unwrap().update(cx, |e, cx| e.update_doc(cx, |d| d.history_enabled = true));
                ws.save_as(&SaveAs, window, cx);
            });
            cx.simulate_new_path_selection(|_| Some(target.clone()));
            settle_until(cx, |_| std::fs::read(&target).unwrap() == b"replacement\n");
        });
        let versions = crate::files::file_history::list_in(&history, &target).unwrap();
        assert_eq!(versions.len(), 1);
        let record = crate::files::file_history::read_in(&history, &target, &versions[0].id).unwrap();
        assert_eq!((record.bytes, record.version.encoding), (old, Encoding::Utf16Le));
    }

    #[test]
    fn save_panels_wait_their_turn_once_each() {
        let mut q = SaveQueue::default();
        assert!(q.request(("a", 1)), "nothing up: it opens");
        assert!(!q.request(("a", 1)), "the one up again: dropped");
        assert!(!q.request(("b", 2)), "another waits");
        assert!(!q.request(("b", 2)), "once");
        assert!(!q.request(("a", 3)), "the same file with another follow-up waits too");
        assert_eq!(q.next(), None, "not while a panel is up");
        q.closed();
        assert_eq!(q.next(), Some(("b", 2)));
        assert!(q.request(("b", 2)), "its panel opens");
        assert_eq!(q.next(), None);
        q.closed();
        assert_eq!(q.next(), Some(("a", 3)));
        assert_eq!(q.next(), None, "it saved without a panel: the queue is empty");
        assert!(!q.busy());
    }
}
