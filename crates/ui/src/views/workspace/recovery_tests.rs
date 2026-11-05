//! Recovery/project boundaries use the real native marked-text handler.
use super::ui_tests::{temp, two_windows};
use super::*;
use crate::editing::text::Selection;
use gpui::{EntityInputHandler, TestAppContext};

#[gpui::test]
fn recovery_snapshot_excludes_native_preview_without_changing_live_commit_cancel_or_transfer(cx: &mut TestAppContext) {
    let original = "e\u{301}X\n😀YZ";
    use crate::config::settings::VirtualSpace;
    for (virtual_space, policy) in
        [(false, VirtualSpace::Off), (true, VirtualSpace::All), (true, VirtualSpace::Rectangular)]
    {
        let path = temp(&format!("recovery-native-{virtual_space}-{policy:?}")).join("source.txt");
        std::fs::write(&path, original).unwrap();
        let (owner, peer, mut a, mut b) = two_windows(cx);
        owner.update(&mut a, |_, cx| cx.global_mut::<Settings>().virtual_space = policy);
        owner.update_in(&mut a, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        peer.update_in(&mut b, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let editor = owner.read_with(&a, |ws, _| ws.active_editor().unwrap());
        let (selections, preview, marked, version, live_folds) = editor.update_in(&mut a, |e, w, cx| {
            e.perform_action(&crate::editor::ToggleOvertype, w, cx);
            e.set_virtual_space(policy, cx);
            let selections = if virtual_space {
                vec![Selection::new(0, 2), Selection::from_points((7, 3), (7, 5))]
            } else {
                vec![Selection::caret(0), Selection::caret(4)]
            };
            let selections = selections
                .into_iter()
                .map(|sel| if policy == VirtualSpace::Rectangular { sel.in_rectangle() } else { sel })
                .collect::<Vec<_>>();
            e.sel(cx).set_selections(selections.clone());
            e.fold_chars_now(std::iter::once(5..7), cx);
            assert!(e.capture_folds(cx).is_some());
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
            (selections, e.text(cx), e.marked.clone(), e.doc(cx).version, e.folds().chars())
        });
        assert!(!live_folds.is_empty(), "the live preview still has a fold to exclude from recovery");
        b.run_until_parked();
        cx.update(sessions::write_autosave);
        ui_tests::settle_until(&mut a, |cx| cx.read(|cx| !cx.global::<sessions::Autosave>().writing));
        let periodic = session::load(&session::path());
        assert!(
            periodic
                .windows
                .iter()
                .flat_map(|w| &w.groups)
                .flat_map(|g| &g.views)
                .filter(|v| v.path == crate::editing::text::resolve(&path))
                .all(|v| v.folds.is_none()),
            "the recovered rope excludes native preview, so live-rope fold fingerprints must be omitted"
        );
        let recovered = periodic
            .windows
            .iter()
            .flat_map(|w| &w.groups)
            .flat_map(|g| &g.views)
            .filter(|v| v.path == crate::editing::text::resolve(&path))
            .find_map(|v| v.unsaved.as_ref())
            .unwrap();
        assert_eq!(recovered.text, original);
        peer.update_in(&mut b, |ws, w, cx| {
            let mut texts = Texts::new();
            let captured = ws.capture(w, true, &mut texts, cx);
            assert!(captured.groups.iter().flat_map(|g| &g.views).all(|v| v.folds.is_none()));
            assert_eq!(texts.iter().find(|(_, text)| text.to_string().contains("YZ")).unwrap().1.to_string(), original);
            let saved = temp(&format!("recovery-roundtrip-{virtual_space}-{policy:?}")).join("session.json");
            session::save(
                &saved,
                &Session { windows: vec![captured], ..Default::default() },
                &texts,
                &mut HashSet::new(),
            )
            .unwrap();
            let restored = session::load(&saved);
            let u = restored.windows[0].groups.iter().flat_map(|g| &g.views).find_map(|v| v.unsaved.as_ref()).unwrap();
            assert_eq!(u.state(&path).unwrap().text(), original);
        });
        // Saving a project keeps the same views and must retain live native composition.
        owner.update_in(&mut a, |ws, w, cx| {
            ws.write_project_as(
                temp(&format!("recovery-save-project-{virtual_space}-{policy:?}")).join("saved.sublime-project"),
                w,
                cx,
            )
        });
        editor.read_with(&a, |e, cx| {
            assert_eq!(
                (e.text(cx), e.marked.clone(), e.doc(cx).version, e.folds().chars()),
                (preview.clone(), marked.clone(), version, live_folds.clone())
            );
        });
        // Move-only removal/attach must preserve the same entity and native composition.
        owner.update(&mut a, |ws, cx| ws.remove_view(&editor, cx));
        peer.update_in(&mut b, |ws, w, cx| ws.attach(editor.clone(), false, w, cx));
        editor.update_in(&mut b, |e, w, cx| {
            assert_eq!((e.text(cx), e.marked.clone(), e.folds().chars()), (preview, marked, live_folds));
            e.replace_text_in_range(None, "", w, cx);
            assert_eq!(e.text(cx), original);
            assert_eq!(e.state.selections, selections);
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
            e.unmark_text(w, cx);
            let committed = if virtual_space { "界X\n😀YZ   界" } else { "界X\n界YZ" };
            assert_eq!(e.text(cx), committed);
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), original);
            e.edit(cx, |s| s.redo());
            assert_eq!(e.text(cx), committed);
        });
    }
}

#[gpui::test]
fn recovery_project_close_switch_and_hot_quit_settle_native_composition_for_shared_peers(cx: &mut TestAppContext) {
    for route in ["close", "switch", "quit"] {
        let root = temp(&format!("recovery-lifecycle-{route}"));
        let project = root.join("one.sublime-project");
        let next = root.join("two.sublime-project");
        let file = root.join("source.txt");
        std::fs::write(&project, "{}").unwrap();
        std::fs::write(&next, "{}").unwrap();
        std::fs::write(&file, "e\u{301}X\n😀YZ").unwrap();
        let (owner, peer, mut a, mut b) = two_windows(cx);
        owner.update_in(&mut a, |ws, w, cx| {
            ws.try_open_project_path(&OpenProjectPath { path: project.clone(), here: true }, w, cx).unwrap();
            ws.open_path(file.clone(), w, cx);
        });
        peer.update_in(&mut b, |ws, w, cx| ws.open_path(file.clone(), w, cx));
        let editor = owner.read_with(&a, |ws, _| ws.active_editor().unwrap());
        editor.update_in(&mut a, |e, w, cx| {
            e.perform_action(&crate::editor::ToggleOvertype, w, cx);
            e.sel(cx).set_selections(vec![Selection::caret(0), Selection::caret(4)]);
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
        });
        if route == "quit" {
            cx.update(|cx| sessions::save_for_quit(cx).unwrap());
        } else {
            owner.update_in(&mut a, |ws, w, cx| {
                if route == "close" {
                    ws.close_project(&CloseProject, w, cx);
                } else {
                    ws.try_open_project_path(&OpenProjectPath { path: next, here: true }, w, cx).unwrap();
                }
            });
        }
        b.run_until_parked();
        peer.read_with(&b, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "界X\n界YZ"));
        editor.update(&mut b, |e, cx| {
            assert!(e.marked.is_empty());
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "e\u{301}X\n😀YZ");
            e.edit(cx, |s| s.redo());
            assert_eq!(e.text(cx), "界X\n界YZ");
        });
        let workspace = root.join("one.orlo-workspace");
        let saved = std::fs::read_to_string(workspace).unwrap();
        assert!(saved.contains("界X"));
        assert!(!saved.contains("界e"));
    }
}
