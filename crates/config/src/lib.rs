//! Shared preferences, keymap commands, appearance, and automation configuration.

pub mod keybindings;
pub mod persistence;
pub mod settings;
pub mod themes;

pub use persistence::automation;
pub use settings::{editorconfig, selector};
