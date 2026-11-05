//! File-scoped commit history and patches, following renames.

use crate::operations::{check_path, command, has_head, path_from, run};
use std::path::{Path, PathBuf};

pub const HISTORY_PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCommit {
    pub id: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    /// The name this file had in this commit, before any later renames.
    pub path: PathBuf,
}

pub fn file_history(root: &Path, path: &Path, skip: usize) -> Result<Vec<FileCommit>, String> {
    check_path(path)?;
    if !has_head(root)? {
        return Ok(Vec::new());
    }
    let bytes = run(command(root)?
        .args([
            "log",
            "--follow",
            "--diff-merges=first-parent",
            "--no-decorate",
            "--no-show-signature",
            "--name-only",
            "-z",
            "--format=%H%x00%an%x00%aI%x00%s",
            // Skipping in Git bypasses rename traversal, so paginate after following the path.
            &format!("--max-count={}", skip.saturating_add(HISTORY_PAGE_SIZE)),
            "--",
        ])
        .arg(path))?;
    let mut fields = bytes.split(|b| *b == 0);
    let mut commits = Vec::new();
    while let Some(id) = fields.next() {
        if id.is_empty() {
            continue;
        }
        let mut next = || fields.next().ok_or("Git returned malformed history output");
        let author = String::from_utf8_lossy(next()?).into_owned();
        let date = String::from_utf8_lossy(next()?).into_owned();
        let subject = String::from_utf8_lossy(next()?).into_owned();
        let path = next()?.strip_prefix(b"\n").ok_or("Git returned malformed history path")?;
        commits.push(FileCommit {
            id: String::from_utf8_lossy(id).into_owned(),
            author,
            date,
            subject,
            path: path_from(path),
        });
    }
    Ok(commits.into_iter().skip(skip).collect())
}

pub fn commit_diff(root: &Path, commit: &FileCommit) -> Result<String, String> {
    check_path(&commit.path)?;
    if !matches!(commit.id.len(), 40 | 64) || !commit.id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Choose a commit from this file's history".into());
    }
    let bytes = run(command(root)?
        .args([
            "log",
            "-1",
            "--follow",
            "--diff-merges=first-parent",
            "--patch",
            "--format=fuller",
            "--no-decorate",
            "--no-notes",
            "--no-show-signature",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            &commit.id,
            "--",
        ])
        .arg(&commit.path))?;
    String::from_utf8(bytes).map_err(|_| "This commit's patch is not valid UTF-8".into())
}
