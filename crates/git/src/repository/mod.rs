//! Repository discovery and initialization.

mod target;
pub(crate) use target::Target;

use crate::errors::failure;
use crate::operations::{command, output, path_from, run};
use crate::status::{parse_status, FileStatus};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub root: PathBuf,
    pub branch: String,
    pub files: Vec<FileStatus>,
}

impl Repository {
    #[cfg(test)]
    pub fn staged_count(&self) -> usize {
        self.files.iter().filter(|f| f.has_staged()).count()
    }

    #[cfg(test)]
    pub fn unstaged_count(&self) -> usize {
        self.files.iter().filter(|f| f.has_unstaged()).count()
    }

    pub fn conflict_count(&self) -> usize {
        self.files.iter().filter(|f| f.is_conflicted()).count()
    }
}

/// Set up the selected folder only; never create a folder or reinitialize existing metadata.
pub fn init(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("Choose an existing project folder to set up Git".into());
    }
    let existing = output(command(path)?.args(["rev-parse", "--git-dir"]))?;
    if existing.status.success() {
        return Err("This folder already belongs to a Git repository".into());
    }
    let error = failure(&existing);
    if !error.contains("not a git repository") {
        return Err(error);
    }
    if std::fs::symlink_metadata(path.join(".git")).is_ok() {
        return Err("This folder already contains Git metadata".into());
    }
    run(command(path)?.arg("init")).map(|_| ())
}

/// Find Git's worktree root, including linked worktrees and paths of deleted files.
pub fn discover(path: &Path) -> Result<Option<Repository>, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(path)
    };
    let directory =
        absolute.ancestors().find(|p| p.is_dir()).ok_or_else(|| "No existing folder for this path".to_string())?;
    // where the worktree starts reads no file a repository's programs filter, so it is asked
    // before trust: a paused repository can be named, and trusted
    let result = output(crate::operations::base(directory).args(["rev-parse", "--show-toplevel"]))?;
    if !result.status.success() {
        let error = failure(&result);
        if error.contains("not a git repository") || error.contains("must be run in a work tree") {
            return Ok(None);
        }
        return Err(error);
    }
    let bytes = result.stdout.strip_suffix(b"\n").unwrap_or(&result.stdout);
    status(path_from(bytes), &[]).map(Some)
}

/// The top of the worktree holding `dir`, if it is in one: what a paused repository's error
/// asks the user to trust. Asking runs nothing the repository's settings name.
pub fn toplevel(dir: &Path) -> Option<PathBuf> {
    let out = output(crate::operations::base(dir).args(["rev-parse", "--show-toplevel"])).ok()?;
    let bytes = out.status.success().then_some(out.stdout)?;
    Some(path_from(bytes.strip_suffix(b"\n").unwrap_or(&bytes)))
}

/// The worktree `root`'s status: of every file, or only of `paths` when there are any.
pub(crate) fn status(root: PathBuf, paths: &[&Path]) -> Result<Repository, String> {
    let status = run(command(&root)?
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--renames",
            // a submodule's commit, not its files: their status would run its own settings
            "--ignore-submodules=dirty",
            "--",
        ])
        .args(paths))?;
    parse_status(root, &status)
}
