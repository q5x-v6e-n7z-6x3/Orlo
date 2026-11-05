//! A parallel vt100 grid carries directory IDs through the engine's own layout operations.
//! Its colors are private metadata, never output attributes. No terminal layout is reimplemented.
use std::{
    collections::{BTreeMap, VecDeque},
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Cheap identity for admitting asynchronous snapshot work without cloning a grid twice.
#[derive(Clone)]
pub struct ScreenIdentity {
    source: Arc<()>,
    layout_revision: u64,
}
impl ScreenIdentity {
    pub fn matches(&self, screen: &Screen) -> bool {
        Arc::ptr_eq(&self.source, &screen.identity) && self.layout_revision == screen.layout_revision
    }
}

#[derive(Clone)]
pub struct Screen {
    identity: Arc<()>,
    revision: u64,
    layout_revision: u64,
    text: vt100::Screen,
    provenance: vt100::Screen,
    directories: BTreeMap<u32, Arc<PathBuf>>,
    capacity: usize,
}

impl Deref for Screen {
    type Target = vt100::Screen;
    fn deref(&self) -> &Self::Target {
        &self.text
    }
}
impl Screen {
    pub fn identity(&self) -> ScreenIdentity {
        ScreenIdentity { source: self.identity.clone(), layout_revision: self.layout_revision }
    }
    /// Snapshot coordinates belong to one session, geometry and screen-mode epoch.
    pub fn same_layout(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity) && self.layout_revision == other.layout_revision
    }
    pub fn same_contents(&self, other: &Self) -> bool {
        self.same_layout(other) && self.revision == other.revision
    }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        self.layout_revision += 1;
        self.revision += 1;
        // vt100 preserves complete rows, but clears visible wrap flags and clips narrower
        // rows. Invalidate just the affected print spans, in both native screen grids.
        // the grid itself goes through the probe and back: a copy would clone the whole
        // scrollback on every frame of a window resize
        let mut probe = vt100::Parser::new(1, 1, 0);
        std::mem::swap(probe.screen_mut(), &mut self.provenance);
        let (alternate, offset) = (probe.screen().alternate_screen(), probe.screen().scrollback());
        for mode in [b"\x1b[?47l", b"\x1b[?47h"] {
            probe.process(mode);
            probe.screen_mut().set_scrollback(0);
            let screen = probe.screen();
            let (old_rows, old_cols) = screen.size();
            for row in 0..old_rows {
                let clipped = (cols..old_cols).any(|col| screen.cell(row, col).is_some_and(|c| c.has_contents()));
                if screen.row_wrapped(row) || row > 0 && screen.row_wrapped(row - 1) || clipped {
                    for col in 0..old_cols {
                        if let Some(cell) = screen.cell(row, col) {
                            self.directories.remove(&color_id(cell.fgcolor()));
                        }
                    }
                }
            }
        }
        probe.process(if alternate { b"\x1b[?47h" } else { b"\x1b[?47l" });
        probe.screen_mut().set_scrollback(offset);
        std::mem::swap(probe.screen_mut(), &mut self.provenance);
        self.text.set_size(rows, cols);
        self.provenance.set_size(rows, cols);
    }
    pub fn set_scrollback(&mut self, rows: usize) {
        self.text.set_scrollback(rows);
        self.provenance.set_scrollback(rows);
    }
    pub fn scrollback_limit(&self) -> usize {
        self.capacity
    }

    pub(crate) fn set_scrollback_limit(&mut self, rows: usize) -> bool {
        if self.capacity == rows {
            return false;
        }
        self.text.set_scrollback_len(rows);
        self.provenance.set_scrollback_len(rows);
        self.capacity = rows;
        self.layout_revision += 1;
        self.revision += 1;
        let limit = rows + self.size().0 as usize + 4096;
        while self.directories.len() > limit {
            self.directories.pop_first();
        }
        true
    }
    pub(crate) fn row_columns(&self, row: u16) -> u16 {
        let (mut low, mut high) = (0u32, u16::MAX as u32);
        while low < high {
            let middle = (low + high) / 2;
            if self.text.cell(row, middle as u16).is_some() {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low as u16
    }
    /// A private background marker travels with each print span's first cell. Its absence
    /// on the oldest row proves that retention removed the beginning of that span.
    pub(crate) fn incomplete_start(&self) -> bool {
        let mut screen = self.provenance.clone();
        screen.set_scrollback(usize::MAX);
        (0..u16::MAX)
            .map_while(|col| screen.cell(0, col))
            .find(|cell| cell.has_contents())
            .is_some_and(|cell| cell.bgcolor() != vt100::Color::Idx(1))
    }
    pub fn directory_at(&self, row: u16, col: u16) -> Option<&Path> {
        self.directory(row, col).map(|p| p.as_path())
    }
    pub(crate) fn directory(&self, row: u16, col: u16) -> Option<&Arc<PathBuf>> {
        let text = self.text.cell(row, col)?;
        let provenance = self.provenance.cell(row, col)?;
        // A future engine discrepancy must fail closed, never resolve against unrelated output.
        if text.contents() != provenance.contents() {
            return None;
        }
        let vt100::Color::Rgb(r, g, b) = provenance.fgcolor() else { return None };
        self.directories.get(&((r as u32) << 16 | (g as u32) << 8 | b as u32))
    }
}

pub(crate) struct Parser {
    parser: vt100::Parser<super::Replies>,
    layout: vt100::Parser,
    decoder: vte::Parser,
    screen: Screen,
    serial: u32,
    id: u32,
    directory: Option<Arc<PathBuf>>,
    start: bool,
    directory_epochs: VecDeque<Arc<PathBuf>>,
}
impl Parser {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        let parser = vt100::Parser::new_with_callbacks(rows, cols, scrollback, super::Replies::default());
        let layout = vt100::Parser::new(rows, cols, scrollback);
        let screen = Screen {
            identity: Arc::new(()),
            revision: 0,
            layout_revision: 0,
            text: parser.screen().clone(),
            provenance: layout.screen().clone(),
            directories: BTreeMap::new(),
            capacity: scrollback,
        };
        Self {
            parser,
            layout,
            decoder: vte::Parser::new(),
            screen,
            serial: 0,
            id: 0,
            directory: None,
            start: true,
            directory_epochs: VecDeque::new(),
        }
    }
    pub fn screen(&self) -> &Screen {
        &self.screen
    }
    pub fn screen_mut(&mut self) -> &mut Screen {
        &mut self.screen
    }
    pub(super) fn reset_mouse(&mut self) {
        // A fresh decoder cannot mistake cleanup for an unfinished escape from the child.
        let mut parser = vt100::Parser::new(1, 1, 0);
        std::mem::swap(parser.screen_mut(), &mut self.screen.text);
        parser.process(b"\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l");
        std::mem::swap(parser.screen_mut(), &mut self.screen.text);
    }
    pub(super) fn callbacks(&self) -> &super::Replies {
        self.parser.callbacks()
    }
    pub(super) fn callbacks_mut(&mut self) -> &mut super::Replies {
        self.parser.callbacks_mut()
    }
    pub fn set_directory(&mut self, path: PathBuf) {
        self.parser.callbacks_mut().1 = Some(path.clone());
        if self.directory.as_deref() == Some(&path) {
            return;
        }
        let path = Arc::new(path);
        self.directory_epochs.push_back(path.clone());
        while self.directory_epochs.len() > 4096 {
            let expired = self.directory_epochs.pop_front().unwrap();
            self.screen.directories.retain(|_, path| !Arc::ptr_eq(path, &expired));
        }
        self.directory = Some(path);
        self.start = true;
        let limit = self.screen.capacity + self.screen.size().0 as usize + 4096;
        self.id = next_id(&mut self.serial, &self.directory, &mut self.screen.directories, limit);
    }
    pub fn clear_directory(&mut self) {
        self.id = 0;
        self.directory = None;
        self.start = true;
        self.parser.callbacks_mut().1 = None;
    }
    pub fn process(&mut self, bytes: &[u8]) {
        self.screen.revision += 1;
        if !self.screen.directories.contains_key(&self.id) {
            self.id = 0;
        }
        let limit = self.screen.capacity + self.screen.size().0 as usize + 4096;
        std::mem::swap(self.parser.screen_mut(), &mut self.screen.text);
        std::mem::swap(self.layout.screen_mut(), &mut self.screen.provenance);
        let old = self.parser.callbacks().1.clone();
        self.parser.process(bytes);
        self.decoder.advance(
            &mut Layout {
                parser: &mut self.layout,
                layout_revision: &mut self.screen.layout_revision,
                id: &mut self.id,
                serial: &mut self.serial,
                directory: &self.directory,
                directories: &mut self.screen.directories,
                limit,
                start: &mut self.start,
            },
            bytes,
        );
        std::mem::swap(self.parser.screen_mut(), &mut self.screen.text);
        std::mem::swap(self.layout.screen_mut(), &mut self.screen.provenance);
        if self.parser.callbacks().1 != old {
            let path = self.parser.callbacks_mut().1.take().unwrap();
            self.set_directory(path);
        }
    }
}

fn color_id(color: vt100::Color) -> u32 {
    match color {
        vt100::Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        _ => 0,
    }
}

fn next_id(
    serial: &mut u32,
    directory: &Option<Arc<PathBuf>>,
    directories: &mut BTreeMap<u32, Arc<PathBuf>>,
    limit: usize,
) -> u32 {
    let Some(directory) = directory else { return 0 };
    // IDs never recycle, including terminal resets. Exhaustion fails closed.
    *serial = serial.saturating_add(1);
    if *serial > 0xff_ffff {
        return 0;
    }
    directories.insert(*serial, directory.clone());
    while directories.len() > limit {
        directories.pop_first();
    }
    *serial
}

struct Layout<'a> {
    layout_revision: &'a mut u64,
    parser: &'a mut vt100::Parser,
    id: &'a mut u32,
    serial: &'a mut u32,
    directory: &'a Option<Arc<PathBuf>>,
    directories: &'a mut BTreeMap<u32, Arc<PathBuf>>,
    limit: usize,
    start: &'a mut bool,
}
impl vte::Perform for Layout<'_> {
    fn print(&mut self, c: char) {
        if *self.id == 0 {
            *self.id = next_id(self.serial, self.directory, self.directories, self.limit);
            *self.start = true;
        }
        let id = *self.id;
        let color = vt100::Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8);
        let before = self.parser.screen().cursor_position();
        let previous = (!c.is_ascii() && before.1 > 0)
            .then(|| {
                let cell = self.parser.screen().cell(before.0, before.1 - 1)?;
                let old = color_id(cell.fgcolor());
                (self.directories.get(&old) != self.directory.as_ref()).then(|| (old, cell.contents().to_string()))
            })
            .flatten();
        let start = u8::from(std::mem::take(self.start));
        if self.parser.screen().fgcolor() != color || self.parser.screen().bgcolor() != vt100::Color::Idx(start) {
            self.parser
                .process(format!("\x1b[38;2;{};{};{};48;5;{start}m", id >> 16, id >> 8 & 255, id & 255).as_bytes());
        }
        self.parser.process(c.encode_utf8(&mut [0; 4]).as_bytes());
        if let Some((old, contents)) = previous {
            if self.parser.screen().cursor_position() == before
                && self.parser.screen().cell(before.0, before.1 - 1).is_some_and(|cell| cell.contents() != contents)
            {
                self.directories.remove(&old); // cross-directory combining mark: fail closed
            }
        }
    }
    fn execute(&mut self, b: u8) {
        self.parser.process(&[b]);
        match b {
            b'\n' => *self.id = 0,
            // a span rewriting its own row (progress lines) keeps its ID: the table holds about
            // an ID per retained row, and resizing invalidates a row's spans together anyway
            b'\r' => *self.start = true,
            _ => {}
        }
    }
    fn esc_dispatch(&mut self, intermediates: &[u8], _: bool, b: u8) {
        if b == b'c' && intermediates.is_empty() {
            *self.layout_revision += 1;
        }
        self.parser.process(b"\x1b");
        self.parser.process(intermediates);
        self.parser.process(&[b]);
        *self.id = 0;
    }
    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _: bool, c: char) {
        let switches_screen = matches!(c, 'h' | 'l')
            && intermediates == b"?"
            && params.iter().any(|p| p.iter().any(|p| matches!(p, 47 | 1047 | 1049)));
        if switches_screen {
            *self.layout_revision += 1;
        }
        if c == 'm' && intermediates.is_empty() {
            return;
        }
        let mut sequence = String::from("\x1b[");
        // Private markers precede parameters; other intermediates follow them.
        for &b in intermediates.iter().filter(|&&b| b >= 0x3c) {
            sequence.push(b as char);
        }
        for (i, param) in params.iter().enumerate() {
            if i != 0 {
                sequence.push(';');
            }
            for (j, value) in param.iter().enumerate() {
                if j != 0 {
                    sequence.push(':');
                }
                sequence.push_str(&value.to_string());
            }
        }
        for &b in intermediates.iter().filter(|&&b| b < 0x3c) {
            sequence.push(b as char);
        }
        sequence.push(c);
        self.parser.process(sequence.as_bytes());
        // Erasing leaves the cursor where the span goes on, and a full-screen program's
        // cursor addressing never reaches retained rows: neither starts a new ID, which would
        // evict the oldest rows' directories (grep --color erases about five times a line).
        let erases = intermediates.is_empty() && matches!(c, 'K' | 'J' | 'X');
        if switches_screen || !erases && !self.parser.screen().alternate_screen() {
            *self.id = 0;
        } else if !erases {
            *self.start = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_identity_rejects_foreign_reset_resized_and_roundtrip_alternate_screens() {
        let mut p = Parser::new(2, 20, 3);
        p.process(b"old");
        let before = p.screen().clone();
        let identity = before.identity();
        p.process(b"new");
        assert!(identity.matches(p.screen()), "ordinary output preserves the snapshot layout epoch");
        assert!(!before.same_contents(p.screen()));
        assert!(!identity.matches(Parser::new(2, 20, 3).screen()));
        p.screen_mut().set_size(3, 20);
        p.screen_mut().set_size(2, 20);
        assert!(!identity.matches(p.screen()), "returning to the old size must not revive a job");
        let identity = p.screen().identity();
        for b in b"\x1b[?1049hOTHER\x1b[?1049l" {
            p.process(&[*b]);
        }
        assert!(!p.screen().alternate_screen());
        assert!(!identity.matches(p.screen()), "alternate-screen round trips have distinct epochs");
        let identity = p.screen().identity();
        p.process(b"\x1b");
        p.process(b"c");
        assert!(!identity.matches(p.screen()), "split reset invalidates retained coordinates");
    }

    #[test]
    fn directory_ids_never_wrap_back_to_old_output() {
        let mut parser = Parser::new(2, 20, 2);
        parser.set_directory("/old".into());
        parser.process(b"old.rs\r\n");
        parser.serial = 0xff_ffff - 1;
        parser.set_directory("/last".into());
        parser.process(b"last.rs");
        assert_eq!(parser.screen().directory_at(1, 0), Some(Path::new("/last")));
        parser.set_directory("/exhausted".into());
        parser.process(b"\rnew.rs");
        assert_eq!(parser.screen().directory_at(1, 0), None);
        assert_eq!(parser.screen().directory_at(0, 0), Some(Path::new("/old")));
    }
}
