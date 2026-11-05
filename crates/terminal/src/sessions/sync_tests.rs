use super::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Fixture {
    home: PathBuf,
    session: TerminalSession,
    owned: PathBuf,
    pid: u32,
}
impl Fixture {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!(
            "orlo-n39-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&home).unwrap();
        let home = home.canonicalize().unwrap();
        std::fs::write(home.join(".zshrc"), "PROMPT='n39> '\n").unwrap();
        let mut command = CommandBuilder::new("/bin/zsh");
        command.args(["-i"]);
        command.env("HOME", &home);
        command.env("ZDOTDIR", &home);
        command.env("TERM", "xterm-256color");
        command.cwd(&home);
        let session = TerminalSession::from_command_with_integration(
            command,
            TerminalConfiguration { cols: 512, ..Default::default() },
            true,
        );
        let owned = session.integration_path().unwrap().to_path_buf();
        let pid = session.pty().unwrap().process_id();
        Self { home, session, owned, pid }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.owned.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!self.owned.exists(), "private integration/request files remain");
        assert_eq!(unsafe { libc::kill(self.pid as libc::pid_t, 0) }, -1, "shell must be reaped");
        std::fs::remove_dir_all(&self.home).unwrap();
    }
}
fn wait(session: &mut TerminalSession, ready: impl Fn(&TerminalSession) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready(session) {
        assert!(Instant::now() < deadline, "error={:?} screen={:?}", session.error(), session.screen().contents());
        session.drain();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn n39_real_integrated_pty_sync_quotes_paths_and_waits_for_report() {
    let mut f = Fixture::new();
    wait(&mut f.session, TerminalSession::can_sync_folder);
    let target = f.home.join("spaces 'quotes' \"$(>unexpected);雪\nnext");
    std::fs::create_dir(&target).unwrap();
    f.session.sync_folder(&target).unwrap();
    assert!(f.session.folder_sync_pending());
    assert!(!f.session.can_sync_folder());
    assert_eq!(f.session.working_directory(), f.home, "dispatch cannot optimistically change cwd");
    wait(&mut f.session, |s| !s.folder_sync_pending() && s.can_sync_folder());
    assert_eq!(
        f.session.working_directory(),
        target,
        "error={:?}, output={:?}",
        f.session.error(),
        f.session.commands().records().last().map(|r| &r.output)
    );
    assert_eq!(f.session.commands().records().last().unwrap().status, Some(0));
    f.session.write(b"print -r -- ACTUAL:$PWD\r".to_vec());
    wait(&mut f.session, |s| s.commands().records().len() == 2 && s.can_sync_folder());
    assert!(f.session.commands().records().last().unwrap().output.contains(&format!("ACTUAL:{}", target.display())));
    assert!(!f.home.join("unexpected").exists());
    f.session.write(b"\x1b[99;39~".to_vec());
    wait(&mut f.session, |s| s.error().is_some_and(|e| e.contains("not empty")));
    assert_eq!(f.session.commands().records().len(), 2, "the consumed private request cannot replay");
    assert_eq!(f.session.working_directory(), target);
}

#[test]
fn n39_partial_input_stale_markers_and_live_zle_recheck_refuse_without_editing_input() {
    let mut f = Fixture::new();
    wait(&mut f.session, TerminalSession::can_sync_folder);
    f.session.write(b"print -r -- KEEP".to_vec());
    assert!(!f.session.can_sync_folder());
    f.session.emulator.process(b"\x1b]133;B\x07\x1b]133;OrloReady\x07");
    assert!(!f.session.can_sync_folder(), "delayed readiness cannot acknowledge unsubmitted input");
    assert!(f.session.sync_folder(&f.home).is_err());
    f.session.write(b"\r".to_vec());
    wait(&mut f.session, |s| s.can_sync_folder() && s.commands().records().len() == 1);
    assert_eq!(f.session.commands().records()[0].output.trim_end(), "KEEP");

    // Bypass the application admission to prove the actual shell widget checks BUFFER.
    f.session.pty().unwrap().send(PtyCommand::Input(b"print -r -- RETAINED".to_vec())).unwrap();
    wait(&mut f.session, |s| s.screen().contents().contains("print -r -- RETAINED"));
    let target = f.home.join("must not enter");
    std::fs::create_dir(&target).unwrap();
    f.session.pty().unwrap().send(PtyCommand::SyncFolder(crate::shell::directory_command(&target).unwrap())).unwrap();
    wait(&mut f.session, |s| s.error().is_some_and(|e| e.contains("not empty")));
    assert_eq!(f.session.working_directory(), f.home);
    f.session.write(b"\r".to_vec());
    wait(&mut f.session, |s| s.commands().records().len() == 2 && s.can_sync_folder());
    assert_eq!(f.session.commands().records()[1].output.trim_end(), "RETAINED");
    f.session.paste("print -r -- PASTED");
    assert!(!f.session.can_sync_folder());
    assert!(f.session.sync_folder(&f.home).is_err());
    f.session.write(b"\r".to_vec());
    wait(&mut f.session, |s| s.commands().records().len() == 3 && s.can_sync_folder());
    assert_eq!(f.session.commands().records()[2].output.trim_end(), "PASTED");
    assert_eq!(f.session.working_directory(), f.home);
}

#[test]
fn n39_foreground_job_and_editor_are_rejected_at_actual_pty_write_then_exit_disables_sync() {
    let mut f = Fixture::new();
    wait(&mut f.session, TerminalSession::can_sync_folder);
    f.session.write(b"print JOB-READY; sleep 30\r".to_vec());
    wait(&mut f.session, |s| {
        s.commands().executing()
            && s.screen().contents().contains("JOB-READY")
            && !s.pty().unwrap().shell_is_foreground()
    });
    assert!(f.session.sync_folder(&f.home).is_err());
    f.session.pty().unwrap().send(PtyCommand::SyncFolder(crate::shell::directory_command(&f.home).unwrap())).unwrap();
    wait(&mut f.session, |s| s.error().is_some_and(|e| e.contains("not foreground")));
    assert_eq!(f.session.working_directory(), f.home);
    f.session.write(vec![3]);
    wait(&mut f.session, |s| !s.commands().executing());
    f.session.write(b"true\r".to_vec());
    wait(&mut f.session, TerminalSession::can_sync_folder);
    assert_eq!(f.session.commands().records()[0].status, Some(130));
    f.session.write(b"/usr/bin/vi -u NONE -n\r".to_vec());
    wait(&mut f.session, |s| s.screen().alternate_screen() && !s.pty().unwrap().shell_is_foreground());
    assert!(f.session.sync_folder(&f.home).is_err());
    f.session.write(b"\x1b:q!\r".to_vec());
    wait(&mut f.session, |s| !s.screen().alternate_screen());
    f.session.write(b"exit\r".to_vec());
    wait(&mut f.session, |s| s.exited().is_some());
    assert!(!f.session.can_sync_folder());
    assert!(f.session.sync_folder(&f.home).is_err());
    assert!(!f.session.folder_sync_pending());
}

#[test]
fn n39_new_input_and_close_cancel_pending_control_path_admission() {
    let mut f = Fixture::new();
    wait(&mut f.session, TerminalSession::can_sync_folder);
    let target = f.home.join("cancelled\nfolder");
    std::fs::create_dir(&target).unwrap();
    f.session.sync_folder(&target).unwrap();
    assert!(f.session.folder_sync_pending());
    f.session.write(vec![3]);
    assert!(!f.session.folder_sync_pending());
    let report = format!("\x1b]7;file://{}/cancelled%0Afolder\x07", f.home.display());
    f.session.emulator.process(report.as_bytes());
    assert_eq!(f.session.working_directory(), f.home, "cancelled sync cannot admit an otherwise rejected OSC7 path");
    f.session.close();
    assert!(!f.session.folder_sync_pending());
    assert!(!f.session.can_sync_folder());
    assert!(f.session.sync_folder(&target).is_err());
}

#[test]
fn n39_failed_cd_waits_for_real_report_and_plain_shells_never_admit_sync() {
    let mut f = Fixture::new();
    wait(&mut f.session, TerminalSession::can_sync_folder);
    f.session.sync_folder(&f.home.join("absent")).unwrap();
    wait(&mut f.session, |s| !s.folder_sync_pending() && s.can_sync_folder());
    assert_eq!(f.session.working_directory(), f.home);
    assert!(f.session.error().unwrap().contains("could not change"));
    assert_ne!(f.session.commands().records().last().unwrap().status, Some(0));
    for shell in ["/bin/sh", "/bin/zsh"] {
        let mut command = CommandBuilder::new(shell);
        command.args(["-i"]);
        command.env("HOME", &f.home);
        command.env("ZDOTDIR", &f.home);
        command.cwd(&f.home);
        let mut plain = TerminalSession::from_command(command, TerminalConfiguration::default());
        plain.emulator.process(b"\x1b]133;OrloReady\x07");
        assert!(!plain.can_sync_folder());
        assert!(plain.sync_folder(&f.home).is_err());
        plain.close();
    }
}

#[test]
fn n39_review_failed_sync_from_newline_cwd_finishes_without_widening_osc7() {
    let mut f = Fixture::new();
    wait(&mut f.session, TerminalSession::can_sync_folder);
    let target = f.home.join("confirmed\nfolder");
    std::fs::create_dir(&target).unwrap();
    f.session.sync_folder(&target).unwrap();
    wait(&mut f.session, |s| !s.folder_sync_pending() && s.can_sync_folder());
    assert_eq!(f.session.working_directory(), target);
    let report = f.session.emulator.directory_generation;
    f.session.sync_folder(&f.home.join("missing")).unwrap();
    wait(&mut f.session, |s| s.commands().records().len() == 2 && s.emulator.sync_ready);
    assert!(!f.session.folder_sync_pending(), "failed cd must finish even if its unchanged cwd report is rejected");
    assert!(f.session.error().is_some_and(|error| error.contains("could not change")));
    assert!(f.session.can_sync_folder());
    assert_ne!(f.session.commands().records().last().unwrap().status, Some(0));
    assert_eq!(f.session.working_directory(), target, "failure cannot invent a cwd change");
    assert_eq!(f.session.emulator.directory_generation, report, "ordinary control-byte OSC7 policy stays strict");
    f.session.emulator.process(b"\x1b]7;file:///unrelated%0Apath\x07");
    assert_eq!(f.session.working_directory(), target);
}

#[test]
fn n39_review_foreground_refusal_revokes_request_before_later_widget_invocation() {
    let mut f = Fixture::new();
    wait(&mut f.session, TerminalSession::can_sync_folder);
    let target = f.home.join("must not replay");
    std::fs::create_dir(&target).unwrap();
    f.session.write(b"print REPLAY-READY; sleep 30\r".to_vec());
    wait(&mut f.session, |s| s.commands().executing() && !s.pty().unwrap().shell_is_foreground());
    f.session.pty().unwrap().send(PtyCommand::SyncFolder(crate::shell::directory_command(&target).unwrap())).unwrap();
    wait(&mut f.session, |s| s.error().is_some_and(|error| error.contains("not foreground")));
    let refused_request = std::fs::read(f.owned.join("sync-folder")).unwrap();
    f.session.write(vec![3]);
    wait(&mut f.session, |s| !s.commands().executing());
    f.session.write(b"true\r".to_vec());
    wait(&mut f.session, |s| s.commands().records().len() == 2 && s.can_sync_folder());
    f.session.write(b"\x1b[99;39~".to_vec());
    wait(&mut f.session, |s| {
        s.error().is_some_and(|error| error.contains("not empty"))
            || s.commands().records().len() > 2 && s.emulator.sync_ready
    });
    assert_eq!(f.session.commands().records().len(), 2, "a refused request must never execute at a later prompt");
    assert_eq!(f.session.working_directory(), f.home);
    assert!(refused_request.is_empty(), "foreground refusal must revoke the private command immediately");
    assert!(f.session.error().is_some_and(|error| error.contains("not empty")));
}
