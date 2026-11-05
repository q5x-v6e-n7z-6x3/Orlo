//! Attribution of saved file contents; uncommitted lines retain Git's zero commit ID.

use crate::operations::{check_path, command, run};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlameLine {
    pub commit: String,
    pub author: String,
    pub timestamp: i64,
    pub summary: String,
    pub text: String,
}

pub fn blame(root: &Path, path: &Path) -> Result<Vec<BlameLine>, String> {
    check_path(path)?;
    let full = root.join(path);
    let meta = std::fs::symlink_metadata(&full).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > 10 << 20 {
        return Err("Blame requires a regular text file under 10 MiB".into());
    }
    let contents = std::fs::read(full).map_err(|e| e.to_string())?;
    if contents.contains(&0) || std::str::from_utf8(&contents).is_err() {
        return Err("Blame requires a UTF-8 text file".into());
    }
    let bytes = run(command(root)?.args(["blame", "--no-textconv", "--line-porcelain", "--"]).arg(path))?;
    let text = String::from_utf8(bytes).map_err(|_| "Git returned non-UTF-8 blame output")?;
    let mut lines = text.lines();
    let mut result = Vec::new();
    while let Some(header) = lines.next() {
        let commit = header.split_whitespace().next().ok_or("Git returned malformed blame output")?;
        if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Git returned malformed blame commit".into());
        }
        let mut entry = BlameLine { commit: commit.into(), ..BlameLine::default() };
        loop {
            let line = lines.next().ok_or("Git returned incomplete blame output")?;
            if let Some(content) = line.strip_prefix('\t') {
                entry.text = content.into();
                break;
            } else if let Some(author) = line.strip_prefix("author ") {
                entry.author = author.into();
            } else if let Some(time) = line.strip_prefix("author-time ") {
                entry.timestamp = time.parse().map_err(|_| "Git returned an invalid blame time")?;
            } else if let Some(summary) = line.strip_prefix("summary ") {
                entry.summary = summary.into();
            }
        }
        result.push(entry);
    }
    Ok(result)
}
