//! The side bar's file operations on disk: create, rename, move, duplicate. None of them
//! writes over a file that is there; links are moved or copied as links, never followed.

pub mod platform;
pub mod reviewed_copy;

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(crate) fn c_path(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name with a NUL in it"))
}

/// `child` of the open folder `parent`, never through a link, never blocking on a FIFO.
pub fn open_at(parent: &File, child: &OsStr, flags: libc::c_int) -> io::Result<File> {
    let child = c_path(Path::new(child))?;
    // SAFETY: a live parent descriptor and NUL-terminated child; a successful fd is owned once.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            child.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new owned fd.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// A name for something new in a folder: one path component, not `.` or `..`.
pub fn check_name(name: &str) -> io::Result<&str> {
    let name = name.trim();
    let bad = |why: &str| Err(io::Error::new(io::ErrorKind::InvalidInput, why.to_string()));
    if name.is_empty() {
        return bad("A name is needed");
    }
    if name.contains('/') || name.contains('\0') {
        return bad("A name cannot contain / (use Move to put it in another folder)");
    }
    if name == "." || name == ".." {
        return bad("That name is taken by the folder itself");
    }
    Ok(name)
}

/// What a conflict says: never "file exists", always which one.
fn exists(p: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", p.display()))
}

/// A new empty file `name` in `dir`; never one that is there.
pub fn create_file(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let p = dir.join(check_name(name)?);
    std::fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => exists(&p),
        _ => e,
    })?;
    Ok(p)
}

/// A new folder `name` in `dir`.
pub fn create_dir(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let p = dir.join(check_name(name)?);
    std::fs::create_dir(&p).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => exists(&p),
        _ => e,
    })?;
    Ok(p)
}

/// Rename or move `from` to `to` in one step, never over something there: a link moves as a
/// link, a folder with everything in it. `to` naming `from` in another case is a case-only
/// rename and is allowed. Across volumes it is refused (it would be a copy and a delete).
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::symlink_metadata(from)?;
    if to.starts_with(from) && to != from {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "A folder cannot go inside itself"));
    }
    if from == to {
        return Err(exists(to));
    }
    let (a, b) = (c_path(from)?, c_path(to)?);
    // SAFETY: two NUL-terminated paths living through the call.
    let r = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) };
    if r == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    Err(match e.raw_os_error() {
        Some(libc::EEXIST) => exists(to),
        Some(libc::EXDEV) => io::Error::other(format!(
            "{} is on another volume: copy it there (Duplicate) and move the original to the \
             Trash instead",
            to.parent().unwrap_or(to).display()
        )),
        _ => e,
    })
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static BEFORE_COPY: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

/// A copy of `from` (a folder with everything in it, links as links) at `to`, which must not
/// be there. Copy privately first, so failure cleanup can only trash our own copy.
pub fn duplicate(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_COPY: AtomicU64 = AtomicU64::new(0);

    let meta = std::fs::symlink_metadata(from)?;
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(exists(to));
    }
    let dir = to.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if meta.is_dir() && dir.canonicalize()?.starts_with(from.canonicalize()?) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "A folder cannot be copied inside itself"));
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(before) = BEFORE_COPY.take() {
        before(to);
    }
    let mut flags = libc::COPYFILE_METADATA | libc::COPYFILE_DATA | libc::COPYFILE_EXCL | libc::COPYFILE_NOFOLLOW;
    if meta.is_dir() {
        flags |= libc::COPYFILE_RECURSIVE;
    }
    let stage = loop {
        let path = dir.join(format!(".orlo-{}-{}.tmp", std::process::id(), NEXT_COPY.fetch_add(1, Ordering::Relaxed)));
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => break path,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let copy = stage.join("copy");
    let (a, b) = (c_path(from)?, c_path(&copy)?);
    // SAFETY: two NUL-terminated paths living through the call, no copyfile state.
    let r = unsafe { libc::copyfile(a.as_ptr(), b.as_ptr(), std::ptr::null_mut(), flags) };
    let result = match r {
        0 => rename(&copy, to),
        _ => Err(io::Error::last_os_error()),
    };
    if result.is_err() && std::fs::symlink_metadata(&copy).is_ok() {
        let _ = crate::filesystem::platform::trash(&copy);
    }
    let _ = std::fs::remove_dir(&stage);
    result
}

/// Finder's name for a copy: "a copy.txt", then "a copy 2.txt" … the first not taken.
pub fn copy_name(of: &Path) -> PathBuf {
    let dir = of.parent().unwrap_or(Path::new("/"));
    let stem = of.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = of.extension().map(|e| format!(".{}", e.to_string_lossy()));
    let ext = ext.unwrap_or_default();
    (1..)
        .map(|n| match n {
            1 => dir.join(format!("{stem} copy{ext}")),
            n => dir.join(format!("{stem} copy {n}{ext}")),
        })
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .unwrap_or_else(|| of.to_path_buf())
}

/// `p` after `from` became `to`: itself or something inside it moves along.
pub fn moved(p: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let rest = p.strip_prefix(from).ok()?;
    Some(match rest.as_os_str().is_empty() {
        true => to.to_path_buf(),
        false => to.join(rest),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        crate::io::resolve(&d)
    }

    #[test]
    fn new_files_and_folders_never_replace_what_is_there() {
        let d = temp("orlo-fsops-create");
        let f = create_file(&d, "a.txt").unwrap();
        std::fs::write(&f, "keep").unwrap();
        let e = create_file(&d, "a.txt").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(e.to_string().contains("a.txt already exists"), "{e}");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "keep");
        assert!(create_dir(&d, "sub").unwrap().is_dir());
        assert!(create_dir(&d, "sub").is_err());
        for bad in ["", "  ", "x/y", ".", ".."] {
            assert!(create_file(&d, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn renames_and_moves_refuse_collisions_and_take_case_only_renames() {
        let d = temp("orlo-fsops-rename");
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::write(d.join("b.txt"), "b").unwrap();
        let e = rename(&d.join("a.txt"), &d.join("b.txt")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "b", "untouched");
        // a case-only rename on this (case-insensitive) disk
        rename(&d.join("a.txt"), &d.join("A.txt")).unwrap();
        let names: Vec<_> =
            std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"A.txt".to_string()), "{names:?}");
        // a folder moves whole, not into itself
        std::fs::create_dir_all(d.join("dir/inner")).unwrap();
        std::fs::write(d.join("dir/inner/x"), "x").unwrap();
        assert!(rename(&d.join("dir"), &d.join("dir/inner/dir")).is_err());
        std::fs::create_dir(d.join("dest")).unwrap();
        rename(&d.join("dir"), &d.join("dest/dir")).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("dest/dir/inner/x")).unwrap(), "x");
        assert!(rename(&d.join("missing"), &d.join("x")).is_err());
    }

    #[test]
    fn renaming_onto_a_distinct_hard_link_is_a_collision() {
        let d = temp("orlo-fsops-hard-link-collision");
        let (from, to) = (d.join("source.txt"), d.join("linked.txt"));
        std::fs::write(&from, "keep").unwrap();
        std::fs::hard_link(&from, &to).unwrap();
        assert_eq!(rename(&from, &to).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "keep");
    }

    #[test]
    fn a_link_moves_and_copies_as_a_link_and_its_target_stays() {
        let d = temp("orlo-fsops-links");
        std::fs::create_dir_all(d.join("target/deep")).unwrap();
        std::fs::write(d.join("target/deep/f"), "f").unwrap();
        std::os::unix::fs::symlink(d.join("target"), d.join("link")).unwrap();
        rename(&d.join("link"), &d.join("link2")).unwrap();
        assert!(std::fs::symlink_metadata(d.join("link2")).unwrap().is_symlink());
        // a folder holding a link is copied with the link, not what it points to
        std::fs::create_dir(d.join("holder")).unwrap();
        std::os::unix::fs::symlink(d.join("target"), d.join("holder/l")).unwrap();
        duplicate(&d.join("holder"), &d.join("holder copy")).unwrap();
        assert!(std::fs::symlink_metadata(d.join("holder copy/l")).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(d.join("target/deep/f")).unwrap(), "f");
    }

    #[test]
    fn duplicates_copy_the_disk_and_get_finders_names() {
        let d = temp("orlo-fsops-dup");
        let a = d.join("a.txt");
        std::fs::write(&a, "on disk").unwrap();
        assert_eq!(copy_name(&a), d.join("a copy.txt"));
        duplicate(&a, &copy_name(&a)).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a copy.txt")).unwrap(), "on disk");
        assert_eq!(copy_name(&a), d.join("a copy 2.txt"));
        assert!(duplicate(&a, &d.join("a copy.txt")).is_err(), "never over a file");
        assert_eq!(copy_name(&d.join("Makefile")), d.join("Makefile copy"));
    }

    #[test]
    fn paths_inside_a_moved_folder_move_with_it() {
        let (f, t) = (Path::new("/a/b"), Path::new("/a/c"));
        assert_eq!(moved(Path::new("/a/b"), f, t), Some(t.to_path_buf()));
        assert_eq!(moved(Path::new("/a/b/x/y"), f, t), Some(PathBuf::from("/a/c/x/y")));
        assert_eq!(moved(Path::new("/a/bb"), f, t), None, "a sibling with a longer name");
    }

    #[test]
    fn duplicate_keeps_a_concurrent_destination() {
        let d = temp("orlo-fsops-copy-collision");
        let (from, to) = (d.join("source.txt"), d.join("destination.txt"));
        std::fs::write(&from, "original").unwrap();
        BEFORE_COPY.set(Some(|to| std::fs::write(to, "another writer's file").unwrap()));
        let error = duplicate(&from, &to).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "another writer's file");
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "original");
    }

    #[test]
    fn duplicate_keeps_destination_when_source_vanishes() {
        let d = temp("orlo-fsops-source-vanishes");
        let (from, to) = (d.join("source.txt"), d.join("destination.txt"));
        std::fs::write(&from, "original").unwrap();
        BEFORE_COPY.set(Some(|to| {
            std::fs::remove_file(to.parent().unwrap().join("source.txt")).unwrap();
            std::fs::write(to, "another writer's file").unwrap();
        }));
        assert!(duplicate(&from, &to).is_err());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "another writer's file");
    }

    #[test]
    fn duplicate_accepts_a_valid_long_destination_name() {
        let d = temp("orlo-fsops-long-copy");
        let from = d.join("a".repeat(245));
        std::fs::write(&from, "long name").unwrap();
        let to = copy_name(&from);
        duplicate(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "long name");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2);
    }

    #[test]
    fn duplicate_refuses_a_destination_inside_the_source() {
        let d = temp("orlo-fsops-recursive-copy");
        std::fs::write(d.join("keep"), "original").unwrap();
        let alias = d.with_extension("alias");
        std::os::unix::fs::symlink(&d, &alias).unwrap();
        for destination in [d.join("nested"), alias.join("nested")] {
            let error = duplicate(&d, &destination).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        }
        std::fs::remove_file(alias).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("keep")).unwrap(), "original");
    }
}
