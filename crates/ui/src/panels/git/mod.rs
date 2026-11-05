//! Git in the window: the repository's status in a dock panel, its changes and conflicts in
//! the editor, and the commands on them.

use crate::integrations::{changes, git};
use crate::workspace::*;
use ropey::Rope;

mod branch;
mod commit;
mod history;
mod remote;
mod selection;
mod stash;
mod tag;
mod worktree;

pub use branch::*;
pub use history::*;
pub use remote::*;
pub use stash::*;
pub use tag::*;
pub use worktree::*;

actions!(
    workspace,
    [
        ShowGit,
        GitRefresh,
        GitOpen,
        GitStage,
        GitUnstage,
        GitCommit,
        GitAmend,
        GitFetch,
        GitPull,
        GitPush,
        GitRemotes,
        GitWorktrees,
        GitCreateWorktree,
        GitPruneWorktrees,
        GitTags,
        GitCreateTag,
        GitStashes,
        GitCreateStash,
        GitSwitchBranch,
        GitCreateBranch,
        GitRenameBranch,
        GitDeleteBranch,
        GitStageLines,
        GitStageHunk,
        GitUnstageHunk,
        GitFileHistory,
        GitBlame,
        ShowChange,
        NextChange,
        PreviousChange,
        RevertChange,
        NextConflict,
        AcceptOurs,
        AcceptTheirs,
        AcceptBoth,
        GitInit
    ]
);

parameter_actions! { workspace, serializable;
    GitSelect { step: isize, }
}

parameter_actions! { workspace, plain;
    GitUseBranch { root: PathBuf, branch: String, }

    GitInitAt { path: PathBuf, }
}

#[derive(Clone, Copy)]
pub struct GitSidebarDrag;

pub struct ChangePanel {
    source: Entity<Editor>,
    path: Option<PathBuf>,
    baseline: Rope,
    version: u64,
    hunk: changes::Hunk,
    old: String,
    pub viewer: Entity<Editor>,
}

/// The largest side the Git panel shows side by side, in bytes; a bigger file shows its patch.
pub(crate) const GIT_SIDES_LIMIT: usize = 10 << 20;

/// The Git panel's diff of the selected file.
pub(crate) enum GitDiff {
    /// The raw patch: for binary, oversized or conflicted files, Git's errors, and no change.
    Patch(Entity<Editor>),
    /// The two versions the patch compares, side by side and aligned.
    Sides {
        path: PathBuf,
        versions: git::Versions,
        viewers: [Entity<Editor>; 2],
        _aligned: Entity<crate::editor::AlignedPair>,
    },
}

impl GitDiff {
    fn viewers(&self) -> Vec<&Entity<Editor>> {
        match self {
            GitDiff::Patch(viewer) => vec![viewer],
            GitDiff::Sides { viewers, .. } => viewers.iter().collect(),
        }
    }

    fn render(&self, cx: &mut Context<Workspace>) -> AnyElement {
        match self {
            GitDiff::Patch(viewer) => viewer.clone().into_any_element(),
            GitDiff::Sides { viewers, .. } => {
                let side = |i: usize| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .on_mouse_down(MouseButton::Left, cx.listener(move |ws, _, _, _| ws.git.diff_side = i))
                        .child(viewers[i].clone())
                };
                div()
                    .size_full()
                    .flex()
                    .child(side(0))
                    .child(side(1).border_l_1().border_color(ui(theme::BORDER)))
                    .into_any_element()
            }
        }
    }
}

/// A diff read off the UI thread: the versions with their comparison, or the patch and why.
enum LoadedDiff {
    Sides(git::Versions, crate::files::comparison::Pair, [crate::editor::ComparisonHighlights; 2]),
    Patch(Result<String, String>, Option<String>),
}

struct Baseline {
    base: Rope,
    path: Option<PathBuf>,
    version: u64,
    label: &'static str,
}

#[derive(Default)]
pub(crate) struct State {
    pub sidebar_open: bool,
    pub selection_active: bool,
    focus: Option<FocusHandle>,
    scroll: ScrollHandle,
    pub repositories: Vec<git::Repository>,
    pub badges: HashMap<PathBuf, String>,
    refreshing: bool,
    operation: bool,
    commit_drafts: [Option<commit::Draft>; 2],
    selected: Option<(PathBuf, PathBuf)>,
    staged: bool,
    diff: Option<GitDiff>,
    diff_target: Option<(PathBuf, PathBuf, bool)>,
    diff_side: usize,
    /// Why the selected file's diff shows as a patch.
    diff_note: Option<String>,
    diff_generation: u64,
    pending_change: Option<Task<()>>,
    pub message: Option<String>,
    /// Repositories whose own settings run programs, in folders not trusted: each worktree's
    /// top and why it waits (`git::PAUSED`).
    pub paused: Vec<(PathBuf, String)>,
    bases: HashMap<EntityId, Baseline>,
}

pub fn git_key_bindings() -> Vec<KeyBinding> {
    let list = Some("GitPanel");
    vec![
        // ⌃` as in Zed and VS Code; ⌘` too, though macOS also uses it to cycle an app's windows
        KeyBinding::new("ctrl-`", ShowTerminal, None),
        KeyBinding::new("ctrl-shift-`", CloseTerminal, None),
        KeyBinding::new("cmd-`", ShowTerminal, None),
        KeyBinding::new("cmd-shift-`", CloseTerminal, None),
        KeyBinding::new("ctrl-shift-g", ShowGit, None),
        KeyBinding::new("alt-down", NextChange, Some("Editor && !single_line")),
        KeyBinding::new("alt-up", PreviousChange, Some("Editor && !single_line")),
        KeyBinding::new("cmd-k cmd-h", ShowChange, Some("Editor && !single_line")),
        KeyBinding::new("up", GitSelect { step: -1 }, list),
        KeyBinding::new("down", GitSelect { step: 1 }, list),
        KeyBinding::new("enter", GitOpen, list),
        KeyBinding::new("space", GitStage, list),
        KeyBinding::new("shift-space", GitUnstage, list),
        KeyBinding::new("cmd-r", GitRefresh, list),
        KeyBinding::new("escape", ShowGit, list),
    ]
}

impl Workspace {
    /// Pickers can replace each other; only a live Git surface owns the command target.
    pub(crate) fn git_palette_focus(&mut self, w: &Window, cx: &App) -> Option<FocusHandle> {
        if !self.git.sidebar_open || !self.git.selection_active {
            return None;
        }
        if let Some(GitDiff::Sides { viewers, .. }) = &self.git.diff {
            if let Some(i) = viewers.iter().position(|v| v.read(cx).is_focused(w)) {
                self.git.diff_side = i;
            }
        }
        self.git.focus.clone()
    }

    /// A single poll owns both status and index baselines, so external Git operations update
    /// the list and markers together. Slow files and subprocesses never block input.
    pub(crate) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.git.refreshing {
            return;
        }
        self.git.refreshing = true;
        let generation = self.restore_gen;
        let editors: Vec<_> = self.views.tabs().cloned().collect();
        let snapshots: Vec<_> = editors
            .iter()
            .filter_map(|e| {
                let s = e.read(cx).read_state(cx);
                (!e.read(cx).results && !s.doc.read_only && !s.doc.scratch)
                    .then(|| (e.entity_id(), s.doc.path.clone(), s.doc.version, s.doc.rope.clone(), s.saved_text()))
            })
            .collect();
        let mut dirs: Vec<_> = crate::config::project::present(&self.roots);
        dirs.extend(snapshots.iter().filter_map(|(_, p, _, _, _)| p.as_deref()?.parent().map(Path::to_path_buf)));
        dirs.sort();
        dirs.dedup();
        let known: HashMap<_, _> =
            self.git.bases.iter().map(|(id, b)| (*id, (b.base.clone(), b.path.clone(), b.version))).collect();
        let task = cx.background_spawn(async move {
            let mut repos = Vec::new();
            let mut errors = Vec::new();
            let mut paused: Vec<(PathBuf, String)> = Vec::new();
            for dir in dirs {
                match git::discover(&dir) {
                    Ok(Some(repo)) if !repos.iter().any(|r: &git::Repository| r.root == repo.root) => repos.push(repo),
                    Err(e) if git::paused(&e) => {
                        let root = git::toplevel(&dir).unwrap_or(dir);
                        if !paused.iter().any(|(r, _)| *r == root) {
                            paused.push((root, e));
                        }
                    }
                    Err(e) => errors.push(e),
                    _ => {}
                }
            }
            let mut bases = Vec::new();
            let mut index = HashMap::new();
            for (id, path, version, current, saved) in snapshots {
                let repo = path.as_deref().and_then(|p| containing(&repos, p));
                let (base, label) = if let (Some(repo), Some(path)) = (repo, path.as_ref()) {
                    let base = index
                        .entry(path.clone())
                        .or_insert_with(|| git::index_text(&repo.root, path.strip_prefix(&repo.root).unwrap()));
                    let Ok(text) = base else { continue };
                    let text = text.as_deref().unwrap_or("");
                    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
                    let text = crate::editing::text::decoded_file(text).0;
                    (text, "Git index")
                } else {
                    let base = saved.or_else(|| path.as_deref().and_then(|p| Loaded::read(p).ok().map(|l| l.rope)));
                    let Some(base) = base else { continue };
                    (base, "Saved file")
                };
                let same = known.get(&id).is_some_and(|(b, p, v)| *v == version && p == &path && b == &base);
                let hunks = (!same).then(|| changes::between(&base, &current));
                bases.push((id, path, version, base, label, hunks));
            }
            (repos, bases, errors, paused)
        });
        cx.spawn(async move |this, cx| {
            let (repos, bases, errors, paused) = task.await;
            this.update(cx, |ws, cx| {
                if ws.restore_gen != generation {
                    return;
                }
                ws.git.refreshing = false;
                let changed = ws.git.repositories != repos;
                if changed {
                    ws.git.badges = badges(&repos);
                }
                ws.git.repositories = repos;
                ws.git.paused = paused;
                if !errors.is_empty() {
                    ws.git.message = Some(errors.join("\n"));
                }
                let live: HashSet<_> = ws.views.tabs().map(|e| e.entity_id()).collect();
                let available: HashSet<_> = bases.iter().map(|b| b.0).collect();
                ws.git.bases.retain(|id, _| live.contains(id) && available.contains(id));
                for editor in &editors {
                    if live.contains(&editor.entity_id()) && !available.contains(&editor.entity_id()) {
                        editor.update(cx, |e, cx| {
                            if !e.changes.is_empty() {
                                e.changes.clear();
                                cx.notify();
                            }
                        });
                    }
                }
                for (id, path, version, base, label, hunks) in bases {
                    let Some(editor) = editors.iter().find(|e| e.entity_id() == id && live.contains(&id)) else {
                        continue;
                    };
                    let s = editor.read(cx).read_state(cx);
                    if s.doc.path != path || s.doc.version != version {
                        continue;
                    }
                    // to every view of the document, in this window or one that is not polling
                    if let Some(hunks) = hunks {
                        let document = editor.read(cx).document.clone();
                        document.update(cx, |d, cx| d.found_changes(version, hunks, cx));
                    }
                    ws.git.bases.insert(id, Baseline { base, path, version, label });
                }
                if ws.git.sidebar_open {
                    ws.ensure_git_selection();
                    ws.load_git_diff(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn show_git(&mut self, _: &ShowGit, window: &mut Window, cx: &mut Context<Self>) {
        if self.git.sidebar_open {
            self.git.sidebar_open = false;
            if self.git.focus.as_ref().is_some_and(|f| f.contains_focused(window, cx)) {
                self.focus_editor(window, cx);
            }
            self.git.selection_active = false;
            autosave(cx);
            cx.notify();
            return;
        }
        self.cancel_quick_panel(cx);
        self.ensure_git_selection();
        self.git.sidebar_open = true;
        self.git.selection_active = true;
        let focus = self.git.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        window.focus(&focus);
        self.refresh_git(cx);
        self.load_git_diff(cx);
        autosave(cx);
        cx.notify();
    }

    pub(crate) fn git_refresh(&mut self, _: &GitRefresh, _: &mut Window, cx: &mut Context<Self>) {
        self.git.message = None;
        self.refresh_git(cx);
        self.load_git_diff(cx);
    }

    /// Whether the sidebar's selection is `path` under `root`, on either side of the index.
    fn git_selected(&self, root: &Path, path: &Path) -> bool {
        (self.git.selected.as_ref()).is_some_and(|(r, p)| r == root && p == path)
    }

    fn ensure_git_selection(&mut self) {
        let rows = self.git_entries();
        if rows.iter().any(|(r, f, staged)| self.git_selected(r, &f.path) && self.git.staged == *staged) {
            return;
        }
        let row = rows.iter().find(|(r, f, _)| self.git_selected(r, &f.path)).or_else(|| rows.first());
        self.git.selected = row.map(|(r, f, _)| (r.clone(), f.path.clone()));
        self.git.staged = row.is_some_and(|(_, _, staged)| *staged);
    }

    /// The sidebar's rows: the work tree's changes, then the index's.
    fn git_entries(&self) -> Vec<(PathBuf, git::FileStatus, bool)> {
        [false, true]
            .into_iter()
            .flat_map(|staged| {
                self.git.repositories.iter().flat_map(move |r| {
                    (r.files.iter())
                        .filter(move |f| on_side(f, staged))
                        .map(move |f| (r.root.clone(), f.clone(), staged))
                })
            })
            .collect()
    }

    fn git_file_status(&self, root: &Path, path: &Path) -> Option<&git::FileStatus> {
        let repo = self.git.repositories.iter().find(|r| r.root == root)?;
        repo.files.iter().find(|f| f.path == path)
    }

    /// The repository holding the active file.
    fn active_repository(&self, cx: &App) -> Option<&git::Repository> {
        let path = self.active_editor()?.read(cx).doc(cx).path.clone()?;
        containing(&self.git.repositories, &path)
    }

    fn git_target(&self, cx: &App) -> Option<(PathBuf, git::FileStatus)> {
        if self.git.sidebar_open && self.git.selection_active {
            let (root, path) = self.git.selected.as_ref()?;
            return Some((root.clone(), self.git_file_status(root, path)?.clone()));
        }
        let path = self.active_editor()?.read(cx).doc(cx).path.clone()?;
        let repo = containing(&self.git.repositories, &path)?;
        let file = repo.files.iter().find(|f| repo.root.join(&f.path) == path)?;
        Some((repo.root.clone(), file.clone()))
    }

    fn git_root(&self, cx: &App) -> Option<PathBuf> {
        self.git_target(cx).map(|(r, _)| r).or_else(|| {
            let repo = self.active_repository(cx);
            repo.or(self.git.repositories.first()).map(|r| r.root.clone())
        })
    }

    /// Whether a view in any window has unsaved changes to `path`.
    fn unsaved_at(&self, path: &Path, cx: &App) -> bool {
        self.open_editors(cx).iter().any(|e| {
            let doc = e.read(cx).doc(cx);
            doc.path.as_deref() == Some(path) && doc.dirty
        })
    }

    pub(crate) fn git_select(&mut self, a: &GitSelect, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.git_entries();
        if rows.is_empty() {
            return;
        }
        let i = rows
            .iter()
            .position(|(r, f, staged)| self.git_selected(r, &f.path) && self.git.staged == *staged)
            .unwrap_or(0);
        let i = (i as isize + a.step).rem_euclid(rows.len() as isize) as usize;
        self.git.selected = Some((rows[i].0.clone(), rows[i].1.path.clone()));
        self.git.selection_active = true;
        self.git.staged = rows[i].2;
        let working = rows.iter().filter(|(_, _, staged)| !*staged).count();
        self.git.scroll.scroll_to_item(i + if rows[i].2 { 2 + usize::from(working == 0) } else { 1 });
        self.load_git_diff(cx);
    }

    pub(crate) fn git_open(&mut self, _: &GitOpen, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((root, file)) = self.git_target(cx) {
            self.open_path(root.join(file.path), window, cx);
        }
    }

    fn load_git_diff(&mut self, cx: &mut Context<Self>) {
        self.git.diff_generation += 1;
        let generation = self.git.diff_generation;
        let Some((root, file)) = self
            .git
            .selected
            .as_ref()
            .and_then(|(root, path)| Some((root.clone(), self.git_file_status(root, path)?.clone())))
        else {
            self.git.diff = None;
            self.git.diff_target = None;
            self.git.diff_note = None;
            return;
        };
        let staged = self.git.staged;
        let target = (root.clone(), file.path.clone(), staged);
        let path = root.join(&file.path);
        let task = cx.background_spawn(async move { load_diff(&root, &file, staged) });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            this.update(cx, |ws, cx| {
                if ws.git.diff_generation != generation {
                    return;
                }
                ws.git.diff_target = Some(target);
                match loaded {
                    LoadedDiff::Patch(result, note) => {
                        ws.git.diff_note = note;
                        ws.show_patch(result, cx);
                    }
                    LoadedDiff::Sides(versions, pair, highlights) => {
                        ws.git.diff_note = None;
                        ws.show_sides(path, versions, &pair, highlights, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The raw patch in the diff viewer, reusing a patch viewer already there.
    fn show_patch(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        let text = match result {
            Ok(t) if t.is_empty() => "No changes on this side of the index.\n".into(),
            Ok(t) => t,
            Err(e) => e,
        };
        match &self.git.diff {
            Some(GitDiff::Patch(viewer)) if viewer.read(cx).doc(cx).text() != text => viewer.update(cx, |e, cx| {
                e.set_text(&text, cx);
                e.update_doc(cx, |d| {
                    d.read_only = true;
                    d.scratch = true;
                    d.dirty = false;
                });
            }),
            Some(GitDiff::Patch(_)) => {}
            _ => self.git.diff = Some(GitDiff::Patch(diff_view(&text, cx))),
        }
    }

    /// The two versions side by side, aligned; the same versions of the same file keep their
    /// viewers, and with them the scroll.
    fn show_sides(
        &mut self,
        path: PathBuf,
        versions: git::Versions,
        pair: &crate::files::comparison::Pair,
        highlights: [crate::editor::ComparisonHighlights; 2],
        cx: &mut Context<Self>,
    ) {
        if let Some(GitDiff::Sides { path: p, versions: v, .. }) = &self.git.diff {
            if *p == path && *v == versions {
                return;
            }
        }
        if crate::workspace::pipelines::reading_blocked(&path, cx) {
            self.show_patch(Err("File is being written; refresh the Git diff".into()), cx);
            return;
        }
        let mut highlights = highlights.into_iter();
        let texts = [versions.old.as_str(), versions.new.as_str()];
        let viewers = texts.map(|text| {
            let marks = highlights.next().unwrap();
            let (view, path) = (self.view.clone(), path.clone());
            cx.new(|cx| {
                let mut state = EditorState::from_source(text);
                state.doc.path = Some(path);
                state.doc.read_only = true;
                state.doc.scratch = true;
                let mut editor = Editor::new(state, false, cx);
                editor.input_enabled = false;
                editor.view = view;
                editor.view.minimap = false;
                editor.comparison = Some(marks);
                editor
            })
        });
        let hunks = crate::editor::hunks_of(pair);
        let sides = viewers.clone();
        let aligned = cx.new(|cx| crate::editor::AlignedPair::new(sides, hunks, cx));
        self.git.diff = Some(GitDiff::Sides { path, versions, viewers, _aligned: aligned });
        self.git.diff_side = 1;
    }

    /// A picker filled off the UI thread: `load` reads Git, then `rows` makes its items and the
    /// note under them. The result is dropped once the picker or the project is replaced.
    fn git_picker<T: Send + 'static>(
        &mut self,
        title: String,
        loading: &str,
        load: impl FnOnce() -> Result<T, String> + Send + 'static,
        rows: impl FnOnce(T) -> (Vec<quick_panel::Item>, Option<String>) + 'static,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_panel(Kind::Choices, Vec::new(), w, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some(title);
        panel.progress = Some(loading.into());
        let input = panel.input.clone();
        let generation = self.restore_gen;
        let task = cx.background_spawn(async move { load() });
        cx.spawn_in(w, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, _, cx| {
                if ws.restore_gen != generation || ws.quick_panel.as_ref().is_none_or(|p| p.input != input) {
                    return;
                }
                let (items, progress) = match result {
                    Ok(found) => rows(found),
                    Err(e) => {
                        ws.cancel_quick_panel(cx);
                        return ws.status_message(e, cx);
                    }
                };
                let panel = ws.quick_panel.as_mut().unwrap();
                panel.progress = progress;
                panel.set_items(Kind::Choices, items);
                panel.refilter(&panel.query.clone());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn git_run(
        &mut self,
        root: PathBuf,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.git_run_checked(root, None, job, |_, _| true, cx);
    }

    fn git_run_checked(
        &mut self,
        root: PathBuf,
        paths: Option<Vec<PathBuf>>,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
        current: impl FnOnce(&Self, &App) -> bool + 'static,
        cx: &mut Context<Self>,
    ) {
        self.git_run_checked_with_refresh(root, paths, job, current, true, cx);
    }

    fn git_run_checked_with_refresh(
        &mut self,
        root: PathBuf,
        paths: Option<Vec<PathBuf>>,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
        current: impl FnOnce(&Self, &App) -> bool + 'static,
        refresh_worktree: bool,
        cx: &mut Context<Self>,
    ) {
        if self.git.operation {
            self.git.message = Some("A Git operation is still running".into());
            return cx.notify();
        }
        let mut paths = paths.unwrap_or_else(|| vec![root]);
        paths.sort();
        paths.dedup();
        if paths.iter().any(|p| pipelines::writing_under(p, cx)) {
            return self.status_message("Wait for repository writes to finish", cx);
        }
        // Worker ownership survives clear_window and closing its original workspace.
        let leases: Vec<_> = paths.iter().map(|p| pipelines::lease(p, cx)).collect();
        self.git.operation = true;
        self.git.message = Some("Running Git…".into());
        let generation = self.restore_gen;
        let task = cx.background_spawn(async move {
            let _leases = leases;
            job()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |ws, cx| {
                if ws.restore_gen != generation {
                    return;
                }
                ws.git.operation = false;
                if !current(ws, cx) {
                    ws.git.message = None;
                    cx.notify();
                    return;
                }

                let message = match result {
                    Ok(()) => "Git operation completed".into(),
                    Err(e) => e,
                };
                ws.git.message = Some(message.clone());
                ws.status_message(message, cx);
                if refresh_worktree {
                    ws.refresh_git(cx);
                    ws.load_git_diff(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn git_stage(&mut self, _: &GitStage, _: &mut Window, cx: &mut Context<Self>) {
        let Some((root, file)) = self.git_target(cx) else {
            return self.status_message("Select a changed file first", cx);
        };
        let path = root.join(&file.path);
        if self.unsaved_at(&path, cx) {
            self.git.message = Some("Save this file before staging; staging reads disk contents".into());
            return self.status_message("Save this file before staging", cx);
        }
        if pipelines::writing(&path, cx) {
            return self.status_message("Wait for the file write to finish", cx);
        }
        self.git_run(root.clone(), move || git::stage(&root, &file), cx);
    }

    pub(crate) fn git_unstage(&mut self, _: &GitUnstage, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((root, file)) = self.git_target(cx) {
            if pipelines::writing_under(&root, cx) {
                return self.status_message("Wait for repository writes to finish", cx);
            }
            self.git_run(root.clone(), move || git::unstage(&root, &file), cx);
        }
    }

    forward_actions! {
    pub(crate) git_switch_branch(GitSwitchBranch, window, cx) => git_choose_branch(BranchOperation::Switch, window, cx);
    }

    pub(crate) fn git_use_branch(&mut self, a: &GitUseBranch, _: &mut Window, cx: &mut Context<Self>) {
        if self.open_editors(cx).iter().any(|e| {
            let s = e.read(cx).read_state(cx);
            s.doc.dirty && s.doc.path.as_ref().is_some_and(|p| p.starts_with(&a.root))
        }) {
            return self.status_message("Save or close unsaved repository files before switching branches", cx);
        }
        if pipelines::writing_under(&a.root, cx) || self.saves.busy() {
            return self.status_message("Wait for file writes before switching branches", cx);
        }
        let a = a.clone();
        self.git_run(a.root.clone(), move || git::switch(&a.root, &a.branch), cx);
    }

    pub fn git_badges(&self) -> &HashMap<PathBuf, String> {
        &self.git.badges
    }

    /// The status bar's repository: the active file's (else the window's), its branch, how many
    /// files changed and how many of them conflict.
    pub(crate) fn git_status(&self, cx: &App) -> Option<(String, usize, usize)> {
        let repo = self.active_repository(cx).or_else(|| {
            let root = self.git_root(cx)?;
            self.git.repositories.iter().find(|r| r.root == root)
        })?;
        Some((repo.branch.clone(), repo.files.len(), repo.conflict_count()))
    }
}

/// The innermost repository holding `path`.
fn containing<'a>(repos: &'a [git::Repository], path: &Path) -> Option<&'a git::Repository> {
    repos.iter().filter(|r| path.starts_with(&r.root)).max_by_key(|r| r.root.components().count())
}

/// Whether `f` is listed on the index's side (`staged`) or the work tree's.
fn on_side(f: &git::FileStatus, staged: bool) -> bool {
    match staged {
        true => f.has_staged(),
        false => f.has_unstaged() || f.is_conflicted(),
    }
}

fn badges(repos: &[git::Repository]) -> HashMap<PathBuf, String> {
    let mut flags: HashMap<PathBuf, [bool; 4]> = HashMap::new();
    for repo in repos {
        for file in &repo.files {
            let path = repo.root.join(&file.path);
            for p in path.ancestors().take_while(|p| p.starts_with(&repo.root)) {
                let f = flags.entry(p.to_path_buf()).or_default();
                f[0] |= file.is_conflicted();
                f[1] |= file.has_staged();
                f[2] |= file.has_unstaged() && !file.is_untracked();
                f[3] |= file.is_untracked();
            }
        }
    }
    flags
        .into_iter()
        .map(|(p, f)| {
            (p, f.iter().zip(["!", "S", "M", "?"]).filter_map(|(on, s)| on.then_some(s)).collect::<Vec<_>>().join(""))
        })
        .collect()
}

/// Off the UI thread: the selected file's two versions and their comparison, or its patch when
/// it cannot be shown side by side (and why).
fn load_diff(root: &Path, file: &git::FileStatus, staged: bool) -> LoadedDiff {
    use crate::files::comparison::{compare, Format, Text};
    let patch = |note| LoadedDiff::Patch(git::diff(root, file, staged), note);
    let versions = match git::versions(root, file, staged, GIT_SIDES_LIMIT) {
        Ok(v) if v.old != v.new => v,
        // nothing to show side by side: the patch says so
        Ok(_) => return patch(None),
        Err(why) => return patch(Some(why)),
    };
    let text = |t: &str| Text {
        endings: None,
        rope: Rope::from_str(t),
        format: Format { line_ending: "\n", bom: false, encoding: Default::default() },
    };
    let stop = std::sync::atomic::AtomicBool::new(false);
    match compare(&text(&versions.old), &text(&versions.new), &stop) {
        Ok(pair) => match crate::editor::highlights_of(&pair, &stop) {
            Some(highlights) => LoadedDiff::Sides(versions, pair, highlights),
            None => patch(None),
        },
        Err(e) => patch(Some(e.to_string())),
    }
}

fn diff_view(text: &str, cx: &mut Context<Workspace>) -> Entity<Editor> {
    cx.new(|cx| {
        let mut s = EditorState::new();
        s.insert(text);
        s.doc.read_only = true;
        s.doc.scratch = true;
        s.doc.dirty = false;
        let mut e = Editor::new(s, false, cx);
        e.input_enabled = false;
        e.view.minimap = false;
        e.view.gutter = false;
        e.highlighter = Highlighter::new(Some(Path::new("changes.diff")));
        e
    })
}

impl Workspace {
    pub(crate) fn git_sidebar_width(&self, window: &Window) -> f32 {
        if !self.git.sidebar_open {
            return 0.;
        }
        let left =
            if self.sidebar.visible { self.sidebar.drawn_width(f32::from(window.viewport_size().width)) } else { 0. };
        docks::drawn(
            self.panel_places.git.width,
            f32::from(window.viewport_size().width) - left - docks::EDITOR_MIN_W,
            240.,
        )
    }

    pub(crate) fn drag_git_sidebar(
        &mut self,
        e: &DragMoveEvent<GitSidebarDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel_places.git.width = f32::from(e.bounds.right() - e.event.position.x);
        cx.notify();
        autosave(cx);
    }

    fn git_setup_directories(&self, cx: &App) -> Vec<PathBuf> {
        let mut dirs: Vec<_> = crate::config::project::present(&self.roots);
        if dirs.is_empty() {
            if let Some(p) = self
                .active_editor()
                .and_then(|e| e.read(cx).doc(cx).path.clone())
                .and_then(|p| p.parent().map(Path::to_path_buf))
            {
                dirs.push(p);
            }
        }
        dirs.retain(|p| !self.git.repositories.iter().any(|r| p.starts_with(&r.root)));
        dirs
    }

    pub(crate) fn git_init(&mut self, _: &GitInit, w: &mut Window, cx: &mut Context<Self>) {
        let dirs = self.git_setup_directories(cx);
        match dirs.as_slice() {
            [] => self.status_message("Open a folder without a repository to set up Git", cx),
            [path] => self.git_init_at(&GitInitAt { path: path.clone() }, w, cx),
            _ => self.open_quick_panel(
                Kind::Commands,
                dirs.into_iter()
                    .map(|path| {
                        quick_panel::Item::command(
                            path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                            tilde(&path),
                            GitInitAt { path },
                        )
                    })
                    .collect(),
                w,
                cx,
            ),
        }
    }

    pub(crate) fn git_init_at(&mut self, a: &GitInitAt, _: &mut Window, cx: &mut Context<Self>) {
        if pipelines::writing_under(&a.path, cx) {
            return self.status_message("Wait for folder writes to finish", cx);
        }
        self.git.sidebar_open = true;
        let path = a.path.clone();
        self.git_run(path.clone(), move || git::init(&path), cx);
    }

    pub(crate) fn render_git_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.git.sidebar_open {
            return None;
        }
        let focus = self.git.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let width = self.git_sidebar_width(window);
        let mut list = div()
            .id("git-files")
            .track_scroll(&self.git.scroll)
            .w_full()
            .max_h(px(260.))
            .min_h(px(48.))
            .flex_shrink_0()
            .overflow_y_scroll()
            .pb(px(4.));
        let rows = self.git_entries();
        for staged in [false, true] {
            let group: Vec<_> =
                (rows.iter()).filter(|row| row.2 == staged).map(|(r, f, _)| (r.clone(), f.clone())).collect();
            list = list.child(
                div()
                    .h(px(26.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(kit::heading(match staged {
                        true => "STAGED CHANGES",
                        false => "CHANGES",
                    }))
                    .child(div().text_size(px(10.)).text_color(ui(theme::TEXT_MUTED)).child(group.len().to_string())),
            );
            if group.is_empty() {
                list = list.child(
                    div().px(px(12.)).h(px(22.)).flex().items_center().text_color(ui(theme::TEXT_MUTED)).child(
                        match staged {
                            true => "Nothing staged",
                            false => "No changes",
                        },
                    ),
                );
            }
            for (i, (root, file)) in group.into_iter().enumerate() {
                let selected = self.git_selected(&root, &file.path) && self.git.staged == staged;
                // the side of the status this list shows: the index's, or the work tree's
                let letter = match () {
                    _ if file.is_conflicted() => '!',
                    _ if file.is_untracked() => 'U',
                    _ if staged => file.index,
                    _ => file.worktree,
                };
                let tint = match letter {
                    '!' | 'D' => theme::GIT_CONFLICT,
                    'U' | 'A' => theme::GIT_ADDED,
                    _ => theme::GIT_MODIFIED,
                };
                let name = (file.path.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.path.display().to_string());
                let folder = (file.path.parent()).map(|p| p.display().to_string()).unwrap_or_default();
                let path = file.path;
                list = list.child(
                    div()
                        .id((if staged { "git-staged-file" } else { "git-working-file" }, i))
                        .h(px(22.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .cursor_pointer()
                        .hover(|s| s.bg(ui(theme::SIDEBAR_HOVER_BG)))
                        .when(selected, |d| d.bg(ui(theme::SIDEBAR_SELECTED_BG)))
                        .child(
                            div()
                                .w(px(10.))
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(ui(tint))
                                .child(letter.to_string()),
                        )
                        .child(div().flex_shrink_0().child(name))
                        .child(div().min_w_0().overflow_hidden().text_color(ui(theme::TEXT_MUTED)).child(folder))
                        .on_click(cx.listener(move |ws, e: &ClickEvent, w, cx| {
                            ws.git.selected = Some((root.clone(), path.clone()));
                            ws.git.staged = staged;
                            ws.git.selection_active = true;
                            if let Some(f) = &ws.git.focus {
                                w.focus(f);
                            }
                            if e.click_count() == 2 {
                                ws.git_open(&GitOpen, w, cx);
                            } else {
                                ws.load_git_diff(cx);
                            }
                        })),
                );
            }
        }
        let mut setup = div().px(px(12.)).flex_shrink_0();
        let setup_directories = self.git_setup_directories(cx);
        let needs_folder = setup_directories.is_empty() && self.roots.is_empty() && self.git.repositories.is_empty();
        // a note over the button that acts on it
        let offer = |py: f32, note: String, button: AnyElement| {
            div()
                .py(px(py))
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.))
                .child(div().text_color(ui(theme::TEXT_MUTED)).child(note))
                .child(button)
        };
        for (i, path) in setup_directories.into_iter().enumerate() {
            let note = format!("{} has no Git repository", path.file_name().unwrap_or_default().to_string_lossy());
            let set_up = button("Set Up Git", Self::git_init_at, GitInitAt { path }, cx);
            setup = setup.child(offer(10., note, set_up).id(("git-setup-folder", i)));
        }
        if needs_folder {
            let open = button("Open Folder…", Self::add_folder, AddFolder, cx);
            setup = setup.child(offer(12., "Open a project folder to use Git.".into(), open));
        }
        let header = div()
            .h(px(32.))
            .pl(px(12.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(2.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(ui(theme::BORDER_VARIANT))
            .child(div().flex_1().text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Git"))
            .child(icon_action("Refresh", Icon::Refresh, "Refresh", Self::git_refresh, GitRefresh, cx))
            .child(icon_action("×", Icon::Close, "Close Git Panel", Self::show_git, ShowGit, cx));
        let repos =
            div().px(px(12.)).pt(px(8.)).pb(px(4.)).flex_shrink_0().children(self.git.repositories.iter().map(|r| {
                div()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(icon(Icon::GitBranch, 13.).text_color(ui(theme::ICON_MUTED)))
                    .child(div().font_weight(gpui::FontWeight::MEDIUM).child(r.branch.clone()))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_color(ui(theme::TEXT_MUTED))
                            .child(r.root.file_name().unwrap_or_default().to_string_lossy().into_owned()),
                    )
            }));
        let actions = div()
            .pl(px(8.))
            .pr(px(10.))
            .py(px(6.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(2.))
            .flex_shrink_0()
            .border_t_1()
            .border_b_1()
            .border_color(ui(theme::BORDER_VARIANT))
            .child(icon_action("Stage", Icon::Plus, "Stage  ␣", Self::git_stage, GitStage, cx))
            .child(icon_action("Unstage", Icon::Minus, "Unstage  ⇧␣", Self::git_unstage, GitUnstage, cx))
            .child(icon_action("Open", Icon::OpenExternal, "Open File  ↩", Self::git_open, GitOpen, cx))
            .child(div().flex_1())
            .child(button("Branch…", Self::git_switch_branch, GitSwitchBranch, cx))
            .child(div().w(px(4.)))
            .child(button("Commit…", Self::git_commit, GitCommit, cx))
            .child(button("Amend…", Self::git_amend, GitAmend, cx));
        let edge = div()
            .id("git-sidebar-edge")
            .absolute()
            .left(px(-theme::DIVIDER_HIT / 2.))
            .top_0()
            .bottom_0()
            .w(px(theme::DIVIDER_HIT))
            .occlude()
            .cursor(CursorStyle::ResizeLeftRight)
            .on_drag(GitSidebarDrag, |_, _, _, cx| cx.new(|_| EmptyView));
        Some(
            div()
                .id("git-sidebar")
                .debug_selector(|| "git-sidebar".into())
                .relative()
                .w(px(width))
                .h_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .min_h_0()
                .bg(ui(theme::SIDEBAR_BG))
                .text_color(ui(theme::SIDEBAR_FG))
                .text_size(px(12.))
                .border_l_1()
                .border_color(ui(theme::BORDER))
                .key_context("GitPanel")
                .track_focus(&focus)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|ws, _: &gpui::MouseDownEvent, window, cx| {
                        ws.git.selection_active = true;
                        let viewers = ws.git.diff.as_ref().map(GitDiff::viewers);
                        if !(viewers.into_iter().flatten()).any(|v| v.read(cx).is_focused(window)) {
                            if let Some(f) = &ws.git.focus {
                                window.focus(f);
                            }
                        }
                    }),
                )
                .child(header)
                .child(repos)
                .child(setup)
                .when_some(self.git.message.clone(), |d, m| {
                    d.child(div().px(px(12.)).py(px(6.)).text_color(ui(theme::TEXT_MUTED)).child(m))
                })
                // a repository whose own settings run programs waits for the user's trust
                .children(self.git.paused.iter().enumerate().map(|(i, (root, why))| {
                    let path = root.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .px(px(12.))
                        .py(px(6.))
                        .child(div().text_color(ui(theme::TEXT_MUTED)).child(why.clone()))
                        .child(
                            kit::button(("git-trust", i), "Trust Folder")
                                .debug_selector(move || format!("git-trust-{i}"))
                                .tooltip(kit::tooltip(format!("Trust {} and run its programs", root.display()).into()))
                                .on_click(cx.listener(move |ws, _: &ClickEvent, w, cx| {
                                    ws.trust_folder(&TrustFolder { path: path.clone() }, w, cx)
                                })),
                        )
                }))
                .when(!self.git.repositories.is_empty(), |d| {
                    d.child(list)
                        .child(actions)
                        .child(
                            div()
                                .flex()
                                .gap(px(4.))
                                .px(px(8.))
                                .flex_wrap()
                                .child(button("Local Tags…", Self::git_tags, GitTags, cx))
                                .child(button("Create Tag…", Self::git_create_tag, GitCreateTag, cx))
                                .child(button("Stashes…", Self::git_stashes, GitStashes, cx))
                                .child(button("Create Stash…", Self::git_create_stash, GitCreateStash, cx)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .px(px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap(px(4.))
                                        .child(button("Fetch…", Self::git_fetch, GitFetch, cx))
                                        .child(button("Pull…", Self::git_pull, GitPull, cx))
                                        .child(button("Push…", Self::git_push, GitPush, cx))
                                        .child(button("Remotes…", Self::git_remotes, GitRemotes, cx)),
                                )
                                .child(button("Worktrees…", Self::git_worktrees, GitWorktrees, cx))
                                .child(button("Create Worktree…", Self::git_create_worktree, GitCreateWorktree, cx))
                                .child(button("Prune…", Self::git_prune_worktrees, GitPruneWorktrees, cx)),
                        )
                        .child(self.git_selection_actions(cx))
                        .child(
                            div()
                                .px(px(12.))
                                .h(px(26.))
                                .flex()
                                .items_center()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(ui(theme::TEXT_MUTED))
                                .child(match (self.git.staged, &self.git.diff_note) {
                                    (true, None) => "Staged diff".to_string(),
                                    (false, None) => "Working tree diff · saved contents".into(),
                                    (true, Some(why)) => format!("Staged patch · {why}"),
                                    (false, Some(why)) => format!("Working tree patch · {why}"),
                                }),
                        )
                        .child(div().flex_1().min_h_0().when_some(self.git.diff.as_ref(), |d, e| d.child(e.render(cx))))
                })
                .child(edge)
                .into_any_element(),
        )
    }

    pub(crate) fn show_change(&mut self, _: &ShowChange, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = self.active_editor() {
            self.show_change_for(e, None, window, cx);
        }
    }

    pub(crate) fn show_change_for(
        &mut self,
        editor: Entity<Editor>,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let e = editor.read(cx);
        if e.changes_version != e.doc(cx).version || !self.git_base_current(&editor, cx) {
            self.refresh_git(cx);
            let task = cx.spawn_in(window, async move |this, cx| {
                for _ in 0..150 {
                    cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
                    let done = this
                        .update_in(cx, |ws, window, cx| {
                            if ws.active_editor().as_ref() != Some(&editor) {
                                return true;
                            }
                            let ready = ws.git_base_current(&editor, cx);
                            if ready {
                                ws.show_change_for(editor.clone(), index, window, cx);
                            } else {
                                ws.refresh_git(cx);
                            }
                            ready
                        })
                        .unwrap_or(true);
                    if done {
                        return;
                    }
                }
                this.update(cx, |ws, cx| ws.status_message("Changes could not be loaded; refresh and try again", cx))
                    .ok();
            });
            self.git.pending_change = Some(task);
            return;
        }
        let Some(base) = self.git.bases.get(&editor.entity_id()) else {
            return;
        };
        let line = e.doc(cx).point(e.state.cursor()).0;
        let i = index
            .or_else(|| e.changes.iter().position(|h| h.new_lines.start <= line && line <= h.new_lines.end))
            .unwrap_or(0);
        let Some(hunk) = e.changes.get(i).cloned() else {
            return self.status_message("No changes", cx);
        };
        let old = base.base.slice(hunk.old_chars.clone()).to_string();
        let new = e.doc(cx).rope.slice(hunk.new_chars.clone()).to_string();
        let text = format!(
            "--- {}\n+++ Buffer\n@@ -{},{} +{},{} @@\n{}{}",
            base.label,
            hunk.old_lines.start + 1,
            hunk.old_lines.len(),
            hunk.new_lines.start + 1,
            hunk.new_lines.len(),
            prefixed(&old, '-'),
            prefixed(&new, '+')
        );
        let version = e.doc(cx).version;
        let path = e.doc(cx).path.clone();
        let baseline = base.base.clone();
        let viewer = diff_view(&text, cx);
        viewer.read(cx).focus(window);
        self.show_panel(Panel::Changes(ChangePanel { source: editor, path, baseline, version, hunk, old, viewer }), cx);
    }

    /// Whether the baseline read for `editor` is of its document as it is now.
    fn git_base_current(&self, editor: &Entity<Editor>, cx: &App) -> bool {
        let doc = editor.read(cx).doc(cx);
        (self.git.bases.get(&editor.entity_id())).is_some_and(|b| b.version == doc.version && b.path == doc.path)
    }

    fn navigate_change(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let e = editor.read(cx);
        if e.changes_version != e.doc(cx).version {
            return self.show_change_for(editor.clone(), None, window, cx);
        }
        let line = e.doc(cx).point(e.state.cursor()).0;
        let i = if forward {
            e.changes.iter().position(|h| h.new_lines.start > line).or((!e.changes.is_empty()).then_some(0))
        } else {
            e.changes.iter().rposition(|h| h.new_lines.start < line).or_else(|| e.changes.len().checked_sub(1))
        };
        let Some(i) = i else {
            return self.status_message("No changes", cx);
        };
        let at = e.changes[i].new_chars.start.min(e.doc(cx).len());
        editor.update(cx, |e, cx| e.jump_to(at, cx));
        self.show_change_for(editor, Some(i), window, cx);
    }

    forward_actions! {
    pub(crate) next_change(NextChange, w, cx) => navigate_change(true, w, cx);
    pub(crate) previous_change(PreviousChange, w, cx) => navigate_change(false, w, cx);
    pub(crate) accept_ours(AcceptOurs, _, cx) => accept_conflict(git::ConflictChoice::Ours, cx);
    pub(crate) accept_theirs(AcceptTheirs, _, cx) => accept_conflict(git::ConflictChoice::Theirs, cx);
    pub(crate) accept_both(AcceptBoth, _, cx) => accept_conflict(git::ConflictChoice::Both, cx);
    }

    pub(crate) fn revert_change(&mut self, _: &RevertChange, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Panel::Changes(p)) = &self.panel else {
            return self.status_message("Review a change before reverting it", cx);
        };
        let s = p.source.read(cx).doc(cx);
        if s.version != p.version
            || s.path != p.path
            || self.views.find(&p.source).is_none()
            || !self.git.bases.get(&p.source.entity_id()).is_some_and(|b| b.base == p.baseline)
        {
            return self.status_message("The buffer changed; review the change again before reverting", cx);
        }
        let (source, range, old) = (p.source.clone(), p.hunk.new_chars.clone(), p.old.clone());
        source.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.replace_ranges(vec![(range, old)]);
            })
        });
        self.close_panel(&ClosePanel, window, cx);
        self.refresh_git(cx);
    }

    pub(crate) fn render_change_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(Panel::Changes(p)) = &self.panel else {
            return div().into_any_element();
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(ui(theme::PANEL_BG))
            .child(
                kit::panel_header()
                    .child(icon(Icon::GitCommit, 14.).text_color(ui(theme::ICON_MUTED)))
                    .child("Change")
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(ui(theme::TEXT_MUTED))
                            .child("buffer versus baseline"),
                    )
                    .child(icon_action(
                        "Previous",
                        Icon::ChevronUp,
                        "Previous Change",
                        Self::previous_change,
                        PreviousChange,
                        cx,
                    ))
                    .child(icon_action("Next", Icon::ChevronDown, "Next Change", Self::next_change, NextChange, cx))
                    .child(icon_action("Revert Hunk", Icon::Undo, "Revert Hunk", Self::revert_change, RevertChange, cx))
                    .child(icon_action("×", Icon::Close, "Close Panel", Self::close_panel, ClosePanel, cx)),
            )
            .child(p.viewer.clone())
            .into_any_element()
    }

    pub(crate) fn next_conflict(&mut self, _: &NextConflict, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let s = editor.read(cx).read_state(cx);
        let text = s.text();
        let at = s.doc.rope.char_to_byte(s.cursor());
        let blocks = git::conflicts(&text);
        let Some(block) = blocks.iter().find(|b| b.range.start > at).or_else(|| blocks.first()) else {
            return self.status_message("No complete merge conflict found", cx);
        };
        let at = s.doc.rope.byte_to_char(block.range.start);
        editor.update(cx, |e, cx| e.jump_to(at, cx));
        editor.read(cx).focus(window);
    }

    fn accept_conflict(&mut self, choice: git::ConflictChoice, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let s = editor.read(cx).read_state(cx);
        let text = s.text();
        let at = s.doc.rope.char_to_byte(s.cursor());
        let blocks = git::conflicts(&text);
        let Some(i) = blocks.iter().position(|b| b.range.contains(&at)) else {
            return self.status_message("Place the caret inside the conflict to resolve", cx);
        };
        let block = &blocks[i];
        let resolved = match git::resolve_conflict(&text, i, choice) {
            Ok(text) => text,
            Err(e) => return self.status_message(e, cx),
        };
        let replacement = resolved[block.range.start..resolved.len() - (text.len() - block.range.end)].to_string();
        let range = s.doc.rope.byte_to_char(block.range.start)..s.doc.rope.byte_to_char(block.range.end);
        editor.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.replace_ranges(vec![(range, replacement)]);
            })
        });
        self.status_message("Conflict resolved in buffer; save, then stage the file", cx);
    }
}

fn prefixed(text: &str, prefix: char) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            format!("{prefix}{line}{}", if line.ends_with('\n') { "" } else { "\n\\ No newline at end of file\n" })
        })
        .collect()
}

/// A text button running `handler`; tests find it by its label.
fn button<A: Action + Clone>(
    label: &'static str,
    handler: fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>),
    action: A,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    kit::button(label, label)
        .debug_selector(move || label.to_string())
        .on_click(cx.listener(move |ws, _: &ClickEvent, w, cx| handler(ws, &action, w, cx)))
        .into_any_element()
}

/// An icon button running `handler`, named by `tip`; tests find it by `id`.
fn icon_action<A: Action + Clone>(
    id: &'static str,
    i: Icon,
    tip: &'static str,
    handler: fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>),
    action: A,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    kit::icon_button(id, i, tip)
        .debug_selector(move || id.to_string())
        .on_click(cx.listener(move |ws, _: &ClickEvent, w, cx| handler(ws, &action, w, cx)))
        .into_any_element()
}

/// The status bar's branch for a folder in a git repo (or inside one: it walks up): HEAD's
/// branch, or its short hash when detached, as Sublime shows it. `.git` may be a file naming the
/// git dir (worktrees, submodules).
pub(crate) fn git_branch(folder: &std::path::Path) -> Option<String> {
    let dot = folder.ancestors().map(|d| d.join(".git")).find(|g| g.exists())?;
    let git = match dot.is_file() {
        true => {
            let link = std::fs::read_to_string(&dot).ok()?;
            dot.parent()?.join(link.trim().strip_prefix("gitdir:")?.trim())
        }
        false => dot,
    };
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => Some(branch.to_string()),
        // Sublime 4215 shows 8 hex digits
        None => (head.len() >= 8 && head.bytes().all(|b| b.is_ascii_hexdigit())).then(|| head[..8].to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{self, text, window};
    use gpui::TestAppContext;

    fn temp(name: &str) -> PathBuf {
        let p = ui_tests::temp(&format!("orlo-git-ui-{name}-{}", std::process::id()));
        crate::editing::text::resolve(&p)
    }
    pub(super) fn git(root: &Path, args: &[&str]) {
        let result = std::process::Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    }
    #[gpui::test]
    fn a_repository_running_its_own_programs_waits_in_the_sidebar_until_trusted(cx: &mut TestAppContext) {
        let root = repo("trust");
        git(&root, &["config", "filter.evil.clean", "cat"]);
        orlo_utils::trust::TRUST_ALL.set(false);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.add_root(root.clone(), w, cx);
            ws.refresh_git(cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.git.repositories.is_empty(), "no Git ran");
            assert_eq!(ws.git.paused.iter().map(|(r, _)| r).collect::<Vec<_>>(), [&root]);
            assert!(ws.git.paused[0].1.contains("filter.evil.clean"), "{}", ws.git.paused[0].1);
        });
        ws.update_in(cx, |ws, w, cx| ws.trust_folder(&TrustFolder { path: root.clone() }, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.git.paused.is_empty());
            assert_eq!(ws.git.repositories.len(), 1, "trusted, its Git runs");
        });
        orlo_utils::trust::TRUST_ALL.set(true);
    }

    pub(super) fn repo(name: &str) -> PathBuf {
        let p = temp(name);
        git(&p, &["init", "-q", "-b", "main"]);
        git(&p, &["config", "core.hooksPath", "/dev/null"]);
        git(&p, &["config", "commit.gpgsign", "false"]);
        git(&p, &["config", "user.name", "Orlo Test"]);
        git(&p, &["config", "user.email", "orlo@example.invalid"]);
        std::fs::write(p.join("file.txt"), "one\ntwo\nthree\n").unwrap();
        git(&p, &["add", "."]);
        commit(&p, "-qm", "initial");
        p
    }
    fn commit(root: &Path, flags: &str, message: &str) {
        git(root, &["-c", "commit.gpgsign=false", "commit", flags, message]);
    }
    fn index(root: &Path) -> String {
        git::index_text(root, Path::new("file.txt")).unwrap().unwrap()
    }
    /// Choose `label` in the command palette.
    pub(super) fn run_command(ws: &mut Workspace, label: &str, w: &mut Window, cx: &mut Context<Workspace>) {
        ws.command_palette(&CommandPalette, w, cx);
        let p = ws.quick_panel.as_mut().unwrap();
        p.selected = (p.filtered.iter()).position(|i| p.items[*i].label == label).unwrap();
        ws.confirm_quick_panel(w, cx);
    }
    fn picker_title(ws: &Workspace) -> &str {
        ws.quick_panel.as_ref().unwrap().title.as_ref().unwrap()
    }

    fn commit_fields(ws: &Workspace) -> Vec<Entity<Editor>> {
        let Some(Panel::Form(p)) = &ws.panel else {
            panic!(
                "commit form missing; Git message {:?}, quick panel {:?}, status {:?}",
                ws.git.message,
                ws.quick_panel.as_ref().map(|p| (&p.title, &p.progress)),
                ws.status.as_ref().map(|s| &s.0)
            )
        };
        p.inputs().cloned().collect()
    }

    #[gpui::test]
    fn composition_form_prefills_amend_and_retains_cancelled_multiline_draft(cx: &mut TestAppContext) {
        let root = repo("composition-cancel");
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Amend HEAD…", w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let fields = commit_fields(ws);
            assert_eq!(fields[0].read(cx).text(cx), "initial");
            fields[0].update(cx, |e, cx| e.set_text("界 amended", cx));
            fields[1].update(cx, |e, cx| e.set_text("body 🦀", cx));
            fields[1].read(cx).focus(w);
        });
        cx.simulate_keystrokes("end enter");
        ws.update_in(cx, |ws, _, cx| {
            assert_eq!(commit_fields(ws)[1].read(cx).text(cx), "body 🦀\n");
        });
        cx.simulate_keystrokes("escape");
        ws.update_in(cx, |ws, w, cx| ws.git_amend(&GitAmend, w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let fields = commit_fields(ws);
            assert_eq!(fields[0].read(cx).text(cx), "界 amended");
            assert_eq!(fields[1].read(cx).text(cx), "body 🦀\n");
            fields[2].update(cx, |e, cx| e.set_text("Zoë <zoe@example.invalid>", cx));
            fields[1].read(cx).focus(w);
        });
        cx.simulate_keystrokes("cmd-enter");
        cx.run_until_parked();
        let prepared = git::prepare_commit(&root, true).unwrap();
        assert_eq!(prepared.draft.subject, "界 amended");
        assert!(prepared.draft.body.contains("Co-authored-by: Zoë <zoe@example.invalid>"));
        ws.read_with(cx, |ws, _| assert!(ws.panel.is_none()));
    }

    #[gpui::test]
    fn composition_form_refuses_stale_head_and_keeps_validation_errors(cx: &mut TestAppContext) {
        let root = repo("composition-stale");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_commit(&GitCommit, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            ws.submit_form(&form::Submit, w, cx);
            let Some(Panel::Form(p)) = &ws.panel else { panic!() };
            assert!(p.error.as_ref().unwrap().contains("subject"));
            commit_fields(ws)[0].update(cx, |e, cx| e.set_text("retained subject", cx));
        });
        git(&root, &["switch", "-qc", "same-object"]);
        ws.update_in(cx, |ws, w, cx| ws.submit_form(&form::Submit, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(commit_fields(ws)[0].read(cx).text(cx), "retained subject");
            assert!(ws.git.message.as_ref().is_some_and(|m| m.contains("changed")));
        });
    }

    #[gpui::test]
    fn composition_cancelled_or_replaced_async_load_never_opens_form(cx: &mut TestAppContext) {
        for change in [0, 1, 2, 3] {
            let root = repo(&format!("composition-loading-{change}"));
            let other = repo(&format!("composition-other-{change}"));
            let (ws, cx) = window(cx);
            ws.update_in(cx, |ws, w, cx| {
                ws.open_path(root.join("file.txt"), w, cx);
                ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
                ws.git_amend(&GitAmend, w, cx);
                match change {
                    0 => ws.cancel_quick_panel(cx),
                    1 => ws.restore_gen += 1,
                    2 => {
                        ws.open_path(other.join("file.txt"), w, cx);
                        ws.git.repositories = vec![git::discover(&other).unwrap().unwrap()];
                    }
                    _ => {
                        ws.open_quick_panel(Kind::Choices, Vec::new(), w, cx);
                        ws.quick_panel.as_mut().unwrap().title = Some("Replacement".into());
                    }
                }
            });
            cx.run_until_parked();
            ws.read_with(cx, |ws, _| {
                assert!(!matches!(ws.panel, Some(Panel::Form(_))));
                if change == 3 {
                    assert_eq!(ws.quick_panel.as_ref().unwrap().title.as_deref(), Some("Replacement"));
                } else {
                    assert!(ws.quick_panel.is_none(), "stale loading must not leave a permanent spinner");
                }
            });
        }
    }

    #[gpui::test]
    fn composition_keeps_edits_made_while_git_runs_and_restores_focus(cx: &mut TestAppContext) {
        let root = repo("composition-during-commit");
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_amend(&GitAmend, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let fields = commit_fields(ws);
            fields[0].update(cx, |e, cx| e.set_text("submitted", cx));
            ws.submit_form(&form::Submit, w, cx);
            fields[0].update(cx, |e, cx| e.set_text("new draft typed during commit", cx));
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(commit_fields(ws)[0].read(cx).text(cx), "new draft typed during commit");
        });
        assert_eq!(git::prepare_commit(&root, true).unwrap().draft.subject, "submitted");
    }

    #[gpui::test]
    fn composition_hook_failure_keeps_exact_fields_and_can_retry(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;
        let root = repo("composition-hook-failure");
        let hooks = root.join("fixture-hooks");
        std::fs::create_dir(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho 'fixture hook refused' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
        git(&root, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_amend(&GitAmend, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let fields = commit_fields(ws);
            fields[0].update(cx, |e, cx| e.set_text("recovery 界", cx));
            fields[1].update(cx, |e, cx| e.set_text("body\nexact 🦀\n", cx));
            fields[2].update(cx, |e, cx| e.set_text("Zoë <zoe@example.invalid>", cx));
            ws.submit_form(&form::Submit, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let fields = commit_fields(ws);
            assert_eq!(fields[1].read(cx).text(cx), "body\nexact 🦀\n");
            assert_eq!(fields[2].read(cx).text(cx), "Zoë <zoe@example.invalid>");
            let Some(Panel::Form(p)) = &ws.panel else { unreachable!() };
            assert!(p.error.as_ref().unwrap().contains("fixture hook refused"));
            assert!(!ws.git.operation);
            ws.close_panel(&ClosePanel, w, cx);
        });
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        ws.update_in(cx, |ws, w, cx| ws.git_amend(&GitAmend, w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(commit_fields(ws)[0].read(cx).text(cx), "recovery 界");
            ws.submit_form(&form::Submit, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert!(ws.panel.is_none());
            assert!(ws.active_editor().unwrap().read(cx).is_focused(w));
        });
        assert_eq!(git::prepare_commit(&root, true).unwrap().draft.subject, "recovery 界");
    }

    #[gpui::test]
    fn composition_open_form_refuses_project_and_repository_changes(cx: &mut TestAppContext) {
        for project in [true, false] {
            let root = repo(&format!("composition-form-context-{project}"));
            let other = repo(&format!("composition-form-other-{project}"));
            let (ws, cx) = window(cx);
            ws.update_in(cx, |ws, w, cx| {
                ws.open_path(root.join("file.txt"), w, cx);
                ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
                ws.git_amend(&GitAmend, w, cx);
            });
            cx.run_until_parked();
            ws.update_in(cx, |ws, w, cx| {
                if project {
                    ws.restore_gen += 1;
                } else {
                    ws.open_path(other.join("file.txt"), w, cx);
                    ws.git.repositories = vec![git::discover(&other).unwrap().unwrap()];
                }
                ws.submit_form(&form::Submit, w, cx);
                let Some(Panel::Form(p)) = &ws.panel else { panic!() };
                assert!(p.error.as_ref().unwrap().contains("project or repository changed"));
                assert!(!ws.git.operation);
            });
            assert_eq!(git::prepare_commit(&root, true).unwrap().draft.subject, "initial");
            assert_eq!(git::prepare_commit(&other, true).unwrap().draft.subject, "initial");
        }
    }

    #[gpui::test]
    fn composition_unborn_amend_refuses_but_ordinary_commit_stays_staged_only(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let root = temp("composition-unborn");
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "user.name", "Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        let path = root.join("file.txt");
        std::fs::write(&path, "staged\n").unwrap();
        git(&root, &["add", "file.txt"]);
        std::fs::write(&path, "disk unstaged\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(path.clone(), w, cx);
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_amend(&GitAmend, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert!(ws.panel.is_none());
            assert!(ws.status.as_ref().unwrap().0.contains("HEAD has no commit"));
            ws.git_commit(&GitCommit, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let fields = commit_fields(ws);
            assert!(fields[0].read(cx).text(cx).is_empty());
            fields[0].update(cx, |e, cx| e.set_text("initial 界", cx));
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| {
                e.sel(cx).select_all();
                e.replace_text_in_range(None, "unsaved editor text", w, cx);
                e.update_doc(cx, |d| d.read_only = true);
            });
            ws.submit_form(&form::Submit, w, cx);
        });
        cx.run_until_parked();
        assert_eq!(git::head_text(&root, Path::new("file.txt")).unwrap().unwrap(), "staged\n");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "disk unstaged\n");
        assert_eq!(text(&ws, cx), "unsaved editor text");
        ws.read_with(cx, |ws, cx| {
            let views = ws.open_editors(cx);
            assert_eq!(views.len(), 2);
            assert!(views.iter().all(|e| e.read(cx).text(cx) == "unsaved editor text"));
        });
        ws.update_in(cx, |ws, w, cx| {
            ws.with_active(cx, |e, cx| e.perform_action(&crate::editor::Undo, w, cx));
        });
        assert_eq!(text(&ws, cx), "disk unstaged\n");
        ws.update_in(cx, |ws, w, cx| {
            ws.with_active(cx, |e, cx| e.perform_action(&crate::editor::Redo, w, cx));
        });
        assert_eq!(text(&ws, cx), "unsaved editor text");
    }

    #[gpui::test]
    fn patch_viewer_rejects_user_input_and_mutating_commands(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;

        let original = "--- Index\n+++ Buffer\n-old\n+new\n";
        let (ws, cx) = window(cx);
        let viewer = ws.update(cx, |_, cx| diff_view(original, cx));
        viewer.update_in(cx, |e, window, cx| {
            e.sel(cx).select_all();
            e.replace_text_in_range(None, "replacement", window, cx);
            e.replace_and_mark_text_in_range(None, "composed", None, window, cx);
            e.perform_action(&crate::editor::Delete, window, cx);
            assert_eq!(e.text(cx), original);
        });
    }

    #[test]
    fn badges_propagate_to_folders_without_leaking_to_sibling_roots() {
        let repo = git::Repository {
            root: "/repo".into(),
            branch: "main".into(),
            files: vec![
                git::FileStatus { path: "src/a".into(), original_path: None, index: 'M', worktree: 'M' },
                git::FileStatus { path: "src/b".into(), original_path: None, index: '?', worktree: '?' },
                git::FileStatus { path: "conflict".into(), original_path: None, index: 'U', worktree: 'U' },
            ],
        };
        let badges = badges(&[repo]);
        assert_eq!(badges[Path::new("/repo/src/a")], "SM");
        assert_eq!(badges[Path::new("/repo/src")], "SM?");
        assert_eq!(badges[Path::new("/repo")], "!SM?");
        assert!(!badges.contains_key(Path::new("/")));
    }

    #[gpui::test]
    fn saved_file_changes_review_revert_and_undo_without_git(cx: &mut TestAppContext) {
        let p = temp("plain").join("file.txt");
        std::fs::write(&p, "one\nold\nthree\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(p.clone(), w, cx);
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.replace_ranges(vec![(4..7, "new".into())]);
                })
            });
            ws.refresh_git(cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).changes.len(), 1);
            ws.show_change_for(e.clone(), Some(0), w, cx);
            let Some(Panel::Changes(p)) = &ws.panel else { panic!("change review") };
            assert!(p.viewer.read(cx).doc(cx).text().contains("-old\n+new\n"));
            ws.revert_change(&RevertChange, w, cx);
            assert_eq!(e.read(cx).doc(cx).text(), "one\nold\nthree\n");
            e.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.undo();
                })
            });
            assert_eq!(e.read(cx).doc(cx).text(), "one\nnew\nthree\n");
        });
    }

    #[gpui::test]
    fn stale_review_cannot_revert_newer_edits(cx: &mut TestAppContext) {
        let p = temp("stale").join("file.txt");
        std::fs::write(&p, "old\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(p, w, cx);
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.replace_ranges(vec![(0..3, "new".into())]);
                })
            });
            ws.refresh_git(cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            ws.show_change(&ShowChange, w, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("later")));
            ws.revert_change(&RevertChange, w, cx);
            assert!(ws.status.as_ref().unwrap().0.contains("buffer changed"));
        });
        assert!(text(&ws, cx).contains("later"));
    }

    #[gpui::test]
    fn delayed_change_review_does_not_follow_the_user_to_another_tab(cx: &mut TestAppContext) {
        let p = temp("review-tab-switch").join("file.txt");
        std::fs::write(&p, "old\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(p, w, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("new")));
            ws.show_change(&ShowChange, w, cx);
            assert!(ws.git.pending_change.is_some());
            ws.new_file(&NewFile, w, cx);
        });
        cx.run_until_parked();
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.panel.is_none(), "a delayed review reopened on another tab");
        });
    }

    #[gpui::test]
    fn repository_panel_reads_index_refuses_unsaved_stage_and_keeps_focus(cx: &mut TestAppContext) {
        let root = repo("status");
        std::fs::write(root.join("file.txt"), "disk\ntwo\nthree\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.refresh_git(cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(ws.git.repositories.len(), 1);
            assert_eq!(ws.active_editor().unwrap().read(cx).changes.len(), 1);
            ws.show_git(&ShowGit, w, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved")));
            ws.git_stage(&GitStage, w, cx);
            assert!(!ws.git.operation);
            assert!(ws.git.message.as_deref().unwrap().contains("Save"));
            assert!(ws.git.sidebar_open);
            assert!(ws.git.focus.as_ref().unwrap().is_focused(w));
            ws.show_git(&ShowGit, w, cx);
            assert!(ws.active_editor().unwrap().read(cx).is_focused(w));
            ws.show_git(&ShowGit, w, cx);
            let mut texts = Texts::new();
            assert!(ws.capture(w, true, &mut texts, cx).git_sidebar_visible);
        });
    }

    #[gpui::test]
    fn conflict_choices_are_normal_undoable_buffer_edits(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let original = "before\n<<<<<<< ours\n猫\n=======\n犬\n>>>>>>> theirs\nafter\n";
        ws.update_in(cx, |ws, w, cx| {
            ws.with_active(cx, |e, cx| e.set_text(original, cx));
            ws.next_conflict(&NextConflict, w, cx);
            ws.accept_both(&AcceptBoth, w, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).text(), "before\n猫\n犬\nafter\n");
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.undo();
                })
            });
        });
        assert_eq!(text(&ws, cx), original);
    }

    /// A repository with a branch `other`, and the action switching to it.
    fn with_other_branch(name: &str) -> (PathBuf, GitUseBranch) {
        let root = repo(name);
        git(&root, &["branch", "other"]);
        let branch = "other".into();
        (root.clone(), GitUseBranch { root, branch })
    }

    #[gpui::test]
    fn branch_switch_refuses_unsaved_open_repository_buffers(cx: &mut TestAppContext) {
        let (root, switch) = with_other_branch("switch");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("draft")));
            ws.git_use_branch(&switch, w, cx);
            assert!(!ws.git.operation);
            assert!(ws.status.as_ref().unwrap().0.contains("unsaved"));
        });
        assert_eq!(git::discover(&root).unwrap().unwrap().branch, "main");
    }

    #[gpui::test]
    fn checkout_reserves_repository_until_worker_finishes(cx: &mut TestAppContext) {
        let (root, switch) = with_other_branch("checkout-lease");
        let file = root.join("file.txt");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(file.clone(), w, cx);
            ws.git_use_branch(&switch, w, cx);
            assert!(pipelines::writing(&file, cx), "checkout reserves descendants before starting");
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("draft")));
            ws.save(&Save, w, cx);
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\nthree\n");
            assert!(ws.active_editor().unwrap().read(cx).doc(cx).dirty);
        });
        cx.run_until_parked();
        ws.read_with(cx, |_, cx| assert!(!pipelines::writing(&file, cx), "lease released after checkout"));
        assert_eq!(git::discover(&root).unwrap().unwrap().branch, "other");
    }

    #[gpui::test]
    fn hidden_git_panel_does_not_target_its_old_selection(cx: &mut TestAppContext) {
        let a = repo("target-a");
        let b = repo("target-b");
        std::fs::write(a.join("file.txt"), "a\n").unwrap();
        std::fs::write(b.join("file.txt"), "b\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(a.join("file.txt"), w, cx);
            ws.open_path(b.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&a).unwrap().unwrap(), git::discover(&b).unwrap().unwrap()];
            ws.show_git(&ShowGit, w, cx);
            ws.git.selected = Some((a.clone(), "file.txt".into()));
            ws.show_git(&ShowGit, w, cx);
            assert_eq!(ws.git_target(cx).unwrap().0, b);
            assert_eq!(ws.git_root(cx), Some(b));
        });
    }

    #[gpui::test]
    fn indexed_bom_strips_one_prefix_and_keeps_literal_feff(cx: &mut TestAppContext) {
        let root = repo("bom");
        let file = root.join("file.txt");
        std::fs::write(&file, "\u{feff}\u{feff}content\r\n").unwrap();
        git(&root, &["add", "file.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(file, w, cx);
            ws.refresh_git(cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(ws.active_editor().unwrap().read(cx).changes.is_empty());
        });
    }

    #[gpui::test]
    fn git_icon_sets_up_selected_folder_and_terminal_button_toggles_bottom(cx: &mut TestAppContext) {
        let root = temp("setup-sidebar");
        std::fs::write(root.join("keep.txt"), "keep").unwrap();
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1100.), px(750.)));
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.clone(), w, cx));
        cx.run_until_parked();
        let git_button = cx.debug_bounds("git-toggle").unwrap();
        let terminal_button = cx.debug_bounds("terminal-toggle").unwrap();
        assert!(git_button.center().x > px(1050.));
        assert!(terminal_button.center().x < px(100.));
        assert!(git_button.origin.y >= px(750. - theme::STATUS_BAR_H));
        assert_eq!(git_button.origin.y, terminal_button.origin.y);
        let click = |cx: &mut gpui::VisualTestContext, id: &'static str| {
            let b = cx.debug_bounds(id).unwrap_or_else(|| panic!("missing {id}"));
            cx.simulate_click(b.center(), gpui::Modifiers::none());
            cx.run_until_parked();
        };
        click(cx, "git-toggle");
        ws.read_with(cx, |ws, _| assert!(ws.git.sidebar_open));
        click(cx, "Set Up Git");
        assert!(root.join(".git").is_dir());
        assert_eq!(std::fs::read_to_string(root.join("keep.txt")).unwrap(), "keep");
        ws.update(cx, |ws, cx| ws.refresh_git(cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert_eq!(ws.git.repositories.len(), 1));
        click(cx, "terminal-toggle");
        let terminal = ws.update_in(cx, |ws, w, _| {
            assert!(ws.git.sidebar_open);
            assert!(matches!(ws.panel, Some(Panel::Terminal(_))));
            assert_eq!(ws.panel_places.terminal.dock, session::Dock::Bottom);
            assert!(ws.git_sidebar_width(w) > 0.);
            ws.terminal.as_ref().unwrap().clone()
        });
        click(cx, "git-toggle");
        ws.read_with(cx, |ws, _| {
            assert!(!ws.git.sidebar_open);
            assert!(matches!(ws.panel, Some(Panel::Terminal(_))));
        });
        click(cx, "terminal-toggle");
        ws.read_with(cx, |ws, _| {
            assert!(ws.panel.is_none());
            assert_eq!(ws.terminal.as_ref(), Some(&terminal));
        });
        click(cx, "terminal-toggle");
        ws.read_with(cx, |ws, _| {
            assert!(matches!(ws.panel, Some(Panel::Terminal(_))));
            assert_eq!(ws.terminal.as_ref(), Some(&terminal));
        });
        ws.update_in(cx, |ws, w, cx| ws.close_terminal(&CloseTerminal, w, cx));
    }

    #[gpui::test]
    fn arrow_navigation_visits_staged_only_and_both_sides(cx: &mut TestAppContext) {
        let root = repo("staged-navigation");
        let file = root.join("file.txt");
        std::fs::write(&file, "staged\n").unwrap();
        git(&root, &["add", "file.txt"]);
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(file.clone(), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.show_git(&ShowGit, w, cx);
            assert!(ws.git.staged);
            ws.git_select(&GitSelect { step: 1 }, w, cx);
            assert!(ws.git.staged);
        });
        cx.run_until_parked();
        assert_eq!(sides(&ws, cx), texts("one\ntwo\nthree\n", "staged\n"));
        std::fs::write(file, "working\n").unwrap();
        ws.update_in(cx, |ws, w, cx| {
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_select(&GitSelect { step: 1 }, w, cx);
            assert!(!ws.git.staged);
        });
        cx.run_until_parked();
        assert_eq!(sides(&ws, cx), texts("staged\n", "working\n"));
        ws.update_in(cx, |ws, w, cx| {
            ws.git_select(&GitSelect { step: 1 }, w, cx);
            assert!(ws.git.staged);
        });
    }

    /// Show the Git panel's diff of `path` on one side of the index, and draw it.
    fn select(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext, root: &Path, path: &str, staged: bool) {
        ws.update_in(cx, |ws, w, cx| {
            if !ws.git.sidebar_open {
                ws.show_git(&ShowGit, w, cx);
            }
            ws.git.repositories = vec![git::discover(root).unwrap().unwrap()];
            ws.git.selected = Some((root.to_path_buf(), PathBuf::from(path)));
            ws.git.staged = staged;
            ws.load_git_diff(cx);
        });
        cx.run_until_parked();
    }

    /// The two versions shown side by side, or `None` for a patch.
    fn sides(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> Option<[String; 2]> {
        ws.read_with(cx, |ws, cx| match ws.git.diff.as_ref()? {
            GitDiff::Sides { viewers, .. } => Some(viewers.each_ref().map(|v| v.read(cx).text(cx))),
            GitDiff::Patch(_) => None,
        })
    }

    fn patch(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> Option<String> {
        ws.read_with(cx, |ws, cx| match ws.git.diff.as_ref()? {
            GitDiff::Patch(viewer) => Some(viewer.read(cx).text(cx)),
            GitDiff::Sides { .. } => None,
        })
    }

    fn texts(old: &str, new: &str) -> Option<[String; 2]> {
        Some([old.to_string(), new.to_string()])
    }

    #[gpui::test]
    fn partial_controls_stage_lines_and_unstage_a_hunk_without_saving_the_diff(cx: &mut TestAppContext) {
        let root = repo("partial-controls");
        let saved = "one\nTWO\nextra\nthree\n";
        std::fs::write(root.join("file.txt"), saved).unwrap();
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1200.), px(850.)));
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.join("file.txt"), w, cx));
        select(&ws, cx, &root, "file.txt", false);
        ws.update_in(cx, |ws, _, cx| {
            let Some(GitDiff::Sides { viewers, .. }) = &ws.git.diff else { panic!() };
            viewers[1].update(cx, |e, cx| e.sel(cx).set_cursor(8));
        });
        let b = cx.debug_bounds("Stage Lines").unwrap();
        cx.simulate_click(b.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(index(&root), "one\ntwo\nextra\nthree\n");
        select(&ws, cx, &root, "file.txt", true);
        ws.update_in(cx, |ws, w, cx| {
            let Some(GitDiff::Sides { viewers, .. }) = &ws.git.diff else { panic!() };
            viewers[1].update(cx, |e, cx| e.sel(cx).set_cursor(8));
            ws.git_unstage_hunk(&GitUnstageHunk, w, cx);
        });
        cx.run_until_parked();
        assert_eq!(index(&root), "one\ntwo\nthree\n");
        assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), saved);
    }

    #[gpui::test]
    fn partial_staging_refuses_unsaved_buffers_and_another_pending_diff(cx: &mut TestAppContext) {
        let root = repo("partial-guards");
        std::fs::write(root.join("file.txt"), "ONE\ntwo\nthree\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.join("file.txt"), w, cx));
        select(&ws, cx, &root, "file.txt", false);
        ws.update_in(cx, |ws, w, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("draft")));
            ws.git_stage_hunk(&GitStageHunk, w, cx);
            assert!(!ws.git.operation);
            assert!(ws.status.as_ref().unwrap().0.contains("Save"));
            ws.show_git(&ShowGit, w, cx);
            ws.git_stage_hunk(&GitStageHunk, w, cx);
            assert!(!ws.git.operation);
            assert!(ws.status.as_ref().unwrap().0.contains("sidebar"));
            ws.show_git(&ShowGit, w, cx);
            ws.git.selected = Some((root.clone(), "another.txt".into()));
            ws.git_stage_hunk(&GitStageHunk, w, cx);
            assert!(!ws.git.operation);
            assert!(ws.status.as_ref().unwrap().0.contains("matching side"));
        });
        assert_eq!(index(&root), "one\ntwo\nthree\n");
    }

    #[gpui::test]
    fn git_history_and_blame_work_for_clean_files_and_open_read_only_tabs(cx: &mut TestAppContext) {
        let root = repo("history-blame");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_file_history(&GitFileHistory, w, cx);
        });
        cx.run_until_parked();
        let commit = git::file_history(&root, Path::new("file.txt"), 0).unwrap().remove(0);
        ws.update_in(cx, |ws, w, cx| {
            assert!(picker_title(ws).starts_with("Git History"));
            let panel = ws.quick_panel.as_ref().unwrap();
            assert_eq!(panel.items.len(), 1);
            assert!(panel.items[0].detail.contains("Orlo Test"));
            ws.cancel_quick_panel(cx);
            ws.git_show_commit(&GitShowCommit { root: root.clone(), commit }, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let editor = ws.active_editor().unwrap();
            assert!(editor.read(cx).doc(cx).read_only && editor.read(cx).doc(cx).scratch);
            assert!(editor.read(cx).text(cx).contains("+one"));
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git_blame(&GitBlame, w, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert!(editor.read(cx).doc(cx).read_only);
            let text = editor.read(cx).text(cx);
            assert!(text.contains("Orlo Test") && text.contains("|     3 | three"));
            assert!(editor.read(cx).title(cx).starts_with("Blame"));
        });
    }

    #[gpui::test]
    fn dismissed_git_history_does_not_reopen_its_picker(cx: &mut TestAppContext) {
        let root = repo("history-dismissed");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_file_history(&GitFileHistory, w, cx);
            ws.cancel_quick_panel(cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none()));
    }

    #[gpui::test]
    fn git_palette_commands_keep_the_sidebar_file_and_partial_staging_requires_it(cx: &mut TestAppContext) {
        let root = repo("palette-target");
        std::fs::write(root.join("other.txt"), "other\n").unwrap();
        git(&root, &["add", "."]);
        commit(&root, "-qm", "other");
        std::fs::write(root.join("file.txt"), "ONE\ntwo\nthree\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.join("other.txt"), w, cx));
        select(&ws, cx, &root, "file.txt", false);
        ws.update_in(cx, |ws, w, cx| {
            ws.focus_editor(w, cx);
            ws.git_stage_hunk(&GitStageHunk, w, cx);
            assert!(!ws.git.operation, "another editor cannot stage the retained diff selection");
        });
        for command in ["Git: File History…", "Git: Blame File", "Git: Stage Hunk"] {
            ws.update_in(cx, |ws, w, cx| {
                ws.git.selection_active = true;
                w.focus(ws.git.focus.as_ref().unwrap());
                run_command(ws, command, w, cx);
            });
            cx.run_until_parked();
            ws.update_in(cx, |ws, w, cx| {
                if command == "Git: File History…" {
                    assert!(picker_title(ws).ends_with("file.txt"));
                    ws.cancel_quick_panel(cx);
                } else if command == "Git: Blame File" {
                    assert!(ws.active_editor().unwrap().read(cx).title(cx).ends_with("file.txt"));
                    ws.open_path(root.join("other.txt"), w, cx);
                }
            });
        }
        assert_eq!(index(&root), "ONE\ntwo\nthree\n");
    }

    #[gpui::test]
    fn branch_picker_exposes_management_and_delete_confirmation(cx: &mut TestAppContext) {
        let root = repo("branch-management");
        git(&root, &["branch", "topic"]);
        let delete = GitBranchChoice { root: root.clone(), branch: "topic".into(), operation: BranchOperation::Delete };
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_switch_branch(&GitSwitchBranch, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let items = &ws.quick_panel.as_ref().unwrap().items;
            for label in ["Create Local Branch…", "Rename Local Branch…"] {
                assert!(items.iter().any(|i| i.label == label));
            }
            ws.cancel_quick_panel(cx);
            ws.git_branch_choice(&delete, w, cx);
        });
        assert!(cx.has_pending_prompt());
        assert!(git::branches(&root).unwrap().contains(&"topic".into()));
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(git::branches(&root).unwrap().contains(&"topic".into()));
        ws.update_in(cx, |ws, w, cx| ws.git_branch_choice(&delete, w, cx));
        cx.simulate_prompt_answer("Delete");
        cx.run_until_parked();
        assert_eq!(git::branches(&root).unwrap(), ["main"]);
    }

    #[gpui::test]
    fn replacing_a_git_picker_with_the_palette_keeps_a_live_command_target(cx: &mut TestAppContext) {
        let root = repo("replace-picker");
        std::fs::write(root.join("file.txt"), "ONE\ntwo\nthree\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.join("file.txt"), w, cx));
        select(&ws, cx, &root, "file.txt", false);
        ws.update_in(cx, |ws, w, cx| ws.git_file_history(&GitFileHistory, w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: File History…", w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(picker_title(ws).starts_with("Git History"));
        });
    }

    #[gpui::test]
    fn either_side_of_the_index_shows_its_two_versions_aligned(cx: &mut TestAppContext) {
        let root = repo("aligned-sides");
        let file = root.join("file.txt");
        std::fs::write(&file, "one\nTWO\nthree\nfour\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(file.clone(), w, cx));
        // the working tree side: the index against the saved file
        select(&ws, cx, &root, "file.txt", false);
        assert_eq!(sides(&ws, cx), texts("one\ntwo\nthree\n", "one\nTWO\nthree\nfour\n"));
        ws.read_with(cx, |ws, cx| {
            let Some(GitDiff::Sides { viewers, .. }) = &ws.git.diff else { unreachable!() };
            // the old side is padded where "four" was added, and both are read-only
            assert_eq!(viewers[0].read(cx).fillers().blocks(), [(3, 1)]);
            assert!(viewers.iter().all(|v| v.read(cx).doc(cx).read_only));
        });
        // the staged side: HEAD against the index, while the working tree moves on
        git(&root, &["add", "file.txt"]);
        std::fs::write(&file, "one\nTWO\nthree\nfour\nfive\n").unwrap();
        select(&ws, cx, &root, "file.txt", true);
        assert_eq!(sides(&ws, cx), texts("one\ntwo\nthree\n", "one\nTWO\nthree\nfour\n"));
        select(&ws, cx, &root, "file.txt", false);
        let saved = "one\nTWO\nthree\nfour\nfive\n";
        assert_eq!(sides(&ws, cx), texts("one\nTWO\nthree\nfour\n", saved));
    }

    #[gpui::test]
    fn a_wrapped_diff_opens_at_the_top_of_both_sides(cx: &mut TestAppContext) {
        let root = repo("aligned-top");
        // Markdown wraps; an untracked file has nothing on the left, all filler rows
        let notes: String = (0..200).map(|i| format!("note {i}\n")).collect();
        std::fs::write(root.join("notes.md"), &notes).unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.join("notes.md"), w, cx));
        let tops = |ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
            ws.read_with(cx, |ws, cx| {
                let Some(GitDiff::Sides { viewers, .. }) = &ws.git.diff else { unreachable!() };
                assert!(viewers.iter().all(|v| v.read(cx).wrapped_now()));
                viewers.each_ref().map(|v| v.read(cx).scroll_rows().0)
            })
        };
        select(&ws, cx, &root, "notes.md", false);
        assert_eq!(tops(&ws, cx), [0., 0.]);
        // a tracked file with lines added at its top
        git(&root, &["add", "notes.md"]);
        let added: String = (0..40).map(|i| format!("added {i}\n")).collect();
        std::fs::write(root.join("notes.md"), added + &notes).unwrap();
        select(&ws, cx, &root, "notes.md", false);
        assert_eq!(tops(&ws, cx), [0., 0.]);
    }

    #[gpui::test]
    fn untracked_and_deleted_files_compare_with_nothing(cx: &mut TestAppContext) {
        let root = repo("aligned-untracked");
        std::fs::write(root.join("new.txt"), "fresh\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.join("new.txt"), w, cx));
        select(&ws, cx, &root, "new.txt", false);
        assert_eq!(sides(&ws, cx), texts("", "fresh\n"));
        std::fs::remove_file(root.join("file.txt")).unwrap();
        select(&ws, cx, &root, "file.txt", false);
        assert_eq!(sides(&ws, cx), texts("one\ntwo\nthree\n", ""));
        git(&root, &["rm", "-q", "file.txt"]);
        select(&ws, cx, &root, "file.txt", true);
        assert_eq!(sides(&ws, cx), texts("one\ntwo\nthree\n", ""));
    }

    #[gpui::test]
    fn binary_oversized_and_conflicted_files_keep_the_raw_patch(cx: &mut TestAppContext) {
        let root = repo("aligned-fallback");
        std::fs::write(root.join("blob.bin"), b"a\0b").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(root.join("file.txt"), w, cx));
        select(&ws, cx, &root, "blob.bin", false);
        assert!(sides(&ws, cx).is_none());
        assert!(patch(&ws, cx).unwrap().contains("Binary files"));
        ws.read_with(cx, |ws, _| {
            let note = ws.git.diff_note.as_deref().unwrap();
            assert!(note.contains("binary"), "{note}");
        });
        // over the side-by-side limit
        let big = "line of text\n".repeat(GIT_SIDES_LIMIT / 13 + 1);
        std::fs::write(root.join("file.txt"), &big).unwrap();
        select(&ws, cx, &root, "file.txt", false);
        assert!(sides(&ws, cx).is_none());
        assert!(patch(&ws, cx).unwrap().contains("+line of text"));
        // a merge conflict
        git(&root, &["checkout", "-q", "file.txt"]);
        git(&root, &["switch", "-qc", "other"]);
        std::fs::write(root.join("file.txt"), "theirs\n").unwrap();
        commit(&root, "-qam", "theirs");
        git(&root, &["switch", "-q", "main"]);
        std::fs::write(root.join("file.txt"), "ours\n").unwrap();
        commit(&root, "-qam", "ours");
        let merge = std::process::Command::new("git").arg("-C").arg(&root).args(["merge", "other"]).output().unwrap();
        assert!(!merge.status.success());
        select(&ws, cx, &root, "file.txt", false);
        assert!(sides(&ws, cx).is_none());
        assert!(patch(&ws, cx).unwrap().contains("<<<<<<<"));
    }

    #[gpui::test]
    fn sidebar_background_takes_focus_before_using_selected_target(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1100.), px(750.)));
        ws.update_in(cx, |ws, w, cx| {
            ws.show_git(&ShowGit, w, cx);
            ws.focus_editor(w, cx);
        });
        cx.run_until_parked();
        let b = cx.debug_bounds("git-sidebar").unwrap();
        cx.simulate_click(b.origin + point(px(20.), px(15.)), gpui::Modifiers::none());
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert!(ws.git.selection_active);
            assert!(ws.git.focus.as_ref().unwrap().contains_focused(w, cx));
            ws.focus_editor(w, cx);
            cx.notify();
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(!ws.git.selection_active));
    }
}

#[cfg(test)]
mod stash_tests;

#[cfg(test)]
mod worktree_tests;

#[cfg(test)]
mod remote_tests;

#[cfg(test)]
mod commit_tests;

#[cfg(test)]
mod tag_tests;
