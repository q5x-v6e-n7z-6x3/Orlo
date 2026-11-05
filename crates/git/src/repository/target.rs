//! Pinned repository and HEAD identity shared by prepared Git operations.

use crate::operations::{command, output, run};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    root: PathBuf,
    git_dir: PathBuf,
    identity: (u64, u64),
    root_identity: (u64, u64),
    pub(crate) head: Option<String>,
    reference: Option<String>,
}

impl Target {
    pub(crate) fn git_dir_identity(&self) -> (u64, u64) {
        self.identity
    }

    pub(crate) fn read(root: &Path) -> Result<Self, String> {
        let root = std::fs::canonicalize(root).map_err(|e| format!("Could not read repository: {e}"))?;
        let root_meta = std::fs::metadata(&root).map_err(|e| format!("Could not read repository identity: {e}"))?;
        let bytes = run(command(&root)?.args(["rev-parse", "--absolute-git-dir"]))?;
        let git_dir = crate::operations::path_from(bytes.strip_suffix(b"\n").unwrap_or(&bytes));
        let meta = std::fs::metadata(&git_dir).map_err(|e| format!("Could not read repository identity: {e}"))?;
        let result = output(command(&root)?.args(["rev-parse", "--verify", "--quiet", "HEAD"]))?;
        let head = if result.status.success() {
            Some(String::from_utf8(result.stdout).map_err(|_| "Invalid HEAD identifier")?.trim().to_string())
        } else if result.status.code() == Some(1) {
            None
        } else {
            return Err("Could not read repository HEAD".into());
        };
        let result = output(command(&root)?.args(["symbolic-ref", "--quiet", "HEAD"]))?;
        let reference = if result.status.success() {
            Some(String::from_utf8(result.stdout).map_err(|_| "Invalid HEAD reference")?.trim().to_string())
        } else if result.status.code() == Some(1) {
            None
        } else {
            return Err("Could not read repository HEAD reference".into());
        };
        Ok(Self {
            root,
            git_dir,
            identity: (meta.dev(), meta.ino()),
            root_identity: (root_meta.dev(), root_meta.ino()),
            head,
            reference,
        })
    }
}
