//! The status bar.

use crate::workspace::*;

fn indentation_text(doc: &Document) -> String {
    match doc.translate_tabs {
        true => format!("Spaces: {}", doc.indent_width()),
        false => format!("Tab Size: {}", doc.tab_size),
    }
}

impl Workspace {
    /// The status bar's left text: where the caret is, what is selected, or the find panel's
    /// match count, in Sublime's wording.
    pub(crate) fn position_text(&self, e: &Editor, cx: &App) -> String {
        let (line, col) = e.doc(cx).point(e.state.cursor());
        let sels = &e.state.selections;
        let matches = &e.find_ranges;
        // the find panel's count while it is open, as Sublime shows it: "N of M" with
        // one selection that is a match, else just the total
        let current = match sels.as_slice() {
            [one] => matches.binary_search_by_key(&one.start(), |r| r.start).ok(),
            _ => None,
        }
        .filter(|&i| matches[i].end == sels[0].end());
        match sels.as_slice() {
            _ if self.find_panel().is_some() && !matches.is_empty() => match current {
                Some(i) => format!("{} of {} matches", i + 1, matches.len()),
                None => format!("{} matches", matches.len()),
            },
            [one] if !one.is_empty() => match one.end() - one.start() {
                1 => "1 character selected".to_string(),
                n => format!("{n} characters selected"),
            },
            [_] => format!("Line {}, Column {}", line + 1, col + 1),
            many => format!("{} selection regions", many.len()),
        }
    }

    /// Shows `text` on the status bar's left for four seconds, in place of the position (Sublime's
    /// `status_message`); a newer message restarts the time.
    pub fn status_message(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        // replacing the task drops the older message's timer
        self.status = Some((
            text.into(),
            cx.spawn(async move |ws, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(4)).await;
                ws.update(cx, |ws, cx| {
                    ws.status = None;
                    cx.notify();
                })
                .ok();
            }),
        ));
        cx.notify();
    }

    /// The status bar's left text: a status message while one shows, else the caret's position
    /// and the view's flags.
    pub(crate) fn left_text(&self, cx: &App) -> String {
        if let Some((message, _)) = &self.status {
            return message.clone();
        }
        // an empty group has no caret
        let Some(editor) = self.active_editor() else {
            return String::new();
        };
        let e = editor.read(cx);
        let d = e.doc(cx);
        let mut position = self.position_text(e, cx);
        position.push_str(&format!("    {}", d.encoding.name()));
        if d.bom {
            position.push_str(" BOM");
        }
        if d.read_only {
            position.push_str("    READ ONLY");
        }
        // three autosaves in a row failed: unsaved work would not survive a crash
        if session_unsaved(cx) {
            position.push_str("    SESSION NOT SAVED");
        }
        position
    }

    /// The status bar: panel toggles as icons at either end, the caret's place and the file's
    /// encoding and flags as quieter segments, the language server's findings, then the branch,
    /// indentation and syntax.
    pub(crate) fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        struct Active {
            indentation: String,
            overtype: bool,
            viewer: bool,
            syntax: String,
            errors: usize,
            warnings: usize,
            lsp_error: Option<String>,
            lsp_waiting: Option<PathBuf>,
        }
        // an empty group has no syntax to show
        let active = self.active_editor().map(|editor| {
            let e = editor.read(cx);
            let current = e.language_current(cx);
            let count = |severity| e.diagnostics.iter().filter(|d| d.severity == severity).count();
            Active {
                // Sublime's wording
                indentation: indentation_text(e.doc(cx)),
                overtype: e.state.overtype,
                viewer: e.doc(cx).viewer_mode(),
                syntax: e.highlighter.syntax_name().to_string(),
                errors: if current { count(1) } else { 0 },
                warnings: if current { count(2) } else { 0 },
                // a running server's status is "LSP: <language>"; a sentence says why it is not
                lsp_error: (e.lsp_status.strip_prefix("LSP: "))
                    .filter(|why| why.contains(' ') && !e.lsp_status.starts_with(LSP_WAITING))
                    .map(str::to_string),
                lsp_waiting: e.lsp_status.strip_prefix(LSP_WAITING).map(PathBuf::from),
            }
        });
        let terminal_shown =
            !self.panel_hidden && !self.panel_collapsed && matches!(self.panel, Some(Panel::Terminal(_)));
        let item = |id: &'static str| {
            div()
                .id(id)
                .h(px(20.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(kit::RADIUS))
                .whitespace_nowrap()
                .cursor_pointer()
                .hover(|s| s.bg(ui(theme::ELEMENT_HOVER)))
        };
        let text = |t: String| div().px(px(6.)).whitespace_nowrap().child(t);
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(theme::STATUS_BAR_H))
            .w_full()
            .px(px(4.))
            .bg(ui(theme::PANEL_BG))
            .border_t_1()
            .border_color(ui(theme::BORDER))
            .text_color(ui(theme::UI_FG))
            .text_size(px(11.))
            .child(
                kit::icon_toggle("sidebar-toggle", Icon::PanelLeft, "Toggle Side Bar", self.sidebar.visible)
                    .size(px(20.))
                    .on_click(
                        cx.listener(|ws, _: &ClickEvent, window, cx| ws.toggle_sidebar(&ToggleSidebar, window, cx)),
                    ),
            )
            .child(
                kit::icon_toggle("terminal-toggle", Icon::Terminal, "Toggle Terminal", terminal_shown)
                    .debug_selector(|| "terminal-toggle".into())
                    .size(px(20.))
                    .on_click(cx.listener(|ws, _: &ClickEvent, w, cx| ws.show_terminal(&ShowTerminal, w, cx))),
            )
            // the position first, then the encoding and flags, quieter; an unsaved session alarms
            .child(div().ml(px(4.)).flex().items_center().gap(px(6.)).children(
                self.left_text(cx).split("    ").enumerate().map(|(i, part)| {
                    let tone = match (i, part) {
                        (_, "SESSION NOT SAVED") => theme::ERROR_FG,
                        (0, _) => theme::UI_FG,
                        _ => theme::TEXT_MUTED,
                    };
                    text(part.to_string()).text_color(ui(tone))
                }),
            ))
            .when_some(
                self.active_editor().and_then(|e| {
                    e.read(cx).document.read(cx).following.as_ref().map(|f| (f.status.clone(), f.paused))
                }),
                |d, (status, paused)| {
                    d.child(
                        item("stop-following-log")
                            .child(if paused { "FOLLOW PAUSED · Stop" } else { "FOLLOW · Stop" })
                            .tooltip(kit::tooltip(status.into()))
                            .on_click(
                                cx.listener(|ws, _: &ClickEvent, w, cx| {
                                    ws.stop_following_log(&StopFollowingLog, w, cx)
                                }),
                            ),
                    )
                },
            )
            .when(active.as_ref().is_some_and(|a| a.viewer), |d| {
                d.child(
                    item("viewer-mode")
                        .debug_selector(|| "viewer-mode".into())
                        .child("VIEWER · Unlock")
                        .tooltip(kit::tooltip("Viewer mode protects all shared views. Turn off viewer mode".into()))
                        .on_click(
                            cx.listener(|ws, _: &ClickEvent, w, cx| ws.toggle_viewer_mode(&ToggleViewerMode, w, cx)),
                        ),
                )
            })
            .when_some(
                active.as_ref().filter(|a| a.errors + a.warnings > 0).map(|a| (a.errors, a.warnings)),
                |d, (errors, warnings)| {
                    d.child(
                        item("diagnostics")
                            .ml(px(4.))
                            .text_color(ui(theme::TEXT_MUTED))
                            .child(icon(Icon::Error, 12.).text_color(ui(theme::GIT_CONFLICT)))
                            .child(errors.to_string())
                            .child(icon(Icon::Warning, 12.).ml(px(4.)).text_color(ui(theme::GIT_MODIFIED)))
                            .child(warnings.to_string())
                            .tooltip(kit::tooltip("Show Diagnostics".into()))
                            .on_click(
                                cx.listener(|ws, _: &ClickEvent, w, cx| ws.lsp_diagnostics(&LspDiagnostics, w, cx)),
                            ),
                    )
                },
            )
            .children(
                [self.search_progress(), self.pipeline_progress(), self.index_status(cx)]
                    .into_iter()
                    .flatten()
                    .map(|p| text(p).ml(px(12.)).text_color(ui(theme::TEXT_MUTED))),
            )
            .child(div().flex_1())
            .when_some(active.as_ref(), |d, a| {
                d.child(
                    item("overtype-toggle")
                        .debug_selector(|| "overtype-toggle".into())
                        .child(
                            div()
                                .debug_selector({
                                    let overtype = a.overtype;
                                    move || if overtype { "overtype-ovr" } else { "overtype-ins" }.into()
                                })
                                .child(if a.overtype { "OVR" } else { "INS" }),
                        )
                        .tooltip(kit::tooltip("Toggle Overtype (INS: insert, OVR: replace)".into()))
                        .on_click(cx.listener(|ws, _: &ClickEvent, window, cx| {
                            if let Some(editor) = ws.active_editor() {
                                editor.update(cx, |e, cx| {
                                    e.perform_action(&editor::ToggleOvertype, window, cx);
                                    window.focus(&e.focus_handle(cx));
                                });
                            }
                            cx.notify();
                        })),
                )
            })
            .when_some(active.as_ref().and_then(|a| a.lsp_waiting.clone()), |d, folder| {
                let tip = format!("Language servers may build the project they serve. Trust {}", tilde(&folder));
                d.child(
                    item("lsp-trust")
                        .debug_selector(|| "lsp-trust".into())
                        .text_color(ui(theme::TEXT_MUTED))
                        .child(icon(Icon::Warning, 12.).text_color(ui(theme::GIT_MODIFIED)))
                        .child("Language server waiting · Trust Folder")
                        .tooltip(kit::tooltip(tip.into()))
                        .on_click(cx.listener(move |ws, _: &ClickEvent, w, cx| {
                            ws.trust_folder(&TrustFolder { path: folder.clone() }, w, cx)
                        })),
                )
            })
            .when_some(active.as_ref().and_then(|a| a.lsp_error.clone()), |d, error| {
                d.child(
                    item("lsp-status")
                        .text_color(ui(theme::TEXT_MUTED))
                        .child(icon(Icon::Warning, 12.).text_color(ui(theme::TEXT_MUTED)))
                        .child("Language server unavailable")
                        .tooltip(kit::tooltip(error.into())),
                )
            })
            .when_some(
                self.git_status(cx).or_else(|| self.active_branch(cx).map(|b| (b, 0, 0))),
                |d, (branch, changed, conflicts)| {
                    d.child(
                        item("git-status")
                            .child(icon(Icon::GitBranch, 12.).text_color(ui(theme::SIDEBAR_TOGGLE)))
                            .child(branch)
                            .when(changed > 0, |d| {
                                d.child(
                                    div()
                                        .px(px(5.))
                                        .h(px(14.))
                                        .flex()
                                        .items_center()
                                        .rounded(px(7.))
                                        .bg(ui(theme::BUTTON_SELECTED_BG))
                                        .text_size(px(10.))
                                        .text_color(ui(match conflicts > 0 {
                                            true => theme::GIT_CONFLICT,
                                            false => theme::TEXT_MUTED,
                                        }))
                                        .child(changed.to_string()),
                                )
                            })
                            .tooltip(kit::tooltip("Switch Branch…".into()))
                            .on_click(
                                cx.listener(|ws, _: &ClickEvent, w, cx| ws.git_switch_branch(&GitSwitchBranch, w, cx)),
                            ),
                    )
                },
            )
            .when_some(active, |d, a| d.child(text(a.indentation).ml(px(8.))).child(text(a.syntax).ml(px(8.))))
            .child(
                kit::icon_toggle("git-toggle", Icon::GitPanel, "Toggle Git Panel", self.git.sidebar_open)
                    .debug_selector(|| "git-toggle".into())
                    .ml(px(6.))
                    .size(px(20.))
                    .on_click(cx.listener(|ws, _: &ClickEvent, w, cx| ws.show_git(&ShowGit, w, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_status_uses_indent_size_while_tabs_status_uses_tab_width() {
        let mut state = EditorState::new();
        state.doc.tab_size = 8;
        state.doc.indent_size = Some(2);
        state.doc.translate_tabs = true;
        assert_eq!(indentation_text(&state.doc), "Spaces: 2");
        state.doc.translate_tabs = false;
        assert_eq!(indentation_text(&state.doc), "Tab Size: 8");
        state.doc.indent_size = None;
        state.doc.translate_tabs = true;
        assert_eq!(indentation_text(&state.doc), "Spaces: 8");
    }
}
