//! Bounded folder inventories; all traversal, byte reads and filtering run on workers.

use super::{check, identity, Error, MAX_BYTES};
use ignore::{gitignore::GitignoreBuilder, WalkBuilder};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static AFTER_CHUNK: std::cell::Cell<Option<fn(&AtomicBool)>> = const { std::cell::Cell::new(None) };
}

pub const MAX_ENTRIES: usize = 100_000;
pub const DEFAULT_STATUSES: [bool; 7] = [false, true, true, true, true, true, true];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Status {
    Equal,
    Different,
    LeftOnly,
    RightOnly,
    TypeConflict,
    Unsupported,
    Error,
}

impl Status {
    pub const ALL: [Self; 7] = [
        Self::Equal,
        Self::Different,
        Self::LeftOnly,
        Self::RightOnly,
        Self::TypeConflict,
        Self::Unsupported,
        Self::Error,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Equal => "Equal",
            Self::Different => "Different",
            Self::LeftOnly => "Left only",
            Self::RightOnly => "Right only",
            Self::TypeConflict => "Type conflict",
            Self::Unsupported => "Unsupported",
            Self::Error => "Error",
        }
    }
}

pub struct Row {
    pub path: PathBuf,
    pub status: Status,
    pub note: String,
    pub text_pair: bool,
    pub directory: bool,
}

pub struct Inventory {
    pub rows: Vec<Row>,
    pub scanned: usize,
    pub incomplete: bool,
    pub limited: bool,
    pub issues: Vec<String>,
}

impl Inventory {
    pub fn summary(&self) -> String {
        format!(
            "{} paths · {} entries scanned{}",
            self.rows.len(),
            self.scanned,
            if self.limited {
                " · Limited / Incomplete (100,000 entry limit)"
            } else if self.incomplete {
                " · Incomplete — absence is unknown"
            } else {
                " · Complete"
            }
        )
    }
}

enum Entry {
    Directory,
    File(Metadata),
    Unsupported(&'static str),
    Error(String),
}

fn error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => error_path(err),
        _ => None,
    }
}

pub fn scan(roots: &[PathBuf; 2], cancel: &AtomicBool, progress: &AtomicUsize) -> Result<Inventory, Error> {
    scan_with_hints(roots, &BTreeMap::new(), cancel, progress)
}

/// Hints from open documents describe disk bytes, not a pending save conversion.
pub fn scan_with_hints(
    roots: &[PathBuf; 2],
    hints: &BTreeMap<PathBuf, crate::encoding::Encoding>,
    cancel: &AtomicBool,
    progress: &AtomicUsize,
) -> Result<Inventory, Error> {
    scan_using(roots, cancel, progress, MAX_ENTRIES, hints)
}

#[cfg(test)]
fn scan_with_limit(
    roots: &[PathBuf; 2],
    cancel: &AtomicBool,
    progress: &AtomicUsize,
    limit: usize,
) -> Result<Inventory, Error> {
    scan_using(roots, cancel, progress, limit, &BTreeMap::new())
}

fn scan_using(
    roots: &[PathBuf; 2],
    cancel: &AtomicBool,
    progress: &AtomicUsize,
    limit: usize,
    hints: &BTreeMap<PathBuf, crate::encoding::Encoding>,
) -> Result<Inventory, Error> {
    let mut entries: BTreeMap<PathBuf, [Option<Entry>; 2]> = BTreeMap::new();
    let mut parents = BTreeSet::new();
    let mut complete = [true; 2];
    let mut scanned = 0;
    let mut limited = false;
    let mut issues = Vec::new();
    for side in 0..2 {
        check(cancel)?;
        let root = &roots[side];
        let before = std::fs::symlink_metadata(root);
        if !before.as_ref().is_ok_and(|m| m.is_dir()) {
            complete[side] = false;
            let error = before.err().map_or_else(|| "Root is no longer a real directory".into(), |e| e.to_string());
            issues.push(format!("{}: {error}", root.display()));
            entries.entry(PathBuf::new()).or_default()[side] = Some(Entry::Error(error));
            continue;
        }
        let mut walk = WalkBuilder::new(root);
        walk.standard_filters(false).follow_links(false).skip_stdout(false);
        for item in walk.build() {
            check(cancel)?;
            if item.as_ref().is_ok_and(|e| e.depth() == 0) {
                continue;
            }
            if scanned == limit {
                limited = true;
                complete[side..].fill(false);
                break;
            }
            scanned += 1;
            progress.store(scanned, Ordering::Relaxed);
            let (path, entry) = match item {
                Ok(e) => {
                    let path = e.path().strip_prefix(root).unwrap().to_path_buf();
                    let entry = match std::fs::symlink_metadata(e.path()) {
                        Ok(m) if m.is_dir() => Entry::Directory,
                        Ok(m) if m.is_file() => Entry::File(m),
                        Ok(m) if m.is_symlink() => Entry::Unsupported("Symlink — not followed"),
                        Ok(_) => Entry::Unsupported("Special file — not read"),
                        Err(e) => {
                            complete[side] = false;
                            Entry::Error(e.to_string())
                        }
                    };
                    (path, entry)
                }
                Err(e) => {
                    complete[side] = false;
                    let path =
                        error_path(&e).and_then(|p| p.strip_prefix(root).ok()).unwrap_or(Path::new("")).to_path_buf();
                    (path, Entry::Error(e.to_string()))
                }
            };
            if let Entry::Error(error) = &entry {
                issues.push(format!("{}: {error}", root.join(&path).display()));
            }
            if let Some(parent) = path.parent() {
                parents.insert(parent.to_path_buf());
            }
            entries.entry(path).or_default()[side] = Some(entry);
            if scanned == limit {
                limited = true;
                complete[side..].fill(false);
                break;
            }
        }
        let intact = before
            .ok()
            .zip(std::fs::symlink_metadata(root).ok())
            .is_some_and(|(a, b)| b.is_dir() && (a.dev(), a.ino()) == (b.dev(), b.ino()));
        if !intact {
            complete[side] = false;
            issues.push(format!("{}: root changed while scanning", root.display()));
        }
        if limited {
            break;
        }
    }
    let mut rows = Vec::with_capacity(entries.len());
    for (path, sides) in entries {
        check(cancel)?;
        let directory = sides.iter().flatten().all(|e| matches!(e, Entry::Directory));
        if directory && parents.contains(&path) {
            continue;
        }
        let mut row = Row { path, status: Status::Error, note: String::new(), text_pair: false, directory };
        let files = [0, 1].map(|side| match &sides[side] {
            Some(Entry::File(m)) => Some((roots[side].join(&row.path), m)),
            _ => None,
        });
        let errors: Vec<_> = sides
            .iter()
            .flatten()
            .filter_map(|e| if let Entry::Error(error) = e { Some(error.as_str()) } else { None })
            .collect();
        if !errors.is_empty() {
            row.note = errors.join(" · ");
        // walk errors make every unknown counterpart unsafe; narrow only with subtree coverage.
        } else if sides.iter().enumerate().any(|(i, e)| e.is_none() && !complete[i]) {
            row.note = "Counterpart unknown — incomplete inventory; Refresh".into();
        } else if sides.iter().flatten().any(|e| matches!(e, Entry::Unsupported(_))) {
            row.status = Status::Unsupported;
            row.note = sides
                .iter()
                .flatten()
                .filter_map(|e| if let Entry::Unsupported(note) = e { Some(*note) } else { None })
                .collect::<Vec<_>>()
                .join(" · ");
        } else if sides.iter().all(Option::is_some)
            && matches!(sides[0], Some(Entry::Directory)) != matches!(sides[1], Some(Entry::Directory))
        {
            row.status = Status::TypeConflict;
            row.note = "File / directory conflict — inspection only".into();
        } else if files.iter().any(Option::is_some) {
            let paths: Vec<_> = files.iter().flatten().map(|(p, m)| (p.as_path(), *m)).collect();
            match stream_files(&paths, hints, cancel) {
                Ok((equal, note)) => {
                    row.status = one_sided(&sides).unwrap_or(if equal { Status::Equal } else { Status::Different });
                    row.text_pair = paths.len() == 2 && note.is_empty();
                    row.note = note;
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => {
                    row.status = if matches!(e, Error::Unsupported(_)) { Status::Unsupported } else { Status::Error };
                    row.note = e.to_string();
                }
            }
        } else {
            row.status = one_sided(&sides).unwrap_or(Status::Equal);
            row.note = "Empty directory — inspection only".into();
        }
        rows.push(row);
    }
    let incomplete = !complete.into_iter().all(|v| v) || rows.iter().any(|row| row.status == Status::Error);
    Ok(Inventory { rows, scanned, incomplete, limited, issues })
}

/// Which side alone has the path, if only one does.
fn one_sided(sides: &[Option<Entry>; 2]) -> Option<Status> {
    if sides[0].is_none() {
        Some(Status::RightOnly)
    } else if sides[1].is_none() {
        Some(Status::LeftOnly)
    } else {
        None
    }
}

#[derive(Default)]
struct Utf8 {
    tail: Vec<u8>,
    invalid: bool,
    binary: bool,
}
impl Utf8 {
    fn feed(&mut self, bytes: &[u8]) {
        self.binary |= bytes.contains(&0);
        if self.invalid {
            return;
        }
        self.tail.extend_from_slice(bytes);
        match std::str::from_utf8(&self.tail) {
            Ok(_) => self.tail.clear(),
            Err(e) if e.error_len().is_none() => {
                self.tail.drain(..e.valid_up_to());
            }
            Err(_) => {
                self.invalid = true;
                self.tail.clear();
            }
        }
    }
}

#[derive(Default)]
struct Unicode {
    encoding: Option<crate::encoding::Encoding>,
    hint: Option<crate::encoding::Encoding>,
    legacy: Option<encoding_rs::Decoder>,
    finished: bool,
    utf8: Utf8,
    tail: Vec<u8>,
    invalid: bool,
    binary: bool,
}

impl Unicode {
    fn feed(&mut self, bytes: &[u8]) {
        use crate::encoding::{self, Encoding};
        let bytes = if self.encoding.is_none() {
            let marked = encoding::bom_encoding(bytes);
            let encoding = marked.or(self.hint).unwrap_or_default();
            self.encoding = Some(encoding);
            self.legacy = encoding.legacy().map(|codec| codec.new_decoder_without_bom_handling());
            if marked.is_some() {
                &bytes[encoding.bom_bytes().len()..]
            } else {
                bytes
            }
        } else {
            bytes
        };
        let encoding = self.encoding.unwrap();
        if encoding == Encoding::Utf8 {
            self.utf8.feed(bytes);
            return;
        }
        if self.invalid {
            return;
        }
        if encoding == Encoding::Iso8859_1 {
            self.binary |= bytes.contains(&0);
            return;
        }
        if let Some(decoder) = &mut self.legacy {
            if self.finished {
                return;
            }
            self.finished = bytes.is_empty();
            let mut rest = bytes;
            let mut text = String::with_capacity(bytes.len().saturating_mul(3).saturating_add(16));
            loop {
                let (result, read, errors) = decoder.decode_to_string(rest, &mut text, bytes.is_empty());
                self.invalid |= errors;
                self.binary |= text.contains(char::from(0));
                if result == encoding_rs::CoderResult::InputEmpty {
                    break;
                }
                rest = &rest[read..];
                text.clear();
            }
            return;
        }
        if matches!(encoding, Encoding::Utf32Le | Encoding::Utf32Be) {
            self.tail.extend_from_slice(bytes);
            let end = self.tail.len() / 4 * 4;
            for unit in self.tail[..end].as_chunks::<4>().0 {
                let scalar =
                    if encoding == Encoding::Utf32Le { u32::from_le_bytes(*unit) } else { u32::from_be_bytes(*unit) };
                self.binary |= scalar == 0;
                self.invalid |= char::from_u32(scalar).is_none();
            }
            self.tail.drain(..end);
            return;
        }
        let unit = |bytes| {
            if encoding == Encoding::Utf16Le {
                u16::from_le_bytes(bytes)
            } else {
                u16::from_be_bytes(bytes)
            }
        };
        self.tail.extend_from_slice(bytes);
        let mut end = self.tail.len() / 2 * 2;
        if end >= 2 && (0xd800..=0xdbff).contains(&unit([self.tail[end - 2], self.tail[end - 1]])) {
            end -= 2;
        }
        let units = self.tail[..end].as_chunks::<2>().0.iter().map(|&bytes| unit(bytes));
        for decoded in char::decode_utf16(units) {
            match decoded {
                Ok('\0') => self.binary = true,
                Err(_) => self.invalid = true,
                _ => {}
            }
        }
        self.tail.drain(..end);
    }

    fn binary(&self) -> bool {
        self.binary || self.utf8.binary
    }

    fn invalid(&self) -> bool {
        self.invalid || !self.tail.is_empty() || self.utf8.invalid || !self.utf8.tail.is_empty()
    }
}

fn stream_files(
    paths: &[(&Path, &Metadata)],
    hints: &BTreeMap<PathBuf, crate::encoding::Encoding>,
    cancel: &AtomicBool,
) -> Result<(bool, String), Error> {
    let mut files: Vec<File> = Vec::new();
    for (path, before) in paths {
        check(cancel)?;
        if before.len() > MAX_BYTES {
            return Err(Error::Unsupported(super::Unsupported::TooLarge));
        }
        let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        let opened = file.metadata()?;
        if !opened.is_file() || identity(before) != identity(&opened) {
            return Err(Error::Changed);
        }
        files.push(file);
    }
    let mut buffers = [[0; 64 << 10]; 2];
    let mut unicode: [Unicode; 2] = Default::default();
    for ((path, _), validator) in paths.iter().zip(&mut unicode) {
        validator.hint = hints.get(*path).copied();
    }
    let mut total = [0; 2];
    let mut equal = true;
    loop {
        let mut lengths = [0; 2];
        for (i, file) in files.iter_mut().enumerate() {
            while lengths[i] < buffers[i].len() {
                check(cancel)?;
                let n = match file.read(&mut buffers[i][lengths[i]..]) {
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    result => result?,
                };
                if n == 0 {
                    break;
                }
                lengths[i] += n;
                total[i] += n as u64;
                if total[i] > MAX_BYTES {
                    return Err(Error::Unsupported(super::Unsupported::TooLarge));
                }
            }
            unicode[i].feed(&buffers[i][..lengths[i]]);
            #[cfg(any(test, feature = "test-support"))]
            if let Some(after) = AFTER_CHUNK.take() {
                after(cancel);
            }
        }
        if files.len() == 2 {
            equal &= buffers[0][..lengths[0]] == buffers[1][..lengths[1]];
        }
        if lengths == [0, 0] {
            break;
        }
    }
    for ((path, before), file) in paths.iter().zip(files) {
        check(cancel)?;
        let after = std::fs::symlink_metadata(path)?;
        if !after.is_file() || identity(before) != identity(&after) || identity(before) != identity(&file.metadata()?) {
            return Err(Error::Changed);
        }
    }
    let note = if unicode.iter().any(Unicode::binary) {
        "NUL-binary — byte comparison; inspection only"
    } else if unicode.iter().any(Unicode::invalid) {
        "Malformed or unknown encoding — byte comparison; inspection only; reopen with the correct encoding"
    } else {
        ""
    };
    Ok((equal, note.into()))
}

pub fn filter(
    inventory: &Inventory,
    patterns: [&str; 2],
    statuses: [bool; 7],
    cancel: &AtomicBool,
) -> Result<Vec<usize>, String> {
    let mut matchers = Vec::new();
    for (side, field) in patterns.into_iter().enumerate() {
        let mut builder = GitignoreBuilder::new("");
        builder.allow_unclosed_class(false);
        for glob in field.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            check(cancel).map_err(|e| e.to_string())?;
            builder
                .add_line(None, glob)
                .map_err(|e| format!("{} glob {glob:?}: {e}", if side == 0 { "Include" } else { "Exclude" }))?;
        }
        matchers.push(builder.build().map_err(|e| e.to_string())?);
    }
    let matches = |index: usize, row: &Row| {
        let matcher = &matchers[index];
        // Keep explicit path/ancestor precedence, then invert to the fields' positive-match meaning.
        let matched = matcher.matched_path_or_any_parents(&row.path, row.directory).invert();
        matched.is_whitelist() || (index == 0 && matched.is_none() && matcher.num_ignores() == 0)
    };
    let mut visible = Vec::new();
    for (i, row) in inventory.rows.iter().enumerate() {
        check(cancel).map_err(|e| e.to_string())?;
        if statuses[row.status as usize] && matches(0, row) && !matches(1, row) {
            visible.push(i);
        }
    }
    Ok(visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn row<'a>(inventory: &'a Inventory, path: &str) -> &'a Row {
        inventory.rows.iter().find(|r| r.path == Path::new(path)).unwrap()
    }

    fn roots(name: &str) -> [PathBuf; 2] {
        let root = std::env::temp_dir().join(format!("orlo-folders-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let roots = [root.join("left"), root.join("right")];
        for r in &roots {
            std::fs::create_dir_all(r).unwrap();
        }
        roots
    }

    #[test]
    fn folder_inventory_uses_explicit_legacy_hints_and_finalizes_shorter_streams() {
        use crate::encoding::Encoding;
        let roots = roots("f01-hints");
        let left = roots[0].join("text");
        let right = roots[1].join("text");
        std::fs::write(&left, [0x93]).unwrap();
        std::fs::write(&right, vec![0x93; 140_000]).unwrap();
        let cancel = AtomicBool::new(false);
        let progress = AtomicUsize::new(0);
        let unknown = scan(&roots, &cancel, &progress).unwrap();
        assert!(!unknown.rows[0].text_pair);
        let hints = BTreeMap::from([(left.clone(), Encoding::Windows1252), (right.clone(), Encoding::Windows1252)]);
        let known = scan_with_hints(&roots, &hints, &cancel, &progress).unwrap();
        assert!(known.rows[0].text_pair);
        assert_eq!(known.rows[0].status, Status::Different);
        let hints = BTreeMap::from([(left, Encoding::ShiftJis), (right, Encoding::ShiftJis)]);
        let malformed = scan_with_hints(&roots, &hints, &cancel, &progress).unwrap();
        assert!(!malformed.rows[0].text_pair);
    }

    #[test]
    fn utf32_streaming_validation_handles_chunks_and_invalid_scalars() {
        for (bytes, bad) in [
            (&[0xff, 0xfe, 0, 0, 0x41, 0, 0, 0, 0, 0xf6, 1, 0][..], &[0xff, 0xfe, 0, 0, 0, 0xd8, 0, 0][..]),
            (&[0, 0, 0xfe, 0xff, 0, 0, 0, 0x41, 0, 1, 0xf6, 0][..], &[0, 0, 0xfe, 0xff, 0, 0x11, 0, 0][..]),
        ] {
            for split in 4..=bytes.len() {
                let mut valid = Unicode::default();
                valid.feed(&bytes[..split]);
                valid.feed(&bytes[split..]);
                assert!(!valid.invalid() && !valid.binary());
            }
            let mut invalid = Unicode::default();
            invalid.feed(bad);
            assert!(invalid.invalid());
        }
    }

    #[test]
    fn folder_inventory_is_exact_sorted_and_keeps_hidden_binary_and_empty_directories() {
        let roots = roots("inventory");
        for r in &roots {
            std::fs::create_dir(r.join("nested")).unwrap();
            std::fs::write(r.join(".gitignore"), "nested/\n.hidden\n").unwrap();
            std::fs::write(r.join(".hidden"), "same").unwrap();
            std::fs::write(r.join("binary"), [0, 1, 2]).unwrap();
            std::fs::write(r.join("lossy"), [0xff]).unwrap();
        }
        std::fs::write(roots[0].join("nested/a"), "left\r\n").unwrap();
        std::fs::write(roots[1].join("nested/a"), "left\n").unwrap();
        std::fs::write(roots[0].join("only-left"), "left").unwrap();
        std::fs::create_dir(roots[1].join("empty")).unwrap();
        std::fs::write(roots[0].join("conflict"), "file").unwrap();
        std::fs::create_dir(roots[1].join("conflict")).unwrap();
        std::os::unix::fs::symlink("missing", roots[0].join("link")).unwrap();
        let result = scan(&roots, &AtomicBool::new(false), &Default::default()).unwrap();
        assert!(!result.incomplete);
        assert!(!result.limited);
        assert!(result.rows.windows(2).all(|w| w[0].path < w[1].path));
        let row = |path| row(&result, path);
        assert_eq!(row(".hidden").status, Status::Equal);
        assert_eq!(row("nested/a").status, Status::Different);
        assert!(row("nested/a").text_pair);
        assert_eq!(row("binary").status, Status::Equal);
        assert!(!row("binary").text_pair);
        assert!(row("binary").note.contains("binary"));
        assert!(!row("lossy").text_pair);
        assert!(row("lossy").note.contains("Malformed"));
        assert_eq!(row("only-left").status, Status::LeftOnly);
        assert_eq!(row("empty").status, Status::RightOnly);
        assert!(row("empty").directory);
        assert_eq!(row("conflict").status, Status::TypeConflict);
        assert_eq!(row("link").status, Status::Unsupported);
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[test]
    fn folder_unicode_text_pairs_and_one_sided_copies_keep_byte_equality() {
        use crate::encoding::{encode, Encoding};
        let roots = roots("unicode");
        for (name, encoding) in [("le", Encoding::Utf16Le), ("be", Encoding::Utf16Be)] {
            let bytes = encode("猫🦀\r\n", encoding, true).unwrap();
            for root in &roots {
                std::fs::write(root.join(name), &bytes).unwrap();
            }
        }
        std::fs::write(roots[0].join("one"), encode("one", Encoding::Utf16Le, true).unwrap()).unwrap();
        std::fs::write(roots[0].join("malformed"), [0xff, 0xfe, 0, 0xd8]).unwrap();
        std::fs::write(roots[1].join("le"), encode("猫🦀\r\n", Encoding::Utf16Be, true).unwrap()).unwrap();
        let inventory = scan(&roots, &AtomicBool::new(false), &Default::default()).unwrap();
        let row = |name| row(&inventory, name);
        assert_eq!(row("le").status, Status::Different);
        assert!(row("le").text_pair);
        assert_eq!(row("be").status, Status::Equal);
        assert!(row("be").text_pair);
        assert_eq!(row("one").status, Status::LeftOnly);
        assert!(row("one").note.is_empty());
        assert!(!row("malformed").note.is_empty());
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[test]
    fn folder_unicode_stream_keeps_partial_code_units_and_surrogates() {
        use crate::encoding::{encode, Encoding};
        for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes = encode("x🦀\u{fffe}\u{feff}z", encoding, true).unwrap();
            let mut stream = Unicode::default();
            stream.feed(&bytes[..2]);
            for byte in &bytes[2..] {
                stream.feed(std::slice::from_ref(byte));
                assert!(stream.tail.len() <= 3);
            }
            assert!(!stream.invalid());
            assert!(!stream.binary());
            let mut binary = Unicode::default();
            binary.feed(&encode("x\0z", encoding, true).unwrap());
            assert!(binary.binary());
            for invalid in ["odd", "surrogate"] {
                let mut bytes = encode("x", encoding, true).unwrap();
                bytes.extend(if invalid == "odd" {
                    vec![0]
                } else {
                    if encoding == Encoding::Utf16Le {
                        vec![0, 0xd8]
                    } else {
                        vec![0xd8, 0]
                    }
                });
                let mut stream = Unicode::default();
                stream.feed(&bytes);
                assert!(stream.invalid());
            }
        }
    }

    #[test]
    fn folder_filters_are_symmetric_visible_and_invalid_patterns_fail() {
        let roots = roots("filters");
        std::fs::write(roots[0].join("left.rs"), "left").unwrap();
        std::fs::write(roots[1].join("right.rs"), "right").unwrap();
        std::fs::write(roots[0].join("other.txt"), "text").unwrap();
        let stop = AtomicBool::new(false);
        let inv = scan(&roots, &stop, &Default::default()).unwrap();
        let visible = filter(&inv, ["*.rs", ""], DEFAULT_STATUSES, &stop).unwrap();
        assert_eq!(visible.len(), 2);
        assert_eq!(filter(&inv, ["*.rs", "right*"], DEFAULT_STATUSES, &stop).unwrap().len(), 1);
        assert!(filter(&inv, ["*.rs;[", ""], DEFAULT_STATUSES, &stop).unwrap_err().contains("Include"));
        assert!(filter(&inv, ["", "["], DEFAULT_STATUSES, &stop).unwrap_err().contains("Exclude"));
        assert!(filter(&inv, ["", ""], [false; 7], &stop).unwrap().is_empty());
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    fn filter_fixture(paths: &[&str]) -> Inventory {
        Inventory {
            rows: paths
                .iter()
                .map(|path| Row {
                    path: path.into(),
                    status: Status::Different,
                    note: String::new(),
                    text_pair: false,
                    directory: false,
                })
                .collect(),
            scanned: paths.len(),
            incomplete: false,
            limited: false,
            issues: Vec::new(),
        }
    }

    #[test]
    fn folder_negative_globs_preserve_directory_and_wildcard_exceptions_in_both_fields() {
        let inventory = filter_fixture(&["keep.rs", "src/keep.rs", "src/skip.rs"]);
        let stop = AtomicBool::new(false);
        let actual = ["src/;!src/skip.rs", "*;!src/skip.rs"].map(|pattern| {
            [
                filter(&inventory, [pattern, ""], DEFAULT_STATUSES, &stop).unwrap(),
                filter(&inventory, ["", pattern], DEFAULT_STATUSES, &stop).unwrap(),
            ]
        });
        assert_eq!(actual, [[vec![1], vec![0, 2]], [vec![0, 1], vec![2]]]);
    }

    #[test]
    fn folder_negative_globs_preserve_negative_only_include_default() {
        let inventory = filter_fixture(&["keep.rs", "scratch.tmp", "src/keep.rs", "src/scratch.tmp"]);
        let stop = AtomicBool::new(false);
        assert_eq!(filter(&inventory, ["!*.tmp", ""], DEFAULT_STATUSES, &stop).unwrap(), vec![0, 2]);
        assert_eq!(filter(&inventory, ["", "!*.tmp"], DEFAULT_STATUSES, &stop).unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn folder_negative_globs_preserve_explicit_and_nearest_ancestor_ordering() {
        let mut inventory = filter_fixture(&[
            "src/keep.rs",
            "src/skip.rs",
            "src/generated/keep.rs",
            "src/generated/skip.rs",
            "src/empty",
            "other.rs",
        ]);
        inventory.rows[4].directory = true;
        let stop = AtomicBool::new(false);
        for (pattern, expected) in [
            ("src/;!src/skip.rs;src/skip.rs", vec![0, 1, 2, 3, 4]),
            ("src/;src/skip.rs;!src/skip.rs", vec![0, 2, 3, 4]),
            ("src/;!src/generated/", vec![0, 1, 4]),
            ("src/;!src/generated/;src/generated/keep.rs", vec![0, 1, 2, 4]),
            ("src/;!src/empty/", vec![0, 1, 2, 3]),
        ] {
            assert_eq!(
                filter(&inventory, [pattern, ""], DEFAULT_STATUSES, &stop).unwrap(),
                expected,
                "Include {pattern}"
            );
            let remaining: Vec<_> = (0..inventory.rows.len()).filter(|i| !expected.contains(i)).collect();
            assert_eq!(
                filter(&inventory, ["", pattern], DEFAULT_STATUSES, &stop).unwrap(),
                remaining,
                "Exclude {pattern}"
            );
        }
    }

    #[test]
    fn folder_unreadable_subtrees_types_limits_and_chunk_cancellation_are_truthful() {
        use std::os::unix::fs::PermissionsExt;
        let roots = roots("errors");
        for root in &roots {
            std::fs::create_dir(root.join("locked")).unwrap();
        }
        std::fs::write(roots[1].join("locked/unknown"), "text").unwrap();
        std::fs::set_permissions(roots[0].join("locked"), std::fs::Permissions::from_mode(0o0)).unwrap();
        let inv = scan(&roots, &AtomicBool::new(false), &Default::default()).unwrap();
        std::fs::set_permissions(roots[0].join("locked"), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(inv.incomplete);
        assert_eq!(row(&inv, "locked/unknown").status, Status::Error);
        assert!(inv.issues.iter().any(|s| s.contains("Permission denied")));
        let large = std::fs::File::create(roots[0].join("large")).unwrap();
        large.set_len(super::super::MAX_BYTES + 1).unwrap();
        let bytes = [vec![b'a'; (64 << 10) - 1], "🦀".as_bytes().to_vec(), vec![b'b'; 64 << 10]].concat();
        for root in &roots {
            std::fs::write(root.join("utf8"), &bytes).unwrap();
        }
        let inv = scan(&roots, &AtomicBool::new(false), &Default::default()).unwrap();
        assert_eq!(row(&inv, "large").status, Status::Unsupported);
        let utf8 = row(&inv, "utf8");
        assert!(utf8.status == Status::Equal && utf8.text_pair);
        AFTER_CHUNK.set(Some(|stop| stop.store(true, Ordering::Relaxed)));
        assert!(matches!(scan(&roots, &AtomicBool::new(false), &Default::default()), Err(Error::Cancelled)));
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[test]
    fn folder_limit_errors_and_cancellation_never_invent_absence_or_equality() {
        let roots = roots("limited");
        for i in 0..4 {
            std::fs::write(roots[0].join(format!("{i}")), "text").unwrap();
        }
        let inv = scan_with_limit(&roots, &AtomicBool::new(false), &Default::default(), 2).unwrap();
        assert!(inv.incomplete && inv.limited);
        assert!(inv.rows.iter().all(|r| r.status == Status::Error));
        assert!(scan(&roots, &AtomicBool::new(true), &Default::default()).is_err());
        std::fs::remove_dir_all(&roots[0]).unwrap();
        std::fs::write(roots[1].join("unknown"), "text").unwrap();
        let inv = scan(&roots, &AtomicBool::new(false), &Default::default()).unwrap();
        assert!(inv.incomplete);
        assert_eq!(row(&inv, "unknown").status, Status::Error);
        assert!(!inv.issues.is_empty());
        std::fs::remove_dir_all(roots[1].parent().unwrap()).unwrap();
    }
}
