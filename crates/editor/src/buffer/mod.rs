//! Editing model: a ropey `Rope`, multiple selections, snapshot undo. No gpui here.

pub(crate) use crate::editing::columns;
pub(crate) use crate::editing::convert::{digest, Hash};
pub(crate) use crate::editing::csv::{self, ColumnOp, Delim};
pub(crate) use crate::editing::snippet::{self, Session, Snippet, Spot};
pub(crate) use crate::search::pattern::{Deadline, Pattern, Replacer, SearchError};
pub(crate) use crate::viewport::{char_at, col_after};
pub use orlo_config::settings::VirtualSpace;
pub(crate) use orlo_config::settings::{Cleanup, Trim};
pub(crate) use orlo_project::watch::{stamp, Stamp};
pub(crate) use regex_automata::{meta, util::syntax};
pub(crate) use regex_syntax::hir::{Hir, Look};
pub(crate) use ropey::Rope;
pub(crate) use std::cell::RefCell;
pub(crate) use std::collections::{HashMap, HashSet};
pub(crate) use std::io;
pub(crate) use std::ops::Range;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::rc::Rc;
pub(crate) use std::sync::atomic::{AtomicU64, Ordering};
pub(crate) use unicode_segmentation::UnicodeSegmentation;

pub use orlo_project::documents::endings::{decoded as decoded_file, LineEndings};

pub const TAB_WIDTH: usize = 4;

/// Highlight styles a view keeps.
pub const HIGHLIGHT_STYLES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Spaces beyond the real line end. These never index the rope.
    pub anchor_virtual: usize,
    pub head_virtual: usize,
    /// Where Up and Down aim, kept from one vertical move to the next.
    pub goal: Option<Goal>,
    /// Created by a rectangular gesture or Add Caret Above/Below; never persisted.
    pub rectangular: bool,
}

/// Where vertical motion aims on the rows it reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    /// An x from the text's left edge, as the view's `Measure` draws its rows.
    X(f32),
    /// The ends of rows: after cmd-right, carets keep to them (Sublime's move_to eol).
    LineEnd,
}

impl Selection {
    pub fn caret(offset: usize) -> Self {
        Self::new(offset, offset)
    }
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head, anchor_virtual: 0, head_virtual: 0, goal: None, rectangular: false }
    }
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }
    pub fn is_empty(&self) -> bool {
        self.anchor_point() == self.head_point()
    }
    pub fn anchor_point(&self) -> (usize, usize) {
        (self.anchor, self.anchor_virtual)
    }
    pub fn head_point(&self) -> (usize, usize) {
        (self.head, self.head_virtual)
    }
    pub fn start_point(&self) -> (usize, usize) {
        self.anchor_point().min(self.head_point())
    }
    pub fn end_point(&self) -> (usize, usize) {
        self.anchor_point().max(self.head_point())
    }
    pub fn has_virtual(&self) -> bool {
        self.anchor_virtual != 0 || self.head_virtual != 0
    }
    pub fn from_points(anchor: (usize, usize), head: (usize, usize)) -> Self {
        Self {
            anchor: anchor.0,
            head: head.0,
            anchor_virtual: anchor.1,
            head_virtual: head.1,
            goal: None,
            rectangular: false,
        }
    }
    pub fn in_rectangle(mut self) -> Self {
        self.rectangular = true;
        self
    }
    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }
}

/// Selections as a view had them, for Undo Selection.
#[derive(Clone, PartialEq)]
pub struct SelState {
    pub selections: Vec<Selection>,
    pub primary: usize,
    pub(crate) whole_words: bool,
}

/// One view's selection history; never saved, never shared with other views of the buffer.
pub(crate) struct SelHistory {
    pub(crate) generation: u64,
    pub(crate) undo: Vec<SelState>,
    pub(crate) redo: Vec<SelState>,
    /// The state at the last checkpoint: what a later change pushes.
    pub(crate) seen: SelState,
    /// A local text change since the last checkpoint: the stacks must not cross it.
    pub(crate) edited: bool,
}

impl SelHistory {
    pub(crate) fn new(selections: Vec<Selection>, primary: usize) -> Self {
        Self {
            generation: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            seen: SelState { selections, primary, whole_words: false },
            edited: false,
        }
    }
}

/// States kept per stack.
// each state is a full copy of the selection set, so 128 states of a huge multi-selection cost memory; store deltas if that shows.
pub(crate) const SEL_HISTORY: usize = 128;

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub(crate) rope: Rope,
    pub(crate) endings: Option<LineEndings>,
    pub(crate) line_ending: &'static str,
    pub(crate) line_ending_override: bool,
    pub(crate) selections: Vec<Selection>,
    /// Which of them the view followed.
    pub(crate) primary: usize,
    /// Find Results' outlines as they were, so undo brings back the ones an edit dropped.
    pub(crate) outlines: Vec<Range<usize>>,
    /// The view that recorded it gets these selections back on undo; other views keep theirs.
    pub(crate) view_id: u64,
    /// That view's bookmarks and highlights as they were, back with its selections.
    pub(crate) bookmarks: Vec<usize>,
    pub(crate) highlights: [Vec<Range<usize>>; HIGHLIGHT_STYLES],
    /// The id of the text state it holds (`Document::state`), which an undo or redo back to it
    /// takes again: the save point is recognized by it.
    pub(crate) state: u64,
}

/// Undo history of one document, whichever view made each step.
pub(crate) struct History {
    pub(crate) undo: Vec<Snapshot>,
    pub(crate) redo: Vec<Snapshot>,
}

impl History {
    pub(crate) fn new() -> Self {
        Self { undo: Vec::new(), redo: Vec::new() }
    }
}

/// What a document knows about its CSV/TSV columns.
#[derive(Default)]
pub(crate) struct CsvMeta {
    /// Edit > Columns > Delimiter, until the document closes.
    pub(crate) forced: Option<Delim>,
    /// Detection's answer for the text of one version and file extension.
    pub(crate) detected: Option<(u64, Option<String>, Delim)>,
}

pub(crate) static NEXT_VIEW: AtomicU64 = AtomicU64::new(1);
pub(crate) static NEXT_BUFFER: AtomicU64 = AtomicU64::new(1);

/// The last `find_all`: the text it is for (version, length), query, options, matches, how
/// many of `changes` were logged when they were last brought up to date, the edits since
/// that `changes` no longer holds, the selection scope and the error that stopped it. They are
/// brought through those edits only when read (`found_up_to_date`): typing after a Find costs
/// nothing however many matches there are.
pub(crate) type Found = (
    (u64, usize),
    String,
    FindOpts,
    Vec<Range<usize>>,
    usize,
    Vec<Change>,
    Option<Vec<Range<usize>>>,
    // the search gave up (a runaway pattern, its time): no matches, and why
    Option<SearchError>,
);

/// One open document: its text, undo history and file. Every view of it reads and edits this
/// one; a view's own selections and marks are its `ViewModel`.
pub struct Document {
    pub rope: Rope,
    pub endings: Option<LineEndings>,
    /// Bumped by every text change; a view's positions belong to one version (`ViewModel`).
    pub version: u64,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub read_only: bool,
    /// Transient shared input lock, distinct from source overwrite protection.
    pub(crate) following: bool,
    /// User protection for every view; transient and independent of source disk permissions.
    pub(crate) viewer_mode: bool,
    /// Preferred for inserted boundaries; map-absent legacy documents use it for every LF.
    pub line_ending: &'static str,
    /// The file began with a UTF-8 byte order mark: kept out of the text, written back on save.
    pub bom: bool,
    pub encoding: orlo_project::encoding::Encoding,
    pub disk_encoding: orlo_project::encoding::Encoding,
    pub history_enabled: bool,
    /// The file as last read or written, so Orlo's own saves never look like outside changes.
    pub disk: Option<Stamp>,
    /// The file is gone from disk; the tab shows a red dot and Save asks for a path.
    pub deleted: bool,
    /// False after Always Ignore: outside changes are no longer reloaded or asked about.
    pub watching: bool,
    /// Columns per tab stop; `tab` and indenting insert `"\t"` unless `translate_tabs`.
    pub tab_size: usize,
    /// EditorConfig can use a different indentation step from the displayed tab width.
    /// `None` keeps the editor's normal tab-size-based indentation.
    pub indent_size: Option<usize>,
    pub translate_tabs: bool,
    /// Indentation chosen from a menu (width, style): settings reloads keep it.
    pub indent_override: [bool; 2],
    /// A line ending chosen from a menu: save-time policies keep it.
    pub line_ending_override: bool,
    /// The indentation the text was found to use when it was opened or guessed.
    pub inferred_indentation: Option<orlo_config::settings::Indentation>,
    /// Find Results: never dirty, so closing it never asks.
    pub scratch: bool,
    /// Find Results' match outlines, sorted and disjoint: edits map them, and undo and redo
    /// restore them.
    pub outlines: Vec<Range<usize>>,
    pub(crate) history: History,
    /// The id of the text as it stands: every change makes a new one, and undo or redo takes
    /// back the one its step was recorded with.
    pub(crate) state: u64,
    /// What the file holds as far as the document knows (`SavedPoint`); `None` when no undo or
    /// redo can bring the text back to it.
    pub(crate) saved: Option<SavedPoint>,
    pub(crate) csv: RefCell<CsvMeta>,
    pub(crate) id: u64,
    /// The view whose typing group is open: its next keystroke joins the last undo step. Any
    /// other view's change closes it.
    pub(crate) typing: std::cell::Cell<Option<u64>>,
    /// Text changes some view of this document has yet to apply, oldest first, and the
    /// version each view's positions belong to (`ViewModel::catch_up`).
    pub(crate) log: RefCell<std::collections::VecDeque<Rc<TextChange>>>,
    pub(crate) views: RefCell<Vec<std::rc::Weak<std::cell::Cell<u64>>>>,
}

/// One text change to a document as every view of it applies it: the versions it took the text
/// from and to, the edits in order, and the view that made it.
#[derive(Clone, Debug, PartialEq)]
pub struct TextChange {
    pub document: u64,
    pub before: u64,
    pub after: u64,
    pub changes: Vec<Change>,
    pub origin: u64,
}

/// A view's marks while a command's splices skip them (`EditorState::set_marks_aside`).
pub(crate) struct MarksAside {
    pub(crate) bookmarks: Vec<usize>,
    pub(crate) highlights: [Vec<Range<usize>>; HIGHLIGHT_STYLES],
    pub(crate) mark: Option<usize>,
    pub(crate) from: usize,
}

/// One view's own state in a document: what it selects and marks, and what it remembers of its
/// commands. Its positions belong to one version of the text (`revision`).
pub struct ViewModel {
    /// Opt-in navigation past line ends; selections remain view-local.
    pub virtual_space: VirtualSpace,
    /// Per-edit permission supplied by a mapped view; ordinary model edits use source tab stops.
    pub(crate) virtual_padding_tabs: bool,
    /// Transient keyboard mode; new and duplicated views always begin with insertion.
    pub overtype: bool,
    pub selections: Vec<Selection>,
    /// Index of the primary (newest) selection, which the caret position, scrolling and
    /// dragging follow; `normalize` keeps it on the same selection.
    pub(crate) primary: usize,
    /// Line starts, sorted; every edit maps them (`map_marks`).
    pub bookmarks: Vec<usize>,
    /// Sublime's Edit > Mark: a char offset that every edit maps.
    pub mark: Option<usize>,
    /// The text marked in each highlight style, sorted and disjoint per style. Edits,
    /// undo and reloads map it, a range whose text goes goes with it; per view, in memory.
    pub highlights: [Vec<Range<usize>>; HIGHLIGHT_STYLES],
    /// Text removed by Delete to Mark / Delete to End / Delete to Beginning, for Yank.
    pub(crate) kill: String,
    pub(crate) dirty_from_line: Option<usize>,
    /// The selections cmd-d left after expanding carets to words; while they stand, it
    /// adds whole-word matches only.
    pub(crate) quick_add_words: Vec<Selection>,
    /// The word Quick Find selected from a caret: while it stands, it finds whole words.
    pub(crate) quick_find_word: Option<Selection>,
    pub(crate) sel_hist: SelHistory,
    pub(crate) view_id: u64,
    /// This view's edits since `take_changes`.
    pub(crate) changes: Vec<Change>,
    pub(crate) metadata_changed: bool,
    /// Taken by `record`; becomes an undo step only once the text actually changes.
    pub(crate) pending: Option<Snapshot>,
    /// The last `find_all`: (version, length), query, options and its matches.
    pub(crate) found: RefCell<Option<Found>>,
    /// How many of `changes` the document's outlines were mapped through.
    pub(crate) outlines_at: usize,
    /// The fields of the snippet this view expanded last, while their session lasts.
    pub(crate) snippet: Option<Session>,
    /// The document version this view's positions belong to; the document reads it to know
    /// which changes some view still needs.
    pub(crate) revision: Rc<std::cell::Cell<u64>>,
}

/// A view at work on its document: what every editing command runs on. `D` is the document or
/// a borrow of it, `V` the view's state or a borrow of it, so a command that only reads the
/// text cannot change it.
pub struct Editing<D, V> {
    pub doc: D,
    pub view: V,
}

/// The document's own reads (text, lines, positions, encoding) through the view at work on it.
impl<D: DocRef, V> std::ops::Deref for Editing<D, V> {
    type Target = Document;
    fn deref(&self) -> &Document {
        &self.doc
    }
}

impl<D: DocMut, V> std::ops::DerefMut for Editing<D, V> {
    fn deref_mut(&mut self) -> &mut Document {
        &mut self.doc
    }
}

/// A document of its own with one view: panel fields, tests and files read off the UI thread.
pub type EditorState = Editing<Box<Document>, Box<ViewModel>>;

/// A view editing a document it shares.
pub type ViewEdit<'a> = Editing<&'a mut Document, &'a mut ViewModel>;

/// A view changing its selections in a document it only reads.
pub type ViewSelect<'a> = Editing<&'a Document, &'a mut ViewModel>;

/// A view and its document, read.
pub type ViewRead<'a> = Editing<&'a Document, &'a ViewModel>;

/// The document a command may change.
pub trait DocMut: std::ops::DerefMut<Target = Document> {}
impl<T: std::ops::DerefMut<Target = Document>> DocMut for T {}

/// The document a command may read.
pub trait DocRef: std::ops::Deref<Target = Document> {}
impl<T: std::ops::Deref<Target = Document>> DocRef for T {}

/// The view state a command may change.
pub trait ViewMut: std::ops::DerefMut<Target = ViewModel> {}
impl<T: std::ops::DerefMut<Target = ViewModel>> ViewMut for T {}

/// The view state a command may read.
pub trait ViewRef: std::ops::Deref<Target = ViewModel> {}
impl<T: std::ops::Deref<Target = ViewModel>> ViewRef for T {}

mod changes;
mod persistence;
mod position;
mod save;
mod state;
pub use crate::commands::{column_editing::*, transform::*};
pub use crate::search::model::*;
pub use crate::selection::*;
pub use changes::*;
pub use orlo_project::io::*;
pub use position::*;
pub use save::*;
pub(crate) use state::indentation;
#[cfg(test)]
mod tests;
