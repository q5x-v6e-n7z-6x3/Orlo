//! Git repository, index, worktree, and text changes without UI dependencies.

pub mod blame;
pub mod branches;
pub mod commits;
pub mod diff;
mod errors;
pub mod history;
mod operations;
pub mod remotes;
pub mod repository;
pub mod staging;
pub mod stashes;
pub mod status;
pub mod tags;
pub mod worktrees;

/// Compatibility surface for callers of the original Git integration module.
pub mod git {
    pub use crate::blame::{blame, BlameLine};
    pub use crate::branches::{branches, create_branch, delete_branch, rename_branch, switch};
    pub use crate::commits::{
        commit, commit_draft, commit_draft_as, prepare_commit, CommitAuthor, CommitDraft, PreparedCommit,
    };
    pub use crate::diff::{
        conflicts, diff, has_conflict_markers, head_text, resolve_conflict, versions, ConflictBlock, ConflictChoice,
        Versions,
    };
    pub use crate::history::{commit_diff, file_history, FileCommit, HISTORY_PAGE_SIZE};
    pub use crate::operations::{own_program, paused, PAUSED};
    pub use crate::remotes::{prepare_remotes, remote_operation, Remote, RemoteOperation, Remotes};
    pub use crate::repository::{discover, init, toplevel, Repository};
    pub use crate::staging::{apply_selection, index_text, stage, unstage, StageSelection};
    pub use crate::stashes::{
        apply_stash, create_stash, drop_stash, prepare_stashes, stash_preview, StashEntry, Stashes,
    };
    pub use crate::status::FileStatus;
    pub use crate::tags::{create_tag, delete_tag, prepare_tag, prepare_tags, PreparedTag, Tag, Tags};
    pub use crate::worktrees::{
        create_worktree, open_worktree, prepare_worktrees, prune_worktrees, remove_worktree, Worktree, Worktrees,
    };
}

/// Line changes between a saved or indexed baseline and the current buffer.
pub mod changes {
    pub use crate::diff::changes::{between, Hunk};
}

#[cfg(test)]
mod tests;
