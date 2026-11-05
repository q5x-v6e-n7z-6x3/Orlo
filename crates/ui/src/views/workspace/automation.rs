//! User-invoked packages, shell filters, tasks, shell worksheets and a JSON API console.

use super::*;
use crate::config::automation::{self as engine, FilterSnapshot, Process, TaskDefinition};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

actions!(
    workspace,
    [
        RunTask,
        EditTasks,
        EditProjectTasks,
        FilterThroughShell,
        KeepSelectionsThroughShell,
        RunShellCommand,
        NewShellWorksheet,
        RunWorksheet,
        DeveloperConsole,
        ManagePackages,
        InstallPackage,
        CancelAutomation,
    ]
);

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct RunPackageCommand {
    pub package: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{text, type_in, window};
    use super::*;
    use crate::editing::text::Selection;
    use gpui::EntityInputHandler;
    use gpui::TestAppContext;

    #[gpui::test]
    fn a_task_the_folder_defines_runs_once_the_folder_is_trusted(cx: &mut TestAppContext) {
        let dir = crate::workspace::ui_tests::temp(&format!("orlo-trust-task-{}", std::process::id()));
        let dir = dir.canonicalize().unwrap();
        orlo_utils::trust::TRUST_ALL.set(false);
        let (ws, cx) = window(cx);
        let task = TaskDefinition { name: "Build".into(), shell: "true".into(), cwd: None, timeout_secs: None };
        let action = RunConfiguredTask { task, base: dir.clone(), package: None, project: true };
        let run = |ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
            let action = action.clone();
            ws.update_in(cx, |ws, w, cx| ws.run_configured_task(&action, w, cx));
        };
        run(&ws, cx);
        assert!(cx.has_pending_prompt(), "asked first");
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.automation.running.is_none(), "cancelled: nothing ran"));
        assert!(!orlo_utils::trust::trusted(&dir));
        run(&ws, cx);
        cx.simulate_prompt_answer("Trust Folder and Run");
        cx.run_until_parked();
        assert!(orlo_utils::trust::trusted(&dir));
        orlo_utils::trust::TRUST_ALL.set(true);
    }

    fn write_package(name: &str, manifest: &str) {
        let path = packages_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("package.json"), manifest).unwrap();
    }

    fn start_filter(
        ws: &mut Workspace,
        editor: &Entity<Editor>,
        shell: &str,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let snapshot = ws.filter_snapshot(editor, false, cx).unwrap();
        let process = ws.automation_process(shell.into(), String::new(), cx);
        let destination = Destination::Filter(editor.downgrade(), snapshot, false);
        ws.start_automation(process, destination, window, cx).unwrap();
    }

    fn submit_console(ws: &mut Workspace, input: &str, window: &mut Window, cx: &mut Context<Workspace>) {
        let Some(Panel::Form(form)) = &ws.panel else {
            panic!("console input missing");
        };
        let field = form.inputs().next().unwrap();
        field.update(cx, |e, cx| e.set_text(input, cx));
        ws.submit_form(&super::super::form::Submit, window, cx);
    }

    #[gpui::test]
    fn package_commands_are_searchable_by_their_visible_name(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        write_package(
            "PalettePackage",
            r#"{"name":"PalettePackage","commands":[{"name":"Uppercase","shell":"cat","filter":true}]}"#,
        );
        ws.update_in(cx, |ws, window, cx| {
            ws.command_palette(&CommandPalette, window, cx);
            let panel = ws.quick_panel.as_mut().unwrap();
            panel.refilter("PalettePackage Uppercase");
            assert_eq!(panel.filtered.len(), 1);
            assert_eq!(panel.items[panel.filtered[0]].label, "PalettePackage: Uppercase");
        });
    }

    #[gpui::test]
    fn successful_filter_is_one_undo_and_a_stale_filter_never_edits(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "aéz");
        let target = ws.update_in(cx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, _| e.state.selections = vec![Selection::new(1, 2)]);
            start_filter(ws, &editor, "printf ZZ", window, cx);
            editor
        });
        cx.run_until_parked();
        assert_eq!(text(&ws, cx), "aZZz");
        target.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(text(&ws, cx), "aéz");
        ws.update_in(cx, |ws, window, cx| {
            start_filter(ws, &target, "printf BAD", window, cx);
            target.update(cx, |e, cx| e.edit(cx, |s| s.insert("changed")));
        });
        let changed = target.read_with(cx, |e, cx| e.text(cx));
        cx.run_until_parked();
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), changed);
        assert!(text(&ws, cx).contains("Filter discarded"));
    }

    #[gpui::test]
    fn n38_multirange_filter_orders_unicode_outputs_and_undoes_atomically(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "é / 界 / fin");
        let target = ws.update_in(cx, |ws, window, cx| {
            let target = ws.active_editor().unwrap();
            target.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)]));
            start_filter(ws, &target, "printf '<'; cat; printf '>'", window, cx);
            target
        });
        cx.run_until_parked();
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "<é> / <界> / fin");
        target.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "é / 界 / fin");
        target.update(cx, |e, cx| e.edit(cx, |s| s.redo()));
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "<é> / <界> / fin");
    }

    #[gpui::test]
    fn n38_predicate_form_keeps_direction_and_selection_history_without_text_edits(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "é / 界 / z");
        let original = vec![Selection::new(1, 0), Selection::new(4, 5), Selection::new(8, 9)];
        let target = ws.update_in(cx, |ws, window, cx| {
            let target = ws.active_editor().unwrap();
            target.update(cx, |e, cx| e.sel(cx).set_selections(original.clone()));
            ws.execute_command(
                &crate::keybindings::Command {
                    command: "workspace::KeepSelectionsThroughShell".into(),
                    args: serde_json::json!({}),
                },
                window,
                cx,
            )
            .unwrap();
            submit_console(ws, "value=$(cat); printf '\\377'; [ \"$value\" != '界' ]", window, cx);
            target
        });
        let before = target.read_with(cx, |e, cx| (e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved()));
        cx.run_until_parked();
        target.update(cx, |e, cx| {
            assert_eq!(e.text(cx), "é / 界 / z");
            assert_eq!(e.state.selections, vec![Selection::new(1, 0), Selection::new(8, 9)]);
            assert_eq!(e.state.primary(), 1);
            assert!((e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved()) == before);
            e.edit(cx, |s| assert!(s.undo_selection()));
            assert_eq!(e.state.selections, original);
            e.edit(cx, |s| assert!(s.redo_selection()));
            assert_eq!(e.state.selections.len(), 2);
        });
        ws.update_in(cx, |ws, window, cx| {
            let snapshot = ws.filter_snapshot(&target, true, cx).unwrap();
            let process = ws.automation_process("exit 1".into(), String::new(), cx);
            ws.start_automation(process, Destination::Filter(target.downgrade(), snapshot, true), window, cx).unwrap();
        });
        cx.run_until_parked();
        target.update(cx, |e, cx| {
            assert_eq!(e.text(cx), "é / 界 / z");
            assert_eq!(e.state.selections, vec![Selection::caret(9)]);
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "", "predicates add no text Undo step");
        });
    }

    #[gpui::test]
    fn n38_batch_failure_and_invalid_utf8_never_partially_replace(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "é / 界 / z");
        let target = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        for (shell, predicate, expected) in [
            (
                "value=$(cat); if [ \"$value\" = '界' ]; then printf problem >&2; exit 7; fi; printf CHANGED",
                false,
                "status 7",
            ),
            (
                "value=$(cat); if [ \"$value\" = '界' ]; then printf '\\377'; else printf CHANGED; fi",
                false,
                "not UTF-8",
            ),
            ("value=$(cat); if [ \"$value\" = '界' ]; then printf problem >&2; exit 2; fi; exit 1", true, "status 2"),
        ] {
            ws.update_in(cx, |ws, window, cx| {
                ws.select_editor(&target);
                target.update(cx, |e, cx| {
                    e.sel(cx).set_selections(vec![Selection::new(1, 0), Selection::new(4, 5), Selection::new(8, 9)])
                });
                let snapshot = ws.filter_snapshot(&target, predicate, cx).unwrap();
                let process = ws.automation_process(shell.into(), String::new(), cx);
                ws.start_automation(process, Destination::Filter(target.downgrade(), snapshot, predicate), window, cx)
                    .unwrap();
            });
            let before =
                target.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.doc(cx).version));
            cx.run_until_parked();
            assert!(
                target.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.doc(cx).version))
                    == before
            );
            assert!(text(&ws, cx).contains(expected), "{}", text(&ws, cx));
            if shell.contains("problem") {
                assert!(text(&ws, cx).contains("problem"));
            }
        }
    }

    #[gpui::test]
    fn n38_cancel_and_stale_selection_replies_leave_source_unchanged(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "é / 界");
        let target = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        for cancel in [true, false] {
            ws.update_in(cx, |ws, window, cx| {
                ws.select_editor(&target);
                target.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)]));
                start_filter(ws, &target, "printf REPLACED", window, cx);
                if cancel {
                    ws.cancel_automation(&CancelAutomation, window, cx);
                } else {
                    target.update(cx, |e, cx| {
                        e.sel(cx).set_selections(vec![Selection::caret(0)]);
                        e.sel(cx).set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)]);
                    });
                }
            });
            let before =
                target.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.doc(cx).version));
            cx.run_until_parked();
            assert!(
                target.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.doc(cx).version))
                    == before
            );
            assert!(text(&ws, cx).to_lowercase().contains(if cancel { "cancelled" } else { "selection changed" }));
        }
    }

    #[gpui::test]
    fn n38_source_edits_during_a_real_process_and_late_cancel_discard_completed_outputs(cx: &mut TestAppContext) {
        let directory = super::super::ui_tests::temp("orlo-n38-real-stale");
        let ready = directory.join("ready");
        let release = directory.join("release");
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "é / 界");
        let (target, snapshot, mut process) = ws.update(cx, |ws, cx| {
            let target = ws.active_editor().unwrap();
            target.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)]));
            let snapshot = ws.filter_snapshot(&target, false, cx).unwrap();
            let process = ws.automation_process(
                ": > \"$ORLO_TEST_READY\"; while [ ! -f \"$ORLO_TEST_RELEASE\" ]; do sleep 0.01; done; printf CHANGED"
                    .into(),
                String::new(),
                cx,
            );
            (target, snapshot, process)
        });
        process.env.extend([
            ("ORLO_TEST_READY".into(), ready.to_string_lossy().into_owned()),
            ("ORLO_TEST_RELEASE".into(), release.to_string_lossy().into_owned()),
        ]);
        process.timeout = std::time::Duration::from_secs(3);
        let inputs = target.read_with(cx, |e, cx| snapshot.inputs(e.doc(cx)));
        let worker =
            std::thread::spawn(move || engine::run_selections(process, inputs, false, &AtomicBool::new(false)));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !ready.exists() {
            assert!(std::time::Instant::now() < deadline, "real subprocess did not start");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        target.update(cx, |e, cx| e.edit(cx, |s| s.insert("new")));
        let changed = target.read_with(cx, |e, cx| e.text(cx));
        std::fs::write(release, "go").unwrap();
        let outputs = worker.join().unwrap();
        assert!(outputs.iter().all(engine::Output::success));
        ws.update_in(cx, |ws, window, cx| {
            ws.finish_filter((target.downgrade(), snapshot, false), &outputs, false, window, cx)
        });
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), changed);
        ws.update_in(cx, |ws, window, cx| {
            ws.select_editor(&target);
            target.update(cx, |e, cx| e.sel(cx).select_all());
            let snapshot = ws.filter_snapshot(&target, false, cx).unwrap();
            ws.finish_filter((target.downgrade(), snapshot, false), &outputs, true, window, cx);
        });
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), changed);
        assert!(text(&ws, cx).contains("cancelled"));
    }

    #[gpui::test]
    fn n38_protected_and_composing_sources_refuse_capture_and_delayed_adoption(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        for mode in ["read-only", "viewer", "follow", "immutable", "local-ime", "shared-ime"] {
            ws.update_in(cx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
            type_in(&ws, cx, "é / 界");
            let (target, snapshot, peer) = ws.update_in(cx, |ws, window, cx| {
                let target = ws.active_editor().unwrap();
                target.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)]));
                let snapshot = ws.filter_snapshot(&target, false, cx).unwrap();
                ws.new_view_into_file(&NewViewIntoFile, window, cx);
                (target, snapshot, ws.active_editor().unwrap())
            });
            match mode {
                "read-only" => target.update(cx, |e, cx| e.document.update(cx, |d, _| d.model.read_only = true)),
                "viewer" => {
                    target.update(cx, |e, cx| e.document.update(cx, |d, _| d.model.set_viewer_mode(true)));
                }
                "follow" => {
                    target.update(cx, |e, cx| e.document.update(cx, |d, _| d.model.set_following(true)));
                }
                "immutable" => target.update(cx, |e, _| e.input_enabled = false),
                "local-ime" | "shared-ime" => {
                    let composing = if mode == "local-ime" { &target } else { &peer };
                    composing.update_in(cx, |e, window, cx| {
                        e.replace_and_mark_text_in_range(Some(0..1), "é", Some(0..1), window, cx)
                    });
                }
                _ => unreachable!(),
            }
            let before =
                target.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.doc(cx).version));
            ws.update_in(cx, |ws, window, cx| {
                assert!(ws.filter_snapshot(&target, false, cx).is_err(), "{mode}");
                assert!(ws.filter_snapshot(&target, true, cx).is_err(), "predicate {mode}");
                ws.select_editor(&target);
                ws.filter_through_shell(&FilterThroughShell, window, cx);
                assert!(ws.panel.is_none(), "{mode}: filter prompt must refuse before taking input focus");
                ws.keep_selections_through_shell(&KeepSelectionsThroughShell, window, cx);
                assert!(ws.panel.is_none(), "{mode}: predicate prompt must refuse before taking input focus");
                let outputs = vec![
                    engine::Output { stdout: b"CHANGED".to_vec(), code: Some(0), ..Default::default() },
                    engine::Output { stdout: b"CHANGED".to_vec(), code: Some(0), ..Default::default() },
                ];
                ws.finish_filter((target.downgrade(), snapshot, false), &outputs, false, window, cx);
            });
            assert!(
                target.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.doc(cx).version))
                    == before,
                "{mode}"
            );
            if mode.ends_with("ime") {
                let composing = if mode == "local-ime" { &target } else { &peer };
                assert!(!composing.read_with(cx, |e, _| e.marked.is_empty()), "refusal preserves composition");
                composing.update_in(cx, |e, window, cx| e.unmark_text(window, cx));
            }
        }
    }

    #[gpui::test]
    fn n38_legacy_document_filter_and_package_ranges_preserve_shared_views_and_stderr(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "é / 界");
        let (target, peer) = ws.update_in(cx, |ws, window, cx| {
            let target = ws.active_editor().unwrap();
            target.update(cx, |e, cx| e.sel(cx).set_cursor(1));
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            let peer = ws.active_editor().unwrap();
            start_filter(ws, &target, "printf whole", window, cx);
            (target, peer)
        });
        cx.run_until_parked();
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "whole");
        assert_eq!(peer.read_with(cx, |e, cx| e.text(cx)), "whole");
        target.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        write_package(
            "N38Ranges",
            r#"{"name":"N38Ranges","commands":[{"name":"Wrap","shell":"printf '('; cat; printf ')'; printf note >&2","filter":true}]}"#,
        );
        ws.update_in(cx, |ws, window, cx| {
            ws.select_editor(&target);
            target.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)]));
            ws.run_package_command(&RunPackageCommand { package: "N38Ranges".into(), name: "Wrap".into() }, window, cx);
        });
        cx.run_until_parked();
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "(é) / (界)");
        assert_eq!(peer.read_with(cx, |e, cx| e.text(cx)), "(é) / (界)");
        assert!(text(&ws, cx).contains("note"));
        target.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "é / 界");
    }

    #[gpui::test]
    fn n38_single_input_package_api_still_applies_after_a_fresh_selection(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "aéz");
        write_package(
            "N38Api",
            r#"{"name":"N38Api","commands":[{"name":"Insert","shell":"printf '%s' '{\"commands\":[{\"command\":\"insert\",\"args\":{\"text\":\"界\"}}]}'","api":true}]}"#,
        );
        let target = ws.update_in(cx, |ws, window, cx| {
            let target = ws.active_editor().unwrap();
            target.update(cx, |e, cx| e.sel(cx).select_preserving_primary(vec![Selection::new(1, 2)]));
            ws.run_package_command(&RunPackageCommand { package: "N38Api".into(), name: "Insert".into() }, window, cx);
            target
        });
        cx.run_until_parked();
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "a界z");
        target.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "aéz");
    }

    #[gpui::test]
    fn n38_legacy_prompt_filters_at_the_natural_typed_caret_and_undo_restores_the_document(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "alpha é");
        let target = ws.update_in(cx, |ws, window, cx| {
            let target = ws.active_editor().unwrap();
            assert_eq!(target.read(cx).state.selections, vec![Selection::caret(7)]);
            ws.filter_through_shell(&FilterThroughShell, window, cx);
            submit_console(ws, "printf '<'; cat; printf '>'", window, cx);
            target
        });
        cx.run_until_parked();
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "<alpha é>");
        target.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(target.read_with(cx, |e, cx| e.text(cx)), "alpha é");
    }

    #[gpui::test]
    fn worksheet_keeps_commands_and_comments_its_output(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.new_shell_worksheet(&NewShellWorksheet, window, cx);
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.replace_ranges(vec![(0..s.len(), "printf worksheet".into())]);
                    s.set_cursor(0);
                });
            });
            ws.run_worksheet(&RunWorksheet, window, cx);
        });
        cx.run_until_parked();
        assert!(text(&ws, cx).starts_with("printf worksheet\n# [Finished successfully]"));
        assert!(text(&ws, cx).contains("# | worksheet"));
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| {
                sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).len() - 2)
            });
            ws.run_worksheet(&RunWorksheet, window, cx);
            assert!(ws.automation.running.is_none(), "output comments are never commands");
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
        });
        assert_eq!(text(&ws, cx), "printf worksheet");
    }

    #[gpui::test]
    fn console_state_keeps_a_live_input_and_the_original_document(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "source");
        let source = ws.update_in(cx, |ws, window, cx| {
            let source = ws.active_editor().unwrap();
            ws.developer_console(&DeveloperConsole, window, cx);
            submit_console(ws, "state", window, cx);
            source
        });
        cx.run_until_parked();
        assert_eq!(source.read_with(cx, |e, cx| e.text(cx)), "source");
        ws.read_with(cx, |ws, cx| {
            assert!(matches!(ws.panel, Some(Panel::Form(_))));
            let transcript = ws.automation.console.as_ref().unwrap().upgrade().unwrap();
            assert!(transcript.read(cx).text(cx).contains("\"buffer_id\""));
            assert_eq!(ws.automation.console_target.as_ref().unwrap().upgrade(), Some(source.clone()));
        });
        ws.update_in(cx, |ws, window, cx| {
            submit_console(ws, r#"{"command":"insert","args":{"text":"!"}}"#, window, cx)
        });
        cx.run_until_parked();
        assert_eq!(source.read_with(cx, |e, cx| e.text(cx)), "source!");
    }

    #[gpui::test]
    fn task_results_take_f4_from_an_old_search_browser(cx: &mut TestAppContext) {
        let directory = super::super::ui_tests::temp("orlo-task-results-f4").canonicalize().unwrap();
        let path = directory.join("diagnostic.txt");
        std::fs::write(&path, "first\nabcdef\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.result_set = Some(crate::searching::results::ResultSet::new(
                "previous search",
                None,
                FindOpts::default(),
                Default::default(),
                "",
            ));
            ws.show_results_panel(window, cx);
            assert!(ws.results_panel().is_some());
            ws.automation_results("diagnostic.txt:2:3: problem", &directory, window, cx);
            assert!(ws.results_panel().is_none());
            assert_eq!(
                ws.result_set.as_ref().unwrap().query,
                "previous search",
                "dismissing the browser preserves its search"
            );
            ws.next_result(&NextResult, window, cx);
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).path.as_ref(), Some(&path));
            assert_eq!(editor.read(cx).doc(cx).point(editor.read(cx).state.cursor()), (1, 2));
        });
    }

    #[gpui::test]
    fn stale_package_task_choices_recheck_enabled_state_and_current_definition(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        write_package("TaskPackage", r#"{"name":"TaskPackage","tasks":[{"name":"Build","shell":"printf stale"}]}"#);
        let action = ws.update(cx, |ws, cx| {
            let rows = ws.task_items(cx).unwrap();
            let row = rows.iter().find(|r| r.label == "TaskPackage: Build").unwrap();
            let ItemAction::Dispatch(action) = &row.action else {
                panic!("task action missing");
            };
            action.as_any().downcast_ref::<RunConfiguredTask>().unwrap().clone()
        });
        engine::set_enabled(&packages_dir(), "TaskPackage", false).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.run_configured_task(&action, window, cx);
            assert!(
                ws.automation.running.is_none(),
                "a choice captured in another window cannot run a disabled package"
            );
        });
        engine::set_enabled(&packages_dir(), "TaskPackage", true).unwrap();
        write_package("TaskPackage", r#"{"name":"TaskPackage","tasks":[{"name":"Build","shell":"printf fresh"}]}"#);
        ws.update_in(cx, |ws, window, cx| ws.run_configured_task(&action, window, cx));
        cx.run_until_parked();
        assert!(
            text(&ws, cx).contains("fresh"),
            "invocation resolves the current manifest instead of cached shell text"
        );
        assert!(!text(&ws, cx).contains("stale"));
        engine::remove_package(&packages_dir(), "TaskPackage").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.run_configured_task(&action, window, cx);
            assert!(ws.automation.running.is_none(), "a removed package cannot execute cached commands");
        });
    }

    #[gpui::test]
    fn package_api_preflights_unavailable_commands_before_any_edit(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "source");
        ws.update_in(cx, |ws, window, cx| {
            let error = ws
                .apply_package_api(
                    br#"{"commands":[{"command":"insert","args":{"text":"changed"}},{"command":"zed::NoAction"}]}"#,
                    window,
                    cx,
                )
                .unwrap_err();
            assert!(error.contains("not available"), "{error}");
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "source");
        });
    }
}

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct RunConfiguredTask {
    pub task: TaskDefinition,
    pub base: PathBuf,
    #[serde(default)]
    pub package: Option<String>,
    /// From the folder's own `.orlo-tasks.json`: it runs once the user trusts the folder.
    #[serde(default)]
    pub project: bool,
}

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct PackageOperation {
    pub name: String,
    pub operation: String,
}

#[derive(Default)]
pub(crate) struct State {
    running: Option<(u64, Arc<AtomicBool>)>,
    generation: u64,
    last_shell: String,
    worksheet: Option<gpui::WeakEntity<Editor>>,
    console: Option<gpui::WeakEntity<Editor>>,
    console_target: Option<gpui::WeakEntity<Editor>>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.running {
            cancel.store(true, Ordering::Release);
        }
    }
}

enum Destination {
    Filter(gpui::WeakEntity<Editor>, FilterSnapshot, bool),
    Api {
        title: String,
        target: gpui::WeakEntity<Editor>,
        snapshot: FilterSnapshot,
    },
    Output {
        title: String,
        diagnostics: bool,
    },
    Worksheet(gpui::WeakEntity<Editor>),
    /// Format Document through a formatter: its argument array, and the document it was given.
    Format(gpui::WeakEntity<Editor>, FormatTarget, Vec<String>),
}

/// The document a formatter was given; any change to it discards the result.
#[derive(Clone, PartialEq)]
pub(crate) struct FormatTarget {
    buffer: u64,
    version: u64,
    path: Option<PathBuf>,
}

impl FormatTarget {
    pub(crate) fn of(editor: &Entity<Editor>, cx: &App) -> Self {
        let doc = editor.read(cx).doc(cx);
        Self { buffer: doc.id(), version: doc.version, path: doc.path.clone() }
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn buffer(&self) -> u64 {
        self.buffer
    }
}

/// The formatter's name in messages.
fn program(command: &[String]) -> String {
    let program = command.first().map_or("", String::as_str);
    Path::new(program).file_name().map_or(program.into(), |name| name.to_string_lossy().into_owned())
}

/// Why a formatter failed: its status and the first line of its stderr.
pub(crate) fn formatter_failure(command: &[String], output: &engine::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    let line: String = line.chars().take(200).collect();
    let detail = if line.is_empty() { String::new() } else { format!(": {line}") };
    format!("Formatter {}: {}{detail}", program(command), output.summary())
}

/// The formatter's stdout as one replacement of the part of `rope` that changed, or `None` when
/// nothing did; carets outside it stay where they were.
pub(crate) fn formatted_edit(
    command: &[String],
    output: &engine::Output,
    rope: &ropey::Rope,
) -> Result<Option<(std::ops::Range<usize>, String)>, String> {
    if !output.success() {
        return Err(formatter_failure(command, output));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| format!("Formatter {}: output is not UTF-8 text", program(command)))?;
    let old = rope.to_string();
    let prefix = old.chars().zip(text.chars()).take_while(|(a, b)| a == b).count();
    let (old_rest, new_rest) = (old.chars().count() - prefix, text.chars().count() - prefix);
    let suffix =
        old.chars().rev().zip(text.chars().rev()).take(old_rest.min(new_rest)).take_while(|(a, b)| a == b).count();
    if old_rest == new_rest && old_rest == suffix {
        return Ok(None);
    }
    let replacement = text.chars().skip(prefix).take(new_rest - suffix).collect();
    Ok(Some((prefix..prefix + old_rest - suffix, replacement)))
}

fn packages_dir() -> PathBuf {
    session::support_dir().join("Packages")
}

fn enabled_packages() -> Result<impl Iterator<Item = engine::Package>, String> {
    Ok(engine::packages_list(&packages_dir())?.into_iter().filter(|p| p.enabled))
}

fn enabled_package(name: &str) -> Result<engine::Package, String> {
    enabled_packages()?.find(|p| p.manifest.name == name).ok_or_else(|| "Package is missing or disabled".into())
}

impl Workspace {
    /// Makes `e` the active tab; false when it is no longer a tab here.
    fn select_editor(&mut self, e: &Entity<Editor>) -> bool {
        let found = self.views.find(e);
        if let Some((g, i)) = found {
            self.views.select(g, i);
        }
        found.is_some()
    }

    /// `e`, while it is still a tab here.
    fn open_tab(&self, e: &gpui::WeakEntity<Editor>) -> Option<Entity<Editor>> {
        e.upgrade().filter(|e| self.views.find(e).is_some())
    }

    fn automation_directory(&self, cx: &App) -> PathBuf {
        let file = self.active_editor().and_then(|e| e.read(cx).doc(cx).path.clone());
        file.as_deref()
            .and_then(|p| self.root_of(p).map(|r| self.roots[r].path.clone()))
            .or_else(|| self.roots.first().map(|r| r.path.clone()))
            .or_else(|| file.and_then(|p| p.parent().map(Path::to_path_buf)))
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    fn automation_process(&self, shell: String, input: String, cx: &App) -> Process {
        let editor = self.active_editor();
        let editor = editor.as_ref().map(|e| e.read(cx));
        let file = editor.and_then(|e| e.doc(cx).path.clone());
        let (start, end) = (editor.and_then(|e| e.state.selections.last())).map_or((0, 0), |s| (s.start(), s.end()));
        let project = self.automation_directory(cx);
        Process {
            shell,
            input: input.into_bytes(),
            env: vec![
                ("ORLO_FILE".into(), file.unwrap_or_default().to_string_lossy().into_owned()),
                ("ORLO_PROJECT".into(), project.to_string_lossy().into_owned()),
                ("ORLO_SELECTION_START".into(), start.to_string()),
                ("ORLO_SELECTION_END".into(), end.to_string()),
            ],
            cwd: project,
            timeout: engine::TIMEOUT,
            output_limit: engine::OUTPUT_LIMIT,
        }
    }

    /// `e`'s formatter at `path`: the `formatters` setting for its language's id over the
    /// language's own command; `[]` means none.
    pub(crate) fn formatter_for(&self, e: &Editor, path: Option<&Path>, cx: &App) -> Option<Vec<String>> {
        let (language, name, _) = self.file_language(e, path, true, cx);
        let id = match &language {
            Some(language) => language.id().to_string(),
            None => crate::languages::treesitter::named(&name)?.registry_id().to_string(),
        };
        let command = match cx.global::<Settings>().formatters.get(&id) {
            Some(command) => command.clone(),
            None => language?.manifest.formatter.as_ref()?.command.clone(),
        };
        (!command.is_empty()).then_some(command)
    }

    /// The document on stdin, run from the file's folder (where formatters look for their
    /// configuration) with the automation environment.
    pub(crate) fn formatter_process(&self, editor: &Entity<Editor>, timeout: std::time::Duration, cx: &App) -> Process {
        let doc = editor.read(cx).doc(cx);
        let file = doc.path.clone().unwrap_or_default();
        let project =
            (self.root_of(&file).map(|r| self.roots[r].path.clone())).unwrap_or_else(|| self.automation_directory(cx));
        let cwd = file.parent().filter(|folder| folder.is_dir()).map_or_else(|| project.clone(), Path::to_path_buf);
        Process {
            shell: String::new(),
            input: doc.rope.to_string().into_bytes(),
            env: vec![
                ("ORLO_FILE".into(), file.to_string_lossy().into_owned()),
                ("ORLO_PROJECT".into(), project.to_string_lossy().into_owned()),
            ],
            cwd,
            timeout,
            output_limit: engine::OUTPUT_LIMIT,
        }
    }

    /// Whether a formatter may run on `editor`, or (given the document it ran on) whether its
    /// output may still replace the text.
    pub(crate) fn format_admission(
        &self,
        editor: &Entity<Editor>,
        target: Option<&FormatTarget>,
        cx: &App,
    ) -> Result<(), String> {
        let e = editor.read(cx);
        if !e.input_enabled || e.single_line || e.doc(cx).read_only || e.doc(cx).edits_blocked() {
            return Err("Formatting requires an editable document".into());
        }
        if self.formatting_composition(e.doc(cx).id(), cx) {
            return Err("Finish or cancel text composition before formatting".into());
        }
        if target.is_some_and(|target| *target != FormatTarget::of(editor, cx)) {
            return Err("the document changed while the formatter ran".into());
        }
        Ok(())
    }

    /// Format Document with `command`: the whole document in, one undoable replacement out,
    /// applied only on exit status 0 and only to the text it was given.
    pub(crate) fn format_with(
        &mut self,
        editor: Entity<Editor>,
        command: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let started = self.format_admission(&editor, None, cx).and_then(|()| {
            editor.update(cx, |e, cx| e.catch_up(cx));
            if editor.read(cx).read_state(cx).view.revision() != editor.read(cx).doc(cx).version {
                return Err("A shared edit is updating this view; try formatting again".into());
            }
            let process = self.formatter_process(&editor, engine::TIMEOUT, cx);
            let target = FormatTarget::of(&editor, cx);
            self.start_automation(process, Destination::Format(editor.downgrade(), target, command), window, cx)
        });
        if let Err(error) = started {
            self.status_message(error, cx);
        }
    }

    fn finish_format(
        &mut self,
        (target, snapshot, command): (gpui::WeakEntity<Editor>, FormatTarget, Vec<String>),
        output: &engine::Output,
        cancelled: bool,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> Result<bool, String> {
            if cancelled {
                return Err("Format Document cancelled".into());
            }
            let target = self.open_tab(&target).ok_or("Format discarded: the source tab closed")?;
            let edit = formatted_edit(&command, output, &target.read(cx).doc(cx).rope)?;
            self.format_admission(&target, Some(&snapshot), cx).map_err(|e| format!("Format discarded: {e}"))?;
            let Some(edit) = edit else { return Ok(false) };
            target.update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(vec![edit])));
            Ok(true)
        })();
        let name = program(&command);
        match result {
            Ok(true) => self.status_message(format!("Formatted with {name} (Undo restores the original)"), cx),
            Ok(false) => self.status_message(format!("Already formatted ({name})"), cx),
            Err(error) => self.status_message(error, cx),
        }
    }

    fn filter_admission(&self, editor: &Entity<Editor>, cx: &App) -> Result<(), String> {
        self.selection_set_admission(editor, cx)?;
        let editor = editor.read(cx);
        if !editor.input_enabled || editor.single_line || editor.doc(cx).read_only || editor.doc(cx).edits_blocked() {
            return Err("Shell filters require an editable document".into());
        }
        if editor.read_state(cx).view.revision() != editor.doc(cx).version {
            return Err("A shared edit is updating this view; try the shell filter again".into());
        }
        Ok(())
    }

    fn filter_snapshot(
        &self,
        editor: &Entity<Editor>,
        predicate: bool,
        cx: &mut Context<Self>,
    ) -> Result<FilterSnapshot, String> {
        self.filter_admission(editor, cx)?;
        editor.update(cx, |editor, cx| {
            editor.catch_up(cx);
            editor.sel(cx).sel_checkpoint();
            FilterSnapshot::for_selections(&editor.read_state(cx), !predicate)
        })
    }

    fn finish_filter(
        &mut self,
        filter: (gpui::WeakEntity<Editor>, FilterSnapshot, bool),
        outputs: &[engine::Output],
        cancelled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (target, snapshot, predicate) = filter;
        let result = (|| -> Result<(), String> {
            if cancelled {
                return Err("Shell filter cancelled".into());
            }
            if let Some(output) = outputs.iter().find(|output| {
                output.problem.is_some() || (!output.success() && !(predicate && output.code == Some(1)))
            }) {
                return Err(output.summary());
            }
            if outputs.len() != snapshot.ranges.len() {
                return Err("Shell filter discarded: incomplete batch".into());
            }
            let target = self.open_tab(&target).ok_or("Filter discarded: the source tab closed")?;
            self.filter_admission(&target, cx)?;
            if !snapshot.current(&target.read(cx).read_state(cx)) {
                return Err("Filter discarded: the document or selection changed while it ran".into());
            }
            if predicate {
                let keep: Vec<_> = outputs.iter().map(|output| output.code == Some(0)).collect();
                let retained = snapshot.retained(&keep);
                target.update(cx, |e, cx| e.edit(cx, |s| s.restore_selection_state(retained)));
            } else {
                let edits = snapshot
                    .ranges
                    .iter()
                    .zip(outputs)
                    .map(|(range, output)| {
                        String::from_utf8(output.stdout.clone())
                            .map(|text| (range.clone(), text))
                            .map_err(|_| "Filter discarded: stdout is not UTF-8 text".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                target.update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(edits)));
            }
            Ok(())
        })();
        let diagnostic = || {
            outputs
                .iter()
                .enumerate()
                .map(|(i, output)| format!("[Selection {}: {}]\n{}\n", i + 1, output.summary(), output.text()))
                .collect::<String>()
        };
        match result {
            Ok(()) => {
                if outputs.iter().any(|output| !output.stderr.is_empty()) {
                    self.scratch_tab(&diagnostic(), "Filter stderr".into(), window, cx);
                }
                self.status_message(
                    if predicate {
                        "Shell predicate applied (Undo Selection restores the original)"
                    } else {
                        "Shell filter applied (Undo restores the original)"
                    },
                    cx,
                );
            }
            Err(why) => {
                self.scratch_tab(&format!("{why}\n\n{}", diagnostic()), "Shell Filter Output".into(), window, cx);
                self.status_message(why, cx);
            }
        }
    }

    fn start_automation(
        &mut self,
        process: Process,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.automation.running.is_some() {
            return Err("A command is running; use Cancel Running Command first".into());
        }
        let inputs = if let Destination::Filter(target, snapshot, predicate) = &destination {
            let target = self.open_tab(target).ok_or("The shell filter source tab closed")?;
            self.filter_admission(&target, cx)?;
            if !snapshot.current(&target.read(cx).read_state(cx)) {
                return Err("The shell filter document or selections changed".into());
            }
            Some((snapshot.inputs(target.read(cx).doc(cx)), *predicate))
        } else {
            None
        };
        self.automation.generation += 1;
        let generation = self.automation.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.automation.running = Some((generation, cancel.clone()));
        let directory = process.cwd.clone();
        let program = match &destination {
            Destination::Format(_, _, command) => Some(command.clone()),
            _ => None,
        };
        self.status_message(
            match &program {
                Some(command) => {
                    format!("Formatting with {}… — Tools > Cancel Running Command to stop", self::program(command))
                }
                None => "Running command… — Tools > Cancel Running Command to stop".into(),
            },
            cx,
        );
        let worker_cancel = cancel.clone();
        let work = cx.background_spawn(async move {
            match (inputs, program) {
                (Some((inputs, predicate)), _) => engine::run_selections(process, inputs, predicate, &worker_cancel),
                (None, Some(command)) => vec![engine::run_program(&command, process, &worker_cancel)],
                (None, None) => vec![engine::run(process, &worker_cancel)],
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let outputs = work.await;
            this.update_in(cx, |ws, window, cx| {
                if ws.automation.running.as_ref().map(|r| r.0) != Some(generation) {
                    return;
                }
                ws.automation.running = None;
                let output = outputs.last().expect("automation always has at least one input");
                let summary = output.summary();
                match destination {
                    Destination::Filter(target, snapshot, predicate) => {
                        ws.finish_filter(
                            (target, snapshot, predicate),
                            &outputs,
                            cancel.load(Ordering::Acquire),
                            window,
                            cx,
                        );
                    }
                    Destination::Format(target, snapshot, command) => {
                        ws.finish_format((target, snapshot, command), output, cancel.load(Ordering::Acquire), cx);
                    }
                    Destination::Api { title, target, snapshot } => {
                        let target = ws.open_tab(&target).filter(|e| snapshot.current(&e.read(cx).read_state(cx)));
                        let result = if !output.success() {
                            Err(summary.clone())
                        } else if let Some(target) = target {
                            ws.select_editor(&target);
                            ws.apply_package_api(&output.stdout, window, cx)
                        } else {
                            Err("Extension result discarded: document or selection changed".into())
                        };
                        let (text, status) = match result {
                            Ok(mut text) => {
                                if !output.stderr.is_empty() {
                                    text.push_str(&format!("\n[stderr]\n{}", String::from_utf8_lossy(&output.stderr)));
                                }
                                (text, summary)
                            }
                            Err(error) => (format!("API error: {error}\n\n{}", output.text()), error),
                        };
                        cx.defer_in(window, move |ws, window, cx| {
                            ws.scratch_tab(&text, title, window, cx);
                            ws.status_message(status, cx);
                        });
                    }
                    Destination::Output { title, diagnostics } => {
                        let text = format!("{title}\n{summary}\n\n{}", output.text());
                        if diagnostics {
                            ws.automation_results(&text, &directory, window, cx);
                        } else {
                            ws.scratch_tab(&text, title, window, cx);
                        }
                        ws.status_message(summary, cx);
                    }
                    Destination::Worksheet(target) => {
                        if let Some(target) = ws.open_tab(&target) {
                            // the output goes in as `#` lines: editable, and not run again with the
                            // worksheet
                            let mut text = format!("\n# [{summary}]\n");
                            for line in output.text().lines() {
                                text += &format!("# | {line}\n");
                            }
                            target.update(cx, |e, cx| {
                                e.edit(cx, |s| {
                                    s.replace_ranges(vec![(s.len()..s.len(), text)]);
                                })
                            });
                        }
                        ws.status_message(summary, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
        Ok(())
    }

    fn automation_results(&mut self, text: &str, directory: &Path, window: &mut Window, cx: &mut Context<Self>) {
        // F4 must follow this task's diagnostics rather than a previous search browser.
        if self.results_panel().is_some() {
            self.close_panel(&ClosePanel, window, cx);
        }
        let (text, outlines) = engine::result_output(text, directory);
        let mut state = EditorState::from_source(&text);
        state.doc.scratch = true;
        state.doc.outlines = outlines;
        let editor = self.new_editor(state, cx);
        editor.update(cx, |e, _| e.results = true);
        self.last_results = Some(editor.downgrade());
        self.attach(editor, false, window, cx);
    }

    pub(crate) fn cancel_automation(&mut self, _: &CancelAutomation, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, cancel)) = &self.automation.running {
            cancel.store(true, Ordering::Release);
            self.status_message("Cancelling command…", cx);
        } else {
            self.status_message("No command is running", cx);
        }
    }

    fn shell_prompt(&mut self, mode: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        if mode.is_some() {
            let Some(editor) = self.active_editor() else {
                return self.status_message("Open an editable document first", cx);
            };
            if let Err(error) = self.filter_admission(&editor, cx) {
                return self.status_message(error, cx);
            }
        }
        let initial = self.automation.last_shell.clone();
        self.show_input(
            match mode {
                Some(true) => "Keep selections through shell (exit 0 keeps, exit 1 drops)",
                Some(false) => "Shell filter (each selection, or document at one caret)",
                None => "Shell command (stdout opens in a new tab)",
            },
            &initial,
            Box::new(move |values, editor, ws, window, cx| {
                let shell = values.text("text").trim().to_string();
                if shell.is_empty() {
                    return Err("Enter a shell command".into());
                }
                ws.automation.last_shell = shell.clone();
                let (destination, input) = if let Some(predicate) = mode {
                    let snapshot = ws.filter_snapshot(&editor, predicate, cx)?;
                    (Destination::Filter(editor.downgrade(), snapshot, predicate), String::new())
                } else {
                    let s = editor.read(cx).read_state(cx);
                    let output = Destination::Output { title: "Shell Output".into(), diagnostics: false };
                    match s.view.selections.last().filter(|s| !s.is_empty()) {
                        Some(selection) => (output, s.doc.rope.slice(selection.range()).to_string()),
                        None => (output, s.text()),
                    }
                };
                let process = ws.automation_process(shell, input, cx);
                ws.start_automation(process, destination, window, cx)
            }),
            window,
            cx,
        );
    }

    forward_actions! {
    pub(crate) filter_through_shell(FilterThroughShell, window, cx) => shell_prompt(Some(false), window, cx);
    pub(crate) keep_selections_through_shell(KeepSelectionsThroughShell, window, cx) => shell_prompt(Some(true), window, cx);
    pub(crate) run_shell_command(RunShellCommand, window, cx) => shell_prompt(None, window, cx);
    pub(crate) edit_tasks(EditTasks, window, cx) => edit_tasks_at(session::support_dir().join("Tasks.json"), window, cx);
    }

    pub(crate) fn new_shell_worksheet(&mut self, _: &NewShellWorksheet, window: &mut Window, cx: &mut Context<Self>) {
        let worksheet = self.automation.worksheet.as_ref().and_then(|e| e.upgrade());
        if worksheet.is_some_and(|e| self.select_editor(&e)) {
            self.focus_editor(window, cx);
        } else {
            self.scratch_tab("# Shell Worksheet — /bin/sh, no PTY or interactive stdin\n# Run selection or current line with Tools > Run Worksheet Selection / Line.\n# Output is appended as editable shell comments. ORLO_FILE and ORLO_PROJECT are available.\n\npwd\n", "Shell Worksheet".into(), window, cx);
            self.automation.worksheet = self.active_editor().map(|e| e.downgrade());
            self.with_active(cx, |e, cx| {
                sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).line_start(4));
                cx.notify();
            });
        }
    }

    pub(crate) fn run_worksheet(&mut self, _: &RunWorksheet, window: &mut Window, cx: &mut Context<Self>) {
        let worksheet = self.automation.worksheet.as_ref().and_then(|w| w.upgrade());
        let Some(editor) = self.active_editor().filter(|e| worksheet.as_ref() == Some(e)) else {
            return self.status_message("Open a Shell Worksheet first", cx);
        };
        let s = editor.read(cx).read_state(cx);
        if s.view.selections.len() != 1 {
            return self.status_message("Use one selection in the worksheet", cx);
        }
        let selection = s.view.selections[0];
        let range = if selection.is_empty() {
            let line = s.point(selection.head).0;
            s.line_start(line)..s.line_end(line)
        } else {
            selection.range()
        };
        let shell = s.doc.rope.slice(range).to_string();
        if (shell.lines().map(str::trim)).all(|l| l.is_empty() || l.starts_with('#')) {
            return self.status_message("Select a command, or place the caret on a command line", cx);
        }
        let process = self.automation_process(shell, String::new(), cx);
        if let Err(error) = self.start_automation(process, Destination::Worksheet(editor.downgrade()), window, cx) {
            self.status_message(error, cx);
        }
    }

    fn task_items(&self, cx: &App) -> Result<Vec<quick_panel::Item>, String> {
        let directory = self.automation_directory(cx);
        let mut items = Vec::new();
        let mut sources = vec![(session::support_dir().join("Tasks.json"), directory.clone(), false)];
        if !self.roots.is_empty() {
            sources.push((directory.join(".orlo-tasks.json"), directory, true));
        }
        for (path, base, project) in sources {
            for task in engine::read_tasks(&path)? {
                items.push(quick_panel::Item::command(
                    task.name.clone(),
                    path.display().to_string(),
                    RunConfiguredTask { task, base: base.clone(), package: None, project },
                ));
            }
        }
        for package in enabled_packages()? {
            for task in package.manifest.tasks {
                items.push(quick_panel::Item::command(
                    format!("{}: {}", package.manifest.name, task.name),
                    "Package task — runs only when selected",
                    RunConfiguredTask {
                        task,
                        base: package.path.clone(),
                        package: Some(package.manifest.name.clone()),
                        project: false,
                    },
                ));
            }
        }
        Ok(items)
    }

    pub(crate) fn run_task(&mut self, _: &RunTask, window: &mut Window, cx: &mut Context<Self>) {
        match self.task_items(cx) {
            Ok(items) if !items.is_empty() => self.open_quick_panel(Kind::Choices, items, window, cx),
            Ok(_) => self.edit_tasks(&EditTasks, window, cx),
            Err(error) => self.status_message(error, cx),
        }
    }

    pub(crate) fn run_configured_task(
        &mut self,
        action: &RunConfiguredTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let resolved = (|| -> Result<(TaskDefinition, PathBuf), String> {
            let Some(name) = &action.package else {
                return Ok((action.task.clone(), action.base.clone()));
            };
            let package = enabled_package(name)?;
            let task = package
                .manifest
                .tasks
                .into_iter()
                .find(|t| t.name == action.task.name)
                .ok_or("Package task no longer exists")?;
            Ok((task, package.path))
        })();
        let (task, base) = match resolved {
            Ok(task) => task,
            Err(error) => return self.status_message(error, cx),
        };
        // a task the folder brought along runs its shell command: shown, and run only once the
        // user trusts the folder
        if action.project && !orlo_utils::trust::trusted(&base) {
            let detail = format!(
                "“{}” comes with {} and runs:\n\n{}\n\nTrusting the folder also lets its own Git settings and language servers run.",
                task.name,
                tilde(&base),
                task.shell
            );
            let choices = ["Trust Folder and Run", "Cancel"];
            let answer =
                window.prompt(PromptLevel::Warning, "Run a task this folder defines?", Some(&detail), &choices, cx);
            let action = action.clone();
            cx.spawn_in(window, async move |this, cx| {
                if answer.await != Ok(0) {
                    return;
                }
                this.update_in(cx, |ws, w, cx| {
                    if let Err(e) = orlo_utils::trust::trust(&action.base) {
                        return ws.status_message(format!("Could not trust {}: {e}", tilde(&action.base)), cx);
                    }
                    ws.run_configured_task(&action, w, cx);
                })
                .ok();
            })
            .detach();
            return;
        }
        let mut process = self.automation_process(task.shell.clone(), String::new(), cx);
        process.cwd = base.join(task.cwd.as_deref().unwrap_or(Path::new(".")));
        if let Some(seconds) = task.timeout_secs {
            process.timeout = std::time::Duration::from_secs(seconds.clamp(1, 3600));
        }
        let destination = Destination::Output { title: format!("Task: {}", task.name), diagnostics: true };
        if let Err(error) = self.start_automation(process, destination, window, cx) {
            self.status_message(error, cx);
        }
    }

    fn edit_tasks_at(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            use std::io::Write;
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => file.write_all(b"{\n  \"tasks\": [\n    { \"name\": \"Build\", \"shell\": \"cargo build\" },\n    { \"name\": \"Test\", \"shell\": \"cargo test\" }\n  ]\n}\n"),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                Err(e) => Err(e),
            }
        })();
        match result {
            Ok(()) => self.open_path_here(path, window, cx),
            Err(error) => self.status_message(error.to_string(), cx),
        }
    }

    pub(crate) fn edit_project_tasks(&mut self, _: &EditProjectTasks, window: &mut Window, cx: &mut Context<Self>) {
        if self.roots.is_empty() {
            return self.status_message("Open a project folder first", cx);
        }
        self.edit_tasks_at(self.automation_directory(cx).join(".orlo-tasks.json"), window, cx);
    }

    /// The packages' commands, added to the Command Palette as it opens; a manifest is read as
    /// data, never run.
    pub(crate) fn automation_command_items(&self) -> Result<Vec<quick_panel::Item>, String> {
        let mut items = Vec::new();
        for package in enabled_packages()? {
            for command in package.manifest.commands {
                items.push(quick_panel::Item::command(
                    format!("{}: {}", package.manifest.name, command.name),
                    if command.filter {
                        "Shell filter"
                    } else if command.api {
                        "Shell extension API command"
                    } else {
                        "Shell package command"
                    },
                    RunPackageCommand { package: package.manifest.name.clone(), name: command.name },
                ));
            }
        }
        Ok(items)
    }

    pub(crate) fn run_package_command(
        &mut self,
        action: &RunPackageCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> Result<(), String> {
            let package = enabled_package(&action.package)?;
            let command = package
                .manifest
                .commands
                .into_iter()
                .find(|c| c.name == action.name)
                .ok_or("Package command no longer exists")?;
            let editor = self.active_editor().ok_or("Open a tab first")?;
            let (snapshot, input) = if command.filter {
                (self.filter_snapshot(&editor, false, cx)?, String::new())
            } else {
                editor.update(cx, |e, cx| {
                    e.catch_up(cx);
                    e.sel(cx).sel_checkpoint();
                    let s = e.read_state(cx);
                    let snapshot = FilterSnapshot::new(&s)?;
                    let input = snapshot.input(s.doc);
                    Ok::<_, String>((snapshot, input))
                })?
            };
            let mut process = self.automation_process(command.shell, input, cx);
            process.cwd = package.path.clone();
            process.env.push(("ORLO_PACKAGE".into(), package.path.to_string_lossy().into_owned()));
            let title = format!("{}: {}", action.package, action.name);
            let destination = if command.filter {
                Destination::Filter(editor.downgrade(), snapshot, false)
            } else if command.api {
                Destination::Api { title, target: editor.downgrade(), snapshot }
            } else {
                Destination::Output { title, diagnostics: false }
            };
            self.start_automation(process, destination, window, cx)
        })();
        if let Err(error) = result {
            self.status_message(error, cx);
        }
    }

    pub(crate) fn manage_packages(&mut self, _: &ManagePackages, window: &mut Window, cx: &mut Context<Self>) {
        let mut items = vec![quick_panel::Item::command(
            "Install Local Package…",
            "Select a folder containing package.json; code runs only on invocation",
            InstallPackage,
        )];
        let result = (|| -> std::io::Result<()> {
            let entries = match std::fs::read_dir(packages_dir()) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if !entry.file_type()?.is_dir() || name.starts_with('.') {
                    continue;
                }
                let verb = if entry.path().join(".disabled").exists() { "Enable" } else { "Disable" };
                items.push(quick_panel::Item::command(
                    format!("{verb} {name}"),
                    entry.path().display().to_string(),
                    PackageOperation { name: name.clone(), operation: verb.to_lowercase() },
                ));
                items.push(quick_panel::Item::command(
                    format!("Remove {name}"),
                    "Removes the installed copy; the source folder is kept",
                    PackageOperation { name, operation: "remove".into() },
                ));
            }
            Ok(())
        })();
        if let Err(error) = result {
            return self.status_message(error.to_string(), cx);
        }
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }

    pub(crate) fn package_operation(&mut self, action: &PackageOperation, window: &mut Window, cx: &mut Context<Self>) {
        let result = match action.operation.as_str() {
            "enable" => engine::set_enabled(&packages_dir(), &action.name, true),
            "disable" => engine::set_enabled(&packages_dir(), &action.name, false),
            "remove" => engine::remove_package(&packages_dir(), &action.name),
            _ => Err("Unknown package operation".into()),
        };
        if result.is_ok() {
            self.reload_customizations(&ReloadCustomizations, window, cx);
        }
        self.status_message(
            match result {
                Ok(()) => format!("Package {}: {}", action.name, action.operation),
                Err(error) => error,
            },
            cx,
        );
    }

    pub(crate) fn install_package(&mut self, _: &InstallPackage, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Install Local Package".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(source) = paths.into_iter().next() else {
                return;
            };
            let result = cx.background_spawn(async move { engine::install_package(&source, &packages_dir()) }).await;
            this.update_in(cx, |ws, window, cx| {
                if result.is_ok() {
                    ws.reload_customizations(&ReloadCustomizations, window, cx);
                }
                ws.status_message(
                    match result {
                        Ok(name) => {
                            format!("Installed {name}; invoke its commands from the palette")
                        }
                        Err(error) => format!("Package installation failed: {error}"),
                    },
                    cx,
                )
            })
            .ok();
        })
        .detach();
    }

    fn apply_package_api(
        &mut self,
        bytes: &[u8],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Reply {
            #[serde(default)]
            commands: Vec<crate::config::customization::Command>,
            #[serde(default)]
            text: String,
        }
        let reply: Reply = serde_json::from_slice(bytes).map_err(|e| format!("Invalid extension API response: {e}"))?;
        if reply.commands.len() > 128 {
            return Err("Extension response exceeds 128 API commands".into());
        }
        for command in &reply.commands {
            self.validate_command(command, cx)?;
        }
        for command in &reply.commands {
            self.execute_command(command, window, cx)?;
        }
        Ok(reply.text)
    }

    fn console_entry(&mut self, entry: &str, window: &mut Window, cx: &mut Context<Self>) {
        let console = self.automation.console.as_ref().and_then(|e| e.upgrade());
        if let Some(editor) = console.filter(|e| self.select_editor(e)) {
            editor.update(cx, |e, cx| e.append(entry, [], cx));
        } else {
            self.scratch_tab(entry, "Developer Console".into(), window, cx);
            self.automation.console = self.active_editor().map(|e| e.downgrade());
        }
    }

    pub(crate) fn developer_console(&mut self, _: &DeveloperConsole, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active_editor();
        let console = self.automation.console.as_ref().and_then(|e| e.upgrade());
        if active != console {
            self.automation.console_target = active.map(|e| e.downgrade());
        }
        self.console_prompt(window, cx);
    }

    fn console_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_window_form("Developer Console — help, state, or a JSON command", vec![Field::text("text", "API command", "")],
            Box::new(|values, _, ws, window, cx| {
                let input = values.text("text").trim();
                if let Some(target) = ws.automation.console_target.as_ref().and_then(|e| e.upgrade()) {
                    ws.select_editor(&target);
                }
                let result = match input {
                    "help" => Ok("help — this reference\nstate — active file, buffer version, selections, project and running task\n{\"command\":\"insert\",\"args\":{\"text\":\"hello\"}} — execute an Orlo command\nCommands use the same dispatcher as key bindings and macros. Shell extensions may return {\"commands\":[...],\"text\":\"optional output\"} with api:true.\n".into()),
                    "state" => {
                        let state = ws.active_editor().map(|e| e.read(cx).read_state(cx));
                        Ok(serde_json::to_string_pretty(&serde_json::json!({
                            "file": state.as_ref().and_then(|s| s.doc.path.clone()),
                            "buffer_id": state.as_ref().map(|s| s.buffer_id()),
                            "version": state.as_ref().map(|s| s.doc.version),
                            "selections": state.as_ref().map(|s| s.view.selections.iter().map(|s| serde_json::json!({"anchor":s.anchor,"head":s.head})).collect::<Vec<_>>()),
                            "project": ws.project.as_ref().map(|p| &p.path),
                            "folders": ws.roots.iter().map(|r| &r.path).collect::<Vec<_>>(),
                            "running": ws.automation.running.is_some(),
                        })).map_err(|e| e.to_string())?)
                    }
                    "" => return Err("Enter help, state, or a JSON command".into()),
                    _ => serde_json::from_str::<crate::config::customization::Command>(input).map_err(|e| e.to_string())
                        .and_then(|command| ws.execute_command(&command, window, cx)).map(|_| "OK".into()),
                };
                let output = match result { Ok(text) => text, Err(error) => format!("Error: {error}") };
                let entry = format!("> {input}\n{output}\n\n");
                // Dispatch queues GPUI actions; append the transcript after they have run.
                cx.defer_in(window, move |ws, window, cx| {
                    let active = ws.active_editor();
                    ws.automation.console_target = active.as_ref().map(|e| e.downgrade());
                    let prompt = ws.panel.is_none();
                    ws.console_entry(&entry, window, cx);
                    if prompt {
                        ws.console_prompt(window, cx);
                    } else if let Some(active) = active {
                        ws.select_editor(&active);
                    }
                });
                Ok(())
            }), window, cx);
    }
}
