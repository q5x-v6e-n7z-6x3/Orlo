//! Window state and composition; behavior lives with its owning UI module.

pub(crate) use crate::overlays::context_menu;
pub(crate) use crate::panels;

pub(crate) mod actions;
pub(crate) mod appearance;
pub(crate) mod automation;
mod bookmarks;
pub(crate) mod cli;
pub(crate) mod command_entries;
pub(crate) mod comparison;
pub(crate) mod customization;
pub(crate) mod discarded;
pub(crate) mod disk;
pub(crate) mod distraction;
pub(crate) mod docks;
pub(crate) mod editors;
pub(crate) mod encoding;
pub(crate) mod explorer;
mod file_autosave;
pub(crate) mod fileops;
pub(crate) mod files;
mod follow;
pub(crate) mod goto;
pub(crate) mod groups;
pub(crate) mod history;
pub(crate) mod index;
pub(crate) mod json;
pub(crate) mod language;
pub(crate) use language::LSP_WAITING;
pub(crate) mod menus;
pub(crate) mod opening;
pub(crate) mod panel;
pub(crate) mod pickers;
pub(crate) mod pipelines;
pub(crate) mod preferences;
mod printing;
pub(crate) mod projects;
pub(crate) mod rendering;
pub(crate) mod roots;
mod selected_target;
pub(crate) mod sessions;
pub(crate) mod snippets;
pub(crate) mod spelling;
pub(crate) mod tabs;
pub(crate) mod tools;
pub(crate) mod transforms;
pub(crate) mod view;
pub(crate) mod window;

#[cfg(test)]
thread_local! {
    /// Runs of the views' focus listeners on this thread: one a focus, however a view was wired.
    static FOCUS_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod menu_sweep_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
pub(crate) mod search_tests;
#[cfg(test)]
pub(crate) mod ui_tests;
#[cfg(test)]
pub(crate) mod variant_tests;
pub use actions::*;
pub(crate) use appearance::sync_theme;
pub use appearance::{check_font, load_customizations};
pub use automation::*;
pub use cli::open_request;
pub use comparison::key_bindings as comparison_key_bindings;
pub use comparison::{
    ApplyLeftToRight, ApplyRightToLeft, CompareFiles, CompareFolders, NextDifference, PreviousDifference,
    RefreshComparison,
};
pub use customization::*;
pub use discarded::{
    BrowseDiscardedUntitled, ClearDiscardHistory, DeleteDiscardedUntitled, DeleteRescuePreview,
    PreviewDiscardedUntitled, RestoreDiscardedUntitled, RestoreRescuePreview,
};
pub use disk::SettingsDisk;
pub use encoding::{ReopenWithEncoding, SaveWithEncoding};
pub use extract_ui::{Extract, ExtractFrom, HighlightMatches};
pub(crate) use find::Search;
pub use form::{Field, FieldKind, FormPanel, FormValues, OnDone};
pub use git_ui::*;
pub use history::{key_bindings as history_key_bindings, BrowseFileHistory, ClearFileHistory, RestoreFileHistory};
pub use json::{FormatJson, MinifyJson, ValidateJson};
pub use language::{
    Choice as LanguageChoice, LspCodeActions, LspCompletion, LspDiagnostics, LspFormat, LspFormatSelection, LspHover,
    LspImplementation, LspImplementationToSide, LspReferences, LspRename, LspSignatureHelp, LspTypeDefinition,
    LspTypeDefinitionToSide, RestartLanguageServers,
};
pub use menus::{extract_entries, highlight_entries, LAYOUTS, WRAP_COLUMNS};
pub use panel::Panel;
#[cfg(test)]
pub(crate) use panels::saved_ui;
pub(crate) use panels::{extract_ui, find, form, git_ui, outline, playground, results_panel, terminal};
pub use playground::{key_bindings as playground_key_bindings, OpenPlayground, OpenPlaygroundWithDocument};
pub use results_panel::key_bindings as results_key_bindings;
pub use sessions::*;
pub(crate) use spelling::Choice as SpellingChoice;
pub(crate) use tabs::{DraggedTabs, TabGhost};
pub use tabs::{TabDrag, TabScope};
pub use terminal::{
    CloseTerminal, CopyLastTerminalOutput, CopyTerminalCommandOutput, ExecuteSelectionInTerminal, FindTerminalOutput,
    NewTerminal, NewTerminalWithProfile, NextTerminal, NextTerminalCommand, PreviousTerminal, PreviousTerminalCommand,
    RenameTerminal, SendSelectionToTerminal, SetTerminalScrollback, ShowTerminal, SyncTerminalFolder,
    TerminalLiveOutput,
};
pub use window::*;

pub(crate) use crate::app::session::{self, GroupState, Session, Unsaved, ViewState, WindowState};
pub(crate) use crate::chrome::find_panel::{self, FindPanel};
pub(crate) use crate::chrome::icons::{icon, Icon};
pub(crate) use crate::chrome::layout::{Axis, Groups, Layout, Preset};
pub(crate) use crate::chrome::quick_panel::{self, ItemAction, Kind, QuickPanel};
pub(crate) use crate::chrome::sidebar::{self, SidebarState};
pub(crate) use crate::chrome::theme::{self, c, ui};
pub(crate) use crate::chrome::ui as kit;
pub(crate) use crate::config::settings::{self, Settings};
pub(crate) use crate::documents::{DocumentEvent, Documents, OpenDocument};
pub(crate) use crate::editing::columns;
pub(crate) use crate::editing::convert::{self, Codec, Hash};
pub(crate) use crate::editing::csv::{ColumnOp, Delim};
pub(crate) use crate::editing::stats;
pub(crate) use crate::editing::text::{Case, Change, Document, EditorState, FindOpts, Loaded, Permute};
pub(crate) use crate::editor::{self, doc_of, sel_of, Editor, EditorEvent, ViewSettings};
pub(crate) use crate::languages::highlight::{self, Highlighter};
pub(crate) use crate::searching::find_in_files::{self, FindInFilesPanel};
pub(crate) use crate::searching::pipeline;
pub(crate) use crate::searching::search::{self, tilde};
pub(crate) use gpui::{
    actions, div, point, prelude::*, px, relative, size, Action, AnyElement, App, Bounds, ClickEvent, ClipboardItem,
    Context, CursorStyle, DragMoveEvent, EmptyView, Entity, EntityId, FocusHandle, Focusable, KeyBinding, Menu,
    MenuItem, MouseButton, OsAction, PathPromptOptions, PromptLevel, Render, ScrollHandle, Subscription,
    SystemMenuType, Task, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
};
pub(crate) use std::collections::{HashMap, HashSet};
pub(crate) use std::path::{Path, PathBuf};

/// Files bigger than this (bytes) open in the background: 4 MB reads in about 5 ms.
pub(crate) const LOAD_IN_BACKGROUND: u64 = 4 << 20;
/// Files bigger than this ask first: the read and the buffer take about twice their size.
pub(crate) const ASK_ABOVE: u64 = 1 << 30;
/// File > Open Recent, app-wide and kept in the session, as Sublime keeps them.
#[derive(Default)]
pub struct Recent {
    pub files: Vec<PathBuf>,
    pub folders: Vec<PathBuf>,
    /// Project and workspace files.
    pub projects: Vec<PathBuf>,
}

impl gpui::Global for Recent {}

/// Goto Anything's walk: each file with the index of the root it was found under.
pub(crate) type FileList = std::sync::Arc<Vec<(usize, PathBuf)>>;
/// gpui drag payloads for the side bar edge and the group dividers.
pub(crate) struct SidebarDrag;
#[derive(Clone, Copy)]
pub(crate) struct DividerDrag(pub(crate) Axis, pub(crate) usize);

/// View menu choices that override the settings file in one window until the file is saved.
#[derive(Default)]
pub(crate) struct Overrides {
    pub(crate) font_size: Option<f32>,
    pub(crate) minimap: Option<bool>,
    pub(crate) rulers: Option<Vec<usize>>,
    pub(crate) line_number_mode: Option<crate::config::settings::LineNumberMode>,
    pub(crate) cursor_column_highlight: Option<bool>,
    /// View > Show White Space: `Some(true)` draws all of it, `Some(false)` none (when the
    /// settings say all), `None` follows the settings.
    pub(crate) white_space: Option<bool>,
}

/// What to do once a save completes.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum After {
    Copy,
    Nothing,
    WithoutFormatting,
    Close,
    /// Close this tab, then carry on closing the rest (File > Close All Files).
    CloseAll,
    /// Save every dirty tab in turn, then quit (`true`) or close the window.
    SaveAllThenClose,
    /// Show Save All's next Save As sheet.
    SaveNext,
    /// Close this tab, then the next of the close queue (Close Other Tabs and the like).
    CloseQueue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingOpen {
    Preview,
    Kept,
    Cancelled,
}

pub(crate) struct BrowserJump {
    pub(crate) origin: Option<EntityId>,
    pub(crate) focus: FocusHandle,
    pub(crate) navigation: u64,
}

pub(crate) struct PendingJump {
    pub(crate) line: usize,
    pub(crate) hit: find::Hit,
    pub(crate) selection: Option<(std::ops::Range<usize>, crate::files::watch::Stamp)>,
    pub(crate) browser: Option<BrowserJump>,
}

pub struct Workspace {
    pub(crate) cli: cli::State,
    pub(crate) json_generation: u64,
    pub(crate) spelling_job: Option<spelling::Job>,
    pub(crate) language: language::State,
    /// The project index: this window's folders and open buffers.
    pub index: Entity<crate::languages::index::ProjectIndex>,
    pub(crate) outline: outline::State,
    pub(crate) completion_recent: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<String>>>,
    pub(crate) automation: automation::State,
    pub(crate) terminal_views: Groups<Entity<crate::integrations::terminal::Terminal>>,
    pub(crate) terminal: Option<Entity<crate::integrations::terminal::Terminal>>,
    pub(crate) git: git_ui::State,
    /// The layout and the views in each group; the active group's active view is the one
    /// commands act on.
    pub views: Groups<Entity<Editor>>,
    /// The side bar's folders in order; a root that is not a folder now stays, marked.
    pub roots: Vec<crate::config::project::Root>,
    /// The window's project and its workspace file, if it has one.
    pub project: Option<crate::config::project::Project>,
    pub workspace_file: Option<PathBuf>,
    pub sidebar: SidebarState,
    pub quick_panel: Option<QuickPanel>,
    /// A quick panel's preview is showing a place: a lone blank untitled tab stays, for Escape
    /// to come back to.
    pub(crate) keep_blank_for_preview: bool,
    /// The bottom panel showing, if any (Find / Replace, Find in Files, the results browser...).
    pub panel: Option<Panel>,
    /// Each root's git branch, for the status bar (the active file's root), as Sublime's git
    /// integration shows it; the poll reads them again.
    pub(crate) branches: HashMap<PathBuf, String>,
    /// Real paths `root_of` has resolved, so the status bar's draw does not canonicalize;
    /// dropped when the roots change.
    pub(crate) real_paths: std::cell::RefCell<HashMap<PathBuf, PathBuf>>,
    pub(crate) last_query: String,
    pub(crate) query_history: search::QueryHistory,
    /// The Find panel's replacement and toggles, kept while it is closed (Replace Next still
    /// uses them) and given back when it opens again.
    pub(crate) last_replace: String,
    pub(crate) last_find_opts: FindOpts,
    /// The Find panel's Preserve Case, kept while it is closed.
    pub(crate) last_preserve_case: bool,
    /// The settings file with `overrides` applied; what every tab shows.
    pub view: ViewSettings,
    pub(crate) overrides: Overrides,
    pub(crate) show_tabs: bool,
    pub(crate) show_status_bar: bool,
    pub(crate) closed: Vec<ViewState>,
    pub(crate) focus_handle: FocusHandle,
    /// Each view's listeners in this window (redraws, events, click-to-activate), dropped when
    /// the view closes or moves to another window.
    pub(crate) focus_subs: HashMap<EntityId, Vec<Subscription>>,
    /// The documents this window's tabs show, each followed once (`watch_document`).
    pub(crate) doc_subs: HashMap<u64, Subscription>,
    /// The file watcher's loop, a poll in flight, and a reload sheet open.
    pub(crate) poller: Option<Task<()>>,
    pub(crate) polling: bool,
    /// Open files whose last stat has not returned (a hung network share): not asked again.
    pub(crate) checking: HashSet<PathBuf>,
    pub(crate) asking: bool,
    /// Reloads in flight, one per buffer: the view each started from and its number. A later one
    /// for the same buffer (a Revert) takes its place, and the earlier one is dropped on landing.
    pub(crate) reloads: Vec<(Entity<Editor>, usize)>,
    pub(crate) next_reload: usize,
    /// Save All's Save As sheets still to show, last first, and the Save panel's queue.
    pub(crate) save_sheets: Vec<Entity<Editor>>,
    pub(crate) saves: files::SaveQueue<(Entity<Editor>, bool, After)>,
    /// Closing waits for a save to land ("Saving…").
    pub(crate) closing: bool,
    /// The Find in Files run streaming into Find Results, if one is going.
    pub(crate) search: Option<Search>,
    /// Run Pipeline on Files, if one is going.
    pub(crate) pipeline_run: Option<pipelines::Run>,
    /// Keep F4 on the results view that opened a target, including cloned views.
    pub(crate) last_results: Option<gpui::WeakEntity<Editor>>,
    /// What the Find in Files panel opens with: the last run's Where, query, replacement and
    /// toggles.
    pub(crate) last_where: String,
    pub(crate) last_fif_query: String,
    pub(crate) last_fif_replace: String,
    pub(crate) fif_opts: FindOpts,
    pub(crate) fif_flags: crate::searching::results::SearchFlags,
    /// The last Find in Files run's results, structured: the results browser, Replace
    /// in Files, `<previous results>` and extraction read them.
    pub(crate) result_set: Option<crate::searching::results::ResultSet>,
    pub(crate) result_pages: results_panel::ResultPages,
    /// Explicitly chosen hit lines survive query, panel and source-result publication changes.
    pub(crate) previous_lines: Option<std::sync::Arc<Vec<crate::searching::results::MatchingLines>>>,
    /// Replace in Files applying the result set's replacements, if it is.
    pub(crate) applying: Option<results_panel::Applying>,
    /// Submitted replacements and Where values in this window, as `query_history` keeps the
    /// queries.
    pub(crate) replace_history: search::QueryHistory,
    pub(crate) where_history: search::QueryHistory,
    /// The saved search last used or saved, offered as the name to save under.
    pub(crate) last_saved_name: String,
    /// Big files being read in the background, so a second open waits for the first.
    pub(crate) loading: HashSet<PathBuf>,
    /// Per-path waiters that open a file after a pipeline write finishes.
    pub(crate) opening_after_write: HashSet<PathBuf>,
    /// Sidebar opens waiting for a background read or a reserved file.
    pub(crate) previewing: HashMap<PathBuf, PendingOpen>,
    /// Each group's tab strip scroll, and the tab it last scrolled into view.
    pub(crate) tab_scroll: Vec<(ScrollHandle, Option<usize>)>,
    /// The view and caret the find panel opened on: find-as-you-type searches from there.
    pub(crate) find_origin: Option<(EntityId, usize)>,
    /// This window, for `refresh_menus`.
    pub(crate) window: gpui::AnyWindowHandle,
    /// Deferred session views and their places: a quit before the read lands keeps them.
    pub(crate) restoring: Vec<sessions::RestoringView>,
    /// Bumped when the window's contents are replaced (a project switch): reads begun for the
    /// views before then are dropped when they land.
    pub(crate) restore_gen: u64,
    pub(crate) comparison: comparison::State,
    /// Where a Find Results line asked to go in a file that is still being read.
    pub(crate) pending_jumps: HashMap<PathBuf, PendingJump>,
    /// A menu bar rebuild is scheduled for the end of this update; how many there were.
    pub(crate) menus_due: bool,
    pub(crate) menus_built: usize,
    /// The active view's Delimiter choice as the menu bar last showed it.
    pub(crate) menu_delim: Option<Delim>,
    /// Range-formatting availability in this window's last installed menu.
    pub(crate) menu_range_formatting: bool,
    pub(crate) menu_language_navigation: language::NavigationCapabilities,
    /// The session's files (group, view) on a drive not mounted at launch: kept for the next
    /// session, unseen in this one.
    pub(crate) unavailable: Vec<(usize, ViewState)>,
    /// A find in a big buffer under way on another thread: (view, version, query, options),
    /// and its cancel flag.
    pub(crate) searching: Option<find::Searching>,
    /// A query the user typed whose search is under way: its first match is selected when it
    /// lands, unless the text or the query changed since.
    pub(crate) find_pending: Option<String>,
    /// The outlines' search after edits to a big buffer, waiting for them to pause.
    pub(crate) find_debounce: Option<Task<()>>,
    /// Goto Anything's files under the roots as the last walk found them (the roots it walked,
    /// and each file with the root it was found under), and the walk under way.
    pub(crate) file_list: Option<(Vec<PathBuf>, FileList)>,
    pub(crate) file_walk: Option<Task<()>>,
    pub(crate) walk_again: bool,
    /// The folder of the last file focused in this window, where an untitled buffer's Save As
    /// opens (Sublime's).
    pub(crate) last_dir: Option<PathBuf>,
    /// The next change to the find query is Orlo's own (a seed, cmd-e), not typing.
    pub(crate) find_quiet: bool,
    /// A transient message on the status bar's left, and the timer that ends it.
    pub(crate) status: Option<(String, Task<()>)>,
    /// The side bar sliding: the share drawn when it started, and when.
    pub(crate) sidebar_anim: Option<(f32, std::time::Instant)>,
    /// The groups area as last drawn: a split must leave room.
    pub(crate) groups_bounds: std::rc::Rc<std::cell::Cell<Bounds<gpui::Pixels>>>,
    /// View > Sync Scrolling: vertical, horizontal.
    pub(crate) sync_scroll: (bool, bool),
    /// Where a tab drag would drop now: (group, index) for the insertion marker.
    pub(crate) tab_drop: Option<(usize, usize)>,
    /// The tab whose context menu is open: tab commands act on it, not the active one.
    pub(crate) context_tab: Option<gpui::WeakEntity<Editor>>,
    /// Tabs Close Other Tabs and the like still have to close, one prompt at a time.
    pub(crate) close_queue: Vec<gpui::WeakEntity<Editor>>,
    /// A right-click menu open on a tab or a side bar row.
    pub(crate) context_menu: Option<context_menu::ContextMenu>,
    /// The side bar's keys: `ctrl-0`, then arrows, Enter, F2, `⌘⌫`.
    pub sidebar_focus: FocusHandle,
    /// The open panel hidden (kept as it is) or collapsed to its title.
    pub(crate) panel_hidden: bool,
    pub(crate) panel_collapsed: bool,
    /// Where the results browser and the playground dock and how big they are.
    pub(crate) panel_places: session::PanelPlaces,
    /// Distraction-free mode, while it is on.
    pub(crate) distraction_free: Option<distraction::DistractionFreeMode>,
    /// The window title last set: macOS relays out the title bar on every set.
    pub(crate) shown_title: String,
    /// Escape for a tab drag or a context menu; app-wide, so it goes with the window.
    pub(crate) escape_keys: Option<Subscription>,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_branch_reads_branches_hashes_worktrees_and_parent_repos() {
        let tmp = std::env::temp_dir().join(format!("orlo-branch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (repo, wt) = (tmp.join("repo"), tmp.join("wt"));
        for d in [repo.join(".git"), repo.join("src/deep"), wt.join("gitdir"), wt.join("co")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let write = |p: PathBuf, s: &str| std::fs::write(p, s).unwrap();
        write(repo.join(".git/HEAD"), "ref: refs/heads/feature/x\n");
        assert_eq!(git_branch(&repo).as_deref(), Some("feature/x"));
        assert_eq!(git_branch(&repo.join("src/deep")).as_deref(), Some("feature/x"), "a folder inside the repo");
        write(repo.join(".git/HEAD"), "0123456789abcdef0123456789abcdef01234567\n");
        assert_eq!(git_branch(&repo).as_deref(), Some("01234567"), "detached: 8, as Sublime");
        // worktrees and submodules: .git is a file naming the git dir
        write(wt.join("gitdir/HEAD"), "ref: refs/heads/wt\n");
        write(wt.join("co/.git"), "gitdir: ../gitdir\n");
        assert_eq!(git_branch(&wt.join("co")).as_deref(), Some("wt"));
        let abs = format!("gitdir: {}\n", wt.join("gitdir").display());
        write(wt.join("co/.git"), &abs);
        assert_eq!(git_branch(&wt.join("co")).as_deref(), Some("wt"));
        assert_eq!(git_branch(&tmp), None, "no repo");
    }
}
