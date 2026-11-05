//! Index mutations and access to the indexed text baseline.

mod index;
mod selection;

pub use index::index_text;
pub(crate) use index::{blob_text, entry, index_blob, object_id, too_large};
pub use selection::{apply_selection, StageSelection};

use crate::diff::has_conflict_markers;
use crate::operations::{check_file, command, has_head, run};
use crate::status::FileStatus;
use std::path::Path;

pub fn stage(root: &Path, file: &FileStatus) -> Result<(), String> {
    check_file(file)?;
    let unmerged = run(command(root)?.args(["ls-files", "--unmerged", "-z", "--"]).arg(&file.path))?;
    if file.is_conflicted() || !unmerged.is_empty() {
        match std::fs::read(root.join(&file.path)) {
            Ok(bytes) if has_conflict_markers(&String::from_utf8_lossy(&bytes)) => {
                return Err("Resolve and save every conflict marker before staging this file".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not read the resolved file: {error}")),
        }
    }
    let mut git = command(root)?;
    git.args(["add", "--all", "--"]).arg(&file.path);
    // An index rename has already removed the old path; a worktree rename has not.
    git.args(file.original_path.iter().filter(|_| file.worktree == 'R'));
    run(&mut git).map(|_| ())
}

/// Restore index entries only. Even an unborn branch never removes worktree files.
pub fn unstage(root: &Path, file: &FileStatus) -> Result<(), String> {
    check_file(file)?;
    let mut git = command(root)?;
    if has_head(root)? {
        git.args(["reset", "--quiet", "HEAD", "--"]);
    } else {
        git.args(["rm", "--cached", "--force", "--ignore-unmatch", "--"]);
    }
    git.arg(&file.path).args(file.rename_source());
    run(&mut git).map(|_| ())
}
