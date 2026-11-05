//! The two texts a file's diff compares, for a side-by-side view of it.

use crate::operations::{check_file, check_path, command, has_head, run};
use crate::staging::{blob_text, entry, index_blob, object_id, too_large};
use crate::status::FileStatus;
use std::path::Path;

/// A file's diff as two texts: `old` before, `new` after.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Versions {
    pub old: String,
    pub new: String,
}

/// The text of `path` at HEAD: none when HEAD has no such file, or there is no commit yet.
pub fn head_text(root: &Path, path: &Path) -> Result<Option<String>, String> {
    (head_blob(root, path)?).map(|oid| blob_text(root, &oid, usize::MAX)).transpose()
}

/// The object ID of `path` at HEAD.
fn head_blob(root: &Path, path: &Path) -> Result<Option<String>, String> {
    check_path(path)?;
    // an unborn branch has no commit to read
    if !has_head(root)? {
        return Ok(None);
    }
    let entries = run(command(root)?.args(["ls-tree", "-z", "HEAD", "--"]).arg(path))?;
    let Some(metadata) = entry(&entries, path, "Git returned malformed tree output")? else {
        return Ok(None);
    };
    match metadata[1] {
        b"blob" => object_id(metadata[2]).map(Some),
        b"commit" => Err("A submodule has no editable text blob".into()),
        _ => Err("This path is a folder at HEAD, with no text to compare".into()),
    }
}

/// The file as saved: refused when it is a symbolic link (the patch shows its target), over
/// `limit` bytes, binary or not UTF-8.
fn saved_text(root: &Path, path: &Path, limit: usize) -> Result<String, String> {
    let full = root.join(path);
    let meta = std::fs::symlink_metadata(&full).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err("A symbolic link shows as a patch".into());
    }
    if meta.len() > limit as u64 {
        return Err(too_large());
    }
    let bytes = std::fs::read(&full).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.contains(&0) {
        return Err("The file contains binary data".into());
    }
    String::from_utf8(bytes).map_err(|_| "The file is not valid UTF-8".into())
}

/// The file as Git compares it with the index, after the line endings, `ident` and clean
/// filters its attributes ask for: the text after Git's patch of it with the whole file as
/// context, or `unchanged` when Git finds no difference. Refused over `limit` bytes, or when
/// Git compares it as binary.
fn cleaned_text(
    root: &Path,
    path: &Path,
    limit: usize,
    unchanged: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    // a line takes a byte at least: a text within the limit is one hunk (Git counts in ints)
    let context = format!("--unified={}", limit.min(1 << 28));
    let patch =
        run(command(root)?.args(["diff", "--no-ext-diff", "--no-textconv", "--no-color", &context, "--"]).arg(path))?;
    if patch.is_empty() {
        return unchanged();
    }
    let patch = String::from_utf8(patch).map_err(|_| "The file is not valid UTF-8".to_string())?;
    match after_patch(&patch) {
        Some(text) if text.len() > limit => Err(too_large()),
        Some(text) => Ok(text),
        None => Err("Git compares the file as binary data".into()),
    }
}

/// The text after a one-file patch: the lines its hunk keeps and adds. `None` when it has no
/// hunk (a file Git compares as binary).
fn after_patch(patch: &str) -> Option<String> {
    let hunk = patch.find("\n@@ ")? + 1;
    let mut out = String::with_capacity(patch.len() - hunk);
    // whether the last line is on the new side, which a "\ No newline" note then ends
    let mut kept = false;
    for line in patch[hunk..].split_inclusive('\n').skip(1) {
        match line.as_bytes()[0] {
            b' ' | b'+' => {
                out.push_str(&line[1..]);
                kept = true;
            }
            // an empty line kept, without its space (diff.suppressBlankEmpty)
            b'\n' => {
                out.push('\n');
                kept = true;
            }
            b'\\' if kept && out.ends_with('\n') => {
                out.pop();
            }
            _ => kept = false,
        }
    }
    Some(out)
}

/// The texts `diff(root, file, staged)` compares. The working tree side compares the index's
/// text (none for an untracked file) with the file as saved, as Git's line endings and filters
/// leave it to compare (none when it is deleted); the staged side compares HEAD's (none for a
/// file new to it; a rename's at its old path) with the index's (none when it is staged as
/// deleted). `Err` says why only the patch can show it: a merge conflict, binary or non-UTF-8
/// text, a side over `limit` bytes, or Git's own error.
pub fn versions(root: &Path, file: &FileStatus, staged: bool, limit: usize) -> Result<Versions, String> {
    check_file(file)?;
    if file.is_conflicted() {
        return Err("The file has a merge conflict".into());
    }
    let text = |oid: Option<String>| oid.map_or(Ok(String::new()), |o| blob_text(root, &o, limit));
    // a rename's text before it is at its old path
    let before = |status: char| match (status, &file.original_path) {
        ('R', Some(original)) => original.as_path(),
        _ => file.path.as_path(),
    };
    if staged {
        let old = text(head_blob(root, before(file.index))?)?;
        let new = text(index_blob(root, &file.path)?)?;
        return Ok(Versions { old, new });
    }
    let old = match file.is_untracked() {
        true => String::new(),
        false => text(index_blob(root, before(file.worktree))?)?,
    };
    let new = match file.worktree {
        'D' => String::new(),
        // the patch compares an untracked file as it is saved
        _ if file.is_untracked() => saved_text(root, &file.path, limit)?,
        _ => {
            saved_text(root, &file.path, limit)?;
            cleaned_text(root, &file.path, limit, || match before(file.worktree) == file.path {
                true => Ok(old.clone()),
                false => text(index_blob(root, &file.path)?),
            })?
        }
    };
    Ok(Versions { old, new })
}
