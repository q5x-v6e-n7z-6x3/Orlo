//! View toggles, font size, ruler, syntax, line endings and the window commands.

use super::*;
use crate::config::settings::{WhiteSpace, WordWrap};

/// How long the side bar slides.
pub const SIDEBAR_SLIDE: std::time::Duration = std::time::Duration::from_millis(150);

/// The side bar's drawn share `elapsed` into a slide from `from` to `to` (ease-out cubic).
pub fn slide_at(from: f32, to: f32, elapsed: std::time::Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / SIDEBAR_SLIDE.as_secs_f32()).min(1.);
    from + (to - from) * (1. - (1. - t).powi(3))
}

/// macOS Reduce Motion as last read (at launch and when a window comes to the front).
#[derive(Default)]
pub struct ReduceMotion(pub bool);

impl gpui::Global for ReduceMotion {}

/// Whether Orlo animates: the `animation_enabled` setting, and not Reduce Motion.
pub fn animations(cx: &App) -> bool {
    cx.global::<Settings>().animation_enabled && !cx.try_global::<ReduceMotion>().is_some_and(|r| r.0)
}

/// Read Reduce Motion again: true when it changed. Tests keep their own value.
pub fn refresh_reduce_motion(cx: &mut App) -> bool {
    #[cfg(not(test))]
    {
        let now = crate::app::macos::reduce_motion();
        let r = cx.default_global::<ReduceMotion>();
        std::mem::replace(&mut r.0, now) != now
    }
    #[cfg(test)]
    {
        let _ = cx;
        false
    }
}

impl Workspace {
    /// View > Side Bar > Show / Hide: slides unless animation is off.
    pub(crate) fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        let from = self.sidebar_shown_fraction();
        self.sidebar.visible = !self.sidebar.visible;
        let to = if self.sidebar.visible { 1. } else { 0. };
        let content = !self.roots.is_empty() || self.sidebar.show_open_files;
        self.sidebar_anim = (animations(cx) && content && from != to).then(|| (from, std::time::Instant::now()));
        self.refresh_menus(cx);
        cx.notify();
    }

    /// View > Side Bar > Show / Hide Open Files; shown, it brings the side bar with it.
    pub(crate) fn toggle_open_files(&mut self, _: &ToggleOpenFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.show_open_files ^= true;
        if self.sidebar.show_open_files {
            self.sidebar.visible = true;
            self.sidebar_anim = None;
        }
        self.refresh_menus(cx);
        autosave(cx);
        cx.notify();
    }

    /// How much of the side bar is drawn, 0 to 1: sliding, or as `visible` says.
    pub(crate) fn sidebar_shown_fraction(&self) -> f32 {
        let to = if self.sidebar.visible { 1. } else { 0. };
        match self.sidebar_anim {
            Some((from, t0)) => slide_at(from, to, t0.elapsed()),
            None => to,
        }
    }

    /// What the active view shows, or the window's settings with no view.
    pub(crate) fn shown_view<'a>(&'a self, cx: &'a App) -> &'a ViewSettings {
        (self.active_editor()).map_or(&self.view, |e| &e.read(cx).view)
    }

    pub(crate) fn toggle_minimap(&mut self, _: &ToggleMinimap, _: &mut Window, cx: &mut Context<Self>) {
        self.overrides.minimap = Some(!self.shown_view(cx).minimap);
        self.apply_view(cx);
    }

    pub(crate) fn toggle_cursor_column_highlight(
        &mut self,
        _: &ToggleCursorColumnHighlight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overrides.cursor_column_highlight = Some(!self.shown_view(cx).cursor_column_highlight);
        self.apply_view(cx);
    }

    pub(crate) fn set_line_number_mode(&mut self, a: &SetLineNumberMode, _: &mut Window, cx: &mut Context<Self>) {
        self.overrides.line_number_mode = Some(a.mode);
        self.apply_view(cx);
    }

    /// View > Show White Space: this window draws all white space, or what the settings say.
    pub(crate) fn toggle_white_space(&mut self, _: &ToggleWhiteSpace, _: &mut Window, cx: &mut Context<Self>) {
        // with the settings at "all" already, the toggle is between all and none
        let setting_is_all = cx.global::<Settings>().draw_white_space.all;
        self.overrides.white_space = match (self.shown_view(cx).white_space.all, setting_is_all) {
            (false, _) => Some(true),
            (true, false) => None,
            (true, true) => Some(false),
        };
        self.apply_view(cx);
    }

    /// View > Word Wrap: flips what the active view shows, in that view only.
    pub(crate) fn toggle_word_wrap(&mut self, _: &ToggleWordWrap, _: &mut Window, cx: &mut Context<Self>) {
        self.with_active(cx, |e, cx| {
            e.hold_top(cx);
            e.wrap_override = Some(!e.wraps());
            cx.notify();
        });
    }

    /// View > Word Wrap Column: the active view's wrap column (`None`: the settings').
    pub(crate) fn set_wrap_column(&mut self, a: &SetWrapColumn, _: &mut Window, cx: &mut Context<Self>) {
        let wraps = self.with_active(cx, |e, cx| {
            e.hold_top(cx);
            e.wrap_width_override = a.col;
            cx.notify();
            e.wraps()
        });
        if wraps == Some(false) {
            self.status_message("Word Wrap is off in this view: the column applies once it is on", cx);
        }
        autosave(cx);
    }

    forward_actions! {
    pub(crate) toggle_tabs(ToggleTabs, _, cx) => toggle_bar(|ws| &mut ws.show_tabs, cx);
    pub(crate) toggle_status_bar(ToggleStatusBar, _, cx) => toggle_bar(|ws| &mut ws.show_status_bar, cx);
    pub(crate) increase_font_size(IncreaseFontSize, _, cx) => step_font_size(1., cx);
    pub(crate) decrease_font_size(DecreaseFontSize, _, cx) => step_font_size(-1., cx);
    }

    fn toggle_bar(&mut self, shown: fn(&mut Self) -> &mut bool, cx: &mut Context<Self>) {
        *shown(self) ^= true;
        self.refresh_menus(cx);
        cx.notify();
    }

    pub(crate) fn toggle_full_screen(&mut self, _: &ToggleFullScreen, window: &mut Window, _: &mut Context<Self>) {
        window.toggle_fullscreen();
    }

    /// The settings file, then the project's settings, then this window's overrides.
    pub(crate) fn effective_view(&self, cx: &App) -> ViewSettings {
        let s = cx.global::<Settings>();
        let p = self.project_settings();
        let o = &self.overrides;
        let v = ViewSettings {
            font_face: s.font_face.clone().into(),
            font_size: o.font_size.unwrap_or(s.font_size),
            minimap: o.minimap.unwrap_or(s.show_minimap),
            rulers: (o.rulers.clone()).or(p.rulers).unwrap_or_else(|| s.rulers.clone()),
            word_wrap: p.word_wrap.unwrap_or(s.word_wrap),
            wrap_width: p.wrap_width.unwrap_or(s.wrap_width),
            animate: animations(cx),
            gutter: true,
            line_number_mode: o.line_number_mode.unwrap_or(s.line_number_mode),
            cursor_column_highlight: o.cursor_column_highlight.unwrap_or(s.cursor_column_highlight),
            centered: false,
            fold_buttons: s.fold_buttons,
            fade_fold_buttons: s.fade_fold_buttons,
            white_space: match o.white_space {
                Some(true) => WhiteSpace::ALL,
                Some(false) => WhiteSpace::NONE,
                None => s.draw_white_space,
            },
            unicode_warnings: s.unicode_warnings,
            selection_occurrences: s.selection_occurrences,
            inline_diagnostics: s.inline_diagnostics,
            inline_diagnostics_severity: s.inline_diagnostics_severity,
            unicode_warning_characters: s.unicode_warning_characters.clone(),
            unicode_warning_ranges: s.unicode_warning_ranges.clone(),
            drag_text: s.drag_text,
            auto_complete: s.auto_complete,
            auto_complete_min_chars: s.auto_complete_min_chars,
            spell_check: s.spell_check,
            spelling_selector: s.spelling_selector.clone(),
        };
        // distraction-free mode's own presentation; a view's own Word Wrap and Word
        // Wrap Column choices still win
        if self.distraction_free.is_none() {
            return v;
        }
        let d = &s.distraction_free;
        ViewSettings {
            word_wrap: if d.word_wrap { WordWrap::On } else { v.word_wrap },
            wrap_width: d.wrap_width,
            gutter: d.line_numbers,
            centered: d.draw_centered,
            ..v
        }
    }

    /// Push display settings to every tab, preserving each buffer's indentation choices.
    pub(crate) fn apply_view(&mut self, cx: &mut Context<Self>) {
        self.view = self.effective_view(cx);
        for t in self.views.tabs() {
            t.update(cx, |e, cx| {
                // the same text stays on top, whatever the new font size or wrap width
                e.hold_top(cx);
                self.apply_editor_preferences(e, cx);
                cx.notify();
            });
        }
        self.refresh_file_autosaves(cx);
        self.refresh_menus(cx);
        cx.notify();
    }

    fn step_font_size(&mut self, by: f32, cx: &mut Context<Self>) {
        self.overrides.font_size = Some((self.shown_view(cx).font_size + by).clamp(6., 72.));
        self.apply_view(cx);
    }

    pub(crate) fn reset_font_size(&mut self, _: &ResetFontSize, _: &mut Window, cx: &mut Context<Self>) {
        // back to the settings file's size
        self.overrides.font_size = None;
        self.apply_view(cx);
    }

    pub(crate) fn set_ruler(&mut self, a: &SetRuler, _: &mut Window, cx: &mut Context<Self>) {
        self.overrides.rulers = Some(a.col.into_iter().collect());
        self.apply_view(cx);
    }

    pub(crate) fn set_syntax(&mut self, a: &SetSyntax, _: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_language_requests(cx);
        self.with_active(cx, |e, cx| {
            e.invalidate_spelling();
            e.highlighter = Highlighter::by_name(&a.name);
            e.language_version = None;
            e.diagnostics.clear();
            e.semantic_spans.clear();
            e.lsp_status.clear();
            cx.notify();
        });
        self.apply_view(cx);
        if self.find_panel().is_some() {
            self.refresh_matches(cx);
        }
    }

    pub(crate) fn set_tab_size(&mut self, a: &SetTabSize, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        editor.update(cx, |e, cx| {
            e.hold_top(cx);
            e.update_doc(cx, |d| {
                d.tab_size = a.size.clamp(1, 16);
                d.indent_size = None;
                d.indent_override[0] = true;
            });
            cx.notify();
        });
        autosave(cx);
        self.refresh_menus(cx);
        cx.notify();
    }

    pub(crate) fn set_indent_style(&mut self, a: &SetIndentStyle, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        editor.update(cx, |e, cx| {
            e.update_doc(cx, |d| {
                d.translate_tabs = a.to_spaces;
                d.indent_override[1] = true;
            });
            cx.notify();
        });
        autosave(cx);
        self.refresh_menus(cx);
        cx.notify();
    }

    /// Explicit conversion changes existing boundaries through shared Undo history.
    pub(crate) fn set_line_ending(&mut self, a: &SetLineEnding, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let document = editor.read(cx).document.clone();
        if self.open_editors(cx).iter().any(|view| {
            let view = view.read(cx);
            view.document == document && !view.marked.is_empty()
        }) {
            self.status_message("Finish or cancel composition in every shared view before converting line endings", cx);
            return;
        }
        if !editor.read(cx).input_enabled || document.read(cx).model.edits_blocked() {
            return;
        }
        editor.update(cx, |e, cx| e.edit(cx, |s| s.convert_line_endings(a.ending)));
        autosave(cx);
        self.refresh_menus(cx);
        cx.notify();
    }

    pub(crate) fn about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        drop(window.prompt(
            PromptLevel::Info,
            "Orlo 0.1.0",
            Some("A Sublime Text look-alike built with gpui."),
            &["OK"],
            cx,
        ));
    }

    pub(crate) fn documentation(&mut self, _: &Documentation, _: &mut Window, cx: &mut Context<Self>) {
        cx.open_url("https://www.sublimetext.com/docs/");
    }

    pub(crate) fn open_guide(&mut self, a: &OpenGuide, _: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&guide_url(a.page));
    }

    /// Trust `a.path` (and what is in it): its repository's own Git programs, the language
    /// servers that build it and the tasks it defines may run.
    pub(crate) fn trust_folder(&mut self, a: &TrustFolder, _: &mut Window, cx: &mut Context<Self>) {
        match orlo_utils::trust::trust(&a.path) {
            Ok(()) => {
                self.status_message(format!("Trusted {}", tilde(&a.path)), cx);
                // Git resumes now; language servers start at their next poll
                self.refresh_git(cx);
            }
            Err(e) => self.status_message(format!("Could not trust {}: {e}", tilde(&a.path)), cx),
        }
    }

    /// Project > Trust Open Folders, once the user confirms.
    pub(crate) fn trust_open_folders(&mut self, _: &TrustOpenFolders, w: &mut Window, cx: &mut Context<Self>) {
        let roots = crate::config::project::present(&self.roots);
        if roots.is_empty() {
            return self.status_message("Open a folder to trust it", cx);
        }
        let listed: Vec<String> = roots.iter().map(|r| tilde(r)).collect();
        let detail = format!(
            "A trusted folder runs its own programs: Git settings in its repository, language servers that build it, and the tasks it defines. Trust only folders whose source you trust.\n\n{}",
            listed.join("\n")
        );
        let answer = w.prompt(PromptLevel::Warning, "Trust the open folders?", Some(&detail), &["Trust", "Cancel"], cx);
        cx.spawn_in(w, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update_in(cx, |ws, w, cx| {
                for path in roots {
                    ws.trust_folder(&TrustFolder { path }, w, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn minimize(&mut self, _: &Minimize, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }

    pub(crate) fn zoom(&mut self, _: &Zoom, window: &mut Window, _: &mut Context<Self>) {
        window.zoom_window();
    }

    pub(crate) fn bring_all_to_front(&mut self, _: &BringAllToFront, _: &mut Window, cx: &mut Context<Self>) {
        cx.activate(true);
    }

    pub(crate) fn hide(&mut self, _: &Hide, _: &mut Window, cx: &mut Context<Self>) {
        cx.hide();
    }

    pub(crate) fn hide_others(&mut self, _: &HideOthers, _: &mut Window, cx: &mut Context<Self>) {
        cx.hide_other_apps();
    }

    pub(crate) fn show_all(&mut self, _: &ShowAll, _: &mut Window, cx: &mut Context<Self>) {
        cx.unhide_other_apps();
    }

    // ---- groups ----

    /// After the active group or its views changed: focus, find outlines, menu captions.
    pub(crate) fn groups_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // the layout and where the views are, kept at once
        autosave(cx);
        self.cancel_quick_panel(cx);
        self.focus_editor(window, cx);
        self.refresh_matches(cx);
        self.refresh_menus(cx);
        cx.notify();
    }

    pub(crate) fn set_layout(&mut self, a: &SetLayout, window: &mut Window, cx: &mut Context<Self>) {
        self.views.set_layout(Layout::preset(a.preset));
        self.groups_changed(window, cx);
    }

    pub(crate) fn focus_group(&mut self, a: &FocusGroup, window: &mut Window, cx: &mut Context<Self>) {
        if a.group < self.views.groups.len() {
            self.views.focus(a.group);
            self.groups_changed(window, cx);
        }
    }

    pub(crate) fn move_to_group(&mut self, a: &MoveToGroup, window: &mut Window, cx: &mut Context<Self>) {
        if a.group < self.views.groups.len() {
            self.views.move_active_to(a.group);
            self.groups_changed(window, cx);
        }
    }

    pub(crate) fn focus_neighbour(&mut self, a: &FocusNeighbour, window: &mut Window, cx: &mut Context<Self>) {
        self.views.focus(self.views.neighbour(a.forward));
        self.groups_changed(window, cx);
    }

    pub(crate) fn move_to_neighbour(&mut self, a: &MoveToNeighbour, window: &mut Window, cx: &mut Context<Self>) {
        self.views.move_active_to(self.views.neighbour(a.forward));
        self.groups_changed(window, cx);
    }
}

/// Where Help's `OpenGuide { page }` goes: guides open on the main branch.
pub(crate) fn guide_url(page: &str) -> String {
    const REPO: &str = "https://github.com/q5x-v6e-n7z-6x3/Orlo";
    match page {
        "" => REPO.to_string(),
        guide if guide.ends_with(".md") => format!("{REPO}/blob/main/{guide}"),
        path => format!("{REPO}/{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{type_in, window};
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn a_slide_eases_out_and_ends_on_its_target() {
        assert_eq!(slide_at(1., 0., std::time::Duration::ZERO), 1.);
        assert_eq!(slide_at(1., 0., SIDEBAR_SLIDE), 0.);
        let mid = slide_at(0., 1., SIDEBAR_SLIDE / 2);
        assert!(0.5 < mid && mid < 1., "{mid}");
    }

    #[gpui::test]
    fn u07_cursor_columns_are_reachable_through_the_public_command(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            let result = ws.execute_command(
                &crate::keybindings::Command {
                    command: "workspace::ToggleCursorColumnHighlight".into(),
                    args: serde_json::json!({}),
                },
                window,
                cx,
            );
            assert!(result.is_ok(), "cursor column command must be registered: {result:?}");
            assert!(ws.shown_view(cx).cursor_column_highlight);
        });
    }

    #[gpui::test]
    fn u06_relative_numbers_are_reachable_through_the_public_command(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            let result = ws.execute_command(
                &crate::keybindings::Command {
                    command: "workspace::SetLineNumberMode".into(),
                    args: serde_json::json!({"mode":"relative"}),
                },
                window,
                cx,
            );
            assert!(result.is_ok(), "relative gutter command must be registered: {result:?}");
            assert_eq!(ws.shown_view(cx).line_number_mode, crate::config::settings::LineNumberMode::Relative);
            let invalid = ws.execute_command(
                &crate::keybindings::Command {
                    command: "workspace::SetLineNumberMode".into(),
                    args: serde_json::json!({"mode":"modal"}),
                },
                window,
                cx,
            );
            assert!(invalid.is_err());
            assert_eq!(ws.shown_view(cx).line_number_mode, crate::config::settings::LineNumberMode::Relative);
        });
    }

    #[gpui::test]
    fn u07_shared_views_menu_palette_and_settings_layering_keep_independent_columns(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "abcdef\nabcdef\n");
        let [first, second] = ws.update_in(cx, |ws, w, cx| {
            let first = ws.active_editor().unwrap();
            first.update(cx, |e, cx| e.select_range(2..2, cx));
            ws.set_layout(&SetLayout { preset: Preset::Columns(2) }, w, cx);
            ws.focus_group(&FocusGroup { group: 0 }, w, cx);
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            let second = ws.active_editor().unwrap();
            ws.move_to_group(&MoveToGroup { group: 1 }, w, cx);
            second.update(cx, |e, cx| e.select_range(11..11, cx));
            [first, second]
        });
        cx.run_until_parked();
        let before = [&first, &second]
            .map(|e| e.read_with(cx, |e, cx| (e.state.selections.clone(), e.doc(cx).version, e.drawn_frame())));
        let menu = ws.read_with(cx, |ws, cx| {
            ws.menus(cx)
                .into_iter()
                .find(|m| m.name == "View")
                .unwrap()
                .items
                .into_iter()
                .find_map(|i| match i {
                    MenuItem::Action { name, action, .. } if name == "Highlight Cursor Columns" => Some(action),
                    _ => None,
                })
                .unwrap()
        });
        cx.update(|w, cx| w.dispatch_action(menu, cx));
        cx.run_until_parked();
        for (i, e) in [&first, &second].into_iter().enumerate() {
            e.read_with(cx, |e, cx| {
                assert!(e.view.cursor_column_highlight);
                assert_eq!((e.state.selections.clone(), e.doc(cx).version, e.drawn_frame()), before[i]);
                let (_, left, space) = e.drawn_frame().unwrap();
                let expected = left + space * if i == 0 { 2. } else { 4. };
                assert!(!editor::cursor_column_bands(e).is_empty());
                assert!(editor::cursor_column_bands(e).iter().all(|b| f32::from(b.1.left() - expected).abs() < 0.1));
            });
        }
        first.read_with(cx, |e, cx| assert!(e.same_document(second.read(cx))));
        ws.read_with(cx, |ws, cx| {
            assert!(ws
                .menus(cx)
                .into_iter()
                .find(|m| m.name == "View")
                .unwrap()
                .items
                .iter()
                .any(|i| matches!(i, MenuItem::Action { name, .. } if name == "✓ Highlight Cursor Columns")))
        });
        let palette =
            quick_panel::command_items().into_iter().find(|i| i.label == "View: Highlight Cursor Columns").unwrap();
        let quick_panel::ItemAction::Dispatch(action) = palette.action else { panic!("palette toggle action") };
        cx.update(|w, cx| w.dispatch_action(action, cx));
        cx.run_until_parked();
        for e in [&first, &second] {
            e.read_with(cx, |e, _| assert!(editor::cursor_column_bands(e).is_empty()));
        }
        ws.update_in(cx, |ws, w, cx| {
            ws.apply_settings(
                r#"{"overrides":[{"language":"Rust","settings":{"cursor_column_highlight":true}}]}"#,
                w,
                cx,
            );
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            ws.set_syntax(&SetSyntax { name: "Rust".into() }, w, cx);
            assert!(ws.shown_view(cx).cursor_column_highlight);
            let path = super::super::ui_tests::temp("u07-project").join("example.sublime-project");
            let (project, warnings) =
                crate::config::project::Project::parse(&path, r#"{"settings":{"cursor_column_highlight":false}}"#)
                    .unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            ws.project = Some(project);
            ws.apply_view(cx);
            assert!(!ws.shown_view(cx).cursor_column_highlight);
            ws.toggle_cursor_column_highlight(&ToggleCursorColumnHighlight, w, cx);
            assert!(ws.shown_view(cx).cursor_column_highlight);
            ws.project = None;
            ws.apply_settings("{}", w, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(ws.overrides.cursor_column_highlight.is_none());
            assert!(!ws.shown_view(cx).cursor_column_highlight);
        });
        // Directly resolved view settings remain independent on the same shared source.
        first.update(cx, |e, cx| {
            e.view.cursor_column_highlight = true;
            cx.notify();
        });
        cx.run_until_parked();
        first.read_with(cx, |e, _| assert!(!editor::cursor_column_bands(e).is_empty()));
        second.read_with(cx, |e, _| assert!(editor::cursor_column_bands(e).is_empty()));
    }

    fn gutter_menu(ws: &Workspace, cx: &App) -> Vec<MenuItem> {
        let view = ws.menus(cx).into_iter().find(|m| m.name == "View").unwrap();
        view.items
            .into_iter()
            .find_map(|item| match item {
                MenuItem::Submenu(m) if m.name == "Line Numbers" => Some(m.items),
                _ => None,
            })
            .expect("View > Line Numbers")
    }

    #[gpui::test]
    fn u06_shared_views_render_independent_origins_and_menu_palette_switch_without_edits(cx: &mut TestAppContext) {
        use crate::config::settings::LineNumberMode::{Absolute, Relative};
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "a\nb\nc\nd\ne\n");
        let [first, second] = ws.update_in(cx, |ws, w, cx| {
            let first = ws.active_editor().unwrap();
            first.update(cx, |e, cx| e.select_range(4..4, cx));
            ws.set_layout(&SetLayout { preset: Preset::Columns(2) }, w, cx);
            ws.focus_group(&FocusGroup { group: 0 }, w, cx);
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            let second = ws.active_editor().unwrap();
            ws.move_to_group(&MoveToGroup { group: 1 }, w, cx);
            second.update(cx, |e, cx| e.select_range(8..8, cx));
            [first, second]
        });
        cx.run_until_parked();
        let before = [&first, &second].map(|e| {
            e.read_with(cx, |e, cx| (e.state.selections.clone(), e.doc(cx).version, e.scroll_rows().0, e.drawn_frame()))
        });
        let action = ws.read_with(cx, |ws, cx| {
            gutter_menu(ws, cx)
                .into_iter()
                .find_map(|item| match item {
                    MenuItem::Action { name, action, .. } if name == "Relative" => Some(action),
                    _ => None,
                })
                .unwrap()
        });
        cx.update(|w, cx| w.dispatch_action(action, cx));
        cx.run_until_parked();
        for (i, e) in [&first, &second].into_iter().enumerate() {
            e.read_with(cx, |e, cx| {
                assert_eq!(e.view.line_number_mode, Relative);
                assert_eq!(
                    (e.state.selections.clone(), e.doc(cx).version, e.scroll_rows().0, e.drawn_frame()),
                    before[i]
                );
                assert_eq!(e.text(cx), "a\nb\nc\nd\ne\n");
                let drawn = editor::gutter_labels(e);
                let labels: Vec<_> = drawn.iter().map(|(_, n)| n.as_ref()).collect();
                assert_eq!(
                    labels,
                    if i == 0 { vec!["2", "1", "3", "1", "2", "3"] } else { vec!["4", "3", "2", "1", "5", "1"] }
                );
            });
        }
        first.read_with(cx, |e, cx| assert!(e.same_document(second.read(cx))));
        ws.read_with(cx, |ws, cx| {
            assert!(gutter_menu(ws, cx).iter().any(|item| matches!(item,
            MenuItem::Action { name, .. } if name == "✓ Relative")))
        });
        // Dispatch the palette's actual action to restore absolute labels.
        let absolute =
            quick_panel::command_items().into_iter().find(|i| i.label == "View: Line Numbers: Absolute").unwrap();
        assert!(quick_panel::command_items().iter().any(|i| i.label == "View: Line Numbers: Relative"));
        let quick_panel::ItemAction::Dispatch(action) = absolute.action else { panic!("palette action") };
        cx.update(|w, cx| w.dispatch_action(action, cx));
        cx.run_until_parked();
        for e in [&first, &second] {
            e.read_with(cx, |e, _| {
                assert_eq!(e.view.line_number_mode, Absolute);
                let labels: Vec<_> = editor::gutter_labels(e).into_iter().map(|(r, n)| (r, n.to_string())).collect();
                assert_eq!(
                    labels,
                    [(0, "1"), (1, "2"), (2, "3"), (3, "4"), (4, "5"), (5, "6")].map(|(r, n)| (r, n.into()))
                );
            });
        }
        ws.update_in(cx, |ws, w, cx| {
            ws.with_active(cx, |e, cx| e.goto_line(4, cx));
            assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 6);
            ws.set_line_number_mode(&SetLineNumberMode { mode: Relative }, w, cx);
            ws.with_active(cx, |e, cx| e.goto_line(4, cx));
            assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 6);
        });
    }

    #[gpui::test]
    fn u06_file_project_window_settings_reload_and_distraction_visibility_keep_the_mode(cx: &mut TestAppContext) {
        use crate::config::settings::LineNumberMode::{Absolute, Relative};
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "a\nb\nc\n");
        ws.update_in(cx, |ws, w, cx| {
            ws.apply_settings(r#"{"overrides":[{"language":"Rust","settings":{"line_number_mode":"relative"}}]}"#, w, cx);
            ws.set_syntax(&SetSyntax { name: "Rust".into() }, w, cx);
            assert_eq!(ws.shown_view(cx).line_number_mode, Relative);
            let path = super::super::ui_tests::temp("u06-project").join("example.sublime-project");
            let (project, warnings) = crate::config::project::Project::parse(&path,
                r#"{"settings":{"line_number_mode":"absolute"}}"#).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            ws.project = Some(project);
            ws.apply_view(cx);
            assert_eq!(ws.shown_view(cx).line_number_mode, Absolute, "project follows global scoped values");
            let (project, warnings) = crate::config::project::Project::parse(&path,
                r#"{"settings":{"line_number_mode":"absolute","overrides":[{"language":"Rust","settings":{"line_number_mode":"relative"}}]}}"#).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            ws.project = Some(project);
            ws.apply_view(cx);
            assert_eq!(ws.shown_view(cx).line_number_mode, Relative);
            ws.set_line_number_mode(&SetLineNumberMode { mode: Absolute }, w, cx);
            assert_eq!(ws.shown_view(cx).line_number_mode, Absolute, "window choice wins");
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            // Applying Preferences publishes through the ordinary settings observer, clearing overrides.
            ws.project = None;
            ws.apply_settings(r#"{"line_number_mode":"relative","distraction_free":{"full_screen":false}}"#, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(ws.shown_view(cx).line_number_mode, Relative);
            assert!(ws.overrides.line_number_mode.is_none());
            ws.toggle_distraction_free(&ToggleDistractionFree, w, cx);
            assert!(!ws.shown_view(cx).gutter);
            assert_eq!(ws.shown_view(cx).line_number_mode, Relative);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| assert!(editor::gutter_labels(ws.active_editor().unwrap().read(cx)).is_empty()));
        ws.update_in(cx, |ws, w, cx| {
            cx.global_mut::<Settings>().distraction_free.line_numbers = true;
            ws.apply_view(cx);
            assert!(ws.shown_view(cx).gutter);
            ws.toggle_distraction_free(&ToggleDistractionFree, w, cx);
            assert!(ws.shown_view(cx).gutter);
            assert_eq!(ws.shown_view(cx).line_number_mode, Relative);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| assert!(!editor::gutter_labels(ws.active_editor().unwrap().read(cx)).is_empty()));
    }

    #[gpui::test]
    fn the_side_bar_slides_turns_back_mid_way_and_not_with_reduce_motion(cx: &mut TestAppContext) {
        let dir = crate::editing::text::resolve(&super::super::ui_tests::temp("orlo-sidebar-slide"));
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(dir.clone(), window, cx);
            assert_eq!(ws.sidebar_shown_fraction(), 1.);
            ws.toggle_sidebar(&ToggleSidebar, window, cx);
            assert!(!ws.sidebar.visible && ws.sidebar_anim.is_some());
            let (from, _) = ws.sidebar_anim.unwrap();
            assert_eq!(from, 1., "from fully shown");
            // pretend half the slide went by, then show it again: it turns back from there
            ws.sidebar_anim = Some((1., std::time::Instant::now() - SIDEBAR_SLIDE / 2));
            let mid = ws.sidebar_shown_fraction();
            assert!(0. < mid && mid < 0.5, "{mid}");
            ws.toggle_sidebar(&ToggleSidebar, window, cx);
            let (from, _) = ws.sidebar_anim.unwrap();
            assert!((from - mid).abs() < 0.05, "{from} vs {mid}");
            // Reduce Motion: at once
            cx.set_global(ReduceMotion(true));
            ws.toggle_sidebar(&ToggleSidebar, window, cx);
            assert!(ws.sidebar_anim.is_none());
            assert_eq!(ws.sidebar_shown_fraction(), 0.);
            assert!(!ws.effective_view(cx).animate, "page glides off too");
            cx.set_global(ReduceMotion(false));
            let mut s = cx.global::<Settings>().clone();
            s.animation_enabled = false;
            cx.set_global(s);
            ws.toggle_sidebar(&ToggleSidebar, window, cx);
            assert!(ws.sidebar_anim.is_none() && ws.sidebar_shown_fraction() == 1.);
        });
        // a slide's frames end: the next paint after it is done drops it
        ws.update_in(cx, |ws, window, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.animation_enabled = true;
            cx.set_global(s);
            ws.toggle_sidebar(&ToggleSidebar, window, cx);
            ws.sidebar_anim = Some((1., std::time::Instant::now() - SIDEBAR_SLIDE * 2));
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.sidebar_anim.is_none()));
    }
}
