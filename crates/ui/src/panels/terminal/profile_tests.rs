//! Profile launch and live retention through the real workspace/terminal owners.
use super::*;
use crate::config::settings::Settings;
use crate::workspace::ui_tests::{settle_until, temp, window};
use crate::workspace::ShowTerminal;

#[gpui::test]
fn default_named_profile_uses_literal_argv_and_start_directory(cx: &mut gpui::TestAppContext) {
    let directory = temp("t05-profile start café");
    let (ws, cx) = window(cx);
    let terminal = ws.update_in(cx, |ws, window, cx| {
        let (settings, warning) = crate::config::settings::parse(&serde_json::json!({"terminal": {
            "shell":"/bin/sh", "arguments":["-c","printf LEGACY"],
            "default_profile":"Build café", "profiles":{"Build café":{
                "shell":"/bin/sh", "arguments":["-c", "printf 'PROFILE:<%s>:<%s> cwd:%s' \"$1\" \"$2\" \"$PWD\"", "orlo-t05", "literal $HOME with spaces", ""],
                "directory": directory, "scrollback": 2
            }}
        }}).to_string()).unwrap();
        assert!(warning.is_none());
        cx.global_mut::<Settings>().terminal = settings.terminal;
        ws.show_terminal(&ShowTerminal, window, cx);
        let terminal = ws.terminal.clone().unwrap();
        terminal.update(cx, |t, _| t._poll = None);
        terminal
    });
    settle_until(cx, |cx| {
        terminal.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.exited().is_some()
        })
    });
    terminal.read_with(cx, |t, _| {
        let text = t.backend.screen().contents();
        assert!(text.contains("PROFILE:<literal $HOME with spaces>:<>"), "selected profile must launch: {text}");
        assert!(text.contains(&directory.display().to_string()));
        assert_eq!(t.cwd, directory);
    });
}

#[test]
fn profile_and_live_retention_commands_are_reachable() {
    let labels: Vec<_> = crate::chrome::quick_panel::command_items().into_iter().map(|item| item.label).collect();
    for label in ["Terminal: New with Profile…", "Terminal: Set Scrollback…"] {
        assert!(labels.iter().any(|name| name == label), "missing command: {label}");
    }
}

fn set_preferences(
    ws: &Entity<crate::workspace::Workspace>,
    cx: &mut gpui::VisualTestContext,
    value: serde_json::Value,
) {
    let (settings, warning) =
        crate::config::settings::parse(&serde_json::json!({"terminal":value}).to_string()).unwrap();
    assert!(warning.is_none(), "{warning:?}");
    ws.update(cx, |_, cx| cx.global_mut::<Settings>().terminal = settings.terminal);
}

fn wait_output(terminal: &Entity<Terminal>, cx: &mut gpui::VisualTestContext, marker: &str) {
    settle_until(cx, |cx| {
        terminal.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.screen().contents().contains(marker)
        })
    });
}

fn submit_rows(ws: &Entity<crate::workspace::Workspace>, cx: &mut gpui::VisualTestContext, rows: &str) {
    ws.update_in(cx, |ws, window, cx| {
        let Some(crate::workspace::Panel::Form(form)) = &ws.panel else { panic!("scrollback form") };
        form.inputs().next().unwrap().update(cx, |e, cx| e.set_text(rows, cx));
        ws.submit_form(&crate::workspace::form::Submit, window, cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn chooser_profiles_keep_independent_real_ptys_and_argv_after_preferences_change(cx: &mut gpui::TestAppContext) {
    use crate::workspace::{NewTerminal, NewTerminalWithProfile, SetTerminalScrollback};
    use std::os::unix::fs::PermissionsExt;
    let first_dir = temp("t05-independent-one").canonicalize().unwrap();
    let second_dir = temp("t05-independent-two café").canonicalize().unwrap();
    let executable = first_dir.join("fake shell with spaces");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
stty -echo
printf '%s START argc:%s arg:<%s> empty:<%s> cwd:%s pid:%s\n' "$1" "$#" "$2" "$3" "$PWD" "$$"
IFS= read -r value
printf '%s GOT:%s pid:%s\n' "$1" "$value" "$$"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (ws, cx) = window(cx);
    set_preferences(
        &ws,
        cx,
        serde_json::json!({"default_profile":"First", "profiles": {
            "First":{"shell":executable,"arguments":["ONE","literal $HOME space 'quoted'",""],"directory":first_dir,"scrollback":3},
            "Second":{"shell":executable,"arguments":["TWO","second arguments",""],"directory":second_dir,"scrollback":0}
        }}),
    );
    let first = ws.update_in(cx, |ws, window, cx| {
        ws.show_terminal(&ShowTerminal, window, cx);
        let t = ws.terminal.clone().unwrap();
        t.update(cx, |t, _| t._poll = None);
        t
    });
    wait_output(&first, cx, "ONE START");
    let before = first.read_with(cx, |t, _| {
        let text = t.backend.screen().contents();
        assert!(text.contains("argc:3 arg:<literal $HOME space 'quoted'> empty:<>"), "{text}");
        (text, t.backend.screen().identity())
    });
    cx.run_until_parked();
    let button = cx.debug_bounds("terminal-profiles").expect("reachable profile chooser");
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    ws.update_in(cx, |ws, window, cx| {
        let picker = ws.quick_panel.as_mut().unwrap();
        picker.selected = picker.items.iter().position(|item| item.label == "Second").unwrap();
        ws.confirm_quick_panel(window, cx);
    });
    cx.run_until_parked();
    let second = ws.read_with(cx, |ws, _| ws.terminal.clone().unwrap());
    second.update(cx, |t, _| t._poll = None);
    assert_ne!(first, second);
    wait_output(&second, cx, "TWO START");
    ws.update_in(cx, |ws, window, cx| {
        cx.global_mut::<Settings>().terminal.default_profile = Some("Second".into());
        ws.set_terminal_scrollback(&SetTerminalScrollback, window, cx);
    });
    for invalid in ["-1", "100001", "1.5"] {
        submit_rows(&ws, cx, invalid);
        ws.read_with(cx, |ws, _| {
            assert!(matches!(&ws.panel, Some(crate::workspace::Panel::Form(form)) if form.error.is_some()))
        });
        second.read_with(cx, |t, _| assert_eq!(t.scrollback_limit(), 0));
    }
    submit_rows(&ws, cx, "2");
    first.read_with(cx, |t, _| {
        assert_eq!(t.scrollback_limit(), 3);
        assert!(before.1.matches(t.backend.screen()), "other profile/retention cannot change the first screen epoch");
        assert_eq!(t.backend.screen().contents(), before.0);
        assert_eq!(t.cwd, first_dir);
    });
    second.read_with(cx, |t, _| {
        assert_eq!(t.scrollback_limit(), 2);
        assert_eq!(t.cwd, second_dir);
    });
    first.update(cx, |t, cx| t.write(b"first input\n".to_vec(), cx));
    second.update(cx, |t, cx| t.write(b"second input\n".to_vec(), cx));
    wait_output(&first, cx, "ONE GOT:first input");
    wait_output(&second, cx, "TWO GOT:second input");
    first.read_with(cx, |t, _| {
        let text = t.backend.screen().contents();
        let original_pid = before.0.split("pid:").nth(1).unwrap().split_whitespace().next().unwrap();
        assert!(
            text.contains(&format!("ONE GOT:first input pid:{original_pid}")),
            "profile and trim keep the original PTY process"
        );
        assert!(!text.contains("second input"));
    });
    ws.update_in(cx, |ws, window, cx| ws.new_terminal(&NewTerminal, window, cx));
    let third = ws.read_with(cx, |ws, _| ws.terminal.clone().unwrap());
    third.update(cx, |t, _| t._poll = None);
    wait_output(&third, cx, "TWO START");
    ws.update_in(cx, |ws, window, cx| ws.close_terminal(&crate::workspace::CloseTerminal, window, cx));
    // Profile chooser remains bindable using the ordinary action catalog.
    cx.update(|_, cx| assert!(cx.build_action("terminal::NewTerminalWithProfile", None).is_ok()));
    ws.update_in(cx, |ws, window, cx| ws.new_terminal_with_profile(&NewTerminalWithProfile, window, cx));
    assert!(ws.read_with(cx, |ws, _| ws.quick_panel.is_some()));
}

#[gpui::test]
fn missing_profile_executable_reports_error_and_stale_choices_do_not_launch(cx: &mut gpui::TestAppContext) {
    use crate::workspace::terminal::LaunchTerminalProfile;
    let (ws, cx) = window(cx);
    let directory = temp("t05-missing-executable");
    set_preferences(
        &ws,
        cx,
        serde_json::json!({"profiles":{"Missing":{"shell":directory.join("missing exe"),"arguments":[]}},"default_profile":"Missing"}),
    );
    let saved = ws.read_with(cx, |_, cx| cx.global::<Settings>().terminal.profiles["Missing"].clone());
    ws.update_in(cx, |ws, window, cx| ws.show_terminal(&ShowTerminal, window, cx));
    let t = ws.read_with(cx, |ws, _| ws.terminal.clone().unwrap());
    t.read_with(cx, |t, _| {
        assert!(t.backend.pty().is_none());
        assert!(t.backend.error().unwrap().contains("Cannot start terminal"));
    });
    ws.update_in(cx, |ws, window, cx| {
        cx.global_mut::<Settings>().terminal.profiles.get_mut("Missing").unwrap().arguments.push("changed".into());
        ws.launch_terminal_profile(
            &LaunchTerminalProfile { name: "Missing".into(), profile: saved.clone() },
            window,
            cx,
        );
        assert_eq!(ws.terminal_views.tabs().count(), 1);
        assert!(ws.status.as_ref().unwrap().0.contains("changed"));
        cx.global_mut::<Settings>().terminal.profiles.clear();
        ws.launch_terminal_profile(&LaunchTerminalProfile { name: "Missing".into(), profile: saved }, window, cx);
        assert_eq!(ws.terminal_views.tabs().count(), 1);
    });
}

#[gpui::test]
fn missing_or_file_start_directory_falls_back_to_active_session_cwd(cx: &mut gpui::TestAppContext) {
    use crate::workspace::NewTerminal;
    let fallback = temp("t05-cwd-fallback café").canonicalize().unwrap();
    let (ws, cx) = window(cx);
    let source = fallback.join("source.txt");
    std::fs::write(&source, "source").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_path(source, window, cx));
    set_preferences(
        &ws,
        cx,
        serde_json::json!({"shell":"/bin/sh","arguments":["-c",r#"printf 'FALLBACK:%s' "$PWD""#]}),
    );
    let first = ws.update_in(cx, |ws, window, cx| {
        ws.show_terminal(&ShowTerminal, window, cx);
        let t = ws.terminal.clone().unwrap();
        t.update(cx, |t, _| t._poll = None);
        t
    });
    wait_output(&first, cx, "FALLBACK");
    let file = fallback.join("file.txt");
    std::fs::write(&file, "file").unwrap();
    for directory in [fallback.join("missing"), file] {
        set_preferences(
            &ws,
            cx,
            serde_json::json!({"default_profile":"Cwd", "profiles":{"Cwd":{"shell":"/bin/sh","arguments":["-c",r#"printf 'cwd:%s' "$PWD""#],"directory":directory,"scrollback":0}}}),
        );
        let t = ws.update_in(cx, |ws, window, cx| {
            ws.new_terminal(&NewTerminal, window, cx);
            let t = ws.terminal.clone().unwrap();
            t.update(cx, |t, _| t._poll = None);
            t
        });
        wait_output(&t, cx, "cwd:");
        t.read_with(cx, |t, _| {
            assert_eq!(t.cwd, fallback);
            assert_eq!(t.backend.working_directory(), fallback);
        });
    }
}

#[gpui::test]
fn live_trim_cancels_held_search_and_removes_old_ranges_without_restarting_pty(cx: &mut gpui::TestAppContext) {
    use crate::workspace::SetTerminalScrollback;
    let (ws, cx) = window(cx);
    let cwd = temp("t05-held-search").canonicalize().unwrap();
    set_preferences(
        &ws,
        cx,
        serde_json::json!({"default_profile":"Live","profiles":{"Live":{
        "shell":"/bin/sh","directory":cwd,"scrollback":100,"arguments":["-c",r#"
stty -echo
printf '\033]133;B\007run\033]133;C\007'
i=0; while [ "$i" -lt 80 ]; do printf 'obsolete-%s\n' "$i"; i=$((i+1)); done
printf '\033]133;D;0\007\033[?1003h\033[?1006hREADY\n'
IFS= read -r value; printf 'received:%s\n' "$value"
"#]}}}),
    );
    let t = ws.update_in(cx, |ws, window, cx| {
        ws.show_terminal(&ShowTerminal, window, cx);
        let t = ws.terminal.clone().unwrap();
        t.update(cx, |t, _| t._poll = None);
        t
    });
    wait_output(&t, cx, "READY");
    t.update_in(cx, |t, window, cx| t.open_search(&FindOutput, window, cx));
    cx.simulate_input("obsolete-0");
    settle_until(cx, |cx| {
        cx.executor().advance_clock(Duration::from_millis(100));
        t.read_with(cx, |t, _| t.search.as_ref().unwrap().results.as_ref().is_some_and(|r| r.len() == 1))
    });
    let epoch = t.update(cx, |t, cx| {
        t.set_display_selection(Some(((0, 0), (0, 6))));
        t.pin_search();
        let epoch = t.backend.screen().identity();
        t.set_scrollback_limit(100, cx);
        assert_eq!(t.display_selection(), Some(((0, 0), (0, 6))), "unchanged limit preserves pinned view");
        assert!(epoch.matches(t.backend.screen()));
        epoch
    });
    let hold = crate::testing::gate::pause("terminal-search-scan", &cwd);
    t.update(cx, |t, cx| t.refresh_search(true, cx));
    settle_until(cx, |cx| {
        cx.executor().advance_clock(Duration::from_millis(100));
        hold.arrived() > 0
    });
    ws.update_in(cx, |ws, window, cx| ws.set_terminal_scrollback(&SetTerminalScrollback, window, cx));
    submit_rows(&ws, cx, "0");
    t.read_with(cx, |t, _| {
        assert!(!epoch.matches(t.backend.screen()));
        assert!(t.display_selection().is_none());
        assert!(t.retained_output.is_none());
        assert!(t.command_review.is_none());
        assert!(t.backend.commands().records().is_empty());
        assert!(
            t.search.as_ref().unwrap().results.as_ref().is_none_or(|r| r.screen().same_layout(t.backend.screen())),
            "trim cannot retain the preceding search epoch"
        );
        assert!(orlo_terminal::mouse::requested(t.backend.screen()));
        assert!(t.backend.pty().is_some());
    });
    hold.release();
    settle_until(cx, |cx| {
        cx.executor().advance_clock(Duration::from_millis(100));
        t.read_with(cx, |t, _| {
            t.search
                .as_ref()
                .unwrap()
                .results
                .as_ref()
                .is_some_and(|r| r.is_empty() && r.screen().same_contents(t.backend.screen()))
        })
    });
    t.update(cx, |t, _| {
        t.backend.scroll_lines(999);
        assert_eq!(t.backend.screen().scrollback(), 0);
        t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"still alive\n".to_vec())).unwrap();
    });
    wait_output(&t, cx, "received:still alive");
}

#[gpui::test]
fn retention_change_drops_retained_command_views_and_rejects_removed_form_targets(cx: &mut gpui::TestAppContext) {
    use crate::workspace::{NewTerminal, SetTerminalScrollback};
    let (ws, cx) = window(cx);
    set_preferences(
        &ws,
        cx,
        serde_json::json!({"shell":"/bin/sh","arguments":["-c",r#"printf '\033]133;B\007run\033]133;C\007old-output\033]133;D;0\007READY'"#],"scrollback":20}),
    );
    let t = ws.update_in(cx, |ws, window, cx| {
        ws.show_terminal(&ShowTerminal, window, cx);
        let t = ws.terminal.clone().unwrap();
        t.update(cx, |t, _| t._poll = None);
        t
    });
    wait_output(&t, cx, "READY");
    t.update(cx, |t, cx| {
        t.review_command(false, cx);
        assert!(t.command_review.is_some());
        t.set_scrollback_limit(10, cx);
        assert!(t.command_review.is_none());
        assert!(t.backend.commands().records().is_empty());
        t.retained_output = orlo_terminal::search::SearchResults::find(
            t.backend.screen().clone(),
            "old-output",
            &std::sync::atomic::AtomicBool::new(false),
        );
        t.set_display_selection(Some(((0, 0), (0, 3))));
        t.set_scrollback_limit(10, cx);
        assert!(t.retained_output.is_some());
        t.set_scrollback_limit(0, cx);
        assert!(t.retained_output.is_none());
        assert!(t.display_selection().is_none());
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.set_terminal_scrollback(&SetTerminalScrollback, window, cx);
        let form = ws.panel.take();
        ws.close_terminal(&crate::workspace::CloseTerminal, window, cx);
        ws.new_terminal(&NewTerminal, window, cx);
        ws.panel = form;
    });
    submit_rows(&ws, cx, "1");
    ws.read_with(cx, |ws, cx| {
        let Some(crate::workspace::Panel::Form(form)) = &ws.panel else { panic!("stale target must keep its error") };
        assert!(form.error.as_deref().unwrap().contains("closed"));
        assert_eq!(ws.terminal.as_ref().unwrap().read(cx).scrollback_limit(), 20);
    });
    t.read_with(cx, |t, _| {
        assert!(t.backend.pty().is_none());
        assert_eq!(t.scrollback_limit(), 0);
    });
}

#[gpui::test]
fn profile_and_retention_menu_palette_and_bound_action_dispatch_reach_existing_widgets(cx: &mut gpui::TestAppContext) {
    use crate::workspace::{CommandPalette, Panel, SetTerminalScrollback};
    let (ws, cx) = window(cx);
    set_preferences(
        &ws,
        cx,
        serde_json::json!({"profiles":{"Plain":{"shell":"/bin/sh","arguments":["-c","printf READY"]}},"default_profile":"Plain"}),
    );
    ws.update_in(cx, |ws, window, cx| ws.show_terminal(&ShowTerminal, window, cx));
    let actions = ws.read_with(cx, |ws, cx| {
        let menus = ws.menus(cx);
        let tools = menus.iter().find(|m| m.name == "Terminal").unwrap();
        ["New Terminal with Profile…", "Set Terminal Scrollback…"].map(|label| {
            tools
                .items
                .iter()
                .find_map(|item| match item {
                    gpui::MenuItem::Action { name, action, .. } if name == label => Some(action.boxed_clone()),
                    _ => None,
                })
                .expect("new commands appear in Tools")
        })
    });
    cx.run_until_parked();
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    ws.update(cx, |ws, cx| {
        let before = ws.menus_built;
        assert!(ws.set_menus_now(cx));
        assert!(ws.menus_built > before);
    });
    cx.update(|window, cx| window.dispatch_action(actions[1].boxed_clone(), cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(matches!(ws.panel, Some(Panel::Form(_)))));
    submit_rows(&ws, cx, "0");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.terminal.as_ref().unwrap().read(cx).scrollback_limit(), 0));
    cx.update(|window, cx| window.dispatch_action(actions[0].boxed_clone(), cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_some()));
    ws.update_in(cx, |ws, window, cx| {
        ws.quick_panel_dismiss(&crate::chrome::quick_panel::Dismiss, window, cx);
        ws.command_palette(&CommandPalette, window, cx);
        let palette = ws.quick_panel.as_mut().unwrap();
        palette.selected =
            palette.filtered.iter().position(|&i| palette.items[i].label == "Terminal: Set Scrollback…").unwrap();
        ws.confirm_quick_panel(window, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(matches!(ws.panel, Some(Panel::Form(_)))));
    cx.update(|_, cx| {
        assert_eq!(
            cx.build_action("terminal::SetTerminalScrollback", None).unwrap().name(),
            gpui::Action::name(&SetTerminalScrollback)
        )
    });
}

#[gpui::test]
fn profile_and_retention_without_configuration_explain_the_next_step(cx: &mut gpui::TestAppContext) {
    use crate::workspace::{NewTerminalWithProfile, SetTerminalScrollback};
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.new_terminal_with_profile(&NewTerminalWithProfile, window, cx);
        assert!(ws.status.as_ref().unwrap().0.contains("terminal.profiles"));
        ws.set_terminal_scrollback(&SetTerminalScrollback, window, cx);
        assert!(ws.status.as_ref().unwrap().0.contains("Open a terminal"));
        assert!(ws.terminal.is_none());
        assert_eq!(ws.terminal_views.tabs().count(), 0);
    });
}
