//! One GPUI entity per open document. Its model (`orlo_editor`'s `Document`) holds the text,
//! undo history and file; every view of it (`Editor`) keeps only its own selections and marks.
//! Changes go through here and come out as `DocumentEvent`s that each view, and each window
//! showing it, follows once. `Documents` is the app-wide registry, by id and by path.

use crate::editing::text::{Document, Editing, TextChange, ViewEdit, ViewModel};
use crate::integrations::changes::Hunk;
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, WeakEntity};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct OpenDocument {
    pub model: Document,
    /// How many tabs, in any window, show it: one with none is not found by its path.
    pub(crate) tabs: usize,
    /// The save under way, if one is (`save.rs`).
    pub(crate) saving: Option<save::Saving>,
    pub(crate) copying: save::Copying,
    pub(crate) following: Option<follow::Following>,
    pub(crate) file_autosave: autosave::FileAutosave,
}

/// What changed in a document. Each kind is its own event: a view that only draws the text
/// need not care about the file, and a tab title need not care about the text.
#[derive(Clone, Debug)]
pub enum DocumentEvent {
    /// The text: the versions it went from and to, the edits in order, and the view that made
    /// them with the selections it left. Every other view maps its own through the edits once.
    Edited(Rc<TextChange>),
    /// How the document is written: path, encoding, line endings, read-only, disk state or
    /// indentation. `renamed` when the path changed, so views pick their syntax again.
    File { document: u64, renamed: bool },
    /// Whether it would lose text if closed: unsaved edits, or a file deleted on disk.
    Saved { document: u64, unsaved: bool },
    /// The changed-line markers a window found for `version` of the text, against the Git
    /// index or the saved file. Each view showing that version draws them, so a window in the
    /// background, which does not look for itself, still has them.
    Changes { document: u64, version: u64, hunks: Rc<[Hunk]> },
}

impl DocumentEvent {
    pub fn document(&self) -> u64 {
        match self {
            Self::Edited(c) => c.document,
            Self::File { document, .. } | Self::Saved { document, .. } | Self::Changes { document, .. } => *document,
        }
    }
}

impl EventEmitter<DocumentEvent> for OpenDocument {}

/// What `File` and `Saved` events report, compared before and after a change.
#[derive(Clone, PartialEq)]
struct Facts {
    path: Option<PathBuf>,
    format: (&'static str, bool, orlo_editor::encoding::Encoding, orlo_editor::encoding::Encoding, u64),
    disk: (Option<crate::files::watch::Stamp>, bool, bool, bool),
    indentation: (usize, Option<usize>, bool, [bool; 2], bool),
    flags: (bool, bool, bool, bool),
    unsaved: bool,
}

impl Facts {
    fn of(d: &Document) -> Self {
        Self {
            path: d.path.clone(),
            format: (d.line_ending, d.bom, d.encoding, d.disk_encoding, d.endings.as_ref().map_or(0, |m| m.identity())),
            disk: (d.disk, d.deleted, d.watching, d.read_only),
            indentation: (d.tab_size, d.indent_size, d.translate_tabs, d.indent_override, d.line_ending_override),
            flags: (d.scratch, d.history_enabled, d.is_following(), d.viewer_mode()),
            unsaved: d.unsaved(),
        }
    }
}

impl OpenDocument {
    pub fn new(model: Document) -> Self {
        let baseline = (!model.dirty && !model.read_only).then_some(model.disk).flatten();
        Self {
            model,
            tabs: 0,
            saving: None,
            copying: save::Copying::default(),
            following: None,
            file_autosave: autosave::FileAutosave { baseline, ..Default::default() },
        }
    }

    pub fn id(&self) -> u64 {
        self.model.id()
    }

    /// A view of this document runs a command: it first applies what other views changed, and
    /// its own change, if any, is published to them. Also reports any file or saved-state
    /// change the command made. Returns `f`'s result, the other views' changes it applied and
    /// its own.
    pub fn edit<R>(
        &mut self,
        view: &mut ViewModel,
        f: impl FnOnce(&mut ViewEdit) -> R,
        cx: &mut Context<Self>,
    ) -> (R, Vec<Rc<TextChange>>, Option<Rc<TextChange>>) {
        let before = Facts::of(&self.model);
        let mut s = Editing { doc: &mut self.model, view };
        let caught = s.catch_up();
        let r = f(&mut s);
        let change = s.take_change();
        if let Some(c) = &change {
            cx.emit(DocumentEvent::Edited(c.clone()));
        }
        self.report(before, cx);
        (r, caught, change)
    }

    /// A change to how the document is written, or its file, made outside a text edit.
    pub fn update_file<R>(&mut self, f: impl FnOnce(&mut Document) -> R, cx: &mut Context<Self>) -> R {
        let before = Facts::of(&self.model);
        let r = f(&mut self.model);
        self.stop_follow_if_moved(cx);
        self.report(before, cx);
        r
    }

    pub(crate) fn set_viewer_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let before = Facts::of(&self.model);
        if let Some(change) = self.model.set_viewer_mode(enabled) {
            cx.emit(DocumentEvent::Edited(change));
        }
        self.report(before, cx);
    }

    /// The changed-line markers found for `version`, for every view of the document.
    pub(crate) fn found_changes(&self, version: u64, hunks: Vec<Hunk>, cx: &mut Context<Self>) {
        cx.emit(DocumentEvent::Changes { document: self.id(), version, hunks: hunks.into() });
    }

    fn report(&mut self, before: Facts, cx: &mut Context<Self>) {
        let after = Facts::of(&self.model);
        if after == before {
            return;
        }
        let document = self.model.id();
        let renamed = after.path != before.path;
        if renamed || after.flags != before.flags || after.disk.3 != before.disk.3 {
            self.file_autosave.cancel();
            if renamed {
                self.file_autosave.baseline = None;
            }
        }
        if renamed && self.tabs > 0 {
            Documents::moved(document, before.path.as_deref(), after.path.clone(), cx);
        }
        let saved_changed = after.unsaved != before.unsaved;
        // every fact but `unsaved`
        if (Facts { unsaved: after.unsaved, ..before }) != after {
            cx.emit(DocumentEvent::File { document, renamed });
        }
        if saved_changed {
            cx.emit(DocumentEvent::Saved { document, unsaved: after.unsaved });
        }
        cx.notify();
    }
}

/// Every document open in the app by its id, and those shown in a tab by their path: a file
/// opened in any window gets the document already open for it, and one closed and opened again
/// gets a new one.
#[derive(Default)]
pub struct Documents {
    by_id: HashMap<u64, WeakEntity<OpenDocument>>,
    /// The documents shown with each path, the one shown or saved there last at the end: a
    /// document saved as a file open in another tab has that path too.
    by_path: HashMap<PathBuf, Vec<u64>>,
}

impl Global for Documents {}

impl Documents {
    /// A new document entity, registered.
    pub fn open(model: Document, cx: &mut App) -> Entity<OpenDocument> {
        let doc = cx.new(|_| OpenDocument::new(model));
        let id = doc.read(cx).id();
        let all = cx.default_global::<Documents>();
        all.by_id.retain(|_, d| d.upgrade().is_some());
        all.by_id.insert(id, doc.downgrade());
        doc
    }

    pub fn get(id: u64, cx: &App) -> Option<Entity<OpenDocument>> {
        cx.try_global::<Documents>()?.by_id.get(&id)?.upgrade()
    }

    /// The document a tab in some window shows for `path`.
    pub fn for_path(path: &Path, cx: &App) -> Option<Entity<OpenDocument>> {
        let ids = cx.try_global::<Documents>()?.by_path.get(path)?;
        (ids.iter().rev()).filter_map(|&id| Self::get(id, cx)).find(|d| {
            let d = d.read(cx);
            d.tabs > 0 && d.model.path.as_deref() == Some(path)
        })
    }

    /// `path` finds document `id` first.
    fn index(&mut self, path: PathBuf, id: u64) {
        let ids = self.by_path.entry(path).or_default();
        ids.retain(|&i| i != id);
        ids.push(id);
    }

    /// `path` no longer finds document `id`; another shown with it still is.
    fn unindex(&mut self, path: &Path, id: u64) {
        if let Some(ids) = self.by_path.get_mut(path) {
            ids.retain(|&i| i != id);
            if ids.is_empty() {
                self.by_path.remove(path);
            }
        }
    }

    /// A tab now shows `doc`: its path finds it.
    pub(crate) fn shown(doc: &Entity<OpenDocument>, cx: &mut App) {
        let (id, path) = doc.update(cx, |d, _| {
            d.tabs += 1;
            (d.id(), d.model.path.clone())
        });
        if let Some(path) = path {
            cx.default_global::<Documents>().index(path, id);
        }
    }

    /// A tab showing `doc` closed or moved away; with none left its path no longer finds it.
    pub(crate) fn hidden(doc: &Entity<OpenDocument>, cx: &mut App) {
        let (id, path, left) = doc.update(cx, |d, cx| {
            d.tabs = d.tabs.saturating_sub(1);
            d.file_autosave.cancel();
            if d.tabs == 0 {
                d.stop_follow(cx);
            }
            (d.id(), d.model.path.clone(), d.tabs)
        });
        if let (0, Some(path)) = (left, path) {
            cx.default_global::<Documents>().unindex(&path, id);
        }
    }

    /// Document `id`, shown in a tab, was saved as or renamed from `from` to `to`.
    fn moved(id: u64, from: Option<&Path>, to: Option<PathBuf>, cx: &mut App) {
        let all = cx.default_global::<Documents>();
        if let Some(from) = from {
            all.unindex(from, id);
        }
        if let Some(to) = to {
            all.index(to, id);
        }
    }
}

pub(crate) mod autosave;
pub(crate) mod follow;
pub mod save;
pub use save::SaveOutcome;

#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod tests;
