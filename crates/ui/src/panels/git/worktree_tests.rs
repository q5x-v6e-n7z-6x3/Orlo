use super::tests::{git, repo, run_command};
use super::*;
use crate::workspace::ui_tests::{two_windows, window};
use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};

fn choose(ws: &mut Workspace, label: &str, w: &mut Window, cx: &mut Context<Workspace>) {
    let p = ws.quick_panel.as_mut().unwrap();
    p.selected = p.filtered.iter().position(|i| p.items[*i].label == label).unwrap();
    ws.confirm_quick_panel(w, cx);
}
fn setup<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
) -> (PathBuf, PathBuf, Entity<Workspace>, &'a mut VisualTestContext) {
    let root = repo(name);
    let path = root.join("linked 界 space\nline");
    git(&root, &["worktree", "add", "-b", "topic", path.to_str().unwrap()]);
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    (root, path, ws, cx)
}
fn menu(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &Path) {
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Worktrees…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| choose(ws, &path.display().to_string(), w, cx));
}

#[gpui::test]
fn worktree_open_route_preserves_current_unsaved_composition_and_session(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-open");
    let project = root.join("Example.sublime-project");
    std::fs::write(&project, r#"{"folders":[{"path":"."}]}"#).unwrap();
    ws.update_in(cx, |ws, w, cx| {
        ws.clear_window(w, cx);
        ws.open_project_path(&OpenProjectPath { path: project.clone(), here: false }, w, cx);
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| e.replace_and_mark_text_in_range(None, "仮", None, w, cx));
    });
    let original = ws.read_with(cx, |ws, cx| {
        (
            ws.active_editor().unwrap(),
            ws.active_editor().unwrap().read(cx).text(cx),
            ws.project.as_ref().map(|p| p.path.clone()),
            ws.restore_gen,
        )
    });
    menu(&ws, cx, &path);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Open in New Window", w, cx));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2);
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor(), Some(original.0.clone()));
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), original.1);
        assert!(!ws.active_editor().unwrap().read(cx).marked.is_empty());
        assert_eq!(ws.project.as_ref().map(|p| p.path.clone()), Some(project));
        assert_eq!(ws.project.as_ref().map(|p| p.path.clone()), original.2);
        assert_eq!(ws.restore_gen, original.3);
        assert!(ws.other_workspaces(cx).any(|w| w.roots.iter().any(|r| r.path == path)));
        assert_eq!(ws.git_root(cx), Some(root));
    });
}

#[gpui::test]
fn worktree_remove_confirmation_cancel_stale_and_success(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-remove");
    menu(&ws, cx, &path);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Remove…", w, cx));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(path.exists());
    menu(&ws, cx, &path);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Remove…", w, cx));
    git(&root, &["worktree", "lock", path.to_str().unwrap()]);
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert!(path.exists());
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    git(&root, &["worktree", "unlock", path.to_str().unwrap()]);
    menu(&ws, cx, &path);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Remove…", w, cx));
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert!(!path.exists());
    assert!(git::branches(&root).unwrap().contains(&"topic".into()));
}

#[gpui::test]
fn worktree_create_forms_existing_branch_and_prune_confirmed_registrations(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-create-prune");
    let destination = root.join("new 界");
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Worktree…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        choose(ws, "New Branch at HEAD…", w, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        let inputs: Vec<_> = p.inputs().cloned().collect();
        inputs[0].update(cx, |e, cx| e.set_text("new-topic", cx));
        inputs[1].update(cx, |e, cx| e.set_text(destination.to_str().unwrap(), cx));
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert!(destination.join("file.txt").exists());
    assert_eq!(cx.windows().len(), 1);
    std::fs::remove_dir_all(&destination).unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Prune Worktrees…", w, cx));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(git::prepare_worktrees(&root).unwrap().entries.len(), 3);
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Prune Worktrees…", w, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Prune");
    cx.run_until_parked();
    assert_eq!(git::prepare_worktrees(&root).unwrap().entries.len(), 1);
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Worktree…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        choose(ws, "new-topic", w, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        p.inputs().next().unwrap().update(cx, |e, cx| e.set_text(destination.to_str().unwrap(), cx));
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert!(destination.join("file.txt").exists());
}

#[gpui::test]
fn worktree_remove_refuses_other_window_dirty_shared_readonly_ime_and_writes(cx: &mut TestAppContext) {
    let root = repo("worktree-shared");
    let path = root.join("linked");
    git(&root, &["worktree", "add", "-b", "topic", path.to_str().unwrap()]);
    let (a, b, mut acx, mut bcx) = two_windows(cx);
    for (ws, cx) in [(&a, &mut acx), (&b, &mut bcx)] {
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(path.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git.selected = Some((root.clone(), PathBuf::from("file.txt")));
        });
        cx.run_until_parked();
    }
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| {
            e.update_doc(cx, |d| d.read_only = true);
            e.replace_and_mark_text_in_range(None, "仮", None, w, cx);
        })
    });
    menu(&a, &mut acx, &path);
    a.update_in(&mut acx, |ws, w, cx| choose(ws, "Remove…", w, cx));
    acx.simulate_prompt_answer("Remove");
    acx.run_until_parked();
    assert!(path.exists());
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Save or close")));
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| {
            e.unmark_text(w, cx);
            e.edit(cx, |s| s.undo());
        })
    });
    let lease = acx.update(|_, cx| pipelines::lease(&path, cx));
    menu(&a, &mut acx, &path);
    a.update_in(&mut acx, |ws, w, cx| choose(ws, "Remove…", w, cx));
    acx.simulate_prompt_answer("Remove");
    acx.run_until_parked();
    assert!(path.exists());
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Wait")));
    drop(lease);
}

#[gpui::test]
fn worktree_pending_reads_and_confirmation_reject_changed_surfaces_selection_and_project(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-stale-ui");
    ws.update_in(cx, |ws, w, cx| {
        ws.git_worktrees(&GitWorktrees, w, cx);
        ws.cancel_quick_panel(cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none()));
    ws.update_in(cx, |ws, w, cx| {
        ws.git_create_worktree(&GitCreateWorktree, w, cx);
        ws.command_palette(&CommandPalette, w, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.label == "Git: Worktrees…"))
    });
    ws.update_in(cx, |ws, w, cx| {
        ws.git_create_worktree(&GitCreateWorktree, w, cx);
        ws.git.selected = Some((root.clone(), PathBuf::from("file.txt")));
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none()));
    menu(&ws, cx, &path);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Remove…", w, cx));
    ws.update(cx, |ws, _| ws.restore_gen += 1);
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert!(path.join("file.txt").exists());
}

#[gpui::test]
fn worktree_create_form_refuses_changed_roots_and_replaced_repository(cx: &mut TestAppContext) {
    let (root, _, ws, cx) = setup(cx, "worktree-stale-form");
    let destination = root.join("new");
    for replace in [false, true] {
        ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Worktree…", w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| choose(ws, "New Branch at HEAD…", w, cx));
        cx.run_until_parked();
        if replace {
            std::fs::rename(root.join(".git"), root.join("old-git")).unwrap();
            git(&root, &["init", "-q"]);
        } else {
            ws.update(cx, |ws, _| ws.roots.push(crate::config::project::Root::new(root.clone())));
        }
        ws.update_in(cx, |ws, w, cx| {
            let Some(Panel::Form(p)) = &ws.panel else { panic!() };
            let fields: Vec<_> = p.inputs().cloned().collect();
            fields[0].update(cx, |e, cx| e.set_text("new", cx));
            fields[1].update(cx, |e, cx| e.set_text(destination.to_str().unwrap(), cx));
            ws.submit_form(&form::Submit, w, cx);
        });
        cx.run_until_parked();
        assert!(!destination.exists());
        ws.read_with(cx, |ws, _| {
            let error = if let Some(Panel::Form(p)) = &ws.panel {
                p.error.as_ref().unwrap()
            } else {
                ws.git.message.as_ref().unwrap()
            };
            assert!(error.contains("changed"), "{error}");
        });
        ws.update_in(cx, |ws, w, cx| ws.close_panel(&ClosePanel, w, cx));
    }
}

#[gpui::test]
fn worktree_remove_refuses_retained_save_preparation(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-save-preparation");
    let prep = root.join("preparation");
    let held = crate::documents::save::gate::pause(&prep);
    let retained = cx.update(|_, cx| {
        let mut model = crate::editing::text::Document::new();
        model.path = Some(path.join("retained 界.txt"));
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
        assert!(!pipelines::writing_under(&path, cx));
        doc
    });
    menu(&ws, cx, &path);
    ws.update_in(cx, |ws, w, cx| choose(ws, "Remove…", w, cx));
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    assert!(path.exists());
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("save preparations")));
    held.release();
    cx.run_until_parked();
    assert!(!retained.read_with(cx, |d, _| d.saving()));
}

#[gpui::test]
fn worktree_create_worker_leases_sibling_destination_after_window_clear(cx: &mut TestAppContext) {
    let (root, _, ws, cx) = setup(cx, "worktree-destination-lease");
    let destination = root.with_file_name(format!("{}-sibling 界", root.file_name().unwrap().to_str().unwrap()));
    assert!(!destination.exists());
    let alias = root.join("parent-alias");
    std::os::unix::fs::symlink(root.parent().unwrap(), &alias).unwrap();
    let entered = alias.join(destination.file_name().unwrap());
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Worktree…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| choose(ws, "New Branch at HEAD…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        let inputs: Vec<_> = p.inputs().cloned().collect();
        inputs[0].update(cx, |e, cx| e.set_text("sibling", cx));
        inputs[1].update(cx, |e, cx| e.set_text(entered.to_str().unwrap(), cx));
        ws.submit_form(&form::Submit, w, cx);
        assert!(ws.git.operation);
        assert!(pipelines::writing(&destination.join("file.txt"), cx));
        ws.clear_window(w, cx);
        assert!(!ws.git.operation);
        assert!(pipelines::writing(&destination.join("file.txt"), cx), "queued worker owns destination after clear");
        assert!(pipelines::writing_under(&root, cx));
    });
    cx.run_until_parked();
    assert!(destination.join("file.txt").exists());
    assert!(!cx.update(|_, cx| pipelines::writing_under(&destination, cx)));
    // All fixture cleanup is confined to the fresh disposable repository and its sibling.
    git(&root, &["worktree", "remove", destination.to_str().unwrap()]);
}

#[gpui::test]
fn worktree_prune_refuses_changed_registration_while_confirmation_is_pending(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-prune-stale");
    std::fs::remove_dir_all(&path).unwrap();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Prune Worktrees…", w, cx));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    git(&root, &["worktree", "add", "-b", "another", root.join("another").to_str().unwrap()]);
    cx.simulate_prompt_answer("Prune");
    cx.run_until_parked();
    assert_eq!(git::prepare_worktrees(&root).unwrap().entries.len(), 3);
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
}

#[gpui::test]
fn worktree_list_shows_native_prunable_reason_separately_from_missing_and_locked(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-native-prunable-list");
    let locked = root.join("locked 界");
    git(&root, &["worktree", "add", "-b", "locked-topic", locked.to_str().unwrap()]);
    git(&root, &["worktree", "lock", "--reason", "portable 界\nkeep", locked.to_str().unwrap()]);
    std::fs::remove_dir_all(&path).unwrap();
    std::fs::remove_dir_all(&locked).unwrap();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Worktrees…", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let items = &ws.quick_panel.as_ref().unwrap().items;
        let missing = items.iter().find(|i| i.label == path.display().to_string()).unwrap();
        assert!(missing.detail.contains("topic") && missing.detail.contains("missing"));
        assert!(missing.detail.contains("prunable: gitdir file points to non-existent location"), "{}", missing.detail);
        let protected = items.iter().find(|i| i.label == locked.display().to_string()).unwrap();
        assert!(protected.detail.contains("missing") && protected.detail.contains("locked: portable 界\nkeep"));
        assert!(!protected.detail.contains("prunable"));
        let main = items.iter().find(|i| i.label == root.display().to_string()).unwrap();
        assert!(main.detail.contains("main") && main.detail.contains("current"));
        assert!(!main.detail.contains("prunable") && !main.detail.contains("missing"));
    });
}

#[gpui::test]
fn worktree_prune_confirmation_shows_native_eligible_reason_and_retains_locked_metadata(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-native-prunable-confirm");
    let locked = root.join("locked 界");
    let live = root.join("live 界");
    for (folder, branch) in [(&locked, "locked-topic"), (&live, "live-topic")] {
        git(&root, &["worktree", "add", "-b", branch, folder.to_str().unwrap()]);
    }
    git(&root, &["worktree", "lock", locked.to_str().unwrap()]);
    std::fs::remove_dir_all(&path).unwrap();
    std::fs::remove_dir_all(&locked).unwrap();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Prune Worktrees…", w, cx));
    cx.run_until_parked();
    let (_, detail) = cx.pending_prompt().expect("explicit prune confirmation");
    assert!(detail.contains(&path.display().to_string()));
    assert!(detail.contains("prunable: gitdir file points to non-existent location"), "{detail}");
    assert!(!detail.contains(&locked.display().to_string()) && !detail.contains(&live.display().to_string()));
    cx.simulate_prompt_answer("Prune");
    cx.run_until_parked();
    let remaining = git::prepare_worktrees(&root).unwrap();
    assert_eq!(remaining.entries.len(), 3);
    assert!(!remaining.entries.iter().any(|e| e.path == path));
    assert!(remaining.entries.iter().any(|e| e.path == locked && e.locked.is_some()));
    assert_eq!(std::fs::read_to_string(live.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    assert!(git::branches(&root).unwrap().contains(&"topic".into()));
}

#[gpui::test]
fn worktree_explicit_new_and_existing_creation_then_open_preserve_source_and_checked_out_branches(
    cx: &mut TestAppContext,
) {
    let (root, linked, ws, cx) = setup(cx, "worktree-create-then-open");
    git(&root, &["branch", "available"]);
    let original = ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("only unsaved ")));
        (ws.active_editor().unwrap(), ws.restore_gen)
    });
    for (index, label, branch) in [(0, "New Branch at HEAD…", "explicit-topic"), (1, "available", "available")] {
        let destination = root.join(format!("created-{index} 界"));
        ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Worktree…", w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| choose(ws, label, w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let Some(Panel::Form(p)) = &ws.panel else { panic!() };
            let inputs: Vec<_> = p.inputs().cloned().collect();
            assert_eq!(inputs.len(), if index == 0 { 2 } else { 1 });
            if index == 0 {
                inputs[0].update(cx, |e, cx| e.set_text(branch, cx));
            }
            inputs.last().unwrap().update(cx, |e, cx| e.set_text(destination.to_str().unwrap(), cx));
            ws.submit_form(&form::Submit, w, cx);
        });
        cx.run_until_parked();
        assert_eq!(git::discover(&destination).unwrap().unwrap().branch, branch);
        assert_eq!(cx.windows().len(), index + 1, "creation keeps the current window");
        menu(&ws, cx, &destination);
        ws.update_in(cx, |ws, w, cx| choose(ws, "Open in New Window", w, cx));
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), index + 2);
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor(), Some(original.0.clone()));
            assert_eq!(ws.restore_gen, original.1);
            assert_eq!(original.0.read(cx).text(cx), "only unsaved one\ntwo\nthree\n");
            assert!(original.0.read(cx).doc(cx).unsaved());
            assert!(ws.other_workspaces(cx).any(|w| w.roots.iter().any(|r| r.path == destination)));
        });
    }
    let refused = root.join("refused");
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Worktree…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| choose(ws, "topic", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        p.inputs().next().unwrap().update(cx, |e, cx| e.set_text(refused.to_str().unwrap(), cx));
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert!(!refused.exists());
    assert_eq!(cx.windows().len(), 3);
    assert_eq!(git::discover(&root).unwrap().unwrap().branch, "main");
    assert_eq!(git::discover(&linked).unwrap().unwrap().branch, "topic");
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("already")));
}

#[gpui::test]
fn worktree_prune_rechecks_new_native_lock_and_has_no_prompt_for_only_ineligible_entries(cx: &mut TestAppContext) {
    let (root, path, ws, cx) = setup(cx, "worktree-prune-native-lock");
    std::fs::remove_dir_all(&path).unwrap();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Prune Worktrees…", w, cx));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    git(&root, &["worktree", "lock", "--reason", "keep missing", path.to_str().unwrap()]);
    cx.simulate_prompt_answer("Prune");
    cx.run_until_parked();
    assert_eq!(git::prepare_worktrees(&root).unwrap().entries.len(), 2);
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Prune Worktrees…", w, cx));
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("No prunable")));
}
