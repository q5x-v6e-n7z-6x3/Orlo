//! Command families shared by native menus and the command palette.

use super::*;
use crate::editing::text::Motion;

/// A label, its action and its key hint; `None` is a separator.
pub(crate) type Command = Option<(&'static str, Box<dyn Action>, &'static str)>;
pub(crate) type Commands = Vec<Command>;

fn entry(label: &'static str, action: impl Action, hint: &'static str) -> Command {
    Some((label, Box::new(action), hint))
}

/// A menu item for an action already boxed.
pub(crate) fn item(name: impl Into<gpui::SharedString>, action: Box<dyn Action>) -> MenuItem {
    MenuItem::Action { name: name.into(), action, os_action: None }
}

pub(crate) fn menu(commands: Commands) -> Vec<MenuItem> {
    (commands.into_iter()).map(|entry| entry.map_or_else(MenuItem::separator, |(label, a, _)| item(label, a))).collect()
}

pub(crate) fn viewer_mode() -> Commands {
    vec![entry("Toggle Viewer Mode", ToggleViewerMode, "")]
}

pub(crate) fn cursor_columns(current: bool) -> Commands {
    vec![entry(
        if current { "✓ Highlight Cursor Columns" } else { "Highlight Cursor Columns" },
        ToggleCursorColumnHighlight,
        "",
    )]
}

pub(crate) fn line_numbers(current: Option<crate::config::settings::LineNumberMode>) -> Commands {
    use crate::config::settings::LineNumberMode::{Absolute, Relative};
    vec![
        entry(
            if current == Some(Absolute) { "✓ Absolute" } else { "Absolute" },
            SetLineNumberMode { mode: Absolute },
            "",
        ),
        entry(
            if current == Some(Relative) { "✓ Relative" } else { "Relative" },
            SetLineNumberMode { mode: Relative },
            "",
        ),
    ]
}

pub(crate) fn syntax_selection() -> Commands {
    vec![
        entry("Expand Syntax Selection", editor::ExpandSyntaxSelection, ""),
        entry("Shrink Syntax Selection", editor::ShrinkSyntaxSelection, ""),
    ]
}

pub(crate) fn cases() -> Commands {
    [
        ("Title Case", Case::Title, ""),
        ("Upper Case", Case::Upper, "⌘K, ⌘U"),
        ("Lower Case", Case::Lower, "⌘K, ⌘L"),
        ("Swap Case", Case::Swap, ""),
        ("lowerCamelCase", Case::LowerCamel, ""),
        ("UpperCamelCase", Case::UpperCamel, ""),
        ("snake_case", Case::Snake, ""),
        ("kebab-case", Case::Kebab, ""),
    ]
    .into_iter()
    .map(|(label, case, hint)| entry(label, editor::ConvertCase { case }, hint))
    .collect()
}

pub(crate) fn conversions() -> Commands {
    let convert = |label, codec, decode| entry(label, editor::Convert { codec, decode }, "");
    let hash = |label, hash| entry(label, editor::CopyHash { hash }, "");
    vec![
        convert("Base64 Encode", Codec::Base64, false),
        convert("Base64 Decode", Codec::Base64, true),
        convert("Base64 URL-safe Encode", Codec::Base64Url, false),
        convert("Base64 URL-safe Decode", Codec::Base64Url, true),
        convert("Hex Encode", Codec::Hex, false),
        convert("Hex Decode", Codec::Hex, true),
        convert("URI Component Encode", Codec::UriComponent, false),
        convert("URI Component Decode", Codec::UriComponent, true),
        convert("HTML Entity Escape", Codec::HtmlEntities, false),
        convert("HTML Entity Unescape", Codec::HtmlEntities, true),
        convert("Escape Regex", Codec::RegexLiteral, false),
        convert("Quote JSON String", Codec::JsonString, false),
        convert("Unquote JSON String", Codec::JsonString, true),
        entry("Decode Text Escapes", editor::DecodeTextEscapes, ""),
        None,
        hash("Copy MD5", Hash::Md5),
        hash("Copy SHA-1", Hash::Sha1),
        hash("Copy SHA-256", Hash::Sha256),
        hash("Copy SHA-512", Hash::Sha512),
        None,
        entry("Hash File…", HashFile, ""),
    ]
}

pub(crate) fn groups() -> Commands {
    let new_group = |label, axis| entry(label, MoveToNewGroup { axis }, "");
    vec![
        entry("Split Right", SplitGroup { axis: Axis::Cols }, ""),
        entry("Split Down", SplitGroup { axis: Axis::Rows }, ""),
        None,
        new_group("Move File to New Group Right", Axis::Cols),
        new_group("Move File to New Group Down", Axis::Rows),
        None,
        entry("Close Group", CloseGroup, ""),
    ]
}

pub(crate) fn tabs(pinned: Option<bool>) -> Commands {
    let pin = match pinned {
        Some(true) => "Unpin Tab",
        Some(false) => "Pin Tab",
        None => "Pin / Unpin",
    };
    let add = |label, forward, hint| entry(label, AddTabToSelection { forward }, hint);
    let focus = |label, forward, hint| entry(label, FocusSelectedTab { forward }, hint);
    vec![
        entry("Keep Open", KeepOpen, "⌘K, ↩"),
        entry(pin, TogglePin, ""),
        None,
        entry("Close Other Tabs", CloseOtherTabs, ""),
        entry("Close Tabs to the Right", CloseTabsToRight, ""),
        None,
        add("Add Next Tab to Selection", true, "⇧⌥⌘→"),
        add("Add Previous Tab to Selection", false, "⇧⌥⌘←"),
        focus("Focus Next Selected Tab", true, "⌘K, ⌘]"),
        focus("Focus Previous Selected Tab", false, "⌘K, ⌘["),
        entry("Deselect Other Tabs", DeselectTabs, ""),
        None,
        entry("Move to New Window", MoveToNewWindow, ""),
        None,
    ]
}

pub(crate) fn pipelines() -> Commands {
    vec![
        entry("New Pipeline…", NewPipeline, ""),
        entry("Edit Pipeline…", EditPipeline, ""),
        None,
        entry("Preview Pipeline…", PreviewPipeline, ""),
        entry("Run Pipeline…", RunPipeline, ""),
        entry("Run Pipeline on Files…", RunPipelineOnFiles, ""),
        entry("Cancel Pipeline Run", CancelPipelineRun, ""),
    ]
}

pub(crate) fn lines() -> Commands {
    vec![
        entry("Indent", editor::Indent, "⌘]"),
        entry("Unindent", editor::Unindent, "⌘["),
        entry("Reindent Selection", editor::ReindentSelection, ""),
        entry("Swap Line Up", editor::MoveLineUp, "⌃⌘↑"),
        entry("Swap Line Down", editor::MoveLineDown, "⌃⌘↓"),
        entry("Duplicate Line", editor::DuplicateLine, "⇧⌘D"),
        entry("Delete Line", editor::DeleteLine, "⌃⇧K"),
        entry("Join Lines", editor::JoinLines, "⇧⌘J"),
    ]
}

pub(crate) fn subwords() -> Commands {
    let motion = |label, motion, select| entry(label, editor::Move { motion, select }, "");
    vec![
        motion("Previous Subword", Motion::SubwordLeft, false),
        motion("Next Subword", Motion::SubwordRight, false),
        motion("Select Previous Subword", Motion::SubwordLeft, true),
        motion("Select Next Subword", Motion::SubwordRight, true),
        entry("Delete Previous Subword", editor::DeleteSubword { forward: false }, ""),
        entry("Delete Next Subword", editor::DeleteSubword { forward: true }, ""),
    ]
}

pub(crate) fn surrounds() -> Commands {
    vec![entry("Change Surround…", ChangeSurroundPrompt, ""), entry("Delete Surround…", DeleteSurroundPrompt, "")]
}

pub(crate) fn word_transposition() -> Commands {
    vec![entry("Transpose Words", editor::TransposeWords, "")]
}

pub(crate) fn prose() -> Commands {
    let motion = |label, motion, select| entry(label, editor::Move { motion, select }, "");
    vec![
        motion("Previous Paragraph", Motion::ParagraphPrevious, false),
        motion("Next Paragraph", Motion::ParagraphNext, false),
        motion("Select Previous Paragraph", Motion::ParagraphPrevious, true),
        motion("Select Next Paragraph", Motion::ParagraphNext, true),
        motion("Previous Sentence", Motion::SentencePrevious, false),
        motion("Next Sentence", Motion::SentenceNext, false),
        motion("Select Previous Sentence", Motion::SentencePrevious, true),
        motion("Select Next Sentence", Motion::SentenceNext, true),
        entry("Select Sentence", editor::SelectSentence, ""),
        entry("Delete Sentence", editor::DeleteSentence, ""),
    ]
}

pub(crate) fn comments() -> Commands {
    vec![
        entry("Toggle Comment", editor::ToggleComment, "⌘/"),
        entry("Toggle Block Comment", editor::ToggleBlockComment, "⌥⌘/"),
    ]
}

pub(crate) fn marks() -> Commands {
    vec![
        entry("Set Mark", editor::SetMark, "⌘K, ⌘M"),
        entry("Select to Mark", editor::SelectToMark, "⌘K, ⌘A"),
        entry("Delete to Mark", editor::DeleteToMark, "⌘K, ⌘W"),
        entry("Swap with Mark", editor::SwapWithMark, "⌘K, ⌘X"),
        entry("Clear Mark", editor::ClearMark, "⌘K, ⌘G"),
        None,
        entry("Yank", editor::Yank, "⌃Y"),
    ]
}

pub(crate) fn saved_searches() -> Commands {
    vec![
        entry("Save Search…", SaveSearch, ""),
        entry("Open Saved Search…", OpenSavedSearch, ""),
        entry("Rename Saved Search…", RenameSavedSearch, ""),
        entry("Delete Saved Search…", DeleteSavedSearch, ""),
        None,
        entry("Import Saved Searches…", ImportSavedSearches, ""),
        entry("Export Saved Searches…", ExportSavedSearches, ""),
    ]
}

pub(crate) fn json() -> Commands {
    vec![entry("Format", FormatJson, ""), entry("Minify", MinifyJson, ""), entry("Validate", ValidateJson, "")]
}

pub(crate) fn macros() -> Commands {
    vec![
        entry("Start / Stop Recording", ToggleMacroRecording, "⌃Q"),
        entry("Replay Last", ReplayMacro, "⌃⇧Q"),
        entry("Repeat Last…", RepeatMacro, ""),
        entry("Save Last…", SaveMacro, ""),
        entry("Load and Replay…", LoadMacro, ""),
    ]
}

pub(crate) fn date_time() -> Commands {
    vec![entry("Insert Date/Time…", InsertDateTimePrompt, "")]
}

pub(crate) fn snippets() -> Commands {
    vec![entry("Insert Snippet…", InsertSnippet, ""), entry("New Snippet…", NewSnippet, "")]
}

pub(crate) fn templates() -> Commands {
    vec![entry("New From Template…", NewFromTemplate, "")]
}

pub(crate) fn file_history() -> Commands {
    vec![
        entry("Browse Versions…", BrowseFileHistory, ""),
        entry("Restore Previewed Version", RestoreFileHistory, ""),
        entry("Clear History…", ClearFileHistory, ""),
    ]
}

pub(crate) fn compare() -> Commands {
    vec![entry("Compare Files…", CompareFiles, ""), entry("Compare Folders…", CompareFolders, "")]
}

pub(crate) fn discarded_untitled() -> Commands {
    vec![
        entry("Restore Discarded Untitled…", BrowseDiscardedUntitled, ""),
        entry("Clear Discard History…", ClearDiscardHistory, ""),
    ]
}

pub(crate) fn selection_sets() -> Commands {
    vec![
        entry("Invert Selections", InvertSelectionsPrompt, ""),
        entry("Split Selections by Regex…", SplitSelectionsByRegexPrompt, ""),
        entry("Keep Selections Matching Regex…", KeepSelectionsMatchingRegexPrompt, ""),
        entry("Remove Matching Selections…", RemoveMatchingSelectionsPrompt, ""),
    ]
}

pub(crate) fn value_adjustments() -> Commands {
    vec![
        entry("Increment Values", IncrementValues, ""),
        entry("Decrement Values", DecrementValues, ""),
        entry("Adjust Values…", AdjustValuesPrompt, ""),
    ]
}

pub(crate) fn field_alignment() -> Commands {
    vec![
        entry("Align on Pattern…", AlignOnPatternPrompt, ""),
        entry("Align Selected Fields", editor::AlignSelectedFields, ""),
    ]
}

pub(crate) fn view_filters() -> Commands {
    vec![
        entry("Narrow to Selection", editor::NarrowToSelection, ""),
        entry("Narrow to Fold", editor::NarrowToFold, ""),
        entry("Show Matching Lines…", ShowMatchingLinesPrompt, ""),
        entry("Clear Filter", editor::ClearFilter, ""),
    ]
}

pub(crate) fn fold_navigation() -> Commands {
    use crate::editing::fold::Motion;
    vec![
        entry("Move to Next Fold", editor::MoveToFold { motion: Motion::Next, select: false }, ""),
        entry("Move to Previous Fold", editor::MoveToFold { motion: Motion::Previous, select: false }, ""),
        entry("Move to Parent Fold", editor::MoveToFold { motion: Motion::Parent, select: false }, ""),
        entry("Select to Next Fold", editor::MoveToFold { motion: Motion::Next, select: true }, ""),
        entry("Select to Previous Fold", editor::MoveToFold { motion: Motion::Previous, select: true }, ""),
        entry("Select to Parent Fold", editor::MoveToFold { motion: Motion::Parent, select: true }, ""),
        entry("Select Fold", editor::SelectFold, ""),
    ]
}

pub(crate) fn selected_targets() -> Commands {
    vec![entry("Open Selected Path/Address", OpenSelectedPath, ""), entry("Open Selected URL", OpenSelectedUrl, "")]
}

/// Explorer locations use a separate window-local history from editor jumps.
pub(crate) fn explorer_navigation() -> Commands {
    vec![
        entry("Explorer Back", ExplorerBack, "⌥← (Side Bar)"),
        entry("Explorer Forward", ExplorerForward, "⌥→ (Side Bar)"),
        entry("Explorer Parent", ExplorerParent, "⌥↑ (Side Bar)"),
        entry("Path Breadcrumbs…", PathBreadcrumbs, ""),
    ]
}
