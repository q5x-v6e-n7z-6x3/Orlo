//! Local composition and pinned asynchronous commit preparation.

use super::*;

/// Retain field editors after Cancel/failure, until successful
/// commit or another repository uses the same mode. No document/session persistence.
pub(super) struct Draft {
    root: PathBuf,
    fields: Vec<Entity<Editor>>,
}

impl Draft {
    fn text(&self, cx: &App) -> (git::CommitDraft, git::CommitAuthor) {
        let draft = git::CommitDraft {
            subject: self.fields[0].read(cx).text(cx),
            body: self.fields[1].read(cx).text(cx),
            coauthors: self.fields[2].read(cx).text(cx),
        };
        let author =
            git::CommitAuthor { name: self.fields[3].read(cx).text(cx), email: self.fields[4].read(cx).text(cx) };
        (draft, author)
    }
}

impl Workspace {
    forward_actions! {
    pub(crate) git_commit(GitCommit, w, cx) => git_compose(false, w, cx);
    pub(crate) git_amend(GitAmend, w, cx) => git_compose(true, w, cx);
    }

    fn git_compose(&mut self, amend: bool, w: &mut Window, cx: &mut Context<Self>) {
        if self.git.operation {
            return self.status_message("A Git operation is still running", cx);
        }
        let Some(root) = self.git_root(cx) else {
            return self.status_message("No Git repository found", cx);
        };
        self.open_quick_panel(Kind::Choices, Vec::new(), w, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some(if amend { "Amend HEAD" } else { "Commit staged changes" }.into());
        panel.progress = Some("Preparing commit…".into());
        let input = panel.input.clone();
        let generation = self.restore_gen;
        let read = root.clone();
        let task = cx.background_spawn(async move { git::prepare_commit(&read, amend) });
        cx.spawn_in(w, async move |this, cx| {
            let prepared = task.await;
            this.update_in(cx, |ws, w, cx| {
                if ws.quick_panel.as_ref().is_none_or(|p| p.input != input) {
                    return;
                }
                if ws.restore_gen != generation || ws.git_root(cx).as_ref() != Some(&root) {
                    ws.cancel_quick_panel(cx);
                    return ws.status_message("The project or repository changed; choose the commit action again", cx);
                }
                ws.cancel_quick_panel(cx);
                match prepared {
                    Ok(prepared) => ws.git_commit_form(root, prepared, amend, w, cx),
                    Err(e) => ws.status_message(e, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn git_commit_form(
        &mut self,
        root: PathBuf,
        prepared: git::PreparedCommit,
        amend: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = usize::from(amend);
        let retained = self.git.commit_drafts[index].as_ref().filter(|d| d.root == root);
        let mut title = if amend {
            format!("Amend HEAD · {}", &prepared.head().unwrap()[..8])
        } else {
            "Commit staged changes".to_string()
        };
        if retained.is_some() {
            title.push_str(" · retained draft");
        }
        let (draft, author) =
            retained.map(|d| d.text(cx)).unwrap_or_else(|| (prepared.draft.clone(), Default::default()));
        let generation = self.restore_gen;
        let target = root.clone();
        let done: OnDone = Box::new(move |values, _, ws, w, cx| {
            let draft = git::CommitDraft {
                subject: values.text("subject").into(),
                body: values.text("body").into(),
                coauthors: values.text("coauthors").into(),
            };
            draft.validate()?;
            let author = git::CommitAuthor {
                name: values.text("author_name").into(),
                email: values.text("author_email").into(),
            };
            let override_author = !author.name.is_empty() || !author.email.is_empty();
            if override_author {
                author.validate()?;
            }
            if ws.restore_gen != generation || ws.git_root(cx).as_ref() != Some(&target) {
                return Err("The project or repository changed; reopen the commit form (your draft is retained)".into());
            }
            if ws.git.operation {
                return Err("A Git operation is still running".into());
            }
            if pipelines::writing_under(&target, cx) {
                return Err("Wait for repository writes to finish".into());
            }
            // Keep the form open during Git: failures retain exact fields and focus, and
            // success only closes this form, never a replacement panel opened meanwhile.
            let fields = ws.git.commit_drafts[index].as_ref().unwrap().fields.clone();
            ws.git.operation = true;
            ws.git.message = Some("Running Git…".into());
            let (root, prepared) = (target.clone(), prepared.clone());
            let submitted = (draft.clone(), author.clone());
            let lease = pipelines::lease(&root, cx);
            let task = cx.background_spawn(async move {
                let _lease = lease;
                git::commit_draft_as(&root, &prepared, &draft, override_author.then_some(&author))
            });
            cx.spawn_in(w, async move |this, cx| {
                let result = task.await;
                this.update_in(cx, |ws, w, cx| {
                    if ws.restore_gen != generation {
                        return;
                    }
                    ws.git.operation = false;
                    let owns_form = matches!(&ws.panel, Some(Panel::Form(p)) if p.inputs().eq(fields.iter()));
                    let message = match result {
                        Ok(()) => {
                            let edited = ws.git.commit_drafts[index].as_ref().is_some_and(|d| d.text(cx) != submitted);
                            if edited {
                                if owns_form {
                                    if let Some(Panel::Form(p)) = &mut ws.panel {
                                        p.error = Some("Commit completed; newer draft edits retained. Reopen before committing again.".into());
                                    }
                                }
                            } else {
                                ws.git.commit_drafts[index] = None;
                                if owns_form {
                                    ws.close_panel(&ClosePanel, w, cx);
                                }
                            }
                            "Git operation completed".to_string()
                        }
                        Err(e) => {
                            if owns_form {
                                if let Some(Panel::Form(p)) = &mut ws.panel {
                                    p.error = Some(e.clone());
                                }
                            }
                            e
                        }
                    };
                    ws.git.message = Some(message.clone());
                    ws.status_message(message, cx);
                    ws.refresh_git(cx);
                    ws.load_git_diff(cx);
                })
                .ok();
            })
            .detach();
            Err("Running Git…".into())
        });
        self.show_window_form(
            &title,
            vec![
                Field::text("subject", "Subject", draft.subject),
                Field::multiline("body", "Body", draft.body),
                Field::multiline("coauthors", "Coauthors: Name <email>", draft.coauthors),
                Field::text("author_name", "Author name (optional override)", author.name),
                Field::text("author_email", "Author email (optional override)", author.email),
            ],
            done,
            w,
            cx,
        );
        let Some(Panel::Form(panel)) = &self.panel else { unreachable!() };
        self.git.commit_drafts[index] = Some(Draft { root, fields: panel.inputs().cloned().collect() });
    }
}
