//! Partial staging acts on the reviewed side and its selected lines, never a live buffer.

use super::*;

impl Workspace {
    forward_actions! {
    pub(crate) git_stage_lines(GitStageLines, w, cx) => git_apply_selection(false, false, w, cx);
    pub(crate) git_stage_hunk(GitStageHunk, w, cx) => git_apply_selection(false, true, w, cx);
    pub(crate) git_unstage_hunk(GitUnstageHunk, w, cx) => git_apply_selection(true, true, w, cx);
    }

    fn git_apply_selection(&mut self, staged: bool, whole_hunks: bool, w: &mut Window, cx: &mut Context<Self>) {
        if !self.git.sidebar_open || !self.git.selection_active {
            return self.status_message("Select a change in the Git sidebar first", cx);
        }
        let Some((root, path, side)) = self.git.diff_target.clone() else {
            return self.status_message("Select a changed file in the Git sidebar first", cx);
        };
        if side != staged
            || self.git.selected.as_ref() != Some(&(root.clone(), path.clone()))
            || self.git.staged != staged
        {
            return self.status_message("Select the change on the matching side of the index first", cx);
        }
        let Some(GitDiff::Sides { versions, viewers, .. }) = &self.git.diff else {
            return self.status_message("Stage this file as a whole; individual changes require a text diff", cx);
        };
        let i = viewers.iter().position(|v| v.read(cx).is_focused(w)).unwrap_or(self.git.diff_side);
        let state = viewers[i].read(cx).read_state(cx);
        let range = state.view.selections[state.primary()].range();
        let start = state.doc.rope.char_to_line(range.start);
        let end = state.doc.rope.char_to_line(range.end.saturating_sub(usize::from(!range.is_empty())));
        let selection = git::StageSelection { old_side: i == 0, lines: start..end + 1, whole_hunks };
        let versions = versions.clone();
        if !staged && self.unsaved_at(&root.join(&path), cx) {
            return self.status_message("Save this file before staging", cx);
        }
        if pipelines::writing_under(&root, cx) || self.saves.busy() {
            return self.status_message("Wait for repository writes to finish", cx);
        }
        let Some(file) = self.git_file_status(&root, &path).cloned() else {
            return;
        };
        self.git_run(root.clone(), move || git::apply_selection(&root, &file, staged, &versions, &selection), cx);
    }

    pub(super) fn git_selection_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .px(px(8.))
            .pb(px(4.))
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .when(matches!(self.git.diff, Some(GitDiff::Sides { .. })), |d| {
                if self.git.staged {
                    d.child(button("Unstage Hunk", Self::git_unstage_hunk, GitUnstageHunk, cx))
                } else {
                    d.child(button("Stage Lines", Self::git_stage_lines, GitStageLines, cx)).child(button(
                        "Stage Hunk",
                        Self::git_stage_hunk,
                        GitStageHunk,
                        cx,
                    ))
                }
            })
            .into_any_element()
    }
}
