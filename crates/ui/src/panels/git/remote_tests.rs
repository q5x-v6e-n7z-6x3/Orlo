use super::tests::{git, repo, run_command};
use super::*;
use crate::workspace::ui_tests::{two_windows, window};
use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};

fn choose(ws: &mut Workspace, label: &str, w: &mut Window, cx: &mut Context<Workspace>) {
    let p = ws.quick_panel.as_mut().unwrap();
    p.selected = p.filtered.iter().position(|i| p.items[*i].label == label).unwrap();
    ws.confirm_quick_panel(w, cx);
}
fn setup<'a>(cx: &'a mut TestAppContext, name: &str) -> (PathBuf, Entity<Workspace>, &'a mut VisualTestContext) {
    let root = repo(name);
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    (root, ws, cx)
}

fn object(root: &Path, reference: &str) -> Vec<u8> {
    let result = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", reference])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    result.stdout
}

fn confirm(cx: &mut VisualTestContext, label: &str) {
    cx.run_until_parked();
    cx.simulate_prompt_answer(label);
    cx.run_until_parked();
}

/// Stop after the confirmation callback acquires leases, before the background Git job runs.
fn start_fetch(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    assert_eq!(cx.pending_prompt().unwrap().0, "Fetch");
    ws.read_with(cx, |ws, _| assert!(!ws.git.operation));
    cx.simulate_prompt_answer("Fetch");
    for _ in 0..1000 {
        if ws.read_with(cx, |ws, _| ws.git.operation) {
            return;
        }
        assert!(cx.executor().tick(), "confirmed Fetch must schedule its worker");
    }
    panic!("Fetch confirmation callback did not start");
}

#[gpui::test]
fn remote_palette_form_add_edit_remove_are_local_and_stale_safe(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-form");
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Remotes & Upstream…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| choose(ws, "Add Remote…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        let inputs: Vec<_> = p.inputs().cloned().collect();
        inputs[0].update(cx, |e, cx| e.set_text("origin", cx));
        inputs[1].update(cx, |e, cx| e.set_text("/missing/界 remote", cx));
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert_eq!(git::prepare_remotes(&root).unwrap().remotes[0].fetch_urls, vec!["/missing/界 remote"]);
}

fn command(ws: &Entity<Workspace>, cx: &mut VisualTestContext, label: &str) {
    ws.update_in(cx, |ws, w, cx| run_command(ws, label, w, cx));
    cx.run_until_parked();
}
fn submit(ws: &Entity<Workspace>, cx: &mut VisualTestContext, values: &[&str]) {
    ws.update_in(cx, |ws, w, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        let inputs: Vec<_> = p.inputs().cloned().collect();
        for (e, value) in inputs.iter().zip(values) {
            e.update(cx, |e, cx| e.set_text(value, cx));
        }
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn remote_forms_edit_remove_cancel_and_changed_url_refusal(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-edit-remove");
    git(&root, &["remote", "add", "origin", "/missing-one"]);
    command(&ws, cx, "Git: Remotes & Upstream…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "Edit origin…", w, cx));
    submit(&ws, cx, &["/missing-two", "/push-only"]);
    let c = git::prepare_remotes(&root).unwrap();
    assert_eq!(c.remotes[0].fetch_urls, vec!["/missing-two"]);
    assert_eq!(c.remotes[0].push_urls, vec!["/push-only"]);
    command(&ws, cx, "Git: Remotes & Upstream…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "Remove origin…", w, cx));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(git::prepare_remotes(&root).unwrap().remotes.len(), 1);
    command(&ws, cx, "Git: Remotes & Upstream…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "Edit origin…", w, cx));
    git(&root, &["remote", "set-url", "origin", "/externally-changed"]);
    submit(&ws, cx, &["/should-not-write", ""]);
    assert_eq!(git::prepare_remotes(&root).unwrap().remotes[0].fetch_urls, vec!["/externally-changed"]);
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    command(&ws, cx, "Git: Remotes & Upstream…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "Remove origin…", w, cx));
    cx.simulate_prompt_answer("Remove Remote");
    cx.run_until_parked();
    assert!(git::prepare_remotes(&root).unwrap().remotes.is_empty());
}

#[gpui::test]
fn remote_cancelled_and_replaced_reads_never_publish_late_picker(cx: &mut TestAppContext) {
    let (_, ws, cx) = setup(cx, "remote-late");
    ws.update_in(cx, |ws, w, cx| {
        ws.git_remotes(&GitRemotes, w, cx);
        ws.cancel_quick_panel(cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none()));
    ws.update_in(cx, |ws, w, cx| {
        ws.git_fetch(&GitFetch, w, cx);
        ws.command_palette(&CommandPalette, w, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.label == "Git: Fetch…")));
    ws.update_in(cx, |ws, w, cx| {
        ws.git_remotes(&GitRemotes, w, cx);
        ws.restore_gen += 1;
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none()));
}

#[gpui::test]
fn remote_pull_refuses_shared_dirty_readonly_ime_and_retained_preparations(cx: &mut TestAppContext) {
    let root = repo("remote-shared");
    git(&root, &["remote", "add", "origin", "/missing"]);
    let (a, b, mut acx, mut bcx) = two_windows(cx);
    for (ws, cx) in [(&a, &mut acx), (&b, &mut bcx)] {
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
        });
        cx.run_until_parked();
    }
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| {
            e.update_doc(cx, |d| d.read_only = true);
            e.replace_and_mark_text_in_range(None, "仮", None, w, cx);
        });
    });
    command(&a, &mut acx, "Git: Pull…");
    acx.simulate_prompt_answer("Pull");
    acx.run_until_parked();
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Save or close")));
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| {
            e.unmark_text(w, cx);
            e.edit(cx, |s| s.undo());
        });
    });
    command(&a, &mut acx, "Git: Pull…");
    acx.simulate_prompt_answer("Pull");
    acx.run_until_parked();
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("read-only")));
    let prep = root.join("preparation");
    let held = crate::documents::save::gate::pause(&prep);
    let retained = acx.update(|_, cx| {
        let mut model = crate::editing::text::Document::new();
        model.path = Some(root.join("retained.txt"));
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
        doc
    });
    command(&a, &mut acx, "Git: Pull…");
    acx.simulate_prompt_answer("Pull");
    acx.run_until_parked();
    a.read_with(&acx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("save preparations")));
    held.release();
    acx.run_until_parked();
    assert!(!retained.read_with(&acx, |d, _| d.saving()));
}

#[gpui::test]
fn remote_fetch_keeps_dirty_buffers_and_leases_shared_metadata_after_clear(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-lease");
    let source = repo("remote-lease-source");
    let bare = repo("remote-lease-bare");
    git(&bare, &["config", "core.bare", "true"]);
    let url = bare.join(".git");
    git(&source, &["remote", "add", "origin", url.to_str().unwrap()]);
    // Fixture histories are independent; use a new branch to seed the local bare remote.
    git(&source, &["push", "origin", "main:incoming"]);
    git(&root, &["remote", "add", "origin", url.to_str().unwrap()]);
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("unsaved 界"))));
    let before = ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).text(cx));
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| {
        let p = ws.quick_panel.as_ref().unwrap();
        let item = p.items.iter().find(|i| i.label == "origin").unwrap();
        let quick_panel::ItemAction::Dispatch(action) = &item.action else { panic!() };
        let action = action.as_any().downcast_ref::<GitRemoteChoice>().unwrap().clone();
        ws.cancel_quick_panel(cx);
        ws.git_remote_choice(&action, w, cx);
    });
    start_fetch(&ws, cx);
    ws.update_in(cx, |ws, w, cx| {
        assert!(ws.git.operation);
        assert!(pipelines::writing(&root.join(".git/config"), cx));
        assert!(!pipelines::writing(&root.join("file.txt"), cx), "Fetch permits source saves");
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), before);
        ws.clear_window(w, cx);
        assert!(pipelines::writing(&root.join(".git/config"), cx));
    });
    cx.run_until_parked();
    assert!(git::prepare_remotes(&root).unwrap().upstream_choices.contains(&"origin/incoming".into()));
    assert!(!cx.update(|_, cx| pipelines::writing_under(&root.join(".git"), cx)));
}

#[gpui::test]
fn remote_linked_common_config_unsaved_and_write_lease_refuse_fetch(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-common-config");
    let linked = root.join("linked");
    git(&root, &["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    git(&root, &["remote", "add", "origin", "/missing"]);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join(".git/config"), w, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("# unsaved\n")));
        ws.git.repositories = vec![git::discover(&linked).unwrap().unwrap()];
        ws.git.selected = Some((linked.clone(), PathBuf::from("file.txt")));
    });
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    confirm(cx, "Fetch");
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Save or close")));
    let held = cx.update(|_, cx| pipelines::lease(&root.join(".git"), cx));
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    confirm(cx, "Fetch");
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Wait")));
    drop(held);
}

#[gpui::test]
fn remote_refuses_unsaved_included_git_config_outside_repository(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-include");
    let external = repo("remote-external-config").join("included.gitconfig");
    std::fs::write(&external, "[remote \"origin\"]\nurl = /missing\nfetch = +refs/heads/*:refs/remotes/origin/*\n")
        .unwrap();
    git(&root, &["config", "include.path", external.to_str().unwrap()]);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(external.clone(), w, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("# unsaved\n")));
        ws.git.selected = Some((root.clone(), PathBuf::from("file.txt")));
    });
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    confirm(cx, "Fetch");
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Save or close")));
}

#[gpui::test]
fn remote_push_form_upstream_and_pull_confirmed_fast_forward(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-push-pull");
    let bare = repo("remote-push-pull-bare");
    git(&bare, &["config", "core.bare", "true"]);
    git(&root, &["remote", "add", "origin", bare.join(".git").to_str().unwrap()]);
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("keep unsaved 界"))));
    let draft = ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).text(cx));
    command(&ws, cx, "Git: Push…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        assert!(p.title().contains(bare.join(".git").to_str().unwrap()), "push must display its URL at submission");
    });
    let toggle = cx.debug_bounds("upstream").unwrap();
    cx.simulate_click(toggle.center(), gpui::Modifiers::none());
    submit(&ws, cx, &["topic"]);
    confirm(cx, "Push");
    assert_eq!(git::prepare_remotes(&root).unwrap().upstream.as_deref(), Some("origin/topic"));
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), draft));
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    let peer = repo("remote-push-pull-peer");
    git(&peer, &["remote", "add", "origin", bare.join(".git").to_str().unwrap()]);
    git(&peer, &["fetch", "origin"]);
    git(&peer, &["switch", "-C", "main", "origin/topic"]);
    std::fs::write(peer.join("file.txt"), "remote update 界\n").unwrap();
    git(&peer, &["commit", "-am", "remote edit"]);
    git(&peer, &["push", "origin", "main:topic"]);
    git(&root, &["config", "pull.ff", "false"]);
    git(&root, &["config", "pull.rebase", "false"]);
    command(&ws, cx, "Git: Pull…");
    let (_, detail) = cx.pending_prompt().unwrap();
    assert!(detail.contains("fast-forward-only"), "{detail}");
    assert!(detail.contains("origin"), "{detail}");
    assert!(detail.contains("refs/heads/topic"), "{detail}");
    assert!(detail.contains(bare.join(".git").to_str().unwrap()), "{detail}");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
    command(&ws, cx, "Git: Pull…");
    cx.simulate_prompt_answer("Pull");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "remote update 界\n");
    assert_eq!(object(&root, "HEAD"), object(&peer, "HEAD"));
    command(&ws, cx, "Git: Remotes & Upstream…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "Clear Upstream", w, cx));
    cx.run_until_parked();
    assert!(git::prepare_remotes(&root).unwrap().upstream.is_none());
    command(&ws, cx, "Git: Remotes & Upstream…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "Set Upstream…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin/topic", w, cx));
    cx.run_until_parked();
    assert_eq!(git::prepare_remotes(&root).unwrap().upstream.as_deref(), Some("origin/topic"));
}

#[gpui::test]
fn remote_form_and_pull_prompt_reject_project_branch_and_selection_changes(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-stale-ui");
    command(&ws, cx, "Git: Remotes & Upstream…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "Add Remote…", w, cx));
    cx.run_until_parked();
    ws.update(cx, |ws, _| ws.roots.push(crate::config::project::Root::new(root.clone())));
    submit(&ws, cx, &["origin", "/missing"]);
    assert!(git::prepare_remotes(&root).unwrap().remotes.is_empty());
    ws.read_with(cx, |ws, _| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        assert!(p.error.as_ref().unwrap().contains("changed"));
    });
    ws.update_in(cx, |ws, w, cx| ws.close_panel(&ClosePanel, w, cx));
    git(&root, &["remote", "add", "origin", "/missing"]);
    command(&ws, cx, "Git: Pull…");
    git(&root, &["switch", "-c", "changed"]);
    cx.simulate_prompt_answer("Pull");
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    command(&ws, cx, "Git: Pull…");
    ws.update(cx, |ws, _| ws.git.selected = Some((root, PathBuf::from("file.txt"))));
    cx.simulate_prompt_answer("Pull");
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("changed")));
}

#[gpui::test]
fn remote_late_worker_clears_progress_without_publishing_to_changed_context(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-late-worker");
    git(&root, &["remote", "add", "origin", "/missing"]);
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| {
        let item = ws.quick_panel.as_ref().unwrap().items.iter().find(|i| i.label == "origin").unwrap();
        let quick_panel::ItemAction::Dispatch(action) = &item.action else { panic!() };
        let action = action.as_any().downcast_ref::<GitRemoteChoice>().unwrap().clone();
        ws.cancel_quick_panel(cx);
        ws.git_remote_choice(&action, w, cx);
    });
    start_fetch(&ws, cx);
    ws.update_in(cx, |ws, _, _| {
        assert!(ws.git.operation);
        ws.git.selected = Some((root.clone(), PathBuf::from("file.txt")));
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(!ws.git.operation);
        assert!(ws.git.message.is_none(), "a stale completion must not leave a running message");
    });
}

#[gpui::test]
fn remote_empty_include_unsaved_settings_refuse_before_transport(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-empty-include");
    let external = repo("remote-empty-include-external").join("empty.config");
    std::fs::write(&external, "# comments only\n\n").unwrap();
    git(&root, &["remote", "add", "origin", "/missing"]);
    git(&root, &["config", "include.path", external.to_str().unwrap()]);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(external.clone(), w, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("[remote \"origin\"]\nurl = /changed\n")));
        ws.git.selected = Some((root.clone(), PathBuf::from("file.txt")));
    });
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    confirm(cx, "Fetch");
    ws.read_with(cx, |ws, _| {
        assert!(!ws.git.operation);
        assert!(ws.status.as_ref().unwrap().0.contains("Save or close"));
    });
    assert_eq!(std::fs::read_to_string(external).unwrap(), "# comments only\n\n");
}

#[gpui::test]
fn remote_worker_leases_empty_and_future_includes_after_window_clear(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-empty-include-lease");
    let external = repo("remote-empty-include-lease-external");
    let empty = external.join("empty.config");
    let future = external.join("future.config");
    std::fs::write(&empty, "# empty\n").unwrap();
    git(&root, &["config", "include.path", empty.to_str().unwrap()]);
    let alias = external.join("future-alias.config");
    std::os::unix::fs::symlink(&future, &alias).unwrap();
    git(&root, &["config", "--add", "include.path", alias.to_str().unwrap()]);
    let bare = repo("remote-empty-include-lease-bare");
    git(&bare, &["config", "core.bare", "true"]);
    git(&root, &["remote", "add", "origin", bare.join(".git").to_str().unwrap()]);
    command(&ws, cx, "Git: Fetch…");
    let gate = crate::documents::save::gate::pause(&future);
    ws.update_in(cx, |ws, w, cx| {
        let item = ws.quick_panel.as_ref().unwrap().items.iter().find(|i| i.label == "origin").unwrap();
        let quick_panel::ItemAction::Dispatch(action) = &item.action else { panic!() };
        let action = action.as_any().downcast_ref::<GitRemoteChoice>().unwrap().clone();
        ws.cancel_quick_panel(cx);
        ws.git_remote_choice(&action, w, cx);
    });
    start_fetch(&ws, cx);
    let saved = ws.update_in(cx, |ws, w, cx| {
        assert!(ws.git.operation);
        assert!(pipelines::writing(&empty, cx));
        assert!(pipelines::writing(&future, cx));
        assert!(pipelines::writing(&alias, cx));
        assert!(!pipelines::writing(&root.join("file.txt"), cx));
        ws.clear_window(w, cx);
        assert!(pipelines::writing(&empty, cx));
        assert!(pipelines::writing(&future, cx));
        assert!(pipelines::writing(&alias, cx));
        let mut model = crate::editing::text::Document::new();
        model.path = Some(future.clone());
        model.rope = Rope::from_str("[remote \"next\"]\nurl = /not-contacted\n");
        model.endings = Some(crate::editing::text::LineEndings::uniform(&model.rope, model.line_ending));
        model.dirty = true;
        let doc = crate::documents::Documents::open(model, cx);
        OpenDocument::save(&doc, None, None, |_| None, |_, _| {}, cx);
        assert!(doc.read(cx).saving());
        assert!(!future.exists());
        doc
    });
    gate.release();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&future).unwrap(), "[remote \"next\"]\nurl = /not-contacted\n");
    assert!(!saved.read_with(cx, |d, _| d.saving()));
    assert!(!cx.update(|_, cx| pipelines::writing(&empty, cx) || pipelines::writing(&future, cx)));
    assert!(git::prepare_remotes(&root).unwrap().upstream_choices.contains(&"origin/main".into()));
}

/// Each branch runs in a separate test process; the host config and parallel tests are untouched.
#[gpui::test]
fn remote_top_level_config_dirty_guards_and_worker_leases_survive_clear(cx: &mut TestAppContext) {
    const CHILD: &str = "ORLO_TEST_UI_CONFIG_ROOTS";
    let Some(base) = std::env::var_os(CHILD).map(PathBuf::from) else {
        for mode in ["defaults", "override", "no-system"] {
            let base = crate::workspace::ui_tests::temp(&format!("remote-roots-child-{mode}"));
            for folder in ["home", "xdg/git"] {
                std::fs::create_dir_all(base.join(folder)).unwrap();
            }
            std::fs::write(base.join("home/.gitconfig"), "# comments only\n").unwrap();
            std::fs::write(base.join("system.config"), "# system comments only\n").unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_CONFIG")) {
                child.env_remove(key);
            }
            child
                .args([
                    "--exact",
                    "panels::git::remote_tests::remote_top_level_config_dirty_guards_and_worker_leases_survive_clear",
                    "--nocapture",
                ])
                .env(CHILD, &base)
                .env("HOME", base.join("home"))
                .env("XDG_CONFIG_HOME", base.join("xdg"))
                .env("GIT_CONFIG_SYSTEM", base.join("system.config"));
            if mode == "override" {
                child.env("GIT_CONFIG_GLOBAL", base.join("override.config"));
            }
            if mode == "no-system" {
                child.env("GIT_CONFIG_NOSYSTEM", "1");
            }
            let result = child.output().unwrap();
            assert!(
                result.status.success(),
                "{mode}: {}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        return;
    };
    let mut roots = match std::env::var_os("GIT_CONFIG_GLOBAL") {
        Some(path) => vec![PathBuf::from(path)],
        None => vec![base.join("home/.gitconfig"), base.join("xdg/git/config")],
    };
    if std::env::var_os("GIT_CONFIG_NOSYSTEM").is_none() {
        roots.push(base.join("system.config"));
    }
    let roots: Vec<_> = roots.iter().map(|p| crate::editing::text::resolve(p)).collect();
    // The actual menu route refuses unsaved content in comments-only and absent roots alike.
    for (index, path) in roots.iter().enumerate() {
        let (root, ws, vcx) = setup(cx, &format!("remote-root-dirty-{index}"));
        git(&root, &["remote", "add", "origin", "/missing-local-fixture"]);
        ws.update_in(vcx, |ws, w, cx| {
            let mut state = EditorState::new();
            state.doc.path = Some(path.clone());
            state.insert("[remote \"origin\"]\nurl = /changed\n");
            ws.add_tab(state, w, cx);
            ws.git.selected = Some((root.clone(), PathBuf::from("file.txt")));
        });
        command(&ws, vcx, "Git: Fetch…");
        ws.update_in(vcx, |ws, w, cx| choose(ws, "origin", w, cx));
        confirm(vcx, "Fetch");
        ws.update_in(vcx, |ws, w, cx| {
            assert!(!ws.git.operation);
            assert!(ws.status.as_ref().unwrap().0.contains("Save or close"), "{:?}", ws.status);
            ws.clear_window(w, cx);
        });
    }

    let (root, ws, vcx) = setup(cx, "remote-root-worker");
    let bare = repo("remote-root-worker-bare");
    git(&bare, &["config", "core.bare", "true"]);
    git(&root, &["remote", "add", "origin", bare.join(".git").to_str().unwrap()]);
    // Gate the native local transport after remote_operation's final context validation.
    // A separate observer releases it, because GPUI's deterministic test executor runs its
    // blocking job on the test thread. No real remote or production test hook is involved.
    let entered = base.join("entered");
    let release = base.join("release");
    let helper = base.join("upload-pack");
    let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
    std::fs::write(&helper, format!("#!/bin/sh\ntouch {}\ni=0\nwhile [ ! -e {} ]; do\n i=$((i+1)); [ $i -lt 1000 ] || exit 1\n sleep 0.01\ndone\nexec git upload-pack \"$@\"\n", quote(&entered), quote(&release))).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    git(&root, &["config", "remote.origin.uploadpack", &quote(&helper)]);
    let originals: Vec<_> = roots.iter().map(|p| (p.clone(), std::fs::read(p).ok())).collect();
    let worker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !entered.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let started = entered.exists();
        let unchanged = originals.iter().all(|(p, before)| std::fs::read(p).ok() == *before);
        std::fs::write(release, "release").unwrap();
        assert!(started, "native transport must have reached the post-validation gate");
        assert!(unchanged, "queued config saves must remain blocked while the cleared window's worker is active");
    });
    command(&ws, vcx, "Git: Fetch…");
    // Route these saves through the real asynchronous path in the test harness.
    let save_gates: Vec<_> = roots.iter().map(|path| crate::documents::save::gate::pause(path)).collect();
    ws.update_in(vcx, |ws, w, cx| {
        let item = ws.quick_panel.as_ref().unwrap().items.iter().find(|i| i.label == "origin").unwrap();
        let quick_panel::ItemAction::Dispatch(action) = &item.action else { panic!() };
        let action = action.as_any().downcast_ref::<GitRemoteChoice>().unwrap().clone();
        ws.cancel_quick_panel(cx);
        ws.git_remote_choice(&action, w, cx);
    });
    start_fetch(&ws, vcx);
    let saved = ws.update_in(vcx, |ws, w, cx| {
        assert!(ws.git.operation);
        for path in &roots {
            assert!(pipelines::writing(path, cx), "{}", path.display());
        }
        let source = ws.active_editor().unwrap();
        source.update(cx, |e, cx| e.edit(cx, |s| s.insert("source save allowed\n")));
        OpenDocument::save(&source.read(cx).document.clone(), None, None, |_| None, |_, _| {}, cx);
        assert!(std::fs::read_to_string(root.join("file.txt")).unwrap().starts_with("source save allowed"));
        ws.clear_window(w, cx);
        roots
            .iter()
            .map(|path| {
                assert!(pipelines::writing(path, cx), "clear must not release {}", path.display());
                let mut model = crate::editing::text::Document::new();
                model.path = Some(path.clone());
                model.rope = Rope::from_str("# saved only after worker release\n");
                model.endings = Some(crate::editing::text::LineEndings::uniform(&model.rope, model.line_ending));
                model.dirty = true;
                let doc = crate::documents::Documents::open(model, cx);
                OpenDocument::save(&doc, None, None, |_| None, |_, _| {}, cx);
                assert!(doc.read(cx).saving());
                doc
            })
            .collect::<Vec<_>>()
    });
    for gate in save_gates {
        gate.release();
    }
    vcx.run_until_parked();
    worker.join().unwrap();
    crate::workspace::ui_tests::settle_until(vcx, |cx| {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));
        saved.iter().all(|doc| !doc.read_with(cx, |d, _| d.saving()))
    });
    for path in &roots {
        assert_eq!(std::fs::read_to_string(path).unwrap(), "# saved only after worker release\n");
        assert!(!vcx.update(|_, cx| pipelines::writing(path, cx)));
    }
    assert!(git::prepare_remotes(&root).unwrap().upstream_choices.contains(&"origin/main".into()));
}

#[gpui::test]
fn remote_fetch_picker_displays_configured_refspec_before_dispatch(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-fetch-target");
    let bare = repo("remote-fetch-target-bare");
    git(&bare, &["config", "core.bare", "true"]);
    let url = bare.join(".git");
    git(&root, &["remote", "add", "origin", url.to_str().unwrap()]);
    let refspec = "+refs/heads/main:refs/remotes/origin/review";
    git(&root, &["config", "remote.origin.fetch", refspec]);
    command(&ws, cx, "Git: Fetch…");
    ws.read_with(cx, |ws, _| {
        let p = ws.quick_panel.as_ref().unwrap();
        let item = p.items.iter().find(|i| i.label == "origin").unwrap();
        assert!(item.detail.contains(url.to_str().unwrap()));
        assert!(item.detail.contains(refspec), "{}", item.detail);
        assert!(!ws.git.operation);
    });
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    confirm(cx, "Fetch");
    assert_eq!(object(&root, "origin/review"), object(&bare, "main"));
}

#[gpui::test]
fn remote_pull_prompt_displays_native_upstream_and_cancel_contacts_nothing(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-pull-target");
    let bare = repo("remote-pull-target-bare");
    git(&bare, &["config", "core.bare", "true"]);
    let url = bare.join(".git");
    git(&root, &["remote", "add", "origin", url.to_str().unwrap()]);
    git(&root, &["config", "remote.origin.fetch", "+refs/heads/published:refs/remotes/origin/review"]);
    git(&root, &["update-ref", "refs/remotes/origin/review", "HEAD"]);
    git(&root, &["branch", "--set-upstream-to=origin/review"]);
    let before = object(&root, "HEAD");
    command(&ws, cx, "Git: Pull…");
    let (_, detail) = cx.pending_prompt().unwrap();
    for expected in
        ["fast-forward-only", "main", "origin/review", "origin", "refs/heads/published", url.to_str().unwrap()]
    {
        assert!(detail.contains(expected), "{expected}: {detail}");
    }
    assert!(!root.join(".git/FETCH_HEAD").exists());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(object(&root, "HEAD"), before);
    assert!(!root.join(".git/FETCH_HEAD").exists());
    ws.read_with(cx, |ws, _| assert!(!ws.git.operation));
    command(&ws, cx, "Git: Push…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        assert_eq!(p.inputs().next().unwrap().read(cx).text(cx), "published");
        assert!(p.title().contains(url.to_str().unwrap()));
    });
}

#[gpui::test]
fn remote_inherited_ssh_cancellation_reports_error_and_releases_worker_leases(cx: &mut TestAppContext) {
    const CHILD: &str = "ORLO_TEST_REMOTE_SSH";
    let Some(base) = std::env::var_os(CHILD).map(PathBuf::from) else {
        let base = crate::workspace::ui_tests::temp("remote-ssh-child");
        let script = base.join("ssh-fixture");
        std::fs::write(&script, "#!/bin/sh\necho $$ > \"$ORLO_TEST_REMOTE_SSH/ssh.pid\"\nprintf '%s\\n' \"$@\" > \"$ORLO_TEST_REMOTE_SSH/ssh.args\"\necho 'Authentication cancelled by fixture SSH command' >&2\nexit 42\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_CONFIG")) {
            child.env_remove(key);
        }
        let result = child
            .args([
                "--exact",
                "panels::git::remote_tests::remote_inherited_ssh_cancellation_reports_error_and_releases_worker_leases",
                "--nocapture",
            ])
            .env(CHILD, &base)
            .env("HOME", base.join("home"))
            .env("XDG_CONFIG_HOME", base.join("xdg"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_SSH_COMMAND", format!("'{}'", script.to_string_lossy().replace('\'', "'\\''")))
            .env("GIT_SSH_VARIANT", "ssh")
            .env("GIT_ASKPASS", "/usr/bin/false")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    };
    let root = repo("remote-ssh-cancel");
    git(&root, &["remote", "add", "origin", "ssh://fixture.invalid/fixture.git"]);
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, w, cx| ws.open_path(root.join("file.txt"), w, cx));
    vcx.run_until_parked();
    command(&ws, vcx, "Git: Refresh");
    command(&ws, vcx, "Git: Remotes & Upstream…");
    command(&ws, vcx, "Git: Fetch…");
    assert!(!base.join("ssh.pid").exists(), "opening, status and local controls must not invoke SSH");
    ws.update_in(vcx, |ws, w, cx| {
        let item = ws.quick_panel.as_ref().unwrap().items.iter().find(|i| i.label == "origin").unwrap();
        let quick_panel::ItemAction::Dispatch(action) = &item.action else { panic!() };
        let action = action.as_any().downcast_ref::<GitRemoteChoice>().unwrap().clone();
        ws.cancel_quick_panel(cx);
        ws.git_remote_choice(&action, w, cx);
    });
    start_fetch(&ws, vcx);
    ws.update_in(vcx, |ws, _, cx| {
        assert!(ws.git.operation);
        assert!(pipelines::writing(&root.join(".git/config"), cx));
    });
    vcx.run_until_parked();
    ws.read_with(vcx, |ws, cx| {
        assert!(!ws.git.operation);
        assert!(ws.git.message.as_ref().unwrap().contains("Authentication cancelled by fixture SSH command"));
        assert!(ws.status.as_ref().unwrap().0.contains("Git failed"));
        assert!(!pipelines::writing_under(&root.join(".git"), cx));
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "one\ntwo\nthree\n");
    });
    assert!(std::fs::read_to_string(base.join("ssh.args")).unwrap().contains("fixture.invalid"));
    let pid = std::fs::read_to_string(base.join("ssh.pid")).unwrap();
    assert!(!std::process::Command::new("/bin/kill").args(["-0", pid.trim()]).output().unwrap().status.success());
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "one\ntwo\nthree\n");
}

#[gpui::test]
fn remote_long_fetch_target_is_confirmed_completely_and_cancel_contacts_nothing(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-long-fetch-target");
    let bare = repo(&format!("remote-long-fetch-{}", "界-target-".repeat(12)));
    git(&bare, &["config", "core.bare", "true"]);
    let url = bare.join(".git");
    let tracking = format!("review-{}", "long-target-".repeat(10));
    let refspec = format!("+refs/heads/main:refs/remotes/origin/{tracking}");
    git(&root, &["remote", "add", "origin", url.to_str().unwrap()]);
    git(&root, &["config", "remote.origin.fetch", &refspec]);
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    let (title, detail) = cx.pending_prompt().expect("long Fetch targets require a wrapped confirmation");
    assert_eq!(title, "Fetch");
    for expected in ["origin", url.to_str().unwrap(), refspec.as_str(), "main", "no upstream"] {
        assert!(detail.contains(expected), "{expected}: {detail}");
    }
    assert!(!root.join(".git/FETCH_HEAD").exists(), "selection must not dispatch transport");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!root.join(".git/FETCH_HEAD").exists());
    ws.read_with(cx, |ws, _| assert!(!ws.git.operation));
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    git(&root, &["remote", "set-url", "origin", "/missing-stale-local-fixture"]);
    confirm(cx, "Fetch");
    assert!(!root.join(".git/FETCH_HEAD").exists());
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    git(&root, &["remote", "set-url", "origin", url.to_str().unwrap()]);
    command(&ws, cx, "Git: Fetch…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Fetch");
    cx.run_until_parked();
    assert_eq!(object(&root, &format!("refs/remotes/origin/{tracking}")), object(&bare, "main"));
}

#[gpui::test]
fn remote_long_push_target_is_confirmed_after_form_and_cancel_contacts_nothing(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "remote-long-push-target");
    let bare = repo(&format!("remote-long-push-{}", "界-target-".repeat(12)));
    git(&bare, &["config", "core.bare", "true"]);
    let url = bare.join(".git");
    let branch = format!("published-{}", "long-target-".repeat(10));
    git(&root, &["remote", "add", "origin", url.to_str().unwrap()]);
    command(&ws, cx, "Git: Push…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    submit(&ws, cx, &[&branch]);
    let (title, detail) = cx.pending_prompt().expect("long Push targets require a wrapped confirmation");
    assert_eq!(title, "Push");
    for expected in ["origin", url.to_str().unwrap(), "refs/heads/main", &format!("refs/heads/{branch}"), "no upstream"]
    {
        assert!(detail.contains(expected), "{expected}: {detail}");
    }
    let target = bare.join(".git/refs/heads").join(&branch);
    assert!(!target.exists(), "form submission must not dispatch transport");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!target.exists());
    ws.read_with(cx, |ws, _| assert!(!ws.git.operation));
    command(&ws, cx, "Git: Push…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    submit(&ws, cx, &[&branch]);
    git(&root, &["config", "remote.origin.pushurl", "/missing-stale-local-fixture"]);
    confirm(cx, "Push");
    assert!(!target.exists());
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    git(&root, &["config", "--unset", "remote.origin.pushurl"]);
    command(&ws, cx, "Git: Push…");
    ws.update_in(cx, |ws, w, cx| choose(ws, "origin", w, cx));
    cx.run_until_parked();
    submit(&ws, cx, &[&branch]);
    cx.simulate_prompt_answer("Push");
    cx.run_until_parked();
    assert_eq!(object(&bare, &branch), object(&root, "HEAD"));
}
