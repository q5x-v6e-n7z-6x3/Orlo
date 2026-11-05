//! Sublime's FOLDERS side bar: a lazily expanded directory tree.

use crate::chrome::icons::{icon, Icon};
use crate::chrome::theme::{self, ui};
use crate::chrome::ui as kit;
use crate::config::project::{Excludes, Root};
use crate::config::settings::{ExplorerSort, Settings};
use crate::editor::Editor;
use crate::workspace::{
    AddFolder, ExplorerBack, ExplorerForward, ExplorerParent, NewFileHere, NewFolderHere, PathBreadcrumbs, Workspace,
};
use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, App, Bounds, ClickEvent, Context, ElementId, Entity, Pixels, Point,
    SharedString, UniformListDecoration, UniformListScrollHandle, Window,
};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use orlo_project::explorer::{entries, ignored_names, reread};

#[derive(Default)]
pub struct SidebarState {
    pub visible: bool,
    pub locations: orlo_project::explorer::LocationHistory,
    /// Invalidates pending location and breadcrumb reads.
    pub location_request: u64,
    /// The width the user chose by dragging the right edge (the workspace starts it at
    /// `theme::SIDEBAR_W`); a narrower window only clamps what is drawn.
    pub width: f32,
    expanded: HashSet<PathBuf>,
    /// What painting shows; only the window's background poll reads the disk.
    listings: HashMap<PathBuf, Vec<(PathBuf, bool)>>,
    /// Each listing's names, sorted as `watch::names` sorts them, for the poll to compare.
    names: HashMap<PathBuf, Arc<Vec<OsString>>>,
    /// Each root's `.gitignore` names, dimmed under it.
    ignored: HashMap<PathBuf, HashSet<String>>,
    /// Refresh Folders: the next poll reads every shown folder again, changed or not.
    reread: bool,
    /// The tree flattened into the rows on screen (`rows`), made again when `changes` moves:
    /// a keystroke redraws the window, and building 5,000 rows took 145 ms of each frame.
    rows: Vec<Row>,
    rows_made: Option<u64>,
    changes: u64,
    sort: ExplorerSort,
    sticky_ancestors: bool,
    /// The roots as last set, with their compiled exclude patterns.
    roots: Vec<(Root, Excludes)>,
    pub scroll: UniformListScrollHandle,
    /// Rows marked with ⌘-click, for Find in Files on them; in memory only.
    pub marked: Vec<PathBuf>,
    /// The row last clicked or reached with the keys: what side bar commands act on.
    pub selected: Option<PathBuf>,
    /// View > Side Bar > Show Open Files: the window's views above the folders.
    pub show_open_files: bool,
}

/// One row of the tree below the FOLDERS heading.
#[derive(Clone)]
pub struct Row {
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
    pub open: bool,
    /// The root it is under (its index in the window's roots).
    pub root: usize,
    /// A root that is not a folder now.
    pub missing: bool,
    parent: Option<usize>,
}

impl SidebarState {
    pub fn configure(&mut self, order: ExplorerSort, sticky: bool) {
        self.sticky_ancestors = sticky;
        if self.sort != order {
            self.sort = order;
            for list in self.listings.values_mut() {
                orlo_project::explorer::sort_entries(list, order);
            }
            self.changes += 1;
        }
    }

    /// Cached parent indices preserve the particular row under overlapping roots.
    fn ancestors(&self, index: usize, capacity: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut parent = self.rows.get(index).and_then(|r| r.parent);
        while let Some(i) = parent {
            if out.len() == capacity {
                break;
            }
            out.push(i);
            parent = self.rows[i].parent;
        }
        out.reverse();
        out
    }

    pub fn reveal_row(&self, index: usize, above: usize) {
        let capacity = self
            .scroll
            .0
            .borrow()
            .last_item_size
            .map_or(0, |size| (f32::from(size.item.height) / ROW_H).floor() as usize)
            .saturating_sub(1);
        // The resulting first visible row can be deeper than the requested row.
        // Reserve the maximum pinned stack while retaining a full content row.
        let offset = if self.sticky_ancestors { capacity } else { 0 };
        self.scroll.scroll_to_item_with_offset(above + 1 + index, gpui::ScrollStrategy::Center, offset);
    }

    /// A new folder or Project > Refresh Folders: the next poll reads every shown folder again.
    pub fn refresh(&mut self) {
        self.reread = true;
        self.changes += 1;
    }

    pub fn expanded(&self) -> impl Iterator<Item = &PathBuf> {
        self.expanded.iter()
    }

    pub fn expand(&mut self, dir: PathBuf) {
        self.expanded.insert(dir);
        self.changes += 1;
    }

    pub fn set_expanded(&mut self, dirs: HashSet<PathBuf>) {
        self.expanded = dirs;
        self.changes += 1;
    }

    /// Expand or collapse `dir`; expanding reads it afresh on the next poll.
    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
            self.listings.remove(dir);
            self.names.remove(dir);
        }
        self.changes += 1;
    }

    /// The window's roots changed (added, removed, reordered, found or lost): the rows and
    /// the exclude patterns follow.
    pub fn set_roots(&mut self, roots: &[Root]) {
        if self.roots.len() == roots.len() && self.roots.iter().zip(roots).all(|(a, b)| a.0 == *b) {
            return;
        }
        self.location_request += 1;
        self.locations.retain(|target| roots.iter().any(|root| target.path.starts_with(&root.path)));
        self.roots = (roots.iter()).map(|r| (r.clone(), r.excludes())).collect();
        self.changes += 1;
    }

    /// The rows of every root as the tree is expanded now, made again only after a change.
    pub fn rows(&mut self) -> &[Row] {
        if self.rows_made != Some(self.changes) {
            let mut rows = Vec::new();
            for (i, (root, _)) in self.roots.iter().enumerate() {
                self.flatten(i, &root.path, 0, root.missing, None, &mut rows);
            }
            (self.rows, self.rows_made) = (rows, Some(self.changes));
        }
        &self.rows
    }

    /// `dir`'s row and, while it is open, the rows under it; a missing root is one closed row.
    fn flatten(&self, root: usize, dir: &Path, depth: usize, missing: bool, parent: Option<usize>, out: &mut Vec<Row>) {
        let open = !missing && self.expanded.contains(dir);
        let index = out.len();
        out.push(Row { path: dir.to_path_buf(), is_dir: true, depth, open, root, missing, parent });
        if !open {
            return;
        }
        let excludes = &self.roots[root].1;
        for (path, is_dir) in self.listings.get(dir).into_iter().flatten() {
            if excludes.hides(path, *is_dir) {
                continue;
            }
            match is_dir {
                true => self.flatten(root, path, depth + 1, false, Some(index), out),
                false => out.push(Row {
                    path: path.clone(),
                    is_dir: false,
                    depth: depth + 1,
                    open: false,
                    root,
                    missing: false,
                    parent: Some(index),
                }),
            }
        }
    }

    /// Whether `dir` is expanded.
    pub fn is_expanded(&self, dir: &Path) -> bool {
        self.expanded.contains(dir)
    }

    /// The width drawn in a window `window_w` wide, leaving the editor and the drag strip room.
    pub fn drawn_width(&self, window_w: f32) -> f32 {
        self.width.min(window_w - theme::SIDEBAR_MAX_MARGIN).max(theme::SIDEBAR_MIN_W)
    }

    /// A drag of the right edge to `x` chooses that width, as far as this window allows.
    pub fn drag_to(&mut self, x: f32, window_w: f32) {
        self.width = x;
        self.width = self.drawn_width(window_w);
    }

    /// The folders a poll lists: the expanded ones under the `roots` whose parents are expanded
    /// too (the ones on screen), each once, with the names it shows, `None` when it was never
    /// read or Refresh Folders asked for a full read.
    pub fn folders_to_poll(&mut self, roots: &[PathBuf]) -> Vec<(PathBuf, Option<Arc<Vec<OsString>>>)> {
        let reread = std::mem::take(&mut self.reread);
        let shown = |d: &Path| {
            roots.iter().any(|root| {
                d.starts_with(root)
                    && d.ancestors().take_while(|a| a.starts_with(root)).all(|a| self.expanded.contains(a))
            })
        };
        self.expanded
            .iter()
            .filter(|d| shown(d))
            .map(|d| (d.clone(), self.names.get(d).filter(|_| !reread).cloned()))
            .collect()
    }

    /// Directory kind already known to the tree; no foreground filesystem lookup.
    pub fn is_directory(&self, path: &Path) -> Option<bool> {
        if self.roots.iter().any(|(r, _)| r.path == path) {
            return Some(true);
        }
        self.listings.values().flatten().find(|(p, _)| p == path).map(|(_, is_dir)| *is_dir)
    }

    /// Explicit location reads publish into the same tree cache as the poll.
    pub fn cache_listings(&mut self, listings: Vec<(PathBuf, Vec<(PathBuf, bool)>)>) {
        self.apply(listings, self.ignored.clone());
    }

    /// Take what a poll read: the folders it listed again and each root's .gitignore names.
    /// True when anything changed.
    pub fn apply(
        &mut self,
        listings: Vec<(PathBuf, Vec<(PathBuf, bool)>)>,
        ignored: HashMap<PathBuf, HashSet<String>>,
    ) -> bool {
        let changed = !listings.is_empty() || ignored != self.ignored;
        for (dir, mut list) in listings {
            // sorted once here, not on the UI thread at every poll
            let mut names: Vec<OsString> =
                (list.iter()).filter_map(|(p, _)| p.file_name().map(OsStr::to_os_string)).collect();
            names.sort();
            self.names.insert(dir.clone(), Arc::new(names));
            orlo_project::explorer::sort_entries(&mut list, self.sort);
            self.listings.insert(dir, list);
        }
        self.ignored = ignored;
        if changed {
            self.changes += 1;
        }
        changed
    }

    /// Paths moved on disk by Orlo: expansion, marks, the selection and the listings
    /// under `from` go to `to`, and the folders above both are read again.
    pub fn moved(&mut self, from: &Path, to: &Path) {
        let map = |p: &PathBuf| match p.strip_prefix(from) {
            Ok(rest) if rest.as_os_str().is_empty() => to.to_path_buf(),
            Ok(rest) => to.join(rest),
            Err(_) => p.clone(),
        };
        self.locations.moved(from, to);
        self.location_request += 1;
        self.expanded = self.expanded.iter().map(map).collect();
        self.marked = self.marked.iter().map(map).collect();
        self.selected = self.selected.as_ref().map(map);
        self.listings.retain(|d, _| !d.starts_with(from));
        self.names.retain(|d, _| !d.starts_with(from));
        self.refresh();
    }
}

/// A row of the OPEN FILES section: a view, with what its tab shows.
#[derive(Clone)]
pub struct OpenRow {
    pub view: Entity<Editor>,
    pub label: String,
    pub active: bool,
    pub dirty: bool,
    pub preview: bool,
    pub pinned: bool,
}

/// A row above the tree.
pub enum OpenEntry {
    /// OPEN FILES
    Heading,
    /// GROUP N, when the window has several groups.
    Group(usize),
    View(OpenRow),
}

/// The rows above the tree: OPEN FILES and its views (under GROUP N headings when there are
/// several groups), straight from the window's groups, so moves, renames, closes and
/// promotions show at once. A name used twice shows its folder; more views of one buffer are
/// numbered.
pub fn open_rows(ws: &Workspace, cx: &App) -> Vec<OpenEntry> {
    let several = ws.views.groups.len() > 1;
    let names: Vec<(String, Option<PathBuf>)> =
        (ws.views.tabs()).map(|t| (t.read(cx).title(cx), t.read(cx).doc(cx).path.clone())).collect();
    let mut seen: Vec<(Entity<Editor>, usize)> = Vec::new();
    let mut out = vec![OpenEntry::Heading];
    for (g, group) in ws.views.groups.iter().enumerate() {
        if several {
            out.push(OpenEntry::Group(g));
        }
        for (i, t) in group.tabs.iter().enumerate() {
            let e = t.read(cx);
            let name = e.title(cx);
            let folder = (e.doc(cx).path.as_deref()).and_then(Path::parent).and_then(Path::file_name);
            let twice = (names.iter()).any(|(n, p)| *n == name && *p != e.doc(cx).path);
            let mut label = match (twice, folder) {
                (true, Some(f)) => format!("{name} — {}", f.to_string_lossy()),
                _ => name,
            };
            match seen.iter_mut().find(|(b, _)| b.read(cx).same_document(e)) {
                Some((_, n)) => {
                    *n += 1;
                    label = format!("{label} ({})", *n);
                }
                None => seen.push((t.clone(), 1)),
            }
            out.push(OpenEntry::View(OpenRow {
                view: t.clone(),
                label,
                active: g == ws.views.active && i == group.active,
                dirty: e.doc(cx).unsaved(),
                preview: group.preview.as_ref() == Some(t),
                pinned: group.pinned.contains(t),
            }));
        }
    }
    out
}

/// Every row of the side bar is this tall (a uniform list measures one).
const ROW_H: f32 = 22.;
/// Where a row's content starts, and the indent of each level.
/// Rows are pills inset this far from the side bar's edges; `ROW_PAD` pads inside them.
const INSET: f32 = 6.;
const ROW_PAD: f32 = 6.;
const INDENT: f32 = 12.;

pub fn render(ws: &mut Workspace, width: f32, cx: &mut Context<Workspace>) -> AnyElement {
    ws.sidebar.set_roots(&ws.roots);
    let settings = cx.global::<Settings>();
    ws.sidebar.configure(settings.explorer_sort, settings.explorer_sticky_ancestors);
    let open = match ws.sidebar.show_open_files {
        true => open_rows(ws, cx),
        false => Vec::new(),
    };
    let above = open.len();
    let folders = !ws.roots.is_empty();
    // the headings scroll with the rows, as in Sublime
    let n = open.len() + if folders { ws.sidebar.rows().len() + 1 } else { 0 };
    div()
        .id("sidebar")
        .group("sidebar")
        .key_context("SideBar")
        .track_focus(&ws.sidebar_focus)
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .flex()
        .flex_col()
        .pt(px(6.))
        .bg(ui(theme::SIDEBAR_BG))
        .text_color(ui(theme::SIDEBAR_FG))
        .when(!folders, |d| {
            // no folder yet: say so, and offer one, as Zed's empty project panel does
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap(px(8.))
                    .px(px(12.))
                    .pt(px(6.))
                    .pb(px(10.))
                    .child(div().text_size(px(11.)).text_color(ui(theme::TEXT_MUTED)).child("No folder open"))
                    .child(control(
                        kit::button("sidebar-open-folder", "Open Folder…"),
                        "sidebar-open-folder",
                        AddFolder,
                        cx,
                    )),
            )
        })
        .when(folders, |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .px(px(8.))
                    .pb(px(4.))
                    .child(control(
                        kit::icon_button("explorer-back", Icon::ChevronLeft, "Explorer Back (⌥←)"),
                        "explorer-back",
                        ExplorerBack,
                        cx,
                    ))
                    .child(control(
                        kit::icon_button("explorer-forward", Icon::ChevronRight, "Explorer Forward (⌥→)"),
                        "explorer-forward",
                        ExplorerForward,
                        cx,
                    ))
                    .child(control(
                        kit::icon_button("explorer-parent", Icon::ChevronUp, "Explorer Parent (⌥↑)"),
                        "explorer-parent",
                        ExplorerParent,
                        cx,
                    ))
                    .child(control(
                        kit::button("path-breadcrumbs", "Path…")
                            .tooltip(kit::tooltip("Active-file Path Breadcrumbs".into())),
                        "path-breadcrumbs",
                        PathBreadcrumbs,
                        cx,
                    )),
            )
        })
        .child(
            // only the rows on screen are built
            uniform_list(
                "sidebar-rows",
                n,
                cx.processor(move |ws, range: std::ops::Range<usize>, _, cx| {
                    let active = ws.active_editor().and_then(|e| e.read(cx).doc(cx).path.clone());
                    let badges = &ws.git.badges;
                    let sb = &mut ws.sidebar;
                    sb.rows();
                    range
                        .map(|i| {
                            if let Some(r) = open.get(i) {
                                return match r {
                                    OpenEntry::Heading => heading("OPEN FILES").into_any_element(),
                                    OpenEntry::Group(g) => group_heading(*g),
                                    OpenEntry::View(row) => open_row(row, cx),
                                };
                            }
                            let i = i - open.len();
                            match i.checked_sub(1).and_then(|r| sb.rows.get(r)) {
                                None => folders_heading(cx),
                                Some(r) => {
                                    let selected = active.as_deref() == Some(r.path.as_path())
                                        || sb.selected.as_deref() == Some(r.path.as_path())
                                        || sb.marked.contains(&r.path);
                                    let root = &sb.roots[r.root].0;
                                    let ignored = sb.ignored.get(&root.path);
                                    row(r, root, selected, ignored, badges.get(&r.path).cloned(), (i - 1, false), cx)
                                }
                            }
                        })
                        .collect()
                }),
            )
            .flex_1()
            .track_scroll(ws.sidebar.scroll.clone())
            .when(ws.sidebar.sticky_ancestors, |list| {
                list.with_decoration(StickyAncestors { workspace: cx.entity(), above })
            }),
        )
        .into_any_element()
}

struct StickyAncestors {
    workspace: Entity<Workspace>,
    above: usize,
}

impl UniformListDecoration for StickyAncestors {
    fn compute(
        &self,
        visible: std::ops::Range<usize>,
        bounds: Bounds<Pixels>,
        offset: Point<Pixels>,
        item_height: Pixels,
        _: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.workspace.update(cx, |ws, cx| {
            let capacity = (bounds.size.height / item_height).floor() as usize;
            let indices = visible
                .start
                .checked_sub(self.above + 1)
                .map(|index| ws.sidebar.ancestors(index, capacity.saturating_sub(1)))
                .unwrap_or_default();
            let active = ws.active_editor().and_then(|e| e.read(cx).doc(cx).path.clone());
            let badges = ws.git_badges();
            let sb = &ws.sidebar;
            div()
                .relative()
                .w(bounds.size.width)
                .h(bounds.size.height)
                .child(
                    div()
                        .absolute()
                        .top(-offset.y)
                        .left(px(0.))
                        .w_full()
                        .flex()
                        .flex_col()
                        .bg(ui(theme::SIDEBAR_BG))
                        .block_mouse_except_scroll()
                        .children(indices.into_iter().map(|index| {
                            let r = &sb.rows[index];
                            let root = &sb.roots[r.root].0;
                            let selected = active.as_deref() == Some(&r.path)
                                || sb.selected.as_deref() == Some(&r.path)
                                || sb.marked.contains(&r.path);
                            row(
                                r,
                                root,
                                selected,
                                sb.ignored.get(&root.path),
                                badges.get(&r.path).cloned(),
                                (index, true),
                                cx,
                            )
                        })),
                )
                .into_any_element()
        })
    }
}

/// `el` runs `action` from the side bar, as its keys would.
fn control<A: gpui::Action + Clone>(
    el: gpui::Stateful<gpui::Div>,
    selector: &'static str,
    action: A,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    el.debug_selector(move || selector.to_string())
        .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| {
            window.focus(&ws.sidebar_focus);
            window.dispatch_action(Box::new(action.clone()), cx);
        }))
        .into_any_element()
}

fn heading(text: &'static str) -> gpui::Div {
    div().h(px(ROW_H)).pl(px(INSET + ROW_PAD)).flex().items_center().child(kit::heading(text))
}

/// FOLDERS, and the side bar's New File and New Folder: they show while the mouse is over the
/// side bar (they stay clickable, for the keyboard-less moment the pointer arrives).
fn folders_heading(cx: &mut Context<Workspace>) -> AnyElement {
    let quiet = |id: &'static str, i: Icon, tip: &'static str| {
        kit::icon_button(id, i, tip).size(px(18.)).opacity(0.).group_hover("sidebar", |s| s.opacity(1.))
    };
    let new_file = quiet("sidebar-new-file", Icon::FilePlus, "New File…");
    let new_folder = quiet("sidebar-new-folder", Icon::FolderPlus, "New Folder…");
    heading("FOLDERS")
        .pr(px(6.))
        .gap(px(2.))
        .child(div().flex_1())
        .child(control(new_file, "sidebar-new-file", NewFileHere, cx))
        .child(control(new_folder, "sidebar-new-folder", NewFolderHere, cx))
        .into_any_element()
}

fn group_heading(g: usize) -> AnyElement {
    div()
        .h(px(ROW_H))
        .pl(px(INSET + ROW_PAD + INDENT))
        .flex()
        .items_center()
        .child(kit::heading(format!("GROUP {}", g + 1)))
        .into_any_element()
}

/// An OPEN FILES row: a click shows the view, `×` (a dot while unsaved) closes it.
fn open_row(r: &OpenRow, cx: &mut Context<Workspace>) -> AnyElement {
    let (view, close) = (r.view.clone(), r.view.clone());
    let id = r.view.entity_id();
    inset(
        div()
            .id(ElementId::Name(SharedString::from(format!("open:{id}"))))
            .group("open-row")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .w_full()
            .h(px(ROW_H))
            .pl(px(ROW_PAD + INDENT))
            .pr(px(4.))
            .rounded(px(kit::RADIUS))
            .whitespace_nowrap()
            .overflow_hidden()
            .when(r.active, |d| d.bg(ui(theme::SIDEBAR_SELECTED_BG)))
            .hover(|s| s.bg(ui(theme::SIDEBAR_HOVER_BG)))
            .cursor_pointer()
            .when(r.pinned, |d| d.child(icon(Icon::Pin, 12.).text_color(ui(theme::SIDEBAR_ICON))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .when(r.active, |d| d.text_color(ui(theme::SIDEBAR_FG_SELECTED)))
                    .when(r.preview, |d| d.italic())
                    .child(r.label.clone()),
            )
            .child(
                div()
                    .id(ElementId::Name(SharedString::from(format!("open-close:{id}"))))
                    .size(px(16.))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .text_color(ui(theme::SIDEBAR_ICON))
                    .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)).text_color(ui(theme::ICON)))
                    .map(|d| match r.dirty {
                        true => d.child(div().size(px(7.)).rounded_full().bg(ui(theme::SIDEBAR_ICON))),
                        false => d
                            .child(icon(Icon::Close, 11.).text_color(ui(theme::SIDEBAR_ICON)))
                            .invisible()
                            .group_hover("open-row", |s| s.visible()),
                    })
                    .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        ws.close_view(close.clone(), window, cx);
                    })),
            )
            .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| ws.show_view(&view, window, cx))),
    )
}

/// A row as a pill, `INSET` in from the side bar's edges.
fn inset(row: impl IntoElement) -> AnyElement {
    div().w_full().px(px(INSET)).child(row).into_any_element()
}

/// A file's icon: code, text, or other, as Sublime tells its `/*`, lines and `<>` apart.
fn file_icon(path: &Path) -> Icon {
    match path.extension().and_then(|e| e.to_str()) {
        Some("md" | "txt" | "markdown" | "rst" | "log") | None => Icon::FileText,
        Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "icns" | "pdf" | "lock") => Icon::File,
        _ => Icon::FileCode,
    }
}

/// A git badge's colour (`!` conflict, `?` new, `S`/`M` changed) and the letters a file shows.
fn git_tint(badge: &str) -> (theme::Ui, String) {
    let tint = if badge.contains('!') {
        theme::GIT_CONFLICT
    } else if badge.contains(['S', 'M']) {
        theme::GIT_MODIFIED
    } else {
        theme::GIT_ADDED
    };
    (tint, badge.replace('?', "U"))
}

fn row(
    r: &Row,
    root: &Root,
    selected: bool,
    ignored: Option<&HashSet<String>>,
    badge: Option<String>,
    (index, pinned): (usize, bool),
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let (path, is_dir, depth, open) = (r.path.as_path(), r.is_dir, r.depth, r.open);
    let name = match depth {
        0 if r.missing => format!("{} (not found)", root.label()),
        0 => root.label(),
        _ => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()),
    };
    let missing = r.missing;
    let dim = missing || depth > 0 && ignored.is_some_and(|i| i.contains(&name));
    let (p, menu_path) = (path.to_path_buf(), path.to_path_buf());
    let git = badge.filter(|b| !b.is_empty()).map(|b| git_tint(&b));
    let (glyph, glyph_tone) = match is_dir {
        true if open && !missing => (Icon::FolderOpen, theme::SIDEBAR_ICON),
        true => (Icon::Folder, theme::SIDEBAR_ICON),
        false => (file_icon(path), theme::SIDEBAR_FILE_GLYPH),
    };
    inset(
        div()
            // a folder under two overlapping roots is two rows
            .id(ElementId::Name(SharedString::from(format!("{}:{}:{}", pinned, r.root, path.to_string_lossy()))))
            .debug_selector({
                let root = r.root;
                move || format!("explorer-{}-{root}-{index}", if pinned { "sticky" } else { "tree" })
            })
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(ROW_H))
            .pl(px(ROW_PAD + depth as f32 * INDENT))
            .pr(px(6.))
            .rounded(px(kit::RADIUS))
            .whitespace_nowrap()
            .overflow_hidden()
            .when(selected, |d| d.bg(ui(theme::SIDEBAR_SELECTED_BG)))
            .hover(|s| s.bg(ui(theme::SIDEBAR_HOVER_BG)))
            .cursor_pointer()
            .child(div().w(px(14.)).flex_shrink_0().flex().items_center().text_color(ui(theme::SIDEBAR_ICON)).when(
                is_dir && !missing,
                |d| {
                    d.child(
                        icon(
                            match open {
                                true => Icon::ChevronDown,
                                false => Icon::ChevronRight,
                            },
                            11.,
                        )
                        .text_color(ui(theme::SIDEBAR_ICON)),
                    )
                },
            ))
            .child(icon(glyph, 14.).text_color(ui(glyph_tone)))
            .child(
                div()
                    .pl(px(6.))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .when(selected, |d| d.text_color(ui(theme::SIDEBAR_FG_SELECTED)))
                    .when_some(git.as_ref(), |d, (tint, _)| d.text_color(ui(*tint)))
                    .when(dim, |d| d.text_color(ui(theme::SIDEBAR_FG_IGNORED)))
                    .child(name),
            )
            // a file says how it changed; a folder's colour is enough
            .when_some(git.filter(|_| !is_dir), |d, (tint, letters)| {
                d.child(
                    div()
                        .pl(px(6.))
                        .flex_shrink_0()
                        .text_size(px(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(ui(tint))
                        .child(letters),
                )
            })
            .on_click(cx.listener(move |ws, e: &ClickEvent, window, cx| {
                // ⌘-click marks a row for Find in Files, as Finder's adds to a selection
                if e.modifiers().platform {
                    let m = &mut ws.sidebar.marked;
                    match m.iter().position(|x| *x == p) {
                        Some(i) => {
                            m.remove(i);
                        }
                        None => m.push(p.clone()),
                    }
                    return cx.notify();
                }
                ws.sidebar.selected = Some(p.clone());
                if missing {
                    return cx.notify();
                }
                if is_dir {
                    window.focus(&ws.sidebar_focus);
                    ws.sidebar.toggle(&p);
                    if ws.sidebar.is_expanded(&p) {
                        ws.navigate_explorer(p.clone(), window, cx);
                    }
                    ws.poll_disk(window, cx);
                    cx.notify();
                } else if e.click_count() > 1 {
                    // a double click keeps it open
                    ws.open_kept_path(p.clone(), window, cx);
                    cx.notify();
                } else {
                    ws.open_preview(p.clone(), window, cx);
                    // the keys stay in the side bar, to go on browsing
                    window.focus(&ws.sidebar_focus);
                }
            }))
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |ws, e: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&ws.sidebar_focus);
                    ws.open_sidebar_menu(menu_path.clone(), depth == 0, e.position, cx);
                }),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) use crate::workspace::ui_tests::temp;

    #[gpui::test]
    fn sidebar_controls_offer_a_folder_and_create_in_the_selected_directory(cx: &mut gpui::TestAppContext) {
        let base = temp(&format!("orlo-sidebar-controls-{}", std::process::id()));
        let root = base.join("project");
        let selected = root.join("selected");
        std::fs::create_dir_all(&selected).unwrap();
        crate::app::session::TEST_SUPPORT.set(Some(base.join("support")));
        cx.update(|cx| {
            cx.set_global(crate::config::settings::Settings::default());
            cx.set_global(crate::workspace::SettingsDisk(None));
            cx.set_global(crate::workspace::Recent::default());
            cx.bind_keys(crate::editor::key_bindings());
            cx.bind_keys(crate::workspace::key_bindings());
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        cx.run_until_parked();
        assert!(cx.debug_bounds("sidebar-open-folder").is_some());
        assert!(cx.debug_bounds("sidebar-new-file").is_none());
        workspace.update_in(cx, |ws, window, cx| {
            ws.open_path(root.clone(), window, cx);
            ws.sidebar.selected = Some(selected.clone());
        });
        cx.run_until_parked();
        for (button, name) in [("sidebar-new-file", "notes.txt"), ("sidebar-new-folder", "assets")] {
            let bounds = cx.debug_bounds(button).expect(button);
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.simulate_input(name);
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
        }
        assert_eq!(std::fs::read_to_string(selected.join("notes.txt")).unwrap(), "");
        assert!(selected.join("assets").is_dir());
        assert!(!root.join("notes.txt").exists());
    }

    /// What a poll does with `sb`: list in the background, then apply. The folders it read.
    pub(super) fn poll_all(sb: &mut SidebarState, roots: &[PathBuf]) -> Vec<PathBuf> {
        let read: Vec<_> = (sb.folders_to_poll(roots).into_iter())
            .filter_map(|(d, shown)| reread(&d, shown.as_deref().map(|n| n.as_slice())).map(|l| (d, l)))
            .collect();
        let dirs = read.iter().map(|(d, _)| d.clone()).collect();
        let ignored = roots.iter().map(|r| (r.clone(), ignored_names(r))).collect();
        sb.apply(read, ignored);
        dirs
    }

    fn poll(sb: &mut SidebarState, root: &Path) -> Vec<PathBuf> {
        poll_all(sb, &[root.to_path_buf()])
    }

    #[test]
    fn a_saves_leftover_temp_file_is_not_listed() {
        let dir = temp("orlo-sidebar-temp");
        for n in ["a.txt", "a.txt.orlo-4242-0.tmp"] {
            std::fs::write(dir.join(n), "").unwrap();
        }
        let names: Vec<_> = entries(&dir).into_iter().map(|(p, _)| p).collect();
        assert_eq!(names, [dir.join("a.txt")]);
        assert_eq!(reread(&dir, Some(&["a.txt".into()])), None, "nothing new to show");
    }

    #[test]
    fn a_poll_lists_new_and_changed_folders_only() {
        let dir = temp("orlo-sidebar-poll");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a"), "").unwrap();
        let mut sb = SidebarState::default();
        sb.toggle(&dir);
        assert_eq!(poll(&mut sb, &dir), [dir.as_path()], "expanded: read once");
        assert_eq!(sb.listings[&dir].len(), 2, "sub, a");
        std::fs::write(dir.join(".DS_Store"), "").unwrap();
        assert!(poll(&mut sb, &dir).is_empty(), "excluded names are no change");
        std::fs::write(dir.join("b"), "").unwrap();
        assert_eq!(poll(&mut sb, &dir), [dir.as_path()]);
        assert_eq!(sb.listings[&dir].len(), 3, "sub, a, b");
        assert!(poll(&mut sb, &dir).is_empty());
        sb.refresh();
        assert_eq!(poll(&mut sb, &dir), [dir.as_path()], "Refresh Folders reads all");
        sb.toggle(&dir);
        sb.toggle(&dir);
        assert_eq!(poll(&mut sb, &dir), [dir.as_path()], "expanding again reads it");
    }

    #[test]
    fn folders_under_a_collapsed_parent_are_not_polled() {
        let root = PathBuf::from("/r");
        let mut sb = SidebarState::default();
        for d in ["/r", "/r/a", "/r/a/b", "/r/c/d", "/elsewhere"] {
            sb.expanded.insert(d.into());
        }
        let mut dirs: Vec<PathBuf> =
            sb.folders_to_poll(std::slice::from_ref(&root)).into_iter().map(|(d, _)| d).collect();
        dirs.sort();
        assert_eq!(dirs, [PathBuf::from("/r"), "/r/a".into(), "/r/a/b".into()]);
        sb.expanded.remove(&root);
        assert!(sb.folders_to_poll(std::slice::from_ref(&root)).is_empty(), "a collapsed root shows nothing");
    }

    #[test]
    fn the_gitignore_is_read_again_by_every_poll() {
        let dir = temp("orlo-sidebar-ignore");
        let mut sb = SidebarState::default();
        sb.toggle(&dir);
        poll(&mut sb, &dir);
        assert!(sb.ignored[&dir].is_empty());
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        poll(&mut sb, &dir);
        assert!(sb.ignored[&dir].contains("target"));
    }

    #[test]
    fn a_narrow_window_clamps_the_drawn_width_not_the_chosen_one() {
        let mut sb = SidebarState { width: 400., ..Default::default() };
        assert_eq!(sb.drawn_width(300.), 300. - theme::SIDEBAR_MAX_MARGIN);
        assert_eq!(sb.width, 400.);
        assert_eq!(sb.drawn_width(800.), 400.);
        assert_eq!(sb.drawn_width(60.), theme::SIDEBAR_MIN_W);
        sb.drag_to(1000., 700.);
        assert_eq!(sb.width, 700. - theme::SIDEBAR_MAX_MARGIN);
        sb.drag_to(10., 700.);
        assert_eq!(sb.width, theme::SIDEBAR_MIN_W);
    }

    #[test]
    fn the_rows_follow_expanding_and_polling() {
        let dir = temp("orlo-sidebar-rows");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/x"), "").unwrap();
        std::fs::write(dir.join("a"), "").unwrap();
        let mut sb = SidebarState::default();
        sb.set_roots(&[Root::new(dir.clone())]);
        sb.expand(dir.clone());
        poll(&mut sb, &dir);
        let names = |sb: &mut SidebarState| {
            (sb.rows().iter()).map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>()
        };
        let root = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(names(&mut sb), [root.as_str(), "sub", "a"]);
        sb.toggle(&dir.join("sub"));
        poll(&mut sb, &dir);
        assert_eq!(names(&mut sb), [root.as_str(), "sub", "x", "a"]);
        std::fs::write(dir.join("b"), "").unwrap();
        poll(&mut sb, &dir);
        assert_eq!(names(&mut sb), [root.as_str(), "sub", "x", "a", "b"]);
    }
}

#[cfg(test)]
mod root_tests {
    use super::tests::{poll_all, temp};
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let dir = temp(&format!("{name}-{}", std::process::id()));
        for d in ["a/node_modules", "b/sub"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in ["a/x.rs", "a/x.pyc", "a/node_modules/m.js", "b/sub/y.txt"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        dir
    }

    fn labels(sb: &mut SidebarState) -> Vec<(usize, String)> {
        (sb.rows().iter()).map(|r| (r.root, r.path.file_name().unwrap().to_string_lossy().into_owned())).collect()
    }

    #[test]
    fn several_roots_list_in_order_expand_on_their_own_and_hide_their_excludes() {
        let dir = tree("orlo-sidebar-roots");
        let (a, b) = (dir.join("a"), dir.join("b"));
        let mut sb = SidebarState::default();
        let mut ra = Root::new(a.clone());
        ra.folder_exclude_patterns = vec!["node_modules".into()];
        ra.file_exclude_patterns = vec!["*.pyc".into()];
        sb.set_roots(&[ra.clone(), Root::new(b.clone())]);
        sb.expand(a.clone());
        poll_all(&mut sb, &[a.clone(), b.clone()]);
        assert_eq!(
            labels(&mut sb),
            [(0, "a".into()), (0, "x.rs".into()), (1, "b".into())],
            "a's excludes hide node_modules and x.pyc; b stays collapsed"
        );
        sb.expand(b.clone());
        sb.expand(b.join("sub"));
        poll_all(&mut sb, &[a.clone(), b.clone()]);
        assert_eq!(labels(&mut sb).len(), 5, "b, sub, y.txt added");
        // reordered: b's rows first
        sb.set_roots(&[Root::new(b.clone()), ra]);
        assert_eq!(labels(&mut sb)[0], (0, "b".into()));
        // a root that went missing is one row, kept
        let mut gone = Root::new(dir.join("gone"));
        gone.missing = true;
        sb.set_roots(&[gone]);
        let rows = sb.rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].missing && rows[0].depth == 0);
    }

    #[test]
    fn overlapping_roots_poll_a_folder_once() {
        let dir = tree("orlo-sidebar-overlap");
        let (a, sub) = (dir.clone(), dir.join("b/sub"));
        let mut sb = SidebarState::default();
        sb.set_roots(&[Root::new(a.clone()), Root::new(sub.clone())]);
        for d in [&a, &dir.join("b"), &sub] {
            sb.expand(d.to_path_buf());
        }
        let polled = sb.folders_to_poll(&[a.clone(), sub.clone()]);
        let mut dirs: Vec<_> = polled.into_iter().map(|(d, _)| d).collect();
        dirs.sort();
        assert_eq!(dirs, [a.clone(), dir.join("b"), sub.clone()], "each once");
        poll_all(&mut sb, &[a, sub.clone()]);
        // the shared folder shows under both roots, its file twice
        let ys = (sb.rows().iter()).filter(|r| r.path.ends_with("y.txt")).count();
        assert_eq!(ys, 2);
        sb.moved(&sub, &dir.join("b/renamed"));
        assert!(sb.is_expanded(&dir.join("b/renamed")) && !sb.is_expanded(&sub));
    }
}

#[cfg(test)]
mod n29_tests;
