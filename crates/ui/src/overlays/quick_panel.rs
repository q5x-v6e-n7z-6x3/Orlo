//! Sublime's quick panel: Goto Anything (cmd-p), Command Palette (cmd-shift-p), Goto Line
//! (ctrl-g), Goto Symbol (cmd-r), Goto Symbol in Project (cmd-shift-r).

use crate::chrome::fuzzy;
use crate::chrome::sidebar;
use crate::chrome::theme::{self, c, ui};
use crate::chrome::ui as kit;
use crate::editing::clipboard_history::ClipEntry;
use crate::editing::csv::{ColumnOp, Delim};
use crate::editing::text::{EditorState, Permute};
use crate::editing::transform::{NormForm, TextOp};
use crate::editor::{self, Editor};
use crate::languages::highlight::{self, Highlighter};
use crate::languages::index::IndexedSymbol;
use crate::workspace::{self, Workspace};
use gpui::{
    actions, div, prelude::*, px, Action, AnyElement, ClickEvent, Context, Entity, Focusable, FontWeight,
    HighlightStyle, KeyBinding, StyledText, WeakEntity, Window,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

actions!(quick_panel, [SelectNext, SelectPrev, Confirm, Dismiss, CloseItem]);

/// Bound after the editor's bindings so they win inside the panel's input.
pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("QuickPanel > Editor");
    vec![
        KeyBinding::new("up", SelectPrev, ctx),
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
        // the tab list closes the highlighted view; elsewhere it deletes in the field as usual
        KeyBinding::new("cmd-backspace", CloseItem, ctx),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The tab list: open views of a group, the window or every window.
    Tabs(workspace::TabScope),
    Files,
    Commands,
    GotoLine,
    Bookmarks,
    Symbols,
    /// Goto Symbol in Project: the index's definitions.
    ProjectSymbols,
    Definitions,
    References,
    ClipboardHistory,
    Spelling,
    /// A search field's history, the Where chooser, saved searches: rows found by their text.
    Choices,
}

pub(crate) enum ItemAction {
    Spelling(Arc<workspace::SpellingChoice>, crate::languages::spell::WordAction),
    Language(workspace::LanguageChoice),
    Template(Arc<workspace::snippets::TemplateChoice>),
    Explorer(Arc<workspace::explorer::BreadcrumbChoice>),
    OpenFile(PathBuf),
    Dispatch(Box<dyn Action>),
    /// A clipboard history entry, and the single-line input that had the keys when the panel
    /// opened (the document otherwise).
    PasteHistory(ClipEntry, Option<WeakEntity<Editor>>),
    /// A search history entry or saved scope: the field's text becomes it.
    SetField(String, WeakEntity<Editor>),
    /// A Where token or path, added to the field after a comma.
    AppendField(String, WeakEntity<Editor>),
    /// An open view, by entity, in any window (the tab list).
    View(gpui::EntityId),
    Bookmark(BookmarkTarget),
    /// A place in a file or an untitled buffer: its 0-based line and the chars there to select.
    /// Highlighting the row previews it.
    Location(Place, usize, std::ops::Range<usize>),
}

/// A live view-owned bookmark snapshot. IDs do not retain closed views/documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BookmarkTarget {
    pub view: gpui::EntityId,
    pub document: u64,
    pub version: u64,
    pub line: usize,
}

/// Where a location row goes: a file, or the view of an untitled buffer.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    File(PathBuf),
    View(gpui::EntityId),
}

pub struct Item {
    pub label: String,
    /// Second line for files: the relative path, which is also what gets matched.
    pub detail: String,
    /// Key shortcut shown at the right of command rows, a symbol's syntax.
    pub hint: String,
    /// Goto Symbol's kind badge: letter and colour.
    pub badge: Option<(char, u32)>,
    pub(crate) action: ItemAction,
}

impl Item {
    pub(crate) fn new(label: impl Into<String>, detail: impl Into<String>, action: ItemAction) -> Self {
        Self { label: label.into(), detail: detail.into(), hint: String::new(), badge: None, action }
    }

    pub(crate) fn command(label: impl Into<String>, detail: impl Into<String>, action: impl Action) -> Self {
        Self::new(label, detail, ItemAction::Dispatch(Box::new(action)))
    }
}

/// Symbol rows and their search keys are prepared off the window thread.
pub struct Rows {
    pub items: Vec<Item>,
    pub keys: Arc<Vec<String>>,
}

impl Rows {
    pub fn new(kind: Kind, items: Vec<Item>) -> Self {
        let keys = items
            .iter()
            .map(|i| {
                if matches!(kind, Kind::Tabs(_) | Kind::Bookmarks) {
                    format!("{} {}", i.label, i.detail)
                } else if i.detail.is_empty()
                    || matches!(
                        kind,
                        Kind::Commands | Kind::ClipboardHistory | Kind::Spelling | Kind::Choices | Kind::ProjectSymbols
                    )
                {
                    i.label.clone()
                } else {
                    i.detail.clone()
                }
            })
            .collect();
        Self { items, keys: Arc::new(keys) }
    }
}

/// The view a preview moved, its selections (primary last) and its top row, for Escape.
pub type Origin = (Entity<Editor>, Vec<crate::editing::text::Selection>, (usize, f32, f32));

pub struct QuickPanel {
    /// Keep the source request through confirmation; this picker owns its initial editor blur.
    pub(crate) language_navigation: Option<workspace::language::Request>,
    /// A Git command returns to its originating sidebar or diff before dispatch.
    pub(crate) git_origin: bool,
    /// Full selected diagnostic text, only in the existing diagnostics picker.
    pub(crate) diagnostic_details: bool,
    /// Full selected metadata for an explicitly enabled choice picker (local tags).
    pub(crate) choice_details: bool,
    pub kind: Kind,
    /// What the panel opened as; Goto Anything flips to `Symbols` while the query starts with `@`.
    pub base: Kind,
    pub input: Entity<Editor>,
    pub items: Vec<Item>,
    pub keys: Arc<Vec<String>>,
    pub query: String,
    pub filtered: Vec<usize>,
    pub selected: usize,
    /// The view a `:N` preview moved and its caret before; escape puts it back.
    pub origin: Option<Origin>,
    /// Location previews restore every touched view and the original tab arrangement.
    pub views: Option<crate::chrome::layout::Groups<Entity<Editor>>>,
    pub moved: Vec<Origin>,
    pub opened: Option<Entity<Editor>>,
    /// Surviving name chars, kept separately because an edit can move the preview's caret.
    pub symbol_preview: Option<(Entity<Editor>, Vec<std::ops::Range<usize>>)>,
    pub rows_task: Option<gpui::Task<()>>,
    pub filter_task: Option<gpui::Task<()>>,
    pub loaded: bool,
    pub confirming: bool,
    pub title: Option<String>,
    pub progress: Option<String>,
    pub lookup: Option<(String, Place, String)>,
    pub(crate) definition_source: Option<(editor::DefinitionSource, usize)>,
    pub to_side: bool,
    pub initial: bool,
    pub reference_rows: Option<Rows>,
    pub reference_generation: Option<u64>,
    pub reference_versions: std::collections::HashMap<u64, u64>,
    pub(crate) reference_search: Option<(crate::editor::Cancel, gpui::Task<()>)>,
}

impl QuickPanel {
    pub fn new(kind: Kind, items: Vec<Item>, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let input = cx.new(|cx| {
            let mut e = Editor::new(EditorState::new(), true, cx);
            e.ui_font = true;
            e
        });
        Editor::on_text_change_in(&input, window, cx, |ws, window, cx| ws.quick_panel_input_changed(window, cx))
            .detach();
        let input_id = input.entity_id();
        cx.on_blur(&input.read(cx).focus_handle(cx), window, move |ws, _, _| {
            if ws
                .quick_panel
                .as_ref()
                .is_some_and(|p| p.input.entity_id() == input_id && p.language_navigation.is_some())
            {
                ws.invalidate_navigation();
            }
        })
        .detach();
        input.read(cx).focus(window);
        let mut p = Self {
            language_navigation: None,
            git_origin: false,
            diagnostic_details: false,
            choice_details: false,
            kind,
            base: kind,
            input,
            items: Vec::new(),
            keys: Arc::default(),
            query: String::new(),
            filtered: Vec::new(),
            selected: 0,
            origin: None,
            views: None,
            moved: Vec::new(),
            opened: None,
            symbol_preview: None,
            rows_task: None,
            filter_task: None,
            loaded: false,
            confirming: false,
            title: None,
            progress: None,
            lookup: None,
            definition_source: None,
            to_side: false,
            initial: false,
            reference_search: None,
            reference_rows: None,
            reference_generation: None,
            reference_versions: Default::default(),
        };
        p.set_items(kind, items);
        p.refilter("");
        p
    }

    pub fn set_items(&mut self, kind: Kind, items: Vec<Item>) {
        self.replace_rows(kind, Rows::new(kind, items));
    }

    pub fn replace_rows(&mut self, kind: Kind, rows: Rows) -> Rows {
        self.kind = kind;
        Rows {
            keys: std::mem::replace(&mut self.keys, rows.keys),
            items: std::mem::replace(&mut self.items, rows.items),
        }
    }

    /// Goto Line's number, or `:N` in Goto Anything: a line jump, not a search. Elsewhere (the
    /// palette, Goto Symbol) a `:` is just a char, as in Sublime.
    fn line_mode(&self) -> bool {
        self.kind == Kind::GotoLine || self.kind == Kind::Files && self.query.starts_with(':')
    }

    pub fn line_query(&self) -> Option<usize> {
        let q = self.query.trim().trim_start_matches(':').trim();
        q.parse().ok().filter(|_| self.line_mode())
    }

    pub fn refilter(&mut self, query: &str) {
        self.query = query.to_string();
        self.filtered = if self.line_mode() { Vec::new() } else { fuzzy::filter(query, &self.keys) };
        self.selected = 0;
    }

    /// What the empty input says it is for, as Zed's pickers say; the palette's kind also lists
    /// branches, layouts and folders, so it just says Search.
    fn placeholder(&self) -> &'static str {
        match self.base {
            Kind::Files => "Go to file, @symbol or :line…",
            Kind::Symbols => "Go to symbol…",
            Kind::ProjectSymbols => "Go to symbol in project…",
            Kind::Definitions | Kind::References => "Filter locations…",
            Kind::GotoLine => "Go to line…",
            Kind::Bookmarks => "Find bookmark by file or text…",
            Kind::Tabs(_) => "Switch to a tab…",
            Kind::ClipboardHistory => "Paste from history…",
            Kind::Spelling => "Spelling suggestions…",
            Kind::Commands | Kind::Choices => "Search…",
        }
    }

    pub(crate) fn diagnostic_detail(&self) -> Option<(usize, &str)> {
        if !self.diagnostic_details {
            return None;
        }
        let index = *self.filtered.get(self.selected)?;
        Some((index, self.items.get(index)?.label.as_str()))
    }

    pub(crate) fn selected_detail(&self) -> Option<(usize, std::borrow::Cow<'_, str>)> {
        if self.choice_details && self.kind == Kind::Choices {
            let index = *self.filtered.get(self.selected)?;
            let item = self.items.get(index)?;
            return Some((index, format!("{}\n{}", item.label, item.detail).into()));
        }
        self.diagnostic_detail().map(|(index, detail)| (index, detail.into()))
    }

    pub fn render(&self, window: &Window, top: f32, cx: &mut Context<Workspace>) -> AnyElement {
        let two_line = matches!(
            self.kind,
            Kind::Files
                | Kind::Bookmarks
                | Kind::ClipboardHistory
                | Kind::Tabs(_)
                | Kind::ProjectSymbols
                | Kind::Definitions
                | Kind::References
        ) || self.kind == Kind::Choices && self.choice_details;
        // Sublime's row heights, as many rows as fit its 250pt list
        let (row_h, visible) = match self.kind {
            _ if two_line => (41., 6),
            Kind::Symbols => (23., 11),
            _ => (25., 10),
        };
        // Ordinary pickers retain their existing layout and avoid building full details.
        let detail_size = (self.diagnostic_details || self.choice_details).then(|| {
            let viewport = window.viewport_size();
            ((viewport.width - px(24.)).max(px(1.)).min(px(600.)), (viewport.height - px(top + 8.)).max(px(1.)))
        });
        let chrome = 46. + if self.title.is_some() { 28. } else { 0. } + if self.progress.is_some() { 24. } else { 0. };
        let detail_height = detail_size
            .map_or(px(0.), |(_, height)| (height * 0.45).min(px(160.)).min((height - px(chrome + row_h)).max(px(1.))));
        let visible = detail_size.map_or(visible, |(_, height)| {
            (((f32::from((height - detail_height - px(chrome)).max(px(0.))) / row_h).floor() as usize).max(1))
                .min(visible)
        });
        // a history row's detail is a caption, not what the query matched
        let detail_query = if self.kind == Kind::Files { self.query.as_str() } else { "" };
        let (start, end) = window_rows(self.selected, visible, self.filtered.len());
        let rows = self.filtered[start..end]
            .iter()
            .enumerate()
            .map(|(i, &idx)| {
                let item = &self.items[idx];
                let row_index = start + i;
                let selected = row_index == self.selected;
                let (label_fg, path_fg) = if selected {
                    (theme::QP_LABEL_SELECTED, theme::QP_PATH_SELECTED)
                } else {
                    (theme::QP_LABEL, theme::QP_PATH)
                };
                // Goto Symbol's badge: the kind's letter on a tile tinted its colour
                let badge = item.badge.map(|(letter, colour)| {
                    div()
                        .flex_shrink_0()
                        .size(px(17.))
                        .mr(px(8.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(kit::RADIUS))
                        .bg(c(colour).opacity(if selected { 0.28 } else { 0.18 }))
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(c(colour))
                        .child(letter.to_string())
                });
                div()
                    .id(("qp", i))
                    .debug_selector(move || format!("qp-row-{idx}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(row_h))
                    .px(px(8.))
                    .rounded(px(kit::RADIUS))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(ui(theme::QP_SELECTED_BG)))
                    .when(!selected, |d| d.hover(|s| s.bg(ui(theme::ELEMENT_HOVER).opacity(0.6))))
                    .children(badge)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .debug_selector(move || format!("qp-label-{idx}"))
                                    .text_size(px(13.))
                                    .text_color(ui(label_fg))
                                    .overflow_hidden()
                                    .child(matched(&self.query, &item.label)),
                            )
                            .when(two_line, |d| {
                                d.child(div().text_size(px(11.)).text_color(ui(path_fg)).overflow_hidden().child(
                                    matched(
                                        detail_query,
                                        if self.choice_details {
                                            item.detail.lines().next().unwrap_or("")
                                        } else {
                                            &item.detail
                                        },
                                    ),
                                ))
                            }),
                    )
                    .when(!item.hint.is_empty(), |d| {
                        d.child(
                            div()
                                .debug_selector(move || format!("qp-hint-{idx}"))
                                .pl(px(12.))
                                .flex_shrink_0()
                                .text_size(px(12.))
                                .text_color(ui(path_fg))
                                // a symbol's syntax is italic, a key binding is not
                                .when(item.badge.is_some(), |d| d.italic())
                                .when(item.hint == "def", |d| {
                                    d.px(px(5.))
                                        .py(px(2.))
                                        .rounded(px(kit::RADIUS))
                                        .bg(ui(theme::QP_MATCH).opacity(0.18))
                                })
                                .child(item.hint.clone()),
                        )
                    })
                    .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| {
                        if let Some(p) = &mut ws.quick_panel {
                            p.selected = row_index;
                        }
                        ws.confirm_quick_panel(window, cx);
                    }))
            })
            .collect::<Vec<_>>();
        let nothing = rows.is_empty() && !self.query.trim().is_empty() && !self.line_mode();
        // under the tab strip, centred over the editor column, as Sublime places it
        div()
            .absolute()
            .top(px(4.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .children(
                        self.title
                            .as_ref()
                            .map(|title| div().px(px(12.)).py(px(6.)).text_size(px(12.)).child(title.clone())),
                    )
                    .key_context("QuickPanel")
                    // the views under it take no clicks or hover through it
                    .occlude()
                    .w(px(600.))
                    .when_some(detail_size, |d, (width, height)| d.w(width).max_w(gpui::relative(1.)).max_h(height))
                    .flex()
                    .flex_col()
                    .bg(ui(theme::OVERLAY_BG))
                    .rounded(px(kit::OVERLAY_RADIUS))
                    .border_1()
                    .border_color(ui(theme::MENU_BORDER))
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        div()
                            .relative()
                            .h(px(36.))
                            .py(px(10.))
                            .px(px(12.))
                            .border_b_1()
                            .border_color(ui(theme::BORDER_VARIANT))
                            .child(self.input.clone())
                            .when(self.query.is_empty(), |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left(px(12.))
                                        .top(px(10.))
                                        .text_size(px(13.))
                                        .text_color(ui(theme::PLACEHOLDER_FG))
                                        .child(self.placeholder()),
                                )
                            }),
                    )
                    .children(self.progress.as_ref().map(|text| {
                        div()
                            .px(px(12.))
                            .py(px(4.))
                            .text_size(px(11.))
                            .text_color(ui(theme::TEXT_MUTED))
                            .child(text.clone())
                    }))
                    .when(!rows.is_empty() || nothing, |d| {
                        d.child(div().flex().flex_col().p(px(4.)).children(rows).when(nothing, |d| {
                            d.child(
                                div()
                                    .h(px(28.))
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .text_size(px(13.))
                                    .text_color(ui(theme::TEXT_MUTED))
                                    .child("No matches"),
                            )
                        }))
                    })
                    .children(self.selected_detail().map(|(index, detail)| {
                        let selector = if self.choice_details { "choice-detail" } else { "diagnostic-detail" };
                        let text_selector =
                            if self.choice_details { "choice-detail-text" } else { "diagnostic-detail-text" };
                        div()
                            .id((selector, index))
                            .debug_selector(move || selector.to_string())
                            .max_h(detail_height)
                            .flex_shrink_0()
                            .overflow_y_scroll()
                            .border_t_1()
                            .border_color(ui(theme::BORDER_VARIANT))
                            .px(px(12.))
                            .py(px(8.))
                            .child(
                                div()
                                    .debug_selector(move || text_selector.to_string())
                                    .w_full()
                                    .whitespace_normal()
                                    .text_size(px(12.))
                                    .text_color(ui(theme::UI_FG))
                                    .child(detail.to_string()),
                            )
                    })),
            )
            .into_any_element()
    }
}

/// The rows shown, keeping `selected` in view; a stale `selected` past the end shows the last.
fn window_rows(selected: usize, visible: usize, len: usize) -> (usize, usize) {
    let start = selected.min(len.saturating_sub(1)).saturating_sub(visible - 1);
    (start, (start + visible).min(len))
}

/// `text` with the letters the query matched picked out in the accent colour, as Zed's are.
pub(crate) fn matched(query: &str, text: &str) -> StyledText {
    let style = HighlightStyle {
        color: Some(ui(theme::QP_MATCH)),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    let query = query.trim_start_matches(['@', ':']);
    StyledText::new(text.to_string()).with_highlights(fuzzy::positions(query, text).into_iter().map(|r| (r, style)))
}

/// Every file under `folder` (off the UI thread: it reads the disk); `file_item` makes the rows.
pub fn file_paths(folder: &Path, excludes: &crate::config::project::Excludes) -> Vec<PathBuf> {
    fn walk(dir: &Path, x: &crate::config::project::Excludes, out: &mut Vec<PathBuf>) {
        // files of a folder come before its sub-folders, like Sublime lists them
        let (dirs, files): (Vec<_>, Vec<_>) = sidebar::entries(dir)
            .into_iter()
            .filter(|(p, is_dir)| !x.hides(p, *is_dir))
            .partition(|(_, is_dir)| *is_dir);
        out.extend(files.into_iter().map(|(path, _)| path));
        for (path, _) in dirs {
            if out.len() >= 20_000 {
                return; // hard cap; index in the background if huge trees matter
            }
            if path.file_name().is_some_and(|n| n == "target" || n == "node_modules") {
                continue;
            }
            // a linked folder is not walked: one pointing up the tree would list it again and
            // again until the cap
            if path.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
                continue;
            }
            walk(&path, x, out);
        }
    }
    #[cfg(test)]
    WALKS.set(WALKS.get() + 1);
    let mut out = Vec::new();
    walk(folder, excludes, &mut out);
    out
}

/// Goto Anything's files of every root, each with its root's index, in root order, as real
/// paths (a root reached through a link is resolved once, not each file). A file reached
/// through two roots (one inside the other, or a link to the same folder) is listed once,
/// under the first.
pub fn root_file_paths(roots: &[crate::config::project::Root]) -> Vec<(usize, PathBuf)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (i, r) in roots.iter().enumerate().filter(|(_, r)| !r.missing) {
        let real = crate::editing::text::resolve(&r.path);
        for p in file_paths(&r.path, &r.excludes()) {
            let p = match p.strip_prefix(&r.path) {
                Ok(rest) => real.join(rest),
                Err(_) => p,
            };
            if seen.insert(p.clone()) {
                out.push((i, p));
            }
        }
    }
    out
}

#[cfg(test)]
thread_local! {
    /// Folder walks made on this thread, for a test to see where one ran.
    pub static WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A file's row: its name, and its path from `root`, under the root's label when the window
/// has several roots (`label`).
pub fn file_item(root: &Path, label: Option<&str>, path: PathBuf) -> Item {
    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy();
    Item::new(
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        match label {
            Some(l) if path.starts_with(root) => format!("{l}/{rel}"),
            _ => rel.into_owned(),
        },
        ItemAction::OpenFile(path),
    )
}

/// Goto Symbol: every definition in the buffer, in file order.
pub fn symbol_items(symbols: &[IndexedSymbol], syntax: &str, view: gpui::EntityId) -> Vec<Item> {
    (symbols.iter()).map(|s| symbol_item(s, Place::View(view), String::new(), syntax)).collect()
}

/// Goto Symbol in Project's row for `s`, in `place` shown as `path`: its kind's badge, its
/// name, and `path:line` under it; the name is selected at the place.
pub fn project_symbol_item(s: &IndexedSymbol, place: &Place, path: &str) -> Item {
    symbol_item(s, place.clone(), format!("{path}:{}", s.line + 1), "")
}

/// A row for `s` with its kind's badge, selecting its name at `place`.
fn symbol_item(s: &IndexedSymbol, place: Place, detail: String, hint: &str) -> Item {
    let name = s.col..s.col + s.name.chars().count();
    Item {
        hint: hint.into(),
        badge: Some(highlight::symbol_badge(&s.kind)),
        ..Item::new(s.name.clone(), detail, ItemAction::Location(place, s.line, name))
    }
}

/// Paste from History's rows, newest first, each pasting into `input`; with nothing copied yet
/// one row that does nothing (its entry has no text).
pub fn history_items(entries: &[ClipEntry], input: Option<WeakEntity<Editor>>) -> Vec<Item> {
    let row = |label: String, detail: String, e: ClipEntry| {
        Item::new(label, detail, ItemAction::PasteHistory(e, input.clone()))
    };
    if entries.is_empty() {
        let none = ClipEntry::default();
        return vec![row("Clipboard history is empty".into(), String::new(), none)];
    }
    (entries.iter()).map(|e| row(e.label(), e.detail(), e.clone())).collect()
}

pub fn command_items() -> Vec<Item> {
    use crate::workspace::command_entries as shared;
    fn cmd(label: &str, a: impl Action) -> Item {
        Item::command(label, "", a)
    }
    fn keyed(label: &str, a: impl Action, hint: impl Into<String>) -> Item {
        Item { hint: hint.into(), ..cmd(label, a) }
    }
    let sort = |case_sensitive| Permute::Sort { case_sensitive };
    let marked = |label: &str, op| cmd(label, editor::BookmarkedLines { op });
    let tab_list = |scope| workspace::ShowTabList { scope };
    let move_panel = |dock| workspace::MovePanel { dock };
    let lines = |label: &str, op: Permute, hint: &'static str| {
        keyed(&format!("Permute Lines: {label}"), editor::PermuteLines { op }, hint)
    };
    let sels =
        |label: &str, op: Permute| cmd(&format!("Permute Selections: {label}"), editor::PermuteSelections { op });
    let column = |label: &str, op| cmd(&format!("Edit: {label}"), editor::ColumnCommand { op });
    let transform =
        |label: &str, a: Box<dyn Action>| Item::new(format!("Transform: {label}"), "", ItemAction::Dispatch(a));
    let text_op = |label: &str, op| transform(label, Box::new(editor::TransformText { op }));
    let affix = |label: &str, suffix, remove| transform(label, Box::new(workspace::AffixLines { suffix, remove }));
    let delimiter =
        |label: &str, delim| cmd(&format!("Edit: Column Delimiter: {label}"), workspace::SetDelimiter { delim });
    let mut v = vec![
        cmd("File: Save with Encoding…", workspace::SaveWithEncoding),
        cmd("File: Reopen with Encoding…", workspace::ReopenWithEncoding),
        cmd("Preferences: Key Bindings", workspace::OpenKeybindings),
        cmd("Preferences: Reload Customizations", workspace::ReloadCustomizations),
        cmd("Preferences: Color Scheme", workspace::ChooseColorScheme),
        cmd("Preferences: Theme", workspace::ChooseAppearance),
        cmd("Preferences: Open Themes Folder", workspace::OpenThemesFolder),
        cmd("Preferences: Open Syntaxes Folder", workspace::OpenSyntaxFolder),
        cmd("Indentation: Guess from Buffer", workspace::GuessIndentation),
        keyed("Tools: Run Task…", workspace::RunTask, "⌘B"),
        cmd("Tools: Edit User Tasks", workspace::EditTasks),
        cmd("Tools: Edit Project Tasks", workspace::EditProjectTasks),
        cmd("Tools: Filter Through Shell…", workspace::FilterThroughShell),
        cmd("Tools: Keep Selections Through Shell…", workspace::KeepSelectionsThroughShell),
        cmd("Tools: Run Shell Command…", workspace::RunShellCommand),
        keyed("Terminal: Show / Hide", workspace::ShowTerminal, "⌃`"),
        keyed("Terminal: Close", workspace::CloseTerminal, "⌃⇧`"),
        cmd("Terminal: Find Output…", workspace::FindTerminalOutput),
        cmd("Terminal: New", workspace::NewTerminal),
        cmd("Terminal: New with Profile…", workspace::NewTerminalWithProfile),
        cmd("Terminal: Set Scrollback…", workspace::SetTerminalScrollback),
        cmd("Terminal: Rename…", workspace::RenameTerminal),
        cmd("Terminal: Next", workspace::NextTerminal),
        cmd("Terminal: Previous", workspace::PreviousTerminal),
        cmd("Terminal: Send Selection", workspace::SendSelectionToTerminal),
        cmd("Terminal: Sync Folder", workspace::SyncTerminalFolder),
        cmd("Terminal: Send and Run", workspace::ExecuteSelectionInTerminal),
        cmd("Terminal: Previous Command", workspace::PreviousTerminalCommand),
        cmd("Terminal: Next Command", workspace::NextTerminalCommand),
        cmd("Terminal: Copy Command Output", workspace::CopyTerminalCommandOutput),
        cmd("Terminal: Copy Last Output", workspace::CopyLastTerminalOutput),
        cmd("Terminal: Live Output", workspace::TerminalLiveOutput),
        keyed("Git: Show / Hide Sidebar", workspace::ShowGit, "⌃⇧G"),
        cmd("Git: Set Up Git…", workspace::GitInit),
        cmd("Git: Refresh", workspace::GitRefresh),
        cmd("Git: Stage File", workspace::GitStage),
        cmd("Git: Unstage File", workspace::GitUnstage),
        cmd("Git: Commit Staged…", workspace::GitCommit),
        cmd("Git: Amend HEAD…", workspace::GitAmend),
        cmd("Git: Fetch…", workspace::GitFetch),
        cmd("Git: Pull…", workspace::GitPull),
        cmd("Git: Push…", workspace::GitPush),
        cmd("Git: Remotes & Upstream…", workspace::GitRemotes),
        cmd("Git: Worktrees…", workspace::GitWorktrees),
        cmd("Git: Create Worktree…", workspace::GitCreateWorktree),
        cmd("Git: Prune Worktrees…", workspace::GitPruneWorktrees),
        cmd("Git: Local Tags…", workspace::GitTags),
        cmd("Git: Create Local Tag…", workspace::GitCreateTag),
        cmd("Git: Stashes…", workspace::GitStashes),
        cmd("Git: Create Stash…", workspace::GitCreateStash),
        cmd("Git: Switch Branch…", workspace::GitSwitchBranch),
        cmd("Git: Create Local Branch…", workspace::GitCreateBranch),
        cmd("Git: Rename Local Branch…", workspace::GitRenameBranch),
        cmd("Git: Delete Local Branch…", workspace::GitDeleteBranch),
        cmd("Git: Stage Selected Lines", workspace::GitStageLines),
        cmd("Git: Stage Hunk", workspace::GitStageHunk),
        cmd("Git: Unstage Hunk", workspace::GitUnstageHunk),
        cmd("Git: File History…", workspace::GitFileHistory),
        cmd("Git: Blame File", workspace::GitBlame),
        cmd("Compare: Apply Left to Right", workspace::ApplyLeftToRight),
        cmd("Compare: Apply Right to Left", workspace::ApplyRightToLeft),
        cmd("Compare: Refresh", workspace::RefreshComparison),
        cmd("Compare: Next Difference", workspace::NextDifference),
        cmd("Compare: Previous Difference", workspace::PreviousDifference),
        keyed("Diff: Show Change", workspace::ShowChange, "⌘K ⌘H"),
        keyed("Diff: Next Change", workspace::NextChange, "⌥↓"),
        keyed("Diff: Previous Change", workspace::PreviousChange, "⌥↑"),
        cmd("Diff: Revert Reviewed Hunk", workspace::RevertChange),
        cmd("Git: Next Conflict", workspace::NextConflict),
        cmd("Git: Accept Ours", workspace::AcceptOurs),
        cmd("Git: Accept Theirs", workspace::AcceptTheirs),
        cmd("Git: Accept Both", workspace::AcceptBoth),
        cmd("Tools: New Shell Worksheet", workspace::NewShellWorksheet),
        cmd("Tools: Run Worksheet Selection / Line", workspace::RunWorksheet),
        cmd("Tools: Developer Console", workspace::DeveloperConsole),
        cmd("Tools: Manage Packages…", workspace::ManagePackages),
        cmd("Tools: Install Package…", workspace::InstallPackage),
        keyed("Tools: Cancel Running Command", workspace::CancelAutomation, "⌘."),
        keyed("File: New File", workspace::NewFile, "⌘N"),
        keyed("File: Open…", workspace::Open, "⌘O"),
        keyed("File: Reopen Closed File", workspace::ReopenClosedFile, "⇧⌘T"),
        keyed("File: Save", workspace::Save, "⌘S"),
        cmd("File: Save Without Formatting", workspace::SaveWithoutFormatting),
        keyed("File: Save As…", workspace::SaveAs, "⇧⌘S"),
        cmd("File: Save a Copy As…", workspace::SaveCopyAs),
        keyed("File: Save All", workspace::SaveAll, "⌥⌘S"),
        cmd("File: Print…", workspace::Print),
        keyed("File: New Window", workspace::NewWindow, "⇧⌘N"),
        keyed("File: Close Window", workspace::CloseWindow, "⇧⌘W"),
        keyed("File: Close File", workspace::CloseTab, "⌘W"),
        cmd("File: New View into File", workspace::NewViewIntoFile),
        cmd("File: Revert File", workspace::Revert),
        cmd("File: Follow Log", workspace::FollowLog),
        cmd("File: Stop Following Log", workspace::StopFollowingLog),
        cmd("File: Close All Files", workspace::CloseAll),
        cmd("Project: Open Project…", workspace::OpenProject),
        keyed("Project: Switch Project…", workspace::SwitchProject, "⌃⌘P"),
        cmd("Project: Save Project As…", workspace::SaveProjectAs),
        cmd("Project: Close Project", workspace::CloseProject),
        cmd("Project: Edit Project", workspace::EditProject),
        cmd("Project: Save Workspace As…", workspace::SaveWorkspaceAs),
        cmd("Project: Remove Folder from Project…", workspace::RemoveFolder),
        cmd("Project: Move Folder Up", workspace::MoveFolder { up: true }),
        cmd("Project: Move Folder Down", workspace::MoveFolder { up: false }),
        keyed("Edit: Undo", editor::Undo, "⌘Z"),
        keyed("Edit: Redo", editor::Redo, "⇧⌘Z"),
        keyed("Edit: Cut", editor::Cut, "⌘X"),
        keyed("Edit: Copy", editor::Copy, "⌘C"),
        cmd("Edit: Copy as HTML", editor::CopyAsHtml),
        keyed("Edit: Show Completions", editor::AutoComplete, "⌃Space"),
        keyed("Edit: Paste", editor::Paste, "⌘V"),
        keyed("Edit: Paste and Indent", editor::PasteAndIndent, "⇧⌘V"),
        keyed("Edit: Paste from History", workspace::PasteFromHistory, "⌘K, ⌘V"),
        cmd("Edit: Clear Clipboard History", workspace::ClearClipboardHistory),
        keyed("Edit: Insert Line After", editor::InsertLineAfter, "⌘↩"),
        keyed("Edit: Insert Line Before", editor::InsertLineBefore, "⇧⌘↩"),
        keyed("Edit: Delete to End", editor::DeleteToLineEdge { forward: true }, "⌃K"),
        keyed("Edit: Delete to Beginning", editor::DeleteToLineEdge { forward: false }, "⌘⌫"),
        keyed("Edit: Transpose", editor::Transpose, "⌃T"),
        keyed("Code Folding: Fold", editor::Fold, "⌥⌘["),
        keyed("Code Folding: Unfold", editor::Unfold, "⌥⌘]"),
        keyed("Code Folding: Unfold All", editor::UnfoldAll, "⌘K, ⌘0"),
        keyed("Code Folding: Fold All", editor::FoldLevel { level: 1 }, "⌘K, ⌘1"),
        cmd("Edit: Document Statistics", workspace::DocumentStatistics),
        cmd("Document: Inspect Line Endings…", workspace::InspectLineEndings),
        cmd("Document: Convert Line Endings to LF", workspace::SetLineEnding { ending: "\n" }),
        cmd("Document: Convert Line Endings to CRLF", workspace::SetLineEnding { ending: "\r\n" }),
        cmd("Document: Convert Line Endings to CR", workspace::SetLineEnding { ending: "\r" }),
        cmd("Document: Next LF Boundary", workspace::NextLineEnding { ending: "\n" }),
        cmd("Document: Next CRLF Boundary", workspace::NextLineEnding { ending: "\r\n" }),
        cmd("Document: Next CR Boundary", workspace::NextLineEnding { ending: "\r" }),
        cmd("Edit: Character Info", workspace::CharacterInfo),
        cmd("Edit: Insert Unicode Character", workspace::InsertUnicodeCharacterPrompt),
        cmd("Edit: Insert Literal Next Key", editor::InsertLiteralNextKey),
        cmd("Edit: Character Viewer", workspace::ShowCharacterViewer),
        cmd("Edit: Column Editor…", workspace::ColumnEditor),
        cmd("Edit: Pad to Column…", workspace::PadToColumn),
        transform("Sort Lines by Pattern…", Box::new(workspace::SortLinesByPattern)),
        transform("Process Duplicate Lines…", Box::new(workspace::ProcessDuplicateLines)),
        transform("Process Lines Matching…", Box::new(workspace::ProcessLinesMatching)),
        affix("Add Line Prefix…", false, false),
        affix("Add Line Suffix…", true, false),
        affix("Remove Line Prefix…", false, true),
        affix("Remove Line Suffix…", true, true),
        text_op("Straighten Quotes", TextOp::StraightenQuotes),
        text_op("Smart Quotes", TextOp::SmartQuotes),
        text_op("Normalize to NFC", TextOp::Normalize(NormForm::Nfc)),
        text_op("Normalize to NFD", TextOp::Normalize(NormForm::Nfd)),
        text_op("Normalize to NFKC", TextOp::Normalize(NormForm::Nfkc)),
        text_op("Normalize to NFKD", TextOp::Normalize(NormForm::Nfkd)),
        text_op("Strip Diacritics", TextOp::StripDiacritics),
        transform("Apply Substitution Table…", Box::new(workspace::ApplySubstitutionTable)),
        column("Select Column", ColumnOp::Select),
        column("Copy Column", ColumnOp::Copy),
        column("Move Column Left", ColumnOp::MoveLeft),
        column("Move Column Right", ColumnOp::MoveRight),
        column("Delete Column", ColumnOp::Delete),
        column("Clear Column", ColumnOp::Clear),
        delimiter("Automatic", None),
        delimiter("Comma", Some(Delim::Comma)),
        delimiter("Tab", Some(Delim::Tab)),
        delimiter("Semicolon", Some(Delim::Semicolon)),
        delimiter("Pipe", Some(Delim::Pipe)),
        keyed("Edit: Wrap Paragraph", editor::WrapLines { width: 0 }, "⌥⌘Q"),
        lines("Sort Lines", sort(false), "F5"),
        lines("Sort Lines (Case Sensitive)", sort(true), "⌃F5"),
        lines("Reverse", Permute::Reverse, ""),
        lines("Unique", Permute::Unique, ""),
        lines("Shuffle", Permute::Shuffle, ""),
        sels("Sort", sort(false)),
        sels("Sort (Case Sensitive)", sort(true)),
        sels("Reverse", Permute::Reverse),
        sels("Unique", Permute::Unique),
        sels("Shuffle", Permute::Shuffle),
        keyed("Edit: Delete Word Backward", editor::DeleteWord { forward: false }, "⌥⌫"),
        keyed("Edit: Delete Word Forward", editor::DeleteWord { forward: true }, "⌥⌦"),
        keyed("Selection: Select All", editor::SelectAll, "⌘A"),
        keyed("Selection: Expand Selection to Line", editor::SelectLine, "⌘L"),
        keyed("Selection: Split Selection into Lines", editor::SplitIntoLines, "⇧⌘L"),
        keyed("Selection: Add Previous Line", editor::SelectLines { forward: false }, "⌃⇧↑"),
        keyed("Selection: Add Next Line", editor::SelectLines { forward: true }, "⌃⇧↓"),
        keyed("Selection: Quick Add Next", editor::SelectNextOccurrence, "⌘D"),
        keyed("Selection: Quick Find All", editor::SelectAllOccurrences, "⌃⌘G"),
        keyed("Selection: Quick Skip Next", editor::QuickSkipNext, "⌘K, ⌘D"),
        keyed("Selection: Undo Selection", editor::UndoSelection, "⌘U"),
        keyed("Selection: Redo Selection", editor::RedoSelection, "⇧⌘U"),
        keyed("Selection: Single Selection", editor::SingleSelection, "⎋"),
        keyed("Selection: Expand Selection", editor::ExpandSmart, "⇧⌘A"),
        keyed("Selection: Expand Selection to Line Upward", editor::SelectLineUpward, "⌥⌘L"),
        cmd("Selection: Expand Selection to Block", editor::ExpandToParagraph),
        keyed("Selection: Expand Selection to Brackets", editor::ExpandToBrackets, "⌃⇧M"),
        cmd("Selection: Expand Selection to Indentation", editor::ExpandToIndentation),
        keyed("Find: Find…", workspace::Find, "⌘F"),
        keyed("Find: Replace…", workspace::Replace, "⌥⌘F"),
        keyed("Find: Find Next", workspace::FindNext, "⌘G"),
        keyed("Find: Find Previous", workspace::FindPrev, "⇧⌘G"),
        cmd("Find: Find All", workspace::FindAll),
        cmd("Text: Trim Trailing Whitespace", editor::TrimTrailingWhitespace),
        cmd("Text: Remove Blank Lines", editor::RemoveBlankLines),
        keyed("Find: Next Result", workspace::NextResult, "F4"),
        keyed("Find: Previous Result", workspace::PrevResult, "⇧F4"),
        cmd("Find: Toggle Regular Expression", workspace::ToggleFindRegex),
        cmd("Find: Toggle in Selection", workspace::ToggleFindSelection),
        keyed("Find: Replace Next", crate::chrome::find_panel::ReplaceOne, "⌥⌘E"),
        keyed("Find: Quick Find", editor::FindUnder, "⌥⌘G"),
        keyed("Find: Use Selection for Find", workspace::UseSelectionForFind, "⌘E"),
        keyed("Find: Use Selection for Replace", workspace::UseSelectionForReplace, "⇧⌘E"),
        keyed("Goto: Goto Anything…", workspace::GotoAnything, "⌘P"),
        keyed("Goto: Goto Symbol…", workspace::GotoSymbol, "⌘R"),
        keyed("Goto: Switch File", workspace::SwitchFile, "⌘⌥↑"),
        cmd("Goto: Related Files…", workspace::RelatedFiles),
        keyed("Goto: Goto Symbol in Project…", workspace::GotoSymbolInProject, "⇧⌘R"),
        keyed("Goto: Goto Definition", workspace::GotoDefinition, "F12"),
        cmd("Goto: Goto Definition to Side", workspace::GotoDefinitionToSide),
        keyed("Goto: Goto Reference", workspace::GotoReference, "⇧F12"),
        cmd("LSP: Complete", workspace::LspCompletion),
        cmd("LSP: Find References", workspace::LspReferences),
        cmd("LSP: Go to Type Definition", workspace::LspTypeDefinition),
        cmd("LSP: Go to Type Definition to Side", workspace::LspTypeDefinitionToSide),
        cmd("LSP: Go to Implementation", workspace::LspImplementation),
        cmd("LSP: Go to Implementation to Side", workspace::LspImplementationToSide),
        keyed("LSP: Show Documentation", workspace::LspHover, "⌃K ⌃I"),
        keyed("LSP: Rename Symbol…", workspace::LspRename, "F2"),
        cmd("LSP: Signature Help", workspace::LspSignatureHelp),
        keyed("LSP: Format Document", workspace::LspFormat, "⌥⇧F"),
        cmd("LSP: Format Selection", workspace::LspFormatSelection),
        keyed("LSP: Code Actions…", workspace::LspCodeActions, "⌃."),
        cmd("LSP: Show Diagnostics", workspace::LspDiagnostics),
        cmd("LSP: Restart Language Servers", workspace::RestartLanguageServers),
        keyed("Goto: Goto Line…", workspace::GotoLine, "⌃G"),
        keyed("Goto: Jump Back", editor::JumpBack, "⌃-"),
        keyed("Goto: Jump Forward", editor::JumpForward, "⌃⇧-"),
        keyed("Goto: Jump to Matching Bracket", editor::JumpToBracket, "⌃M"),
        keyed("Goto: Scroll to Selection", editor::ShowAtCenter, "⌃L"),
        keyed("Goto: Scroll Line Up", editor::ScrollLines { up: true }, "⌃⌥↑"),
        keyed("Goto: Scroll Line Down", editor::ScrollLines { up: false }, "⌃⌥↓"),
        keyed("Goto: Toggle Bookmark", editor::ToggleBookmark, "⌘F2"),
        cmd("Goto: Next Bookmark", editor::NextBookmark { forward: true }),
        keyed("Goto: Prev Bookmark", editor::NextBookmark { forward: false }, "⇧F2"),
        keyed("Goto: Clear Bookmarks", editor::ClearBookmarks, "⇧⌘F2"),
        keyed("Goto: Select All Bookmarks", editor::SelectAllBookmarks, "⌥F2"),
        keyed("Goto: Next File", workspace::NextTab, "⇧⌘]"),
        keyed("Goto: Previous File", workspace::PrevTab, "⇧⌘["),
        keyed("View: Toggle Side Bar", workspace::ToggleSidebar, "⌘K, ⌘B"),
        cmd("View: Toggle Minimap", workspace::ToggleMinimap),
        cmd("View: Show Outline", workspace::ToggleOutline),
        cmd("View: Toggle Tabs", workspace::ToggleTabs),
        cmd("View: Toggle Status Bar", workspace::ToggleStatusBar),
        cmd("View: Toggle White Space", workspace::ToggleWhiteSpace),
        cmd("Word Wrap: Toggle", workspace::ToggleWordWrap),
        keyed("View: Spell Check", editor::ToggleSpellCheck, "F6"),
        cmd("Edit: Toggle Overtype", editor::ToggleOvertype),
        keyed("Spelling: Next Misspelling", editor::NextMisspelling, "⌃F6"),
        keyed("Spelling: Previous Misspelling", editor::PrevMisspelling, "⌃⇧F6"),
        cmd("Spelling Suggestions…", editor::SpellingSuggestions),
        keyed("Find: Find in Files…", workspace::FindInFiles, "⇧⌘F"),
        cmd("Find: Find in Side Bar Selection…", workspace::FindInSidebarSelection),
        cmd("Find: Replace in Files…", workspace::ReplaceInFiles),
        keyed("Find: Toggle Case Sensitive", workspace::ToggleFindCase, "⌥⌘C"),
        keyed("Find: Toggle Whole Word", workspace::ToggleFindWord, "⌥⌘W"),
        keyed("Find: Toggle Preserve Case", workspace::TogglePreserveCase, "⌥⌘A"),
        cmd("Find in Files: Toggle Show Context", workspace::ToggleShowContext),
        cmd("Find in Files: Toggle Use Buffer", workspace::ToggleUseBuffer),
        cmd("Find in Files: Toggle Use gitignore", workspace::ToggleUseGitignore),
        cmd("Find in Files: Choose Where…", workspace::ChooseWhere),
        cmd("Find: Search Results", workspace::ShowResults),
        cmd("Find: Regex Playground", workspace::OpenPlayground),
        cmd("Find: Regex Playground with a Copy of the Document", workspace::OpenPlaygroundWithDocument),
        cmd("Find: Refresh Search Results", workspace::RefreshResults),
        cmd("Find: Apply Replacements", workspace::ApplyReplacements),
        cmd("Find: Cancel Replacements", workspace::CancelReplacements),
        keyed("Find: Search History…", workspace::ShowFieldHistory, "⌥↓"),
        cmd("Find: Clear Search History", workspace::ClearSearchHistory),
        cmd("Bookmarks: Bookmark Matching Lines", workspace::BookmarkMatches),
        cmd("Bookmarks: Find Bookmark Across Open Files…", workspace::GotoBookmark),
        marked("Bookmarks: Invert", editor::MarkedLines::Invert),
        marked("Bookmarks: Copy Bookmarked Lines", editor::MarkedLines::Copy),
        marked("Bookmarks: Cut Bookmarked Lines", editor::MarkedLines::Cut),
        marked("Bookmarks: Delete Bookmarked Lines", editor::MarkedLines::Delete),
        marked("Bookmarks: Keep Only Bookmarked Lines", editor::MarkedLines::KeepOnly),
        marked("Bookmarks: Paste to Bookmarked Lines", editor::MarkedLines::Paste),
        cmd("Find: Cancel Find in Files", workspace::CancelFindInFiles),
        keyed("View: Toggle Full Screen", workspace::ToggleFullScreen, "⌃⌘F"),
        cmd("Indentation: Convert to Spaces", editor::ConvertIndentation { to_spaces: true }),
        cmd("Indentation: Indent Using Spaces", workspace::SetIndentStyle { to_spaces: true }),
        cmd("Indentation: Indent Using Tabs", workspace::SetIndentStyle { to_spaces: false }),
        cmd("Indentation: Convert to Tabs", editor::ConvertIndentation { to_spaces: false }),
        cmd("Project: Add Folder to Project…", workspace::AddFolder),
        cmd("Project: Remove all Folders from Project", workspace::RemoveFolders),
        cmd("Project: Refresh Folders", workspace::RefreshFolders),
        keyed("Preferences: Settings", workspace::OpenSettings, "⌘,"),
        keyed("Preferences: Font: Larger", workspace::IncreaseFontSize, "⌘="),
        keyed("Preferences: Font: Smaller", workspace::DecreaseFontSize, "⌘-"),
        cmd("Preferences: Font: Reset Size", workspace::ResetFontSize),
        keyed("Application: Quit", workspace::Quit, "⌘Q"),
        keyed("Tab List: This Window", tab_list(workspace::TabScope::Window), "⌘K, ⌘T"),
        cmd("Tab List: All Windows", tab_list(workspace::TabScope::AllWindows)),
        cmd("View: Toggle Open Files", workspace::ToggleOpenFiles),
        keyed("View: Focus Side Bar", workspace::FocusSideBar, "⌃0"),
        keyed("View: Show / Hide Panel", workspace::TogglePanel, "⌘K, ⌘J"),
        cmd("View: Collapse / Expand Panel", workspace::ToggleCollapsePanel),
        cmd("View: Focus Panel", workspace::FocusPanel),
        cmd("View: Move Panel to Bottom", move_panel(crate::app::session::Dock::Bottom)),
        cmd("View: Move Panel to Right", move_panel(crate::app::session::Dock::Right)),
        keyed("View: Toggle Distraction Free Mode", workspace::ToggleDistractionFree, "⌃⇧⌘F"),
        cmd("Side Bar: New File…", workspace::NewFileHere),
        cmd("Side Bar: New Folder…", workspace::NewFolderHere),
        keyed("Side Bar: Rename…", workspace::RenamePath, "F2"),
        cmd("Side Bar: Move…", workspace::MovePaths),
        cmd("Side Bar: Duplicate…", workspace::DuplicatePath),
        keyed("Side Bar: Move to Trash", workspace::TrashPaths, "⌘⌫"),
        cmd("Side Bar: Reveal in Finder", workspace::RevealInFinder),
        cmd("Side Bar: Find in Folder…", workspace::FindInSidebarItem),
        cmd("View: Toggle Vertical Scroll Sync", workspace::ToggleSyncScroll { vertical: true }),
        cmd("View: Toggle Horizontal Scroll Sync", workspace::ToggleSyncScroll { vertical: false }),
        cmd("Layout: Save Layout…", workspace::SaveLayout),
        cmd("Layout: Apply Saved Layout…", workspace::ChooseSavedLayout { delete: false }),
        cmd("Layout: Delete Saved Layout…", workspace::ChooseSavedLayout { delete: true }),
    ];
    v.extend(
        workspace::WRAP_COLUMNS
            .iter()
            .map(|(label, col)| cmd(&format!("Word Wrap Column: {label}"), workspace::SetWrapColumn { col: *col })),
    );
    // Sublime's palette has Fold All alone; Orlo lists every menu item
    v.extend((2..=9).map(|level| {
        keyed(&format!("Code Folding: Fold Level {level}"), editor::FoldLevel { level }, format!("⌘K, ⌘{level}"))
    }));
    // Sublime's palette has none of these; Orlo lists every menu item
    let layout_keys = ["⌥⌘1", "⌥⌘2", "⌥⌘3", "⌥⌘4", "⇧⌥⌘2", "⇧⌥⌘3", "⌥⌘5"];
    v.extend(workspace::LAYOUTS.iter().zip(layout_keys).map(|((label, preset), key)| {
        keyed(&format!("Layout: {label}"), workspace::SetLayout { preset: *preset }, key)
    }));
    v.extend(
        [1, 2, 3, 4, 8, 16].map(|size| cmd(&format!("Indentation: Tab Width {size}"), workspace::SetTabSize { size })),
    );
    v.extend([
        keyed("Focus Group: Next", workspace::FocusNeighbour { forward: true }, "⌘K, ⌘→"),
        keyed("Focus Group: Previous", workspace::FocusNeighbour { forward: false }, "⌘K, ⌘←"),
        keyed("Move File to Group: Next", workspace::MoveToNeighbour { forward: true }, "⌘K, ⇧⌘→"),
        keyed("Move File to Group: Previous", workspace::MoveToNeighbour { forward: false }, "⌘K, ⇧⌘←"),
    ]);
    let group_keys = [("⌃1", "⌃⇧1"), ("⌃2", "⌃⇧2"), ("⌃3", "⌃⇧3"), ("⌃4", "⌃⇧4")];
    for (g, (focus, mv)) in group_keys.into_iter().enumerate() {
        v.push(keyed(&format!("Focus Group: Group {}", g + 1), workspace::FocusGroup { group: g }, focus));
        v.push(keyed(&format!("Move File to Group: Group {}", g + 1), workspace::MoveToGroup { group: g }, mv));
    }
    v.extend(
        Highlighter::syntax_names()
            .into_iter()
            .map(|name| cmd(&format!("Set Syntax: {name}"), workspace::SetSyntax { name: name.clone() })),
    );
    v.extend(workspace::extract_entries().into_iter().map(|(label, a)| cmd(&format!("Extract: {label}"), a)));
    for (name, items) in workspace::highlight_entries() {
        v.extend(
            items.into_iter().map(|(style, a)| Item::new(format!("{name}: {style}"), "", ItemAction::Dispatch(a))),
        );
    }
    for (prefix, entries) in [
        ("Selection", shared::syntax_selection()),
        ("Edit", shared::lines()),
        ("Edit", shared::date_time()),
        ("Edit", shared::comments()),
        ("Edit", shared::subwords()),
        ("Edit", shared::prose()),
        ("Edit", shared::word_transposition()),
        ("Selection", shared::selection_sets()),
        ("View", shared::view_filters()),
        ("View: Line Numbers", shared::line_numbers(None)),
        ("View", shared::cursor_columns(false)),
        ("Fold Navigation", shared::fold_navigation()),
        ("Goto", shared::selected_targets()),
        ("Edit", shared::surrounds()),
        ("Edit", shared::value_adjustments()),
        ("Edit", shared::field_alignment()),
        ("Edit", shared::marks()),
        ("Find", shared::saved_searches()),
        ("JSON", shared::json()),
        ("Macros", shared::macros()),
        ("Snippets", shared::snippets()),
        ("Explorer", shared::explorer_navigation()),
        ("File", shared::viewer_mode()),
        ("File", shared::templates()),
        ("File", shared::file_history()),
        ("File", shared::discarded_untitled()),
        ("Compare", shared::compare()),
        ("Convert Case", shared::cases()),
        ("Convert", shared::conversions()),
        ("Groups", shared::groups()),
        ("Tab", shared::tabs(None)),
        ("Text Pipelines", shared::pipelines()),
    ] {
        v.extend(entries.into_iter().flatten().map(|(label, action, hint)| {
            let label = match (prefix, label) {
                ("File", "Browse Versions…") => "Browse File History…",
                ("File", "Clear History…") => "Clear File History…",
                _ => label,
            };
            Item { hint: hint.into(), ..Item::new(format!("{prefix}: {label}"), "", ItemAction::Dispatch(action)) }
        }));
    }
    // Sublime lists its commands alphabetically
    v.sort_by_cached_key(|i| i.label.to_lowercase());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goto_anything_does_not_walk_into_linked_folders() {
        let dir = std::env::temp_dir().join("orlo-qp-links");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("a/x.txt"), "").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("a/up")).unwrap();
        let labels: Vec<String> = (file_paths(&dir, &crate::config::project::Excludes::none()).into_iter())
            .map(|p| file_item(&dir, None, p).label)
            .collect();
        assert_eq!(labels, ["x.txt"]);
    }

    /// Asserts the palette has each of `want`: a label and its key hint.
    #[track_caller]
    fn assert_rows(want: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) {
        let rows: Vec<(String, String)> = (command_items().into_iter()).map(|i| (i.label, i.hint)).collect();
        for (label, hint) in want {
            let row = (label.into(), hint.into());
            assert!(rows.contains(&row), "{row:?}");
        }
    }

    /// Asserts the palette has a row labelled each of `want`.
    #[track_caller]
    fn assert_labels(want: &[&str]) {
        let labels: Vec<String> = command_items().into_iter().map(|i| i.label).collect();
        for want in want {
            assert!(labels.iter().any(|l| l == want), "{want}");
        }
    }

    #[test]
    fn syntax_selection_actions_are_reachable_in_the_palette() {
        assert_rows([("Selection: Expand Syntax Selection", ""), ("Selection: Shrink Syntax Selection", "")]);
    }

    #[test]
    fn a_stale_selection_past_the_rows_shows_none_instead_of_panicking() {
        assert_eq!(window_rows(0, 6, 0), (0, 0));
        assert_eq!(window_rows(40, 6, 0), (0, 0));
        assert_eq!(window_rows(40, 6, 3), (0, 3));
        assert_eq!(window_rows(40, 6, 20), (14, 20));
        assert_eq!(window_rows(8, 6, 20), (3, 9));
        assert_eq!(window_rows(2, 6, 20), (0, 6));
    }

    /// The workspace's later F2 binding wins over the editor's at the same context depth, so F2
    /// is LSP rename; the palette must not claim F2 for Next Bookmark too.
    #[gpui::test]
    fn f2_is_lsp_rename_and_the_palette_says_so(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::config::customization::bind_defaults(cx);
            let context = [gpui::KeyContext::parse("Editor").unwrap()];
            let (bindings, _) =
                cx.key_bindings().borrow().bindings_for_input(&[gpui::Keystroke::parse("f2").unwrap()], &context);
            assert!(bindings[0].action().as_any().is::<workspace::LspRename>());
        });
        let claims: Vec<String> = command_items()
            .into_iter()
            .filter(|i| i.hint == "F2" && !i.label.starts_with("Side Bar:"))
            .map(|i| i.label)
            .collect();
        assert_eq!(claims, ["LSP: Rename Symbol…"]);
    }

    #[test]
    fn the_palette_lists_the_clipboard_history_commands() {
        assert_rows([("Edit: Paste from History", "⌘K, ⌘V"), ("Edit: Clear Clipboard History", "")]);
    }

    #[test]
    fn the_palette_shows_completions_on_ctrl_space_and_lsp_complete_has_no_key() {
        assert_rows([("Edit: Show Completions", "⌃Space"), ("LSP: Complete", "")]);
    }

    #[test]
    fn the_palette_lists_insert_snippet_and_new_snippet() {
        assert_rows([
            ("Snippets: Insert Snippet…", ""),
            ("Snippets: New Snippet…", ""),
            ("File: New From Template…", ""),
        ]);
    }

    #[test]
    fn the_palette_lists_copy_as_html_and_it_has_no_key() {
        assert_rows([("Edit: Copy as HTML", "")]);
    }

    #[test]
    fn palette_spell_check_has_the_f6_hint() {
        let item = command_items().into_iter().find(|i| i.label == "View: Spell Check").unwrap();
        assert_eq!(item.hint, "F6");
        assert!(matches!(item.action, ItemAction::Dispatch(a) if a.as_any().is::<editor::ToggleSpellCheck>()));
    }

    #[test]
    fn the_palette_lists_the_code_folding_commands_with_their_keys() {
        assert_rows([
            ("Code Folding: Fold", "⌥⌘["),
            ("Code Folding: Unfold", "⌥⌘]"),
            ("Code Folding: Unfold All", "⌘K, ⌘0"),
            ("Code Folding: Fold All", "⌘K, ⌘1"),
        ]);
        assert_rows((2..=9).map(|n| (format!("Code Folding: Fold Level {n}"), format!("⌘K, ⌘{n}"))));
    }

    #[test]
    fn the_palette_toggles_white_space() {
        assert_labels(&["View: Toggle White Space"]);
    }

    #[test]
    fn the_palette_lists_the_statistics_commands() {
        assert_labels(&["Edit: Document Statistics", "Edit: Character Info"]);
    }

    #[test]
    fn the_palette_lists_the_convert_commands() {
        assert_labels(&[
            "Convert: Base64 Encode",
            "Convert: Base64 Decode",
            "Convert: Base64 URL-safe Encode",
            "Convert: Base64 URL-safe Decode",
            "Convert: Hex Encode",
            "Convert: Hex Decode",
            "Convert: URI Component Encode",
            "Convert: URI Component Decode",
            "Convert: HTML Entity Escape",
            "Convert: HTML Entity Unescape",
            "Convert: Copy MD5",
            "Convert: Copy SHA-1",
            "Convert: Copy SHA-256",
            "Convert: Copy SHA-512",
            "Convert: Hash File…",
        ]);
    }

    #[test]
    fn uri_component_palette_actions_use_the_serialized_codec() {
        for (label, decode) in [("Convert: URI Component Encode", false), ("Convert: URI Component Decode", true)] {
            let item = command_items().into_iter().find(|i| i.label == label).expect("URI conversion in palette");
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch action") };
            let convert = action.as_any().downcast_ref::<editor::Convert>().expect("conversion action");
            assert_eq!(
                serde_json::to_value(convert).unwrap(),
                serde_json::json!({"codec": "uri_component", "decode": decode})
            );
        }
    }

    #[test]
    fn n03_palette_has_named_literal_operations() {
        for (label, codec, decode) in [
            ("Escape Regex", "regex_literal", false),
            ("Quote JSON String", "json_string", false),
            ("Unquote JSON String", "json_string", true),
        ] {
            let item = command_items().into_iter().find(|i| i.label == format!("Convert: {label}")).expect(label);
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch action") };
            let convert = action.as_any().downcast_ref::<editor::Convert>().expect("existing Convert action");
            assert_eq!(serde_json::to_value(convert).unwrap(), serde_json::json!({"codec":codec,"decode":decode}));
        }
        assert_labels(&["Convert: Decode Text Escapes"]);
        let item = command_items().into_iter().find(|i| i.label == "Convert: Decode Text Escapes").unwrap();
        assert!(
            matches!(item.action, ItemAction::Dispatch(action) if action.as_any().is::<editor::DecodeTextEscapes>())
        );
    }

    #[test]
    fn html_entities_palette_actions_use_the_serialized_codec() {
        for (label, decode) in [("Convert: HTML Entity Escape", false), ("Convert: HTML Entity Unescape", true)] {
            let item = command_items().into_iter().find(|i| i.label == label).expect("HTML conversion in palette");
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch action") };
            let convert = action.as_any().downcast_ref::<editor::Convert>().expect("conversion action");
            assert_eq!(
                serde_json::to_value(convert).unwrap(),
                serde_json::json!({"codec": "html_entities", "decode": decode})
            );
        }
    }

    #[test]
    fn the_palette_lists_the_transform_commands() {
        assert_labels(&[
            "Transform: Sort Lines by Pattern…",
            "Transform: Process Duplicate Lines…",
            "Transform: Process Lines Matching…",
            "Transform: Add Line Prefix…",
            "Transform: Add Line Suffix…",
            "Transform: Remove Line Prefix…",
            "Transform: Remove Line Suffix…",
            "Transform: Straighten Quotes",
            "Transform: Smart Quotes",
            "Transform: Normalize to NFC",
            "Transform: Normalize to NFD",
            "Transform: Normalize to NFKC",
            "Transform: Normalize to NFKD",
            "Transform: Strip Diacritics",
            "Transform: Apply Substitution Table…",
        ]);
    }

    #[test]
    fn the_palette_lists_the_column_commands() {
        assert_labels(&[
            "Edit: Column Editor…",
            "Edit: Pad to Column…",
            "Edit: Select Column",
            "Edit: Copy Column",
            "Edit: Move Column Left",
            "Edit: Move Column Right",
            "Edit: Delete Column",
            "Edit: Clear Column",
            "Edit: Column Delimiter: Automatic",
            "Edit: Column Delimiter: Comma",
            "Edit: Column Delimiter: Tab",
            "Edit: Column Delimiter: Semicolon",
            "Edit: Column Delimiter: Pipe",
        ]);
    }

    #[test]
    fn subword_palette_actions_match_all_six_shared_commands() {
        use crate::editing::text::Motion;
        for (label, motion, select) in [
            ("Previous Subword", Motion::SubwordLeft, false),
            ("Next Subword", Motion::SubwordRight, false),
            ("Select Previous Subword", Motion::SubwordLeft, true),
            ("Select Next Subword", Motion::SubwordRight, true),
        ] {
            let item = command_items().into_iter().find(|i| i.label == format!("Edit: {label}")).unwrap();
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch") };
            assert_eq!(action.as_any().downcast_ref::<editor::Move>().unwrap(), &editor::Move { motion, select });
            assert!(item.hint.is_empty(), "no default subword binding");
        }
        for (label, forward) in [("Delete Previous Subword", false), ("Delete Next Subword", true)] {
            let item = command_items().into_iter().find(|i| i.label == format!("Edit: {label}")).unwrap();
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch") };
            let a = action.as_any().downcast_ref::<editor::DeleteSubword>().unwrap();
            assert_eq!(serde_json::to_value(a).unwrap(), serde_json::json!({"forward": forward}));
            assert!(item.hint.is_empty());
        }
    }

    #[test]
    fn prose_palette_exposes_sentence_selection_and_deletion() {
        let missing: Vec<_> = ["Select Sentence", "Delete Sentence"]
            .into_iter()
            .filter(|label| !command_items().iter().any(|i| i.label == format!("Edit: {label}")))
            .collect();
        assert!(missing.is_empty(), "missing prose commands {missing:?}");
    }

    #[test]
    fn prose_palette_actions_match_all_ten_shared_commands_without_defaults() {
        use crate::editing::text::Motion;
        for (label, motion, select) in [
            ("Previous Paragraph", Motion::ParagraphPrevious, false),
            ("Next Paragraph", Motion::ParagraphNext, false),
            ("Select Previous Paragraph", Motion::ParagraphPrevious, true),
            ("Select Next Paragraph", Motion::ParagraphNext, true),
            ("Previous Sentence", Motion::SentencePrevious, false),
            ("Next Sentence", Motion::SentenceNext, false),
            ("Select Previous Sentence", Motion::SentencePrevious, true),
            ("Select Next Sentence", Motion::SentenceNext, true),
        ] {
            let item = command_items().into_iter().find(|i| i.label == format!("Edit: {label}")).unwrap();
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch") };
            assert_eq!(action.as_any().downcast_ref::<editor::Move>().unwrap(), &editor::Move { motion, select });
            assert!(item.hint.is_empty());
        }
        for (label, select) in [("Select Sentence", true), ("Delete Sentence", false)] {
            let item = command_items().into_iter().find(|i| i.label == format!("Edit: {label}")).unwrap();
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch") };
            assert_eq!(action.as_any().is::<editor::SelectSentence>(), select);
            assert_eq!(action.as_any().is::<editor::DeleteSentence>(), !select);
            assert!(item.hint.is_empty());
        }
    }

    #[test]
    fn n07_surrounds_palette_reaches_parameter_forms_without_default_keys() {
        for (label, change) in [("Change Surround…", true), ("Delete Surround…", false)] {
            let item = command_items().into_iter().find(|i| i.label == format!("Edit: {label}")).unwrap();
            let ItemAction::Dispatch(action) = item.action else { panic!("dispatch") };
            assert_eq!(action.as_any().is::<workspace::ChangeSurroundPrompt>(), change);
            assert_eq!(action.as_any().is::<workspace::DeleteSurroundPrompt>(), !change);
            assert!(item.hint.is_empty());
        }
    }

    #[test]
    fn n06_transpose_words_palette_uses_shared_recordable_action_without_default_key() {
        let item = command_items().into_iter().find(|i| i.label == "Edit: Transpose Words").unwrap();
        let ItemAction::Dispatch(action) = item.action else { panic!("dispatch") };
        assert!(action.as_any().is::<editor::TransposeWords>());
        assert!(item.hint.is_empty());
    }
    #[test]
    fn n08_selection_set_palette_uses_native_wrapper_actions_without_default_keys() {
        for label in [
            "Invert Selections",
            "Split Selections by Regex…",
            "Keep Selections Matching Regex…",
            "Remove Matching Selections…",
        ] {
            let item = command_items().into_iter().find(|i| i.label == format!("Selection: {label}")).unwrap();
            assert!(matches!(item.action, ItemAction::Dispatch(_)));
            assert!(item.hint.is_empty());
        }
    }
}
