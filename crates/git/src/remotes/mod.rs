//! Explicit remote operations. Preparing a context reads only local state.

use crate::operations::{command, output, path_from, run};
use crate::repository::Target;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_urls: Vec<String>,
    pub push_urls: Vec<String>,
    pub fetch_refspecs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remotes {
    pub remotes: Vec<Remote>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub upstream_remote: Option<String>,
    pub upstream_ref: Option<String>,
    pub upstream_choices: Vec<String>,
    pub metadata_dirs: Vec<PathBuf>,
    pub config_files: Vec<PathBuf>,
    target: Target,
    config: Vec<u8>,
    refs: Vec<u8>,
    identities: Vec<(u64, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteOperation {
    Fetch { remote: String },
    Pull,
    Push { remote: String, branch: String, set_upstream: bool },
    AddRemote { name: String, url: String },
    EditRemote { name: String, url: String, push_url: Option<String> },
    RemoveRemote { name: String },
    SetUpstream { upstream: Option<String> },
}

fn text(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(run(command(root)?.args(args))?).map_err(|_| "Git returned non-UTF-8 remote information".into())
}

/// Canonicalize the existing ancestor, retaining the future destination of optional includes.
/// This keeps directory aliases and future save leases on the same absolute path.
fn config_path(path: &Path, paths: &mut Vec<PathBuf>, depth: usize) -> Result<(), String> {
    if depth >= 40 {
        return Err("Git configuration path contains too many symbolic links".into());
    }
    for ancestor in path.ancestors() {
        match std::fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for part in path.strip_prefix(ancestor).unwrap().components() {
                    match part {
                        std::path::Component::ParentDir => {
                            resolved.pop();
                        }
                        std::path::Component::CurDir => {}
                        part => resolved.push(part),
                    }
                }
                // A missing component followed by .. can reveal another existing symlink.
                if resolved != path {
                    config_path(&resolved, paths, depth + 1)?;
                }
                paths.push(resolved);
                return Ok(());
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
                if let Ok(target) = std::fs::read_link(ancestor) {
                    let target = if target.is_absolute() {
                        target
                    } else {
                        ancestor.parent().ok_or("Git configuration link has no parent")?.join(target)
                    };
                    config_path(&target.join(path.strip_prefix(ancestor).unwrap()), paths, depth + 1)?;
                    // Also retain the normalized dangling alias: saving through it and saving
                    // directly to its future target must both wait for this worker.
                }
            }
            Err(e) => return Err(format!("Could not read Git configuration path: {e}")),
        }
    }
    Err("Could not resolve Git configuration path".into())
}

/// Git 2.42+ reports applicable roots even when absent and applies its own HOME/XDG,
/// GIT_CONFIG_GLOBAL/SYSTEM and GIT_CONFIG_NOSYSTEM semantics. Origins cannot do that.
fn config_roots(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    for variable in ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"] {
        let result = output(command(root)?.args(["var", variable]))?;
        if result.status.code() == Some(1) && result.stdout.is_empty() && result.stderr.is_empty() {
            continue; // Native Git says this scope is disabled or has no applicable path.
        }
        if !result.status.success() {
            return Err(format!(
                "Cannot discover Git configuration roots; remote controls require Git 2.42 or newer: {}",
                crate::errors::failure(&result)
            ));
        }
        // `git var` has no NUL mode. Never mistake a newline in an environment-derived
        // path for another root. Included paths still use the NUL-delimited config API.
        let inputs: &[&str] = if variable == "GIT_CONFIG_SYSTEM" {
            &["GIT_CONFIG_SYSTEM"]
        } else if std::env::var_os("GIT_CONFIG_GLOBAL").is_some() {
            &["GIT_CONFIG_GLOBAL"]
        } else {
            &["HOME", "XDG_CONFIG_HOME"]
        };
        if inputs.iter().any(|key| std::env::var_os(key).is_some_and(|p| p.as_bytes().contains(&b'\n'))) {
            return Err("Cannot safely discover a Git configuration root containing a newline; use a root path without newlines for remote controls".into());
        }
        let bytes = result.stdout.strip_suffix(b"\n").unwrap_or(&result.stdout);
        if bytes.is_empty() || variable == "GIT_CONFIG_SYSTEM" && bytes.contains(&b'\n') {
            return Err("Git returned ambiguous configuration root paths".into());
        }
        for bytes in bytes.split(|b| *b == b'\n') {
            if bytes.is_empty() {
                return Err("Git returned an empty configuration root path".into());
            }
            let path = path_from(bytes);
            config_path(&if path.is_absolute() { path } else { root.join(path) }, paths, 0)?;
        }
    }
    Ok(())
}

fn config_paths(root: &Path, origins: &[u8]) -> Result<Vec<PathBuf>, String> {
    let origin = |bytes: &[u8]| {
        bytes.strip_prefix(b"file:").map(|p| {
            let path = path_from(p);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
    };
    let mut paths = Vec::new();
    config_roots(root, &mut paths)?;
    let fields: Vec<_> = origins.split(|b| *b == 0).collect();
    for entry in fields.as_chunks::<2>().0 {
        if let Some(path) = origin(entry[0]) {
            config_path(&path, &mut paths, 0)?;
        }
    }
    // Origins alone omit files with no settings. Let Git parse/traverse includes and expand
    // ~ / %(prefix), then resolve relative values against their declaring file. Guard all
    // declared conditional targets returned by Git, without duplicating its condition engine.
    let includes = output(command(root)?.args([
        "config",
        "--includes",
        "--null",
        "--show-origin",
        "--path",
        "--get-regexp",
        r"^(include\.path|includeif\..*\.path)$",
    ]))?;
    if !includes.status.success() && includes.status.code() != Some(1) {
        return Err(crate::errors::failure(&includes));
    }
    let fields: Vec<_> = includes.stdout.split(|b| *b == 0).collect();
    for entry in fields.as_chunks::<2>().0 {
        let Some(newline) = entry[1].iter().position(|b| *b == b'\n') else {
            return Err("Invalid Git include path output".into());
        };
        let path = path_from(&entry[1][newline + 1..]);
        let path = if path.is_absolute() {
            path
        } else {
            let source = origin(entry[0]).ok_or("Relative Git include has no source file")?;
            source.parent().ok_or("Git include source has no parent")?.join(path)
        };
        config_path(&path, &mut paths, 0)?;
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub fn prepare_remotes(root: &Path) -> Result<Remotes, String> {
    let target = Target::read(root)?;
    let config = run(command(root)?.args(["config", "--null", "--list"]))?;
    let origins = run(command(root)?.args(["config", "--null", "--show-origin", "--list"]))?;
    let config_files = config_paths(root, &origins)?;
    let mut metadata_dirs = Vec::new();
    let mut identities = Vec::new();
    for arg in ["--git-common-dir", "--git-dir"] {
        let bytes = run(command(root)?.args(["rev-parse", "--path-format=absolute", arg]))?;
        let path = std::fs::canonicalize(path_from(bytes.strip_suffix(b"\n").unwrap_or(&bytes)))
            .map_err(|e| format!("Could not read Git metadata: {e}"))?;
        let m = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        identities.push((m.dev(), m.ino()));
        metadata_dirs.push(path);
    }
    let mut remotes = Vec::new();
    for name in text(root, &["remote"])?.lines() {
        let fetch = output(command(root)?.args(["config", "--null", "--get-all", &format!("remote.{name}.fetch")]))?;
        if !fetch.status.success() && fetch.status.code() != Some(1) {
            return Err(crate::errors::failure(&fetch));
        }
        remotes.push(Remote {
            name: name.into(),
            fetch_urls: text(root, &["remote", "get-url", "--all", "--", name])?.lines().map(str::to_owned).collect(),
            push_urls: text(root, &["remote", "get-url", "--push", "--all", "--", name])?
                .lines()
                .map(str::to_owned)
                .collect(),
            fetch_refspecs: String::from_utf8(fetch.stdout)
                .map_err(|_| "Git returned non-UTF-8 fetch refspecs")?
                .split_terminator('\0')
                .map(str::to_owned)
                .collect(),
        });
    }
    let symbolic = output(command(root)?.args(["symbolic-ref", "--quiet", "--short", "HEAD"]))?;
    let branch = if symbolic.status.success() {
        Some(String::from_utf8(symbolic.stdout).map_err(|_| "Invalid branch name")?.trim_end().to_string())
    } else {
        None
    };
    let upstream_info = branch
        .as_ref()
        .map(|b| {
            text(
                root,
                &[
                    "for-each-ref",
                    "--format=%(upstream:short)%00%(upstream:remotename)%00%(upstream:remoteref)",
                    &format!("refs/heads/{b}"),
                ],
            )
        })
        .transpose()?
        .unwrap_or_default();
    let mut upstream_fields = upstream_info.trim_end_matches('\n').split('\0');
    let mut field = || upstream_fields.next().filter(|s| !s.is_empty()).map(str::to_owned);
    let upstream = field();
    let upstream_remote = field();
    let upstream_ref = field();
    let refs =
        run(command(root)?.args(["for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)", "refs/remotes/"]))?;
    let upstream_choices = refs
        .split(|b| *b == b'\n')
        .filter_map(|line| {
            let fields: Vec<_> = line.split(|b| *b == 0).collect();
            if fields.len() != 3 || !fields[2].is_empty() {
                return None;
            }
            std::str::from_utf8(fields[0]).ok()?.strip_prefix("refs/remotes/").map(str::to_owned)
        })
        .collect();
    Ok(Remotes {
        remotes,
        branch,
        upstream,
        upstream_remote,
        upstream_ref,
        upstream_choices,
        metadata_dirs,
        config_files,
        target,
        config,
        refs,
        identities,
    })
}

impl Remotes {
    pub fn validate(&self, root: &Path) -> Result<(), String> {
        if prepare_remotes(root).as_ref() != Ok(self) {
            return Err("The repository, branch, remote URL or upstream changed; reopen remote controls".into());
        }
        Ok(())
    }
    fn remote(&self, name: &str) -> Result<(), String> {
        if name.starts_with('-') || !self.remotes.iter().any(|r| r.name == name) {
            return Err("Choose a configured remote".into());
        }
        Ok(())
    }
    fn branch(&self) -> Result<&str, String> {
        if self.target.head.is_none() {
            return Err("Commit the current branch before this operation".into());
        }
        self.branch.as_deref().ok_or_else(|| "Switch to a branch before this operation (HEAD is detached)".into())
    }
}

fn valid_value(value: &str) -> Result<(), String> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err("Enter a nonempty name or URL without control characters or a leading dash".into());
    }
    Ok(())
}

/// Drain both pipes while retaining at most 16 KiB per stream, including noisy helpers/hooks.
fn bounded_run(cmd: &mut std::process::Command) -> Result<(), String> {
    use std::io::Read;
    use std::process::Stdio;
    fn drain(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
        let mut kept = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(kept);
            }
            let take = n.min(16384_usize.saturating_sub(kept.len()));
            kept.extend_from_slice(&chunk[..take]);
        }
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run Git: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (status, out, err) = std::thread::scope(|s| {
        let out = s.spawn(|| drain(stdout));
        let err = s.spawn(|| drain(stderr));
        (child.wait(), out.join(), err.join())
    });
    let status = status.map_err(|e| format!("Could not wait for Git: {e}"))?;
    let read = |r: std::thread::Result<std::io::Result<Vec<u8>>>| {
        r.map_err(|_| "Git output reader failed".to_string())?.map_err(|e| e.to_string())
    };
    let out = read(out)?;
    let err = read(err)?;
    if status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(if err.is_empty() { &out } else { &err });
    Err(format!("Git failed ({status}): {}", message.trim()))
}

pub fn remote_operation(root: &Path, context: &Remotes, operation: &RemoteOperation) -> Result<(), String> {
    context.validate(root)?;
    let mut cmd = command(root)?;
    // the transports remotes use; a repository's own settings cannot allow `ext::` (a shell)
    cmd.env("GIT_ALLOW_PROTOCOL", "https:http:ssh:git:file");
    match operation {
        RemoteOperation::Fetch { remote } => {
            context.remote(remote)?;
            cmd.args(["fetch", "--no-recurse-submodules", "--", remote]);
        }
        RemoteOperation::Pull => {
            context.branch()?;
            if context.upstream.is_none() {
                return Err("Set an upstream for the current branch before pulling".into());
            }
            let remote =
                context.upstream_remote.as_deref().ok_or("Choose a configured upstream remote before pulling")?;
            let reference = context.upstream_ref.as_deref().ok_or("Choose an upstream branch before pulling")?;
            if remote != "." {
                context.remote(remote)?;
            }
            // The explicit UI action never rebases or creates a merge commit, regardless of config.
            cmd.args(["pull", "--ff-only", "--no-rebase", "--no-recurse-submodules", "--", remote, reference]);
        }
        RemoteOperation::Push { remote, branch, set_upstream } => {
            context.remote(remote)?;
            let current = context.branch()?;
            valid_value(branch)?;
            run(command(root)?.args(["check-ref-format", &format!("refs/heads/{branch}")]))?;
            // Git loads remote.<name>.mirror after parsing --no-mirror; override that
            // one setting for this invocation so an explicit branch push stays non-mirroring.
            cmd.args([
                "-c",
                &format!("remote.{remote}.mirror=false"),
                "push",
                "--no-force",
                "--no-mirror",
                "--recurse-submodules=no",
            ]);
            if *set_upstream {
                cmd.arg("--set-upstream");
            }
            cmd.arg("--").arg(remote).arg(format!("refs/heads/{current}:refs/heads/{branch}"));
        }
        RemoteOperation::AddRemote { name, url } => {
            valid_value(name)?;
            valid_value(url)?;
            cmd.args(["remote", "add", "--", name, url]);
        }
        RemoteOperation::EditRemote { name, url, push_url } => {
            context.remote(name)?;
            valid_value(url)?;
            if let Some(url) = push_url {
                valid_value(url)?;
            }
            // Replace all URLs deliberately; the form displays the existing list before editing.
            run(command(root)?.args(["config", "--local", "--replace-all", &format!("remote.{name}.url"), url]))?;
            if let Some(url) = push_url {
                cmd.args(["config", "--local", "--replace-all", &format!("remote.{name}.pushurl"), url]);
            } else {
                let result = output(command(root)?.args([
                    "config",
                    "--local",
                    "--unset-all",
                    &format!("remote.{name}.pushurl"),
                ]))
                .map_err(|e| format!("Fetch URL updated, but clearing the push URL failed: {e}"))?;
                if result.status.success() || result.status.code() == Some(5) {
                    return Ok(());
                }
                return Err(format!(
                    "Fetch URL updated, but clearing the push URL failed: {}",
                    crate::errors::failure(&result)
                ));
            }
        }
        RemoteOperation::RemoveRemote { name } => {
            context.remote(name)?;
            cmd.args(["remote", "remove", "--", name]);
        }
        RemoteOperation::SetUpstream { upstream } => {
            let current = context.branch()?;
            cmd.arg("branch");
            if let Some(upstream) = upstream {
                if !context.upstream_choices.contains(upstream) {
                    return Err("Choose a fetched remote-tracking branch".into());
                }
                cmd.arg(format!("--set-upstream-to={upstream}"));
            } else {
                cmd.arg("--unset-upstream");
            }
            cmd.arg("--").arg(current);
        }
    }
    let result = bounded_run(&mut cmd);
    if matches!(operation, RemoteOperation::EditRemote { .. }) {
        result.map_err(|e| format!("Fetch URL updated, but updating the push URL failed: {e}"))
    } else {
        result
    }
}
