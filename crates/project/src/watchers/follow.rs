//! Bounded, stable snapshots for explicit path following. Called only off the UI thread.
use super::Stamp;
use crate::{encoding::Encoding, io::Loaded};
use orlo_utils::text::Change;
use ropey::Rope;
use std::{io, path::Path};

/// A failed attempt retains the identity observed BEFORE reading/decoding. A later stat
/// may describe a replacement and must never be used as the failed bytes' provenance.
#[derive(Debug)]
pub struct Failure {
    pub error: io::Error,
    pub disk: Option<Stamp>,
    pub file: Option<(u64, u64)>,
}

pub enum Read {
    Same,
    Missing,
    Retry,
    Incomplete { disk: Stamp, file: (u64, u64) },
    Snapshot { loaded: Loaded, changes: Vec<Change>, partial: bool, file: (u64, u64) },
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Inject a filesystem change after the stable bytes were read, before decoding them.
    pub static BEFORE_DECODE: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

pub fn read(path: &Path, known: Option<Stamp>, hint: Encoding, rope: &Rope) -> Result<Read, Failure> {
    use std::os::unix::fs::MetadataExt;
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Read::Missing),
        Err(error) => return Err(Failure { error, disk: None, file: None }),
    };
    let before = Stamp::of(&metadata);
    let file = (metadata.dev(), metadata.ino());
    let failure = |error| Failure { error, disk: Some(before), file: Some(file) };
    if !metadata.is_file() {
        return Err(failure(io::Error::other("Not a regular log file")));
    }
    if known == Some(before) {
        return Ok(Read::Same);
    }
    let bytes = crate::io::read_bytes(path)
        .map_err(|e| failure(io::Error::other(e.unwrap_or_else(|| "Log exceeds the 100 MiB file limit".into()))))?;
    if super::stamp(path) != Some(before) {
        return Ok(Read::Retry);
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(change) = BEFORE_DECODE.take() {
        change(path);
    }
    let Some((decoded, partial)) = crate::encoding::decode_follow(&bytes, hint).map_err(failure)? else {
        return Ok(Read::Incomplete { disk: before, file });
    };
    let (next, line_ending, endings) = crate::documents::endings::decoded(&decoded.text);
    let loaded = Loaded {
        endings: Some(endings),
        rope: next,
        line_ending,
        read_only: partial || metadata.permissions().readonly(),
        disk: Some(before),
        bom: decoded.bom,
        encoding: decoded.encoding,
    };
    let changes = loaded.diff(rope);
    Ok(Read::Snapshot { loaded, changes, partial, file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn path() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orlo-follow-backend-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("log")
    }
    fn snapshot(p: &Path, expected: &str, partial_expected: bool) -> Loaded {
        let Read::Snapshot { loaded, partial, .. } = read(p, None, Encoding::Utf8, &Rope::new()).unwrap() else {
            panic!("expected snapshot")
        };
        assert_eq!(loaded.rope.to_string(), expected);
        assert_eq!(partial, partial_expected);
        loaded
    }
    #[test]
    fn follow_real_append_truncate_rotation_missing_and_partial_tail() {
        let p = path();
        std::fs::write(&p, "a\n").unwrap();
        let l = snapshot(&p, "a\n", false);
        assert!(matches!(read(&p, l.disk, l.encoding, &l.rope).unwrap(), Read::Same));
        std::fs::write(&p, b"a\n\xf0\x9f").unwrap();
        snapshot(&p, "a\n", true);
        std::fs::write(&p, "a\n😀").unwrap();
        snapshot(&p, "a\n😀", false);
        std::fs::write(&p, "").unwrap();
        snapshot(&p, "", false);
        std::fs::rename(&p, p.with_extension("old")).unwrap();
        assert!(matches!(read(&p, l.disk, l.encoding, &l.rope).unwrap(), Read::Missing));
        std::fs::write(&p, "rotated\r\n中").unwrap();
        snapshot(&p, "rotated\n中", false);
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
    #[test]
    fn follow_rejects_malformed_nonfiles_and_oversize() {
        let p = path();
        std::fs::write(&p, b"bad\xff").unwrap();
        assert!(read(&p, None, Encoding::Utf8, &Rope::new()).is_err());
        assert!(read(p.parent().unwrap(), None, Encoding::Utf8, &Rope::new()).is_err());
        std::fs::File::create(&p).unwrap().set_len(crate::io::MAX_SIZE + 1).unwrap();
        assert!(read(&p, None, Encoding::Utf8, &Rope::new()).is_err());
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
    #[test]
    fn follow_retries_a_file_changed_during_the_read() {
        let p = path();
        std::fs::write(&p, "before").unwrap();
        crate::io::BEFORE_READ.set(Some(|p| std::fs::write(p, "changed").unwrap()));
        assert!(matches!(read(&p, None, Encoding::Utf8, &Rope::new()).unwrap(), Read::Retry));
        snapshot(&p, "changed", false);
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
    #[test]
    fn follow_incomplete_bom_is_not_an_unchanged_or_safe_file() {
        let p = path();
        std::fs::write(&p, b"\xff").unwrap();
        assert!(!matches!(read(&p, None, Encoding::Utf8, &Rope::new()).unwrap(), Read::Same | Read::Retry));
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
    #[test]
    fn follow_real_encoded_files_withhold_partial_utf16_utf32_and_stateful_cjk() {
        let p = path();
        for (encoding, text) in [
            (Encoding::Utf16Le, "A😀中"),
            (Encoding::Utf32Be, "A😀中"),
            (Encoding::Iso2022Jp, "A日本"),
            (Encoding::Gb18030, "A😀中"),
        ] {
            let bytes = crate::encoding::encode(text, encoding, false).unwrap();
            for end in 1..=bytes.len() {
                std::fs::write(&p, &bytes[..end]).unwrap();
                match read(&p, None, encoding, &Rope::new()).unwrap() {
                    Read::Snapshot { loaded, partial, .. } => {
                        assert!(text.starts_with(&loaded.rope.to_string()), "{encoding:?}: prefix {end}");
                        assert_eq!(loaded.read_only, partial);
                    }
                    Read::Incomplete { .. } => {
                        // The leading zero(s) of BOM-less UTF-32 BE also prefix its BOM.
                        assert_eq!(encoding, Encoding::Utf32Be);
                        assert!(end <= 2);
                    }
                    _ => panic!("expected a decoded prefix or an incomplete BOM"),
                }
                assert_eq!(std::fs::read(&p).unwrap(), bytes[..end]);
            }
        }
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
    #[test]
    fn follow_failures_keep_the_failed_sources_stamp_and_file_identity() {
        use std::os::unix::fs::MetadataExt;
        let p = path();
        std::fs::write(&p, b"old malformed\xff").unwrap();
        let before = std::fs::metadata(&p).unwrap();
        BEFORE_DECODE.set(Some(|p| {
            let next = p.with_extension("replacement");
            std::fs::write(&next, "valid replacement").unwrap();
            std::fs::rename(next, p).unwrap();
        }));
        let Err(failure) = read(&p, None, Encoding::Utf8, &Rope::new()) else { panic!("expected old bytes to fail") };
        assert_eq!(failure.error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(failure.disk, Some(Stamp::of(&before)));
        assert_eq!(failure.file, Some((before.dev(), before.ino())));
        assert_ne!(failure.disk, super::super::stamp(&p));
        snapshot(&p, "valid replacement", false);
        std::fs::File::create(&p).unwrap().set_len(crate::io::MAX_SIZE + 1).unwrap();
        let too_large = std::fs::metadata(&p).unwrap();
        let Err(failure) = read(&p, None, Encoding::Utf8, &Rope::new()) else { panic!("expected size failure") };
        assert_eq!(failure.disk, Some(Stamp::of(&too_large)));
        assert_eq!(failure.file, Some((too_large.dev(), too_large.ino())));
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}
