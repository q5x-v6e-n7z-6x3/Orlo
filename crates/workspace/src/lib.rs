//! UI-independent layout, persistent workspace state, and launch navigation protocols.

pub mod layout;
pub mod navigation;
pub mod panes;
pub mod sessions;
pub mod tabs;

pub use navigation::{cli, handoff};
pub use sessions as session;
