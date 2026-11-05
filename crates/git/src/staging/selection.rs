//! Apply reviewed text changes to the index, leaving the worktree untouched.

use crate::diff::{changes, versions, Versions};
use crate::operations::{check_file, command, input, run};
use crate::status::FileStatus;
use ropey::Rope;
use std::ops::Range;
use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct StageSelection {
    pub old_side: bool,
    /// Zero-based lines in the displayed side. A caret selects one line.
    pub lines: Range<usize>,
    pub whole_hunks: bool,
}

pub fn apply_selection(
    root: &Path,
    file: &FileStatus,
    staged: bool,
    shown: &Versions,
    selection: &StageSelection,
) -> Result<(), String> {
    check_file(file)?;
    if file.is_conflicted() {
        return Err("Resolve conflicts before staging individual changes".into());
    }
    // both names of a rename, so Git still pairs them
    let paths: Vec<&Path> = std::iter::once(file.path.as_path()).chain(file.original_path.as_deref()).collect();
    let live = crate::repository::status(root.to_path_buf(), &paths)?.files.into_iter().find(|f| f.path == file.path);
    if live.as_ref() != Some(file) {
        return Err("The file's Git status changed; refresh the diff and select the change again".into());
    }
    let current = versions(root, file, staged, 10 << 20)?;
    if &current != shown {
        return Err("The file or index changed; refresh the diff and select the change again".into());
    }
    let (before, after, old_side) = if staged {
        (&shown.new, &shown.old, !selection.old_side)
    } else {
        (&shown.old, &shown.new, selection.old_side)
    };
    let old: Vec<_> = before.split_inclusive('\n').collect();
    let new: Vec<_> = after.split_inclusive('\n').collect();
    let mut target = Vec::new();
    let mut end = 0;
    for hunk in changes::between(&Rope::from_str(before), &Rope::from_str(after)) {
        target.extend(old[end..hunk.old_lines.start].iter().copied());
        let side = if old_side { &hunk.old_lines } else { &hunk.new_lines };
        let intersects = if side.is_empty() {
            selection.lines.contains(&side.start)
        } else {
            side.start < selection.lines.end && selection.lines.start < side.end
        };
        let whole = selection.whole_hunks && intersects;
        for i in hunk.old_lines.clone() {
            if !(whole || (!selection.whole_hunks && old_side && selection.lines.contains(&i))) {
                target.push(old[i]);
            }
        }
        for i in hunk.new_lines.clone() {
            if whole || (!selection.whole_hunks && !old_side && selection.lines.contains(&i)) {
                target.push(new[i]);
            }
        }
        end = hunk.old_lines.end;
    }
    target.extend(old[end..].iter().copied());
    if target.windows(2).any(|lines| !lines[0].ends_with('\n')) {
        return Err("This selection needs the final newline change; stage or unstage the whole hunk".into());
    }
    let target = target.concat();
    if target == *before {
        return Err("Select a changed line or hunk in the diff first".into());
    }
    // A full-context patch also checks the entire index baseline under Git's index lock.
    let entry = run(command(root)?.args(["ls-files", "--stage", "-z", "--"]).arg(&file.path))?;
    let exists = !entry.is_empty();
    let first_field = |bytes: &[u8], absent: &str| {
        let text = String::from_utf8_lossy(bytes);
        text.split_whitespace().next().unwrap_or(absent).to_string()
    };
    let mode = if exists {
        first_field(&entry, "")
    } else if staged {
        let tree = run(command(root)?.args(["ls-tree", "HEAD", "--"]).arg(&file.path))?;
        first_field(&tree, "100644")
    } else {
        let meta = std::fs::symlink_metadata(root.join(&file.path)).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("Stage this file as a whole".into());
        }
        if meta.permissions().mode() & 0o111 != 0 { "100755" } else { "100644" }.into()
    };
    if !matches!(mode.as_str(), "100644" | "100755") {
        return Err("Stage symbolic links and submodules as whole files".into());
    }
    let removed = target.is_empty() && ((!staged && file.worktree == 'D') || (staged && file.index == 'A'));
    let a = quoted_path(b"a/", &file.path);
    let b = quoted_path(b"b/", &file.path);
    let mut patch = format!("diff --git {a} {b}\n");
    if !exists {
        patch.push_str(&format!("new file mode {mode}\n"));
    }
    if removed {
        patch.push_str(&format!("deleted file mode {mode}\n"));
    }
    patch.push_str(&format!(
        "--- {}\n+++ {}\n",
        if exists { &a } else { "/dev/null" },
        if removed { "/dev/null" } else { &b }
    ));
    let new_count = target.split_inclusive('\n').count();
    patch.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        usize::from(!old.is_empty()),
        old.len(),
        usize::from(new_count > 0),
        new_count
    ));
    for (text, prefix) in [(before.as_str(), '-'), (target.as_str(), '+')] {
        for line in text.split_inclusive('\n') {
            patch.push(prefix);
            patch.push_str(line);
            if !line.ends_with('\n') {
                patch.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    input(command(root)?.args(["apply", "--cached", "--whitespace=nowarn", "-"]), patch.as_bytes()).map(|_| ())
}

fn quoted_path(prefix: &[u8], path: &Path) -> String {
    let mut result = String::from("\"");
    for byte in prefix.iter().chain(path.as_os_str().as_bytes()) {
        match byte {
            b'"' | b'\\' => {
                result.push('\\');
                result.push(*byte as char);
            }
            32..=126 => result.push(*byte as char),
            _ => result.push_str(&format!("\\{byte:03o}")),
        }
    }
    result.push('"');
    result
}
