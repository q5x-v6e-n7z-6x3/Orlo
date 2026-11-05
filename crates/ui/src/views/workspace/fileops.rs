//! Side bar file operations: new file and folder, rename, move, duplicate, move to the
//! Trash, reveal in Finder; keyboard focus in the side bar. A file open in Orlo keeps its
//! buffer (text, unsaved changes, undo) through a rename or move, in every window.

use super::*;
use crate::files::fsops;
use crate::searching::results::Status;

/// What the results browser says of a file moved since its preview.
const MOVED: &str = "moved or renamed since the preview; Refresh";

/// Where `p` itself is, its folder's links resolved but not `p`'s own: a link renamed or
/// trashed is the link, never what it points to.
fn real_location(p: &Path) -> PathBuf {
    match (p.parent(), p.file_name()) {
        (Some(dir), Some(name)) => crate::editing::text::resolve(dir).join(name),
        _ => p.to_path_buf(),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `name`, checked, beside `p`.
fn sibling(p: &Path, name: &str) -> Result<PathBuf, String> {
    let name = fsops::check_name(name).map_err(|e| e.to_string())?;
    Ok(p.parent().unwrap_or(Path::new("/")).join(name))
}

impl Workspace {
    /// What a side bar command acts on: the `⌘`-marked rows when the selected row is one of
    /// them, else the selected row.
    fn sidebar_targets(&self) -> Vec<PathBuf> {
        let Some(sel) = self.sidebar.selected.clone() else {
            return Vec::new();
        };
        match self.sidebar.marked.contains(&sel) {
            true => self.sidebar.marked.clone(),
            false => vec![sel],
        }
    }

    /// The folder something new goes in: the selected folder, the selected file's, else the
    /// first root.
    fn target_dir(&self) -> Option<PathBuf> {
        match self.sidebar.selected.clone() {
            Some(p) if p.is_dir() => Some(p),
            Some(p) => p.parent().map(Path::to_path_buf),
            None => self.roots.iter().find(|r| !r.missing).map(|r| r.path.clone()),
        }
    }

    /// Replace in Files or a pipeline run writing files in any window: no file is moved under
    /// them.
    fn writes_under_way(&self, cx: &App) -> Option<&'static str> {
        let busy = |ws: &Workspace| ws.applying.is_some() || ws.pipeline_run.is_some();
        (busy(self) || self.other_workspaces(cx).any(busy))
            .then_some("Files are being written (Replace in Files or a pipeline run); try again when it is done")
    }

    fn no_target(&mut self, cx: &mut Context<Self>) {
        self.status_message("Select a file or folder in the side bar first", cx);
    }

    forward_actions! {
    /// File > New File in the side bar's folder: a name, then an empty file made there (never
    /// over one), opened in a tab.
    pub(crate) new_file_here(NewFileHere, window, cx) => ask_new(false, window, cx);
    pub(crate) new_folder_here(NewFolderHere, window, cx) => ask_new(true, window, cx);
    }

    fn ask_new(&mut self, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.target_dir() else {
            return self.no_target(cx);
        };
        let title = format!("New {} in {}", if folder { "folder" } else { "file" }, tilde(&dir));
        let done: OnDone = Box::new(move |v, _, ws, window, cx| {
            if pipelines::writing(&crate::editing::text::resolve(&dir), cx) {
                return Err("Wait for the directory write to finish".into());
            }
            let name = v.text("text");
            let made = match folder {
                true => fsops::create_dir(&dir, name),
                false => fsops::create_file(&dir, name),
            }
            .map_err(|e| e.to_string())?;
            ws.sidebar.expand(dir.clone());
            ws.sidebar.selected = Some(made.clone());
            ws.sidebar.refresh();
            ws.poll_disk(window, cx);
            if !folder {
                ws.open_path_here(made, window, cx);
            }
            Ok(())
        });
        let field = Field::text("text", "Name", "");
        self.show_window_form(&title, vec![field], done, window, cx);
    }

    /// Rename… : the selected file or folder, in its folder; a name taken is refused. Changing
    /// only the case of letters works.
    pub(crate) fn rename_path(&mut self, _: &RenamePath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.sidebar.selected.clone() else {
            return self.no_target(cx);
        };
        if let Some(why) = self.writes_under_way(cx) {
            return self.status_message(why, cx);
        }
        let name = file_name(&from);
        let title = format!("Rename {}", tilde(&from));
        let done: OnDone = Box::new(move |v, _, ws, window, cx| {
            let to = sibling(&from, v.text("text"))?;
            if to == from {
                return Ok(());
            }
            ws.move_one(&from, &to, window, cx).map_err(|e| e.to_string())
        });
        let field = Field::text("text", "New name", name);
        self.show_window_form(&title, vec![field], done, window, cx);
    }

    /// Move… : the selected (or `⌘`-marked) items into a folder; each that would land on
    /// something there is refused, the others move.
    pub(crate) fn move_paths(&mut self, _: &MovePaths, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.sidebar_targets();
        let Some(first) = items.first().cloned() else {
            return self.no_target(cx);
        };
        if let Some(why) = self.writes_under_way(cx) {
            return self.status_message(why, cx);
        }
        let from_dir = first.parent().map(Path::to_path_buf).unwrap_or_default();
        let title = match items.len() {
            1 => format!("Move {} into the folder", tilde(&first)),
            n => format!("Move {n} items from {} into the folder", tilde(&from_dir)),
        };
        let initial = tilde(&from_dir);
        let done: OnDone = Box::new(move |v, _, ws, window, cx| {
            let dest = from_dir.join(crate::searching::search::untilde(v.text("text").trim()));
            if !dest.is_dir() {
                return Err(format!("{} is not a folder", tilde(&dest)));
            }
            let mut failed = Vec::new();
            let mut moved = 0;
            for p in &items {
                let Some(name) = p.file_name() else { continue };
                let to = dest.join(name);
                if to == *p {
                    continue;
                }
                match ws.move_one(p, &to, window, cx) {
                    Ok(()) => moved += 1,
                    Err(e) => failed.push(e.to_string()),
                }
            }
            if !failed.is_empty() {
                return Err(format!("Moved {moved} of {}. {}", items.len(), failed.join("; ")));
            }
            ws.sidebar.expand(dest);
            Ok(())
        });
        let field = Field::text("text", "Folder", initial);
        self.show_window_form(&title, vec![field], done, window, cx);
    }

    /// Rename or move one item on disk, then everything in Orlo that names it follows. A
    /// failure changes nothing here.
    fn move_one(&mut self, from: &Path, to: &Path, window: &mut Window, cx: &mut Context<Self>) -> std::io::Result<()> {
        if let Some(why) = self.writes_under_way(cx) {
            return Err(std::io::Error::other(why));
        }
        let from_real = real_location(from);
        if pipelines::writing_under(&from_real, cx) || pipelines::writing_under(&real_location(to), cx) {
            return Err(std::io::Error::other("another operation is writing this path"));
        }
        fsops::rename(from, to)?;
        let to_real = real_location(to);
        self.after_move(from, to, &from_real, &to_real, window, cx);
        Ok(())
    }

    /// `from` is now `to` on disk (`*_real`: the same, links resolved): every open buffer under
    /// it takes its new path, in every window, keeping its text, unsaved changes and undo; the
    /// roots, side bars, results browsers, Goto Anything, the recent lists and the session
    /// follow.
    pub(crate) fn after_move(
        &mut self,
        from: &Path,
        to: &Path,
        from_real: &Path,
        to_real: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffers = editors::one_per_document(self.open_editors(cx), cx, |v| v.doc(cx).path.is_some());
        for e in buffers {
            let Some(old) = e.read(cx).doc(cx).path.clone() else {
                continue;
            };
            let Some(new) = fsops::moved(&old, from_real, to_real).or_else(|| fsops::moved(&old, from, to)) else {
                continue;
            };
            let ext_changed = old.extension() != new.extension();
            e.update(cx, |e, cx| {
                // the file as Orlo last saw it, under its new name: a change made by another
                // app meanwhile still shows at the next check
                let now = crate::files::watch::stamp(&new);
                e.update_doc(cx, |d| {
                    if let (Some(was), Some(now)) = (d.disk, now) {
                        if was.mtime == now.mtime && was.len == now.len {
                            d.disk = Some(now);
                        }
                    }
                    d.path = Some(new.clone());
                });
                if ext_changed {
                    e.on_path_changed(cx);
                }
                cx.notify();
            });
        }
        let r = cx.global_mut::<Recent>();
        for p in (r.files.iter_mut()).chain(&mut r.folders).chain(&mut r.projects) {
            if let Some(n) = fsops::moved(p, from_real, to_real).or_else(|| fsops::moved(p, from, to)) {
                *p = n;
            }
        }
        self.follow_move(from, to, from_real, to_real, window, cx);
        let others = (cx.windows().into_iter()).filter(|w| *w != self.window).filter_map(|w| w.downcast::<Workspace>());
        for w in others {
            w.update(cx, |ws, window, cx| ws.follow_move(from, to, from_real, to_real, window, cx)).ok();
        }
        autosave(cx);
    }

    /// This window's part of a move: roots, side bar, results, closed files, Goto Anything.
    fn follow_move(
        &mut self,
        from: &Path,
        to: &Path,
        from_real: &Path,
        to_real: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let map = |p: &Path| fsops::moved(p, from, to).or_else(|| fsops::moved(p, from_real, to_real));
        if let Some(p) = &mut self.project {
            if let Some(path) = map(&p.path) {
                p.path = path;
            }
        }
        for path in [&mut self.workspace_file, &mut self.last_dir] {
            if let Some(moved) = path.as_deref().and_then(map) {
                *path = Some(moved);
            }
        }
        let mut roots = false;
        for r in &mut self.roots {
            if let Some(n) = map(&r.path) {
                r.path = n;
                roots = true;
            }
        }
        if roots {
            self.save_project_roots(window, cx);
        }
        self.sidebar.moved(from, to);
        if from_real != from {
            self.sidebar.moved(from_real, to_real);
        }
        for p in &mut self.closed {
            if let Some(n) = map(&p.path) {
                p.path = n;
            }
        }
        // a preview over files that moved is not applied to them: Refresh first
        let mut stale = false;
        if let Some(set) = &mut self.result_set {
            for file in &mut set.files {
                if map(&file.path).is_some() || map(&file.shown).is_some() {
                    file.status = Some(Status::Stale(MOVED.into()));
                    stale = true;
                }
            }
        }
        if stale {
            if let Some(p) = self.results_panel_mut() {
                p.note = Some("Files in these results were moved or renamed: Refresh before applying".into());
            }
        }
        self.roots_changed(window, cx);
        self.apply_view(cx);
    }

    /// Whether Apply must leave file `i` of the results alone: it moved since the preview.
    pub(crate) fn moved_since_preview(&self, i: usize) -> bool {
        (self.result_set.as_ref())
            .and_then(|s| s.files.get(i))
            .is_some_and(|f| matches!(&f.status, Some(Status::Stale(w)) if w == MOVED))
    }

    /// Duplicate… : a copy of the selected file or folder beside it, made from what is on
    /// disk (a file's unsaved changes in Orlo stay in its own buffer). New View into File is
    /// the command for a second view of the same buffer.
    pub(crate) fn duplicate_path(&mut self, _: &DuplicatePath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.sidebar.selected.clone() else {
            return self.no_target(cx);
        };
        let real = real_location(&from);
        let unsaved = (self.open_editors(cx).iter()).any(|e| {
            (e.read(cx).doc(cx).path.as_ref()).is_some_and(|p| p.starts_with(&real)) && e.read(cx).doc(cx).dirty
        });
        let title = match unsaved {
            true => {
                format!("Duplicate {} as saved on disk (its unsaved changes in Orlo are not copied) as", tilde(&from))
            }
            false => format!("Duplicate {} (as saved on disk) as", tilde(&from)),
        };
        let initial = file_name(&fsops::copy_name(&from));
        let done: OnDone = Box::new(move |v, _, ws, window, cx| {
            let to = sibling(&from, v.text("text"))?;
            if pipelines::writing(&real_location(&to), cx) {
                return Err("Wait for the directory write to finish".into());
            }
            fsops::duplicate(&from, &to).map_err(|e| e.to_string())?;
            ws.sidebar.selected = Some(to);
            ws.sidebar.refresh();
            ws.poll_disk(window, cx);
            Ok(())
        });
        let field = Field::text("text", "Name", initial);
        self.show_window_form(&title, vec![field], done, window, cx);
    }

    /// Move to Trash: the selected (or `⌘`-marked) items into the macOS Trash, from where
    /// Finder's Put Back returns them. Files open in Orlo stay open, marked deleted, their
    /// text kept.
    pub(crate) fn trash_paths(&mut self, _: &TrashPaths, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.sidebar_targets();
        if items.is_empty() {
            return self.no_target(cx);
        }
        if let Some(why) = self.writes_under_way(cx) {
            return self.status_message(why, cx);
        }
        let mut failed = Vec::new();
        let mut gone = 0;
        for p in &items {
            let real = real_location(p);
            if pipelines::writing_under(&real, cx) {
                failed.push(format!("{}: another operation is writing this path", tilde(p)));
                continue;
            }
            match crate::app::macos::trash(p) {
                Ok(_) => {
                    gone += 1;
                    self.mark_deleted_under(p, &real, cx);
                    self.sidebar.marked.retain(|m| !m.starts_with(p));
                }
                Err(e) => failed.push(format!("{}: {e}", tilde(p))),
            }
        }
        self.sidebar.selected = None;
        self.sidebar.refresh();
        self.poll_disk(window, cx);
        if !failed.is_empty() {
            let detail = format!("Moved {gone} of {} to the Trash.\n\n{}", items.len(), failed.join("\n"));
            return self.alert(window, cx, "Could not move to the Trash", &detail);
        }
        self.status_message(format!("Moved {gone} item{} to the Trash", if gone == 1 { "" } else { "s" }), cx);
    }

    /// Every buffer whose file was under `p` shows it deleted at once (the next poll would).
    fn mark_deleted_under(&mut self, p: &Path, real: &Path, cx: &mut Context<Self>) {
        for e in self.open_editors(cx) {
            let under = (e.read(cx).doc(cx).path.as_ref()).is_some_and(|f| f.starts_with(p) || f.starts_with(real));
            if under && !e.read(cx).doc(cx).deleted {
                e.update(cx, |e, cx| {
                    e.update_doc(cx, |d| d.deleted = true);
                    cx.notify();
                });
            }
        }
    }

    /// Reveal in Finder.
    pub(crate) fn reveal_in_finder(&mut self, _: &RevealInFinder, _: &mut Window, cx: &mut Context<Self>) {
        let p = (self.sidebar.selected.clone())
            .or_else(|| self.active_editor().and_then(|e| e.read(cx).doc(cx).path.clone()));
        match p {
            Some(p) => cx.reveal_path(&p),
            None => self.no_target(cx),
        }
    }

    /// A side bar row's right-click menu; its commands act on that row.
    pub fn open_sidebar_menu(
        &mut self,
        path: PathBuf,
        root_row: bool,
        at: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        use context_menu::item;
        self.sidebar.location_request += 1;
        if !self.sidebar.marked.contains(&path) {
            self.sidebar.marked.clear();
        }
        self.sidebar.selected = Some(path);
        let mut items = vec![
            item("New File…", NewFileHere),
            item("New Folder…", NewFolderHere),
            None,
            item("Rename…", RenamePath),
            item("Move…", MovePaths),
            item("Duplicate…", DuplicatePath),
            None,
            item("Move to Trash", TrashPaths),
            None,
            item("Reveal in Finder", RevealInFinder),
            item("Find in Folder…", FindInSidebarItem),
        ];
        if root_row {
            items.extend([
                None,
                item("Remove Folder from Project", RemoveFolder),
                item("Move Folder Up", MoveFolder { up: true }),
                item("Move Folder Down", MoveFolder { up: false }),
            ]);
        }
        self.open_context_menu(at, items, cx);
    }

    /// Find in Folder…: Find in Files over the selected (or marked) rows.
    pub(crate) fn find_in_sidebar_item(&mut self, _: &FindInSidebarItem, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.sidebar_targets();
        if items.is_empty() {
            return self.no_target(cx);
        }
        let where_ = (items.iter()).fold(String::new(), |w, p| search::append_token(&w, &tilde(p)));
        self.find_in_files(&FindInFiles, window, cx);
        if let Some(field) = self.fif_panel().map(|p| p.where_.clone()) {
            field.update(cx, |e, cx| e.set_text(&where_, cx));
        }
    }

    // ---- keyboard in the side bar ----

    /// View > Side Bar > Focus Side Bar (`ctrl-0`): the keys go to the side bar's tree.
    pub(crate) fn focus_sidebar(&mut self, _: &FocusSideBar, window: &mut Window, cx: &mut Context<Self>) {
        if self.roots.is_empty() {
            return self.status_message("Add a folder to the project first", cx);
        }
        if !self.sidebar.visible {
            self.sidebar.visible = true;
            self.sidebar_anim = None;
        }
        if self.sidebar.selected.is_none() {
            self.sidebar.selected = self.sidebar.rows().first().map(|r| r.path.clone());
        }
        window.focus(&self.sidebar_focus);
        cx.notify();
    }

    /// Up / Down in the side bar.
    pub(crate) fn sidebar_step(&mut self, a: &SideBarStep, _: &mut Window, cx: &mut Context<Self>) {
        let rows: Vec<PathBuf> = self.sidebar.rows().iter().map(|r| r.path.clone()).collect();
        if rows.is_empty() {
            return;
        }
        let at = (self.sidebar.selected.as_ref()).and_then(|s| rows.iter().position(|r| r == s));
        let next = match (at, a.down) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(rows.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.sidebar.selected = Some(rows[next].clone());
        self.reveal_sidebar_row(next);
        cx.notify();
    }

    /// The side bar scrolls to tree row `i`.
    pub(super) fn reveal_sidebar_row(&self, i: usize) {
        // the OPEN FILES rows above: its heading, a heading a group when there are several,
        // and the views
        let groups = self.views.groups.len();
        let above = match self.sidebar.show_open_files {
            true => 1 + if groups > 1 { groups } else { 0 } + self.views.tabs().count(),
            false => 0,
        };
        self.sidebar.reveal_row(i, above);
    }

    /// Left / Right in the side bar: collapse or expand the selected folder (Left on a file or
    /// a closed folder goes to its parent's row).
    pub(crate) fn sidebar_fold(&mut self, a: &SideBarFold, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.sidebar.selected.clone() else {
            return;
        };
        let is_root = self.roots.iter().any(|r| r.path == sel);
        if sel.is_dir() && self.sidebar.is_expanded(&sel) != a.open {
            self.sidebar.toggle(&sel);
            if self.sidebar.is_expanded(&sel) {
                self.navigate_explorer(sel.clone(), window, cx);
            }
            self.poll_disk(window, cx);
        } else if !a.open && !is_root {
            self.sidebar.selected = sel.parent().map(Path::to_path_buf);
        }
        cx.notify();
    }

    /// Enter in the side bar: a folder opens or closes, a file opens in a tab kept open
    /// (Space previews it).
    pub(crate) fn sidebar_open(&mut self, a: &SideBarOpen, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.sidebar.selected.clone() else {
            return;
        };
        if sel.is_dir() {
            self.sidebar.toggle(&sel);
            if self.sidebar.is_expanded(&sel) {
                self.navigate_explorer(sel.clone(), window, cx);
            }
            self.poll_disk(window, cx);
            return cx.notify();
        }
        match a.preview {
            true => {
                self.open_preview(sel, window, cx);
                window.focus(&self.sidebar_focus);
            }
            false => {
                self.open_kept_path(sel, window, cx);
            }
        }
        cx.notify();
    }

    /// Escape in the side bar: back to the editor.
    pub(crate) fn sidebar_leave(&mut self, _: &SideBarLeave, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_editor(window, cx);
        cx.notify();
    }
}

/// The side bar's keys, in its own context.
pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("SideBar");
    vec![
        KeyBinding::new("ctrl-0", FocusSideBar, None),
        KeyBinding::new("alt-left", ExplorerBack, ctx),
        KeyBinding::new("alt-right", ExplorerForward, ctx),
        KeyBinding::new("alt-up", ExplorerParent, ctx),
        KeyBinding::new("up", SideBarStep { down: false }, ctx),
        KeyBinding::new("down", SideBarStep { down: true }, ctx),
        KeyBinding::new("left", SideBarFold { open: false }, ctx),
        KeyBinding::new("right", SideBarFold { open: true }, ctx),
        KeyBinding::new("enter", SideBarOpen { preview: false }, ctx),
        KeyBinding::new("space", SideBarOpen { preview: true }, ctx),
        KeyBinding::new("escape", SideBarLeave, ctx),
        KeyBinding::new("f2", RenamePath, ctx),
        KeyBinding::new("cmd-backspace", TrashPaths, ctx),
    ]
}

#[cfg(test)]
mod tests {
    use super::super::form::Submit;
    use super::super::ui_tests::{temp, window};
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    /// A project folder `name` with `files` (real paths), open as the window's root.
    fn project(ws: &Entity<Workspace>, cx: &mut VisualTestContext, name: &str, files: &[&str]) -> PathBuf {
        let d = crate::editing::text::resolve(&temp(name));
        for f in files {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, format!("{f}\n")).unwrap();
        }
        ws.update_in(cx, |ws, window, cx| ws.open_path(d.clone(), window, cx));
        d
    }

    /// Type `text` into the open form and press Enter; the form's error, if it stayed open.
    fn answer(ws: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str) -> Option<String> {
        ws.update_in(cx, |ws, window, cx| {
            let Some(Panel::Form(p)) = &ws.panel else {
                panic!("no form open");
            };
            let field = p.inputs().next().unwrap().clone();
            field.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert(text)
                })
            });
            ws.submit_form(&Submit, window, cx);
            match &ws.panel {
                Some(Panel::Form(p)) => Some(p.error.clone().unwrap_or_default()),
                _ => None,
            }
        })
    }

    fn select(ws: &Entity<Workspace>, cx: &mut VisualTestContext, p: &Path) {
        ws.update(cx, |ws, _| ws.sidebar.selected = Some(p.to_path_buf()));
    }

    #[gpui::test]
    fn n28_explorer_parent_json_action_reveals_nested_parent(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-parent-red", &["nested/deep/file.txt"]);
        select(&ws, cx, &d.join("nested/deep"));
        let action = cx.update(|_, cx| cx.build_action("workspace::ExplorerParent", None));
        assert!(action.is_ok(), "explorer Parent must be reachable through JSON/custom keys");
        cx.update(|window, cx| window.dispatch_action(action.unwrap(), cx));
        super::super::ui_tests::settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.sidebar.selected.as_deref() == Some(d.join("nested").as_path()))
        });
        ws.read_with(cx, |ws, _| {
            assert_eq!(ws.roots.len(), 1);
            assert_eq!(ws.roots[0].path, d);
            assert!(ws.sidebar.is_expanded(&d));
        });
        ws.update_in(cx, |ws, window, cx| ws.explorer_back(&ExplorerBack, window, cx));
        super::super::ui_tests::settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.sidebar.selected.as_deref() == Some(d.join("nested/deep").as_path()))
        });
    }

    #[gpui::test]
    fn renaming_an_open_file_reapplies_its_scoped_and_editorconfig_preferences(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-ops-rename-preferences", &["a.txt"]);
        std::fs::write(d.join(".editorconfig"), "root=true\n[*.rs]\nindent_style=space\nindent_size=2\ntab_width=8")
            .unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(r#"{"overrides":[{"language":"Rust","settings":{"font_size":24}}]}"#, window, cx);
            ws.open_path(d.join("a.txt"), window, cx);
            ws.move_one(&d.join("a.txt"), &d.join("a.rs"), window, cx).unwrap();
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert_eq!(e.highlighter.syntax_name(), "Rust");
            assert_eq!(e.view.font_size, 24.);
            assert_eq!((e.doc(cx).indent_width(), e.doc(cx).tab_size), (2, 8));
            assert_eq!(e.text(cx), "a.txt\n");
        });
    }

    #[gpui::test]
    fn renaming_an_open_unsaved_file_keeps_its_buffer_in_every_window(cx: &mut TestAppContext) {
        let (ws, _) = window(cx);
        let (ws2, _) = cx.add_window_view(Workspace::new);
        let (w1, w2) = (cx.windows()[0], cx.windows()[1]);
        let mut cx1 = VisualTestContext::from_window(w1, cx);
        let mut cx2 = VisualTestContext::from_window(w2, cx);
        let d = project(&ws, &mut cx1, "orlo-ops-rename", &["a.txt", "b.txt"]);
        // a view of a.txt in each window, sharing one unsaved buffer
        let (a, other) = ws.update_in(&mut cx1, |ws, window, cx| {
            ws.open_path(d.join("a.txt"), window, cx);
            let a = ws.active_editor().unwrap();
            a.update(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved ")));
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            let other = ws.active_editor().unwrap();
            let released = ws.release_views(std::slice::from_ref(&other), cx);
            (a, (other, released))
        });
        let (other, released) = other;
        ws2.update_in(&mut cx2, |ws, window, cx| ws.adopt_views(released, 0, 0, None, window, cx));
        select(&ws, &mut cx1, &d.join("a.txt"));
        ws.update_in(&mut cx1, |ws, window, cx| ws.rename_path(&RenamePath, window, cx));
        // a name taken: said in the form, nothing changes
        let e = answer(&ws, &mut cx1, "b.txt").expect("the form stays");
        assert!(e.contains("b.txt already exists"), "{e}");
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "b.txt\n");
        assert_eq!(a.read_with(&cx1, |e, cx| e.doc(cx).path.clone()), Some(d.join("a.txt")));
        assert_eq!(answer(&ws, &mut cx1, "c.md"), None, "renamed");
        cx1.run_until_parked();
        for v in [&a, &other] {
            v.read_with(&cx1, |e, cx| {
                assert_eq!(e.doc(cx).path.as_deref(), Some(d.join("c.md").as_path()));
                assert!(e.doc(cx).dirty);
                assert_eq!(e.doc(cx).text(), "unsaved a.txt\n");
                assert_eq!(e.title(cx), "c.md");
            });
        }
        assert!(a.read_with(&cx1, |e, app| e.same_document(other.read(app))));
        // undo still works, in the window it moved to
        other.update(&mut cx2, |e, cx| e.edit(cx, |s| s.undo()));
        cx2.run_until_parked();
        assert_eq!(a.read_with(&cx1, |e, cx| e.doc(cx).text()), "a.txt\n");
        ws.read_with(&cx1, |ws, _| {
            assert_eq!(ws.sidebar.selected.as_deref(), Some(d.join("c.md").as_path()));
        });
        // a case-only rename
        select(&ws, &mut cx1, &d.join("c.md"));
        ws.update_in(&mut cx1, |ws, window, cx| ws.rename_path(&RenamePath, window, cx));
        assert_eq!(answer(&ws, &mut cx1, "C.md"), None);
        assert_eq!(a.read_with(&cx1, |e, cx| e.doc(cx).path.clone()), Some(d.join("C.md")));
        let names: Vec<_> =
            std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"C.md".to_string()), "{names:?}");
    }

    #[gpui::test]
    fn moving_a_folder_moves_its_open_files_and_a_root_inside_it(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-ops-move", &["src/x.rs", "src/deep/y.rs", "dest/keep"]);
        let x = ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("src/x.rs"), window, cx);
            let x = ws.active_editor().unwrap();
            x.update(cx, |e, cx| e.edit(cx, |s| s.insert("// edited\n")));
            ws.open_path(d.join("src/deep"), window, cx);
            x
        });
        select(&ws, cx, &d.join("src"));
        ws.update_in(cx, |ws, window, cx| ws.move_paths(&MovePaths, window, cx));
        let e = answer(&ws, cx, &d.join("nowhere").display().to_string()).unwrap();
        assert!(e.contains("not a folder"), "{e}");
        assert_eq!(answer(&ws, cx, &d.join("dest").display().to_string()), None);
        cx.run_until_parked();
        x.read_with(cx, |e, cx| {
            assert_eq!(e.doc(cx).path.as_deref(), Some(d.join("dest/src/x.rs").as_path()));
            assert!(e.doc(cx).dirty && e.doc(cx).text().starts_with("// edited"));
        });
        ws.read_with(cx, |ws, _| {
            let paths: Vec<_> = ws.roots.iter().map(|r| r.path.clone()).collect();
            assert_eq!(paths, [d.clone(), d.join("dest/src/deep")], "the root moved along");
        });
        assert_eq!(std::fs::read_to_string(d.join("dest/src/deep/y.rs")).unwrap(), "src/deep/y.rs\n");
    }

    #[gpui::test]
    fn a_link_renamed_leaves_the_file_open_through_it_alone(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-ops-link", &["real/f.txt"]);
        std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
        let f = ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("link/f.txt"), window, cx);
            ws.active_editor().unwrap()
        });
        assert_eq!(f.read_with(cx, |e, cx| e.doc(cx).path.clone()), Some(d.join("real/f.txt")));
        select(&ws, cx, &d.join("link"));
        ws.update_in(cx, |ws, window, cx| ws.rename_path(&RenamePath, window, cx));
        assert_eq!(answer(&ws, cx, "link2"), None);
        assert!(std::fs::symlink_metadata(d.join("link2")).unwrap().is_symlink());
        assert_eq!(f.read_with(cx, |e, cx| e.doc(cx).path.clone()), Some(d.join("real/f.txt")));
    }

    #[gpui::test]
    fn duplicate_copies_the_disk_and_trash_keeps_the_open_text(cx: &mut TestAppContext) {
        // a name of this run's own, so the test takes back exactly what it put in the Trash
        let unique = format!("orlo-test-trash-{}", std::process::id());
        let b = format!("{unique}/b.txt");
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-ops-dup", &["a.txt", &b]);
        let a = ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("a.txt"), window, cx);
            let a = ws.active_editor().unwrap();
            a.update(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved ")));
            a
        });
        select(&ws, cx, &d.join("a.txt"));
        ws.update_in(cx, |ws, window, cx| ws.duplicate_path(&DuplicatePath, window, cx));
        let title = ws.read_with(cx, |ws, _| match &ws.panel {
            Some(Panel::Form(p)) => p.title().to_string(),
            _ => String::new(),
        });
        assert!(title.contains("unsaved changes in Orlo are not copied"), "{title}");
        assert_eq!(answer(&ws, cx, "a copy.txt"), None);
        assert_eq!(std::fs::read_to_string(d.join("a copy.txt")).unwrap(), "a.txt\n");
        assert_eq!(a.read_with(cx, |e, cx| e.doc(cx).path.clone()), Some(d.join("a.txt")));
        // to the Trash: the open file stays, marked deleted, its text whole
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join(&b), window, cx);
            ws.sidebar.selected = Some(d.join(&unique));
            ws.trash_paths(&TrashPaths, window, cx);
        });
        assert!(!d.join(&unique).exists());
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert!(e.read(cx).doc(cx).deleted);
            assert_eq!(e.read(cx).doc(cx).text(), format!("{b}\n"));
        });
        // the user's Trash is left as it was: this run's folder, and only it, deleted
        if let Some(home) = std::env::var_os("USER").map(|u| PathBuf::from("/Users").join(u)) {
            let trashed = home.join(".Trash").join(&unique);
            if std::fs::read_to_string(trashed.join("b.txt")).is_ok_and(|t| t == format!("{b}\n")) {
                std::fs::remove_dir_all(&trashed).unwrap();
            }
        }
    }

    #[gpui::test]
    fn a_file_changed_elsewhere_during_a_rename_is_still_noticed(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-ops-external", &["a.txt"]);
        let a = ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("a.txt"), window, cx);
            ws.active_editor().unwrap()
        });
        // another app writes it just before Orlo renames it
        std::fs::write(d.join("a.txt"), "changed elsewhere, and longer\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.move_one(&d.join("a.txt"), &d.join("b.txt"), window, cx).unwrap();
            assert_ne!(a.read(cx).doc(cx).disk, crate::files::watch::stamp(&d.join("b.txt")));
            ws.poll_disk(window, cx);
        });
        cx.run_until_parked();
        cx.executor().advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        assert_eq!(a.read_with(cx, |e, cx| e.doc(cx).text()), "changed elsewhere, and longer\n");
    }

    #[gpui::test]
    fn a_replace_preview_over_a_moved_file_is_stale_and_not_applied(cx: &mut TestAppContext) {
        use crate::searching::results::SearchFlags;
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-ops-preview", &["a.txt"]);
        ws.update_in(cx, |ws, window, cx| {
            ws.find_in_files(&FindInFiles, window, cx);
            let flags = SearchFlags::default();
            ws.run_search("a", "", Some("Z"), FindOpts::default(), flags, window, cx).unwrap();
        });
        for _ in 0..100 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
            if ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done)) {
                break;
            }
        }
        ws.update_in(cx, |ws, window, cx| {
            ws.move_one(&d.join("a.txt"), &d.join("moved.txt"), window, cx).unwrap();
            let f = &ws.result_set.as_ref().unwrap().files[0];
            assert!(matches!(&f.status, Some(Status::Stale(w)) if w == MOVED));
            ws.apply_replacements(&ApplyReplacements, window, cx);
        });
        for _ in 0..50 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
        }
        assert_eq!(std::fs::read_to_string(d.join("moved.txt")).unwrap(), "a.txt\n", "not applied");
        assert!(!d.join("a.txt").exists(), "nothing written at the old place");
    }
    #[gpui::test]
    fn directory_rename_blocks_descendant_write(cx: &mut TestAppContext) {
        let d = crate::editing::text::resolve(&temp("orlo-pr17-review-lease"));
        let source = d.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("x.txt");
        std::fs::write(&file, "keep").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.sidebar.selected = Some(source.clone());
            ws.rename_path(&RenamePath, window, cx);
            // A background write starts after the rename form is already open.
            let lease = pipelines::lease(&file, cx);
            assert!(pipelines::writing(&file, cx));
            let Some(Panel::Form(p)) = &ws.panel else { panic!("no form") };
            let field = p.inputs().next().unwrap().clone();
            field.update(cx, |e, cx| e.set_text("renamed", cx));
            ws.submit_form(&form::Submit, window, cx);
            assert!(source.exists(), "renamed a directory while a descendant writer still owns its old path");
            drop(lease);
            ws.submit_form(&form::Submit, window, cx);
            assert!(!source.exists());
            assert_eq!(std::fs::read_to_string(d.join("renamed/x.txt")).unwrap(), "keep");
        });
    }
    #[gpui::test]
    fn moving_project_folder_updates_its_recovery_paths(cx: &mut TestAppContext) {
        let d = temp("orlo-move-project").canonicalize().unwrap();
        let from = d.join("original");
        let to = d.join("renamed");
        std::fs::create_dir(&from).unwrap();
        let project = from.join("Mine.sublime-project");
        std::fs::write(&project, r#"{"folders":[{"path":"."}]}"#).unwrap();
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_project_path(&OpenProjectPath { path: project.clone(), here: false }, window, cx);
            ws.new_file(&NewFile, window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved project draft")));
            ws.last_dir = Some(from.clone());
            ws.move_one(&from, &to, window, cx).unwrap();
            assert_eq!(ws.project.as_ref().unwrap().path, to.join("Mine.sublime-project"));
            assert_eq!(ws.workspace_file, Some(to.join("Mine.orlo-workspace")));
            assert_eq!(ws.last_dir, Some(to.clone()));
            assert!(cx.global::<Recent>().projects.contains(&to.join("Mine.sublime-project")));
            ws.save_workspace(window, true, cx).unwrap();
        });
        assert!(!from.exists());
        let saved = std::fs::read_to_string(to.join("Mine.orlo-workspace")).unwrap();
        assert!(saved.contains("unsaved project draft"));
    }
}
