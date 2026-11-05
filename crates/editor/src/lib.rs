//! Orlo's text editing engine; GPUI views live in orlo-ui.
pub mod buffer;
pub mod commands;
pub mod cursor;
pub mod editing;
pub mod history;
pub mod rendering;
pub mod search;
pub mod selection;
pub mod viewport;
pub use buffer as text;
pub use editing::{clipboard_history, columns, convert, csv, json, snippet, stats, transform};
pub use orlo_project::encoding;
pub use rendering::fold;
pub use search as searching;
pub use viewport as display;
