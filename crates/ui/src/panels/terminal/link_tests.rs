//! Real mouse routing, document navigation and PTY isolation for displayed output links.
use super::*;
use crate::config::settings::Settings;
use crate::workspace::ui_tests::{settle_until, temp, window};
use crate::workspace::{ShowTerminal, Workspace};
use gpui::{TestAppContext, VisualTestContext};

fn terminal_with_script(
    ws: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    script: &str,
    arguments: &[String],
) -> Entity<Terminal> {
    ws.update_in(cx, |ws, w, cx| {
        let settings = &mut cx.global_mut::<Settings>().terminal;
        settings.shell = Some("/bin/sh".into());
        settings.arguments = Some([vec!["-c".into(), script.into(), "orlo-links".into()], arguments.to_vec()].concat());
        ws.show_terminal(&ShowTerminal, w, cx);
        let terminal = ws.terminal.clone().unwrap();
        terminal.update(cx, |t, _| t._poll = None);
        terminal
    })
}

fn wait_output(terminal: &Entity<Terminal>, cx: &mut VisualTestContext, text: &str) {
    settle_until(cx, |cx| {
        terminal.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.screen().contents().contains(text)
        })
    });
    cx.run_until_parked();
}

fn cell(terminal: &Entity<Terminal>, cx: &VisualTestContext, row: u16, column: u16) -> Point<Pixels> {
    terminal.read_with(cx, |t, _| {
        let layout = t.layout.unwrap();
        layout.bounds.origin + point(layout.cell_width * (column as f32 + 0.2), layout.line_height * (row as f32 + 0.5))
    })
}

fn assert_location(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &std::path::Path, cursor: usize) {
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| {
            ws.active_editor().is_some_and(|e| {
                let e = e.read(cx);
                e.doc(cx).path.as_deref() == Some(path) && e.state.cursor() == cursor
            })
        })
    });
}

#[gpui::test]
fn cmd_click_uses_paused_retained_and_command_cwd_after_live_output_changes(cx: &mut TestAppContext) {
    let root = temp("orlo-displayed-link-cwd").canonicalize().unwrap();
    let one = root.join("café one");
    let two = root.join("第二 two");
    // Longer than the actual output grid: clicking a continuation needs its offscreen prefix.
    let name = format!("sub/中 my {}.txt", "x".repeat(220));
    for (dir, contents) in [(&one, "first\n中😀target\n"), (&two, "second\nnew text")] {
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join(&name), contents).unwrap();
    }
    let (ws, cx) = window(cx);
    let terminal = terminal_with_script(
        &ws,
        cx,
        r#"
        stty -echo
        printf 'BOOT\r\n'; IFS= read -r start
        printf '\033[2J\033[H\033]7;file://%s\007\033]133;C\007"%s:2:3"\r\n\033]133;D;0\007' "$1" "$3"
        i=0; while [ "$i" -lt 45 ]; do printf 'filler\r\n'; i=$((i+1)); done
        printf 'READY\r\n'; IFS= read -r go
        printf '\033[2J\033[H\033]7;file://%s\007\033[?1000h\033[?1006h"%s":999:999:\r\nNEW\r\n' "$2" "$3"
        IFS= read -r value
        printf 'INPUT:'; printf '%s' "$value" | od -An -tx1 | tr -d ' \n'; printf '\r\nINPUT-DONE\r\n'
        IFS= read -r value
    "#,
        &[one.display().to_string(), two.display().to_string(), name.clone()],
    );
    cx.simulate_resize(size(px(900.), px(700.)));
    wait_output(&terminal, cx, "BOOT");
    terminal.update(cx, |t, _| t.backend.write(b"start\n".to_vec()));
    wait_output(&terminal, cx, "READY");
    terminal.update_in(cx, |t, w, cx| {
        t.backend.scroll_lines(10_000);
        t.open_search(&FindOutput, w, cx);
    });
    cx.run_until_parked();
    let bounds = terminal.read_with(cx, |t, _| {
        assert!(
            t.display_screen().row_wrapped(0),
            "size={:?} scroll={} text={:?}",
            t.display_screen().size(),
            t.display_screen().scrollback(),
            t.display_screen().contents()
        );
        assert!(t.display_screen().scrollback() > 0);
        t.layout.unwrap().bounds
    });
    // A real plain drag pins Find and must stay local even once the live app requests reports.
    let a = cell(&terminal, cx, 0, 0);
    let b = cell(&terminal, cx, 0, 5);
    cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(b, Some(MouseButton::Left), gpui::Modifiers::none());
    cx.simulate_mouse_up(b, MouseButton::Left, gpui::Modifiers::none());
    terminal.update(cx, |t, _| {
        assert!(t.display_selection().is_some_and(|(a, b)| a != b));
        t.backend.write(b"go\n".to_vec());
    });
    wait_output(&terminal, cx, "NEW");
    let mut opened_document = None;
    for mode in ["paused", "retained", "command"] {
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.select_range(0..0, cx)));
        if mode == "retained" {
            terminal.update_in(cx, |t, w, cx| t.close_search(&CloseSearch, w, cx));
        } else if mode == "command" {
            terminal.update(cx, |t, cx| t.review_command(false, cx));
        }
        cx.run_until_parked();
        terminal.read_with(cx, |t, _| {
            assert_eq!(t.layout.unwrap().bounds, bounds, "{mode}");
            assert_eq!(t.cwd, two);
            assert!(
                t.display_screen().row_wrapped(0),
                "size={:?} scroll={} text={:?}",
                t.display_screen().size(),
                t.display_screen().scrollback(),
                t.display_screen().contents()
            );
        });
        // Click the wrapped continuation, not the easily recognized prefix.
        let link = cell(&terminal, cx, 1, 1);
        cx.simulate_click(link, gpui::Modifiers { platform: true, ..Default::default() });
        assert_location(&ws, cx, &one.join(&name), 8); // row2 col3 counts Unicode source characters.
        ws.read_with(cx, |ws, cx| {
            let document = ws.active_editor().unwrap().read(cx).document.clone();
            if let Some(opened) = &opened_document {
                assert_eq!(&document, opened, "{mode} reuses the existing document");
            }
            opened_document = Some(document);
        });
        terminal.read_with(cx, |t, _| assert!(t.reporting_button.is_none(), "{mode}"));
    }
    terminal.update(cx, |t, cx| t.return_to_live(cx));
    cx.run_until_parked();
    cx.simulate_click(cell(&terminal, cx, 0, 4), gpui::Modifiers { platform: true, ..Default::default() });
    assert_location(&ws, cx, &two.join(&name), 15); // line/column past EOF clamp to end.
    terminal.update_in(cx, |t, w, _| t.focus(w));
    cx.simulate_input("SENTINEL");
    cx.simulate_keystrokes("enter");
    wait_output(&terminal, cx, "INPUT-DONE");
    terminal.read_with(cx, |t, _| {
        assert!(
            t.backend.screen().contents().lines().any(|line| line == "INPUT:53454e54494e454c"),
            "{}",
            t.backend.screen().contents()
        );
    });
}

#[gpui::test]
fn cmd_click_external_urls_missing_files_and_refused_tokens_do_not_write_to_the_pty(cx: &mut TestAppContext) {
    let root = temp("orlo-external-link-targets").canonicalize().unwrap();
    std::fs::write(root.join("Makefile"), "first\n中😀target\n").unwrap();
    let (ws, cx) = window(cx);
    let terminal = terminal_with_script(
        &ws,
        cx,
        r#"
        stty -echo
        printf '\033]7;file://%s\007\033[?1000h\033[?1006hhttps://example.invalid/a?q=one\r\nhttp://example.invalid/two\r\nmissing-file.rs:2:3\r\nMakefile:2:3:\r\njavascript:alert(1)\r\nfile://remote.invalid/tmp/x\r\nfile:///tmp/%%GG\r\nbareword\r\nREADY\r\n' "$1"
        IFS= read -r value
        printf 'INPUT:'; printf '%s' "$value" | od -An -tx1 | tr -d ' \n'; printf '\r\nINPUT-DONE\r\n'
        IFS= read -r value
    "#,
        &[root.display().to_string()],
    );
    wait_output(&terminal, cx, "READY");
    let before =
        ws.read_with(cx, |ws, cx| (ws.active_editor().unwrap().read(cx).document.clone(), ws.views.tabs().count()));
    // Shift-drag over a real URL selects rather than opening or forwarding reports.
    let a = cell(&terminal, cx, 0, 0);
    let b = cell(&terminal, cx, 0, 5);
    cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers { shift: true, ..Default::default() });
    cx.simulate_mouse_move(b, Some(MouseButton::Left), gpui::Modifiers::none());
    cx.simulate_mouse_up(b, MouseButton::Left, gpui::Modifiers::none());
    assert!(cx.opened_url().is_none());
    terminal.read_with(cx, |t, _| assert_eq!(t.backend.selected_text().as_deref(), Some("https")));
    // Refused schemes, malformed/remote file URLs and prose have no external side effect.
    for row in 4..=7 {
        cx.simulate_click(cell(&terminal, cx, row, 2), gpui::Modifiers { platform: true, ..Default::default() });
        assert!(cx.opened_url().is_none());
    }
    cx.simulate_click(cell(&terminal, cx, 2, 2), gpui::Modifiers { platform: true, ..Default::default() });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.status.as_ref().unwrap().0.contains("Terminal file not found"));
        assert_eq!(ws.active_editor().unwrap().read(cx).document, before.0);
        assert_eq!(ws.views.tabs().count(), before.1);
        assert!(!root.join("missing-file.rs").exists());
    });
    cx.simulate_click(cell(&terminal, cx, 3, 2), gpui::Modifiers { platform: true, ..Default::default() });
    assert_location(&ws, cx, &root.join("Makefile"), 8);
    for (row, url) in [(0, "https://example.invalid/a?q=one"), (1, "http://example.invalid/two")] {
        cx.simulate_click(cell(&terminal, cx, row, 2), gpui::Modifiers { platform: true, ..Default::default() });
        assert_eq!(cx.opened_url().as_deref(), Some(url));
    }
    terminal.update_in(cx, |t, w, _| t.focus(w));
    cx.simulate_input("SENTINEL");
    cx.simulate_keystrokes("enter");
    wait_output(&terminal, cx, "INPUT-DONE");
    terminal.read_with(cx, |t, _| {
        assert!(
            t.backend.screen().contents().lines().any(|line| line == "INPUT:53454e54494e454c"),
            "{}",
            t.backend.screen().contents()
        );
        assert!(t.reporting_button.is_none());
    });
}
