//! Prepared local worktree operations. Git owns checkout/removal protection and locking.

use crate::operations::{command, path_from, run};
use crate::repository::Target;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub current: bool,
    pub main: bool,
    pub locked: Option<String>,
    /// Native eligibility/reason under the same immediate expiry policy as explicit Prune.
    pub prunable: Option<String>,
    pub missing: bool,
    target: Option<Target>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktrees {
    pub entries: Vec<Worktree>,
    pub branches: Vec<String>,
    pub common_dir: PathBuf,
    target: Target,
    common_identity: (u64, u64),
    listing: Vec<u8>,
    registrations: Vec<(PathBuf, (u64, u64), Vec<u8>)>,
}

fn identity(path: &Path) -> Result<(u64, u64), String> {
    let m = std::fs::metadata(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    Ok((m.dev(), m.ino()))
}

fn git_path(root: &Path, arg: &str) -> Result<PathBuf, String> {
    let bytes = run(command(root)?.args(["rev-parse", "--path-format=absolute", arg]))?;
    let path = path_from(bytes.strip_suffix(b"\n").unwrap_or(&bytes));
    std::fs::canonicalize(path).map_err(|e| format!("Could not read repository path: {e}"))
}

pub fn prepare_worktrees(root: &Path) -> Result<Worktrees, String> {
    let target = Target::read(root)?;
    let current = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let common_dir = git_path(root, "--git-common-dir")?;
    let common_identity = identity(&common_dir)?;
    let listing = run(command(root)?.args(["worktree", "list", "--porcelain", "-z", "--expire", "now"]))?;
    let mut entries = Vec::new();
    let mut fields = listing.split(|b| *b == 0).peekable();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let path = field.strip_prefix(b"worktree ").ok_or("Invalid Git worktree listing")?;
        let path = path_from(path);
        let mut branch = None;
        let mut locked = None;
        let mut prunable = None;
        let mut bare = false;
        while fields.peek().is_some_and(|f| !f.is_empty()) {
            let f = fields.next().unwrap();
            if let Some(name) = f.strip_prefix(b"branch refs/heads/") {
                branch = Some(String::from_utf8(name.to_vec()).map_err(|_| "Invalid branch name")?);
            } else if f == b"locked" {
                locked = Some(String::new());
            } else if let Some(reason) = f.strip_prefix(b"locked ") {
                locked = Some(String::from_utf8_lossy(reason).into_owned());
            } else if f == b"prunable" {
                prunable = Some(String::new());
            } else if let Some(reason) = f.strip_prefix(b"prunable ") {
                prunable = Some(String::from_utf8_lossy(reason).into_owned());
            } else if f == b"bare" {
                bare = true;
            }
        }
        let missing = !path.exists();
        let entry_target = if missing || bare {
            None
        } else {
            if git_path(&path, "--show-toplevel")? != path || git_path(&path, "--git-common-dir")? != common_dir {
                return Err(format!(
                    "Worktree registration does not match {}. Repair it with Git before continuing.",
                    path.display()
                ));
            }
            Some(Target::read(&path)?)
        };
        entries.push(Worktree {
            current: path == current,
            main: entries.is_empty(),
            path,
            branch,
            locked,
            prunable,
            missing,
            target: entry_target,
        });
    }
    let mut registrations = Vec::new();
    let admin = common_dir.join("worktrees");
    if admin.exists() {
        for dir in std::fs::read_dir(&admin).map_err(|e| e.to_string())? {
            let path = dir.map_err(|e| e.to_string())?.path();
            let gitdir =
                std::fs::read(path.join("gitdir")).map_err(|e| format!("Could not read worktree registration: {e}"))?;
            registrations.push((path.clone(), identity(&path)?, gitdir));
        }
        registrations.sort_by(|a, b| a.0.cmp(&b.0));
    }
    for entry in entries.iter().filter(|e| !e.main && e.target.is_some()) {
        let expected = std::fs::canonicalize(entry.path.join(".git")).map_err(|e| e.to_string())?;
        let git_dir_id = entry.target.as_ref().unwrap().git_dir_identity();
        let matches = registrations.iter().any(|(admin, id, backlink)| {
            if *id != git_dir_id {
                return false;
            }
            let backlink = path_from(backlink.strip_suffix(b"\n").unwrap_or(backlink));
            let backlink = if backlink.is_absolute() { backlink } else { admin.join(backlink) };
            std::fs::canonicalize(backlink).is_ok_and(|path| path == expected)
        });
        if !matches {
            return Err(format!(
                "Worktree registration does not match {}. Repair it with Git before continuing.",
                entry.path.display()
            ));
        }
    }
    let branches = crate::branches::branches(root)?;
    Ok(Worktrees { entries, branches, common_dir, target, common_identity, listing, registrations })
}

impl Worktrees {
    pub fn validate(&self, root: &Path) -> Result<(), String> {
        if prepare_worktrees(root).as_ref() != Ok(self) {
            return Err("The repository or worktree registrations changed; reopen Worktrees".into());
        }
        Ok(())
    }
    fn selected(&self, root: &Path, entry: &Worktree) -> Result<(), String> {
        self.validate(root)?;
        if !self.entries.contains(entry) {
            return Err("The selected worktree changed; reopen Worktrees".into());
        }
        Ok(())
    }
}

/// An explicit existing local branch or a new branch at HEAD; never reset a branch.
pub fn create_worktree(root: &Path, context: &Worktrees, path: &Path, branch: &str, new: bool) -> Result<(), String> {
    context.validate(root)?;
    if !path.is_absolute() || path.file_name().is_none() {
        return Err("Enter an absolute destination folder".into());
    }
    if path.parent().is_none_or(|p| !p.is_dir()) {
        return Err("The destination parent folder must already exist".into());
    }
    if branch.is_empty() || branch.starts_with('-') || branch == "HEAD" {
        return Err("Enter a valid local branch name".into());
    }
    run(command(root)?.args(["check-ref-format", &format!("refs/heads/{branch}")]))?;
    if !new && !context.branches.iter().any(|b| b == branch) {
        return Err("Choose an existing local branch".into());
    }
    let mut cmd = command(root)?;
    cmd.args(["worktree", "add"]);
    if new {
        cmd.args(["-b", branch]);
    }
    cmd.arg("--").arg(path);
    if !new {
        cmd.arg(branch);
    }
    run(&mut cmd).map(|_| ())
}

pub fn open_worktree(root: &Path, context: &Worktrees, entry: &Worktree) -> Result<PathBuf, String> {
    context.selected(root, entry)?;
    if entry.missing || entry.target.is_none() {
        return Err("This worktree folder is missing or bare and cannot be opened".into());
    }
    Ok(entry.path.clone())
}

pub fn remove_worktree(root: &Path, context: &Worktrees, entry: &Worktree) -> Result<(), String> {
    context.selected(root, entry)?;
    if entry.main {
        return Err("The main worktree cannot be removed".into());
    }
    if entry.current {
        return Err("The current worktree cannot be removed".into());
    }
    if entry.locked.is_some() {
        return Err("This worktree is locked; unlock it with Git before removing it".into());
    }
    if entry.missing {
        return Err("This worktree is missing; use Prune Worktrees instead".into());
    }
    run(command(root)?.args(["worktree", "remove", "--"]).arg(&entry.path)).map(|_| ())
}

/// Explicit immediate expiry; Git retains existing and locked worktree registrations.
pub fn prune_worktrees(root: &Path, context: &Worktrees) -> Result<(), String> {
    context.validate(root)?;
    run(command(root)?.args(["worktree", "prune", "--expire", "now"])).map(|_| ())
}
