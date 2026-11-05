//! Stash pickers and forms reuse the existing async Git and document-write boundaries.

use super::*;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct StashContext {
    root: PathBuf,
    generation: u64,
    roots: Vec<crate::config::project::Root>,
    stashes: Arc<git::Stashes>,
}

impl StashContext {
    fn current(&self, ws: &Workspace, cx: &App) -> bool {
        ws.restore_gen == self.generation && ws.roots == self.roots && ws.git_root(cx).as_ref() == Some(&self.root)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashOperation {
    Preview,
    Apply,
    ApplyIndex,
    Pop,
    PopIndex,
    Drop,
}

parameter_actions! { workspace, plain;
    GitStashEntry { context: StashContext, entry: git::StashEntry, }
    GitStashAction { context: StashContext, entry: git::StashEntry, operation: StashOperation, }
}

impl Workspace {
    forward_actions! {
    pub(crate) git_stashes(GitStashes, w, cx) => git_stash_prepare(false, w, cx);
    pub(crate) git_create_stash(GitCreateStash, w, cx) => git_stash_prepare(true, w, cx);
    }

    fn git_stash_prepare(&mut self, create: bool, w: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.git_root(cx) else {
            return self.status_message("No Git repository found", cx);
        };
        let generation = self.restore_gen;
        let roots = self.roots.clone();
        let read = root.clone();
        self.stash_read(
            root.clone(),
            move || git::prepare_stashes(&read),
            move |ws, stashes, w, cx| {
                let context = StashContext { root, generation, roots, stashes: Arc::new(stashes) };
                if create {
                    ws.git_stash_form(context, w, cx);
                } else {
                    let items = context
                        .stashes
                        .entries
                        .iter()
                        .map(|entry| {
                            quick_panel::Item::command(
                                format!("{} · {}", entry.selector, entry.subject),
                                format!("{} · {}", &entry.id[..8], context.root.display()),
                                GitStashEntry { context: context.clone(), entry: entry.clone() },
                            )
                        })
                        .collect();
                    ws.open_quick_panel(Kind::Choices, items, w, cx);
                    let panel = ws.quick_panel.as_mut().unwrap();
                    panel.title = Some("Stashes".into());
                    panel.progress = Some(
                        if panel.items.is_empty() {
                            "No stashes"
                        } else {
                            "Choose a stash to preview, apply, pop or drop"
                        }
                        .into(),
                    );
                }
            },
            w,
            cx,
        );
    }

    /// The loading picker is the cancellation token. A changed tab, project, selection or
    /// replacement picker cannot publish a late preview/form over the user's new surface.
    fn stash_read<T: Send + 'static>(
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
        panel.title = Some("Stashes".into());
        panel.progress = Some("Reading stashes…".into());
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
                    Ok(result) => show(ws, result, w, cx),
                    Err(e) => ws.status_message(e, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn git_stash_entry(&mut self, a: &GitStashEntry, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or repository changed; reopen Stashes", cx);
        }
        let items = [
            ("Preview", "Read-only patch, including untracked files", StashOperation::Preview),
            ("Apply", "Keep stash; restore changes without restoring staging", StashOperation::Apply),
            ("Apply and Restore Index", "Keep stash; restore staged and unstaged changes", StashOperation::ApplyIndex),
            ("Pop", "Remove stash only after successful apply", StashOperation::Pop),
            ("Pop and Restore Index", "Restore staging; remove stash only after success", StashOperation::PopIndex),
            ("Drop…", "Permanently remove this stash after confirmation", StashOperation::Drop),
        ]
        .into_iter()
        .map(|(label, detail, operation)| {
            quick_panel::Item::command(
                label,
                detail,
                GitStashAction { context: a.context.clone(), entry: a.entry.clone(), operation },
            )
        })
        .collect();
        self.open_quick_panel(Kind::Choices, items, w, cx);
        self.quick_panel.as_mut().unwrap().title = Some(format!("{} · {}", a.entry.selector, a.entry.subject));
    }

    pub(crate) fn git_stash_action(&mut self, a: &GitStashAction, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or repository changed; reopen Stashes", cx);
        }
        let a = a.clone();
        match a.operation {
            StashOperation::Preview => {
                let label = format!("Stash · {} · {}", a.entry.selector, &a.entry.id[..8]);
                self.stash_read(
                    a.context.root.clone(),
                    move || git::stash_preview(&a.context.root, &a.context.stashes, &a.entry),
                    move |ws, text, w, cx| {
                        let viewer = diff_view(&text, cx);
                        viewer.update(cx, |e, _| e.label = Some(label));
                        ws.attach(viewer, true, w, cx);
                    },
                    w,
                    cx,
                );
            }
            StashOperation::Drop => {
                let answer = w.prompt(
                    PromptLevel::Warning,
                    &format!("Drop {} · {}?", a.entry.selector, a.entry.subject),
                    Some("This permanently removes the stash. Its changes will not be applied."),
                    &["Drop", "Cancel"],
                    cx,
                );
                cx.spawn_in(w, async move |this, cx| {
                    if answer.await != Ok(0) {
                        return;
                    }
                    this.update_in(cx, |ws, w, cx| ws.git_stash_mutate(a, w, cx)).ok();
                })
                .detach();
            }
            _ => self.git_stash_mutate(a, w, cx),
        }
    }

    fn git_stash_mutate(&mut self, a: GitStashAction, w: &mut Window, cx: &mut Context<Self>) {
        let change_files = a.operation != StashOperation::Drop;
        let context = a.context.clone();
        let result = self.stash_run(
            context,
            change_files,
            change_files,
            move || match a.operation {
                StashOperation::Drop => git::drop_stash(&a.context.root, &a.context.stashes, &a.entry),
                operation => git::apply_stash(
                    &a.context.root,
                    &a.context.stashes,
                    &a.entry,
                    matches!(operation, StashOperation::ApplyIndex | StashOperation::PopIndex),
                    matches!(operation, StashOperation::Pop | StashOperation::PopIndex),
                ),
            },
            w,
            cx,
        );
        if let Err(e) = result {
            self.status_message(e, cx);
        }
    }

    fn git_stash_form(&mut self, context: StashContext, w: &mut Window, cx: &mut Context<Self>) {
        let done: OnDone = Box::new(move |v, _, ws, w, cx| {
            let message = v.text("message").to_string();
            if message.chars().any(char::is_control) {
                return Err("The stash message must be one line without control characters".into());
            }
            let include = v.flag("untracked");
            let prepared = context.clone();
            ws.stash_run(
                context.clone(),
                true,
                false,
                move || git::create_stash(&prepared.root, &prepared.stashes, &message, include),
                w,
                cx,
            )
        });
        self.show_window_form(
            "Create stash · saved tracked changes",
            vec![
                Field::text("message", "Message (optional)", ""),
                Field::toggle("untracked", "Include untracked files", false),
            ],
            done,
            w,
            cx,
        );
    }

    fn stash_run(
        &mut self,
        context: StashContext,
        change_files: bool,
        review_conflicts: bool,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !context.current(self, cx) {
            return Err("The project or repository changed; reopen Stashes".into());
        }
        if self.git.operation || self.other_workspaces(cx).any(|ws| ws.git.operation) {
            return Err("A Git operation is still running".into());
        }
        if pipelines::writing_under(&context.root, cx) || crate::documents::Documents::saving(cx) {
            return Err("Wait for file writes and save preparations to finish".into());
        }
        if change_files
            && self.open_editors(cx).iter().any(|e| {
                let e = e.read(cx);
                let d = e.doc(cx);
                !d.scratch
                    && d.path.as_ref().is_some_and(|p| p.starts_with(&context.root))
                    && (d.unsaved() || !e.marked.is_empty())
            })
        {
            return Err("Save or close unsaved repository files before changing the worktree".into());
        }
        // Even Drop holds the directory lease to serialize local Git stash mutations.
        let lease = pipelines::lease(&context.root, cx);
        self.git.operation = true;
        self.git.message = Some("Running stash operation…".into());
        let editor = self.active_editor();
        let origin = editor.as_ref().map(|e| {
            let state = e.read(cx).read_state(cx);
            (state.buffer_id(), state.doc.version, state.selection_generation())
        });
        let selection = self.git.selected.clone();
        let side = (self.git.staged, self.git.selection_active);
        let focus = w.focused(cx);
        let prepared = context.clone();
        let task = cx.background_spawn(async move {
            let _lease = lease;
            let result = job();
            // Read native unmerged status, never infer conflicts from diagnostic wording.
            let conflicts = (review_conflicts && result.is_err() && prepared.stashes.validate(&prepared.root).is_ok())
                .then(|| git::discover(&prepared.root).ok().flatten())
                .flatten()
                .filter(|repo| repo.root == prepared.root && repo.conflict_count() > 0);
            (result, conflicts)
        });
        cx.spawn_in(w, async move |this, cx| {
            let (result, conflicts) = task.await;
            this.update_in(cx, |ws, w, cx| {
                if ws.restore_gen != context.generation {
                    return;
                }
                ws.git.operation = false;
                if !context.current(ws, cx) {
                    return;
                }
                let message = match result {
                    Ok(()) => "Stash operation completed".to_string(),
                    Err(e) => e,
                };
                if let Some(repo) = conflicts.filter(|_| {
                    ws.active_editor() == editor
                        && editor.as_ref().map(|e| {
                            let state = e.read(cx).read_state(cx);
                            (state.buffer_id(), state.doc.version, state.selection_generation())
                        }) == origin
                        && ws.git.selected == selection
                        && (ws.git.staged, ws.git.selection_active) == side
                        && ws.quick_panel.is_none()
                        && ws.panel.is_none()
                        && w.focused(cx) == focus
                }) {
                    ws.git_stash_conflicts(repo, context.clone(), w, cx);
                }
                // Navigation must never replace the failed Git outcome with success.
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

    fn git_stash_conflicts(
        &mut self,
        repo: git::Repository,
        context: StashContext,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = repo.files.iter().find(|f| f.is_conflicted()) else {
            return;
        };
        let path = repo.root.join(&file.path);
        self.git.selected = Some((repo.root.clone(), file.path.clone()));
        let selected = self.git.selected.clone();
        self.git.staged = false;
        self.git.sidebar_open = true;
        self.git.selection_active = true;
        if let Some(old) = self.git.repositories.iter_mut().find(|r| r.root == repo.root) {
            *old = repo;
        }
        self.git.badges = badges(&self.git.repositories);
        self.load_git_diff(cx);
        self.open_path(path.clone(), w, cx);
        let Some(editor) = self.editor_for_path(&path, cx) else {
            return; // A large-file open retains its existing confirmation/background-read path.
        };
        if !self.stash_conflict_source_ready(&editor, cx) {
            return;
        }
        let state = editor.read(cx).read_state(cx);
        let before = (state.buffer_id(), state.doc.version, state.doc.save_point(), state.doc.disk);
        let (rope, encoding, selection) =
            (state.doc.rope.clone(), state.doc.disk_encoding, state.selection_generation());
        let focus = w.focused(cx);
        let source = path.clone();
        let prepared = context.clone();
        let task = cx.background_spawn(async move {
            let loaded = Loaded::read_with_hint(&source, Some(encoding)).map(|l| (l.diff(&rope), l));
            (prepared.stashes.validate(&prepared.root).is_ok(), loaded)
        });
        cx.spawn_in(w, async move |this, cx| {
            let (current, loaded) = task.await;
            #[cfg(test)]
            crate::testing::gate::wait("stash-conflict-read", &path).await;
            this.update_in(cx, |ws, w, cx| {
                let state = editor.read(cx).read_state(cx);
                if !current
                    || !context.current(ws, cx)
                    || ws.active_editor().as_ref() != Some(&editor)
                    || ws.git.selected != selected
                    || ws.git.staged
                    || ws.panel.is_some()
                    || ws.quick_panel.is_some()
                    || w.focused(cx) != focus
                    || (state.buffer_id(), state.doc.version, state.doc.save_point(), state.doc.disk) != before
                    || state.doc.path.as_ref() != Some(&path)
                    || state.doc.disk_encoding != encoding
                    || state.selection_generation() != selection
                    || !ws.stash_conflict_source_ready(&editor, cx)
                {
                    return;
                }
                if let Ok((changes, loaded)) = loaded {
                    if crate::files::watch::stamp(&path) == loaded.disk {
                        editor.update(cx, |e, cx| e.reload(loaded, &changes, cx));
                        if !git::conflicts(&editor.read(cx).text(cx)).is_empty() {
                            ws.next_conflict(&NextConflict, w, cx);
                        }
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn stash_conflict_source_ready(&self, editor: &Entity<Editor>, cx: &App) -> bool {
        let e = editor.read(cx);
        let d = e.doc(cx);
        !d.unsaved()
            && !d.read_only
            && !d.scratch
            && !d.is_following()
            && !e.document.read(cx).saving()
            && d.path.as_ref().is_some_and(|p| !pipelines::writing(p, cx))
            && !self.open_editors(cx).iter().any(|view| {
                let view = view.read(cx);
                view.same_document(e) && !view.marked.is_empty()
            })
    }
}
