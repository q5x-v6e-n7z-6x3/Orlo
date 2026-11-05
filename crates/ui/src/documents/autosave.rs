//! Existing-file autosave state. Session recovery keeps its independent scheduler.
use super::*;
use crate::config::settings::Settings;
use crate::editing::text::SavedPoint;
use crate::files::watch::Stamp;
use gpui::{EntityId, Task};

#[derive(Clone, PartialEq)]
pub(crate) struct Key {
    pub path: Option<PathBuf>,
    pub version: u64,
    pub point: SavedPoint,
}

impl Key {
    pub fn of(d: &Document) -> Self {
        Self { path: d.path.clone(), version: d.version, point: d.save_point() }
    }
}

#[derive(Default)]
pub(crate) struct FileAutosave {
    /// Authoritative complete read/save, never the disk poll's acknowledged stamp.
    pub baseline: Option<Stamp>,
    pub generation: u64,
    pub owner: Option<EntityId>,
    pub policy: Option<Settings>,
    pub timer: Option<Task<()>>,
    pub pending: Option<Key>,
    pub failed: Option<Key>,
    pub attempt: Option<Key>,
    pub suspensions: usize,
    pub quitting: bool,
}

impl FileAutosave {
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.timer = None;
        self.pending = None;
    }
}

impl OpenDocument {
    pub(crate) fn file_autosave_loaded(&mut self) {
        self.file_autosave.cancel();
        self.file_autosave.baseline = (!self.model.read_only).then_some(self.model.disk).flatten();
        self.file_autosave.failed = None;
    }
}

impl Documents {
    pub(crate) fn resume_file_autosaves(cx: &mut App) {
        let docs: Vec<_> = cx
            .try_global::<Documents>()
            .into_iter()
            .flat_map(|all| all.by_id.values())
            .filter_map(WeakEntity::upgrade)
            .collect();
        for doc in docs {
            doc.update(cx, |d, _| d.file_autosave.quitting = false);
        }
    }
    pub(crate) fn cancel_file_autosaves(cx: &mut App) {
        let docs: Vec<_> = cx
            .try_global::<Documents>()
            .into_iter()
            .flat_map(|all| all.by_id.values())
            .filter_map(WeakEntity::upgrade)
            .collect();
        for doc in docs {
            doc.update(cx, |d, _| {
                d.file_autosave.cancel();
                d.file_autosave.quitting = true;
            });
        }
    }
}
