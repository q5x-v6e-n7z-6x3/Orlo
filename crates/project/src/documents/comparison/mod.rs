//! File comparison snapshots and exact character-coordinate differences, prepared off-thread.

pub mod folders;

use crate::{encoding, watch::Stamp};
use orlo_git::changes;
use ropey::Rope;
use std::fmt;
use std::io::{self, Read};
use std::ops::Range;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_segmentation::UnicodeSegmentation;

const MAX_BYTES: u64 = 100 << 20;
const MAX_GRAPHEMES: usize = 20_000;

/// Supported text is decoded Unicode; lossy decoding is never editable text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub line_ending: &'static str,
    pub bom: bool,
    pub encoding: encoding::Encoding,
}

pub struct Text {
    pub rope: Rope,
    pub endings: Option<super::endings::LineEndings>,
    pub format: Format,
}

/// The exact disk bytes a comparison showed and the opened file's identity, kept to tell
/// later whether the file changed since.
pub struct DiskSnapshot {
    pub text: Text,
    pub bytes: Vec<u8>,
    pub stamp: Stamp,
    pub device: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Inline {
    pub old_chars: Range<usize>,
    pub new_chars: Range<usize>,
}

pub struct Hunk {
    pub lines: changes::Hunk,
    pub inline: Vec<Inline>,
}

pub struct Pair {
    pub hunks: Vec<Hunk>,
    pub formats_differ: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    Symlink,
    NotRegular,
    TooLarge,
    Binary,
    Lossy,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Unsupported(Unsupported),
    Changed,
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Unsupported(reason) => f.write_str(match reason {
                Unsupported::Symlink => "Symlinks do not support text comparison",
                Unsupported::NotRegular => "Not a regular file",
                Unsupported::TooLarge => "File exceeds the 100 MiB comparison limit",
                Unsupported::Binary => "NUL-binary files do not support text comparison",
                Unsupported::Lossy => {
                    "Malformed or unknown encoding; reopen with the correct encoding before comparing"
                }
            }),
            Self::Changed => f.write_str("File changed while loading; refresh the comparison"),
            Self::Cancelled => f.write_str("Comparison cancelled"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn check(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

pub(crate) fn identity(meta: &std::fs::Metadata) -> (u64, Stamp) {
    (meta.dev(), Stamp::of(meta))
}

/// Run on a worker. O_NOFOLLOW/O_NONBLOCK and fstat protect the actual opened file too.
#[cfg(any(test, feature = "test-support"))]
pub fn read(path: &Path, cancel: &AtomicBool) -> Result<DiskSnapshot, Error> {
    read_with_hint(path, None, cancel)
}

/// A remembered encoding only applies to a BOM-less disk file.
pub fn read_with_hint(
    path: &Path,
    hint: Option<encoding::Encoding>,
    cancel: &AtomicBool,
) -> Result<DiskSnapshot, Error> {
    check(cancel)?;
    let before = std::fs::symlink_metadata(path)?;
    if before.is_symlink() {
        return Err(Error::Unsupported(Unsupported::Symlink));
    }
    if !before.is_file() {
        return Err(Error::Unsupported(Unsupported::NotRegular));
    }
    let file =
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(
            |error| {
                if error.raw_os_error() == Some(libc::ELOOP) {
                    Error::Unsupported(Unsupported::Symlink)
                } else {
                    Error::Io(error)
                }
            },
        )?;
    read_opened_with_hint(path, &before, file, hint, cancel)
}

#[cfg(any(test, feature = "test-support"))]
pub fn read_opened(
    path: &Path,
    before: &std::fs::Metadata,
    file: std::fs::File,
    cancel: &AtomicBool,
) -> Result<DiskSnapshot, Error> {
    read_opened_with_hint(path, before, file, None, cancel)
}

pub(crate) fn read_opened_with_hint(
    path: &Path,
    before: &std::fs::Metadata,
    mut file: std::fs::File,
    hint: Option<encoding::Encoding>,
    cancel: &AtomicBool,
) -> Result<DiskSnapshot, Error> {
    check(cancel)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(Error::Unsupported(Unsupported::NotRegular));
    }
    if opened.len() > MAX_BYTES {
        return Err(Error::Unsupported(Unsupported::TooLarge));
    }
    let (device, stamp) = identity(&opened);
    if identity(before) != (device, stamp) {
        return Err(Error::Changed);
    }
    let bytes = read_bytes(&mut file, cancel)?;
    let after = std::fs::symlink_metadata(path)?;
    if !after.is_file() || identity(&after) != (device, stamp) || identity(&file.metadata()?) != (device, stamp) {
        return Err(Error::Changed);
    }
    check(cancel)?;
    let decoded = encoding::decode(&bytes, encoding::bom_encoding(&bytes).or(hint));
    if decoded.lossy {
        return Err(Error::Unsupported(Unsupported::Lossy));
    }
    if decoded.text.contains('\0') {
        return Err(Error::Unsupported(Unsupported::Binary));
    }
    let (rope, line_ending, endings) = super::endings::decoded(&decoded.text);
    check(cancel)?;
    check(cancel)?;
    Ok(DiskSnapshot {
        text: Text {
            rope,
            endings: Some(endings),
            format: Format { line_ending, bom: decoded.bom, encoding: decoded.encoding },
        },
        bytes,
        stamp,
        device,
    })
}

fn read_bytes(reader: &mut impl Read, cancel: &AtomicBool) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 64 << 10];
    loop {
        check(cancel)?;
        let room = chunk.len().min((MAX_BYTES + 1 - bytes.len() as u64) as usize);
        let count = match reader.read(&mut chunk[..room]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Error::Unsupported(Unsupported::TooLarge));
        }
    }
}

/// Run on a worker: retains changes::between's exact line ranges and bounded edit fallback.
pub fn compare(left: &Text, right: &Text, cancel: &AtomicBool) -> Result<Pair, Error> {
    check(cancel)?;
    let lines = changes::between(&left.rope, &right.rope);
    check(cancel)?;
    let mut hunks = Vec::with_capacity(lines.len());
    for lines in lines {
        let mut inline = Vec::new();
        for i in 0..lines.old_lines.len().max(lines.new_lines.len()) {
            check(cancel)?;
            let range = |rope: &Rope, lines: &Range<usize>, end, i| {
                if i < lines.len() {
                    let start = rope.line_to_char(lines.start + i);
                    start..start + rope.line(lines.start + i).len_chars()
                } else {
                    end..end
                }
            };
            let old = range(&left.rope, &lines.old_lines, lines.old_chars.end, i);
            let new = range(&right.rope, &lines.new_lines, lines.new_chars.end, i);
            if old.is_empty() || new.is_empty() {
                inline.push(Inline { old_chars: old, new_chars: new });
                continue;
            }
            let copy = |rope: &Rope, range: Range<usize>| -> Result<String, Error> {
                let slice = rope.slice(range);
                let mut text = String::with_capacity(slice.len_bytes());
                for chunk in slice.chunks() {
                    check(cancel)?;
                    text.push_str(chunk);
                }
                Ok(text)
            };
            inline.extend(within_line(
                &copy(&left.rope, old.clone())?,
                &copy(&right.rope, new.clone())?,
                old,
                new,
                cancel,
            )?);
        }
        hunks.push(Hunk { lines, inline });
    }
    check(cancel)?;
    let uniform = |text: &Text| {
        let counts = text.endings.as_ref().map(|m| m.counts());
        match counts {
            None => Some(text.format.line_ending),
            Some(c) if c.iter().filter(|n| **n > 0).count() <= 1 => {
                c.iter().position(|n| *n > 0).map(|i| ["\n", "\r\n", "\r"][i]).or(Some(text.format.line_ending))
            }
            _ => None,
        }
    };
    let endings_differ = match (uniform(left), uniform(right)) {
        (Some(a), Some(b)) => a != b,
        _ => left.endings != right.endings,
    };
    Ok(Pair { hunks, formats_differ: left.format != right.format || endings_differ })
}

struct Tokens<'a> {
    values: Vec<&'a str>,
    chars: Vec<usize>,
}

fn tokens<'a>(line: &'a str, cancel: &AtomicBool) -> Result<Option<Tokens<'a>>, Error> {
    check(cancel)?;
    let mut out = Tokens { values: Vec::new(), chars: vec![0] };
    for grapheme in line.graphemes(true) {
        check(cancel)?;
        if out.values.len() == MAX_GRAPHEMES {
            return Ok(None);
        }
        out.values.push(grapheme);
        out.chars.push(out.chars.last().unwrap() + grapheme.chars().count());
    }
    Ok(Some(out))
}

fn within_line(
    old: &str,
    new: &str,
    old_chars: Range<usize>,
    new_chars: Range<usize>,
    cancel: &AtomicBool,
) -> Result<Vec<Inline>, Error> {
    if old == new {
        check(cancel)?;
        return Ok(Vec::new());
    }
    // over 20,000 graphemes highlight the line; raise only with a bounded detail algorithm.
    let (Some(a), Some(b)) = (tokens(old, cancel)?, tokens(new, cancel)?) else {
        return Ok(vec![Inline { old_chars, new_chars }]);
    };
    let prefix = a.values.iter().zip(&b.values).take_while(|(a, b)| a == b).count();
    let suffix =
        a.values[prefix..].iter().rev().zip(b.values[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (ae, be) = (a.values.len() - suffix, b.values.len() - suffix);
    check(cancel)?;
    // The same 500-edit fallback as line hunks, with equal grapheme edges retained.
    let pairs = orlo_utils::text::common_lines(&a.values[prefix..ae], &b.values[prefix..be]).unwrap_or_default();
    check(cancel)?;
    let (mut i, mut j) = (prefix, prefix);
    let mut spans = Vec::new();
    for (mi, mj) in pairs.into_iter().map(|(i, j)| (prefix + i, prefix + j)).chain([(ae, be)]) {
        if i < mi || j < mj {
            spans.push(Inline {
                old_chars: old_chars.start + a.chars[i]..old_chars.start + a.chars[mi],
                new_chars: new_chars.start + b.chars[j]..new_chars.start + b.chars[mj],
            });
        }
        (i, j) = (mi + 1, mj + 1);
    }
    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn text(value: &str) -> Text {
        Text {
            endings: None,
            rope: ropey::Rope::from_str(value),
            format: Format { line_ending: "\n", bom: false, encoding: encoding::Encoding::Utf8 },
        }
    }

    fn span(old_chars: Range<usize>, new_chars: Range<usize>) -> Inline {
        Inline { old_chars, new_chars }
    }

    fn pair(old: &str, new: &str) -> Pair {
        compare(&text(old), &text(new), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn separated_spans_keep_original_character_coordinates() {
        let p = pair("same\na cat and dog!\n", "same\na bat and hog!\n");
        assert_eq!(p.hunks.len(), 1);
        assert_eq!(p.hunks[0].inline, [span(7..8, 7..8), span(15..16, 15..16)]);
        assert!(!p.formats_differ);
    }

    #[test]
    fn combining_and_emoji_clusters_are_never_split() {
        let p = pair("é\ne\u{301} / 👩‍💻 / 猫\n", "é\ne\u{302} / 👨‍💻 / 犬\n");
        assert_eq!(p.hunks[0].inline, [span(2..4, 2..4), span(7..10, 7..10), span(13..14, 13..14)]);
    }

    #[test]
    fn insertion_deletion_and_eof_have_explicit_empty_sides() {
        assert!(pair("", "").hunks.is_empty());
        assert_eq!(pair("", "猫").hunks[0].inline, [span(0..0, 0..1)]);
        assert_eq!(pair("猫", "").hunks[0].inline, [span(0..1, 0..0)]);
        assert_eq!(pair("a\n", "a").hunks[0].inline, [span(1..2, 1..1)]);
        assert_eq!(pair("ac", "abc").hunks[0].inline, [span(1..1, 1..2)]);
        assert_eq!(pair("abc", "ac").hunks[0].inline, [span(1..2, 1..1)]);
        assert_eq!(pair("one\ntwo\n", "1\n").hunks[0].inline.last().unwrap(), &span(4..8, 2..2));
    }

    #[test]
    fn exact_hunks_reconstruct_either_side_including_repeated_lines() {
        let values = ["", "a", "a\n", "a\n\n", "x\na\nx\nb\nx\n", "x\nA\nx\nB\nx\n", "é\r\n猫\r\n", "👩‍💻\nlast"];
        for old in values {
            for new in values {
                let (left, right) = (text(old), text(new));
                let p = compare(&left, &right, &AtomicBool::new(false)).unwrap();
                assert_eq!(
                    p.hunks.iter().map(|h| h.lines.clone()).collect::<Vec<_>>(),
                    orlo_git::changes::between(&left.rope, &right.rope)
                );
                let (mut forward, mut backward) = (left.rope.clone(), right.rope.clone());
                for h in p.hunks.iter().rev() {
                    forward.remove(h.lines.old_chars.clone());
                    forward.insert(h.lines.old_chars.start, &right.rope.slice(h.lines.new_chars.clone()).to_string());
                    backward.remove(h.lines.new_chars.clone());
                    backward.insert(h.lines.new_chars.start, &left.rope.slice(h.lines.old_chars.clone()).to_string());
                    for span in &h.inline {
                        assert!(
                            span.old_chars.start >= h.lines.old_chars.start
                                && span.old_chars.end <= h.lines.old_chars.end
                        );
                        assert!(
                            span.new_chars.start >= h.lines.new_chars.start
                                && span.new_chars.end <= h.lines.new_chars.end
                        );
                    }
                }
                assert_eq!(forward.to_string(), new, "{old:?} -> {new:?}");
                assert_eq!(backward.to_string(), old, "{old:?} <- {new:?}");
            }
        }
        assert_eq!(pair(values[4], values[5]).hunks.len(), 2);
    }

    #[test]
    fn line_and_inline_edit_bounds_keep_truthful_coarse_ranges() {
        let p = pair(&format!("top\n{}end\n", "old\n".repeat(501)), &format!("top\n{}end\n", "new\n".repeat(501)));
        assert_eq!(p.hunks.len(), 1);
        assert_eq!(p.hunks[0].lines.old_lines, 1..502);
        let p = pair(&format!("prefix{}suffix", "a".repeat(501)), &format!("prefix{}suffix", "b".repeat(501)));
        assert_eq!(p.hunks[0].inline, [span(6..507, 6..507)]);
        let p = pair(&"a".repeat(20_001), &"b".repeat(20_001));
        assert_eq!(p.hunks[0].inline, [span(0..20_001, 0..20_001)]);
        // A 99 MiB single line stops before creating a token vector proportional to its length.
        assert!(tokens(&"x".repeat(99 << 20), &AtomicBool::new(false)).unwrap().is_none());
        assert!(tokens(&"e\u{301}".repeat(20_000), &AtomicBool::new(false)).unwrap().is_some());
    }

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "orlo-comparison-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn n25_comparison_reports_boundary_only_differences_without_source_hunks() {
        let fixture = Fixture::new();
        let cancel = AtomicBool::new(false);
        let left = read(&fixture.file("n25-left", b"a\r\nb\nc\rd"), &cancel).unwrap();
        let right = read(&fixture.file("n25-right", b"a\nb\rc\r\nd"), &cancel).unwrap();
        assert_eq!(left.text.rope, right.text.rope);
        let pair = compare(&left.text, &right.text, &cancel).unwrap();
        assert!(pair.hunks.is_empty() && pair.formats_differ);
        assert_eq!(
            left.text
                .endings
                .as_ref()
                .unwrap()
                .text(&left.text.rope, 0..left.text.rope.len_chars())
                .unwrap()
                .as_bytes(),
            left.bytes
        );
        assert_eq!(
            right
                .text
                .endings
                .as_ref()
                .unwrap()
                .text(&right.text.rope, 0..right.text.rope.len_chars())
                .unwrap()
                .as_bytes(),
            right.bytes
        );
    }

    #[test]
    fn background_load_matches_orlo_decode_and_reports_format_only_changes() {
        let fixture = Fixture::new();
        for (name, bytes) in [("crlf", "\u{feff}猫\r\na\r\n"), ("cr", "猫\ra\r"), ("mixed", "a\r\nb\nc\r")] {
            let path = fixture.file(name, bytes.as_bytes());
            let expected = crate::io::Loaded::read(&path).unwrap();
            let snapshot = std::thread::spawn(move || read(&path, &AtomicBool::new(false)).unwrap()).join().unwrap();
            assert_eq!(snapshot.text.rope, expected.rope);
            assert_eq!(
                (snapshot.text.format.line_ending, snapshot.text.format.bom),
                (expected.line_ending, expected.bom)
            );
            assert_eq!(snapshot.bytes, bytes.as_bytes());
            assert_eq!(Some(snapshot.stamp), expected.disk);
            assert!(snapshot.device > 0);
        }
        let left = read(&fixture.file("lf", b"same\n"), &AtomicBool::new(false)).unwrap();
        let right = read(&fixture.file("bom", "\u{feff}same\r\n".as_bytes()), &AtomicBool::new(false)).unwrap();
        let p = compare(&left.text, &right.text, &AtomicBool::new(false)).unwrap();
        assert!(p.hunks.is_empty());
        assert!(p.formats_differ);
    }

    #[test]
    fn unicode_snapshots_keep_exact_bytes_and_compare_encoding_only_changes() {
        use crate::encoding::{encode, Encoding};
        let fixture = Fixture::new();
        let cancel = AtomicBool::new(false);
        let mut previous = None;
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
            let bytes = encode("猫🦀\r\n", encoding, true).unwrap();
            let path = fixture.file(encoding.name(), &bytes);
            let snapshot = read(&path, &cancel).unwrap();
            assert_eq!(snapshot.bytes, bytes);
            assert_eq!(snapshot.text.rope.to_string(), "猫🦀\n");
            assert_eq!(snapshot.text.format.line_ending, "\r\n");
            assert!(snapshot.text.format.bom);
            if let Some(previous) = previous {
                let pair = compare(&previous, &snapshot.text, &cancel).unwrap();
                assert!(pair.hunks.is_empty());
                assert!(pair.formats_differ);
            }
            previous = Some(snapshot.text);
        }
        for bytes in [&[0xff, 0xfe, 0][..], &[0xfe, 0xff, 0xd8, 0][..]] {
            assert!(matches!(
                read(&fixture.file("malformed", bytes), &cancel),
                Err(Error::Unsupported(Unsupported::Lossy))
            ));
        }
    }

    #[test]
    fn unsupported_files_are_distinct_from_real_read_errors() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let cancel = AtomicBool::new(false);
        for (name, bytes, reason) in
            [("binary", b"a\0b".as_slice(), Unsupported::Binary), ("lossy", b"a\xffb".as_slice(), Unsupported::Lossy)]
        {
            assert!(
                matches!(read(&fixture.file(name, bytes), &cancel), Err(Error::Unsupported(found)) if found == reason)
            );
        }
        let big = fixture.file("big", b"");
        std::fs::OpenOptions::new().write(true).open(&big).unwrap().set_len((100 << 20) + 1).unwrap();
        assert!(matches!(read(&big, &cancel), Err(Error::Unsupported(Unsupported::TooLarge))));
        assert!(matches!(read(&fixture.0, &cancel), Err(Error::Unsupported(Unsupported::NotRegular))));
        assert!(matches!(
            read(std::path::Path::new("/dev/null"), &cancel),
            Err(Error::Unsupported(Unsupported::NotRegular))
        ));
        let link = fixture.0.join("link");
        symlink(&big, &link).unwrap();
        assert!(matches!(read(&link, &cancel), Err(Error::Unsupported(Unsupported::Symlink))));
        assert!(
            matches!(read(&fixture.0.join("missing"), &cancel), Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound)
        );
        for reason in [
            Unsupported::Binary,
            Unsupported::Lossy,
            Unsupported::TooLarge,
            Unsupported::NotRegular,
            Unsupported::Symlink,
        ] {
            assert!(!Error::Unsupported(reason).to_string().is_empty());
        }
        assert!(!Error::Changed.to_string().is_empty());
        assert!(!Error::Cancelled.to_string().is_empty());
        assert!(!Error::Io(std::io::ErrorKind::PermissionDenied.into()).to_string().is_empty());
    }

    #[test]
    fn reads_are_bounded_even_when_input_grows_and_check_each_chunk() {
        struct Growing {
            remaining: u64,
            cancel: Option<std::sync::Arc<AtomicBool>>,
        }
        impl Read for Growing {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = out.len().min(self.remaining as usize);
                out[..n].fill(b'a');
                self.remaining -= n as u64;
                if let Some(cancel) = &self.cancel {
                    cancel.store(true, Ordering::Relaxed);
                }
                Ok(n)
            }
        }
        let cancel = AtomicBool::new(false);
        let mut growing = Growing { remaining: MAX_BYTES + 10, cancel: None };
        assert!(matches!(read_bytes(&mut growing, &cancel), Err(Error::Unsupported(Unsupported::TooLarge))));
        assert_eq!(growing.remaining, 9);
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let mut growing = Growing { remaining: MAX_BYTES, cancel: Some(cancel.clone()) };
        assert!(matches!(read_bytes(&mut growing, &cancel), Err(Error::Cancelled)));
        assert!(growing.remaining > MAX_BYTES - (1 << 20));
        assert!(matches!(read_bytes(&mut std::io::repeat(0).take(2), &AtomicBool::new(true)), Err(Error::Cancelled)));
    }

    #[test]
    fn opened_file_validation_rejects_replaced_paths_and_actual_special_handles() {
        use std::os::unix::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        let path = fixture.file("reviewed", b"reviewed");
        let before = std::fs::symlink_metadata(&path).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        std::fs::rename(&path, fixture.0.join("previous")).unwrap();
        fixture.file("reviewed", b"replacement");
        assert!(matches!(read_opened(&path, &before, file, &AtomicBool::new(false)), Err(Error::Changed)));
        let special = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open("/dev/null").unwrap();
        assert!(matches!(
            read_opened(&path, &before, special, &AtomicBool::new(false)),
            Err(Error::Unsupported(Unsupported::NotRegular))
        ));
        let file = std::fs::File::open(&path).unwrap();
        assert!(matches!(read_opened(&path, &before, file, &AtomicBool::new(false)), Err(Error::Changed)));
    }

    #[test]
    fn cancellation_rejects_old_work_before_loading_or_diffing() {
        let cancel = AtomicBool::new(true);
        assert!(matches!(compare(&text("old"), &text("new"), &cancel), Err(Error::Cancelled)));
        assert!(matches!(read(std::path::Path::new("missing"), &cancel), Err(Error::Cancelled)));
        assert!(matches!(tokens("👩‍💻", &cancel), Err(Error::Cancelled)));
    }
}
