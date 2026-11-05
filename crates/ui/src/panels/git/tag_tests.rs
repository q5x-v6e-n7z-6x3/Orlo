use super::tests::{git, repo, run_command};
use super::*;
use crate::workspace::ui_tests::window;
use gpui::{TestAppContext, VisualTestContext};

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
fn choose(ws: &mut Workspace, label: &str, w: &mut Window, cx: &mut Context<Workspace>) {
    let p = ws.quick_panel.as_mut().unwrap();
    p.selected = p.filtered.iter().position(|i| p.items[*i].label == label).unwrap();
    ws.confirm_quick_panel(w, cx);
}
fn fields(ws: &Workspace) -> Vec<Entity<Editor>> {
    let Some(Panel::Form(p)) = &ws.panel else { panic!("missing tag form: {:?}", ws.status) };
    p.inputs().cloned().collect()
}
fn fill(ws: &mut Workspace, values: &[&str], w: &mut Window, cx: &mut Context<Workspace>) {
    let fields = fields(ws);
    assert_eq!(fields.len(), values.len());
    for (e, value) in fields.iter().zip(values) {
        e.update(cx, |e, cx| e.set_text(value, cx));
    }
    ws.submit_form(&form::Submit, w, cx);
}
fn create_form(ws: &Entity<Workspace>, cx: &mut VisualTestContext, revision: &str) {
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Local Tag…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        assert_eq!(fields(ws)[0].read(cx).text(cx), "HEAD");
        fill(ws, &[revision], w, cx);
    });
    cx.run_until_parked();
}
fn list(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Local Tags…", w, cx));
    cx.run_until_parked();
}

#[gpui::test]
fn tags_review_completion_never_fetches_missing_index_blobs(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let (root, ws, cx) = setup(cx, "tags-promisor-completion");
    let remote = repo("tags-promisor-source");
    git(&remote, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    git(&remote, &["config", "uploadpack.allowFilter", "true"]);
    let output =
        std::process::Command::new("git").arg("-C").arg(&root).args(["rev-parse", "HEAD:file.txt"]).output().unwrap();
    assert!(output.status.success());
    let blob = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let marker = root.join(".git/tag-completion-contacted");
    let ssh = root.join(".git/tag-completion-local-ssh");
    let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
    std::fs::write(
        &ssh,
        format!("#!/bin/sh\nprintf contacted >> {}\nexec git upload-pack {}\n", quote(&marker), quote(&remote)),
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    git(&root, &["remote", "add", "origin", "ssh://orlo-local-fixture/unused"]);
    git(&root, &["config", "core.sshCommand", &quote(&ssh)]);
    git(&root, &["config", "ssh.variant", "simple"]);
    // Git's promisor remote enables lazy fetch without an extension that libgit2
    // cannot open. The native-Git positive control below proves that it is active.
    git(&root, &["config", "remote.origin.promisor", "true"]);
    create_form(&ws, cx, "HEAD");
    std::fs::remove_file(root.join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
    ws.update_in(cx, |ws, w, cx| fill(ws, &["local-only", ""], w, cx));
    cx.run_until_parked();
    assert!(!marker.exists(), "tag success completion must not fetch unrelated index/diff blobs");
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("completed")));
    create_form(&ws, cx, "HEAD");
    ws.update_in(cx, |ws, w, cx| fill(ws, &["local-only", ""], w, cx));
    cx.run_until_parked();
    assert!(!marker.exists(), "tag error completion must not fetch unrelated blobs either");
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("already exists")));
    list(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "local-only", w, cx));
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    assert!(!marker.exists(), "tag deletion completion must stay reference-only");
    ws.read_with(cx, |ws, _| {
        assert!(git::prepare_tags(&root).unwrap().entries.is_empty(), "tag deletion failed: {:?}", ws.git.message);
    });
    let control = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["cat-file", "blob", &blob])
        .env("GIT_NO_LAZY_FETCH", "0")
        .env_remove("GIT_ALLOW_PROTOCOL")
        .output()
        .unwrap();
    assert!(control.status.success(), "{}", String::from_utf8_lossy(&control.stderr));
    assert!(marker.exists(), "the local promisor positive control must contact its fixture");
    assert_eq!(control.stdout, b"one\ntwo\nthree\n");
}

#[gpui::test]
fn tags_review_metadata_is_rendered_and_scrollable_in_a_narrow_picker(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "tags-rendered-metadata");
    cx.simulate_resize(gpui::size(gpui::px(400.), gpui::px(300.)));
    let message = format!("Release 界\n{}\nTAG MESSAGE ENDPOINT", "annotation body 🦀 ".repeat(100));
    git(&root, &["-c", "tag.gpgSign=false", "tag", "--cleanup=verbatim", "-a", "annotated/界", "-m", &message]);
    let target = git::prepare_tags(&root).unwrap().entries[0].target.clone();
    list(&ws, cx);
    ws.update_in(cx, |ws, _, cx| {
        let p = ws.quick_panel.as_mut().unwrap();
        p.selected = p.filtered.iter().position(|i| p.items[*i].label == "annotated/界").unwrap();
        let item = &p.items[p.filtered[p.selected]];
        assert!(item.detail.contains("Annotated") && item.detail.contains("Release 界"));
        assert!(item.detail.contains("TAG MESSAGE ENDPOINT"));
        let (_, rendered) = p.selected_detail().unwrap();
        assert!(rendered.contains("annotated/界\nAnnotated") && rendered.contains(&target));
        assert!(rendered.contains(&message), "render the entire multiline message, including its endpoint");
        cx.notify();
    });
    cx.run_until_parked();
    let details = cx.debug_bounds("choice-detail").expect("tag metadata needs an actual rendered detail area");
    let body = cx.debug_bounds("choice-detail-text").unwrap();
    let row = cx.debug_bounds("qp-row-1").unwrap();
    let label = cx.debug_bounds("qp-label-1").unwrap();
    assert!(
        label.top() >= row.top() && label.bottom() <= row.bottom(),
        "a multiline annotation must not move the tag name outside its selectable row: {label:?} vs {row:?}"
    );
    assert!(body.size.height > details.size.height, "the full multiline annotation must wrap and scroll");
    assert!(
        body.size.width <= details.size.width
            && details.right() <= gpui::px(400.)
            && details.bottom() <= gpui::px(300.)
    );
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: details.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-100_000.))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let tail = cx.debug_bounds("choice-detail-text").unwrap();
    assert!(
        tail.top() < body.top() && tail.bottom() <= details.bottom() + gpui::px(1.) && tail.bottom() > details.top(),
        "the complete annotation endpoint must be visible after scrolling"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(ws.read_with(cx, |ws, _| ws.quick_panel.is_none()));
    assert_eq!(git::prepare_tags(&root).unwrap().entries.len(), 1, "reading metadata cannot delete its tag");
}

#[gpui::test]
fn tags_menu_catalog_forms_and_picker_create_local_types_and_show_metadata(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "tags-create");
    let first = git::prepare_tag(&root, &git::prepare_tags(&root).unwrap(), "HEAD").unwrap().commit().to_owned();
    std::fs::write(root.join("file.txt"), "next\n").unwrap();
    git(&root, &["commit", "-qam", "next"]);
    std::fs::write(root.join("file.txt"), "dirty saved source\n").unwrap();
    let before = git::discover(&root).unwrap();
    let index = git::index_text(&root, Path::new("file.txt")).unwrap();
    git(&root, &["config", "tag.gpgSign", "true"]);
    git(&root, &["config", "gpg.program", "/orlo-no-tag-signing"]);
    git(&root, &["config", "core.editor", "false"]);
    ws.update_in(cx, |ws, w, cx| {
        let git = ws.menus(cx).into_iter().find(|m| m.name == "Git").unwrap();
        let items = git
            .items
            .into_iter()
            .find_map(|i| match i {
                MenuItem::Submenu(m) if m.name == "Tags" => Some(m.items),
                _ => None,
            })
            .unwrap();
        for label in ["Local Tags…", "Create Local Tag…"] {
            assert!(items.iter().any(|i| matches!(i, MenuItem::Action { name, .. } if name == label)));
        }
        let action = items
            .into_iter()
            .find_map(|i| match i {
                MenuItem::Action { name, action, .. } if name == "Create Local Tag…" => Some(action),
                _ => None,
            })
            .unwrap();
        w.dispatch_action(action, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| fill(ws, &["HEAD~1"], w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| fill(ws, &["annotated/界", "Release 界\n\nBody 🦀"], w, cx));
    cx.run_until_parked();
    list(&ws, cx);
    ws.update_in(cx, |ws, w, cx| {
        let p = ws.quick_panel.as_ref().unwrap();
        let item = p.items.iter().find(|i| i.label == "annotated/界").unwrap();
        assert!(item.detail.contains("Annotated"));
        assert!(item.detail.contains(&first));
        assert!(item.detail.contains("Body 🦀"));
        choose(ws, "Create Local Tag…", w, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| fill(ws, &["HEAD"], w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| fill(ws, &["lightweight", ""], w, cx));
    cx.run_until_parked();
    list(&ws, cx);
    ws.read_with(cx, |ws, _| {
        let p = ws.quick_panel.as_ref().unwrap();
        assert!(p.items.iter().any(|i| i.label == "lightweight" && i.detail.contains("Lightweight")));
    });
    let tags = git::prepare_tags(&root).unwrap();
    assert_eq!(tags.entries.len(), 2);
    assert!(tags.entries[0].annotated);
    assert_eq!(tags.entries[0].target, first);
    assert!(!tags.entries[1].annotated);
    assert_eq!(git::discover(&root).unwrap(), before);
    assert_eq!(git::index_text(&root, Path::new("file.txt")).unwrap(), index);
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "dirty saved source\n");
}

#[gpui::test]
fn tags_creation_errors_and_stale_head_leave_refs_unchanged(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "tags-errors");
    for (name, error) in [("invalid name", "Invalid local tag name"), ("same", "completed"), ("same", "already exists")]
    {
        create_form(&ws, cx, "HEAD");
        ws.update_in(cx, |ws, w, cx| fill(ws, &[name, ""], w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains(error), "{:?}", ws.git.message));
    }
    let before = git::prepare_tags(&root).unwrap().entries;
    create_form(&ws, cx, "HEAD");
    std::fs::write(root.join("file.txt"), "changed\n").unwrap();
    git(&root, &["commit", "-qam", "changed"]);
    ws.update_in(cx, |ws, w, cx| fill(ws, &["stale", ""], w, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    assert_eq!(git::prepare_tags(&root).unwrap().entries, before);
    for revision in ["missing", "HEAD:file.txt"] {
        create_form(&ws, cx, revision);
        ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("Cannot resolve a commit")));
    }
    assert_eq!(git::prepare_tags(&root).unwrap().entries, before);
}

#[gpui::test]
fn tags_delete_cancel_stale_raw_annotation_and_selected_only(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "tags-delete");
    git(&root, &["-c", "tag.gpgSign=false", "tag", "-a", "selected", "-m", "original"]);
    git(&root, &["-c", "tag.gpgSign=false", "tag", "keep"]);
    list(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "selected", w, cx));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(git::prepare_tags(&root).unwrap().entries.len(), 2);
    list(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "selected", w, cx));
    git(&root, &["-c", "tag.gpgSign=false", "tag", "-fa", "selected", "-m", "replacement"]);
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.git.message.as_ref().unwrap().contains("changed")));
    let replacement = git::prepare_tags(&root).unwrap();
    assert_eq!(replacement.entries.len(), 2);
    assert!(replacement.entries[1].message.contains("replacement"));
    list(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "selected", w, cx));
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    assert_eq!(git::prepare_tags(&root).unwrap().entries, vec![replacement.entries[0].clone()]);
}

#[gpui::test]
fn tags_project_change_refuses_form_and_pending_confirmation(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "tags-project-stale");
    create_form(&ws, cx, "HEAD");
    ws.update_in(cx, |ws, w, cx| {
        ws.restore_gen += 1;
        fill(ws, &["must-not-create", ""], w, cx);
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        assert!(p.error.as_ref().unwrap().contains("changed"));
    });
    assert!(git::prepare_tags(&root).unwrap().entries.is_empty());
    git(&root, &["-c", "tag.gpgSign=false", "tag", "keep"]);
    list(&ws, cx);
    ws.update_in(cx, |ws, w, cx| choose(ws, "keep", w, cx));
    ws.update(cx, |ws, _| ws.restore_gen += 1);
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    assert_eq!(git::prepare_tags(&root).unwrap().entries.len(), 1);
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("changed")));
}

#[gpui::test]
fn tags_late_preparation_and_resolution_preserve_new_palette(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "tags-late");
    ws.update_in(cx, |ws, w, cx| {
        ws.git_tags(&GitTags, w, cx);
        ws.command_palette(&CommandPalette, w, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.label == "Git: Local Tags…"))
    });
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Create Local Tag…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        fill(ws, &["HEAD"], w, cx);
        ws.command_palette(&CommandPalette, w, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.panel.is_none());
        assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.label == "Git: Local Tags…"));
    });
    assert!(git::prepare_tags(&root).unwrap().entries.is_empty());
}

#[gpui::test]
fn tags_unborn_head_reports_resolution_error_without_creating_tag(cx: &mut TestAppContext) {
    let (root, ws, cx) = setup(cx, "tags-unborn");
    git(&root, &["update-ref", "-d", "refs/heads/main"]);
    create_form(&ws, cx, "HEAD");
    ws.read_with(cx, |ws, _| assert!(ws.status.as_ref().unwrap().0.contains("unborn")));
    assert!(git::prepare_tags(&root).unwrap().entries.is_empty());
}
