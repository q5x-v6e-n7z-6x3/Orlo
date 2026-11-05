//! Workspace actions and their key bindings.

use super::*;

actions!(
    workspace,
    [
        NewFile,
        NewFromTemplate,
        Open,
        Save,
        SaveWithoutFormatting,
        SaveAs,
        SaveCopyAs,
        SaveAll,
        CloseTab,
        NextTab,
        PrevTab,
        LastTab,
        ToggleSidebar,
        ToggleOutline,
        SwitchFile,
        RelatedFiles,
        GotoAnything,
        CommandPalette,
        GotoLine,
        GotoBookmark,
        OpenSelectedPath,
        OpenSelectedUrl,
        Find,
        Replace,
        FindNext,
        FindPrev,
        FindAll,
        ToggleFindRegex,
        ToggleFindSelection,
        ToggleFindCase,
        ToggleFindWord,
        TogglePreserveCase,
        ToggleShowContext,
        ToggleUseBuffer,
        ToggleUseGitignore,
        PreviousFindQuery,
        NextFindQuery,
        UseSelectionForFind,
        ReplaceAll,
        ClosePanel,
        Quit,
        ReopenClosedFile,
        NewWindow,
        CloseWindow,
        CloseAll,
        Revert,
        FollowLog,
        ToggleViewerMode,
        StopFollowingLog,
        ToggleMinimap,
        ToggleCursorColumnHighlight,
        ToggleWhiteSpace,
        ToggleTabs,
        ToggleStatusBar,
        ToggleFullScreen,
        GotoSymbol,
        GotoSymbolInProject,
        GotoDefinition,
        GotoDefinitionToSide,
        GotoReference,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        UseSelectionForReplace,
        AddFolder,
        RemoveFolders,
        RemoveFolder,
        RefreshFolders,
        CloseGroup,
        SaveLayout,
        KeepOpen,
        TogglePin,
        CloseOtherTabs,
        CloseTabsToRight,
        DeselectTabs,
        MoveToNewWindow,
        CloseTabTarget,
        ToggleOpenFiles,
        NewFileHere,
        NewFolderHere,
        RenamePath,
        MovePaths,
        DuplicatePath,
        TrashPaths,
        RevealInFinder,
        FindInSidebarItem,
        FocusSideBar,
        ExplorerBack,
        ExplorerForward,
        ExplorerParent,
        PathBreadcrumbs,
        SideBarLeave,
        TogglePanel,
        ToggleCollapsePanel,
        FocusPanel,
        ToggleDistractionFree,
        OpenProject,
        SwitchProject,
        SaveProjectAs,
        CloseProject,
        EditProject,
        SaveWorkspaceAs,
        ClearRecent,
        SessionProblem,
        About,
        TrustOpenFolders,
        Documentation,
        Minimize,
        Zoom,
        BringAllToFront,
        Hide,
        HideOthers,
        ShowAll,
        NewViewIntoFile,
        OpenSettings,
        ToggleWordWrap,
        FindInFiles,
        RunFindInFiles,
        ReplaceInFiles,
        RefreshResults,
        ApplyReplacements,
        CancelReplacements,
        ShowResults,
        ChooseWhere,
        ShowFieldHistory,
        ClearSearchHistory,
        FindInSidebarSelection,
        BookmarkMatches,
        SaveSearch,
        OpenSavedSearch,
        RenameSavedSearch,
        DeleteSavedSearch,
        ImportSavedSearches,
        ExportSavedSearches,
        CancelFindInFiles,
        NextResult,
        PrevResult,
        PasteFromHistory,
        ClearClipboardHistory,
        HashFile,
        DocumentStatistics,
        InspectLineEndings,
        Print,
        CharacterInfo,
        InsertUnicodeCharacterPrompt,
        ShowCharacterViewer,
        ColumnEditor,
        PadToColumn,
        ShowMatchingLinesPrompt,
        InvertSelectionsPrompt,
        SplitSelectionsByRegexPrompt,
        KeepSelectionsMatchingRegexPrompt,
        RemoveMatchingSelectionsPrompt,
        IncrementValues,
        DecrementValues,
        AdjustValuesPrompt,
        AlignOnPatternPrompt,
        ChangeSurroundPrompt,
        DeleteSurroundPrompt,
        SortLinesByPattern,
        ProcessDuplicateLines,
        ProcessLinesMatching,
        ApplySubstitutionTable,
        NewPipeline,
        EditPipeline,
        PreviewPipeline,
        RunPipeline,
        RunPipelineOnFiles,
        CancelPipelineRun,
        InsertSnippet,
        InsertDateTimePrompt,
        NewSnippet,
    ]
);

/// What a pipeline chosen in the quick panel is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PipelineUse {
    Edit,
    Preview,
    Run,
    RunOnFiles,
}

parameter_actions! { workspace, serializable;

    SelectTab { index: usize, }

    SetRuler { col: Option<usize>, }

    SetLineNumberMode { mode: crate::config::settings::LineNumberMode, }

    SetTabSize { size: usize, }

    SetIndentStyle { to_spaces: bool, }

    SetSyntax { name: String, }

    /// View > Focus Group > Group N (0-based).
    FocusGroup { group: usize, }

    /// View > Move File to Group > Group N (0-based).
    MoveToGroup { group: usize, }

    /// View > Focus Group > Next / Previous.
    FocusNeighbour { forward: bool, }

    /// View > Move File to Group > Next / Previous.
    MoveToNeighbour { forward: bool, }

    /// Edit > Transform > Add / Remove Line Prefix / Suffix.
    AffixLines { suffix: bool, remove: bool, }

    /// View > Word Wrap Column: `None` follows the settings, `Some(0)` the view's width.
    SetWrapColumn { col: Option<usize>, }
}

parameter_actions! { workspace, plain;
    /// Trust a folder: its repository's own Git programs, its language servers and its tasks run.
    TrustFolder { path: PathBuf, }

    /// Help: a page of Orlo's repository (`""` its front page, a `.md` path a guide).
    OpenGuide { page: &'static str, }

    #[derive(serde::Serialize)]
    NextLineEnding { ending: &'static str, }
    #[derive(serde::Serialize)]
    SetLineEnding { ending: &'static str, }

    OpenRecent { index: usize, folder: bool, }

    /// View > Layout.
    SetLayout { preset: Preset, }

    /// A row of the pipeline list: the pipeline file and what to do with it.
    UsePipeline { path: PathBuf, how: PipelineUse, }

    /// View > Panel > Move Panel to Bottom / Right.
    MovePanel { dock: crate::app::session::Dock, }

    /// Up / Down in the side bar.
    SideBarStep { down: bool, }

    /// Right / Left in the side bar: expand or collapse.
    SideBarFold { open: bool, }

    /// Enter (or Space, a preview) in the side bar.
    SideBarOpen { preview: bool, }

    /// Tab > Add Next / Previous Tab to Selection.
    AddTabToSelection { forward: bool, }

    /// Tab > Focus Next / Previous Selected Tab.
    FocusSelectedTab { forward: bool, }

    /// The tab list: a group's tabs, the window's, or every window's.
    ShowTabList { scope: TabScope, }

    /// View > Groups > Split Right (`Cols`) / Split Down (`Rows`).
    SplitGroup { axis: Axis, }

    /// View > Groups > Move File to New Group Right / Down.
    MoveToNewGroup { axis: Axis, }

    /// View > Sync Scrolling > Vertical / Horizontal.
    ToggleSyncScroll { vertical: bool, }

    /// A saved layout, by name: applied, or deleted.
    UseSavedLayout { name: String, delete: bool, }

    /// View > Layout > Apply / Delete Saved Layout…
    ChooseSavedLayout { delete: bool, }

    /// Remove the window's root `index` (a row of Project > Remove Folder from Project's list).
    RemoveRoot { index: usize, }

    /// Project > Move Folder Up / Down: the selected side bar row's root.
    MoveFolder { up: bool, }

    /// Open a project or workspace file; `here` replaces this window's project (Switch Project).
    OpenProjectPath { path: PathBuf, here: bool, }

    /// Edit > Columns > Delimiter: `None` is Automatic.
    SetDelimiter { delim: Option<Delim>, }
}

pub fn key_bindings() -> Vec<KeyBinding> {
    fn k(keys: &str, action: impl Action) -> KeyBinding {
        KeyBinding::new(keys, action, None)
    }
    let mut b = vec![
        k("cmd-n", NewFile),
        KeyBinding::new("ctrl-q", ToggleMacroRecording, Some("Workspace")),
        KeyBinding::new("ctrl-shift-q", ReplayMacro, Some("Workspace")),
        KeyBinding::new("cmd-b", RunTask, Some("Workspace")),
        KeyBinding::new("cmd-.", CancelAutomation, Some("Workspace")),
        KeyBinding::new("cmd-enter", RunWorksheet, Some("Editor && worksheet")),
        k("cmd-o", Open),
        k("cmd-s", Save),
        k("cmd-shift-s", SaveAs),
        k("cmd-alt-s", SaveAll),
        k("cmd-w", CloseTab),
        k("cmd-shift-]", NextTab),
        k("cmd-shift-[", PrevTab),
        k("cmd-alt-right", NextTab),
        k("cmd-alt-left", PrevTab),
        k("ctrl-tab", NextTab),
        k("ctrl-shift-tab", PrevTab),
        k("cmd-9", LastTab),
        k("cmd-k cmd-b", ToggleSidebar),
        k("cmd-k cmd-v", PasteFromHistory),
        k("cmd-p", GotoAnything),
        k("cmd-t", GotoAnything),
        k("cmd-shift-p", CommandPalette),
        k("ctrl-g", GotoLine),
        k("cmd-f", Find),
        k("cmd-alt-f", Replace),
        k("cmd-g", FindNext),
        k("cmd-shift-g", FindPrev),
        k("cmd-e", UseSelectionForFind),
        k("cmd-shift-e", UseSelectionForReplace),
        k("cmd-i", Find),
        k("cmd-shift-f", FindInFiles),
        k("f4", NextResult),
        k("shift-f4", PrevResult),
        k("cmd-q", Quit),
        k("cmd-shift-t", ReopenClosedFile),
        k("cmd-shift-n", NewWindow),
        k("cmd-shift-w", CloseWindow),
        k("ctrl-cmd-f", ToggleFullScreen),
        k("cmd-r", GotoSymbol),
        k("cmd-shift-r", GotoSymbolInProject),
        k("f12", GotoDefinition),
        KeyBinding::new("shift-f12", GotoReference, Some("Editor && !single_line")),
        KeyBinding::new("ctrl-k ctrl-i", LspHover, Some("Editor && !single_line")),
        KeyBinding::new("f2", LspRename, Some("Editor && !single_line")),
        KeyBinding::new("alt-shift-f", LspFormat, Some("Editor && !single_line")),
        KeyBinding::new("ctrl-.", LspCodeActions, Some("Editor && !single_line")),
        k("cmd-alt-down", GotoDefinition),
        k("cmd-alt-up", SwitchFile),
        k("cmd-=", IncreaseFontSize),
        k("cmd-shift-=", IncreaseFontSize),
        k("cmd-+", IncreaseFontSize),
        k("cmd--", DecreaseFontSize),
        k("cmd-,", OpenSettings),
        k("cmd-m", Minimize),
        k("cmd-h", Hide),
        k("ctrl-cmd-p", SwitchProject),
        k("cmd-k enter", KeepOpen),
        k("cmd-k cmd-j", TogglePanel),
        k("ctrl-cmd-shift-f", ToggleDistractionFree),
        k("cmd-alt-shift-right", AddTabToSelection { forward: true }),
        k("cmd-alt-shift-left", AddTabToSelection { forward: false }),
        k("cmd-k cmd-]", FocusSelectedTab { forward: true }),
        k("cmd-k cmd-[", FocusSelectedTab { forward: false }),
        k("cmd-k cmd-t", ShowTabList { scope: TabScope::Window }),
        k("cmd-alt-h", HideOthers),
    ];
    b.extend(form::key_bindings());
    b.extend(fileops::key_bindings());
    b.extend((1..=8).map(|i| k(&format!("cmd-{i}"), SelectTab { index: i - 1 })));
    // Sublime's macOS layout keys
    let layouts = [
        ("cmd-alt-1", Preset::Single),
        ("cmd-alt-2", Preset::Columns(2)),
        ("cmd-alt-3", Preset::Columns(3)),
        ("cmd-alt-4", Preset::Columns(4)),
        ("cmd-alt-5", Preset::Grid4),
        ("cmd-alt-shift-2", Preset::Rows(2)),
        ("cmd-alt-shift-3", Preset::Rows(3)),
    ];
    b.extend(layouts.map(|(keys, preset)| k(keys, SetLayout { preset })));
    for i in 1..=9 {
        b.push(k(&format!("ctrl-{i}"), FocusGroup { group: i - 1 }));
        b.push(k(&format!("ctrl-shift-{i}"), MoveToGroup { group: i - 1 }));
    }
    b.extend([
        k("cmd-k cmd-right", FocusNeighbour { forward: true }),
        k("cmd-k cmd-left", FocusNeighbour { forward: false }),
        k("cmd-k cmd-shift-right", MoveToNeighbour { forward: true }),
        k("cmd-k cmd-shift-left", MoveToNeighbour { forward: false }),
    ]);
    b
}

fn line_ending_argument<'de, D: serde::Deserializer<'de>>(d: D) -> Result<&'static str, D::Error> {
    #[derive(serde::Deserialize)]
    struct Arguments {
        ending: String,
    }
    let value = <Arguments as serde::Deserialize>::deserialize(d)?.ending;
    ["\n", "\r\n", "\r"]
        .into_iter()
        .find(|ending| *ending == value)
        .ok_or_else(|| serde::de::Error::custom("Line ending must be LF, CRLF or CR"))
}

impl<'de> serde::Deserialize<'de> for SetLineEnding {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        line_ending_argument(d).map(|ending| Self { ending })
    }
}
impl<'de> serde::Deserialize<'de> for NextLineEnding {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        line_ending_argument(d).map(|ending| Self { ending })
    }
}
