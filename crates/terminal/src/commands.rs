//! Bounded OSC133 command records, independent of the live terminal's scroll position.

use crate::{Screen, TerminalConfiguration};

type DirectorySpan = (std::ops::Range<usize>, Option<std::sync::Arc<std::path::PathBuf>>);

const MAX_TEXT: usize = 1024 * 1024;

#[derive(Clone)]
pub struct Command {
    pub id: u64,
    pub command: String,
    pub output: String,
    pub status: Option<i32>,
    directories: Vec<DirectorySpan>,
}

impl Command {
    fn provenance_bytes(&self) -> usize {
        self.directories
            .iter()
            .map(|(_, path)| std::mem::size_of::<DirectorySpan>() + path.as_ref().map_or(0, |p| p.as_os_str().len()))
            .sum()
    }

    /// Reflow the bounded plain review output while restoring each retained directory span.
    pub fn review_screen(&self, rows: u16, cols: u16) -> Screen {
        let mut parser = crate::emulation::provenance::Parser::new(rows, cols, 10_000);
        for (range, directory) in &self.directories {
            if let Some(directory) = directory {
                parser.set_directory(directory.as_ref().clone());
            } else {
                parser.clear_directory();
            }
            parser.process(self.output[range.clone()].replace('\n', "\r\n").as_bytes());
        }
        parser.screen().clone()
    }
}

/// Keep the last `max` bytes of `text` (from a character boundary), and its directories.
fn trim_head(text: &mut String, directories: &mut Vec<DirectorySpan>, partial_head: &mut bool, max: usize) {
    let mut from = text.len() - max;
    while !text.is_char_boundary(from) {
        from += 1;
    }
    *partial_head = text.as_bytes()[from - 1] != b'\n';
    text.drain(..from);
    directories.retain_mut(|(range, _)| {
        range.start = range.start.saturating_sub(from);
        range.end = range.end.saturating_sub(from);
        range.start < range.end
    });
}

fn retained_output(mut screen: Screen, max: usize) -> (String, Vec<DirectorySpan>) {
    screen.set_scrollback(usize::MAX);
    let retained = screen.scrollback();
    let (rows, _) = screen.size();
    let mut incomplete = screen.incomplete_start();
    let mut partial_head = false;
    let mut text = String::new();
    let mut directories: Vec<DirectorySpan> = Vec::new();
    for index in 0..retained + rows as usize {
        screen.set_scrollback(retained.saturating_sub(index));
        let row = index.saturating_sub(retained) as u16;
        let cols = screen.row_columns(row);
        let line = screen.rows(0, cols).nth(row as usize).unwrap();
        let end = text.len() + line.len();
        for col in 0..cols {
            let cell = screen.cell(row, col).unwrap();
            if cell.is_wide_continuation() || text.len() >= end {
                continue;
            }
            let start = text.len();
            text.push_str(if cell.contents().is_empty() { " " } else { cell.contents() });
            let directory = if incomplete { None } else { screen.directory(row, col).cloned() };
            if let Some((range, _)) = directories.last_mut().filter(|(_, previous)| *previous == directory) {
                range.end = text.len();
            } else {
                directories.push((start..text.len(), directory));
            }
        }
        if !screen.row_wrapped(row) {
            let start = text.len();
            text.push('\n');
            incomplete = false;
            if let Some((range, _)) = directories.last_mut() {
                range.end = text.len();
            } else {
                directories.push((start..text.len(), None));
            }
        }
        // twice the cap before trimming, so each byte is shifted out once, not once a row
        if text.len() > max.saturating_mul(2) {
            trim_head(&mut text, &mut directories, &mut partial_head, max);
        }
    }
    if text.len() > max {
        trim_head(&mut text, &mut directories, &mut partial_head, max);
    }
    text.truncate(text.trim_end_matches('\n').len());
    directories.retain_mut(|(range, _)| {
        range.end = range.end.min(text.len());
        range.start < range.end
    });
    if partial_head {
        let end = text.find('\n').map_or(text.len(), |at| at + 1);
        directories = directories
            .into_iter()
            .flat_map(|(range, path)| {
                let mut spans = Vec::new();
                if range.start < end {
                    spans.push((range.start..range.end.min(end), None));
                }
                if range.end > end {
                    spans.push((range.start.max(end)..range.end, path));
                }
                spans
            })
            .collect();
    }
    (text, directories)
}

pub struct Commands {
    records: Vec<Command>,
    capture: Option<crate::emulation::provenance::Parser>,
    executing: bool,
    command: String,
    serial: u64,
    configuration: TerminalConfiguration,
}

fn retain_tail(text: &mut String, max: usize) {
    if text.len() > max {
        let mut from = text.len() - max;
        while !text.is_char_boundary(from) {
            from += 1;
        }
        text.drain(..from);
    }
}

pub fn retained_text(mut screen: Screen, max: usize) -> String {
    screen.set_scrollback(usize::MAX);
    let retained = screen.scrollback();
    let (rows, cols) = screen.size();
    let mut base = 0;
    let mut text = String::new();
    while base < retained + rows as usize {
        screen.set_scrollback(retained.saturating_sub(base));
        let count = if base < retained { (retained - base).min(rows as usize) } else { rows as usize };
        for (row, line) in screen.rows(0, cols).take(count).enumerate() {
            text.push_str(&line);
            if !screen.row_wrapped(row as u16) {
                text.push('\n');
            }
        }
        retain_tail(&mut text, max);
        base += count;
    }
    text.truncate(text.trim_end_matches('\n').len());
    text
}

impl Commands {
    pub fn new(configuration: TerminalConfiguration) -> Self {
        Self { records: Vec::new(), capture: None, executing: false, command: String::new(), serial: 0, configuration }
    }
    pub fn records(&self) -> &[Command] {
        &self.records
    }
    pub fn executing(&self) -> bool {
        self.executing
    }
    pub(crate) fn process(&mut self, bytes: &[u8]) {
        if let Some(capture) = &mut self.capture {
            capture.process(bytes);
        }
    }
    pub(crate) fn resize(&mut self, rows: u16, cols: u16) {
        self.configuration.rows = rows;
        self.configuration.cols = cols;
        if let Some(capture) = &mut self.capture {
            capture.screen_mut().set_size(rows, cols);
        }
    }

    pub(crate) fn set_scrollback_limit(&mut self, rows: usize) {
        if rows < self.configuration.scrollback {
            // Archived and in-flight ranges can otherwise keep deliberately discarded output.
            self.records.clear();
            self.abort();
        } else if let Some(capture) = &mut self.capture {
            capture.screen_mut().set_scrollback_limit(rows);
        }
        self.configuration.scrollback = rows;
    }

    pub(crate) fn abort(&mut self) {
        self.capture = None;
        self.command.clear();
        self.executing = false;
    }

    pub(crate) fn exited(&mut self, status: u32) {
        if self.executing {
            self.finish(i32::try_from(status).ok());
        }
    }
    fn begin_capture(&mut self, cwd: Option<std::path::PathBuf>) {
        let c = self.configuration;
        let mut parser = crate::emulation::provenance::Parser::new(c.rows, c.cols, c.scrollback);
        if let Some(cwd) = cwd {
            parser.set_directory(cwd);
        }
        self.capture = Some(parser);
    }
    pub(crate) fn marker(&mut self, payload: &[u8], cwd: Option<std::path::PathBuf>) {
        let Some(payload) = payload.strip_prefix(b"133;") else { return };
        let mut args = payload.split(|&b| b == b';');
        match args.next() {
            Some(b"A") => {
                if self.executing {
                    self.finish(None);
                }
                self.capture = None;
            }
            Some(b"B") => {
                if self.executing {
                    self.finish(None);
                }
                self.command.clear();
                self.begin_capture(cwd);
            }
            Some(b"C") => {
                if self.executing {
                    return;
                }
                self.command =
                    self.capture.take().map_or_else(String::new, |c| retained_text(c.screen().clone(), 16_384));
                self.executing = true;
                self.begin_capture(cwd);
            }
            Some(b"D") if self.executing => {
                let status = args.next().and_then(|s| std::str::from_utf8(s).ok()).and_then(|s| s.parse().ok());
                self.finish(status);
            }
            _ => {}
        }
    }
    fn finish(&mut self, status: Option<i32>) {
        let (output, directories) = self
            .capture
            .take()
            .map_or_else(Default::default, |c| retained_output(c.screen().clone(), MAX_TEXT - self.command.len()));
        self.serial += 1;
        self.records.push(Command {
            id: self.serial,
            command: std::mem::take(&mut self.command),
            output,
            status,
            directories,
        });
        self.executing = false;
        while self.records.len() > 32
            || self.records.iter().map(|c| c.command.len() + c.output.len()).sum::<usize>() > MAX_TEXT
        {
            self.records.remove(0);
        }
        // Directory churn cannot turn the bounded plain-text archive into an unbounded path
        // archive. Expire old provenance only; absolute/web links and copied output survive.
        let mut bytes: usize = self.records.iter().map(Command::provenance_bytes).sum();
        for record in &mut self.records {
            if bytes <= MAX_TEXT {
                break;
            }
            bytes -= record.provenance_bytes();
            record.directories = vec![(0..record.output.len(), None)];
            bytes += record.provenance_bytes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{retained_output, MAX_TEXT};
    use crate::Emulator;

    #[test]
    fn retained_output_keeps_the_last_max_bytes_and_their_directories() {
        let mut e = Emulator::with_size(4, 40, 5_000);
        e.set_directory("/proj".into());
        for i in 0..4_000 {
            e.process(format!("row {i:04} 中 src/a.rs:1:1\r\n").as_bytes());
        }
        let full = retained_output(e.screen().clone(), usize::MAX).0;
        for max in [1, 7, 1_000, 33_333] {
            let (text, directories) = retained_output(e.screen().clone(), max);
            // the cap counts the blank rows' line breaks (4 rows), then the text drops them;
            // a cut inside 中 moves up to its next character
            assert!(full.ends_with(&text) && text.len() <= max, "max {max}");
            assert!(text.len() + 4 + 3 >= max, "max {max}: {}", text.len());
            // a row cut in two has no directory left for its tail, the rows after it keep theirs
            let at = full.len() - text.len();
            let partial = at > 0 && full.as_bytes()[at - 1] != b'\n';
            let first = if partial { text.find('\n').map_or(text.len(), |at| at + 1) } else { 0 };
            assert!(directories.iter().all(|(r, d)| d.is_none() || r.start >= first), "max {max}");
            let proj = std::path::PathBuf::from("/proj");
            assert!(max < 1_000 || directories.iter().any(|(_, d)| d.as_deref() == Some(&proj)), "max {max}");
        }
    }

    #[test]
    #[ignore = "release performance gate"]
    fn bench_retained_output_past_its_cap() {
        let mut e = Emulator::with_size(24, 200, 10_000);
        for _ in 0..10_100 {
            e.process(format!("{}\r\n", "x".repeat(199)).as_bytes());
        }
        let start = std::time::Instant::now();
        let (text, _) = retained_output(e.screen().clone(), MAX_TEXT);
        let elapsed = start.elapsed();
        eprintln!("retained output of 2 MB past its 1 MB cap: {elapsed:?}");
        assert!(text.len() <= MAX_TEXT);
        assert!(elapsed < std::time::Duration::from_millis(150));
    }

    #[test]
    fn success_failure_interrupt_and_optional_status_exclude_prompts_and_input() {
        let mut e = Emulator::with_size(3, 80, 10);
        for (number, end, status) in [
            (0, "D;0", Some(0)),
            (1, "D;2", Some(2)),
            (2, "D;130", Some(130)),
            (3, "D", None),
            (4, "D;invalid", None),
            (5, "A", None),
        ] {
            let bytes = format!("\x1b]133;A\x07prompt> \x1b]133;B\x07command{number}\r\n\x1b]133;C\x07out{number}\r\n\x1b]133;{end}\x07");
            for byte in bytes.bytes() {
                e.process(&[byte]);
            }
            let record = e.commands().records().last().unwrap();
            assert_eq!(record.command, format!("command{number}"));
            assert_eq!(record.output, format!("out{number}"));
            assert_eq!(record.status, status);
            assert!(!e.commands().executing());
        }
        assert_eq!(e.commands().records().len(), 6);
    }

    #[test]
    fn partial_malformed_and_unmarked_text_do_not_infer_commands() {
        let mut e = Emulator::with_size(3, 80, 10);
        e.process(b"prompt> false\r\nlooks like an error\r\nprompt> ");
        assert!(e.commands().records().is_empty());
        e.process(b"\x1b]133;C");
        assert!(!e.commands().executing(), "only a terminated marker can begin output");
        e.process(b"\x18\x1b]133;unknown\x07\x1b]133;D;0\x07");
        assert!(e.commands().records().is_empty());
        e.process(b"\x1b]133;C\x07kept\x1b]133;D;");
        assert!(e.commands().records().is_empty());
        e.process(b"9\x1b\\");
        assert_eq!(e.commands().records()[0].status, Some(9));
        assert_eq!(e.commands().records()[0].output, "kept");
        e.process(b"\x1b]133;C\x07interrupted");
        e.exited(130);
        e.exited(130);
        assert_eq!(e.commands().records().len(), 2, "exit finalizes an executing range once");
        assert_eq!(e.commands().records()[1].status, Some(130));
    }

    #[test]
    fn screen_clear_reset_and_eviction_keep_only_bounded_completed_output() {
        let mut e = Emulator::with_size(2, 30, 1);
        e.process(b"\x1b]133;B\x07clear\x1b]133;C\x07before\x1b[2J\x1b[Hafter\x1b]133;D;0\x07");
        assert_eq!(e.commands().records()[0].output, "after");
        let first_id = e.commands().records()[0].id;
        e.process(b"\x1b]133;C\x07discard unfinished\x1bc\x1b]133;D;0\x07");
        assert_eq!(e.commands().records().len(), 1, "RIS aborts unfinished capture");
        assert_eq!(e.commands().records()[0].output, "after", "completed archives remain independently bounded");
        e.process(b"\x1b]133;C\x07old0\r\nold1\r\nkept2\r\nkept3\r\nkept4\x1b]133;D;0\x07");
        assert_eq!(e.commands().records()[1].output, "kept2\nkept3\nkept4");
        assert!(e.commands().records()[1].id > first_id);
        e.set_scrollback_limit(0);
        assert!(e.commands().records().is_empty());
        e.process(b"\x1b]133;C\x07zero0\r\nzero1\r\nzero2\x1b]133;D\x07");
        assert_eq!(e.commands().records()[0].output, "zero1\nzero2");
        assert!(e.commands().records()[0].id > first_id + 1);
    }

    #[test]
    fn command_and_output_byte_limits_evict_utf8_on_character_boundaries() {
        let mut e = Emulator::with_size(3, 512, 5000);
        e.process(b"\x1b]133;B\x07");
        e.process("中".repeat(6000).as_bytes());
        e.process(b"\x1b]133;C\x07");
        let line = "中".repeat(200);
        for n in 0..2100 {
            e.process(format!("{n:04}{line}\r\n").as_bytes());
        }
        e.process(b"\x1b]133;D;0\x07");
        let record = &e.commands().records()[0];
        assert!(!record.command.is_empty());
        assert!(record.command.len() <= 16_384);
        assert!(record.command.len() + record.output.len() <= super::MAX_TEXT);
        assert!(!record.output.contains("0000"));
        assert!(record.output.ends_with(&format!("2099{line}")));
        assert_eq!(record.status, Some(0));
    }

    #[test]
    fn directory_churn_expires_review_provenance_without_losing_copied_output_or_urls() {
        let mut e = Emulator::with_size(3, 40, 1000);
        e.process(b"\x1b]133;C\x07");
        for n in 0..240 {
            e.process(format!("\x1b]7;file:///{}-{n}\x07a.rs https://example.com\r\n", "x".repeat(5000)).as_bytes());
        }
        e.process(b"\x1b]133;D;0\x07");
        let command = &e.commands().records()[0];
        assert_eq!(command.output.lines().count(), 240);
        let mut screen = command.review_screen(3, 40);
        screen.set_scrollback(usize::MAX);
        assert_eq!(crate::links::link_at(&screen, (0, 1)), None);
        assert_eq!(crate::links::link_at(&screen, (0, 8)), Some(crate::links::Link::Url("https://example.com".into())));
    }

    #[test]
    fn split_shell_markers_capture_unicode_output_exit_status_and_bound_retention() {
        let mut emulator = Emulator::with_size(3, 20, 10);
        for byte in
            "\x1b]133;A\x07prompt> \x1b]133;B\x07echo 中\r\n\x1b]133;C\x07\x1b[31m中😀\r\n\x1b]133;D;7\x1b\\".bytes()
        {
            emulator.process(&[byte]);
        }
        let command = &emulator.commands().records()[0];
        assert_eq!(command.command, "echo 中");
        assert_eq!(command.output, "中😀");
        assert_eq!(command.status, Some(7));
        for _ in 0..100 {
            emulator.process(b"\x1b]133;B\x07true\r\n\x1b]133;C\x07output\r\n\x1b]133;D;0\x07");
        }
        assert_eq!(emulator.commands().records().len(), 32);
        assert!(emulator.commands().records()[0].id > 1);
    }

    #[test]
    fn missing_or_bad_boundaries_do_not_invent_successful_commands() {
        let mut emulator = Emulator::with_size(3, 20, 2);
        emulator.process(b"\x1b]133;D;0\x07noise\x1b]133;C\x07output\x1b]133;D;invalid\x07");
        assert_eq!(emulator.commands().records().len(), 1);
        assert_eq!(emulator.commands().records()[0].status, None);
    }

    #[test]
    fn cancelled_and_nested_control_strings_do_not_capture_fake_commands_or_delay_text() {
        let mut e = Emulator::with_size(3, 80, 10);
        e.process(b"\x1b]unclosed\x18visible\x1b]133;C\x07real\x1b]133;D;0\x07");
        assert!(e.screen().contents().contains("visible"));
        assert_eq!(e.commands().records()[0].output, "real");
        let mut e = Emulator::with_size(3, 80, 10);
        e.process(b"\x1bPignored\x1b]133;C\x07fake\x1b]133;D;0\x07\x1b\\visible");
        assert!(e.commands().records().is_empty());
        e.process(b"\x1b]133;C\x07output\x1bc\x1b]133;D;0\x07");
        assert!(e.commands().records().is_empty());
    }
}
