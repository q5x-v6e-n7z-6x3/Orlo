//! Paths stored in shared project and workspace files.

use std::path::{Component, Path, PathBuf};

/// `p` relative to `dir` when it is inside it, written with `/`; else absolute.
pub fn relative(p: &Path, dir: &Path) -> String {
    match p.strip_prefix(dir) {
        Ok(r) if r.as_os_str().is_empty() => ".".into(),
        Ok(r) => r.to_string_lossy().into_owned(),
        Err(_) => p.to_string_lossy().into_owned(),
    }
}

/// A folder entry's path made absolute: relative to the project's folder, `~` for home;
/// `.` and `..` folded away.
pub fn absolute(spelt: &str, dir: &Path) -> PathBuf {
    let p = orlo_utils::paths::untilde(spelt);
    folded(&if p.is_absolute() { p } else { dir.join(p) })
}

/// `p` with `.` and `..` folded away, without reading the disk.
pub(crate) fn folded(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}
