use crate::operations::{check_path, command, run};
use std::path::Path;

/// The stage-0 blob, without textconv or filters. Absence is distinct from unreadable text.
pub fn index_text(root: &Path, path: &Path) -> Result<Option<String>, String> {
    (index_blob(root, path)?).map(|oid| blob_text(root, &oid, usize::MAX)).transpose()
}

/// The object ID of `path`'s stage-0 entry; none when the index has no such file.
pub(crate) fn index_blob(root: &Path, path: &Path) -> Result<Option<String>, String> {
    check_path(path)?;
    let entries = run(command(root)?.args(["ls-files", "--stage", "-z", "--"]).arg(path))?;
    let Some(metadata) = entry(&entries, path, "Git returned malformed index output")? else {
        return Ok(None);
    };
    if metadata[2] != b"0" {
        return Err("The index has unresolved conflict entries for this file".into());
    }
    if metadata[0] == b"160000" {
        return Err("A submodule has no editable text blob".into());
    }
    object_id(metadata[1]).map(Some)
}

/// The three space-separated fields before the tab of `path`'s entry in a `-z` listing of the
/// index or a tree.
pub(crate) fn entry<'a>(entries: &'a [u8], path: &Path, malformed: &str) -> Result<Option<Vec<&'a [u8]>>, String> {
    use std::os::unix::ffi::OsStrExt;
    for entry in entries.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let tab = (entry.iter().position(|b| *b == b'\t')).ok_or(malformed)?;
        if &entry[tab + 1..] != path.as_os_str().as_bytes() {
            continue;
        }
        let metadata: Vec<_> = entry[..tab].split(|b| *b == b' ').collect();
        if metadata.len() != 3 {
            return Err(malformed.into());
        }
        return Ok(Some(metadata));
    }
    Ok(None)
}

pub(crate) fn object_id(bytes: &[u8]) -> Result<String, String> {
    std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| "Git returned an invalid object ID".into())
}

/// A blob's text: refused when it is over `limit` bytes, binary or not UTF-8.
pub(crate) fn blob_text(root: &Path, oid: &str, limit: usize) -> Result<String, String> {
    if limit < usize::MAX {
        let size = run(command(root)?.args(["cat-file", "-s", oid]))?;
        let size = std::str::from_utf8(&size).ok().and_then(|s| s.trim().parse::<usize>().ok());
        if size.ok_or("Git returned an invalid object size")? > limit {
            return Err(too_large());
        }
    }
    let bytes = run(command(root)?.args(["cat-file", "blob", oid]))?;
    if bytes.contains(&0) {
        return Err("The index contains binary data".into());
    }
    String::from_utf8(bytes).map_err(|_| "The index text is not valid UTF-8".into())
}

pub(crate) fn too_large() -> String {
    "The file is too large to show side by side".into()
}
