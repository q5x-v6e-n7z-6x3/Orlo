//! Compare Files and Compare Folders: a panel whose sides are read-only snapshots in ordinary
//! editors, kept only while it is open; Apply copies a difference from one side to the other.

mod copy;

use super::*;
use crate::editor::ComparisonHighlights;
use crate::files::comparison::folders::{self, Inventory};
use crate::files::comparison::{self, DiskSnapshot, Format, Pair, Text};
use gpui::{uniform_list, UniformListScrollHandle};
use ropey::Rope;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

actions!(
    workspace,
    [
        CompareFiles,
        CompareFolders,
        NextFolderRow,
        PreviousFolderRow,
        OpenFolderRow,
        FocusComparisonNext,
        ApplyLeftToRight,
        ApplyRightToLeft,
        RefreshComparison,
        NextDifference,
        PreviousDifference
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", PreviousFolderRow, Some("ComparisonRows")),
        KeyBinding::new("down", NextFolderRow, Some("ComparisonRows")),
        KeyBinding::new("enter", OpenFolderRow, Some("ComparisonRows")),
        KeyBinding::new("tab", FocusComparisonNext, Some("ComparisonRows")),
        KeyBinding::new("tab", FocusComparisonNext, Some("ComparisonPanel > Editor")),
        KeyBinding::new("escape", ClosePanel, Some("ComparisonPanel")),
    ]
}

#[cfg(test)]
thread_local! {
    static FOLDER_ROW_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    picker_generation: u64,
    stop: Option<Arc<AtomicBool>>,
    validation: Option<Arc<AtomicBool>>,
    apply: Option<Arc<AtomicBool>>,
    copy: Option<Arc<AtomicBool>>,
    folder_stop: Option<Arc<AtomicBool>>,
    filter_stop: Option<Arc<AtomicBool>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Pick {
    Cancelled,
    Expired,
    Right(PathBuf),
    Pair([PathBuf; 2]),
}

impl State {
    fn cancel(&mut self) {
        self.cancel_work();
        self.begin_pick();
    }

    fn cancel_work(&mut self) {
        for slot in [
            &mut self.folder_stop,
            &mut self.filter_stop,
            &mut self.copy,
            &mut self.apply,
            &mut self.stop,
            &mut self.validation,
        ] {
            cancel(slot);
        }
        self.generation += 1;
    }

    fn begin_pick(&mut self) -> u64 {
        self.picker_generation += 1;
        self.picker_generation
    }

    fn pick(&self, generation: u64, left: Option<PathBuf>, chosen: Option<PathBuf>) -> Pick {
        if self.picker_generation != generation {
            return Pick::Expired;
        }
        match (left, chosen) {
            (_, None) => Pick::Cancelled,
            (None, Some(left)) => Pick::Right(left),
            (Some(left), Some(right)) => Pick::Pair([left, right]),
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Stops the work in `slot`, if any, and forgets it.
fn cancel(slot: &mut Option<Arc<AtomicBool>>) -> bool {
    slot.take().inspect(|stop| stop.store(true, Ordering::Relaxed)).is_some()
}

/// Stops the work in `slot` and installs a fresh stop flag for its replacement.
fn renew(slot: &mut Option<Arc<AtomicBool>>) -> Arc<AtomicBool> {
    cancel(slot);
    slot.insert(Arc::new(AtomicBool::new(false))).clone()
}

/// Whether `stop` still belongs to the work in `slot`.
fn active(slot: &Option<Arc<AtomicBool>>, stop: &Arc<AtomicBool>) -> bool {
    slot.as_ref().is_some_and(|active| Arc::ptr_eq(active, stop))
}

fn ownership(buffers: &[Buffer]) -> Vec<(u64, PathBuf)> {
    buffers.iter().map(|b| (b.owner.buffer, b.owner.path.clone())).collect()
}

#[derive(Clone, PartialEq, Eq)]
struct Owner {
    path: PathBuf,
    buffer: u64,
    version: u64,
    format: Format,
    disk_encoding: crate::editing::encoding::Encoding,
    dirty: bool,
    read_only: bool,
    deleted: bool,
    disk: Option<crate::files::watch::Stamp>,
    watching: bool,
}

struct Buffer {
    owner: Owner,
    endings: Option<crate::editing::text::LineEndings>,
    rope: Rope,
}

struct Source {
    path: PathBuf,
    disk: DiskSnapshot,
    live: Option<Buffer>,
    label: String,
}

impl Source {
    fn text(&self) -> Text {
        let (rope, format) = match &self.live {
            Some(b) => (&b.rope, b.owner.format),
            None => (&self.disk.text.rope, self.disk.text.format),
        };
        let endings = match &self.live {
            Some(b) => &b.endings,
            None => &self.disk.text.endings,
        };
        Text { endings: endings.clone(), rope: rope.clone(), format }
    }
}

struct Prepared {
    sources: [Source; 2],
    pair: Pair,
    highlights: [ComparisonHighlights; 2],
}

struct Review {
    sources: Arc<[Source; 2]>,
    pair: Pair,
    viewers: [Entity<Editor>; 2],
    /// Keeps the viewers level, scrolling as one and folding alike while the review lasts.
    _aligned: Entity<crate::editor::AlignedPair>,
}

struct ClosedDestination {
    path: PathBuf,
    loaded: crate::editing::text::Loaded,
    indentation: Option<crate::config::settings::Indentation>,
    policy: crate::config::editorconfig::Properties,
    warnings: Vec<String>,
}

struct Application {
    range: std::ops::Range<usize>,
    replacement: String,
    closed: Option<ClosedDestination>,
}

#[derive(PartialEq, Eq)]
struct Environment {
    busy: Vec<PathBuf>,
    views: Vec<(EntityId, gpui::AnyWindowHandle, u64)>,
}

fn matches_snapshot(path: &Path, source: &Path, disk: &DiskSnapshot) -> Result<bool, String> {
    if path == source {
        return Ok(true);
    }
    match std::fs::metadata(path) {
        Ok(m) => Ok(m.dev() == disk.device && m.ino() == disk.stamp.ino),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("Cannot validate {}: {e}", path.display())),
    }
}

/// Off the UI thread: the files and their aliases must still be the snapshots compared, and
/// only the hunk being applied is built.
fn prepare_apply(
    sources: &[Source; 2],
    buffers: Vec<Buffer>,
    busy: Vec<PathBuf>,
    source: usize,
    ranges: [std::ops::Range<usize>; 2],
    stop: &AtomicBool,
) -> Result<Application, String> {
    for side in sources {
        stopped(stop)?;
        let current = comparison::read_with_hint(&side.path, Some(side.disk.text.format.encoding), stop)
            .map_err(|e| e.to_string())?;
        if current.device != side.disk.device || current.stamp != side.disk.stamp || current.bytes != side.disk.bytes {
            return Err("A reviewed file changed on disk".into());
        }
        let mut owners = Vec::new();
        for buffer in &buffers {
            stopped(stop)?;
            if matches_snapshot(&buffer.owner.path, &side.path, &side.disk)? {
                owners.push(&buffer.owner);
            }
        }
        if owners != side.live.iter().map(|b| &b.owner).collect::<Vec<_>>() {
            return Err("A reviewed buffer changed or has ambiguous owners".into());
        }
        if side.live.as_ref().is_some_and(|b| b.owner.deleted) {
            return Err("A reviewed buffer is deleted".into());
        }
        let resolved = std::fs::canonicalize(&side.path).map_err(|e| e.to_string())?;
        for path in &busy {
            stopped(stop)?;
            let canonical = match std::fs::canonicalize(path) {
                Ok(path) => path,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.clone(),
                Err(e) => return Err(format!("Cannot validate {}: {e}", path.display())),
            };
            if side.path.starts_with(path)
                || path.starts_with(&side.path)
                || resolved.starts_with(&canonical)
                || canonical.starts_with(&resolved)
                || matches_snapshot(path, &side.path, &side.disk)?
            {
                return Err("A reviewed file is loading, restoring or reserved for writing".into());
            }
        }
    }
    let target = &sources[1 - source];
    if target.live.as_ref().is_some_and(|b| b.owner.read_only)
        || std::fs::metadata(&target.path).map_err(|e| e.to_string())?.permissions().readonly()
    {
        return Err("The destination is read-only".into());
    }
    stopped(stop)?;
    let replacement = sources[source].text().rope.slice(ranges[source].clone()).to_string();
    stopped(stop)?;
    let closed = if target.live.is_none() {
        let path = std::fs::canonicalize(&target.path).map_err(|e| e.to_string())?;
        let (policy, warnings) = crate::config::editorconfig::load(Some(&path));
        Some(ClosedDestination {
            path,
            loaded: crate::editing::text::Loaded {
                endings: target.disk.text.endings.clone(),
                rope: target.disk.text.rope.clone(),
                line_ending: target.disk.text.format.line_ending,
                bom: target.disk.text.format.bom,
                encoding: target.disk.text.format.encoding,
                read_only: false,
                disk: Some(target.disk.stamp),
            },
            indentation: super::preferences::infer(&target.disk.text.rope),
            policy,
            warnings,
        })
    } else {
        None
    };
    stopped(stop)?;
    Ok(Application { range: ranges[1 - source].clone(), replacement, closed })
}

struct FolderReview {
    roots: [PathBuf; 2],
    inventory: Option<Arc<Inventory>>,
    summary: String,
    copy_message: Option<String>,
    inputs: [Entity<Editor>; 2],
    statuses: [bool; 7],
    visible: Arc<Vec<usize>>,
    filtering: bool,
    filter_error: Option<String>,
    selected: Option<usize>,
    cursor: usize,
    list_focus: FocusHandle,
    progress: Arc<AtomicUsize>,
    scroll: UniformListScrollHandle,
}

pub struct ComparisonPanel {
    folder: Option<Box<FolderReview>>,
    paths: [PathBuf; 2],
    review: Option<Box<Review>>,
    current: usize,
    message: Option<String>,
    pub(crate) focus: FocusHandle,
}

impl ComparisonPanel {
    fn copy_candidate(&self) -> Option<(usize, usize)> {
        if !self.visible_pair() {
            return None;
        }
        let folder = self.folder.as_ref()?;
        let index = folder.selected?;
        let row = folder.inventory.as_ref()?.rows.get(index)?;
        if row.directory || !row.note.is_empty() {
            return None;
        }
        match row.status {
            folders::Status::LeftOnly => Some((0, index)),
            folders::Status::RightOnly => Some((1, index)),
            _ => None,
        }
    }

    fn visible_pair(&self) -> bool {
        self.folder.as_ref().is_none_or(|f| {
            !f.filtering && f.filter_error.is_none() && f.selected.is_some_and(|i| f.visible.binary_search(&i).is_ok())
        })
    }

    fn counter(&self) -> String {
        let n = self.review.as_ref().map_or(0, |r| r.pair.hunks.len());
        if n == 0 {
            if self.review.as_ref().is_some_and(|r| r.pair.formats_differ) {
                "No text differences — formats differ".into()
            } else {
                "No text or format differences".into()
            }
        } else {
            format!("Difference {} of {n}", self.current + 1)
        }
    }

    fn range_label(&self, side: usize) -> String {
        let Some(review) = &self.review else {
            return String::new();
        };
        let Some(h) = review.pair.hunks.get(self.current) else {
            return String::new();
        };
        let (lines, chars) =
            if side == 0 { (&h.lines.old_lines, &h.lines.old_chars) } else { (&h.lines.new_lines, &h.lines.new_chars) };
        if chars.is_empty() {
            let eof = chars.start == review.sources[side].text().rope.len_chars();
            format!("Empty range {}line {}", if eof { "at EOF, " } else { "before " }, lines.start + 1)
        } else {
            format!("Lines {}–{}", lines.start + 1, lines.end.max(lines.start + 1))
        }
    }

    pub(crate) fn viewers(&self) -> Vec<&Entity<Editor>> {
        let viewers = self.review.iter().flat_map(|r| &r.viewers);
        viewers.chain(self.folder.iter().flat_map(|f| &f.inputs)).collect()
    }
}

/// The next or previous of `count` places after `at`, wrapping round.
fn cycle(at: usize, count: usize, next: bool) -> usize {
    if next {
        (at + 1) % count
    } else {
        (at + count - 1) % count
    }
}

/// A side's caption, found in tests by its text.
fn side_label(id: &'static str, side: usize, label: String) -> gpui::Stateful<gpui::Div> {
    div().id((id, side)).px(px(8.)).text_size(px(11.)).child(label.clone()).debug_selector(move || label)
}

fn stopped(stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::Relaxed) {
        Err("Comparison cancelled".into())
    } else {
        Ok(())
    }
}

fn picker_options(folders: bool, left: bool) -> PathPromptOptions {
    let kind = if folders { "Folder" } else { "File" };
    let side = if left { "Left" } else { "Right" };
    PathPromptOptions {
        files: !folders,
        directories: folders,
        multiple: false,
        prompt: Some(format!("Compare {kind}s — Choose {side} {kind}").into()),
    }
}

fn validate_paths(paths: &[PathBuf; 2], stop: &AtomicBool, folders: bool) -> Result<(), String> {
    let mut identities = Vec::new();
    for path in paths {
        stopped(stop)?;
        let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if folders && !metadata.is_dir() {
            return Err(format!("{}: Choose an existing real directory", path.display()));
        }
        let unsupported = if metadata.is_symlink() {
            Some(comparison::Unsupported::Symlink)
        } else if !folders && !metadata.is_file() {
            Some(comparison::Unsupported::NotRegular)
        } else {
            None
        };
        if let Some(reason) = unsupported {
            return Err(format!("{}: {}", path.display(), comparison::Error::Unsupported(reason)));
        }
        identities.push((metadata.dev(), metadata.ino()));
    }
    if identities[0] == identities[1] {
        return Err(if folders {
            "Choose two different underlying folders"
        } else {
            "Choose two different underlying files"
        }
        .into());
    }
    stopped(stop)
}

/// Whether `buffer` is open at `path` or at another name of the file `device`/`inode`.
fn owns(buffer: &Buffer, path: &Path, device: u64, inode: u64) -> bool {
    buffer.owner.path == path
        || std::fs::metadata(&buffer.owner.path).is_ok_and(|m| m.dev() == device && m.ino() == inode)
}

fn disk_hint(
    path: &Path,
    buffers: &[Buffer],
    stop: &AtomicBool,
) -> Result<Option<(crate::editing::encoding::Encoding, u64, u64)>, String> {
    if buffers.is_empty() {
        return Ok(None);
    }
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let (device, inode) = (metadata.dev(), metadata.ino());
    for buffer in buffers {
        stopped(stop)?;
        if owns(buffer, path, device, inode) {
            return Ok(Some((buffer.owner.disk_encoding, device, inode)));
        }
    }
    Ok(None)
}

/// All identity lookups, reads, text scans and range preparation run on the worker.
fn prepare(
    paths: [PathBuf; 2],
    buffers: Vec<Buffer>,
    hints: HashMap<PathBuf, crate::editing::encoding::Encoding>,
    stop: &AtomicBool,
) -> Result<Prepared, String> {
    stopped(stop)?;
    let mut sources = Vec::new();
    for path in paths {
        stopped(stop)?;
        let hint = disk_hint(&path, &buffers, stop)?;
        let disk = comparison::read_with_hint(&path, hint.map(|h| h.0).or_else(|| hints.get(&path).copied()), stop)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if hint.is_some_and(|(_, device, inode)| device != disk.device || inode != disk.stamp.ino) {
            return Err("A file changed while matching its open buffer; Refresh".into());
        }
        let mut live: Option<Buffer> = None;
        for b in &buffers {
            stopped(stop)?;
            if !owns(b, &path, disk.device, disk.stamp.ino) {
                continue;
            }
            if let Some(previous) = &live {
                if previous.owner.buffer != b.owner.buffer {
                    return Err(format!("{}: ambiguous open buffers; reconcile them and Refresh", path.display()));
                }
                continue;
            }
            if b.rope.len_bytes() > 100 << 20 {
                return Err(format!("{}: buffer exceeds the 100 MiB comparison limit", path.display()));
            }
            for chunk in b.rope.chunks() {
                stopped(stop)?;
                if chunk.contains('\0') {
                    return Err(format!("{}: NUL-binary buffer", path.display()));
                }
            }
            live = Some(Buffer { owner: b.owner.clone(), endings: b.endings.clone(), rope: b.rope.clone() });
        }
        let text = live.as_ref().map_or(&disk.text.rope, |b| &b.rope);
        let format = live.as_ref().map_or(disk.text.format, |b| b.owner.format);
        let origin = match &live {
            Some(b) if b.owner.dirty => "Live buffer · unsaved",
            Some(_) => "Live buffer",
            None => "Disk",
        };
        let ending = match format.line_ending {
            "\r\n" => "CRLF",
            "\r" => "CR",
            _ => "LF",
        };
        let label = format!(
            "{origin} · {} · {ending} · {} · {} bytes{}",
            format.encoding.name(),
            if format.bom { "BOM" } else { "no BOM" },
            if live.is_some() { text.len_bytes() } else { disk.bytes.len() },
            if live.as_ref().is_some_and(|b| b.owner.deleted) { " · deleted on disk" } else { "" }
        );
        sources.push(Source { path, disk, live, label });
    }
    let [left, right]: [Source; 2] = sources.try_into().ok().unwrap();
    if left.disk.device == right.disk.device && left.disk.stamp.ino == right.disk.stamp.ino {
        return Err("Choose two different underlying files".into());
    }
    let pair = comparison::compare(&left.text(), &right.text(), stop).map_err(|e| e.to_string())?;
    let highlights = crate::editor::highlights_of(&pair, stop).ok_or_else(|| "Comparison cancelled".to_string())?;
    stopped(stop)?;
    Ok(Prepared { sources: [left, right], pair, highlights })
}

impl Workspace {
    pub(crate) fn cancel_comparison(&mut self) {
        self.comparison.cancel();
    }

    fn comparison_buffers(&self, cx: &App) -> Vec<Buffer> {
        let mut seen = HashSet::new();
        let mut buffers: Vec<_> = self
            .open_editors(cx)
            .into_iter()
            .filter_map(|e| {
                let s = e.read(cx).read_state(cx);
                if s.doc.scratch || !seen.insert(s.buffer_id()) {
                    return None;
                }
                Some(Buffer {
                    endings: s.doc.endings.clone(),
                    owner: Owner {
                        path: s.doc.path.clone()?,
                        buffer: s.buffer_id(),
                        version: s.doc.version,
                        format: Format { line_ending: s.doc.line_ending, bom: s.doc.bom, encoding: s.doc.encoding },
                        disk_encoding: s.doc.disk_encoding,
                        dirty: s.doc.dirty,
                        read_only: s.doc.read_only,
                        deleted: s.doc.deleted,
                        disk: s.doc.disk,
                        watching: s.doc.watching,
                    },
                    rope: s.doc.rope.clone(),
                })
            })
            .collect();
        buffers.sort_by_key(|b| b.owner.buffer);
        buffers
    }

    fn comparison_environment(&self, cx: &App) -> Environment {
        self.comparison_environment_with_writes(pipelines::write_paths(cx), cx)
    }

    /// The handles of the app's other windows' workspaces, for updating them.
    pub(super) fn other_workspace_handles(&self, cx: &App) -> impl Iterator<Item = WindowHandle<Workspace>> {
        let this = self.window;
        cx.windows().into_iter().filter(move |w| *w != this).filter_map(|w| w.downcast::<Workspace>())
    }

    fn comparison_environment_with_writes(&self, mut busy: Vec<PathBuf>, cx: &App) -> Environment {
        let mut views = Vec::new();
        let mut capture = |ws: &Workspace| {
            busy.extend(ws.loading.iter().cloned());
            busy.extend(ws.restoring.iter().map(|v| v.view.path.clone()));
            views.extend(ws.views.tabs().map(|e| (e.entity_id(), ws.window, ws.restore_gen)));
        };
        capture(self);
        for window in self.other_workspace_handles(cx) {
            if let Ok(ws) = window.read(cx) {
                capture(ws);
            }
        }
        busy.sort();
        busy.dedup();
        views.sort_by_key(|view| view.0);
        Environment { busy, views }
    }

    fn apply_comparison(&mut self, source: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Panel::Comparison(panel)) = &mut self.panel else {
            return;
        };
        if panel.message.is_some() || self.comparison.apply.is_some() || !panel.visible_pair() {
            return;
        }
        let Some(review) = &panel.review else { return };
        let Some(hunk) = review.pair.hunks.get(panel.current) else {
            return;
        };
        let sources = review.sources.clone();
        let ranges = [hunk.lines.old_chars.clone(), hunk.lines.new_chars.clone()];
        let current = panel.current;
        panel.message = Some("Validating Apply…".into());
        let generation = self.comparison.generation;
        let project = self.restore_gen;
        let stop = renew(&mut self.comparison.apply);
        let buffers = self.comparison_buffers(cx);
        let owned = ownership(&buffers);
        let environment = self.comparison_environment(cx);
        let busy = environment.busy.clone();
        let input = sources.clone();
        let worker_stop = stop.clone();
        let task =
            cx.background_spawn(async move { prepare_apply(&input, buffers, busy, source, ranges, &worker_stop) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, window, cx| {
                let same_review = matches!(&ws.panel, Some(Panel::Comparison(p))
                    if p.current == current && p.review.as_ref()
                        .is_some_and(|r| Arc::ptr_eq(&r.sources, &sources)));
                if ws.comparison.generation != generation
                    || ws.restore_gen != project
                    || stop.load(Ordering::Relaxed)
                    || !same_review
                    || !active(&ws.comparison.apply, &stop)
                {
                    return;
                }
                ws.comparison.apply = None;
                let buffers = ws.comparison_buffers(cx);
                // any change in which buffers own the files, or what is loading or leased, makes
                // Apply stale; a narrower test would need another alias check
                let intact = ownership(&buffers) == owned
                    && ws.comparison_environment(cx) == environment
                    && sources
                        .iter()
                        .all(|s| s.live.as_ref().is_none_or(|b| buffers.iter().any(|now| now.owner == b.owner)));
                let result = if intact {
                    result
                } else {
                    Err("Open buffers, loading or write reservations changed while validating".into())
                };
                let application = match result {
                    Ok(application) => application,
                    Err(error) => {
                        let message = format!("Apply rejected: {error}; Refresh to review current sources");
                        if let Some(Panel::Comparison(p)) = &mut ws.panel {
                            p.message = Some(message.clone());
                        }
                        ws.status_message(message, cx);
                        cx.notify();
                        return;
                    }
                };
                let target = &sources[1 - source];
                let editor = if let Some(closed) = application.closed {
                    let state = EditorState::from_loaded(&closed.path, closed.loaded);
                    let editor = cx.new(|cx| {
                        let mut e = Editor::new(state, false, cx);
                        e.view = ws.view.clone();
                        e.update_doc(cx, |d| d.inferred_indentation = closed.indentation);
                        ws.apply_editor_properties(&mut e, &closed.policy, cx);
                        e.preference_warning = (!closed.warnings.is_empty()).then(|| closed.warnings.join("\n"));
                        e
                    });
                    ws.attach(editor.clone(), false, window, cx);
                    editor
                } else {
                    let buffer = target.live.as_ref().unwrap().owner.buffer;
                    let Some(editor) = ws.open_editors(cx).into_iter().find(|e| e.read(cx).doc(cx).id() == buffer)
                    else {
                        return;
                    };
                    editor
                };
                editor.update(cx, |e, cx| {
                    e.edit(cx, |s| {
                        s.replace_ranges(vec![(application.range, application.replacement)]);
                    });
                });
                if let Some(Panel::Comparison(p)) = &mut ws.panel {
                    p.message = Some("Applied to buffer — not saved; Refresh before another Apply".into());
                }
                ws.status_message("Applied to buffer — not saved", cx);
                if let Some((g, i)) = ws.views.find(&editor) {
                    ws.select_view(g, i, window, cx);
                } else {
                    for handle in ws.other_workspace_handles(cx) {
                        if handle.read(cx).is_ok_and(|other| other.views.find(&editor).is_some()) {
                            handle
                                .update(cx, |other, window, cx| {
                                    other.show_view(&editor, window, cx);
                                    window.activate_window();
                                    other.status_message("Applied to buffer — not saved", cx);
                                    cx.notify();
                                })
                                .ok();
                            break;
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    forward_actions! {
    pub(crate) apply_left_to_right(ApplyLeftToRight, window, cx) => apply_comparison(0, window, cx);
    pub(crate) apply_right_to_left(ApplyRightToLeft, window, cx) => apply_comparison(1, window, cx);
    pub(crate) compare_files(CompareFiles, window, cx) => choose_comparison(false, window, cx);
    pub(crate) compare_folders(CompareFolders, window, cx) => choose_comparison(true, window, cx);
    pub(crate) next_folder_row(NextFolderRow, _, cx) => move_folder_cursor(true, cx);
    pub(crate) previous_folder_row(PreviousFolderRow, _, cx) => move_folder_cursor(false, cx);
    pub(crate) next_difference(NextDifference, _, cx) => navigate_difference(true, cx);
    pub(crate) previous_difference(PreviousDifference, _, cx) => navigate_difference(false, cx);
    }

    fn choose_comparison(&mut self, folders: bool, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.comparison.begin_pick();
        let rx = cx.prompt_for_paths(picker_options(folders, true));
        cx.spawn_in(window, async move |this, cx| {
            let mut receiver = rx;
            let mut left = None;
            loop {
                let Ok(Ok(paths)) = receiver.await else {
                    return;
                };
                let chosen = paths.and_then(|p| p.into_iter().next());
                let Ok(step) = this.update_in(cx, |ws, _, _| ws.comparison.pick(generation, left.take(), chosen))
                else {
                    return;
                };
                match step {
                    Pick::Cancelled | Pick::Expired => return,
                    Pick::Right(path) => {
                        left = Some(path);
                        let Ok(rx) = this.update_in(cx, |_, _, cx| cx.prompt_for_paths(picker_options(folders, false)))
                        else {
                            return;
                        };
                        receiver = rx;
                    }
                    Pick::Pair(paths) => {
                        this.update_in(cx, |ws, w, cx| {
                            if ws.comparison.picker_generation == generation {
                                ws.start_comparison(paths, folders, w, cx);
                            }
                        })
                        .ok();
                        return;
                    }
                }
            }
        })
        .detach();
    }

    fn start_comparison(&mut self, paths: [PathBuf; 2], folders: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.comparison.begin_pick();
        let project = self.restore_gen;
        let stop = renew(&mut self.comparison.validation);
        self.status_message("Loading comparison…", cx);
        let input = paths.clone();
        let worker_stop = stop.clone();
        let validation = cx.background_spawn(async move { validate_paths(&input, &worker_stop, folders) });
        cx.spawn_in(window, async move |this, cx| {
            let result = validation.await;
            this.update_in(cx, |ws, window, cx| {
                if ws.restore_gen != project || !active(&ws.comparison.validation, &stop) {
                    return;
                }
                ws.comparison.validation = None;
                match result {
                    Ok(()) if folders => ws.load_folder_comparison(paths, None, window, cx),
                    Ok(()) => ws.load_comparison(paths, window, cx),
                    Err(error) => ws.status_message(format!("Comparison: {error}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn comparison_error(&mut self, paths: &[PathBuf; 2], error: String, cx: &mut Context<Self>) {
        if let Some(Panel::Comparison(p)) = &mut self.panel {
            if p.paths == *paths {
                p.message = Some(error.clone());
            }
        }
        self.status_message(format!("Comparison: {error}"), cx);
        cx.notify();
    }

    fn load_comparison(&mut self, paths: [PathBuf; 2], window: &mut Window, cx: &mut Context<Self>) {
        self.comparison.cancel_work();
        let focus = cx.focus_handle();
        self.remember_find(cx);
        self.panel = Some(Panel::Comparison(ComparisonPanel {
            paths: paths.clone(),
            folder: None,
            review: None,
            current: 0,
            message: Some("Loading comparison…".into()),
            focus: focus.clone(),
        }));
        self.reveal_panel(cx);
        window.focus(&focus);
        let generation = self.comparison.generation;
        let project = self.restore_gen;
        let stop = renew(&mut self.comparison.stop);
        let buffers = self.comparison_buffers(cx);
        let owners: Vec<_> = buffers.iter().map(|b| b.owner.clone()).collect();
        let input = paths.clone();
        let hints = self.closed.iter().filter_map(|v| Some((v.path.clone(), v.disk_encoding_hint?))).collect();
        let task = cx.background_spawn(async move { prepare(input, buffers, hints, &stop) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, window, cx| {
                if ws.comparison.generation != generation || ws.restore_gen != project {
                    return;
                }
                ws.comparison.stop = None;
                let current: Vec<_> = ws.comparison_buffers(cx).into_iter().map(|b| b.owner).collect();
                // any change of owner makes the result stale: even a new alias needs the worker's
                // identity checks
                let result = if owners != current {
                    Err("Open buffers changed while comparing; Refresh to review current sources".into())
                } else {
                    result
                };
                match result {
                    Ok(Prepared { sources, pair, highlights }) => {
                        if sources.iter().any(|source| pipelines::reading_blocked(&source.path, cx)) {
                            return ws.comparison_error(
                                &paths,
                                "Files are being written; refresh comparison".into(),
                                cx,
                            );
                        }
                        let mut highlights = highlights.into_iter();
                        let viewers = std::array::from_fn(|side| {
                            let text = sources[side].text();
                            let marks = highlights.next().unwrap();
                            let view = ws.view.clone();
                            cx.new(|cx| {
                                let mut state = EditorState::new();
                                state.doc.rope = text.rope;
                                state.doc.endings = text.endings;
                                state.doc.path = Some(sources[side].path.clone());
                                state.doc.read_only = true;
                                state.doc.scratch = true;
                                state.doc.line_ending = text.format.line_ending;
                                state.doc.bom = text.format.bom;
                                state.doc.encoding = text.format.encoding;
                                state.doc.disk_encoding = sources[side].disk.text.format.encoding;
                                let mut editor = Editor::new(state, false, cx);
                                editor.input_enabled = false;
                                editor.view = view;
                                editor.view.minimap = false;
                                editor.comparison = Some(marks);
                                editor
                            })
                        });
                        let hunks = crate::editor::hunks_of(&pair);
                        let sides = viewers.clone();
                        let aligned = cx.new(|cx| crate::editor::AlignedPair::new(sides, hunks, cx));
                        let Some(Panel::Comparison(p)) = &mut ws.panel else {
                            return;
                        };
                        let take_focus = p.focus.contains_focused(window, cx)
                            && p.folder
                                .as_ref()
                                .is_none_or(|f| !f.inputs.iter().any(|e| e.read(cx).is_focused(window)));
                        p.review =
                            Some(Box::new(Review { sources: Arc::new(sources), pair, viewers, _aligned: aligned }));
                        p.message = None;
                        if take_focus {
                            p.review.as_ref().unwrap().viewers[0].read(cx).focus(window);
                        }
                        ws.jump_difference(cx);
                        ws.status_message("Comparison ready", cx);
                    }
                    Err(error) => ws.comparison_error(&paths, error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_folder_comparison(
        &mut self,
        roots: [PathBuf; 2],
        retained: Option<Box<FolderReview>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.comparison.cancel_work();
        let mut folder = retained.unwrap_or_else(|| {
            let inputs = std::array::from_fn(|_| cx.new(|cx| Editor::new(EditorState::new(), true, cx)));
            for input in &inputs {
                let field = input.clone();
                Editor::on_text_change(input, cx, move |ws, cx| {
                    let current = matches!(&ws.panel,Some(Panel::Comparison(p))
                        if p.folder.as_ref().is_some_and(|f| f.inputs.contains(&field)));
                    if current {
                        ws.filter_folder_rows(cx);
                    }
                })
                .detach();
            }
            Box::new(FolderReview {
                roots: roots.clone(),
                inventory: None,
                summary: String::new(),
                copy_message: None,
                inputs,
                statuses: folders::DEFAULT_STATUSES,
                visible: Arc::new(Vec::new()),
                filtering: false,
                filter_error: None,
                selected: None,
                cursor: 0,
                list_focus: cx.focus_handle(),
                progress: Arc::new(AtomicUsize::new(0)),
                scroll: UniformListScrollHandle::new(),
            })
        });
        folder.inventory = None;
        folder.visible = Arc::new(Vec::new());
        folder.summary = "Scanning folders…".into();
        folder.filter_error = None;
        folder.selected = None;
        folder.cursor = 0;
        folder.filtering = false;
        folder.progress = Arc::new(AtomicUsize::new(0));
        let progress = folder.progress.clone();
        let list_focus = folder.list_focus.clone();
        let focus = cx.focus_handle();
        self.remember_find(cx);
        self.panel = Some(Panel::Comparison(ComparisonPanel {
            paths: roots.clone(),
            folder: Some(folder),
            review: None,
            current: 0,
            message: Some("Scanning folders…".into()),
            focus: focus.clone(),
        }));
        self.reveal_panel(cx);
        window.focus(&list_focus);
        let generation = self.comparison.generation;
        let project = self.restore_gen;
        let stop = renew(&mut self.comparison.folder_stop);
        let mut hints = self
            .closed
            .iter()
            .filter_map(|v| Some((v.path.clone(), v.disk_encoding_hint?)))
            .collect::<std::collections::BTreeMap<_, _>>();
        hints.extend(self.open_editors(cx).into_iter().filter_map(|e| {
            let d = e.read(cx).doc(cx);
            Some((d.path.clone()?, d.disk_encoding))
        }));
        let input = roots.clone();
        let worker_stop = stop.clone();
        let task = cx.background_spawn(async move {
            folders::scan_with_hints(&input, &hints, &worker_stop, &progress)
                .map(|inventory| {
                    let summary = inventory.summary();
                    (Arc::new(inventory), summary)
                })
                .map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, _, cx| {
                if ws.comparison.generation != generation
                    || ws.restore_gen != project
                    || !active(&ws.comparison.folder_stop, &stop)
                {
                    return;
                }
                ws.comparison.folder_stop = None;
                let Some(Panel::Comparison(p)) = &mut ws.panel else {
                    return;
                };
                let Some(folder) = &mut p.folder else {
                    return;
                };
                if folder.roots != roots {
                    return;
                }
                match result {
                    Ok((inventory, summary)) => {
                        folder.inventory = Some(inventory);
                        folder.summary = summary;
                        p.message = None;
                        ws.filter_folder_rows(cx);
                    }
                    Err(error) => {
                        p.message = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
            let done = this
                .update(cx, |ws, cx| {
                    if ws.comparison.generation != generation
                        || ws.restore_gen != project
                        || ws.comparison.folder_stop.is_none()
                    {
                        return true;
                    }
                    cx.notify();
                    false
                })
                .unwrap_or(true);
            if done {
                break;
            }
        })
        .detach();
    }

    fn filter_folder_rows(&mut self, cx: &mut Context<Self>) {
        let Some(Panel::Comparison(p)) = &mut self.panel else {
            return;
        };
        let Some(folder) = &mut p.folder else {
            return;
        };
        let Some(inventory) = folder.inventory.clone() else {
            return;
        };
        cancel(&mut self.comparison.copy);
        if cancel(&mut self.comparison.apply) {
            p.message = None;
        }
        folder.cursor = 0;
        let patterns = folder.inputs.each_ref().map(|input| input.read(cx).doc(cx).rope.clone());
        let statuses = folder.statuses;
        folder.visible = Arc::new(Vec::new());
        folder.filtering = true;
        folder.filter_error = None;
        folder.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        let generation = self.comparison.generation;
        let project = self.restore_gen;
        let stop = renew(&mut self.comparison.filter_stop);
        let worker_stop = stop.clone();
        let worker_inventory = inventory.clone();
        let task = cx.background_spawn(async move {
            let patterns = patterns.map(|rope| rope.to_string());
            folders::filter(&worker_inventory, [&patterns[0], &patterns[1]], statuses, &worker_stop).map(Arc::new)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |ws, cx| {
                if ws.comparison.generation != generation
                    || ws.restore_gen != project
                    || !active(&ws.comparison.filter_stop, &stop)
                {
                    return;
                }
                ws.comparison.filter_stop = None;
                let Some(folder) = ws.folder_review_mut() else {
                    return;
                };
                if !folder.inventory.as_ref().is_some_and(|i| Arc::ptr_eq(i, &inventory)) {
                    return;
                }
                folder.filtering = false;
                match result {
                    Ok(rows) => folder.visible = rows,
                    Err(error) => folder.filter_error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn select_folder_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Panel::Comparison(p)) = &mut self.panel else {
            return;
        };
        let Some(folder) = &mut p.folder else {
            return;
        };
        let Ok(cursor) = folder.visible.binary_search(&index) else {
            return;
        };
        if folder.filtering {
            return;
        }
        let Some(row) = folder.inventory.as_ref().and_then(|i| i.rows.get(index)) else {
            return;
        };
        folder.selected = Some(index);
        folder.cursor = cursor;
        if !row.text_pair {
            let message = format!("{}: {} · {} — inspection only", row.path.display(), row.status.label(), row.note);
            p.paths = folder.roots.clone();
            p.review = None;
            p.message = Some(message.clone());
            self.comparison.cancel_work();
            self.status_message(message, cx);
            cx.notify();
            return;
        }
        let paths = folder.roots.each_ref().map(|root| root.join(&row.path));
        let folder = p.folder.take();
        self.load_comparison(paths, window, cx);
        if let Some(Panel::Comparison(p)) = &mut self.panel {
            p.folder = folder;
        }
    }

    fn folder_review(&self) -> Option<&FolderReview> {
        match &self.panel {
            Some(Panel::Comparison(p)) => p.folder.as_deref(),
            _ => None,
        }
    }

    fn folder_review_mut(&mut self) -> Option<&mut FolderReview> {
        match &mut self.panel {
            Some(Panel::Comparison(p)) => p.folder.as_deref_mut(),
            _ => None,
        }
    }

    fn move_folder_cursor(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some(folder) = self.folder_review_mut() else {
            return;
        };
        let count = folder.visible.len();
        if count == 0 {
            return;
        }
        folder.cursor = cycle(folder.cursor, count, next);
        folder.scroll.scroll_to_item(folder.cursor, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    pub(crate) fn open_folder_row(&mut self, _: &OpenFolderRow, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(&index) = self.folder_review().and_then(|f| f.visible.get(f.cursor)) {
            self.select_folder_row(index, window, cx);
        }
    }

    pub(crate) fn focus_comparison_next(
        &mut self,
        _: &FocusComparisonNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(folder) = self.folder_review() else {
            return;
        };
        if folder.inputs[0].read(cx).is_focused(window) {
            folder.inputs[1].read(cx).focus(window);
        } else if folder.inputs[1].read(cx).is_focused(window) {
            window.focus(&folder.list_focus);
        } else {
            folder.inputs[0].read(cx).focus(window);
        }
    }

    fn render_folder_rows(&self, folder: &FolderReview, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div().flex().flex_col().min_h_0().flex_1().child(
            div()
                .id("comparison-folder-roots")
                .debug_selector(|| "comparison-folder-roots".into())
                .px(px(8.))
                .text_size(px(11.))
                .child(format!("Left: {} · Right: {}", folder.roots[0].display(), folder.roots[1].display())),
        );
        for (side, label) in ["Include globs (; separated)", "Exclude globs (; separated)"].into_iter().enumerate() {
            let selector = if side == 0 { "comparison-folder-include" } else { "comparison-folder-exclude" };
            body = body.child(
                div().flex().items_center().gap(px(6.)).px(px(8.)).child(div().text_size(px(11.)).child(label)).child(
                    kit::field()
                        .id(selector)
                        .debug_selector(move || selector.into())
                        .flex_1()
                        .h(px(26.))
                        .child(folder.inputs[side].clone()),
                ),
            );
        }
        let mut statuses = div().flex().flex_wrap().gap(px(4.)).px(px(8.));
        for status in folders::Status::ALL {
            let on = folder.statuses[status as usize];
            let label = format!(
                "{} {}",
                if on { "☑" } else { "☐" },
                if status == folders::Status::Equal { "Show Equal" } else { status.label() }
            );
            statuses = statuses.child(
                kit::ghost_button(status.label(), label)
                    .debug_selector(move || format!("comparison-status-{}", status as usize))
                    .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                    .on_click(cx.listener(move |ws, _, _, cx| {
                        if let Some(f) = ws.folder_review_mut() {
                            f.statuses[status as usize] ^= true;
                        }
                        ws.filter_folder_rows(cx);
                    })),
            );
        }
        let summary = if self.comparison.folder_stop.is_some() {
            format!("Scanning folders… {} entries", folder.progress.load(Ordering::Relaxed))
        } else {
            format!(
                "{} · {} shown / {} filtered{}",
                folder.summary,
                folder.visible.len(),
                folder.inventory.as_ref().map_or(0, |i| i.rows.len()).saturating_sub(folder.visible.len()),
                if folder.filtering { " · Filtering…" } else { "" }
            )
        };
        body = body.child(statuses).child(
            div()
                .id("comparison-folder-summary")
                .debug_selector(|| "comparison-folder-summary".into())
                .px(px(8.))
                .text_size(px(11.))
                .child(summary),
        );
        if let Some(message) = &folder.copy_message {
            body = body.child(div().px(px(8.)).text_size(px(11.)).child(message.clone()));
        }
        if let Some(Panel::Comparison(panel)) = &self.panel {
            if let Some((source, _)) = panel.copy_candidate() {
                body = body.child(
                    kit::ghost_button(
                        "comparison-copy-file",
                        if source == 0 { "Copy File to Right" } else { "Copy File to Left" },
                    )
                    .debug_selector(|| "comparison-copy-file".into())
                    .when(self.comparison.copy.is_none(), |button| {
                        button.on_click(cx.listener(|ws, _, w, cx| ws.copy_comparison_file(w, cx)))
                    }),
                );
            }
        }
        if let Some(error) = &folder.filter_error {
            body = body.child(
                div()
                    .id("comparison-folder-filter-error")
                    .debug_selector(|| "comparison-folder-filter-error".into())
                    .px(px(8.))
                    .child(error.clone()),
            );
        }
        if let Some(inventory) = &folder.inventory {
            for issue in inventory.issues.iter().take(3) {
                body = body.child(div().px(px(8.)).text_size(px(11.)).child(issue.clone()));
            }
        }
        body.child(
            uniform_list(
                "comparison-folder-rows",
                folder.visible.len(),
                cx.processor(|ws, range: std::ops::Range<usize>, _, cx| {
                    let Some(folder) = ws.folder_review() else {
                        return Vec::new();
                    };
                    let Some(inventory) = &folder.inventory else {
                        return Vec::new();
                    };
                    #[cfg(test)]
                    FOLDER_ROW_VISITS.set(FOLDER_ROW_VISITS.get() + range.len());
                    range
                        .filter_map(|at| {
                            let index = *folder.visible.get(at)?;
                            let row = inventory.rows.get(index)?;
                            Some(
                                div()
                                    .id(("comparison-folder-row", index))
                                    .debug_selector(move || format!("comparison-folder-row-{index}"))
                                    .h(px(24.))
                                    .px(px(8.))
                                    .flex()
                                    .gap(px(8.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .when(folder.cursor == at, |d| d.bg(ui(theme::SIDEBAR_SELECTED_BG)))
                                    .hover(|s| s.bg(ui(theme::SIDEBAR_HOVER_BG)))
                                    .cursor_pointer()
                                    .child(row.status.label())
                                    .child(row.path.display().to_string())
                                    .child(row.note.clone())
                                    .on_click(cx.listener(move |ws, _, w, cx| ws.select_folder_row(index, w, cx)))
                                    .into_any_element(),
                            )
                        })
                        .collect()
                }),
            )
            .flex_1()
            .min_h_0()
            .track_scroll(folder.scroll.clone())
            .track_focus(&folder.list_focus)
            .key_context("ComparisonRows"),
        )
        .into_any_element()
    }

    fn jump_difference(&self, cx: &mut Context<Self>) {
        let Some(Panel::Comparison(p)) = &self.panel else {
            return;
        };
        let Some(r) = &p.review else { return };
        let Some(h) = r.pair.hunks.get(p.current) else {
            return;
        };
        for (editor, at) in r.viewers.iter().zip([h.lines.old_chars.start, h.lines.new_chars.start]) {
            editor.update(cx, |e, cx| e.jump_to(at, cx));
        }
    }

    fn navigate_difference(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some(Panel::Comparison(p)) = &mut self.panel else {
            return;
        };
        let n = p.review.as_ref().map_or(0, |r| r.pair.hunks.len());
        if n == 0 || p.message.is_some() {
            return;
        }
        p.current = cycle(p.current, n, next);
        self.jump_difference(cx);
        cx.notify();
    }

    pub(crate) fn refresh_comparison(&mut self, _: &RefreshComparison, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Panel::Comparison(p)) = &mut self.panel else {
            return;
        };
        // Refresh owns an accepted pair; a new selection that is refused leaves its worker
        // running
        self.comparison.begin_pick();
        if let Some(folder) = p.folder.take() {
            let roots = folder.roots.clone();
            self.load_folder_comparison(roots, Some(folder), window, cx);
        } else {
            let paths = p.paths.clone();
            self.load_comparison(paths, window, cx);
        }
    }

    pub(crate) fn render_comparison_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(Panel::Comparison(p)) = &self.panel else { unreachable!() };
        let mut body = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(ui(theme::PANEL_BG))
            .track_focus(&p.focus)
            .key_context("ComparisonPanel")
            .child(
                kit::panel_header()
                    .child(div().flex_1().child("Comparison"))
                    .child(
                        kit::icon_button("comparison-previous", Icon::ChevronUp, "Previous Difference")
                            .debug_selector(|| "comparison-previous".into())
                            .on_click(cx.listener(|ws, _, w, cx| ws.previous_difference(&PreviousDifference, w, cx))),
                    )
                    .child(
                        kit::icon_button("comparison-next", Icon::ChevronDown, "Next Difference")
                            .debug_selector(|| "comparison-next".into())
                            .on_click(cx.listener(|ws, _, w, cx| ws.next_difference(&NextDifference, w, cx))),
                    )
                    .child(
                        kit::icon_button("comparison-refresh", Icon::Refresh, "Refresh")
                            .debug_selector(|| "comparison-refresh".into())
                            .on_click(cx.listener(|ws, _, w, cx| ws.refresh_comparison(&RefreshComparison, w, cx))),
                    )
                    .child(
                        kit::icon_button("comparison-close", Icon::Close, "Close Comparison")
                            .debug_selector(|| "comparison-close".into())
                            .on_click(cx.listener(|ws, _, w, cx| ws.close_panel(&ClosePanel, w, cx))),
                    ),
            )
            .child(
                div()
                    .id("comparison-counter")
                    .debug_selector(|| "comparison-counter".into())
                    .px(px(8.))
                    .text_size(px(11.))
                    .child(if p.review.is_some() {
                        p.counter()
                    } else if p.folder.is_some() {
                        "Select a matched text file to review".into()
                    } else {
                        "Preparing snapshots".into()
                    }),
            );
        if let Some(folder) = &p.folder {
            let list = self.render_folder_rows(folder, cx);
            body = body.child(if p.review.is_some() {
                div().h(px(180.)).min_h_0().flex().child(list).into_any_element()
            } else {
                list
            });
            if p.review.is_none() {
                if let Some(message) = &p.message {
                    body = body.child(div().px(px(8.)).child(message.clone()));
                }
                return body.into_any_element();
            }
        }
        let can_apply = p.message.is_none()
            && p.visible_pair()
            && self.comparison.apply.is_none()
            && p.review.as_ref().is_some_and(|r| !r.pair.hunks.is_empty());
        let mut controls = div().flex().gap(px(6.)).px(px(8.));
        for (side, label) in ["Apply Left to Right", "Apply Right to Left"].into_iter().enumerate() {
            let mut button = kit::button(label, label).debug_selector(move || label.into());
            if can_apply {
                button = button.on_click(cx.listener(move |ws, _, w, cx| ws.apply_comparison(side, w, cx)));
            } else {
                button = button.opacity(0.4).cursor_default();
            }
            controls = controls.child(button);
        }
        body = body.child(controls);
        if let Some(message) = &p.message {
            body = body.child(div().id("comparison-message").px(px(10.)).child(message.clone()));
        }
        if p.review.as_ref().is_some_and(|r| r.pair.formats_differ) {
            body = body.child(div().px(px(10.)).child("Formats differ — text is compared after normalization"));
        }
        let mut sides = div().flex_1().min_h_0().flex().overflow_hidden();
        for side in 0..2 {
            let path = format!("{}: {}", if side == 0 { "Left" } else { "Right" }, p.paths[side].display());
            let mut column = div()
                .id(("comparison-side", side))
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .border_l_1()
                .border_color(ui(theme::BORDER))
                .child(side_label("comparison-path", side, path));
            if let Some(r) = &p.review {
                column = column
                    .child(side_label("comparison-source", side, r.sources[side].label.clone()))
                    .child(side_label("comparison-range", side, p.range_label(side)))
                    .child(div().flex_1().min_h_0().child(r.viewers[side].clone()));
            }
            sides = sides.child(column);
        }
        body = body.child(sides);
        body.into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::super::ui_tests::window;
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    fn files(name: &str, left: &str, right: &str) -> [PathBuf; 2] {
        let root = std::env::temp_dir().join(format!("orlo-comparison-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = [root.join("left.txt"), root.join("right.txt")];
        std::fs::write(&paths[0], left).unwrap();
        std::fs::write(&paths[1], right).unwrap();
        paths
    }

    fn folder_roots(name: &str) -> [PathBuf; 2] {
        let files = files(name, "", "");
        let roots = [files[0].with_extension("dir"), files[1].with_extension("dir")];
        for root in &roots {
            let _ = std::fs::remove_dir_all(root);
            std::fs::create_dir_all(root).unwrap();
        }
        roots
    }

    fn panel(ws: &Workspace) -> &ComparisonPanel {
        let Some(Panel::Comparison(p)) = &ws.panel else { panic!("comparison") };
        p
    }

    fn status(ws: &Workspace) -> &str {
        &ws.status.as_ref().unwrap().0
    }

    fn review(ws: &Workspace) -> &Review {
        panel(ws).review.as_deref().unwrap()
    }

    fn folder(ws: &Workspace) -> &FolderReview {
        ws.folder_review().unwrap()
    }

    /// The test's workspace, one in a second window, and a context for each.
    fn two_windows(
        cx: &mut TestAppContext,
    ) -> (Entity<Workspace>, Entity<Workspace>, VisualTestContext, VisualTestContext) {
        let (ws, _) = window(cx);
        let (other, _) = cx.add_window_view(Workspace::new);
        let [first, second] =
            [&ws, &other].map(|ws| VisualTestContext::from_window(ws.read_with(cx, |ws, _| ws.window), cx));
        (ws, other, first, second)
    }

    /// A fresh hard link to `path` at `alias`.
    fn link(path: &Path, alias: PathBuf) -> PathBuf {
        let _ = std::fs::remove_file(&alias);
        std::fs::hard_link(path, &alias).unwrap();
        alias
    }

    fn open(ws: &mut Workspace, path: &Path, w: &mut Window, cx: &mut Context<Workspace>) -> Entity<Editor> {
        ws.open_path(path.to_path_buf(), w, cx);
        ws.active_editor().unwrap()
    }

    fn add_tab(ws: &mut Workspace, text: &str, path: &Path, w: &mut Window, cx: &mut Context<Workspace>) {
        let mut state = EditorState::from_text(text);
        state.doc.path = Some(path.to_path_buf());
        ws.add_tab(state, w, cx);
    }

    fn restoring(path: &Path) -> sessions::RestoringView {
        sessions::RestoringView {
            view: ViewState { path: path.to_path_buf(), ..Default::default() },
            ..Default::default()
        }
    }

    fn hide_left_only(ws: &mut Workspace, cx: &mut Context<Workspace>) {
        if let Some(f) = ws.folder_review_mut() {
            f.statuses[folders::Status::LeftOnly as usize] = false;
        }
        ws.filter_folder_rows(cx);
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    /// Compares `paths`, as folders when `folders`, and waits for the result.
    fn compare(ws: &Entity<Workspace>, paths: &[PathBuf; 2], folders: bool, cx: &mut VisualTestContext) {
        ws.update_in(cx, |ws, w, cx| ws.start_comparison(paths.clone(), folders, w, cx));
        cx.run_until_parked();
    }

    /// Compares the folders `roots` and asks to copy the first row's file.
    fn start_copy(ws: &Entity<Workspace>, roots: &[PathBuf; 2], cx: &mut VisualTestContext) {
        compare(ws, roots, true, cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.select_folder_row(0, w, cx);
            ws.copy_comparison_file(w, cx);
        });
        cx.run_until_parked();
    }

    /// The running Copy's stop flag, the comparison's generation and its folder inventory.
    fn copy_state(ws: &Workspace) -> (Arc<AtomicBool>, u64, Arc<Inventory>) {
        (ws.comparison.copy.clone().unwrap(), ws.comparison.generation, folder(ws).inventory.clone().unwrap())
    }

    #[gpui::test]
    fn closed_legacy_folder_copy_preserves_disk_encoding_and_bytes(cx: &mut TestAppContext) {
        use crate::editing::encoding::Encoding;
        for source in 0..2 {
            for (encoding, bytes) in
                [(Encoding::Windows1252, &b"caf\xe9\r\n"[..]), (Encoding::Iso8859_1, &b"\x80caf\xe9\r\n"[..])]
            {
                let roots = folder_roots(&format!("closed-legacy-copy-{source}-{encoding:?}"))
                    .map(|path| path.canonicalize().unwrap());
                let from = roots[source].join("source.txt");
                let to = roots[1 - source].join("source.txt");
                std::fs::write(&from, bytes).unwrap();
                let (ws, cx) = window(cx);
                cx.simulate_resize(size(px(1200.), px(900.)));
                ws.update_in(cx, |ws, window, cx| {
                    let loaded = Loaded::read_encoding(&from, Some(encoding)).unwrap();
                    ws.add_tab(EditorState::from_loaded(&from, loaded), window, cx);
                    let editor = ws.active_editor().unwrap();
                    assert!(!editor.read(cx).doc(cx).read_only);
                    ws.close_editor(&editor, window, cx);
                    assert_eq!(ws.remembered_disk_encoding(&from), Some(encoding));
                });
                compare(&ws, &roots, true, cx);
                ws.update_in(cx, |ws, window, cx| {
                    ws.select_folder_row(0, window, cx);
                    assert!(panel(ws).copy_candidate().is_some(), "known legacy source must be copy-eligible");
                });
                cx.run_until_parked();
                click(cx, "comparison-copy-file");
                cx.run_until_parked();
                assert!(cx.has_pending_prompt(), "closed {encoding:?} source must reach Copy confirmation");
                assert!(!to.exists());
                cx.simulate_prompt_answer("Copy File");
                cx.run_until_parked();
                assert_eq!(std::fs::read(&from).unwrap(), bytes);
                assert_eq!(std::fs::read(&to).unwrap(), bytes);
                ws.read_with(cx, |ws, cx| {
                    assert!(ws.comparison.copy.is_none());
                    assert!(!pipelines::writing(&to, cx));
                    assert!(folder(ws).copy_message.as_ref().unwrap().starts_with("Copied file"));
                });
            }
        }
    }

    #[gpui::test]
    fn copy_file_confirmation_cancel_and_both_directions(cx: &mut TestAppContext) {
        for source in 0..2 {
            let roots = folder_roots(&format!("copy-ui-{source}"));
            std::fs::create_dir_all(roots[source].join("a/b")).unwrap();
            let from = roots[source].join("a/b/file.txt");
            let to = roots[1 - source].join("a/b/file.txt");
            std::fs::write(&from, "reviewed\r\n").unwrap();
            let (ws, cx) = window(cx);
            cx.simulate_resize(size(px(1200.), px(900.)));
            compare(&ws, &roots, true, cx);
            ws.update_in(cx, |ws, w, cx| ws.select_folder_row(0, w, cx));
            cx.run_until_parked();
            click(cx, "comparison-copy-file");
            cx.run_until_parked();
            let (title, detail) = cx.pending_prompt().expect("concrete confirmation");
            assert!(title.contains("Copy File"));
            assert!(detail.contains(&from.display().to_string()), "{detail}");
            assert!(detail.contains(&to.display().to_string()), "{detail}");
            assert!(detail.contains("a/b"), "{detail}");
            assert!(!detail.contains("Undo"));
            assert!(!to.exists());
            cx.simulate_prompt_answer("Cancel");
            cx.run_until_parked();
            assert!(!roots[1 - source].join("a").exists());
            ws.update_in(cx, |ws, w, cx| ws.copy_comparison_file(w, cx));
            cx.run_until_parked();
            cx.simulate_prompt_answer("Copy File");
            cx.run_until_parked();
            assert_eq!(std::fs::read(&to).unwrap(), b"reviewed\r\n");
            ws.read_with(cx, |ws, cx| {
                assert!(ws.comparison.copy.is_none());
                assert!(!pipelines::writing(&to, cx));
                let f = folder(ws);
                assert!(f.copy_message.as_ref().unwrap().starts_with("Copied file"));
                assert_eq!(f.inventory.as_ref().unwrap().rows[0].status, folders::Status::Equal);
            });
        }
    }

    fn copy_file_other_window_close(cx: &mut TestAppContext, answer: Option<&str>) {
        let roots = folder_roots(&format!("copy-window-close-{answer:?}"));
        std::fs::create_dir_all(roots[0].join("a/b")).unwrap();
        let source = roots[0].join("a/b/file");
        let destination = roots[1].join("a/b/file");
        std::fs::write(&source, "reviewed\r\n").unwrap();
        let (ws, other, mut vcx, _) = two_windows(cx);
        let other_handle = other.read_with(cx, |ws, _| ws.window);
        vcx.simulate_resize(size(px(1200.), px(900.)));
        compare(&ws, &roots, true, &mut vcx);
        ws.update_in(&mut vcx, |ws, w, cx| ws.select_folder_row(0, w, cx));
        let (stop, generation, inventory) = ws.update_in(&mut vcx, |ws, w, cx| {
            ws.copy_comparison_file(w, cx);
            if answer.is_none() {
                // Close before the capture task gets its first executor turn.
                other_handle.update(cx, |_, w, _| w.remove_window()).unwrap();
            }
            copy_state(ws)
        });
        vcx.run_until_parked();
        if let Some(answer) = answer {
            assert!(vcx.has_pending_prompt(), "real pending confirmation");
            other_handle.update(&mut vcx, |_, w, _| w.remove_window()).unwrap();
            assert!(stop.load(Ordering::Relaxed));
            vcx.simulate_prompt_answer(answer);
            vcx.run_until_parked();
        }
        assert!(stop.load(Ordering::Relaxed));
        assert!(!vcx.has_pending_prompt());
        assert_eq!(std::fs::read(&source).unwrap(), b"reviewed\r\n");
        assert_eq!(std::fs::read_dir(&roots[1]).unwrap().count(), 0);
        ws.read_with(&vcx, |ws, cx| {
            assert!(ws.comparison.copy.is_none(), "cancelled Copy must retire");
            assert!(!pipelines::writing(&destination, cx));
            assert_eq!(ws.comparison.generation, generation, "no implicit Refresh");
            let folder = folder(ws);
            assert!(Arc::ptr_eq(folder.inventory.as_ref().unwrap(), &inventory));
            assert_eq!(folder.copy_message.as_deref(), Some("Copy cancelled — no files created"));
        });
        click(&mut vcx, "comparison-copy-file");
        vcx.run_until_parked();
        assert!(vcx.has_pending_prompt(), "fresh Copy control must work");
        ws.read_with(&vcx, |ws, _| {
            let active = ws.comparison.copy.as_ref().unwrap();
            assert!(!Arc::ptr_eq(active, &stop));
            assert!(!active.load(Ordering::Relaxed));
        });
        vcx.simulate_prompt_answer("Copy File");
        vcx.run_until_parked();
        assert_eq!(std::fs::read(&destination).unwrap(), b"reviewed\r\n");
        ws.read_with(&vcx, |ws, _| assert!(ws.comparison.copy.is_none()));
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[gpui::test]
    fn copy_file_window_close_during_capture_retires_copy(cx: &mut TestAppContext) {
        copy_file_other_window_close(cx, None);
    }

    #[gpui::test]
    fn copy_file_window_close_during_confirmation_accept_retires_copy(cx: &mut TestAppContext) {
        copy_file_other_window_close(cx, Some("Copy File"));
    }

    #[gpui::test]
    fn copy_file_window_close_during_confirmation_cancel_retires_copy(cx: &mut TestAppContext) {
        copy_file_other_window_close(cx, Some("Cancel"));
    }

    fn copy_file_stale_completion(cx: &mut TestAppContext, confirmation: bool) {
        let roots = folder_roots(&format!("copy-stale-completion-{confirmation}"));
        std::fs::create_dir_all(roots[0].join("a/b")).unwrap();
        std::fs::write(roots[0].join("a/b/file"), "reviewed\n").unwrap();
        let (ws, cx) = window(cx);
        compare(&ws, &roots, true, cx);
        ws.update_in(cx, |ws, w, cx| ws.select_folder_row(0, w, cx));
        let old = ws.update_in(cx, |ws, w, cx| {
            ws.copy_comparison_file(w, cx);
            ws.comparison.copy.as_ref().unwrap().clone()
        });
        if confirmation {
            cx.run_until_parked();
            assert!(cx.has_pending_prompt());
            ws.update_in(cx, |ws, w, cx| ws.refresh_comparison(&RefreshComparison, w, cx));
            cx.run_until_parked();
            // Reply wakes the old task but does not run it before the replacement starts.
            cx.simulate_prompt_answer("Copy File");
        }
        let (new, generation, inventory) = ws.update_in(cx, |ws, w, cx| {
            ws.select_folder_row(0, w, cx);
            ws.copy_comparison_file(w, cx);
            copy_state(ws)
        });
        assert!(old.load(Ordering::Relaxed));
        assert!(!Arc::ptr_eq(&old, &new));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(Arc::ptr_eq(ws.comparison.copy.as_ref().unwrap(), &new));
            assert!(!new.load(Ordering::Relaxed));
            assert_eq!(ws.comparison.generation, generation);
            let folder = folder(ws);
            assert!(Arc::ptr_eq(folder.inventory.as_ref().unwrap(), &inventory));
            assert!(folder.copy_message.is_none(), "no stale cancellation result");
        });
        assert!(cx.has_pending_prompt(), "new Copy still awaits its own answer");
        assert_eq!(std::fs::read_dir(&roots[1]).unwrap().count(), 0);
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.comparison.copy.is_none()));
        assert!(!cx.has_pending_prompt());
        assert_eq!(std::fs::read_dir(&roots[1]).unwrap().count(), 0);
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[gpui::test]
    fn copy_file_stale_capture_cannot_retire_new_copy(cx: &mut TestAppContext) {
        copy_file_stale_completion(cx, false);
    }

    #[gpui::test]
    fn copy_file_stale_confirmation_cannot_retire_new_copy(cx: &mut TestAppContext) {
        copy_file_stale_completion(cx, true);
    }

    #[gpui::test]
    fn copy_file_rejects_dirty_other_window_and_global_busy_destinations(cx: &mut TestAppContext) {
        for operation in ["dirty", "ambiguous", "owner", "loading", "restoring", "lease", "alias-error"] {
            let roots = folder_roots(&format!("copy-initial-{operation}"));
            let source = roots[0].join("file");
            let destination = roots[1].join("file");
            std::fs::write(&source, "reviewed\n").unwrap();
            let alias = link(&source, source.with_extension("alias"));
            let (ws, other, mut vcx, mut other_cx) = two_windows(cx);
            let lease = other.update_in(&mut other_cx, |ws, w, cx| {
                match operation {
                    "dirty" => {
                        ws.open_path(source.clone(), w, cx);
                        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved")));
                    }
                    "ambiguous" => {
                        ws.open_path(source.clone(), w, cx);
                        add_tab(ws, "independent", &alias, w, cx);
                    }
                    "owner" => add_tab(ws, "keep open target", &destination, w, cx),
                    "loading" => {
                        ws.loading.insert(destination.clone());
                    }
                    "restoring" => ws.restoring.push(restoring(&destination)),
                    "lease" => return Some(pipelines::lease(&roots[1], cx)),
                    _ => {
                        let bad = roots[1].join("loop");
                        std::os::unix::fs::symlink("loop", &bad).unwrap();
                        add_tab(ws, "unresolvable owner", &bad, w, cx);
                    }
                }
                None
            });
            start_copy(&ws, &roots, &mut vcx);
            assert!(!vcx.has_pending_prompt(), "no stale/unsafe confirmation: {operation}");
            ws.read_with(&vcx, |ws, _| {
                let message = folder(ws).copy_message.as_ref().expect("visible rejection");
                assert!(message.contains("Copy rejected"), "{operation}: {message}");
            });
            assert!(!destination.exists(), "{operation}");
            assert_eq!(std::fs::read(&source).unwrap(), b"reviewed\n");
            drop(lease);
        }
    }

    #[gpui::test]
    fn copy_file_staging_rechecks_owners_context_loads_and_leases(cx: &mut TestAppContext) {
        for operation in [
            "dirty",
            "move",
            "close",
            "reopen",
            "alias",
            "destination",
            "loading",
            "restoring",
            "lease",
            "replace-lease",
            "close-panel",
            "refresh",
            "project",
            "filter",
            "closed-window",
            "open-waits",
        ] {
            let roots = folder_roots(&format!("copy-staged-{operation}"));
            std::fs::create_dir_all(roots[0].join("a/b")).unwrap();
            let source = roots[0].join("a/b/file");
            let destination = roots[1].join("a/b/file");
            std::fs::write(&source, "reviewed\n").unwrap();
            let alias = link(&source, roots[0].join("alias"));
            let (ws, other, mut vcx, mut other_cx) = two_windows(cx);
            let editor = other.update_in(&mut other_cx, |ws, w, cx| open(ws, &source, w, cx));
            // a/b/file sorts before the separate hardlink alias row.
            start_copy(&ws, &roots, &mut vcx);
            assert!(vcx.has_pending_prompt(), "{operation}");
            let leases = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let held = leases.clone();
            let to = destination.clone();
            let from = source.clone();
            let hook_editor = editor.clone();
            let other_handle = other_cx.window_handle().downcast::<Workspace>().unwrap();
            copy::AFTER_STAGE.with_borrow_mut(|hook| {
                *hook = Some(Box::new(move |ws, w, cx| {
                    assert!(
                        pipelines::writing(&crate::editing::text::resolve(&to), cx),
                        "destination lease active through stage"
                    );
                    assert!(pipelines::writing(&to.parent().unwrap().join("another"), cx), "missing subtree reserved");
                    match operation {
                        "dirty" => hook_editor.update(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved"))),
                        "move" => hook_editor
                            .update(cx, |e, cx| e.update_doc(cx, |d| d.path = Some(from.with_extension("moved")))),
                        "close" | "reopen" => {
                            other_handle
                                .update(cx, |other, w, cx| {
                                    other.close_view(hook_editor.clone(), w, cx);
                                    if operation == "reopen" {
                                        add_tab(other, "reviewed\n", &from, w, cx);
                                    }
                                })
                                .unwrap();
                        }
                        "alias" | "destination" => {
                            let path = if operation == "alias" { &alias } else { &to };
                            other_handle
                                .update(cx, |other, w, cx| add_tab(other, "independent owner", path, w, cx))
                                .unwrap();
                        }
                        "loading" => {
                            other.update(cx, |other, _| {
                                other.loading.insert(to.clone());
                            });
                        }
                        "restoring" => {
                            other.update(cx, |other, _| other.restoring.push(restoring(&to)));
                        }
                        "lease" => held.borrow_mut().push(pipelines::lease(&to.with_extension("child"), cx)),
                        "replace-lease" => held.borrow_mut().push(pipelines::lease(&to, cx)),
                        "close-panel" => ws.close_panel(&ClosePanel, w, cx),
                        "refresh" => ws.refresh_comparison(&RefreshComparison, w, cx),
                        "project" => ws.clear_window(w, cx),
                        "closed-window" => w.remove_window(),
                        "filter" => hide_left_only(ws, cx),
                        _ => {
                            other_handle
                                .update(cx, |other, w, cx| {
                                    other.open_path(to.clone(), w, cx);
                                    assert!(
                                        !other.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref()
                                            == Some(&crate::editing::text::resolve(&to))),
                                        "ordinary open waits on lease"
                                    );
                                })
                                .unwrap();
                        }
                    }
                }))
            });
            vcx.simulate_prompt_answer("Copy File");
            vcx.run_until_parked();
            if operation == "open-waits" {
                assert_eq!(std::fs::read(&destination).unwrap(), b"reviewed\n");
            } else {
                assert!(!destination.exists(), "staged review must expire: {operation}");
                assert!(!roots[1].join("a").exists(), "only owned empty parents removed: {operation}");
            }
            ws.read_with(&vcx, |ws, cx| {
                if operation != "closed-window" {
                    assert!(ws.comparison.copy.is_none(), "{operation}");
                }
                if operation != "replace-lease" {
                    assert!(!pipelines::writing(&destination, cx), "released: {operation}");
                }
            });
            assert_eq!(std::fs::read(&source).unwrap(), b"reviewed\n");
            drop(leases);
        }
    }

    #[gpui::test]
    fn copy_file_confirmation_rechecks_fresh_disk_and_app_state(cx: &mut TestAppContext) {
        for operation in
            ["source-disk", "dirty", "destination-file", "destination-link", "loading", "restoring", "lease", "hidden"]
        {
            let roots = folder_roots(&format!("copy-confirm-{operation}"));
            let source = roots[0].join("file");
            let destination = roots[1].join("file");
            std::fs::write(&source, "reviewed\n").unwrap();
            let (ws, vcx) = window(cx);
            let editor = ws.update_in(vcx, |ws, w, cx| open(ws, &source, w, cx));
            start_copy(&ws, &roots, vcx);
            assert!(vcx.has_pending_prompt(), "{operation}");
            let lease = ws.update_in(vcx, |ws, _, cx| {
                match operation {
                    "source-disk" => std::fs::write(&source, "new disk source").unwrap(),
                    "dirty" => editor.update(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved"))),
                    "destination-file" => std::fs::write(&destination, "keep target").unwrap(),
                    "destination-link" => std::os::unix::fs::symlink(&source, &destination).unwrap(),
                    "loading" => {
                        ws.loading.insert(destination.clone());
                    }
                    "restoring" => ws.restoring.push(restoring(&destination)),
                    "lease" => return Some(pipelines::lease(&roots[1], cx)),
                    _ => hide_left_only(ws, cx),
                }
                None
            });
            vcx.simulate_prompt_answer("Copy File");
            vcx.run_until_parked();
            match operation {
                "destination-file" => {
                    assert_eq!(std::fs::read(&destination).unwrap(), b"keep target")
                }
                "destination-link" => assert!(std::fs::symlink_metadata(&destination).unwrap().is_symlink()),
                _ => assert!(!destination.exists(), "{operation}"),
            }
            assert!(std::fs::read_dir(&roots[1]).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".orlo-")));
            ws.read_with(vcx, |ws, _| assert!(ws.comparison.copy.is_none()));
            drop(lease);
        }
    }

    fn copy_after_guard_races(operations: &[&'static str], cx: &mut TestAppContext) {
        let mut results = Vec::new();
        for &operation in operations {
            let roots = folder_roots(&format!("copy-after-guard-{operation}"));
            std::fs::create_dir_all(roots[0].join("a/b")).unwrap();
            let source = roots[0].join("a/b/file");
            let destination = roots[1].join("a/b/file");
            std::fs::write(&source, "reviewed\n").unwrap();
            let alias = link(&source, roots[0].parent().unwrap().join("outside-alias"));
            let alias = crate::editing::text::resolve(&alias);
            let (ws, other, mut vcx, mut other_cx) = two_windows(cx);
            let editor = matches!(operation, "close" | "move" | "reopen" | "close-window")
                .then(|| other.update_in(&mut other_cx, |ws, w, cx| open(ws, &source, w, cx)));
            if operation == "unrelated-edit" {
                let unrelated = roots[0].parent().unwrap().join("unrelated");
                std::fs::write(&unrelated, "unrelated text").unwrap();
                other.update_in(&mut other_cx, |ws, w, cx| ws.open_path(unrelated, w, cx));
            }
            start_copy(&ws, &roots, &mut vcx);
            assert!(vcx.has_pending_prompt(), "{operation}");
            let other_window = other_cx.window_handle().downcast::<Workspace>().unwrap();
            let held = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let held_hook = held.clone();
            let to = destination.clone();
            let from = source.clone();
            let hook_editor = editor.clone();
            copy::AFTER_GUARD.with_borrow_mut(|hook| {
                *hook = Some(Box::new(move |ws, _, cx| {
                    assert!(pipelines::writing(&to, cx), "leases remain held after final UI guard");
                    match operation {
                        "new-dirty-alias" => {
                            other_window
                                .update(cx, |other, w, cx| {
                                    other.open_path(alias.clone(), w, cx);
                                    assert_eq!(
                                        other.active_editor().unwrap().read(cx).doc(cx).path.as_ref(),
                                        Some(&crate::editing::text::resolve(&alias))
                                    );
                                    other.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved alias text")));
                                })
                                .unwrap();
                        }
                        "new-workspace" => {
                            let handle = cx
                                .open_window(WindowOptions::default(), |w, cx| cx.new(|cx| Workspace::new(w, cx)))
                                .unwrap();
                            handle
                                .update(cx, |other, w, cx| {
                                    other.open_path(alias.clone(), w, cx);
                                    other.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved new window")));
                                })
                                .unwrap();
                        }
                        "close-window" => {
                            other_window.update(cx, |_, w, _| w.remove_window()).unwrap();
                        }
                        "unrelated-edit" => {
                            other_window
                                .update(cx, |other, _, cx| {
                                    other.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unrelated unsaved")))
                                })
                                .unwrap();
                        }
                        "notify" => ws.status_message("unrelated notification", cx),
                        "close" | "reopen" => {
                            other_window
                                .update(cx, |other, w, cx| {
                                    other.close_view(hook_editor.clone().unwrap(), w, cx);
                                    if operation == "reopen" {
                                        other.open_path(from.clone(), w, cx);
                                    }
                                })
                                .unwrap();
                        }
                        "move" => hook_editor.unwrap().update(cx, |e, cx| {
                            e.update_doc(cx, |d| d.path = Some(alias));
                            cx.notify();
                        }),
                        "loading" => {
                            other_window
                                .update(cx, |other, w, cx| {
                                    other.read_in_background(alias.clone(), w, cx);
                                    assert!(other.loading.contains(&crate::editing::text::resolve(&alias)));
                                })
                                .unwrap();
                        }
                        "restoring" => {
                            other_window
                                .update(cx, |other, w, cx| {
                                    other.restore_later(
                                        vec![(0, 0, true, ViewState { path: alias, ..Default::default() })],
                                        w,
                                        cx,
                                    )
                                })
                                .unwrap();
                        }
                        "alias-lease" => held_hook.borrow_mut().push(pipelines::lease(&alias, cx)),
                        _ => held_hook.borrow_mut().push(pipelines::lease(&to, cx)),
                    }
                }))
            });
            vcx.simulate_prompt_answer("Copy File");
            vcx.run_until_parked();
            copy::AFTER_GUARD
                .with_borrow(|hook| assert!(hook.is_none(), "must reach after final allowed guard: {operation}"));
            let absent = !destination.exists();
            let should_publish = matches!(operation, "unrelated-edit" | "notify");
            results.push((operation, absent != should_publish, (!roots[1].join("a").exists()) != should_publish));
            if should_publish {
                assert_eq!(std::fs::read(&destination).unwrap(), b"reviewed\n");
            }
            assert_eq!(std::fs::read(&source).unwrap(), b"reviewed\n");
            ws.read_with(&vcx, |ws, cx| {
                assert!(ws.comparison.copy.is_none());
                if operation != "destination-lease" {
                    assert!(!pipelines::writing(&destination, cx));
                }
            });
            if operation == "reopen" {
                other_cx.executor().advance_clock(std::time::Duration::from_millis(20));
                other_cx.run_until_parked();
                other.read_with(&other_cx, |other, cx| {
                    let reopened = other.active_editor().unwrap();
                    assert_eq!(reopened.read(cx).doc(cx).path.as_ref(), Some(&crate::editing::text::resolve(&source)));
                    assert_ne!(reopened.read(cx).doc(cx).id(), editor.as_ref().unwrap().read(cx).doc(cx).id());
                });
            }
            if operation == "new-dirty-alias" {
                other.read_with(&other_cx, |other, cx| {
                    let e = other.active_editor().unwrap();
                    assert!(e.read(cx).doc(cx).dirty);
                    assert!(e.read(cx).text(cx).contains("unsaved alias text"));
                });
            }
            drop(held);
        }
        assert!(
            results.iter().all(|(_, absent, clean)| *absent && *clean),
            "after-guard publication/owned cleanup: {results:?}"
        );
    }

    #[gpui::test]
    fn copy_file_after_guard_rejects_new_dirty_preexisting_alias(cx: &mut TestAppContext) {
        copy_after_guard_races(&["new-dirty-alias", "new-workspace"], cx);
    }
    #[gpui::test]
    fn copy_file_after_guard_rejects_source_owner_close_move_and_reopen(cx: &mut TestAppContext) {
        copy_after_guard_races(&["close", "move", "reopen", "close-window"], cx);
    }
    #[gpui::test]
    fn copy_file_after_guard_rejects_load_restore_and_write_changes(cx: &mut TestAppContext) {
        copy_after_guard_races(&["loading", "restoring", "alias-lease", "destination-lease"], cx);
    }

    #[gpui::test]
    fn copy_file_after_guard_allows_unrelated_text_and_notifications(cx: &mut TestAppContext) {
        copy_after_guard_races(&["unrelated-edit", "notify"], cx);
    }

    #[gpui::test]
    fn folder_rows_filters_render_and_matched_apply_retain_the_roots(cx: &mut TestAppContext) {
        let roots = folder_roots("folder-review");
        std::fs::write(roots[0].join("matched.rs"), "left\n").unwrap();
        std::fs::write(roots[1].join("matched.rs"), "right\n").unwrap();
        std::fs::write(roots[0].join("left.rs"), "only").unwrap();
        std::fs::write(roots[1].join("right.rs"), "only").unwrap();
        for root in &roots {
            std::fs::write(root.join("z-equal.rs"), "same").unwrap();
        }
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1200.), px(900.)));
        compare(&ws, &roots, true, cx);
        let inputs = ws.update_in(cx, |ws, _, _| {
            let f = folder(ws);
            assert_eq!(f.inventory.as_ref().unwrap().rows.len(), 4);
            assert_eq!(f.visible.len(), 3);
            f.inputs.clone()
        });
        for selector in [
            "comparison-folder-roots",
            "comparison-folder-summary",
            "comparison-folder-row-0",
            "comparison-folder-include",
            "comparison-folder-exclude",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "{selector}");
        }
        for expected in [4, 3] {
            click(cx, "comparison-status-0");
            cx.run_until_parked();
            ws.read_with(cx, |ws, _| assert_eq!(folder(ws).visible.len(), expected));
        }
        click(cx, "comparison-folder-row-1");
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let p = panel(ws);
            assert!(p.folder.is_some() && p.review.is_some());
            for viewer in &p.review.as_ref().unwrap().viewers {
                assert!(
                    viewer.read(cx).drawn_frame().is_some(),
                    "paired viewers receive a real frame beside the folder list"
                );
            }
            assert_eq!(p.paths, [roots[0].join("matched.rs"), roots[1].join("matched.rs")]);
            ws.apply_comparison(0, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            let p = panel(ws);
            assert!(p.folder.is_some());
            assert!(p.message.as_deref().unwrap().contains("Refresh"));
            inputs[0].update(cx, |e, cx| e.set_text("*.rs;[", cx));
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            let f = folder(ws);
            assert!(f.filter_error.as_ref().unwrap().contains("Include"));
            assert_eq!(inputs[0].read(cx).doc(cx).text(), "*.rs;[");
            assert!(f.visible.is_empty());
            inputs[0].update(cx, |e, cx| e.set_text("*", cx));
            inputs[1].update(cx, |e, cx| e.set_text("matched*", cx));
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(folder(ws).visible.len(), 2);
            assert!(panel(ws).review.is_some(), "visibility does not replace reviewed sources");
            ws.refresh_comparison(&RefreshComparison, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, _| {
            assert!(panel(ws).review.is_none());
            assert_eq!(folder(ws).roots, roots);
        });
        assert_eq!(std::fs::read_to_string(roots[1].join("matched.rs")).unwrap(), "right\n");
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[gpui::test]
    fn folder_hidden_or_inspection_rows_cannot_apply_a_retained_pair(cx: &mut TestAppContext) {
        let roots = folder_roots("folder-hidden-apply");
        for (i, root) in roots.iter().enumerate() {
            std::fs::write(root.join("a.txt"), if i == 0 { "left" } else { "right" }).unwrap();
        }
        std::fs::write(roots[0].join("b-only.txt"), "inspection").unwrap();
        let (ws, cx) = window(cx);
        compare(&ws, &roots, true, cx);
        ws.update_in(cx, |ws, w, cx| ws.select_folder_row(0, w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            ws.apply_comparison(0, w, cx);
            let stop = ws.comparison.apply.as_ref().unwrap().clone();
            let input = folder(ws).inputs[1].clone();
            input.update(cx, |e, cx| e.set_text("a.txt", cx));
            ws.filter_folder_rows(cx);
            assert!(stop.load(Ordering::Relaxed));
            assert!(ws.comparison.apply.is_none());
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            folder(ws).inputs[1].clone().update(cx, |e, cx| e.set_text("a.txt", cx));
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            ws.apply_comparison(0, w, cx);
            assert!(ws.comparison.apply.is_none(), "a hidden row cannot authorize Apply");
            ws.select_folder_row(1, w, cx);
            assert!(panel(ws).review.is_none(), "inspection row must not leave another row's Apply active");
        });
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(roots[1].join("a.txt")).unwrap(), "right");
        assert!(!roots[1].join("b-only.txt").exists());
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[gpui::test]
    fn folder_inventory_size_only_renders_visible_rows_and_filters_off_thread(cx: &mut TestAppContext) {
        let roots = folder_roots("folder-virtual");
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1200.), px(900.)));
        compare(&ws, &roots, true, cx);
        ws.update_in(cx, |ws, _, cx| {
            let f = ws.folder_review_mut().unwrap();
            f.inventory = Some(Arc::new(Inventory {
                rows: (0..folders::MAX_ENTRIES)
                    .map(|i| folders::Row {
                        path: format!("file-{i:06}.rs").into(),
                        status: folders::Status::Different,
                        note: String::new(),
                        text_pair: false,
                        directory: false,
                    })
                    .collect(),
                scanned: folders::MAX_ENTRIES,
                incomplete: false,
                limited: false,
                issues: Vec::new(),
            }));
            f.summary = f.inventory.as_ref().unwrap().summary();
            FOLDER_ROW_VISITS.set(0);
            ws.filter_folder_rows(cx);
            assert!(ws.comparison.filter_stop.is_some());
            assert!(folder(ws).filtering);
            assert!(folder(ws).visible.is_empty(), "no UI-thread filter scan");
        });
        cx.run_until_parked();
        assert!(FOLDER_ROW_VISITS.get() > 0 && FOLDER_ROW_VISITS.get() < 1000);
        ws.update_in(cx, |ws, _, cx| {
            let f = ws.folder_review_mut().unwrap();
            assert_eq!(f.visible.len(), folders::MAX_ENTRIES);
            FOLDER_ROW_VISITS.set(0);
            f.scroll.scroll_to_item(folders::MAX_ENTRIES - 1, gpui::ScrollStrategy::Top);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("comparison-folder-row-99999").is_some());
        eprintln!(
            "folder inventory: 100000 rows, {} visible-row render visits after deep scroll",
            FOLDER_ROW_VISITS.get()
        );
        assert!(FOLDER_ROW_VISITS.get() > 0 && FOLDER_ROW_VISITS.get() < 1000);
        ws.update_in(cx, |ws, _, cx| {
            let input = folder(ws).inputs[0].clone();
            input.update(cx, |e, cx| e.set_text("*000001*", cx));
            ws.filter_folder_rows(cx);
            let stop = ws.comparison.filter_stop.as_ref().unwrap().clone();
            input.update(cx, |e, cx| e.set_text("*099999*", cx));
            ws.filter_folder_rows(cx);
            assert!(stop.load(Ordering::Relaxed));
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(folder(ws).visible.as_slice(), [99999]);
            for _ in 0..3 {
                ws.focus_comparison_next(&FocusComparisonNext, w, cx);
            }
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert!(folder(ws).list_focus.is_focused(w));
            ws.next_folder_row(&NextFolderRow, w, cx);
            ws.open_folder_row(&OpenFolderRow, w, cx);
            assert!(panel(ws).message.as_ref().unwrap().contains("inspection"));
        });
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[gpui::test]
    fn folder_picker_preflight_preserves_ready_review_refresh_and_newer_picker(cx: &mut TestAppContext) {
        let roots = folder_roots("folder-picker");
        let options = picker_options(true, true);
        assert!(!options.files && options.directories && !options.multiple);
        assert!(options.prompt.as_deref().unwrap().contains("Left"));
        assert!(picker_options(true, false).prompt.as_deref().unwrap().contains("Right"));
        let (ws, cx) = window(cx);
        let picker = ws.update_in(cx, |ws, w, cx| {
            ws.start_comparison(roots.clone(), true, w, cx);
            ws.comparison.begin_pick()
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(ws.comparison.pick(picker, None, Some(roots[0].clone())), Pick::Right(roots[0].clone()));
            ws.refresh_comparison(&RefreshComparison, w, cx);
            let stop = ws.comparison.folder_stop.as_ref().unwrap().clone();
            let picker = ws.comparison.begin_pick();
            assert_eq!(ws.comparison.pick(picker, Some(roots[0].clone()), None), Pick::Cancelled);
            ws.start_comparison([roots[0].clone(), roots[0].join(".")], true, w, cx);
            assert!(!stop.load(Ordering::Relaxed));
        });
        cx.run_until_parked();
        let input = ws.update_in(cx, |ws, _, _| {
            assert!(panel(ws).message.is_none());
            folder(ws).inputs[0].clone()
        });
        let wrong = roots[0].join("file");
        std::fs::write(&wrong, "text").unwrap();
        ws.update_in(cx, |ws, w, cx| ws.start_comparison([roots[0].clone(), wrong], true, w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws,_,cx| {
            assert!(panel(ws).message.is_none());
            assert_eq!(folder(ws).inputs[0],input);
            let menus = ws.menus(cx);
            let compare = menus.iter().find(|m| m.name == "Tools").unwrap().items.iter().find_map(|i| match i { MenuItem::Submenu(m) if m.name == "Compare" => Some(m), _ => None }).unwrap();
            assert!(compare.items.iter().any(|i| matches!(i,MenuItem::Action { name, action, .. } if name == "Compare Folders…" && action.as_any().is::<CompareFolders>())));
            assert!(crate::chrome::quick_panel::command_items().iter().any(|i| matches!(&i.action,crate::chrome::quick_panel::ItemAction::Dispatch(a) if a.as_any().is::<CompareFolders>())));
        });
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    #[gpui::test]
    fn folder_scan_refresh_filter_and_pair_lifetimes_reject_late_results(cx: &mut TestAppContext) {
        let roots = folder_roots("folder-lifetime");
        for root in &roots {
            std::fs::write(root.join("same.txt"), "text").unwrap();
        }
        let (ws, cx) = window(cx);
        compare(&ws, &roots, true, cx);
        for change in 0..5 {
            ws.update_in(cx, |ws, w, cx| {
                ws.refresh_comparison(&RefreshComparison, w, cx);
                let stop = ws.comparison.folder_stop.as_ref().unwrap().clone();
                match change {
                    0 => ws.close_panel(&ClosePanel, w, cx),
                    1 => ws.find(&Find, w, cx),
                    2 => ws.clear_window(w, cx),
                    3 => ws.load_comparison([roots[0].join("same.txt"), roots[1].join("same.txt")], w, cx),
                    _ => ws.refresh_comparison(&RefreshComparison, w, cx),
                }
                assert!(stop.load(Ordering::Relaxed));
            });
            cx.run_until_parked();
            compare(&ws, &roots, true, cx);
        }
        ws.update_in(cx, |ws, w, cx| {
            ws.refresh_comparison(&RefreshComparison, w, cx);
            let stop = ws.comparison.folder_stop.as_ref().unwrap().clone();
            let generation = ws.comparison.generation;
            let picker = ws.comparison.begin_pick();
            assert_eq!(ws.comparison.pick(picker, None, None), Pick::Cancelled);
            ws.start_comparison([roots[0].clone(), roots[1].clone()], false, w, cx);
            assert!(!stop.load(Ordering::Relaxed));
            assert_eq!(ws.comparison.generation, generation);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(folder(ws).inventory.is_some()));
        std::fs::remove_dir_all(roots[0].parent().unwrap()).unwrap();
    }

    fn apply_live(source: usize, cx: &mut TestAppContext) {
        use crate::editing::text::Selection;
        let paths = files(&format!("apply-live-{source}"), "disk left\n", "\u{feff}disk right\r\n");
        let (ws, other, mut first, mut second) = two_windows(cx);
        let target = 1 - source;
        let (destination, sibling) = other.update_in(&mut second, |ws, w, cx| {
            ws.open_path(paths[source].clone(), w, cx);
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("source expanded\nkeep\nsource tail\n");
                })
            });
            ws.open_path(paths[target].clone(), w, cx);
            let destination = ws.active_editor().unwrap();
            destination.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("target\nkeep\nunrelated unsaved\n");
                    s.set_selection(12, 17);
                })
            });
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            (destination, ws.active_editor().unwrap())
        });
        second.run_until_parked();
        for (editor, selection) in [(&destination, (12, 17)), (&sibling, (7, 11))] {
            editor.update(&mut second, |e, cx| {
                e.sel(cx).set_selection(selection.0, selection.1);
                cx.notify();
            });
        }
        let before =
            [&destination, &sibling].map(|editor| editor.read_with(&second, |e, _| e.state.selections.clone()));
        let format = destination.read_with(&second, |e, cx| (e.doc(cx).bom, e.doc(cx).line_ending));
        compare(&ws, &paths, false, &mut first);
        ws.update_in(&mut first, |ws, w, cx| {
            ws.apply_comparison(source, w, cx);
            assert_eq!(destination.read(cx).text(cx), "target\nkeep\nunrelated unsaved\n");
            ws.apply_comparison(source, w, cx);
        });
        first.run_until_parked();
        let applied = "source expanded\nkeep\nunrelated unsaved\n";
        for (editor, selection) in [(&destination, (21, 26)), (&sibling, (16, 20))] {
            editor.read_with(&second, |e, cx| {
                assert_eq!(e.text(cx), applied);
                assert!(e.doc(cx).dirty);
                assert_eq!((e.doc(cx).bom, e.doc(cx).line_ending), format);
                assert_eq!(e.state.selections, vec![Selection::new(selection.0, selection.1)]);
            });
        }
        other.update_in(&mut second, |ws, w, cx| {
            let editor = ws.active_editor().unwrap();
            assert!(editor.read(cx).same_document(destination.read(cx)));
            assert!(editor.read(cx).is_focused(w));
        });
        ws.read_with(&first, |ws, _| {
            assert_eq!(status(ws), "Applied to buffer — not saved");
            assert!(panel(ws).message.as_ref().unwrap().contains("Refresh"));
        });
        destination.update(&mut second, |e, cx| e.edit(cx, |s| s.undo()));
        second.run_until_parked();
        for (editor, before) in [&destination, &sibling].into_iter().zip(before) {
            assert_eq!(editor.read_with(&second, |e, cx| e.text(cx)), "target\nkeep\nunrelated unsaved\n");
            assert_eq!(editor.read_with(&second, |e, _| e.state.selections.clone()), before);
        }
        sibling.update(&mut second, |e, cx| e.edit(cx, |s| s.redo()));
        second.run_until_parked();
        assert_eq!(destination.read_with(&second, |e, cx| e.text(cx)), applied);
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "disk left\n");
        assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "\u{feff}disk right\r\n");
        other.update_in(&mut second, |ws, w, cx| ws.save(&Save, w, cx));
        second.run_until_parked();
        let saved = format!("{}{}", if format.0 { "\u{feff}" } else { "" }, applied.replace('\n', format.1));
        assert_eq!(std::fs::read_to_string(&paths[target]).unwrap(), saved);
    }

    #[gpui::test]
    fn apply_left_to_right_uses_unsaved_global_buffers_one_undo_and_save(cx: &mut TestAppContext) {
        apply_live(0, cx);
    }

    #[gpui::test]
    fn apply_right_to_left_uses_unsaved_global_buffers_one_undo_and_save(cx: &mut TestAppContext) {
        apply_live(1, cx);
    }

    #[gpui::test]
    fn apply_closed_destination_attaches_an_ordinary_editor_and_handles_empty_ranges(cx: &mut TestAppContext) {
        for (source, left, right, expected) in [
            (0, "😀 added\nkeep\n", "keep\n", "😀 added\nkeep\n"),
            (1, "\u{feff}keep\r\nextra\r\n", "keep\n", "keep\n"),
            (0, "", "remove\n", ""),
            (1, "", "insert at EOF", "insert at EOF"),
        ] {
            let paths = files(&format!("apply-closed-{source}-{}", left.len()), left, right);
            let (ws, vcx) = window(cx);
            vcx.simulate_resize(size(px(1200.), px(800.)));
            compare(&ws, &paths, false, vcx);
            let label = if source == 0 { "Apply Left to Right" } else { "Apply Right to Left" };
            click(vcx, label);
            vcx.run_until_parked();
            let editor = ws.update_in(vcx, |ws, w, cx| {
                let editor = ws.active_editor().unwrap();
                let e = editor.read(cx);
                assert_eq!(e.doc(cx).path.as_ref(), Some(&crate::editing::text::resolve(&paths[1 - source])));
                assert_eq!(e.text(cx), expected);
                assert!(!e.doc(cx).scratch && !e.doc(cx).read_only && e.doc(cx).dirty);
                assert!(e.is_focused(w));
                assert_eq!(e.doc(cx).disk, crate::files::watch::stamp(&paths[1 - source]));
                assert_eq!(e.doc(cx).bom, source == 1 && left.starts_with('\u{feff}'));
                assert_eq!(e.doc(cx).line_ending, if source == 1 && left.contains("\r\n") { "\r\n" } else { "\n" });
                let captured = ws.capture(w, true, &mut Texts::new(), cx);
                assert!(captured
                    .groups
                    .iter()
                    .flat_map(|g| &g.views)
                    .any(|v| v.path == crate::editing::text::resolve(&paths[1 - source])));
                editor
            });
            editor.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
            vcx.run_until_parked();
            assert_eq!(
                editor.read_with(vcx, |e, cx| e.text(cx)),
                crate::editing::text::Loaded::read(&paths[1 - source]).unwrap().rope.to_string()
            );
            editor.update(vcx, |e, cx| e.edit(cx, |s| s.redo()));
            vcx.run_until_parked();
            assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), expected);
            assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), left);
            assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), right);
        }
    }

    #[gpui::test]
    fn legacy_comparison_apply_keeps_named_disk_codec_and_failed_save_preserves_bytes(cx: &mut TestAppContext) {
        use crate::editing::encoding::Encoding;
        let paths = files("f01-legacy-comparison", "🦀\n", "");
        std::fs::write(&paths[1], b"caf\xe9\r\n").unwrap();
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            let loaded = Loaded::read_encoding(&paths[1], Some(Encoding::Windows1252)).unwrap();
            ws.add_tab(EditorState::from_loaded(&paths[1], loaded), window, cx);
            let editor = ws.active_editor().unwrap();
            ws.close_editor(&editor, window, cx);
            ws.start_comparison(paths.clone(), false, window, cx);
        });
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, window, cx| ws.apply_comparison(0, window, cx));
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).text(cx), "🦀\n");
            assert_eq!(editor.read(cx).doc(cx).encoding, Encoding::Windows1252);
            assert_eq!(editor.read(cx).doc(cx).disk_encoding, Encoding::Windows1252);
            ws.save(&Save, window, cx);
            assert!(editor.read(cx).doc(cx).unsaved());
        });
        assert_eq!(std::fs::read(&paths[1]).unwrap(), b"caf\xe9\r\n");
    }

    #[gpui::test]
    fn closed_unicode_apply_keeps_format_and_history_before_save(cx: &mut TestAppContext) {
        use crate::editing::encoding::{encode, Encoding};
        for encoding in [Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
            let paths = files(encoding.name(), "before\n", "");
            let original = encode("after🦀\r\n", encoding, true).unwrap();
            std::fs::write(&paths[1], &original).unwrap();
            let history = paths[0].parent().unwrap().canonicalize().unwrap().join("history");
            let _ = std::fs::remove_dir_all(&history);
            let (ws, vcx) = window(cx);
            ws.update_in(vcx, |ws, w, cx| {
                cx.global_mut::<Settings>().file_history = true;
                ws.start_comparison(paths.clone(), false, w, cx);
            });
            vcx.run_until_parked();
            ws.update_in(vcx, |ws, w, cx| ws.apply_comparison(0, w, cx));
            vcx.run_until_parked();
            ws.update_in(vcx, |ws, _, cx| {
                let editor = ws.active_editor().expect("destination opened for Apply");
                editor.update(cx, |editor, cx| {
                    assert_eq!(editor.doc(cx).encoding, encoding);
                    assert_eq!(editor.doc(cx).disk_encoding, encoding);
                    assert!(editor.doc(cx).bom);
                    assert_eq!(editor.doc(cx).line_ending, "\r\n");
                    assert_eq!(editor.text(cx), "before\n");
                    assert!(editor.doc(cx).dirty);
                    assert_eq!(std::fs::read(&paths[1]).unwrap(), original);
                    editor.edit(cx, |state| state.undo());
                    assert_eq!(editor.text(cx), "after🦀\n");
                    editor.edit(cx, |state| state.redo());
                    assert!(editor.doc(cx).history_enabled);
                    let blocked = paths[0].parent().unwrap().join("blocked-history");
                    std::fs::write(&blocked, b"not a directory").unwrap();
                    assert!(crate::files::file_history::with_root(blocked, || { editor.update_doc(cx, |d| d.save()) })
                        .is_err());
                    assert_eq!(std::fs::read(&paths[1]).unwrap(), original);
                    assert!(editor.doc(cx).dirty);
                    crate::files::file_history::with_root(history.clone(), || {
                        editor.update_doc(cx, |d| d.save()).unwrap()
                    });
                });
            });
            assert_eq!(std::fs::read(&paths[1]).unwrap(), encode("before\r\n", encoding, true).unwrap());
            let versions = crate::files::file_history::list_in(&history, &paths[1]).unwrap();
            assert_eq!(versions.len(), 1);
            let captured = crate::files::file_history::read_in(&history, &paths[1], &versions[0].id).unwrap();
            assert_eq!(captured.bytes, original);
            assert_eq!(captured.version.encoding, encoding);
            std::fs::remove_dir_all(paths[0].parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn bomless_live_alias_uses_disk_encoding_before_unsaved_conversion() {
        use crate::editing::encoding::{encode, Encoding};
        let paths = files("bomless-alias", "source\n", "");
        let original = encode("disk🦀\r\n", Encoding::Utf16Le, false).unwrap();
        std::fs::write(&paths[1], &original).unwrap();
        let alias = link(&paths[1], paths[1].with_extension("alias"));
        let buffer = || Buffer {
            endings: None,
            owner: Owner {
                path: alias.clone(),
                buffer: 1,
                version: 2,
                format: Format { line_ending: "\r\n", bom: true, encoding: Encoding::Utf16Be },
                disk_encoding: Encoding::Utf16Le,
                dirty: true,
                read_only: false,
                deleted: false,
                disk: crate::files::watch::stamp(&alias),
                watching: true,
            },
            rope: Rope::from_str("unsaved conversion\n"),
        };
        let stop = AtomicBool::new(false);
        let prepared = prepare(paths.clone(), vec![buffer()], HashMap::new(), &stop).unwrap();
        assert_eq!(prepared.sources[1].disk.bytes, original);
        assert_eq!(prepared.sources[1].disk.text.format.encoding, Encoding::Utf16Le);
        assert_eq!(prepared.sources[1].text().format.encoding, Encoding::Utf16Be);
        assert!(prepared.sources[1].label.contains("UTF-16 BE"));
        let hunk = &prepared.pair.hunks[0];
        let application = prepare_apply(
            &prepared.sources,
            vec![buffer()],
            Vec::new(),
            0,
            [hunk.lines.old_chars.clone(), hunk.lines.new_chars.clone()],
            &stop,
        )
        .unwrap();
        assert!(application.closed.is_none());
        assert_eq!(application.replacement, "source\n");
        assert_eq!(std::fs::read(&paths[1]).unwrap(), original);
        std::fs::remove_dir_all(paths[0].parent().unwrap()).unwrap();
    }

    #[gpui::test]
    fn apply_rejects_old_versions_and_readonly_deleted_or_ambiguous_reviewed_buffers(cx: &mut TestAppContext) {
        for operation in [
            "source-version",
            "target-version",
            "readonly",
            "deleted",
            "format",
            "disk-stamp",
            "ambiguous-source",
            "ambiguous-target",
        ] {
            let paths = files(&format!("apply-stale-{operation}"), "source\nkeep\n", "target\nkeep\n");
            let aliases = paths.each_ref().map(|path| link(path, path.with_extension("alias")));
            let (ws, vcx) = window(cx);
            let editors = ws.update_in(vcx, |ws, w, cx| paths.each_ref().map(|p| open(ws, p, w, cx)));
            compare(&ws, &paths, false, vcx);
            let expected = ws.update_in(vcx, |ws, w, cx| {
                match operation {
                    "source-version" => editors[0].update(cx, |e, cx| e.edit(cx, |s| s.insert("new source"))),
                    "target-version" => editors[1].update(cx, |e, cx| e.edit(cx, |s| s.insert("new target"))),
                    "readonly" => editors[1].update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true)),
                    "deleted" => editors[1].update(cx, |e, cx| e.update_doc(cx, |d| d.deleted = true)),
                    "format" => editors[0].update(cx, |e, cx| e.update_doc(cx, |d| d.bom = true)),
                    "disk-stamp" => editors[0].update(cx, |e, cx| e.update_doc(cx, |d| d.disk = None)),
                    _ => {
                        let alias = &aliases[usize::from(operation == "ambiguous-target")];
                        add_tab(ws, "independent alias", alias, w, cx);
                    }
                }
                let expected = editors.each_ref().map(|e| e.read(cx).text(cx));
                ws.apply_comparison(0, w, cx);
                expected
            });
            vcx.run_until_parked();
            ws.read_with(vcx, |ws, cx| {
                assert!(status(ws).starts_with("Apply rejected:"), "{operation}: {:?}", ws.status);
                if operation.starts_with("ambiguous") {
                    assert!(status(ws).contains("ambiguous owners"));
                }
                assert_eq!(editors.each_ref().map(|e| e.read(cx).text(cx)), expected, "{operation}");
            });
        }
    }

    #[gpui::test]
    fn apply_pending_validation_rejects_owner_load_lease_disk_and_newer_ui_races(cx: &mut TestAppContext) {
        for operation in [
            "source-version",
            "target-version",
            "close-source",
            "reopen-target",
            "rename-source",
            "readonly",
            "ambiguous",
            "loading",
            "restoring",
            "lease",
            "alias-lease",
            "disk-source",
            "disk-target",
            "disk-type",
            "close-panel",
            "replace-panel",
            "refresh",
            "project",
            "new-pair",
        ] {
            let paths = files(&format!("apply-race-{operation}"), "source\nkeep\n", "target\nkeep\n");
            let alias = link(&paths[1], paths[1].with_extension("alias"));
            let (ws, vcx) = window(cx);
            let editors = ws.update_in(vcx, |ws, w, cx| paths.each_ref().map(|p| open(ws, p, w, cx)));
            compare(&ws, &paths, false, vcx);
            let (expected, reservation) = ws.update_in(vcx, |ws, w, cx| {
                ws.apply_comparison(0, w, cx);
                assert!(ws.comparison.apply.is_some());
                let mut reservation = None;
                match operation {
                    "source-version" => editors[0].update(cx, |e, cx| e.edit(cx, |s| s.insert("new source"))),
                    "target-version" => editors[1].update(cx, |e, cx| e.edit(cx, |s| s.insert("new target"))),
                    "close-source" => ws.close_view(editors[0].clone(), w, cx),
                    "reopen-target" => {
                        ws.close_view(editors[1].clone(), w, cx);
                        ws.open_path(paths[1].clone(), w, cx);
                        assert_ne!(ws.active_editor().unwrap().read(cx).doc(cx).id(), editors[1].read(cx).doc(cx).id());
                    }
                    "rename-source" => editors[0]
                        .update(cx, |e, cx| e.update_doc(cx, |d| d.path = Some(paths[0].with_extension("moved")))),
                    "readonly" => editors[1].update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true)),
                    "ambiguous" => add_tab(ws, "new alias owner", &alias, w, cx),
                    "loading" => {
                        ws.loading.insert(paths[0].clone());
                    }
                    "restoring" => ws.restoring.push(restoring(&paths[1])),
                    "lease" => reservation = Some(pipelines::lease(paths[0].parent().unwrap(), cx)),
                    "alias-lease" => {
                        reservation = Some(pipelines::lease(&alias, cx));
                    }
                    "disk-source" => std::fs::write(&paths[0], "changed source\n").unwrap(),
                    "disk-target" => std::fs::write(&paths[1], "changed target\n").unwrap(),
                    "disk-type" => {
                        std::fs::remove_file(&paths[0]).unwrap();
                        std::os::unix::fs::symlink(&paths[1], &paths[0]).unwrap();
                    }
                    "close-panel" => ws.close_panel(&ClosePanel, w, cx),
                    "replace-panel" => ws.find(&Find, w, cx),
                    "refresh" => ws.refresh_comparison(&RefreshComparison, w, cx),
                    "project" => ws.clear_window(w, cx),
                    _ => ws.load_comparison([paths[1].clone(), paths[0].clone()], w, cx),
                }
                (editors.each_ref().map(|e| e.read(cx).text(cx)), reservation)
            });
            vcx.run_until_parked();
            for (editor, expected) in editors.iter().zip(expected) {
                assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), expected, "{operation}");
            }
            ws.read_with(vcx, |ws, _| {
                if !matches!(operation, "close-panel" | "replace-panel" | "refresh" | "project" | "new-pair") {
                    assert!(status(ws).starts_with("Apply rejected:"), "{operation}: {:?}", ws.status);
                }
            });
            drop(reservation);
        }
    }

    #[gpui::test]
    fn apply_pending_validation_rejects_cross_window_owner_move_and_global_busy_aliases(cx: &mut TestAppContext) {
        for operation in
            ["move-source", "move-target", "loading-alias", "restoring-alias", "leased-alias", "new-closed-owner"]
        {
            let paths = files(&format!("apply-global-{operation}"), "source\n", "target\n");
            let alias = link(&paths[1], paths[1].with_extension("alias"));
            let (ws, other, mut first, mut second) = two_windows(cx);
            let owner = other.update_in(&mut second, |ws, w, cx| {
                let path = &paths[usize::from(operation == "move-target")];
                (operation == "move-source" || operation == "move-target").then(|| open(ws, path, w, cx))
            });
            compare(&ws, &paths, false, &mut first);
            let reservation = ws.update_in(&mut first, |ws, w, cx| {
                ws.apply_comparison(0, w, cx);
                match operation {
                    "move-source" | "move-target" => {
                        let released =
                            other.update(cx, |other, cx| other.release_views(&[owner.as_ref().unwrap().clone()], cx));
                        ws.adopt_views(released, 0, 0, owner.clone(), w, cx);
                    }
                    "loading-alias" => {
                        other.update(cx, |ws, _| ws.loading.insert(alias.clone()));
                    }
                    "restoring-alias" => other.update(cx, |ws, _| ws.restoring.push(restoring(&alias))),
                    "new-closed-owner" => add_tab(ws, "appeared owner", &alias, w, cx),
                    _ => return Some(pipelines::lease(&alias, cx)),
                }
                None
            });
            first.run_until_parked();
            ws.read_with(&first, |ws, cx| {
                assert!(status(ws).starts_with("Apply rejected:"), "{operation}: {:?}", ws.status);
                assert!(ws
                    .open_editors(cx)
                    .iter()
                    .all(|e| e.read(cx).text(cx) != "source\n" || e.read(cx).doc(cx).path.as_ref() != Some(&paths[1])));
            });
            if let Some(owner) = owner {
                let expected = if operation == "move-source" { "source\n" } else { "target\n" };
                assert_eq!(owner.read_with(&second, |e, cx| e.text(cx)), expected);
            }
            assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "target\n");
            drop(reservation);
        }
    }

    #[gpui::test]
    fn apply_allows_unrelated_buffer_edits_and_retains_newer_picker_while_pending(cx: &mut TestAppContext) {
        let paths = files("apply-unrelated", "source\nkeep\n", "target\nkeep\n");
        let unrelated_path = paths[0].with_extension("unrelated");
        std::fs::write(&unrelated_path, "unrelated file\n").unwrap();
        let (ws, vcx) = window(cx);
        let unrelated = ws.update_in(vcx, |ws, w, cx| open(ws, &unrelated_path, w, cx));
        compare(&ws, &paths, false, vcx);
        let picker = ws.update_in(vcx, |ws, w, cx| {
            ws.apply_comparison(0, w, cx);
            unrelated.update(cx, |e, cx| e.edit(cx, |s| s.insert("unrelated edit")));
            ws.comparison.begin_pick()
        });
        vcx.run_until_parked();
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "source\nkeep\n");
            assert_eq!(unrelated.read(cx).text(cx), "unrelated editunrelated file\n");
            assert_eq!(ws.comparison.pick(picker, None, Some(paths[0].clone())), Pick::Right(paths[0].clone()));
            assert_eq!(status(ws), "Applied to buffer — not saved");
        });
    }

    #[gpui::test]
    fn apply_rejects_existing_global_alias_load_restore_and_write_reservations(cx: &mut TestAppContext) {
        for operation in ["loading", "restoring", "lease", "ancestor-lease"] {
            let paths = files(&format!("apply-existing-busy-{operation}"), "source\n", "target\n");
            let alias = link(&paths[1], paths[1].with_extension("alias"));
            let (ws, other, mut first, _) = two_windows(cx);
            compare(&ws, &paths, false, &mut first);
            let reservation = other.update(cx, |ws, cx| {
                match operation {
                    "loading" => {
                        ws.loading.insert(alias.clone());
                    }
                    "restoring" => ws.restoring.push(restoring(&alias)),
                    "lease" => return Some(pipelines::lease(&alias, cx)),
                    _ => return Some(pipelines::lease(paths[1].parent().unwrap(), cx)),
                }
                None
            });
            ws.update_in(&mut first, |ws, w, cx| ws.apply_comparison(0, w, cx));
            first.run_until_parked();
            ws.read_with(&first, |ws, _| {
                assert!(status(ws).contains("reserved for writing"), "{operation}: {:?}", ws.status);
                assert_eq!(ws.views.tabs().count(), 1, "rejection must not attach a destination");
            });
            assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "target\n");
            drop(reservation);
        }
    }

    #[gpui::test]
    fn apply_readonly_destination_rejects_but_readonly_source_and_unique_aliases_work(cx: &mut TestAppContext) {
        for operation in ["readonly-buffer", "readonly-disk", "readonly-source", "unique-alias"] {
            let paths = files(&format!("apply-readonly-{operation}"), "source\n", "target\n");
            let aliases = paths.each_ref().map(|path| link(path, path.with_extension("alias")));
            let original_permissions = std::fs::metadata(&paths[1]).unwrap().permissions();
            if operation == "readonly-disk" {
                let mut readonly = original_permissions.clone();
                readonly.set_readonly(true);
                std::fs::set_permissions(&paths[1], readonly).unwrap();
            }
            let (ws, vcx) = window(cx);
            let owner = ws.update_in(vcx, |ws, w, cx| {
                if operation == "readonly-buffer" || operation == "readonly-source" {
                    let side = usize::from(operation == "readonly-buffer");
                    let editor = open(ws, &paths[side], w, cx);
                    editor.update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
                    Some(editor)
                } else if operation == "unique-alias" {
                    ws.open_path(aliases[0].clone(), w, cx);
                    ws.with_active(cx, |e, cx| {
                        e.edit(cx, |s| {
                            s.select_all();
                            s.insert("alias unsaved source\n");
                        })
                    });
                    Some(open(ws, &aliases[1], w, cx))
                } else {
                    None
                }
            });
            compare(&ws, &paths, false, vcx);
            ws.update_in(vcx, |ws, w, cx| ws.apply_left_to_right(&ApplyLeftToRight, w, cx));
            vcx.run_until_parked();
            if operation == "readonly-disk" {
                std::fs::set_permissions(&paths[1], original_permissions).unwrap();
            }
            ws.read_with(vcx, |ws, cx| {
                if operation.starts_with("readonly-") && operation != "readonly-source" {
                    assert!(status(ws).contains("destination is read-only"), "{operation}: {:?}", ws.status);
                    assert_eq!(ws.views.tabs().count(), 1);
                } else {
                    let editor = ws.active_editor().unwrap();
                    assert_eq!(
                        editor.read(cx).text(cx),
                        if operation == "unique-alias" { "alias unsaved source\n" } else { "source\n" }
                    );
                    if operation == "unique-alias" {
                        assert_eq!(owner.as_ref(), Some(&editor));
                        assert_eq!(
                            editor.read(cx).doc(cx).path.as_ref(),
                            Some(&crate::editing::text::resolve(&aliases[1]))
                        );
                    }
                }
            });
            assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "target\n");
        }
    }

    #[gpui::test]
    fn apply_cannot_treat_global_alias_metadata_errors_as_absence(cx: &mut TestAppContext) {
        for operation in ["owner", "loading", "lease"] {
            let paths = files(&format!("apply-unknown-alias-{operation}"), "source\n", "target\n");
            let alias = paths[0].with_extension("loop");
            let _ = std::fs::remove_file(&alias);
            std::os::unix::fs::symlink(&alias, &alias).unwrap();
            let (ws, vcx) = window(cx);
            ws.update_in(vcx, |ws, w, cx| {
                if operation == "owner" {
                    add_tab(ws, "unresolved owner", &alias, w, cx);
                }
                ws.start_comparison(paths.clone(), false, w, cx);
            });
            vcx.run_until_parked();
            let reservation = ws.update_in(vcx, |ws, w, cx| {
                let reservation = match operation {
                    "loading" => {
                        ws.loading.insert(alias.clone());
                        None
                    }
                    "lease" => Some(pipelines::lease(&alias, cx)),
                    _ => None,
                };
                ws.apply_comparison(0, w, cx);
                reservation
            });
            vcx.run_until_parked();
            ws.read_with(vcx, |ws, _| {
                assert!(status(ws).contains("Cannot validate"), "{operation}: {:?}", ws.status);
                assert_eq!(ws.views.tabs().count(), if operation == "owner" { 2 } else { 1 });
            });
            assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), "target\n");
            drop(reservation);
        }
    }

    #[gpui::test]
    fn apply_closed_destination_uses_ordinary_path_identity_on_reopen(cx: &mut TestAppContext) {
        let paths = files("apply-closed-reopen", "source\n", "target\n");
        let alias = paths[0].parent().unwrap().with_extension("alias");
        let _ = std::fs::remove_file(&alias);
        std::os::unix::fs::symlink(paths[0].parent().unwrap(), &alias).unwrap();
        let selected = [alias.join("left.txt"), alias.join("right.txt")];
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, w, cx| ws.start_comparison(selected, false, w, cx));
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, w, cx| ws.apply_comparison(0, w, cx));
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, w, cx| {
            let destination = ws.active_editor().unwrap();
            ws.open_path(paths[1].clone(), w, cx);
            assert_eq!(ws.active_editor().as_ref(), Some(&destination));
            assert_eq!(destination.read(cx).text(cx), "source\n");
        });
    }

    #[gpui::test]
    fn file_pair_preserves_tabs_and_uses_readonly_snapshots(cx: &mut TestAppContext) {
        let paths = files("readonly", "old a old\nkeep\n", "new a new\nkeep\nextra\n");
        let (ws, cx) = window(cx);
        let tab = ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("ordinary document")));
            ws.active_editor().unwrap()
        });
        compare(&ws, &paths, false, cx);
        ws.read_with(cx, |ws, cx| {
            let p = panel(ws);
            assert_eq!(ws.views.tabs().count(), 1);
            assert_eq!(ws.active_editor().unwrap(), tab);
            assert_eq!(p.paths, paths);
            let r = p.review.as_ref().unwrap();
            assert_eq!(r.pair.hunks.len(), 2);
            for e in &r.viewers {
                assert!(e.read(cx).doc(cx).read_only && e.read(cx).doc(cx).scratch);
            }
            assert_eq!(p.counter(), "Difference 1 of 2");
        });
        cx.simulate_keystrokes("right");
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(review(ws).viewers[0].read(cx).text(cx), "old a old\nkeep\n");
            let state = ws.capture(w, true, &mut Texts::new(), cx);
            assert_eq!(state.groups.iter().map(|g| g.views.len()).sum::<usize>(), 1);
        });
        ws.update_in(cx, |ws, w, cx| {
            ws.next_difference(&NextDifference, w, cx);
            let p = panel(ws);
            assert!(p.range_label(0).contains("Empty"));
            assert!(p.range_label(0).contains("EOF"));
            assert_eq!(p.counter(), "Difference 2 of 2");
            ws.previous_difference(&PreviousDifference, w, cx);
            assert_eq!(ws.panel_place(), Some(&ws.panel_places.changes));
        });
    }

    #[gpui::test]
    fn a_file_pair_is_aligned_scrolls_as_one_and_next_difference_keeps_it(cx: &mut TestAppContext) {
        use gpui::{point, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};
        // line 10 becomes three lines on the right, line 60 is gone there
        let left: String = (0..120).map(|i| format!("line {i}\n")).collect();
        let right: String = (0..120)
            .map(|i| match i {
                10 => "new a\nnew b\nnew c\n".to_string(),
                60 => String::new(),
                _ => format!("line {i}\n"),
            })
            .collect();
        let paths = files("aligned", &left, &right);
        let (ws, cx) = window(cx);
        compare(&ws, &paths, false, cx);
        let viewers = ws.read_with(cx, |ws, _| review(ws).viewers.clone());
        let blocks = viewers.each_ref().map(|v| v.read_with(cx, |e, _| e.fillers().blocks()));
        assert_eq!(blocks, [vec![(11, 2)], vec![(62, 1)]], "padded at each hunk's end");
        let rows = viewers.each_ref().map(|v| v.read_with(cx, |e, cx| e.display_rows(cx)));
        assert_eq!(rows[0], rows[1]);
        // the wheel over the left side scrolls both to one display row
        let (bounds, _, _) = viewers[0].read_with(cx, |e, _| e.drawn_frame().unwrap());
        cx.simulate_event(ScrollWheelEvent {
            position: bounds.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-15. * 70.))),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();
        let tops =
            |cx: &mut gpui::VisualTestContext| viewers.each_ref().map(|v| v.read_with(cx, |e, _| e.scroll_rows().0));
        let [a, b] = tops(cx);
        assert!(a > 60., "scrolled past the second hunk: {a}");
        assert_eq!(a, b);
        // Next Difference takes both views to one scroll; a changed block's carets are on one
        // row, and a deletion's empty side has its caret just below the filler row it is
        // padded with there (a caret is never on a filler row)
        let carets =
            |cx: &mut gpui::VisualTestContext| viewers.each_ref().map(|v| v.read_with(cx, |e, cx| e.caret_row(cx)));
        assert_eq!(carets(cx), [10, 10], "the first difference, from loading");
        ws.update_in(cx, |ws, w, cx| ws.next_difference(&NextDifference, w, cx));
        cx.run_until_parked();
        assert_eq!(carets(cx), [62, 63], "line 60, removed; the right's gap is row 62");
        let [a, b] = tops(cx);
        assert_eq!(a, b);
        ws.update_in(cx, |ws, w, cx| ws.next_difference(&NextDifference, w, cx));
        cx.run_until_parked();
        assert_eq!(carets(cx), [10, 10], "round to the first again");
        let [a, b] = tops(cx);
        assert_eq!(a, b);
    }

    #[gpui::test]
    fn snapshot_input_attempts_independently_preserve_both_reviewed_texts(cx: &mut TestAppContext) {
        use gpui::{ClipboardItem, EntityInputHandler};
        let paths = files("immutable-input", "left\n", "right\n");
        let (ws, cx) = window(cx);
        let mut observed = Vec::new();
        let mut expected = Vec::new();
        for side in 0..2 {
            for attempt in ["insert", "backspace", "delete", "paste", "compose", "commit", "command", "cut"] {
                compare(&ws, &paths, false, cx);
                let viewer = ws.update_in(cx, |ws, w, cx| {
                    let viewer = review(ws).viewers[side].clone();
                    viewer.update(cx, |e, cx| {
                        e.select_range(1..1, cx);
                        e.focus(w);
                    });
                    viewer
                });
                match attempt {
                    "insert" => cx.simulate_keystrokes("x"),
                    "backspace" => cx.simulate_keystrokes("backspace"),
                    "delete" => {
                        viewer.update_in(cx, |e, w, cx| {
                            e.perform_action(&editor::Delete, w, cx);
                        });
                    }
                    "paste" => {
                        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("pasted".into())));
                        cx.simulate_keystrokes("cmd-v");
                    }
                    "compose" => viewer.update_in(cx, |e, w, cx| {
                        e.replace_and_mark_text_in_range(Some(0..1), "仮", Some(0..1), w, cx)
                    }),
                    "commit" => viewer.update_in(cx, |e, w, cx| e.replace_text_in_range(Some(0..1), "確", w, cx)),
                    "command" => {
                        viewer.update_in(cx, |e, w, cx| {
                            e.perform_action(&editor::DuplicateLine, w, cx);
                        });
                    }
                    "cut" => {
                        viewer.update_in(cx, |e, w, cx| {
                            e.perform_action(&editor::Cut, w, cx);
                        });
                    }
                    _ => unreachable!(),
                }
                observed.push((side, attempt, viewer.read_with(cx, |e, cx| e.text(cx))));
                expected.push((side, attempt, if side == 0 { "left\n" } else { "right\n" }.to_string()));
            }
            let viewer = ws.read_with(cx, |ws, _| review(ws).viewers[side].clone());
            viewer.update_in(cx, |e, w, cx| {
                e.select_range(0..0, cx);
                e.focus(w);
            });
            cx.simulate_keystrokes("right");
            viewer.read_with(cx, |e, _| assert_eq!(e.state.cursor(), 1));
            cx.simulate_keystrokes("cmd-a cmd-c");
            viewer.read_with(cx, |e, _| assert_eq!(e.state.selections[0].range(), 0..if side == 0 { 5 } else { 6 }));
            cx.update(|_, cx| {
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().as_deref(),
                    Some(if side == 0 { "left\n" } else { "right\n" })
                )
            });
        }
        assert_eq!(observed, expected, "each attempt starts with a fresh snapshot");
        ws.update_in(cx, |ws, w, cx| ws.apply_comparison(1, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "right\n"));
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "left\n");
    }

    #[gpui::test]
    fn inspection_only_one_sided_rows_do_not_offer_copy(cx: &mut TestAppContext) {
        for side in 0..2 {
            let roots = folder_roots(&format!("inspection-copy-{side}"));
            std::fs::write(roots[side].join("binary"), b"a\0b").unwrap();
            std::fs::write(roots[side].join("invalid"), b"a\xffb").unwrap();
            let (ws, cx) = window(cx);
            cx.simulate_resize(size(px(1200.), px(900.)));
            compare(&ws, &roots, true, cx);
            for (index, reason) in [(0, "NUL-binary"), (1, "Malformed or unknown encoding")] {
                ws.update_in(cx, |ws, w, cx| ws.select_folder_row(index, w, cx));
                cx.run_until_parked();
                ws.read_with(cx, |ws, _| {
                    let (p, f) = (panel(ws), folder(ws));
                    let row = &f.inventory.as_ref().unwrap().rows[index];
                    assert_eq!(
                        row.status,
                        if side == 0 { folders::Status::LeftOnly } else { folders::Status::RightOnly }
                    );
                    assert!(row.note.contains(reason) && row.note.contains("inspection only"));
                    assert!(f.visible.contains(&index));
                    assert!(p.message.as_ref().unwrap().contains(reason));
                    assert!(p.copy_candidate().is_none());
                });
                assert!(cx.debug_bounds(["comparison-folder-row-0", "comparison-folder-row-1"][index]).is_some());
                assert!(cx.debug_bounds("comparison-copy-file").is_none());
            }
        }
    }

    #[gpui::test]
    fn invalid_and_same_underlying_files_preserve_existing_panel(cx: &mut TestAppContext) {
        let paths = files("invalid", "left", "right");
        let alias = link(&paths[0], paths[0].with_extension("alias"));
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.find(&Find, w, cx));
        for invalid in [paths[0].parent().unwrap().to_path_buf(), alias] {
            ws.update_in(cx, |ws, w, cx| ws.start_comparison([paths[0].clone(), invalid], false, w, cx));
            cx.run_until_parked();
            ws.read_with(cx, |ws, _| {
                assert!(matches!(ws.panel, Some(Panel::Find(_))));
                assert!(status(ws).contains("Comparison:"));
            });
        }
    }

    #[gpui::test]
    fn refresh_close_replace_and_project_cancel_old_publication(cx: &mut TestAppContext) {
        let paths = files("cancel", "left", "right");
        let (ws, cx) = window(cx);
        for operation in 0..4 {
            ws.update_in(cx, |ws, w, cx| {
                ws.start_comparison(paths.clone(), false, w, cx);
                let stop = ws.comparison.validation.as_ref().unwrap().clone();
                match operation {
                    0 => ws.start_comparison([paths[1].clone(), paths[0].clone()], false, w, cx),
                    1 => ws.close_panel(&ClosePanel, w, cx),
                    2 => ws.find(&Find, w, cx),
                    _ => ws.clear_window(w, cx),
                }
                assert!(stop.load(Ordering::Relaxed));
            });
            cx.run_until_parked();
            ws.read_with(cx, |ws, _| match operation {
                0 => assert!(matches!(&ws.panel, Some(Panel::Comparison(p)) if p.paths[0] == paths[1])),
                2 | 3 => assert!(matches!(ws.panel, Some(Panel::Find(_)))),
                _ => assert!(ws.panel.is_none()),
            });
        }
    }

    #[test]
    fn picker_cancellation_and_expired_native_completion_do_not_adopt_paths() {
        let mut state = State::default();
        let generation = state.begin_pick();
        assert_eq!(state.pick(generation, None, None), Pick::Cancelled);
        assert_eq!(state.pick(generation, None, Some("left".into())), Pick::Right("left".into()));
        assert_eq!(state.pick(generation, Some("left".into()), None), Pick::Cancelled);
        assert_eq!(
            state.pick(generation, Some("left".into()), Some("right".into())),
            Pick::Pair(["left".into(), "right".into()])
        );
        let stop = Arc::new(AtomicBool::new(false));
        state.stop = Some(stop.clone());
        let validation = Arc::new(AtomicBool::new(false));
        state.validation = Some(validation.clone());
        state.begin_pick();
        assert!(
            !validation.load(Ordering::Relaxed),
            "opening/cancelling a picker preserves current path validation too"
        );
        assert!(
            !stop.load(Ordering::Relaxed),
            "opening/cancelling another picker leaves the current comparison work alone"
        );
        assert_eq!(state.pick(generation, Some("left".into()), Some("right".into())), Pick::Expired);
        let generation = state.begin_pick();
        state.cancel();
        assert!(stop.load(Ordering::Relaxed));
        assert!(validation.load(Ordering::Relaxed));
        assert_eq!(state.pick(generation, None, Some("late".into())), Pick::Expired);
    }
    #[gpui::test]
    fn rendered_separated_backgrounds_labels_and_focus_are_real(cx: &mut TestAppContext) {
        let paths = files("render", "one cat and dog\nkeep\n", "one fox and hen\nkeep\n");
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1200.), px(800.)));
        compare(&ws, &paths, false, cx);
        let (viewers, labels) = ws.update_in(cx, |ws, w, cx| {
            let r = review(ws);
            assert!(r.viewers[0].read(cx).is_focused(w));
            (r.viewers.clone(), r.sources.iter().map(|s| s.label.clone()).collect::<Vec<_>>())
        });
        for side in 0..2 {
            assert!(cx
                .debug_bounds(Box::leak(
                    format!("{}: {}", if side == 0 { "Left" } else { "Right" }, paths[side].display()).into_boxed_str()
                ))
                .is_some());
            assert!(cx.debug_bounds(Box::leak(labels[side].clone().into_boxed_str())).is_some());
            viewers[side].read_with(cx, |e, _| {
                let drawn = e.comparison_backgrounds();
                let spans: Vec<_> = drawn.iter().filter(|q| q.3).collect();
                assert_eq!(spans.iter().map(|q| q.0.clone()).collect::<Vec<_>>(), [4..7, 12..15]);
                let (_, left, space) = e.drawn_frame().unwrap();
                for (span, bounds, color, _) in spans {
                    assert!((f32::from(bounds.origin.x - left - space * span.start as f32)).abs() < 0.1);
                    assert!((f32::from(bounds.size.width - space * span.len() as f32)).abs() < 0.1);
                    assert!(bounds.size.height > px(0.));
                    assert_eq!(*color, theme::ca(if side == 0 { 0xec5f6766 } else { 0x99c79466 }));
                }
                let line = drawn.iter().find(|q| !q.3).unwrap();
                assert!(line.1.size.width > space * 15.);
                assert_eq!(line.2, theme::ca(if side == 0 { 0xec5f6724 } else { 0x99c79424 }));
            });
        }
        assert!(cx.debug_bounds("comparison-counter").is_some());
        ws.update_in(cx, |ws, w, cx| {
            ws.toggle_panel(&TogglePanel, w, cx);
            assert!(ws.active_editor().unwrap().read(cx).is_focused(w));
            ws.toggle_panel(&TogglePanel, w, cx);
            assert!(viewers[0].read(cx).is_focused(w));
            ws.toggle_collapse_panel(&ToggleCollapsePanel, w, cx);
            assert!(ws.active_editor().unwrap().read(cx).is_focused(w));
            ws.focus_panel(&FocusPanel, w, cx);
            assert!(viewers[0].read(cx).is_focused(w));
            ws.move_panel(&MovePanel { dock: session::Dock::Right }, w, cx);
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("comparison-refresh").is_some());
    }

    #[gpui::test]
    fn unique_dirty_global_buffer_and_siblings_win_over_disk_but_ambiguity_is_visible(cx: &mut TestAppContext) {
        let paths = files("global", "disk", "other");
        let (ws, other, mut first, mut second) = two_windows(cx);
        other.update_in(&mut second, |ws, w, cx| {
            ws.open_path(paths[0].clone(), w, cx);
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("unsaved source");
                })
            });
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
        });
        compare(&ws, &paths, false, &mut first);
        ws.read_with(&first, |ws, cx| {
            let r = review(ws);
            assert_eq!(r.viewers[0].read(cx).text(cx), "unsaved source");
            assert!(r.sources[0].label.starts_with("Live buffer · unsaved"));
            assert_eq!(ws.views.tabs().count(), 1);
        });
        let alias = link(&paths[0], paths[0].with_extension("hardlink"));
        // a buffer of its own, at a hard link to the same file
        other.update_in(&mut second, |ws, w, cx| add_tab(ws, "independent", &alias, w, cx));
        ws.update_in(&mut first, |ws, w, cx| ws.refresh_comparison(&RefreshComparison, w, cx));
        first.run_until_parked();
        ws.read_with(&first, |ws, _| {
            let p = panel(ws);
            assert!(p.review.is_none());
            assert!(p.message.as_ref().unwrap().contains("ambiguous"));
        });
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "disk");
    }

    #[gpui::test]
    fn format_only_and_identical_pairs_have_truthful_visible_labels(cx: &mut TestAppContext) {
        let paths = files("formats", "\u{feff}same\r\n", "same\n");
        let (ws, cx) = window(cx);
        compare(&ws, &paths, false, cx);
        let labels = ws.read_with(cx, |ws, _| {
            let p = panel(ws);
            assert_eq!(p.counter(), "No text differences — formats differ");
            let r = p.review.as_ref().unwrap();
            assert!(r.sources[0].label.contains("UTF-8 · CRLF · BOM"));
            assert!(r.sources[1].label.contains("UTF-8 · LF · no BOM"));
            r.sources.iter().map(|s| s.label.clone()).collect::<Vec<_>>()
        });
        for label in labels {
            assert!(cx.debug_bounds(Box::leak(label.into_boxed_str())).is_some());
        }
        std::fs::write(&paths[0], "same\n").unwrap();
        ws.update_in(cx, |ws, w, cx| ws.refresh_comparison(&RefreshComparison, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert_eq!(panel(ws).counter(), "No text or format differences");
        });
    }

    #[gpui::test]
    fn repeated_refresh_and_source_changes_during_load_expire_old_reviews(cx: &mut TestAppContext) {
        let paths = files("refresh", "left", "right");
        let (ws, cx) = window(cx);
        compare(&ws, &paths, false, cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.refresh_comparison(&RefreshComparison, w, cx);
            let stop = ws.comparison.stop.as_ref().unwrap().clone();
            ws.refresh_comparison(&RefreshComparison, w, cx);
            assert!(stop.load(Ordering::Relaxed));
            ws.close_panel(&ClosePanel, w, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.panel.is_none()));
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(paths[0].clone(), w, cx);
            // straight to the second worker step, so the edit always comes after the capture
            ws.load_comparison(paths, w, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("changed")));
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let p = panel(ws);
            assert!(p.review.is_none());
            assert!(p.message.as_ref().unwrap().contains("Open buffers changed"));
        });
    }

    #[gpui::test]
    fn commands_are_registered_in_tools_palette_and_custom_bindings(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.read_with(cx, |ws, cx| {
            for command in ["CompareFiles", "CompareFolders", "ApplyLeftToRight", "ApplyRightToLeft", "RefreshComparison", "NextDifference", "PreviousDifference"] {
                ws.validate_command(&crate::config::customization::Command { command: format!("workspace::{command}"), args: serde_json::json!({}) }, cx).unwrap();
            }
            let tools = ws.menus(cx).into_iter().find(|m| m.name == "Tools").unwrap();
            let compare = tools.items.iter().find_map(|i| if let MenuItem::Submenu(m) = i { (m.name == "Compare").then_some(m) } else { None }).unwrap();
            assert!(matches!(&compare.items[0], MenuItem::Action { name, action, .. } if name == "Compare Files…" && action.as_any().is::<CompareFiles>()));
            let items = quick_panel::command_items();
            assert!(items.iter().any(|i| matches!(&i.action, ItemAction::Dispatch(a) if a.as_any().is::<CompareFiles>())));
            assert!(items.iter().any(|i| matches!(&i.action, ItemAction::Dispatch(a) if a.as_any().is::<ApplyLeftToRight>())));
            assert!(items.iter().any(|i| matches!(&i.action, ItemAction::Dispatch(a) if a.as_any().is::<ApplyRightToLeft>())));
        });
        for left in [true, false] {
            let options = picker_options(false, left);
            assert!(options.files && !options.directories && !options.multiple);
            assert!(options.prompt.unwrap().contains(if left { "Left" } else { "Right" }));
        }
    }
    #[gpui::test]
    fn added_blank_lines_have_a_visible_full_line_background(cx: &mut TestAppContext) {
        let paths = files("blank", "keep\n", "\nkeep\n");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.start_comparison(paths, false, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let r = review(ws);
            assert!(r.viewers[1]
                .read(cx)
                .comparison_backgrounds()
                .iter()
                .any(|q| !q.3 && q.0 == (0..1) && q.1.size.width > px(0.)));
        });
    }
    #[gpui::test]
    fn a_new_owner_during_disk_preparation_requires_refresh(cx: &mut TestAppContext) {
        let paths = files("late-owner", "left", "right");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.load_comparison(paths.clone(), w, cx);
            add_tab(ws, "new unsaved owner", &paths[0], w, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let p = panel(ws);
            assert!(p.review.is_none());
            assert!(p.message.as_ref().unwrap().contains("Open buffers changed"));
        });
    }
    #[gpui::test]
    fn closing_terminal_while_validating_a_pair_cancels_the_old_request(cx: &mut TestAppContext) {
        let paths = files("close-terminal", "left", "right");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.show_terminal(&ShowTerminal, w, cx);
            ws.start_comparison(paths, false, w, cx);
            let stop = ws.comparison.validation.as_ref().unwrap().clone();
            ws.close_terminal(&CloseTerminal, w, cx);
            assert!(stop.load(Ordering::Relaxed));
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.panel.is_none()));
    }
    #[gpui::test]
    fn invalid_new_pair_preserves_the_ready_comparison_and_navigation(cx: &mut TestAppContext) {
        let paths = files("keep-comparison", "left\nkeep\n", "right\nkeep\nextra\n");
        let (ws, cx) = window(cx);
        compare(&ws, &paths, false, cx);
        let viewer = ws.read_with(cx, |ws, _| review(ws).viewers[0].clone());
        ws.update_in(cx, |ws, w, cx| {
            ws.start_comparison([paths[0].clone(), paths[0].parent().unwrap().into()], false, w, cx)
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(review(ws).viewers[0], viewer);
            assert!(panel(ws).message.is_none(), "invalid selection must leave the previous review usable");
            ws.next_difference(&NextDifference, w, cx);
            assert_eq!(panel(ws).counter(), "Difference 2 of 2");
        });
    }
    #[gpui::test]
    fn many_inline_spans_in_a_coarse_hunk_only_visit_visible_ranges(cx: &mut TestAppContext) {
        let left: String = (0..20_000).map(|i| format!("old line {i}\n")).collect();
        let right: String = (0..20_000).map(|i| format!("new line {i}\n")).collect();
        let paths = files("many-spans", &left, &right);
        let (ws, cx) = window(cx);
        cx.simulate_resize(size(px(1200.), px(800.)));
        ws.update_in(cx, |ws, w, cx| {
            ws.load_comparison(paths, w, cx);
            let p = panel(ws);
            assert!(p.review.is_none(), "read/diff/range preparation is deferred to the worker");
            assert_eq!(p.message.as_deref(), Some("Loading comparison…"));
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            let r = review(ws);
            assert_eq!(r.pair.hunks.len(), 1, ">500 line edits retain the coarse line hunk");
            for viewer in &r.viewers {
                assert!(viewer.read(cx).comparison.as_ref().unwrap().inline.len() >= 20_000);
                viewer.update(cx, |e, cx| e.goto_line(19_000, cx));
            }
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            for viewer in &review(ws).viewers {
                let e = viewer.read(cx);
                eprintln!(
                    "comparison paint: {} inline ranges, {} lookup/visible-range visits, {} rectangles",
                    e.comparison.as_ref().unwrap().inline.len(),
                    e.comparison_range_visits(),
                    e.comparison_backgrounds().len()
                );
                assert!(e.comparison_range_visits() > 0);
                assert!(
                    e.comparison_range_visits() < 1_000,
                    "lookup visits scale with visible rows and log(range count)"
                );
                let deep = e.doc(cx).rope.line_to_char(18_000);
                assert!(e.comparison_backgrounds().iter().any(|q| q.3 && q.0.start > deep));
            }
        });
    }
    #[gpui::test]
    fn text_preparation_cannot_publish_after_close_replace_refresh_or_project(cx: &mut TestAppContext) {
        let paths = files("text-lifecycle", "left", "right");
        let (ws, cx) = window(cx);
        for operation in 0..4 {
            ws.update_in(cx, |ws, w, cx| {
                ws.load_comparison(paths.clone(), w, cx);
                let stop = ws.comparison.stop.as_ref().unwrap().clone();
                match operation {
                    0 => ws.close_panel(&ClosePanel, w, cx),
                    1 => ws.find(&Find, w, cx),
                    2 => ws.refresh_comparison(&RefreshComparison, w, cx),
                    _ => ws.clear_window(w, cx),
                }
                assert!(stop.load(Ordering::Relaxed));
            });
            cx.run_until_parked();
            ws.read_with(cx, |ws, _| match operation {
                1 => assert!(matches!(ws.panel, Some(Panel::Find(_)))),
                2 => assert!(
                    matches!(&ws.panel, Some(Panel::Comparison(p)) if p.review.is_some() && p.message.is_none())
                ),
                _ => assert!(ws.panel.is_none()),
            });
        }
    }
    #[gpui::test]
    fn picker_cancellation_leaves_pending_refresh_running(cx: &mut TestAppContext) {
        let paths = files("picker-refresh", "left", "right");
        let (ws, cx) = window(cx);
        compare(&ws, &paths, false, cx);
        for right_picker in [false, true] {
            let refreshed = format!("refreshed {right_picker}");
            std::fs::write(&paths[1], &refreshed).unwrap();
            ws.update_in(cx, |ws, w, cx| {
                ws.refresh_comparison(&RefreshComparison, w, cx);
                let generation = ws.comparison.generation;
                let stop = ws.comparison.stop.as_ref().unwrap().clone();
                let picker = ws.comparison.begin_pick();
                let left = if right_picker {
                    assert_eq!(ws.comparison.pick(picker, None, Some(paths[0].clone())), Pick::Right(paths[0].clone()));
                    Some(paths[0].clone())
                } else {
                    None
                };
                assert_eq!(ws.comparison.pick(picker, left, None), Pick::Cancelled);
                assert_eq!(ws.comparison.generation, generation);
                assert!(
                    !stop.load(Ordering::Relaxed),
                    "cancelling either native picker leaves the pending Refresh running"
                );
                assert!(active(&ws.comparison.stop, &stop));
                assert_eq!(panel(ws).message.as_deref(), Some("Loading comparison…"));
            });
            cx.run_until_parked();
            ws.read_with(cx, |ws, cx| {
                assert!(panel(ws).message.is_none());
                assert_eq!(review(ws).viewers[1].read(cx).text(cx), refreshed);
            });
        }
    }
    #[gpui::test]
    fn invalid_or_same_file_replacement_keeps_pending_refresh_running(cx: &mut TestAppContext) {
        let paths = files("invalid-refresh", "left", "right");
        let (ws, cx) = window(cx);
        compare(&ws, &paths, false, cx);
        for invalid in [paths[0].parent().unwrap().to_path_buf(), paths[0].clone()] {
            let refreshed = format!("refreshed {}", invalid.display());
            std::fs::write(&paths[1], &refreshed).unwrap();
            ws.update_in(cx, |ws, w, cx| {
                ws.refresh_comparison(&RefreshComparison, w, cx);
                let generation = ws.comparison.generation;
                let stop = ws.comparison.stop.as_ref().unwrap().clone();
                ws.start_comparison([paths[0].clone(), invalid], false, w, cx);
                assert_eq!(ws.comparison.generation, generation);
                assert!(!stop.load(Ordering::Relaxed), "invalid choices must preserve the accepted pending Refresh");
                assert!(active(&ws.comparison.stop, &stop));
            });
            cx.run_until_parked();
            ws.read_with(cx, |ws, cx| {
                let p = panel(ws);
                assert_eq!(p.paths, paths);
                assert!(p.message.is_none());
                assert_eq!(review(ws).viewers[1].read(cx).text(cx), refreshed);
            });
        }
    }
    #[gpui::test]
    fn older_preflight_promotion_preserves_a_newer_native_picker(cx: &mut TestAppContext) {
        let first = files("review-promotion-first", "first left", "first right");
        let newer = files("review-promotion-newer", "new left", "new right");
        let (ws, cx) = window(cx);
        let picker = ws.update_in(cx, |ws, w, cx| {
            ws.start_comparison(first.clone(), false, w, cx);
            assert!(ws.comparison.validation.is_some());
            ws.comparison.begin_pick()
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let p = panel(ws);
            assert_eq!(p.paths, first);
            assert!(p.review.is_some());
            assert_eq!(ws.comparison.pick(picker, None, Some(newer[0].clone())), Pick::Right(newer[0].clone()));
            let selection = ws.comparison.pick(picker, Some(newer[0].clone()), Some(newer[1].clone()));
            assert_eq!(selection, Pick::Pair(newer.clone()));
            let Pick::Pair(paths) = selection else { unreachable!() };
            ws.start_comparison(paths, false, w, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let p = panel(ws);
            assert_eq!(p.paths, newer);
            assert!(p.review.is_some());
        });
        for operation in 0..4 {
            ws.update_in(cx, |ws, w, cx| {
                ws.load_comparison(newer.clone(), w, cx);
                let picker = ws.comparison.begin_pick();
                match operation {
                    0 => ws.close_panel(&ClosePanel, w, cx),
                    1 => ws.find(&Find, w, cx),
                    2 => ws.refresh_comparison(&RefreshComparison, w, cx),
                    _ => ws.clear_window(w, cx),
                }
                assert_eq!(ws.comparison.pick(picker, None, Some(first[0].clone())), Pick::Expired);
            });
        }
        cx.run_until_parked();
    }
    #[gpui::test]
    fn same_path_kind_or_symlink_rejection_preserves_the_ready_review(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        for symlink in [false, true] {
            let paths = files(
                if symlink { "review-current-link" } else { "review-current-kind" },
                "left\nkeep\n",
                "right\nkeep\nextra\n",
            );
            compare(&ws, &paths, false, cx);
            let viewers = ws.read_with(cx, |ws, _| review(ws).viewers.clone());
            std::fs::remove_file(&paths[1]).unwrap();
            if symlink {
                std::os::unix::fs::symlink(&paths[0], &paths[1]).unwrap();
            } else {
                std::fs::create_dir(&paths[1]).unwrap();
            }
            ws.update_in(cx, |ws, w, cx| {
                ws.start_comparison(paths.clone(), false, w, cx);
                assert_eq!(review(ws).viewers, viewers);
                assert!(panel(ws).message.is_none(), "new-selection progress must not disable the retained review");
                ws.next_difference(&NextDifference, w, cx);
            });
            cx.run_until_parked();
            ws.update_in(cx, |ws, w, cx| {
                let p = panel(ws);
                assert_eq!(p.paths, paths);
                assert_eq!(review(ws).viewers, viewers);
                assert!(p.message.is_none(), "rejected current paths must not disable the retained review");
                assert_eq!(p.counter(), "Difference 2 of 2");
                assert!(status(ws).contains(if symlink { "Symlinks" } else { "Not a regular file" }));
                ws.previous_difference(&PreviousDifference, w, cx);
                assert_eq!(panel(ws).counter(), "Difference 1 of 2");
            });
        }
    }
}
