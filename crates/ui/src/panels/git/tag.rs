//! Local tag selection and pinned commit forms reuse Git's async task owners.

use super::*;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct TagContext {
    root: PathBuf,
    generation: u64,
    roots: Vec<crate::config::project::Root>,
    editor: Option<Entity<Editor>>,
    selection: Option<(PathBuf, PathBuf)>,
    tags: Arc<git::Tags>,
}

impl TagContext {
    fn current(&self, ws: &Workspace, cx: &App) -> bool {
        ws.restore_gen == self.generation
            && ws.roots == self.roots
            && ws.git_root(cx).as_ref() == Some(&self.root)
            && ws.active_editor() == self.editor
            && ws.git.selected == self.selection
    }
}

parameter_actions! { workspace, plain;
    GitTagCreateAt { context: TagContext, }
    GitTagDelete { context: TagContext, entry: git::Tag, }
}

impl Workspace {
    forward_actions! {
    pub(crate) git_tags(GitTags, w, cx) => git_tag_prepare(false, w, cx);
    pub(crate) git_create_tag(GitCreateTag, w, cx) => git_tag_prepare(true, w, cx);
    }

    fn git_tag_prepare(&mut self, create: bool, w: &mut Window, cx: &mut Context<Self>) {
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
            move || git::prepare_tags(&read),
            move |ws, tags, w, cx| {
                let context = TagContext { root, generation, roots, editor, selection, tags: Arc::new(tags) };
                if create {
                    ws.git_tag_create_at(&GitTagCreateAt { context }, w, cx);
                    return;
                }
                let mut items = vec![quick_panel::Item::command(
                    "Create Local Tag…",
                    "Resolve HEAD or a chosen commit, then enter a name and optional message",
                    GitTagCreateAt { context: context.clone() },
                )];
                items.extend(context.tags.entries.iter().map(|entry| {
                    let kind = if entry.annotated { "Annotated" } else { "Lightweight" };
                    quick_panel::Item::command(
                        entry.name.clone(),
                        format!("{kind} · {} · {}", entry.target, entry.message),
                        GitTagDelete { context: context.clone(), entry: entry.clone() },
                    )
                }));
                ws.open_quick_panel(Kind::Choices, items, w, cx);
                let panel = ws.quick_panel.as_mut().unwrap();
                panel.choice_details = true;
                panel.title = Some("Local Tags".into());
                panel.progress = Some(
                    if context.tags.entries.is_empty() { "No local tags" } else { "Choose a tag to delete it locally" }
                        .into(),
                );
            },
            w,
            cx,
        );
    }

    pub(crate) fn git_tag_create_at(&mut self, a: &GitTagCreateAt, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or selection changed; reopen Local Tags", cx);
        }
        let context = a.context.clone();
        let done: OnDone = Box::new(move |values, _, ws, w, cx| {
            if !context.current(ws, cx) {
                return Err("The project or selection changed; reopen Local Tags".into());
            }
            let revision = values.text("revision").to_owned();
            if revision.trim().is_empty() {
                return Err("Enter HEAD or a commit revision".into());
            }
            let read = context.clone();
            let context = context.clone();
            ws.git_read(
                context.root.clone(),
                move || git::prepare_tag(&read.root, &read.tags, &revision),
                move |ws, prepared, w, cx| ws.git_tag_form(context, prepared, w, cx),
                w,
                cx,
            );
            Ok(())
        });
        self.show_window_form(
            "Create Local Tag · Choose Commit",
            vec![Field::text("revision", "Commit revision", "HEAD")],
            done,
            w,
            cx,
        );
    }

    fn git_tag_form(
        &mut self,
        context: TagContext,
        prepared: git::PreparedTag,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = format!("Create Local Tag at {}", prepared.commit());
        let done: OnDone = Box::new(move |values, _, ws, _, cx| {
            let name = values.text("name").to_owned();
            if name.is_empty() {
                return Err("Enter a local tag name".into());
            }
            let message = values.text("message").to_owned();
            let prepared = prepared.clone();
            let root = context.root.clone();
            ws.git_tag_run(context.clone(), move || git::create_tag(&root, &prepared, &name, &message), cx)
        });
        self.show_window_form(
            &title,
            vec![
                Field::text("name", "Name", ""),
                Field::multiline("message", "Message (optional; creates an unsigned annotated tag)", ""),
            ],
            done,
            w,
            cx,
        );
    }

    pub(crate) fn git_tag_delete(&mut self, a: &GitTagDelete, w: &mut Window, cx: &mut Context<Self>) {
        if !a.context.current(self, cx) {
            return self.status_message("The project or selection changed; reopen Local Tags", cx);
        }
        let detail =
            format!("Delete only this local tag at {}. Remote tags and source files stay unchanged.", a.entry.target);
        let answer = w.prompt(
            PromptLevel::Warning,
            &format!("Delete local tag ‘{}’?", a.entry.name),
            Some(&detail),
            &["Delete", "Cancel"],
            cx,
        );
        let a = a.clone();
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |ws, cx| {
                if let Err(e) = ws.git_tag_run(
                    a.context.clone(),
                    move || git::delete_tag(&a.context.root, &a.context.tags, &a.entry),
                    cx,
                ) {
                    ws.status_message(e, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn git_tag_run(
        &mut self,
        context: TagContext,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !context.current(self, cx) {
            return Err("The project or selection changed; reopen Local Tags".into());
        }
        if self.git.operation {
            return Err("A Git operation is still running".into());
        }
        // Tags change refs only. Loading unrelated index/diff blobs here could implicitly
        // fetch from a promisor remote after an otherwise entirely local operation.
        self.git_run_checked_with_refresh(
            context.root.clone(),
            None,
            job,
            move |ws, cx| context.current(ws, cx),
            false,
            cx,
        );
        Ok(())
    }
}
