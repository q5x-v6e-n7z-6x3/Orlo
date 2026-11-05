//! Application support folders and home-relative paths.

use std::path::{Path, PathBuf};

/// `~/Library/Application Support/Orlo`: the session, the settings file and the hand-off
/// socket. `home_dir` asks the user database when `HOME` is unset; a relative path would put
/// them wherever Orlo was started from.
/// Found once: the session's path must not move mid-run.
fn user_support_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| match std::env::home_dir() {
        Some(home) => home.join("Library/Application Support/Orlo"),
        None => private_dir(&std::env::temp_dir()),
    });
    dir.clone()
}

/// Resolve the current user's support path in normal application processes.
#[cfg(not(any(test, feature = "test-support")))]
pub fn support_dir() -> PathBuf {
    user_support_dir()
}

/// Real app processes use the user's support folder even when built with test helpers.
/// Unit tests retain their disposable default and may override it with `TEST_SUPPORT`.
pub fn use_user_support_dir() {
    #[cfg(any(test, feature = "test-support"))]
    USE_USER_SUPPORT_DIR.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-support"))]
static USE_USER_SUPPORT_DIR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// No home folder: `Orlo-<uid>` in `tmp`, this user's alone (0700). One another user made first
/// (a shared `/tmp`), or a link, is not used: a folder for this run alone is made instead.
pub fn private_dir(tmp: &Path) -> PathBuf {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    // SAFETY: getuid takes nothing, cannot fail and touches no memory of ours.
    let uid = unsafe { libc::getuid() };
    let ours = |d: &Path| {
        std::fs::symlink_metadata(d).is_ok_and(|m| m.is_dir() && m.uid() == uid && m.permissions().mode() & 0o077 == 0)
    };
    let dir = tmp.join(format!("Orlo-{uid}"));
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
    if ours(&dir) {
        return dir;
    }
    let run = tmp.join(format!("Orlo-{uid}-{}", std::process::id()));
    match std::fs::DirBuilder::new().mode(0o700).create(&run) {
        Ok(()) if ours(&run) => run,
        _ => panic!("Orlo: no home folder, and no private folder in {}", tmp.display()),
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// A test's Application Support, set by `ui_tests::window`; gpui runs a test's tasks on
    /// its own thread.
    pub static TEST_SUPPORT: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Tests never touch the user's files: a folder of the test's own, else one per test run.
#[cfg(any(test, feature = "test-support"))]
pub fn support_dir() -> PathBuf {
    TEST_SUPPORT.with_borrow(|d| d.clone()).unwrap_or_else(|| {
        if USE_USER_SUPPORT_DIR.load(std::sync::atomic::Ordering::Relaxed) {
            user_support_dir()
        } else {
            std::env::temp_dir().join(format!("orlo-support-{}", std::process::id()))
        }
    })
}

/// `~` and `~/…` with the home folder in place.
pub fn untilde(s: &str) -> PathBuf {
    match (s.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(h)) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(h).join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(s),
    }
}

/// The most of an ignore file (`.gitignore`) a reader may take.
pub const IGNORE_FILE_LIMIT: u64 = 8 << 20;

/// Whether `path` is safe to hand to an ignore-file reader, which reads it whole: absent, or a
/// regular file (through any link) within `IGNORE_FILE_LIMIT`. A cloned repository's
/// `.gitignore` can be a link to `/dev/zero` or a FIFO, which would be read forever (or block).
/// One that cannot be looked at is passed: its reader fails at once too.
pub fn readable_ignore_file(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.is_file() && m.len() <= IGNORE_FILE_LIMIT,
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_files_that_never_end_are_not_read() {
        let dir = std::env::temp_dir().join(format!("orlo-ignore-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plain"), "target/\n").unwrap();
        let _ = std::fs::remove_file(dir.join("zero"));
        std::os::unix::fs::symlink("/dev/zero", dir.join("zero")).unwrap();
        assert!(readable_ignore_file(&dir.join("plain")));
        assert!(readable_ignore_file(&dir.join("absent")));
        assert!(!readable_ignore_file(&dir.join("zero")), "a link to a device");
        assert!(!readable_ignore_file(&dir), "a folder");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn with_no_home_a_link_in_tmp_is_not_written_through() {
        let tmp = std::env::temp_dir().join("orlo-no-home-link");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let victim = tmp.join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        // another user's link where a probe file for this process's user id once went
        let probe = tmp.join(format!(".orlo-uid-{}", std::process::id()));
        std::os::unix::fs::symlink(&victim, probe).unwrap();
        private_dir(&tmp);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
    }

    #[test]
    fn with_no_home_the_folder_is_this_users_alone() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let tmp = std::env::temp_dir().join("orlo-no-home");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let dir = private_dir(&tmp);
        let m = std::fs::metadata(&dir).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o700);
        assert_eq!(dir, private_dir(&tmp), "the same folder each launch");
        // one anyone may write to (as if another user made it first) is not used
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let other = private_dir(&tmp);
        assert_ne!(other, dir);
        assert_eq!(std::fs::metadata(&other).unwrap().uid(), m.uid());
    }
}
