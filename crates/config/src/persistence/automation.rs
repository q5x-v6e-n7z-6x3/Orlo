//! Explicit shell automation and local packages. No package code runs during discovery.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::ops::Range;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Process {
    pub shell: String,
    pub cwd: PathBuf,
    pub input: Vec<u8>,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub output_limit: usize,
}

impl Process {
    #[cfg(test)]
    fn with_input(mut self, text: &[u8]) -> Self {
        self.input = text.to_vec();
        self
    }
}

#[derive(Debug, Default)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: Option<i32>,
    pub problem: Option<String>,
}

impl Output {
    fn drain_pipes(&mut self, stdout: &mut impl Read, stderr: &mut impl Read, limit: usize) -> Result<(), String> {
        let capped = drain(stdout, &mut self.stdout, limit.saturating_sub(self.stderr.len()))?
            || drain(stderr, &mut self.stderr, limit.saturating_sub(self.stdout.len()))?;
        if capped {
            return Err(format!("Command exceeded its {} byte output limit", limit));
        }
        Ok(())
    }

    pub fn success(&self) -> bool {
        self.problem.is_none() && self.code == Some(0)
    }

    pub fn summary(&self) -> String {
        self.problem.clone().unwrap_or_else(|| match self.code {
            Some(0) => "Finished successfully".into(),
            Some(n) => format!("Exited with status {n}"),
            None => "Terminated by signal".into(),
        })
    }

    pub fn text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.stdout).into_owned();
        if !self.stderr.is_empty() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str("[stderr]\n");
            text.push_str(&String::from_utf8_lossy(&self.stderr));
        }
        text
    }
}

struct ChildGroup(Child);

impl Drop for ChildGroup {
    fn drop(&mut self) {
        // The shell may have exited while a grandchild still owns its pipes.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}

fn nonblocking(fd: &impl AsRawFd) -> Result<(), String> {
    let fd = fd.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

fn drain(pipe: &mut impl Read, target: &mut Vec<u8>, remaining: usize) -> Result<bool, String> {
    let mut buf = [0; 8192];
    // Bounded per tick so a busy stdout cannot starve timeout/cancel/stderr.
    for _ in 0..32 {
        match pipe.read(&mut buf) {
            Ok(0) => return Ok(false),
            Ok(n) => {
                let keep = n.min(remaining.saturating_sub(target.len()));
                target.extend_from_slice(&buf[..keep]);
                if keep < n {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
}

/// Only for a command the user ran. Pipes never block the UI or cancellation.
pub fn run(process: Process, cancel: &AtomicBool) -> Output {
    run_since(process, cancel, Instant::now())
}

/// `argv` run directly, without a shell (`process.shell` is unused), under the same limits.
pub fn run_program(argv: &[String], process: Process, cancel: &AtomicBool) -> Output {
    let Some((program, arguments)) = argv.split_first() else {
        return Output { problem: Some("No program to run".into()), ..Output::default() };
    };
    let mut command = Command::new(program);
    command.args(arguments);
    run_command(command, process, cancel, Instant::now())
}

fn run_since(process: Process, cancel: &AtomicBool, started: Instant) -> Output {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(&process.shell);
    run_command(command, process, cancel, started)
}

fn run_command(mut command: Command, process: Process, cancel: &AtomicBool, started: Instant) -> Output {
    let mut output = Output::default();
    if cancel.load(Ordering::Acquire) {
        output.problem = Some("Cancelled".into());
        return output;
    }
    let result = (|| -> Result<(), String> {
        command
            .current_dir(&process.cwd)
            .envs(process.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        // Setup and earlier selections consume the same budget; never spawn after expiry.
        if started.elapsed() >= process.timeout {
            return Err("Command timed out".into());
        }
        let child = command.spawn().map_err(|e| e.to_string())?;
        let mut child = ChildGroup(child);
        let mut input = child.0.stdin.take();
        let mut stdout = child.0.stdout.take().ok_or("No stdout pipe")?;
        let mut stderr = child.0.stderr.take().ok_or("No stderr pipe")?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        if let Some(input) = &input {
            nonblocking(input)?;
        }
        let mut written = 0;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Cancelled".into());
            }
            if started.elapsed() >= process.timeout {
                return Err("Command timed out".into());
            }
            output.drain_pipes(&mut stdout, &mut stderr, process.output_limit)?;
            if let Some(pipe) = &mut input {
                if written == process.input.len() {
                    input = None;
                } else {
                    match pipe.write(&process.input[written..process.input.len().min(written + 65536)]) {
                        Ok(n) => written += n,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => input = None,
                        Err(e) => return Err(e.to_string()),
                    }
                }
            }
            if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                output.code = status.code();
                // Stop descendants before a final drain; an exited shell cannot leave a task behind.
                unsafe {
                    libc::kill(-(child.0.id() as i32), libc::SIGKILL);
                }
                loop {
                    let before = output.stdout.len() + output.stderr.len();
                    output.drain_pipes(&mut stdout, &mut stderr, process.output_limit)?;
                    if before == output.stdout.len() + output.stderr.len() {
                        break;
                    }
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    output.problem = result.err();
    output
}

/// A serial shell-filter batch shares one deadline, output budget and cancellation flag.
/// Inputs are data on STDIN; only the explicitly supplied shell is executable text.
pub fn run_selections(
    process: Process,
    inputs: Vec<(Range<usize>, String)>,
    predicate: bool,
    cancel: &AtomicBool,
) -> Vec<Output> {
    let started = Instant::now();
    let mut remaining = process.output_limit;
    let mut outputs = Vec::with_capacity(inputs.len());
    for (index, (range, text)) in inputs.into_iter().enumerate() {
        let mut child = process.clone();
        child.input = text.into_bytes();
        child.output_limit = remaining;
        child.env.retain(|(name, _)| !matches!(name.as_str(), "ORLO_SELECTION_START" | "ORLO_SELECTION_END"));
        child.env.push(("ORLO_SELECTION_START".into(), range.start.to_string()));
        child.env.push(("ORLO_SELECTION_END".into(), range.end.to_string()));
        let mut output = run_since(child, cancel, started);
        let accepted = output.problem.is_none() && (output.code == Some(0) || predicate && output.code == Some(1));
        if !accepted {
            output.problem = Some(format!("Selection {}: {}", index + 1, output.summary()));
        }
        remaining = remaining.saturating_sub(output.stdout.len() + output.stderr.len());
        outputs.push(output);
        if !accepted {
            break;
        }
    }
    outputs
}

/// GCC/rustc-style `path:line:column: message` (and `path:line: message`).
pub fn diagnostic(line: &str) -> Option<(&str, usize, usize, &str)> {
    let line = line.trim_start();
    let line = line
        .strip_prefix("--> ")
        .or_else(|| line.strip_prefix("::: "))
        .or_else(|| line.strip_prefix("In file included from "))
        .or_else(|| line.strip_prefix("from "))
        .unwrap_or(line);
    for (at, _) in line.match_indices(':') {
        let tail = &line[at + 1..];
        let (number, rest) = tail.split_once(':')?;
        let Ok(row) = number.parse::<usize>() else {
            continue;
        };
        let file = &line[..at];
        if row == 0 || at == 0 || file.contains(": ") || file.starts_with(['[', '(']) {
            return None;
        }
        let (col, message) = (rest.split_once(':'))
            .and_then(|(col, message)| Some((col.parse().ok()?, message)))
            .or_else(|| Some((rest.trim().parse().ok()?, "diagnostic")))
            .unwrap_or((1, rest));
        if col == 0 || col > 100_000 {
            return None;
        }
        return Some((file, row, col, message.trim_start()));
    }
    None
}

pub fn result_output(text: &str, cwd: &Path) -> (String, Vec<Range<usize>>) {
    let (mut out, mut ranges, mut chars) = (String::new(), Vec::new(), 0);
    for line in text.lines() {
        let line = match diagnostic(line) {
            Some((file, row, col, message)) => {
                let path = cwd.join(file);
                // a path header and a numbered line, as in Find Results, so F4 steps through them
                let header = format!("\n{}:\n{row:>5}: {}", path.to_string_lossy(), " ".repeat(col - 1));
                if out.len() + header.len() + message.len() > OUTPUT_LIMIT * 4 {
                    out.push_str("\n[Formatted diagnostics truncated]\n");
                    break;
                }
                out.push_str(&header);
                chars += header.chars().count();
                let message = Some(message).filter(|m| !m.is_empty()).unwrap_or("diagnostic");
                ranges.push(chars..chars + message.chars().count());
                message
            }
            None => line,
        };
        out.push_str(line);
        out.push('\n');
        chars += line.chars().count() + 1;
    }
    (out, ranges)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageCommand {
    pub name: String,
    pub shell: String,
    #[serde(default)]
    pub filter: bool,
    #[serde(default)]
    pub api: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    pub name: String,
    pub shell: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub commands: Vec<PackageCommand>,
    #[serde(default)]
    pub tasks: Vec<TaskDefinition>,
}

pub struct Package {
    pub manifest: Manifest,
    pub path: PathBuf,
    pub enabled: bool,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.len() <= 80
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

fn validate_command(name: &str, shell: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
        return Err("Command name must contain 1–120 visible characters".into());
    }
    if shell.trim().is_empty() || shell.contains('\0') {
        return Err(format!("{name}: shell command is empty or contains NUL"));
    }
    Ok(())
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let manifest: Manifest = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if !valid_name(&manifest.name) {
        return Err("Package name must be a single safe name (letters, numbers, dots, underscores, hyphens)".into());
    }
    let mut names = std::collections::HashSet::new();
    for command in &manifest.commands {
        validate_command(&command.name, &command.shell)?;
        if command.api && command.filter {
            return Err("A package command cannot be both api and filter".into());
        }
        if !names.insert(&command.name) {
            return Err(format!("Duplicate command: {}", command.name));
        }
    }
    validate_tasks(&manifest.tasks)?;
    Ok(manifest)
}

fn validate_tasks(tasks: &[TaskDefinition]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for task in tasks {
        validate_command(&task.name, &task.shell)?;
        if !names.insert(&task.name) {
            return Err(format!("Duplicate task: {}", task.name));
        }
        if task.timeout_secs.is_some_and(|t| t == 0 || t > 3600) {
            return Err(format!("{}: timeout_secs must be 1–3600", task.name));
        }
    }
    Ok(())
}

pub fn read_tasks(path: &Path) -> Result<Vec<TaskDefinition>, String> {
    let file = match File::options().read(true).custom_flags(libc::O_NONBLOCK).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let text = String::from_utf8(read_bounded(file, 1024 * 1024)?).map_err(|e| e.to_string())?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Tasks {
        tasks: Vec<TaskDefinition>,
    }
    let tasks: Tasks = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    validate_tasks(&tasks.tasks)?;
    Ok(tasks.tasks)
}

pub fn packages_list(directory: &Path) -> Result<Vec<Package>, String> {
    packages_with_errors(directory).map(|(packages, _)| packages)
}

/// The packages that load, and one error for each enabled package that does not.
pub fn packages_with_errors(directory: &Path) -> Result<(Vec<Package>, Vec<String>), String> {
    let entries = match fs::read_dir(directory) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(e.to_string()),
    };
    let (mut packages, mut errors) = (Vec::new(), Vec::new());
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                errors.push(e.to_string());
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_name(&name) {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => {}
            Ok(_) => continue,
            Err(e) => {
                errors.push(format!("{}: {e}", path.display()));
                continue;
            }
        }
        let enabled = !path.join(".disabled").exists();
        let manifest = read_regular(&path.join("package.json"), 1024 * 1024)
            .and_then(|text| parse_manifest(&text))
            .and_then(|manifest| match manifest.name == name {
                true => Ok(manifest),
                false => Err("manifest name differs from folder name".into()),
            });
        match manifest {
            Ok(manifest) => packages.push(Package { enabled, manifest, path }),
            Err(error) if enabled => errors.push(format!("{}: {error}", path.display())),
            Err(_) => {}
        }
    }
    packages.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    Ok((packages, errors))
}

fn read_regular(path: &Path, limit: usize) -> Result<String, String> {
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(format!("{}: regular files only", path.display()));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("{}: file exceeds size limit", path.display()));
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

static NEXT_INSTALL: AtomicU64 = AtomicU64::new(0);

/// Stage a bounded local package, reject links/special files, then publish it with one rename.
pub fn install_package(source: &Path, directory: &Path) -> Result<String, String> {
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let source_dir = File::options()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&source)
        .map_err(|e| e.to_string())?;
    let manifest_file = open_package_entry(&source_dir, c"package.json")?;
    let manifest =
        parse_manifest(&String::from_utf8(read_bounded(manifest_file, 1024 * 1024)?).map_err(|e| e.to_string())?)?;
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    if fs::symlink_metadata(directory).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err("Packages directory cannot be a symlink".into());
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if directory.starts_with(&source) {
        return Err("The package source cannot contain the Packages directory".into());
    }
    let destination = directory.join(&manifest.name);
    if destination.symlink_metadata().is_ok() {
        return Err(format!("{} is already installed; remove it first", manifest.name));
    }
    let stage =
        directory.join(format!(".install-{}-{}", std::process::id(), NEXT_INSTALL.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = copy_tree(&source_dir, &stage, &mut (0usize, 0usize), 0).and_then(|_| {
        let copied = parse_manifest(&read_regular(&stage.join("package.json"), 1024 * 1024)?)?;
        if copied.name != manifest.name {
            return Err("Package manifest changed during installation".into());
        }
        if destination.symlink_metadata().is_ok() {
            return Err("Package was installed concurrently".into());
        }
        fs::rename(&stage, &destination).map_err(|e| e.to_string())
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result.map(|_| manifest.name)
}

fn copy_tree(source: &File, destination: &Path, budget: &mut (usize, usize), depth: usize) -> Result<(), String> {
    if depth > 32 {
        return Err("Package directory nesting exceeds 32".into());
    }
    // SAFETY: F_DUPFD_CLOEXEC only makes a new descriptor of the open source directory, one a
    // child process (a language server, Git) does not inherit as it would a plain dup's.
    let fd = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: fcntl returned a new descriptor owned by this directory stream.
    let mut directory =
        orlo_utils::directory::Directory::new(unsafe { File::from_raw_fd(fd) }).map_err(|e| e.to_string())?;
    while let Some(name) = directory.next_name().map_err(|e| e.to_string())? {
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        budget.0 += 1;
        if budget.0 > 10_000 {
            return Err("Package exceeds 10,000 entries".into());
        }
        // Resolve each child against the held directory descriptor, with no symlink traversal.
        let file = open_package_entry(source, name)?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let target = destination.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|e| e.to_string())?;
            copy_tree(&file, &target, budget, depth + 1)?;
        } else if metadata.is_file() {
            let bytes = read_bounded(file, (64 * 1024 * 1024usize).saturating_sub(budget.1))?;
            budget.1 += bytes.len();
            let mut file = File::options().write(true).create_new(true).open(&target).map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.set_permissions(fs::Permissions::from_mode(metadata.permissions().mode() & 0o777))
                .map_err(|e| e.to_string())?;
        } else {
            return Err(format!("{}: regular files and directories only", target.display()));
        }
    }
    Ok(())
}

fn open_package_entry(directory: &File, name: &std::ffi::CStr) -> Result<File, String> {
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        Err(format!("{}: {} (package links are not allowed)", name.to_string_lossy(), std::io::Error::last_os_error()))
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn read_bounded(file: File, limit: usize) -> Result<Vec<u8>, String> {
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Packages accept regular files only".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Package exceeds its file size limit".into());
    }
    Ok(bytes)
}

fn package_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err("Invalid package name".into());
    }
    let path = directory.join(name);
    if !fs::symlink_metadata(&path).map_err(|e| e.to_string())?.is_dir() {
        return Err("Package is not a regular directory".into());
    }
    Ok(path)
}

pub fn set_enabled(directory: &Path, name: &str, enabled: bool) -> Result<(), String> {
    let marker = package_path(directory, name)?.join(".disabled");
    // Already in the state asked for is success.
    let (result, already) = if enabled {
        (fs::remove_file(marker), std::io::ErrorKind::NotFound)
    } else {
        let created = File::options().write(true).create_new(true).open(marker);
        (created.map(drop), std::io::ErrorKind::AlreadyExists)
    };
    match result {
        Err(e) if e.kind() != already => Err(e.to_string()),
        _ => Ok(()),
    }
}

pub fn remove_package(directory: &Path, name: &str) -> Result<(), String> {
    fs::remove_dir_all(package_path(directory, name)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uncancelled(process: Process) -> Output {
        run(process, &AtomicBool::new(false))
    }

    fn command(shell: &str) -> Process {
        Process {
            shell: shell.into(),
            cwd: std::env::temp_dir(),
            input: Vec::new(),
            env: Vec::new(),
            timeout: Duration::from_secs(2),
            output_limit: 4096,
        }
    }

    #[test]
    fn captures_both_streams_and_preserves_failed_status() {
        let result = uncancelled(command("cat; printf problem >&2; exit 7").with_input(b"hello"));
        assert_eq!(result.stdout, b"hello");
        assert_eq!(result.stderr, b"problem");
        assert!(!result.success());
        assert_eq!(result.code, Some(7));
    }

    #[test]
    fn output_timeout_and_cancel_are_bounded() {
        let result = uncancelled(command("while :; do printf 1234567890; done"));
        assert!(result.problem.unwrap().contains("output limit"));
        assert!(result.stdout.len() + result.stderr.len() <= 4096);
        let mut p = command("sleep 10 & wait");
        p.timeout = Duration::from_millis(60);
        let started = Instant::now();
        assert!(uncancelled(p).problem.unwrap().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(run(command("sleep 10"), &AtomicBool::new(true)).problem.unwrap().contains("Cancelled"));
    }

    #[test]
    fn a_large_stdin_and_stdout_cannot_deadlock_each_other() {
        let input = vec![b'x'; 256 * 1024];
        let mut process = command("cat").with_input(&input);
        process.output_limit = 512 * 1024;
        let result = uncancelled(process);
        assert!(result.success(), "{}", result.summary());
        assert_eq!(result.stdout, input);
    }

    #[test]
    fn cancellation_stops_an_already_running_group() {
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let thread_flag = flag.clone();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            thread_flag.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let result = run(command("sleep 10 & wait"), &flag);
        cancel.join().unwrap();
        assert!(result.summary().contains("Cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_child_holding_the_pipes_does_not_hang_completion() {
        let started = Instant::now();
        assert!(uncancelled(command("sleep 10 & printf done")).success());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn programs_run_without_a_shell_and_share_the_shell_runners_limits() {
        let argv = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let result =
            run_program(&argv(&["/bin/echo", "$(printf INJECTED)", "*"]), command(""), &AtomicBool::new(false));
        assert!(result.success(), "{}", result.summary());
        assert_eq!(result.stdout, b"$(printf INJECTED) *\n", "arguments are literal");
        let result = run_program(
            &argv(&["/bin/sh", "-c", "tr a-z A-Z; printf oops >&2; exit 3"]),
            command("").with_input(b"text"),
            &AtomicBool::new(false),
        );
        assert_eq!(
            (result.stdout.as_slice(), result.stderr.as_slice(), result.code),
            (&b"TEXT"[..], &b"oops"[..], Some(3))
        );
        let mut slow = command("");
        slow.timeout = Duration::from_millis(60);
        let started = Instant::now();
        let result = run_program(&argv(&["/bin/sh", "-c", "sleep 10 & wait"]), slow, &AtomicBool::new(false));
        assert!(result.problem.unwrap().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2), "the process group is killed");
        let result = run_program(&argv(&["/usr/bin/yes"]), command(""), &AtomicBool::new(false));
        assert!(result.problem.unwrap().contains("output limit"));
        let result = run_program(&argv(&["orlo-no-such-formatter"]), command(""), &AtomicBool::new(false));
        assert!(result.problem.is_some() && !result.success());
        assert!(run_program(&[], command(""), &AtomicBool::new(false)).problem.is_some());
    }

    #[test]
    fn n38_batch_preserves_order_unicode_offsets_and_literal_stdin() {
        let input = "$(printf INJECTED); `printf BAD` 'é界'\n";
        let outputs = run_selections(
            command("printf '%s:%s:' \"$ORLO_SELECTION_START\" \"$ORLO_SELECTION_END\"; cat"),
            vec![(7..9, "界é".into()), (20..40, input.into())],
            false,
            &AtomicBool::new(false),
        );
        assert_eq!(outputs.len(), 2);
        assert!(outputs.iter().all(Output::success));
        assert_eq!(outputs[0].stdout, "7:9:界é".as_bytes());
        assert_eq!(outputs[1].stdout, format!("20:40:{input}").as_bytes());
    }

    #[test]
    fn n38_batch_failure_predicate_and_aggregate_output_are_bounded() {
        let run_batch = |shell, predicate| {
            run_selections(
                command(shell),
                vec![(0..1, "a\n".into()), (2..3, "b\n".into()), (4..5, "c\n".into())],
                predicate,
                &AtomicBool::new(false),
            )
        };
        let outputs =
            run_batch("read x; if [ \"$x\" = b ]; then printf problem >&2; exit 7; fi; printf changed", false);
        assert_eq!(outputs.len(), 2, "failure must stop later inputs");
        assert!(outputs[0].success());
        assert!(outputs[1].summary().contains("Selection 2: Exited with status 7"));
        assert_eq!(outputs[1].stderr, b"problem");
        let outputs = run_batch("read x; printf '\\377'; [ \"$x\" != b ]", true);
        assert_eq!(outputs.iter().map(|output| output.code).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(0)]);
        assert!(outputs.iter().all(|output| output.problem.is_none()));
        assert!(String::from_utf8(outputs[0].stdout.clone()).is_err(), "predicate stdout need not be UTF-8");
        let outputs = run_batch("kill -TERM $$", true);
        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].summary().contains("Terminated by signal"));
        let mut process = command("printf abc");
        process.output_limit = 5;
        let outputs =
            run_selections(process, vec![(0..1, "a".into()), (2..3, "b".into())], false, &AtomicBool::new(false));
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs.iter().map(|o| o.stdout.len() + o.stderr.len()).sum::<usize>(), 5);
        assert!(outputs[1].summary().contains("output limit"));
        let mut process = command("sleep 0.6; printf ok");
        process.timeout = Duration::from_secs(1);
        let outputs =
            run_selections(process, vec![(0..1, "a".into()), (2..3, "b".into())], false, &AtomicBool::new(false));
        assert!(outputs[0].success());
        assert!(outputs[1].summary().contains("timed out"), "one deadline covers the whole batch");
    }

    #[test]
    fn n38_expired_batch_refuses_before_attempting_spawn() {
        let missing = std::env::temp_dir().join(format!(
            "orlo-n38-expired-no-cwd-{}-{}",
            std::process::id(),
            NEXT_INSTALL.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(!missing.exists());
        for predicate in [false, true] {
            let mut process = command("true");
            process.cwd = missing.clone();
            process.timeout = Duration::ZERO;
            let outputs = run_selections(
                process.clone(),
                vec![(0..1, "a".into()), (2..3, "b".into())],
                predicate,
                &AtomicBool::new(false),
            );
            assert_eq!(outputs.len(), 1, "an expired batch skips all remaining inputs");
            assert_eq!(outputs[0].problem.as_deref(), Some("Selection 1: Command timed out"));
            assert!(outputs[0].stdout.is_empty() && outputs[0].stderr.is_empty());
            assert_eq!(uncancelled(process).problem.as_deref(), Some("Command timed out"));
        }
    }

    #[test]
    fn n38_early_exit_filters_and_predicates_accept_large_unconsumed_stdin() {
        for (shell, predicate, expected) in
            [("true", true, 0), ("false", true, 1), ("grep -q match", true, 0), ("head -c 1", false, 0)]
        {
            let input = format!("match\n{}", "x".repeat(256 * 1024));
            let outputs =
                run_selections(command(shell), vec![(0..input.len(), input)], predicate, &AtomicBool::new(false));
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0].code, Some(expected), "{shell}: {}", outputs[0].summary());
            assert!(outputs[0].problem.is_none(), "{shell}: {}", outputs[0].summary());
            if !predicate {
                assert_eq!(outputs[0].stdout, b"m");
            }
        }
    }

    #[test]
    fn n38_batch_cancel_reaps_the_running_shell_and_skips_later_inputs() {
        let directory = std::env::temp_dir().join(format!(
            "orlo-n38-{}-{}",
            std::process::id(),
            NEXT_INSTALL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let pid_file = directory.join("pid");
        let later = directory.join("later");
        let mut process = command("read x; if [ \"$x\" = slow ]; then printf '%s' \"$$\" > \"$ORLO_TEST_PID\"; sleep 10 & wait; else : > \"$ORLO_TEST_LATER\"; fi");
        process.env = vec![
            ("ORLO_TEST_PID".into(), pid_file.to_string_lossy().into_owned()),
            ("ORLO_TEST_LATER".into(), later.to_string_lossy().into_owned()),
        ];
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let child_flag = flag.clone();
        let worker = std::thread::spawn(move || {
            run_selections(process, vec![(0..4, "slow\n".into()), (5..10, "later\n".into())], false, &child_flag)
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let pid = loop {
            if let Ok(text) = fs::read_to_string(&pid_file) {
                if let Ok(pid) = text.parse::<i32>() {
                    break pid;
                }
            }
            assert!(Instant::now() < deadline, "real command did not become ready");
            std::thread::sleep(Duration::from_millis(2));
        };
        flag.store(true, Ordering::Release);
        let outputs = worker.join().unwrap();
        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].summary().contains("Cancelled"));
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "cancelled shell must be reaped before return");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        assert!(!later.exists(), "cancelled batches cannot launch later selections");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn diagnostics_reject_make_targets_and_follow_include_chains() {
        assert_eq!(diagnostic("make: *** [Makefile:12: all] Error 1"), None);
        assert_eq!(diagnostic("(x.c:3:4) note"), None);
        assert_eq!(diagnostic("In file included from x.h:3:"), Some(("x.h", 3, 1, "")));
        assert_eq!(diagnostic("                 from src/a.c:1:"), Some(("src/a.c", 1, 1, "")));
        assert_eq!(diagnostic("src/a file.rs:12:3: failure"), Some(("src/a file.rs", 12, 3, "failure")));
        assert_eq!(diagnostic("  --> src/main.rs:4:9"), Some(("src/main.rs", 4, 9, "diagnostic")));
    }

    #[test]
    fn manifest_rejects_traversal_duplicate_names_and_empty_commands() {
        for name in ["../x", ".", "..", "/tmp/x", "a/b", "a\\b", ""] {
            assert!(parse_manifest(&format!(r#"{{"name":{}}}"#, serde_json::to_string(name).unwrap())).is_err());
        }
        assert!(parse_manifest(
            r#"{"name":"Useful.Package","commands":[{"name":"Uppercase","shell":"tr a-z A-Z","filter":true}]}"#
        )
        .is_ok());
        assert!(parse_manifest(r#"{"name":"x","commands":[{"name":"a","shell":""}]}"#).is_err());
        assert!(parse_manifest(r#"{"name":"x","commands":[{"name":"a","shell":"pwd"},{"name":"a","shell":"pwd"}]}"#)
            .is_err());
    }

    #[test]
    fn install_is_complete_or_absent_and_never_follows_links() {
        let base = std::env::temp_dir().join(format!(
            "orlo-package-test-{}-{}",
            std::process::id(),
            NEXT_INSTALL.fetch_add(1, Ordering::Relaxed)
        ));
        let source = base.join("source");
        let packages = base.join("Packages");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("package.json"), r#"{"name":"safe"}"#).unwrap();
        fs::write(source.join("tool.sh"), "printf safe").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", source.join("escape")).unwrap();
        assert!(install_package(&source, &packages).is_err());
        assert!(!packages.join("safe").exists());
        fs::remove_file(source.join("escape")).unwrap();
        assert_eq!(install_package(&source, &packages).unwrap(), "safe");
        assert_eq!(fs::read_to_string(packages.join("safe/tool.sh")).unwrap(), "printf safe");
        assert!(install_package(&source, &packages).is_err());
        set_enabled(&packages, "safe", false).unwrap();
        assert!(!packages_list(&packages).unwrap()[0].enabled);
        fs::write(packages.join("safe/package.json"), "not json").unwrap();
        assert!(
            packages_list(&packages).unwrap().is_empty(),
            "disabled package errors cannot prevent other customizations loading"
        );
        remove_package(&packages, "../source").unwrap_err();
        remove_package(&packages, "safe").unwrap();
        assert!(source.exists());
        fs::remove_dir_all(base).unwrap();
    }
}
