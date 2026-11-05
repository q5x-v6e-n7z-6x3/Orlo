//! Folders the user trusts to run their own programs: a repository's Git filter drivers, the
//! language servers that build a project, and the tasks a project defines. Trusting a folder
//! covers every folder in it. The list lives in the support folder, readable by the user alone.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The trusted folders, canonical; read from disk once.
static TRUSTED: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

/// One path a line, as its bytes.
fn file() -> PathBuf {
    crate::paths::support_dir().join("Trusted Folders.txt")
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn with_list<R>(f: impl FnOnce(&mut Vec<PathBuf>) -> R) -> R {
    let mut list = TRUSTED.lock().unwrap_or_else(|e| e.into_inner());
    let list = list.get_or_insert_with(|| {
        use std::os::unix::ffi::OsStrExt;
        let bytes = std::fs::read(file()).unwrap_or_default();
        let lines = bytes.split(|&b| b == b'\n').filter(|l| l.starts_with(b"/"));
        lines.map(|l| PathBuf::from(std::ffi::OsStr::from_bytes(l))).collect()
    });
    f(list)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Tests run every folder trusted unless they test trust itself.
    pub static TRUST_ALL: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether `path` is in a folder the user trusts.
pub fn trusted(path: &Path) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if TRUST_ALL.get() {
        return true;
    }
    let path = canonical(path);
    with_list(|list| list.iter().any(|t| path.starts_with(t)))
}

/// Trust `folder` (and the folders in it) from now on.
pub fn trust(folder: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let folder = canonical(folder);
    if !folder.is_absolute() || folder.as_os_str().as_bytes().contains(&b'\n') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "This folder's name cannot be trusted"));
    }
    let text = with_list(|list| {
        if !list.iter().any(|t| folder.starts_with(t)) {
            list.retain(|t| !t.starts_with(&folder));
            list.push(folder);
            list.sort();
        }
        list.iter().flat_map(|t| [t.as_os_str().as_bytes(), b"\n"]).flatten().copied().collect::<Vec<u8>>()
    });
    write_private(&file(), &text)
}

/// `text` to `path` through a temp file of the user's alone, renamed over it.
fn write_private(path: &Path, text: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("txt.orlo-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(text)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trusted_folder_covers_the_folders_in_it_and_persists() {
        TRUST_ALL.set(false);
        let support = crate::paths::support_dir();
        let dir = support.join("trust-test");
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        let outside = support.join("trust-test-other");
        std::fs::create_dir_all(&outside).unwrap();
        assert!(!trusted(&dir.join("inner")));
        trust(&dir).unwrap();
        assert!(trusted(&dir.join("inner")) && trusted(&dir));
        assert!(!trusted(&outside), "a sibling sharing the name's start is not inside it");
        // read back from disk, as the next launch does
        *TRUSTED.lock().unwrap() = None;
        assert!(trusted(&dir.join("inner")));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(file()).unwrap().permissions().mode() & 0o777, 0o600);
        TRUST_ALL.set(true);
    }
}
