//! Editor actions and their key bindings.

use crate::editing::convert::{Codec, Hash};
use crate::editing::csv::ColumnOp;
use crate::editing::text::{Case, Motion, Permute, Surround};
use crate::editing::transform::TextOp;
use gpui::{actions, KeyBinding};

// One action catalog drives declarations, dispatch, recording and JSON reconstruction.
macro_rules! editor_actions {
    ($operation:ident) => {
        $operation! {
            InsertText [insert, record] => insert_text,
            ToggleOvertype [unit, record] => toggle_overtype,
            InsertUnicodeCharacter [parameter, record] => insert_unicode_character,
            InsertLiteralNextKey [unit, ignore] => insert_literal_next_key,
            InsertLiteralText [parameter, record] => insert_literal_text,
            Backspace [unit, record] => backspace(edit_text => backspace),
            Delete [unit, record] => delete(edit_text => delete),
            Newline [unit, record] => newline,
            Tab [unit, record] => tab,
            Indent [unit, record] => indent(edit_text => indent),
            Unindent [unit, record] => unindent(edit_text => unindent),
            ReindentSelection [unit, record] => reindent_selection,
            SelectAll [unit, record] => select_all(edit => select_all),
            SelectLine [unit, record] => select_line,
            SelectSentence [unit, record] => select_sentence(edit => select_sentence),
            DeleteSentence [unit, record] => delete_sentence(edit_text => delete_sentence),
            InvertSelections [unit, record] => invert_selections,
            SplitSelectionsByRegex [parameter, record] => split_selections_by_regex,
            KeepSelectionsMatchingRegex [parameter, record] => keep_selections_matching_regex,
            RemoveMatchingSelections [parameter, record] => remove_matching_selections,
            SplitIntoLines [unit, record] => split_into_lines(edit => split_into_lines),
            SelectNextOccurrence [unit, record] => select_next(edit => select_next_occurrence),
            SingleSelection [unit, record] => single_selection,
            DuplicateLine [unit, record] => duplicate_line(edit_text => duplicate_line),
            DeleteLine [unit, record] => delete_line(edit_text => delete_line),
            FieldNewline [unit, ignore] => field_newline,
            MoveLineUp [unit, record] => move_line_up(edit_text => move_line_up),
            MoveLineDown [unit, record] => move_line_down(edit_text => move_line_down),
            InsertLineAfter [unit, record] => insert_line_after(edit_text => insert_line_after),
            InsertLineBefore [unit, record] => insert_line_before(edit_text => insert_line_before),
            ToggleComment [unit, record] => toggle_comment,
            Copy [unit, record] => copy,
            CopyAsHtml [unit, record] => copy_as_html,
            ToggleSpellCheck [unit, record] => toggle_spell_check,
            NextMisspelling [unit, record] => next_misspelling,
            PrevMisspelling [unit, record] => prev_misspelling,
            SpellingSuggestions [unit, record] => spelling_suggestions,
            Cut [unit, record] => cut,
            Paste [unit, record] => paste,
            Undo [unit, record] => undo(edit_text => undo),
            Redo [unit, record] => redo(edit_text => redo),
            UndoSelection [unit, record] => undo_selection,
            RedoSelection [unit, record] => redo_selection,
            QuickSkipNext [unit, record] => quick_skip_next,
            Move [parameter, record] => r#move,
            SelectLines [parameter, record] => select_lines,
            SelectAllOccurrences [unit, record] => select_all_occurrences(edit => select_all_occurrences),
            DeleteWord [parameter, record] => delete_word,
            DeleteSubword [parameter, record] => delete_subword,
            Transpose [unit, record] => transpose(edit_text => transpose),
            TransposeWords [unit, record] => transpose_words,
            AdjustValues [parameter, record] => adjust_values,
            AlignOnPattern [parameter, record] => align_on_pattern,
            AlignSelectedFields [unit, record] => align_selected_fields,
            ChangeSurround [parameter, record] => change_surround,
            DeleteSurround [parameter, record] => delete_surround,
            JoinLines [unit, record] => join_lines(edit_text => join_lines),
            SetMark [unit, record] => set_mark(edit => set_mark),
            SelectToMark [unit, record] => select_to_mark(edit => select_to_mark),
            DeleteToMark [unit, record] => delete_to_mark(edit_text => delete_to_mark),
            SwapWithMark [unit, record] => swap_with_mark(edit => swap_with_mark),
            ClearMark [unit, record] => clear_mark(edit => clear_mark),
            Yank [unit, record] => yank(edit_text => yank),
            SelectLineUpward [unit, record] => select_line_upward,
            ExpandToParagraph [unit, record] => expand_to_paragraph(edit => expand_to_paragraph),
            ExpandToIndentation [unit, record] => expand_to_indentation(edit => expand_to_indentation),
            ExpandToBrackets [unit, record] => expand_to_brackets(edit => expand_to_brackets),
            ExpandSyntaxSelection [unit, record] => expand_syntax_selection,
            ShrinkSyntaxSelection [unit, record] => shrink_syntax_selection,
            ExpandSmart [unit, record] => expand_smart(edit => expand_smart),
            JumpToBracket [unit, record] => jump_to_bracket(edit => jump_to_bracket),
            FindUnder [unit, record] => find_under(edit => find_under),
            PasteAndIndent [unit, record] => paste_and_indent,
            ToggleBookmark [unit, record] => toggle_bookmark,
            NextBookmark [parameter, record] => next_bookmark,
            ClearBookmarks [unit, record] => clear_bookmarks,
            SelectAllBookmarks [unit, record] => select_all_bookmarks(edit => select_all_bookmarks),
            ToggleBlockComment [unit, record] => toggle_block_comment,
            DeleteToLineEdge [parameter, record] => delete_to_line_edge,
            ConvertCase [parameter, record] => convert_case,
            Convert [parameter, record] => convert,
            DecodeTextEscapes [unit, record] => decode_text_escapes,
            TransformText [parameter, record] => transform_text,
            CopyHash [parameter, record] => copy_hash,
            PermuteLines [parameter, record] => permute_lines,
            PermuteSelections [parameter, record] => permute_selections,
            ColumnCommand [parameter, record] => column_command,
            BookmarkedLines [parameter, record] => bookmarked_lines,
            HighlightSelection [parameter, record] => highlight_selection,
            NextHighlight [parameter, record] => next_highlight,
            CopyHighlighted [parameter, record] => copy_highlighted,
            ClearHighlights [parameter, record] => clear_highlights,
            WrapLines [parameter, record] => wrap_lines,
            ConvertIndentation [parameter, record] => convert_indentation,
            TrimTrailingWhitespace [unit, record] => trim_trailing_whitespace,
            RemoveBlankLines [unit, record] => remove_blank_lines,
            JumpBack [unit, record] => jump_back,
            JumpForward [unit, record] => jump_forward,
            ScrollLines [parameter, record] => scroll_lines,
            ShowAtCenter [unit, record] => show_at_center,
            NarrowToSelection [unit, record] => narrow_to_selection,
            NarrowToFold [unit, record] => narrow_to_fold,
            ShowMatchingLines [parameter, record] => show_matching_lines,
            ClearFilter [unit, record] => clear_filter,
            MoveToFold [parameter, record] => move_to_fold,
            SelectFold [unit, record] => select_fold,
            Fold [unit, record] => fold,
            Unfold [unit, record] => unfold,
            UnfoldAll [unit, record] => unfold_all,
            FoldLevel [parameter, record] => fold_level,
            ToggleFoldAt [parameter, build] => toggle_fold_at,
            OpenFold [parameter, ignore] => open_fold,
            AutoComplete [unit, ignore] => auto_complete,
            CompletionNext [unit, ignore] => completion_next,
            CompletionPrev [unit, ignore] => completion_prev,
            CompletionCommit [unit, ignore] => completion_commit,
            CompletionClose [parameter, build] => completion_close,
            NextField [unit, record] => next_field,
            PrevField [unit, record] => prev_field,
            ClearFields [unit, record] => clear_fields,
            ExpandSnippet [parameter, record] => expand_snippet,
            InsertDateTime [parameter, record] => insert_date_time,
        }
    };
}
pub(crate) use editor_actions;

macro_rules! declare_unit_action {
    ($ty:ident, unit) => {
        actions!(editor, [$ty]);
    };
    ($ty:ident, $kind:ident) => {};
}
macro_rules! declare_actions {
    ($($ty:ident [$kind:ident, $policy:ident] => $handler:ident $(($mode:ident => $method:ident))?,)*) => {
        $(declare_unit_action!($ty, $kind);)*
    };
}
editor_actions!(declare_actions);

fn date_time_format() -> String {
    orlo_utils::date_time::DEFAULT_FORMAT.into()
}

parameter_actions! { editor, serializable;
    InsertUnicodeCharacter { value: String, }
    /// Resolved literal input, for replay without transient keyboard/source state.
    InsertLiteralText { text: String, #[serde(default)] replace: Option<[isize; 2]>, }

    InsertDateTime { #[serde(default = "date_time_format")] format: String, #[serde(default)] utc: bool, }

    /// A snippet expanded at the carets, in place of `trigger` before each (of each selection
    /// when empty); the quick panel's rows dispatch it.
    ExpandSnippet { path: std::path::PathBuf, trigger: String, }

    /// Close the completion list; with `fall_through` (Shift-Tab) the key then does what it
    /// does without the list.
    CompletionClose { fall_through: bool, }

    /// Edit > Code Folding > Fold Level N: the regions whose header is at indent level N - 1.
    FoldLevel { level: usize, }

    MoveToFold { motion: crate::editing::fold::Motion, #[serde(default)] select: bool, }

    ShowMatchingLines { pattern: String, #[serde(default)] context: usize, #[serde(default)] ignore_case: bool, }

    /// A gutter fold button: fold the region `line` heads, or unfold it.
    ToggleFoldAt { line: usize, }

    /// A click on a fold's placeholder: open the outermost fold that starts at `at`.
    OpenFold { at: usize, }

    Move { motion: Motion, select: bool, }

    SelectLines { forward: bool, }

    DeleteWord { forward: bool, }

    DeleteSubword { forward: bool, }

    DeleteToLineEdge { forward: bool, }

    NextBookmark { forward: bool, }

    ScrollLines { up: bool, }

    SplitSelectionsByRegex { pattern: String, #[serde(default)] ignore_case: bool, }
    KeepSelectionsMatchingRegex { pattern: String, #[serde(default)] ignore_case: bool, }
    RemoveMatchingSelections { pattern: String, #[serde(default)] ignore_case: bool, }

    ChangeSurround {
        target: Surround,
        replacement: Surround,
        #[serde(default = "surround_count")]
        count: std::num::NonZeroUsize,
    }

    DeleteSurround {
        target: Surround,
        #[serde(default = "surround_count")]
        count: std::num::NonZeroUsize,
    }

    AdjustValues { #[serde(default)] step: crate::editing::columns::ValueStep, }

    AlignOnPattern { pattern: String, #[serde(default)] capture: usize, #[serde(default)] ignore_case: bool, }

    ConvertCase { case: Case, }

    PermuteLines { op: Permute, }

    PermuteSelections { op: Permute, }

    WrapLines {
        /// Columns; 0 for Sublime's default (the first ruler, else 78).
        width: usize,
    }

    ConvertIndentation { to_spaces: bool, }

    /// Edit > Transform: quotes and Unicode forms, on each selection.
    TransformText { op: TextOp, }

    /// Edit > Convert: named text codecs; literal regex encoding and JSON string quoting.
    Convert { codec: Codec, decode: bool, }

    /// Edit > Convert > Copy MD5, SHA-1, SHA-256 and SHA-512.
    CopyHash { hash: Hash, }
}

#[cfg(test)]
impl Move {
    /// Tests: `motion`, moving the carets or (`select`) extending the selections.
    pub(crate) fn to(motion: Motion, select: bool) -> Self {
        Self { motion, select }
    }
}

/// What Goto > Bookmarks does with the bookmarked lines.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkedLines {
    Copy,
    Cut,
    Delete,
    KeepOnly,
    Paste,
    Invert,
}

parameter_actions! { editor, serializable;
    BookmarkedLines { op: MarkedLines, }

    /// The selections marked in highlight style `style` (0-based).
    HighlightSelection { style: usize, }

    NextHighlight { style: usize, forward: bool, }

    CopyHighlighted { style: usize, }

    /// One style's highlights cleared, or (`None`) every style's.
    ClearHighlights { style: Option<usize>, }

    /// Edit > Columns: a CSV/TSV column command on the field at the caret.
    ColumnCommand { op: ColumnOp, }
}

fn surround_count() -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::MIN
}

pub const PAGE_LINES: usize = 40;

pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("Editor");
    // line commands make no sense in a one-line input (the find field, the palette)
    let lines = Some("Editor && !single_line");
    let sort = |case_sensitive| PermuteLines { op: Permute::Sort { case_sensitive } };
    let mut b = vec![
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("enter", Newline, lines),
        // Sublime's: a line break in the Find and Replace fields, whose Enter finds
        KeyBinding::new("ctrl-enter", FieldNewline, Some("Editor && multiline")),
        KeyBinding::new("tab", Tab, lines),
        KeyBinding::new("shift-tab", Unindent, lines),
        KeyBinding::new("cmd-]", Indent, lines),
        KeyBinding::new("cmd-[", Unindent, lines),
        KeyBinding::new("cmd-a", SelectAll, ctx),
        KeyBinding::new("cmd-l", SelectLine, ctx),
        KeyBinding::new("cmd-shift-l", SplitIntoLines, lines),
        KeyBinding::new("cmd-d", SelectNextOccurrence, ctx),
        KeyBinding::new("escape", SingleSelection, ctx),
        KeyBinding::new("cmd-shift-d", DuplicateLine, lines),
        KeyBinding::new("ctrl-shift-k", DeleteLine, lines),
        KeyBinding::new("ctrl-cmd-up", MoveLineUp, lines),
        KeyBinding::new("ctrl-cmd-down", MoveLineDown, lines),
        KeyBinding::new("ctrl-shift-up", SelectLines { forward: false }, lines),
        KeyBinding::new("ctrl-shift-down", SelectLines { forward: true }, lines),
        KeyBinding::new("ctrl-cmd-g", SelectAllOccurrences, lines),
        KeyBinding::new("alt-backspace", DeleteWord { forward: false }, ctx),
        KeyBinding::new("alt-delete", DeleteWord { forward: true }, ctx),
        KeyBinding::new("cmd-y", Redo, ctx),
        KeyBinding::new("cmd-enter", InsertLineAfter, lines),
        KeyBinding::new("cmd-shift-enter", InsertLineBefore, lines),
        KeyBinding::new("cmd-/", ToggleComment, lines),
        KeyBinding::new("cmd-c", Copy, ctx),
        KeyBinding::new("cmd-x", Cut, ctx),
        KeyBinding::new("cmd-v", Paste, ctx),
        KeyBinding::new("cmd-z", Undo, ctx),
        KeyBinding::new("cmd-shift-z", Redo, ctx),
        KeyBinding::new("cmd-u", UndoSelection, ctx),
        KeyBinding::new("cmd-shift-u", RedoSelection, ctx),
        KeyBinding::new("cmd-k cmd-d", QuickSkipNext, ctx),
        KeyBinding::new("ctrl-t", Transpose, ctx),
        KeyBinding::new("cmd-shift-j", JoinLines, lines),
        KeyBinding::new("cmd-k cmd-m", SetMark, ctx),
        KeyBinding::new("cmd-k cmd-a", SelectToMark, ctx),
        KeyBinding::new("cmd-k cmd-w", DeleteToMark, ctx),
        KeyBinding::new("cmd-k cmd-x", SwapWithMark, ctx),
        KeyBinding::new("cmd-k cmd-g", ClearMark, ctx),
        KeyBinding::new("ctrl-y", Yank, ctx),
        KeyBinding::new("cmd-k cmd-y", Yank, ctx),
        KeyBinding::new("cmd-alt-l", SelectLineUpward, lines),
        KeyBinding::new("cmd-shift-a", ExpandSmart, ctx),
        KeyBinding::new("ctrl-shift-m", ExpandToBrackets, ctx),
        KeyBinding::new("ctrl-m", JumpToBracket, ctx),
        KeyBinding::new("cmd-alt-g", FindUnder, ctx),
        KeyBinding::new("cmd-shift-v", PasteAndIndent, lines),
        KeyBinding::new("cmd-f2", ToggleBookmark, lines),
        KeyBinding::new("shift-f2", NextBookmark { forward: false }, lines),
        KeyBinding::new("cmd-shift-f2", ClearBookmarks, lines),
        KeyBinding::new("alt-f2", SelectAllBookmarks, lines),
        KeyBinding::new("cmd-alt-/", ToggleBlockComment, lines),
        KeyBinding::new("ctrl-l", ShowAtCenter, lines),
        KeyBinding::new("cmd-k cmd-c", ShowAtCenter, lines),
        KeyBinding::new("ctrl-alt-up", ScrollLines { up: true }, lines),
        KeyBinding::new("ctrl-alt-down", ScrollLines { up: false }, lines),
        KeyBinding::new("ctrl-k", DeleteToLineEdge { forward: true }, ctx),
        KeyBinding::new("cmd-delete", DeleteToLineEdge { forward: true }, ctx),
        KeyBinding::new("cmd-k cmd-k", DeleteToLineEdge { forward: true }, ctx),
        KeyBinding::new("cmd-backspace", DeleteToLineEdge { forward: false }, ctx),
        KeyBinding::new("cmd-k cmd-backspace", DeleteToLineEdge { forward: false }, ctx),
        KeyBinding::new("cmd-k cmd-u", ConvertCase { case: Case::Upper }, ctx),
        KeyBinding::new("cmd-k cmd-l", ConvertCase { case: Case::Lower }, ctx),
        KeyBinding::new("f5", sort(false), lines),
        KeyBinding::new("ctrl-f5", sort(true), lines),
        KeyBinding::new("cmd-alt-q", WrapLines { width: 0 }, lines),
        KeyBinding::new("ctrl--", JumpBack, lines),
        KeyBinding::new("ctrl-shift--", JumpForward, lines),
        KeyBinding::new("cmd-alt-[", Fold, lines),
        KeyBinding::new("cmd-alt-]", Unfold, lines),
        // Sublime's other key for it, cmd-k cmd-j, is Orlo's Show / Hide Panel
        KeyBinding::new("cmd-k cmd-0", UnfoldAll, lines),
        KeyBinding::new("ctrl-space", AutoComplete, lines),
        KeyBinding::new("f6", ToggleSpellCheck, lines),
        KeyBinding::new("ctrl-f6", NextMisspelling, lines),
        KeyBinding::new("ctrl-shift-f6", PrevMisspelling, lines),
        // macOS's own completion key, as ctrl-space can be taken by input-source switching; second,
        // so the menu keeps showing ctrl-space
        KeyBinding::new("alt-escape", AutoComplete, lines),
    ];
    for level in 1..=9 {
        let keys = format!("cmd-k cmd-{level}");
        b.push(KeyBinding::new(&keys, FoldLevel { level }, lines));
    }
    let motions = [
        ("left", Motion::Left),
        ("right", Motion::Right),
        ("up", Motion::Up),
        ("down", Motion::Down),
        ("alt-left", Motion::WordLeft),
        ("alt-right", Motion::WordRight),
        ("ctrl-left", Motion::WordLeft),
        ("ctrl-right", Motion::WordRight),
        ("cmd-left", Motion::LineStart),
        ("cmd-right", Motion::LineEnd),
        ("ctrl-a", Motion::HardLineStart),
        ("ctrl-e", Motion::HardLineEnd),
        ("home", Motion::LineStart),
        ("end", Motion::LineEnd),
        ("cmd-up", Motion::DocStart),
        ("cmd-down", Motion::DocEnd),
        ("pageup", Motion::PageUp(PAGE_LINES)),
        ("pagedown", Motion::PageDown(PAGE_LINES)),
    ];
    for (keys, motion) in motions {
        for select in [false, true] {
            let keys = match select {
                true => format!("shift-{keys}"),
                false => keys.to_string(),
            };
            b.push(KeyBinding::new(&keys, Move { motion, select }, ctx));
        }
    }
    // a snippet's fields: over the editor's keys, under the completion list's
    let snippet = Some("Editor && snippet");
    b.extend([
        KeyBinding::new("tab", NextField, snippet),
        KeyBinding::new("shift-tab", PrevField, snippet),
        KeyBinding::new("escape", ClearFields, snippet),
    ]);
    let choice = Some("Editor && snippet_choice");
    b.extend([
        KeyBinding::new("enter", CompletionCommit, choice),
        KeyBinding::new("down", CompletionNext, choice),
        KeyBinding::new("up", CompletionPrev, choice),
        KeyBinding::new("escape", CompletionClose { fall_through: false }, choice),
    ]);
    let signature = Some("Editor && signature");
    b.extend([
        KeyBinding::new("down", CompletionNext, signature),
        KeyBinding::new("up", CompletionPrev, signature),
        KeyBinding::new("escape", CompletionClose { fall_through: false }, signature),
    ]);
    // last: with the list open its keys win over the editor's own (the keymap's later binding)
    let completing = Some("Editor && completing");
    b.extend([
        KeyBinding::new("tab", CompletionCommit, completing),
        KeyBinding::new("enter", CompletionCommit, completing),
        KeyBinding::new("down", CompletionNext, completing),
        KeyBinding::new("up", CompletionPrev, completing),
        KeyBinding::new("escape", CompletionClose { fall_through: false }, completing),
        KeyBinding::new("shift-tab", CompletionClose { fall_through: true }, completing),
    ]);
    b
}
