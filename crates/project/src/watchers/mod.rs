//! Noticing changes on disk: file stamps, what changed, what to do about it, folder listings.

pub mod follow;

use std::ffi::OsString;
use std::path::Path;

/// What a file looked like when Orlo last read or wrote it. The ctime catches writes that
/// put the mtime back (`cp -p`, `rsync -t`), the inode a file replaced by a rename. Times are
/// (seconds, nanoseconds) since 1970, so a file dated before it still goes in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stamp {
    #[serde(deserialize_with = "mtime")]
    pub mtime: (i64, i64),
    pub len: u64,
    pub ctime: (i64, i64),
    pub ino: u64,
}

/// A session from before kept the mtime as serde's `SystemTime`.
fn mtime<'de, D: serde::Deserializer<'de>>(d: D) -> Result<(i64, i64), D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Mtime {
        Pair(i64, i64),
        Old { secs_since_epoch: i64, nanos_since_epoch: i64 },
    }
    Ok(match serde::Deserialize::deserialize(d)? {
        Mtime::Pair(s, n) => (s, n),
        Mtime::Old { secs_since_epoch: s, nanos_since_epoch: n } => (s, n),
    })
}

impl Stamp {
    pub(crate) fn of(m: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self { mtime: (m.mtime(), m.mtime_nsec()), len: m.len(), ctime: (m.ctime(), m.ctime_nsec()), ino: m.ino() }
    }

    /// The same file with the same contents, perhaps moved (a rename changes the ctime).
    pub fn moved(&self, other: &Stamp) -> bool {
        (self.mtime, self.len, self.ino) == (other.mtime, other.len, other.ino)
    }
}

/// `None` when the file is missing (or unreadable).
pub fn stamp(path: &Path) -> Option<Stamp> {
    Some(Stamp::of(&std::fs::metadata(path).ok()?))
}

#[derive(Debug, PartialEq)]
pub enum Disk {
    Same,
    Changed(Stamp),
    Gone,
    /// There again after it was gone (a save by rename, `git checkout`).
    Back(Stamp),
}

/// Compare the file with the buffer's `known` stamp; `deleted` is the buffer already marked gone.
pub fn check(path: &Path, known: Option<Stamp>, deleted: bool) -> Disk {
    match (stamp(path), deleted) {
        (None, true) => Disk::Same,
        // never written, e.g. a settings template not saved yet
        (None, false) if known.is_none() => Disk::Same,
        (None, false) => Disk::Gone,
        (Some(s), true) => Disk::Back(s),
        (Some(s), false) if known == Some(s) => Disk::Same,
        (Some(s), false) => Disk::Changed(s),
    }
}

/// A buffer's stamp and deleted flag, as a poll saw them.
pub type Known = (Option<Stamp>, bool);

#[derive(Debug, PartialEq)]
pub enum Act {
    Nothing,
    /// Take this (stamp, deleted) without reading the file: marked gone, back unchanged, or
    /// set to Always Ignore (the stamp is taken so the next poll does not report it again).
    Mark(Option<Stamp>, bool),
    Reload(Stamp),
    Ask(Stamp),
}

/// A clean buffer reloads, a dirty one asks, one set to Always Ignore does neither; a deleted
/// file is only marked, never closed. `then` is the buffer's stamp and deleted flag when the
/// poll started, `now` when the result lands: a buffer saved, reloaded or marked in between is
/// left to the next poll, since the result may be about a file it no longer expects (or a stat
/// taken in the middle of its own save).
pub fn act(disk: Disk, then: Known, now: Known, dirty: bool, watching: bool) -> Act {
    let known = now.0;
    if then != now {
        return Act::Nothing;
    }
    match disk {
        Disk::Same => Act::Nothing,
        Disk::Gone => Act::Mark(known, true),
        // moved away and back, or Finder's Put Back: only the red dot goes
        Disk::Back(s) if known.is_some_and(|k| k.moved(&s)) => Act::Mark(Some(s), false),
        Disk::Changed(s) | Disk::Back(s) if !watching => Act::Mark(Some(s), false),
        Disk::Changed(s) | Disk::Back(s) if dirty => Act::Ask(s),
        Disk::Changed(s) | Disk::Back(s) => Act::Reload(s),
    }
}

/// The entry names of `dir`, sorted, without a stat per entry; `None` when it cannot be read.
pub fn names(dir: &Path) -> Option<Vec<OsString>> {
    let mut v: Vec<OsString> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name()).collect();
    v.sort();
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `p`'s mtime put back to `mtime`.
    fn set_mtime(p: &Path, mtime: std::time::SystemTime) {
        std::fs::File::options().write(true).open(p).unwrap().set_modified(mtime).unwrap();
    }

    #[test]
    fn stamp_of_a_missing_file_is_none() {
        assert_eq!(stamp(&temp_dir("orlo-watch-none").join("nope")), None);
    }

    #[test]
    fn a_buffer_never_written_is_not_gone() {
        let missing = temp_dir("orlo-watch-new").join("nope");
        assert_eq!(check(&missing, None, false), Disk::Same);
    }

    #[test]
    fn check_reports_same_changed_gone_and_back() {
        let p = temp_dir("orlo-watch-check").join("f");
        std::fs::write(&p, "a").unwrap();
        let s = stamp(&p);
        assert_eq!(check(&p, s, false), Disk::Same);
        std::fs::write(&p, "ab").unwrap();
        let s2 = stamp(&p).unwrap();
        assert_eq!(check(&p, s, false), Disk::Changed(s2));
        std::fs::remove_file(&p).unwrap();
        assert_eq!(check(&p, Some(s2), false), Disk::Gone);
        assert_eq!(check(&p, Some(s2), true), Disk::Same, "already marked deleted");
        std::fs::write(&p, "abc").unwrap();
        assert_eq!(check(&p, Some(s2), true), Disk::Back(stamp(&p).unwrap()));
    }

    #[test]
    fn a_write_that_keeps_the_mtime_and_size_is_still_a_change() {
        let dir = temp_dir("orlo-watch-ctime");
        let p = dir.join("f");
        std::fs::write(&p, "aaa").unwrap();
        let known = stamp(&p);
        let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        // `cp -p`: same size, the mtime put back
        std::fs::write(&p, "bbb").unwrap();
        set_mtime(&p, mtime);
        assert!(matches!(check(&p, known, false), Disk::Changed(_)));
        // replaced by a rename (another editor's atomic save), mtime and size the same
        let known = stamp(&p);
        let other = dir.join("g");
        std::fs::write(&other, "ccc").unwrap();
        set_mtime(&other, mtime);
        std::fs::rename(&other, &p).unwrap();
        assert!(matches!(check(&p, known, false), Disk::Changed(_)));
    }

    #[test]
    fn act_table() {
        let s = stamp(&std::env::current_exe().unwrap()).unwrap();
        let older = Some(Stamp { len: s.len + 1, ..s });
        let act = |disk, deleted, dirty, watching| act(disk, (older, deleted), (older, deleted), dirty, watching);
        assert_eq!(act(Disk::Same, false, true, true), Act::Nothing);
        assert_eq!(act(Disk::Changed(s), false, false, true), Act::Reload(s));
        assert_eq!(act(Disk::Changed(s), false, true, true), Act::Ask(s));
        assert_eq!(act(Disk::Back(s), true, false, true), Act::Reload(s));
        assert_eq!(act(Disk::Back(s), true, true, true), Act::Ask(s));
        assert_eq!(act(Disk::Gone, false, false, true), Act::Mark(older, true));
        assert_eq!(act(Disk::Gone, false, true, false), Act::Mark(older, true));
        // Always Ignore: no red dot, no retry every poll
        assert_eq!(act(Disk::Changed(s), false, true, false), Act::Mark(Some(s), false));
        assert_eq!(act(Disk::Back(s), true, false, false), Act::Mark(Some(s), false));
    }

    #[test]
    fn a_file_moved_away_and_back_loses_its_red_dot() {
        let dir = temp_dir("orlo-watch-back");
        let (p, away) = (dir.join("f"), dir.join("g"));
        std::fs::write(&p, "a").unwrap();
        let known = stamp(&p);
        std::fs::rename(&p, &away).unwrap();
        let k = (known, false);
        assert_eq!(act(check(&p, known, false), k, k, false, true), Act::Mark(known, true));
        std::fs::rename(&away, &p).unwrap();
        let back = stamp(&p);
        assert!(back.unwrap().moved(&known.unwrap()), "a move keeps the file");
        let k = (known, true);
        for dirty in [false, true] {
            assert_eq!(
                act(check(&p, known, true), k, k, dirty, true),
                Act::Mark(back, false),
                "deleted is cleared, nothing is read or asked"
            );
        }
        assert_eq!(check(&p, back, false), Disk::Same);
    }

    #[test]
    fn names_are_sorted_and_see_adds_and_removes() {
        let dir = temp_dir("orlo-watch-names");
        std::fs::write(dir.join("b"), "").unwrap();
        std::fs::write(dir.join("a"), "").unwrap();
        assert_eq!(names(&dir).unwrap(), ["a", "b"]);
        std::fs::write(dir.join("c"), "").unwrap();
        std::fs::remove_file(dir.join("a")).unwrap();
        assert_eq!(names(&dir).unwrap(), ["b", "c"]);
        assert_eq!(names(&dir.join("nope")), None);
    }
}
