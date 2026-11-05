//! Local branch enumeration and switching.

use crate::operations::{command, run};
use std::path::Path;

/// Local branches only; switching never guesses a remote or creates a branch.
pub fn branches(root: &Path) -> Result<Vec<String>, String> {
    let bytes =
        run(command(root)?.args(["for-each-ref", "--sort=refname", "--format=%(refname:lstrip=2)", "refs/heads/"]))?;
    let text = String::from_utf8(bytes).map_err(|_| "A branch name is not valid UTF-8".to_string())?;
    Ok(text.lines().map(str::to_owned).collect())
}

pub fn switch(root: &Path, branch: &str) -> Result<(), String> {
    if branch.is_empty() || branch.starts_with('-') {
        return Err("Choose a local branch".into());
    }
    run(command(root)?.args(["switch", "--no-guess", "--", branch])).map(|_| ())
}

fn check_name(root: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() || name.starts_with('-') || name == "HEAD" {
        return Err("Enter a valid local branch name".into());
    }
    run(command(root)?.args(["check-ref-format", &format!("refs/heads/{name}")])).map(|_| ())
}

/// Create a local branch at HEAD without changing files or switching branches.
pub fn create_branch(root: &Path, name: &str) -> Result<(), String> {
    check_name(root, name)?;
    run(command(root)?.args(["branch", "--", name])).map(|_| ())
}

pub fn rename_branch(root: &Path, old: &str, new: &str) -> Result<(), String> {
    check_name(root, old)?;
    check_name(root, new)?;
    run(command(root)?.args(["branch", "--move", "--", old, new])).map(|_| ())
}

/// Git refuses the current branch, worktree branches, and branches with unmerged commits.
pub fn delete_branch(root: &Path, name: &str) -> Result<(), String> {
    check_name(root, name)?;
    run(command(root)?.args(["branch", "--delete", "--", name])).map(|_| ())
}
