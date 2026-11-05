//! Real terminal mouse listeners and disposable mouse-aware PTY applications.
use super::*;
use crate::workspace::ui_tests::settle_until;
use gpui::{TestAppContext, VisualTestContext};
use orlo_terminal::{CommandBuilder, PtyCommand, TerminalConfiguration};

fn terminal<'a>(cx: &'a mut TestAppContext, script: &str) -> (Entity<Terminal>, &'a mut VisualTestContext) {
    cx.add_window_view(|window, cx| {
        let mut t = Terminal::new(std::env::temp_dir(), cx);
        t._poll = None;
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", script]);
        command.cwd(std::env::temp_dir());
        t.backend = TerminalSession::from_command(command, TerminalConfiguration::default());
        t.focus(window);
        window.activate_window();
        t
    })
}

fn wait(t: &Entity<Terminal>, cx: &mut VisualTestContext, text: &str) {
    settle_until(cx, |cx| {
        t.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.screen().contents().contains(text)
        })
    });
    cx.run_until_parked();
}

fn cell(t: &Entity<Terminal>, cx: &VisualTestContext, row: f32, col: f32) -> Point<Pixels> {
    t.read_with(cx, |t, _| {
        let l = t.layout.unwrap();
        l.bounds.origin + point(l.cell_width * col, l.line_height * row)
    })
}

#[gpui::test]
fn exited_mouse_application_clears_reporting_drag_and_preserves_final_selection_and_geometry(cx: &mut TestAppContext) {
    let (t, cx) = terminal(cx, "stty raw -echo; printf '\\033[?1002h\\033[?1006hREADY'; dd bs=1 count=9 >/dev/null 2>&1; printf '\\r\\nSEEN'; dd bs=1 count=1 >/dev/null 2>&1; exit 7");
    wait(&t, cx, "READY");
    let p = cell(&t, cx, 1.8, 2.7);
    cx.simulate_mouse_down(p, MouseButton::Left, gpui::Modifiers::default());
    wait(&t, cx, "SEEN");
    let (screen, bounds, cwd) = t.update(cx, |t, _| {
        assert_eq!(t.reporting_button, Some(MouseButton::Left));
        t.backend.set_selection(Some(((0, 0), (0, 5))));
        let state = (t.backend.screen().clone(), t.layout.unwrap().bounds, t.cwd.clone());
        // Send a fixture handshake without clearing the selection being tested.
        t.backend.pty().unwrap().send(PtyCommand::Input(vec![b'x'])).unwrap();
        state
    });
    settle_until(cx, |cx| {
        t.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.exited() == Some(7)
        })
    });
    cx.run_until_parked();
    t.read_with(cx, |t, _| {
        assert!(t.reporting_button.is_none());
        assert!(t.last_mouse.is_none());
        assert!(!t.dragging);
        assert!(!mouse::requested(t.backend.screen()));
        assert!(t.backend.screen().same_contents(&screen));
        assert_eq!(t.backend.screen().contents(), screen.contents());
        assert_eq!(t.backend.selected_text().as_deref(), Some("READY"));
        assert_eq!(t.layout.unwrap().bounds, bounds);
        assert_eq!(t.cwd, cwd);
        assert!(t.backend.error().is_none());
    });
    cx.simulate_mouse_up(p, MouseButton::Left, gpui::Modifiers::default());
    t.read_with(cx, |t, _| {
        assert!(t.backend.error().is_none());
        assert_eq!(t.backend.selected_text().as_deref(), Some("READY"));
    });
}

#[gpui::test]
fn application_exit_keeps_an_in_progress_shift_selection_until_mouse_up(cx: &mut TestAppContext) {
    let (t, cx) =
        terminal(cx, "stty raw -echo; printf '\\033[?1002h\\033[?1006hREADY'; dd bs=1 count=1 >/dev/null 2>&1; exit 7");
    wait(&t, cx, "READY");
    let a = cell(&t, cx, 0.5, 0.1);
    let middle = cell(&t, cx, 0.5, 2.0);
    let end = cell(&t, cx, 0.5, 5.0);
    cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers { shift: true, ..Default::default() });
    cx.simulate_mouse_move(middle, Some(MouseButton::Left), gpui::Modifiers::default());
    let (before, bounds) = t.update(cx, |t, _| {
        assert_eq!(t.backend.selected_text().as_deref(), Some("RE"));
        assert!(t.dragging);
        assert!(t.reporting_button.is_none());
        t.backend.pty().unwrap().send(PtyCommand::Input(vec![b'x'])).unwrap();
        (t.backend.screen().clone(), t.layout.unwrap().bounds)
    });
    settle_until(cx, |cx| {
        t.update(cx, |t, cx| {
            t.drain(cx);
            t.backend.exited() == Some(7)
        })
    });
    cx.run_until_parked();
    t.read_with(cx, |t, _| {
        assert!(t.dragging, "exit must preserve the local Shift gesture");
        assert!(t.backend.screen().same_contents(&before));
        assert_eq!(t.layout.unwrap().bounds, bounds);
    });
    cx.simulate_mouse_move(end, Some(MouseButton::Left), gpui::Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::default());
    t.update_in(cx, |t, window, cx| {
        assert!(!t.dragging);
        assert_eq!(t.backend.selected_text().as_deref(), Some("READY"));
        t.copy(&crate::editor::Copy, window, cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("READY"));
        assert!(t.backend.error().is_none());
    });
}

#[gpui::test]
fn mouse_application_receives_exact_drag_wheel_blur_bytes_while_shift_selects(cx: &mut TestAppContext) {
    // Independent literal bytes: floor cells, clamp outside release, two wheel ticks,
    // modifiers, middle button and one blur release. SENTINEL detects extra local input.
    let expected = b"\x1b[<0;3;2M\x1b[<32;5;3M\x1b[<0;1;1m\x1b[<88;3;2M\x1b[<88;3;2M\x1b[<65;3;2M\x1b[<25;3;2M\x1b[<1;3;2mSENTINEL";
    let script = format!("stty raw -echo; printf '\\033[?1049h\\033[?1002h\\033[?1006hREADY'; dd bs=1 count={} 2>/dev/null | od -An -tx1 | tr -d ' \\n'; printf '\\r\\nDONE'; dd bs=1 count=1 >/dev/null 2>&1", expected.len());
    let (t, cx) = terminal(cx, &script);
    wait(&t, cx, "READY");
    let a = cell(&t, cx, 0.5, 0.1);
    let b = cell(&t, cx, 0.5, 5.0);
    cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers { shift: true, ..Default::default() });
    cx.simulate_mouse_move(b, Some(MouseButton::Left), gpui::Modifiers::default());
    cx.simulate_mouse_up(b, MouseButton::Left, gpui::Modifiers::default());
    t.read_with(cx, |t, _| {
        assert!(t.backend.screen().alternate_screen());
        assert_eq!(t.backend.selected_text().as_deref(), Some("READY"));
        assert!(t.reporting_button.is_none());
    });
    let p = cell(&t, cx, 1.8, 2.7);
    cx.simulate_mouse_down(p, MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(cell(&t, cx, 1.1, 2.1), Some(MouseButton::Left), gpui::Modifiers::default());
    cx.simulate_mouse_move(cell(&t, cx, 2.8, 4.7), Some(MouseButton::Left), gpui::Modifiers::default());
    let outside = cell(&t, cx, -0.5, -0.5);
    cx.simulate_mouse_up(outside, MouseButton::Left, gpui::Modifiers::default());
    for (dy, modifiers) in [
        (30., gpui::Modifiers { alt: true, control: true, ..Default::default() }),
        (-15., gpui::Modifiers::default()),
        (15., gpui::Modifiers { shift: true, ..Default::default() }),
    ] {
        cx.simulate_event(ScrollWheelEvent {
            position: p,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(dy))),
            modifiers,
            touch_phase: gpui::TouchPhase::Moved,
        });
    }
    cx.simulate_mouse_down(p, MouseButton::Middle, gpui::Modifiers { alt: true, control: true, ..Default::default() });
    cx.update(|window, cx| {
        window.draw(cx).clear();
        window.blur();
        window.draw(cx).clear();
    });
    settle_until(cx, |cx| t.read_with(cx, |t, _| t.reporting_button.is_none()));
    cx.run_until_parked();
    t.read_with(cx, |t, _| {
        assert!(t.reporting_button.is_none());
        assert!(t.last_mouse.is_none());
    });
    cx.simulate_mouse_move(cell(&t, cx, 3.5, 6.5), None, gpui::Modifiers::default());
    cx.simulate_mouse_up(p, MouseButton::Middle, gpui::Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position: p,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(15.))),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    t.update(cx, |t, _| t.backend.write(b"SENTINEL".to_vec()));
    wait(&t, cx, "DONE");
    let hex: String = expected.iter().map(|byte| format!("{byte:02x}")).collect();
    t.read_with(cx, |t, _| {
        let output = t.backend.screen().contents().split_whitespace().collect::<String>();
        assert_eq!(output, format!("READY{hex}DONE"));
        assert!(t.backend.error().is_none());
    });
}

#[gpui::test]
fn terminal_reset_restores_plain_selection_and_scrolling_without_mouse_input(cx: &mut TestAppContext) {
    let (t, cx) = terminal(cx, "stty raw -echo; printf '\\033[?1003h\\033[?1006hREADY'; dd bs=1 count=1 >/dev/null 2>&1; printf '\\033c'; i=0; while [ \"$i\" -lt 100 ]; do printf 'output%03d\\r\\n' \"$i\"; i=$((i+1)); done; printf 'RESET-DONE'; dd bs=1 count=8 2>/dev/null | od -An -tx1 | tr -d ' \\n'; printf '\\r\\nINPUT-DONE'; dd bs=1 count=1 >/dev/null 2>&1");
    wait(&t, cx, "READY");
    t.update(cx, |t, _| t.backend.write(vec![b'x']));
    wait(&t, cx, "RESET-DONE");
    t.read_with(cx, |t, _| assert!(!mouse::requested(t.backend.screen())));
    let p = cell(&t, cx, 1.5, 2.5);
    cx.simulate_event(ScrollWheelEvent {
        position: p,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(30.))),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    t.read_with(cx, |t, _| assert_eq!(t.backend.screen().scrollback(), 2));
    let start = cell(&t, cx, 0.5, 0.1);
    let end = cell(&t, cx, 0.5, 6.0);
    cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), gpui::Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::default());
    t.read_with(cx, |t, _| {
        assert_eq!(t.backend.selected_text().as_deref(), Some("output"));
        assert!(t.reporting_button.is_none());
    });
    t.update(cx, |t, _| t.backend.write(b"SENTINEL".to_vec()));
    wait(&t, cx, "INPUT-DONE");
    t.read_with(cx, |t, _| {
        assert!(t.backend.screen().contents().contains("RESET-DONE53454e54494e454c"));
        assert!(t.backend.error().is_none());
    });
}

#[gpui::test]
fn a_fast_wheel_flick_reports_every_step_as_one_input(cx: &mut TestAppContext) {
    // 30 SGR wheel reports of 9 bytes each (`ESC [ < 6 4 ; 3 ; 2 M`), then DONE
    let (t, cx) = terminal(cx, "stty raw -echo; printf '\\033[?1000h\\033[?1006hREADY'; dd bs=270 count=1 >/dev/null 2>&1; printf DONE; sleep 5");
    wait(&t, cx, "READY");
    cx.simulate_event(ScrollWheelEvent {
        position: cell(&t, cx, 1.5, 2.5),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(450.))),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    t.read_with(cx, |t, _| assert!(t.backend.error().is_none(), "{:?}", t.backend.error()));
    wait(&t, cx, "DONE");
}
