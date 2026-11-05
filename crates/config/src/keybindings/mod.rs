//! Serialized commands shared by keymaps and recorded editor macros.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub fn arguments() -> Value {
    serde_json::json!({})
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub command: String,
    #[serde(default = "arguments")]
    pub args: Value,
}

pub fn keymap_path() -> PathBuf {
    orlo_utils::paths::support_dir().join("Default.sublime-keymap")
}
