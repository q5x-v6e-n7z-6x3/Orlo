//! Real signals to a running Orlo, built and started as a child process with a home folder of
//! its own: SIGTERM, SIGINT and SIGHUP write the session as Quit writes it, then Orlo exits.
//! Orlo opens a window, so this needs a logged-in GUI session (a Mac's, or GitHub's macOS
//! runners).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The child Orlo, ended with the test whatever happens.
struct Orlo(std::process::Child);

impl Drop for Orlo {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Wait for `done`; a minute is a hang, not a slow machine.
fn wait_for(what: &str, log: &Path, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        if start.elapsed() > Duration::from_secs(60) {
            let log = std::fs::read_to_string(log).unwrap_or_default();
            panic!("no {what} after a minute\n{log}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_signal_writes_the_session_then_orlo_exits() {
    for signal in ["TERM", "INT", "HUP"] {
        // short: the hand-off socket's path must fit in 104 bytes
        let home = PathBuf::from(format!("/tmp/orlo-sig-{signal}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let file = home.join("notes.txt");
        std::fs::write(&file, "on disk\n").unwrap();
        let session = home.join("Library/Application Support/Orlo/Session.json");
        let log = home.join("orlo.log");
        let mut orlo = Orlo(
            Command::new(env!("CARGO_BIN_EXE_Orlo"))
                .arg(&file)
                .env("HOME", &home)
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(&log).unwrap())
                .spawn()
                .unwrap(),
        );
        // the launch's first autosave: Orlo is up, and its signal handler with it
        let mut gone = None;
        wait_for("session at launch", &log, || {
            gone = orlo.0.try_wait().unwrap();
            session.exists() || gone.is_some()
        });
        let said = || std::fs::read_to_string(&log).unwrap_or_default();
        assert!(gone.is_none(), "Orlo ended at launch ({gone:?})\n{}", said());
        std::fs::remove_file(&session).unwrap();
        let kill = Command::new("kill").arg(format!("-{signal}")).arg(orlo.0.id().to_string()).output().unwrap();
        let err = String::from_utf8_lossy(&kill.stderr);
        assert!(kill.status.success(), "kill -{signal}: {err}");
        let mut status = None;
        wait_for("exit", &log, || {
            status = orlo.0.try_wait().unwrap();
            status.is_some()
        });
        assert!(status.unwrap().success(), "SIG{signal}: {status:?}\n{}", said());
        let saved =
            std::fs::read_to_string(&session).unwrap_or_else(|e| panic!("SIG{signal}: no session ({e})\n{}", said()));
        assert!(saved.contains("notes.txt"), "SIG{signal}: {saved}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
