//! App-global ownership of the editor's pure clipboard history model.

use std::ops::{Deref, DerefMut};

pub use orlo_editor::clipboard_history::ClipEntry;

#[derive(Default)]
pub struct ClipboardHistory(orlo_editor::clipboard_history::ClipboardHistory);

impl gpui::Global for ClipboardHistory {}

impl Deref for ClipboardHistory {
    type Target = orlo_editor::clipboard_history::ClipboardHistory;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ClipboardHistory {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
