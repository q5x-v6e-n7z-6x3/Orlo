//! Shared Git process execution and literal-path validation.

use crate::errors::failure;
use crate::status::FileStatus;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

pub(crate) fn input(command: &mut Command, bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run Git: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    // Hooks may fill stdout before Git reads stdin; drain output while writing input.
    let (writing, result) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(bytes));
        let result = child.wait_with_output();
        let writing = writer.join().unwrap_or_else(|_| Err(std::io::Error::other("Git input writer panicked")));
        (writing, result)
    });
    let result = result.map_err(|e| format!("Could not wait for Git: {e}"))?;
    if !result.status.success() {
        return Err(failure(&result));
    }
    writing.map_err(|e| format!("Could not write to Git: {e}"))?;
    Ok(result.stdout)
}

/// How the error of a Git command Orlo would not run begins (`paused`): one in a repository
/// whose own settings run a program, in a folder the user has not trusted.
pub const PAUSED: &str = "Git is paused for this repository";

/// Whether `error` is a command refused for want of trust (`PAUSED`).
pub fn paused(error: &str) -> bool {
    error.starts_with(PAUSED)
}

/// The first of `dir`'s repository's own settings (its `.git/config`, a worktree's, and the
/// files they include) that makes ordinary commands run a program: a filter driver, which
/// `status`, `diff` and staging run on the files whose attributes name it. A downloaded folder
/// brings these along. The user's and the system's settings are the user's own (git-lfs puts
/// its filter there). Looked up again after a few seconds; reading settings runs nothing.
pub fn own_program(dir: &Path) -> Option<String> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static SEEN: Mutex<Vec<(PathBuf, Instant, Option<String>)>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    seen.retain(|(_, at, _)| at.elapsed() < Duration::from_secs(5));
    if let Some((_, _, found)) = seen.iter().find(|(d, _, _)| d == dir) {
        return found.clone();
    }
    let key = r"^filter\..+\.(clean|smudge|process)$";
    let listed = base(dir).args(["config", "--show-scope", "--name-only", "--get-regexp", key]).output();
    let found = match listed {
        // 1: there are none
        Ok(out) if matches!(out.status.code(), Some(0 | 1)) => (String::from_utf8_lossy(&out.stdout).lines())
            .filter_map(|line| line.split_once('\t'))
            .find(|(scope, _)| matches!(*scope, "local" | "worktree"))
            .map(|(_, key)| key.to_string()),
        // a Git older than 2.26 cannot say where a setting comes from
        Ok(out) if out.status.code() == Some(129) => {
            Some("settings this Git cannot check: 2.26 or later is needed".into())
        }
        _ => None,
    };
    seen.push((dir.to_path_buf(), Instant::now(), found.clone()));
    found
}

/// A Git command in `root`, unless the repository's own settings run a program there and the
/// user has not trusted the folder (`own_program`): `status` alone would run it.
pub(crate) fn command(root: &Path) -> Result<Command, String> {
    if !orlo_utils::trust::trusted(root) {
        if let Some(key) = own_program(root) {
            return Err(format!(
                "{PAUSED}: its own settings run a program ({key}). Trust the folder to use Git in it."
            ));
        }
    }
    Ok(base(root))
}

/// `command` without the trust check, for what runs no repository program (`rev-parse`,
/// `config`).
pub(crate) fn base(root: &Path) -> Command {
    let mut command = Command::new("git");
    // a repository's own core.fsmonitor names a program `status` would run; Orlo needs none
    command.args(["--no-pager", "--literal-pathspecs", "-c", "core.fsmonitor=false", "-C"]).arg(root);
    command.env("LC_ALL", "C").env("GIT_TERMINAL_PROMPT", "0");
    // A launched editor must not inherit a terminal's alternate repository or index.
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] {
        command.env_remove(key);
    }
    command
}

pub(crate) fn output(command: &mut Command) -> Result<Output, String> {
    command.output().map_err(|e| format!("Could not run Git: {e}"))
}

pub(crate) fn run(command: &mut Command) -> Result<Vec<u8>, String> {
    let result = output(command)?;
    if result.status.success() {
        Ok(result.stdout)
    } else {
        Err(failure(&result))
    }
}

/// A path from Git's output, keeping its original OS bytes.
pub(crate) fn path_from(bytes: &[u8]) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

/// Whether HEAD names a commit; an unborn branch has none.
pub(crate) fn has_head(root: &Path) -> Result<bool, String> {
    let head = output(command(root)?.args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]))?;
    match head.status.code() {
        _ if head.status.success() => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failure(&head)),
    }
}

pub(crate) fn check_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("Git file paths must stay inside the repository".into());
    }
    Ok(())
}

pub(crate) fn check_file(file: &FileStatus) -> Result<(), String> {
    check_path(&file.path)?;
    if let Some(original) = &file.original_path {
        check_path(original)?;
    }
    Ok(())
}
