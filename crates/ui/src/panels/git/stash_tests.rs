use super::tests::{git, repo, run_command};
use super::*;
use crate::workspace::ui_tests::{two_windows, window};
use gpui::TestAppContext;

fn choose(ws: &mut Workspace, label: &str, w: &mut Window, cx: &mut Context<Workspace>) {
    let p = ws.quick_panel.as_mut().unwrap();
    p.selected = p.filtered.iter().position(|i| p.items[*i].label == label).unwrap();
    ws.confirm_quick_panel(w, cx);
}
fn action(ws: &Workspace, label: &str) -> GitStashAction {
    let item = ws.quick_panel.as_ref().unwrap().items.iter().find(|i| i.label == label).unwrap();
    let quick_panel::ItemAction::Dispatch(a) = &item.action else { panic!() };
    a.as_any().downcast_ref::<GitStashAction>().unwrap().clone()
}
fn setup<'a>(cx: &'a mut TestAppContext, name: &str) -> (PathBuf, Entity<Workspace>, &'a mut gpui::VisualTestContext) {
    let root = repo(name);
    std::fs::write(root.join("file.txt"), "stashed 界\n").unwrap();
    git(&root, &["stash", "push", "-m", "draft 界"]);
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    (root, ws, cx)
}
fn open_entry(ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Stashes…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        assert_eq!(ws.quick_panel.as_ref().unwrap().items.len(), 1);
        ws.confirm_quick_panel(w, cx);
    });
}
#[gpui::test]
fn stash_picker_preview_and_cancel_preserve_file_selection(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-preview");
    let selected = (root.clone(), PathBuf::from("file.txt"));
    ws.update_in(cx, |ws, w, cx| {
        ws.git.selected = Some(selected.clone());
        ws.git_stashes(&GitStashes, w, cx);
        ws.cancel_quick_panel(cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.quick_panel.is_none());
        assert_eq!(ws.git.selected.as_ref(), Some(&selected));
    });
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Preview", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        assert!(e.read(cx).doc(cx).read_only && e.read(cx).doc(cx).scratch);
        assert!(e.read(cx).text(cx).contains("+stashed 界"));
        assert_eq!(ws.git.selected.as_ref(), Some(&selected));
    });
}
#[gpui::test]
fn stash_drop_requires_confirmation_and_rejects_changed_stack(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-drop");
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Drop…", w, cx));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Drop…", w, cx));
    std::fs::write(root.join("file.txt"), "new stash\n").unwrap();
    git(&root, &["stash", "push"]);
    cx.simulate_prompt_answer("Drop");
    cx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 2);
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
}
#[gpui::test]
fn stash_apply_refuses_dirty_read_only_shared_view_and_global_writer(cx: &mut TestAppContext) {
    let root = repo("stash-shared");
    std::fs::write(root.join("file.txt"), "stashed\n").unwrap();
    git(&root, &["stash", "push"]);
    let (a, b, mut acx, mut bcx) = two_windows(cx);
    for (ws, cx) in [(&a, &mut acx), (&b, &mut bcx)] {
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
        });
        cx.run_until_parked();
    }
    b.update(&mut bcx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.update_doc(cx, |d| d.read_only = true);
            e.edit(cx, |s| s.insert("draft"));
        })
    });
    open_entry(&a, &mut acx);
    a.update_in(&mut acx, |ws, w, cx| choose(ws, "Apply", w, cx));
    acx.run_until_parked();
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Save or close")));
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    b.update(&mut bcx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    let lease = acx.update(|_, cx| pipelines::lease(&root, cx));
    open_entry(&a, &mut acx);
    a.update_in(&mut acx, |ws, w, cx| choose(ws, "Pop", w, cx));
    acx.run_until_parked();
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Wait")));
    drop(lease);
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
}

#[gpui::test]
fn stash_create_defaults_and_index_restore_are_reachable(cx: &mut TestAppContext) {
    let root = repo("stash-create");
    std::fs::write(root.join("file.txt"), "staged\n").unwrap();
    git(&root, &["add", "."]);
    std::fs::write(root.join("file.txt"), "working\n").unwrap();
    std::fs::write(root.join("new 界.txt"), "untracked\n").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Stash…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        p.inputs().next().unwrap().update(cx, |e, cx| e.set_text("draft 界", cx));
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    assert!(root.join("new 界.txt").exists());
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Apply and Restore Index", w, cx));
    cx.run_until_parked();
    assert_eq!(git::index_text(&root, Path::new("file.txt")).unwrap().unwrap(), "staged\n");
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "working\n");
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Drop…", w, cx));
    cx.simulate_prompt_answer("Drop");
    cx.run_until_parked();
    assert!(git::prepare_stashes(&root).unwrap().entries.is_empty());
}

#[gpui::test]
fn stash_pending_reads_do_not_replace_new_surface_or_project(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-late");
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| {
        let preview = action(ws, "Preview");
        ws.cancel_quick_panel(cx);
        ws.git_stash_action(&preview, w, cx);
        ws.command_palette(&CommandPalette, w, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(!ws.active_editor().unwrap().read(cx).doc(cx).scratch);
        assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.label == "Git: Stashes…"));
    });
    ws.update_in(cx, |ws, w, cx| {
        ws.cancel_quick_panel(cx);
        ws.git_create_stash(&GitCreateStash, w, cx);
        ws.roots.push(crate::config::project::Root::new(root));
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(!matches!(ws.panel, Some(Panel::Form(_)))));
}

#[gpui::test]
fn stash_pending_create_form_refuses_changed_target_and_replaced_repository(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-form-stale");
    for replace in [false, true] {
        ws.update_in(cx, |ws, w, cx| ws.git_create_stash(&GitCreateStash, w, cx));
        cx.run_until_parked();
        if replace {
            std::fs::rename(root.join(".git"), root.join("old-git")).unwrap();
            git(&root, &["init", "-q"]);
        } else {
            ws.update(cx, |ws, _| ws.roots.push(crate::config::project::Root::new(root.clone())));
        }
        ws.update_in(cx, |ws, w, cx| ws.submit_form(&form::Submit, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let message = if let Some(Panel::Form(p)) = &ws.panel {
                p.error.as_ref().unwrap()
            } else {
                ws.git.message.as_ref().unwrap()
            };
            assert!(message.contains("changed"), "{message}");
        });
        ws.update_in(cx, |ws, w, cx| ws.close_panel(&ClosePanel, w, cx));
    }
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
}

#[gpui::test]
fn stash_pop_conflict_keeps_entry_and_stale_apply_never_changes_worktree(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-conflict");
    open_entry(&ws, cx);
    std::fs::write(root.join("file.txt"), "second\n").unwrap();
    git(&root, &["stash", "push"]);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Apply", w, cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    git(&root, &["stash", "drop"]);
    std::fs::write(root.join("file.txt"), "new head\n").unwrap();
    git(&root, &["commit", "-qam", "new head"]);
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Pop", w, cx));
    cx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    assert!(std::fs::read_to_string(root.join("file.txt")).unwrap().contains("<<<<<<<"));
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("kept")));
}

#[gpui::test]
fn stash_refuses_retained_document_save_preparation_before_write_lease(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-preparation");
    let prep = root.join("preparation");
    let held = crate::documents::save::gate::pause(&prep);
    let retained = cx.update(|_, cx| {
        let mut model = crate::editing::text::Document::new();
        model.path = Some(root.join("retained 界.txt"));
        let doc = crate::documents::Documents::open(model, cx);
        OpenDocument::save(
            &doc,
            None,
            None,
            move |cx| {
                Some(cx.spawn(async move |_| {
                    crate::documents::save::gate::wait(&prep).await;
                }))
            },
            |_, _| {},
            cx,
        );
        assert!(doc.read(cx).saving());
        assert!(!pipelines::writing_under(&root, cx));
        doc
    });
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Pop", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("save preparations")));
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    held.release();
    cx.run_until_parked();
    assert!(!retained.read_with(cx, |d, _| d.saving()));
}

#[gpui::test]
fn stash_worker_holds_directory_lease_and_completion_preserves_new_surface(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-worker");
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| {
        let apply = action(ws, "Apply");
        ws.cancel_quick_panel(cx);
        ws.git_stash_action(&apply, w, cx);
        assert!(pipelines::writing_under(&root, cx));
        ws.command_palette(&CommandPalette, w, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(!pipelines::writing_under(&root, cx));
        assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.label == "Git: Stashes…"));
        assert!(!ws.active_editor().unwrap().read(cx).doc(cx).scratch);
    });
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "stashed 界\n");
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
}

#[gpui::test]
fn stash_preview_actions_refuse_stack_changes_and_drop_refuses_new_project(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-preview-stale");
    open_entry(&ws, cx);
    let stale = ws.update_in(cx, |ws, w, cx| {
        let stale = action(ws, "Pop");
        choose(ws, "Preview", w, cx);
        stale
    });
    cx.run_until_parked();
    std::fs::write(root.join("file.txt"), "replacement\n").unwrap();
    git(&root, &["stash", "push"]);
    ws.update_in(cx, |ws, w, cx| ws.git_stash_action(&stale, w, cx));
    cx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 2);
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    git(&root, &["stash", "drop"]);
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Drop…", w, cx));
    assert!(cx.has_pending_prompt());
    ws.update(cx, |ws, _| ws.restore_gen += 1);
    cx.simulate_prompt_answer("Drop");
    cx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
}

#[gpui::test]
fn stash_refuses_another_windows_pending_git_operation(cx: &mut TestAppContext) {
    let root = repo("stash-other-window-operation");
    std::fs::write(root.join("file.txt"), "stashed\n").unwrap();
    git(&root, &["stash", "push"]);
    let (a, b, mut acx, mut bcx) = two_windows(cx);
    a.update_in(&mut acx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    acx.run_until_parked();
    open_entry(&a, &mut acx);
    b.update(&mut bcx, |ws, _| ws.git.operation = true);
    a.update_in(&mut acx, |ws, w, cx| choose(ws, "Pop", w, cx));
    acx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Git operation")));
}

fn cleared_worker(cx: &mut TestAppContext, commit: bool) {
    let root = repo(&format!("stash-cleared-worker-{commit}"));
    std::fs::write(root.join("file.txt"), "stashed\n").unwrap();
    git(&root, &["stash", "push"]);
    std::fs::write(root.join("other.txt"), "stage this\n").unwrap();
    if commit {
        git(&root, &["add", "other.txt"]);
    }
    let (a, b, mut acx, mut bcx) = two_windows(cx);
    for (ws, cx) in [(&a, &mut acx), (&b, &mut bcx)] {
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("other.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
        });
        cx.run_until_parked();
    }
    open_entry(&b, &mut bcx);
    let pop = b.read_with(&bcx, |ws, _| action(ws, "Pop"));
    if commit {
        a.update_in(&mut acx, |ws, w, cx| ws.git_commit(&GitCommit, w, cx));
        acx.run_until_parked();
    }
    // The test scheduler does not poll background work between these foreground
    // updates: clear removes the window flag while the actual worker is still queued.
    a.update_in(&mut acx, |ws, w, cx| {
        if commit {
            let Some(Panel::Form(p)) = &ws.panel else { panic!() };
            p.inputs().next().unwrap().update(cx, |e, cx| e.set_text("pending commit", cx));
            ws.submit_form(&form::Submit, w, cx);
        } else {
            ws.git_stage(&GitStage, w, cx);
        }
        assert!(ws.git.operation);
        ws.clear_window(w, cx);
        assert!(!ws.git.operation);
    });
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.cancel_quick_panel(cx);
        ws.git_stash_action(&pop, w, cx);
        assert!(!ws.git.operation, "stash must wait for the cleared window's {commit} worker");
        assert!(ws.status.as_ref().unwrap().0.contains("Wait"));
    });
    acx.run_until_parked();
    bcx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    assert_eq!(git::index_text(&root, Path::new("other.txt")).unwrap().as_deref(), Some("stage this\n"));
    if commit {
        assert_eq!(git::prepare_commit(&root, true).unwrap().draft.subject, "pending commit");
    }
    assert!(!bcx.update(|_, cx| pipelines::writing_under(&root, cx)));
}

#[gpui::test]
fn stash_waits_for_index_worker_after_owning_window_is_cleared(cx: &mut TestAppContext) {
    cleared_worker(cx, false);
}

#[gpui::test]
fn stash_waits_for_commit_worker_after_owning_window_is_cleared(cx: &mut TestAppContext) {
    cleared_worker(cx, true);
}

#[gpui::test]
fn stash_pop_removal_error_reports_applied_changes_to_the_user(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-pop-removal-error");
    let lock = root.join(".git/logs/refs/stash.lock");
    std::fs::write(&lock, "external reflog writer").unwrap();
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Pop", w, cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "stashed 界\n");
    assert_eq!(git::prepare_stashes(&root).unwrap().entries.len(), 1);
    ws.read_with(cx, |ws, _| {
        let message = ws.git.message.as_ref().unwrap();
        assert!(message.contains("Changes applied, but stash removal failed"), "{message}");
        assert!(message.contains("do not apply these changes again"));
    });
    assert!(!root.join(".git/refs/stash.lock").exists());
    std::fs::remove_file(lock).unwrap();
}

fn conflicted_stash(cx: &mut TestAppContext, pop: bool) {
    let root = repo(&format!("stash-conflict-tools-{pop}"));
    std::fs::write(root.join("file.txt"), "stashed 界\n").unwrap();
    git(&root, &["stash", "push", "-m", "keep this object"]);
    std::fs::write(root.join("file.txt"), "new head 🦀\n").unwrap();
    std::fs::write(root.join("other.txt"), "unrelated source\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "new head"]);
    let entries = git::prepare_stashes(&root).unwrap().entries;
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        // Pop reloads an already-open clean source; Apply opens a closed source.
        ws.open_path(root.join(if pop { "file.txt" } else { "other.txt" }), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, if pop { "Pop" } else { "Apply" }, w, cx));
    cx.run_until_parked();
    assert_eq!(git::prepare_stashes(&root).unwrap().entries, entries, "the exact selected stash must survive");
    let conflicted = std::fs::read_to_string(root.join("file.txt")).unwrap();
    assert!(conflicted.contains("<<<<<<<"));
    ws.update_in(cx, |ws, w, cx| {
        let error = ws.git.message.clone().unwrap();
        assert!(error.contains("kept"), "{error}");
        assert!(!error.contains("operation completed"));
        assert_eq!(ws.status.as_ref().unwrap().0, error);
        assert!(ws.git.sidebar_open, "a failed stash must open the existing conflict tools");
        assert_eq!(ws.git.selected, Some((root.clone(), "file.txt".into())));
        assert!(!ws.git.staged);
        let editor = ws.active_editor().unwrap();
        let e = editor.read(cx);
        assert_eq!(e.doc(cx).path.as_ref(), Some(&root.join("file.txt")));
        assert_eq!(e.text(cx), conflicted);
        let block = git::conflicts(&conflicted).remove(0);
        assert_eq!(e.state.cursor(), e.doc(cx).rope.byte_to_char(block.range.start));
        assert!(e.is_focused(w));
        assert!(!e.doc(cx).unsaved());
        run_command(ws, "Git: Accept Both", w, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, _, cx| {
        let editor = ws.active_editor().unwrap();
        assert_eq!(editor.read(cx).text(cx), "new head 🦀\nstashed 界\n");
        editor.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(editor.read(cx).text(cx), conflicted);
    });
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), conflicted);
    assert_eq!(git::prepare_stashes(&root).unwrap().entries, entries);
}

#[gpui::test]
fn stash_conflicted_apply_opens_existing_conflict_tools_and_keeps_exact_entry(cx: &mut TestAppContext) {
    conflicted_stash(cx, false);
}

#[gpui::test]
fn stash_conflicted_pop_reloads_clean_source_and_opens_existing_conflict_tools(cx: &mut TestAppContext) {
    conflicted_stash(cx, true);
}

#[gpui::test]
fn stash_git_failure_without_unmerged_paths_keeps_error_and_source(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-no-conflict-error");
    let entries = git::prepare_stashes(&root).unwrap().entries;
    std::fs::write(root.join(".git/index.lock"), "external writer").unwrap();
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Pop", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let error = ws.git.message.as_ref().unwrap();
        assert!(error.contains("index.lock"), "{error}");
        assert_eq!(&ws.status.as_ref().unwrap().0, error);
        assert!(!ws.git.sidebar_open);
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "one\ntwo\nthree\n");
    });
    assert_eq!(git::prepare_stashes(&root).unwrap().entries, entries);
}

#[gpui::test]
fn stash_conflict_completion_preserves_new_picker_and_git_selection(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "stash-conflict-new-surface");
    std::fs::write(root.join("file.txt"), "new head\n").unwrap();
    git(&root, &["commit", "-qam", "new head"]);
    let entries = git::prepare_stashes(&root).unwrap().entries;
    open_entry(&ws, cx);
    let selected = Some((root.clone(), PathBuf::from("other.txt")));
    ws.update_in(cx, |ws, w, cx| {
        let pop = action(ws, "Pop");
        ws.cancel_quick_panel(cx);
        ws.git_stash_action(&pop, w, cx);
        ws.git.selected = selected.clone();
        ws.command_palette(&CommandPalette, w, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.label == "Git: Stashes…"));
        assert_eq!(ws.git.selected, selected);
        assert!(!ws.git.sidebar_open);
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "one\ntwo\nthree\n");
        assert!(ws.git.message.as_ref().unwrap().contains("kept"));
    });
    assert!(std::fs::read_to_string(root.join("file.txt")).unwrap().contains("<<<<<<<"));
    assert_eq!(git::prepare_stashes(&root).unwrap().entries, entries);
}

fn late_conflict_source(cx: &mut TestAppContext, change: &str) {
    use gpui::EntityInputHandler;
    let root = repo(&format!("stash-conflict-late-{change}"));
    let path = root.join("file.txt");
    std::fs::write(&path, "stashed\n").unwrap();
    git(&root, &["stash", "push"]);
    std::fs::write(&path, "new head\n").unwrap();
    git(&root, &["commit", "-qam", "new head"]);
    let entries = git::prepare_stashes(&root).unwrap().entries;
    let (a, b, mut acx, mut bcx) = two_windows(cx);
    for (ws, cx) in [(&a, &mut acx), (&b, &mut bcx)] {
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(path.clone(), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
        });
        cx.run_until_parked();
    }
    let held = crate::testing::gate::pause("stash-conflict-read", &path);
    open_entry(&a, &mut acx);
    a.update_in(&mut acx, |ws, w, cx| choose(ws, "Pop", w, cx));
    acx.run_until_parked();
    assert_eq!(held.arrived(), 1, "pause after the real conflict source read");
    let editor = b.update_in(&mut bcx, |ws, w, cx| {
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| match change {
            "dirty" => {
                e.update_doc(cx, |d| d.read_only = true);
                e.edit(cx, |s| s.insert("late draft"));
            }
            "ime" => e.replace_and_mark_text_in_range(None, "かな", None, w, cx),
            "read-only" => e.update_doc(cx, |d| d.read_only = true),
            _ => panic!("unknown change"),
        });
        editor
    });
    let before = editor.read_with(&bcx, |e, cx| (e.text(cx), e.doc(cx).version, e.marked.clone(), e.doc(cx).disk));
    held.release();
    acx.run_until_parked();
    bcx.run_until_parked();
    assert_eq!(
        editor.read_with(&bcx, |e, cx| (e.text(cx), e.doc(cx).version, e.marked.clone(), e.doc(cx).disk)),
        before
    );
    a.read_with(&acx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), before.0);
        assert!(ws.git.sidebar_open);
        assert!(ws.git.message.as_ref().unwrap().contains("kept"));
        assert_eq!(ws.status.as_ref().unwrap().0, *ws.git.message.as_ref().unwrap());
    });
    assert!(std::fs::read_to_string(&path).unwrap().contains("<<<<<<<"));
    assert_eq!(git::prepare_stashes(&root).unwrap().entries, entries);
}

#[gpui::test]
fn stash_conflict_reload_preserves_late_shared_dirty_read_only_source(cx: &mut TestAppContext) {
    late_conflict_source(cx, "dirty");
}

#[gpui::test]
fn stash_conflict_reload_preserves_late_shared_ime_source(cx: &mut TestAppContext) {
    late_conflict_source(cx, "ime");
}

#[gpui::test]
fn stash_conflict_reload_respects_late_read_only_source(cx: &mut TestAppContext) {
    late_conflict_source(cx, "read-only");
}

#[gpui::test]
fn stash_create_explicit_untracked_toggle_previews_and_restores_chosen_files(cx: &mut TestAppContext) {
    let root = repo("stash-explicit-untracked");
    std::fs::write(root.join("file.txt"), "staged\n").unwrap();
    git(&root, &["add", "file.txt"]);
    std::fs::write(root.join("file.txt"), "unstaged\n").unwrap();
    let untracked = root.join("new 界.txt");
    std::fs::write(&untracked, "chosen untracked 🦀\n").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Stash…", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(matches!(ws.panel, Some(Panel::Form(_)))));
    let toggle = cx.debug_bounds("untracked").unwrap();
    cx.simulate_click(toggle.center(), gpui::Modifiers::none());
    ws.update_in(cx, |ws, w, cx| ws.submit_form(&form::Submit, w, cx));
    cx.run_until_parked();
    assert!(!untracked.exists());
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Preview", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        let preview = editor.read(cx);
        assert!(preview.doc(cx).read_only && preview.doc(cx).scratch);
        assert!(preview.text(cx).contains("+unstaged"));
        assert!(preview.text(cx).contains("+chosen untracked 🦀"));
    });
    open_entry(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Pop and Restore Index", w, cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&untracked).unwrap(), "chosen untracked 🦀\n");
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "unstaged\n");
    assert_eq!(git::index_text(&root, Path::new("file.txt")).unwrap().unwrap(), "staged\n");
    assert!(git::prepare_stashes(&root).unwrap().entries.is_empty());
}
