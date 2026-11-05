//! The menu bar, in the macOS order: the app, File, Edit, Selection, Find, View, Goto, then a
//! menu per tool large enough to need one (Terminal, Git), Tools for the rest, Project, Window
//! and Help. Each menu runs from its most used commands to its least, in groups a separator
//! names; long families nest one level, never two where one will do.

use super::command_entries as shared;
use super::*;
use crate::editing::transform::{NormForm, TextOp};
use crate::editor::MarkedLines;
use crate::searching::extract::What;

fn a(name: &str, action: impl Action) -> MenuItem {
    MenuItem::action(name.to_string(), action)
}

fn sub(name: &str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem::submenu(menu(name, items))
}

fn menu(name: &str, items: Vec<MenuItem>) -> Menu {
    Menu { name: name.to_string().into(), items }
}

/// Find > Extract: matches or lines, of the Find query in the document or of the last Find in
/// Files results, to the clipboard or a new document, with locations or without.
pub fn extract_entries() -> Vec<(String, Extract)> {
    let mut v = Vec::new();
    for (from, src) in [(ExtractFrom::Document, ""), (ExtractFrom::Results, " of Search Results")] {
        for (what, name) in [(What::Matches, "Matches"), (What::Lines, "Matching Lines")] {
            for (to_clipboard, verb) in [(true, "Copy"), (false, "Extract")] {
                for (locations, with) in [(false, ""), (true, " with Locations")] {
                    let label = format!("{verb} {name}{src}{with}");
                    let a = Extract { what, from, to_clipboard, locations };
                    v.push((label, a));
                }
            }
        }
    }
    v
}

fn extract_items() -> Vec<MenuItem> {
    (extract_entries().into_iter()).map(|(label, action)| MenuItem::action(label, action)).collect()
}

/// A command and its five styles' actions, by label.
pub type StyleCommands = Vec<(String, Vec<(String, Box<dyn Action>)>)>;

/// Find > Highlight: each command in a submenu of the five styles.
pub fn highlight_entries() -> StyleCommands {
    let each = |f: &dyn Fn(usize) -> Box<dyn Action>| -> Vec<(String, Box<dyn Action>)> {
        (theme::HIGHLIGHT_NAMES.iter().enumerate()).map(|(i, n)| (format!("Style {}: {n}", i + 1), f(i))).collect()
    };
    let next = |forward| each(&move |style| Box::new(editor::NextHighlight { style, forward }));
    let mut clear = each(&|i| Box::new(editor::ClearHighlights { style: Some(i) }));
    clear.push(("All Styles".into(), Box::new(editor::ClearHighlights { style: None })));
    vec![
        ("Highlight Matches".into(), each(&|i| Box::new(HighlightMatches { style: i }))),
        ("Highlight Selection".into(), each(&|i| Box::new(editor::HighlightSelection { style: i }))),
        ("Next Highlight".into(), next(true)),
        ("Previous Highlight".into(), next(false)),
        ("Copy Highlighted Text".into(), each(&|i| Box::new(editor::CopyHighlighted { style: i }))),
        ("Clear Highlights".into(), clear),
    ]
}

fn highlight_items() -> Vec<MenuItem> {
    let item = |(label, action): (String, Box<dyn Action>)| shared::item(label, action);
    (highlight_entries().into_iter()).map(|(name, items)| sub(&name, items.into_iter().map(item).collect())).collect()
}

/// View > Word Wrap Column: the settings' `wrap_width`, the view's width, or a column.
pub const WRAP_COLUMNS: [(&str, Option<usize>); 8] = [
    ("Default (wrap_width setting)", None),
    ("Automatic (view width)", Some(0)),
    ("70", Some(70)),
    ("72", Some(72)),
    ("78", Some(78)),
    ("80", Some(80)),
    ("100", Some(100)),
    ("120", Some(120)),
];

/// View > Layout, in Sublime's order and wording.
pub const LAYOUTS: [(&str, Preset); 7] = [
    ("Single", Preset::Single),
    ("Columns: 2", Preset::Columns(2)),
    ("Columns: 3", Preset::Columns(3)),
    ("Columns: 4", Preset::Columns(4)),
    ("Rows: 2", Preset::Rows(2)),
    ("Rows: 3", Preset::Rows(3)),
    ("Grid: 4", Preset::Grid4),
];

impl Workspace {
    pub(crate) fn menus(&self, cx: &App) -> Vec<Menu> {
        let sep = MenuItem::separator;
        let range_formatting = self.has_range_formatting(cx);
        let navigation = self.navigation_capabilities(cx);
        let either = |on: bool, yes: &str, no: &str, what: &str| format!("{} {what}", if on { yes } else { no });
        let show = |on: bool, what: &str| either(on, "Hide", "Show", what);
        let check = |on: bool, label: &str| format!("{}{label}", if on { "✓ " } else { "" });
        // each list ends in a separator
        let recent = |paths: &[PathBuf], folder: bool| -> Vec<MenuItem> {
            (paths.iter().enumerate())
                .map(|(i, p)| a(&tilde(p), OpenRecent { index: i, folder }))
                .chain([sep()])
                .collect()
        };
        let mut open_recent = vec![a("Reopen Closed File", ReopenClosedFile), sep()];
        let r = cx.global::<Recent>();
        open_recent.extend(recent(&r.files, false));
        open_recent.extend(recent(&r.folders, true));
        open_recent.push(a("Clear Items", ClearRecent));
        let recent_projects: Vec<MenuItem> = (r.projects.iter())
            .map(|p| {
                let action = OpenProjectPath { path: p.clone(), here: false };
                a(&tilde(p), action)
            })
            .chain([sep(), a("Clear Items", ClearRecent)])
            .collect();
        let syntaxes: Vec<MenuItem> =
            (Highlighter::syntax_names().into_iter()).map(|name| a(&name.clone(), SetSyntax { name })).collect();
        let widths = [70, 72, 78, 80, 100, 120];
        let mut rulers = vec![a("None", SetRuler { col: None }), sep()];
        rulers.extend(widths.map(|w| a(&w.to_string(), SetRuler { col: Some(w) })));
        let mut wrap = vec![a("Wrap Paragraph", editor::WrapLines { width: 0 })];
        wrap.extend(widths.map(|w| a(&format!("Wrap paragraph at {w} characters"), editor::WrapLines { width: w })));
        let wrap_columns: Vec<MenuItem> =
            (WRAP_COLUMNS.iter()).map(|(label, col)| a(label, SetWrapColumn { col: *col })).collect();
        let mut layouts: Vec<MenuItem> =
            (LAYOUTS.iter()).map(|(label, preset)| a(label, SetLayout { preset: *preset })).collect();
        // the user's own, by name
        let saved = (cx.try_global::<groups::SavedLayoutNames>())
            .map_or_else(|| groups::SavedLayoutNames::read().0, |n| n.0.clone());
        if !saved.is_empty() {
            layouts.push(sep());
            layouts.extend(saved.into_iter().map(|name| {
                let action = UseSavedLayout { name: name.clone(), delete: false };
                a(&name, action)
            }));
        }
        layouts.extend([
            sep(),
            a("Save Layout…", SaveLayout),
            a("Apply Saved Layout…", ChooseSavedLayout { delete: false }),
            a("Delete Saved Layout…", ChooseSavedLayout { delete: true }),
        ]);
        let pinned = (self.active_editor()).is_some_and(|t| self.views.is_pinned(&t));
        let tab_list = |label, scope| a(label, ShowTabList { scope });
        let mut tabs_menu = shared::menu(shared::tabs(Some(pinned)));
        tabs_menu.push(tab_list("Tab List…", TabScope::Window));
        let dock = |label, dock| a(label, MovePanel { dock });
        let panel_menu = vec![
            a(&show(self.panel.is_some() && !self.panel_hidden, "Panel"), TogglePanel),
            a(&either(self.panel_collapsed, "Expand", "Collapse", "Panel"), ToggleCollapsePanel),
            a("Focus Panel", FocusPanel),
            sep(),
            dock("Move Panel to Bottom", session::Dock::Bottom),
            dock("Move Panel to Right", session::Dock::Right),
        ];
        let sync = |label, on, vertical| a(&check(on, label), ToggleSyncScroll { vertical });
        // Sublime lists only the groups the layout has
        let groups = self.views.groups.len();
        let step = |label, forward| a(label, FocusNeighbour { forward });
        let mut focus = vec![step("Next", true), step("Previous", false), sep()];
        focus.extend((0..groups).map(|g| a(&format!("Group {}", g + 1), FocusGroup { group: g })));
        let step = |label, forward| a(label, MoveToNeighbour { forward });
        let mut moves = vec![step("Next", true), step("Previous", false), sep()];
        moves.extend((0..groups).map(|g| a(&format!("Group {}", g + 1), MoveToGroup { group: g })));
        let lines = |label: &str, op: Permute| a(label, editor::PermuteLines { op });
        let sels = |label: &str, op: Permute| a(label, editor::PermuteSelections { op });
        let sort = |cs| Permute::Sort { case_sensitive: cs };
        let text_op = |label: &str, op| a(label, editor::TransformText { op });
        let affix = |label: &str, suffix, remove| a(label, AffixLines { suffix, remove });
        let column = |label: &str, op| a(label, editor::ColumnCommand { op });
        let marked = |label, op| a(label, editor::BookmarkedLines { op });
        let word = |label, forward| a(label, editor::DeleteWord { forward });
        let edge = |label, forward| a(label, editor::DeleteToLineEdge { forward });
        let convert = |label, to_spaces| a(label, editor::ConvertIndentation { to_spaces });
        // Sublime's: level 1 is Fold All
        let mut folding = vec![
            a("Fold", editor::Fold),
            a("Unfold", editor::Unfold),
            a("Unfold All", editor::UnfoldAll),
            sep(),
            a("Fold All", editor::FoldLevel { level: 1 }),
        ];
        folding.extend((2..=9).map(|level| a(&format!("Fold Level {level}"), editor::FoldLevel { level })));
        // gpui 0.2.2 menu items have no checked state, so the choice is a "✓ " in the label
        let chosen = (self.active_editor()).and_then(|e| e.read(cx).doc(cx).delimiter_override());
        let delimiter = |label: &str, delim: Option<Delim>| {
            let mark = if delim == chosen { "✓ " } else { "" };
            a(&format!("{mark}{label}"), SetDelimiter { delim })
        };
        let blocked = cx.try_global::<SessionBlocked>().is_some_and(|b| b.0.is_some());
        let guide = |label, page| a(label, OpenGuide { page });
        vec![
            menu(
                "Orlo",
                vec![
                    a("About Orlo", About),
                    sep(),
                    a("Settings…", OpenSettings),
                    a("Key Bindings", OpenKeybindings),
                    sub(
                        "Appearance",
                        vec![
                            a("Theme…", ChooseAppearance),
                            a("Color Scheme…", ChooseColorScheme),
                            sep(),
                            a("Larger Font", IncreaseFontSize),
                            a("Smaller Font", DecreaseFontSize),
                            a("Reset Font Size", ResetFontSize),
                        ],
                    ),
                    sub(
                        "Customizations",
                        vec![
                            a("Open Themes Folder", OpenThemesFolder),
                            a("Open Syntaxes Folder", OpenSyntaxFolder),
                            sep(),
                            a("Reload Customizations", ReloadCustomizations),
                        ],
                    ),
                    sep(),
                    MenuItem::os_submenu("Services", SystemMenuType::Services),
                    sep(),
                    a("Hide Orlo", Hide),
                    a("Hide Others", HideOthers),
                    a("Show All", ShowAll),
                    sep(),
                    a("Quit Orlo", Quit),
                ],
            ),
            // create, open, save, recover, special modes, print, close
            menu(
                "File",
                vec![
                    a("New File", NewFile),
                    a("New From Template…", NewFromTemplate),
                    a("New Window", NewWindow),
                    sep(),
                    a("Open…", Open),
                    sub("Open Recent", open_recent),
                    sep(),
                    a("Save", Save),
                    a("Save As…", SaveAs),
                    a("Save a Copy As…", SaveCopyAs),
                    a("Save with Encoding…", SaveWithEncoding),
                    a("Save Without Formatting", SaveWithoutFormatting),
                    a("Save All", SaveAll),
                    sep(),
                    a("Revert File", Revert),
                    a("Reopen with Encoding…", ReopenWithEncoding),
                    sub("File History", shared::menu(shared::file_history())),
                    sub("Discarded Untitled", shared::menu(shared::discarded_untitled())),
                    sep(),
                    shared::menu(shared::viewer_mode()).remove(0),
                    a("Follow Log", FollowLog),
                    a("Stop Following Log", StopFollowingLog),
                    sep(),
                    a("Print…", Print),
                    sep(),
                    a("Close File", CloseTab),
                    a("Close All Files", CloseAll),
                    a("Close Window", CloseWindow),
                ]
                .into_iter()
                .chain(blocked.then(|| [sep(), a("Session Could Not Be Read…", SessionProblem)]).into_iter().flatten())
                .collect(),
            ),
            menu(
                "Edit",
                vec![
                    MenuItem::os_action("Undo", editor::Undo, OsAction::Undo),
                    MenuItem::os_action("Redo", editor::Redo, OsAction::Redo),
                    sep(),
                    MenuItem::os_action("Cut", editor::Cut, OsAction::Cut),
                    MenuItem::os_action("Copy", editor::Copy, OsAction::Copy),
                    a("Copy as HTML", editor::CopyAsHtml),
                    MenuItem::os_action("Paste", editor::Paste, OsAction::Paste),
                    a("Paste and Indent", editor::PasteAndIndent),
                    a("Paste from History", PasteFromHistory),
                    a("Clear Clipboard History", ClearClipboardHistory),
                    sep(),
                    shared::menu(shared::date_time()).remove(0),
                    a("Insert Unicode Character…", InsertUnicodeCharacterPrompt),
                    a("Insert Literal Next Key", editor::InsertLiteralNextKey),
                    a("Character Viewer", ShowCharacterViewer),
                    sep(),
                    sub("Line", shared::menu(shared::lines())),
                    sub("Comment", shared::menu(shared::comments())),
                    sub(
                        "Text",
                        vec![
                            a("Insert Line Before", editor::InsertLineBefore),
                            a("Insert Line After", editor::InsertLineAfter),
                            a("Trim Trailing Whitespace", editor::TrimTrailingWhitespace),
                            a("Remove Blank Lines", editor::RemoveBlankLines),
                            sep(),
                            word("Delete Word Forward", true),
                            word("Delete Word Backward", false),
                            edge("Delete to End", true),
                            edge("Delete to Beginning", false),
                            sep(),
                            a("Transpose", editor::Transpose),
                        ]
                        .into_iter()
                        .chain(shared::menu(shared::word_transposition()))
                        .chain(shared::menu(shared::surrounds()))
                        .chain(shared::menu(shared::value_adjustments()))
                        .collect(),
                    ),
                    sub("Subwords", shared::menu(shared::subwords())),
                    sub("Prose", shared::menu(shared::prose())),
                    sub("Convert Case", shared::menu(shared::cases())),
                    sub("Convert", shared::menu(shared::conversions())),
                    sub("Wrap", wrap),
                    a("Show Completions", editor::AutoComplete),
                    a("Toggle Overtype", editor::ToggleOvertype),
                    sep(),
                    sub("Mark", shared::menu(shared::marks())),
                    sub("Code Folding", folding),
                    sub("Fold Navigation", shared::menu(shared::fold_navigation())),
                    sub("Narrow View", shared::menu(shared::view_filters())),
                    sep(),
                    sub(
                        "Sort & Permute",
                        vec![
                            lines("Sort Lines", sort(false)),
                            lines("Sort Lines (Case Sensitive)", sort(true)),
                            sep(),
                            lines("Reverse Lines", Permute::Reverse),
                            lines("Unique Lines", Permute::Unique),
                            lines("Shuffle Lines", Permute::Shuffle),
                            sep(),
                            sels("Sort Selections", sort(false)),
                            sels("Sort Selections (Case Sensitive)", sort(true)),
                            sels("Reverse Selections", Permute::Reverse),
                            sels("Unique Selections", Permute::Unique),
                            sels("Shuffle Selections", Permute::Shuffle),
                        ],
                    ),
                    sub(
                        "Transform",
                        vec![
                            a("Sort Lines by Pattern…", SortLinesByPattern),
                            a("Process Duplicate Lines…", ProcessDuplicateLines),
                            a("Process Lines Matching…", ProcessLinesMatching),
                            sep(),
                            affix("Add Line Prefix…", false, false),
                            affix("Add Line Suffix…", true, false),
                            affix("Remove Line Prefix…", false, true),
                            affix("Remove Line Suffix…", true, true),
                            sep(),
                            text_op("Straighten Quotes", TextOp::StraightenQuotes),
                            text_op("Smart Quotes", TextOp::SmartQuotes),
                            sep(),
                            sub(
                                "Unicode Normalization",
                                vec![
                                    text_op("NFC", TextOp::Normalize(NormForm::Nfc)),
                                    text_op("NFD", TextOp::Normalize(NormForm::Nfd)),
                                    text_op("NFKC", TextOp::Normalize(NormForm::Nfkc)),
                                    text_op("NFKD", TextOp::Normalize(NormForm::Nfkd)),
                                ],
                            ),
                            text_op("Strip Diacritics", TextOp::StripDiacritics),
                            sep(),
                            a("Apply Substitution Table…", ApplySubstitutionTable),
                        ],
                    ),
                    sub(
                        "Columns",
                        vec![a("Column Editor…", ColumnEditor), a("Pad to Column…", PadToColumn)]
                            .into_iter()
                            .chain(shared::menu(shared::field_alignment()))
                            .chain(vec![
                                sep(),
                                column("Select Column", ColumnOp::Select),
                                column("Copy Column", ColumnOp::Copy),
                                column("Move Column Left", ColumnOp::MoveLeft),
                                column("Move Column Right", ColumnOp::MoveRight),
                                column("Delete Column", ColumnOp::Delete),
                                column("Clear Column", ColumnOp::Clear),
                                sep(),
                                sub(
                                    "Delimiter",
                                    vec![
                                        delimiter("Automatic", None),
                                        delimiter("Comma", Some(Delim::Comma)),
                                        delimiter("Tab", Some(Delim::Tab)),
                                        delimiter("Semicolon", Some(Delim::Semicolon)),
                                        delimiter("Pipe", Some(Delim::Pipe)),
                                    ],
                                ),
                            ])
                            .collect(),
                    ),
                    sub("JSON", shared::menu(shared::json())),
                    sep(),
                    a("Document Statistics", DocumentStatistics),
                    a("Character Info", CharacterInfo),
                ],
            ),
            menu(
                "Selection",
                vec![
                    MenuItem::os_action("Select All", editor::SelectAll, OsAction::SelectAll),
                    a("Single Selection", editor::SingleSelection),
                    a("Split into Lines", editor::SplitIntoLines),
                    a("Add Previous Line", editor::SelectLines { forward: false }),
                    a("Add Next Line", editor::SelectLines { forward: true }),
                    sep(),
                    a("Expand Selection", editor::ExpandSmart),
                    a("Expand Selection to Word", editor::SelectNextOccurrence),
                    a("Expand Selection to Line", editor::SelectLine),
                    a("Expand Selection to Line Upward", editor::SelectLineUpward),
                    a("Expand Selection to Paragraph", editor::ExpandToParagraph),
                    a("Expand Selection to Brackets", editor::ExpandToBrackets),
                    a("Expand Selection to Indentation", editor::ExpandToIndentation),
                    sep(),
                    a("Undo Selection", editor::UndoSelection),
                    a("Redo Selection", editor::RedoSelection),
                    sub("Selection Sets", shared::menu(shared::selection_sets())),
                    sep(),
                ]
                .into_iter()
                .chain(shared::menu(shared::syntax_selection()))
                .collect(),
            ),
            menu(
                "Find",
                vec![
                    a("Find…", Find),
                    a("Find Next", FindNext),
                    a("Find Previous", FindPrev),
                    a("Replace…", Replace),
                    a("Replace Next", find_panel::ReplaceOne),
                    sep(),
                    a("Quick Find", editor::FindUnder),
                    a("Quick Find All", editor::SelectAllOccurrences),
                    a("Quick Add Next", editor::SelectNextOccurrence),
                    a("Quick Skip Next", editor::QuickSkipNext),
                    sep(),
                    a("Use Selection for Find", UseSelectionForFind),
                    a("Use Selection for Replace", UseSelectionForReplace),
                    sub(
                        "Find Options",
                        vec![
                            a("Regular Expression", ToggleFindRegex),
                            a("Case Sensitive", ToggleFindCase),
                            a("Whole Word", ToggleFindWord),
                            a("Preserve Case", TogglePreserveCase),
                            a("Find in Selection", ToggleFindSelection),
                        ],
                    ),
                    sep(),
                    a("Find in Files…", FindInFiles),
                    a("Find in Side Bar Selection…", FindInSidebarSelection),
                    a("Replace in Files…", ReplaceInFiles),
                    a("Next Result", NextResult),
                    a("Previous Result", PrevResult),
                    sub(
                        "Search Results",
                        vec![
                            a("Show Search Results", ShowResults),
                            a("Refresh Search Results", RefreshResults),
                            sep(),
                            a("Apply Replacements", ApplyReplacements),
                            a("Cancel Replacements", CancelReplacements),
                            sep(),
                            a("Cancel Find in Files", CancelFindInFiles),
                        ],
                    ),
                    sep(),
                    sub("Extract", extract_items()),
                    sub("Highlight", highlight_items()),
                    sep(),
                    sub("Saved Searches", shared::menu(shared::saved_searches())),
                    a("Search History…", ShowFieldHistory),
                    a("Clear Search History", ClearSearchHistory),
                    sep(),
                    a("Regex Playground", OpenPlayground),
                    a("Regex Playground with a Copy of the Document", OpenPlaygroundWithDocument),
                ],
            ),
            // the window's chrome, then how text shows, then the layout, then the document's format
            menu(
                "View",
                vec![
                    sub(
                        "Side Bar",
                        vec![
                            a(&show(self.sidebar.visible, "Side Bar"), ToggleSidebar),
                            a(&show(self.sidebar.show_open_files, "Open Files"), ToggleOpenFiles),
                            a("Focus Side Bar", FocusSideBar),
                        ]
                        .into_iter()
                        .chain(shared::menu(shared::explorer_navigation()))
                        .collect(),
                    ),
                    a(&show(self.outline.visible, "Outline"), ToggleOutline),
                    a(&show(self.shown_view(cx).minimap, "Minimap"), ToggleMinimap),
                    a(&show(self.show_tabs, "Tabs"), ToggleTabs),
                    a(&show(self.show_status_bar, "Status Bar"), ToggleStatusBar),
                    sub("Panel", panel_menu),
                    a(
                        match self.distraction_free.is_some() {
                            true => "Exit Distraction Free Mode",
                            false => "Enter Distraction Free Mode",
                        },
                        ToggleDistractionFree,
                    ),
                    // macOS appends its own "Enter Full Screen" item to a View menu
                    sep(),
                    a(&check(self.shown_view(cx).white_space.all, "Show White Space"), ToggleWhiteSpace),
                    sub("Line Numbers", shared::menu(shared::line_numbers(Some(self.shown_view(cx).line_number_mode)))),
                    shared::menu(shared::cursor_columns(self.shown_view(cx).cursor_column_highlight)).remove(0),
                    // no key and no checkmark, as in Sublime
                    a("Word Wrap", ToggleWordWrap),
                    sub("Word Wrap Column", wrap_columns),
                    sub("Ruler", rulers),
                    sub(
                        "Spelling",
                        vec![
                            a("Spell Check", editor::ToggleSpellCheck),
                            sep(),
                            a("Next Misspelling", editor::NextMisspelling),
                            a("Previous Misspelling", editor::PrevMisspelling),
                            a("Spelling Suggestions…", editor::SpellingSuggestions),
                        ],
                    ),
                    sep(),
                    sub("Layout", layouts),
                    sub("Groups", shared::menu(shared::groups())),
                    sub("Tabs", tabs_menu),
                    sub("Focus Group", focus),
                    sub("Move File to Group", moves),
                    sub(
                        "Sync Scrolling",
                        vec![sync("Vertical", self.sync_scroll.0, true), sync("Horizontal", self.sync_scroll.1, false)],
                    ),
                    sep(),
                    sub("Syntax", syntaxes),
                    sub(
                        "Indentation",
                        [
                            a("Indent Using Spaces", SetIndentStyle { to_spaces: true }),
                            a("Indent Using Tabs", SetIndentStyle { to_spaces: false }),
                            sep(),
                        ]
                        .into_iter()
                        .chain([1, 2, 3, 4, 8, 16].map(|size| a(&format!("Tab Width: {size}"), SetTabSize { size })))
                        .chain([
                            sep(),
                            convert("Convert Indentation to Spaces", true),
                            convert("Convert Indentation to Tabs", false),
                        ])
                        .collect(),
                    ),
                    sub(
                        "Line Endings",
                        vec![
                            a("Inspect Line Endings…", InspectLineEndings),
                            sep(),
                            a("Windows", SetLineEnding { ending: "\r\n" }),
                            a("Unix", SetLineEnding { ending: "\n" }),
                            a("Mac OS 9", SetLineEnding { ending: "\r" }),
                        ],
                    ),
                ],
            ),
            menu(
                "Goto",
                vec![
                    a("Goto Anything…", GotoAnything),
                    a("Goto Symbol…", GotoSymbol),
                    a("Goto Symbol in Project…", GotoSymbolInProject),
                    a("Goto Line…", GotoLine),
                    sep(),
                    a("Goto Definition…", GotoDefinition),
                    a("Goto Definition to Side", GotoDefinitionToSide),
                    a("Goto Reference…", GotoReference),
                    sub(
                        "Language Server",
                        vec![
                            a("Complete", LspCompletion),
                            a("Find References…", LspReferences),
                            a("Go to Type Definition", LspTypeDefinition),
                            a("Go to Type Definition to Side", LspTypeDefinitionToSide),
                            a("Go to Implementation", LspImplementation),
                            a("Go to Implementation to Side", LspImplementationToSide),
                            a("Show Documentation", LspHover),
                            a("Rename Symbol…", LspRename),
                            a("Signature Help", LspSignatureHelp),
                            a("Format Document", LspFormat),
                            a("Format Selection", LspFormatSelection),
                            a("Code Actions…", LspCodeActions),
                            a("Show Diagnostics…", LspDiagnostics),
                            a("Restart Language Servers", RestartLanguageServers),
                        ]
                        .into_iter()
                        .filter(|item| match item {
                            MenuItem::Action { action, .. } if action.as_any().is::<LspFormatSelection>() => {
                                range_formatting
                            }
                            MenuItem::Action { action, .. }
                                if action.as_any().is::<LspTypeDefinition>()
                                    || action.as_any().is::<LspTypeDefinitionToSide>() =>
                            {
                                navigation.type_definition
                            }
                            MenuItem::Action { action, .. }
                                if action.as_any().is::<LspImplementation>()
                                    || action.as_any().is::<LspImplementationToSide>() =>
                            {
                                navigation.implementation
                            }
                            _ => true,
                        })
                        .collect(),
                    ),
                    sep(),
                    a("Switch File", SwitchFile),
                    a("Next File", NextTab),
                    a("Previous File", PrevTab),
                    a("Related Files…", RelatedFiles),
                    sub("Open Selected", shared::menu(shared::selected_targets())),
                    sep(),
                    a("Jump Back", editor::JumpBack),
                    a("Jump Forward", editor::JumpForward),
                    a("Jump to Matching Bracket", editor::JumpToBracket),
                    sep(),
                    sub(
                        "Bookmarks",
                        vec![
                            a("Find Bookmark Across Open Files…", GotoBookmark),
                            sep(),
                            a("Toggle Bookmark", editor::ToggleBookmark),
                            a("Next Bookmark", editor::NextBookmark { forward: true }),
                            a("Previous Bookmark", editor::NextBookmark { forward: false }),
                            a("Clear Bookmarks", editor::ClearBookmarks),
                            a("Select All Bookmarks", editor::SelectAllBookmarks),
                            sep(),
                            a("Bookmark Matching Lines", BookmarkMatches),
                            marked("Invert Bookmarks", MarkedLines::Invert),
                            sep(),
                            marked("Copy Bookmarked Lines", MarkedLines::Copy),
                            marked("Cut Bookmarked Lines", MarkedLines::Cut),
                            marked("Delete Bookmarked Lines", MarkedLines::Delete),
                            marked("Keep Only Bookmarked Lines", MarkedLines::KeepOnly),
                            marked("Paste to Bookmarked Lines", MarkedLines::Paste),
                        ],
                    ),
                    sub(
                        "Scroll",
                        vec![
                            a("Scroll to Selection", editor::ShowAtCenter),
                            a("Line Up", editor::ScrollLines { up: true }),
                            a("Line Down", editor::ScrollLines { up: false }),
                        ],
                    ),
                ],
            ),
            // the panel's terminals, then sending to them, then reading what they printed
            menu(
                "Terminal",
                vec![
                    a("New Terminal", NewTerminal),
                    a("New Terminal with Profile…", NewTerminalWithProfile),
                    a("Show / Hide Terminal", ShowTerminal),
                    sep(),
                    a("Next Terminal", NextTerminal),
                    a("Previous Terminal", PreviousTerminal),
                    a("Rename Terminal…", RenameTerminal),
                    a("Close Terminal", CloseTerminal),
                    sep(),
                    a("Send Selection to Terminal", SendSelectionToTerminal),
                    a("Send and Run Selection in Terminal", ExecuteSelectionInTerminal),
                    a("Sync Terminal Folder", SyncTerminalFolder),
                    sep(),
                    a("Previous Terminal Command", PreviousTerminalCommand),
                    a("Next Terminal Command", NextTerminalCommand),
                    a("Copy Terminal Command Output", CopyTerminalCommandOutput),
                    a("Copy Last Terminal Output", CopyLastTerminalOutput),
                    a("Find Terminal Output…", FindTerminalOutput),
                    a("Terminal Live Output", TerminalLiveOutput),
                    sep(),
                    a("New Shell Worksheet", NewShellWorksheet),
                    a("Run Worksheet Selection / Line", RunWorksheet),
                    sep(),
                    a("Set Terminal Scrollback…", SetTerminalScrollback),
                ],
            ),
            // the day's loop first (stage, commit, sync), then the repository's other objects
            menu(
                "Git",
                vec![
                    a("Show / Hide Git Sidebar", ShowGit),
                    a("Refresh", GitRefresh),
                    a("Set Up Git…", GitInit),
                    sep(),
                    a("Stage File", GitStage),
                    a("Unstage File", GitUnstage),
                    a("Stage Selected Lines", GitStageLines),
                    a("Stage Hunk", GitStageHunk),
                    a("Unstage Hunk", GitUnstageHunk),
                    sep(),
                    a("Commit Staged…", GitCommit),
                    a("Amend HEAD…", GitAmend),
                    sep(),
                    a("Fetch…", GitFetch),
                    a("Pull…", GitPull),
                    a("Push…", GitPush),
                    a("Remotes & Upstream…", GitRemotes),
                    sep(),
                    a("Switch Branch…", GitSwitchBranch),
                    sub(
                        "Branches",
                        vec![
                            a("Create Local Branch…", GitCreateBranch),
                            a("Rename Local Branch…", GitRenameBranch),
                            a("Delete Local Branch…", GitDeleteBranch),
                        ],
                    ),
                    sub("Tags", vec![a("Local Tags…", GitTags), a("Create Local Tag…", GitCreateTag)]),
                    sub("Stashes", vec![a("Browse Stashes…", GitStashes), a("Create Stash…", GitCreateStash)]),
                    sub(
                        "Worktrees",
                        vec![
                            a("Browse Worktrees…", GitWorktrees),
                            a("Create Worktree…", GitCreateWorktree),
                            a("Prune Worktrees…", GitPruneWorktrees),
                        ],
                    ),
                    sep(),
                    a("File History…", GitFileHistory),
                    a("Blame File", GitBlame),
                    sep(),
                    a("Show Change", ShowChange),
                    a("Next Change", NextChange),
                    a("Previous Change", PreviousChange),
                    a("Revert Reviewed Hunk", RevertChange),
                    sub(
                        "Conflicts",
                        vec![
                            a("Next Conflict", NextConflict),
                            sep(),
                            a("Accept Ours", AcceptOurs),
                            a("Accept Theirs", AcceptTheirs),
                            a("Accept Both", AcceptBoth),
                        ],
                    ),
                ],
            ),
            menu(
                "Tools",
                vec![
                    a("Command Palette…", CommandPalette),
                    sub("Snippets", shared::menu(shared::snippets())),
                    sub("Macros", shared::menu(shared::macros())),
                    sub("Text Pipelines", shared::menu(shared::pipelines())),
                    a("Guess Indentation", GuessIndentation),
                    sep(),
                    a("Run Task…", RunTask),
                    a("Edit User Tasks", EditTasks),
                    a("Edit Project Tasks", EditProjectTasks),
                    sep(),
                    a("Run Shell Command…", RunShellCommand),
                    a("Filter Through Shell…", FilterThroughShell),
                    a("Keep Selections Through Shell…", KeepSelectionsThroughShell),
                    a("Cancel Running Command", CancelAutomation),
                    sep(),
                    sub("Compare", shared::menu(shared::compare())),
                    sep(),
                    a("Manage Packages…", ManagePackages),
                    a("Install Package…", InstallPackage),
                    a("Developer Console", DeveloperConsole),
                ],
            ),
            menu(
                "Project",
                vec![
                    a("Open Project…", OpenProject),
                    a("Switch Project…", SwitchProject),
                    sub("Open Recent", recent_projects),
                    sep(),
                    a("Save Project As…", SaveProjectAs),
                    a("Save Workspace As…", SaveWorkspaceAs),
                    a("Edit Project", EditProject),
                    a("Close Project", CloseProject),
                    sep(),
                    a("Add Folder to Project…", AddFolder),
                    a("Remove Folder from Project…", RemoveFolder),
                    a("Remove All Folders from Project", RemoveFolders),
                    a("Move Folder Up", MoveFolder { up: true }),
                    a("Move Folder Down", MoveFolder { up: false }),
                    a("Refresh Folders", RefreshFolders),
                    a("Trust Open Folders…", TrustOpenFolders),
                    sep(),
                    sub(
                        "Side Bar Item",
                        vec![
                            a("New File…", NewFileHere),
                            a("New Folder…", NewFolderHere),
                            sep(),
                            a("Rename…", RenamePath),
                            a("Move…", MovePaths),
                            a("Duplicate…", DuplicatePath),
                            sep(),
                            a("Move to Trash", TrashPaths),
                            sep(),
                            a("Reveal in Finder", RevealInFinder),
                            a("Find in Folder…", FindInSidebarItem),
                        ],
                    ),
                ],
            ),
            menu(
                "Window",
                vec![
                    a("Minimize", Minimize),
                    a("Zoom", Zoom),
                    sep(),
                    tab_list("All Open Tabs…", TabScope::AllWindows),
                    sep(),
                    a("Bring All to Front", BringAllToFront),
                ],
            ),
            menu(
                "Help",
                vec![
                    guide("Orlo Help", ""),
                    guide("Local Alpha Guide", "docs/user-guide/local-alpha.md"),
                    guide("Customization and Automation", "docs/user-guide/customization.md"),
                    guide("Languages", "docs/user-guide/languages.md"),
                    sep(),
                    a("Sublime Text Documentation", Documentation),
                    sep(),
                    guide("Report an Issue", "issues/new"),
                ],
            ),
        ]
    }

    /// Rebuild the menu bar once this update is done: closing every tab asks once per tab,
    /// and each rebuild costs (gpui keeps the old menus' actions).
    pub(crate) fn refresh_menus(&mut self, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.menus_due, true) {
            let this = cx.weak_entity();
            cx.defer(move |cx| {
                this.update(cx, |ws, cx| {
                    ws.menus_due = false;
                    ws.set_menus_now(cx);
                })
                .ok();
            });
        }
    }

    /// The menu bar is the active window's: another window's settings save or background
    /// load must not put its groups and files there. True when it set them.
    pub(crate) fn set_menus_now(&mut self, cx: &mut App) -> bool {
        if !cx.has_global::<groups::SavedLayoutNames>() {
            cx.set_global(groups::SavedLayoutNames::read());
        }
        if cx.active_window() != Some(self.window) {
            return false;
        }
        let menus = self.menus(cx);
        cx.set_menus(menus);
        self.menu_delim = (self.active_editor()).and_then(|e| e.read(cx).doc(cx).delimiter_override());
        self.menu_range_formatting = self.has_range_formatting(cx);
        self.menu_language_navigation = self.navigation_capabilities(cx);
        self.menus_built += 1;
        true
    }
}
