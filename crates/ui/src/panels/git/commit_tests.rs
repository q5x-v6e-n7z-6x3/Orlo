use super::tests::{git, repo, run_command};
use super::*;
use crate::workspace::ui_tests::window;
use gpui::TestAppContext;

fn fields(ws: &Workspace) -> Vec<Entity<Editor>> {
    let Some(Panel::Form(p)) = &ws.panel else { panic!("commit form missing") };
    p.inputs().cloned().collect()
}

fn checked_git(root: &Path, args: &[&str]) -> String {
    let result = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    String::from_utf8(result.stdout).unwrap()
}

#[gpui::test]
fn composition_optional_author_is_validated_retained_and_applied_only_when_explicit(cx: &mut TestAppContext) {
    let root = repo("composition-optional-author");
    git(&root, &["commit", "--amend", "--no-edit", "--author=Original 界 <original@example.invalid>"]);
    let original = checked_git(&root, &["show", "-s", "--format=%an <%ae>"]);
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| run_command(ws, "Git: Amend HEAD…", w, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let fields = fields(ws);
        assert_eq!(fields.len(), 5, "explicit optional author name/email controls must be reachable");
        assert!(fields[3].read(cx).text(cx).is_empty());
        assert!(fields[4].read(cx).text(cx).is_empty());
        fields[0].update(cx, |e, cx| e.set_text("Preserved author 界", cx));
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert_eq!(checked_git(&root, &["show", "-s", "--format=%an <%ae>"]), original);
    ws.update_in(cx, |ws, w, cx| ws.git_amend(&GitAmend, w, cx));
    cx.run_until_parked();
    let before = checked_git(&root, &["rev-parse", "HEAD"]);
    ws.update_in(cx, |ws, w, cx| {
        let fields = fields(ws);
        fields[3].update(cx, |e, cx| e.set_text("New Zoë 界", cx));
        ws.submit_form(&form::Submit, w, cx);
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        assert!(p.error.as_ref().unwrap().contains("author"));
        assert!(!ws.git.operation);
        fields[4].update(cx, |e, cx| e.set_text("new@example.invalid", cx));
        fields[2].update(cx, |e, cx| {
            e.set_text("Co 界 <co@example.invalid>\nOther <other@example.invalid>\nCo 界 <co@example.invalid>", cx)
        });
        ws.close_panel(&ClosePanel, w, cx);
        ws.git_amend(&GitAmend, w, cx);
    });
    cx.run_until_parked();
    assert_eq!(checked_git(&root, &["rev-parse", "HEAD"]), before);
    ws.update_in(cx, |ws, w, cx| {
        assert_eq!(fields(ws)[3].read(cx).text(cx), "New Zoë 界");
        assert_eq!(fields(ws)[4].read(cx).text(cx), "new@example.invalid");
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert_eq!(checked_git(&root, &["show", "-s", "--format=%an <%ae>"]).trim(), "New Zoë 界 <new@example.invalid>");
    assert_eq!(checked_git(&root, &["show", "-s", "--format=%cn <%ce>"]).trim(), "Orlo Test <orlo@example.invalid>");
    let parsed = checked_git(&root, &["show", "-s", "--format=%(trailers:only,unfold)"]);
    assert_eq!(parsed.lines().filter(|line| line.contains("<co@example.invalid>")).count(), 1, "{parsed}");
    ws.read_with(cx, |ws, _| assert!(ws.panel.is_none()));
}

#[gpui::test]
fn composition_normal_author_override_is_staged_only_and_does_not_change_git_identity(cx: &mut TestAppContext) {
    for explicit in [false, true] {
        let root = repo(&format!("composition-normal-author-{explicit}"));
        std::fs::write(root.join("file.txt"), "staged 界\n").unwrap();
        git(&root, &["add", "file.txt"]);
        std::fs::write(root.join("file.txt"), "unstaged 🦀\n").unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_path(root.join("file.txt"), w, cx);
            ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
            ws.git_commit(&GitCommit, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let fields = fields(ws);
            assert!(fields[3].read(cx).text(cx).is_empty());
            assert!(fields[4].read(cx).text(cx).is_empty());
            fields[0].update(cx, |e, cx| e.set_text("Ordinary staged commit", cx));
            if explicit {
                fields[3].update(cx, |e, cx| e.set_text("Selected 界", cx));
                fields[4].update(cx, |e, cx| e.set_text("selected@example.invalid", cx));
            }
            ws.submit_form(&form::Submit, w, cx);
        });
        cx.run_until_parked();
        assert_eq!(checked_git(&root, &["show", "HEAD:file.txt"]), "staged 界\n");
        assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "unstaged 🦀\n");
        assert_eq!(
            checked_git(&root, &["show", "-s", "--format=%an <%ae>"]).trim(),
            if explicit { "Selected 界 <selected@example.invalid>" } else { "Orlo Test <orlo@example.invalid>" }
        );
        assert_eq!(checked_git(&root, &["config", "user.name"]).trim(), "Orlo Test");
        assert_eq!(checked_git(&root, &["rev-list", "--count", "HEAD"]).trim(), "2");
    }
}

#[gpui::test]
fn composition_signing_failure_retains_author_and_newer_author_edits_survive_retry(cx: &mut TestAppContext) {
    let root = repo("composition-signing-author");
    git(&root, &["config", "gpg.format", "openpgp"]);
    git(&root, &["config", "gpg.program", "/usr/bin/false"]);
    git(&root, &["config", "user.signingkey", "fixture-key"]);
    git(&root, &["config", "commit.gpgsign", "true"]);
    std::fs::write(root.join("file.txt"), "staged\n").unwrap();
    git(&root, &["add", "file.txt"]);
    std::fs::write(root.join("file.txt"), "unstaged 界\n").unwrap();
    let before = checked_git(&root, &["rev-parse", "HEAD"]);
    let staged = checked_git(&root, &["ls-files", "--stage"]);
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(root.join("file.txt"), w, cx);
        ws.git.repositories = vec![git::discover(&root).unwrap().unwrap()];
        ws.git_amend(&GitAmend, w, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let fields = fields(ws);
        fields[0].update(cx, |e, cx| e.set_text("Signing retry 界", cx));
        fields[3].update(cx, |e, cx| e.set_text("Submitted 界", cx));
        fields[4].update(cx, |e, cx| e.set_text("submitted@example.invalid", cx));
        ws.submit_form(&form::Submit, w, cx);
    });
    cx.run_until_parked();
    assert_eq!(checked_git(&root, &["rev-parse", "HEAD"]), before);
    assert_eq!(checked_git(&root, &["ls-files", "--stage"]), staged);
    assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "unstaged 界\n");
    ws.update_in(cx, |ws, _, cx| {
        assert_eq!(fields(ws)[3].read(cx).text(cx), "Submitted 界");
        assert_eq!(fields(ws)[4].read(cx).text(cx), "submitted@example.invalid");
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        assert!(p.error.as_ref().unwrap().contains("sign"));
        assert!(!ws.git.operation);
    });
    git(&root, &["config", "commit.gpgsign", "false"]);
    ws.update_in(cx, |ws, w, cx| {
        ws.submit_form(&form::Submit, w, cx);
        let fields = fields(ws);
        fields[3].update(cx, |e, cx| e.set_text("Newer Author", cx));
        fields[4].update(cx, |e, cx| e.set_text("newer@example.invalid", cx));
    });
    cx.run_until_parked();
    assert_eq!(
        checked_git(&root, &["show", "-s", "--format=%an <%ae>"]).trim(),
        "Submitted 界 <submitted@example.invalid>"
    );
    ws.read_with(cx, |ws, cx| {
        assert_eq!(fields(ws)[3].read(cx).text(cx), "Newer Author");
        assert_eq!(fields(ws)[4].read(cx).text(cx), "newer@example.invalid");
        let Some(Panel::Form(p)) = &ws.panel else { panic!() };
        assert!(p.error.as_ref().unwrap().contains("newer draft edits retained"));
    });
}
