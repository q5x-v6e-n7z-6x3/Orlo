//! Git porcelain status and repository-relative file paths.

use crate::operations::path_from;
use crate::repository::Repository;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// Paths are relative to Repository::root; preserve their original OS bytes.
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
}

impl FileStatus {
    pub fn is_conflicted(&self) -> bool {
        self.index == 'U' || self.worktree == 'U' || matches!((self.index, self.worktree), ('A', 'A') | ('D', 'D'))
    }

    pub fn is_untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }

    pub fn has_staged(&self) -> bool {
        !self.is_conflicted() && !matches!(self.index, ' ' | '?' | '!')
    }

    pub fn has_unstaged(&self) -> bool {
        !self.is_conflicted() && !matches!(self.worktree, ' ' | '!')
    }

    /// The old path of a rename, in the index or the worktree.
    pub(crate) fn rename_source(&self) -> Option<&PathBuf> {
        (self.original_path.as_ref()).filter(|_| self.index == 'R' || self.worktree == 'R')
    }
}

pub(crate) fn parse_status(root: PathBuf, bytes: &[u8]) -> Result<Repository, String> {
    let malformed = || "Git returned malformed status output".to_string();
    let mut branch = String::new();
    let mut oid = String::new();
    let mut files = Vec::new();
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        if let Some(name) = record.strip_prefix(b"# branch.head ") {
            branch = String::from_utf8_lossy(name).into_owned();
        } else if let Some(hash) = record.strip_prefix(b"# branch.oid ") {
            oid = String::from_utf8_lossy(hash).into_owned();
        } else if record.starts_with(b"# ") || record.starts_with(b"! ") {
            continue;
        } else if let Some(path) = record.strip_prefix(b"? ") {
            files.push(FileStatus { path: path_from(path), original_path: None, index: '?', worktree: '?' });
        } else {
            let fields = match record.first() {
                Some(b'1') => 9,
                Some(b'2') => 10,
                Some(b'u') => 11,
                _ => return Err(malformed()),
            };
            let parts: Vec<_> = record.splitn(fields, |b| *b == b' ').collect();
            if parts.len() != fields || parts[1].len() != 2 || parts[fields - 1].is_empty() {
                return Err(malformed());
            }
            let original_path =
                if record[0] == b'2' { Some(path_from(records.next().ok_or_else(malformed)?)) } else { None };
            let status = |b| if b == b'.' { ' ' } else { b as char };
            files.push(FileStatus {
                path: path_from(parts[fields - 1]),
                original_path,
                index: status(parts[1][0]),
                worktree: status(parts[1][1]),
            });
        }
    }
    if branch == "(detached)" {
        branch = format!("detached {}", oid.get(..7).unwrap_or(&oid));
    } else if oid == "(initial)" {
        branch.push_str(" (unborn)");
    }
    Ok(Repository { root, branch, files })
}
