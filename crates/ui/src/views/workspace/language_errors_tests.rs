//! A user language's grammar errors reach the customization alert: compiled on the highlight
//! worker after a reload, joined with the folders' load errors, each shown once a generation.
use super::super::ui_tests::{settle_until, window};
use super::*;
use gpui::{TestAppContext, VisualTestContext};

const QUERY_ERROR: &str = "queries/highlights.scm: line 1, column 2: unknown node type";

/// `Languages/<id>`: the json-wasm fixture under `id`, with `highlights` as its query.
fn wasm_language(id: &str, highlights: &str) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/languages/json-wasm");
    let folder = session::support_dir().join("Languages").join(id);
    std::fs::create_dir_all(folder.join("queries")).unwrap();
    std::fs::copy(fixture.join("grammar.wasm"), folder.join("grammar.wasm")).unwrap();
    let manifest = std::fs::read_to_string(fixture.join("language.json")).unwrap();
    let manifest = manifest.replace("\"json-wasm\"", &format!("\"{id}\"")).replace("jsonw", id);
    std::fs::write(folder.join("language.json"), manifest).unwrap();
    std::fs::write(folder.join("queries/highlights.scm"), highlights).unwrap();
    folder
}

fn reload(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
}

/// The alert, once the background compile has landed.
fn alert(cx: &mut VisualTestContext) -> (String, String) {
    settle_until(cx, |cx| cx.has_pending_prompt());
    let prompt = cx.pending_prompt().unwrap();
    cx.simulate_prompt_answer("Ok");
    prompt
}

fn status(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    ws.read_with(cx, |ws, _| ws.status.as_ref().map(|s| s.0.clone()).unwrap_or_default())
}

#[gpui::test]
fn a_user_grammar_error_joins_the_reload_alert_once_a_generation(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let folder = wasm_language("badquery", "(no_such_node) @string\n");
    let broken = session::support_dir().join("Languages/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("language.json"), r#"{"schema":1,"id":"broken","nmae":"Broken"}"#).unwrap();
    let query = format!("{}/{QUERY_ERROR}", folder.display());
    reload(&ws, cx);
    let (title, detail) = alert(cx);
    assert_eq!(title, "Could not apply customizations");
    assert!(detail.contains("unknown field `nmae`"), "the load error is in the same alert: {detail}");
    assert_eq!(detail.matches(&query).count(), 1, "{detail}");
    // a buffer of it finds the generation's grammar failed and is not parsed: nothing new
    let file = folder.join("sample.badquery");
    std::fs::write(&file, "[1, \"two\"]\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_path(file.clone(), window, cx));
    cx.run_until_parked();
    // the next generation compiles it again, and says so again, once
    reload(&ws, cx);
    let (_, detail) = alert(cx);
    assert_eq!(detail.matches(&query).count(), 1, "{detail}");
    std::fs::remove_dir_all(&broken).unwrap();
    std::fs::write(folder.join("queries/highlights.scm"), "(string) @string\n").unwrap();
    reload(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx) == "Customizations reloaded");
    assert!(!cx.has_pending_prompt());
}

#[gpui::test]
fn a_reload_with_only_good_user_grammars_reports_success_after_compiling_them(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    wasm_language("goodquery", "(string) @string\n");
    reload(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx) == "Customizations reloaded");
    assert!(!cx.has_pending_prompt());
    let registry = crate::languages::registry::registry();
    let grammar = crate::languages::treesitter::Grammar::of(&registry, registry.get("goodquery").unwrap()).unwrap();
    assert!(!grammar.failed(), "compiled for the generation, and it works");
}

#[gpui::test]
fn a_grammar_that_fails_while_parsing_is_named_by_the_next_reload(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/languages/trap");
    let folder = session::support_dir().join("Languages/trap");
    for file in ["language.json", "grammar.wasm", "queries/highlights.scm"] {
        std::fs::create_dir_all(folder.join(file).parent().unwrap()).unwrap();
        std::fs::copy(fixture.join(file), folder.join(file)).unwrap();
    }
    reload(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx) == "Customizations reloaded");
    assert!(!cx.has_pending_prompt(), "it compiles");
    let file = folder.join("sample.trap");
    std::fs::write(&file, "a!\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_path(file.clone(), window, cx));
    let highlighting =
        |cx: &mut VisualTestContext| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).highlighting().0);
    ws.update(cx, |ws, cx| ws.active_editor().unwrap().update(cx, |e, cx| e.highlight_in_background(0, 1, cx)));
    settle_until(cx, |cx| !highlighting(cx));
    reload(&ws, cx);
    let (title, detail) = alert(cx);
    assert_eq!(title, "Could not apply customizations");
    let trapped = format!("{}: the grammar failed while parsing", folder.join("grammar.wasm").display());
    assert_eq!(detail.matches(&trapped).count(), 1, "{detail}");
}

#[gpui::test]
fn a_grammar_that_fails_while_parsing_is_named_by_the_windows_poll(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/languages/trap");
    let folder = session::support_dir().join("Languages/trap");
    for file in ["language.json", "grammar.wasm", "queries/highlights.scm"] {
        std::fs::create_dir_all(folder.join(file).parent().unwrap()).unwrap();
        std::fs::copy(fixture.join(file), folder.join(file)).unwrap();
    }
    reload(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx) == "Customizations reloaded");
    let file = folder.join("sample.trap");
    std::fs::write(&file, "a!\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_path(file.clone(), window, cx));
    let highlighting =
        |cx: &mut VisualTestContext| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).highlighting().0);
    ws.update(cx, |ws, cx| ws.active_editor().unwrap().update(cx, |e, cx| e.highlight_in_background(0, 1, cx)));
    settle_until(cx, |cx| !highlighting(cx));
    // what the poll does each tick while the window is in front: no reload needed
    ws.update_in(cx, |ws, window, cx| ws.report_grammar_failures(window, cx));
    let (title, detail) = alert(cx);
    assert_eq!(title, "A language's grammar failed");
    let trapped = format!("{}: the grammar failed while parsing", folder.join("grammar.wasm").display());
    assert_eq!(detail.matches(&trapped).count(), 1, "{detail}");
    // said once
    ws.update_in(cx, |ws, window, cx| ws.report_grammar_failures(window, cx));
    assert!(!cx.has_pending_prompt());
}
