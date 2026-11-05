//! The window's project roots: adding, removing and reordering folders, which root a
//! path is under, and its branch.

use super::*;
use crate::config::project::{same_path, Root};

impl Workspace {
    /// The roots, as a session or a project gives them: a folder not there now stays, marked.
    pub(crate) fn set_roots(&mut self, roots: Vec<Root>, cx: &mut Context<Self>) {
        self.roots = (roots.into_iter())
            .map(|mut r| {
                r.missing = !r.path.is_dir();
                r
            })
            .collect();
        self.branches = (self.roots.iter())
            .filter(|r| !r.missing)
            .filter_map(|r| Some((r.path.clone(), git_branch(&r.path)?)))
            .collect();
        self.real_paths.get_mut().clear();
        self.sidebar.set_roots(&self.roots);
        self.sidebar.refresh();
        self.walk_files(cx);
        self.index_roots(cx);
    }

    /// The roots changed: the side bar, Goto Anything, the index, the poll, the menus, the
    /// session and the project file follow.
    pub(crate) fn roots_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.real_paths.get_mut().clear();
        self.sidebar.set_roots(&self.roots);
        self.sidebar.refresh();
        self.walk_files(cx);
        self.index_roots(cx);
        self.poll_disk(window, cx);
        self.refresh_menus(cx);
        autosave(cx);
        cx.notify();
    }

    /// Project > Add Folder to Project, a folder opened or dropped: appended to the roots and
    /// expanded; a folder that is a root already is shown, not added twice.
    pub fn add_root(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        push_recent(&mut cx.global_mut::<Recent>().folders, path.clone());
        self.sidebar.visible = true;
        if !self.roots.iter().any(|r| same_path(&r.path, &path)) {
            if let Some(b) = git_branch(&path) {
                self.branches.insert(path.clone(), b);
            }
            self.roots.push(Root::new(path.clone()));
            self.save_project_roots(window, cx);
        }
        self.sidebar.expand(path);
        self.roots_changed(window, cx);
    }

    /// The root a path is under (the innermost of overlapping roots), by its own spelling or
    /// its real path.
    pub(crate) fn root_of(&self, path: &Path) -> Option<usize> {
        if self.roots.is_empty() {
            return None;
        }
        let deepest = |under: &dyn Fn(&Path) -> bool| {
            (self.roots.iter().enumerate())
                .filter(|(_, r)| under(&r.path))
                .max_by_key(|(_, r)| r.path.components().count())
                .map(|(i, _)| i)
        };
        deepest(&|root| path.starts_with(root)).or_else(|| {
            let real = |p: &Path| {
                let mut known = self.real_paths.borrow_mut();
                known.entry(p.to_path_buf()).or_insert_with(|| crate::editing::text::resolve(p)).clone()
            };
            let path = real(path);
            deepest(&|root| path.starts_with(real(root)))
        })
    }

    /// The status bar's branch: the active file's root's, else the first root's that has one.
    pub(crate) fn active_branch(&self, cx: &App) -> Option<String> {
        if self.branches.is_empty() {
            return None;
        }
        let active = self.active_editor()?.read(cx).doc(cx).path.clone();
        let root = active.and_then(|p| self.root_of(&p));
        (root.into_iter().chain(0..self.roots.len())).find_map(|i| self.branches.get(&self.roots[i].path)).cloned()
    }

    /// The root a side bar command acts on: the selected row's, else `None`.
    fn selected_root(&self) -> Option<usize> {
        self.root_of(self.sidebar.selected.as_deref()?)
    }

    /// Project > Remove Folder from Project: the selected row's root, else a list to pick one.
    /// Its open files stay open, edits and all.
    pub(crate) fn remove_folder(&mut self, _: &RemoveFolder, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected_root() {
            return self.remove_root(&RemoveRoot { index }, window, cx);
        }
        let items = (self.roots.iter().enumerate())
            .map(|(index, r)| quick_panel::Item::command(r.label(), tilde(&r.path), RemoveRoot { index }))
            .collect();
        self.open_quick_panel(Kind::Commands, items, window, cx);
    }

    pub(crate) fn remove_root(&mut self, a: &RemoveRoot, window: &mut Window, cx: &mut Context<Self>) {
        if a.index >= self.roots.len() {
            return;
        }
        let gone = self.roots.remove(a.index);
        self.branches.remove(&gone.path);
        if (self.sidebar.selected.as_deref()).is_some_and(|s| s.starts_with(&gone.path)) {
            self.sidebar.selected = None;
        }
        self.save_project_roots(window, cx);
        self.roots_changed(window, cx);
    }

    /// Project > Remove all Folders from Project; the open files stay.
    pub(crate) fn remove_folders(&mut self, _: &RemoveFolders, window: &mut Window, cx: &mut Context<Self>) {
        self.roots.clear();
        self.branches.clear();
        self.sidebar.selected = None;
        self.save_project_roots(window, cx);
        self.roots_changed(window, cx);
    }

    /// Project > Move Folder Up / Down: the selected row's root changes places with its
    /// neighbour.
    pub(crate) fn move_folder(&mut self, a: &MoveFolder, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected_root() else {
            return self.status_message("Select a folder in the side bar first", cx);
        };
        let j = match a.up {
            true => i.checked_sub(1),
            false => Some(i + 1).filter(|&j| j < self.roots.len()),
        };
        if let Some(j) = j {
            self.roots.swap(i, j);
            self.save_project_roots(window, cx);
            self.roots_changed(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::window;
    use super::*;
    use gpui::TestAppContext;

    /// Real paths: a folder opened is resolved (`/var` is `/private/var`).
    fn dirs(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["a/src", "b", "a/src/inner"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        let d = crate::editing::text::resolve(&d);
        std::fs::write(d.join("a/src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(d.join("b/notes.txt"), "notes\n").unwrap();
        d
    }

    #[gpui::test]
    fn adding_folders_appends_roots_and_removing_one_keeps_its_files_open(cx: &mut TestAppContext) {
        let d = dirs("orlo-roots-add");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("a"), window, cx);
            ws.open_path(d.join("b"), window, cx);
            ws.open_path(d.join("a"), window, cx);
            let paths: Vec<_> = ws.roots.iter().map(|r| r.path.clone()).collect();
            assert_eq!(paths, [d.join("a"), d.join("b")], "appended, a not twice");
            ws.open_path(d.join("b/notes.txt"), window, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
            ws.sidebar.selected = Some(d.join("b/notes.txt"));
            ws.remove_folder(&RemoveFolder, window, cx);
            assert_eq!(ws.roots.len(), 1);
            let e = ws.active_editor().unwrap();
            assert!(e.read(cx).doc(cx).dirty, "its file stays open, edited");
            assert_eq!(e.read(cx).doc(cx).text(), "xnotes\n");
            // reordering by the selected row's root
            ws.open_path(d.join("b"), window, cx);
            ws.sidebar.selected = Some(d.join("b"));
            ws.move_folder(&MoveFolder { up: true }, window, cx);
            assert_eq!(ws.roots[0].path, d.join("b"));
            ws.move_folder(&MoveFolder { up: true }, window, cx);
            assert_eq!(ws.roots[0].path, d.join("b"), "already first");
            assert_eq!(ws.root_of(&d.join("a/src/main.rs")), Some(1));
            ws.remove_folders(&RemoveFolders, window, cx);
            assert!(ws.roots.is_empty());
            assert_eq!(ws.views.tabs().count(), 1, "notes.txt is still open");
        });
    }

    #[gpui::test]
    fn the_innermost_root_owns_a_path_and_a_link_resolves_to_its_target(cx: &mut TestAppContext) {
        let d = dirs("orlo-roots-overlap");
        let link = d.join("link-to-a");
        std::os::unix::fs::symlink(d.join("a"), &link).unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("a"), window, cx);
            ws.open_path(d.join("a/src"), window, cx);
            assert_eq!(ws.root_of(&d.join("a/src/main.rs")), Some(1), "innermost");
            assert_eq!(ws.root_of(&d.join("a/x")), Some(0));
            // the link is a root of its own, unless it names one already
            ws.open_path(link.clone(), window, cx);
            assert_eq!(ws.roots.len(), 2, "the link names root a");
            assert_eq!(ws.root_of(&link.join("src/main.rs")), Some(1));
            // a frame later: from the paths already resolved
            let resolved = ws.real_paths.borrow().len();
            assert!(resolved > 0);
            assert_eq!(ws.root_of(&link.join("src/main.rs")), Some(1));
            assert_eq!(ws.real_paths.borrow().len(), resolved);
            assert_eq!(ws.root_of(Path::new("/nowhere/x")), None);
            ws.remove_folders(&RemoveFolders, window, cx);
            assert!(ws.real_paths.borrow().is_empty(), "dropped with the roots");
            assert_eq!(ws.root_of(&link.join("src/main.rs")), None);
            assert!(ws.real_paths.borrow().is_empty(), "no roots: nothing resolved");
        });
    }

    #[gpui::test]
    fn goto_anything_lists_every_roots_files_once(cx: &mut TestAppContext) {
        let d = dirs("orlo-roots-goto");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("a"), window, cx);
            ws.open_path(d.join("a/src"), window, cx);
            ws.open_path(d.join("b"), window, cx);
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            let items = ws.file_items(cx);
            let mut details: Vec<_> = items.iter().map(|i| i.detail.clone()).collect();
            details.sort();
            assert_eq!(details, ["a/src/main.rs", "b/notes.txt"], "main.rs once, under a");
        });
    }

    #[gpui::test]
    fn find_in_files_searches_every_root_with_its_excludes(cx: &mut TestAppContext) {
        use crate::searching::results::SearchFlags;
        let d = dirs("orlo-roots-search");
        std::fs::write(d.join("a/hit.rs"), "needle\n").unwrap();
        std::fs::write(d.join("a/hit.log"), "needle\n").unwrap();
        std::fs::create_dir_all(d.join("a/gen")).unwrap();
        std::fs::write(d.join("a/gen/x.rs"), "needle\n").unwrap();
        std::fs::write(d.join("b/hit.log"), "needle\n").unwrap();
        let (ws, cx) = window(cx);
        let mut a = Root::new(d.join("a"));
        a.file_exclude_patterns = vec!["*.log".into()];
        a.folder_exclude_patterns = vec!["gen".into()];
        let st = WindowState { folders: vec![a, Root::new(d.join("b"))], ..Default::default() };
        ws.update_in(cx, |ws, window, cx| {
            ws.restore(st, window, cx);
            ws.find_in_files(&FindInFiles, window, cx);
            let flags = SearchFlags::default();
            ws.run_search("needle", "", None, FindOpts::default(), flags, window, cx).unwrap();
        });
        // the results drain on a 100 ms timer of the app's clock
        for _ in 0..600 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
            if ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done)) {
                break;
            }
        }
        ws.read_with(cx, |ws, _| {
            let mut found: Vec<_> = (ws.result_set.as_ref().unwrap().files.iter())
                .map(|f| f.path.strip_prefix(&d).unwrap().to_path_buf())
                .collect();
            found.sort();
            assert_eq!(
                found,
                [PathBuf::from("a/hit.rs"), PathBuf::from("b/hit.log")],
                "a's patterns hide a/hit.log and a/gen; b keeps its .log"
            );
        });
    }

    #[gpui::test]
    fn a_session_from_before_brings_its_folder_back_as_the_only_root(cx: &mut TestAppContext) {
        let d = dirs("orlo-roots-session");
        let (ws, cx) = window(cx);
        let old = format!(r#"{{"folder": "{}", "expanded": ["{}"]}}"#, d.join("a").display(), d.join("a").display());
        let mut st: WindowState = serde_json::from_str(&old).unwrap();
        st.sanitize();
        ws.update_in(cx, |ws, window, cx| {
            ws.restore(st, window, cx);
            assert_eq!(ws.roots.len(), 1);
            assert!(ws.sidebar.is_expanded(&d.join("a")));
            // and the session written now names the roots, and the folder for an older Orlo
            let mut texts = Texts::new();
            let back = ws.capture(window, true, &mut texts, cx);
            assert_eq!(back.folder, Some(d.join("a")));
            assert_eq!(back.folders.len(), 1);
            assert!(!back.folders[0].missing);
        });
    }

    #[gpui::test]
    fn a_root_that_goes_missing_stays_listed_and_comes_back(cx: &mut TestAppContext) {
        let d = dirs("orlo-roots-missing");
        let (ws, cx) = window(cx);
        let st = WindowState { folders: vec![Root::new(d.join("a")), Root::new(d.join("gone"))], ..Default::default() };
        ws.update_in(cx, |ws, window, cx| {
            ws.restore(st, window, cx);
            assert_eq!(ws.roots.len(), 2, "kept though it is not there");
            assert!(ws.roots[1].missing && !ws.roots[0].missing);
        });
        std::fs::create_dir_all(d.join("gone")).unwrap();
        std::fs::rename(d.join("a"), d.join("a-renamed")).unwrap();
        ws.update_in(cx, |ws, window, cx| ws.poll_disk(window, cx));
        cx.run_until_parked();
        ws.update(cx, |ws, _| {
            assert!(!ws.roots[1].missing, "back");
            assert!(ws.roots[0].missing, "renamed away: kept, marked");
        });
    }
}
