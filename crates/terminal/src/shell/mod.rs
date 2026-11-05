//! Select and configure the user's interactive login shell.

use portable_pty::CommandBuilder;
use std::path::Path;

pub mod integration;

pub fn login_command(cwd: &Path) -> CommandBuilder {
    configured_command(cwd, None, None)
}

pub fn configured_command(cwd: &Path, shell: Option<&str>, arguments: Option<&[String]>) -> CommandBuilder {
    let shell = shell
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("SHELL").filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "/bin/zsh".into());
    let mut command = CommandBuilder::new(shell);
    if let Some(arguments) = arguments {
        command.args(arguments);
    } else {
        command.args(["-l", "-i"]);
    }
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "Orlo");
    command
}

/// Zsh ANSI-C quoting keeps every path byte literal, including newlines and quotes.
pub(crate) fn directory_command(directory: &Path) -> Result<Vec<u8>, String> {
    use std::fmt::Write;
    use std::os::unix::ffi::OsStrExt;
    if !directory.is_absolute() || directory.as_os_str().as_bytes().contains(&0) {
        return Err("Choose an absolute terminal folder".into());
    }
    let mut command = String::from("builtin cd -- $'");
    for byte in directory.as_os_str().as_bytes() {
        write!(command, "\\x{byte:02x}").unwrap();
    }
    command.push_str("'\n");
    if command.len() > crate::configuration::MAX_INPUT {
        return Err("Terminal folder path is too long".into());
    }
    Ok(command.into_bytes())
}

/// A profile directory is optional; stale paths keep the caller's normal directory fallback.
pub fn startup_directory(preferred: Option<&Path>, fallback: &Path) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    preferred
        .filter(|path| {
            path.is_dir()
                && std::ffi::CString::new(path.as_os_str().as_bytes())
                    .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::X_OK) } == 0)
        })
        .unwrap_or(fallback)
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_login_and_explicit_argv_preserve_compatibility_without_expansion() {
        let cwd = std::env::temp_dir();
        let default = login_command(&cwd);
        assert_eq!(&default.get_argv()[1..], ["-l", "-i"]);
        let args = vec!["literal $HOME with spaces".to_string(), String::new(), "'quoted'".to_string()];
        let explicit = configured_command(&cwd, Some("/tmp/fake shell"), Some(&args));
        assert_eq!(explicit.get_argv()[0], "/tmp/fake shell");
        assert_eq!(&explicit.get_argv()[1..], args.iter().map(std::ffi::OsString::from).collect::<Vec<_>>());
        assert_eq!(explicit.get_cwd().map(std::ffi::OsString::as_os_str), Some(cwd.as_os_str()));
        assert_eq!(crate::TerminalConfiguration::default().scrollback, 10_000);
        assert_eq!(startup_directory(Some(&cwd.join("nonexistent-orlo-t05-directory")), &cwd), cwd);
    }

    #[test]
    fn inaccessible_profile_directory_uses_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let fallback = std::env::temp_dir();
        let preferred = fallback.join(format!("orlo-t05-inaccessible-{}", std::process::id()));
        std::fs::create_dir_all(&preferred).unwrap();
        std::fs::set_permissions(&preferred, std::fs::Permissions::from_mode(0o600)).unwrap();
        let selected = startup_directory(Some(&preferred), &fallback);
        std::fs::set_permissions(&preferred, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir(&preferred).unwrap();
        assert_eq!(selected, fallback, "a directory without search permission cannot be a child cwd");
    }

    #[test]
    fn zero_and_small_retention_bound_real_pty_output() {
        for limit in [0, 2] {
            let args = vec![
                "-c".into(),
                "i=0; while [ \"$i\" -lt 9 ]; do printf 'line%s\\n' \"$i\"; i=$((i+1)); done; printf line9".into(),
            ];
            let command = configured_command(&std::env::temp_dir(), Some("/bin/sh"), Some(&args));
            let mut session = crate::TerminalSession::from_command(
                command,
                crate::TerminalConfiguration { rows: 3, cols: 20, scrollback: limit },
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while session.exited().is_none() && std::time::Instant::now() < deadline {
                session.drain();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(session.exited(), Some(0));
            assert_eq!(session.error(), None);
            assert!(session.screen().contents().starts_with("line7"));
            session.scroll_lines(999);
            assert_eq!(session.screen().scrollback(), limit);
            assert!(session.screen().contents().starts_with(if limit == 0 { "line7" } else { "line5" }));
        }
    }
}
