//! The session file: windows, layouts, open files with carets, scroll and bookmarks, and the
//! recent lists, restored on the next launch.

pub mod discarded;
pub mod folds;
pub mod project;
pub use project::WorkspaceFile;

use crate::layout::Layout;
use orlo_editor::text::{DocMut, DocRef, Document, Editing, EditorState, Selection, ViewMut, ViewRef};
use orlo_project::project::Root;
use orlo_project::watch::Stamp;
use ropey::Rope;
use serde::de::{DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Session {
    #[serde(deserialize_with = "each", serialize_with = "each_ok")]
    pub windows: Vec<WindowState>,
    #[serde(serialize_with = "each_ok")]
    pub recent_files: Vec<PathBuf>,
    #[serde(serialize_with = "each_ok")]
    pub recent_folders: Vec<PathBuf>,
    /// Project and workspace files opened lately, newest first.
    #[serde(serialize_with = "each_ok")]
    pub recent_projects: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct WindowState {
    /// x, y, w, h in points, relative to the display.
    pub bounds: Option<[f32; 4]>,
    /// The display's UUID, so the window comes back on the same monitor.
    pub display: Option<String>,
    pub maximized: bool,
    /// The first root, as sessions kept it before a window had several folders; still written, so an older
    /// Orlo opens it. `folders` wins when both are there.
    pub folder: Option<PathBuf>,
    /// The side bar's roots in order.
    #[serde(deserialize_with = "each", serialize_with = "each_ok")]
    pub folders: Vec<Root>,
    /// The window's project file and workspace file, if it has a project.
    pub project: Option<PathBuf>,
    pub workspace: Option<PathBuf>,
    pub expanded: Vec<PathBuf>,
    pub sidebar_visible: bool,
    pub outline_visible: bool,
    /// The width the user chose, not the one a narrow window drew.
    pub sidebar_width: f32,
    /// View > Side Bar > Show Open Files.
    pub show_open_files: bool,
    pub show_tabs: bool,
    pub show_status_bar: bool,
    pub git_sidebar_visible: bool,
    /// The window's View menu overrides of the settings file.
    pub font_size: Option<f32>,
    pub minimap: Option<bool>,
    pub rulers: Option<Vec<usize>>,
    /// View > Show White Space: `true` draws all, `false` none, over the settings.
    pub white_space: Option<bool>,
    pub layout: Layout,
    #[serde(deserialize_with = "each", serialize_with = "each_ok")]
    pub groups: Vec<GroupState>,
    pub active_group: usize,
    /// Recently closed named views, oldest first; never contains recovery text.
    #[serde(deserialize_with = "each", serialize_with = "each_ok")]
    pub closed_views: Vec<ViewState>,
    /// View > Sync Scrolling: vertical, horizontal.
    pub sync_scroll: [bool; 2],
    /// Where the results browser and the regex playground dock, and their sizes.
    pub panels: PanelPlaces,
    /// Distraction-free mode was on: what it showed. The fields above are the window as
    /// it was before, which leaving the mode brings back.
    pub distraction_free: Option<Chrome>,
}

/// Where a panel that moves sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dock {
    #[default]
    Bottom,
    Right,
}

/// A movable panel's dock and the sizes the user chose (clamped to the window when drawn).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelPlace {
    pub dock: Dock,
    pub height: f32,
    pub width: f32,
}

impl Default for PanelPlace {
    fn default() -> Self {
        Self { dock: Dock::Bottom, height: 330., width: 440. }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelPlaces {
    pub results: PanelPlace,
    pub playground: PanelPlace,
    pub terminal: PanelPlace,
    pub git: PanelPlace,
    pub changes: PanelPlace,
}

impl Default for PanelPlaces {
    fn default() -> Self {
        Self {
            results: PanelPlace::default(),
            terminal: PanelPlace::default(),
            git: PanelPlace { width: 360., ..PanelPlace::default() },
            changes: PanelPlace::default(),
            playground: PanelPlace { height: 230., ..PanelPlace::default() },
        }
    }
}

/// The parts of a window that distraction-free mode hides or shows.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chrome {
    pub sidebar: bool,
    pub git_sidebar: bool,
    pub tabs: bool,
    pub status_bar: bool,
    /// The window's minimap choice over the settings file (`None`: the file's).
    pub minimap: Option<bool>,
    pub panel_hidden: bool,
}

impl Default for Chrome {
    fn default() -> Self {
        Self { sidebar: true, git_sidebar: false, tabs: true, status_bar: true, minimap: None, panel_hidden: false }
    }
}

impl WindowState {
    /// A session from before a window had several folders: its one folder becomes the only root.
    /// Hand-edited or received files (a workspace file comes with a folder) may hold values a
    /// window cannot draw with: they fall back to sane ones.
    pub fn sanitize(&mut self) {
        self.migrate();
        if !self.layout.is_valid() {
            self.layout = Layout::default();
        }
        self.font_size = self.font_size.map(|f| if f.is_finite() { f.clamp(6., 72.) } else { 12. });
        for p in [
            &mut self.panels.results,
            &mut self.panels.playground,
            &mut self.panels.terminal,
            &mut self.panels.git,
            &mut self.panels.changes,
        ] {
            (p.height, p.width) = (p.height.clamp(60., 4000.), p.width.clamp(160., 6000.));
        }
        if let Some([_, _, width, height]) = &mut self.bounds {
            (*width, *height) = (width.max(200.), height.max(200.));
        }
    }

    pub fn migrate(&mut self) {
        if self.folders.is_empty() {
            self.folders.extend(self.folder.clone().map(Root::new));
        }
    }
}

/// A fresh window's values, so a key missing from the file changes nothing.
impl Default for WindowState {
    fn default() -> Self {
        Self {
            bounds: None,
            display: None,
            maximized: false,
            folder: None,
            folders: Vec::new(),
            project: None,
            workspace: None,
            expanded: Vec::new(),
            sidebar_visible: true,
            outline_visible: false,
            sidebar_width: 150., // theme::SIDEBAR_W
            show_open_files: false,
            show_tabs: true,
            show_status_bar: true,
            git_sidebar_visible: false,
            font_size: None,
            minimap: None,
            rulers: None,
            white_space: None,
            layout: Layout::default(),
            groups: Vec::new(),
            active_group: 0,
            closed_views: Vec::new(),
            sync_scroll: [false, false],
            panels: PanelPlaces::default(),
            distraction_free: None,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct GroupState {
    #[serde(deserialize_with = "each", serialize_with = "each_ok")]
    pub views: Vec<ViewState>,
    pub active: usize,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(default)]
pub struct ViewState {
    pub path: PathBuf,
    pub encoding: Option<orlo_editor::encoding::Encoding>,
    /// The last disk interpretation for BOM-less reads, including Reopen Closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_encoding_hint: Option<orlo_editor::encoding::Encoding>,
    /// (anchor, head) char offsets, primary last.
    pub selections: Vec<(usize, usize)>,
    /// First visible line, fractional, so a font change does not move it.
    pub scroll: f32,
    /// The first visible row's first char: a wrapped view's top row can be one inside `scroll`'s
    /// line.
    pub top: Option<usize>,
    /// Line numbers.
    pub bookmarks: Vec<usize>,
    /// Exact active folds, validated against the restored text before use.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "fold_record")]
    pub folds: Option<folds::FoldState>,
    /// View > Word Wrap's choice for this view, over the settings file.
    pub wrap: Option<bool>,
    /// View > Word Wrap Column's choice for this view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_width: Option<usize>,
    /// A pinned tab, a preview tab.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
    pub tab_size: Option<usize>,
    pub indent_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_override: Option<[bool; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_ending_override: Option<String>,
    pub translate_tabs: Option<bool>,
    /// Hot exit: the text of a buffer with unsaved changes (or deleted on disk), on the first
    /// of its views. `path` is empty for an untitled buffer.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "one")]
    pub unsaved: Option<Unsaved>,
    /// Views of one unsaved buffer share this number within the window; 0 for none.
    #[serde(skip_serializing_if = "is_zero")]
    pub buffer: usize,
}

fn fold_record<'de, D: Deserializer<'de>>(d: D) -> Result<Option<folds::FoldState>, D::Error> {
    let value = Value::deserialize(d)?;
    if value.is_null() {
        return Ok(None);
    }
    match serde_json::from_value(value) {
        Ok(folds) => Ok(Some(folds)),
        Err(_) => {
            DROPPED.set(true);
            Ok(None)
        }
    }
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What Sublime's hot exit keeps of a buffer with unsaved changes: its text and the file
/// facts a save needs, so it comes back exactly as it was left.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(default)]
pub struct Unsaved {
    /// The text, read from `file` by `load`; an older session has it inline.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// The text's file in the `Session` folder next to Session.json, as Sublime keeps them:
    /// a 100 MB buffer is not one JSON string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
    pub line_ending: String,
    /// Optional v1 compact l/w/r boundary kinds. Present invalid values never fall back.
    #[serde(default, deserialize_with = "present_endings", skip_serializing_if = "Option::is_none")]
    pub endings: Option<Value>,
    pub bom: bool,
    pub encoding: orlo_editor::encoding::Encoding,
    pub disk_encoding: orlo_editor::encoding::Encoding,
    /// Decoded lossily: Save must not overwrite the original.
    pub read_only: bool,
    /// The file as last read or written, so a change made while Orlo was closed is asked
    /// about at the first poll.
    pub disk: Option<Stamp>,
}

impl Default for Unsaved {
    fn default() -> Self {
        Self {
            text: String::new(),
            file: None,
            dirty: true,
            deleted: false,
            line_ending: "\n".into(),
            endings: None,
            bom: false,
            encoding: Default::default(),
            disk_encoding: Default::default(),
            read_only: false,
            disk: None,
        }
    }
}

impl Unsaved {
    /// `s`'s file facts, its text to go in `file` (see `save`).
    pub fn capture(s: &Document, file: String) -> Self {
        Self {
            text: String::new(),
            file: Some(file),
            dirty: s.dirty,
            deleted: s.deleted,
            line_ending: s.line_ending.into(),
            endings: s.endings.as_ref().map(|m| Value::String(m.compact())),
            bom: s.bom,
            encoding: s.encoding,
            disk_encoding: s.disk_encoding,
            read_only: s.read_only,
            disk: s.disk,
        }
    }

    /// The buffer again, unsaved: no undo step brings it back to a clean state.
    pub fn state(&self, path: &Path) -> io::Result<EditorState> {
        let mut s = EditorState::from_text(&self.text);
        s.doc.path = (!path.as_os_str().is_empty()).then(|| path.to_path_buf());
        s.doc.line_ending = line_ending(&self.line_ending).unwrap_or("\n");
        s.doc.endings = Some(
            self.boundaries(&s.doc.rope)?
                .unwrap_or_else(|| orlo_editor::buffer::LineEndings::uniform(&s.doc.rope, s.doc.line_ending)),
        );
        (s.doc.bom, s.doc.read_only, s.doc.deleted, s.doc.disk) = (self.bom, self.read_only, self.deleted, self.disk);
        s.doc.encoding = self.encoding;
        s.doc.disk_encoding = self.disk_encoding;
        if self.dirty {
            s.forget_saved_point();
        }
        if !self.dirty {
            s.doc.mark_saved();
        }
        Ok(s)
    }

    fn boundaries(&self, source: &ropey::Rope) -> io::Result<Option<orlo_editor::buffer::LineEndings>> {
        self.endings
            .as_ref()
            .map(|value| {
                let kinds = value
                    .as_str()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid line ending metadata"))?;
                orlo_editor::buffer::LineEndings::restore(kinds, source)
            })
            .transpose()
    }
}

fn present_endings<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

/// The document's line ending for one the session kept; `None` for a hand-edited other one.
fn line_ending(ending: &str) -> Option<&'static str> {
    ["\n", "\r\n", "\r"].into_iter().find(|&e| e == ending)
}

impl ViewState {
    /// `top` is the view's text on top and how far into its row; `wrap` is the view's, set by
    /// the caller.
    pub fn capture<D: DocRef, V: ViewRef>(path: PathBuf, s: &Editing<D, V>, (top, into): (usize, f32)) -> Self {
        Self {
            path,
            encoding: Some(s.doc.encoding),
            disk_encoding_hint: Some(s.doc.disk_encoding),
            selections: s.selections_primary_last().iter().map(|s| (s.anchor, s.head)).collect(),
            scroll: s.point(top).0 as f32 + into,
            top: Some(top),
            bookmarks: s.view.bookmarks.iter().map(|&b| s.point(b).0).collect(),
            folds: None,
            wrap: None,
            wrap_width: None,
            pinned: false,
            preview: false,
            tab_size: Some(s.doc.tab_size),
            indent_size: s.doc.indent_size,
            indent_override: None,
            line_ending_override: None,
            translate_tabs: Some(s.doc.translate_tabs),
            unsaved: None,
            buffer: 0,
        }
    }

    /// The text to put on top of a freshly loaded buffer and how far into its row: the saved
    /// row's start if it is still on the saved line, else that line's start.
    pub fn top(&self, s: &Document) -> (usize, f32) {
        let scroll = self.scroll.max(0.);
        let line = (scroll as usize).min(s.line_count() - 1);
        let at = (self.top).filter(|&t| t <= s.len() && s.point(t).0 == line).unwrap_or_else(|| s.line_start(line));
        (at, scroll.fract())
    }

    /// Selections and bookmarks onto a view of a freshly loaded document, clamped to its text;
    /// the indentation and line ending the view kept go to the document.
    pub fn apply<D: DocMut, V: ViewMut>(&self, s: &mut Editing<D, V>) {
        if let Some(size) = self.tab_size {
            s.doc.tab_size = size.clamp(1, 16);
        }
        s.doc.indent_size = self.indent_size.map(|n| n.clamp(1, 16));
        if let Some(ending) = self.line_ending_override.as_deref().and_then(line_ending) {
            s.doc.line_ending = ending;
        }
        if let Some(spaces) = self.translate_tabs {
            s.doc.translate_tabs = spaces;
        }
        self.apply_view(s);
    }

    /// Selections and bookmarks alone, onto a view of a document open already: its
    /// indentation and line ending stay as its other views have them.
    pub fn apply_view<D: DocRef, V: ViewMut>(&self, s: &mut Editing<D, V>) {
        let len = s.len();
        let sels = self.selections.iter().map(|&(a, h)| Selection::new(a.min(len), h.min(len))).collect::<Vec<_>>();
        if !sels.is_empty() {
            s.set_selections(sels);
        }
        let last = s.line_count() - 1;
        s.view.bookmarks = self.bookmarks.iter().map(|&l| s.line_start(l.min(last))).collect();
        s.view.bookmarks.sort();
        s.view.bookmarks.dedup();
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use orlo_utils::paths::TEST_SUPPORT;
pub use orlo_utils::paths::{private_dir, support_dir};

/// `~/Library/Application Support/Orlo/Session.json`
pub fn path() -> PathBuf {
    support_dir().join("Session.json")
}

thread_local! {
    /// `load` dropped something: the file is kept as a `.bad` snapshot before it is overwritten.
    static DROPPED: Cell<bool> = const { Cell::new(false) };
}

/// `T` from a JSON object, dropping each key whose value does not fit, so that field alone
/// falls back to its default; `None` when `v` is not an object.
fn lenient<T: DeserializeOwned>(v: Value) -> Option<T> {
    let Value::Object(map) = v else {
        DROPPED.set(true);
        return None;
    };
    let fits = |k: &String, v: &Value| {
        let one = Map::from_iter([(k.clone(), v.clone())]);
        let ok = serde_json::from_value::<T>(Value::Object(one)).is_ok();
        if !ok {
            DROPPED.set(true);
        }
        ok
    };
    let map: Map<String, Value> = map.into_iter().filter(|(k, v)| fits(k, v)).collect();
    serde_json::from_value(Value::Object(map)).ok()
}

/// A list read entry by entry with `lenient`; entries that are not objects are dropped.
fn each<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(v) => v.into_iter().filter_map(lenient).collect(),
        Value::Null => Vec::new(),
        _ => {
            DROPPED.set(true);
            Vec::new()
        }
    })
}

/// An optional entry read with `lenient`: a wrong field inside it loses that field alone,
/// not the hot-exit text beside it.
fn one<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Null => None,
        v => lenient(v),
    })
}

/// A list written entry by entry: one that cannot be (a path that is not UTF-8) is left out
/// and reported, where serde would fail the whole session.
fn each_ok<S: Serializer, T: Serialize>(v: &[T], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(v.iter().filter_map(|x| match serde_json::to_value(x) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("Orlo: left out of the session: {e}");
            None
        }
    }))
}

/// The file name for `s`'s text as it is now: this run, the document, the version, so a name is
/// never reused for other text.
pub fn text_name(s: &Document) -> String {
    static RUN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let run = RUN.get_or_init(|| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        format!("{:x}{:x}", std::process::id(), t.map_or(0, |d| d.as_nanos()))
    });
    format!("{run}-{}-{}.txt", s.id(), s.version)
}

/// Where the session keeps unsaved texts: `Session/` beside `Session.json`.
fn texts_dir(session: &Path) -> PathBuf {
    session.with_file_name("Session")
}

/// Draft references are file names generated by `text_name`, never paths out of Session/.
fn text_path(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let mut parts = Path::new(name).components();
    if !name.contains('\0')
        && matches!(parts.next(), Some(std::path::Component::Normal(n)) if n == name)
        && parts.next().is_none()
    {
        Ok(dir.join(name))
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid session draft file name"))
    }
}

fn read_text(mut file: std::fs::File) -> io::Result<String> {
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a regular file"));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

fn read_session(path: &Path) -> io::Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    read_text(std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?)
}

/// Pin the real drafts directory and refuse links or special files at its leaf names.
fn read_draft(dir: &Path, name: &str) -> io::Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    text_path(dir, name)?;
    let dir = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(dir)?;
    read_text(orlo_project::fsops::open_at(&dir, name.as_ref(), libc::O_RDONLY)?)
}

/// How the session file read at launch: what this run may then do to its files.
#[derive(Debug, PartialEq)]
pub enum Status {
    /// Read whole (or missing: a first launch). The texts it names are this run's to delete
    /// once no longer named, if every view is placed (`keep_bad` otherwise).
    Clean,
    /// Read in part: it and its texts were kept by `keep_bad`, and nothing of them is this
    /// run's to delete.
    Damaged,
    /// Unreadable, or damaged and not yet kept whole: saving must first preserve it.
    Unreadable(String),
}

/// The texts `s` names.
pub fn named(s: &Session) -> HashSet<String> {
    let views = s.windows.iter().flat_map(|w| &w.groups);
    (views.flat_map(|g| &g.views)).filter_map(|v| v.unsaved.as_ref()?.file.clone()).collect()
}

#[cfg(any(test, feature = "test-support"))]
pub fn load(path: &Path) -> Session {
    load_with(path).0
}

/// A snapshot `keep_bad` made, as Finder should show it: the session file kept, else the
/// folder of its texts.
pub type Snapshot = Option<PathBuf>;

/// A missing file is a first launch. An unreadable one is left as it is. One that is not
/// JSON, or that has parts that cannot be read, is kept as `keep_bad` keeps a session, and
/// wrong values in a readable one fall back to their defaults one by one. Texts in `Session/`
/// that the session does not name (a crash between a text and the session that names it) are
/// kept the same way, out of this run's reach; the snapshot made, if any, is returned.
pub fn load_with(path: &Path) -> (Session, Status, Snapshot) {
    let text = match read_session(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let kept = keep_unnamed(path, &HashSet::new());
            return (Session::default(), Status::Clean, kept);
        }
        Err(e) => {
            eprintln!("Orlo: cannot read session {}: {e}", path.display());
            return (Session::default(), Status::Unreadable(e.to_string()), None);
        }
    };
    let parsed = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string());
    DROPPED.set(false);
    let mut s = match parsed.and_then(|v| lenient::<Session>(v).ok_or("not an object".into())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Orlo: session {} is broken ({e})", path.display());
            let (status, kept) = keep_damaged(path, &HashSet::new());
            return (Session::default(), status, kept);
        }
    };
    // each unsaved text from its file; one that is gone leaves the view on the file as saved
    let dir = texts_dir(path);
    let names = named(&s);
    let views = s.windows.iter_mut().flat_map(|w| &mut w.groups);
    for v in views.flat_map(|g| &mut g.views) {
        let Some(u) = &mut v.unsaved else { continue };
        if let Some(name) = &u.file {
            match read_draft(&dir, name) {
                Ok(t) => u.text = t,
                Err(e) => {
                    eprintln!("Orlo: lost the unsaved text of {}: {e}", v.path.display());
                    DROPPED.set(true);
                    v.unsaved = None;
                    continue;
                }
            }
        }
        if u.boundaries(&ropey::Rope::from_str(&u.text)).is_err() {
            eprintln!("Orlo: invalid line ending metadata for {}", v.path.display());
            DROPPED.set(true);
            v.unsaved = None;
        }
    }
    let (status, kept) = match DROPPED.take() {
        true => {
            eprintln!("Orlo: parts of session {} could not be read", path.display());
            keep_damaged(path, &names)
        }
        false => (Status::Clean, keep_unnamed(path, &names)),
    };
    for w in &mut s.windows {
        w.sanitize();
    }
    (s, status, kept)
}

fn keep_damaged(path: &Path, names: &HashSet<String>) -> (Status, Snapshot) {
    match keep_bad(path, names) {
        Ok(snapshot) => (Status::Damaged, Some(snapshot)),
        Err(error) => (Status::Unreadable(format!("could not preserve the damaged session: {error}")), None),
    }
}

/// UTC now as `YYYYMMDD-HHMMSS`, which sorts as it reads.
fn now_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil_from_days (Howard Hinnant): days since 1970-01-01 to a Gregorian date
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, min, sec) = (rem / 3600, rem % 3600 / 60, rem % 60);
    format!("{y:04}{m:02}{d:02}-{h:02}{min:02}{sec:02}")
}

/// A new snapshot's names beside `path`: `Session.json.<time>.bad` and `Session.<time>.bad/`;
/// a second one in the same second gets `-2`, `-3` ... after the highest there, so they sort
/// in the order they were made.
fn bad_paths(path: &Path) -> (PathBuf, PathBuf) {
    bad_paths_at(path, now_stamp())
}

fn bad_paths_at(path: &Path, base: String) -> (PathBuf, PathBuf) {
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("Session.json");
    let prefix = format!("{file}.{base}");
    // A missing Session.json leaves only the drafts folder. It still owns this stamp:
    // reusing it would let rename replace an earlier snapshot's draft with the same name.
    let texts = texts_dir(path);
    let texts_name = texts.file_name().and_then(|n| n.to_str()).unwrap_or("Session");
    let texts_prefix = format!("{texts_name}.{base}");
    let dir = path.parent().unwrap_or(Path::new("."));
    let taken = (std::fs::read_dir(dir).into_iter().flatten().flatten())
        .filter_map(|e| {
            let n = e.file_name().into_string().ok()?;
            let rest = n.strip_prefix(&prefix).or_else(|| n.strip_prefix(&texts_prefix))?.strip_suffix(".bad")?;
            match rest {
                "" => Some(1),
                r => r.strip_prefix('-')?.parse::<u32>().ok(),
            }
        })
        .max();
    let stamp = match taken {
        None => base,
        Some(n) => format!("{base}-{}", n + 1),
    };
    let json = path.with_extension(format!("json.{stamp}.bad"));
    (json, texts_dir(path).with_extension(format!("{stamp}.bad")))
}

/// Keep the session as launch found it: the file copied to `Session.json.<time>.bad` (moved
/// when it cannot be read), and in `Session.<time>.bad/` the texts it names (copied: a view may
/// still be using them) and those it does not (moved there, out of this run's way). Nothing is
/// moved when the file cannot be kept. Older snapshots are pruned (`prune_bad`). The snapshot,
/// as Finder should show it.
pub fn keep_bad(path: &Path, named: &HashSet<String>) -> io::Result<PathBuf> {
    let (json, dir) = bad_paths(path);
    if path.exists() {
        let kept = orlo_project::io::copy_private(path, &json);
        if let Err(e) = kept.or_else(|error| {
            if error.kind() != io::ErrorKind::PermissionDenied {
                return Err(error);
            }
            orlo_project::io::make_private_file(path)?;
            orlo_project::fsops::rename(path, &json)
        }) {
            eprintln!("Orlo: could not keep the session as {}: {e}", json.display());
            return Err(e);
        }
    }
    move_texts(path, &dir, |n| !named.contains(n), true)?;
    eprintln!("Orlo: kept the session as {}", json.display());
    prune_bad(path, &json);
    Ok(if json.exists() { json } else { dir })
}

/// Texts in `Session/` the session does not name, moved to a `Session.<time>.bad/` folder
/// beside a copy of the session, when there are any.
fn keep_unnamed(path: &Path, named: &HashSet<String>) -> Snapshot {
    let texts = texts_dir(path);
    let unnamed = (std::fs::read_dir(&texts).into_iter().flatten().flatten())
        .any(|e| e.file_name().to_str().is_none_or(|n| !named.contains(n)));
    unnamed.then(|| keep_bad(path, named).ok()).flatten()
}

/// Each text in `Session/` into `to`: moved when `moved` says so, else copied when `copy`.
fn move_texts(path: &Path, to: &Path, moved: impl Fn(&str) -> bool, copy: bool) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let texts = texts_dir(path);
    match std::fs::symlink_metadata(&texts) {
        Ok(meta) if meta.is_dir() => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(_) => {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "The session draft folder is not a real directory"))
        }
    }
    let mut entries = match std::fs::read_dir(&texts) {
        Ok(entries) => entries.peekable(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if entries.peek().is_some() {
        std::fs::DirBuilder::new().mode(0o700).create(to)?;
        orlo_project::io::make_private_directory(to)?;
    }
    for e in entries {
        let e = e?;
        let name = e.file_name();
        let r = match name.to_str().is_none_or(&moved) || !e.file_type()?.is_file() {
            true => std::fs::rename(e.path(), to.join(&name)),
            false if copy => orlo_project::io::copy_private(&e.path(), &to.join(&name)),
            false => Ok(()),
        };
        if let Err(e) = r {
            eprintln!("Orlo: could not keep {}: {e}", to.join(&name).display());
            return Err(e);
        }
    }
    Ok(())
}

/// Snapshots beyond the newest five go, the file with its folder, oldest first by when the
/// filesystem says they were made (a clock set back does not reorder them); a folder alone
/// counts too. One holding unsaved texts stays until it is 30 days old as well, and the one
/// just `made` always stays.
fn prune_bad(path: &Path, made: &Path) {
    let (Some(dir), Some(file)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let texts = texts_dir(path);
    let Some(texts_name) = texts.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let (file_prefix, texts_prefix) = (format!("{file}."), format!("{texts_name}."));
    let mut made_at: HashMap<String, std::time::SystemTime> = HashMap::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(n) = e.file_name().into_string() else {
            continue;
        };
        let rest =
            (n.strip_prefix(&file_prefix)).or_else(|| n.strip_prefix(&texts_prefix).filter(|_| e.path().is_dir()));
        let Some(stamp) = rest.and_then(|r| r.strip_suffix(".bad")) else {
            continue;
        };
        let Ok(m) = e.metadata() else { continue };
        let t = m.created().or_else(|_| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        let at = made_at.entry(stamp.to_string()).or_insert(t);
        *at = (*at).min(t);
    }
    let mut all: Vec<_> = made_at.into_iter().map(|(stamp, t)| (t, stamp)).collect();
    all.sort();
    let now = std::time::SystemTime::now();
    let month = std::time::Duration::from_secs(30 * 86_400);
    let beyond = all.len().saturating_sub(5);
    for (t, stamp) in &all[..beyond] {
        let json = path.with_extension(format!("json.{stamp}.bad"));
        let folder = texts.with_extension(format!("{stamp}.bad"));
        let drafts = std::fs::read_dir(&folder).is_ok_and(|mut d| d.next().is_some());
        let old = now.duration_since(*t).is_ok_and(|age| age > month);
        if json == made || folder == made || (drafts && !old) {
            continue;
        }
        let _ = std::fs::remove_file(json);
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// The session could not be read, and the user asked for a new one, or Orlo is quitting: its
/// file and every text are kept by `keep_bad` (bytes, not parsed), and saving goes on from
/// scratch. An error when the file could not be kept: it must not be written over then.
pub fn set_aside(path: &Path) -> io::Result<()> {
    keep_bad(path, &HashSet::new()).map(drop)
}

/// Whether the session file reads now (it could not at launch).
pub fn readable(path: &Path) -> bool {
    read_session(path).is_ok()
}

/// Saved bounds onto a `screen` (w, h) big (a display gone, or now at a lower resolution):
/// shrunk to fit it, then shifted inside.
pub fn clamp_onto([x, y, w, h]: [f32; 4], [sw, sh]: [f32; 2]) -> [f32; 4] {
    let (w, h) = (w.min(sw), h.min(sh));
    [x.clamp(0., sw - w), y.clamp(0., sh - h), w, h]
}

/// A path on a drive that is not mounted (`/Volumes/<name>` missing): the session keeps it for
/// when the drive is back, where a file deleted from a mounted drive is dropped.
pub fn on_missing_volume(p: &Path) -> bool {
    use std::path::Component::{Normal, RootDir};
    let mut c = p.components();
    match (c.next(), c.next(), c.next()) {
        (Some(RootDir), Some(Normal(v)), Some(Normal(name))) if v == "Volumes" => {
            !Path::new("/Volumes").join(name).exists()
        }
        _ => false,
    }
}

/// Pretty JSON, written atomically, after the unsaved `texts` it names (file name, text) that
/// are not on disk yet. Then the texts it no longer names go, of those this run may delete:
/// `owned`, which gains what this write wrote. `Session/` is the user's only (0700), and so
/// is each text (0600).
pub fn save(path: &Path, s: &Session, texts: &[(String, Rope)], owned: &mut HashSet<String>) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = texts_dir(path);
    let names = named(s);
    for name in names.iter().chain(texts.iter().map(|(name, _)| name)) {
        text_path(&dir, name)?;
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    orlo_project::io::make_private_directory(&dir)?;
    for (name, text) in texts {
        // a name is a buffer at one version: there already means written whole
        let p = text_path(&dir, name)?;
        if !p.exists() {
            orlo_project::io::write_atomic_rope_private(&p, text)?;
            // Only a whole, private draft becomes this run's to delete.
            owned.insert(name.clone());
        }
    }
    orlo_project::io::write_atomic_private(path, &serde_json::to_string_pretty(s)?)?;
    for e in std::fs::read_dir(&dir)?.flatten() {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if !names.contains(&name) && owned.remove(&name) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn n25_recovery_new_old_and_corrupt_maps_are_isolated_and_retained() {
        let dir = temp_dir("orlo-n25-session");
        let path = dir.join("Session.json");
        let source = dir.join("source.txt");
        std::fs::write(&source, "a\r\nb\nc\rd").unwrap();
        let state = EditorState::load(&source).unwrap();
        let mut saved = Unsaved::capture(&state.doc, "valid.txt".into());
        saved.text = state.text();
        saved.file = None;
        let restored =
            serde_json::from_str::<Unsaved>(&serde_json::to_string(&saved).unwrap()).unwrap().state(&source).unwrap();
        assert_eq!(restored.doc.bytes_as_saved(None).unwrap(), b"a\r\nb\nc\rd");
        let legacy: Unsaved = serde_json::from_str(r#"{"text":"a\nb","line_ending":"\r\n"}"#).unwrap();
        assert_eq!(legacy.state(&source).unwrap().doc.bytes_as_saved(None).unwrap(), b"a\r\nb");
        let mut old = legacy.state(&source).unwrap();
        old.doc.set_line_ending("\r");
        old.set_cursor(old.len());
        old.insert("\nc");
        assert_eq!(old.doc.bytes_as_saved(None).unwrap(), b"a\r\nb\rc");
        for bad in [
            Value::Null,
            Value::Bool(false),
            Value::String("wl".into()),
            Value::String("wlx".into()),
            Value::String("wlrr".into()),
        ] {
            let mut invalid = saved.clone();
            invalid.endings = Some(bad);
            assert!(invalid.state(&source).is_err());
            let good = ViewState { unsaved: Some(saved.clone()), ..Default::default() };
            let broken = ViewState { unsaved: Some(invalid), ..Default::default() };
            let session = with_views(vec![good, broken]);
            std::fs::write(&path, serde_json::to_string(&session).unwrap()).unwrap();
            let (loaded, status, retained) = load_with(&path);
            assert_eq!(status, Status::Damaged);
            assert!(retained.is_some());
            assert!(loaded.windows[0].groups[0].views[0].unsaved.is_some());
            assert!(loaded.windows[0].groups[0].views[1].unsaved.is_none());
            assert_eq!(std::fs::read(&source).unwrap(), b"a\r\nb\nc\rd");
        }
    }

    #[test]
    fn navigation_outline_visibility_round_trips_per_window() {
        let state = super::WindowState { outline_visible: true, ..Default::default() };
        let text = serde_json::to_string(&state).unwrap();
        let restored: super::WindowState = serde_json::from_str(&text).unwrap();
        assert!(restored.outline_visible);
        assert!(!super::WindowState::default().outline_visible);
    }
    #[test]
    fn session_keeps_buffer_indentation_choices() {
        let mut before = EditorState::from_text("\tx");
        before.doc.tab_size = 2;
        before.doc.translate_tabs = false;
        let v = ViewState::capture(PathBuf::new(), &before, (0, 0.));
        let json = serde_json::to_string(&v).unwrap();
        let decoded: ViewState = serde_json::from_str(&json).unwrap();
        let mut after = EditorState::from_text("\tx");
        decoded.apply(&mut after);
        assert_eq!((after.doc.tab_size, after.doc.translate_tabs), (2, false));
    }

    /// A session with one window holding `views`.
    fn with_views(views: Vec<ViewState>) -> Session {
        Session {
            windows: vec![WindowState { groups: vec![GroupState { views, active: 0 }], ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn a_draft_reference_cannot_read_outside_the_session_folder() {
        let dir = temp_dir("orlo-session-draft-boundary");
        let path = dir.join("Session.json");
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, "outside private text").unwrap();
        std::fs::create_dir(texts_dir(&path)).unwrap();
        std::fs::write(texts_dir(&path).join("valid.txt"), "valid draft").unwrap();
        for name in [
            "../outside.txt".to_string(),
            outside.to_string_lossy().into_owned(),
            "".into(),
            ".".into(),
            "..".into(),
            "nested/draft".into(),
            "draft/".into(),
            "nul\0draft".into(),
        ] {
            let session = with_views(vec![draft_view(&name), draft_view("valid.txt")]);
            let json = serde_json::to_string(&session).unwrap();
            std::fs::write(&path, &json).unwrap();
            let (loaded, status, kept) = load_with(&path);
            assert_eq!(status, Status::Damaged, "{name:?}");
            let views = &loaded.windows[0].groups[0].views;
            assert!(views[0].unsaved.is_none(), "{name:?}");
            assert_eq!(views[1].unsaved.as_ref().unwrap().text, "valid draft");
            assert_eq!(std::fs::read_to_string(kept.unwrap()).unwrap(), json);
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside private text");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recovery_does_not_follow_a_draft_symlink() {
        let dir = temp_dir("orlo-session-draft-symlink");
        let path = dir.join("Session.json");
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, "outside private text").unwrap();
        std::fs::create_dir(texts_dir(&path)).unwrap();
        std::os::unix::fs::symlink(&outside, texts_dir(&path).join("draft.txt")).unwrap();
        let session = with_views(vec![draft_view("draft.txt")]);
        std::fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        let (loaded, _, _) = load_with(&path);
        assert!(loaded.windows[0].groups[0].views[0].unsaved.is_none());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside private text");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn session_save_refuses_a_symlinked_draft_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("orlo-session-folder-symlink");
        let path = dir.join("Session.json");
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&outside, texts_dir(&path)).unwrap();
        assert!(save(&path, &Session::default(), &[], &mut HashSet::new()).is_err());
        assert_eq!(mode(&outside), 0o755);
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recovery_does_not_move_files_from_a_symlinked_draft_directory() {
        let dir = temp_dir("orlo-session-linked-recovery");
        let path = dir.join("Session.json");
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "outside text").unwrap();
        std::os::unix::fs::symlink(&outside, texts_dir(&path)).unwrap();
        std::fs::write(&path, "{broken session").unwrap();
        let (_, status, _) = load_with(&path);
        assert!(matches!(status, Status::Unreadable(_)), "{status:?}");
        assert_eq!(std::fs::read_to_string(outside.join("keep.txt")).unwrap(), "outside text");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_session_pipe_does_not_wait_for_a_writer() {
        use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
        let dir = temp_dir("orlo-session-pipe");
        let path = dir.join("Session.json");
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: cpath is a live NUL-terminated pathname.
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let (send, receive) = std::sync::mpsc::channel();
        let reader_path = path.clone();
        let reader = std::thread::spawn(move || send.send(load_with(&reader_path).1).unwrap());
        let result = receive.recv_timeout(std::time::Duration::from_secs(2));
        if result.is_err() {
            // Release an old blocking reader before reporting the failed deadline.
            drop(std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(&path));
        }
        reader.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert!(matches!(result, Ok(Status::Unreadable(_))), "{result:?}");
    }

    #[test]
    fn a_draft_reference_cannot_write_outside_the_session_folder() {
        let dir = temp_dir("orlo-session-draft-write-boundary");
        let path = dir.join("Session.json");
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, "outside private text").unwrap();
        for name in ["../new.txt".to_string(), outside.to_string_lossy().into_owned()] {
            let result =
                save(&path, &Session::default(), &[(name, Rope::from_str("wrong destination"))], &mut HashSet::new());
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert!(!path.exists());
            assert!(!dir.join("new.txt").exists());
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside private text");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_clean_view_is_written_as_before_and_unsaved_text_round_trips() {
        let clean = ViewState::default();
        let json = serde_json::to_string(&clean).unwrap();
        assert!(!json.contains("unsaved") && !json.contains("buffer"), "{json}");
        let mut s = EditorState::from_text("a\nb");
        s.doc.line_ending = "\r\n";
        s.doc.bom = true;
        s.insert("x");
        let name = text_name(&s.doc);
        let v = ViewState { unsaved: Some(Unsaved::capture(&s.doc, name.clone())), buffer: 1, ..ViewState::default() };
        let dir = temp_dir("orlo-session-unsaved");
        let p = dir.join("Session.json");
        save(&p, &with_views(vec![v]), &[(name.clone(), s.doc.rope.clone())], &mut HashSet::new()).unwrap();
        let json = std::fs::read_to_string(&p).unwrap();
        assert!(!json.contains("xa"), "the text is in its own file: {json}");
        assert_eq!(std::fs::read_to_string(dir.join("Session").join(&name)).unwrap(), "xa\nb");
        let back = load(&p).windows.remove(0).groups.remove(0).views.remove(0);
        let r = back.unsaved.unwrap().state(Path::new("")).unwrap();
        assert_eq!((r.text(), r.doc.line_ending, r.doc.bom, r.doc.dirty), (s.text(), "\r\n", true, true));
        assert_eq!(r.doc.path, None);
        assert!(!dir.join("Session.json.bad").exists());
        // a session from before keeps its text inline
        std::fs::write(&p, r#"{"windows": [{"groups": [{"views": [{"path": "", "unsaved": {"text": "old"}}]}]}]}"#)
            .unwrap();
        let back = load(&p).windows.remove(0).groups.remove(0).views.remove(0);
        assert_eq!(back.unsaved.unwrap().text, "old");
    }

    #[test]
    fn texts_no_longer_named_are_deleted_after_a_write() {
        let dir = temp_dir("orlo-session-orphans");
        let p = dir.join("Session.json");
        let a = EditorState::from_text("a");
        let unsaved = |s: &EditorState| ViewState {
            unsaved: Some(Unsaved::capture(&s.doc, text_name(&s.doc))),
            ..ViewState::default()
        };
        // what this run wrote is its own to delete
        let mut owned = HashSet::new();
        let first = [(text_name(&a.doc), a.doc.rope.clone())];
        save(&p, &with_views(vec![unsaved(&a)]), &first, &mut owned).unwrap();
        let mut b = EditorState::from_text("b");
        b.insert("x");
        let second = [(text_name(&b.doc), b.doc.rope.clone())];
        save(&p, &with_views(vec![unsaved(&b)]), &second, &mut owned).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.join("Session"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [text_name(&b.doc)]);
        // Cleanup owns directory entries, even if one was replaced by a symlink.
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, "outside text").unwrap();
        let link = dir.join("Session/owned-link.txt");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        owned.insert("owned-link.txt".into());
        save(&p, &Session::default(), &[], &mut owned).unwrap();
        assert_eq!(std::fs::symlink_metadata(&link).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "outside text");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn session_drafts_do_not_inherit_public_read_or_search_acls() {
        let dir = temp_dir("orlo-session-draft-acl");
        let path = dir.join("Session.json");
        chmod(INHERITED_READ, &dir);
        let editor = EditorState::from_text("private draft text");
        let name = text_name(&editor.doc);
        let session = with_views(vec![ViewState {
            unsaved: Some(Unsaved::capture(&editor.doc, name.clone())),
            ..Default::default()
        }]);
        let saved = save(&path, &session, &[(name.clone(), editor.doc.rope.clone())], &mut HashSet::new());
        let folder_acl = acl(&texts_dir(&path));
        let draft_acl = acl(&texts_dir(&path).join(name));
        chmod(&["-RN"], &dir);
        std::fs::remove_dir_all(dir).unwrap();
        saved.unwrap();
        assert_eq!(folder_acl.lines().count(), 1, "{folder_acl}\n{draft_acl}");
        assert_eq!(draft_acl.lines().count(), 1, "{draft_acl}");
    }

    #[test]
    fn unsaved_texts_are_readable_by_their_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("orlo-session-private");
        let p = dir.join("Session.json");
        // the folder as an older Orlo left it
        std::fs::create_dir_all(dir.join("Session")).unwrap();
        std::fs::set_permissions(dir.join("Session"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let a = EditorState::from_text("draft");
        let view = ViewState { unsaved: Some(Unsaved::capture(&a.doc, text_name(&a.doc))), ..ViewState::default() };
        let texts = [(text_name(&a.doc), a.doc.rope.clone())];
        save(&p, &with_views(vec![view]), &texts, &mut HashSet::new()).unwrap();
        let text = dir.join("Session").join(text_name(&a.doc));
        assert_eq!((mode(&dir.join("Session")), mode(&text)), (0o700, 0o600));
    }

    #[test]
    fn session_json_keeps_inline_recovery_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("orlo-session-private-json");
        let path = dir.join("Session.json");
        std::fs::write(&path, "old session").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let session = with_views(vec![ViewState {
            unsaved: Some(Unsaved { text: "private inline recovery".into(), ..Default::default() }),
            ..Default::default()
        }]);
        save(&path, &session, &[], &mut HashSet::new()).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(load(&path), session);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_dated_before_1970_does_not_lose_the_session() {
        let dir = temp_dir("orlo-session-1969");
        let f = dir.join("old.txt");
        std::fs::write(&f, "x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH - std::time::Duration::from_secs(86400))
            .unwrap();
        let mut s = EditorState::load(&f).unwrap();
        assert_eq!(s.doc.disk.unwrap().mtime.0, -86400);
        s.insert("y");
        let name = text_name(&s.doc);
        let v = ViewState {
            path: f.clone(),
            unsaved: Some(Unsaved::capture(&s.doc, name.clone())),
            ..ViewState::default()
        };
        let p = dir.join("Session.json");
        save(&p, &with_views(vec![v]), &[(name, s.doc.rope.clone())], &mut HashSet::new()).unwrap();
        let back = load(&p).windows.remove(0).groups.remove(0).views.remove(0);
        assert_eq!(back.unsaved.unwrap().disk, s.doc.disk);
    }

    #[test]
    fn a_malformed_hot_exit_entry_keeps_its_text_and_the_file() {
        let dir = temp_dir("orlo-session-malformed");
        let p = dir.join("Session.json");
        let json = r#"{"windows": [{"groups": [{"views": [{"path": "/a",
            "unsaved": {"text": "keep me", "dirty": "yes", "disk": {"mtime": "?"}}}]}]}]}"#;
        std::fs::write(&p, json).unwrap();
        let back = load(&p).windows.remove(0).groups.remove(0).views.remove(0);
        let u = back.unsaved.unwrap();
        assert_eq!((u.text.as_str(), u.dirty, u.disk), ("keep me", true, None));
        assert_eq!(kept(&p)[0].0, json, "kept before the next write replaces it");
        // a text whose file is gone: the view stays, on the file as saved
        std::fs::write(
            &p,
            r#"{"windows": [{"groups": [{"views": [{"path": "/a", "unsaved": {"file": "gone.txt"}}]}]}]}"#,
        )
        .unwrap();
        let back = load(&p).windows.remove(0).groups.remove(0).views.remove(0);
        assert_eq!((back.path, back.unsaved), (PathBuf::from("/a"), None));
    }

    #[test]
    fn an_old_sessions_stamp_still_reads() {
        let old = r#"{"mtime": {"secs_since_epoch": 5, "nanos_since_epoch": 6}, "len": 1,
                      "ctime": [7, 8], "ino": 9}"#;
        let s: Stamp = serde_json::from_str(old).unwrap();
        assert_eq!((s.mtime, s.ctime), ((5, 6), (7, 8)));
    }

    #[test]
    fn a_wrapped_views_top_row_comes_back() {
        let s = EditorState::from_text("aaaa\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\ncc");
        // the top row starts at 12, inside line 1 (which starts at 5), half scrolled past
        let v = ViewState::capture(PathBuf::from("/x"), &s, (12, 0.5));
        assert_eq!((v.scroll, v.top), (1.5, Some(12)));
        assert_eq!(v.top(&s.doc), (12, 0.5));
        let shorter = EditorState::from_text("aaaa\nb\ncc");
        assert_eq!(v.top(&shorter.doc), (5, 0.5), "changed since: the line's start");
        let old: ViewState = serde_json::from_str(r#"{"path": "/x", "scroll": 1.5}"#).unwrap();
        assert_eq!(old.top(&s.doc), (5, 0.5), "a session from before");
    }

    use super::*;

    #[test]
    fn a_session_that_cannot_be_read_is_set_aside_whole() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("orlo-session-set-aside");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Session")).unwrap();
        let path = dir.join("Session.json");
        std::fs::write(&path, "{\"windows\": []}").unwrap();
        std::fs::write(dir.join("Session/a.txt"), "draft").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        set_aside(&path).unwrap();
        let kept: Vec<_> = (std::fs::read_dir(&dir).unwrap().flatten())
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".bad"))
            .collect();
        let json = kept.iter().find(|p| p.is_file()).expect("the file kept");
        std::fs::set_permissions(json, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read_to_string(json).unwrap(), "{\"windows\": []}");
        assert!(kept.iter().any(|p| p.join("a.txt").exists()), "{kept:?}");
    }

    use crate::layout::Preset;
    use orlo_editor::text::{EditorState, Selection};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `p`'s permission bits.
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    /// The `.bad` snapshots in `dir`, by name.
    fn bad_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = (std::fs::read_dir(dir).unwrap().flatten())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".bad"))
            .collect();
        names.sort();
        names
    }

    /// A view whose unsaved text is in the session's file `file`.
    fn draft_view(file: &str) -> ViewState {
        ViewState { unsaved: Some(Unsaved { file: Some(file.into()), ..Default::default() }), ..Default::default() }
    }

    #[cfg(target_os = "macos")]
    fn chmod(args: &[&str], p: &Path) {
        let status = std::process::Command::new("chmod").args(args).arg(p).status();
        assert!(status.unwrap().success());
    }

    /// `ls -lde`: one line, or more when `p` has an access control list.
    #[cfg(target_os = "macos")]
    fn acl(p: &Path) -> String {
        let out = std::process::Command::new("ls").arg("-lde").arg(p).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    }

    #[cfg(target_os = "macos")]
    const INHERITED_READ: &[&str] = &["+a", "everyone allow read,execute,file_inherit,directory_inherit"];

    fn view(path: &str) -> ViewState {
        ViewState {
            path: path.into(),
            selections: vec![(1, 3), (7, 7)],
            scroll: 2.5,
            top: Some(12),
            bookmarks: vec![0, 4],
            wrap: Some(false),
            ..ViewState::default()
        }
    }

    #[test]
    fn session_round_trips_through_json() {
        let s = Session {
            windows: vec![
                WindowState {
                    bounds: Some([10., 20., 800., 600.]),
                    display: Some("37D8832A-2D66-02CA-B9F7-8F30A301B230".into()),
                    folder: Some("/tmp/x".into()),
                    folders: vec![
                        Root::new("/tmp/x".into()),
                        Root {
                            name: Some("Lib".into()),
                            file_exclude_patterns: vec!["*.o".into()],
                            ..Root::new("/tmp/lib".into())
                        },
                    ],
                    project: Some("/tmp/x/x.sublime-project".into()),
                    workspace: Some("/tmp/x/x.orlo-workspace".into()),
                    expanded: vec!["/tmp/x".into(), "/tmp/x/src".into()],
                    sidebar_width: 222.,
                    rulers: Some(vec![80]),
                    layout: Layout::preset(Preset::Columns(2)),
                    groups: vec![
                        GroupState { views: vec![view("/tmp/x/a"), view("/tmp/x/b")], active: 1 },
                        GroupState { views: vec![view("/tmp/x/a")], active: 0 },
                    ],
                    active_group: 1,
                    ..Default::default()
                },
                WindowState { maximized: true, ..Default::default() },
            ],
            recent_files: vec!["/tmp/x/a".into()],
            recent_folders: vec!["/tmp/x".into()],
            recent_projects: vec!["/tmp/x/x.sublime-project".into()],
        };
        let dir = temp_dir("orlo-session-json-round-trip");
        let p = dir.join("Session.json");
        save(&p, &s, &[], &mut HashSet::new()).unwrap();
        assert_eq!(load(&p), s);
    }

    #[test]
    fn missing_or_invalid_file_loads_empty() {
        let dir = temp_dir("orlo-session-invalid");
        assert_eq!(load(&dir.join("nope.json")), Session::default());
        let p = dir.join("Session.json");
        std::fs::write(&p, "{oops").unwrap();
        assert_eq!(load(&p), Session::default());
    }

    /// The snapshots `keep_bad` made beside `p`, oldest first: (the file's text, its folder's
    /// texts).
    fn kept(p: &Path) -> Vec<(String, Vec<String>)> {
        let dir = p.parent().unwrap();
        let mut stamps: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| {
                let n = e.unwrap().file_name().into_string().ok()?;
                Some(n.strip_prefix("Session.json.")?.strip_suffix(".bad")?.into())
            })
            .collect();
        stamps.sort_by_key(|st| {
            let (t, k) = st.split_at(st.len().min(15));
            (t.to_string(), k.trim_start_matches('-').parse::<u32>().unwrap_or(1))
        });
        stamps
            .iter()
            .map(|stamp| {
                let folder = dir.join(format!("Session.{stamp}.bad"));
                let mut texts: Vec<String> = std::fs::read_dir(folder)
                    .into_iter()
                    .flatten()
                    .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
                    .collect();
                texts.sort();
                let json = dir.join(format!("Session.json.{stamp}.bad"));
                (std::fs::read_to_string(json).unwrap(), texts)
            })
            .collect()
    }

    #[test]
    fn an_unparsable_session_is_kept_with_its_texts_and_five_newer_are_kept() {
        let dir = temp_dir("orlo-session-bad");
        let p = dir.join("Session.json");
        std::fs::create_dir_all(dir.join("Session")).unwrap();
        std::fs::write(dir.join("Session/a.txt"), "draft").unwrap();
        std::fs::write(&p, "{oops").unwrap();
        assert_eq!(load(&p), Session::default());
        assert_eq!(kept(&p), [("{oops".to_string(), vec!["draft".to_string()])]);
        // a second in the same second does not replace it; the sixth drops the oldest without
        // drafts, and the one with them stays (a month)
        for k in 0..6 {
            std::fs::write(&p, format!("{{oops {k}")).unwrap();
            load(&p);
        }
        let all: Vec<_> = kept(&p).into_iter().map(|(json, _)| json).collect();
        assert_eq!(all, ["{oops", "{oops 1", "{oops 2", "{oops 3", "{oops 4", "{oops 5"]);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn recovery_drafts_remain_private_when_the_support_folder_has_inherited_acls() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("orlo-session-recovery-acl");
        let path = dir.join("Session.json");
        let live = texts_dir(&path);
        std::fs::create_dir(&live).unwrap();
        std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o700)).unwrap();
        let draft = live.join("draft.txt");
        std::fs::write(&draft, b"private recovery text\0exact bytes").unwrap();
        std::fs::set_permissions(&draft, std::fs::Permissions::from_mode(0o600)).unwrap();
        let original = b"{broken session with private inline text\xff";
        std::fs::write(&path, original).unwrap();
        chmod(INHERITED_READ, &dir);
        let mut snapshots = Vec::new();
        for copy in [true, false] {
            let named = if copy { HashSet::from(["draft.txt".into()]) } else { HashSet::new() };
            let json = keep_bad(&path, &named).unwrap();
            let folder = json.with_file_name(json.file_name().unwrap().to_str().unwrap().replacen(
                "Session.json.",
                "Session.",
                1,
            ));
            let saved = folder.join("draft.txt");
            snapshots.push((
                mode(&folder),
                acl(&folder),
                acl(&saved),
                std::fs::read(&saved).unwrap(),
                mode(&json),
                acl(&json),
                std::fs::read(&json).unwrap(),
                draft.exists(),
                copy,
            ));
        }
        chmod(&["-RN"], &dir);
        std::fs::remove_dir_all(dir).unwrap();
        for (mode, folder_acl, draft_acl, bytes, json_mode, json_acl, json, source_exists, copy) in snapshots {
            assert_eq!(folder_acl.lines().count(), 1, "{folder_acl}");
            assert_eq!(draft_acl.lines().count(), 1, "{draft_acl}");
            assert_eq!(mode, 0o700);
            assert_eq!(bytes, b"private recovery text\0exact bytes");
            assert_eq!(json_mode, 0o600);
            assert_eq!(json_acl.lines().count(), 1, "{json_acl}");
            assert_eq!(json, original);
            assert_eq!(source_exists, copy);
        }
    }

    #[test]
    fn a_failed_private_recovery_backup_keeps_the_original_session_and_drafts() {
        let dir = temp_dir("orlo-session-recovery-write-fails");
        let path = dir.join("Session.json");
        let original = b"{private inline text\xff\0";
        std::fs::write(&path, original).unwrap();
        std::fs::create_dir(texts_dir(&path)).unwrap();
        let draft = texts_dir(&path).join("draft.txt");
        std::fs::write(&draft, "private draft").unwrap();
        orlo_project::io::FAULT.set(Some(("write", io::ErrorKind::Other)));
        let result = keep_bad(&path, &HashSet::new());
        orlo_project::io::FAULT.set(None);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&draft).unwrap(), "private draft");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn an_unreadable_recovery_file_is_private_before_its_rename() {
        let dir = temp_dir("orlo-session-recovery-unreadable-acl");
        let path = dir.join("Session.json");
        let original = b"private raw session\xff\0";
        std::fs::write(&path, original).unwrap();
        chmod(&["+a", "everyone deny read"], &path);
        assert_eq!(std::fs::read(&path).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let saved = keep_bad(&path, &HashSet::new()).unwrap();
        assert_eq!(acl(&saved).lines().count(), 1);
        assert_eq!(mode(&saved), 0o600);
        assert_eq!(std::fs::read(&saved).unwrap(), original);
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_recovery_backup_blocks_saving_over_a_damaged_session() {
        let dir = temp_dir("orlo-session-recovery-blocked");
        let path = dir.join("Session.json");
        for original in [
            "{broken private session",
            r#"{"windows":[{"groups":[{"views":[{"unsaved":{"text":"private","dirty":"yes"}}]}]}]}"#,
        ] {
            std::fs::write(&path, original).unwrap();
            orlo_project::io::FAULT.set(Some(("write", io::ErrorKind::Other)));
            let (_, status, kept) = load_with(&path);
            orlo_project::io::FAULT.set(None);
            assert!(matches!(status, Status::Unreadable(_)), "{status:?}");
            assert!(kept.is_none());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_incomplete_draft_backup_blocks_saving_and_can_be_retried() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("orlo-session-recovery-draft-fails");
        let path = dir.join("Session.json");
        let original = r#"{"windows":[{"groups":[{"views":[{"unsaved":{"file":"draft.txt"}}]}]}]}"#;
        std::fs::write(&path, original).unwrap();
        std::fs::create_dir(texts_dir(&path)).unwrap();
        let draft = texts_dir(&path).join("draft.txt");
        std::fs::write(&draft, "private draft bytes").unwrap();
        std::fs::set_permissions(&draft, std::fs::Permissions::from_mode(0o000)).unwrap();
        let (_, status, kept) = load_with(&path);
        std::fs::set_permissions(&draft, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(status, Status::Unreadable(_)), "{status:?}");
        assert!(kept.is_none(), "a partial snapshot is not a complete backup");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&draft).unwrap(), "private draft bytes");
        let before = self::kept(&path);
        assert_eq!(before, [(original.to_string(), Vec::<String>::new())]);
        set_aside(&path).unwrap();
        assert!(!draft.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(self::kept(&path), [before[0].clone(), (original.to_string(), vec!["private draft bytes".into()])]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_recovery_folder_collision_does_not_follow_a_symlink_or_move_a_draft() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("orlo-session-recovery-folder-collision");
        let path = dir.join("Session.json");
        std::fs::create_dir(texts_dir(&path)).unwrap();
        let draft = texts_dir(&path).join("draft.txt");
        std::fs::write(&draft, "private draft").unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
        let target = dir.join("snapshot");
        std::os::unix::fs::symlink(&outside, &target).unwrap();
        assert_eq!(move_texts(&path, &target, |_| true, true).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(std::fs::symlink_metadata(&target).unwrap().is_symlink());
        assert_eq!(mode(&outside), 0o755);
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        assert_eq!(std::fs::read_to_string(&draft).unwrap(), "private draft");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Make `p` (a snapshot's file or folder) look `days` old: an earlier modified time moves
    /// the birth time back too.
    fn age(p: &Path, days: u64) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
        std::fs::File::open(p).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn a_snapshot_just_made_stays_though_the_clock_went_back() {
        let dir = temp_dir("orlo-session-clock-back");
        let p = dir.join("Session.json");
        // five made "in 2099": a clock set back since names the next one earlier
        for k in 0..5 {
            std::fs::write(dir.join(format!("Session.json.20990101-00000{k}.bad")), "{}").unwrap();
        }
        std::fs::write(&p, "{oops").unwrap();
        let (_, status, made) = load_with(&p);
        assert_eq!(status, Status::Damaged);
        let made = made.unwrap();
        assert_eq!(std::fs::read_to_string(&made).unwrap(), "{oops");
        let names = bad_names(&dir);
        assert_eq!(names.len(), 5, "{names:?}");
        assert!(!names.contains(&"Session.json.20990101-000000.bad".into()), "{names:?}");
    }

    #[test]
    fn folders_of_drafts_alone_count_and_go_only_beyond_five_and_a_month_old() {
        let dir = temp_dir("orlo-session-draft-folders");
        let p = dir.join("Session.json");
        // six launches each found a draft no session named (no Session.json: none to copy)
        for k in 0..6 {
            let d = dir.join(format!("Session.20260101-00000{k}.bad"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("a.txt"), format!("draft {k}")).unwrap();
        }
        for k in 0..2 {
            age(&dir.join(format!("Session.20260101-00000{k}.bad")), 40);
        }
        age(&dir.join("Session.20260101-000002.bad"), 20);
        std::fs::create_dir_all(dir.join("Session")).unwrap();
        std::fs::write(dir.join("Session/b.txt"), "draft b").unwrap();
        let (_, status, made) = load_with(&p);
        assert_eq!(status, Status::Clean);
        assert!(made.unwrap().join("b.txt").exists());
        // seven: the two beyond five that are a month old go; the one 20 days old stays
        let left = bad_names(&dir);
        assert_eq!(left.len(), 5, "{left:?}");
        assert_eq!(left[0], "Session.20260101-000002.bad");
    }

    #[test]
    fn folder_only_snapshots_reserve_their_timestamp_and_suffix() {
        let dir = temp_dir("orlo-session-folder-snapshot-collision");
        let path = dir.join("Session.json");
        let stamp = "20260101-000000";
        let mut snapshots = Vec::new();
        for text in ["first draft", "second draft", "third draft"] {
            std::fs::create_dir_all(dir.join("Session")).unwrap();
            std::fs::write(dir.join("Session/a.txt"), text).unwrap();
            let (json, folder) = bad_paths_at(&path, stamp.into());
            assert!(!json.exists());
            move_texts(&path, &folder, |_| true, true).unwrap();
            snapshots.push(folder);
        }
        assert_eq!(snapshots[0], dir.join("Session.20260101-000000.bad"));
        assert_eq!(snapshots[1], dir.join("Session.20260101-000000-2.bad"));
        assert_eq!(snapshots[2], dir.join("Session.20260101-000000-3.bad"));
        let texts: Vec<_> = snapshots.iter().map(|p| std::fs::read_to_string(p.join("a.txt")).unwrap()).collect();
        assert_eq!(texts, ["first draft", "second draft", "third draft"]);
    }

    #[test]
    fn a_draft_whose_privacy_cannot_be_secured_keeps_the_previous_session() {
        let dir = temp_dir("orlo-session-chmod-fails");
        let p = dir.join("Session.json");
        let mut s = EditorState::from_text("");
        s.insert("x");
        let name = text_name(&s.doc);
        let with_draft = with_views(vec![ViewState {
            unsaved: Some(Unsaved::capture(&s.doc, name.clone())),
            ..ViewState::default()
        }]);
        let texts = [(name.clone(), s.doc.rope.clone())];
        let mut owned = HashSet::new();
        let original = serde_json::to_string(&Session::default()).unwrap();
        std::fs::write(&p, &original).unwrap();
        orlo_project::io::FAULT.set(Some(("chmod", io::ErrorKind::PermissionDenied)));
        let r = save(&p, &with_draft, &texts, &mut owned);
        orlo_project::io::FAULT.set(None);
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        assert!(!owned.contains(&name));
        assert!(!dir.join("Session").join(&name).exists());
        // A retry owns only a complete private draft, which can be removed when closed.
        save(&p, &with_draft, &texts, &mut owned).unwrap();
        assert!(owned.contains(&name));
        save(&p, &Session::default(), &[], &mut owned).unwrap();
        assert!(!dir.join("Session").join(&name).exists());
    }

    #[test]
    fn a_wrong_typed_value_falls_back_alone() {
        let dir = temp_dir("orlo-session-lenient");
        let p = dir.join("Session.json");
        std::fs::write(
            &p,
            r#"{"recent_files": ["/a"], "recent_folders": 5,
                "windows": [{"folder": "/x", "sidebar_width": "wide", "show_tabs": false,
                             "groups": [{"views": [{"path": "/a", "scroll": "x", "bookmarks": [2]},
                                                   7, {"path": "/b"}]}]},
                            "not a window"]}"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.recent_files, [PathBuf::from("/a")]);
        assert!(s.recent_folders.is_empty());
        let w = &s.windows[..];
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].folder, Some("/x".into()));
        assert_eq!(w[0].sidebar_width, 150., "the window default, not 0");
        assert!(!w[0].show_tabs);
        let views = &w[0].groups[0].views;
        assert_eq!(views.len(), 2);
        assert_eq!((views[0].scroll, &views[0].bookmarks[..]), (0., &[2][..]));
        assert_eq!(views[1].path, PathBuf::from("/b"));
        assert!(p.exists(), "a readable session is not moved away");
        std::fs::write(&p, r#"{"windows": "nope", "recent_files": ["/a"]}"#).unwrap();
        assert_eq!(load(&p).recent_files, [PathBuf::from("/a")]);
    }

    #[test]
    fn no_saved_selection_leaves_a_caret() {
        let mut s = EditorState::from_text("abc");
        ViewState { selections: vec![], ..view("x") }.apply(&mut s);
        assert_eq!(s.view.selections, vec![Selection::caret(0)]);
    }

    #[test]
    fn a_window_from_a_display_that_is_gone_is_moved_onto_the_screen() {
        assert_eq!(clamp_onto([3000., 900., 800., 600.], [1440., 900.]), [640., 300., 800., 600.]);
        assert_eq!(clamp_onto([-50., -20., 2000., 1000.], [1440., 900.]), [0., 0., 1440., 900.]);
        assert_eq!(clamp_onto([10., 20., 800., 600.], [1440., 900.]), [10., 20., 800., 600.]);
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let dir = temp_dir("orlo-session-temp");
        save(&dir.join("Session.json"), &Session::default(), &[], &mut HashSet::new()).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["Session", "Session.json"]);
    }

    #[test]
    fn apply_clamps_to_a_shorter_file() {
        let mut s = EditorState::from_text("ab\ncd");
        ViewState {
            path: "x".into(),
            selections: vec![(1, 99)],
            scroll: 50.,
            top: None,
            bookmarks: vec![1, 7],
            ..ViewState::default()
        }
        .apply(&mut s);
        assert_eq!(s.view.selections, vec![Selection::new(1, 5)]);
        assert_eq!(s.view.bookmarks, vec![3]);
    }

    #[test]
    fn apply_makes_the_last_selection_primary() {
        let mut s = EditorState::from_text("abcdef");
        let v = ViewState { selections: vec![(4, 4), (0, 1)], ..view("x") };
        v.apply(&mut s);
        assert_eq!(s.cursor(), 1);
        assert_eq!(ViewState::capture("x".into(), &s, (0, 0.)).selections, v.selections);
    }

    #[test]
    fn a_hand_edited_session_cannot_break_a_window() {
        let dir = temp_dir("orlo-session-broken");
        let p = dir.join("Session.json");
        std::fs::write(
            &p,
            r#"{"windows":[{"layout":{"cols":[0,1],"rows":[0,1],"cells":[[0,0,2,1]]},
                "font_size":0,"bounds":[10,10,0,-5]}]}"#,
        )
        .unwrap();
        let w = &load(&p).windows[0];
        assert_eq!(w.layout, Layout::default());
        assert_eq!(w.font_size, Some(6.));
        let [_, _, width, height] = w.bounds.unwrap();
        assert!(width >= 200. && height >= 200.);
    }

    #[test]
    fn unknown_and_missing_keys_use_defaults() {
        let dir = temp_dir("orlo-session-defaults");
        let p = dir.join("Session.json");
        std::fs::write(&p, r#"{"windows":[{"folder":"/x","extra":1}]}"#).unwrap();
        let s = load(&p);
        assert_eq!(
            s.windows,
            vec![WindowState {
                folder: Some("/x".into()),
                // a session from before a window had several folders: its folder is the only root
                folders: vec![Root::new("/x".into())],
                ..Default::default()
            }]
        );
        let w = &s.windows[0];
        assert!(w.sidebar_visible && w.show_tabs && w.show_status_bar);
        assert_eq!(w.layout, Layout::default());
    }
}

#[cfg(test)]
mod encoding_recovery_tests {
    use super::*;
    use orlo_editor::text::Loaded;
    #[test]
    fn named_legacy_hints_and_conversion_survive_serialized_recovery() {
        use orlo_project::encoding::Encoding;
        for encoding in
            [Encoding::Windows1252, Encoding::Iso8859_1, Encoding::ShiftJis, Encoding::Utf32Le, Encoding::Utf32Be]
        {
            let mut state = EditorState::from_text("ascii\n");
            state.doc.encoding = encoding;
            state.doc.disk_encoding = encoding;
            state.doc.line_ending = "\r\n";
            let view = ViewState::capture(PathBuf::from("/legacy"), &state, (0, 0.));
            let encoded = serde_json::to_vec(&view).unwrap();
            let restored: ViewState = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(restored.disk_encoding_hint, Some(encoding));
            state.set_encoding(Encoding::Utf8, true);
            let mut unsaved = Unsaved::capture(&state.doc, "draft".into());
            unsaved.text = state.text();
            let encoded = serde_json::to_vec(&unsaved).unwrap();
            let recovery: Unsaved = serde_json::from_slice(&encoded).unwrap();
            let recovered = recovery.state(Path::new("/legacy")).unwrap();
            assert_eq!(recovered.doc.disk_encoding, encoding);
            assert_eq!(recovered.doc.encoding, Encoding::Utf8);
            assert!(recovered.doc.bom && recovered.unsaved());
            assert_eq!(recovered.doc.line_ending, "\r\n");
        }
        let old: ViewState = serde_json::from_str(r#"{"path":"/old","encoding":"Utf16Le"}"#).unwrap();
        assert!(old.disk_encoding_hint.is_none());
        assert_eq!(old.encoding, Some(Encoding::Utf16Le));
    }

    #[test]
    fn bomless_encoding_survives_recovery_and_shared_views() {
        use orlo_project::encoding::Encoding;
        let p = std::env::temp_dir().join(format!("orlo-encoding-bomless-{}", std::process::id()));
        let bytes = orlo_project::encoding::encode("a🦀\n", Encoding::Utf16Le, false).unwrap();
        std::fs::write(&p, bytes).unwrap();
        let loaded = Loaded::read_encoding(&p, Some(Encoding::Utf16Le)).unwrap();
        let mut state = EditorState::from_loaded(&p, loaded);
        // another view of the same document reads the same conversion
        let sibling = state.view.duplicate(&state.doc);
        state.set_encoding(Encoding::Utf16Be, true);
        let other = Editing { doc: &*state.doc, view: &sibling };
        assert_eq!((other.doc.encoding, other.doc.disk_encoding), (Encoding::Utf16Be, Encoding::Utf16Le));
        state.insert("x");
        state.undo();
        assert!(state.unsaved(), "format conversion survives undoing text");
        let mut recovery = Unsaved::capture(&state.doc, "unused".into());
        recovery.text = state.text();
        let restored = recovery.state(&p).unwrap();
        assert_eq!(
            (restored.doc.encoding, restored.doc.disk_encoding, restored.doc.bom),
            (Encoding::Utf16Be, Encoding::Utf16Le, true)
        );
        std::fs::remove_file(p).unwrap();
    }
}
