//! Nonblocking, cancellable PTY I/O with bounded input and output queues.

use crate::configuration::MAX_INPUT;
use crate::input::PtyCommand;
use crate::output::PtyEvent;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Closed sessions may still be reaping their shell. Keep their existing workers available
// to the application's asynchronous quit hook, without blocking tab/window closure.
static CLOSING_SESSIONS: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new());

/// Finish workers for sessions already closed. Call off the foreground thread, after closing
/// the application's terminals. Active sessions are unaffected.
pub fn wait_for_closing_sessions() {
    let workers = std::mem::take(&mut *CLOSING_SESSIONS.lock().unwrap_or_else(|e| e.into_inner()));
    for worker in workers {
        if worker.join().is_err() {
            eprintln!("Orlo: terminal worker panicked during cleanup");
        }
    }
}

pub struct PtySession {
    input: SyncSender<PtyCommand>,
    output: Receiver<PtyEvent>,
    stop: Arc<AtomicBool>,
    foreground_shell: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    cancellation: Option<UnixStream>,
    #[cfg(test)]
    pid: u32,
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}

/// An I/O error that only means "try again".
fn retry(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted)
}

/// `<sys/ttycom.h>`'s `TIOCSIG`, `_IO('t', 95)`, which libc does not define for Apple: on a
/// master, flush the tty's queues (unless `NOFLSH`) and signal the foreground process group.
const TIOCSIG: libc::c_ulong = 0x2000_745f;

/// The signal a lone `bytes` raises: the tty's own interrupt, quit or suspend character
/// while it turns those into signals (`ISIG`).
fn signal_for(fd: std::os::fd::RawFd, bytes: &[u8]) -> Option<libc::c_int> {
    let [byte] = bytes else { return None };
    // SAFETY: tcgetattr writes the termios we own; a master reports its slave's settings.
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut t) } != 0 || t.c_lflag & libc::ISIG == 0 {
        return None;
    }
    let signals = [(libc::VINTR, libc::SIGINT), (libc::VQUIT, libc::SIGQUIT), (libc::VSUSP, libc::SIGTSTP)];
    signals.into_iter().find(|&(i, _)| t.c_cc[i] == *byte).map(|(_, signal)| signal)
}

impl PtySession {
    #[cfg(test)]
    pub(crate) fn test_events(events: Vec<PtyEvent>) -> (Self, SyncSender<PtyEvent>) {
        let (input, _) = mpsc::sync_channel(1);
        let (sender, output) = mpsc::sync_channel(events.len().max(1));
        for event in events {
            sender.send(event).unwrap();
        }
        (
            Self {
                input,
                output,
                stop: Arc::new(AtomicBool::new(false)),
                foreground_shell: Arc::new(AtomicBool::new(false)),
                worker: None,
                cancellation: None,
                pid: 0,
            },
            sender,
        )
    }

    pub fn spawn(command: CommandBuilder, rows: u16, cols: u16) -> Result<Self, String> {
        Self::spawn_with_integration(command, rows, cols, None)
    }

    pub(crate) fn spawn_with_integration(
        command: CommandBuilder,
        rows: u16,
        cols: u16,
        integration: Option<crate::shell::integration::Integration>,
    ) -> Result<Self, String> {
        let pair = native_pty_system().openpty(pty_size(rows, cols)).map_err(|e| e.to_string())?;
        let fd = pair.master.as_raw_fd().ok_or("PTY has no file descriptor")?;
        // Nonblocking reads and writes keep a stalled child and full output queue cancellable.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let (cancellation, wake) = UnixStream::pair().map_err(|e| e.to_string())?;
        let mut child = pair.slave.spawn_command(command).map_err(|e| e.to_string())?;
        let pid = child.process_id().unwrap_or(0);
        drop(pair.slave);
        let (input, commands) = mpsc::sync_channel(16);
        let (events, output) = mpsc::sync_channel(32);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let foreground_shell = Arc::new(AtomicBool::new(false));
        let foreground = foreground_shell.clone();
        let worker = std::thread::spawn(move || {
            let master = pair.master;
            let mut pending_output = None;
            let mut pending_input = Vec::new();
            let mut queued = std::collections::VecDeque::new();
            let mut written = 0;
            let mut syncing_folder = false;
            let mut eof = false;
            let mut exit = None;
            let mut buffer = [0; 8192];
            'io: while !stopped.load(Ordering::Acquire) {
                if integration.is_some() {
                    foreground
                        .store(pid > 0 && master.process_group_leader() == Some(pid as libc::pid_t), Ordering::Relaxed);
                }
                if let Some(event) = pending_output.take() {
                    match events.try_send(event) {
                        Ok(()) => {}
                        Err(TrySendError::Full(event)) => pending_output = Some(event),
                        Err(TrySendError::Disconnected(_)) => break,
                    }
                }
                // Commands keep arriving while input waits on a program that is not reading it:
                // a resize applies at once, and an interrupt pre-empts. The kernel takes no
                // byte while the tty's queue is full, so the interrupt is raised as the line
                // discipline would raise it, and the input before it is dropped, as it flushes.
                while queued.len() < 16 {
                    match commands.try_recv() {
                        Ok(PtyCommand::Resize(rows, cols)) => {
                            if let Err(e) = master.resize(pty_size(rows, cols)) {
                                let _ = events.try_send(PtyEvent::Error(format!("Cannot resize terminal: {e}")));
                            }
                        }
                        Ok(PtyCommand::Input(bytes))
                            if !syncing_folder
                                && (!pending_input.is_empty()
                                    || queued.iter().any(|c| matches!(c, PtyCommand::Input(_)))) =>
                        {
                            // SAFETY: TIOCSIG takes the signal number by value.
                            let sent = signal_for(fd, &bytes)
                                .is_some_and(|signal| unsafe { libc::ioctl(fd, TIOCSIG, signal) } == 0);
                            if sent {
                                pending_input.clear();
                                written = 0;
                                queued.retain(|c| !matches!(c, PtyCommand::Input(_)));
                            } else {
                                queued.push_back(PtyCommand::Input(bytes));
                            }
                        }
                        Ok(command) => queued.push_back(command),
                        Err(TryRecvError::Disconnected) => break 'io,
                        Err(TryRecvError::Empty) => break,
                    }
                }
                if pending_input.is_empty() {
                    match queued.pop_front() {
                        Some(PtyCommand::Input(bytes)) => {
                            syncing_folder = false;
                            pending_input = bytes;
                            written = 0;
                        }
                        Some(PtyCommand::SyncFolder(command)) => {
                            let result = integration
                                .as_ref()
                                .ok_or_else(|| "Shell integration is unavailable".to_string())
                                .and_then(|integration| {
                                    integration.write_folder_command(&command).map_err(|e| e.to_string())
                                });
                            match result {
                                Ok(()) => {
                                    syncing_folder = true;
                                    pending_input = b"\x1b[99;39~".to_vec();
                                    written = 0;
                                }
                                Err(error) => {
                                    let _ = events.try_send(PtyEvent::Error(error));
                                }
                            }
                        }
                        // applied as they arrive
                        Some(PtyCommand::Resize(..)) | None => {}
                    }
                }
                let (reading, writing) = (pending_output.is_none() && !eof, !pending_input.is_empty());
                let pty_poll = libc::pollfd {
                    fd,
                    events: if reading { libc::POLLIN } else { 0 } | if writing { libc::POLLOUT } else { 0 },
                    revents: 0,
                };
                // Closing the owner's socket wakes an idle poll immediately, independently
                // of the poll's timer deadline during application termination.
                let mut polls = [pty_poll, libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
                let result = unsafe { libc::poll(polls.as_mut_ptr(), 2, 10) };
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let poll = polls[0];
                if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    let _ = events.try_send(PtyEvent::Error(std::io::Error::last_os_error().to_string()));
                    break;
                }
                if writing && poll.revents & libc::POLLOUT != 0 {
                    // Admission is checked at the write, not from a cached UI prompt marker.
                    if syncing_folder && (pid == 0 || master.process_group_leader() != Some(pid as libc::pid_t)) {
                        pending_input.clear();
                        syncing_folder = false;
                        written = 0;
                        let revoked =
                            integration.as_ref().map(|integration| integration.write_folder_command(&[])).transpose();
                        let error = match revoked {
                            Ok(_) => "Terminal folder sync refused: the shell is not foreground".into(),
                            Err(error) => format!("Terminal folder sync refused: the shell is not foreground; cannot revoke request: {error}"),
                        };
                        let _ = events.try_send(PtyEvent::Error(error));
                        continue;
                    }
                    match writer.write(&pending_input[written..]) {
                        Ok(n) if n > 0 => {
                            written += n;
                            if written == pending_input.len() {
                                pending_input.clear();
                            }
                        }
                        Err(e) if retry(&e) => {}
                        result => {
                            let _ = events.try_send(PtyEvent::Error(format!("Cannot write to terminal: {result:?}")));
                            break;
                        }
                    }
                }
                if reading && poll.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                    match reader.read(&mut buffer) {
                        Ok(0) => eof = true,
                        Ok(n) => pending_output = Some(PtyEvent::Output(buffer[..n].to_vec())),
                        Err(e) if retry(&e) => {}
                        Err(e) if e.raw_os_error() == Some(libc::EIO) => eof = true,
                        Err(e) => {
                            pending_output = Some(PtyEvent::Error(e.to_string()));
                            eof = true;
                        }
                    }
                }
                if exit.is_none() {
                    match child.try_wait() {
                        Ok(Some(status)) => exit = Some(status.exit_code()),
                        Ok(None) => {}
                        Err(e) => {
                            let _ = events.try_send(PtyEvent::Error(format!("Cannot wait for shell: {e}")));
                            break 'io;
                        }
                    }
                }
                if eof && pending_output.is_none() {
                    if let Some(code) = exit {
                        match events.try_send(PtyEvent::Exited(code)) {
                            Err(TrySendError::Full(_)) => {}
                            _ => break,
                        }
                    }
                }
                if poll.events == 0 {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            foreground.store(false, Ordering::Relaxed);
            if exit.is_none() {
                crate::processes::terminate_shell(&*master, &mut *child, pid);
            }
            drop(writer);
            drop(reader);
            drop(master);
            if exit.is_none() {
                // macOS can keep a killed writer in tty close until its master is closed.
                let _ = child.wait();
            }
            // Launch files belong to the worker: close/Drop only signal it, and the child
            // can still read startup files or write history until it has been reaped.
            drop(integration);
        });
        Ok(Self {
            input,
            output,
            stop,
            foreground_shell,
            worker: Some(worker),
            cancellation: Some(cancellation),
            #[cfg(test)]
            pid,
        })
    }

    pub fn send(&self, command: PtyCommand) -> Result<(), String> {
        if matches!(&command, PtyCommand::Input(bytes) | PtyCommand::SyncFolder(bytes) if bytes.len() > MAX_INPUT) {
            return Err("Terminal input is limited to 1 MiB per paste".into());
        }
        self.input.try_send(command).map_err(|e| match e {
            TrySendError::Full(_) => "Terminal input is busy; wait and try again".into(),
            TrySendError::Disconnected(_) => "The shell has exited; close the terminal to start another".into(),
        })
    }
    pub(crate) fn shell_is_foreground(&self) -> bool {
        self.foreground_shell.load(Ordering::Relaxed)
    }

    pub fn try_recv(&self) -> Result<PtyEvent, TryRecvError> {
        self.output.try_recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<PtyEvent, mpsc::RecvTimeoutError> {
        self.output.recv_timeout(timeout)
    }

    #[cfg(test)]
    pub(crate) fn process_id(&self) -> u32 {
        self.pid
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.cancellation.take();
        if let Some(worker) = self.worker.take() {
            let mut closing = CLOSING_SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
            closing.retain(|worker| !worker.is_finished());
            closing.push(worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{key_bytes, Key, Modifiers};
    use std::time::{Duration, Instant};

    fn command(script: &str) -> portable_pty::CommandBuilder {
        let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
        cmd.args(["-c", script]);
        cmd.cwd(std::env::temp_dir());
        cmd
    }

    fn until(session: &PtySession, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = String::new();
        while Instant::now() < deadline {
            if let Ok(event) = session.output.recv_timeout(Duration::from_millis(50)) {
                match event {
                    PtyEvent::Output(bytes) => output.push_str(&String::from_utf8_lossy(&bytes)),
                    PtyEvent::Error(error) => panic!("PTY error: {error}"),
                    PtyEvent::Exited(_) => break,
                }
                if output.find(needle).is_some_and(|at| output[at..].contains('\n')) {
                    return output;
                }
            }
        }
        panic!("missing {needle:?} in {output:?}");
    }

    #[test]
    fn interrupt_reaches_a_program_behind_a_stalled_paste() {
        let s = PtySession::spawn(command("trap 'echo interrupted; exit' INT; echo ready; sleep 60"), 24, 80).unwrap();
        until(&s, "ready");
        // more than the tty's queue holds, which `sleep` never reads
        s.send(PtyCommand::Input("line\n".repeat(2000).into_bytes())).unwrap();
        s.send(PtyCommand::Resize(30, 100)).unwrap();
        s.send(PtyCommand::Input(vec![3])).unwrap();
        assert!(until(&s, "interrupted").contains("interrupted"));
    }

    /// Whether `pid` is still alive after waiting up to `secs` for it to go.
    fn alive_after(pid: libc::pid_t, secs: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// Kills `pid` if `alive_after` found it still running, so a failed test leaves nothing.
    fn reap(pid: libc::pid_t, secs: u64) -> bool {
        let alive = alive_after(pid, secs);
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        alive
    }

    #[test]
    fn real_tty_accepts_input_and_reports_resized_dimensions() {
        let session = PtySession::spawn(
            command("test -t 0 && test -t 1 && echo tty-ready; read value; printf 'got:%s\\n' \"$value\"; stty size"),
            24,
            80,
        )
        .unwrap();
        until(&session, "tty-ready");
        session.send(PtyCommand::Resize(31, 97)).unwrap();
        session.send(PtyCommand::Input(b"hello\r".to_vec())).unwrap();
        let output = until(&session, "31 97");
        assert!(output.contains("got:hello"), "{output:?}");
    }

    #[test]
    fn dropping_session_kills_and_reaps_shell() {
        let session = PtySession::spawn(command("echo ready; exec sleep 60"), 24, 80).unwrap();
        until(&session, "ready");
        let pid = session.pid as libc::pid_t;
        drop(session);
        assert!(!alive_after(pid, 5), "shell remains alive");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[test]
    fn closing_session_allows_the_shell_hangup_handler_to_persist_its_state() {
        let directory = std::env::temp_dir().join(format!(
            "orlo-pty-hangup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let saved = directory.join("saved-state");
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args([
            "-c",
            "trap 'printf saved > \"$1\"; exit' HUP; echo hangup-ready; while :; do read value; done",
            "orlo-hangup",
            saved.to_str().unwrap(),
        ]);
        let session = PtySession::spawn(cmd, 24, 80).unwrap();
        until(&session, "hangup-ready");
        let pid = session.pid;
        drop(session);
        wait_for_closing_sessions();
        assert_eq!(std::fs::read_to_string(saved).unwrap(), "saved", "ordinary HUP cleanup must run before forcing");
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn closing_terminal_stops_background_jobs_in_the_shell() {
        let mut cmd = CommandBuilder::new("/bin/zsh");
        cmd.args(["-f", "-i", "-c", "sleep 60 & print child:$!; wait"]);
        let session = PtySession::spawn(cmd, 24, 80).unwrap();
        let output = until(&session, "child:");
        let pid: libc::pid_t =
            output.split("child:").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
        drop(session);
        assert!(!reap(pid, 3), "the shell's background job remains alive");
    }

    #[test]
    fn closing_an_idle_shell_stops_jobs_even_with_no_hup_configured() {
        let mut cmd = CommandBuilder::new("/bin/zsh");
        cmd.args(["-f", "-i", "-o", "NO_HUP"]);
        let session = PtySession::spawn(cmd, 24, 80).unwrap();
        session.send(PtyCommand::Input(b"sleep 60 & print background:$!\r".to_vec())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = String::new();
        let pid = loop {
            assert!(Instant::now() < deadline, "no background pid in {output:?}");
            if let Ok(PtyEvent::Output(bytes)) = session.output.recv_timeout(Duration::from_millis(50)) {
                output.push_str(&String::from_utf8_lossy(&bytes));
                if let Some(pid) = output
                    .split_inclusive('\n')
                    .filter(|line| line.ends_with('\n'))
                    .find_map(|line| line.strip_prefix("background:")?.trim().parse::<libc::pid_t>().ok())
                {
                    break pid;
                }
            }
        };
        drop(session);
        assert!(!reap(pid, 3), "the idle shell left its background job running");
    }

    #[test]
    fn output_backpressure_does_not_prevent_cleanup() {
        let session = PtySession::spawn(command("echo ready; exec yes x"), 24, 80).unwrap();
        until(&session, "ready");
        let pid = session.pid as libc::pid_t;
        std::thread::sleep(Duration::from_millis(100));
        drop(session);
        assert!(!alive_after(pid, 3));
    }

    #[test]
    fn control_c_interrupts_the_real_foreground_process() {
        let session =
            PtySession::spawn(command("trap 'echo interrupted; exit' INT; echo ready; sleep 60"), 24, 80).unwrap();
        until(&session, "ready");
        session
            .send(PtyCommand::Input(
                key_bytes(Key { name: "c", modifiers: Modifiers { control: true, ..Modifiers::default() } }, false)
                    .unwrap(),
            ))
            .unwrap();
        assert!(until(&session, "interrupted").contains("interrupted"));
    }
}
