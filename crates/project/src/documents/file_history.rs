//! Opt-in snapshots of original file bytes, separate from session recovery.

use crate::comparison::identity;
use crate::encoding::Encoding;
use crate::fsops::{c_path, open_at};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_VERSIONS: usize = 20;
pub const MAX_TOTAL_BYTES: u64 = 200 << 20;
pub const MAX_FILE_BYTES: u64 = 100 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    pub created_ms: u64,
    pub encoding: Encoding,
    pub bytes: u64,
}

pub struct Record {
    pub version: Version,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Limits {
    versions: usize,
    total: u64,
    file: u64,
}

pub fn root() -> PathBuf {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(root) = TEST_ROOT.with_borrow(Clone::clone) {
        return root;
    }
    let support = orlo_utils::paths::support_dir();
    support.canonicalize().unwrap_or(support).join("File History")
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The synchronous calls in this thread use a disposable root; nested scopes and panics
/// restore the prior root. Background writer tests enter this scope on their worker thread.
#[cfg(any(test, feature = "test-support"))]
pub fn with_root<T>(root: PathBuf, run: impl FnOnce() -> T) -> T {
    let name = root.file_name().map(OsStr::to_owned);
    let root = match (root.parent().and_then(|p| p.canonicalize().ok()), name) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => root,
    };
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_ROOT.set(self.0.take());
        }
    }
    let _restore = Restore(TEST_ROOT.replace(Some(root)));
    run()
}

/// Call only for an enabled save, against the writer's validated, resolved target. A failure
/// must abort that overwrite; the writer revalidates its original bytes after this returns.
pub fn capture(path: &Path, encoding: Encoding) -> io::Result<()> {
    capture_in(&root(), path, encoding)
}

pub fn capture_in(root: &Path, path: &Path, encoding: Encoding) -> io::Result<()> {
    capture_with_limits(
        root,
        path,
        encoding,
        Limits { versions: MAX_VERSIONS, total: MAX_TOTAL_BYTES, file: MAX_FILE_BYTES },
    )
}

fn capture_with_limits(root: &Path, path: &Path, encoding: Encoding, limits: Limits) -> io::Result<()> {
    let mut source = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let before = source.metadata()?;
    if !before.is_file() {
        return Err(invalid("File history requires a regular file"));
    }
    if before.len() > limits.file {
        return Err(invalid("File history supports files up to 100 MiB; disable file_history to save this file"));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&mut source).take(limits.file + 1).read_to_end(&mut bytes)?;
    let after = source.metadata()?;
    if bytes.len() as u64 > limits.file || identity(&before) != identity(&after) || bytes.len() as u64 != before.len() {
        return Err(invalid("The file changed while its history was captured; save again"));
    }
    let key = path_key(path)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let store = Store::open(root, true)?.expect("created history directory");
    store.remove_where(|name| name.strip_prefix(".tmp-").is_some_and(valid_id))?;
    let mut records = store.records(&key)?;
    if let Some(record) = records.iter().find(|r| {
        r.meta.key == key && r.meta.hash == hash && r.meta.encoding == encoding && r.meta.bytes == bytes.len() as u64
    }) {
        store.read_record(&record.id)?;
        return Ok(());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?;
    let id = format!("{key}-{:032x}-{:016x}.hist", now.as_nanos(), NEXT.fetch_add(1, Ordering::Relaxed));
    let meta = Metadata {
        version: 1,
        key: key.clone(),
        created_ms: now.as_millis().try_into().map_err(io::Error::other)?,
        encoding,
        bytes: bytes.len() as u64,
        hash,
    };
    let header = serde_json::to_vec(&meta)?;
    let new_size = MAGIC.len() as u64 + 4 + header.len() as u64 + bytes.len() as u64;
    if new_size > limits.total {
        return Err(invalid("This snapshot exceeds the file history storage limit"));
    }
    let temporary = format!(".tmp-{id}");
    let result = (|| {
        let mut file = store.open_file(&temporary, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        file.write_all(MAGIC)?;
        file.write_all(&(header.len() as u32).to_le_bytes())?;
        file.write_all(&header)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        let mut count = records.iter().filter(|r| r.meta.key == key).count();
        let mut total: u64 = records.iter().map(|r| r.disk_bytes).sum();
        records.sort_by(|a, b| a.order().cmp(&b.order()));
        for record in &records {
            if (record.meta.key == key && count >= limits.versions) || total + new_size > limits.total {
                store.remove(&record.id)?;
                total -= record.disk_bytes;
                if record.meta.key == key {
                    count -= 1;
                }
            }
        }
        let (from, to) = (c_path(Path::new(&temporary))?, c_path(Path::new(&id))?);
        // SAFETY: both names are validated leaf names and the live directory fd anchors them.
        if unsafe { libc::renameat(store.dir.as_raw_fd(), from.as_ptr(), store.dir.as_raw_fd(), to.as_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        store.dir.sync_all()
    })();
    if result.is_err() {
        let _ = store.remove(&temporary);
    }
    result
}

pub fn list_in(root: &Path, path: &Path) -> io::Result<Vec<Version>> {
    let key = path_key(path)?;
    let Some(store) = Store::open(root, false)? else {
        return Ok(Vec::new());
    };
    let mut records = store.records_of(&key)?;
    records.sort_by(|a, b| b.order().cmp(&a.order()));
    Ok(records.into_iter().map(|r| r.version()).collect())
}

pub fn read_in(root: &Path, path: &Path, id: &str) -> io::Result<Record> {
    let key = path_key(path)?;
    if !valid_id(id) || !id.starts_with(&format!("{key}-")) {
        return Err(invalid("This history version belongs to a different file"));
    }
    let store =
        Store::open(root, false)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "File history was cleared"))?;
    store.read_record(id)
}

pub fn clear_in(root: &Path, path: &Path) -> io::Result<()> {
    let prefix = format!("{}-", path_key(path)?);
    let Some(store) = Store::open(root, false)? else {
        return Ok(());
    };
    store.remove_where(|name| {
        let id = name.strip_prefix(".tmp-").unwrap_or(name);
        valid_id(id) && id.starts_with(&prefix)
    })?;
    store.dir.sync_all()
}

const MAGIC: &[u8] = b"ORLOH1\n";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u8,
    key: String,
    created_ms: u64,
    encoding: Encoding,
    bytes: u64,
    hash: String,
}

struct Stored {
    id: String,
    meta: Metadata,
    disk_bytes: u64,
}

impl Stored {
    fn version(&self) -> Version {
        Version {
            id: self.id.clone(),
            created_ms: self.meta.created_ms,
            encoding: self.meta.encoding,
            bytes: self.meta.bytes,
        }
    }

    fn order(&self) -> (u64, &str) {
        (self.meta.created_ms, &self.id[65..])
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    Ok(if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) })
}

fn path_key(path: &Path) -> io::Result<String> {
    let absolute = absolute(path)?;
    let parent = absolute.parent().ok_or_else(|| invalid("A file history path needs a parent"))?;
    let parent = match parent.canonicalize() {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => normalized_path(parent)?,
        Err(error) => return Err(error),
    };
    let name = absolute.file_name().ok_or_else(|| invalid("A file history path needs a file name"))?;
    Ok(format!("{:x}", Sha256::digest(parent.join(name).as_os_str().as_bytes())))
}

fn valid_id(id: &str) -> bool {
    id.len() == 119
        && id.as_bytes()[64] == b'-'
        && id.as_bytes()[97] == b'-'
        && id.ends_with(".hist")
        && id[..64].bytes().all(|b| b.is_ascii_hexdigit())
        && id[65..97].bytes().all(|b| b.is_ascii_hexdigit())
        && id[98..114].bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_header(file: &mut File, id: &str) -> io::Result<Stored> {
    let mut magic = [0; 7];
    file.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(invalid("Not an Orlo file history snapshot"));
    }
    let mut length = [0; 4];
    file.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 4096 {
        return Err(invalid("The history snapshot metadata is too large"));
    }
    let mut header = vec![0; length];
    file.read_exact(&mut header)?;
    let meta: Metadata = serde_json::from_slice(&header)?;
    let disk_bytes = file.metadata()?.len();
    if meta.version != 1
        || meta.key != id[..64]
        || meta.bytes > MAX_FILE_BYTES
        || meta.hash.len() != 64
        || !meta.hash.bytes().all(|b| b.is_ascii_hexdigit())
        || disk_bytes != MAGIC.len() as u64 + 4 + length as u64 + meta.bytes
    {
        return Err(invalid("The history snapshot metadata is invalid or incomplete"));
    }
    Ok(Stored { id: id.to_string(), meta, disk_bytes })
}

fn normalized_path(path: &Path) -> io::Result<PathBuf> {
    Ok(crate::paths::folded(&absolute(path)?))
}

fn directory(path: &Path, create: bool) -> io::Result<Option<File>> {
    let clean = normalized_path(path)?;
    let mut dir = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")?;
    for part in clean.components().filter_map(|part| match part {
        std::path::Component::Normal(name) => Some(name),
        _ => None,
    }) {
        match open_at(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY) {
            Ok(next) => dir = next,
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                let name = c_path(Path::new(part))?;
                // SAFETY: the parent fd and leaf name are live; mkdirat never follows that leaf.
                if unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) } != 0
                    && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
                {
                    return Err(io::Error::last_os_error());
                }
                dir = open_at(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(dir))
}

struct Store {
    dir: File,
    _lock: File,
    _held: std::sync::MutexGuard<'static, ()>,
}

/// One store open at a time in this process. The flock keeps other processes out, but two
/// saves creating `.lock` at once can see `openat` fail with ENOENT.
static OPEN: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Store {
    fn open(root: &Path, create: bool) -> io::Result<Option<Self>> {
        let held = OPEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(dir) = directory(root, create)? else {
            return Ok(None);
        };
        let meta = dir.metadata()?;
        // SAFETY: geteuid takes no arguments and cannot fail.
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(invalid("File history storage must be a private directory owned by this user"));
        }
        dir.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        super::io::clear_directory_acl(&dir)?;
        let lock = open_at(&dir, OsStr::new(".lock"), libc::O_RDWR | libc::O_CREAT)?;
        private_file(&lock)?;
        // SAFETY: lock is a live fd. Holding it serializes snapshots and retention across saves.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(Self { dir, _lock: lock, _held: held }))
    }

    fn open_file(&self, name: &str, flags: libc::c_int) -> io::Result<File> {
        let file = open_at(&self.dir, OsStr::new(name), flags)?;
        private_file(&file)?;
        Ok(file)
    }

    fn remove(&self, name: &str) -> io::Result<()> {
        let name = c_path(Path::new(name))?;
        // SAFETY: the live directory fd anchors the leaf; unlinkat never follows a symlink.
        if unsafe { libc::unlinkat(self.dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn read_record(&self, id: &str) -> io::Result<Record> {
        let mut file = self.open_file(id, libc::O_RDONLY)?;
        let stored = read_header(&mut file, id)?;
        let mut bytes = Vec::with_capacity(stored.meta.bytes as usize);
        file.take(stored.meta.bytes + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != stored.meta.bytes || format!("{:x}", Sha256::digest(&bytes)) != stored.meta.hash {
            return Err(invalid("The history snapshot is incomplete or corrupt"));
        }
        Ok(Record { version: stored.version(), bytes })
    }

    /// Removes the private files whose names `matches` accepts.
    fn remove_where(&self, matches: impl Fn(&str) -> bool) -> io::Result<()> {
        for name in self.names()?.iter().filter(|name| matches(name.as_str())) {
            self.open_file(name, libc::O_RDONLY)?;
            self.remove(name)?;
        }
        Ok(())
    }

    /// The directory's UTF-8 names; no record or temporary name is anything else.
    fn names(&self) -> io::Result<Vec<String>> {
        let file = open_at(&self.dir, OsStr::new("."), libc::O_RDONLY | libc::O_DIRECTORY)?;
        let mut stream = orlo_utils::directory::Directory::new(file)?;
        let mut names = Vec::new();
        while let Some(name) = stream.next_name()? {
            if let Ok(name) = name.to_str() {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }

    fn ids(&self) -> io::Result<impl Iterator<Item = String>> {
        Ok(self.names()?.into_iter().filter(|name| valid_id(name)))
    }

    fn header(&self, id: &str) -> io::Result<Stored> {
        read_header(&mut self.open_file(id, libc::O_RDONLY)?, id)
    }

    /// The records of `key`; an unreadable one is an error.
    fn records_of(&self, key: &str) -> io::Result<Vec<Stored>> {
        let prefix = format!("{key}-");
        self.ids()?.filter(|id| id.starts_with(&prefix)).map(|id| self.header(&id)).collect()
    }

    /// Every record; another file's unreadable one is skipped, so it cannot block this save.
    fn records(&self, key: &str) -> io::Result<Vec<Stored>> {
        let prefix = format!("{key}-");
        let mut records = Vec::new();
        for id in self.ids()? {
            match self.header(&id) {
                Ok(record) => records.push(record),
                Err(error) if id.starts_with(&prefix) => return Err(error),
                Err(_) => {}
            }
        }
        Ok(records)
    }
}

fn private_file(file: &File) -> io::Result<()> {
    let meta = file.metadata()?;
    // SAFETY: geteuid takes no arguments and cannot fail.
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 || meta.nlink() != 1 {
        return Err(invalid("A file history record is not a private regular file"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "orlo-file-history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn root(&self) -> PathBuf {
            self.0.join("history")
        }

        fn source(&self, bytes: &[u8]) -> PathBuf {
            let path = self.0.join("document.txt");
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn raw_bytes_and_bomless_encoding_round_trip_in_private_atomic_records() {
        let d = Temp::new();
        let bytes = b"c\0a\0f\0\xe9\0\r\0\n\0";
        let path = d.source(bytes);
        capture_in(&d.root(), &path, Encoding::Utf16Le).unwrap();
        let versions = list_in(&d.root(), &path).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].encoding, Encoding::Utf16Le);
        assert_eq!(versions[0].bytes, bytes.len() as u64);
        let record = read_in(&d.root(), &path, &versions[0].id).unwrap();
        assert_eq!(record.bytes, bytes);
        assert_eq!(record.version, versions[0]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::metadata(d.root()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(d.root().join(&versions[0].id)).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(std::fs::read_dir(d.root())
            .unwrap()
            .all(|entry| { !entry.unwrap().file_name().to_string_lossy().starts_with(".tmp-") }));
    }

    #[test]
    fn duplicate_bytes_are_skipped_but_the_encoding_is_part_of_a_version() {
        let d = Temp::new();
        let path = d.source(b"a\0");
        capture_in(&d.root(), &path, Encoding::Utf16Le).unwrap();
        capture_in(&d.root(), &path, Encoding::Utf16Le).unwrap();
        std::fs::write(&path, b"b\0").unwrap();
        capture_in(&d.root(), &path, Encoding::Utf16Le).unwrap();
        std::fs::write(&path, b"a\0").unwrap();
        capture_in(&d.root(), &path, Encoding::Utf16Le).unwrap();
        assert_eq!(list_in(&d.root(), &path).unwrap().len(), 2);
        capture_in(&d.root(), &path, Encoding::Utf16Be).unwrap();
        assert_eq!(list_in(&d.root(), &path).unwrap().len(), 3);
    }

    #[test]
    fn path_identity_survives_atomic_replacement_and_non_utf8_names() {
        let d = Temp::new();
        let path = d.0.join("document-café.txt");
        std::fs::write(&path, b"first").unwrap();
        std::fs::create_dir(d.0.join("child")).unwrap();
        // APFS rejects non-UTF-8 leaves; the identity calculation must still preserve their bytes.
        let non_utf8 = d.0.join(OsString::from_vec(b"document-\xff.txt".to_vec()));
        let other = d.0.join(OsString::from_vec(b"document-\xfe.txt".to_vec()));
        assert_ne!(path_key(&non_utf8).unwrap(), path_key(&other).unwrap());
        assert_eq!(
            path_key(&non_utf8).unwrap(),
            path_key(&d.0.join("child/../").join(non_utf8.file_name().unwrap())).unwrap()
        );
        capture_in(&d.root(), &d.0.join("child/../").join(path.file_name().unwrap()), Encoding::Utf8).unwrap();
        let replacement = d.0.join("replacement");
        std::fs::write(&replacement, b"second").unwrap();
        std::fs::rename(replacement, &path).unwrap();
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        let versions = list_in(&d.root(), &path).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(read_in(&d.root(), &path, &versions[0].id).unwrap().bytes, b"second");
        assert_eq!(read_in(&d.root(), &path, &versions[1].id).unwrap().bytes, b"first");
    }

    #[test]
    fn keeps_only_the_twenty_newest_versions_of_each_file() {
        let d = Temp::new();
        let path = d.source(b"initial");
        for n in 0..24 {
            std::fs::write(&path, n.to_string()).unwrap();
            capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        }
        let versions = list_in(&d.root(), &path).unwrap();
        assert_eq!(versions.len(), MAX_VERSIONS);
        assert_eq!(read_in(&d.root(), &path, &versions[0].id).unwrap().bytes, b"23");
        assert_eq!(read_in(&d.root(), &path, &versions[19].id).unwrap().bytes, b"4");
    }

    #[test]
    fn global_budget_evicts_the_oldest_records_across_paths() {
        let d = Temp::new();
        let limits = Limits { versions: MAX_VERSIONS, total: 900, file: 100 };
        let mut paths = Vec::new();
        for n in 0..6 {
            let path = d.0.join(format!("file-{n}"));
            std::fs::write(&path, vec![n; 100]).unwrap();
            capture_with_limits(&d.root(), &path, Encoding::Utf8, limits).unwrap();
            paths.push(path);
        }
        let total: u64 =
            std::fs::read_dir(d.root()).unwrap().map(|entry| entry.unwrap().metadata().unwrap().len()).sum();
        assert!(total <= limits.total, "history used {total} bytes");
        assert!(list_in(&d.root(), &paths[0]).unwrap().is_empty());
        assert_eq!(list_in(&d.root(), &paths[5]).unwrap().len(), 1);
    }

    #[test]
    fn refuses_oversize_sources_without_creating_history() {
        let d = Temp::new();
        let path = d.source(b"");
        std::fs::File::options().write(true).open(&path).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        let error = capture_in(&d.root(), &path, Encoding::Utf8).unwrap_err();
        assert!(error.to_string().contains("100 MiB"));
        assert!(!d.root().exists());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_FILE_BYTES + 1);
    }

    #[test]
    fn absent_source_and_absent_history_have_no_storage_side_effects() {
        let d = Temp::new();
        let path = d.0.join("absent");
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        assert!(list_in(&d.root(), &path).unwrap().is_empty());
        clear_in(&d.root(), &path).unwrap();
        assert!(!d.root().exists());
    }

    #[test]
    fn refuses_symlink_sources_roots_and_records_without_touching_the_targets() {
        let d = Temp::new();
        let path = d.source(b"original");
        let source_link = d.0.join("source-link");
        symlink(&path, &source_link).unwrap();
        assert!(capture_in(&d.root(), &source_link, Encoding::Utf8).is_err());
        let outside = d.0.join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, d.root()).unwrap();
        assert!(capture_in(&d.root(), &path, Encoding::Utf8).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        std::fs::remove_file(d.root()).unwrap();
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        let version = list_in(&d.root(), &path).unwrap().remove(0);
        let stored = d.root().join(&version.id);
        std::fs::remove_file(&stored).unwrap();
        symlink(&path, &stored).unwrap();
        assert!(read_in(&d.root(), &path, &version.id).is_err());
        assert!(list_in(&d.root(), &path).is_err());
        assert!(capture_in(&d.root(), &path, Encoding::Utf8).is_err());
        assert!(clear_in(&d.root(), &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn refuses_insecure_storage_and_hardlinked_records() {
        let d = Temp::new();
        let path = d.source(b"original");
        std::fs::create_dir(d.root()).unwrap();
        std::fs::set_permissions(d.root(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(capture_in(&d.root(), &path, Encoding::Utf8).is_err());
        std::fs::remove_dir(d.root()).unwrap();
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        let version = list_in(&d.root(), &path).unwrap().remove(0);
        std::fs::hard_link(d.root().join(&version.id), d.0.join("leaked")).unwrap();
        assert!(read_in(&d.root(), &path, &version.id).is_err());
        assert!(capture_in(&d.root(), &path, Encoding::Utf8).is_err());
    }

    #[test]
    fn refuses_corrupt_truncated_and_cross_path_records_and_traversal_ids() {
        let d = Temp::new();
        let path = d.source(b"original");
        let other = d.0.join("other");
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        let version = list_in(&d.root(), &path).unwrap().remove(0);
        assert!(read_in(&d.root(), &other, &version.id).is_err());
        assert!(read_in(&d.root(), &path, "../document.txt").is_err());
        let stored = d.root().join(&version.id);
        let bytes = std::fs::read(&stored).unwrap();
        std::fs::write(&stored, &bytes[..bytes.len() - 1]).unwrap();
        assert!(read_in(&d.root(), &path, &version.id).is_err());
        std::fs::write(&stored, &bytes).unwrap();
        let mut corrupt = bytes;
        *corrupt.last_mut().unwrap() ^= 1;
        std::fs::write(&stored, corrupt).unwrap();
        assert!(read_in(&d.root(), &path, &version.id).is_err());
        assert!(capture_in(&d.root(), &path, Encoding::Utf8).is_err());
    }

    #[test]
    fn another_files_damaged_record_does_not_block_a_capture() {
        let d = Temp::new();
        let path = d.source(b"original");
        let other = d.0.join("other.txt");
        std::fs::write(&other, b"other").unwrap();
        capture_in(&d.root(), &other, Encoding::Utf8).unwrap();
        let version = list_in(&d.root(), &other).unwrap().remove(0);
        let stored = d.root().join(&version.id);
        let bytes = std::fs::read(&stored).unwrap();
        std::fs::write(&stored, &bytes[..bytes.len() - 1]).unwrap();
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        assert_eq!(list_in(&d.root(), &path).unwrap().len(), 1);
        assert!(list_in(&d.root(), &other).is_err());
    }

    #[test]
    fn interrupted_temporary_records_are_removed_before_the_next_capture() {
        let d = Temp::new();
        let path = d.source(b"old");
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        let version = list_in(&d.root(), &path).unwrap().remove(0);
        let interrupted = d.root().join(format!(".tmp-{}", version.id));
        std::fs::copy(d.root().join(&version.id), &interrupted).unwrap();
        std::fs::write(&path, b"next").unwrap();
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        assert!(!interrupted.exists());
        assert_eq!(list_in(&d.root(), &path).unwrap().len(), 2);
    }

    #[test]
    fn refuses_symlink_storage_ancestors_and_nonregular_sources() {
        let d = Temp::new();
        let path = d.source(b"original");
        let parent = d.0.join("parent");
        std::fs::create_dir(&parent).unwrap();
        let link = d.0.join("linked-parent");
        symlink(&parent, &link).unwrap();
        assert!(capture_in(&link.join("history"), &path, Encoding::Utf8).is_err());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
        assert!(capture_in(&d.root(), &d.0, Encoding::Utf8).is_err());
    }

    #[test]
    fn clear_only_removes_the_requested_paths_versions() {
        let d = Temp::new();
        let a = d.source(b"a");
        let b = d.0.join("b");
        std::fs::write(&b, b"b").unwrap();
        capture_in(&d.root(), &a, Encoding::Utf8).unwrap();
        capture_in(&d.root(), &b, Encoding::Utf8).unwrap();
        clear_in(&d.root(), &a).unwrap();
        assert!(list_in(&d.root(), &a).unwrap().is_empty());
        assert_eq!(list_in(&d.root(), &b).unwrap().len(), 1);
        assert_eq!(std::fs::read(&a).unwrap(), b"a");
        assert_eq!(std::fs::read(&b).unwrap(), b"b");
    }

    #[test]
    fn history_remains_readable_and_clearable_after_the_source_folder_is_deleted() {
        let d = Temp::new();
        let parent = d.0.join("deleted-folder");
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("document.txt");
        std::fs::write(&path, b"recoverable").unwrap();
        capture_in(&d.root(), &path, Encoding::Utf8).unwrap();
        let before = list_in(&d.root(), &path).unwrap();
        std::fs::remove_dir_all(&parent).unwrap();
        assert_eq!(list_in(&d.root(), &path).unwrap(), before);
        assert_eq!(read_in(&d.root(), &path, &before[0].id).unwrap().bytes, b"recoverable");
        clear_in(&d.root(), &path).unwrap();
        assert!(list_in(&d.root(), &path).unwrap().is_empty());
    }

    #[test]
    fn scoped_root_override_is_nested_and_restored_even_after_a_panic() {
        let d = Temp::new();
        let original = root();
        let a = d.0.join("a");
        let b = d.0.join("b");
        with_root(a.clone(), || {
            assert_eq!(root(), a);
            with_root(b.clone(), || assert_eq!(root(), b));
            assert_eq!(root(), a);
            let result = std::panic::catch_unwind(|| {
                with_root(b.clone(), || panic!("test restoration"));
            });
            assert!(result.is_err());
            assert_eq!(root(), a);
            let path = d.source(b"private test bytes");
            capture(&path, Encoding::Utf8).unwrap();
            assert_eq!(list_in(&a, &path).unwrap().len(), 1);
        });
        assert_eq!(root(), original);
        assert!(!b.exists());
    }
}
