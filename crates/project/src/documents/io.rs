//! Document loading and safe writes, independent of editor state.

use crate::watch::{stamp, Stamp};
use orlo_utils::text::{diff, Change};
use ropey::Rope;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// sys/acl.h and the filesec API; libc does not expose them.
#[cfg(target_os = "macos")]
extern "C" {
    fn acl_init(count: libc::c_int) -> *mut libc::c_void;
    fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    fn acl_set_fd(fd: libc::c_int, acl: *mut libc::c_void) -> libc::c_int;
    fn acl_set_link_np(path: *const libc::c_char, kind: libc::c_int, acl: *mut libc::c_void) -> libc::c_int;
    fn acl_get_flagset_np(acl: *mut libc::c_void, flags: *mut *mut libc::c_void) -> libc::c_int;
    fn acl_add_flag_np(flags: *mut libc::c_void, flag: libc::c_uint) -> libc::c_int;
    fn acl_set_flagset_np(acl: *mut libc::c_void, flags: *mut libc::c_void) -> libc::c_int;
    fn filesec_init() -> *mut libc::c_void;
    fn filesec_free(security: *mut libc::c_void);
    fn filesec_set_property(
        security: *mut libc::c_void,
        property: libc::c_int,
        value: *const libc::c_void,
    ) -> libc::c_int;
    fn openx_np(path: *const libc::c_char, flags: libc::c_int, security: *mut libc::c_void) -> libc::c_int;
}

/// Where a save to `path` must write: a symlink's target (the link stays a link), and for a
/// file not there yet, its name in the real folder.
pub fn resolve(path: &Path) -> PathBuf {
    path.canonicalize()
        .ok()
        .or_else(|| {
            let dir = path.parent()?.canonicalize().ok()?;
            Some(dir.join(path.file_name()?))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

/// Write `contents` so a failure (disk full, a dropped network share, power loss) never
/// leaves the file half written: a temp file beside it, synced, renamed over it. The temp
/// file starts empty and takes the old file's mode, extended attributes and ACL
/// (`copy_metadata`). Hard-linked files, files owned by someone else and files in folders
/// Orlo may not create files in are written in place instead, so their other names, owner
/// and group stay.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;
    write_safely(path, &|f| f.write_all(contents.as_bytes())).map_err(|(e, _)| e)
}

/// Write private workspace text without exposing it through the old file's permissions.
pub fn write_atomic_private(path: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;
    write_safely_with(path, &|f| f.write_all(contents.as_bytes()), true).map_err(|(e, _)| e)
}

/// Private session text, streamed without copying the whole rope into a string.
pub fn write_atomic_rope_private(path: &Path, rope: &Rope) -> io::Result<()> {
    use std::io::Write;
    let write = |f: &mut std::fs::File| rope.chunks().try_for_each(|c| f.write_all(c.as_bytes()));
    write_safely_with(path, &write, true).map_err(|(e, _)| e)
}

/// Keep raw recovery bytes private and publish only at an unused backup name.
pub fn copy_private(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    write_safely_to(
        to.to_path_buf(),
        &|file| {
            let mut source = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(from)?;
            if !source.metadata()?.is_file() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a regular file"));
            }
            std::io::copy(&mut source, file).map(drop)
        },
        true,
        &crate::fsops::rename,
    )
    .map_err(|(error, _)| error)
}

/// Secure an unreadable recovery file before moving it; never follow a leaf symlink.
pub fn make_private_file(path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a regular file"));
    }
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // Restrict the mode before removing ACL denies that may currently protect the file.
    // SAFETY: path is NUL-terminated and nofollow protects a replaced symlink's target.
    if unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), 0o600, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(target_os = "macos")]
    {
        // sys/acl.h; libc does not expose this native ACL kind.
        const ACL_TYPE_EXTENDED: libc::c_int = 0x100;
        // SAFETY: the empty native ACL and NUL-terminated path live through the call;
        // acl_set_link_np does not follow a replaced leaf symlink.
        set_empty_acl(|acl| unsafe { acl_set_link_np(path.as_ptr(), ACL_TYPE_EXTENDED, acl) })?;
    }
    Ok(())
}

/// Sets an empty native ACL through `set`, freeing it on every path.
#[cfg(target_os = "macos")]
fn set_empty_acl(set: impl FnOnce(*mut libc::c_void) -> libc::c_int) -> io::Result<()> {
    // SAFETY: the allocated empty ACL stays live through `set` and is freed once.
    unsafe {
        let acl = acl_init(0);
        if acl.is_null() {
            return Err(io::Error::last_os_error());
        }
        let result = match set(acl) {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        };
        acl_free(acl);
        result
    }
}

/// Restrict a session directory before creating any drafts inside it. Mode alone does not
/// remove macOS ACL entries inherited when the directory was created.
pub fn make_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(path)?;
    dir.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    clear_directory_acl(&dir)
}

pub(crate) fn clear_directory_acl(dir: &std::fs::File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: the directory fd and allocated empty ACL stay live through acl_set_fd.
        set_empty_acl(|acl| unsafe { acl_set_fd(dir.as_raw_fd(), acl) })?;
    }
    Ok(())
}

/// Preserve the validated original before a user-file overwrite; a raced capture aborts it.
pub fn capture_history(path: &Path, hint: crate::encoding::Encoding, enabled: bool) -> io::Result<()> {
    if !enabled {
        return Ok(());
    }
    use sha2::{Digest, Sha256};
    let target = resolve(path);
    match std::fs::symlink_metadata(&target) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    let before = stamp(&target);
    let read = || {
        read_bytes(&target)
            .map_err(|e| io::Error::other(e.unwrap_or_else(|| "File history supports files up to 100 MiB".into())))
    };
    let bytes = read()?;
    let encoding = crate::encoding::bom_encoding(&bytes).unwrap_or(hint);
    let hash = Sha256::digest(&bytes);
    drop(bytes);
    crate::file_history::capture(&target, encoding)?;
    if resolve(path) != target
        || stamp(&target) != before
        || Sha256::digest(read()?) != hash
        || stamp(&target) != before
    {
        return Err(io::Error::other("The file changed while its history was captured; save again"));
    }
    Ok(())
}

/// What a save writes into a file it has open.
type Writer<'a> = &'a dyn Fn(&mut std::fs::File) -> io::Result<()>;

/// `write_atomic`; a failure says too whether the file itself was written to (the in-place
/// way), so it is no longer what was last read. The temp file starts empty and takes only the
/// old file's metadata (`copy_metadata`): the old contents are never read.
pub fn write_safely(path: &Path, write: Writer) -> Result<(), (io::Error, bool)> {
    write_safely_with(path, write, false)
}

/// The same safe writer with a source check before each destructive step. The returned
/// stamp comes from the written file handle and is trusted only while the path still matches;
/// an external replacement at completion leaves it unknown. This is not a filesystem CAS.
pub fn write_safely_checked(
    path: &Path,
    write: Writer,
    check: &dyn Fn() -> io::Result<()>,
) -> Result<Option<Stamp>, (io::Error, bool)> {
    check().map_err(|e| (e, false))?;
    write_safely_to_checked(resolve(path), write, false, &|from, to| std::fs::rename(from, to), check)
}

pub fn write_safely_with(path: &Path, write: Writer, private: bool) -> Result<(), (io::Error, bool)> {
    write_safely_to(resolve(path), write, private, &|from, to| std::fs::rename(from, to))
}

pub fn write_safely_to(
    target: PathBuf,
    write: Writer,
    private: bool,
    publish: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), (io::Error, bool)> {
    write_safely_to_checked(target, write, private, publish, &|| Ok(())).map(drop)
}

fn write_safely_to_checked(
    target: PathBuf,
    write: Writer,
    private: bool,
    publish: &dyn Fn(&Path, &Path) -> io::Result<()>,
    check: &dyn Fn() -> io::Result<()>,
) -> Result<Option<Stamp>, (io::Error, bool)> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let meta = std::fs::metadata(&target).ok();
    // the file as it was looked at: one replaced since (a link swapped in, in a folder another
    // user can write) is refused rather than written through
    let identity = meta.as_ref().map(|m| (m.dev(), m.ino()));
    let in_place = || {
        check().map_err(|e| (e, false))?;
        write_in_place_stamped(&target, write, identity)
    };
    static N: AtomicU64 = AtomicU64::new(0);
    if !private && meta.as_ref().is_some_and(|m| m.nlink() > 1) {
        return in_place();
    }
    let mut tmp = temp_stem(target.file_name().unwrap_or_default());
    let n = N.fetch_add(1, Ordering::Relaxed);
    tmp.push(format!(".orlo-{}-{n}.tmp", std::process::id()));
    let tmp = target.with_file_name(tmp);
    #[cfg(any(test, feature = "test-support"))]
    if let Some(create) = BEFORE_CREATE.take() {
        create(&tmp);
    }
    let made = fault("copy").and_then(|()| {
        if private {
            fault("chmod")?;
            #[cfg(target_os = "macos")]
            return create_private(&tmp);
        }
        // the user's alone until the old file's mode is copied onto it: another user who opened
        // it before then could read what is written next, the mode notwithstanding
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(if private || meta.is_some() { 0o600 } else { 0o666 })
            .open(&tmp)
    });
    let mut f = match made {
        Ok(f) => f,
        Err(e) => {
            // nothing was created, so nothing is removed: on AlreadyExists the file is another's
            // a folder Orlo may not write in, or a read-only volume: the file may still be
            // writable; anything else (a full disk, an I/O error) would cut it short
            use io::ErrorKind::{PermissionDenied, ReadOnlyFilesystem};
            return match e.kind() {
                PermissionDenied | ReadOnlyFilesystem if !private => in_place(),
                _ => Err((e, false)),
            };
        }
    };
    #[cfg(any(test, feature = "test-support"))]
    if let Some(inspect) = AFTER_CREATE.take() {
        inspect(&tmp);
    }
    let mut write_tmp = || {
        if let Some(m) = meta.as_ref().filter(|_| !private) {
            copy_metadata(&target, &f, m);
            let mine = f.metadata()?;
            if mine.uid() != m.uid() {
                return Ok(None);
            }
            if mine.gid() != m.gid() && std::os::unix::fs::fchown(&f, None, Some(m.gid())).is_err() {
                return Ok(None);
            }
        }
        write(&mut f)?;
        fault("write")?;
        f.sync_all()?;
        check()?;
        let prepared = f.metadata().ok().map(|m| Stamp::of(&m));
        publish(&tmp, &target)?;
        let written = f.metadata().ok().map(|m| Stamp::of(&m));
        let written = written.filter(|s| prepared.is_some_and(|p| p.moved(s)));
        #[cfg(any(test, feature = "test-support"))]
        if let Some(replace) = AFTER_PUBLISH.take() {
            replace(&target);
        }
        // the rename itself survives a power loss once the folder is synced
        if let Some(dir) = target.parent() {
            let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
        }
        Ok(Some(written.filter(|s| stamp(&target) == Some(*s))))
    };
    match write_tmp() {
        Ok(Some(written)) => Ok(written),
        Ok(None) => {
            let _ = std::fs::remove_file(&tmp);
            in_place()
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err((e, false))
        }
    }
}

#[cfg(target_os = "macos")]
pub fn create_private(path: &Path) -> io::Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    const ACL_FLAG_NO_INHERIT: libc::c_uint = 1 << 17;
    const FILESEC_MODE: libc::c_int = 4;
    const FILESEC_ACL: libc::c_int = 5;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: all pointers live through their calls; native allocations are freed on every path,
    // and a successful exclusive open transfers its sole owned fd into File.
    unsafe {
        let acl = acl_init(0);
        if acl.is_null() {
            return Err(io::Error::last_os_error());
        }
        let security = filesec_init();
        if security.is_null() {
            let error = io::Error::last_os_error();
            acl_free(acl);
            return Err(error);
        }
        let result = (|| {
            let mut flags = std::ptr::null_mut();
            let mode: libc::mode_t = 0o600;
            // sys/acl.h and sys/fcntl.h: suppress inheritance in the same call that creates.
            if acl_get_flagset_np(acl, &mut flags) != 0
                || acl_add_flag_np(flags, ACL_FLAG_NO_INHERIT) != 0
                || acl_set_flagset_np(acl, flags) != 0
                || filesec_set_property(security, FILESEC_MODE, std::ptr::from_ref(&mode).cast()) != 0
                || filesec_set_property(security, FILESEC_ACL, std::ptr::from_ref(&acl).cast()) != 0
            {
                return Err(io::Error::last_os_error());
            }
            let fd = openx_np(path.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC, security);
            match fd {
                -1 => Err(io::Error::last_os_error()),
                _ => Ok(std::fs::File::from_raw_fd(fd)),
            }
        })();
        filesec_free(security);
        acl_free(acl);
        result
    }
}

/// The old file's mode, extended attributes and ACL onto a save's empty temp file, none of
/// its data; its mode alone when the rest cannot be copied, said on stderr. Not its flags or
/// times: a Finder lock (`uchg`) would lock the temp file before it is written.
pub fn copy_metadata(from: &Path, to: &std::fs::File, meta: &std::fs::Metadata) {
    if let Err(e) = copy_acl_and_xattrs(from, to) {
        eprintln!("Orlo: could not copy the attributes of {} ({e}); keeping its mode", from.display());
    }
    let _ = to.set_permissions(meta.permissions());
}

#[cfg(target_os = "macos")]
pub fn copy_acl_and_xattrs(from: &Path, to: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // a write-only file's extended attributes cannot be read, even by its owner; its ACL can,
    // through a descriptor open for writing (nothing is written)
    let (src, what) = match std::fs::File::open(from) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            (std::fs::OpenOptions::new().write(true).open(from)?, libc::COPYFILE_ACL)
        }
        src => (src?, libc::COPYFILE_ACL | libc::COPYFILE_XATTR),
    };
    // SAFETY: both descriptors belong to files open for the whole call (`src` here, `to`
    // borrowed from the caller); a null state is allowed; fcopyfile touches no memory of ours.
    let r = unsafe { libc::fcopyfile(src.as_raw_fd(), to.as_raw_fd(), std::ptr::null_mut(), what) };
    match r {
        0 if what == libc::COPYFILE_ACL => {
            Err(io::Error::other("it is write-only: its extended attributes cannot be read"))
        }
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

/// The Orlo that made `name` if it is a save's temp file (`name.orlo-PID-N.tmp`).
pub fn temp_pid(name: &str) -> Option<u32> {
    let (head, n) = name.strip_suffix(".tmp")?.rsplit_once('-')?;
    n.parse::<u64>().ok()?;
    head.rsplit_once(".orlo-")?.1.parse().ok()
}

/// Leave room for the suffix even when the target already uses a filesystem's full name limit.
fn temp_stem(name: &std::ffi::OsStr) -> std::ffi::OsString {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    if name.as_bytes().len() > 200 {
        format!(".orlo-save-{:x}", Sha256::digest(name.as_bytes())).into()
    } else {
        name.to_owned()
    }
}

/// The Orlo that made `name` if it is a copy's private staging folder (`.orlo-PID-N.tmp`).
fn stage_pid(name: &str) -> Option<u32> {
    let (pid, n) = (name.strip_prefix(".orlo-")?.strip_suffix(".tmp")?).split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(pid) && digits(n)).then(|| pid.parse().ok())?
}

/// Delete the temp files saves left beside `files` (a crash between the write and the
/// rename), and the staging folders copies left there, when the Orlo that made them is gone.
pub fn sweep_temp_files(files: &[PathBuf]) {
    let me = std::process::id();
    // only "No such process" is gone: another user's process (EPERM) is alive
    let alive = |pid: u32| {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return true;
        };
        // SAFETY: signal 0 only checks that `pid` (positive, never a group) exists.
        let gone = unsafe { libc::kill(pid, 0) } != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        !gone
    };
    let mut dirs: HashMap<&Path, Vec<String>> = HashMap::new();
    for f in files {
        if let (Some(dir), Some(name)) = (f.parent(), f.file_name()) {
            let stem = temp_stem(name);
            let name = stem.to_string_lossy() + ".orlo-";
            dirs.entry(dir).or_default().push(name.into_owned());
        }
    }
    for (dir, prefixes) in dirs {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ours = prefixes.iter().any(|p| name.starts_with(p.as_str()));
            if let Some(pid) = temp_pid(&name).filter(|&p| ours && p != me && !alive(p)) {
                eprintln!("Orlo: removing {} left by Orlo {pid}", e.path().display());
                let _ = std::fs::remove_file(e.path());
            }
            // a real folder, never a link; removal does not follow links inside it
            let folder = e.file_type().is_ok_and(|t| t.is_dir());
            if let Some(pid) = stage_pid(&name).filter(|&p| folder && p != me && !alive(p)) {
                eprintln!("Orlo: removing {} left by Orlo {pid}", e.path().display());
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// The file truncated and written; once it is open, a failure has touched it.
pub fn write_in_place(path: &Path, write: Writer) -> Result<(), (io::Error, bool)> {
    write_in_place_stamped(path, write, None).map(drop)
}

/// Over the file itself: opened without following a link, and without truncating it until it is
/// still the one at `identity` (device, inode) when there was one.
fn write_in_place_stamped(
    path: &Path,
    write: Writer,
    identity: Option<(u64, u64)>,
) -> Result<Option<Stamp>, (io::Error, bool)> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(identity.is_none())
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| (e, false))?;
    let opened = f.metadata().map_err(|e| (e, false))?;
    if identity.is_some_and(|id| id != (opened.dev(), opened.ino())) {
        let error = io::Error::other("The file was replaced while saving; save again to write the new one");
        return Err((error, false));
    }
    f.set_len(0).map_err(|e| (e, true))?;
    write(&mut f).and_then(|()| fault("write")).and_then(|()| f.sync_all()).map_err(|e| (e, true))?;
    let written = f.metadata().ok().map(|m| Stamp::of(&m));
    #[cfg(any(test, feature = "test-support"))]
    if let Some(replace) = AFTER_PUBLISH.take() {
        replace(path);
    }
    Ok(written.filter(|s| stamp(path) == Some(*s)))
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// A write another app makes to a file while `Loaded::read` reads it (between its stat
    /// and its read).
    pub static MID_READ: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// A save step ("copy", "write") that fails in this test, and how.
    pub static FAULT: std::cell::Cell<Option<(&'static str, io::ErrorKind)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// An external writer replacing a just-published file before completion observes it.
    pub static AFTER_PUBLISH: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
    /// A competing file created just before the save tries its exclusive temp-file create.
    pub static BEFORE_CREATE: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
    /// Inspect creation-time permissions before the writer can adjust them.
    pub static AFTER_CREATE: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

/// Tests make a step of a save fail (`FAULT`); nothing otherwise.
pub fn fault(step: &str) -> io::Result<()> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some((s, kind)) = FAULT.get() {
        if s == step {
            return Err(kind.into());
        }
    }
    let _ = step;
    Ok(())
}

/// `text` (LF line breaks, as a buffer holds it) as a file holds it: the line ending applied
/// and a byte order mark first.
pub fn file_text(mut text: String, line_ending: &str, bom: bool) -> String {
    if line_ending != "\n" {
        text = text.replace('\n', line_ending);
    }
    if bom {
        text.insert(0, '\u{feff}');
    }
    text
}

/// Arbitrary-text normalization used by clipboard and transformations. CRLF becomes LF;
/// lone CR stays literal in LF/CRLF-majority text. Disk sources use endings::decoded instead.
pub fn normalized(text: &str) -> (std::borrow::Cow<'_, str>, &'static str) {
    let mut out = String::new();
    let ending = normalize(text, |piece| {
        if out.capacity() == 0 {
            out.reserve(text.len());
        }
        out.push_str(piece);
    });
    match ending {
        Some(ending) => (out.into(), ending),
        None => (text.into(), "\n"),
    }
}

/// `text` in pieces to `push`, its line breaks as a buffer holds them, and the line ending a
/// save would write; `None`, pushing nothing, when it already holds them so. The majority
/// ending wins; CRLF always becomes a line break (a kept one would put its CR mid-line once
/// edited), and lone CRs in an LF or CRLF file stay content.
pub(crate) fn normalize(text: &str, mut push: impl FnMut(&str)) -> Option<&'static str> {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let cr = text.matches('\r').count() - crlf;
    if crlf == 0 && cr <= lf {
        return None;
    }
    if cr > lf && cr > crlf {
        // a CR file: every CR and CRLF is a line break (a kept CRLF would save as two CRs)
        let mut pieces = text.split('\r');
        push(pieces.next().unwrap_or(""));
        for p in pieces {
            if !p.starts_with('\n') {
                push("\n");
            }
            push(p);
        }
        return Some("\r");
    }
    let mut pieces = text.split("\r\n");
    push(pieces.next().unwrap_or(""));
    for p in pieces {
        push("\n");
        push(p);
    }
    Some(if crlf > lf && crlf >= cr { "\r\n" } else { "\n" })
}

/// A file as read from disk, not yet a buffer. It is `Send`, so the read and the diff against
/// a buffer can run off the UI thread.
pub struct Loaded {
    pub rope: Rope,
    pub endings: Option<super::endings::LineEndings>,
    pub line_ending: &'static str,
    /// Malformed text, decoded lossily: `save` must not overwrite the original.
    pub read_only: bool,
    pub disk: Option<Stamp>,
    pub bom: bool,
    pub encoding: crate::encoding::Encoding,
}

impl Loaded {
    pub fn read_with_hint(path: &Path, encoding: Option<crate::encoding::Encoding>) -> io::Result<Self> {
        Self::read_using(path, encoding, false, None)
    }

    pub fn read(path: &Path) -> io::Result<Self> {
        Self::read_encoding(path, None)
    }

    pub fn read_encoding(path: &Path, encoding: Option<crate::encoding::Encoding>) -> io::Result<Self> {
        Self::read_using(path, encoding, true, None)
    }

    pub(crate) fn read_with_identity(
        path: &Path,
        encoding: Option<crate::encoding::Encoding>,
        identity: (u64, u64),
    ) -> io::Result<Self> {
        Self::read_using(path, encoding, false, Some(identity))
    }

    fn read_using(
        path: &Path,
        encoding: Option<crate::encoding::Encoding>,
        explicit: bool,
        identity: Option<(u64, u64)>,
    ) -> io::Result<Self> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;

        // stamped before the read: a write during it shows up as a change at the next poll
        let disk = stamp(path);
        #[cfg(any(test, feature = "test-support"))]
        if let Some(write) = MID_READ.take() {
            write(path);
        }
        // Check the opened file itself; a path can become a pipe between stat and open.
        let mut file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a regular file"));
        }
        if let Some(identity) = identity {
            use std::os::unix::fs::MetadataExt;
            if (metadata.dev(), metadata.ino()) != identity {
                return Err(io::Error::other("File was replaced before reading; browse it again"));
            }
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if identity.is_some() && Some(Stamp::of(&file.metadata()?)) != disk {
            return Err(io::Error::other("File changed while reading; browse it again"));
        }
        // Remembered encodings describe BOM-less files; an external writer may change the BOM.
        let encoding = if explicit { encoding } else { crate::encoding::bom_encoding(&bytes).or(encoding) };
        let decoded = crate::encoding::decode_owned(bytes, encoding);
        let (rope, line_ending, endings) = super::endings::decoded(&decoded.text);
        Ok(Self {
            rope,
            endings: Some(endings),
            line_ending,
            read_only: decoded.lossy,
            disk,
            bom: decoded.bom,
            encoding: decoded.encoding,
        })
    }

    /// The edits that turn `rope` into this text, for `reload_from`.
    pub fn diff(&self, rope: &Rope) -> Vec<Change> {
        diff(rope, &self.rope)
    }
}

/// Larger files are listed as "File too large, skipping".
pub const MAX_SIZE: u64 = 100 << 20;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static BEFORE_READ: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

/// A regular file's bytes, bounded even if it grows after opening. `Err(None)` means too
/// large; `Err(Some(why))` means it cannot be read.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>, Option<String>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| Some(e.to_string()))?;
    let meta = file.metadata().map_err(|e| Some(e.to_string()))?;
    if !meta.is_file() {
        return Err(Some("Not a regular file".into()));
    }
    if meta.len() > MAX_SIZE {
        return Err(None);
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(grow) = BEFORE_READ.take() {
        grow(path);
    }
    let mut bytes = Vec::new();
    file.take(MAX_SIZE + 1).read_to_end(&mut bytes).map_err(|e| Some(e.to_string()))?;
    if bytes.len() as u64 > MAX_SIZE {
        return Err(None);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_save_rechecks_atomic_publication_and_in_place_fallback() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("orlo-guarded-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("source");
        std::fs::write(&path, "old").unwrap();
        let before = stamp(&path);
        let check = || if stamp(&path) == before { Ok(()) } else { Err(io::Error::other("changed")) };
        let failed = write_safely_checked(
            &path,
            &|file| {
                file.write_all(b"mine")?;
                std::fs::write(&path, "external")
            },
            &check,
        )
        .unwrap_err();
        assert!(!failed.1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::write(&path, "old").unwrap();
        let before = stamp(&path);
        BEFORE_CREATE.set(Some(|tmp| {
            std::fs::write(tmp.parent().unwrap().join("source"), "fallback external").unwrap();
        }));
        FAULT.set(Some(("copy", io::ErrorKind::PermissionDenied)));
        let failed = write_safely_checked(&path, &|f| f.write_all(b"mine"), &|| {
            if stamp(&path) == before {
                Ok(())
            } else {
                Err(io::Error::other("changed"))
            }
        })
        .unwrap_err();
        assert!(!failed.1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fallback external");
        let alias = dir.join("alias");
        std::fs::hard_link(&path, &alias).unwrap();
        let calls = std::cell::Cell::new(0);
        let before = stamp(&path);
        let failed = write_safely_checked(&path, &|f| f.write_all(b"mine"), &|| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                std::fs::write(&alias, "hardlink external")?;
            }
            if stamp(&path) == before {
                Ok(())
            } else {
                Err(io::Error::other("changed"))
            }
        })
        .unwrap_err();
        assert!(!failed.1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hardlink external");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_private_files_save_is_never_readable_by_others() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("orlo-io-mode-{}", std::process::id()));
        std::fs::write(&path, "secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        // before the old file's mode is copied on: at once the user's alone
        AFTER_CREATE.set(Some(|tmp| {
            assert_eq!(std::fs::metadata(tmp).unwrap().permissions().mode() & 0o777, 0o600);
        }));
        write_safely(&path, &|f| f.write_all(b"new secret")).unwrap();
        assert!(AFTER_CREATE.take().is_none(), "the temp file was made");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_in_place_save_never_writes_a_file_swapped_in_after_the_check() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("orlo-io-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (target, other) = (dir.join("shared.txt"), dir.join("victim.txt"));
        std::fs::write(&target, "shared").unwrap();
        std::fs::write(&other, "victim").unwrap();
        // a second link keeps the save in place
        std::fs::hard_link(&target, dir.join("link")).unwrap();
        for swap in [0, 1] {
            // the first check is before the path is resolved (a link there is the user's
            // file); the swap comes between the file's look and its opening in place
            let calls = std::cell::Cell::new(0);
            let check = || {
                calls.set(calls.get() + 1);
                if calls.get() != 2 {
                    return Ok(());
                }
                let _ = std::fs::remove_file(&target);
                match swap {
                    0 => std::os::unix::fs::symlink(&other, &target),
                    _ => std::fs::hard_link(&other, &target),
                }
            };
            let result = write_safely_checked(&target, &|f| f.write_all(b"attacker"), &check);
            assert!(result.is_err(), "swap {swap} written through");
            assert_eq!(std::fs::read_to_string(&other).unwrap(), "victim");
            let _ = std::fs::remove_file(&target);
            std::fs::hard_link(dir.join("link"), &target).unwrap();
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn atomic_saves_accept_long_valid_file_names() {
        let dir = std::env::temp_dir().join(format!("orlo-long-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a".repeat(245));
        std::fs::write(&path, "old").unwrap();
        write_atomic(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn staging_folders_a_dead_orlo_left_are_swept_and_others_kept() {
        let dir = std::env::temp_dir().join(format!("orlo-stage-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("kept"), "").unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, "").unwrap();
        // a PID no process has (macOS stops below 100000), launchd's, and this process's
        let dead = dir.join(".orlo-99999-1.tmp");
        std::fs::create_dir_all(dead.join("copy")).unwrap();
        std::os::unix::fs::symlink(&outside, dead.join("copy/link")).unwrap();
        let theirs = dir.join(".orlo-1-2.tmp");
        let live = dir.join(format!(".orlo-{}-3.tmp", std::process::id()));
        let named = dir.join(".orlo-+99999-4.tmp");
        for p in [&theirs, &live, &named] {
            std::fs::create_dir(p).unwrap();
        }
        let link = dir.join(".orlo-99999-5.tmp");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert_eq!(stage_pid(".orlo-99999-1.tmp"), Some(99999));
        assert_eq!(stage_pid("a.txt.orlo-99999-1.tmp"), None);
        sweep_temp_files(std::slice::from_ref(&f));
        assert!(!dead.exists());
        assert!(theirs.exists() && live.exists() && named.exists() && f.exists());
        assert!(std::fs::symlink_metadata(&link).is_ok());
        assert!(outside.join("kept").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
