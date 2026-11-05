//! User customizations: key bindings and command files, macros, colour scheme and appearance,
//! the Syntaxes and Themes folders, Guess Indentation. User commands and macros are dispatched
//! as the built-in actions are.

use super::*;
use crate::config::customization::{self, Command, Recorder};
use crate::keybindings::CommandAction;
use crate::languages::{registry, treesitter};

#[cfg(test)]
#[path = "language_errors_tests.rs"]
mod language_errors_tests;

actions!(
    workspace,
    [
        OpenKeybindings,
        ReloadCustomizations,
        ToggleMacroRecording,
        ReplayMacro,
        RepeatMacro,
        SaveMacro,
        LoadMacro,
        ChooseColorScheme,
        ChooseAppearance,
        OpenSyntaxFolder,
        OpenThemesFolder,
        GuessIndentation
    ]
);

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct UseMacro {
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct ChooseAsset {
    pub name: String,
    pub scheme: bool,
}

macro_rules! workspace_actions {
    ($operation:ident) => {
        $operation! {
            new_file,
            new_from_template,
            follow_log,
            toggle_viewer_mode,
            stop_following_log,
            reopen_with_encoding,
            save_with_encoding,
            change_file_encoding,
            browse_discarded_untitled,
            restore_discarded_untitled,
            preview_discarded_untitled,
            delete_discarded_untitled,
            restore_rescue_preview,
            delete_rescue_preview,
            clear_discard_history,
            browse_file_history,
            preview_file_history,
            restore_file_history,
            clear_file_history,
            format_json,
            minify_json,
            validate_json,
            show_terminal,
            sync_terminal_folder,
            find_terminal_output,
            open_terminal_file,
            new_terminal,
            new_terminal_with_profile,
            launch_terminal_profile,
            set_terminal_scrollback,
            rename_terminal,
            next_terminal,
            previous_terminal,
            select_terminal,
            select_terminal_tab,
            close_terminal_tab,
            send_selection_to_terminal,
            execute_selection_in_terminal,
            previous_terminal_command,
            next_terminal_command,
            copy_terminal_command_output,
            copy_last_terminal_output,
            terminal_live_output,
            close_terminal,
            show_git,
            git_init,
            git_init_at,
            git_refresh,
            git_select,
            git_open,
            git_stage,
            git_unstage,
            git_commit,
            git_amend,
            git_fetch,
            git_pull,
            git_push,
            git_remotes,
            git_remote_choice,
            git_worktrees,
            git_create_worktree,
            git_prune_worktrees,
            git_worktree_entry,
            git_worktree_open,
            git_worktree_remove,
            git_worktree_create_at,
            git_tags,
            git_create_tag,
            git_tag_create_at,
            git_tag_delete,
            git_stashes,
            git_create_stash,
            git_stash_entry,
            git_stash_action,
            git_switch_branch,
            git_create_branch,
            git_rename_branch,
            git_delete_branch,
            git_branch_menu,
            git_branch_choice,
            git_create_at,
            git_stage_lines,
            git_stage_hunk,
            git_unstage_hunk,
            git_file_history,
            git_history_page,
            git_show_commit,
            git_blame,
            git_use_branch,
            show_change,
            compare_files,
            compare_folders,
            next_folder_row,
            previous_folder_row,
            open_folder_row,
            focus_comparison_next,
            apply_left_to_right,
            apply_right_to_left,
            refresh_comparison,
            next_difference,
            previous_difference,
            next_change,
            previous_change,
            revert_change,
            next_conflict,
            accept_ours,
            accept_theirs,
            accept_both,
            guess_indentation,
            run_task,
            edit_tasks,
            edit_project_tasks,
            filter_through_shell,
            keep_selections_through_shell,
            run_shell_command,
            new_shell_worksheet,
            run_worksheet,
            developer_console,
            manage_packages,
            install_package,
            cancel_automation,
            run_package_command,
            run_configured_task,
            package_operation,
            open_keybindings,
            reload_customizations,
            toggle_macro_recording,
            replay_macro,
            repeat_macro,
            save_macro,
            load_macro,
            use_macro,
            choose_color_scheme,
            choose_appearance,
            choose_asset,
            open_syntax_folder,
            open_themes_folder,
            open,
            save,
            save_without_formatting,
            save_as,
            save_copy_as,
            save_all,
            close_tab,
            next_tab,
            prev_tab,
            last_tab,
            select_tab,
            toggle_sidebar,
            toggle_outline,
            switch_file,
            related_files,
            goto_anything,
            command_palette,
            goto_line,
            goto_bookmark,
            open_selected_path,
            open_selected_url,
            paste_from_history,
            clear_clipboard_history,
            hash_file,
            document_statistics,
            inspect_line_endings,
            next_line_ending,
            print,
            character_info,
            insert_unicode_character_prompt,
            insert_unicode_character,
            insert_literal_next_key,
            insert_literal_text,
            show_character_viewer,
            column_editor,
            pad_to_column,
            show_matching_lines_prompt,
            bound_move_to_fold,
            bound_select_fold,
            bound_narrow_to_selection,
            bound_narrow_to_fold,
            bound_show_matching_lines,
            bound_clear_filter,
            invert_selections_prompt,
            split_selections_by_regex_prompt,
            keep_selections_matching_regex_prompt,
            remove_matching_selections_prompt,
            bound_invert_selections,
            bound_split_selections_by_regex,
            bound_keep_selections_matching_regex,
            bound_remove_matching_selections,
            increment_values,
            decrement_values,
            adjust_values_prompt,
            bound_adjust_values,
            align_on_pattern_prompt,
            bound_align_on_pattern,
            bound_align_selected_fields,
            change_surround_prompt,
            delete_surround_prompt,
            sort_lines_by_pattern,
            process_duplicate_lines,
            process_lines_matching,
            affix_lines,
            apply_substitution_table,
            set_delimiter,
            new_pipeline,
            edit_pipeline,
            preview_pipeline,
            run_pipeline,
            run_pipeline_on_files,
            cancel_pipeline_run,
            use_pipeline,
            insert_snippet,
            insert_date_time_prompt,
            bound_insert_date_time,
            new_snippet,
            quick_panel_next,
            quick_panel_prev,
            quick_panel_confirm,
            quick_panel_dismiss,
            find,
            replace,
            find_next,
            find_prev,
            find_all,
            toggle_find_regex,
            toggle_find_selection,
            replace_in_files,
            refresh_results,
            apply_replacements,
            cancel_replacements,
            show_results,
            select_next_row,
            select_prev_row,
            toggle_row,
            open_row,
            collapse_row,
            expand_row,
            include_all,
            exclude_all,
            expand_all,
            collapse_all,
            focus_filter,
            focus_list,
            choose_where,
            where_add_paths,
            clear_search_history,
            find_in_sidebar_selection,
            open_playground,
            open_playground_with_document,
            playground_next,
            playground_prev,
            playground_focus_next,
            playground_to_find,
            playground_to_replace,
            playground_to_find_in_files,
            playground_preview,
            save_search,
            bookmark_matches,
            highlight_matches,
            extract,
            open_saved_search,
            rename_saved_search,
            delete_saved_search,
            use_saved_search,
            import_saved_searches,
            export_saved_searches,
            show_field_history,
            focus_prev_fif_field,
            toggle_show_context,
            toggle_use_buffer,
            toggle_use_gitignore,
            toggle_find_case,
            toggle_find_word,
            toggle_preserve_case,
            previous_find_query,
            next_find_query,
            replace_all,
            replace_one,
            use_selection_for_find,
            focus_next_field,
            find_in_files,
            run_find_in_files,
            cancel_find_in_files,
            next_result,
            prev_result,
            focus_next_fif_field,
            close_panel,
            submit_form,
            next_form_field,
            prev_form_field,
            quit,
            close_all,
            toggle_minimap,
            toggle_cursor_column_highlight,
            set_line_number_mode,
            toggle_white_space,
            toggle_word_wrap,
            set_wrap_column,
            toggle_tabs,
            toggle_status_bar,
            toggle_full_screen,
            increase_font_size,
            decrease_font_size,
            reset_font_size,
            set_ruler,
            set_tab_size,
            set_indent_style,
            set_syntax,
            set_line_ending,
            revert,
            reopen_closed_file,
            open_recent,
            clear_recent,
            session_problem,
            new_window,
            close_window,
            add_folder,
            remove_folders,
            remove_folder,
            remove_root,
            move_folder,
            open_project,
            switch_project,
            open_project_path,
            save_project_as,
            close_project,
            edit_project,
            save_workspace_as,
            refresh_folders,
            about,
            documentation,
            open_guide,
            trust_folder,
            trust_open_folders,
            minimize,
            zoom,
            bring_all_to_front,
            hide,
            hide_others,
            show_all,
            goto_symbol,
            goto_symbol_in_project,
            goto_definition,
            goto_definition_to_side,
            goto_reference,
            lsp_completion,
            lsp_references,
            lsp_type_definition,
            lsp_type_definition_to_side,
            lsp_implementation,
            lsp_implementation_to_side,
            lsp_hover,
            lsp_rename,
            lsp_format,
            lsp_format_selection,
            lsp_signature_help,
            lsp_code_actions,
            lsp_diagnostics,
            restart_language_servers,
            use_selection_for_replace,
            set_layout,
            new_view_into_file,
            open_settings,
            focus_group,
            move_to_group,
            focus_neighbour,
            move_to_neighbour,
            split_group,
            move_to_new_group,
            close_group,
            toggle_sync_scroll,
            save_layout,
            use_saved_layout,
            choose_saved_layout,
            keep_open,
            toggle_pin,
            close_other_tabs,
            close_tabs_to_right,
            add_tab_to_selection,
            focus_selected_tab,
            deselect_tabs,
            move_to_new_window,
            show_tab_list,
            close_listed,
            close_tab_target,
            toggle_open_files,
            new_file_here,
            new_folder_here,
            rename_path,
            move_paths,
            duplicate_path,
            trash_paths,
            reveal_in_finder,
            find_in_sidebar_item,
            focus_sidebar,
            explorer_back,
            explorer_forward,
            explorer_parent,
            path_breadcrumbs,
            sidebar_step,
            sidebar_fold,
            sidebar_open,
            sidebar_leave,
            toggle_panel,
            toggle_collapse_panel,
            move_panel,
            focus_panel,
            toggle_distraction_free,
        }
    };
}

pub(crate) fn bind_workspace_actions(d: gpui::Div, cx: &mut Context<Workspace>) -> gpui::Div {
    macro_rules! bind {
        ($($handler:ident,)*) => {{
            let mut d=d;
            $(d=d.on_action(cx.listener(Workspace::$handler));)*
            d
        }};
    }
    workspace_actions!(bind)
}

impl Workspace {
    pub(crate) fn validate_command(&self, command: &Command, cx: &App) -> Result<(), String> {
        let action = command.build(cx)?;
        if Editor::handles_action(action.as_ref()) {
            return self.active_editor().map(|_| ()).ok_or_else(|| "Open a document first".into());
        }
        fn handles<A: Action>(
            action: &dyn Action,
            _handler: fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>),
        ) -> bool {
            action.as_any().is::<A>()
        }
        macro_rules! available {
            ($($handler:ident,)*) => { $(
                if handles(action.as_ref(),Self::$handler) {return Ok(());}
            )* };
        }
        workspace_actions!(available);
        Err(format!("Command {} is not available in this workspace", command.command))
    }
    fn perform_workspace_action(&mut self, action: &dyn Action, window: &mut Window, cx: &mut Context<Self>) -> bool {
        fn invoke<A: Action>(
            ws: &mut Workspace,
            action: &dyn Action,
            handler: fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>),
            window: &mut Window,
            cx: &mut Context<Workspace>,
        ) -> bool {
            let a = action.as_any().downcast_ref::<A>();
            a.map(|a| handler(ws, a, window, cx)).is_some()
        }
        macro_rules! dispatch {
            ($($handler:ident,)*) => {$(
                if invoke(self,action,Self::$handler,window,cx) {return true;}
            )*};
        }
        workspace_actions!(dispatch);
        false
    }

    pub(crate) fn execute_command(
        &mut self,
        command: &Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let action = command.build(cx)?;
        if action.as_any().is::<editor::MoveToFold>()
            || action.as_any().is::<editor::SelectFold>()
            || action.as_any().is::<editor::NarrowToSelection>()
            || action.as_any().is::<editor::NarrowToFold>()
            || action.as_any().is::<editor::ShowMatchingLines>()
            || action.as_any().is::<editor::ClearFilter>()
            || action.as_any().is::<editor::InvertSelections>()
            || action.as_any().is::<editor::SplitSelectionsByRegex>()
            || action.as_any().is::<editor::KeepSelectionsMatchingRegex>()
            || action.as_any().is::<editor::RemoveMatchingSelections>()
        {
            if let Some(editor) = self.active_editor() {
                self.selection_set_admission(&editor, cx)?;
            }
        }
        if action.as_any().is::<editor::InsertDateTime>()
            || action.as_any().is::<editor::InsertUnicodeCharacter>()
            || action.as_any().is::<editor::InsertLiteralNextKey>()
            || action.as_any().is::<editor::InsertLiteralText>()
        {
            if let Some(editor) = self.active_editor() {
                self.selection_set_admission(&editor, cx)?;
            }
        }
        if action.as_any().is::<editor::AdjustValues>() {
            if let Some(editor) = self.active_editor() {
                self.adjust_values_admission(&editor, cx)?;
            }
        }
        if action.as_any().is::<editor::AlignOnPattern>() || action.as_any().is::<editor::AlignSelectedFields>() {
            if let Some(editor) = self.active_editor() {
                self.alignment_admission(&editor, cx)?;
            }
        }
        self.focus_editor(window, cx);
        if let Some(editor) = self.active_editor() {
            let peers = action.as_any().is::<editor::InsertLiteralNextKey>().then(|| self.open_editors(cx));
            let handled = editor.update(cx, |e, cx| {
                let handled = e.perform_action(action.as_ref(), window, cx);
                if let Some(peers) = peers {
                    e.set_literal_input_peers(peers, cx);
                }
                handled
            });
            if handled {
                if !action.as_any().is::<editor::InsertLiteralNextKey>() {
                    customization::record(command.clone(), cx);
                }
                return Ok(());
            }
        }
        if !self.perform_workspace_action(action.as_ref(), window, cx) {
            return Err(format!("Command {} is not available in this workspace", command.command));
        }
        Ok(())
    }

    pub(crate) fn open_keybindings(&mut self, _: &OpenKeybindings, window: &mut Window, cx: &mut Context<Self>) {
        self.open_or_template(customization::keymap_path(), "[\n\t// {\"keys\":[\"cmd-k\",\"cmd-i\"],\"command\":\"insert\",\"args\":{\"text\":\"Hello\"},\"context\":\"Editor && !single_line\"}\n]\n", window, cx);
    }

    pub(crate) fn reload_customizations(
        &mut self,
        _: &ReloadCustomizations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut errors: Vec<_> = customization::install(cx).err().into_iter().collect();
        let settings = cx.global::<Settings>().clone();
        errors.extend(super::appearance::load_customizations(&settings));
        self.invalidate_language_requests(cx);
        self.invalidate_navigation();
        self.language.invalidate_caches();
        for tab in self.open_editors(cx) {
            tab.update(cx, |e, cx| {
                e.close_completion(cx);
                e.signature_triggers = Default::default();
                e.invalidate_spelling();
                e.highlighter.reload();
                e.language_version = None;
                e.diagnostics.clear();
                e.semantic_spans.clear();
                e.lsp_status.clear();
                cx.notify();
            });
        }
        // Other windows drop syntax caches too, including background jobs from the old generation.
        // The current window is already borrowed by this action.
        let others = (cx.windows().into_iter()).filter(|w| *w != self.window).filter_map(|w| w.downcast::<Workspace>());
        for handle in others {
            handle
                .update(cx, |ws, _, cx| {
                    ws.index.update(cx, |i, cx| i.reparse_buffers(cx));
                    ws.apply_view(cx);
                    cx.notify();
                })
                .ok();
        }
        self.index.update(cx, |i, cx| i.reparse_buffers(cx));
        self.apply_view(cx);
        self.report_customizations(
            "Could not apply customizations",
            errors,
            Some("Customizations reloaded"),
            window,
            cx,
        );
        cx.notify();
    }

    /// After the language folders loaded: `errors` in one alert titled `title`, with the grammar
    /// failures reported since the last report and what compiling the user's grammars for this
    /// generation (on the highlight worker) reports, each once. With nothing to show, `done` in
    /// the status bar.
    pub fn report_customizations(
        &mut self,
        title: &'static str,
        errors: Vec<String>,
        done: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = registry::registry();
        let grammars: Vec<_> = (generation.languages().iter())
            .filter(|l| l.origin == registry::Origin::User)
            .filter_map(|l| treesitter::Grammar::of(&generation, l))
            .collect();
        if grammars.is_empty() {
            return self.alert_customizations(title, errors, &generation, done, window, cx);
        }
        // as a buffer's first parse would: kept for the generation, a failure reported once
        let compiled = highlight::on_worker(move || grammars.iter().for_each(|g| _ = g.compiled()));
        cx.spawn_in(window, async move |ws, cx| {
            compiled.await;
            ws.update_in(cx, |ws, window, cx| {
                // a later reload reports for its own generation
                if std::sync::Arc::ptr_eq(&generation, &registry::registry()) {
                    ws.alert_customizations(title, errors, &generation, done, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A grammar that failed while parsing (a trap) since the last report, named once without
    /// waiting for a reload: the window's poll asks.
    pub(crate) fn report_grammar_failures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let errors = grammar_failures(&registry::registry());
        if !errors.is_empty() {
            self.alert(window, cx, "A language's grammar failed", &errors.join("\n\n"));
        }
    }

    fn alert_customizations(
        &mut self,
        title: &str,
        mut errors: Vec<String>,
        generation: &registry::Registry,
        done: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for error in grammar_failures(generation) {
            if !errors.contains(&error) {
                errors.push(error);
            }
        }
        if !errors.is_empty() {
            self.alert(window, cx, title, &errors.join("\n\n"));
        } else if let Some(done) = done {
            self.status_message(done, cx);
        }
    }

    pub(crate) fn customization_saved(&mut self, editor: &Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = editor.read(cx).doc(cx).path.as_ref() else {
            return;
        };
        let support = session::support_dir();
        let reload = path == &customization::keymap_path()
            || (path.starts_with(&support)
                && (matches!(
                    path.extension().and_then(|s| s.to_str()),
                    Some("tmTheme" | "sublime-syntax" | "orlo-theme")
                ) || path.file_name().is_some_and(|n| n == crate::languages::manifest::FILE)));
        if reload {
            self.reload_customizations(&ReloadCustomizations, window, cx);
        }
    }

    pub(crate) fn toggle_macro_recording(&mut self, _: &ToggleMacroRecording, _: &mut Window, cx: &mut Context<Self>) {
        let r = cx.default_global::<Recorder>();
        if r.playing {
            return;
        }
        r.recording = !r.recording;
        let message = if r.recording {
            r.commands.clear();
            "Recording macro".to_string()
        } else {
            r.last = std::mem::take(&mut r.commands);
            format!("Recorded {} commands", r.last.len())
        };
        self.status_message(message, cx);
    }

    fn play_macro(
        &mut self,
        commands: Vec<Command>,
        times: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if times == 0 || times.checked_mul(commands.len()).is_none_or(|n| n > customization::MAX_STEPS) {
            return Err(format!("Playback is limited to 1–{} commands", customization::MAX_STEPS));
        }
        let text = serde_json::to_string(&commands).map_err(|e| e.to_string())?;
        customization::parse_macro(&text, cx)?;
        let r = cx.default_global::<Recorder>();
        if r.recording || r.playing {
            return Err("Stop recording or playback first".into());
        }
        if self.active_editor().is_none() {
            return Err("Open a document first".into());
        }
        r.playing = true;
        self.focus_editor(window, cx);
        let result = (0..times)
            .try_for_each(|_| (commands.iter()).try_for_each(|command| self.execute_command(command, window, cx)));
        cx.default_global::<Recorder>().playing = false;
        result
    }

    pub(crate) fn replay_macro(&mut self, _: &ReplayMacro, window: &mut Window, cx: &mut Context<Self>) {
        let commands = cx.default_global::<Recorder>().last.clone();
        if let Err(e) = self.play_macro(commands, 1, window, cx) {
            self.alert(window, cx, "Could not replay macro", &e);
        }
    }

    pub(crate) fn repeat_macro(&mut self, _: &RepeatMacro, window: &mut Window, cx: &mut Context<Self>) {
        self.show_input(
            "Repeat macro",
            "2",
            Box::new(|v, _, ws, window, cx| {
                let times = v.text("text").trim().parse::<usize>().map_err(|_| "Enter a positive whole number")?;
                let commands = cx.default_global::<Recorder>().last.clone();
                ws.play_macro(commands, times, window, cx)
            }),
            window,
            cx,
        );
    }

    pub(crate) fn save_macro(&mut self, _: &SaveMacro, window: &mut Window, cx: &mut Context<Self>) {
        self.show_input(
            "Save macro as",
            "",
            Box::new(|v, _, _, _, cx| {
                let path = customization::macro_path(v.text("text"))?;
                if path.exists() {
                    return Err(
                        "A macro with that name already exists; edit its JSON file or choose another name".into()
                    );
                }
                let r = cx.default_global::<Recorder>();
                if r.recording {
                    return Err("Stop recording before saving".into());
                }
                let text = serde_json::to_string_pretty(&r.last).map_err(|e| e.to_string())?;
                customization::parse_macro(&text, cx)?;
                std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
                crate::editing::text::write_atomic(&path, &(text + "\n")).map_err(|e| e.to_string())
            }),
            window,
            cx,
        );
    }

    pub(crate) fn load_macro(&mut self, _: &LoadMacro, window: &mut Window, cx: &mut Context<Self>) {
        let dir = session::support_dir().join("Macros");
        let mut items = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    items.push(quick_panel::Item::command(
                        path.file_stem().unwrap_or_default().to_string_lossy(),
                        "Saved macro",
                        UseMacro { path: path.clone() },
                    ));
                }
            }
        }
        items.sort_by_key(|i| i.label.to_lowercase());
        if items.is_empty() {
            return self.status_message("No saved macros", cx);
        }
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }

    pub(crate) fn use_macro(&mut self, a: &UseMacro, window: &mut Window, cx: &mut Context<Self>) {
        let loaded = std::fs::read_to_string(&a.path)
            .map_err(|e| e.to_string())
            .and_then(|text| customization::parse_macro(&text, cx));
        match loaded {
            Ok(commands) => {
                cx.default_global::<Recorder>().last = commands;
                self.replay_macro(&ReplayMacro, window, cx);
            }
            Err(e) => self.alert(window, cx, "Could not load macro", &e),
        }
    }

    forward_actions! {
    pub(crate) choose_color_scheme(ChooseColorScheme, window, cx) => choose_asset_list(true, window, cx);
    pub(crate) choose_appearance(ChooseAppearance, window, cx) => choose_asset_list(false, window, cx);
    pub(crate) open_syntax_folder(OpenSyntaxFolder, window, cx) => open_asset_folder("Syntaxes", window, cx);
    pub(crate) open_themes_folder(OpenThemesFolder, window, cx) => open_asset_folder("Themes", window, cx);
    }

    fn choose_asset_list(&mut self, scheme: bool, window: &mut Window, cx: &mut Context<Self>) {
        let support = session::support_dir();
        let names = if scheme { highlight::color_schemes(&support) } else { theme::appearance_names(&support) };
        let items = names
            .into_iter()
            .map(|name| quick_panel::Item::command(name.clone(), "", ChooseAsset { name, scheme }))
            .collect();
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }
    pub(crate) fn choose_asset(&mut self, a: &ChooseAsset, window: &mut Window, cx: &mut Context<Self>) {
        let settings = cx.global_mut::<Settings>();
        if a.scheme {
            settings.color_scheme = a.name.clone();
        } else {
            settings.theme = a.name.clone();
        }
        self.reload_customizations(&ReloadCustomizations, window, cx);
        self.status_message("Appearance applied; set color_scheme/theme in Settings to keep it", cx);
    }

    fn open_asset_folder(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        match super::appearance::customization_folder(kind) {
            Ok(path) => cx.reveal_path(&path),
            Err(e) => self.alert(window, cx, "Could not open customization folder", &e.to_string()),
        }
    }
}

/// The reported grammar failures about `registry`'s folders. One about a folder it doesn't load
/// waits, the newest 64 kept, for a registry that does (each test window has its own).
fn grammar_failures(registry: &registry::Registry) -> Vec<String> {
    static UNCLAIMED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut unclaimed = UNCLAIMED.lock().unwrap_or_else(|e| e.into_inner());
    unclaimed.extend(treesitter::take_failures());
    let ours = |e: &String| registry.languages().iter().any(|l| Path::new(e).starts_with(&l.folder));
    let (ours, mut others): (Vec<_>, Vec<_>) = std::mem::take(&mut *unclaimed).into_iter().partition(ours);
    others.drain(..others.len().saturating_sub(64));
    *unclaimed = others;
    ours
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{text, window};
    use super::*;
    use gpui::TestAppContext;

    fn set_syntax(ws: &mut Workspace, name: &str, window: &mut Window, cx: &mut Context<Workspace>) {
        ws.set_syntax(&SetSyntax { name: name.into() }, window, cx);
    }

    fn syntax(ws: &Workspace, cx: &App) -> String {
        let editor = ws.active_editor().unwrap();
        editor.read(cx).highlighter.syntax_name().into()
    }

    #[gpui::test]
    fn recording_replays_unicode_text_and_navigation_in_order(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        cx.simulate_keystrokes("ctrl-q");
        cx.simulate_input("a");
        cx.simulate_keystrokes("cmd-left");
        cx.simulate_input("é");
        cx.simulate_keystrokes("ctrl-q");
        assert_eq!(text(&ws, cx), "éa");
        ws.update_in(cx, |ws, window, cx| {
            let r = cx.default_global::<Recorder>();
            assert!(!r.recording);
            assert_eq!(r.last.len(), 3);
            let saved = serde_json::to_string(&r.last).unwrap();
            let loaded = customization::parse_macro(&saved, cx).unwrap();
            ws.new_file(&NewFile, window, cx);
            ws.play_macro(loaded, 1, window, cx).unwrap();
        });
        cx.run_until_parked();
        assert_eq!(text(&ws, cx), "éa");
        ws.update(cx, |_, cx| assert!(!cx.default_global::<Recorder>().playing));
    }

    #[gpui::test]
    fn recorded_input_replacements_replay_at_the_new_caret(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_macro_recording(&ToggleMacroRecording, window, cx);
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| {
                e.replace_text_in_range(None, "e", window, cx);
                e.replace_text_in_range(Some(0..1), "é", window, cx);
            });
            ws.toggle_macro_recording(&ToggleMacroRecording, window, cx);
            assert_eq!(editor.read(cx).text(cx), "é");
            ws.new_file(&NewFile, window, cx);
            ws.replay_macro(&ReplayMacro, window, cx);
        });
        cx.run_until_parked();
        assert_eq!(text(&ws, cx), "é");
    }

    #[gpui::test]
    fn reload_preserves_plain_text_syntax_chosen_for_a_detectable_document(cx: &mut TestAppContext) {
        for (name, contents) in [(Some("file.rs"), "fn main() {}\n"), (None, "#!/usr/bin/env python3\n")] {
            let (ws, cx) = window(cx);
            ws.update_in(cx, |ws, window, cx| {
                if let Some(name) = name {
                    let path = super::super::ui_tests::temp("orlo-reload-plain-text").join(name);
                    std::fs::write(&path, contents).unwrap();
                    ws.open_path(path, window, cx);
                } else {
                    ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(contents)));
                }
                set_syntax(ws, "Plain Text", window, cx);
                assert_eq!(syntax(ws, cx), "Plain Text");
                ws.reload_customizations(&ReloadCustomizations, window, cx);
                assert_eq!(syntax(ws, cx), "Plain Text");
            });
        }
    }

    #[gpui::test]
    fn command_api_preserves_workspace_then_editor_order(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        super::super::ui_tests::type_in(&ws, cx, "original");
        let original = ws.update_in(cx, |ws, window, cx| {
            let original = ws.active_editor().unwrap();
            ws.execute_command(
                &Command { command: "workspace::NewFile".into(), args: serde_json::json!({}) },
                window,
                cx,
            )
            .unwrap();
            ws.execute_command(
                &Command { command: "insert".into(), args: serde_json::json!({"text":"new"}) },
                window,
                cx,
            )
            .unwrap();
            original
        });
        cx.run_until_parked();
        assert_eq!(original.read_with(cx, |e, cx| e.text(cx)), "original");
        assert_eq!(text(&ws, cx), "new");
    }

    #[gpui::test]
    fn buffer_choices_survive_session_restore_and_settings_reload(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let path = super::super::ui_tests::temp("orlo-customization-session").join("file.rs");
        std::fs::write(&path, "fn main() {}\r\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(path, window, cx);
            ws.set_tab_size(&SetTabSize { size: 8 }, window, cx);
            ws.set_indent_style(&SetIndentStyle { to_spaces: false }, window, cx);
            ws.with_active(cx, |e, cx| e.update_doc(cx, |d| d.line_ending_override = true));
            let snapshot = ws.capture(window, false, &mut Vec::new(), cx);
            ws.restore(snapshot, window, cx);
            ws.apply_settings(r#"{"tab_size":2,"translate_tabs_to_spaces":true}"#, window, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert_eq!((e.doc(cx).tab_size, e.doc(cx).translate_tabs, e.doc(cx).line_ending), (8, false, "\r\n"));
            assert_eq!(e.doc(cx).indent_override, [true; 2]);
            assert!(e.doc(cx).line_ending_override);
        });
    }

    #[gpui::test]
    fn scoped_display_controls_start_from_the_active_view(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(
                r#"{"overrides":[{"language":"Rust","settings":{"font_size":24,"show_minimap":false}}]}"#,
                window,
                cx,
            );
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            set_syntax(ws, "Rust", window, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).view.font_size, 24.);
            ws.increase_font_size(&IncreaseFontSize, window, cx);
            ws.toggle_minimap(&ToggleMinimap, window, cx);
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert_eq!(e.view.font_size, 25.);
            assert!(e.view.minimap);
        });
    }

    #[gpui::test]
    fn rust_lifetime_typing_replays_without_quote_pairing(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| set_syntax(ws, "Rust", window, cx));
        cx.simulate_keystrokes("ctrl-q");
        cx.simulate_input("'");
        cx.simulate_input("a");
        cx.simulate_keystrokes("ctrl-q");
        ws.update_in(cx, |ws, window, cx| {
            ws.new_file(&NewFile, window, cx);
            set_syntax(ws, "Rust", window, cx);
            ws.replay_macro(&ReplayMacro, window, cx);
        });
        cx.run_until_parked();
        assert_eq!(text(&ws, cx), "'a");
    }

    #[gpui::test]
    fn repeated_macro_is_bounded_and_does_not_record_form_input(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_macro_recording(&ToggleMacroRecording, window, cx);
            ws.open_keybindings(&OpenKeybindings, window, cx);
            ws.command_palette(&CommandPalette, window, cx);
        });
        cx.simulate_input("ignored");
        cx.simulate_keystrokes("escape");
        ws.update_in(cx, |ws, window, cx| {
            ws.toggle_macro_recording(&ToggleMacroRecording, window, cx);
            assert!(cx.default_global::<Recorder>().last.is_empty());
            let commands = vec![Command { command: "insert".into(), args: serde_json::json!({"text":"x\n"}) }];
            assert!(ws.play_macro(commands.clone(), customization::MAX_STEPS + 1, window, cx).is_err());
            ws.new_file(&NewFile, window, cx);
            ws.play_macro(commands, 3, window, cx).unwrap();
        });
        cx.run_until_parked();
        assert_eq!(text(&ws, cx), "x\nx\nx\n");
    }

    #[gpui::test]
    fn a_broken_language_folder_is_named_in_the_reload_alert(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let languages = session::support_dir().join("Languages");
        for id in ["pony", "broken"] {
            std::fs::create_dir_all(languages.join(id)).unwrap();
        }
        std::fs::write(
            languages.join("pony/language.json"),
            r#"{"schema":1,"id":"pony","name":"Pony","scope":"source.pony","files":{"extensions":["pony"]}}"#,
        )
        .unwrap();
        let broken = languages.join("broken/language.json");
        std::fs::write(&broken, r#"{"schema":1,"id":"broken","nmae":"Broken"}"#).unwrap();
        ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
        let (title, detail) = cx.pending_prompt().expect("the reload alert");
        assert_eq!(title, "Could not apply customizations");
        assert!(detail.contains(&broken.display().to_string()), "{detail}");
        assert!(detail.contains("unknown field `nmae`"), "{detail}");
        assert!(!detail.contains("pony"), "{detail}");
        cx.simulate_prompt_answer("Ok");
        std::fs::remove_dir_all(&languages).unwrap();
    }

    #[gpui::test]
    fn reloading_the_keymap_keeps_the_terminal_and_git_keys(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
        cx.simulate_keystrokes("ctrl-shift-g");
        ws.read_with(cx, |ws, _| assert!(ws.git.sidebar_open, "Show Git"));
        cx.simulate_keystrokes("ctrl-`");
        ws.read_with(cx, |ws, _| assert!(matches!(ws.panel, Some(Panel::Terminal(_))), "Show Terminal"));
        // the terminal has the keys: ⌃G goes to the shell, not to Goto Line
        cx.simulate_keystrokes("ctrl-g");
        ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none(), "Goto Line opened"));
    }

    #[gpui::test]
    fn invalid_keymap_keeps_last_working_context_binding(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx,|ws,window,cx| {
            set_syntax(ws, "Rust", window, cx);
            std::fs::write(customization::keymap_path(),r#"[{"keys":["cmd-j"],"command":"insert","args":{"text":"works"},"context":"Editor && syntax == source-rust && !single_line"}]"#).unwrap();
            ws.reload_customizations(&ReloadCustomizations, window, cx);
        });
        cx.simulate_keystrokes("cmd-j");
        assert_eq!(text(&ws, cx), "works");
        ws.update(cx, |_, cx| {
            std::fs::write(customization::keymap_path(), "broken").unwrap();
            assert!(customization::install(cx).is_err());
        });
        cx.simulate_keystrokes("cmd-j");
        assert_eq!(text(&ws, cx), "worksworks");
        ws.update_in(cx, |ws, window, cx| set_syntax(ws, "Plain Text", window, cx));
        cx.simulate_keystrokes("cmd-j");
        assert_eq!(text(&ws, cx), "worksworks");
    }
}
