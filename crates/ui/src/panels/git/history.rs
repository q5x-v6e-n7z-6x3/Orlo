//! Git history and blame are on-demand read-only views, separate from saved-file backups.

use super::*;

parameter_actions! { workspace, plain;
    GitHistoryPage { root: PathBuf, path: PathBuf, skip: usize, }

    GitShowCommit { root: PathBuf, commit: git::FileCommit, }
}

impl Workspace {
    fn git_file(&self, cx: &App) -> Option<(PathBuf, PathBuf)> {
        if self.git.sidebar_open && self.git.selection_active {
            if let Some(selected) = &self.git.selected {
                return Some(selected.clone());
            }
        }
        let path = self.active_editor()?.read(cx).doc(cx).path.clone()?;
        let repo = containing(&self.git.repositories, &path)?;
        Some((repo.root.clone(), path.strip_prefix(&repo.root).ok()?.to_path_buf()))
    }

    pub(crate) fn git_file_history(&mut self, _: &GitFileHistory, w: &mut Window, cx: &mut Context<Self>) {
        let Some((root, mut path)) = self.git_file(cx) else {
            return self.status_message("Open a repository file to view its Git history", cx);
        };
        // Before a rename is committed, its history still has the old name.
        if let Some(original) = (self.git_file_status(&root, &path))
            .filter(|f| f.index == 'R' || f.worktree == 'R')
            .and_then(|f| f.original_path.clone())
        {
            path = original;
        }
        self.git_history_page(&GitHistoryPage { root, path, skip: 0 }, w, cx);
    }

    pub(crate) fn git_history_page(&mut self, a: &GitHistoryPage, w: &mut Window, cx: &mut Context<Self>) {
        let title = format!("Git History · {}", a.path.display());
        let (a, read) = (a.clone(), a.clone());
        let rows = move |commits: Vec<git::FileCommit>| {
            let more = commits.len() == git::HISTORY_PAGE_SIZE;
            let mut items: Vec<_> = commits
                .into_iter()
                .map(|commit| {
                    quick_panel::Item::command(
                        format!("{} · {}", &commit.id[..8], commit.subject),
                        format!("{} · {}", commit.author, commit.date),
                        GitShowCommit { root: a.root.clone(), commit },
                    )
                })
                .collect();
            if more {
                items.push(quick_panel::Item::command(
                    "Older commits…",
                    a.path.display().to_string(),
                    GitHistoryPage { skip: a.skip + git::HISTORY_PAGE_SIZE, ..a },
                ));
            }
            let progress = match items.is_empty() {
                true => "No commits for this file",
                false => "Choose a commit to view its changes",
            };
            (items, Some(progress.into()))
        };
        let load = move || git::file_history(&read.root, &read.path, read.skip);
        self.git_picker(title, "Loading commits…", load, rows, w, cx);
    }

    pub(crate) fn git_show_commit(&mut self, a: &GitShowCommit, w: &mut Window, cx: &mut Context<Self>) {
        let label =
            format!("{} · {}", a.commit.path.file_name().unwrap_or_default().to_string_lossy(), &a.commit.id[..8]);
        let a = a.clone();
        self.git_read_view(label, move || git::commit_diff(&a.root, &a.commit), w, cx);
    }

    pub(crate) fn git_blame(&mut self, _: &GitBlame, w: &mut Window, cx: &mut Context<Self>) {
        let Some((root, path)) = self.git_file(cx) else {
            return self.status_message("Open a repository file to view Git blame", cx);
        };
        let full = root.join(&path);
        if self.unsaved_at(&full, cx) {
            return self.status_message("Save this file before viewing blame; blame reads saved contents", cx);
        }
        if pipelines::writing(&full, cx) {
            return self.status_message("Wait for the file write to finish", cx);
        }
        let label = format!("Blame · {}", path.file_name().unwrap_or_default().to_string_lossy());
        self.git_read_view(
            label,
            move || {
                let lines = git::blame(&root, &path)?;
                let mut text = format!(
                    "Git blame · {} · saved contents\nCommit    Author · Date (local time) | Line | Text\n\n",
                    path.display()
                );
                for (i, line) in lines.iter().enumerate() {
                    let uncommitted = line.commit.bytes().all(|b| b == b'0');
                    let id = if uncommitted { "Working tree" } else { &line.commit[..8] };
                    let date = if uncommitted { "Uncommitted".into() } else { blame_date(line.timestamp) };
                    text.push_str(&format!("{id:<12} {} · {date} | {:>5} | {}\n", line.author, i + 1, line.text));
                }
                if lines.is_empty() {
                    text.push_str("No lines to annotate.\n");
                }
                Ok(text)
            },
            w,
            cx,
        );
    }

    fn git_read_view(
        &mut self,
        label: String,
        read: impl FnOnce() -> Result<String, String> + Send + 'static,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_message("Reading Git…", cx);
        let generation = self.restore_gen;
        let task = cx.background_spawn(async move { read() });
        cx.spawn_in(w, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, w, cx| {
                if ws.restore_gen != generation {
                    return;
                }
                match result {
                    Ok(text) => {
                        let viewer = diff_view(&text, cx);
                        viewer.update(cx, |e, _| e.label = Some(label));
                        ws.attach(viewer, true, w, cx);
                    }
                    Err(e) => ws.status_message(e, cx),
                }
            })
            .ok();
        })
        .detach();
    }
}

fn blame_date(seconds: i64) -> String {
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    let mut buffer = [0u8; 64];
    // SAFETY: both buffers have their declared sizes; strftime only reads an initialized tm.
    let length = unsafe {
        if libc::localtime_r(&seconds, local.as_mut_ptr()).is_null() {
            return "Unknown date".into();
        }
        libc::strftime(buffer.as_mut_ptr().cast(), buffer.len(), c"%Y-%m-%d %H:%M:%S %z".as_ptr(), local.as_ptr())
    };
    String::from_utf8_lossy(&buffer[..length]).into_owned()
}
