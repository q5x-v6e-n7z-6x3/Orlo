//! Named projects and their workspaces: open, switch, save, close and edit a project;
//! the project's settings between the settings file and the window's own choices.

use super::*;
use crate::config::project::{self, Project, ProjectSettings, Root, WorkspaceFile};
use gpui::EntityInputHandler;

impl Workspace {
    /// The project's settings, empty without one.
    pub(crate) fn project_settings(&self) -> ProjectSettings {
        (self.project.as_ref()).map(|p| p.settings.clone()).unwrap_or_default()
    }

    /// A new buffer's tab size and indentation: the project's, else the settings file's.
    pub(crate) fn default_indent(&self, cx: &App) -> (usize, bool) {
        let s = cx.global::<Settings>();
        let p = self.project_settings();
        (p.tab_size.unwrap_or(s.tab_size), p.translate_tabs_to_spaces.unwrap_or(s.translate_tabs_to_spaces))
    }

    /// A session's window names its project: read it again, so a folder added to the file
    /// elsewhere shows. A file that cannot be read keeps the session's folders and the name,
    /// and is never written over (it may be on a drive not mounted).
    pub(crate) fn restore_project(&mut self, st: &mut WindowState, cx: &mut Context<Self>) {
        self.workspace_file = st.workspace.clone();
        let Some(path) = st.project.clone() else {
            self.project = None;
            return;
        };
        match Project::load(&path) {
            Ok((p, _)) => {
                st.folders = p.roots.clone();
                self.project = Some(p);
            }
            Err(e) => {
                eprintln!("Orlo: could not read the project {}: {e}", path.display());
                let mut p = Project::new(path, st.folders.clone());
                p.unread = true;
                self.project = Some(p);
                self.status_message(format!("The project file could not be read: {e}"), cx);
            }
        }
    }

    /// The project file follows the roots (added, removed, reordered); a write that fails
    /// says so and the roots stay as they are in the window.
    pub(crate) fn save_project_roots(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        p.roots = self.roots.clone();
        if p.unread {
            return;
        }
        if pipelines::writing(&crate::editing::text::resolve(&p.path), cx) {
            return self.status_message("The project is being written; save it again after the operation finishes", cx);
        }
        if let Err(e) = p.save() {
            let detail = format!("{}: {e}", p.path.display());
            self.alert(window, cx, "Could not save the project", &detail);
        }
    }

    /// The window as its workspace file keeps it: unsaved and untitled text inline (`hot`).
    fn workspace_state(&self, window: &Window, hot: bool, cx: &App) -> WindowState {
        let mut texts = Texts::new();
        let mut st = self.capture(window, hot, &mut texts, cx);
        let views = st.groups.iter_mut().flat_map(|g| &mut g.views);
        for u in views.filter_map(|v| v.unsaved.as_mut()) {
            let Some(name) = u.file.take() else { continue };
            if let Some((_, rope)) = texts.iter().find(|(n, _)| *n == name) {
                u.text = rope.to_string();
            }
        }
        // the window's place on screen is the session's, not the project's
        (st.bounds, st.display, st.maximized) = (None, None, false);
        st
    }

    /// Write the window's workspace file (with unsaved text when `hot`). An error when the
    /// window has no project or the write failed.
    pub(crate) fn save_workspace(&self, window: &Window, hot: bool, cx: &App) -> Result<(), String> {
        let Some(p) = &self.project else {
            return Err("This window has no project".into());
        };
        let ws_path = (self.workspace_file.clone()).unwrap_or_else(|| p.default_workspace());
        if pipelines::writing(&crate::editing::text::resolve(&ws_path), cx) {
            return Err("The workspace is being written; try again after the operation finishes".into());
        }
        let st = self.workspace_state(window, hot, cx);
        WorkspaceFile::save(&ws_path, &p.path, st).map_err(|e| format!("{}: {e}", ws_path.display()))
    }

    /// Nothing in the window worth keeping: no project, no folder, no text.
    fn is_empty_window(&self, cx: &App) -> bool {
        self.project.is_none()
            && self.roots.is_empty()
            && self.restoring.iter().all(|view| view.view.unsaved.is_none())
            && self.views.tabs().all(|t| Self::is_blank_untitled(t, cx))
    }

    /// The window showing the project at `path` (this one included), if any.
    fn window_of_project(&self, path: &Path, cx: &App) -> Option<gpui::AnyWindowHandle> {
        let same = |ws: &Workspace| (ws.project.as_ref()).is_some_and(|p| project::same_path(&p.path, path));
        if same(self) {
            return Some(self.window);
        }
        self.other_workspaces(cx).find(|ws| same(ws)).map(|ws| ws.window)
    }

    /// Project > Open Project…: a `.sublime-project` or `.orlo-workspace` file.
    pub(crate) fn open_project(&mut self, _: &OpenProject, window: &mut Window, cx: &mut Context<Self>) {
        let options = PathPromptOptions { prompt: Some("Open Project".into()), ..files::picker(true, false, false) };
        self.prompt_path(options, window, cx, |ws, path, window, cx| {
            ws.open_project_path(&OpenProjectPath { path, here: false }, window, cx)
        });
    }

    /// Project > Switch Project…: the recent projects and workspaces, the chosen one replacing
    /// this window's project.
    pub(crate) fn switch_project(&mut self, _: &SwitchProject, window: &mut Window, cx: &mut Context<Self>) {
        let recent = cx.global::<Recent>().projects.clone();
        let mut items: Vec<quick_panel::Item> = (recent.iter())
            .map(|path| {
                quick_panel::Item::command(
                    project_label(path),
                    tilde(path),
                    OpenProjectPath { path: path.clone(), here: true },
                )
            })
            .collect();
        items.push(quick_panel::Item::command("Open Project…", "", OpenProject));
        self.open_quick_panel(Kind::Commands, items, window, cx);
    }

    /// Open a project or workspace file: the window that has it comes to the front; else it
    /// replaces this window's project (`here`, or this window is empty), or opens in a new
    /// window. Nothing changes when a file cannot be read.
    pub(crate) fn open_project_path(&mut self, a: &OpenProjectPath, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.try_open_project_path(a, window, cx) {
            self.alert(window, cx, "Could not open the project", &error);
        }
    }

    pub(crate) fn try_open_project_path(
        &mut self,
        a: &OpenProjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let path = crate::editing::text::resolve(&a.path);
        let (project_path, ws_path, state) = read_project_files(&path)?;
        let (project, warnings) = Project::load(&project_path)?;
        push_recent(&mut cx.global_mut::<Recent>().projects, path.clone());
        if let Some(w) = self.window_of_project(&project_path, cx) {
            if w != self.window {
                w.update(cx, |_, window, _| window.activate_window()).ok();
            }
            return Ok(());
        }
        let here = (a.here && self.project.is_some()) || self.is_empty_window(cx);
        if !here {
            let new = open_window(None, cx);
            let a = a.clone();
            cx.defer(move |cx| {
                new.update(cx, |ws, window, cx| ws.open_project_path(&a, window, cx)).ok();
            });
            return Ok(());
        }
        // the project left behind keeps this window's views, unsaved text included
        self.settle_compositions(window, cx);
        if self.project.is_some() {
            (self.save_workspace(window, true, cx)).map_err(|e| format!("{e}\n\nThe window keeps its project."))?;
        }
        self.enter_project(project, ws_path, state.unwrap_or_default(), window, cx);
        if !warnings.is_empty() {
            self.alert(window, cx, "Some project settings are ignored", &warnings.join("\n"));
        }
        Ok(())
    }

    /// Replace the window's contents with `project` and its workspace state `st`.
    fn enter_project(
        &mut self,
        project: Project,
        ws_path: PathBuf,
        mut st: WindowState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_window(window, cx);
        st.folders = project.roots.clone();
        st.project = Some(project.path.clone());
        st.workspace = Some(ws_path);
        // `restore` reads the project file again: what was just read is what it finds
        self.restore(st, window, cx);
        self.apply_view(cx);
        autosave(cx);
    }

    /// Actual removal/quit commits through the native input handler. Move-only remove_view
    /// deliberately does not call this, retaining the entity's live composition.
    pub(crate) fn settle_compositions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for editor in self.views.tabs().cloned().collect::<Vec<_>>() {
            editor.update(cx, |e, cx| {
                if !e.marked.is_empty() {
                    e.unmark_text(window, cx);
                }
            });
        }
    }

    /// Every view out of the window without asking (their text is in the workspace file just
    /// written), the roots and the project gone: a fresh window's state.
    pub(crate) fn clear_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_compositions(window, cx);
        self.exit_distraction_free(true, window, cx);
        self.cancel_quick_panel(cx);
        let all: Vec<_> = self.views.tabs().cloned().collect();
        for t in &all {
            self.remove_view(t, cx);
        }
        self.cancel_comparison();
        self.restore_gen += 1;
        self.git = Default::default();
        self.close_terminals(cx);
        if matches!(
            self.panel,
            Some(Panel::Terminal(_) | Panel::Changes(_) | Panel::Comparison(_) | Panel::History(_) | Panel::Rescue(_))
        ) {
            self.panel = None;
        }
        self.restoring.clear();
        self.loading.clear();
        self.opening_after_write.clear();
        self.previewing.clear();
        self.pending_jumps.clear();
        self.unavailable.clear();
        self.roots.clear();
        self.branches.clear();
        self.project = None;
        self.workspace_file = None;
        self.sidebar.locations = Default::default();
        self.sidebar.location_request += 1;
        self.sidebar.selected = None;
        self.sidebar.marked.clear();
    }

    /// Project > Close Project: its workspace (unsaved text included) is written, then the
    /// window is emptied. A failed write leaves it as it is.
    pub(crate) fn close_project(&mut self, _: &CloseProject, window: &mut Window, cx: &mut Context<Self>) {
        if self.project.is_none() {
            return self.status_message("This window has no project", cx);
        }
        self.settle_compositions(window, cx);
        if let Err(e) = self.save_workspace(window, true, cx) {
            let detail = format!("{e}\n\nThe project stays open.");
            return self.alert(window, cx, "Could not save the workspace", &detail);
        }
        self.clear_window(window, cx);
        self.sidebar.set_roots(&self.roots);
        self.views.set_layout(crate::chrome::layout::Layout::default());
        self.add_tab(EditorState::new(), window, cx);
        self.apply_view(cx);
        self.roots_changed(window, cx);
    }

    /// Project > Save Project As…: the window's roots (and settings, and the keys of the
    /// project it had) in a new project file, with a workspace beside it; the window has that
    /// project from then on.
    pub(crate) fn save_project_as(&mut self, _: &SaveProjectAs, window: &mut Window, cx: &mut Context<Self>) {
        let dir = (self.roots.first().map(|r| r.path.clone()))
            .or_else(|| self.last_dir.clone())
            .or_else(std::env::home_dir)
            .unwrap_or_default();
        let name = match &self.project {
            Some(p) => p.name(),
            None => self.roots.first().map_or("Untitled".into(), |r| r.label()),
        };
        let name = format!("{name}.{}", project::PROJECT_EXT);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            this.update_in(cx, |ws, window, cx| ws.write_project_as(path, window, cx)).ok();
        })
        .detach();
    }

    pub(crate) fn write_project_as(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = match path.extension().and_then(|e| e.to_str()) {
            Some(project::PROJECT_EXT) => path,
            _ => path.with_extension(project::PROJECT_EXT),
        };
        let held = |p: &Path| pipelines::writing(&crate::editing::text::resolve(p), cx);
        if held(&path) || held(&path.with_extension(project::WORKSPACE_EXT)) {
            return self.status_message("Wait for repository writes before saving the project", cx);
        }
        if let Some(w) = self.window_of_project(&path, cx).filter(|w| *w != self.window) {
            w.update(cx, |_, window, _| window.activate_window()).ok();
            let detail = format!("{} is open in another window.", path.display());
            return self.alert(window, cx, "Could not save the project", &detail);
        }
        let mut p = (self.project.clone()).unwrap_or_else(|| Project::new(path.clone(), Vec::new()));
        (p.path, p.unread) = (path.clone(), false);
        // relative to the new place
        p.roots = (self.roots.iter()).map(|r| Root { spelt: None, ..r.clone() }).collect();
        if let Err(e) = p.save() {
            let detail = format!("{}: {e}", path.display());
            return self.alert(window, cx, "Could not save the project", &detail);
        }
        self.workspace_file = Some(p.default_workspace());
        self.project = Some(p);
        if let Err(e) = self.save_workspace(window, true, cx) {
            self.alert(window, cx, "Could not save the workspace", &e);
        }
        push_recent(&mut cx.global_mut::<Recent>().projects, path);
        self.refresh_menus(cx);
        autosave(cx);
        cx.notify();
    }

    /// Project > Save Workspace As…: this window's state under another name beside the
    /// project; the window uses it from then on.
    pub(crate) fn save_workspace_as(&mut self, _: &SaveWorkspaceAs, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = &self.project else {
            return self.alert(
                window,
                cx,
                "Could not save the workspace",
                "Save the project first (Project > Save Project As…).",
            );
        };
        let dir = p.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = format!("{} 2.{}", p.name(), project::WORKSPACE_EXT);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            this.update_in(cx, |ws, window, cx| ws.write_workspace_as(path, window, cx)).ok();
        })
        .detach();
    }

    pub(crate) fn write_workspace_as(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = match path.extension().and_then(|e| e.to_str()) {
            Some(project::WORKSPACE_EXT) => path,
            _ => path.with_extension(project::WORKSPACE_EXT),
        };
        // another window's recovery data is never written over
        let theirs = self.other_workspaces(cx).any(|ws| {
            let file = (ws.workspace_file.clone()).or_else(|| ws.project.as_ref().map(Project::default_workspace));
            file.is_some_and(|f| project::same_path(&f, &path))
        });
        if theirs {
            let detail = format!("{} belongs to a project open in another window.", path.display());
            return self.alert(window, cx, "Could not save the workspace", &detail);
        }
        let was = self.workspace_file.replace(path.clone());
        match self.save_workspace(window, true, cx) {
            Ok(()) => {
                push_recent(&mut cx.global_mut::<Recent>().projects, path);
                self.refresh_menus(cx);
                autosave(cx);
            }
            Err(e) => {
                self.workspace_file = was;
                self.alert(window, cx, "Could not save the workspace", &e);
            }
        }
    }

    /// Project > Edit Project: the project file in a tab; saving it applies it (`apply_if_project`).
    pub(crate) fn edit_project(&mut self, _: &EditProject, window: &mut Window, cx: &mut Context<Self>) {
        match self.project.as_ref().map(|p| p.path.clone()) {
            Some(path) => self.open_path_here(path, window, cx),
            None => self.status_message("This window has no project", cx),
        }
    }

    /// A project file saved in Orlo, as `written`, applies to the window that has that project:
    /// its folders and settings. One that does not read is reported, and the window keeps what
    /// it had.
    pub(crate) fn apply_if_project(
        &mut self,
        editor: &Entity<Editor>,
        written: &ropey::Rope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = editor.read(cx).doc(cx).path.clone() else {
            return;
        };
        if path.extension().and_then(|e| e.to_str()) != Some(project::PROJECT_EXT) {
            return;
        }
        // the window that has the project: this one, or another
        let Some(target) = self.window_of_project(&path, cx) else {
            return;
        };
        let text = written.to_string();
        let apply = move |ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| match Project::parse(
            &path, &text,
        ) {
            Ok((p, warnings)) => {
                ws.project = Some(p.clone());
                ws.set_roots(p.roots, cx);
                ws.apply_view(cx);
                ws.roots_changed(window, cx);
                if !warnings.is_empty() {
                    let w = warnings.join("\n");
                    ws.alert(window, cx, "Some project settings are ignored", &w);
                }
            }
            Err(e) => {
                let detail = format!("{e}\n\nThe window keeps the project as it was.");
                ws.alert(window, cx, "Could not apply the project", &detail);
            }
        };
        if target == self.window {
            apply(self, window, cx)
        } else if let Some(w) = target.downcast::<Workspace>() {
            cx.defer(move |cx| {
                w.update(cx, |ws, window, cx| apply(ws, window, cx)).ok();
            });
        }
    }
}

/// A recent entry's label: the project's name, and the workspace's when it is not the
/// project's own.
fn project_label(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(project::WORKSPACE_EXT) => format!("{} (workspace)", project::name_of(path)),
        _ => project::name_of(path),
    }
}

/// What opening `path` reads: the project file, the workspace file and its window state
/// (none when the project has no workspace yet). A workspace that does not read is set aside
/// beside itself and the project opens without it, so its text is never written over.
fn read_project_files(path: &Path) -> Result<(PathBuf, PathBuf, Option<WindowState>), String> {
    let migrated = |f: WorkspaceFile| {
        let mut st = f.window;
        st.sanitize();
        Some(st)
    };
    if path.extension().and_then(|e| e.to_str()) == Some(project::WORKSPACE_EXT) {
        let f = WorkspaceFile::load(path)?;
        return Ok((f.project_path(path), path.to_path_buf(), migrated(f)));
    }
    let ws = path.with_extension(project::WORKSPACE_EXT);
    if !ws.exists() {
        return Ok((path.to_path_buf(), ws, None));
    }
    match WorkspaceFile::load(&ws) {
        Ok(f) => Ok((path.to_path_buf(), ws, migrated(f))),
        Err(e) => {
            let mut aside = ws.with_extension(format!("{}.bad", project::WORKSPACE_EXT));
            let mut n = 1;
            loop {
                match crate::files::fsops::rename(&ws, &aside) {
                    Ok(()) => break,
                    Err(r) if r.kind() == std::io::ErrorKind::AlreadyExists => {
                        n += 1;
                        aside = ws.with_extension(format!("{}.{n}.bad", project::WORKSPACE_EXT));
                    }
                    Err(r) => {
                        let detail =
                            format!("{e}; it could not be set aside either ({r}), so the project stays closed");
                        return Err(detail);
                    }
                }
            }
            eprintln!("Orlo: {e}; kept as {}", aside.display());
            Ok((path.to_path_buf(), ws, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{temp, window};
    use super::*;

    #[gpui::test]
    fn clearing_project_drops_runtime_panels_and_terminal(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = window(cx);
        let terminal = ws.update_in(cx, |ws, w, cx| {
            ws.show_terminal(&ShowTerminal, w, cx);
            let terminal = ws.terminal.as_ref().unwrap().downgrade();
            ws.clear_window(w, cx);
            assert!(ws.panel.is_none());
            assert!(ws.terminal.is_none());
            cx.notify();
            terminal
        });
        cx.run_until_parked();
        assert!(terminal.upgrade().is_none());
    }

    #[gpui::test]
    fn checkout_lease_keeps_project_open_and_preserves_workspace(cx: &mut gpui::TestAppContext) {
        let d = crate::editing::text::resolve(&temp("orlo-checkout-workspace"));
        let project = d.join("test.sublime-project");
        std::fs::write(&project, "{}").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_project_path(&action(&project, false), w, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("only draft")));
            let workspace = ws.project.as_ref().unwrap().default_workspace();
            let lease = pipelines::lease(&d, cx);
            assert!(ws.save_workspace(w, true, cx).is_err());
            ws.close_project(&CloseProject, w, cx);
            assert!(ws.project.is_some());
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).text(), "only draft");
            assert!(!workspace.exists());
            drop(lease);
            ws.save_workspace(w, true, cx).unwrap();
            assert!(std::fs::read_to_string(workspace).unwrap().contains("only draft"));
        });
    }
    use gpui::{TestAppContext, VisualTestContext};

    fn action(path: &Path, here: bool) -> OpenProjectPath {
        let path = path.to_path_buf();
        OpenProjectPath { path, here }
    }

    /// Two project folders with a file each and a project file each, real paths.
    fn two_projects(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let d = crate::editing::text::resolve(&temp(name));
        for (sub, file) in [("one", "a.txt"), ("two", "b.txt")] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
            std::fs::write(d.join(sub).join(file), format!("{file}\n")).unwrap();
        }
        let one = d.join("one/One.sublime-project");
        let two = d.join("two/Two.sublime-project");
        std::fs::write(&one, r#"{ "folders": [{ "path": "." }] }"#).unwrap();
        std::fs::write(&two, r#"{ "folders": [{ "path": "." }], "build_systems": [] }"#).unwrap();
        (d, one, two)
    }

    fn open(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &Path, here: bool) {
        let a = action(path, here);
        ws.update_in(cx, |ws, window, cx| ws.open_project_path(&a, window, cx));
        cx.run_until_parked();
    }

    fn titles(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<(String, bool, String)> {
        ws.read_with(cx, |ws, cx| {
            (ws.views.tabs())
                .map(|t| {
                    let e = t.read(cx);
                    (e.title(cx), e.doc(cx).dirty, e.doc(cx).text())
                })
                .collect()
        })
    }

    #[gpui::test]
    fn switching_projects_keeps_unsaved_and_untitled_text_in_the_workspace_left(cx: &mut TestAppContext) {
        let (d, one, two) = two_projects("orlo-project-switch");
        let (ws, cx) = window(cx);
        open(&ws, cx, &one, false);
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(ws.roots.len(), 1);
            assert_eq!(ws.roots[0].path, d.join("one"));
            assert_eq!(ws.project.as_ref().unwrap().name(), "One");
            ws.open_path(d.join("one/a.txt"), window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("edited ")));
            ws.new_file(&NewFile, window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("draft")));
        });
        open(&ws, cx, &two, true);
        let saved = std::fs::read_to_string(d.join("one/One.orlo-workspace")).unwrap();
        assert!(saved.contains("draft") && saved.contains("edited a.txt"), "{saved}");
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.roots[0].path, d.join("two"));
            assert_eq!(ws.project.as_ref().unwrap().name(), "Two");
            assert!(ws.views.tabs().all(|t| Workspace::is_blank_untitled(t, cx)));
        });
        // and back: both buffers as they were, still unsaved
        open(&ws, cx, &one, true);
        let mut back = titles(&ws, cx);
        back.sort();
        assert_eq!(back, [("a.txt".into(), true, "edited a.txt\n".into()), ("untitled".into(), true, "draft".into())]);
        assert!(d.join("two/Two.orlo-workspace").exists());
        // the recent list has both, newest first
        ws.read_with(cx, |_, cx| {
            let r = &cx.global::<Recent>().projects;
            assert_eq!(r[..2], [one.clone(), two.clone()]);
        });
    }

    #[gpui::test]
    fn a_project_that_does_not_read_or_a_workspace_that_cannot_be_written_changes_nothing(cx: &mut TestAppContext) {
        let (d, one, two) = two_projects("orlo-project-failures");
        let (ws, cx) = window(cx);
        open(&ws, cx, &one, false);
        std::fs::write(&two, "{ \"folders\": 3 }").unwrap();
        open(&ws, cx, &two, true);
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Ok");
        ws.read_with(cx, |ws, _| assert_eq!(ws.project.as_ref().unwrap().name(), "One"));
        // the workspace of One cannot be written: the switch does not happen
        std::fs::write(&two, r#"{ "folders": [{ "path": "." }] }"#).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let ro = std::fs::Permissions::from_mode(0o555);
        std::fs::set_permissions(d.join("one"), ro).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("one/a.txt"), window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
        });
        open(&ws, cx, &two, true);
        std::fs::set_permissions(d.join("one"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(cx.has_pending_prompt(), "the failed write is said");
        cx.simulate_prompt_answer("Ok");
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.project.as_ref().unwrap().name(), "One");
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).text(), "xa.txt\n", "the edit is still here");
        });
        // a workspace that does not read is set aside, never written over
        std::fs::write(d.join("two/Two.orlo-workspace"), "not json").unwrap();
        open(&ws, cx, &two, true);
        ws.read_with(cx, |ws, _| assert_eq!(ws.project.as_ref().unwrap().name(), "Two"));
        let aside = d.join("two/Two.orlo-workspace.bad");
        assert_eq!(std::fs::read_to_string(aside).unwrap(), "not json");
    }

    #[gpui::test]
    fn a_project_open_in_another_window_is_not_opened_twice(cx: &mut TestAppContext) {
        let (_, one, _) = two_projects("orlo-project-twice");
        let (ws1, _) = window(cx);
        let (ws2, _) = cx.add_window_view(Workspace::new);
        let (w1, w2) = (cx.windows()[0], cx.windows()[1]);
        let mut vcx1 = VisualTestContext::from_window(w1, cx);
        let mut vcx2 = VisualTestContext::from_window(w2, cx);
        open(&ws1, &mut vcx1, &one, false);
        open(&ws2, &mut vcx2, &one, true);
        ws2.read_with(&vcx2, |ws, _| assert!(ws.project.is_none()));
        assert_eq!(vcx2.windows().len(), 2, "no third window");
    }

    #[gpui::test]
    fn project_settings_sit_between_the_settings_file_and_the_windows_and_views_choices(cx: &mut TestAppContext) {
        let (d, one, _) = two_projects("orlo-project-precedence");
        std::fs::write(
            &one,
            r#"{ "folders": [{ "path": "." }],
                 "settings": { "tab_size": 2, "wrap_width": 60, "rulers": [70], "word_wrap": true } }"#,
        )
        .unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, _, cx| {
            let mut s = Settings::default();
            (s.tab_size, s.wrap_width, s.rulers) = (4, 0, vec![90]);
            cx.set_global(s);
            ws.apply_view(cx);
            assert_eq!((ws.view.wrap_width, ws.view.rulers.clone()), (0, vec![90]));
        });
        open(&ws, cx, &one, false);
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(ws.view.wrap_width, 60, "the project's");
            assert_eq!(ws.view.rulers, [70]);
            assert_eq!(ws.view.word_wrap, crate::config::settings::WordWrap::On);
            ws.open_path(d.join("one/a.txt"), window, cx);
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).tab_size, 2, "the project's, not the file's 4");
            // the window's menu choice beats the project
            ws.set_ruler(&SetRuler { col: Some(100) }, window, cx);
            assert_eq!(ws.view.rulers, [100]);
            // the buffer's and the view's beat both
            ws.set_tab_size(&SetTabSize { size: 8 }, window, cx);
            ws.set_wrap_column(&SetWrapColumn { col: Some(40) }, window, cx);
            let e = ws.active_editor().unwrap().read(cx);
            assert_eq!((e.doc(cx).tab_size, e.wrap_cols()), (8, 40));
        });
    }

    #[gpui::test]
    fn editing_the_project_file_applies_it_and_a_broken_one_is_refused(cx: &mut TestAppContext) {
        let (d, one, _) = two_projects("orlo-project-edit");
        std::fs::create_dir_all(d.join("lib")).unwrap();
        let (ws, cx) = window(cx);
        open(&ws, cx, &one, false);
        ws.update_in(cx, |ws, window, cx| {
            ws.edit_project(&EditProject, window, cx);
            let e = ws.active_editor().unwrap();
            let retype = |e: &Entity<Editor>, t: &'static str, cx: &mut Context<Workspace>| {
                e.update(cx, |e, cx| {
                    e.edit(cx, |s| {
                        s.select_all();
                        s.insert(t)
                    })
                })
            };
            retype(&e, r#"{ "folders": [{ "path": "." }, { "path": "../lib" }] }"#, cx);
            let written = e.read(cx).doc(cx).rope.clone();
            ws.apply_if_project(&e, &written, window, cx);
            let paths: Vec<_> = ws.roots.iter().map(|r| r.path.clone()).collect();
            assert_eq!(paths, [d.join("one"), d.join("lib")]);
            retype(&e, "{ \"folders\": [{}] }", cx);
            let written = e.read(cx).doc(cx).rope.clone();
            ws.apply_if_project(&e, &written, window, cx);
        });
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Ok");
        ws.read_with(cx, |ws, _| assert_eq!(ws.roots.len(), 2, "kept"));
    }

    #[gpui::test]
    fn save_project_as_writes_relative_folders_and_follows_the_roots(cx: &mut TestAppContext) {
        let (d, _, _) = two_projects("orlo-project-save-as");
        let (ws, cx) = window(cx);
        let p = d.join("Mine.sublime-project");
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("one"), window, cx);
            ws.write_project_as(p.clone(), window, cx);
            ws.open_path(d.join("two"), window, cx);
        });
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"one\"") && text.contains("\"two\""), "{text}");
        assert!(d.join("Mine.orlo-workspace").exists());
        ws.update_in(cx, |ws, window, cx| {
            ws.remove_root(&RemoveRoot { index: 0 }, window, cx);
        });
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("\"one\""), "{text}");
    }

    #[gpui::test]
    fn a_session_naming_a_project_file_that_is_gone_keeps_its_folders_and_writes_nothing(cx: &mut TestAppContext) {
        let (d, _, _) = two_projects("orlo-project-gone");
        let gone = d.join("Gone.sublime-project");
        let st =
            WindowState { project: Some(gone.clone()), folders: vec![Root::new(d.join("one"))], ..Default::default() };
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.restore(st, window, cx);
            assert_eq!(ws.roots.len(), 1);
            assert!(ws.project.as_ref().unwrap().unread);
            ws.open_path(d.join("two"), window, cx);
            let mut texts = Texts::new();
            let back = ws.capture(window, true, &mut texts, cx);
            assert_eq!(back.project, Some(gone.clone()), "still named");
        });
        assert!(!gone.exists(), "never written");
    }
    #[gpui::test]
    fn project_switch_restores_normal_mode(cx: &mut TestAppContext) {
        let d = crate::editing::text::resolve(&temp("orlo-pr17-review-project-switch"));
        let one = d.join("one.sublime-project");
        let two = d.join("two.sublime-project");
        std::fs::write(&one, "{}").unwrap();
        std::fs::write(&two, "{}").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_project_path(&action(&one, false), window, cx);
            ws.toggle_distraction_free(&ToggleDistractionFree, window, cx);
            assert!(ws.distraction_free.is_some());
            ws.open_project_path(&action(&two, true), window, cx);
            assert!(ws.distraction_free.is_none(), "new project had no saved distraction-free mode");
        });
    }
    #[test]
    fn corrupt_workspace_preserves_previous_recovery_copy() {
        let d = temp("orlo-workspace-recovery");
        let project = d.join("Example.sublime-project");
        let workspace = d.join("Example.orlo-workspace");
        let previous = d.join("Example.orlo-workspace.bad");
        std::fs::write(&workspace, "new damaged recovery").unwrap();
        std::fs::write(&previous, "older unsaved draft").unwrap();
        assert!(read_project_files(&project).is_ok());
        assert_eq!(std::fs::read_to_string(&previous).unwrap(), "older unsaved draft");
        let copies: Vec<_> =
            std::fs::read_dir(&d).unwrap().flatten().filter_map(|e| std::fs::read_to_string(e.path()).ok()).collect();
        assert!(copies.contains(&"new damaged recovery".to_string()));
    }
}
