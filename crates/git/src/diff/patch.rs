use crate::errors::failure;
use crate::operations::{check_file, command, output};
use crate::status::FileStatus;
use std::path::Path;

pub fn diff(root: &Path, file: &FileStatus, staged: bool) -> Result<String, String> {
    check_file(file)?;
    let mut git = command(root)?;
    git.args(["diff", "--no-ext-diff", "--no-textconv", "--no-color"]);
    let untracked = file.is_untracked() && !staged;
    if untracked {
        git.args(["--no-index", "--", "/dev/null"]).arg(root.join(&file.path));
    } else {
        if staged {
            git.arg("--cached");
        }
        git.args(["--find-renames", "--"]).arg(&file.path).args(file.rename_source());
    }
    let result = output(&mut git)?;
    if !result.status.success() && !(untracked && result.status.code() == Some(1)) {
        return Err(failure(&result));
    }
    String::from_utf8(result.stdout).map_err(|_| "The diff contains non-UTF-8 text".into())
}
