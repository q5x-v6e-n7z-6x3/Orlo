//! VT100 screen state and replies to terminal device queries.

use crate::configuration::TerminalConfiguration;
pub(crate) mod provenance;
pub use provenance::Screen;
pub use vt100::Color;

pub struct Emulator {
    parser: provenance::Parser,
    commands: crate::commands::Commands,
    osc: Vec<u8>,
    ignored_string: bool,
    /// The rest of an OSC longer than any Orlo reads, dropped up to its end.
    skipping_osc: bool,
    pub(crate) sync_ready: bool,
    pub(crate) execution_generation: u64,
    pub(crate) directory_generation: u64,
    pub(crate) sync_refused: bool,
}

impl Emulator {
    pub fn new(configuration: TerminalConfiguration) -> Self {
        Self::with_size(configuration.rows, configuration.cols, configuration.scrollback)
    }

    pub fn with_size(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self {
            parser: provenance::Parser::new(rows, cols, scrollback),
            commands: crate::commands::Commands::new(TerminalConfiguration { rows, cols, scrollback }),
            osc: Vec::new(),
            ignored_string: false,
            skipping_osc: false,
            sync_ready: false,
            execution_generation: 0,
            directory_generation: 0,
            sync_refused: false,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            if self.skipping_osc {
                // up to its terminator (BEL, CAN, SUB) or an ESC (ST, or what comes next)
                match bytes[i..].iter().position(|&b| matches!(b, 7 | 0x18 | 0x1a | 27)) {
                    None => return,
                    Some(n) => i += n + usize::from(bytes[i + n] != 27),
                }
                self.skipping_osc = false;
                continue;
            }
            if self.osc.is_empty() {
                let count = bytes[i..]
                    .iter()
                    .position(|&b| b == 27 || self.ignored_string && matches!(b, 0x18 | 0x1a))
                    .unwrap_or(bytes.len() - i);
                self.feed(&bytes[i..i + count]);
                i += count;
                if i < bytes.len() {
                    if bytes[i] == 27 {
                        self.osc.push(27);
                    } else {
                        self.feed(&bytes[i..i + 1]);
                        self.ignored_string = false;
                    }
                    i += 1;
                }
            } else if self.osc.len() == 1 && (bytes[i] != b']' || self.ignored_string) {
                if bytes[i] == b'c' {
                    self.sync_ready = false;
                    self.commands.abort();
                    self.ignored_string = false;
                }
                if self.ignored_string && bytes[i] == b'\\' {
                    self.ignored_string = false;
                } else if matches!(bytes[i], b'P' | b'_' | b'^' | b'X') {
                    self.ignored_string = true;
                }
                self.feed(&[27]);
                self.osc.clear();
            } else {
                self.osc.push(bytes[i]);
                i += 1;
                let terminated = self.osc.last() == Some(&7) || self.osc.ends_with(b"\x1b\\");
                let cancelled = matches!(self.osc.last(), Some(0x18 | 0x1a))
                    || self.osc.len() > 2 && self.osc[self.osc.len() - 2] == 27 && self.osc.last() != Some(&b'\\');
                if terminated || cancelled || self.osc.len() >= 8192 {
                    let too_long = !terminated && !cancelled;
                    let osc = std::mem::take(&mut self.osc);
                    if cancelled && osc.ends_with(b"\x1bc") {
                        self.sync_ready = false;
                        self.commands.abort();
                        self.ignored_string = false;
                    }
                    if terminated {
                        let end = osc.len() - if osc.last() == Some(&7) { 1 } else { 2 };
                        match &osc[2..end] {
                            b"133;OrloReady" => self.sync_ready = true,
                            b"133;OrloSyncRefused" => {
                                self.sync_ready = false;
                                self.sync_refused = true;
                            }
                            b"133;C" => {
                                self.sync_ready = false;
                                self.execution_generation += 1;
                            }
                            b"133;A" => self.sync_ready = false,
                            payload
                                if payload
                                    .strip_prefix(b"7;")
                                    .and_then(|url| std::str::from_utf8(url).ok())
                                    .and_then(|url| self.parser.callbacks().directory(url))
                                    .is_some() =>
                            {
                                self.directory_generation += 1
                            }
                            _ => {}
                        }
                        self.commands.marker(&osc[2..end], self.working_directory().map(std::path::Path::to_path_buf));
                    }
                    self.feed(&osc);
                    // ended in every parser too: one that never ends would grow their buffers
                    // without bound (vte keeps an OSC whole)
                    if too_long {
                        self.feed(&[0x18]);
                        self.skipping_osc = true;
                    }
                }
            }
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.commands.process(bytes);
    }
    pub fn commands(&self) -> &crate::commands::Commands {
        &self.commands
    }

    pub fn exited(&mut self, status: u32) {
        self.sync_ready = false;
        self.parser.reset_mouse();
        self.commands.exited(status);
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
        self.commands.resize(rows, cols);
    }

    pub fn screen(&self) -> &Screen {
        self.parser.screen()
    }

    pub fn screen_mut(&mut self) -> &mut Screen {
        self.parser.screen_mut()
    }

    /// Change retention in place; old snapshots belong to the preceding layout epoch.
    pub fn set_scrollback_limit(&mut self, rows: usize) -> bool {
        if !self.parser.screen_mut().set_scrollback_limit(rows) {
            return false;
        }
        self.commands.set_scrollback_limit(rows);
        true
    }

    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.parser.callbacks_mut().0)
    }

    pub fn set_directory(&mut self, path: std::path::PathBuf) {
        self.parser.set_directory(path);
    }

    pub(crate) fn expect_directory(&mut self, path: Option<std::path::PathBuf>) {
        self.parser.callbacks_mut().2 = path;
    }

    pub fn working_directory(&self) -> Option<&std::path::Path> {
        self.parser.callbacks().1.as_deref()
    }
}

#[derive(Default)]
struct Replies(Vec<u8>, Option<std::path::PathBuf>, Option<std::path::PathBuf>);

impl Replies {
    fn directory(&self, url: &str) -> Option<std::path::PathBuf> {
        crate::links::local_file_url(url)
            .or_else(|| crate::links::directory_url(url).filter(|path| self.2.as_ref() == Some(path)))
    }
}

impl vt100::Callbacks for Replies {
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if let [b"7", url] = params {
            if let Some(path) = std::str::from_utf8(url).ok().and_then(|url| self.directory(url)) {
                self.1 = Some(path);
            }
        }
    }
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        first: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        let value = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let reply = match (first, command, value) {
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                format!("\x1b[{};{}R", row + 1, col + 1)
            }
            (None, 'n', 5) => "\x1b[0n".into(),
            (None, 'c', 0) => "\x1b[?1;2c".into(),
            (Some(b'>'), 'c', 0) => "\x1b[>0;1;0c".into(),
            _ => return,
        };
        if self.0.len() < 8192 {
            self.0.extend_from_slice(reply.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_osc_that_never_ends_is_cut_off_and_its_rest_dropped() {
        let mut e = Emulator::with_size(4, 40, 10);
        e.process(b"\x1b]0;");
        for _ in 0..2_000 {
            e.process(&[b'a'; 4096]);
        }
        e.process(b"\x07visible");
        let shown = e.screen().contents();
        assert!(shown.contains("visible") && !shown.contains('a'), "{shown:?}");
    }

    #[test]
    fn ansi_screen_keeps_split_unicode_and_restores_primary_screen() {
        let mut parser = Emulator::with_size(3, 20, 100);
        let bytes = "\x1b[31m中e\u{301}".as_bytes();
        for byte in bytes {
            parser.process(&[*byte]);
        }
        assert_eq!(parser.screen().contents(), "中e\u{301}");
        assert!(parser.screen().cell(0, 0).unwrap().is_wide());
        assert_eq!(parser.screen().cell(0, 0).unwrap().fgcolor(), vt100::Color::Idx(1));
        parser.process(b"\x1b[?1049hfullscreen\x1b[?1h\x1b[?2004h");
        assert!(parser.screen().alternate_screen());
        assert!(parser.screen().application_cursor());
        assert!(parser.screen().bracketed_paste());
        parser.process(b"\x1b[?1049l");
        assert_eq!(parser.screen().contents(), "中e\u{301}");
        parser.process(b"\r\n1\r\n2\r\n3\r\n4");
        parser.screen_mut().set_scrollback(2);
        assert_eq!(parser.screen().scrollback(), 2);
        assert!(parser.screen().contents().contains("1"));
    }

    #[test]
    fn terminal_answers_cursor_and_device_queries() {
        let mut parser = Emulator::with_size(24, 80, 0);
        parser.process(b"\x1b[3;5H\x1b[6n\x1b[5n\x1b[c");
        assert_eq!(parser.take_replies(), b"\x1b[3;5R\x1b[0n\x1b[?1;2c");
    }

    #[test]
    fn native_scrollback_capacity_trims_primary_during_alternate_and_preserves_decoder_modes() {
        let mut parser = vt100::Parser::new(2, 30, 8);
        parser.process(b"old0\r\nold1\r\nnew2\r\nnew3\r\nnew4\x1b[31m\x1b[?1003h\x1b[?1006h\x1b[?2004h");
        let primary = parser.screen().contents();
        let cursor = parser.screen().cursor_position();
        parser.process(b"\x1b[?1049hALT");
        let alt_cursor = parser.screen().cursor_position();
        let unicode = "中".as_bytes();
        parser.process(&unicode[..1]);
        parser.screen_mut().set_scrollback_len(1);
        parser.process(&unicode[1..]);
        assert!(parser.screen().alternate_screen());
        assert_eq!(parser.screen().contents(), "ALT中");
        assert_eq!(parser.screen().cursor_position(), (alt_cursor.0, alt_cursor.1 + 2));
        assert_eq!(parser.screen().cell(0, 3).unwrap().fgcolor(), vt100::Color::Idx(1));
        assert_eq!(parser.screen().mouse_protocol_mode(), vt100::MouseProtocolMode::AnyMotion);
        assert!(parser.screen().bracketed_paste());
        parser.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(parser.screen().scrollback(), 0);
        parser.process(b"\x1b[?1049l");
        assert_eq!(parser.screen().contents(), primary);
        assert_eq!(parser.screen().cursor_position(), cursor);
        parser.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(parser.screen().scrollback(), 1, "lower capacity must immediately drop oldest primary rows");
        assert!(parser.screen().contents().starts_with("new2"));
        parser.screen_mut().set_scrollback_len(4);
        assert_eq!(parser.screen().scrollback(), 1, "growth cannot resurrect old rows");
        parser.screen_mut().set_scrollback_len(0);
        assert_eq!(parser.screen().scrollback(), 0);
        assert_eq!(parser.screen().contents(), primary);
        parser.process(b"\r\nnext\r\nlast");
        parser.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(parser.screen().scrollback(), 0, "new output must honor zero history");
    }

    #[test]
    fn paired_retention_preserves_partial_unicode_and_directory_while_alternate_is_active() {
        let mut e = Emulator::with_size(2, 30, 8);
        e.set_directory("/fixture/old".into());
        e.process(b"old0.rs\r\nold1.rs\r\n\x1b]7;file:///fixture/new\x07new2.rs\r\nnew3.rs\r\nnew4.rs");
        let old = e.screen().clone();
        e.process(b"\x1b[?1049hALT");
        let unicode = "中".as_bytes();
        e.process(&unicode[..1]);
        assert!(e.set_scrollback_limit(1));
        e.process(&unicode[1..]);
        assert_eq!(e.screen().contents(), "ALT中");
        assert_eq!(e.screen().directory_at(0, 3), Some(std::path::Path::new("/fixture/new")));
        e.process(b"\x1b[?1049l");
        assert!(!e.screen().same_layout(&old));
        e.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(e.screen().scrollback(), 1);
        assert!(e.screen().contents().starts_with("new2.rs"));
        assert_eq!(e.screen().directory_at(0, 1), Some(std::path::Path::new("/fixture/new")));
        assert!(e.set_scrollback_limit(0));
        assert_eq!(e.screen().scrollback(), 0);
        e.resize(3, 40);
        e.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(e.screen().scrollback(), 0);
    }
}
