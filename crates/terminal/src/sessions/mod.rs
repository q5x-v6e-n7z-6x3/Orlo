//! Persistent shell, screen, and selection state; hiding a view does not drop a session.

use crate::configuration::TerminalConfiguration;
use crate::emulation::{Emulator, Screen};
use crate::input::{paste_bytes, PtyCommand};
use crate::output::{DrainStatus, PtyEvent};
use crate::pty::PtySession;
use crate::shell::login_command;
use portable_pty::CommandBuilder;
use std::path::Path;
use std::sync::mpsc::TryRecvError;

pub type CellPosition = (u16, u16);
pub type Selection = (CellPosition, CellPosition);

pub struct TerminalSession {
    #[cfg(test)]
    integration_path: Option<std::path::PathBuf>,
    pty: Option<PtySession>,
    emulator: Emulator,
    error: Option<String>,
    exited: Option<u32>,
    selection: Option<Selection>,
    initial_cwd: std::path::PathBuf,
    folder_sync: FolderSync,
}

#[derive(Default)]
struct FolderSync {
    integrated: bool,
    input_execution: Option<u64>,
    pending: Option<(std::path::PathBuf, u64, u64)>,
}

impl TerminalSession {
    pub fn new(cwd: &Path) -> Self {
        Self::from_command(login_command(cwd), TerminalConfiguration::default())
    }

    pub fn from_command(command: CommandBuilder, configuration: TerminalConfiguration) -> Self {
        Self::from_command_with_integration(command, configuration, false)
    }

    /// Launch-local zsh hooks are opt-in; existing sessions are never reconfigured.
    pub fn from_command_with_integration(
        mut command: CommandBuilder,
        configuration: TerminalConfiguration,
        enabled: bool,
    ) -> Self {
        let installed = if enabled { crate::shell::integration::Integration::install(&mut command) } else { Ok(None) };
        let (integration, integration_error) = match installed {
            Ok(integration) => (integration, None),
            Err(error) => (None, Some(format!("Shell integration unavailable: {error}"))),
        };
        let initial_cwd = command
            .get_cwd()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let integrated = integration.is_some();
        #[cfg(test)]
        let integration_path = integration.as_ref().map(|i| i.path().to_path_buf());
        let (pty, error) =
            match PtySession::spawn_with_integration(command, configuration.rows, configuration.cols, integration) {
                Ok(pty) => (Some(pty), None),
                Err(error) => (None, Some(format!("Cannot start terminal: {error}"))),
            };
        let mut emulator = Emulator::new(configuration);
        emulator.set_directory(initial_cwd.clone());
        Self {
            pty,
            emulator,
            error: error.or(integration_error),
            exited: None,
            selection: None,
            initial_cwd,
            folder_sync: FolderSync { integrated, ..Default::default() },
            #[cfg(test)]
            integration_path,
        }
    }

    pub fn can_sync_folder(&self) -> bool {
        self.folder_sync.integrated
            && self.folder_sync.pending.is_none()
            && self.exited.is_none()
            && self.emulator.sync_ready
            && !self.screen().alternate_screen()
            && self.folder_sync.input_execution.is_none_or(|generation| generation < self.emulator.execution_generation)
            && self.pty.as_ref().is_some_and(PtySession::shell_is_foreground)
    }

    pub fn sync_folder(&mut self, directory: &Path) -> Result<(), String> {
        if !self.can_sync_folder() {
            return Err("Terminal folder sync needs an integrated foreground shell at a fresh empty prompt".into());
        }
        let command = crate::shell::directory_command(directory)?;
        self.pty.as_ref().unwrap().send(PtyCommand::SyncFolder(command))?;
        self.folder_sync.pending =
            Some((directory.to_path_buf(), self.emulator.execution_generation, self.emulator.directory_generation));
        self.folder_sync.input_execution = Some(self.emulator.execution_generation);
        self.emulator.sync_ready = false;
        self.emulator.expect_directory(Some(directory.to_path_buf()));
        self.error = None;
        Ok(())
    }

    pub fn folder_sync_pending(&self) -> bool {
        self.folder_sync.pending.is_some()
    }

    pub fn working_directory(&self) -> &Path {
        self.emulator.working_directory().unwrap_or(&self.initial_cwd)
    }

    #[cfg(test)]
    pub(crate) fn integration_path(&self) -> Option<&Path> {
        self.integration_path.as_deref()
    }

    pub fn pty(&self) -> Option<&PtySession> {
        self.pty.as_ref()
    }

    /// Release the PTY through its normal process cleanup even if a view retains this session.
    pub fn close(&mut self) {
        self.pty = None;
        self.folder_sync.pending = None;
        self.emulator.expect_directory(None);
        self.emulator.sync_ready = false;
        #[cfg(test)]
        {
            self.integration_path = None;
        }
    }

    pub fn commands(&self) -> &crate::commands::Commands {
        self.emulator.commands()
    }

    pub fn screen(&self) -> &Screen {
        self.emulator.screen()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn exited(&self) -> Option<u32> {
        self.exited
    }

    /// Apply at most one full output queue per UI tick so large output remains responsive.
    pub fn drain(&mut self) -> DrainStatus {
        const STOPPED: DrainStatus = DrainStatus { running: false, changed: true };
        let mut changed = false;
        for _ in 0..32 {
            let Some(pty) = &self.pty else {
                return DrainStatus { running: false, changed };
            };
            match pty.try_recv() {
                Ok(PtyEvent::Output(bytes)) => self.emulator.process(&bytes),
                Ok(PtyEvent::Error(error)) => {
                    self.folder_sync.pending = None;
                    self.emulator.expect_directory(None);
                    self.error = Some(error);
                }
                Ok(PtyEvent::Exited(code)) => {
                    self.folder_sync.pending = None;
                    self.emulator.expect_directory(None);
                    self.exited = Some(code);
                    self.emulator.exited(code);
                    return STOPPED;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.folder_sync.pending = None;
                    self.emulator.expect_directory(None);
                    self.emulator.exited(*self.exited.get_or_insert(1));
                    return STOPPED;
                }
            }
            changed = true;
        }
        if std::mem::take(&mut self.emulator.sync_refused) {
            self.folder_sync.pending = None;
            self.emulator.expect_directory(None);
            self.error = Some("Terminal folder sync refused: the shell input line is not empty".into());
        }
        if let Some((directory, execution, report)) = &self.folder_sync.pending {
            let failed = self
                .commands()
                .records()
                .last()
                .is_some_and(|command| command.status.is_some_and(|status| status != 0));
            if self.emulator.sync_ready
                && self.emulator.execution_generation > *execution
                && (failed || self.emulator.directory_generation > *report)
            {
                if failed || self.working_directory() != directory {
                    self.error = Some("The terminal could not change to the requested folder".into());
                }
                self.folder_sync.pending = None;
                self.emulator.expect_directory(None);
            }
        }
        // the terminal's own answers (cursor reports, device attributes) are not the user's
        // input: they leave the view's scrollback, selection and a pending folder sync alone
        let replies = self.emulator.take_replies();
        if let Some(pty) = self.pty.as_ref().filter(|_| !replies.is_empty() && self.exited.is_none()) {
            if let Err(error) = pty.send(PtyCommand::Input(replies)) {
                self.error = Some(error);
            }
        }
        if changed {
            self.selection = None;
        }
        DrainStatus { running: true, changed }
    }

    pub fn write(&mut self, bytes: Vec<u8>) {
        self.folder_sync.pending = None;
        self.emulator.expect_directory(None);
        self.folder_sync.input_execution = Some(self.emulator.execution_generation);
        self.emulator.sync_ready = false;
        self.error = if self.exited.is_some() {
            Some("The shell has exited; close the terminal to start another".into())
        } else {
            self.pty
                .as_ref()
                .ok_or_else(|| "The terminal is not running".to_string())
                .and_then(|pty| pty.send(PtyCommand::Input(bytes)))
                .err()
        };
        if self.error.is_none() {
            self.emulator.screen_mut().set_scrollback(0);
            self.selection = None;
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.write(paste_bytes(text, self.screen().bracketed_paste()));
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if self.screen().size() == (rows, cols) {
            return;
        }
        match self.pty.as_ref().map(|pty| pty.send(PtyCommand::Resize(rows, cols))) {
            Some(Err(error)) => self.error = Some(error),
            _ => self.emulator.resize(rows, cols),
        }
        self.selection = None;
    }

    /// Trim oldest primary history immediately without restarting the PTY or resetting modes.
    pub fn set_scrollback_limit(&mut self, rows: usize) -> bool {
        let changed = self.emulator.set_scrollback_limit(rows);
        if changed {
            self.selection = None;
        }
        changed
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    pub fn set_selection(&mut self, selection: Option<Selection>) {
        self.selection = selection;
    }

    pub fn extend_selection(&mut self, position: CellPosition) -> bool {
        self.selection.as_mut().map(|(_, end)| *end = position).is_some()
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection.map(|(a, b)| {
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            self.screen().contents_between(a.0, a.1, b.0, b.1)
        })
    }

    pub fn select_all(&mut self) {
        let (rows, cols) = self.screen().size();
        self.selection = Some(((0, 0), (rows - 1, cols)));
    }

    /// Return whether the primary screen accepted scrolling.
    pub fn scroll_lines(&mut self, lines: isize) -> bool {
        if self.screen().alternate_screen() {
            return false;
        }
        let screen = self.emulator.screen_mut();
        let offset = (screen.scrollback() as isize + lines).max(0) as usize;
        screen.set_scrollback(offset);
        self.selection = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t07_rejected_input_preserves_terminal_selection_and_scrollback() {
        let mut emulator = Emulator::with_size(2, 30, 20);
        emulator.process(b"one\r\ntwo\r\nthree");
        emulator.screen_mut().set_scrollback(1);
        let (pty, _sender) = PtySession::test_events(vec![]);
        let selection = Some(((0, 0), (0, 3)));
        let mut session = TerminalSession {
            integration_path: None,
            pty: Some(pty),
            emulator,
            error: None,
            exited: None,
            selection,
            initial_cwd: "/fixture".into(),
            folder_sync: FolderSync::default(),
        };
        session.write(b"unadmitted".to_vec());
        assert!(session.error().unwrap().contains("exited"));
        assert_eq!(session.selection(), selection);
        assert_eq!(session.screen().scrollback(), 1);
    }

    #[test]
    fn disconnected_pty_clears_mouse_modes_and_finalizes_command_without_losing_final_state() {
        let mut emulator = Emulator::with_size(3, 40, 20);
        emulator.set_directory("/old".into());
        emulator.process(b"\x1b]133;A\x07$ \x1b]133;B\x07run\x1b]133;C\x07final.rs:2:3\x1b[?1003h\x1b[?1006h");
        let (pty, sender) = PtySession::test_events(vec![PtyEvent::Error("read failure".into())]);
        drop(sender);
        let mut session = TerminalSession {
            integration_path: None,
            pty: Some(pty),
            emulator,
            error: None,
            exited: None,
            selection: Some(((0, 0), (0, 5))),
            initial_cwd: "/initial".into(),
            folder_sync: FolderSync::default(),
        };
        let before = session.screen().clone();
        assert_eq!(session.drain(), DrainStatus { running: false, changed: true });
        assert!(!crate::mouse::requested(session.screen()));
        assert_eq!(session.screen().mouse_protocol_encoding(), vt100::MouseProtocolEncoding::Default);
        assert_eq!(session.exited(), Some(1));
        assert_eq!(session.error(), Some("read failure"));
        assert_eq!(session.screen().contents(), before.contents());
        assert!(session.screen().same_contents(&before));
        assert_eq!(session.selection(), Some(((0, 0), (0, 5))));
        assert_eq!(session.working_directory(), Path::new("/old"));
        assert_eq!(session.screen().directory_at(0, 5), Some(Path::new("/old")));
        let records = session.commands().records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, Some(1));
        assert_eq!(records[0].output, "final.rs:2:3");
        session.drain();
        assert_eq!(session.commands().records().len(), 1);
    }

    #[test]
    fn device_replies_are_not_user_input() {
        let report = b"\x1b]133;OrloReady\x07\x1b[6n".to_vec();
        let (pty, _sender) = PtySession::test_events(vec![PtyEvent::Output(report)]);
        let mut session = TerminalSession {
            integration_path: None,
            pty: Some(pty),
            emulator: Emulator::with_size(3, 40, 20),
            error: None,
            exited: None,
            selection: None,
            initial_cwd: "/initial".into(),
            folder_sync: FolderSync::default(),
        };
        session.drain();
        assert!(session.emulator.sync_ready, "a cursor report keeps Sync Folder available");
        assert_eq!(session.folder_sync.input_execution, None);
    }

    #[test]
    fn recoverable_error_keeps_reporting_until_channel_disconnects() {
        let mut emulator = Emulator::with_size(3, 40, 0);
        emulator.process(b"\x1b[?1000h\x1b[?1005hFINAL");
        let (pty, sender) = PtySession::test_events(vec![PtyEvent::Error("resize failure".into())]);
        let mut session = TerminalSession {
            integration_path: None,
            pty: Some(pty),
            emulator,
            error: None,
            exited: None,
            selection: None,
            initial_cwd: "/initial".into(),
            folder_sync: FolderSync::default(),
        };
        assert_eq!(session.drain(), DrainStatus { running: true, changed: true });
        assert!(crate::mouse::requested(session.screen()));
        assert_eq!(session.screen().mouse_protocol_encoding(), vt100::MouseProtocolEncoding::Utf8);
        assert_eq!(session.exited(), None);
        assert_eq!(session.error(), Some("resize failure"));
        drop(sender);
        assert_eq!(session.drain(), DrainStatus { running: false, changed: true });
        assert!(!crate::mouse::requested(session.screen()));
        assert_eq!(session.screen().contents(), "FINAL");
        assert_eq!(session.error(), Some("resize failure"));
    }

    #[test]
    fn live_retention_trims_paired_provenance_and_invalidates_selection_and_command_ranges() {
        let mut emulator = Emulator::with_size(2, 30, 20);
        emulator.set_directory("/fixture/old".into());
        emulator.process(b"\x1b]133;B\x07run\x1b]133;C\x07old.rs\r\n\x1b]7;file:///fixture/new\x07new.rs\r\nlast.rs\x1b]133;D;0\x07\x1b[?1002h\x1b[?1006h");
        assert_eq!(emulator.commands().records().len(), 1);
        let (pty, _sender) = PtySession::test_events(vec![]);
        let mut session = TerminalSession {
            integration_path: None,
            pty: Some(pty),
            emulator,
            error: None,
            exited: None,
            selection: Some(((0, 0), (0, 6))),
            initial_cwd: "/fixture/old".into(),
            folder_sync: FolderSync::default(),
        };
        let identity = session.screen().identity();
        let before = session.screen().contents();
        assert!(!session.set_scrollback_limit(20));
        assert!(session.selection().is_some());
        assert!(identity.matches(session.screen()));
        assert!(session.set_scrollback_limit(0));
        assert!(session.pty().is_some());
        assert!(session.selection().is_none());
        assert!(!identity.matches(session.screen()));
        assert_eq!(session.screen().contents(), before);
        assert_eq!(session.screen().directory_at(0, 0), Some(Path::new("/fixture/new")));
        assert_eq!(session.working_directory(), Path::new("/fixture/new"));
        assert_eq!(session.screen().mouse_protocol_mode(), vt100::MouseProtocolMode::ButtonMotion);
        assert_eq!(session.screen().mouse_protocol_encoding(), vt100::MouseProtocolEncoding::Sgr);
        assert!(session.commands().records().is_empty());
        session.scroll_lines(999);
        assert_eq!(session.screen().scrollback(), 0);
        session.emulator.process(b"\x1b]133;C\x07fresh.rs\x1b]133;D;0\x07");
        assert_eq!(session.commands().records().len(), 1);
        assert!(session.commands().records()[0].output.contains("fresh.rs"));
        let epoch = session.screen().identity();
        assert!(session.set_scrollback_limit(2));
        assert!(!epoch.matches(session.screen()));
        assert_eq!(session.commands().records().len(), 1, "growth retains valid command archives");
        session.emulator.process(b"\r\none\r\ntwo\r\nthree\r\nfour");
        session.scroll_lines(999);
        assert_eq!(session.screen().scrollback(), 2);
        assert_eq!(session.screen().directory_at(0, 0), Some(Path::new("/fixture/new")));
        session.emulator.process(b"\x1b]133;C\x07unclosed output");
        session.set_scrollback_limit(1);
        session.emulator.process(b"\x1b]133;D;0\x07");
        assert!(session.commands().records().is_empty(), "a discarded in-flight capture cannot republish");
    }
}

#[cfg(test)]
mod sync_tests;
