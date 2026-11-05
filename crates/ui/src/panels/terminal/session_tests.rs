//! Real independent PTYs exercised through workspace tabs and accepted window close.
use super::*;
use crate::config::settings::Settings;
use crate::workspace::terminal::{CloseTerminalTab, SelectTerminalTab};
use crate::workspace::ui_tests::{settle_until, temp, two_windows, window};
use crate::workspace::{
    ClosePanel, CloseTerminal, CloseWindow, NewTerminal, Panel, RenameTerminal, Save, ShowTerminal, Workspace,
};
use gpui::{TestAppContext, VisualTestContext};
use std::path::Path;
use std::time::Instant;

fn launch(
    ws: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    directory: &Path,
    tag: &str,
    count: usize,
) -> Entity<Terminal> {
    ws.update_in(cx, |ws, window, cx| {
        let settings = &mut cx.global_mut::<Settings>().terminal;
        settings.shell = Some("/bin/sh".into());
        settings.arguments = Some(vec![
            "-c".into(),
            r#"cd "$2" || exit; stty raw -echo
printf '\033]7;file://%s\007%s READY pid:%s\r\n' "$PWD" "$1" "$$"
printf '%s bytes:' "$1"; dd bs=1 count="$3" 2>/dev/null | od -An -tx1 | tr -d ' \n'
printf '\r\nINPUT-DONE\r\n'
dd bs=1 count=1 >/dev/null 2>&1; printf 'SIZE:'; stty size
printf 'SIZE-DONE\r\n'; exec sleep 60"#
                .into(),
            "orlo-t04".into(),
            tag.into(),
            directory.display().to_string(),
            count.to_string(),
        ]);
        if ws.terminal.is_none() {
            ws.show_terminal(&ShowTerminal, window, cx);
        } else {
            ws.new_terminal(&NewTerminal, window, cx);
        }
        let terminal = ws.terminal.clone().unwrap();
        terminal.update(cx, |t, _| t._poll = None);
        terminal
    })
}

fn wait(t: &Entity<Terminal>, cx: &mut VisualTestContext, text: &str) {
    settle_until(cx, |cx| {
        t.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.screen().contents().contains(text)
        })
    });
}

#[derive(Default)]
struct Processes(Vec<libc::pid_t>);

impl Processes {
    fn track(&mut self, t: &Entity<Terminal>, cx: &mut VisualTestContext) -> libc::pid_t {
        wait(t, cx, "READY pid:");
        let pid = t.read_with(cx, |t, _| {
            t.backend
                .screen()
                .contents()
                .split("pid:")
                .nth(1)
                .unwrap()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap()
                .parse()
                .unwrap()
        });
        assert!(pid > 0);
        self.0.push(pid);
        pid
    }

    fn stopped(&self, pid: libc::pid_t) {
        let start = Instant::now();
        while unsafe { libc::kill(pid, 0) } == 0 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "fixture PTY process {pid} remains alive after accepted close");
    }
}

impl Drop for Processes {
    fn drop(&mut self) {
        for &pid in &self.0 {
            if unsafe { libc::kill(pid, 0) } == 0 {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
}

fn select(ws: &Entity<Workspace>, cx: &mut VisualTestContext, t: &Entity<Terminal>) {
    ws.update_in(cx, |ws, window, cx| ws.select_terminal_tab(&SelectTerminalTab { id: t.entity_id() }, window, cx));
    cx.run_until_parked();
    cx.update(|window, cx| assert!(t.read(cx).focus_handle.is_focused(window)));
}

fn rename_form(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.rename_terminal(&RenameTerminal, window, cx));
    cx.run_until_parked();
}

fn submit_name(ws: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str) {
    ws.update_in(cx, |ws, window, cx| {
        let Some(Panel::Form(form)) = &ws.panel else { panic!("rename form") };
        form.inputs().next().unwrap().update(cx, |e, cx| e.set_text(text, cx));
        ws.submit_form(&crate::workspace::form::Submit, window, cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn rename_trims_unicode_clears_cancels_and_refuses_invalid_names_without_changing_the_pty(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-rename-policies");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let t = launch(&ws, cx, &directory, "NAME", 1);
    let pid = processes.track(&t, cx);
    let button = cx.debug_bounds("rename-terminal").expect("terminal header rename");
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    submit_name(&ws, cx, "  build café 中😀  ");
    assert_eq!(t.read_with(cx, |t, _| t.name.clone()).as_deref(), Some("build café 中😀"));
    assert!(cx.debug_bounds("rename-terminal").is_some());
    for invalid in ["bad\nname".to_string(), "bad\u{0}name".to_string(), "界".repeat(129)] {
        rename_form(&ws, cx);
        submit_name(&ws, cx, &invalid);
        ws.read_with(cx, |ws, _| {
            let Some(Panel::Form(form)) = &ws.panel else { panic!("invalid name must keep its form") };
            assert!(form.error.is_some());
        });
        assert_eq!(t.read_with(cx, |t, _| t.name.clone()).as_deref(), Some("build café 中😀"));
    }
    ws.update_in(cx, |ws, window, cx| ws.close_panel(&ClosePanel, window, cx));
    assert_eq!(t.read_with(cx, |t, _| t.name.clone()).as_deref(), Some("build café 中😀"));
    rename_form(&ws, cx);
    submit_name(&ws, cx, "  ");
    assert!(t.read_with(cx, |t, _| t.name.is_none()));
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "rename/clear/cancel never restarts or stops a shell");
    t.update(cx, |t, _| t.backend.write(b"Z".to_vec()));
    wait(&t, cx, "NAME bytes:5a");
    ws.update_in(cx, |ws, window, cx| ws.close_terminal(&CloseTerminal, window, cx));
    processes.stopped(pid);
}

#[gpui::test]
fn a_pending_rename_does_not_keep_a_closed_pty_alive_or_rename_its_replacement(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-stale-rename");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let first = launch(&ws, cx, &directory, "OLD", 1);
    let old_pid = processes.track(&first, cx);
    let weak = first.downgrade();
    rename_form(&ws, cx);
    let pending = ws.update(cx, |ws, _| ws.panel.take().unwrap());
    ws.update_in(cx, |ws, window, cx| ws.close_terminal(&CloseTerminal, window, cx));
    drop(first);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "the rename callback must only hold a weak target");
    processes.stopped(old_pid);
    let replacement = launch(&ws, cx, &directory, "NEW", 1);
    let replacement_pid = processes.track(&replacement, cx);
    ws.update(cx, |ws, cx| {
        ws.panel = Some(pending);
        cx.notify();
    });
    submit_name(&ws, cx, "wrong target");
    ws.read_with(cx, |ws, cx| {
        let Some(Panel::Form(form)) = &ws.panel else { panic!("stale target error") };
        assert_eq!(form.error.as_deref(), Some("This terminal has closed"));
        assert_eq!(ws.terminal.as_ref(), Some(&replacement));
        assert!(replacement.read(cx).name.is_none());
    });
    ws.update_in(cx, |ws, window, cx| ws.close_terminal(&CloseTerminal, window, cx));
    processes.stopped(replacement_pid);
}

#[gpui::test]
fn pending_rename_rejects_a_removed_target_even_when_another_controller_retains_its_entity(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-retained-rename");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let first = launch(&ws, cx, &directory, "OLD", 1);
    let old_pid = processes.track(&first, cx);
    rename_form(&ws, cx);
    let pending = ws.update(cx, |ws, _| ws.panel.take().unwrap());
    ws.update_in(cx, |ws, window, cx| ws.close_terminal(&CloseTerminal, window, cx));
    processes.stopped(old_pid);
    let replacement = launch(&ws, cx, &directory, "NEW", 1);
    let replacement_pid = processes.track(&replacement, cx);
    ws.update(cx, |ws, cx| {
        ws.panel = Some(pending);
        cx.notify();
    });
    submit_name(&ws, cx, "closed target");
    ws.read_with(cx, |ws, cx| {
        let Some(Panel::Form(form)) = &ws.panel else { panic!("removed membership must refuse even a live entity") };
        assert_eq!(form.error.as_deref(), Some("This terminal has closed"));
        assert!(first.read(cx).name.is_none());
        assert!(replacement.read(cx).name.is_none());
        assert_eq!(ws.terminal.as_ref(), Some(&replacement));
    });
    ws.update_in(cx, |ws, window, cx| ws.close_terminal(&CloseTerminal, window, cx));
    processes.stopped(replacement_pid);
}

#[gpui::test]
fn pending_rename_keeps_its_live_identity_when_an_earlier_tab_closes_and_active_tab_changes(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-rename-reordered");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let one = launch(&ws, cx, &directory, "ONE", 1);
    let one_pid = processes.track(&one, cx);
    let two = launch(&ws, cx, &directory, "TWO", 1);
    let two_pid = processes.track(&two, cx);
    rename_form(&ws, cx);
    let pending = ws.update(cx, |ws, _| ws.panel.take().unwrap());
    let three = launch(&ws, cx, &directory, "THREE", 1);
    let three_pid = processes.track(&three, cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.close_terminal_tab(&CloseTerminalTab { id: one.entity_id() }, window, cx);
        ws.panel = Some(pending);
    });
    processes.stopped(one_pid);
    submit_name(&ws, cx, "original second");
    ws.read_with(cx, |ws, cx| {
        assert_eq!(two.read(cx).name.as_deref(), Some("original second"));
        assert!(three.read(cx).name.is_none());
        assert_eq!(ws.terminal.as_ref(), Some(&three), "rename must not change the active identity");
    });
    ws.update_in(cx, |ws, window, cx| ws.clear_window(window, cx));
    processes.stopped(two_pid);
    processes.stopped(three_pid);
}

#[gpui::test]
fn tab_click_and_queued_actions_close_only_their_identity_after_indices_change(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-tab-identities");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let one = launch(&ws, cx, &directory, "ONE", 1);
    let one_pid = processes.track(&one, cx);
    let two = launch(&ws, cx, &directory, "TWO", 1);
    let two_pid = processes.track(&two, cx);
    let three = launch(&ws, cx, &directory, "THREE", 1);
    let three_pid = processes.track(&three, cx);
    cx.run_until_parked();
    let queued_select = SelectTerminalTab { id: two.entity_id() };
    let stale_select = SelectTerminalTab { id: one.entity_id() };
    let stale_close = CloseTerminalTab { id: one.entity_id() };
    let close = cx.debug_bounds("terminal-tab-close-0").expect("each tab has a close control");
    ws.read_with(cx, |ws, _| assert_eq!(ws.terminal.as_ref(), Some(&three)));
    assert!(
        close.right() <= cx.debug_bounds("terminal-tab-1").unwrap().left(),
        "a long cwd label must not overlap its neighbour's close control"
    );
    cx.simulate_click(close.center(), gpui::Modifiers::none());
    processes.stopped(one_pid);
    ws.read_with(cx, |ws, _| assert_eq!(ws.terminal.as_ref(), Some(&three), "inactive close keeps active identity"));
    ws.update_in(cx, |ws, window, cx| {
        ws.select_terminal_tab(&queued_select, window, cx);
        ws.select_terminal_tab(&stale_select, window, cx);
        ws.close_terminal_tab(&stale_close, window, cx);
        assert_eq!(ws.terminal.as_ref(), Some(&two));
        assert_eq!(ws.terminal_views.groups[0].tabs, vec![two.clone(), three.clone()]);
    });
    cx.run_until_parked();
    let close = cx.debug_bounds("terminal-tab-close-1").unwrap();
    cx.simulate_click(close.center(), gpui::Modifiers::none());
    processes.stopped(three_pid);
    cx.update(|window, cx| assert!(two.read(cx).focus_handle.is_focused(window)));
    cx.simulate_input("Q");
    wait(&two, cx, "TWO bytes:51");
    ws.update_in(cx, |ws, window, cx| {
        ws.close_terminal(&CloseTerminal, window, cx);
        ws.close_terminal(&CloseTerminal, window, cx);
        assert!(ws.terminal.is_none());
        assert!(ws.panel.is_none());
    });
    processes.stopped(two_pid);
    one.read_with(cx, |t, _| {
        assert!(t.backend.screen().contents().contains("ONE READY"), "close preserves emulator output")
    });
}

#[gpui::test]
fn real_tab_ptys_isolate_exact_input_output_cwd_search_errors_and_active_resize(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-independent-ptys").canonicalize().unwrap();
    let a = directory.join("one");
    let b = directory.join("two");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let one = launch(&ws, cx, &a, "ALPHA", 12);
    let one_pid = processes.track(&one, cx);
    let two = launch(&ws, cx, &b, "BETA", 8);
    let two_pid = processes.track(&two, cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    cx.run_until_parked();
    let tab = cx.debug_bounds("terminal-tab-0").unwrap();
    cx.simulate_click(tab.center(), gpui::Modifiers::none());
    cx.simulate_input("alpha中😀");
    wait(&one, cx, "INPUT-DONE");
    one.read_with(cx, |t, _| {
        assert!(t.backend.screen().contents().contains("ALPHA bytes:616c706861e4b8adf09f9880"));
        assert_eq!(t.cwd, a);
    });
    two.read_with(cx, |t, _| {
        assert!(!t.backend.screen().contents().contains("INPUT-DONE"));
        assert!(!t.backend.screen().contents().contains("616c706861"));
        assert_eq!(t.cwd, b);
    });
    select(&ws, cx, &two);
    ws.update_in(cx, |ws, window, cx| ws.close_panel(&ClosePanel, window, cx));
    two.update(cx, |t, _| t.backend.write("beta😀".as_bytes().to_vec()));
    wait(&two, cx, "INPUT-DONE");
    ws.update_in(cx, |ws, window, cx| {
        assert_eq!(ws.terminal.as_ref(), Some(&two));
        ws.show_terminal(&ShowTerminal, window, cx);
        assert_eq!(ws.terminal.as_ref(), Some(&two));
    });
    cx.run_until_parked();
    two.read_with(cx, |t, _| assert!(t.backend.screen().contents().contains("BETA bytes:62657461f09f9880")));
    one.read_with(cx, |t, _| assert!(!t.backend.screen().contents().contains("62657461f09f9880")));
    let inactive_size = one.read_with(cx, |t, _| t.backend.screen().size());
    let old_active_size = two.read_with(cx, |t, _| t.backend.screen().size());
    cx.simulate_resize(size(px(1120.), px(780.)));
    cx.run_until_parked();
    let active_size = two.read_with(cx, |t, _| t.backend.screen().size());
    assert_ne!(active_size, old_active_size);
    assert_eq!(one.read_with(cx, |t, _| t.backend.screen().size()), inactive_size);
    cx.update(|window, cx| assert!(two.read(cx).focus_handle.is_focused(window)));
    cx.simulate_input("S");
    wait(&two, cx, "SIZE-DONE");
    two.read_with(cx, |t, _| {
        assert!(t.backend.screen().contents().contains(&format!("SIZE:{} {}", active_size.0, active_size.1)))
    });
    one.update(cx, |t, _| t.backend.write(b"S".to_vec()));
    wait(&one, cx, "SIZE-DONE");
    one.read_with(cx, |t, _| {
        assert!(t.backend.screen().contents().contains(&format!("SIZE:{} {}", inactive_size.0, inactive_size.1)))
    });
    for (t, query) in [(&one, "ALPHA bytes"), (&two, "BETA bytes")] {
        select(&ws, cx, t);
        t.update_in(cx, |t, window, cx| t.open_search(&FindOutput, window, cx));
        cx.run_until_parked();
        cx.simulate_input(query);
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            t.read_with(cx, |t, _| t.search.as_ref().and_then(|s| s.results.as_ref()).is_some_and(|r| r.len() == 1))
        });
    }
    one.update(cx, |t, _| {
        let results = t.search.as_mut().unwrap().results.as_mut().unwrap();
        results.set_selection(Some(((1, 0), (1, 5))));
        t.pin_search();
        t.backend.write(vec![b'x'; (1 << 20) + 1]);
        assert!(t.backend.error().unwrap().contains("1 MiB"));
    });
    two.update_in(cx, |t, window, cx| {
        t.close_search(&CloseSearch, window, cx);
        assert!(t.retained_output.is_some());
        assert!(t.backend.error().is_none());
    });
    select(&ws, cx, &one);
    one.read_with(cx, |t, _| {
        assert_eq!(t.display_selection(), Some(((1, 0), (1, 5))));
        assert_eq!(t.search.as_ref().unwrap().results.as_ref().unwrap().len(), 1);
        assert!(!t.display_screen().contents().contains("BETA"));
    });
    rename_form(&ws, cx);
    submit_name(&ws, cx, "reviewed ALPHA");
    one.read_with(cx, |t, _| {
        assert_eq!(t.name.as_deref(), Some("reviewed ALPHA"));
        assert_eq!(t.cwd, a);
        assert_eq!(t.display_selection(), Some(((1, 0), (1, 5))));
        assert_eq!(t.search.as_ref().unwrap().results.as_ref().unwrap().len(), 1);
    });
    two.read_with(cx, |t, _| assert!(t.retained_output.is_some() && t.backend.error().is_none()));
    assert_eq!(unsafe { libc::kill(one_pid, 0) }, 0);
    assert_eq!(unsafe { libc::kill(two_pid, 0) }, 0);
    ws.update_in(cx, |ws, window, cx| ws.clear_window(window, cx));
    processes.stopped(one_pid);
    processes.stopped(two_pid);
}

#[gpui::test]
fn closing_a_tab_stops_its_pty_even_when_the_view_is_retained(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-retained-tab");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let first = launch(&ws, cx, &directory, "ONE", 1);
    let pid = processes.track(&first, cx);
    ws.update_in(cx, |ws, window, cx| ws.close_terminal(&CloseTerminal, window, cx));
    cx.run_until_parked();
    processes.stopped(pid);
    assert!(first.read_with(cx, |t, _| t.backend.pty().is_none()));
}

#[gpui::test]
fn accepted_last_window_close_stops_all_ptys_even_when_the_controller_is_retained(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-last-window");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let first = launch(&ws, cx, &directory, "ONE", 1);
    let first_pid = processes.track(&first, cx);
    let second = launch(&ws, cx, &directory, "TWO", 1);
    let second_pid = processes.track(&second, cx);
    drop((first, second));
    ws.update_in(cx, |ws, window, cx| ws.close_window(&CloseWindow, window, cx));
    cx.run_until_parked();
    assert!(cx.cx.windows().is_empty());
    processes.stopped(first_pid);
    processes.stopped(second_pid);
    ws.read_with(cx, |ws, _| assert!(ws.terminal.is_none() && ws.terminal_views.tabs().next().is_none()));
}

#[gpui::test]
fn native_clean_window_close_stops_only_that_windows_sessions_then_the_last_windows(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-native-close");
    let (first, second, mut a, mut b) = two_windows(cx);
    let mut processes = Processes::default();
    let one = launch(&first, &mut a, &directory, "ONE", 1);
    let one_pid = processes.track(&one, &mut a);
    let two = launch(&second, &mut b, &directory, "TWO", 1);
    let two_pid = processes.track(&two, &mut b);
    assert!(a.simulate_close(), "native clean close is admitted");
    a.update(|window, _| window.remove_window());
    a.run_until_parked();
    processes.stopped(one_pid);
    assert_eq!(unsafe { libc::kill(two_pid, 0) }, 0, "other window remains live");
    two.update(&mut b, |t, _| t.backend.write(b"K".to_vec()));
    wait(&two, &mut b, "TWO bytes:4b");
    assert!(b.simulate_close());
    b.update(|window, _| window.remove_window());
    b.run_until_parked();
    processes.stopped(two_pid);
}

#[gpui::test]
fn cancelled_native_close_keeps_the_pty_running_until_discard_is_accepted(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-cancel-close");
    let (ws, cx) = window(cx);
    ws.update(cx, |ws, cx| ws.active_editor().unwrap().update(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved source"))));
    let mut processes = Processes::default();
    let t = launch(&ws, cx, &directory, "LIVE", 1);
    let pid = processes.track(&t, cx);
    assert!(!cx.simulate_close());
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    t.update(cx, |t, _| t.backend.write(b"X".to_vec()));
    wait(&t, cx, "LIVE bytes:58");
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "unsaved source");
        assert_eq!(ws.terminal.as_ref(), Some(&t));
    });
    assert!(!cx.simulate_close());
    cx.simulate_prompt_answer("Discard All");
    cx.run_until_parked();
    processes.stopped(pid);
}

#[gpui::test]
fn native_close_waiting_for_a_source_save_preserves_the_pty_until_the_write_lands(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-save-close").canonicalize().unwrap();
    let path = directory.join("source.txt");
    std::fs::write(&path, "on disk").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        ws.active_editor().unwrap().update(cx, |e, cx| e.set_text("saved source", cx));
    });
    let mut processes = Processes::default();
    let t = launch(&ws, cx, &directory, "SAVING", 1);
    let pid = processes.track(&t, cx);
    let pause = crate::documents::save::gate::pause(&path);
    ws.update_in(cx, |ws, window, cx| ws.save(&Save, window, cx));
    cx.run_until_parked();
    assert_eq!(pause.arrived(), 1);
    assert!(!cx.simulate_close());
    cx.run_until_parked();
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "pending save must retain the PTY");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "on disk");
    pause.release();
    settle_until(cx, |cx| cx.cx.windows().is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved source");
    processes.stopped(pid);
}

#[gpui::test]
fn application_shutdown_stops_ptys_even_when_the_workspace_and_views_are_retained(cx: &mut TestAppContext) {
    let directory = temp("orlo-t04-app-shutdown");
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let t = launch(&ws, cx, &directory, "QUIT", 1);
    let pid = processes.track(&t, cx);
    cx.update(|_, cx| crate::workspace::save_on_quit(cx));
    cx.cx.update(|cx| cx.shutdown());
    processes.stopped(pid);
    assert!(t.read_with(cx, |t, _| t.backend.pty().is_none()));
}

#[gpui::test]
fn application_shutdown_finishes_private_shell_cleanup_including_a_previously_closed_tab(cx: &mut TestAppContext) {
    let directory = temp("orlo-t06-quit-cleanup").canonicalize().unwrap();
    std::fs::write(
        directory.join(".zshrc"),
        "trap '' HUP\nprint -r -- $ORLO_INTEGRATION_DIR:$$ >> \"$HOME/cleanup-records\"\nPROMPT='cleanup-ready> '\n",
    )
    .unwrap();
    let (ws, cx) = window(cx);
    let mut processes = Processes::default();
    let mut terminals = Vec::new();
    let mut owned = Vec::new();
    for _ in 0..2 {
        let t = launch(&ws, cx, &directory, "REPLACED", 1);
        t.update(cx, |t, _| {
            t.backend.close();
            let mut command =
                orlo_terminal::shell::configured_command(&directory, Some("/bin/zsh"), Some(&["-i".into()]));
            command.env("HOME", &directory);
            command.env("ZDOTDIR", &directory);
            t.backend = TerminalSession::from_command_with_integration(command, Default::default(), true);
        });
        wait(&t, cx, "cleanup-ready>");
        let records = std::fs::read_to_string(directory.join("cleanup-records")).unwrap();
        let line = records.lines().last().unwrap();
        let (path, pid) = line.rsplit_once(':').unwrap();
        let pid = pid.parse::<libc::pid_t>().unwrap();
        assert!(pid > 0);
        processes.0.push(pid);
        let path = std::path::PathBuf::from(path);
        assert!(path.is_dir(), "positive control: startup files must exist while the shell is alive");
        owned.push(path);
        terminals.push(t);
    }
    ws.update_in(cx, |ws, w, cx| {
        ws.close_terminal_tab(&CloseTerminalTab { id: terminals[0].entity_id() }, w, cx);
    });
    cx.update(|_, cx| crate::workspace::save_on_quit(cx));
    cx.cx.update(|cx| cx.shutdown());
    for path in owned {
        assert!(!path.exists(), "quit returned before private shell files were removed: {}", path.display());
    }
    for pid in processes.0.iter().copied() {
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "quit returned before reaping shell {pid}");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
    for terminal in terminals {
        assert!(terminal.read_with(cx, |t, _| t.backend.pty().is_none()));
    }
}
