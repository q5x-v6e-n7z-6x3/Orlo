//! Explicit network commands and local remote configuration, using prepared identities.
use super::*;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteContext {
    root: PathBuf,
    generation: u64,
    roots: Vec<crate::config::project::Root>,
    selection: Option<(PathBuf, PathBuf)>,
    editor: Option<Entity<Editor>>,
    remotes: Arc<git::Remotes>,
}
impl RemoteContext {
    fn current(&self, ws: &Workspace, cx: &App) -> bool {
        self.generation == ws.restore_gen
            && self.roots == ws.roots
            && ws.git_root(cx).as_ref() == Some(&self.root)
            && self.selection == ws.git.selected
            && self.editor == ws.active_editor()
    }
    fn label(&self) -> String {
        format!(
            "{} · {} → {}",
            self.root.display(),
            self.remotes.branch.as_deref().unwrap_or("Detached HEAD"),
            self.remotes.upstream.as_deref().unwrap_or("no upstream")
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RemoteChoice {
    Fetch(String),
    Push(String),
    Edit(String),
    Remove(String),
    Add,
    Upstream,
    SetUpstream(Option<String>),
}
parameter_actions! { workspace, plain;
    GitRemoteChoice { context: RemoteContext, choice: RemoteChoice, }
}

impl Workspace {
    forward_actions! {
    pub(crate) git_fetch(GitFetch, w, cx) => git_remote_prepare("Fetch", w, cx);
    pub(crate) git_pull(GitPull, w, cx) => git_remote_prepare("Pull", w, cx);
    pub(crate) git_push(GitPush, w, cx) => git_remote_prepare("Push", w, cx);
    pub(crate) git_remotes(GitRemotes, w, cx) => git_remote_prepare("Remotes & Upstream", w, cx);
    }

    fn git_remote_prepare(&mut self, mode: &'static str, w: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.git_root(cx) else {
            return self.status_message("No Git repository found", cx);
        };
        let generation = self.restore_gen;
        let roots = self.roots.clone();
        let editor = self.active_editor();
        let selection = self.git.selected.clone();
        let read = root.clone();
        self.git_read(
            root.clone(),
            move || git::prepare_remotes(&read),
            move |ws, remotes, w, cx| {
                let context = RemoteContext { root, generation, roots, selection, editor, remotes: Arc::new(remotes) };
                if mode == "Pull" {
                    return ws.remote_confirm(context, git::RemoteOperation::Pull, "Pull", w, cx);
                }
                let item = |label: String, detail: String, choice| {
                    quick_panel::Item::command(label, detail, GitRemoteChoice { context: context.clone(), choice })
                };
                let mut items = Vec::new();
                for remote in &context.remotes.remotes {
                    let refs = if remote.fetch_refspecs.is_empty() {
                        "remote HEAD".into()
                    } else {
                        remote.fetch_refspecs.join(", ")
                    };
                    let detail = format!(
                        "Fetch: {} · Refs: {refs} · Push: {}",
                        remote.fetch_urls.join(", "),
                        remote.push_urls.join(", ")
                    );
                    if mode == "Fetch" {
                        items.push(item(remote.name.clone(), detail, RemoteChoice::Fetch(remote.name.clone())));
                    } else if mode == "Push" {
                        items.push(item(remote.name.clone(), detail, RemoteChoice::Push(remote.name.clone())));
                    } else {
                        items.push(item(
                            format!("Edit {}…", remote.name),
                            detail.clone(),
                            RemoteChoice::Edit(remote.name.clone()),
                        ));
                        items.push(item(
                            format!("Remove {}…", remote.name),
                            detail,
                            RemoteChoice::Remove(remote.name.clone()),
                        ));
                    }
                }
                if mode == "Remotes & Upstream" {
                    items.push(item(
                        "Add Remote…".into(),
                        "Configure a URL locally; no connection".into(),
                        RemoteChoice::Add,
                    ));
                    items.push(item(
                        "Set Upstream…".into(),
                        "Choose a locally fetched remote-tracking branch".into(),
                        RemoteChoice::Upstream,
                    ));
                    items.push(item(
                        "Clear Upstream".into(),
                        "Remove current branch tracking configuration".into(),
                        RemoteChoice::SetUpstream(None),
                    ));
                }
                ws.open_quick_panel(Kind::Choices, items, w, cx);
                ws.quick_panel.as_mut().unwrap().title = Some(format!("{mode} · {}", context.label()));
            },
            w,
            cx,
        );
    }

    pub(crate) fn git_remote_choice(&mut self, a: &GitRemoteChoice, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or selection changed; reopen remote controls", cx);
        }
        let context = a.context.clone();
        match &a.choice {
            RemoteChoice::Fetch(remote) => {
                self.remote_confirm(context, git::RemoteOperation::Fetch { remote: remote.clone() }, "Fetch", w, cx);
            }
            RemoteChoice::Remove(name) => self.remote_confirm(
                context,
                git::RemoteOperation::RemoveRemote { name: name.clone() },
                "Remove Remote",
                w,
                cx,
            ),
            RemoteChoice::SetUpstream(upstream) => {
                if let Err(e) =
                    self.remote_run(context, git::RemoteOperation::SetUpstream { upstream: upstream.clone() }, cx)
                {
                    self.status_message(e, cx);
                }
            }
            RemoteChoice::Upstream => {
                let items = context
                    .remotes
                    .upstream_choices
                    .iter()
                    .map(|name| {
                        quick_panel::Item::command(
                            name.clone(),
                            "Set current branch upstream (local configuration only)",
                            GitRemoteChoice {
                                context: context.clone(),
                                choice: RemoteChoice::SetUpstream(Some(name.clone())),
                            },
                        )
                    })
                    .collect();
                self.open_quick_panel(Kind::Choices, items, w, cx);
                self.quick_panel.as_mut().unwrap().title = Some(format!("Set Upstream · {}", context.label()));
            }
            RemoteChoice::Add | RemoteChoice::Edit(_) | RemoteChoice::Push(_) => self.remote_form(a.clone(), w, cx),
        }
    }

    fn remote_form(&mut self, a: GitRemoteChoice, w: &mut Window, cx: &mut Context<Self>) {
        let (title, fields) = match &a.choice {
            RemoteChoice::Push(remote) => {
                let upstream = if a.context.remotes.upstream_remote.as_deref() == Some(remote.as_str()) {
                    a.context.remotes.upstream_ref.as_deref().and_then(|r| r.strip_prefix("refs/heads/"))
                } else {
                    None
                };
                let branch = upstream.or(a.context.remotes.branch.as_deref()).unwrap_or("");
                (
                    "Push Current Branch",
                    vec![
                        Field::text("branch", "Destination branch", branch),
                        Field::toggle("upstream", "Set as current branch upstream", false),
                    ],
                )
            }
            RemoteChoice::Edit(name) => {
                let Some(remote) = a.context.remotes.remotes.iter().find(|r| &r.name == name) else {
                    return;
                };
                (
                    "Replace Remote URLs",
                    vec![
                        Field::text(
                            "url",
                            "Fetch URL (replace all)",
                            remote.fetch_urls.first().map(String::as_str).unwrap_or(""),
                        ),
                        Field::text(
                            "push_url",
                            "Push URL (blank uses fetch URL)",
                            if remote.push_urls == remote.fetch_urls {
                                ""
                            } else {
                                remote.push_urls.first().map(String::as_str).unwrap_or("")
                            },
                        ),
                    ],
                )
            }
            _ => (
                "Add Remote",
                vec![Field::text("name", "Remote name", "origin"), Field::text("url", "URL or local path", "")],
            ),
        };
        let remote = match &a.choice {
            RemoteChoice::Push(name) | RemoteChoice::Edit(name) => name.as_str(),
            _ => "new remote",
        };
        let urls = a
            .context
            .remotes
            .remotes
            .iter()
            .find(|r| r.name == remote)
            .map(|r| if matches!(a.choice, RemoteChoice::Push(_)) { &r.push_urls } else { &r.fetch_urls })
            .map(|urls| format!(" · {}", urls.join(", ")))
            .unwrap_or_default();
        let heading = format!("{title} · {remote}{urls} · {}", a.context.label());
        let done: OnDone = Box::new(move |values, _, ws, w, cx| {
            let operation = match &a.choice {
                RemoteChoice::Push(remote) => git::RemoteOperation::Push {
                    remote: remote.clone(),
                    branch: values.text("branch").into(),
                    set_upstream: values.flag("upstream"),
                },
                RemoteChoice::Edit(name) => git::RemoteOperation::EditRemote {
                    name: name.clone(),
                    url: values.text("url").into(),
                    push_url: (!values.text("push_url").is_empty()).then(|| values.text("push_url").into()),
                },
                _ => {
                    git::RemoteOperation::AddRemote { name: values.text("name").into(), url: values.text("url").into() }
                }
            };
            if matches!(operation, git::RemoteOperation::Push { .. }) {
                if !a.context.current(ws, cx) {
                    return Err("The project or selection changed; reopen remote controls".into());
                }
                ws.remote_confirm(a.context.clone(), operation, "Push", w, cx);
                Ok(())
            } else {
                ws.remote_run(a.context.clone(), operation, cx)
            }
        });
        self.show_window_form(&heading, fields, done, w, cx);
    }

    fn remote_confirm(
        &mut self,
        context: RemoteContext,
        operation: git::RemoteOperation,
        label: &'static str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = match &operation {
            git::RemoteOperation::Fetch { remote } => {
                let Some(target) = context.remotes.remotes.iter().find(|r| &r.name == remote) else {
                    return self.status_message("Choose a configured remote", cx);
                };
                let refs = if target.fetch_refspecs.is_empty() {
                    "remote HEAD".into()
                } else {
                    target.fetch_refspecs.join("\n")
                };
                format!(
                    "Fetch from {remote}.\nURLs: {}\nRefspecs: {refs}\n{}",
                    target.fetch_urls.join("\n"),
                    context.label()
                )
            }
            git::RemoteOperation::Push { remote, branch, set_upstream } => {
                let Some(target) = context.remotes.remotes.iter().find(|r| &r.name == remote) else {
                    return self.status_message("Choose a configured remote", cx);
                };
                let current = context.remotes.branch.as_deref().unwrap_or("Detached HEAD");
                let tracking =
                    if *set_upstream { "Set as current branch upstream." } else { "Keep the current upstream." };
                format!("Push refs/heads/{current} → {remote}/refs/heads/{branch}.\nURLs: {}\n{tracking}\nOrdinary push; no force or mirror.\n{}", target.push_urls.join("\n"), context.label())
            }
            git::RemoteOperation::RemoveRemote { name } => format!(
                "Remove ‘{name}’ and its local remote-tracking references. No remote branches are deleted.\n{}",
                context.label()
            ),
            _ => {
                let remote = context.remotes.upstream_remote.as_deref().unwrap_or("no remote");
                let reference = context.remotes.upstream_ref.as_deref().unwrap_or("no upstream ref");
                let urls = context
                    .remotes
                    .remotes
                    .iter()
                    .find(|r| r.name == remote)
                    .map(|r| r.fetch_urls.join(", "))
                    .unwrap_or_else(
                        || if remote == "." { context.root.display().to_string() } else { "no URL".into() },
                    );
                format!("Pull fast-forward-only from {remote} · {reference} · {urls}. Divergent history is refused; use terminal Git to choose how to integrate it.\n{}", context.label())
            }
        };
        let answer = w.prompt(PromptLevel::Warning, label, Some(&detail), &[label, "Cancel"], cx);
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |ws, cx| {
                if let Err(e) = ws.remote_run(context, operation, cx) {
                    ws.status_message(e, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn remote_run(
        &mut self,
        context: RemoteContext,
        operation: git::RemoteOperation,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !context.current(self, cx) {
            return Err("The project or selection changed; reopen remote controls".into());
        }
        if self.git.operation || self.other_workspaces(cx).any(|ws| ws.git.operation) {
            return Err("A Git operation is still running".into());
        }
        let mut paths = context.remotes.metadata_dirs.clone();
        paths.extend(context.remotes.config_files.iter().cloned());
        let pull = operation == git::RemoteOperation::Pull;
        if pull {
            paths.push(context.root.clone());
        }
        if paths.iter().any(|p| pipelines::writing_under(p, cx)) || crate::documents::Documents::saving(cx) {
            return Err("Wait for file writes and save preparations to finish".into());
        }
        if self.open_editors(cx).iter().any(|e| {
            let e = e.read(cx);
            let d = e.doc(cx);
            let affected = d.path.as_ref().is_some_and(|p| {
                (context.remotes.metadata_dirs.iter().any(|dir| p.starts_with(dir))
                    || context.remotes.config_files.contains(p))
                    || (pull && p.starts_with(&context.root))
            });
            affected && (d.unsaved() || !e.marked.is_empty() || (pull && d.read_only))
        }) {
            return Err("Save or close unsaved, read-only or composing repository files before this operation".into());
        }
        // The shared worker owns root and common/per-worktree metadata leases together.
        let progress = match &operation {
            git::RemoteOperation::Fetch { remote } => format!("Fetching {remote}…"),
            git::RemoteOperation::Pull => format!("Pulling {}…", context.label()),
            git::RemoteOperation::Push { remote, branch, .. } => {
                format!("Pushing current branch to {remote}/{branch}…")
            }
            _ => "Updating local Git configuration…".into(),
        };
        let prepared = context.clone();
        self.git_run_checked(
            context.root.clone(),
            Some(paths),
            move || git::remote_operation(&prepared.root, &prepared.remotes, &operation),
            move |ws, cx| context.current(ws, cx),
            cx,
        );
        self.git.message = Some(progress);
        Ok(())
    }
}
