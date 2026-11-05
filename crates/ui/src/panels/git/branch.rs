//! Local branches use the existing quick panel and window form.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchOperation {
    Switch,
    Rename,
    Delete,
}

parameter_actions! { workspace, plain;
    GitBranchMenu { root: PathBuf, operation: BranchOperation, }

    GitBranchChoice { root: PathBuf, branch: String, operation: BranchOperation, }

    GitCreateAt { root: PathBuf, }
}

impl Workspace {
    pub(crate) fn git_create_branch(&mut self, _: &GitCreateBranch, w: &mut Window, cx: &mut Context<Self>) {
        match self.git_root(cx) {
            Some(root) => self.git_branch_form(root, None, w, cx),
            None => self.status_message("No Git repository found", cx),
        }
    }

    forward_actions! {
    pub(crate) git_rename_branch(GitRenameBranch, w, cx) => git_choose_branch(BranchOperation::Rename, w, cx);
    pub(crate) git_delete_branch(GitDeleteBranch, w, cx) => git_choose_branch(BranchOperation::Delete, w, cx);
    }

    pub(super) fn git_choose_branch(&mut self, operation: BranchOperation, w: &mut Window, cx: &mut Context<Self>) {
        match self.git_root(cx) {
            Some(root) => self.git_branch_menu(&GitBranchMenu { root, operation }, w, cx),
            None => self.status_message("No Git repository found", cx),
        }
    }

    pub(crate) fn git_branch_menu(&mut self, a: &GitBranchMenu, w: &mut Window, cx: &mut Context<Self>) {
        let (root, operation) = (a.root.clone(), a.operation);
        let title = match operation {
            BranchOperation::Switch => "Local Branches",
            BranchOperation::Rename => "Rename Local Branch",
            BranchOperation::Delete => "Delete Local Branch",
        };
        let read = root.clone();
        let rows = move |branches: Vec<String>| {
            let mut items = Vec::new();
            if operation == BranchOperation::Switch {
                items.push(quick_panel::Item::command(
                    "Create Local Branch…",
                    "Create at HEAD; keep the current branch",
                    GitCreateAt { root: root.clone() },
                ));
                for (label, operation) in [
                    ("Rename Local Branch…", BranchOperation::Rename),
                    ("Delete Local Branch…", BranchOperation::Delete),
                ] {
                    let root = root.clone();
                    let detail = root.display().to_string();
                    let menu = GitBranchMenu { root, operation };
                    items.push(quick_panel::Item::command(label, detail, menu));
                }
            }
            let verb = match operation {
                BranchOperation::Switch => "Switch branch",
                BranchOperation::Rename => "Rename branch",
                BranchOperation::Delete => "Delete merged branch",
            };
            for branch in branches {
                items.push(quick_panel::Item::command(
                    branch.clone(),
                    format!("{verb} · {}", root.display()),
                    GitBranchChoice { root: root.clone(), branch, operation },
                ));
            }
            let progress = items.is_empty().then(|| "No local branches yet".into());
            (items, progress)
        };
        let loading = "Loading local branches…";
        self.git_picker(title.into(), loading, move || git::branches(&read), rows, w, cx);
    }

    pub(crate) fn git_create_at(&mut self, a: &GitCreateAt, w: &mut Window, cx: &mut Context<Self>) {
        self.git_branch_form(a.root.clone(), None, w, cx);
    }

    fn git_branch_form(&mut self, root: PathBuf, old: Option<String>, w: &mut Window, cx: &mut Context<Self>) {
        let title = if old.is_some() { "Rename local branch" } else { "Create local branch at HEAD" };
        let initial = old.clone().unwrap_or_default();
        let generation = self.restore_gen;
        let done: OnDone = Box::new(move |v, _, ws, _, cx| {
            if ws.restore_gen != generation {
                return Err("The project changed; choose the repository again".into());
            }
            if ws.git.operation {
                return Err("A Git operation is still running".into());
            }
            let name = v.text("text").to_string();
            if name.is_empty() {
                return Err("Enter a branch name".into());
            }
            let (root, old) = (root.clone(), old.clone());
            ws.git_run(
                root.clone(),
                move || match old {
                    Some(old) => git::rename_branch(&root, &old, &name),
                    None => git::create_branch(&root, &name),
                },
                cx,
            );
            Ok(())
        });
        self.show_window_form(title, vec![Field::text("text", "Name", initial)], done, w, cx);
    }

    pub(crate) fn git_branch_choice(&mut self, a: &GitBranchChoice, w: &mut Window, cx: &mut Context<Self>) {
        match a.operation {
            BranchOperation::Switch => {
                self.git_use_branch(&GitUseBranch { root: a.root.clone(), branch: a.branch.clone() }, w, cx)
            }
            BranchOperation::Rename => self.git_branch_form(a.root.clone(), Some(a.branch.clone()), w, cx),
            BranchOperation::Delete => {
                let answer = w.prompt(
                    PromptLevel::Warning,
                    &format!("Delete local branch ‘{}’?", a.branch),
                    Some("Git will refuse branches with unmerged commits or branches checked out in a worktree."),
                    &["Delete", "Cancel"],
                    cx,
                );
                let a = a.clone();
                let generation = self.restore_gen;
                cx.spawn(async move |this, cx| {
                    if answer.await != Ok(0) {
                        return;
                    }
                    this.update(cx, |ws, cx| {
                        if ws.restore_gen == generation {
                            ws.git_run(a.root.clone(), move || git::delete_branch(&a.root, &a.branch), cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
    }
}
