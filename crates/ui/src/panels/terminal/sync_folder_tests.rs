//! N39 routes explicit directory sync through the existing workspace and integrated PTY.
use super::*;
use crate::config::settings::Settings;
use crate::workspace::ui_tests::{settle_until, temp, window};
use crate::workspace::{NewTerminal, ShowTerminal, SyncTerminalFolder};
use gpui::TestAppContext;

fn ready(terminal: &Entity<Terminal>, cx: &mut gpui::VisualTestContext) {
    settle_until(cx, |cx| {
        terminal.update(cx, |t, cx| {
            t.drain(cx);
            t.can_sync_folder()
        })
    });
}

#[gpui::test]
fn n39_sync_uses_file_parent_then_project_fallback_only_for_the_selected_terminal(cx: &mut TestAppContext) {
    let root = temp("n39 project '雪").canonicalize().unwrap();
    let file_dir = root.join("file folder 'quoted'");
    std::fs::create_dir(&file_dir).unwrap();
    let home = root.join("shell home");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".zshrc"), "PROMPT='ui-n39> '\n").unwrap();
    let (ws, cx) = window(cx);
    let (first, second) = ws.update_in(cx, |ws, w, cx| {
        let settings = &mut cx.global_mut::<Settings>().terminal;
        settings.shell = Some("/bin/sh".into());
        settings.arguments = Some(vec!["-c".into(), "exec sleep 60".into()]);
        settings.shell_integration = false;
        // Use isolated HOME/ZDOTDIR without changing process-global test environment.
        ws.show_terminal(&ShowTerminal, w, cx);
        let first = ws.terminal.clone().unwrap();
        ws.new_terminal(&NewTerminal, w, cx);
        let second = ws.terminal.clone().unwrap();
        for terminal in [&first, &second] {
            terminal.update(cx, |t, _| {
                t._poll = None;
                t.backend.close();
                let mut command =
                    orlo_terminal::shell::configured_command(&home, Some("/bin/zsh"), Some(&["-i".into()]));
                command.env("HOME", &home);
                command.env("ZDOTDIR", &home);
                t.backend = TerminalSession::from_command_with_integration(command, Default::default(), true);
            });
        }
        ws.roots = vec![crate::config::project::Root::new(root.clone())];
        ws.active_editor().unwrap().update(cx, |e, cx| {
            e.document.update(cx, |doc, _| doc.model.path = Some(file_dir.join("active.rs")));
        });
        (first, second)
    });
    ready(&first, cx);
    ready(&second, cx);
    first.read_with(cx, |t, _| assert_eq!(t.cwd, home));
    ws.update_in(cx, |ws, w, cx| {
        ws.sync_terminal_folder(&SyncTerminalFolder, w, cx);
        assert!(second.read(cx).backend.folder_sync_pending());
        assert_eq!(second.read(cx).cwd, home);
    });
    ready(&second, cx);
    second.read_with(cx, |t, _| assert_eq!(t.cwd, file_dir));
    first.read_with(cx, |t, _| assert_eq!(t.cwd, home));
    ws.update_in(cx, |ws, w, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.document.update(cx, |doc, _| doc.model.path = None));
        assert_eq!(second.read(cx).cwd, file_dir, "document changes never sync automatically");
        ws.sync_terminal_folder(&SyncTerminalFolder, w, cx);
    });
    ready(&second, cx);
    second.read_with(cx, |t, _| assert_eq!(t.cwd, root));
    ws.update_in(cx, |ws, w, cx| {
        second.update(cx, |t, cx| t.write(b"print -r -- USER-TEXT".to_vec(), cx));
        ws.sync_terminal_folder(&SyncTerminalFolder, w, cx);
        assert!(ws.status.as_ref().unwrap().0.contains("fresh empty prompt"));
        second.update(cx, |t, cx| t.write(b"\r".to_vec(), cx));
    });
    ready(&second, cx);
    second.read_with(cx, |t, _| assert!(t.backend.commands().records().last().unwrap().output.contains("USER-TEXT")));
    ws.update_in(cx, |ws, w, cx| {
        second.update(cx, |t, _| t.marked = "composition".into());
        ws.sync_terminal_folder(&SyncTerminalFolder, w, cx);
        assert!(ws.status.as_ref().unwrap().0.contains("composition"));
        second.update(cx, |t, _| t.marked.clear());
        ws.roots.clear();
        ws.sync_terminal_folder(&SyncTerminalFolder, w, cx);
        assert!(ws.status.as_ref().unwrap().0.contains("file or project"));
        ws.close_terminals(cx);
        ws.sync_terminal_folder(&SyncTerminalFolder, w, cx);
        assert!(ws.status.as_ref().unwrap().0.contains("Open a terminal"));
        assert!(ws.terminal.is_none());
    });
}

#[gpui::test]
fn n39_sync_folder_action_is_in_tools_palette_and_json_catalog(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    ws.read_with(cx, |ws, cx| {
        let tools = ws.menus(cx).into_iter().find(|menu| menu.name == "Terminal").unwrap();
        assert!(tools.items.iter().any(|item| matches!(item, gpui::MenuItem::Action { name, action, .. }
            if name == "Sync Terminal Folder" && action.name() == "terminal::SyncTerminalFolder")));
        assert!(cx.build_action("terminal::SyncTerminalFolder", None).is_ok());
    });
    let item = crate::chrome::quick_panel::command_items()
        .into_iter()
        .find(|item| item.label == "Terminal: Sync Folder")
        .unwrap();
    let crate::chrome::quick_panel::ItemAction::Dispatch(action) = item.action else { panic!("dispatch") };
    assert_eq!(action.name(), "terminal::SyncTerminalFolder");
}
