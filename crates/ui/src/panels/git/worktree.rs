//! Local worktree pickers, forms and confirmed mutations; no window replacement.

use super::*;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct WorktreeContext {
    root: PathBuf,
    generation: u64,
    roots: Vec<crate::config::project::Root>,
    selection: Option<(PathBuf, PathBuf)>,
    editor: Option<Entity<Editor>>,
    worktrees: Arc<git::Worktrees>,
}

impl WorktreeContext {
    fn current(&self, ws: &Workspace, cx: &App) -> bool {
        ws.restore_gen == self.generation
            && ws.roots == self.roots
            && ws.git_root(cx).as_ref() == Some(&self.root)
            && ws.git.selected == self.selection
            && ws.active_editor() == self.editor
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorktreeOperation {
    List,
    Create,
    Prune,
}

parameter_actions! { workspace, plain;
    GitWorktreeEntry { context: WorktreeContext, entry: git::Worktree, }
    GitWorktreeOpen { context: WorktreeContext, entry: git::Worktree, }
    GitWorktreeRemove { context: WorktreeContext, entry: git::Worktree, }
    GitWorktreeCreateAt { context: WorktreeContext, branch: Option<String>, }
}

impl Workspace {
    forward_actions! {
    pub(crate) git_worktrees(GitWorktrees, w, cx) => git_worktree_prepare(WorktreeOperation::List, w, cx);
    pub(crate) git_create_worktree(GitCreateWorktree, w, cx) => git_worktree_prepare(WorktreeOperation::Create, w, cx);
    pub(crate) git_prune_worktrees(GitPruneWorktrees, w, cx) => git_worktree_prepare(WorktreeOperation::Prune, w, cx);
    }

    fn git_worktree_prepare(&mut self, operation: WorktreeOperation, w: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.git_root(cx) else {
            return self.status_message("No Git repository found", cx);
        };
        let generation = self.restore_gen;
        let roots = self.roots.clone();
        let selection = self.git.selected.clone();
        let editor = self.active_editor();
        let read = root.clone();
        self.git_read(
            root.clone(),
            move || git::prepare_worktrees(&read),
            move |ws, worktrees, w, cx| {
                let context =
                    WorktreeContext { root, generation, roots, selection, editor, worktrees: Arc::new(worktrees) };
                match operation {
                    WorktreeOperation::List => {
                        let items = context
                            .worktrees
                            .entries
                            .iter()
                            .map(|entry| {
                                let mut detail = entry.branch.clone().unwrap_or_else(|| "Detached or bare".into());
                                for (yes, label) in
                                    [(entry.main, "main"), (entry.current, "current"), (entry.missing, "missing")]
                                {
                                    if yes {
                                        detail.push_str(&format!(" · {label}"));
                                    }
                                }
                                if let Some(reason) = &entry.locked {
                                    detail.push_str(&format!(" · locked: {reason}"));
                                }
                                if let Some(reason) = &entry.prunable {
                                    detail.push_str(&format!(" · prunable: {reason}"));
                                }
                                quick_panel::Item::command(
                                    entry.path.display().to_string(),
                                    detail,
                                    GitWorktreeEntry { context: context.clone(), entry: entry.clone() },
                                )
                            })
                            .collect();
                        ws.open_quick_panel(Kind::Choices, items, w, cx);
                        ws.quick_panel.as_mut().unwrap().title = Some("Worktrees".into());
                    }
                    WorktreeOperation::Create => {
                        let mut items = vec![quick_panel::Item::command(
                            "New Branch at HEAD…",
                            "Create a branch and worktree",
                            GitWorktreeCreateAt { context: context.clone(), branch: None },
                        )];
                        items.extend(context.worktrees.branches.iter().map(|branch| {
                            quick_panel::Item::command(
                                branch.clone(),
                                "Existing local branch; Git refuses branches already checked out",
                                GitWorktreeCreateAt { context: context.clone(), branch: Some(branch.clone()) },
                            )
                        }));
                        ws.open_quick_panel(Kind::Choices, items, w, cx);
                        ws.quick_panel.as_mut().unwrap().title = Some("Create Worktree · Choose Branch".into());
                    }
                    WorktreeOperation::Prune => ws.git_worktree_prune_confirm(context, w, cx),
                }
            },
            w,
            cx,
        );
    }

    /// The retained loading input identifies the request; changed surfaces cannot publish a late result.
    pub(super) fn git_read<T: Send + 'static>(
        &mut self,
        root: PathBuf,
        read: impl FnOnce() -> Result<T, String> + Send + 'static,
        show: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.restore_gen;
        let roots = self.roots.clone();
        let editor = self.active_editor();
        let selection = self.git.selected.clone();
        self.open_quick_panel(Kind::Choices, Vec::new(), w, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some("Git".into());
        panel.progress = Some("Reading local Git state…".into());
        let input = panel.input.clone();
        let task = cx.background_spawn(async move { read() });
        cx.spawn_in(w, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, w, cx| {
                if ws.quick_panel.as_ref().is_none_or(|p| p.input != input) {
                    return;
                }
                ws.cancel_quick_panel(cx);
                if ws.restore_gen != generation
                    || ws.roots != roots
                    || ws.git_root(cx).as_ref() != Some(&root)
                    || ws.active_editor() != editor
                    || ws.git.selected != selection
                {
                    return;
                }
                match result {
                    Ok(value) => show(ws, value, w, cx),
                    Err(e) => ws.status_message(e, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn git_worktree_entry(&mut self, a: &GitWorktreeEntry, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or selection changed; reopen Worktrees", cx);
        }
        let items = vec![
            quick_panel::Item::command(
                "Open in New Window",
                "Keep this window and its documents",
                GitWorktreeOpen { context: a.context.clone(), entry: a.entry.clone() },
            ),
            quick_panel::Item::command(
                "Remove…",
                "Confirm removal of a clean linked worktree; keep its branch",
                GitWorktreeRemove { context: a.context.clone(), entry: a.entry.clone() },
            ),
        ];
        self.open_quick_panel(Kind::Choices, items, w, cx);
        self.quick_panel.as_mut().unwrap().title = Some(a.entry.path.display().to_string());
    }

    pub(crate) fn git_worktree_open(&mut self, a: &GitWorktreeOpen, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or selection changed; reopen Worktrees", cx);
        }
        let a = a.clone();
        self.git_read(
            a.context.root.clone(),
            move || git::open_worktree(&a.context.root, &a.context.worktrees, &a.entry),
            |_, path, _, cx| {
                let new = open_window(None, cx);
                cx.defer(move |cx| {
                    new.update(cx, |ws, w, cx| ws.open_path(path, w, cx)).ok();
                });
            },
            w,
            cx,
        );
    }

    pub(crate) fn git_worktree_remove(&mut self, a: &GitWorktreeRemove, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or selection changed; reopen Worktrees", cx);
        }
        let answer = w.prompt(PromptLevel::Warning, &format!("Remove worktree ‘{}’?", a.entry.path.display()),
            Some("This deletes the worktree folder, keeping its branch. Git refuses dirty, untracked or locked worktrees. Open documents stay open."), &["Remove", "Cancel"], cx);
        let a = a.clone();
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |ws, cx| {
                let context = a.context.clone();
                let paths = vec![a.entry.path.clone()];
                let result = ws.worktree_run(
                    context,
                    paths,
                    move || git::remove_worktree(&a.context.root, &a.context.worktrees, &a.entry),
                    cx,
                );
                if let Err(e) = result {
                    ws.status_message(e, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn git_worktree_prune_confirm(&mut self, context: WorktreeContext, w: &mut Window, cx: &mut Context<Self>) {
        let prunable: Vec<_> = context.worktrees.entries.iter().filter(|e| e.prunable.is_some()).collect();
        if prunable.is_empty() {
            return self.status_message("No prunable worktree registrations", cx);
        }
        let detail = format!("Remove registrations marked prunable by Git immediately (expire now). Locked and ineligible registrations stay. No worktree folders are deleted.\n\n{}",
            prunable.iter().map(|e| format!("{} · prunable: {}", e.path.display(), e.prunable.as_deref().unwrap_or_default())).collect::<Vec<_>>().join("\n"));
        let paths = prunable.iter().map(|e| e.path.clone()).collect();
        let answer =
            w.prompt(PromptLevel::Warning, "Prune worktree registrations?", Some(&detail), &["Prune", "Cancel"], cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |ws, cx| {
                let prepared = context.clone();
                let result = ws.worktree_run(
                    context,
                    paths,
                    move || git::prune_worktrees(&prepared.root, &prepared.worktrees),
                    cx,
                );
                if let Err(e) = result {
                    ws.status_message(e, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn git_worktree_create_at(&mut self, a: &GitWorktreeCreateAt, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or selection changed; reopen Worktrees", cx);
        }
        let mut fields = Vec::new();
        if a.branch.is_none() {
            fields.push(Field::text("branch", "New branch at HEAD", ""));
        }
        fields.push(Field::text("destination", "Absolute destination (parent must exist)", ""));
        let a = a.clone();
        let done: OnDone = Box::new(move |values, _, ws, _, cx| {
            let path = PathBuf::from(values.text("destination"));
            if !path.is_absolute() {
                return Err("Enter an absolute destination folder".into());
            }
            let path = crate::editing::text::resolve(&path);
            let branch = a.branch.clone().unwrap_or_else(|| values.text("branch").to_string());
            if branch.is_empty() {
                return Err("Enter a branch name".into());
            }
            let context = a.context.clone();
            let prepared = context.clone();
            let new = a.branch.is_none();
            ws.worktree_run(
                context,
                vec![path.clone()],
                move || git::create_worktree(&prepared.root, &prepared.worktrees, &path, &branch, new),
                cx,
            )
        });
        self.show_window_form("Create Worktree", fields, done, w, cx);
    }

    fn worktree_run(
        &mut self,
        context: WorktreeContext,
        affected: Vec<PathBuf>,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !context.current(self, cx) {
            return Err("The project or selection changed; reopen Worktrees".into());
        }
        if self.git.operation || self.other_workspaces(cx).any(|ws| ws.git.operation) {
            return Err("A Git operation is still running".into());
        }
        let mut paths: Vec<_> = context.worktrees.entries.iter().map(|e| e.path.clone()).collect();
        paths.extend(affected.iter().cloned());
        paths.push(context.worktrees.common_dir.clone());
        paths.sort();
        paths.dedup();
        if paths.iter().any(|p| pipelines::writing_under(p, cx)) || crate::documents::Documents::saving(cx) {
            return Err("Wait for file writes and save preparations to finish".into());
        }
        if self.open_editors(cx).iter().any(|e| {
            let e = e.read(cx);
            let d = e.doc(cx);
            d.path.as_ref().is_some_and(|p| affected.iter().any(|root| p.starts_with(root)))
                && (d.unsaved() || !e.marked.is_empty())
        }) {
            return Err("Save or close unsaved worktree files before removing or changing the worktree".into());
        }
        // Lease every registered folder (plus the common metadata and destination), so a
        // worker in any linked worktree cannot overlap even after its window is cleared.
        let leases: Vec<_> = paths.iter().map(|p| pipelines::lease(p, cx)).collect();
        self.git.operation = true;
        self.git.message = Some("Running worktree operation…".into());
        let task = cx.background_spawn(async move {
            let _leases = leases;
            job()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |ws, cx| {
                if ws.restore_gen != context.generation {
                    return;
                }
                ws.git.operation = false;
                if !context.current(ws, cx) {
                    return;
                }
                let message = match result {
                    Ok(()) => "Worktree operation completed".to_string(),
                    Err(e) => e,
                };
                ws.git.message = Some(message.clone());
                ws.status_message(message, cx);
                ws.refresh_git(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
        Ok(())
    }
}
