//! Launch-local zsh hooks; user startup files are read, never rewritten.

use portable_pty::CommandBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

pub struct Integration(PathBuf);

impl Integration {
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
    pub(crate) fn write_folder_command(&self, command: &[u8]) -> io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(self.0.join("sync-folder"))?
            .write_all(command)
    }

    pub fn install(command: &mut CommandBuilder) -> io::Result<Option<Self>> {
        let args = command.get_argv();
        if args.first().and_then(|s| Path::new(s).file_name()).is_none_or(|s| s != "zsh")
            || !args.iter().any(|a| matches!(a.to_str(), Some("-i" | "-li" | "-il")))
            // Other flags can bypass startup files, preventing restoration of ZDOTDIR.
            || args.iter().skip(1).any(|a| !matches!(a.to_str(), Some("-i" | "-l" | "-li" | "-il")))
        {
            return Ok(None);
        }
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "orlo-shell-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        let integration = Self(path);
        std::fs::write(
            integration.0.join(".zshenv"),
            r#"
if [[ $ORLO_USER_ZDOTDIR_PRESENT == 1 ]]; then ZDOTDIR=$ORLO_USER_ZDOTDIR; else unset ZDOTDIR; fi
[[ -r "${ZDOTDIR:-$HOME}/.zshenv" ]] && source "${ZDOTDIR:-$HOME}/.zshenv"
typeset -g __orlo_zdotdir_present=${+ZDOTDIR}
typeset -g __orlo_user_zdotdir=${ZDOTDIR:-$HOME}
if [[ -o rcs && -o interactive && ${(t)ZDOTDIR} != *readonly* ]]; then ZDOTDIR=$ORLO_INTEGRATION_DIR; fi
"#,
        )?;
        std::fs::write(
            integration.0.join(".zprofile"),
            r#"
if [[ $__orlo_zdotdir_present == 1 ]]; then ZDOTDIR=$__orlo_user_zdotdir; else unset ZDOTDIR; fi
[[ -r "$__orlo_user_zdotdir/.zprofile" ]] && source "$__orlo_user_zdotdir/.zprofile"
__orlo_zdotdir_present=${+ZDOTDIR}
__orlo_user_zdotdir=${ZDOTDIR:-$HOME}
if [[ -o rcs && -o interactive && ${(t)ZDOTDIR} != *readonly* ]]; then ZDOTDIR=$ORLO_INTEGRATION_DIR; fi
"#,
        )?;
        std::fs::write(
            integration.0.join(".zshrc"),
            format!(
                r#"
if [[ $__orlo_zdotdir_present == 1 ]]; then ZDOTDIR=$__orlo_user_zdotdir; else unset ZDOTDIR; fi
if [[ "${{HISTFILE-}}" == "$ORLO_INTEGRATION_DIR/.zsh_history" && ${{(t)HISTFILE}} != *readonly* ]]; then
    HISTFILE="$__orlo_user_zdotdir/.zsh_history"
fi
[[ -r "$__orlo_user_zdotdir/.zshrc" ]] && source "$__orlo_user_zdotdir/.zshrc"
{}
"#,
                include_str!("zsh-integration.zsh")
            ),
        )?;
        let present = command.get_env("ZDOTDIR").is_some();
        let user = command
            .get_env("ZDOTDIR")
            .map(std::ffi::OsString::from)
            .or_else(|| command.get_env("HOME").map(std::ffi::OsString::from))
            .or_else(|| std::env::var_os("HOME"))
            .unwrap_or_else(|| "/".into());
        command.env("ORLO_USER_ZDOTDIR_PRESENT", if present { "1" } else { "0" });
        command.env("ORLO_USER_ZDOTDIR", user);
        command.env("ORLO_INTEGRATION_DIR", &integration.0);
        command.env("ZDOTDIR", &integration.0);
        Ok(Some(integration))
    }
}

impl Drop for Integration {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!("Orlo: could not remove shell integration directory {}: {error}", self.0.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TerminalConfiguration, TerminalSession};
    use std::time::{Duration, Instant};

    fn wait_for(session: &mut TerminalSession, ready: impl Fn(&TerminalSession) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready(session) {
            assert!(Instant::now() < deadline, "{:?}: {:?}", session.error(), session.screen().contents());
            session.drain();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_cleanup(path: &Path, pid: u32) {
        assert!(pid > 0);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let reaped = unsafe { libc::kill(pid as libc::pid_t, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if reaped && !path.exists() {
                return;
            }
            assert!(Instant::now() < deadline, "shell {pid} reaped={reaped}, private directory {path:?} still exists");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn opt_in_zsh_preserves_startup_order_zdotdir_hooks_status_interrupt_and_owned_cleanup() {
        let home = std::env::temp_dir().join(format!(
            "orlo-zsh-order-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
        let home = home.canonicalize().unwrap();
        for name in ["profile", "interactive", "login"] {
            std::fs::create_dir(home.join(name)).unwrap();
        }
        let files = [
            (".zshenv", "typeset -g ORDER=env\nZDOTDIR=$HOME/profile\n"),
            ("profile/.zprofile", "ORDER+=,profile\nZDOTDIR=$HOME/interactive\n"),
            ("interactive/.zshrc", "ORDER+=,rc\nZDOTDIR=$HOME/login\nPROMPT='fixture> '\nfunction precmd() { print -r -- USER-PRECMD-FUNCTION; }\nfunction fixture_precmd() { print -r -- USER-PRECMD-HOOK; return 0; }\nfunction fixture_preexec() { print -r -- USER-PREEXEC-HOOK; }\nprecmd_functions=(fixture_precmd)\npreexec_functions=(fixture_preexec)\n"),
            ("login/.zlogin", "ORDER+=,login\n"),
        ];
        for (path, text) in files {
            std::fs::write(home.join(path), text).unwrap();
        }
        let mut command = CommandBuilder::new("/bin/zsh");
        command.args(["-l", "-i"]);
        command.env("HOME", &home);
        command.env("ZDOTDIR", &home);
        command.cwd(&home);
        let mut session = TerminalSession::from_command_with_integration(
            command,
            TerminalConfiguration { cols: 1024, scrollback: 20, ..Default::default() },
            true,
        );
        wait_for(&mut session, |s| s.screen().contents().contains("fixture>"));
        let integration = session.integration_path().unwrap().to_path_buf();
        session.write(b"printf 'STARTUP:%s:%s:%s\\n' \"$ORDER\" \"$ZDOTDIR\" \"$options[login]\"; false\r".to_vec());
        wait_for(&mut session, |s| s.commands().records().len() == 1);
        let failed = &session.commands().records()[0];
        assert_eq!(failed.status, Some(1), "user precmd hooks must preserve the command's status");
        assert!(
            failed.output.contains(&format!("STARTUP:env,profile,rc,login:{}/login:on", home.display())),
            "{}",
            failed.output
        );
        assert!(!failed.output.contains("fixture>"));
        assert!(session.screen().contents().contains("USER-PRECMD-FUNCTION"));
        assert!(session.screen().contents().contains("USER-PRECMD-HOOK"));
        assert!(session.screen().contents().contains("USER-PREEXEC-HOOK"));
        session.write(b"printf 'SUCCESS\\n'\r".to_vec());
        wait_for(&mut session, |s| s.commands().records().len() == 2);
        assert_eq!(session.commands().records()[1].status, Some(0));
        assert!(session.commands().records()[1].output.contains("SUCCESS"));
        session.write(b"printf 'INTERRUPT-READY\\n'; sleep 30\r".to_vec());
        wait_for(&mut session, |s| s.commands().executing() && s.screen().contents().contains("INTERRUPT-READY"));
        session.write(vec![3]);
        wait_for(&mut session, |s| s.commands().records().len() == 3);
        assert_eq!(session.commands().records()[2].status, Some(130));
        assert!(session.commands().records()[2].output.contains("INTERRUPT-READY"));
        for (path, text) in files {
            assert_eq!(std::fs::read_to_string(home.join(path)).unwrap(), text, "startup file changed: {path}");
        }
        let pid = session.pty().unwrap().process_id();
        session.close();
        wait_for_cleanup(&integration, pid);
        assert!(!integration.exists());
        assert!(session.pty().is_none());
        assert_eq!(session.commands().records().len(), 3, "close retains review output");
        session.close();
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn ordinary_zsh_launch_does_not_inject_or_create_private_startup_files() {
        let home = std::env::temp_dir().join(format!("orlo-zsh-default-off-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zshrc"), "PROMPT='plain> '\n").unwrap();
        let mut command = CommandBuilder::new("/bin/zsh");
        command.args(["-l", "-i"]);
        command.env("HOME", &home);
        command.env("ZDOTDIR", &home);
        command.cwd(&home);
        let mut session = TerminalSession::from_command(command, TerminalConfiguration::default());
        assert!(session.integration_path().is_none(), "ordinary launches require explicit opt-in");
        session.close();
        assert_eq!(std::fs::read_to_string(home.join(".zshrc")).unwrap(), "PROMPT='plain> '\n");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn unsupported_launches_remain_plain_and_supported_flags_keep_literal_argv() {
        for (shell, arguments) in [
            ("/bin/sh", vec!["-l", "-i"]),
            ("/bin/zsh", vec![]),
            ("/bin/zsh", vec!["-c", "true"]),
            ("/bin/zsh", vec!["-f", "-i"]),
            ("/bin/zsh", vec!["-i", "literal $HOME", ""]),
        ] {
            let mut command = CommandBuilder::new(shell);
            command.args(arguments);
            command.env("ZDOTDIR", "/fixture/user");
            let argv = command.get_argv().to_vec();
            assert!(Integration::install(&mut command).unwrap().is_none());
            assert_eq!(command.get_argv(), &argv);
            assert_eq!(command.get_env("ZDOTDIR"), Some(std::ffi::OsStr::new("/fixture/user")));
            assert!(command.get_env("ORLO_INTEGRATION_DIR").is_none());
        }
        for arguments in [vec!["-i"], vec!["-li"], vec!["-il"], vec!["-l", "-i"]] {
            let mut command = CommandBuilder::new("/bin/zsh");
            command.args(arguments);
            let argv = command.get_argv().to_vec();
            let installed = Integration::install(&mut command).unwrap().unwrap();
            assert_eq!(command.get_argv(), &argv);
            let path = installed.path().to_path_buf();
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
            drop(installed);
            assert!(!path.exists());
        }
    }

    #[test]
    fn failed_supported_executable_keeps_startup_error_and_releases_owned_integration() {
        let mut command = CommandBuilder::new("/nonexistent/orlo-t06/zsh");
        command.args(["-l", "-i"]);
        command.env("HOME", "/fixture/home");
        command.env("ZDOTDIR", "/fixture/home");
        let mut session =
            TerminalSession::from_command_with_integration(command, TerminalConfiguration::default(), true);
        assert!(session.pty().is_none());
        assert!(session.error().unwrap().contains("Cannot start terminal:"));
        let owned = session.integration_path().unwrap().to_path_buf();
        assert!(!owned.exists(), "failed spawn releases launch files before returning");
        session.close();
        assert!(!owned.exists());
    }

    #[test]
    fn opt_in_without_zdotdir_reads_home_and_restores_its_absence() {
        let home = std::env::temp_dir().join(format!("orlo-zsh-unset-zdotdir-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
        let original = "PROMPT='unset-fixture> '\n";
        std::fs::write(home.join(".zshrc"), original).unwrap();
        let mut command = CommandBuilder::new("/bin/zsh");
        command.args(["-i"]);
        command.env("HOME", &home);
        command.env_remove("ZDOTDIR");
        command.cwd(&home);
        let mut session =
            TerminalSession::from_command_with_integration(command, TerminalConfiguration::default(), true);
        wait_for(&mut session, |s| s.screen().contents().contains("unset-fixture>"));
        session.write(b"printf 'ZDOTDIR_PRESENT:%s\\n' \"${+ZDOTDIR}\"\r".to_vec());
        wait_for(&mut session, |s| s.commands().records().len() == 1);
        assert_eq!(session.commands().records()[0].output.lines().next(), Some("ZDOTDIR_PRESENT:0"));
        assert_eq!(std::fs::read_to_string(home.join(".zshrc")).unwrap(), original);
        let owned = session.integration_path().unwrap().to_path_buf();
        let pid = session.pty().unwrap().process_id();
        session.close();
        wait_for_cleanup(&owned, pid);
        assert!(!owned.exists());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn custom_norcs_launch_arguments_preserve_the_users_zdotdir() {
        let home = std::env::temp_dir().join(format!("orlo-zsh-launch-norcs-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let home = home.canonicalize().unwrap();
        for arguments in [vec!["-o", "norcs", "-i"], vec!["-fi", "-i"]] {
            let mut command = CommandBuilder::new("/bin/zsh");
            command.args(arguments);
            command.env("HOME", &home);
            command.env("ZDOTDIR", &home);
            command.cwd(&home);
            let mut session = TerminalSession::from_command_with_integration(
                command,
                TerminalConfiguration { cols: 512, ..Default::default() },
                true,
            );
            session.write(b"printf 'LAUNCH_DIR:%s\\n' \"$ZDOTDIR\"; exit\r".to_vec());
            let deadline = Instant::now() + Duration::from_secs(5);
            while session.exited().is_none() {
                assert!(Instant::now() < deadline);
                session.drain();
                std::thread::sleep(Duration::from_millis(5));
            }
            let text = session.screen().contents();
            assert!(text.contains(&format!("LAUNCH_DIR:{}", home.display())), "{text:?}");
            assert!(session.integration_path().is_none());
        }
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn disabling_startup_files_does_not_leak_the_private_zdotdir_into_child_shells() {
        let home = std::env::temp_dir().join(format!("orlo-zsh-norcs-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let home = home.canonicalize().unwrap();
        std::fs::write(home.join(".zshenv"), "unsetopt RCS\n").unwrap();
        let mut command = CommandBuilder::new("/bin/zsh");
        command.args(["-l", "-i"]);
        command.env("HOME", &home);
        command.env("ZDOTDIR", &home);
        command.cwd(&home);
        let mut session = TerminalSession::from_command_with_integration(
            command,
            TerminalConfiguration { cols: 512, ..Default::default() },
            true,
        );
        session.write(
            b"printf 'RESULT:%s\\n' \"$ZDOTDIR\"; /bin/zsh -i -c 'printf \"CHILD:%s\\n\" \"$ZDOTDIR\"'; exit\r"
                .to_vec(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.exited().is_none() {
            assert!(Instant::now() < deadline);
            session.drain();
            std::thread::sleep(Duration::from_millis(5));
        }
        let text = session.screen().contents();
        assert!(text.contains(&format!("RESULT:{}", home.display())), "{text:?}");
        assert!(text.contains(&format!("CHILD:{}", home.display())), "{text:?}");
        drop(session);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn launch_hooks_forward_configuration_report_commands_and_clean_up_private_files() {
        let home = std::env::temp_dir().join(format!("orlo-zsh-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let original = "typeset -g ORLO_FIXTURE_VALUE=forwarded\nPROMPT='test> '\n";
        std::fs::write(home.join(".zshrc"), original).unwrap();
        let mut command = CommandBuilder::new("/bin/zsh");
        command.args(["-l", "-i"]);
        command.env("HOME", &home);
        command.env("ZDOTDIR", &home);
        command.cwd(&home);
        let mut session =
            TerminalSession::from_command_with_integration(command, TerminalConfiguration::default(), true);
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.working_directory() != home || !session.screen().contents().contains("test>") {
            assert!(
                Instant::now() < deadline,
                "{:?} {:?} cwd={:?} home={:?}",
                session.error(),
                session.screen().contents(),
                session.working_directory(),
                home
            );
            session.drain();
            std::thread::sleep(Duration::from_millis(5));
        }
        session.write(b"printf '%s\\n' \"$ORLO_FIXTURE_VALUE\"; false\r".to_vec());
        while session.commands().records().is_empty() {
            assert!(Instant::now() < deadline, "{:?}", session.screen().contents());
            session.drain();
            std::thread::sleep(Duration::from_millis(5));
        }
        let record = &session.commands().records()[0];
        assert!(record.output.contains("forwarded"), "{:?}", record.output);
        assert_eq!(record.status, Some(1));
        assert_eq!(std::fs::read_to_string(home.join(".zshrc")).unwrap(), original);
        let path = session.integration_path().unwrap().to_path_buf();
        let pid = session.pty().unwrap().process_id();
        drop(session);
        wait_for_cleanup(&path, pid);
        assert!(!path.exists());
        std::fs::remove_dir_all(home).unwrap();
    }

    mod lifetime_tests {
        use super::*;

        fn fixture(mode: usize) -> (PathBuf, PathBuf, Vec<(PathBuf, String)>) {
            let root = std::env::temp_dir().join(format!(
                "orlo-t06-history-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            let home = root.join("雪 history");
            std::fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
            let home = home.canonicalize().unwrap();
            let logical = match mode {
                0 => home.clone(),
                1 | 3 => root.canonicalize().unwrap().join("profile files"),
                _ => home.join("配置 files"),
            };
            std::fs::create_dir_all(&logical).unwrap();
            let mut files = vec![(
                logical.join(".zshrc"),
                format!(
                    "PROMPT='history-fixture> '\nHISTSIZE=100\nSAVEHIST=100\n{}",
                    if mode == 3 { "HISTFILE=\"$HOME/custom history\"\n" } else { "" }
                ),
            )];
            if mode == 2 {
                files.push((home.join(".zprofile"), "ZDOTDIR=\"$HOME/配置 files\"\n".into()));
            }
            for (path, text) in &files {
                std::fs::write(path, text).unwrap();
            }
            eprintln!("T06_FIXTURE_HOME={}", home.display());
            (home, logical, files)
        }

        fn integrated(home: &Path, logical: &Path, mode: usize) -> TerminalSession {
            let mut command = CommandBuilder::new("/bin/zsh");
            command.args(if mode == 2 { vec!["-l", "-i"] } else { vec!["-i"] });
            command.env("HOME", home);
            command.env("TERM_PROGRAM", "Orlo");
            if matches!(mode, 1 | 3 | 4) {
                command.env("ZDOTDIR", logical);
            } else {
                command.env_remove("ZDOTDIR");
            }
            command.cwd(home);
            let mut session = TerminalSession::from_command_with_integration(
                command,
                TerminalConfiguration { cols: 512, ..Default::default() },
                true,
            );
            wait_for(&mut session, |s| s.screen().contents().contains("history-fixture>"));
            session
        }

        #[test]
        fn t06_integration_history_matches_plain_startup_and_persists_outside_private_directory() {
            for mode in 0..5 {
                let (home, logical, files) = fixture(mode);
                let mut plain = std::process::Command::new("/bin/zsh");
                if mode == 2 {
                    plain.arg("-l");
                }
                plain.args(["-i", "-c", "print -r -- \"HISTFILE:$HISTFILE\""]);
                plain.env("HOME", &home).env("TERM_PROGRAM", "Orlo").current_dir(&home);
                if matches!(mode, 1 | 3) {
                    plain.env("ZDOTDIR", &logical);
                } else {
                    plain.env_remove("ZDOTDIR");
                }
                let baseline = plain.output().unwrap();
                assert!(baseline.status.success(), "{:?}", baseline);
                // The installed macOS zsh corrupts inherited Unicode ZDOTDIR during plain
                // startup. Compare its valid baselines and verify the explicit Unicode case
                // against the declared directory instead; no lossy decoding is an oracle.
                let expected = if mode == 4 {
                    format!("HISTFILE:{}", logical.join(".zsh_history").display())
                } else {
                    let output = String::from_utf8(baseline.stdout).unwrap();
                    output.lines().find(|line| line.starts_with("HISTFILE:")).unwrap().to_string()
                };
                let mut session = integrated(&home, &logical, mode);
                let owned = session.integration_path().unwrap().to_path_buf();
                session.write(b"print -r -- \"HISTFILE:$HISTFILE\"\r".to_vec());
                wait_for(&mut session, |s| s.commands().records().len() == 1);
                let actual = session.commands().records()[0].output.lines().next().unwrap();
                eprintln!("T06_HISTORY mode={mode} expected={expected:?} actual={actual:?} private={owned:?}");
                assert_eq!(actual, expected, "integration must preserve the ordinary history destination");
                session.write(b"print -r -- ORLO-T06-HISTORY; exit\r".to_vec());
                wait_for(&mut session, |s| s.exited().is_some() && !owned.exists());
                let history = PathBuf::from(expected.strip_prefix("HISTFILE:").unwrap());
                assert!(std::fs::read_to_string(history).unwrap().contains("ORLO-T06-HISTORY"));
                for (path, text) in files {
                    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
                }
                assert!(!owned.exists());
                session.close();
                session.close();
                std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
            }
        }

        #[test]
        fn t06_private_launch_files_are_released_after_exit_while_review_session_remains_alive() {
            let (home, logical, files) = fixture(3);
            let mut session = integrated(&home, &logical, 3);
            let owned = session.integration_path().unwrap().to_path_buf();
            session.write(b"print -r -- LIFETIME-PID:$$; exit\r".to_vec());
            wait_for(&mut session, |s| s.exited().is_some());
            let output = session.commands().records().last().unwrap().output.clone();
            let pid: libc::pid_t =
                output.lines().find_map(|line| line.strip_prefix("LIFETIME-PID:")).unwrap().parse().unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "exit must have reaped the child");
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
            eprintln!("T06_LIFETIME pid={pid} reaped=true private_exists={} private={owned:?}", owned.exists());
            wait_for(&mut session, |_| !owned.exists());
            assert!(!owned.exists(), "private launch files outlive neither child nor PTY worker");
            assert_eq!(session.exited(), Some(0));
            assert_eq!(session.commands().records().last().unwrap().output, output);
            assert!(session.screen().contents().contains("LIFETIME-PID:"));
            for (path, text) in files {
                assert_eq!(std::fs::read_to_string(path).unwrap(), text);
            }
            session.close();
            assert_eq!(session.commands().records().last().unwrap().output, output);
            std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
        }
    }
}
