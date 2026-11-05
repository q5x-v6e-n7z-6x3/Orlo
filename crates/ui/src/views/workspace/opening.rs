use super::explorer::BreadcrumbOpen;
use super::*;

impl Workspace {
    /// Goto Anything and side bar clicks open in the active group, as Sublime does: a view
    /// already there is focused, one in another group gets a second view here.
    pub fn open_path_here(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = crate::editing::text::resolve(&path);
        self.open_path_here_checked(path, None, window, cx);
    }

    pub(super) fn open_breadcrumb_file(&mut self, guard: BreadcrumbOpen, window: &mut Window, cx: &mut Context<Self>) {
        if !guard.current(self, window, cx) {
            return;
        }
        if pipelines::reading_blocked(&guard.target().path, cx) {
            return self.status_message("Path Breadcrumbs: file is being written; try navigation again", cx);
        }
        let target = guard.target().clone();
        let task = cx.background_spawn(async move {
            target.validate()?;
            if !std::fs::metadata(&target.path)?.is_file() {
                return Err(std::io::Error::other("Not a regular file"));
            }
            let path = target.path.canonicalize()?;
            target.validate()?;
            Ok::<_, std::io::Error>(path)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, window, cx| {
                if !guard.current(ws, window, cx) {
                    return;
                }
                match result {
                    Ok(path) => ws.open_path_here_checked(path, Some(guard), window, cx),
                    Err(error) => ws.status_message(format!("Path Breadcrumbs: {error}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_path_here_checked(
        &mut self,
        path: PathBuf,
        guard: Option<BreadcrumbOpen>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(guard) = &guard {
            if !guard.current(self, window, cx) {
                return;
            }
            if pipelines::reading_blocked(&path, cx) {
                return self.status_message("Path Breadcrumbs: file is being written; try navigation again", cx);
            }
        }
        let g = self.views.active;
        let here =
            (self.views.groups[g].tabs.iter()).position(|t| t.read(cx).doc(cx).path.as_deref() == Some(path.as_path()));
        if let Some(i) = here {
            if let Some(guard) = &guard {
                guard.finish(self, window, cx);
            }
            self.views.select(g, i);
            self.focus_editor(window, cx);
            self.refresh_matches(cx);
            cx.notify();
            return;
        }
        match self.tab_for_path(&path, cx) {
            Some(other) => {
                if let Some(guard) = &guard {
                    guard.finish(self, window, cx);
                }
                self.drop_blank_untitled(cx);
                let view = other.update(cx, |src, cx| cx.new(|cx| src.another_view(cx)));
                self.attach(view, false, window, cx);
            }
            None => self.open_path_checked(path, guard, window, cx),
        }
    }

    /// A project or workspace file from Finder, the Dock or `Orlo <path>` opens as a project;
    /// anything else as `open_path` does. File > Open… opens one as text.
    pub fn open_path_or_project(&mut self, p: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        use crate::config::project::{PROJECT_EXT, WORKSPACE_EXT};
        match p.extension().and_then(|e| e.to_str()) {
            Some(PROJECT_EXT | WORKSPACE_EXT) => {
                let a = OpenProjectPath { path: p, here: false };
                self.open_project_path(&a, window, cx)
            }
            _ => self.open_path(p, window, cx),
        }
    }

    /// A file opens in a tab (reusing an existing one, or a blank untitled); a directory becomes the folder.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // one tab per file, however the path was spelled
        let path = crate::editing::text::resolve(&path);
        self.open_path_checked(path, None, window, cx);
    }

    fn open_path_checked(
        &mut self,
        path: PathBuf,
        guard: Option<BreadcrumbOpen>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if guard.as_ref().is_some_and(|g| !g.current(self, window, cx)) {
            return;
        }
        self.revive_pending_open(&path);
        if pipelines::reading_blocked(&path, cx) {
            if guard.is_some() {
                return self.status_message("Path Breadcrumbs: file is being written; try navigation again", cx);
            }
            if !self.opening_after_write.insert(path.clone()) {
                return;
            }
            let gen = self.restore_gen;
            let preview = self.previewing.contains_key(&path);
            cx.spawn_in(window, async move |this, cx| {
                pipelines::wait_for_write(&path, cx).await;
                this.update_in(cx, |ws, window, cx| {
                    if ws.restore_gen != gen {
                        return;
                    }
                    ws.opening_after_write.remove(&path);
                    if preview && !ws.pending_sidebar_open(&path) {
                        ws.previewing.remove(&path);
                        return;
                    }
                    ws.open_after_write(path, window, cx);
                })
                .ok();
            })
            .detach();
            return;
        }
        if path.is_dir() {
            if guard.is_some() {
                return self.status_message("Path Breadcrumbs: file was replaced by a folder; browse it again", cx);
            }
            self.previewing.remove(&path);
            return self.add_root(path, window, cx);
        }
        if let Some((g, i)) = (self.tab_for_path(&path, cx)).and_then(|e| self.views.find(&e)) {
            if let Some(guard) = &guard {
                guard.finish(self, window, cx);
            }
            self.views.select(g, i);
            self.focus_editor(window, cx);
            self.finish_pending_jump(&path, cx);
            self.finish_cli_open(&path, None, cx);
            self.refresh_matches(cx);
            cx.notify();
            return;
        }
        // open in another window: another view of that document, not a second copy of the file
        if Documents::for_path(&path, cx).is_some() {
            if let Some(guard) = &guard {
                guard.finish(self, window, cx);
            }
            self.cancel_quick_panel(cx);
            return self.opened_unread(path, window, cx);
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(error) if guard.is_some() => return self.status_message(format!("Path Breadcrumbs: {error}"), cx),
            // `subl missing.txt` opens an empty buffer of that name; saving creates the file
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return self.opened_unread(path, window, cx),
            Err(e) => return self.opened(path, Err(e), window, cx),
        };
        // /dev/zero would fill memory and a named pipe would hang the window
        if !meta.is_file() {
            if guard.is_some() {
                return self.status_message("Path Breadcrumbs: target is no longer a regular file", cx);
            }
            self.previewing.remove(&path);
            let detail = format!("{} is not a regular file.", path.display());
            self.finish_cli_open(&path, Some(detail.clone()), cx);
            return self.alert(window, cx, "Could not open file", &detail);
        }
        if meta.len() > ASK_ABOVE {
            let gb = meta.len() as f64 / (1u64 << 30) as f64;
            let detail = format!(
                "{} is {gb:.1} GB. Opening it takes about {:.0} GB of memory.",
                path.display(),
                (gb * 2.).ceil()
            );
            let answer =
                window.prompt(PromptLevel::Warning, "Open a very large file?", Some(&detail), &["Open", "Cancel"], cx);
            let gen = self.restore_gen;
            let preview = self.previewing.contains_key(&path);
            cx.spawn_in(window, async move |this, cx| {
                let open = answer.await == Ok(0);
                this.update_in(cx, |ws, window, cx| {
                    if ws.restore_gen == gen && (!preview || ws.pending_sidebar_open(&path)) {
                        if guard.as_ref().is_some_and(|g| !g.current(ws, window, cx)) {
                            return;
                        }
                        if open {
                            ws.read_in_background_checked(path, guard, window, cx);
                        } else {
                            ws.previewing.remove(&path);
                            ws.finish_cli_open(&path, Some(format!("opening {} was cancelled", path.display())), cx);
                        }
                    }
                })
                .ok();
            })
            .detach();
            return;
        }
        // a big file is read in the background so the window keeps drawing; a small one at
        // once, so keys typed right after the click land in its tab
        if guard.is_some() || meta.len() > LOAD_IN_BACKGROUND {
            return self.read_in_background_checked(path, guard, window, cx);
        }
        // the file's tab takes the keys from Goto Anything or the palette
        self.cancel_quick_panel(cx);
        let state = Loaded::read_with_hint(&path, self.remembered_disk_encoding(&path))
            .map(|loaded| EditorState::from_loaded(&path, loaded));
        self.opened(path, state, window, cx);
    }

    pub(super) fn remembered_disk_encoding(&self, path: &Path) -> Option<crate::editing::encoding::Encoding> {
        self.closed.iter().rev().find(|view| view.path == path).and_then(|view| view.disk_encoding_hint)
    }

    /// A view of `path` with no text read: another of a document open elsewhere, or a new file.
    fn opened_unread(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let mut state = EditorState::new();
        state.doc.path = Some(path.clone());
        self.opened(path, Ok(state), window, cx)
    }

    /// A results-browser jump to `path` that the user has since left.
    fn browser_jump_superseded(&self, path: &Path, window: &Window, cx: &App) -> bool {
        (self.pending_jumps.get(path)).is_some_and(|j| j.browser.is_some())
            && !self.browser_jump_is_current(path, window, cx)
    }

    // A small leased file can open synchronously; the browser still owns navigation and keys.
    pub(crate) fn open_after_write(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let browser = (self.pending_jumps.get(&path)).is_some_and(|j| j.browser.is_some());
        let focused = window.focused(cx);
        let active = (self.active_editor()).filter(|_| self.browser_jump_superseded(&path, window, cx));
        self.open_path(path, window, cx);
        if browser {
            if let Some((g, i)) = active.and_then(|e| self.views.find(&e)) {
                self.views.select(g, i);
            }
            if let Some(focus) = focused {
                window.focus(&focus);
            }
        }
    }

    pub(crate) fn browser_jump_is_current(&self, path: &Path, window: &Window, cx: &App) -> bool {
        self.pending_jumps.get(path).and_then(|jump| jump.browser.as_ref()).is_some_and(|jump| {
            jump.origin == self.active_editor().map(|e| e.entity_id())
                && window.focused(cx).as_ref() == Some(&jump.focus)
                && self.results_panel().is_some_and(|p| p.focus == jump.focus && p.navigation == jump.navigation)
        })
    }

    pub(crate) fn read_in_background(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.read_in_background_checked(path, None, window, cx);
    }

    fn read_in_background_checked(
        &mut self,
        path: PathBuf,
        guard: Option<BreadcrumbOpen>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if guard.as_ref().is_some_and(|g| !g.current(self, window, cx)) {
            return;
        }
        self.revive_pending_open(&path);
        let gen = self.restore_gen;
        let preview = self.previewing.contains_key(&path);
        if pipelines::reading_blocked(&path, cx) {
            if guard.is_some() {
                return self.status_message("Path Breadcrumbs: file is being written; try navigation again", cx);
            }
            cx.spawn_in(window, async move |this, cx| {
                pipelines::wait_for_write(&path, cx).await;
                this.update_in(cx, |ws, window, cx| {
                    if ws.restore_gen == gen && (!preview || ws.pending_sidebar_open(&path)) {
                        ws.read_in_background(path, window, cx);
                    }
                })
                .ok();
            })
            .detach();
            return;
        }
        if !self.loading.insert(path.clone()) {
            if guard.is_some() {
                self.status_message("Path Breadcrumbs: file is already being read; try navigation again", cx);
            }
            return;
        }
        cx.notify();
        let p = path.clone();
        let focused = window.focused(cx);
        let encoding = self.remembered_disk_encoding(&path);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                if guard.is_none() {
                    pipelines::wait_for_write(&path, cx).await;
                }
                let observed = if let Some(guard) = &guard {
                    this.update_in(cx, |ws, window, cx| {
                        if !guard.current(ws, window, cx) {
                            ws.loading.remove(&path);
                            return None;
                        }
                        if pipelines::reading_blocked(&path, cx) {
                            ws.loading.remove(&path);
                            ws.status_message("Path Breadcrumbs: file is being written; try navigation again", cx);
                            return None;
                        }
                        Some(pipelines::observe_read(&path, cx))
                    })
                    .ok()
                    .flatten()
                } else {
                    cx.update(|_, cx| pipelines::observe_read(&path, cx)).ok()
                };
                let Some(observed) = observed else { return };
                let p = p.clone();
                let target = guard.as_ref().map(|g| g.target().clone());
                let read = cx
                    .background_spawn(async move {
                        match target {
                            Some(target) => target.read_file(encoding),
                            None => Loaded::read_with_hint(&p, encoding),
                        }
                    })
                    .await;
                #[cfg(test)]
                crate::testing::gate::wait("open-read", &path).await;
                // the grammar compiles its patterns on first use: on the highlighting thread, not
                // in a frame, and the lines it coloured go to the new view
                let warm = match &read {
                    Ok(l) => {
                        let (p, rope, first) = (path.clone(), l.rope.clone(), editor::first_line(&l.rope));
                        let h = Highlighter::detect(Some(&p), &first);
                        // ahead of other views' jobs: the file shows coloured as it opens
                        crate::languages::highlight::on_worker_urgent(move || {
                            crate::languages::highlight::run_job(h, &rope, &[50])
                        })
                        .await
                    }
                    Err(_) => None,
                };
                let target_current = match guard.as_ref().map(|g| g.target().clone()) {
                    Some(target) => cx.background_spawn(async move { target.validate() }).await,
                    None => Ok(()),
                };
                let retry = this
                    .update_in(cx, |ws, window, cx| {
                        if ws.restore_gen != gen {
                            return false;
                        }
                        if let Some(guard) = &guard {
                            ws.loading.remove(&path);
                            if !guard.current(ws, window, cx) {
                                return false;
                            }
                            let unchanged = read.as_ref().is_ok_and(|loaded| {
                                loaded.disk.is_some() && loaded.disk == crate::files::watch::stamp(&path)
                            });
                            if target_current.is_err()
                                || !unchanged
                                || pipelines::reading_blocked(&path, cx)
                                || observed.changed()
                            {
                                let why = read
                                    .as_ref()
                                    .err()
                                    .map(|e| e.to_string())
                                    .unwrap_or_else(|| "File changed or is being written; browse it again".into());
                                ws.status_message(format!("Path Breadcrumbs: {why}"), cx);
                                return false;
                            }
                            guard.finish(ws, window, cx);
                            // A document opened or edited during the read keeps its live owner.
                            if ws.tab_for_path(&path, cx).is_some() || Documents::for_path(&path, cx).is_some() {
                                ws.open_path_here(path.clone(), window, cx);
                                return false;
                            }
                        } else if pipelines::reading_blocked(&path, cx) || observed.changed() {
                            return true;
                        }
                        ws.loading.remove(&path);
                        if preview && !ws.pending_sidebar_open(&path) {
                            ws.previewing.remove(&path);
                            return false;
                        }
                        let state = read.map(|l| EditorState::from_loaded(&path, l));
                        // Unrelated loading must leave the foreground document and keys alone.
                        let now = window.focused(cx);
                        let was_view = now.as_ref().is_some_and(|f| ws.is_view_focus(f, cx));
                        let busy =
                            guard.is_none() && (ws.panel.is_some() || ws.quick_panel.is_some() || now != focused);
                        // Browser navigation changes the visible source while retaining the list's keys.
                        let show_result = ws.browser_jump_is_current(&path, window, cx);
                        let active = ws.active_editor().filter(|_| busy && !show_result);
                        ws.opened(path.clone(), state, window, cx);
                        let new = ws.editor_for_path(&path, cx);
                        if show_result {
                            if let Some((group, tab)) = new.as_ref().and_then(|e| ws.views.find(e)) {
                                ws.views.select(group, tab);
                                cx.notify();
                            }
                        }
                        if let (Some(e), Some(warm)) = (&new, warm) {
                            e.update(cx, |e, cx| e.highlighter.adopt(warm, &doc_of(&e.document, cx).rope));
                        }
                        // the keys stay where they were, even when the tab they were in (a blank
                        // untitled one) gave way to the file
                        if busy {
                            if let Some(at) = active.and_then(|a| ws.views.find(&a)) {
                                ws.views.select(at.0, at.1);
                            }
                            // a blank untitled tab that had the keys and gave way: they go to the editor
                            match now.filter(|f| !was_view || ws.is_view_focus(f, cx)) {
                                Some(f) => window.focus(&f),
                                None => ws.focus_editor(window, cx),
                            }
                        }
                        false
                    })
                    .unwrap_or(false);
                if !retry {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn opened(
        &mut self,
        path: PathBuf,
        state: std::io::Result<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let error = state.as_ref().err().map(|e| format!("{}: {e}", path.display()));
        match state {
            // opened another way (Find Results, a session) while it was read: keep that one
            Ok(_) if self.tab_for_path(&path, cx).is_some() => {}
            Ok(state) => {
                if !self.browser_jump_superseded(&path, window, cx) {
                    self.drop_blank_untitled(cx);
                }
                push_recent(&mut cx.global_mut::<Recent>().files, path.clone());
                self.refresh_menus(cx);
                self.add_tab(state, window, cx);
            }
            Err(_) => {
                let detail = error.as_deref().unwrap_or_default();
                self.alert(window, cx, "Could not open file", detail)
            }
        }
        if self.previewing.remove(&path) == Some(PendingOpen::Preview) {
            if let Some(editor) = self.tab_for_path(&path, cx) {
                self.finish_preview(editor, window, cx);
            }
        }
        self.finish_pending_jump(&path, cx);
        self.finish_cli_open(&path, error, cx);
    }

    pub(crate) fn finish_pending_jump(&mut self, path: &Path, cx: &mut Context<Self>) {
        let jump = self.pending_jumps.remove(path);
        let Some((jump, editor)) = jump.zip(self.editor_for_path(path, cx)) else {
            return;
        };
        let selection = jump.selection.filter(|(range, known)| {
            let state = editor.read(cx).read_state(cx);
            !state.doc.dirty
                && state.doc.disk == Some(*known)
                && crate::files::watch::stamp(path) == Some(*known)
                && range.end <= state.len()
        });
        match selection {
            Some((range, _)) => editor.update(cx, |e, cx| e.select_range(range, cx)),
            None => find::jump_to_result(&editor, jump.line, &jump.hit, cx),
        }
    }
}
