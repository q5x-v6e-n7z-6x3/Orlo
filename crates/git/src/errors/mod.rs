//! Preserve Git's diagnostic text for callers.

use std::process::Output;

pub(crate) fn failure(output: &Output) -> String {
    [&output.stderr, &output.stdout]
        .into_iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
        .find(|message| !message.is_empty())
        .unwrap_or_else(|| format!("Git exited with {}", output.status))
}
