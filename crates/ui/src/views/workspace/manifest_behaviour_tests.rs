//! What a language's manifest changes beyond detection: Format Document and format on save
//! through its formatter (argv, stdin to stdout), which buffers have a language server, and the
//! comment markers of a language with no grammar.
use super::super::ui_tests::{settle_until, temp, text, window};
use super::*;
use gpui::{TestAppContext, VisualTestContext};

fn status(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    ws.read_with(cx, |ws, _| ws.status.as_ref().map(|s| s.0.clone()).unwrap_or_default())
}

/// A user language `fmt` ("Fmt") for `.orlofmt` files, with `formatter` as its manifest's.
fn fmt_language(formatter: Value) {
    let folder = session::support_dir().join("Languages/fmt");
    std::fs::create_dir_all(&folder).unwrap();
    let manifest = json!({"schema":1, "id":"fmt", "name":"Fmt", "scope":"source.fmt",
        "files":{"extensions":["orlofmt"]}, "formatter":formatter});
    std::fs::write(folder.join("language.json"), manifest.to_string()).unwrap();
}

/// `contents` in a new `name` under a fresh folder, opened with `fmt` using `formatter` (unless
/// null), the languages reloaded and `settings` applied.
fn open<'a>(
    cx: &'a mut TestAppContext,
    label: &str,
    name: &str,
    contents: &str,
    settings: Value,
    formatter: Value,
) -> (Entity<Workspace>, &'a mut VisualTestContext, PathBuf) {
    let root = temp(&format!("orlo-formatter-{label}-{}", std::process::id())).canonicalize().unwrap();
    std::fs::write(root.join(".editorconfig"), "root=true").unwrap();
    let path = root.join(name);
    std::fs::write(&path, contents).unwrap();
    let (ws, cx) = window(cx);
    if !formatter.is_null() {
        fmt_language(formatter);
    }
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.apply_settings(&settings.to_string(), window, cx);
        ws.open_path(path.clone(), window, cx);
    });
    (ws, cx, path)
}

fn format(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.lsp_format(&LspFormat, window, cx));
}

const UPPER: [&str; 3] = ["/usr/bin/tr", "a-z", "A-Z"];

#[gpui::test]
fn a_formatter_exiting_0_replaces_only_what_changed_in_one_undo_step(cx: &mut TestAppContext) {
    let (ws, cx, _) = open(
        cx,
        "applies",
        "a.orlofmt",
        "keep\nshout\nkeep\n",
        json!({}),
        json!({"command":["/usr/bin/sed", "s/shout/SHOUT/"]}),
    );
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(13)));
    format(&ws, cx);
    settle_until(cx, |cx| text(&ws, cx) == "keep\nSHOUT\nkeep\n");
    assert_eq!(status(&ws, cx), "Formatted with sed (Undo restores the original)");
    ws.update(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 13, "a caret outside the change stays");
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
    });
    assert_eq!(text(&ws, cx), "keep\nshout\nkeep\n");
    format(&ws, cx);
    settle_until(cx, |cx| text(&ws, cx) == "keep\nSHOUT\nkeep\n");
    format(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx) == "Already formatted (sed)");
}

#[gpui::test]
fn a_formatter_exiting_non_zero_leaves_the_text_and_reports_its_stderr(cx: &mut TestAppContext) {
    let script = "cat; echo 'parse error at 1:1' >&2; exit 3";
    let (ws, cx, _) =
        open(cx, "fails", "a.orlofmt", "echoed\n", json!({}), json!({"command":["/bin/sh", "-c", script]}));
    format(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx).starts_with("Formatter"));
    assert_eq!(status(&ws, cx), "Formatter sh: Exited with status 3: parse error at 1:1");
    assert_eq!(text(&ws, cx), "echoed\n");
    let dirty = ws.update(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).dirty);
    assert!(!dirty);
    fmt_language(json!({"command":["orlo-no-such-formatter"]}));
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    format(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx).starts_with("Formatter orlo-no-such-formatter:"));
    assert_eq!(text(&ws, cx), "echoed\n");
}

#[gpui::test]
fn an_edit_while_the_formatter_runs_discards_its_output(cx: &mut TestAppContext) {
    let (ws, cx, _) = open(cx, "stale", "a.orlofmt", "text\n", json!({}), json!({"command":UPPER}));
    ws.update_in(cx, |ws, window, cx| {
        ws.lsp_format(&LspFormat, window, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("new ")));
    });
    settle_until(cx, |cx| status(&ws, cx).starts_with("Format discarded"));
    assert_eq!(text(&ws, cx), "new text\n");
}

#[gpui::test]
fn the_formatters_setting_overrides_or_disables_a_languages_command(cx: &mut TestAppContext) {
    let (ws, cx, _) = open(
        cx,
        "override",
        "a.orlofmt",
        "a\n",
        json!({"formatters":{"fmt":UPPER}}),
        json!({"command":["/usr/bin/sed", "s/a/b/"]}),
    );
    format(&ws, cx);
    settle_until(cx, |cx| text(&ws, cx) == "A\n");
    assert_eq!(status(&ws, cx), "Formatted with tr (Undo restores the original)");
    ws.update_in(cx, |ws, window, cx| ws.apply_settings(r#"{"formatters":{"fmt":[]}}"#, window, cx));
    format(&ws, cx);
    cx.run_until_parked();
    assert_eq!(status(&ws, cx), "No language server is configured for Fmt", "[] leaves Format Document to LSP");
    assert_eq!(text(&ws, cx), "A\n");
}

#[gpui::test]
fn a_formatter_setting_needs_no_manifest_command(cx: &mut TestAppContext) {
    let (ws, cx, _) = open(cx, "bundled", "a.rs", "fn main() {}\n", json!({"formatters":{"rust":UPPER}}), Value::Null);
    format(&ws, cx);
    settle_until(cx, |cx| text(&ws, cx) == "FN MAIN() {}\n");
}

#[test]
fn formatters_are_rejected_in_project_settings() {
    let (project, warnings) = crate::config::project::Project::parse(
        Path::new("/tmp/Formatters.sublime-project"),
        r#"{"settings":{"formatters":{"fmt":["/usr/bin/tr","a-z","A-Z"]},"tab_size":3}}"#,
    )
    .unwrap();
    assert_eq!(warnings, [r#"settings: "formatters" is application-wide; set it in global preferences"#]);
    assert!(!project.settings.preferences.contains_key("formatters"));
}

#[gpui::test]
fn format_on_save_uses_the_formatter_and_any_failure_keeps_the_edits_unsaved(cx: &mut TestAppContext) {
    let settings = json!({"format_on_save":true, "format_on_save_timeout_ms":300});
    let (ws, cx, path) = open(cx, "save", "a.orlofmt", "saved\n", settings, json!({"command":UPPER}));
    let save = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| {
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
            ws.save(&Save, window, cx);
        })
    };
    save(&ws, cx);
    settle_until(cx, |_| std::fs::read_to_string(&path).unwrap() == "XSAVED\n");
    assert_eq!(text(&ws, cx), "XSAVED\n");
    for (command, why) in [
        (json!(["/bin/sh", "-c", "echo no >&2; exit 4"]), "Formatter sh: Exited with status 4: no"),
        (json!(["/bin/sleep", "5"]), "Save formatting timed out"),
    ] {
        ws.update_in(cx, |ws, window, cx| {
            let settings =
                json!({"format_on_save":true, "format_on_save_timeout_ms":300, "formatters":{"fmt":command}});
            ws.apply_settings(&settings.to_string(), window, cx);
        });
        let started = std::time::Instant::now();
        save(&ws, cx);
        settle_until(cx, |cx| status(&ws, cx).starts_with("Save formatting failed"));
        assert!(status(&ws, cx).contains(why), "{}", status(&ws, cx));
        assert!(started.elapsed() < std::time::Duration::from_secs(4), "the formatter is stopped at the deadline");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "XSAVED\n", "nothing is written");
        let doc = ws.update(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).dirty);
        assert!(doc, "{why}: the edit stays unsaved");
        assert!(text(&ws, cx).starts_with('x'));
    }
}

#[gpui::test]
fn only_a_language_with_a_configured_server_is_routed_to_one(cx: &mut TestAppContext) {
    let disabled = json!({"language_servers":{"fmt-server":{"command":[]}, "rust":{"command":[]}}});
    let (ws, cx, path) = open(cx, "gate", "a.orlofmt", "text\n", disabled, Value::Null);
    fmt_language(Value::Null);
    let manifest = session::support_dir().join("Languages/fmt/language.json");
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    value["lsp"] = json!({"language_id":"fmt", "server":"fmt-server", "command":["/bin/cat"]});
    std::fs::write(&manifest, value.to_string()).unwrap();
    let rust = path.with_file_name("a.rs");
    std::fs::write(&rust, "fn main() {}\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.open_path(path.clone(), window, cx);
        assert_eq!(ws.served_language_request(cx).err().as_deref(), Some("No language server is configured for Fmt"));
        assert!(!ws.has_language_server(cx), "navigation stays lexical");
        ws.open_path(rust.clone(), window, cx);
        assert_eq!(ws.served_language_request(cx).err().as_deref(), Some("No language server is configured for Rust"));
        ws.apply_settings("{}", window, cx);
        assert!(ws.has_language_server(cx), "rust-analyzer by default");
        ws.open_path(path.clone(), window, cx);
        assert!(ws.has_language_server(cx), "the manifest's command");
    });
}

#[gpui::test]
fn a_language_without_a_grammar_comments_with_its_manifests_markers(cx: &mut TestAppContext) {
    let (ws, cx, path) = open(cx, "comments", "a.orlofmt", "one\ntwo\n", json!({}), Value::Null);
    let manifest = session::support_dir().join("Languages/fmt/language.json");
    fmt_language(Value::Null);
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    value["comments"] = json!({"line":"--", "block":["{-", "-}"]});
    std::fs::write(&manifest, value.to_string()).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.open_path(path.clone(), window, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).highlighter.syntax_name(), "Plain Text");
    });
    cx.simulate_keystrokes("cmd-/");
    assert_eq!(text(&ws, cx), "-- one\ntwo\n");
    cx.simulate_keystrokes("cmd-/");
    assert_eq!(text(&ws, cx), "one\ntwo\n");
    ws.update_in(cx, |ws, window, cx| ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx));
    cx.simulate_keystrokes("cmd-/");
    assert_eq!(text(&ws, cx), "one\ntwo\n", "plain text chosen in View > Syntax has no markers");
}
