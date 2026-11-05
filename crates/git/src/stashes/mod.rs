//! Stash operations pin both repository context and the complete ordered stash stack.

use crate::operations::{command, run};
use crate::repository::Target;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub selector: String,
    pub id: String,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stashes {
    target: Target,
    pub entries: Vec<StashEntry>,
}

fn entries(root: &Path) -> Result<Vec<StashEntry>, String> {
    let bytes = run(command(root)?.args(["stash", "list", "-z", "--format=%gd%x00%H%x00%gs"]))?;
    let mut fields = bytes.split(|b| *b == 0);
    let mut entries = Vec::new();
    while let Some(selector) = fields.next().filter(|s| !s.is_empty()) {
        let id = fields.next().ok_or("Invalid Git stash identifier")?;
        let subject = fields.next().ok_or("Invalid Git stash subject")?;
        if !matches!(id.len(), 40 | 64) || !id.iter().all(u8::is_ascii_hexdigit) {
            return Err("Invalid Git stash identifier".into());
        }
        entries.push(StashEntry {
            selector: String::from_utf8_lossy(selector).into_owned(),
            id: String::from_utf8_lossy(id).into_owned(),
            subject: String::from_utf8_lossy(subject).into_owned(),
        });
    }
    Ok(entries)
}

pub fn prepare_stashes(root: &Path) -> Result<Stashes, String> {
    let target = Target::read(root)?;
    let prepared = Stashes { target, entries: entries(root)? };
    prepared.validate(root)?;
    Ok(prepared)
}

impl Stashes {
    pub fn validate(&self, root: &Path) -> Result<(), String> {
        if Target::read(root)? != self.target {
            return Err("The repository or HEAD changed; reopen Stashes".into());
        }
        if entries(root)? != self.entries {
            return Err("The stash stack changed or an entry is missing; reopen Stashes".into());
        }
        Ok(())
    }

    fn selected(&self, root: &Path, entry: &StashEntry) -> Result<(), String> {
        self.validate(root)?;
        if !self.entries.contains(entry) {
            return Err("Choose an entry from the current stash list".into());
        }
        Ok(())
    }
}

pub fn create_stash(root: &Path, context: &Stashes, message: &str, untracked: bool) -> Result<(), String> {
    if message.chars().any(char::is_control) {
        return Err("The stash message must be one line without control characters".into());
    }
    context.validate(root)?;
    let mut cmd = command(root)?;
    // stash push internally uses the magic :/ pathspec to clean untracked files.
    // There are no caller pathspecs here; literal mode would silently skip that cleanup.
    cmd.args(["--no-literal-pathspecs", "stash", "push"]);
    if untracked {
        cmd.arg("--include-untracked");
    }
    if !message.trim().is_empty() {
        cmd.arg("--message").arg(message);
    }
    run(&mut cmd)?;
    if entries(root)? == context.entries {
        return Err("No saved changes to stash (untracked files are excluded unless selected)".into());
    }
    Ok(())
}

pub fn apply_stash(root: &Path, context: &Stashes, entry: &StashEntry, index: bool, pop: bool) -> Result<(), String> {
    context.selected(root, entry)?;
    if pop {
        // Unsupported native ref/object formats must fail before changing the worktree.
        drop_repository(root).map_err(|e| format!("Cannot pop this stash; no changes applied: {e}"))?;
    }
    let mut cmd = command(root)?;
    cmd.args(["stash", "apply"]);
    if index {
        cmd.arg("--index");
    }
    // Applying the object ID cannot apply a different stash if an external process pushes.
    run(cmd.arg(&entry.id)).map_err(|e| format!("{e}\nThe stash was kept; inspect conflicts before retrying."))?;
    if pop {
        // A failed apply never drops. An externally changed stack also stays intact.
        drop_stash(root, context, entry).map_err(|e| format!("Changes applied, but stash removal failed: {e}\nRefresh Stashes before retrying; do not apply these changes again."))?;
    }
    Ok(())
}

/// Caller must confirm permanent removal before invoking this operation.
pub fn drop_stash(root: &Path, context: &Stashes, entry: &StashEntry) -> Result<(), String> {
    context.selected(root, entry)?;
    #[cfg(test)]
    crate::tests::stash_drop_boundary(root, false);
    let repo = drop_repository(root)?;
    let mut transaction = repo.transaction().map_err(removal_error)?;
    transaction.lock_ref("refs/stash").map_err(removal_error)?;
    // Git's ref lock also excludes CLI stash push/drop/reflog changes. The earlier
    // UI/CLI validation is insufficient: read both views again while holding it.
    context.selected(root, entry)?;
    let mut reflog = repo.reflog("refs/stash").map_err(removal_error)?;
    if reflog.len() != context.entries.len()
        || reflog.iter().zip(&context.entries).enumerate().any(|(i, (logged, expected))| {
            logged.id_new().to_string() != expected.id || expected.selector != format!("stash@{{{i}}}")
        })
    {
        return Err("The stash log changed or cannot be matched safely; reopen Stashes".into());
    }
    let reference = repo.find_reference("refs/stash").map_err(removal_error)?;
    let first = reflog.get(0).ok_or("The stash is missing; reopen Stashes")?;
    if reference.target() != Some(first.id_new()) {
        return Err("The stash reference and log disagree; no stash removed".into());
    }
    // A removal adds no identity/trailer metadata and must not require user.name/email.
    let signature = first.committer().to_owned();
    let position = context
        .entries
        .iter()
        .position(|candidate| candidate == entry)
        .ok_or("The selected stash is missing; reopen Stashes")?;
    #[cfg(test)]
    crate::tests::stash_drop_boundary(root, true);
    reflog.remove(position, true).map_err(removal_error)?;
    match reflog.get(0) {
        None => transaction.remove("refs/stash").map_err(removal_error)?,
        Some(next) if position == 0 => transaction
            .set_target("refs/stash", next.id_new(), Some(&signature), "stash: remove selected entry")
            .map_err(removal_error)?,
        Some(_) => {} // Removing an older entry must leave the current ref unchanged.
    }
    transaction.set_reflog("refs/stash", reflog).map_err(removal_error)?;
    transaction.commit().map_err(|e| {
        format!("Stash removal may be incomplete: {e}. Refresh Stashes before retrying. Selected object: {}", entry.id)
    })
}

fn removal_error(error: git2::Error) -> String {
    format!("Cannot safely remove stash: {error}")
}

/// Open only this repository, without inherited Git environment/search redirection.
/// libgit2 rejects unsupported extensions (including reftable/SHA-256 in this build).
fn drop_repository(root: &Path) -> Result<git2::Repository, String> {
    git2::Repository::open_ext(root, git2::RepositoryOpenFlags::NO_SEARCH, std::iter::empty::<&Path>())
        .map_err(removal_error)
}

pub fn stash_preview(root: &Path, context: &Stashes, entry: &StashEntry) -> Result<String, String> {
    context.selected(root, entry)?;
    let (bytes, truncated) = bounded_patch(command(root)?.args([
        "-c",
        "core.quotePath=false",
        "stash",
        "show",
        "--patch",
        "--include-untracked",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        &entry.id,
    ]))?;
    context.selected(root, entry)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if text.is_empty() {
        text.push_str("No file changes in this stash.\n");
    }
    if truncated {
        text.push_str("\n\n[Preview truncated at 2 MiB; the stash itself is unchanged.]\n");
    }
    Ok(text)
}

/// Drain both pipes without retaining an unbounded patch or error. Binary patches stay summaries.
fn bounded_patch(cmd: &mut std::process::Command) -> Result<(Vec<u8>, bool), String> {
    use std::io::Read;
    use std::process::Stdio;
    fn read(mut pipe: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
        let mut bytes = Vec::new();
        let mut buf = [0; 8192];
        let mut truncated = false;
        loop {
            let n = pipe.read(&mut buf)?;
            if n == 0 {
                return Ok((bytes, truncated));
            }
            let keep = n.min(limit - bytes.len());
            bytes.extend_from_slice(&buf[..keep]);
            truncated |= keep < n;
        }
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (out, err, status) = std::thread::scope(|s| {
        let errors = s.spawn(move || read(stderr, 16 << 10));
        let out = read(stdout, 2 << 20);
        (out, errors.join().unwrap(), child.wait())
    });
    let (err, _) = err.map_err(|e| e.to_string())?;
    if !status.map_err(|e| e.to_string())?.success() {
        return Err(format!("Could not preview stash: {}", String::from_utf8_lossy(&err)));
    }
    out.map_err(|e| e.to_string())
}
