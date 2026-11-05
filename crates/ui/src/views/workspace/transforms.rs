//! Edit > Transform: the line, quote, Unicode and substitution-table transformations that ask first.

use super::*;
use crate::editing::transform::{
    self, Compare, DupAction, DupOpts, MatchAction, MatchOpts, SortOpts, SortOrder, Table, TableOpts,
};
use std::cell::RefCell;

/// The form's options, in the order the form offers them.
const SORT_ORDER: [SortOrder; 4] = [SortOrder::Lexical, SortOrder::Natural, SortOrder::Locale, SortOrder::Length];
const COMPARE: [Compare; 3] = [Compare::Exact, Compare::IgnoreCase, Compare::IgnoreSpace];
const DUP_ACTIONS: [DupAction; 4] =
    [DupAction::KeepFirst, DupAction::KeepLast, DupAction::DeleteAll, DupAction::KeepOnlyDuplicated];
const MATCH_ACTIONS: [MatchAction; 4] =
    [MatchAction::Keep, MatchAction::Delete, MatchAction::Copy, MatchAction::Extract];
/// Problems a form shows before "and N more": the panel is one line of the window.
const SHOWN_PROBLEMS: usize = 12;
const MAX_TABLE_BYTES: u64 = 16 * 1024 * 1024;

/// `f` over the view's selections (the document when none) as one edit; `Err` changes nothing.
pub(super) fn apply(
    editor: &Entity<Editor>,
    cx: &mut App,
    lines: bool,
    f: impl Fn(&str) -> Result<String, String>,
) -> Result<(), String> {
    editor.update(cx, |e, cx| e.edit(cx, |s| s.transform_selections(lines, f)))
}

impl Workspace {
    forward_actions! {
        pub(crate) change_surround_prompt(ChangeSurroundPrompt, window, cx) => surround_form(true, window, cx);
        pub(crate) delete_surround_prompt(DeleteSurroundPrompt, window, cx) => surround_form(false, window, cx);
    }

    fn surround_form(&mut self, change: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.active_editor() else { return };
        let (buffer, version, selection, generation) = target.update(cx, |e, cx| {
            e.catch_up(cx);
            // Settle the previous command before pinning the frame-checkpoint generation.
            e.sel(cx).sel_checkpoint();
            let state = e.read_state(cx);
            (state.buffer_id(), state.doc.version, state.selection_state(), state.selection_generation())
        });
        let options = vec!["(", "[", "{", "'", "\""];
        let mut fields = vec![Field::choice("target", "Existing pair", options.clone())];
        if change {
            fields.push(Field::choice("replacement", "Replacement pair", vec!["[", "(", "{", "'", "\""]));
        }
        fields.push(Field::text("count", "Outer pair count", "1"));
        let done: OnDone = Box::new(move |values, editor, ws, window, cx| {
            let count = values
                .text("count")
                .trim()
                .parse::<std::num::NonZeroUsize>()
                .map_err(|_| "Outer pair count must be a positive whole number".to_string())?;
            let current = editor.update(cx, |e, cx| {
                e.catch_up(cx);
                let state = e.read_state(cx);
                state.buffer_id() == buffer
                    && state.doc.version == version
                    && state.selection_state() == selection
                    && state.selection_generation() == generation
            });
            if !current {
                return Err("The document or selections changed; reopen Surround".into());
            }
            let mut args = serde_json::json!({"target": options[values.choice("target").min(4)], "count": count});
            if change {
                args["replacement"] =
                    serde_json::json!(["[", "(", "{", "'", "\""][values.choice("replacement").min(4)]);
            }
            ws.execute_command(
                &crate::keybindings::Command {
                    command: if change { "editor::ChangeSurround" } else { "editor::DeleteSurround" }.into(),
                    args,
                },
                window,
                cx,
            )
        });
        self.show_form(if change { "Change Surround" } else { "Delete Surround" }, fields, done, window, cx);
    }

    pub(crate) fn insert_date_time_prompt(
        &mut self,
        _: &InsertDateTimePrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.active_editor() else { return };
        if let Err(error) = self.selection_set_admission(&target, cx) {
            return self.status_message(error, cx);
        }
        let (buffer, version, selection, generation) = target.update(cx, |e, cx| {
            e.catch_up(cx);
            e.sel(cx).sel_checkpoint();
            let s = e.read_state(cx);
            (s.buffer_id(), s.doc.version, s.selection_state(), s.selection_generation())
        });
        let done: OnDone = Box::new(move |values, editor, ws, _, cx| {
            orlo_utils::date_time::validate_format(values.text("format"))?;
            ws.selection_set_admission(&editor, cx)?;
            let current = editor.update(cx, |e, cx| {
                e.catch_up(cx);
                let s = e.read_state(cx);
                s.buffer_id() == buffer
                    && s.doc.version == version
                    && s.selection_state() == selection
                    && s.selection_generation() == generation
            });
            if !current {
                return Err("The document or selections changed; reopen Insert Date/Time".into());
            }
            let action =
                editor::InsertDateTime { format: values.text("format").into(), utc: values.choice("zone") == 1 };
            editor.update(cx, |e, cx| e.try_insert_date_time(&action, cx))?;
            crate::config::customization::record(
                crate::keybindings::Command {
                    command: "editor::InsertDateTime".into(),
                    args: serde_json::json!({"format":action.format,"utc":action.utc}),
                },
                cx,
            );
            Ok(())
        });
        self.show_form(
            "Insert Date/Time",
            vec![
                Field::text("format", "Format", orlo_utils::date_time::DEFAULT_FORMAT),
                Field::choice("zone", "Time zone", vec!["Local", "UTC"]),
            ],
            done,
            window,
            cx,
        );
    }

    pub(crate) fn bound_insert_date_time(
        &mut self,
        action: &editor::InsertDateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else { return };
        if !editor.read(cx).is_focused(window) {
            return;
        }
        if let Err(error) = self.selection_set_admission(&editor, cx) {
            return self.status_message(error, cx);
        }
        editor.update(cx, |e, cx| e.insert_date_time(action, window, cx));
    }

    /// Edit > Transform > Sort Lines by Pattern…
    pub(crate) fn sort_lines_by_pattern(
        &mut self,
        _: &SortLinesByPattern,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields = vec![
            Field::text("pattern", "Pattern", ""),
            Field::text("capture", "Capture (blank = automatic, 0 = match)", ""),
            Field::choice("order", "Order", vec!["lexical", "natural", "current macOS locale", "grapheme length"]),
            Field::toggle("case_insensitive", "Ignore case", false),
            Field::toggle("numeric", "Numeric", false),
            Field::toggle("reverse", "Reverse", false),
            Field::toggle("keep_header", "Keep First Selected Line as Header", false),
        ];
        let done: OnDone = Box::new(|v, editor, _, _, cx| {
            let o = SortOpts {
                pattern: v.text("pattern").to_string(),
                capture: match v.text("capture").trim() {
                    "" => None,
                    value => Some(value.parse::<usize>().map_err(|_| "Enter a nonnegative capture index".to_string())?),
                },
                order: SORT_ORDER[v.choice("order").min(3)],
                keep_header: v.flag("keep_header"),
                case_insensitive: v.flag("case_insensitive"),
                numeric: v.flag("numeric"),
                reverse: v.flag("reverse"),
            };
            // a bad pattern is said before any text is read, without a line number
            transform::sort_by_pattern("", &o)?;
            apply(&editor, cx, true, |t| transform::sort_by_pattern(t, &o))
        });
        self.show_form("Sort Lines by Pattern", fields, done, window, cx);
    }

    /// Edit > Transform > Process Duplicate Lines…
    pub(crate) fn process_duplicate_lines(
        &mut self,
        _: &ProcessDuplicateLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields = vec![
            Field::text("pattern", "Key pattern", ""),
            Field::choice("compare", "Compare", vec!["exact", "ignore case", "ignore surrounding white space"]),
            Field::choice(
                "action",
                "Action",
                vec!["keep first", "keep last", "delete all duplicated", "keep only duplicated"],
            ),
            Field::toggle("copy_removed", "Copy removed lines", false),
        ];
        let done: OnDone = Box::new(|v, editor, ws, _, cx| {
            let o = DupOpts {
                pattern: v.text("pattern").to_string(),
                compare: COMPARE[v.choice("compare").min(2)],
                action: DUP_ACTIONS[v.choice("action").min(3)],
            };
            transform::process_duplicates("", &o)?;
            let removed = RefCell::new(Vec::new());
            apply(&editor, cx, true, |t| {
                let (kept, gone) = transform::process_duplicates(t, &o)?;
                removed.borrow_mut().push(gone);
                Ok(kept)
            })?;
            let removed = removed.into_inner().concat();
            if v.flag("copy_removed") && !removed.is_empty() {
                ws.status_message(format!("Copied {} removed lines", removed.lines().count()), cx);
                editor::copy_to_clipboard(cx, (removed, false));
            }
            Ok(())
        });
        self.show_form("Process Duplicate Lines (empty key = whole line)", fields, done, window, cx);
    }

    /// Edit > Transform > Process Lines Matching…
    pub(crate) fn process_lines_matching(
        &mut self,
        _: &ProcessLinesMatching,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields = vec![
            Field::text("pattern", "Pattern", ""),
            Field::toggle("regex", "Regex", true),
            Field::toggle("case_insensitive", "Ignore case", false),
            Field::toggle("invert", "Invert match", false),
            Field::choice("action", "Action", vec!["keep", "delete", "copy", "extract to new document"]),
        ];
        let done: OnDone = Box::new(|v, editor, ws, window, cx| {
            let o = MatchOpts {
                pattern: v.text("pattern").to_string(),
                regex: v.flag("regex"),
                case_insensitive: v.flag("case_insensitive"),
                invert: v.flag("invert"),
                action: MATCH_ACTIONS[v.choice("action").min(3)],
            };
            transform::process_matching("", &o)?;
            if matches!(o.action, MatchAction::Keep | MatchAction::Delete) {
                return apply(&editor, cx, true, |t| transform::process_matching(t, &o).map(|(kept, _)| kept));
            }
            // copy and extract read the lines and leave the document alone
            let texts = editor.update(cx, |e, cx| sel_of(&e.document, &mut e.state, cx).transform_texts(true));
            let mut matched = Vec::new();
            for t in texts {
                matched.push(transform::process_matching(&t, &o)?.1);
            }
            let matched = matched.concat();
            if matched.is_empty() {
                return Err("No lines match".into());
            }
            if o.action == MatchAction::Copy {
                ws.status_message(format!("Copied {} lines", matched.lines().count()), cx);
                editor::copy_to_clipboard(cx, (matched, false));
            } else {
                let mut doc = EditorState::from_source(&matched);
                // unsaved, so closing it asks
                doc.forget_saved_point();
                ws.add_tab(doc, window, cx);
            }
            Ok(())
        });
        self.show_form("Process Lines Matching", fields, done, window, cx);
    }

    /// Edit > Transform > Add / Remove Line Prefix / Suffix…
    pub(crate) fn affix_lines(&mut self, a: &AffixLines, window: &mut Window, cx: &mut Context<Self>) {
        let (suffix, remove) = (a.suffix, a.remove);
        let label = if suffix { "Suffix" } else { "Prefix" };
        let title = format!("{} Line {label}", if remove { "Remove" } else { "Add" });
        let mut fields = vec![Field::text("text", label, "")];
        if !remove {
            fields.push(Field::toggle("skip_blank", "Skip blank lines", true));
        }
        let done: OnDone = Box::new(move |v, editor, _, _, cx| {
            let t = v.text("text");
            if t.is_empty() {
                return Err(format!("Enter the {} text", label.to_lowercase()));
            }
            let skip = v.flag("skip_blank");
            apply(&editor, cx, true, |lines| {
                Ok(match (suffix, remove) {
                    (false, false) => transform::prefix(lines, t, skip),
                    (true, false) => transform::suffix(lines, t, skip),
                    (false, true) => transform::remove_prefix(lines, t),
                    (true, true) => transform::remove_suffix(lines, t),
                })
            })
        });
        self.show_form(&title, fields, done, window, cx);
    }

    forward_actions! {
    /// Edit > Transform > Apply Substitution Table…: choose the table, then its options.
    /// Gpui 0.2.2's file dialog cannot filter to .tsv/.txt, so any file can be chosen.
    pub(crate) apply_substitution_table(ApplySubstitutionTable, window, cx) => prompt_path(files::picker(true, false, false), window, cx, Self::substitution_form);
    }

    /// The options for a table file. The file is read at Enter, so a fix on disk needs no
    /// second dialog; its path stays editable (and gives the panel a field to take Enter).
    pub(crate) fn substitution_form(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let fields = vec![
            Field::text("file", "Table file", path.to_string_lossy()),
            Field::choice("mode", "Mode", vec!["literal", "regex"]),
            Field::toggle("case_insensitive", "Ignore case", false),
            Field::toggle("whole_word", "Whole word", false),
        ];
        let done: OnDone = Box::new(|v, editor, _, _, cx| {
            let o = TableOpts {
                regex: v.choice("mode") == 1,
                case_insensitive: v.flag("case_insensitive"),
                whole_word: v.flag("whole_word"),
            };
            let file = v.text("file").trim();
            let size = std::fs::metadata(file).map_err(|e| format!("{file}: {e}"))?.len();
            if size > MAX_TABLE_BYTES {
                return Err(format!("{file}: a substitution table is limited to 16 MiB"));
            }
            let rows = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
            let table = Table::parse(rows.trim_start_matches('\u{feff}'), &o).map_err(|mut e| {
                let more = e.len().saturating_sub(SHOWN_PROBLEMS);
                e.truncate(SHOWN_PROBLEMS);
                if more > 0 {
                    e.push(format!("…and {more} more"));
                }
                e.join("\n")
            })?;
            apply(&editor, cx, false, |t| Ok(table.apply(t)))
        });
        self.show_form("Apply Substitution Table", fields, done, window, cx);
    }
}

impl Workspace {
    forward_actions! {
        pub(crate) split_selections_by_regex_prompt(SplitSelectionsByRegexPrompt, window, cx) => selection_regex_form("Split Selections by Regex", "editor::SplitSelectionsByRegex", window, cx);
        pub(crate) keep_selections_matching_regex_prompt(KeepSelectionsMatchingRegexPrompt, window, cx) => selection_regex_form("Keep Selections Matching Regex", "editor::KeepSelectionsMatchingRegex", window, cx);
        pub(crate) remove_matching_selections_prompt(RemoveMatchingSelectionsPrompt, window, cx) => selection_regex_form("Remove Matching Selections", "editor::RemoveMatchingSelections", window, cx);
    }

    pub(crate) fn selection_set_admission(&self, editor: &Entity<Editor>, cx: &App) -> Result<(), String> {
        let document = &editor.read(cx).document;
        if self.open_editors(cx).iter().chain(std::iter::once(editor)).any(|other| {
            let other = other.read(cx);
            other.document == *document && !other.marked.is_empty()
        }) {
            return Err("Finish or cancel text composition before changing selections".into());
        }
        Ok(())
    }

    pub(crate) fn invert_selections_prompt(
        &mut self,
        _: &InvertSelectionsPrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command =
            crate::keybindings::Command { command: "editor::InvertSelections".into(), args: serde_json::json!({}) };
        if let Err(error) = self.execute_command(&command, window, cx) {
            self.status_message(error, cx);
        }
    }

    fn selection_regex_form(
        &mut self,
        title: &'static str,
        command: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.active_editor() else { return };
        if let Err(error) = self.selection_set_admission(&target, cx) {
            return self.status_message(error, cx);
        }
        let (buffer, version, selection, generation) = target.update(cx, |e, cx| {
            e.catch_up(cx);
            e.sel(cx).sel_checkpoint();
            let state = e.read_state(cx);
            (state.buffer_id(), state.doc.version, state.selection_state(), state.selection_generation())
        });
        let done: OnDone = Box::new(move |values, editor, ws, window, cx| {
            ws.selection_set_admission(&editor, cx)?;
            let current = editor.update(cx, |e, cx| {
                e.catch_up(cx);
                let state = e.read_state(cx);
                state.buffer_id() == buffer
                    && state.doc.version == version
                    && state.selection_state() == selection
                    && state.selection_generation() == generation
            });
            if !current {
                return Err("The document or selections changed; reopen the selection command".into());
            }
            orlo_editor::search::pattern::Pattern::new(
                values.text("pattern"),
                &crate::editing::text::FindOpts {
                    regex: true,
                    case_sensitive: !values.flag("ignore_case"),
                    ..Default::default()
                },
            )?;
            ws.execute_command(&crate::keybindings::Command {
                command: command.into(), args: serde_json::json!({"pattern":values.text("pattern"),"ignore_case":values.flag("ignore_case")})
            }, window, cx)
        });
        self.show_form(
            title,
            vec![Field::text("pattern", "Regex", ""), Field::toggle("ignore_case", "Ignore case", false)],
            done,
            window,
            cx,
        );
    }
}

impl Workspace {
    pub(crate) fn bound_adjust_values(
        &mut self,
        action: &editor::AdjustValues,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else { return };
        if !editor.read(cx).is_focused(window) {
            return;
        }
        if let Err(error) = self.adjust_values_admission(&editor, cx) {
            return self.status_message(error, cx);
        }
        // Native capture records once; JSON/macros retain their synchronous command route.
        editor.update(cx, |e, cx| e.adjust_values(action, window, cx));
    }

    pub(crate) fn adjust_values_admission(&self, editor: &Entity<Editor>, cx: &App) -> Result<(), String> {
        let document = &editor.read(cx).document;
        if self.open_editors(cx).iter().any(|other| {
            other != editor && {
                let other = other.read(cx);
                other.document == *document && !other.marked.is_empty()
            }
        }) {
            return Err("Finish or cancel composition in the other shared view before adjusting values".into());
        }
        Ok(())
    }
    forward_actions! {
        pub(crate) increment_values(IncrementValues, window, cx) => adjust_values_command(1, window, cx);
        pub(crate) decrement_values(DecrementValues, window, cx) => adjust_values_command(-1, window, cx);
    }
    fn adjust_values_command(&mut self, step: i128, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.execute_command(
            &crate::keybindings::Command {
                command: "editor::AdjustValues".into(),
                args: serde_json::json!({"step": step.to_string()}),
            },
            window,
            cx,
        ) {
            self.status_message(error, cx);
        }
    }
    pub(crate) fn adjust_values_prompt(&mut self, _: &AdjustValuesPrompt, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.active_editor() else { return };
        if let Err(error) = self.adjust_values_admission(&target, cx) {
            return self.status_message(error, cx);
        }
        let (buffer, version, selection, generation) = target.update(cx, |e, cx| {
            e.catch_up(cx);
            e.sel(cx).sel_checkpoint();
            let s = e.read_state(cx);
            (s.buffer_id(), s.doc.version, s.selection_state(), s.selection_generation())
        });
        let done: OnDone = Box::new(move |values, editor, ws, window, cx| {
            let step = values
                .text("step")
                .parse::<i128>()
                .map_err(|_| "Enter a signed 128-bit whole decimal step".to_string())?;
            ws.adjust_values_admission(&editor, cx)?;
            let current = editor.update(cx, |e, cx| {
                e.catch_up(cx);
                let s = e.read_state(cx);
                s.buffer_id() == buffer
                    && s.doc.version == version
                    && s.selection_state() == selection
                    && s.selection_generation() == generation
            });
            if !current {
                return Err("The document or selections changed; reopen Adjust Values".into());
            }
            // Preparation errors belong in the form, before the resolved recordable action.
            editor.read(cx).read_state(cx).adjust_value_edits(crate::editing::columns::ValueStep(step))?;
            ws.execute_command(
                &crate::keybindings::Command {
                    command: "editor::AdjustValues".into(),
                    args: serde_json::json!({"step": step.to_string()}),
                },
                window,
                cx,
            )
        });
        self.show_form(
            "Adjust Values",
            vec![Field::text("step", "Signed step (days for dates, minutes for times)", "1")],
            done,
            window,
            cx,
        );
    }
}

// Shared composition must be checked before borrowing the native action's editor.
macro_rules! bound_selection_set_actions {
    ($($name:ident($action:ty) => $handler:ident;)*) => {
        impl Workspace { $(
            pub(crate) fn $name(&mut self, action: &$action, window: &mut Window, cx: &mut Context<Self>) {
                let Some(editor) = self.active_editor() else { return };
                if !editor.read(cx).is_focused(window) { return; }
                if let Err(error) = self.selection_set_admission(&editor, cx) {
                    return self.status_message(error, cx);
                }
                editor.update(cx, |e, cx| e.$handler(action, window, cx));
            }
        )* }
    };
}
bound_selection_set_actions! {
    bound_move_to_fold(editor::MoveToFold) => move_to_fold;
    bound_select_fold(editor::SelectFold) => select_fold;
    bound_narrow_to_selection(editor::NarrowToSelection) => narrow_to_selection;
    bound_narrow_to_fold(editor::NarrowToFold) => narrow_to_fold;
    bound_show_matching_lines(editor::ShowMatchingLines) => show_matching_lines;
    bound_clear_filter(editor::ClearFilter) => clear_filter;
    bound_invert_selections(editor::InvertSelections) => invert_selections;
    bound_split_selections_by_regex(editor::SplitSelectionsByRegex) => split_selections_by_regex;
    bound_keep_selections_matching_regex(editor::KeepSelectionsMatchingRegex) => keep_selections_matching_regex;
    bound_remove_matching_selections(editor::RemoveMatchingSelections) => remove_matching_selections;
}

impl Workspace {
    pub(crate) fn alignment_admission(&self, editor: &Entity<Editor>, cx: &App) -> Result<(), String> {
        self.selection_set_admission(editor, cx)
    }

    pub(crate) fn bound_align_on_pattern(
        &mut self,
        action: &editor::AlignOnPattern,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else { return };
        if !editor.read(cx).is_focused(window) {
            return;
        }
        if let Err(error) = self.alignment_admission(&editor, cx) {
            return self.status_message(error, cx);
        }
        editor.update(cx, |e, cx| e.align_on_pattern(action, window, cx));
    }
    pub(crate) fn bound_align_selected_fields(
        &mut self,
        action: &editor::AlignSelectedFields,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else { return };
        if !editor.read(cx).is_focused(window) {
            return;
        }
        if let Err(error) = self.alignment_admission(&editor, cx) {
            return self.status_message(error, cx);
        }
        editor.update(cx, |e, cx| e.align_selected_fields(action, window, cx));
    }
    pub(crate) fn align_on_pattern_prompt(
        &mut self,
        _: &AlignOnPatternPrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.active_editor() else { return };
        if let Err(error) = self.alignment_admission(&target, cx) {
            return self.status_message(error, cx);
        }
        let (buffer, version, selection, generation) = target.update(cx, |e, cx| {
            e.catch_up(cx);
            e.sel(cx).sel_checkpoint();
            let s = e.read_state(cx);
            (s.buffer_id(), s.doc.version, s.selection_state(), s.selection_generation())
        });
        let done: OnDone = Box::new(move |values, editor, ws, window, cx| {
            let capture = values
                .text("capture")
                .parse::<usize>()
                .map_err(|_| "Capture index must be a nonnegative whole number".to_string())?;
            ws.alignment_admission(&editor, cx)?;
            let current = editor.update(cx, |e, cx| {
                e.catch_up(cx);
                let s = e.read_state(cx);
                s.buffer_id() == buffer
                    && s.doc.version == version
                    && s.selection_state() == selection
                    && s.selection_generation() == generation
            });
            if !current {
                return Err("The document or selections changed; reopen Align on Pattern".into());
            }
            editor.read(cx).read_state(cx).align_pattern_edits(
                values.text("pattern"),
                capture,
                values.flag("ignore_case"),
            )?;
            ws.execute_command(&crate::keybindings::Command {
                command: "editor::AlignOnPattern".into(), args: serde_json::json!({"pattern":values.text("pattern"),"capture":capture,"ignore_case":values.flag("ignore_case")})
            }, window, cx)
        });
        self.show_form(
            "Align on Pattern",
            vec![
                Field::text("pattern", "Regex", "="),
                Field::text("capture", "Capture index (0 = whole match)", "0"),
                Field::toggle("ignore_case", "Ignore case", false),
            ],
            done,
            window,
            cx,
        );
    }
}

impl Workspace {
    pub(crate) fn show_matching_lines_prompt(
        &mut self,
        _: &ShowMatchingLinesPrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.active_editor() else { return };
        if let Err(error) = self.selection_set_admission(&target, cx) {
            return self.status_message(error, cx);
        }
        let (buffer, version, selection, generation) = target.update(cx, |e, cx| {
            e.catch_up(cx);
            e.sel(cx).sel_checkpoint();
            let state = e.read_state(cx);
            (state.buffer_id(), state.doc.version, state.selection_state(), state.selection_generation())
        });
        let done: OnDone = Box::new(move |values, editor, ws, window, cx| {
            ws.selection_set_admission(&editor, cx)?;
            let context = values
                .text("context")
                .trim()
                .parse::<usize>()
                .map_err(|_| "Context lines must be a nonnegative whole number".to_string())?;
            let current = editor.update(cx, |e, cx| {
                e.catch_up(cx);
                let state = e.read_state(cx);
                state.buffer_id() == buffer
                    && state.doc.version == version
                    && state.selection_state() == selection
                    && state.selection_generation() == generation
            });
            if !current {
                return Err("The document or selections changed; reopen Show Matching Lines".into());
            }
            orlo_editor::search::pattern::Pattern::new(
                values.text("pattern"),
                &crate::editing::text::FindOpts {
                    regex: true,
                    case_sensitive: !values.flag("ignore_case"),
                    ..Default::default()
                },
            )?;
            ws.execute_command(&crate::keybindings::Command {
                command: "editor::ShowMatchingLines".into(),
                args: serde_json::json!({"pattern":values.text("pattern"),"context":context,"ignore_case":values.flag("ignore_case")}),
            }, window, cx)
        });
        self.show_form(
            "Show Matching Lines",
            vec![
                Field::text("pattern", "Regex", ""),
                Field::text("context", "Context lines", "0"),
                Field::toggle("ignore_case", "Ignore case", false),
            ],
            done,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{temp, text, type_in, window};
    use super::*;

    #[gpui::test]
    fn an_oversized_substitution_table_is_refused_before_it_is_read(cx: &mut gpui::TestAppContext) {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "a");
        let path = temp("orlo-table-oversized").join("table.tsv");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_TABLE_BYTES + 1).unwrap();
        ws.update_in(vcx, |ws, window, cx| ws.substitution_form(path, window, cx));
        vcx.run_until_parked();
        vcx.simulate_keystrokes("enter");
        let error = ws.read_with(vcx, |ws, _| match &ws.panel {
            Some(Panel::Form(p)) => p.error.clone(),
            _ => None,
        });
        assert!(error.is_some_and(|e| e.contains("16 MiB")));
        assert_eq!(text(&ws, vcx), "a");
    }
}
