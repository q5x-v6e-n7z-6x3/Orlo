//! The window's terminal tabs. Hiding the panel retains every independent PTY session.

pub mod emulator;

use crate::workspace::*;
use emulator::Terminal;

actions!(
    terminal,
    [
        ShowTerminal,
        SyncTerminalFolder,
        CloseTerminal,
        FindTerminalOutput,
        NewTerminal,
        NewTerminalWithProfile,
        SetTerminalScrollback,
        RenameTerminal,
        NextTerminal,
        PreviousTerminal,
        SendSelectionToTerminal,
        ExecuteSelectionInTerminal,
        PreviousTerminalCommand,
        NextTerminalCommand,
        CopyTerminalCommandOutput,
        CopyLastTerminalOutput,
        TerminalLiveOutput
    ]
);

parameter_actions! { terminal, plain;
    LaunchTerminalProfile { name: String, profile: crate::config::settings::TerminalProfile, }
}

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = terminal, no_json)]
pub struct SelectTerminal {
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = terminal, no_json)]
pub struct SelectTerminalTab {
    pub id: gpui::EntityId,
}

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = terminal, no_json)]
pub struct CloseTerminalTab {
    pub id: gpui::EntityId,
}

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = terminal, no_json)]
pub struct OpenTerminalFile {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

impl Workspace {
    pub(crate) fn previous_terminal_command(
        &mut self,
        _: &PreviousTerminalCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(t) = &self.terminal {
            t.update(cx, |t, cx| {
                t.review_command(false, cx);
                t.focus(window);
            });
        }
    }
    pub(crate) fn next_terminal_command(
        &mut self,
        _: &NextTerminalCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(t) = &self.terminal {
            t.update(cx, |t, cx| {
                t.review_command(true, cx);
                t.focus(window);
            });
        }
    }
    pub(crate) fn copy_terminal_command_output(
        &mut self,
        _: &CopyTerminalCommandOutput,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(t) = &self.terminal {
            t.update(cx, |t, cx| t.copy_command_output(cx));
        }
    }
    pub(crate) fn copy_last_terminal_output(
        &mut self,
        _: &CopyLastTerminalOutput,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(t) = &self.terminal {
            t.update(cx, |t, cx| t.copy_last_output(cx));
        }
    }
    pub(crate) fn terminal_live_output(&mut self, _: &TerminalLiveOutput, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = &self.terminal {
            t.update(cx, |t, cx| {
                t.return_to_live(cx);
                t.focus(window);
            });
        }
    }

    pub(crate) fn sync_terminal_folder(&mut self, _: &SyncTerminalFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal.clone() else {
            return self.status_message("Open a terminal before syncing its folder", cx);
        };
        let directory = self
            .active_editor()
            .and_then(|editor| editor.read(cx).doc(cx).path.as_ref()?.parent().map(Path::to_path_buf))
            .or_else(|| self.roots.first().map(|root| root.path.clone()));
        let Some(directory) = directory else {
            return self.status_message("Open a file or project folder to sync the terminal", cx);
        };
        if let Err(error) = terminal.update(cx, |terminal, cx| terminal.sync_folder(&directory, cx)) {
            self.status_message(error, cx);
        }
        cx.notify();
    }

    pub(crate) fn send_selection_to_terminal(
        &mut self,
        _: &SendSelectionToTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_editor_selection(false, window, cx);
    }
    pub(crate) fn execute_selection_in_terminal(
        &mut self,
        _: &ExecuteSelectionInTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_editor_selection(true, window, cx);
    }
    fn send_editor_selection(&mut self, execute: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let e = editor.read(cx);
        if self.open_editors(cx).iter().chain(std::iter::once(&editor)).any(|other| {
            let other = other.read(cx);
            other.document == e.document && !other.marked.is_empty()
        }) {
            return self.status_message("Finish or cancel text composition before sending to the terminal", cx);
        }
        if e.state.revision() != e.doc(cx).version {
            return self.status_message("A shared edit is updating this view; try sending again", cx);
        }
        let mut ranges: Vec<_> =
            e.state.selections.iter().map(|s| s.range()).filter(|range| !range.is_empty()).collect();
        ranges.sort_by_key(|range| (range.start, range.end));
        let text =
            ranges.into_iter().map(|range| e.doc(cx).rope.slice(range).to_string()).collect::<Vec<_>>().join("\n");
        if text.is_empty() {
            return self.status_message("Select editor text to send to the terminal", cx);
        }
        let terminal = self.terminal.clone().unwrap_or_else(|| self.make_terminal(cx));
        self.show_panel(Panel::Terminal(terminal.clone()), cx);
        terminal.read(cx).focus(window);
        if let Err(error) = terminal.update(cx, |t, cx| t.send_selection(&text, execute, cx)) {
            self.status_message(error, cx);
        }
    }

    pub(crate) fn open_terminal_file(
        &mut self,
        action: &OpenTerminalFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !action.path.is_file() {
            return self.status_message(format!("Terminal file not found: {}", action.path.display()), cx);
        }
        let path = crate::editing::text::resolve(&action.path);
        self.pending_jumps.insert(
            path.clone(),
            PendingJump {
                line: action.line.saturating_sub(1),
                hit: Some((action.column.saturating_sub(1), None)),
                selection: None,
                browser: None,
            },
        );
        self.open_path_here(path.clone(), window, cx);
        if self.editor_for_path(&path, cx).is_some() {
            self.finish_pending_jump(&path, cx);
        }
    }
    pub(crate) fn find_terminal_output(&mut self, _: &FindTerminalOutput, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel_hidden || self.panel_collapsed || !matches!(self.panel, Some(Panel::Terminal(_))) {
            self.show_terminal(&ShowTerminal, window, cx);
        }
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |t, cx| t.open_search(&emulator::FindOutput, window, cx));
        }
    }

    pub(crate) fn show_terminal(&mut self, _: &ShowTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.git.selection_active = false;
        if !self.panel_hidden && !self.panel_collapsed && matches!(self.panel, Some(Panel::Terminal(_))) {
            return self.close_panel(&ClosePanel, window, cx);
        }
        self.panel_places.terminal.dock = session::Dock::Bottom;
        let terminal = self.terminal.clone().unwrap_or_else(|| self.make_terminal(cx));
        self.show_panel(Panel::Terminal(terminal.clone()), cx);
        terminal.read(cx).focus(window);
    }

    fn make_terminal(&mut self, cx: &mut Context<Self>) -> Entity<Terminal> {
        self.make_terminal_profile(None, cx)
    }

    fn make_terminal_profile(
        &mut self,
        profile: Option<crate::config::settings::TerminalProfile>,
        cx: &mut Context<Self>,
    ) -> Entity<Terminal> {
        let cwd = self
            .terminal
            .as_ref()
            .map(|t| t.read(cx).cwd.clone())
            .filter(|p| p.is_dir())
            .or_else(|| {
                self.active_editor()
                    .and_then(|e| Some(e.read(cx).doc(cx).path.as_ref()?.parent()?.to_path_buf()))
                    .filter(|p| p.is_dir())
            })
            .or_else(|| self.roots.iter().find(|r| r.path.is_dir()).map(|r| r.path.clone()))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_dir()))
            .unwrap_or_else(|| PathBuf::from("/"));
        let terminal = cx.new(|cx| match profile {
            Some(profile) => Terminal::with_profile(cwd, Some(&profile), cx),
            None => Terminal::new(cwd, cx),
        });
        cx.subscribe(&terminal, |_, _, _: &emulator::MetadataChanged, cx| cx.notify()).detach();
        self.terminal_views.add(terminal.clone());
        self.terminal = Some(terminal.clone());
        terminal
    }

    pub(crate) fn new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let terminal = self.make_terminal(cx);
        self.panel_places.terminal.dock = session::Dock::Bottom;
        self.show_panel(Panel::Terminal(terminal.clone()), cx);
        terminal.read(cx).focus(window);
    }

    pub(crate) fn new_terminal_with_profile(
        &mut self,
        _: &NewTerminalWithProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profiles = &cx.global::<Settings>().terminal.profiles;
        if profiles.is_empty() {
            return self.status_message("Add named profiles in Preferences under terminal.profiles", cx);
        }
        let items = profiles
            .iter()
            .map(|(name, profile)| {
                quick_panel::Item::command(
                    name,
                    &profile.shell,
                    LaunchTerminalProfile { name: name.clone(), profile: profile.clone() },
                )
            })
            .collect();
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }

    pub(crate) fn launch_terminal_profile(
        &mut self,
        action: &LaunchTerminalProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.global::<Settings>().terminal.profiles.get(&action.name) != Some(&action.profile) {
            return self.status_message("This terminal profile changed; choose it again", cx);
        }
        let terminal = self.make_terminal_profile(Some(action.profile.clone()), cx);
        self.panel_places.terminal.dock = session::Dock::Bottom;
        self.show_panel(Panel::Terminal(terminal.clone()), cx);
        terminal.read(cx).focus(window);
    }

    pub(crate) fn set_terminal_scrollback(
        &mut self,
        _: &SetTerminalScrollback,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = &self.terminal else {
            return self.status_message("Open a terminal to set its scrollback", cx);
        };
        let initial = terminal.read(cx).scrollback_limit().to_string();
        let target = terminal.downgrade();
        let done: OnDone = Box::new(move |values, _, ws, window, cx| {
            let terminal =
                target.upgrade().filter(|t| ws.terminal_views.find(t).is_some()).ok_or("This terminal has closed")?;
            let rows = values
                .text("rows")
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|&n| n <= 100_000)
                .ok_or("Use a whole number from 0 to 100000")?;
            terminal.update(cx, |t, cx| t.set_scrollback_limit(rows, cx));
            if let Some(active) = ws.terminal.clone() {
                ws.show_panel(Panel::Terminal(active.clone()), cx);
                active.read(cx).focus(window);
            }
            Ok(())
        });
        self.show_window_form(
            "Set Terminal Scrollback (oldest output is discarded)",
            vec![Field::text("rows", "Rows (0–100000)", initial)],
            done,
            window,
            cx,
        );
    }

    pub(crate) fn rename_terminal(&mut self, _: &RenameTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = &self.terminal else { return };
        let initial = terminal.read(cx).name.clone().unwrap_or_default();
        let target = terminal.downgrade();
        let done: OnDone = Box::new(move |values, _, ws, window, cx| {
            let terminal =
                target.upgrade().filter(|t| ws.terminal_views.find(t).is_some()).ok_or("This terminal has closed")?;
            let text = values.text("name");
            if text.chars().any(char::is_control) {
                return Err("A terminal name cannot contain control characters".into());
            }
            let name = text.trim();
            if name.chars().count() > 128 {
                return Err("Use at most 128 characters for a terminal name".into());
            }
            terminal.update(cx, |t, cx| {
                t.name = (!name.is_empty()).then(|| name.to_owned());
                cx.notify();
            });
            if let Some(active) = ws.terminal.clone() {
                ws.show_panel(Panel::Terminal(active.clone()), cx);
                active.read(cx).focus(window);
            }
            Ok(())
        });
        self.show_window_form(
            "Rename Terminal (empty restores the directory label)",
            vec![Field::text("name", "Name", initial)],
            done,
            window,
            cx,
        );
    }

    pub(crate) fn select_terminal(&mut self, action: &SelectTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal_views.select(0, action.index);
        self.terminal = self.terminal_views.active_tab().cloned();
        if let Some(terminal) = &self.terminal {
            self.show_panel(Panel::Terminal(terminal.clone()), cx);
            self.terminal.as_ref().unwrap().read(cx).focus(window);
        }
    }

    pub(crate) fn select_terminal_tab(
        &mut self,
        action: &SelectTerminalTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.terminal_views.groups[0].tabs.iter().position(|t| t.entity_id() == action.id) {
            self.select_terminal(&SelectTerminal { index }, window, cx);
        }
    }

    pub(crate) fn next_terminal(&mut self, _: &NextTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.step_terminal(true, window, cx);
    }
    pub(crate) fn previous_terminal(&mut self, _: &PreviousTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.step_terminal(false, window, cx);
    }
    fn step_terminal(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let group = &self.terminal_views.groups[0];
        let count = group.tabs.len();
        if count > 0 {
            let index = (group.active + if forward { 1 } else { count - 1 }) % count;
            self.select_terminal(&SelectTerminal { index }, window, cx);
        }
    }

    pub(crate) fn close_terminal(&mut self, _: &CloseTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            self.close_terminal_tab(&CloseTerminalTab { id: terminal.entity_id() }, window, cx);
        }
    }

    pub(crate) fn close_terminal_tab(
        &mut self,
        action: &CloseTerminalTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminal_views.tabs().find(|t| t.entity_id() == action.id).cloned() else { return };
        let active = self.terminal.as_ref() == Some(&terminal);
        terminal.update(cx, |t, _| t.close());
        self.terminal_views.remove(&terminal);
        self.terminal = self.terminal_views.active_tab().cloned();
        if active && matches!(&self.panel, Some(Panel::Terminal(_))) {
            self.cancel_comparison();
            if let Some(terminal) = &self.terminal {
                self.panel = Some(Panel::Terminal(terminal.clone()));
                terminal.read(cx).focus(window);
            } else {
                self.panel = None;
                self.focus_editor(window, cx);
            }
        }
        self.refresh_menus(cx);
        cx.notify();
    }

    pub(crate) fn close_terminals(&mut self, cx: &mut Context<Self>) {
        for terminal in self.terminal_views.tabs() {
            terminal.update(cx, |t, _| t.close());
        }
        self.terminal = None;
        self.terminal_views = Groups::new();
        if matches!(self.panel, Some(Panel::Terminal(_))) {
            self.panel = None;
        }
    }

    pub(crate) fn render_terminal_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(terminal) = &self.terminal else {
            return div().into_any_element();
        };
        let cwd = search::tilde(&terminal.read(cx).cwd);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(ui(theme::BORDER))
            .child(
                kit::panel_header()
                    .child(icon(Icon::Terminal, 14.).text_color(ui(theme::ICON_MUTED)))
                    .child(
                        div()
                            .id("terminal-title")
                            .min_w_0()
                            .max_w(px(240.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .tooltip(kit::tooltip(
                                terminal.read(cx).name.clone().unwrap_or_else(|| "Terminal".into()).into(),
                            ))
                            .child(terminal.read(cx).name.clone().unwrap_or_else(|| "Terminal".into())),
                    )
                    .child(
                        kit::ghost_button("new-terminal", "+").tooltip(kit::tooltip("New Terminal".into())).on_click(
                            cx.listener(|ws, _: &ClickEvent, window, cx| ws.new_terminal(&NewTerminal, window, cx)),
                        ),
                    )
                    .child(
                        kit::ghost_button("rename-terminal", "Rename…")
                            .debug_selector(|| "rename-terminal".into())
                            .on_click(cx.listener(|ws, _: &ClickEvent, window, cx| {
                                ws.rename_terminal(&RenameTerminal, window, cx)
                            })),
                    )
                    .child(
                        kit::ghost_button("terminal-profiles", "Profiles…")
                            .debug_selector(|| "terminal-profiles".into())
                            .on_click(cx.listener(|ws, _: &ClickEvent, window, cx| {
                                ws.new_terminal_with_profile(&NewTerminalWithProfile, window, cx)
                            })),
                    )
                    .child(
                        kit::ghost_button("sync-terminal-folder", "Sync Folder")
                            .debug_selector(|| "sync-terminal-folder".into())
                            .tooltip(kit::tooltip("Sync Terminal Folder (requires a fresh empty shell prompt)".into()))
                            .opacity(if terminal.read(cx).can_sync_folder() { 1. } else { 0.4 })
                            .when(terminal.read(cx).can_sync_folder(), |button| {
                                button.on_click(cx.listener(|ws, _: &ClickEvent, window, cx| {
                                    ws.sync_terminal_folder(&SyncTerminalFolder, window, cx)
                                }))
                            }),
                    )
                    .child(kit::icon_button("terminal-find", Icon::Search, "Find Terminal Output  ⌘F").on_click(
                        cx.listener(|ws, _: &ClickEvent, window, cx| {
                            if let Some(terminal) = &ws.terminal {
                                terminal.update(cx, |t, cx| t.open_search(&emulator::FindOutput, window, cx));
                            }
                        }),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(11.))
                            .text_color(ui(theme::TEXT_MUTED))
                            .child(cwd),
                    )
                    .child(kit::icon_button("hide-terminal", Icon::ChevronDown, "Hide Terminal").on_click(
                        cx.listener(|ws, _: &ClickEvent, window, cx| ws.close_panel(&ClosePanel, window, cx)),
                    ))
                    .child(kit::icon_button("close-terminal", Icon::Trash, "Close Terminal").on_click(
                        cx.listener(|ws, _: &ClickEvent, window, cx| ws.close_terminal(&CloseTerminal, window, cx)),
                    )),
            )
            .when(self.terminal_views.groups[0].tabs.len() > 1, |d| {
                d.child(kit::panel_header().overflow_hidden().children(
                    self.terminal_views.groups[0].tabs.iter().enumerate().map(|(i, t)| {
                        let id = t.entity_id();
                        let label = format!(
                            "{} {}",
                            i + 1,
                            t.read(cx).name.clone().unwrap_or_else(|| search::tilde(&t.read(cx).cwd))
                        );
                        div()
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .max_w(px(260.))
                            .when(self.terminal_views.groups[0].active == i, |d| d.bg(ui(theme::BUTTON_SELECTED_BG)))
                            .child(
                                kit::ghost_button(("terminal-tab", i), label.clone())
                                    .min_w_0()
                                    .max_w(px(238.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .debug_selector(move || format!("terminal-tab-{i}"))
                                    .tooltip(kit::tooltip(label.into()))
                                    .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        ws.select_terminal_tab(&SelectTerminalTab { id }, window, cx)
                                    })),
                            )
                            .child(
                                kit::icon_button(("terminal-tab-close", i), Icon::Close, "Close Terminal")
                                    .debug_selector(move || format!("terminal-tab-close-{i}"))
                                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                                    .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        ws.close_terminal_tab(&CloseTerminalTab { id }, window, cx)
                                    })),
                            )
                    }),
                ))
            })
            // the shell's text is inset from the panel's edges, as an editor's from its gutter
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pl(px(10.))
                    .pr(px(4.))
                    .pt(px(4.))
                    .bg(c(theme::EDITOR_BG))
                    .child(terminal.clone()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{temp, window};

    #[gpui::test]
    fn rename_terminal_is_reachable_without_an_editor_and_keeps_the_session_identity(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = window(cx);
        let (original, action) = ws.update_in(cx, |ws, window, cx| {
            let settings = &mut cx.global_mut::<Settings>().terminal;
            settings.shell = Some("/bin/sh".into());
            settings.arguments = Some(vec!["-c".into(), "read value".into()]);
            let editors: Vec<_> = ws.views.tabs().cloned().collect();
            for editor in editors {
                ws.remove_view(&editor, cx);
            }
            ws.show_terminal(&ShowTerminal, window, cx);
            let original = ws.terminal.as_ref().unwrap().downgrade();
            let action = cx.build_action("terminal::RenameTerminal", None).expect("Rename Terminal is custom-bindable");
            let menu_action = ws
                .menus(cx)
                .into_iter()
                .find(|m| m.name == "Terminal")
                .unwrap()
                .items
                .into_iter()
                .find_map(|item| match item {
                    gpui::MenuItem::Action { name, action, .. } if name == "Rename Terminal…" => Some(action),
                    _ => None,
                })
                .expect("Rename Terminal is in the Terminal menu");
            assert_eq!(action.name(), menu_action.name());
            (original, menu_action)
        });
        cx.run_until_parked();
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            let before = ws.menus_built;
            assert!(ws.set_menus_now(cx), "publish the actual active-window menu");
            assert!(ws.menus_built > before);
        });
        cx.update(|window, cx| window.dispatch_action(action, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(matches!(ws.panel, Some(Panel::Form(_)))));
        cx.simulate_input("  build café 中  ");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(ws.terminal.as_ref().unwrap().entity_id(), original.entity_id());
            assert!(matches!(ws.panel, Some(Panel::Terminal(_))), "successful rename returns to its terminal");
            ws.command_palette(&CommandPalette, window, cx);
            let palette = ws.quick_panel.as_mut().unwrap();
            palette.selected = palette
                .filtered
                .iter()
                .position(|&i| palette.items[i].label == "Terminal: Rename…")
                .expect("palette rename route");
            ws.confirm_quick_panel(window, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let Some(Panel::Form(form)) = &ws.panel else { panic!("rename form") };
            assert_eq!(form.inputs().next().unwrap().read(cx).text(cx), "build café 中");
        });
    }

    #[gpui::test]
    fn terminal_tabs_keep_neighbours_alive_and_close_only_the_selected_session(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = window(cx);
        let (first, second) = ws.update_in(cx, |ws, window, cx| {
            ws.show_terminal(&ShowTerminal, window, cx);
            let first = ws.terminal.as_ref().unwrap().downgrade();
            ws.new_terminal(&NewTerminal, window, cx);
            let second = ws.terminal.as_ref().unwrap().downgrade();
            assert_ne!(first.entity_id(), second.entity_id());
            ws.previous_terminal(&PreviousTerminal, window, cx);
            assert_eq!(ws.terminal.as_ref().unwrap().entity_id(), first.entity_id());
            ws.close_terminal(&CloseTerminal, window, cx);
            assert_eq!(ws.terminal_views.groups[0].tabs.len(), 1);
            assert_eq!(ws.terminal.as_ref().unwrap().entity_id(), second.entity_id());
            ws.close_panel(&ClosePanel, window, cx);
            assert!(second.upgrade().is_some());
            ws.show_terminal(&ShowTerminal, window, cx);
            assert_eq!(ws.terminal.as_ref().unwrap().entity_id(), second.entity_id());
            ws.close_terminal(&CloseTerminal, window, cx);
            assert!(ws.terminal.is_none());
            (first, second)
        });
        cx.run_until_parked();
        assert!(first.upgrade().is_none());
        assert!(second.upgrade().is_none());
    }

    #[gpui::test]
    fn terminal_file_locations_reuse_documents_and_jump_in_an_already_open_file(cx: &mut gpui::TestAppContext) {
        let path = temp("orlo-terminal-links").canonicalize().unwrap().join("my file.txt");
        std::fs::write(&path, "first\n中😀target\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(path.clone(), window, cx);
            let document = ws.active_editor().unwrap().read(cx).document.clone();
            ws.open_terminal_file(&OpenTerminalFile { path, line: 2, column: 3 }, window, cx);
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).document, document);
            assert_eq!(editor.read(cx).state.selections[0].end(), 8);
        });
    }
}
