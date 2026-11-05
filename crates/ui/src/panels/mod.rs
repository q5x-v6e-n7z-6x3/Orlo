//! Workspace panels and the explorer sidebar.

pub mod explorer;
pub mod git;
pub mod outline;
pub mod search;
pub mod terminal;

pub use crate::components::form;
pub use git as git_ui;
pub use search::{extract as extract_ui, find, playground, results as results_panel, saved as saved_ui};
