//! Editor-to-terminal actions exercise the chosen real PTY, source guards and paste protocol.
use super::*;
use crate::config::settings::{Settings, TerminalProfile};
use crate::editing::text::{Goal, Selection, VirtualSpace};
use crate::workspace::terminal::SelectTerminalTab;
use crate::workspace::ui_tests::{settle_until, temp, window};
use crate::workspace::{
    ClosePanel, ExecuteSelectionInTerminal, NewTerminal, NewViewIntoFile, Panel, SendSelectionToTerminal, ShowTerminal,
    Workspace,
};
use gpui::{Action, TestAppContext, VisualTestContext};

fn capture_script(count: usize, bracketed: bool) -> String {
    format!(
        "stty raw -echo\nprintf '\\033]133;C\\007pinned output\\033]133;D;0\\007\\r\\n'\n{}\nprintf 'READY\\r\\nBYTES:'\ndd bs=1 count={} 2>/dev/null | od -An -tx1 | tr -d ' \\n'\nprintf '\\r\\nDONE\\r\\n'\nexec sleep 60",
        if bracketed { "printf '\\033[?2004h'" } else { "" },
        count + 1,
    )
}

fn configure_capture(cx: &mut Context<Workspace>, count: usize, bracketed: bool) {
    let settings = &mut cx.global_mut::<Settings>().terminal;
    settings.shell = Some("/bin/sh".into());
    settings.arguments = Some(vec!["-c".into(), capture_script(count, bracketed)]);
}

fn ready(terminal: &Entity<Terminal>, cx: &mut VisualTestContext) {
    settle_until(cx, |cx| {
        terminal.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.screen().contents().contains("READY")
        })
    });
}

fn send(ws: &mut Workspace, execute: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    if execute {
        ws.execute_selection_in_terminal(&ExecuteSelectionInTerminal, window, cx);
    } else {
        ws.send_selection_to_terminal(&SendSelectionToTerminal, window, cx);
    }
}

fn expect_bytes(terminal: &Entity<Terminal>, expected: &[u8], cx: &mut VisualTestContext) {
    // A sentinel after the action catches an unwanted extra newline as well as missing bytes.
    terminal.update(cx, |t, _| t.backend.write(b"Q".to_vec()));
    settle_until(cx, |cx| {
        terminal.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.screen().contents().contains("DONE")
        })
    });
    let hex: String = expected.iter().chain(b"Q").map(|b| format!("{b:02x}")).collect();
    terminal.read_with(cx, |t, _| {
        let output = t.backend.screen().contents();
        assert_eq!(output.split("BYTES:").nth(1).unwrap().split_whitespace().next(), Some(hex.as_str()), "{output}");
    });
}

#[gpui::test]
fn t07_physical_selections_send_in_document_order_without_changing_shared_read_only_history(cx: &mut TestAppContext) {
    for (bracketed, execute, expected) in [
        (false, false, "é\r中\rZ"),
        (false, true, "é\r中\rZ\r"),
        (true, false, "\x1b[200~é\n中\nZ\x1b[201~"),
        (true, true, "\x1b[200~é\n中\nZ\x1b[201~\r"),
    ] {
        let (ws, cx) = window(cx);
        let (source, sibling, terminal) = ws.update_in(cx, |ws, w, cx| {
            configure_capture(cx, expected.len(), bracketed);
            let source = ws.active_editor().unwrap();
            source.update(cx, |e, cx| {
                e.set_text("é\n中\nZ", cx);
                e.document.update(cx, |d, _| d.model.mark_saved());
                e.edit(cx, |s| {
                    s.set_cursor(s.len());
                    s.insert("!");
                });
                e.edit(cx, |s| s.undo());
            });
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            let sibling = ws.active_editor().unwrap();
            sibling.update(cx, |e, cx| {
                e.set_virtual_space(VirtualSpace::All, cx);
                e.document.update(cx, |d, _| d.model.read_only = true);
            });
            ws.show_terminal(&ShowTerminal, w, cx);
            let terminal = ws.terminal.clone().unwrap();
            terminal.update(cx, |t, _| t._poll = None);
            (source, sibling, terminal)
        });
        ready(&terminal, cx);
        ws.update_in(cx, |ws, w, cx| {
            // No frame may normalize the deliberately reversed extraction order before dispatch.
            sibling.update(cx, |e, _| {
                e.state.selections = vec![
                    Selection { goal: Some(Goal::X(42.)), rectangular: true, ..Selection::new(5, 4) },
                    Selection::from_points((1, 2), (1, 5)).in_rectangle(),
                    Selection::caret(2),
                    Selection::new(3, 0),
                ];
            });
            let before = sibling.read(cx).read_state(cx);
            let selections = before.selection_state();
            let generation = before.selection_generation();
            let identity = (before.doc.save_point(), before.doc.version, before.doc.dirty);
            let other = source.read(cx).read_state(cx).selection_state();
            terminal.update(cx, |t, cx| t.review_command(false, cx));
            assert!(terminal.read(cx).command_review.is_some());
            send(ws, execute, w, cx);
            let after = sibling.read(cx).read_state(cx);
            assert!(after.selection_state() == selections);
            assert_eq!(after.selection_generation(), generation);
            assert_eq!((after.doc.save_point(), after.doc.version, after.doc.dirty), identity);
            assert!(after.doc.read_only);
            assert_eq!(after.doc.rope.to_string(), "é\n中\nZ");
            assert!(source.read(cx).read_state(cx).selection_state() == other);
            assert_eq!(ws.terminal.as_ref(), Some(&terminal));
            assert!(terminal.read(cx).command_review.is_none(), "admitted input returns to live output");
            sibling.update(cx, |e, cx| {
                e.edit(cx, |s| s.redo());
                assert_eq!(e.text(cx), "é\n中\nZ!");
                e.edit(cx, |s| s.undo());
                assert_eq!(e.text(cx), "é\n中\nZ");
                assert!(!e.doc(cx).dirty, "Undo must still return to the original save point");
            });
        });
        expect_bytes(&terminal, expected.as_bytes(), cx);
    }
}

#[gpui::test]
fn t07_empty_and_virtual_only_selections_never_launch_or_write(cx: &mut TestAppContext) {
    for execute in [false, true] {
        for virtual_only in [false, true] {
            let (ws, cx) = window(cx);
            ws.update_in(cx, |ws, w, cx| {
                let source = ws.active_editor().unwrap();
                source.update(cx, |e, cx| {
                    e.set_text("one\ntwo", cx);
                    e.set_virtual_space(VirtualSpace::All, cx);
                    e.state.selections = if virtual_only {
                        vec![Selection::from_points((3, 2), (3, 5)), Selection::from_points((7, 1), (7, 4))]
                    } else {
                        vec![Selection::caret(0), Selection::caret(4)]
                    };
                });
                let before = source.read(cx).read_state(cx);
                let selections = before.selection_state();
                let identity = (before.doc.save_point(), before.doc.version, before.doc.dirty);
                send(ws, execute, w, cx);
                assert!(ws.terminal.is_none(), "physical text is required before creating a PTY");
                assert!(ws.status.as_ref().unwrap().0.contains("Select editor text"));
                let after = source.read(cx).read_state(cx);
                assert!(after.selection_state() == selections);
                assert_eq!((after.doc.save_point(), after.doc.version, after.doc.dirty), identity);
                assert_eq!(after.doc.rope.to_string(), "one\ntwo");
            });
        }
    }
}

#[gpui::test]
fn t07_dead_selected_pty_reports_error_and_keeps_command_search_and_retained_review(cx: &mut TestAppContext) {
    for review in ["command", "search", "retained"] {
        let (ws, cx) = window(cx);
        let terminal = ws.update_in(cx, |ws, w, cx| {
            let settings = &mut cx.global_mut::<Settings>().terminal;
            settings.shell = Some("/bin/sh".into());
            settings.arguments =
                Some(vec!["-c".into(), "printf '\\033]133;C\\007pinned output\\033]133;D;0\\007'; exit 7".into()]);
            ws.show_terminal(&ShowTerminal, w, cx);
            let terminal = ws.terminal.clone().unwrap();
            terminal.update(cx, |t, _| t._poll = None);
            ws.active_editor().unwrap().update(cx, |e, cx| {
                e.set_text("echo 中", cx);
                e.select_range(0..6, cx);
            });
            terminal
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.exited() == Some(7)
            })
        });
        for execute in [false, true] {
            ws.update_in(cx, |ws, w, cx| {
                terminal.update(cx, |t, cx| {
                    if review == "command" {
                        t.review_command(false, cx);
                    } else {
                        t.open_search(&FindOutput, w, cx);
                        if review == "retained" {
                            t.close_search(&CloseSearch, w, cx);
                        }
                    }
                    t.set_display_selection(Some(((0, 0), (0, 6))));
                    assert_eq!(t.display_selection(), Some(((0, 0), (0, 6))));
                });
                let before = terminal.read(cx).display_screen().clone();
                send(ws, execute, w, cx);
                assert_eq!(ws.terminal.as_ref(), Some(&terminal), "dead session is never replaced");
                assert_eq!(ws.terminal_views.tabs().count(), 1);
                assert!(ws.status.as_ref().unwrap().0.contains("exited"));
                let t = terminal.read(cx);
                assert_eq!(t.command_review.is_some(), review == "command");
                assert_eq!(t.search.is_some(), review == "search");
                assert_eq!(t.retained_output.is_some(), review == "retained");
                assert!(t.display_screen().same_contents(&before));
                assert_eq!(t.display_selection(), Some(((0, 0), (0, 6))));
            });
        }
    }
}

#[gpui::test]
fn t07_source_refusals_preserve_review_and_leave_real_pty_input_untouched(cx: &mut TestAppContext) {
    for refusal in ["empty", "virtual", "oversize", "stale", "ime", "busy", "closed"] {
        let (ws, cx) = window(cx);
        let (source, sibling, terminal) = ws.update_in(cx, |ws, w, cx| {
            configure_capture(cx, 0, false);
            if refusal == "busy" {
                cx.global_mut::<Settings>().terminal.arguments = Some(vec![
                    "-c".into(), "stty raw -echo; printf '\\033]133;C\\007pinned output\\033]133;D;0\\007\\r\\nPRIMING\\r\\n'; dd bs=1 count=1 >/dev/null 2>&1; printf 'READY\\r\\n'; exec sleep 60".into(),
                ]);
            }
            let source = ws.active_editor().unwrap();
            source.update(cx, |e, cx| e.set_text("a\nb", cx));
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            let sibling = ws.active_editor().unwrap();
            sibling.update(cx, |e, cx| e.select_range(0..3, cx));
            ws.show_terminal(&ShowTerminal, w, cx);
            let terminal = ws.terminal.clone().unwrap();
            terminal.update(cx, |t, _| t._poll = None);
            (source, sibling, terminal)
        });
        if refusal == "busy" {
            settle_until(cx, |cx| {
                terminal.update(cx, |t, cx| {
                    t.drain(cx);
                    t.backend.screen().contents().contains("PRIMING")
                })
            });
            // READY acknowledges the first write; its unread remainder keeps the queue from draining.
            terminal.update(cx, |t, _| {
                t.backend.write(vec![b'x'; orlo_terminal::configuration::MAX_INPUT]);
                assert!(t.backend.error().is_none(), "priming input must be admitted before filling the queue");
            });
        }
        ready(&terminal, cx);
        ws.update_in(cx, |ws, w, cx| {
            match refusal {
                "empty" => sibling.update(cx, |e, cx| e.sel(cx).set_cursor(0)),
                "virtual" => sibling.update(cx, |e, cx| {
                    e.set_virtual_space(VirtualSpace::All, cx);
                    e.state.selections =
                        vec![Selection::from_points((1, 2), (1, 4)), Selection::from_points((3, 1), (3, 2))];
                }),
                "oversize" => sibling.update(cx, |e, cx| {
                    e.set_text(&"x".repeat(orlo_terminal::configuration::MAX_INPUT + 1), cx);
                    e.select_range(0..orlo_terminal::configuration::MAX_INPUT + 1, cx);
                }),
                "stale" => {
                    source.update(cx, |e, cx| e.edit(cx, |s| s.insert("!")));
                    assert_ne!(sibling.read(cx).state.revision(), sibling.read(cx).doc(cx).version);
                }
                "ime" => {
                    source.update(cx, |e, cx| e.replace_and_mark_text_in_range(None, "界", None, w, cx));
                    sibling.update(cx, |e, cx| e.catch_up(cx));
                    assert!(!source.read(cx).marked.is_empty());
                    assert_eq!(sibling.read(cx).state.revision(), sibling.read(cx).doc(cx).version);
                }
                "busy" => terminal.update(cx, |t, _| {
                    for _ in 0..64 {
                        t.backend.write(vec![b'x'; orlo_terminal::configuration::MAX_INPUT]);
                        if t.backend.error().is_some() {
                            break;
                        }
                    }
                    assert!(t.backend.error().unwrap().contains("busy"));
                }),
                "closed" => terminal.update(cx, |t, _| t.close()),
                _ => unreachable!(),
            }
            terminal.update(cx, |t, cx| t.review_command(false, cx));
            assert!(terminal.read(cx).command_review.is_some());
            let text = sibling.read(cx).text(cx);
            let selections = sibling.read(cx).read_state(cx).selection_state();
            let generation = sibling.read(cx).read_state(cx).selection_generation();
            let identity = sibling.read(cx).doc(cx).save_point();
            let marked = source.read(cx).marked.clone();
            for execute in [false, true] {
                send(ws, execute, w, cx);
                assert!(terminal.read(cx).command_review.is_some(), "{refusal}: refused input keeps review");
                assert_eq!(terminal.read(cx).display_screen().contents(), "pinned output");
                assert!(sibling.read(cx).read_state(cx).selection_state() == selections);
                assert_eq!(sibling.read(cx).read_state(cx).selection_generation(), generation);
                assert_eq!(sibling.read(cx).doc(cx).save_point(), identity);
                assert_eq!(sibling.read(cx).text(cx), text);
                assert_eq!(source.read(cx).marked, marked);
                let status = &ws.status.as_ref().unwrap().0;
                assert!(
                    status.contains(match refusal {
                        "empty" | "virtual" => "Select editor text",
                        "oversize" => "1 MiB",
                        "stale" => "updating",
                        "ime" => "composition",
                        "busy" => "busy",
                        "closed" => "not running",
                        _ => unreachable!(),
                    }),
                    "{refusal}: {status}"
                );
            }
        });
        if !matches!(refusal, "busy" | "closed") {
            expect_bytes(&terminal, b"", cx);
        }
    }
}

#[gpui::test]
fn t07_hidden_terminal_targets_only_the_chosen_session(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let (first, second) = ws.update_in(cx, |ws, w, cx| {
        configure_capture(cx, 1, false);
        ws.show_terminal(&ShowTerminal, w, cx);
        let first = ws.terminal.clone().unwrap();
        ws.new_terminal(&NewTerminal, w, cx);
        let second = ws.terminal.clone().unwrap();
        for t in [&first, &second] {
            t.update(cx, |t, _| t._poll = None);
        }
        ws.active_editor().unwrap().update(cx, |e, cx| {
            e.set_text("X", cx);
            e.select_range(0..1, cx);
        });
        (first, second)
    });
    ready(&first, cx);
    ready(&second, cx);
    for chosen in [&first, &second] {
        ws.update_in(cx, |ws, w, cx| {
            ws.select_terminal_tab(&SelectTerminalTab { id: chosen.entity_id() }, w, cx);
            ws.close_panel(&ClosePanel, w, cx);
            assert!(ws.panel.is_none());
            ws.panel_hidden = true;
            send(ws, false, w, cx);
            assert!(!ws.panel_hidden);
            assert!(matches!(&ws.panel, Some(Panel::Terminal(t)) if t == chosen));
        });
        expect_bytes(chosen, b"X", cx);
    }
}

#[gpui::test]
fn t07_absent_terminal_uses_default_profile_and_run_appends_after_selected_newline(cx: &mut TestAppContext) {
    for (bracketed, expected) in [(false, "éé\r\r"), (true, "é\x1b[200~é\n\x1b[201~\r")] {
        let (ws, cx) = window(cx);
        let directory = temp(&format!("t07 default profile cwd {bracketed}"));
        let terminal = ws.update_in(cx, |ws, w, cx| {
            let settings = &mut cx.global_mut::<Settings>().terminal;
            assert!(!settings.shell_integration);
            settings.shell = Some("/missing/legacy-shell".into());
            settings.profiles.insert(
                "selected".into(),
                TerminalProfile {
                    shell: "/bin/sh".into(),
                    arguments: vec!["-c".into(), capture_script(expected.len(), bracketed)],
                    directory: Some(directory.clone()),
                    scrollback: 17,
                },
            );
            settings.default_profile = Some("selected".into());
            ws.active_editor().unwrap().update(cx, |e, cx| {
                e.set_text("é", cx);
                e.select_range(0..1, cx);
            });
            assert!(ws.terminal.is_none());
            send(ws, false, w, cx);
            let terminal = ws.terminal.clone().unwrap();
            terminal.update(cx, |t, _| {
                t._poll = None;
                assert_eq!(t.cwd, directory);
                assert_eq!(t.scrollback_limit(), 17);
                assert!(t.backend.pty().is_some());
            });
            terminal
        });
        ready(&terminal, cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| {
                e.set_text("é\n", cx);
                e.select_range(0..2, cx);
            });
            send(ws, true, w, cx);
        });
        expect_bytes(&terminal, expected.as_bytes(), cx);
    }
}

#[gpui::test]
fn t07_send_and_run_is_reachable_under_stable_action_identity(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    ws.read_with(cx, |ws, cx| {
        let tools = ws.menus(cx).into_iter().find(|menu| menu.name == "Terminal").unwrap();
        for (label, identity) in [
            ("Send Selection to Terminal", "terminal::SendSelectionToTerminal"),
            ("Send and Run Selection in Terminal", "terminal::ExecuteSelectionInTerminal"),
        ] {
            assert!(cx.build_action(identity, None).is_ok(), "custom-bindable {identity}");
            assert!(tools.items.iter().any(|item| matches!(item, gpui::MenuItem::Action { name, action, .. } if name == label && action.name() == identity)), "{label}");
        }
    });
    let items = crate::chrome::quick_panel::command_items();
    let item = items.iter().find(|i| i.label == "Terminal: Send and Run").expect("explicit Run palette action");
    let crate::chrome::quick_panel::ItemAction::Dispatch(action) = &item.action else { panic!("dispatch action") };
    assert_eq!(action.name(), ExecuteSelectionInTerminal.name());
}
