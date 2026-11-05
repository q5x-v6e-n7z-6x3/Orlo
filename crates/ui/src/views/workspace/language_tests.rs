//! Workspace-level language tests using the real transport and editor edit path.
use super::super::ui_tests::{settle_until, temp, text, type_in, window};
use super::*;
use crate::editing::text::Selection;
use crate::languages::complete::Kind;
use gpui::{TestAppContext, VisualTestContext};

fn config(target: &Path) -> std::collections::BTreeMap<String, ServerConfig> {
    language::default_servers()
        .into_keys()
        .map(|name| {
            (
                name,
                ServerConfig {
                    command: vec![
                        "python3".into(),
                        format!("{}/../../tests/fixtures/language_features.py", env!("CARGO_MANIFEST_DIR")),
                    ],
                    initialization_options: json!({"target": lsp::uri(target).unwrap()}),
                },
            )
        })
        .collect()
}

fn document(root: &Path, file: &str, buffer: u64) -> Document {
    let path = root.join(file);
    std::fs::write(&path, "😀pri\n").unwrap();
    Document {
        registry: crate::languages::registry::registry(),
        syntax_override: None,
        modeline_language: false,
        path: path.clone(),
        root: Some(root.into()),
        buffer,
        version: 0,
        rope: Rope::from_str("😀pri\n"),
        disk: crate::files::watch::stamp(&path),
        read_only: false,
    }
}

fn input(root: &Path, documents: Vec<Document>) -> Input {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    Input {
        registry: crate::languages::registry::registry(),
        serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        active: documents.first().map(|d| d.path.clone()),
        documents,
        config: config(&root.join("target.rs")),
        enabled: true,
    }
}

fn request(input: &Input) -> Request {
    Request {
        document: input.documents[0].clone(),
        documents: input.documents.clone(),
        cursor: 4,
        selection: 1..4,
        format_selection: None,
        generation: (0, 0),
        navigation_generation: None,
        navigation_source: None,
        epoch: Arc::default(),
        service_generation: 0,
        started: std::time::SystemTime::now(),
    }
}

fn state(manager: &Manager, document: &Document) -> Value {
    manager.sessions[&server_key(document).unwrap()].as_ref().unwrap().client.request("test/state", json!({})).unwrap()
}

/// How many of the server's recorded `list` ("requests" or "notifications") are `method`.
fn count(state: &Value, list: &str, method: &str) -> usize {
    (state[list].as_array().unwrap().iter()).filter(|n| n["method"] == method).count()
}

/// The server's recorded state for the open document at `path`, else the first one.
fn captured(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: Option<&Path>) -> Value {
    let (manager, input) = ws.read_with(cx, |ws, cx| (ws.language.manager.clone(), ws.language_input(cx)));
    let documents = &input.documents;
    let document = path.map_or(&documents[0], |path| documents.iter().find(|d| d.path == path).unwrap());
    let manager = manager.lock().unwrap();
    state(&manager, document)
}

/// A server response of `value` with no document versions.
fn response(value: Value, root: &Path) -> Response {
    Response { command: None, value, encoding: Encoding::Utf16, versions: HashMap::new(), root: root.into() }
}

/// Language servers on, with `servers`.
fn serve(cx: &mut App, servers: std::collections::BTreeMap<String, ServerConfig>) {
    let mut settings = cx.global::<Settings>().clone();
    settings.lsp_enabled = true;
    settings.language_servers = servers;
    cx.set_global(settings);
}

#[test]
fn manager_syncs_every_language_id_and_shares_js_ts_sessions() {
    let root = temp("orlo-lsp-language-families").canonicalize().unwrap();
    let cases = [
        ("a.rs", "rust"),
        ("a.go", "go"),
        ("a.js", "javascript"),
        ("a.jsx", "javascriptreact"),
        ("a.ts", "typescript"),
        ("a.tsx", "typescriptreact"),
        ("a.py", "python"),
        ("Main.java", "java"),
        ("a.c", "c"),
        ("a.cpp", "cpp"),
        ("a.cs", "csharp"),
        ("a.php", "php"),
        ("a.rb", "ruby"),
        ("a.swift", "swift"),
        ("a.kt", "kotlin"),
        ("a.sh", "shellscript"),
        ("a.hcl", "hcl"),
        ("a.tf", "terraform"),
        ("a.tfvars", "terraform-vars"),
        ("a.tfcomponent.hcl", "terraform-stack"),
        ("a.tfdeploy.hcl", "terraform-deploy"),
        ("a.tfquery.hcl", "terraform-search"),
        ("a.policy.hcl", "terraform-policy"),
        ("a.policytest.hcl", "terraform-policytest"),
    ];
    let docs = cases.into_iter().enumerate().map(|(n, (file, _))| document(&root, file, n as u64 + 1)).collect();
    let input = input(&root, docs);
    let mut manager = Manager::default();
    let decorations = manager.sync(&input);
    assert_eq!(manager.sessions.len(), 14);
    assert_eq!(decorations.len(), cases.len());
    assert_eq!(server_key(&input.documents[2]), server_key(&input.documents[4]));
    assert_eq!(server_key(&input.documents[8]), server_key(&input.documents[9]));
    assert_ne!(server_key(&input.documents[16]), server_key(&input.documents[17]));
    for (doc, (_, id)) in input.documents.iter().zip(cases) {
        let state = state(&manager, doc);
        assert_eq!(state["documents"][lsp::uri(&doc.path).unwrap()]["languageId"], id);
        let mut request = request(&input);
        request.document = doc.clone();
        let response = manager.request(&input, &request, "textDocument/completion", json!({})).unwrap();
        assert_eq!(response.value[0]["label"], "print", "{id}");
    }
}

#[test]
fn language_roots_follow_build_manifests_and_dotnet_solution_files() {
    let root = temp("orlo-lsp-new-language-roots").canonicalize().unwrap();
    for (folder, marker, file) in [
        ("java", "pom.xml", "Main.java"),
        ("kotlin", "settings.gradle.kts", "Main.kt"),
        ("c", "compile_commands.json", "main.c"),
        ("cpp", "CMakeLists.txt", "main.cpp"),
        ("cs", "App.sln", "Main.cs"),
        ("csproj", "App.csproj", "Main.cs"),
        ("ruby", "Gemfile", "main.rb"),
        ("php", "composer.json", "main.php"),
        ("swift", "Package.swift", "main.swift"),
        ("bash", ".shellcheckrc", "main.sh"),
        ("terraform", ".terraform.lock.hcl", "variables.tf"),
    ] {
        let project = root.join(folder);
        let nested = project.join("src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(project.join(marker), "").unwrap();
        let mut doc = document(&nested, file, 1);
        doc.root = None;
        let language = doc.language().unwrap();
        assert_eq!(root_for(&doc, &language), project, "{file}: {marker}");
    }
}

#[test]
fn manager_tracks_open_change_save_close_and_ignores_stale_diagnostics() {
    let root = temp("orlo-lsp-document-lifecycle").canonicalize().unwrap();
    let a = document(&root, "a.rs", 1);
    let b = document(&root, "b.rs", 2);
    let mut input = input(&root, vec![a, b]);
    let mut manager = Manager::default();
    let decorations = manager.sync(&input);
    let active = decorations.iter().find(|d| d.document.path == input.documents[0].path).unwrap();
    assert_eq!(active.diagnostics[0].range, 1..4);
    assert_eq!(active.semantics[0].range, 1..4);
    input.documents[0].version = 5;
    input.documents[0].rope = Rope::from_str("😀longer\n");
    std::fs::write(&input.documents[0].path, "saved version\n").unwrap();
    input.documents[0].disk = crate::files::watch::stamp(&input.documents[0].path);
    manager.sync(&input);
    let uri = lsp::uri(&input.documents[0].path).unwrap();
    let session = manager.sessions.get_mut(&server_key(&input.documents[0]).unwrap()).unwrap().as_mut().unwrap();
    assert_eq!(session.documents[&uri].serial, 2);
    session.client.request("test/staleDiagnostics", json!({"uri":uri})).unwrap();
    session.drain();
    assert_eq!(session.documents[&uri].diagnostics[0].range, 1..7);
    input.documents.remove(1);
    manager.sync(&input);
    let state = state(&manager, &input.documents[0]);
    let notes = state["notifications"].as_array().unwrap();
    assert_eq!(count(&state, "notifications", "textDocument/didOpen"), 2);
    let change = notes.iter().find(|n| n["method"] == "textDocument/didChange").unwrap();
    assert_eq!(change["params"]["textDocument"]["version"], 2);
    assert_eq!(change["params"]["contentChanges"][0]["text"], "😀longer\n");
    assert_eq!(notes.iter().find(|n| n["method"] == "textDocument/didSave").unwrap()["params"]["text"], "😀longer\n");
    assert_eq!(count(&state, "notifications", "textDocument/didClose"), 1);
    assert_eq!(state["documents"].as_object().unwrap().len(), 1);
    input.enabled = false;
    assert!(manager.sync(&input).is_empty());
    assert!(manager.sessions.is_empty());
}

/// The fixture's servers syncing `sync` (1 in full, 2 incrementally), agreeing `encoding` when
/// one is given (UTF-16 otherwise), and with any other `options` set at initialize.
fn sync_config(
    target: &Path,
    sync: u64,
    encoding: Option<&str>,
    options: Value,
) -> std::collections::BTreeMap<String, ServerConfig> {
    let mut servers = config(target);
    for server in servers.values_mut() {
        let init = &mut server.initialization_options;
        init["sync"] = json!(sync);
        if let Some(encoding) = encoding {
            init["encoding"] = json!(encoding);
        }
        for (key, value) in options.as_object().into_iter().flatten() {
            init[key] = value.clone();
        }
    }
    servers
}

/// Both sync kinds, each with the encoding left to the default and each one agreed.
const SYNC_MODES: [(u64, Option<&str>); 8] = [
    (1, None),
    (1, Some("utf-8")),
    (1, Some("utf-16")),
    (1, Some("utf-32")),
    (2, None),
    (2, Some("utf-8")),
    (2, Some("utf-16")),
    (2, Some("utf-32")),
];

/// The text the fixture server rebuilt for `document` from everything it was sent; it must
/// have placed every range it was given.
fn server_text(manager: &Manager, document: &Document) -> String {
    let state = state(manager, document);
    assert_eq!(state["errors"], json!([]), "the server could not apply a change");
    state["documents"][lsp::uri(&document.path).unwrap()]["text"].as_str().unwrap().to_owned()
}

/// The `contentChanges` of the last `didChange` the server got for `document`.
fn last_changes(manager: &Manager, document: &Document) -> Vec<Value> {
    let uri = lsp::uri(&document.path).unwrap();
    let state = state(manager, document);
    let notes = state["notifications"].as_array().unwrap();
    notes
        .iter()
        .rev()
        .find(|n| n["method"] == "textDocument/didChange" && n["params"]["textDocument"]["uri"] == uri)
        .map(|n| n["params"]["contentChanges"].as_array().unwrap().clone())
        .unwrap_or_default()
}

/// `document` with the text and version `state` has now: what the next poll sends.
fn now(document: &Document, state: &EditorState) -> Document {
    Document { buffer: state.buffer_id(), version: state.doc.version, rope: state.doc.rope.clone(), ..document.clone() }
}

/// Unicode of every width on short lines, then enough plain lines that a few small edits go as
/// ranges rather than as the whole text.
fn long_text() -> String {
    let mut text = String::from("😀pri café e\u{301}cole\n\tindented 日本語 한국어\n𝄞 clef 🦀\n");
    text.push_str(&"filler line of plain ascii text\n".repeat(40));
    text.push_str("last 中文\n");
    text
}

#[test]
fn every_sync_mode_and_encoding_rebuilds_orlo_text_from_multi_caret_edits_and_undo() {
    for (n, (sync, encoding)) in SYNC_MODES.into_iter().enumerate() {
        let root = temp(&format!("orlo-lsp-sync-modes-{n}")).canonicalize().unwrap();
        let mut s = EditorState::from_text(&long_text());
        let opened = now(&document(&root, "a.rs", 0), &s);
        let mut input = input(&root, vec![opened]);
        input.config = sync_config(&root.join("target.rs"), sync, encoding, json!({}));
        let mut manager = Manager::default();
        let mode = format!("sync {sync}, {encoding:?}");
        let mut cycle = |s: &EditorState, what: &str| {
            input.documents[0] = now(&input.documents[0], s);
            manager.sync(&input);
            assert_eq!(server_text(&manager, &input.documents[0]), s.text(), "{what}, {mode}");
            last_changes(&manager, &input.documents[0])
        };
        cycle(&s, "open");

        // several edits at three carets between two polls: after the emoji, inside the tab's
        // line and at the very end
        let end = s.len();
        s.set_selections(vec![Selection::caret(1), Selection::caret(s.offset(1, 3)), Selection::caret(end)]);
        s.type_text("é");
        s.type_text("😀\u{301}");
        s.type_text("\n");
        s.backspace();
        s.type_text("\t𝄞");
        // an incremental server gets ranges only, at least one per caret; a full one, the text
        let ranges_only = |changes: Vec<Value>| {
            let ranged = changes.iter().filter(|c| c["range"].is_object()).count();
            match sync {
                2 => ranged == changes.len() && ranged >= 3,
                _ => ranged == 0 && changes.len() == 1,
            }
        };
        let changes = cycle(&s, "three carets");
        assert!(ranges_only(changes), "{mode}");

        // line breaks typed at three carets: each change below the first starts lines further
        // down than it did in the text the server had
        s.set_selections(vec![
            Selection::caret(s.offset(0, 3)),
            Selection::caret(s.offset(3, 6)),
            Selection::caret(s.offset(20, 4)),
        ]);
        s.type_text("➜ new\n\t");
        let changes = cycle(&s, "line breaks at three carets");
        assert!(ranges_only(changes), "{mode}");

        // across line ends: a selection from the first line into the second replaced
        s.set_selections(vec![Selection::new(s.offset(0, 2), s.offset(1, 4))]);
        s.type_text("日本\n語");
        cycle(&s, "a selection across a line end");
        s.set_selections(vec![Selection::caret(s.offset(2, 0))]);
        s.backspace();
        cycle(&s, "a line joined to the one above");

        // undo and redo bring back older text, from any edit the server has seen
        s.undo();
        cycle(&s, "undo");
        s.undo();
        s.undo();
        cycle(&s, "two undos");
        s.redo();
        cycle(&s, "redo");

        // the whole text replaced, then nothing left
        s.select_all();
        s.type_text("fresh 🦀\n");
        cycle(&s, "everything replaced");
        s.select_all();
        s.backspace();
        cycle(&s, "everything deleted");
    }
}

#[test]
fn a_server_that_takes_no_changes_still_hears_of_saves_and_keeps_answering() {
    let root = temp("orlo-lsp-sync-none").canonicalize().unwrap();
    let mut s = EditorState::from_text(&long_text());
    let opened = now(&document(&root, "a.rs", 0), &s);
    let mut input = input(&root, vec![opened]);
    input.config = sync_config(&root.join("target.rs"), 0, None, json!({}));
    let mut manager = Manager::default();
    poll(&mut manager, &mut input, &s);
    s.set_selections(vec![Selection::caret(1)]);
    s.type_text("é");
    std::fs::write(&input.documents[0].path, s.text()).unwrap();
    input.documents[0].disk = crate::files::watch::stamp(&input.documents[0].path);
    let (document, status) = poll(&mut manager, &mut input, &s);
    assert!(!status.contains("does not support"), "{status}");
    let notes = state(&manager, &document)["notifications"].clone();
    let methods: Vec<&str> = (notes.as_array().unwrap().iter()).map(|n| n["method"].as_str().unwrap()).collect();
    assert_eq!(methods, ["textDocument/didOpen", "textDocument/didSave"]);
    // the session answers for the text as it is now, so a request on it is not stale
    let session = manager.sessions[&server_key(&document).unwrap()].as_ref().unwrap();
    let open = &session.documents[&lsp::uri(&document.path).unwrap()];
    assert_eq!(open.document.version, document.version);
}

#[test]
fn the_whole_text_goes_where_ranges_would_land_a_line_off_or_cost_more_than_it() {
    let root = temp("orlo-lsp-sync-fallbacks").canonicalize().unwrap();
    // a lone \r ends a line for the protocol but not for the rope's lines
    let mut s = EditorState::from_text(&format!("one\rtwo\n{}", long_text()));
    let opened = now(&document(&root, "a.rs", 0), &s);
    let mut input = input(&root, vec![opened]);
    input.config = sync_config(&root.join("target.rs"), 2, Some("utf-8"), json!({}));
    let mut manager = Manager::default();
    manager.sync(&input);
    let mut cycle = |s: &EditorState, what: &str| {
        input.documents[0] = now(&input.documents[0], s);
        manager.sync(&input);
        assert_eq!(server_text(&manager, &input.documents[0]), s.text(), "{what}");
        let changes = last_changes(&manager, &input.documents[0]);
        changes.len() == 1 && changes[0].get("range").is_none()
    };
    s.set_selections(vec![Selection::caret(s.offset(3, 2))]);
    s.type_text("typed");
    assert!(cycle(&s, "below a \\r"), "ranges sent below a \\r");
    // the \r gone, small edits go as ranges again, until one types a \r
    s.set_selections(vec![Selection::new(3, 4)]);
    s.type_text(" ");
    assert!(cycle(&s, "the \\r removed"));
    s.set_selections(vec![Selection::caret(s.offset(5, 1))]);
    s.type_text("x");
    assert!(!cycle(&s, "no \\r left"), "the whole text sent for one char");
    s.insert("\r");
    assert!(cycle(&s, "a \\r typed"), "a range sent with a \\r in it");
    // a change on every line costs more as ranges than the text does
    let lines = s.line_count();
    s.set_selections((0..lines).map(|l| Selection::caret(s.offset(l, 0))).collect());
    s.type_text("#");
    assert!(cycle(&s, "every line"), "ranges sent that outweigh the text");
}

/// One poll after `s`'s edits: the document as the workspace sent it, and its status line.
fn poll(manager: &mut Manager, input: &mut Input, s: &EditorState) -> (Document, String) {
    input.documents[0] = now(&input.documents[0], s);
    let decorations = manager.sync(input);
    (input.documents[0].clone(), decorations[0].status.clone())
}

/// How many times the server was given `document`'s whole text by a `didOpen`, once it is known
/// to have rebuilt the text Orlo has.
fn opened(manager: &Manager, document: &Document, what: &str) -> usize {
    let text = document.rope.to_string();
    assert_eq!(server_text(manager, document), text, "{what}");
    count(&state(manager, document), "notifications", "textDocument/didOpen")
}

#[test]
fn reopening_renaming_and_restarting_resend_the_text_and_a_failed_server_loses_no_edit() {
    let root = temp("orlo-lsp-sync-lifecycle").canonicalize().unwrap();
    let mut s = EditorState::from_text(&long_text());
    let first = now(&document(&root, "a.rs", 0), &s);
    let old_uri = lsp::uri(&first.path).unwrap();
    let mut input = input(&root, vec![first]);
    let options = json!({"exitAfterChanges": 3});
    input.config = sync_config(&root.join("target.rs"), 2, Some("utf-16"), options);
    let mut manager = Manager::default();
    poll(&mut manager, &mut input, &s);
    s.set_cursor(1);
    s.type_text("one ");
    let (sent, _) = poll(&mut manager, &mut input, &s);
    assert_eq!(opened(&manager, &sent, "first change"), 1);

    // a reload or a reopened tab is another buffer: closed, then opened with its whole text
    let mut s = EditorState::from_text(&s.text());
    s.set_cursor(5);
    s.type_text("two ");
    let (sent, _) = poll(&mut manager, &mut input, &s);
    assert_eq!(opened(&manager, &sent, "reopened"), 2);

    // renamed: the old URI closes and the new one opens with the whole text
    let renamed = root.join("b.rs");
    std::fs::rename(&input.documents[0].path, &renamed).unwrap();
    input.documents[0].path = renamed;
    s.type_text("three ");
    let (sent, _) = poll(&mut manager, &mut input, &s);
    assert_eq!(opened(&manager, &sent, "renamed"), 3);
    assert!(state(&manager, &sent)["documents"].get(&old_uri).is_none());

    // the server stops after its third change; edits after that wait for the next server
    s.type_text("four ");
    let (sent, _) = poll(&mut manager, &mut input, &s);
    opened(&manager, &sent, "second change");
    s.type_text("five ");
    poll(&mut manager, &mut input, &s);
    let key = server_key(&input.documents[0]).unwrap();
    let session = manager.sessions[&key].as_ref().unwrap();
    assert!(session.client.request("test/state", json!({})).is_err(), "the server still runs");
    s.type_text("six ");
    let (_, status) = poll(&mut manager, &mut input, &s);
    assert!(status.starts_with("LSP: ") && status != "LSP: rust", "{status}");

    // Restart Language Servers drops the sessions: the new server gets every edit since
    manager.sessions.clear();
    let (sent, _) = poll(&mut manager, &mut input, &s);
    assert_eq!(opened(&manager, &sent, "restarted"), 1);
    assert_eq!(sent.rope.to_string(), s.text());
}

#[gpui::test]
fn typing_quickly_in_a_workspace_view_reaches_an_incremental_server_whole(cx: &mut TestAppContext) {
    let root = temp("orlo-lsp-ui-incremental").canonicalize().unwrap();
    let a = root.join("a.rs");
    std::fs::write(&a, long_text()).unwrap();
    let (ws, cx) = window(cx);
    cx.update(|_, cx| serve(cx, sync_config(&root.join("b.rs"), 2, Some("utf-8"), json!({}))));
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(root.clone(), window, cx);
        ws.open_path(a.clone(), window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
    });
    let manager = ws.read_with(cx, |ws, _| ws.language.manager.clone());
    let poll = |cx: &mut VisualTestContext| {
        let input = ws.read_with(cx, |ws, cx| ws.language_input(cx));
        manager.lock().unwrap().sync(&input);
        input.documents.into_iter().find(|d| d.path == a).unwrap()
    };
    poll(cx);
    // a key at a time, as fast as one types: every edit lands between the same two polls
    for key in "fn café() {\n\tlet 雪 = \"🦀\";\n}".chars() {
        type_in(&ws, cx, &key.to_string());
    }
    let document = poll(cx);
    let manager = manager.lock().unwrap();
    assert_eq!(server_text(&manager, &document), text(&ws, cx));
    let changes = last_changes(&manager, &document);
    assert!(!changes.is_empty() && changes.iter().all(|c| c["range"].is_object()));
}

#[test]
fn manager_requests_real_feature_results_and_carries_document_versions() {
    let root = temp("orlo-lsp-feature-requests").canonicalize().unwrap();
    let input = input(&root, vec![document(&root, "a.rs", 1), document(&root, "b.rs", 2)]);
    let request = request(&input);
    let mut manager = Manager::default();
    let mut ask = |method, extra| manager.request(&input, &request, method, extra).unwrap();
    let completion = ask("textDocument/completion", json!({}));
    assert_eq!(completion.value[0]["label"], "print");
    let resolved = ask("completionItem/resolve", completion.value[0].clone());
    assert_eq!(resolved.value["additionalTextEdits"][0]["newText"], "imported\n");
    let definition = ask("textDocument/definition", json!({}));
    let locations = language::locations(&definition.value).unwrap();
    assert_eq!(locations[0].1.start.character, 2);
    let references = ask("textDocument/references", json!({"context":{"includeDeclaration":true}}));
    assert_eq!(references.value.as_array().unwrap().len(), 2);
    let hover = ask("textDocument/hover", json!({}));
    assert_eq!(language::hover_text(&hover.value), "fixture docs 😀");
    let formatted = ask("textDocument/formatting", json!({"options":{"tabSize":4,"insertSpaces":true}}));
    assert_eq!(formatted.value[0]["newText"], "formatted");
    let rename = ask("textDocument/rename", json!({"newName":"renamed"}));
    assert_eq!(rename.versions.len(), 2);
    assert_eq!(prepare_edits(&request, rename).unwrap().len(), 2);
    let actions = ask("textDocument/codeAction", json!({}));
    assert_eq!(actions.value[0]["title"], "Fix now");
    let resolved = ask("codeAction/resolve", actions.value[1].clone());
    assert_eq!(
        resolved.value["edit"]["changes"][lsp::uri(&input.documents[0].path).unwrap()][0]["newText"],
        "resolved"
    );
}

#[test]
fn n40_manager_routes_both_destinations_with_negotiated_unicode_positions() {
    for (encoding, cursor, start, end) in [("utf-8", 7, 4, 10), ("utf-16", 5, 2, 8), ("utf-32", 4, 1, 7)] {
        let root = temp(&format!("orlo-n40-manager-{encoding}")).canonicalize().unwrap();
        let mut input = input(&root, vec![document(&root, "雪 source.rs", 1)]);
        for server in input.config.values_mut() {
            server.initialization_options["encoding"] = encoding.into();
        }
        let request = request(&input);
        let mut manager = Manager::default();
        for method in ["textDocument/typeDefinition", "textDocument/implementation"] {
            let response = manager.request(&input, &request, method, json!({})).unwrap();
            let targets = language::locations(&response.value).unwrap();
            assert_eq!(targets.len(), 1);
            assert_eq!((targets[0].1.start.character, targets[0].1.end.character), (start, end));
            assert_eq!(
                response.encoding,
                match encoding {
                    "utf-8" => Encoding::Utf8,
                    "utf-32" => Encoding::Utf32,
                    _ => Encoding::Utf16,
                }
            );
            let state = state(&manager, &request.document);
            let sent = state["requests"].as_array().unwrap().iter().find(|r| r["method"] == method).unwrap();
            assert_eq!(sent["params"]["position"], json!({"line":0,"character":cursor}));
            assert_eq!(
                state["clientCapabilities"]["textDocument"][method.split('/').nth(1).unwrap()]["linkSupport"],
                true
            );
        }
    }
}

#[test]
fn n40_manager_gates_missing_false_and_object_providers_independently() {
    for providers in [
        json!({}),
        json!({"typeDefinition":false,"implementation":null}),
        json!({"typeDefinition":{},"implementation":false}),
        json!({"implementation":{"workDoneProgress":true}}),
    ] {
        let root = temp("orlo-n40-provider-gates").canonicalize().unwrap();
        let mut input = input(&root, vec![document(&root, "a.rs", 1)]);
        for server in input.config.values_mut() {
            server.initialization_options["navigationProviders"] = providers.clone();
        }
        let request = request(&input);
        let mut manager = Manager::default();
        for destination in ["typeDefinition", "implementation"] {
            let method = format!("textDocument/{destination}");
            let result = manager.request(&input, &request, &method, json!({}));
            let supported = providers.get(destination).is_some_and(|v| !v.is_null() && v != false);
            assert_eq!(result.is_ok(), supported, "{providers}: {destination}");
            if let Err(error) = result {
                assert!(error.contains(&format!("does not support {method}")), "{error}");
            }
            assert_eq!(count(&state(&manager, &request.document), "requests", &method), usize::from(supported));
        }
    }
}

#[test]
fn n40_fixture_returns_single_links_and_location_arrays_for_both_methods() {
    for destination in ["typeDefinition", "implementation"] {
        let root = temp(&format!("orlo-n40-result-shapes-{destination}")).canonicalize().unwrap();
        let target = lsp::uri(&root.join("雪 target.rs")).unwrap();
        let range = json!({"start":{"line":0,"character":2},"end":{"line":0,"character":8}});
        for value in [
            json!([{"targetUri":target,"targetRange":range,"targetSelectionRange":range}]),
            json!([{"uri":target,"range":range},{"uri":target,"range":range}]),
        ] {
            let mut input = input(&root, vec![document(&root, "a.rs", 1)]);
            for server in input.config.values_mut() {
                server.initialization_options["navigationResults"] = json!({destination:value});
            }
            let mut manager = Manager::default();
            let response =
                manager.request(&input, &request(&input), &format!("textDocument/{destination}"), json!({})).unwrap();
            let locations = language::locations(&response.value).unwrap();
            assert_eq!(locations.len(), value.as_array().unwrap().len());
            for (uri, span) in locations {
                assert_eq!(uri, target);
                assert_eq!(span, lsp::Span { start: lsp::Position::new(0, 2), end: lsp::Position::new(0, 8) });
            }
        }
    }
}

fn n40_menu_actions(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<(String, Box<dyn gpui::Action>)> {
    ws.read_with(cx, |ws, cx| {
        ws.menus(cx)
            .into_iter()
            .find(|menu| menu.name == "Goto")
            .unwrap()
            .items
            .into_iter()
            .find_map(|item| {
                let MenuItem::Submenu(menu) = item else {
                    return None;
                };
                (menu.name == "Language Server").then(|| {
                    menu.items
                        .into_iter()
                        .filter_map(|item| {
                            let MenuItem::Action { name, action, .. } = item else {
                                return None;
                            };
                            (action.as_any().is::<LspTypeDefinition>()
                                || action.as_any().is::<LspTypeDefinitionToSide>()
                                || action.as_any().is::<LspImplementation>()
                                || action.as_any().is::<LspImplementationToSide>())
                            .then(|| (name.to_string(), action))
                        })
                        .collect()
                })
            })
            .unwrap()
    })
}

fn n40_poll(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
}

pub(super) fn navigation_gate_path(path: &Path) -> PathBuf {
    path.with_extension("navigation-gate")
}

#[gpui::test]
fn n40_menu_palette_and_single_unicode_targets_use_distinct_methods(cx: &mut TestAppContext) {
    for (encoding, destination, side) in [
        ("utf-8", "Type Definition", false),
        ("utf-16", "Implementation", true),
        ("utf-32", "Type Definition", true),
        ("utf-32", "Implementation", false),
    ] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-n40-single-{encoding}-{destination}"), "😀target\n");
        let target = a.parent().unwrap().join("雪 target.rs");
        std::fs::rename(b, &target).unwrap();
        rust_options(cx, |o| {
            o["target"] = lsp::uri(&target).unwrap().into();
            o["encoding"] = encoding.into();
        });
        assert!(n40_menu_actions(&ws, cx).is_empty(), "unknown capabilities must be hidden");
        activate(cx);
        n40_poll(&ws, cx);
        let label = format!("Go to {destination}{}", if side { " to Side" } else { "" });
        if side {
            palette_command(&ws, cx, &format!("LSP: {label}"));
        } else {
            let (_, action) = n40_menu_actions(&ws, cx).into_iter().find(|(name, _)| name == &label).unwrap();
            ws.update_in(cx, |_, window, cx| window.dispatch_action(action, cx));
        }
        settle_until(cx, |cx| showing(&ws, cx, &target));
        ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).state.selections[0].range(), 1..7);
            assert_eq!(ws.views.active, usize::from(side));
            assert!(ws.quick_panel.is_none());
        });
        let state = captured(&ws, cx, Some(&a));
        assert_eq!(
            count(&state, "requests", "textDocument/typeDefinition"),
            usize::from(destination == "Type Definition")
        );
        assert_eq!(
            count(&state, "requests", "textDocument/implementation"),
            usize::from(destination == "Implementation")
        );
        assert_eq!(count(&state, "requests", "textDocument/definition"), 0);
    }
}

#[gpui::test]
fn n40_picker_keyboard_selects_links_and_keeps_side_open(cx: &mut TestAppContext) {
    for (destination, side) in [("Type Definition", false), ("Implementation", true)] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-n40-picker-{destination}"), "😀target\n");
        let method = if side { "implementation" } else { "typeDefinition" };
        rust_options(cx, |o| {
            o["navigationResults"] = json!({method:[
                {"targetUri":lsp::uri(&a).unwrap(),"targetRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":5}},"targetSelectionRange":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}}},
                {"targetUri":lsp::uri(&b).unwrap(),"targetRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":8}},"targetSelectionRange":{"start":{"line":0,"character":2},"end":{"line":0,"character":8}}}
            ]})
        });
        activate(cx);
        n40_poll(&ws, cx);
        palette_command(&ws, cx, &format!("LSP: Go to {destination}{}", if side { " to Side" } else { "" }));
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.kind == quick_panel::Kind::Choices))
        });
        assert!(showing(&ws, cx, &a));
        cx.run_until_parked();
        cx.simulate_keystrokes("down enter");
        settle_until(cx, |cx| showing(&ws, cx, &b));
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), 1..7);
            assert_eq!(ws.views.active, usize::from(side));
        });
    }
}

#[gpui::test]
fn n40_replacement_picker_ignores_its_predecessor_blur(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-n40-replacement-picker", "😀target\n");
    rust_options(cx, |o| {
        o["navigationResults"] = json!({"typeDefinition":[
            {"uri":lsp::uri(&a).unwrap(),"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}}},
            {"uri":lsp::uri(&b).unwrap(),"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":8}}}
        ]})
    });
    activate(cx);
    let open_picker = |ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
        let (input, request) = ws.language_request(cx).unwrap();
        let response = ws
            .language
            .manager
            .lock()
            .unwrap()
            .request(&input, &request, "textDocument/typeDefinition", json!({}))
            .unwrap();
        ws.language_response(Feature::TypeDefinition, response, request, window, cx);
    };
    ws.update_in(cx, open_picker);
    cx.run_until_parked();
    let previous = ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().unwrap().input.clone());
    ws.update_in(cx, open_picker);
    cx.run_until_parked();
    cx.simulate_keystrokes("down enter");
    settle_until(cx, |cx| showing(&ws, cx, &b) || status(&ws, cx).contains("navigation discarded"));
    assert!(showing(&ws, cx, &b), "a replaced input's blur invalidated its successor: {}", status(&ws, cx));
    drop(previous);
}

#[gpui::test]
fn n40_capability_loss_hides_menu_palette_and_direct_actions_report_without_jump(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-n40-ui-capabilities", "😀target\n");
    activate(cx);
    for (providers, types, implementations) in
        [(json!({"typeDefinition":{}}), 2, 0), (json!({"implementation":true}), 0, 2), (json!({}), 0, 0)]
    {
        rust_options(cx, |o| o["navigationProviders"] = providers);
        n40_poll(&ws, cx);
        let actions = n40_menu_actions(&ws, cx);
        assert_eq!(actions.iter().filter(|(name, _)| name.contains("Type Definition")).count(), types);
        assert_eq!(actions.iter().filter(|(name, _)| name.contains("Implementation")).count(), implementations);
        ws.update_in(cx, |ws, window, cx| ws.command_palette(&CommandPalette, window, cx));
        ws.read_with(cx, |ws, _| {
            let items = &ws.quick_panel.as_ref().unwrap().items;
            assert_eq!(items.iter().filter(|item| item.label.starts_with("LSP: Go to Type Definition")).count(), types);
            assert_eq!(
                items.iter().filter(|item| item.label.starts_with("LSP: Go to Implementation")).count(),
                implementations
            );
        });
        cx.simulate_keystrokes("escape");
    }
    for action in [LspTypeDefinition.boxed_clone(), LspImplementation.boxed_clone()] {
        ws.update_in(cx, |_, window, cx| window.dispatch_action(action, cx));
        settle_until(cx, |cx| status(&ws, cx).contains("does not support"));
        assert!(showing(&ws, cx, &a));
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 4));
    }
    let state = captured(&ws, cx, Some(&a));
    assert_eq!(count(&state, "requests", "textDocument/typeDefinition"), 0);
    assert_eq!(count(&state, "requests", "textDocument/implementation"), 0);
}

#[gpui::test]
fn n40_empty_errors_and_invalid_locations_leave_the_source_untouched(cx: &mut TestAppContext) {
    for destination in ["typeDefinition", "implementation"] {
        for (case, result, expected) in [
            ("null", Value::Null, "No locations"),
            ("empty", json!([]), "No locations"),
            (
                "invalid",
                json!([{"targetUri":"file:///tmp/no-target","targetSelectionRange":{"start":{"line":0,"character":-1},"end":{"line":0,"character":2}}}]),
                "Invalid location",
            ),
            ("error", Value::Null, "fixture navigation unsupported"),
        ] {
            let (ws, cx, a, _) = open(cx, &format!("orlo-n40-{destination}-{case}"), "😀target\n");
            rust_options(cx, |o| {
                o["navigationResults"] = json!({destination:result});
                o["navigationError"] = (case == "error").into();
            });
            activate(cx);
            let before =
                ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).read_state(cx).selection_state());
            let action = if destination == "typeDefinition" {
                LspTypeDefinition.boxed_clone()
            } else {
                LspImplementation.boxed_clone()
            };
            ws.update_in(cx, |_, window, cx| window.dispatch_action(action, cx));
            settle_until(cx, |cx| status(&ws, cx).contains(expected));
            assert!(showing(&ws, cx, &a));
            assert_eq!(text(&ws, cx), "😀pri\n");
            ws.read_with(cx, |ws, cx| {
                assert!(ws.active_editor().unwrap().read(cx).read_state(cx).selection_state() == before);
                assert!(ws.quick_panel.is_none());
            });
        }
    }
}

#[gpui::test]
fn n40_pending_results_reject_edits_and_exact_selection_changes(cx: &mut TestAppContext) {
    for destination in ["typeDefinition", "implementation"] {
        for mode in ["edit", "selection", "away-back"] {
            let (ws, cx, a, _) = open(cx, &format!("orlo-n40-pending-{destination}-{mode}"), "😀target\n");
            activate(cx);
            let pause = crate::documents::save::gate::pause(&navigation_gate_path(&a));
            let action = if destination == "typeDefinition" {
                LspTypeDefinition.boxed_clone()
            } else {
                LspImplementation.boxed_clone()
            };
            ws.update_in(cx, |_, window, cx| window.dispatch_action(action, cx));
            settle_until(cx, |_| pause.arrived() == 1);
            ws.update(cx, |ws, cx| {
                ws.with_active(cx, |e, cx| match mode {
                    "edit" => e.edit(cx, |s| {
                        s.insert("changed");
                        s.set_cursor(4);
                    }),
                    "selection" => e.sel(cx).reselect(vec![Selection::new(1, 4)]),
                    _ => {
                        e.sel(cx).set_cursor(0);
                        e.sel(cx).set_cursor(4);
                    }
                })
            });
            let before =
                ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).read_state(cx).selection_state());
            pause.release();
            if mode == "edit" {
                cx.run_until_parked();
            } else {
                settle_until(cx, |cx| status(&ws, cx).contains("discarded") || !showing(&ws, cx, &a));
                assert!(status(&ws, cx).contains("discarded"), "{destination}/{mode}: {}", status(&ws, cx));
            }
            assert!(showing(&ws, cx, &a));
            ws.read_with(cx, |ws, cx| {
                assert!(ws.active_editor().unwrap().read(cx).read_state(cx).selection_state() == before);
                assert!(ws.quick_panel.is_none());
            });
        }
    }
}

#[gpui::test]
fn n40_explicit_restart_cancels_pending_navigation_before_the_server_replies(cx: &mut TestAppContext) {
    for destination in ["typeDefinition", "implementation"] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-n40-restart-{destination}"), "😀target\n");
        let entered = a.with_extension("entered");
        let release = a.with_extension("release");
        rust_options(cx, |o| {
            o["navigationHoldMethod"] = destination.into();
            o["navigationEntered"] = json!(entered);
            o["navigationRelease"] = json!(release);
        });
        activate(cx);
        let (request, manager, key, token) = ws.update(cx, |ws, cx| {
            let (input, mut request) = ws.language_request(cx).unwrap();
            ws.language.manager.lock().unwrap().request(&input, &request, "textDocument/hover", json!({})).unwrap();
            request.navigation_generation = Some(ws.language.navigation_generation);
            let token = Arc::new(AtomicBool::new(false));
            ws.language.navigation_cancel = Some(token.clone());
            let key = server_key(&request.document).unwrap();
            (request, ws.language.manager.clone(), key, token)
        });
        let method = format!("textDocument/{destination}");
        let cancel = token.clone();
        let uri = lsp::uri(&a).unwrap();
        let worker = std::thread::spawn(move || {
            let mut manager = manager.lock().unwrap();
            let client = &mut manager.sessions.get_mut(&key).unwrap().as_mut().unwrap().client;
            client.request_cancellable(
                &method,
                json!({"textDocument":{"uri":uri},"position":{"line":0,"character":5}}),
                &cancel,
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !entered.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(entered.exists());
        let start = std::time::Instant::now();
        ws.update_in(cx, |ws, window, cx| ws.restart_language_servers(&RestartLanguageServers, window, cx));
        let result = worker.join().unwrap();
        assert!(result.is_err_and(|error| error.contains("cancelled")));
        assert!(token.load(Ordering::Relaxed));
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
        assert!(start.elapsed() < std::time::Duration::from_secs(4), "restart waited for the held navigation response");
        assert!(!release.exists(), "the held request was never released");
        ws.read_with(cx, |ws, cx| assert!(!ws.language_request_current(&request, cx)));
        assert!(showing(&ws, cx, &a));
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 4);
            assert!(!ws.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)));
        });
    }
}

#[gpui::test]
fn n40_late_command_response_cannot_open_after_explicit_restart(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-n40-late-restart", "😀target\n");
    activate(cx);
    let pause = crate::documents::save::gate::pause(&navigation_gate_path(&a));
    ws.update_in(cx, |_, window, cx| window.dispatch_action(LspTypeDefinition.boxed_clone(), cx));
    settle_until(cx, |_| pause.arrived() == 1);
    ws.update_in(cx, |ws, window, cx| ws.restart_language_servers(&RestartLanguageServers, window, cx));
    pause.release();
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    assert!(showing(&ws, cx, &a));
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 4);
        assert!(!ws.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)));
    });
}

#[gpui::test]
fn n40_pending_navigation_keeps_newer_palette_tab_focus_and_window_context(cx: &mut TestAppContext) {
    for mode in ["palette", "tab", "focus", "window"] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-n40-context-{mode}"), "😀target\n");
        activate(cx);
        let pause = crate::documents::save::gate::pause(&navigation_gate_path(&a));
        ws.update_in(cx, |_, window, cx| window.dispatch_action(LspImplementationToSide.boxed_clone(), cx));
        settle_until(cx, |_| pause.arrived() == 1);
        let other = ws.update_in(cx, |ws, window, cx| {
            let other = cx.focus_handle();
            match mode {
                "palette" => ws.command_palette(&CommandPalette, window, cx),
                "tab" => ws.new_file(&NewFile, window, cx),
                "focus" => window.focus(&other),
                _ => {}
            }
            other
        });
        if mode == "window" {
            cx.deactivate_window();
        }
        cx.run_until_parked();
        let before = ws.read_with(cx, |ws, _| {
            (ws.active_editor(), ws.views.groups.len(), ws.quick_panel.as_ref().map(|p| p.input.entity_id()))
        });
        pause.release();
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(
                (ws.active_editor(), ws.views.groups.len(), ws.quick_panel.as_ref().map(|p| p.input.entity_id())),
                before
            );
            assert!(!ws.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)));
        });
        drop(other);
    }
}

#[gpui::test]
fn n40_invalid_target_coordinates_never_attach_or_change_side_groups(cx: &mut TestAppContext) {
    for loaded in [false, true] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-n40-invalid-target-{loaded}"), "😀target\n");
        rust_options(cx, |o| {
            o["navigationResults"] = json!({"implementation": {
                "uri":lsp::uri(&b).unwrap(),"range":{"start":{"line":99,"character":0},"end":{"line":99,"character":1}}
            }})
        });
        if loaded {
            ws.update_in(cx, |ws, window, cx| ws.open_path(b.clone(), window, cx));
            settle_until(cx, |cx| showing(&ws, cx, &b));
            ws.update_in(cx, |ws, window, cx| ws.open_path(a.clone(), window, cx));
        }
        activate(cx);
        let origin = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, |_, window, cx| window.dispatch_action(LspImplementationToSide.boxed_clone(), cx));
        settle_until(cx, |cx| status(&ws, cx).contains("outside document"));
        ws.read_with(cx, |ws, _| {
            assert_eq!(ws.active_editor(), Some(origin));
            assert_eq!(ws.views.groups.len(), 1);
        });
        assert!(showing(&ws, cx, &a));
    }
}

#[gpui::test]
fn n40_target_reads_recheck_selection_epoch_and_completed_writer_tickets(cx: &mut TestAppContext) {
    for mode in ["selection", "epoch", "write"] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-n40-target-read-{mode}"), "😀target\n");
        activate(cx);
        let origin = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, |ws, window, cx| {
            let (_, request) = ws.language_request(cx).unwrap();
            ws.language_response(Feature::TypeDefinitionToSide,
                response(json!({"uri":lsp::uri(&b).unwrap(),"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":8}}}), a.parent().unwrap()), request, window, cx);
            match mode {
                "selection" => origin.update(cx, |editor, cx| editor.sel(cx).reselect(vec![Selection::new(1, 4)])),
                "epoch" => ws.invalidate_language_requests(cx),
                _ => { let lease = super::super::pipelines::lease_file(&b, None, cx); drop(lease); }
            }
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor(), Some(origin));
            assert_eq!(ws.views.groups.len(), 1);
            assert!(!ws.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)));
        });
        if mode == "write" {
            assert!(status(&ws, cx).contains("File changed"));
        }
    }
}

#[gpui::test]
fn n40_picker_rejects_stale_source_restart_and_focus_even_after_return(cx: &mut TestAppContext) {
    for mode in ["edit", "selection", "restart", "focus"] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-n40-stale-picker-{mode}"), "😀target\n");
        rust_options(cx, |o| {
            o["navigationResults"] = json!({"implementation":[
                {"targetUri":lsp::uri(&a).unwrap(),"targetRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":5}},"targetSelectionRange":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}}},
                {"targetUri":lsp::uri(&b).unwrap(),"targetRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":8}},"targetSelectionRange":{"start":{"line":0,"character":2},"end":{"line":0,"character":8}}}
            ]})
        });
        activate(cx);
        n40_poll(&ws, cx);
        palette_command(&ws, cx, "LSP: Go to Implementation");
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.kind == quick_panel::Kind::Choices))
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            ws.quick_panel.as_mut().unwrap().selected = 1;
            match mode {
                "edit" => {
                    ws.with_active(cx, |e, cx| {
                        e.edit(cx, |s| {
                            s.insert("changed");
                            s.set_cursor(4);
                        })
                    });
                }
                "selection" => {
                    ws.with_active(cx, |e, cx| {
                        e.sel(cx).set_cursor(0);
                        e.sel(cx).set_cursor(4);
                    });
                }
                "restart" => ws.restart_language_servers(&RestartLanguageServers, window, cx),
                _ => {
                    let other = cx.focus_handle();
                    window.focus(&other);
                }
            }
        });
        cx.run_until_parked();
        if mode == "restart" {
            settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
        }
        ws.update_in(cx, |ws, window, cx| {
            ws.quick_panel.as_ref().unwrap().input.read(cx).focus(window);
        });
        cx.run_until_parked();
        let before = ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).read_state(cx).selection_state());
        cx.simulate_keystrokes("enter");
        assert!(status(&ws, cx).contains("navigation discarded"), "{mode}: {}", status(&ws, cx));
        assert!(showing(&ws, cx, &a));
        ws.read_with(cx, |ws, cx| {
            assert!(ws.active_editor().unwrap().read(cx).read_state(cx).selection_state() == before);
            assert!(!ws.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)));
        });
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
    target_text: &str,
) -> (Entity<Workspace>, &'a mut VisualTestContext, PathBuf, PathBuf) {
    let root = temp(name).canonicalize().unwrap();
    let a = root.join("a.rs");
    let b = root.join("b.rs");
    std::fs::write(&a, "😀pri\n").unwrap();
    std::fs::write(&b, target_text).unwrap();
    let (ws, cx) = window(cx);
    cx.update(|_, cx| serve(cx, config(&b)));
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(root, window, cx);
        ws.open_path(a.clone(), window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
    });
    (ws, cx, a, b)
}

/// Whether the active view is of the file at `path`.
fn showing(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &Path) -> bool {
    ws.read_with(cx, |ws, cx| (ws.active_editor()).is_some_and(|e| e.read(cx).doc(cx).path.as_deref() == Some(path)))
}

/// The active view's whole text replaced by `text`, the caret at `cursor`.
fn replace_text(ws: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str, cursor: usize) {
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert(text);
                s.set_cursor(cursor);
            })
        })
    });
}

/// Definitions held by the server, at `b`'s target, until a `release` file appears beside
/// `a` once it has written `entered`: the two paths.
fn hold_definitions(cx: &mut VisualTestContext, a: &Path, b: &Path) -> (PathBuf, PathBuf) {
    let entered = a.parent().unwrap().join("entered");
    let release = a.parent().unwrap().join("release");
    rust_options(cx, |o| {
        *o = json!({
            "target":lsp::uri(b).unwrap(), "definitionEntered":entered, "definitionRelease":release
        })
    });
    (entered, release)
}

fn status(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    ws.read_with(cx, |ws, _| ws.status.as_ref().map(|s| s.0.clone()).unwrap_or_default())
}

/// gpui's test windows start inactive, and a completion list closes in an inactive window.
/// Edits the Rust server's initialization options, which script the test server.
fn rust_options(cx: &mut VisualTestContext, edit: impl FnOnce(&mut serde_json::Value)) {
    cx.update(|_, cx| {
        let mut settings = cx.global::<Settings>().clone();
        edit(&mut (settings.language_servers.get_mut("rust").unwrap()).initialization_options);
        cx.set_global(settings);
    });
}

fn activate(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
}

/// The completion list in the active view as shown, best first; empty while it is not showing.
fn rows(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<crate::languages::complete::Candidate> {
    ws.read_with(cx, |ws, cx| (ws.active_editor()).map_or_else(Vec::new, |e| e.read(cx).completion_rows()))
}

fn listed(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    rows(ws, cx).into_iter().map(|c| c.label).collect()
}

fn shown(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<(Vec<String>, usize)> {
    ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).completion_list())
}

/// The list's selected row.
fn selected(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    let (labels, at) = shown(ws, cx).expect("the list is showing");
    labels[at].clone()
}

/// Select the list's row `label` with the keys.
fn select(ws: &Entity<Workspace>, cx: &mut VisualTestContext, label: &str) {
    let (labels, at) = shown(ws, cx).expect("the list is showing");
    let to = labels.iter().position(|l| l == label).expect(label);
    for _ in at..to {
        cx.simulate_keystrokes("down");
    }
}

/// Commit the list's row `label` with the keys: down to it, then Enter.
fn pick(ws: &Entity<Workspace>, cx: &mut VisualTestContext, label: &str) {
    select(ws, cx, label);
    cx.simulate_keystrokes("enter");
}

/// Whether the active view's open list waits for the server's items; `None`: no list is open.
fn waiting(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<bool> {
    ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).completion_waiting())
}

/// LSP: Complete in the active view.
fn lsp_complete(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.lsp_completion(&LspCompletion, window, cx));
}

/// The completion requests the server has had.
fn asked(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    count(&captured(ws, cx, None), "requests", "textDocument/completion")
}

#[gpui::test]
fn completion_resolves_plain_text_in_one_undo_and_updates_shared_views(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-lsp-ui-completion", "😀target\n");
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
        ws.lsp_completion(&LspCompletion, window, cx);
    });
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
    assert!(listed(&ws, cx).contains(&"print snippet".into()), "snippet item must be offered");
    pick(&ws, cx, "print");
    settle_until(cx, |cx| text(&ws, cx) == "😀print\nimported\n");
    let editors = ws.read_with(cx, |ws, cx| ws.open_editors(cx));
    assert_eq!(editors.len(), 2);
    for editor in &editors {
        assert_eq!(editor.read_with(cx, |e, cx| e.doc(cx).text()), "😀print\nimported\n");
    }
    editors[0].update(cx, |e, cx| e.edit(cx, |s| s.undo()));
    for editor in &editors {
        assert_eq!(editor.read_with(cx, |e, cx| e.doc(cx).text()), "😀pri\n");
    }
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
}

#[gpui::test]
fn server_snippet_completion_reuses_tabstops_and_shares_atomic_undo(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-snippet", "😀target\n");
    rust_options(cx, |o| o["importFirst"] = true.into());
    activate(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
        ws.lsp_completion(&LspCompletion, window, cx);
    });
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print snippet".into()));
    pick(&ws, cx, "print snippet");
    settle_until(cx, |cx| text(&ws, cx) == "imported\n😀f(中, 中, one)\n");
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        assert!(e.read(cx).state.in_snippet());
        assert_eq!(e.read(cx).state.selections[0].range(), 12..13);
        for e in ws.open_editors(cx) {
            assert_eq!(e.read(cx).doc(cx).text(), "imported\n😀f(中, 中, one)\n");
        }
    });
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn n31_hidden_source_view_dismisses_owned_choice_list_on_blur(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    activate(cx);
    let source = ws.update(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        e.update(cx, |e, cx| e.edit(cx, |s| s.complete_snippet_ranges(0..0, "${1|one,two|} $0", Vec::new()).unwrap()));
        e
    });
    cx.run_until_parked();
    assert!(source.read_with(cx, |e, _| e.snippet_choice_list().is_some()));
    ws.update_in(cx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    cx.run_until_parked();
    assert!(source.read_with(cx, |e, _| e.snippet_choice_list().is_none()), "hidden source keeps no stale picker");
    assert!(source.read_with(cx, |e, _| e.state.in_snippet()), "blur preserves ordinary field navigation");
}

fn n31_empty_choice<'a>(name: &str, cx: &'a mut TestAppContext) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (ws, cx, _, _) = open(cx, name, "target\n");
    rust_options(cx, |o| {
        o["completionItems"] = json!([
            {"label":"n31-alpha", "insertText":"ALPHA"},
            {"label":"n31-beta", "insertText":"BETA"}
        ]);
        o["signatureResult"] = json!({"signatures":[{"label":"first()"},{"label":"second()"}]});
    });
    activate(cx);
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| s.complete_snippet_ranges(0..4, "${1|,foo|}${2:end}$0", Vec::new()).unwrap());
        })
    });
    cx.run_until_parked();
    assert_eq!(text(&ws, cx), "end\n");
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).snippet_choice_list().is_some()));
    (ws, cx)
}

#[gpui::test]
fn n31_explicit_completion_owns_keys_over_an_empty_choice(cx: &mut TestAppContext) {
    let (ws, cx) = n31_empty_choice("orlo-n31-popup-completion", cx);
    cx.simulate_keystrokes("ctrl-space");
    settle_until(cx, |cx| listed(&ws, cx).contains(&"n31-beta".into()));
    let row = listed(&ws, cx).iter().position(|label| label == "n31-beta").unwrap();
    assert!(row > 0, "exercise keyboard selection rather than the initial completion");
    for _ in 0..row {
        cx.simulate_keystrokes("down");
    }
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.completion_list().unwrap().1, row, "keys belong to the displayed server list");
        assert!(e.snippet_choice_list().is_none());
        assert!(e.state.in_snippet(), "popup dismissal retains field navigation");
    });
    cx.simulate_keystrokes("escape tab");
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.doc(cx).rope.slice(e.state.selections[0].range()).to_string(), "end");
    });
    cx.simulate_keystrokes("shift-tab ctrl-space");
    settle_until(cx, |cx| listed(&ws, cx).contains(&"n31-beta".into()));
    let row = listed(&ws, cx).iter().position(|label| label == "n31-beta").unwrap();
    for _ in 0..row {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("enter");
    settle_until(cx, |cx| text(&ws, cx) == "BETAend\n");
}

#[gpui::test]
fn n31_explicit_signature_owns_overload_keys_over_an_empty_choice(cx: &mut TestAppContext) {
    let (ws, cx) = n31_empty_choice("orlo-n31-popup-signature", cx);
    ws.update_in(cx, |ws, w, cx| ws.lsp_signature_help(&LspSignatureHelp, w, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    cx.simulate_keystrokes("down");
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.signature_help.as_ref().unwrap().help.active, 1, "keys navigate the displayed overloads");
        assert!(e.snippet_choice_list().is_none());
        assert!(e.state.in_snippet());
    });
    assert_eq!(text(&ws, cx), "end\n");
    cx.simulate_keystrokes("escape tab");
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.doc(cx).rope.slice(e.state.selections[0].range()).to_string(), "end");
    });
    cx.simulate_keystrokes("shift-tab down enter");
    assert_eq!(text(&ws, cx), "fooend\n");
}

#[gpui::test]
fn n32_server_snippet_captures_runtime_values_at_actual_commit(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "orlo-n32-lsp-context", "😀target\n");
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    cx.update(|_, cx| editor::fixed_clock(cx, calls.clone()));
    rust_options(cx, |o| {
        o["completionItems"] = json!([{"label":"print clock", "insertTextFormat":2,
            "textEdit":{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},
                "newText":"$CURRENT_YEAR-$CURRENT_MONTH-$CURRENT_DATE $CURRENT_SECONDS_UNIX $CLIPBOARD ${WORKSPACE_NAME:outside} $RELATIVE_FILEPATH ${1|one,two|}=$1$0"},
            "data":{"resolve":true}}]);
        o["importFirst"] = true.into();
    });
    activate(cx);
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4)));
    lsp_complete(&ws, cx);
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print clock".into()));
    assert_eq!(calls.get(), 0);
    cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("界$(date)".into())));
    pick(&ws, cx, "print clock");
    let root = path.parent().unwrap().file_name().unwrap().to_string_lossy();
    let initial = format!("imported\n😀2024-02-29 1709251645 界$(date) {root} a.rs one=one\n");
    settle_until(cx, |cx| text(&ws, cx) == initial);
    assert_eq!(calls.get(), 1);
    cx.simulate_keystrokes("down enter");
    assert_eq!(text(&ws, cx), initial.replace("one=one", "two=two"));
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn n31_server_choice_values_survive_real_completion_and_provider_shutdown(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "orlo-n31-lsp-owned-choice", "😀target\n");
    rust_options(cx, |o| {
        o["completionItems"] = json!([{"label":"print choice", "insertTextFormat":2,
            "textEdit":{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},
                "newText":r"${1|a\,b,c\|d,e\\f,界,|} $1 ${1/(.*)/<$1>/} ${2:next}$0"},
            "data":{"resolve":true}}]);
        o["importFirst"] = true.into();
    });
    activate(cx);
    ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4)));
    lsp_complete(&ws, cx);
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print choice".into()));
    pick(&ws, cx, "print choice");
    let initial = "imported\n😀a,b a,b <a,b> next\n";
    settle_until(cx, |cx| text(&ws, cx) == initial);
    let choice = |cx: &mut VisualTestContext| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).snippet_choice_list())
    };
    assert_eq!(choice(cx), Some((vec!["a,b".into(), "c|d".into(), r"e\f".into(), "界".into(), "".into()], 0)));
    cx.update(|_, cx| {
        let mut settings = cx.global::<Settings>().clone();
        settings.lsp_enabled = false;
        cx.set_global(settings);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, w, cx| {
        let input = ws.language_input(cx);
        let mut manager = ws.language.manager.lock().unwrap();
        manager.sync(&input);
        assert!(manager.sessions.is_empty(), "the actual provider has shut down");
        assert!(ws.active_editor().unwrap().read(cx).is_focused(w));
    });
    assert!(choice(cx).is_some(), "shutdown must keep the owned alternatives");
    cx.simulate_keystrokes("down enter");
    let chosen = "imported\n😀c|d c|d <c|d> next\n";
    assert_eq!(text(&ws, cx), chosen);
    assert!(choice(cx).is_none());
    cx.simulate_keystrokes("tab shift-tab");
    assert_eq!(choice(cx).unwrap().1, 1, "return selects the current value");
    cx.simulate_keystrokes("escape");
    cx.simulate_input("é");
    let typed = "imported\n😀é é <é> next\n";
    assert_eq!(text(&ws, cx), typed);
    for (key, expected) in [
        ("cmd-z", chosen),
        ("cmd-z", initial),
        ("cmd-z", "😀pri\n"),
        ("cmd-shift-z", initial),
        ("cmd-shift-z", chosen),
        ("cmd-shift-z", typed),
    ] {
        cx.simulate_keystrokes(key);
        ws.read_with(cx, |ws, cx| {
            for e in ws.open_editors(cx) {
                assert_eq!(e.read(cx).doc(cx).text(), expected);
            }
        });
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), "😀pri\n");
}

// These server-scripted combinations exercise the picker, negotiated protocol coordinates,
// snippet model and shared history together; local snippet grammar tests cover their primitives.
#[gpui::test]
fn server_snippet_explicit_ranges_nesting_transforms_and_shared_redo(cx: &mut TestAppContext) {
    for (encoding, emoji) in [("utf-8", 4), ("utf-16", 2), ("utf-32", 1)] {
        for replace in [false, true] {
            let name = format!("orlo-lsp-snippet-explicit-{encoding}-{replace}");
            let (ws, cx, path, _) = open(cx, &name, "😀target\n");
            let original = "😀priextra\n😀old\n";
            replace_text(&ws, cx, original, 4);
            let span = |line, start, end| {
                json!({"start":{"line":line,"character":start},
                "end":{"line":line,"character":end}})
            };
            let mut edit = json!({"newText":r"${1:中 ${2:word}} / $2 / ${2/(.)/\u$1/}$0!"});
            if replace {
                edit["insert"] = span(0, emoji, emoji + 3);
                edit["replace"] = span(0, emoji, emoji + 8);
            } else {
                edit["range"] = span(0, emoji, emoji + 3);
            }
            rust_options(cx, |o| {
                o["encoding"] = encoding.into();
                o["completionItems"] = json!([{"label":"print snippet", "insertTextFormat":2,
                    "textEdit":edit,"data":{"resolve":true}}]);
                o["completionAdditional"] = json!([{"range":span(1, emoji, emoji + 3), "newText":"新"}]);
            });
            activate(cx);
            ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
            ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4)));
            lsp_complete(&ws, cx);
            settle_until(cx, |cx| listed(&ws, cx).contains(&"print snippet".into()));
            pick(&ws, cx, "print snippet");
            let suffix = if replace { "" } else { "extra" };
            let expanded = format!("😀中 word / word / Word!{suffix}\n😀新\n");
            settle_until(cx, |cx| text(&ws, cx) == expanded);
            let selected_texts = |cx: &mut VisualTestContext| {
                ws.read_with(cx, |ws, cx| {
                    let e = ws.active_editor().unwrap().read(cx);
                    e.state.selections.iter().map(|s| e.doc(cx).rope.slice(s.range()).to_string()).collect::<Vec<_>>()
                })
            };
            assert_eq!(selected_texts(cx), ["中 word"]);
            cx.simulate_keystrokes("tab");
            assert_eq!(selected_texts(cx), ["word", "word"]);
            cx.simulate_keystrokes("shift-tab");
            assert_eq!(selected_texts(cx), ["中 word"]);
            cx.simulate_keystrokes("tab");
            cx.simulate_input("é");
            let edited = format!("😀中 é / é / É!{suffix}\n😀新\n");
            assert_eq!(text(&ws, cx), edited);
            cx.simulate_keystrokes("tab");
            assert_eq!(selections(&ws, cx), caret("😀中 é / é / É".chars().count()));
            ws.read_with(cx, |ws, cx| assert!(!ws.active_editor().unwrap().read(cx).state.in_snippet()));
            let editors = ws.read_with(cx, |ws, _| ws.views.tabs().cloned().collect::<Vec<_>>());
            assert_eq!(editors.len(), 2);
            for (redo, expected) in
                [(false, expanded.as_str()), (false, original), (true, expanded.as_str()), (true, edited.as_str())]
            {
                ws.update(cx, |ws, cx| {
                    ws.with_active(cx, |e, cx| {
                        e.edit(cx, |s| {
                            if redo {
                                s.redo();
                            } else {
                                s.undo();
                            }
                        })
                    })
                });
                for editor in &editors {
                    assert_eq!(editor.read_with(cx, |e, cx| e.doc(cx).text()), expected);
                }
            }
            assert_eq!(std::fs::read_to_string(path).unwrap(), "😀pri\n", "completion never saves");
        }
    }
}

#[gpui::test]
fn server_snippet_keeps_deliberately_typed_prefix_mapping(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-snippet-type-on", "😀target\n");
    activate(cx);
    cx.simulate_keystrokes("ctrl-space");
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print snippet".into()));
    cx.simulate_input("n");
    pick(&ws, cx, "print snippet");
    settle_until(cx, |cx| text(&ws, cx) == "😀f(中, 中, one)\nimported\n");
    assert_eq!(asked(&ws, cx), 1);
    assert_eq!(selections(&ws, cx), vec![Selection::new(3, 4), Selection::new(6, 7)]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, cx), "😀prin\n");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, cx), "😀f(中, 中, one)\nimported\n");
}

#[gpui::test]
fn server_snippet_invalid_syntax_or_overlap_reports_failure_without_edits(cx: &mut TestAppContext) {
    for overlap in [false, true] {
        let (ws, cx, _, _) = open(cx, &format!("orlo-lsp-snippet-invalid-{overlap}"), "😀target\n");
        rust_options(cx, |o| {
            o["completionItems"] = json!([{"label":"print snippet", "insertTextFormat":2,
                "insertText":if overlap { "${1:ok}$0" } else { "${1/x/y/z}" }, "data":{"resolve":true}}]);
            if overlap {
                o["completionAdditional"] = json!([{"range":{"start":{"line":0,"character":3},
                    "end":{"line":0,"character":4}}, "newText":"bad"}]);
            }
        });
        activate(cx);
        lsp_complete(&ws, cx);
        settle_until(cx, |cx| listed(&ws, cx).contains(&"print snippet".into()));
        let before = ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap().read(cx);
            (e.doc(cx).version, e.state.selections.clone())
        });
        pick(&ws, cx, "print snippet");
        settle_until(cx, |cx| !status(&ws, cx).is_empty());
        let error = status(&ws, cx);
        assert!(
            if overlap { error.to_lowercase().contains("overlap") } else { error.contains("line 1, column") },
            "{error}"
        );
        assert_eq!(text(&ws, cx), "😀pri\n");
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap().read(cx);
            assert_eq!((e.doc(cx).version, e.state.selections.clone()), before);
            assert!(!e.state.in_snippet());
        });
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
        assert_eq!(text(&ws, cx), "😀pri\n", "failure must not create an undo record");
    }
}

#[gpui::test]
fn server_snippet_delayed_resolve_respects_readonly_follow_composition_and_version(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in ["read-only", "follow", "composition", "version"] {
        let (ws, cx, path, _) = open(cx, &format!("orlo-lsp-snippet-guard-{mode}"), "😀target\n");
        activate(cx);
        lsp_complete(&ws, cx);
        settle_until(cx, |cx| listed(&ws, cx).contains(&"print snippet".into()));
        let pause = crate::documents::save::gate::pause(&completion_gate_path(&path));
        pick(&ws, cx, "print snippet");
        settle_until(cx, |_| pause.arrived() == 1);
        let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        editor.update_in(cx, |e, window, cx| match mode {
            "read-only" => e.update_doc(cx, |d| d.read_only = true),
            "follow" => e.update_doc(cx, |d| {
                d.set_following(true);
            }),
            "composition" => e.replace_and_mark_text_in_range(None, "中", None, window, cx),
            _ => e.edit(cx, |s| {
                s.set_cursor(5);
                s.insert("later");
                s.set_cursor(4);
            }),
        });
        let before =
            editor.read_with(cx, |e, cx| (e.text(cx), e.doc(cx).version, e.state.selections.clone(), e.marked.clone()));
        pause.release();
        settle_until(cx, |cx| !status(&ws, cx).is_empty());
        assert!(status(&ws, cx).contains("discarded") || status(&ws, cx).contains("read only"), "{}", status(&ws, cx));
        editor.read_with(cx, |e, cx| {
            assert_eq!((e.text(cx), e.doc(cx).version, e.state.selections.clone(), e.marked.clone()), before);
            assert!(!e.state.in_snippet());
        });
        assert_eq!(std::fs::read_to_string(path).unwrap(), "😀pri\n");
    }
}

#[gpui::test]
fn closing_completion_cancels_resolve_before_any_additional_edits_apply(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-snippet-cancel", "😀target\n");
    activate(cx);
    lsp_complete(&ws, cx);
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print snippet".into()));
    // Cancel after scheduling the resolve, before its callback. GPUI's test executor runs
    // synchronous background requests to completion when simulated keys drain it.
    ws.update_in(cx, |ws, window, cx| {
        let index = ws
            .language
            .completion
            .as_ref()
            .unwrap()
            .2
            .iter()
            .position(|item| item["label"] == "print snippet")
            .unwrap();
        ws.commit_server_item(index, Vec::new(), window, cx);
        ws.active_editor().unwrap().update(cx, |e, cx| e.close_completion(cx));
    });
    settle_until(cx, |cx| status(&ws, cx).contains("discarded"));
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn signature_help_highlights_the_current_unicode_parameter_and_escape_dismisses(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-signature", "😀target\n");
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        let result = &e.signature_help.as_ref().unwrap().help;
        let help = &result.signatures[result.active];
        assert_eq!(&help.label[help.parameter.clone().unwrap()], "value: int");
        assert_eq!(e.doc(cx).text(), "😀pri\n");
    });
    cx.simulate_keystrokes("escape");
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
    assert_eq!(count(&captured(&ws, cx, None), "requests", "textDocument/signatureHelp"), 1);
}

#[gpui::test]
fn mouse_caret_and_superseding_navigation_dismiss_signature_help(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-signature-lifecycle", "😀target\n");
    activate(cx);
    let show = |cx: &mut VisualTestContext| {
        ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
        settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    };
    show(cx);
    let point = ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).char_bounds(0, cx).unwrap().center());
    cx.simulate_mouse_down(point, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(point, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
    show(cx);
    ws.update(cx, |ws, cx| {
        ws.invalidate_navigation();
        cx.notify();
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
}

#[gpui::test]
fn resized_signature_request_does_not_create_a_late_popup(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-signature-resize", "😀target\n");
    activate(cx);
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |_| pause.arrived() == 1);
    cx.simulate_resize(gpui::size(px(900.), px(500.)));
    pause.release();
    settle_until(cx, |cx| count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/signatureHelp") == 1);
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
}

#[gpui::test]
fn range_formatting_sends_the_unicode_selection_and_keeps_document_formatting(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-lsp-range-format", "😀target\n");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.select_range(1..4, cx));
        ws.lsp_format_selection(&LspFormatSelection, window, cx);
    });
    settle_until(cx, |cx| text(&ws, cx) == "😀selected\n");
    let state = captured(&ws, cx, None);
    let request =
        state["requests"].as_array().unwrap().iter().find(|r| r["method"] == "textDocument/rangeFormatting").unwrap();
    assert_eq!(request["params"]["options"], json!({"tabSize":4,"insertSpaces":true}));
    assert_eq!(request["params"]["range"], json!({"start":{"line":0,"character":2},"end":{"line":0,"character":5}}));
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
        ws.lsp_format(&LspFormat, window, cx);
    });
    settle_until(cx, |cx| text(&ws, cx) == "😀formatted\n");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
}

// Reverting to primary-only formatting leaves this empty primary unformatted and sends
// no requests; applying replies independently also breaks the single Undo assertion.
#[gpui::test]
fn range_formatting_batches_all_physical_unicode_ranges_in_one_undo(cx: &mut TestAppContext) {
    for (encoding, first_end, second_start, second_end) in
        [("utf-8", 9, 5, 8), ("utf-16", 5, 3, 5), ("utf-32", 4, 2, 4)]
    {
        let (ws, cx, a, _) = open(cx, &format!("orlo-range-batch-{encoding}"), "target\n");
        let original = "😀\t雪a\n🦀\tbé\nkeep\n";
        replace_text(&ws, cx, original, 14);
        rust_options(cx, |o| o["encoding"] = encoding.into());
        let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        let other = ws.update_in(cx, |ws, window, cx| {
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            let other = ws.active_editor().unwrap();
            ws.activate_view(source.entity_id(), window, cx);
            other
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| {
                e.set_virtual_space(true, cx);
                e.sel(cx).reselect(vec![
                    Selection::new(1, 4).in_rectangle(),
                    Selection::new(9, 7),
                    Selection::from_points((14, 1), (14, 3)).in_rectangle(),
                ]);
            });
            ws.lsp_format_selection(&LspFormatSelection, window, cx);
        });
        settle_until(cx, |cx| status(&ws, cx) != "Language server: working…");
        let formatted = "😀selected\n🦀\tselected\nkeep\n";
        assert_eq!(text(&ws, cx), formatted, "{encoding}: {}", status(&ws, cx));
        assert_eq!(other.read_with(cx, |e, cx| e.doc(cx).text()), formatted);
        source.read_with(cx, |e, _| {
            assert_eq!(e.state.selections.iter().map(|s| s.rectangular).collect::<Vec<_>>(), vec![true, false, true]);
            let last = e.state.selections.last().unwrap();
            assert_eq!((last.anchor_virtual, last.head_virtual), (1, 3));
        });
        let state = captured(&ws, cx, Some(&a));
        let ranges = state["requests"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "textDocument/rangeFormatting")
            .map(|r| {
                assert_eq!(r["params"]["options"], json!({"tabSize":4,"insertSpaces":true}));
                r["params"]["range"].clone()
            })
            .collect::<Vec<_>>();
        let first_start = if encoding == "utf-8" {
            4
        } else if encoding == "utf-16" {
            2
        } else {
            1
        };
        assert_eq!(
            ranges,
            vec![
                json!({"start":{"line":0,"character":first_start},"end":{"line":0,"character":first_end}}),
                json!({"start":{"line":1,"character":second_start},"end":{"line":1,"character":second_end}})
            ]
        );
        assert_eq!(count(&state, "requests", "textDocument/formatting"), 0);
        assert!(state["errors"].as_array().unwrap().is_empty());
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
        assert_eq!(text(&ws, cx), original, "one Undo restores both ranges");
        assert_eq!(other.read_with(cx, |e, cx| e.doc(cx).text()), original);
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.redo())));
        assert_eq!(text(&ws, cx), formatted);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
    }
}

pub(super) fn range_gate_path(path: &Path) -> PathBuf {
    path.with_extension("range-format-gate")
}

#[gpui::test]
fn range_formatting_rejects_complete_selection_changes_and_no_frame_away_back(cx: &mut TestAppContext) {
    for mode in ["secondary", "away-back", "rectangular", "virtual", "goal"] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-range-selection-{mode}"), "target");
        let pause = crate::documents::save::gate::pause(&range_gate_path(&a));
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.sel(cx).reselect(vec![Selection::new(0, 1), Selection::new(3, 4)]));
            ws.lsp_format_selection(&LspFormatSelection, window, cx);
        });
        // The real batch has returned, but it has not passed UI admission yet.
        settle_until(cx, |_| pause.arrived() == 1);
        ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| {
                let mut sels = e.state.selections.clone();
                match mode {
                    "secondary" => sels[0] = Selection::new(1, 2),
                    "rectangular" => sels[0].rectangular = true,
                    "virtual" => sels[0].anchor_virtual = 1,
                    "goal" => sels[0].goal = Some(crate::editing::text::Goal::LineEnd),
                    _ => {
                        e.sel(cx).set_cursor(2);
                        e.sel(cx).reselect(sels.clone());
                    }
                }
                e.sel(cx).reselect(sels);
            });
        });
        pause.release();
        settle_until(cx, |cx| status(&ws, cx) != "Language server: working…");
        assert_eq!(text(&ws, cx), "😀pri\n", "{mode}: {}", status(&ws, cx));
        assert!(status(&ws, cx).contains("discarded"), "{mode}: {}", status(&ws, cx));
    }
}

fn range_format_menu(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    ws.read_with(cx, |ws, cx| {
        ws.menus(cx)
            .into_iter()
            .find(|m| m.name == "Goto")
            .unwrap()
            .items
            .into_iter()
            .find_map(|item| {
                match item {
            MenuItem::Submenu(menu) if menu.name == "Language Server" => Some(menu.items.into_iter().any(|item|
                matches!(item, MenuItem::Action { action, .. } if action.as_any().is::<LspFormatSelection>()))),
            _ => None,
        }
            })
            .unwrap()
    })
}

#[gpui::test]
fn range_formatting_menu_and_palette_require_advertised_capability(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-range-capabilities", "target");
    assert!(!range_format_menu(&ws, cx), "unknown capability is not exposed");
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    assert!(range_format_menu(&ws, cx));
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).reselect(vec![Selection::new(1, 2), Selection::new(3, 4)]))
    });
    activate(cx);
    palette_command(&ws, cx, "LSP: Format Selection");
    settle_until(cx, |cx| status(&ws, cx).contains("applied"));
    assert_eq!(text(&ws, cx), "😀selectedrselected\n");
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| s.undo());
            e.sel(cx).set_cursor(4);
        })
    });
    palette_command(&ws, cx, "LSP: Format Selection");
    assert!(status(&ws, cx).contains("Select a range"));
    rust_options(cx, |o| o["noRangeFormatting"] = true.into());
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    assert!(!range_format_menu(&ws, cx));
    ws.update_in(cx, |ws, window, cx| ws.command_palette(&CommandPalette, window, cx));
    ws.read_with(cx, |ws, _| {
        assert!(ws.quick_panel.as_ref().unwrap().items.iter().all(
            |item| !matches!(&item.action, ItemAction::Dispatch(action) if action.as_any().is::<LspFormatSelection>())
        ))
    });
}

#[gpui::test]
fn range_formatting_split_focus_rebuilds_the_native_menu_with_equal_delimiters(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-range-split-native-menu", "target");
    activate(cx);
    let supported = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let plain_path = a.parent().unwrap().join("plain.txt");
    std::fs::write(&plain_path, "plain\n").unwrap();
    let plain = ws.update_in(cx, |ws, window, cx| {
        ws.set_layout(&SetLayout { preset: Preset::Columns(2) }, window, cx);
        ws.focus_group(&FocusGroup { group: 1 }, window, cx);
        ws.open_path(plain_path, window, cx);
        ws.active_editor().unwrap()
    });
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    cx.run_until_parked();
    ws.update(cx, |ws, cx| assert!(ws.set_menus_now(cx)));
    assert!(!range_format_menu(&ws, cx));
    assert_eq!(
        supported.read_with(cx, |e, cx| e.doc(cx).delimiter_override()),
        plain.read_with(cx, |e, cx| e.doc(cx).delimiter_override()),
        "the delimiter-only refresh condition cannot distinguish these views"
    );
    let left = supported.read_with(cx, |e, cx| e.char_bounds(0, cx).unwrap().center());
    let right = plain.read_with(cx, |e, cx| e.char_bounds(0, cx).unwrap().center());
    assert!((left.x - right.x).abs() > gpui::px(20.), "both groups are visibly distinct");
    for (target, available) in [(&supported, true), (&plain, false), (&supported, true), (&plain, false)] {
        let before = ws.read_with(cx, |ws, _| ws.menus_built);
        let point = target.read_with(cx, |e, cx| e.char_bounds(0, cx).unwrap().center());
        cx.simulate_click(point, gpui::Modifiers::none());
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert_eq!(ws.active_editor().unwrap().entity_id(), target.entity_id()));
        cx.update(|window, cx| assert!(target.read(cx).focus_handle(cx).is_focused(window)));
        // An ordinary poll sees the already-selected view on both sides of its comparison.
        ws.update(cx, |ws, cx| ws.poll_language(cx));
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
        // TestPlatform discards menus. This counter advances only after the real
        // active-window set_menus call, whose stable payload comes from this factory.
        assert!(
            ws.read_with(cx, |ws, _| ws.menus_built) > before,
            "focus changes must publish a new native menu, availability={available}"
        );
        assert_eq!(range_format_menu(&ws, cx), available);
        assert_eq!(ws.read_with(cx, |ws, _| ws.menu_range_formatting), available);
        assert!(!ws.read_with(cx, |ws, _| ws.menus_due));
    }
    assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/rangeFormatting"), 0);
}

#[gpui::test]
fn range_formatting_modeline_route_loss_rebuilds_the_installed_menu_after_poll(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-range-modeline-native-menu", "target");
    activate(cx);
    cx.update(|_, cx| {
        let mut settings = cx.global::<Settings>().clone();
        settings.modelines = true;
        cx.set_global(settings);
    });
    ws.update(cx, |ws, cx| {
        ws.apply_view(cx);
        ws.poll_language(cx);
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    assert!(ws.read_with(cx, |ws, cx| ws.has_range_formatting(cx)));
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("// vim: set ft=rust:\nlet value = 1;\n");
            })
        })
    });
    cx.run_until_parked();
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    ws.update(cx, |ws, cx| {
        assert!(ws.has_range_formatting(cx));
        assert!(ws.set_menus_now(cx));
    });
    let before = ws.read_with(cx, |ws, _| ws.menus_built);
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("// vim: set ft=plain:\nlet value = 1;\n");
            })
        })
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).modeline_language.as_deref(), Some("Plain Text"));
        assert!(!ws.has_range_formatting(cx));
    });
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    assert!(
        ws.read_with(cx, |ws, _| ws.menus_built) > before,
        "a lost route must retire the previously installed action"
    );
    assert!(!ws.read_with(cx, |ws, _| ws.menu_range_formatting));
    assert!(!range_format_menu(&ws, cx));
}

#[gpui::test]
fn range_formatting_skips_empty_and_pure_virtual_selections(cx: &mut TestAppContext) {
    for virtual_only in [false, true] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-range-empty-{virtual_only}"), "target");
        ws.update(cx, |ws, cx| ws.poll_language(cx));
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| {
                e.set_virtual_space(true, cx);
                let last = if virtual_only {
                    Selection::from_points((4, 1), (4, 3)).in_rectangle()
                } else {
                    Selection::caret(4)
                };
                e.sel(cx).reselect(vec![Selection::caret(0), last]);
            });
            let e = ws.active_editor().unwrap();
            let before = e.read(cx).read_state(cx).selection_state();
            let version = e.read(cx).doc(cx).version;
            ws.lsp_format_selection(&LspFormatSelection, window, cx);
            assert!(before == e.read(cx).read_state(cx).selection_state());
            assert_eq!(version, e.read(cx).doc(cx).version);
        });
        assert!(status(&ws, cx).contains("Select a range"));
        let state = captured(&ws, cx, Some(&a));
        assert_eq!(count(&state, "requests", "textDocument/rangeFormatting"), 0);
        assert_eq!(count(&state, "requests", "textDocument/formatting"), 0);
        assert_eq!(text(&ws, cx), "😀pri\n");
    }
}

#[gpui::test]
fn range_formatting_rejects_overlapping_source_ranges_before_server_start(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-range-overlap-input", "target");
    let (input, mut request) = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap());
    request.format_selection = Some(ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        let state = editor.read(cx).read_state(cx);
        let mut origin = FormatSelection {
            editor: editor.entity_id(),
            generation: state.selection_generation(),
            state: state.selection_state(),
        };
        // Normal UI selection placement merges overlaps. Exercise the request trust boundary too.
        origin.state.selections = vec![Selection::new(1, 3), Selection::new(2, 4)];
        origin
    }));
    let mut manager = Manager::default();
    let result = manager.request(
        &input,
        &request,
        "textDocument/rangeFormatting",
        json!({"options":{"tabSize":4,"insertSpaces":true}}),
    );
    assert!(result.err().unwrap().contains("Overlapping formatting selections"));
    assert!(manager.sessions.is_empty(), "invalid requests do not start a server");
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn range_formatting_rejects_invalid_batches_without_partial_changes(cx: &mut TestAppContext) {
    let first = json!({"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":3}},"newText":"ONE"});
    let second = json!({"range":{"start":{"line":0,"character":4},"end":{"line":0,"character":5}},"newText":"TWO"});
    let insert = json!({"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":2}},"newText":"insert"});
    let split = json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":2}},"newText":"split"});
    for (name, replies, error_at, expected, error) in [
        ("null", json!([null, null]), 0, "😀pri\n", "No edits returned"),
        ("one-null", json!([null, [second.clone()]]), 0, "😀prTWO\n", "applied"),
        ("error", json!([[first.clone()], [second]]), 2, "😀pri\n", "fixture range failure"),
        ("malformed", json!([[first.clone()], {}]), 0, "😀pri\n", "edits must be an array"),
        ("unicode", json!([[first.clone()], [split]]), 0, "😀pri\n", "LSP"),
        ("overlap", json!([[first.clone()], [first.clone()]]), 0, "😀pri\n", "Overlapping"),
        ("insert-overlap", json!([[insert.clone()], [insert]]), 0, "😀pri\n", "Overlapping"),
        ("internal-overlap", json!([[first.clone()], [first.clone(), first]]), 0, "😀pri\n", "Overlapping"),
    ] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-range-reply-{name}"), "target");
        rust_options(cx, |o| {
            o["rangeResults"] = replies;
            o["rangeErrorAt"] = error_at.into();
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.sel(cx).reselect(vec![Selection::new(1, 2), Selection::new(3, 4)]));
            ws.lsp_format_selection(&LspFormatSelection, window, cx);
        });
        settle_until(cx, |cx| status(&ws, cx) != "Language server: working…");
        assert_eq!(text(&ws, cx), expected, "{name}: {}", status(&ws, cx));
        assert!(status(&ws, cx).contains(error), "{name}: {}", status(&ws, cx));
        let state = captured(&ws, cx, Some(&a));
        assert_eq!(count(&state, "requests", "textDocument/rangeFormatting"), 2, "{name}");
        assert_eq!(state["rangeSnapshots"][0], state["rangeSnapshots"][1], "{name}: one original server version");
        assert_eq!(state["rangeSnapshots"][0]["text"], "😀pri\n");
        let before_undo = ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).version);
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
        if name != "one-null" {
            assert_eq!(
                before_undo,
                ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).version),
                "{name}: a discarded/no-op batch creates no undo step"
            );
        }
        assert_eq!(text(&ws, cx), "😀pri\n");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
    }
}

#[gpui::test]
fn range_formatting_cancellation_during_second_reply_returns_no_partial_batch(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-range-transport-cancel", "target");
    let entered = a.with_extension("range-entered");
    let release = a.with_extension("range-release");
    rust_options(cx, |o| {
        o["rangeHoldAt"] = 2.into();
        o["rangeEntered"] = json!(entered);
        o["rangeRelease"] = json!(release);
    });
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).reselect(vec![Selection::new(1, 2), Selection::new(3, 4)]))
    });
    let (input, mut request) = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap());
    request.format_selection = Some(ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        let view = editor.read(cx).read_state(cx);
        FormatSelection {
            editor: editor.entity_id(),
            generation: view.selection_generation(),
            state: view.selection_state(),
        }
    }));
    let document = request.document.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let returned = Arc::new(AtomicBool::new(false));
    let observed_return = returned.clone();
    // Keep Manager and its captured registry on their original test thread.
    // Only the observer runs independently of GPUI's blocking test executor.
    let observer = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !entered.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(entered.exists());
        assert!(!observed_return.load(Ordering::Relaxed), "the first reply cannot return a partial response");
        flag.store(true, Ordering::Relaxed);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !observed_return.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(observed_return.load(Ordering::Relaxed), "cancellation must release the request");
    });
    let mut manager = Manager::default();
    let result = manager.request_with_cancel(
        &input,
        &request,
        "textDocument/rangeFormatting",
        json!({"options":{"tabSize":4,"insertSpaces":true}}),
        Some(&cancel),
    );
    returned.store(true, Ordering::Relaxed);
    std::fs::write(&release, "release").unwrap();
    observer.join().unwrap();
    assert!(result.err().unwrap().contains("cancelled"));
    let state = state(&manager, &document);
    assert_eq!(count(&state, "requests", "textDocument/rangeFormatting"), 2);
    assert_eq!(count(&state, "notifications", "$/cancelRequest"), 1);
    assert_eq!(state["rangeSnapshots"][0], state["rangeSnapshots"][1]);
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn range_formatting_waits_for_all_replies_and_preserves_midflight_guards(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in [
        "success",
        "typing",
        "restart",
        "disabled",
        "route",
        "read-only",
        "follow",
        "immutable",
        "ime",
        "shared-ime",
        "writing",
    ] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-range-midflight-{mode}"), "target");
        // GPUI runs blocking background transport work on the test thread; pause
        // the real completed batch before UI admission, as the save-format tests do.
        let pause = crate::documents::save::gate::pause(&range_gate_path(&a));
        let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        let shared = ws.update_in(cx, |ws, window, cx| {
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            let shared = ws.active_editor().unwrap();
            ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx);
            ws.activate_view(source.entity_id(), window, cx);
            shared
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.sel(cx).reselect(vec![Selection::new(1, 2), Selection::new(3, 4)]));
            ws.lsp_format_selection(&LspFormatSelection, window, cx);
        });
        settle_until(cx, |_| pause.arrived() == 1);
        assert_eq!(text(&ws, cx), "😀pri\n", "{mode}: the complete batch stays staged");
        assert_eq!(shared.read_with(cx, |e, cx| e.doc(cx).text()), "😀pri\n");
        assert!(!source.read_with(cx, |e, cx| e.doc(cx).dirty));
        ws.update_in(cx, |ws, window, cx| match mode {
            "typing" => source.update(cx, |e, cx| e.edit(cx, |s| s.insert("later"))),
            "restart" => ws.restart_language_servers(&RestartLanguageServers, window, cx),
            "disabled" => cx.global_mut::<Settings>().lsp_enabled = false,
            "route" => ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx),
            "read-only" => source.update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true)),
            "follow" => source.update(cx, |e, cx| {
                e.update_doc(cx, |d| {
                    d.set_following(true);
                })
            }),
            "immutable" => source.update(cx, |e, _| e.input_enabled = false),
            "ime" => source.update(cx, |e, _| e.marked = std::iter::once(1..2).collect()),
            "shared-ime" => shared.update(cx, |e, _| e.marked = std::iter::once(1..2).collect()),
            _ => {}
        });
        let _write = (mode == "writing").then(|| cx.update(|_, cx| super::super::pipelines::lease(&a, cx)));
        let before =
            source.read_with(cx, |e, cx| (e.text(cx), e.doc(cx).version, e.state.selections.clone(), e.marked.clone()));
        let shared_marks = shared.read_with(cx, |e, _| e.marked.clone());
        pause.release();
        settle_until(cx, |cx| {
            status(&ws, cx).contains(match mode {
                "success" => "applied",
                "writing" => "being written",
                _ => "discarded",
            })
        });
        if mode == "success" {
            assert_eq!(text(&ws, cx), "😀selectedrselected\n");
            let snapshots = captured(&ws, cx, Some(&a))["rangeSnapshots"].clone();
            assert_eq!(snapshots[0], snapshots[1]);
        } else {
            source.read_with(cx, |e, cx| {
                assert_eq!(
                    (e.text(cx), e.doc(cx).version, e.state.selections.clone(), e.marked.clone()),
                    before,
                    "{mode}"
                )
            });
            assert_eq!(shared.read_with(cx, |e, _| e.marked.clone()), shared_marks, "{mode}");
        }
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
        // Exercise actual native composition too: an action must refuse before sending.
        if mode == "success" {
            source.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "中", None, window, cx));
            let before = text(&ws, cx);
            ws.update_in(cx, |ws, window, cx| ws.lsp_format_selection(&LspFormatSelection, window, cx));
            assert!(status(&ws, cx).contains("composition"));
            assert_eq!(text(&ws, cx), before);
            assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/rangeFormatting"), 2);
        }
    }
}

#[gpui::test]
fn range_formatting_refuses_unsupported_and_read_only_documents(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-range-format-refused", "😀target\n");
    rust_options(cx, |o| o["noRangeFormatting"] = true.into());
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.select_range(1..4, cx));
        ws.lsp_format_selection(&LspFormatSelection, window, cx);
    });
    settle_until(cx, |cx| status(&ws, cx).contains("does not support"));
    assert_eq!(text(&ws, cx), "😀pri\n");
    rust_options(cx, |o| o["noRangeFormatting"] = false.into());
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
        ws.lsp_format_selection(&LspFormatSelection, window, cx);
    });
    settle_until(cx, |cx| status(&ws, cx).contains("read only"));
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn formatting_and_cross_file_rename_leave_disk_untouched(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-lsp-ui-rename", "😀pri\n");
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(b.clone(), window, cx);
        ws.open_path(a.clone(), window, cx);
        ws.lsp_format(&LspFormat, window, cx);
    });
    settle_until(cx, |cx| text(&ws, cx) == "😀formatted\n");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "😀pri\n");
    ws.update_in(cx, |ws, window, cx| ws.request_language(Feature::Rename, json!({"newName":"renamed"}), window, cx));
    settle_until(cx, |cx| status(&ws, cx).contains("applied to 2 file"));
    for editor in ws.read_with(cx, |ws, cx| ws.open_editors(cx)) {
        editor.read_with(cx, |e, cx| {
            assert_eq!(e.doc(cx).text(), "😀renamed\n");
            assert!(e.doc(cx).dirty);
        });
    }
    for path in [a, b] {
        assert_eq!(std::fs::read_to_string(path).unwrap(), "😀pri\n");
    }
}

#[gpui::test]
fn stale_second_buffer_rejects_all_prepared_edits_and_readonly_rejects_formatting(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-lsp-ui-atomic", "😀pri\n");
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(b.clone(), window, cx);
        ws.open_path(a.clone(), window, cx);
    });
    let (input, request) = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap());
    let versions = input.documents.iter().map(|d| (lsp::uri(&d.path).unwrap(), (d.buffer, d.version, 1))).collect();
    let value = json!({"changes":{
        lsp::uri(&a).unwrap():[{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},"newText":"changed"}],
        lsp::uri(&b).unwrap():[{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},"newText":"changed"}]
    }});
    let response = Response { versions, ..response(value, a.parent().unwrap()) };
    let prepared = prepare_edits(&request, response).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        let second = ws.open_editors(cx).into_iter().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)).unwrap();
        second.update(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
        assert!(ws.apply_language_edits(prepared, window, cx).unwrap_err().contains("changed"));
        ws.with_active(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
        ws.lsp_format(&LspFormat, window, cx);
    });
    assert_eq!(text(&ws, cx), "😀pri\n", "first file must not change when second is stale");
    settle_until(cx, |cx| status(&ws, cx).contains("read only"));
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn definition_selects_the_exact_unicode_range_in_a_loaded_target(cx: &mut TestAppContext) {
    let (ws, cx, _, b) = open(cx, "orlo-lsp-ui-definition", "😀target\n");
    ws.update_in(cx, |ws, window, cx| ws.lsp_definition(false, window, cx));
    settle_until(cx, |cx| showing(&ws, cx, &b));
    ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        let e = editor.read(cx);
        assert_eq!(e.doc(cx).text(), "😀target\n");
        assert_eq!(e.state.selections[0].range(), 1..7);
        assert_eq!((e.doc(cx).rope).slice(e.state.selections[0].range()).to_string(), "target");
    });
}

#[gpui::test]
fn a_disk_change_after_preparation_rejects_the_entire_cross_file_edit(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-lsp-ui-stale-disk", "😀pri\n");
    let (_, request) = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap());
    let edit = json!({"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},"newText":"changed"});
    let value = json!({"changes":{lsp::uri(&a).unwrap():[edit.clone()],lsp::uri(&b).unwrap():[edit]}});
    let prepared = prepare_edits(&request, response(value, a.parent().unwrap())).unwrap();
    std::fs::write(&b, "external update\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        assert!(ws.apply_language_edits(prepared, window, cx).is_err(), "changed unopened file must reject every edit");
    });
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert_eq!(std::fs::read_to_string(b).unwrap(), "external update\n");
}

#[gpui::test]
fn direct_and_resolved_code_actions_apply_the_chosen_fix(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-code-actions", "😀target\n");
    for (index, expected) in [(0, "😀fixed\n"), (1, "😀resolved\n")] {
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            ws.lsp_code_actions(&LspCodeActions, window, cx);
        });
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.quick_panel.is_some()));
        ws.update_in(cx, |ws, window, cx| {
            let panel = ws.quick_panel.as_mut().unwrap();
            assert_eq!(panel.items.len(), 2, "disabled actions must be hidden");
            panel.selected = index;
            ws.confirm_quick_panel(window, cx);
        });
        settle_until(cx, |cx| text(&ws, cx) == expected);
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
        assert_eq!(text(&ws, cx), "😀pri\n");
    }
}

#[gpui::test]
fn a_completion_response_is_discarded_after_the_caret_moves(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-stale-request", "😀target\n");
    activate(cx);
    // ctrl-space's request is dropped saying nothing
    let before = status(&ws, cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.server_completions(true, window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(0));
    });
    cx.run_until_parked();
    assert_eq!(asked(&ws, cx), 1, "it was asked");
    assert_eq!(status(&ws, cx), before);
    assert_eq!(waiting(&ws, cx), None);
    // LSP: Complete's says so
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
        ws.lsp_completion(&LspCompletion, window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(0));
    });
    settle_until(cx, |cx| status(&ws, cx).contains("discarded"));
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert!(listed(&ws, cx).is_empty(), "{:?}", listed(&ws, cx));
}

#[test]
fn an_older_manager_snapshot_cannot_roll_back_text_or_close_newer_documents() {
    let root = temp("orlo-lsp-old-manager-snapshot").canonicalize().unwrap();
    let old = input(&root, vec![document(&root, "a.rs", 1)]);
    let mut newer = input(&root, old.documents.clone());
    newer.documents[0].version = 1;
    newer.documents[0].rope = Rope::from_str("😀newer\n");
    newer.documents.push(document(&root, "b.rs", 2));
    let mut manager = Manager::default();
    manager.sync(&old);
    manager.sync(&newer);
    assert!(manager.sync(&old).is_empty());
    let state = state(&manager, &old.documents[0]);
    assert_eq!(state["documents"].as_object().unwrap().len(), 2);
    assert_eq!(state["documents"][lsp::uri(&old.documents[0].path).unwrap()]["text"], "😀newer\n");
    assert_eq!(count(&state, "notifications", "textDocument/didChange"), 1);
}

#[gpui::test]
fn two_windows_share_one_server_and_keep_both_documents_synchronized(cx: &mut TestAppContext) {
    let root = temp("orlo-lsp-ui-two-windows").canonicalize().unwrap();
    let a = root.join("a.rs");
    let b = root.join("b.rs");
    for path in [&a, &b] {
        std::fs::write(path, "😀pri\n").unwrap();
    }
    let (ws1, _) = window(cx);
    let (ws2, _) = cx.add_window_view(Workspace::new);
    cx.update(|cx| serve(cx, config(&b)));
    let mut cx1 = VisualTestContext::from_window(cx.windows()[0], cx);
    let mut cx2 = VisualTestContext::from_window(cx.windows()[1], cx);
    ws1.update_in(&mut cx1, |ws, window, cx| {
        ws.open_path(root.clone(), window, cx);
        ws.open_path(a.clone(), window, cx);
    });
    ws2.update_in(&mut cx2, |ws, window, cx| ws.open_path(b.clone(), window, cx));
    let input1 = ws1.read_with(&cx1, |ws, cx| ws.language_input(cx));
    let input2 = ws2.read_with(&cx2, |ws, cx| ws.language_input(cx));
    assert_eq!(input1.documents.len(), 2);
    assert_eq!(input2.documents.len(), 2);
    let manager1 = ws1.read_with(&cx1, |ws, _| ws.language.manager.clone());
    let manager2 = ws2.read_with(&cx2, |ws, _| ws.language.manager.clone());
    manager1.lock().unwrap().sync(&input1);
    manager2.lock().unwrap().sync(&input2);
    let manager = manager1.lock().unwrap();
    assert_eq!(manager.sessions.len(), 1);
    let state = state(&manager, &input1.documents[0]);
    assert_eq!(state["documents"].as_object().unwrap().len(), 2);
    assert_eq!(count(&state, "notifications", "textDocument/didOpen"), 2);
}

#[gpui::test]
fn a_code_action_command_runs_after_its_edit_is_applied_and_synchronized(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-lsp-ui-edit-command", "😀target\n");
    let marker = a.parent().unwrap().join("command-observed.json");
    rust_options(cx, |o| o["commandMarker"] = marker.to_string_lossy().as_ref().into());
    ws.update_in(cx, |ws, window, cx| ws.lsp_code_actions(&LspCodeActions, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.quick_panel.is_some()));
    ws.update_in(cx, |ws, window, cx| ws.confirm_quick_panel(window, cx));
    settle_until(cx, |cx| marker.exists() || status(&ws, cx).starts_with("LSP:"));
    assert!(marker.exists(), "follow-up command failed: {}", status(&ws, cx));
    let observed: Value = serde_json::from_str(&std::fs::read_to_string(marker).unwrap()).unwrap();
    assert_eq!(observed["text"], "😀fixed\n");
    assert_eq!(observed["version"], 2);
    assert_eq!(text(&ws, cx), "😀fixed\n");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
}

#[gpui::test]
fn command_code_actions_with_data_do_not_require_resolve_support(cx: &mut TestAppContext) {
    for (name, boolean_provider, resolve_provider) in [
        ("orlo-lsp-ui-no-resolve-command", false, false),
        ("orlo-lsp-ui-boolean-command", true, false),
        ("orlo-lsp-ui-ready-command", false, true),
    ] {
        let (ws, cx, a, _) = open(cx, name, "😀target\n");
        let marker = a.parent().unwrap().join("command-observed.json");
        rust_options(cx, |options| {
            options["resolveProvider"] = resolve_provider.into();
            options["booleanCodeActions"] = boolean_provider.into();
            options["commandOnlyAction"] = true.into();
            options["commandMarker"] = marker.to_string_lossy().as_ref().into();
        });
        ws.update_in(cx, |ws, window, cx| ws.lsp_code_actions(&LspCodeActions, window, cx));
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.quick_panel.is_some()));
        ws.update_in(cx, |ws, window, cx| ws.confirm_quick_panel(window, cx));
        settle_until(cx, |cx| {
            marker.exists()
                || status(&ws, cx).contains("resolve unsupported")
                || status(&ws, cx).contains("does not support")
                || status(&ws, cx).contains("closed stdout")
        });
        assert!(marker.exists(), "valid command action was blocked: {}", status(&ws, cx));
        let captured = captured(&ws, cx, Some(&a));
        assert_eq!(count(&captured, "requests", "codeAction/resolve") > 0, resolve_provider);
        assert_eq!(text(&ws, cx), if resolve_provider { "😀resolved\n" } else { "😀pri\n" });
    }
}

#[gpui::test]
fn data_bearing_completion_does_not_send_an_unsupported_resolve_request(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-lsp-ui-no-resolve-completion", "😀target\n");
    rust_options(cx, |o| o["resolveProvider"] = false.into());
    activate(cx);
    lsp_complete(&ws, cx);
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
    pick(&ws, cx, "print");
    settle_until(cx, |cx| text(&ws, cx) == "😀print\n");
    let captured = captured(&ws, cx, Some(&a));
    assert!(
        count(&captured, "requests", "completionItem/resolve") == 0,
        "completion must use available text without requesting unsupported resolution"
    );
}

#[test]
fn a_transient_semantic_failure_is_retried_without_a_document_change() {
    let root = temp("orlo-lsp-semantic-retry").canonicalize().unwrap();
    let mut first = input(&root, vec![document(&root, "a.rs", 1)]);
    first.config.get_mut("rust").unwrap().initialization_options["semantic_fail_once"] = true.into();
    let mut manager = Manager::default();
    let initial = manager.sync(&first);
    assert!(initial[0].semantics.is_empty());
    let mut retry = input(&root, first.documents.clone());
    retry.config = first.config.clone();
    let second = manager.sync(&retry);
    assert_eq!(
        second[0].semantics,
        vec![SemanticSpan { range: 1..4, kind: "function".into() }],
        "the unchanged document must recover semantic tokens on the next poll"
    );
    let captured = state(&manager, &first.documents[0]);
    assert_eq!(count(&captured, "requests", "textDocument/semanticTokens/full"), 2);
}

#[gpui::test]
fn choices_from_another_server_epoch_cannot_request_or_apply_edits(cx: &mut TestAppContext) {
    for restart in [true, false] {
        let name = if restart { "orlo-lsp-ui-other-window-restart" } else { "orlo-lsp-ui-preferences-epoch" };
        let (ws, cx, _, _) = open(cx, name, "😀target\n");
        let (other, _) = cx.cx.add_window_view(Workspace::new);
        let mut other_cx = VisualTestContext::from_window(*cx.cx.windows().last().unwrap(), &cx.cx);
        activate(cx);
        lsp_complete(&ws, cx);
        settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
        let Insert::Server(saved_item) = rows(&ws, cx).into_iter().find(|c| c.label == "print").unwrap().insert else {
            panic!("the fixture's print completion must come from its server");
        };
        // what the items were asked with
        let (saved_input, saved_request, manager) = ws.read_with(cx, |ws, cx| {
            let (input, request) = ws.language_request(cx).unwrap();
            (input, request, ws.language.manager.clone())
        });
        if restart {
            other.update_in(&mut other_cx, |ws, window, cx| {
                ws.restart_language_servers(&RestartLanguageServers, window, cx)
            });
        } else {
            rust_options(&mut other_cx, |o| o["positionEncoding"] = "utf-8".into());
        }
        cx.run_until_parked();
        assert!(!ws.read_with(cx, |ws, cx| ws.language_request_current(&saved_request, cx)));
        let error = manager
            .lock()
            .unwrap()
            .request(&saved_input, &saved_request, "completionItem/resolve", json!({"label":"print"}))
            .err()
            .unwrap();
        assert!(error.contains("restarted"), "old server request must be rejected: {error}");
        if restart {
            pick(&ws, cx, "print");
            assert!(status(&ws, cx).contains("discarded"), "{}", status(&ws, cx));
        } else {
            assert!(shown(&ws, cx).is_none(), "settings changes dismiss obsolete choices");
            ws.read_with(cx, |ws, _| assert!(ws.language.completion.is_none()));
            let before = status(&ws, cx);
            // A click queued before dismissal cannot resolve or apply its old item afterward.
            ws.update_in(cx, |ws, window, cx| ws.commit_server_item(saved_item, Vec::new(), window, cx));
            assert_eq!(status(&ws, cx), before);
        }
        assert_eq!(text(&ws, cx), "😀pri\n");
    }
}

#[gpui::test]
fn ctrl_space_adds_the_servers_items_and_typing_asks_the_server_nothing(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-popup", "😀target\n");
    activate(cx);
    // typed, the list offers the buffer's words alone
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(5)));
    cx.simulate_input("prism pr");
    settle_until(cx, |cx| listed(&ws, cx).contains(&"prism".into()));
    assert!(!listed(&ws, cx).contains(&"print".into()));
    // ctrl-space over the showing list asks the server, saying nothing: its items join the
    // words, ranked with them, and the row the user is on stays selected
    select(&ws, cx, "prism");
    let before = status(&ws, cx);
    cx.simulate_keystrokes("ctrl-space");
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
    assert!(listed(&ws, cx).contains(&"pri".into()), "{:?}", listed(&ws, cx));
    assert_eq!(selected(&ws, cx), "prism");
    assert_eq!(status(&ws, cx), before);
    let print = rows(&ws, cx).into_iter().find(|c| c.label == "print");
    let print = print.unwrap();
    assert_eq!(print.kind, Kind::Symbol("function".into()), "the server's kind");
    assert_eq!(print.detail, "fn print()", "the server's detail");
    assert_eq!(asked(&ws, cx), 1, "typing asked the server nothing");
}

#[gpui::test]
fn typing_on_filters_the_servers_items_and_a_commit_takes_in_what_was_typed(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-type-on", "😀target\n");
    activate(cx);
    cx.simulate_keystrokes("ctrl-space");
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
    cx.simulate_input("n");
    assert!(listed(&ws, cx).contains(&"print".into()), "typing filters the server's items with the rest");
    pick(&ws, cx, "print");
    // the server's edit for "pri", resolved, now over "prin"
    settle_until(cx, |cx| text(&ws, cx) == "😀print\nimported\n");
    assert_eq!(selections(&ws, cx), caret(6), "after the item, typed `n` and all");
    assert_eq!(asked(&ws, cx), 1, "no request per keystroke");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, cx), "😀prin\n", "one undo step, then the typing");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, cx), "😀print\nimported\n");
}

/// The active view's selections.
fn selections(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<crate::editing::text::Selection> {
    ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).state.selections.clone())
}

fn caret(at: usize) -> Vec<crate::editing::text::Selection> {
    vec![crate::editing::text::Selection::caret(at)]
}

/// The list opened by ctrl-space (`quiet`) or Goto > Language Server > Complete, the server's
/// `print` for `😀pri` committed from it, and the text once it and its import are in.
fn commit_print(ws: &Entity<Workspace>, cx: &mut VisualTestContext, quiet: bool, done: &str) {
    match quiet {
        true => cx.simulate_keystrokes("ctrl-space"),
        false => lsp_complete(ws, cx),
    }
    settle_until(cx, |cx| listed(ws, cx).contains(&"print".into()));
    pick(ws, cx, "print");
    settle_until(cx, |cx| text(ws, cx) == done);
}

#[gpui::test]
fn a_server_commit_says_nothing_whichever_way_the_list_was_opened(cx: &mut TestAppContext) {
    for quiet in [true, false] {
        let name = format!("orlo-lsp-ui-quiet-commit-{quiet}");
        let (ws, cx, _, _) = open(cx, &name, "😀target\n");
        activate(cx);
        commit_print(&ws, cx, quiet, "😀print\nimported\n");
        assert_eq!(status(&ws, cx), "", "ctrl-space: {quiet}");
    }
}

#[gpui::test]
fn a_server_commit_leaves_the_caret_after_the_item_and_one_undo_step_takes_both_back(cx: &mut TestAppContext) {
    for quiet in [true, false] {
        let name = format!("orlo-lsp-ui-commit-caret-{quiet}");
        let (ws, cx, _, _) = open(cx, &name, "😀target\n");
        activate(cx);
        commit_print(&ws, cx, quiet, "😀print\nimported\n");
        assert_eq!(selections(&ws, cx), caret(6), "after `print`; ctrl-space: {quiet}");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&ws, cx), "😀pri\n");
        assert_eq!(selections(&ws, cx), caret(4), "the caret as it was");
        cx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(text(&ws, cx), "😀print\nimported\n");
        assert_eq!(selections(&ws, cx), caret(6), "redo puts it after the item again");
    }
}

#[gpui::test]
fn an_import_above_the_word_moves_the_caret_down_with_the_word(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-import-first", "😀target\n");
    rust_options(cx, |o| o["importFirst"] = true.into());
    activate(cx);
    commit_print(&ws, cx, true, "imported\n😀print\n");
    // 9 chars of import, the emoji, `print`: the import does not pull the caret to its own end
    assert_eq!(selections(&ws, cx), caret(15));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!((text(&ws, cx).as_str(), selections(&ws, cx)), ("😀pri\n", caret(4)));
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(selections(&ws, cx), caret(15));
}

#[gpui::test]
fn ctrl_space_says_nothing_where_no_server_serves_the_buffer_and_lsp_complete_says_why(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-unserved", "😀target\n");
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, cx, "alpha alps\n");
    let before = status(&ws, cx);
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(listed(&ws, cx), ["alps", "alpha"], "the words, at once");
    // over the showing list it does nothing
    cx.simulate_keystrokes("down ctrl-space");
    assert_eq!(shown(&ws, cx), Some((vec!["alps".into(), "alpha".into()], 1)));
    assert_eq!(status(&ws, cx), before, "an untitled buffer has no server");
    cx.simulate_keystrokes("escape");
    lsp_complete(&ws, cx);
    assert_eq!(status(&ws, cx), "Save the source file before using LSP");
    assert!(listed(&ws, cx).is_empty());
}

#[gpui::test]
fn lsp_complete_says_why_the_list_cannot_open_and_ctrl_space_says_nothing(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-cannot-open", "😀target\n");
    activate(cx);
    let complete = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| {
        lsp_complete(ws, cx);
        status(ws, cx)
    };
    // over a selection
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_selection(1, 4)));
    let before = status(&ws, cx);
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(status(&ws, cx), before, "ctrl-space says nothing");
    assert_eq!(complete(&ws, cx), "Completion unavailable: text is selected");
    // after a word longer than the list completes
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(4);
                s.insert(&"x".repeat(100));
            })
        })
    });
    let before = status(&ws, cx);
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(status(&ws, cx), before, "ctrl-space says nothing");
    assert_eq!(complete(&ws, cx), "Completion unavailable: the word before the caret is over 100 characters");
    assert!(listed(&ws, cx).is_empty());
}

#[gpui::test]
fn ctrl_space_asks_quietly_where_the_server_cannot_start_and_lsp_complete_says_why(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-no-server", "😀target\n");
    activate(cx);
    cx.update(|_, cx| {
        let mut settings = cx.global::<Settings>().clone();
        let rust = settings.language_servers.get_mut("rust").unwrap();
        rust.command = vec!["/nonexistent/orlo-language-server".into()];
        cx.set_global(settings);
    });
    let before = status(&ws, cx);
    // over a showing list: the words stay
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(5)));
    cx.simulate_input("prism pr");
    settle_until(cx, |cx| listed(&ws, cx).contains(&"prism".into()));
    cx.simulate_keystrokes("ctrl-space");
    settle_until(cx, |cx| waiting(&ws, cx) == Some(false));
    assert!(listed(&ws, cx).contains(&"prism".into()));
    assert_eq!(status(&ws, cx), before, "nothing said");
    // with nothing else to show (no word has `zzq`), the list does not wait on
    cx.simulate_keystrokes("escape");
    cx.simulate_input(" zzq");
    cx.simulate_keystrokes("ctrl-space");
    settle_until(cx, |cx| waiting(&ws, cx).is_none());
    assert_eq!(status(&ws, cx), before, "nothing said");
    // LSP: Complete says why
    lsp_complete(&ws, cx);
    settle_until(cx, |cx| status(&ws, cx).starts_with("LSP: Cannot start"));
}

#[gpui::test]
fn empty_semantic_definition_falls_back_to_project_index(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-lsp-empty-definition", "fn target() {}\n");
    replace_text(&ws, cx, "fn caller() { target(); }\n", 16);
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.index.read(cx).progress().is_none()));
    ws.update_in(cx, |ws, window, cx| {
        let (_, request) = ws.language_request(cx).unwrap();
        let response = response(Value::Null, a.parent().unwrap());
        ws.language_response(Feature::Definition, response, request, window, cx);
    });
    settle_until(cx, |cx| showing(&ws, cx, &b));
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        let s = e.read(cx).read_state(cx);
        assert_eq!(s.doc.rope.slice(s.view.selections[s.primary()].range()).to_string(), "target");
    });
}

#[gpui::test]
fn explicit_lsp_references_stay_semantic_when_server_answers(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-reference-precedence", "😀target\n");
    ws.update_in(cx, |ws, window, cx| ws.lsp_references(&LspReferences, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.items.len() == 2)));
    ws.read_with(cx, |ws, _| {
        let p = ws.quick_panel.as_ref().unwrap();
        assert!(p.title.is_none());
        assert!(p.items.iter().all(|i| matches!(i.action, ItemAction::Language(Choice::Location { .. }))));
    });
}

#[gpui::test]
fn empty_explicit_lsp_references_stay_semantic(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-lsp-empty-references", "unrelated\n");
    ws.update_in(cx, |ws, window, cx| {
        let (_, request) = ws.language_request(cx).unwrap();
        let response = response(json!([]), a.parent().unwrap());
        ws.language_response(Feature::References, response, request, window, cx);
        assert!(ws.quick_panel.is_none());
    });
}

#[gpui::test]
fn semantic_definition_to_side_clones_same_file_in_destination(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-lsp-definition-side", "unrelated\n");
    let origin = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let before = origin.read_with(cx, |e, _| e.state.selections.clone());
    ws.update_in(cx, |ws, window, cx| {
        ws.choose_language(
            Choice::Location {
                uri: lsp::uri(&a).unwrap(),
                range: lsp::Span { start: lsp::Position::new(0, 2), end: lsp::Position::new(0, 5) },
                encoding: Encoding::Utf16,
                side: true,
            },
            window,
            cx,
        )
    });
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.views.active, 1);
        let target = ws.active_editor().unwrap();
        assert_ne!(target, origin);
        assert!(target.read(cx).same_document(origin.read(cx)));
        assert_eq!(target.read(cx).state.selections[0].range(), 1..4);
        assert_eq!(origin.read(cx).state.selections, before);
    });
}

fn semantic_file_load(cx: &mut TestAppContext, side: bool, supersede: &str) {
    let (ws, cx, a, b) = open(cx, &format!("orlo-final-file-load-{side}-{supersede}"), "fn target() {}\n");
    activate(cx);
    let origin = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let (panel, active, group) = ws.update_in(cx, |ws, window, cx| {
        let (_, mut request) = ws.language_request(cx).unwrap();
        request.navigation_generation = Some(ws.language.navigation_generation);
        ws.language_response(
            if side { Feature::DefinitionToSide } else { Feature::Definition },
            response(json!({"uri":lsp::uri(&b).unwrap(),"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":9}}}), a.parent().unwrap()),
            request,
            window,
            cx,
        );
        assert!(!ws.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)), "target must still be loading");
        match supersede {
            "palette" => ws.command_palette(&CommandPalette, window, cx),
            "cursor" => origin.update(cx, |e, cx| e.sel(cx).set_cursor(0)),
            "version" => origin.update(cx, |e, cx| e.edit(cx, |s| s.insert("changed"))),
            "epoch" => ws.invalidate_language_requests(cx),
            "language" => ws.language.generation += 1,
            "restore" => ws.restore_gen += 1,
            "disabled" => {
                let mut settings = cx.global::<Settings>().clone();
                settings.lsp_enabled = false;
                cx.set_global(settings);
            }
            "tab" => ws.new_file(&NewFile, window, cx),
            "success" => {}
            _ => unreachable!(),
        }
        (ws.quick_panel.as_ref().map(|p| p.input.entity_id()), ws.active_editor(), ws.views.active)
    });
    cx.run_until_parked();
    if supersede == "success" {
        settle_until(cx, |cx| showing(&ws, cx, &b));
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.active, usize::from(side));
            assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), 3..9);
            assert_eq!(origin.read(cx).doc(cx).path.as_ref(), Some(&a));
        });
    } else {
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.quick_panel.as_ref().map(|p| p.input.entity_id()), panel, "old load replaced newer palette");
            assert_eq!(ws.active_editor(), active, "old load replaced newer active view ({supersede})");
            assert_eq!(ws.views.active, group, "old load forced its destination group");
            assert!(
                !ws.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&b)),
                "superseded target was attached ({supersede})"
            );
        });
    }
}

#[gpui::test]
fn final_semantic_file_load_keeps_same_update_newer_palette(cx: &mut TestAppContext) {
    semantic_file_load(cx, false, "palette");
}

#[gpui::test]
fn final_semantic_side_file_load_keeps_same_update_newer_palette(cx: &mut TestAppContext) {
    semantic_file_load(cx, true, "palette");
}

#[gpui::test]
fn final_semantic_file_load_succeeds(cx: &mut TestAppContext) {
    semantic_file_load(cx, false, "success");
}

#[gpui::test]
fn final_semantic_side_file_load_succeeds(cx: &mut TestAppContext) {
    semantic_file_load(cx, true, "success");
}

#[gpui::test]
fn final_semantic_file_load_rejects_changed_source_and_service(cx: &mut TestAppContext) {
    for changed in ["cursor", "version", "epoch", "language", "restore", "disabled", "tab"] {
        semantic_file_load(cx, false, changed);
    }
}

#[gpui::test]
fn final_semantic_side_file_load_rejects_changed_source(cx: &mut TestAppContext) {
    semantic_file_load(cx, true, "cursor");
}

fn late_definition_keeps_new_palette(cx: &mut TestAppContext, empty: bool) {
    let (ws, cx, a, b) = open(cx, if empty { "orlo-late-fallback" } else { "orlo-late-semantic" }, "fn target() {}\n");
    let entered = a.parent().unwrap().join("entered");
    let release = a.parent().unwrap().join("release");
    cx.update(|_, cx| {
        let mut settings = cx.global::<Settings>().clone();
        let server = settings.language_servers.get_mut("rust").unwrap();
        server.initialization_options = json!({"target":lsp::uri(&b).unwrap(), "definitionEntered":entered, "definitionRelease":release, "definitionEmpty":empty});
        cx.set_global(settings);
    });
    replace_text(&ws, cx, "fn caller() { target(); }\n", 16);
    cx.run_until_parked();
    let panel = ws.update_in(cx, |ws, window, cx| {
        ws.goto_definition(&GotoDefinition, window, cx);
        assert!(ws.quick_panel.is_none(), "must wait for semantic response");
        ws.command_palette(&CommandPalette, window, cx);
        let panel = ws.quick_panel.as_ref().unwrap().input.entity_id();
        std::fs::write(&release, "release").unwrap();
        panel
    });
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, _| {
            ws.status.as_ref().is_some_and(|s| s.0.starts_with("Language result discarded"))
                || !ws.quick_panel.as_ref().is_some_and(|p| p.input.entity_id() == panel)
        })
    });
    ws.read_with(cx, |ws, _| {
        assert!(
            ws.quick_panel.as_ref().is_some_and(|p| p.input.entity_id() == panel),
            "late semantic result closed the user's newer command palette"
        );
    });
}

#[gpui::test]
fn late_empty_definition_does_not_replace_new_command_palette(cx: &mut TestAppContext) {
    late_definition_keeps_new_palette(cx, true);
}

#[gpui::test]
fn late_semantic_definition_does_not_replace_new_command_palette(cx: &mut TestAppContext) {
    late_definition_keeps_new_palette(cx, false);
}

#[gpui::test]
fn goto_reference_exposes_lexical_results_with_a_working_server(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lexical-contract", "fn target() {}\n");
    replace_text(&ws, cx, "fn target() {}\n// target\n", 4);
    cx.simulate_keystrokes("shift-f12");
    settle_until(cx, |cx| {
        cx.executor().advance_clock(std::time::Duration::from_millis(50));
        ws.read_with(cx, |ws, _| {
            ws.quick_panel.as_ref().is_some_and(|p| {
                p.loaded && p.reference_search.is_none() && p.rows_task.is_none() && p.filter_task.is_none()
            })
        })
    });
    ws.read_with(cx, |ws, _| {
        let p = ws.quick_panel.as_ref().unwrap();
        assert!(p.items.iter().any(|i| i.hint == "def"));
        assert!(p.items.iter().all(|i| matches!(i.action, ItemAction::Location(..))));
        assert!(p.symbol_preview.is_some());
        assert_eq!(
            p.title.as_deref(),
            Some("Text matches for target — lexical, not semantic"),
            "the lexical references command must remain accessible while the server is enabled"
        );
    });
}

#[gpui::test]
fn navigation_generation_expires_on_focus_and_new_requests(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-navigation-generation", "fn target() {}\n");
    cx.run_until_parked();
    let (request, _focus) = ws.update_in(cx, |ws, window, cx| {
        let (_, mut request) = ws.language_request(cx).unwrap();
        request.navigation_generation = Some(ws.language.navigation_generation);
        assert!(ws.language_request_current(&request, cx));
        ws.lsp_definition(false, window, cx);
        assert!(!ws.language_request_current(&request, cx));
        request.navigation_generation = Some(ws.language.navigation_generation);
        let other = cx.focus_handle();
        window.focus(&other);
        (request, other)
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(!ws.language_request_current(&request, cx), "blur must expire navigation before editor focus returns")
    });
    ws.update_in(cx, |ws, window, cx| ws.focus_editor(window, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(!ws.language_request_current(&request, cx)));
}

fn palette_command(ws: &Entity<Workspace>, cx: &mut VisualTestContext, label: &str) {
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input(label);
    ws.read_with(cx, |ws, _| {
        let p = ws.quick_panel.as_ref().unwrap();
        assert_eq!(p.items[p.filtered[0]].label, label);
    });
    cx.simulate_keystrokes("enter");
}

#[gpui::test]
fn palette_side_definition_survives_its_own_focus_restore(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-palette-side", "😀target\n");
    let (entered, release) = hold_definitions(cx, &a, &b);
    activate(cx);
    palette_command(&ws, cx, "Goto: Goto Definition to Side");
    settle_until(cx, |_| entered.exists());
    cx.run_until_parked();
    std::fs::write(release, "release").unwrap();
    settle_until(cx, |cx| status(&ws, cx).starts_with("Language result discarded") || showing(&ws, cx, &b));
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).path, Some(b), "{:?}", ws.status);
        assert_eq!(ws.views.groups.len(), 2);
    });
}

#[gpui::test]
fn palette_explicit_references_survive_own_focus_restore(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-palette-references", "😀target\n");
    activate(cx);
    palette_command(&ws, cx, "LSP: Find References");
    settle_until(cx, |cx| {
        status(&ws, cx).starts_with("Language result discarded")
            || ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.items.len() == 2))
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.quick_panel.as_ref().is_some_and(|p| p.items.len() == 2), "{:?}", ws.status);
    });
}

#[gpui::test]
fn palette_hover_survives_own_focus_restore(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-palette-hover", "😀target\n");
    activate(cx);
    palette_command(&ws, cx, "LSP: Show Documentation");
    settle_until(cx, |cx| cx.has_pending_prompt() || status(&ws, cx).starts_with("Language result discarded"));
    assert!(cx.has_pending_prompt(), "{}", status(&ws, cx));
    let (title, detail) = cx.pending_prompt().unwrap();
    assert_eq!(title, "Language documentation");
    assert!(detail.contains("fixture docs"));
}

#[gpui::test]
fn second_f12_accepts_latest_reply(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-second-f12", "😀target\n");
    let (_, release) = hold_definitions(cx, &a, &b);
    activate(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.goto_definition(&GotoDefinition, window, cx);
        ws.goto_definition(&GotoDefinition, window, cx);
        std::fs::write(release, "release").unwrap();
    });
    settle_until(cx, |cx| showing(&ws, cx, &b));
}

#[gpui::test]
fn untagged_edit_request_keeps_existing_freshness_after_panel_focus(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-untagged", "😀target\n");
    activate(cx);
    let (_, request) = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap());
    assert!(request.navigation_generation.is_none());
    cx.simulate_keystrokes("cmd-shift-p escape");
    cx.run_until_parked();
    assert!(ws.read_with(cx, |ws, cx| ws.language_request_current(&request, cx)));
}

#[gpui::test]
fn pending_definition_keeps_newer_completion(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-definition-v-completion", "😀target\n");
    let (_, release) = hold_definitions(cx, &a, &b);
    activate(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.goto_definition(&GotoDefinition, window, cx);
        ws.server_completions(true, window, cx);
        assert_eq!(
            ws.active_editor().unwrap().read(cx).completion_waiting(),
            Some(true),
            "completion must open while definition is pending"
        );
        std::fs::write(release, "release").unwrap();
    });
    settle_until(cx, |cx| {
        listed(&ws, cx).contains(&"print".into())
            || status(&ws, cx).starts_with("No definition named")
            || showing(&ws, cx, &b)
    });
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_editor().unwrap().read(cx).doc(cx).path,
            Some(a),
            "late definition replaced newer completion UI"
        );
    });
    assert!(listed(&ws, cx).contains(&"print".into()));
}

#[gpui::test]
fn navigation_expires_when_switching_windows(cx: &mut TestAppContext) {
    let (ws, _, _, _) = open(cx, "orlo-navigation-window", "fn target() {}\n");
    cx.add_window_view(Workspace::new);
    let mut first = VisualTestContext::from_window(cx.windows()[0], cx);
    let mut second = VisualTestContext::from_window(cx.windows()[1], cx);
    activate(&mut first);
    let request = ws.read_with(&first, |ws, cx| {
        let (_, mut request) = ws.language_request(cx).unwrap();
        request.navigation_generation = Some(ws.language.navigation_generation);
        assert!(ws.language_request_current(&request, cx));
        request
    });
    activate(&mut second);
    ws.read_with(&first, |ws, cx| {
        assert!(!ws.language_request_current(&request, cx), "window deactivation must expire pending navigation")
    });
    activate(&mut first);
    ws.read_with(&first, |ws, cx| assert!(!ws.language_request_current(&request, cx)));
}

fn enable_format_on_save(cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        let settings = cx.global::<Settings>().overlay(json!({"format_on_save":true}).as_object().unwrap()).unwrap();
        cx.set_global(settings);
    });
}

fn save_document(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.save(&Save, window, cx));
}

#[gpui::test]
fn formatting_and_queued_save_reject_unrepresentable_legacy_output(cx: &mut TestAppContext) {
    use crate::editing::encoding::Encoding;
    let (ws, cx, path, _) = open(cx, "f01-format-lossless", "target\n");
    enable_format_on_save(cx);
    let original = std::fs::read(&path).unwrap();
    let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    editor.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Windows1252, false)));
    let formatting = hold_formatting(&path);
    save_document(&ws, cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    save_document(&ws, cx);
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    editor.read_with(cx, |e, cx| {
        assert_eq!(e.doc(cx).text(), "😀formatted\n");
        assert!(e.doc(cx).unsaved());
        assert_eq!(e.doc(cx).encoding, Encoding::Windows1252);
        assert_eq!(e.doc(cx).disk_encoding, Encoding::Utf8);
    });
    editor.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Utf32Be, true)));
    save_document(&ws, cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(crate::editing::text::Loaded::read(&path).unwrap().rope.to_string(), "😀formatted\n");
    assert_eq!(crate::editing::text::Loaded::read(&path).unwrap().encoding, Encoding::Utf32Be);
}

#[gpui::test]
fn format_on_save_is_off_until_enabled_and_formats_the_shared_document(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-opt-in", "target\n");
    save_document(&ws, cx);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
    enable_format_on_save(cx);
    let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    save_document(&ws, cx);
    settle_until(cx, |_| std::fs::read_to_string(&a).unwrap() == "😀formatted\n");
    editor.read_with(cx, |e, cx| {
        assert_eq!(e.doc(cx).rope.to_string(), "😀formatted\n");
        assert!(!e.doc(cx).unsaved());
    });
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "😀pri\n");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.redo())));
    assert_eq!(text(&ws, cx), "😀formatted\n");
    assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/formatting"), 1);
}

// GPUI's deterministic background executor polls blocking LSP requests on the test
// thread. Pause the real fixture response before application to act while it is pending.
pub(super) fn format_gate_path(path: &Path) -> PathBuf {
    path.join("format-response")
}

fn hold_formatting(a: &Path) -> crate::documents::save::gate::Pause {
    crate::documents::save::gate::pause(&format_gate_path(a))
}

#[gpui::test]
fn format_on_save_discards_stale_edits_and_serializes_queued_saves(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-queue", "target\n");
    enable_format_on_save(cx);
    let formatting = hold_formatting(&a);
    let pause = crate::documents::save::gate::pause(&a);
    save_document(&ws, cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    assert!(cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    type_in(&ws, cx, "typed");
    save_document(&ws, cx);
    formatting.release();
    let second = hold_formatting(&a);
    settle_until(cx, |_| second.arrived() == 1);
    assert_eq!(text(&ws, cx), "😀prityped\n", "stale formatter must not overwrite typing");
    assert!(cx.has_pending_prompt());
    assert_eq!(pause.arrived(), 0, "the failed save never captures unformatted source");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
    second.release();
    settle_until(cx, |_| pause.arrived() == 1);
    assert_eq!(text(&ws, cx), "😀formatted\n");
    pause.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀formatted\n");
    let state = captured(&ws, cx, Some(&a));
    assert_eq!(count(&state, "requests", "textDocument/formatting"), 2);
    let changes = state["notifications"].as_array().unwrap();
    assert!(
        changes
            .iter()
            .any(|n| n["method"] == "textDocument/didChange"
                && n["params"]["contentChanges"][0]["text"] == "😀prityped\n")
    );
}

#[gpui::test]
fn format_on_save_keeps_saving_when_the_active_tab_changes(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-format-save-tabs", "other text\n");
    enable_format_on_save(cx);
    let formatting = hold_formatting(&a);
    save_document(&ws, cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    ws.update_in(cx, |ws, window, cx| ws.open_path(b.clone(), window, cx));
    formatting.release();
    settle_until(cx, |_| std::fs::read_to_string(&a).unwrap() == "😀formatted\n");
    assert_eq!(text(&ws, cx), "other text\n");
    assert!(showing(&ws, cx, &b));
}

#[gpui::test]
fn format_on_save_skips_cancelled_and_read_only_responses(cx: &mut TestAppContext) {
    for reason in ["epoch", "disabled", "read-only"] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-format-save-{reason}"), "target\n");
        enable_format_on_save(cx);
        type_in(&ws, cx, "mine");
        let formatting = hold_formatting(&a);
        let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        save_document(&ws, cx);
        settle_until(cx, |_| formatting.arrived() == 1);
        ws.update(cx, |ws, cx| match reason {
            "epoch" => ws.invalidate_language_requests(cx),
            "disabled" => cx.global_mut::<Settings>().lsp_enabled = false,
            _ => editor.update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true)),
        });
        formatting.release();
        settle_until(cx, |cx| !editor.read_with(cx, |e, cx| e.document.read(cx).saving()));
        assert_eq!(text(&ws, cx), "😀primine\n", "{reason}");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
        assert!(status(&ws, cx).contains("formatting failed"));
        assert!(editor.read_with(cx, |e, cx| e.doc(cx).unsaved()));
        assert!(cx.has_pending_prompt(), "{reason}: failed save reports the explicit bypass");
    }
}

#[gpui::test]
fn format_on_save_failures_retain_dirty_text_and_cleanup_follows_success(cx: &mut TestAppContext) {
    for (name, options) in [
        ("unsupported", json!({"noFormatting":true})),
        ("error", json!({"formatError":true})),
        (
            "malformed",
            json!({"formatResult":[{"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":2}},"newText":"split emoji"}]}),
        ),
        (
            "overlap",
            json!({"formatResult":[{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},"newText":"one"},{"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":5}},"newText":"two"}]}),
        ),
        ("empty", json!({"formatResult":null})),
        ("cleanup", json!({"formattedText":"formatted  \n\n"})),
        ("missing", json!({})),
    ] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-format-save-{name}"), "target\n");
        enable_format_on_save(cx);
        rust_options(cx, |o| *o = options);
        cx.update(|_, cx| {
            let settings = cx.global_mut::<Settings>();
            settings.trim_trailing_white_space_on_save = settings::Trim::All;
            settings.remove_trailing_blank_lines_on_save = true;
            if name == "missing" {
                settings.language_servers.get_mut("rust").unwrap().command = vec!["/orlo-missing-server".into()];
            }
        });
        type_in(&ws, cx, "typed  ");
        save_document(&ws, cx);
        settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
        let success = matches!(name, "empty" | "cleanup");
        let expected = match name {
            "cleanup" => "😀formatted\n",
            "empty" => "😀prityped\n",
            _ => "😀prityped  \n",
        };
        assert_eq!(text(&ws, cx), expected, "{name}");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), if success { expected } else { "😀pri\n" }, "{name}");
        assert_eq!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()), !success);
        if !success {
            assert!(cx.has_pending_prompt(), "{name}: report a failed save");
            ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
            assert_eq!(text(&ws, cx), "😀pri\n", "{name}: no formatter/cleanup undo entry");
        }
    }
}

#[gpui::test]
fn format_on_save_close_waits_for_preparation_and_the_disk_write(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-close", "target\n");
    enable_format_on_save(cx);
    type_in(&ws, cx, "typed");
    let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let formatting = hold_formatting(&a);
    let write = crate::documents::save::gate::pause(&a);
    ws.update_in(cx, |ws, window, cx| ws.request_close(editor.clone(), After::Close, window, cx));
    cx.simulate_prompt_answer("Save");
    settle_until(cx, |_| formatting.arrived() == 1);
    assert!(ws.read_with(cx, |ws, _| ws.views.find(&editor).is_some()));
    assert_eq!(write.arrived(), 0);
    formatting.release();
    settle_until(cx, |_| write.arrived() == 1);
    assert!(ws.read_with(cx, |ws, _| ws.views.find(&editor).is_some()));
    write.release();
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.views.find(&editor).is_none()));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀formatted\n");
}

#[gpui::test]
fn format_on_save_quit_waits_for_preparation_queued_saves_and_disk(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-quit", "target\n");
    enable_format_on_save(cx);
    let formatting = hold_formatting(&a);
    let write = crate::documents::save::gate::pause(&a);
    save_document(&ws, cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    save_document(&ws, cx);
    let session = session::path();
    let _ = std::fs::remove_file(&session);
    cx.cx.update(quit);
    cx.run_until_parked();
    assert!(!session.exists());
    assert_eq!(status(&ws, cx), "Saving…");
    formatting.release();
    settle_until(cx, |_| write.arrived() == 1);
    assert!(!session.exists());
    write.release();
    settle_until(cx, |_| session.exists());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀formatted\n");
    assert!(session::load(&session).windows[0].groups[0].views[0].unsaved.is_none());
    assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/formatting"), 2);
}

#[gpui::test]
fn format_on_save_save_as_uses_destination_settings_and_queued_save_uses_new_path(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-as", "target\n");
    let copy = a.with_file_name("copy.rs");
    cx.update(|_, cx| {
        let settings = cx
            .global::<Settings>()
            .overlay(
                json!({"overrides":[
                    {"path":"**/copy.rs", "settings":{"format_on_save":true}}
                ]})
                .as_object()
                .unwrap(),
            )
            .unwrap();
        cx.set_global(settings);
    });
    let formatting = hold_formatting(&a);
    ws.update_in(cx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
    cx.simulate_new_path_selection(|_| Some(copy.clone()));
    settle_until(cx, |_| formatting.arrived() == 1);
    save_document(&ws, cx);
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "😀formatted\n");
    assert!(showing(&ws, cx, &copy));
}

#[gpui::test]
fn format_on_save_first_save_as_requires_bypass_and_read_only_copy_does_not_format(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-copy", "target\n");
    enable_format_on_save(cx);
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true)));
    save_document(&ws, cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Ok");
    let copy = a.with_file_name("readonly-copy.rs");
    ws.update_in(cx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
    cx.simulate_new_path_selection(|_| Some(copy.clone()));
    settle_until(cx, |_| copy.exists());
    assert_eq!(std::fs::read_to_string(copy).unwrap(), "😀pri\n");
    ws.update_in(cx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, cx, "first save\n");
    let first = a.with_file_name("first.rs");
    save_document(&ws, cx);
    cx.simulate_new_path_selection(|_| Some(first.clone()));
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert!(!first.exists(), "a missing formatter route cannot silently bypass opt-in");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Ok");
    cx.dispatch_action(SaveWithoutFormatting);
    cx.simulate_new_path_selection(|_| Some(first.clone()));
    settle_until(cx, |_| first.exists());
    assert_eq!(std::fs::read_to_string(first).unwrap(), "first save\n");
}

#[gpui::test]
fn format_on_save_save_all_formats_each_shared_document_once(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-all", "target\n");
    enable_format_on_save(cx);
    type_in(&ws, cx, "typed");
    ws.update_in(cx, |ws, window, cx| {
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.save_all(&SaveAll, window, cx);
    });
    settle_until(cx, |_| std::fs::read_to_string(&a).unwrap() == "😀formatted\n");
    assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/formatting"), 1);
}

#[gpui::test]
fn format_on_save_reports_disk_errors_after_formatting(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-write-error", "target\n");
    enable_format_on_save(cx);
    let formatting = hold_formatting(&a);
    save_document(&ws, cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    std::fs::remove_file(&a).unwrap();
    std::fs::create_dir(&a).unwrap();
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert!(cx.has_pending_prompt());
    assert_eq!(text(&ws, cx), "😀formatted\n");
    assert!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
}

#[gpui::test]
fn format_on_save_stale_response_retains_dirty_shared_text_without_writing(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-format-save-latest", "target\n");
    enable_format_on_save(cx);
    let formatting = hold_formatting(&a);
    save_document(&ws, cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    type_in(&ws, cx, "typed");
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(text(&ws, cx), "😀prityped\n");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "😀pri\n");
    assert!(ws.read_with(cx, |ws, cx| ws.open_editors(cx).iter().all(|e| e.read(cx).doc(cx).unsaved())));
    assert!(cx.has_pending_prompt());
}

#[gpui::test]
fn format_on_save_default_deadline_cancels_transport_and_keeps_dirty_source(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-default-deadline", "target\n");
    enable_format_on_save(cx);
    rust_options(cx, |o| o["holdFormatting"] = true.into());
    type_in(&ws, cx, "mine");
    let started = std::time::Instant::now();
    save_document(&ws, cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert!(started.elapsed() < std::time::Duration::from_secs(7), "save formatter uses its own five-second deadline");
    assert!(started.elapsed() >= std::time::Duration::from_millis(4800));
    assert_eq!(text(&ws, cx), "😀primine\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    assert!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
    assert!(cx.has_pending_prompt());
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "notifications", "$/cancelRequest"), 1);
}

#[gpui::test]
fn format_on_save_bypass_palette_uses_cleanup_and_protected_save(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-bypass-palette", "target\n");
    enable_format_on_save(cx);
    rust_options(cx, |o| o["formatError"] = true.into());
    cx.update(|_, cx| cx.global_mut::<Settings>().trim_trailing_white_space_on_save = settings::Trim::All);
    type_in(&ws, cx, "mine  ");
    save_document(&ws, cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    cx.simulate_prompt_answer("Ok");
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.command_palette(&CommandPalette, window, cx));
    ws.update_in(cx, |ws, window, cx| {
        let panel = ws.quick_panel.as_mut().unwrap();
        panel.selected =
            panel.items.iter().position(|i| i.label == "File: Save Without Formatting").expect("reachable bypass");
        ws.confirm_quick_panel(window, cx);
    });
    settle_until(cx, |_| std::fs::read_to_string(&path).unwrap() == "😀primine\n");
    assert!(!ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 1);
}

#[gpui::test]
fn file_autosave_formatter_error_retains_dirty_source_and_suppresses_same_edit(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-auto-format-error", "target\n");
    enable_format_on_save(cx);
    rust_options(cx, |o| o["formatError"] = true.into());
    cx.update(|_, cx| {
        let s = cx.global_mut::<Settings>();
        s.file_autosave = true;
        s.trim_trailing_white_space_on_save = settings::Trim::All;
    });
    type_in(&ws, cx, "mine  ");
    auto_format_tick(cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    assert_eq!(text(&ws, cx), "😀primine  \n");
    assert!(status(&ws, cx).contains("Automatic save paused"));
    assert!(!cx.has_pending_prompt(), "autosave errors stay nonmodal");
    for _ in 0..4 {
        auto_format_tick(cx);
    }
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 1);
}

#[gpui::test]
fn format_on_save_bypass_stays_distinct_between_queued_normal_saves(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-bypass-queue", "target\n");
    enable_format_on_save(cx);
    let first = crate::documents::save::gate::pause(&path);
    save_document(&ws, cx);
    settle_until(cx, |_| first.arrived() == 1);
    type_in(&ws, cx, "mine");
    save_document(&ws, cx);
    cx.dispatch_action(SaveWithoutFormatting);
    save_document(&ws, cx);
    first.release();
    let second = crate::documents::save::gate::pause(&path);
    settle_until(cx, |_| second.arrived() == 1);
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 2);
    type_in(&ws, cx, "bypass");
    let bypass_text = text(&ws, cx);
    second.release();
    let third = crate::documents::save::gate::pause(&path);
    settle_until(cx, |_| third.arrived() == 1);
    assert_eq!(text(&ws, cx), bypass_text, "bypass captures its own turn without a formatter");
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 2);
    third.release();
    let fourth = crate::documents::save::gate::pause(&path);
    settle_until(cx, |_| fourth.arrived() == 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bypass_text);
    assert_eq!(text(&ws, cx), "😀formatted\n");
    fourth.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀formatted\n");
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 3);
}

#[gpui::test]
fn format_on_save_guards_and_selection_independence(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in [
        "selection",
        "route",
        "immutable",
        "follow",
        "ime",
        "shared-ime",
        "same-version-ime",
        "same-version-shared-ime",
        "writer",
        "hardlink-writer",
    ] {
        let (ws, cx, path, _) = open(cx, &format!("l04-format-guard-{mode}"), "target\n");
        enable_format_on_save(cx);
        type_in(&ws, cx, "mine");
        let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        let formatting = hold_formatting(&path);
        save_document(&ws, cx);
        settle_until(cx, |_| formatting.arrived() == 1);
        let mut lease = None;
        ws.update_in(cx, |ws, window, cx| match mode {
            "selection" => {
                editor.update(cx, |e, cx| e.sel(cx).reselect(vec![Selection::caret(0), Selection::new(1, 3)]));
            }
            "route" => ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx),
            "immutable" => {
                editor.update(cx, |e, _| e.input_enabled = false);
            }
            "follow" => {
                editor.update(cx, |e, cx| e.update_doc(cx, |d| d.set_following(true)));
            }
            "ime" | "shared-ime" | "same-version-ime" | "same-version-shared-ime" => {
                if mode.ends_with("shared-ime") {
                    ws.new_view_into_file(&NewViewIntoFile, window, cx);
                }
                let composing = ws.active_editor().unwrap();
                if mode.starts_with("same-version") {
                    composing.update(cx, |e, _| e.marked = std::iter::once(1..2).collect());
                } else {
                    composing.update(cx, |e, cx| e.replace_and_mark_text_in_range(None, "界", None, window, cx));
                }
            }
            "hardlink-writer" => {
                let alias = path.with_extension("alias");
                std::fs::hard_link(&path, &alias).unwrap();
                lease = Some(pipelines::lease_file(&alias, pipelines::file_id(&alias), cx));
            }
            _ => lease = Some(pipelines::lease(&path, cx)),
        });
        let before = text(&ws, cx);
        formatting.release();
        settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
        if mode == "selection" {
            assert_eq!(text(&ws, cx), "😀formatted\n", "document formatting has no range-selection pin");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀formatted\n");
        } else {
            assert_eq!(text(&ws, cx), before, "{mode}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n", "{mode}");
            assert!(editor.read_with(cx, |e, cx| e.doc(cx).unsaved()));
            assert!(cx.has_pending_prompt(), "{mode}");
        }
        drop(lease);
    }
}

#[gpui::test]
fn format_on_save_bypass_is_installed_and_dispatches_native_menu_action(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-native-bypass", "target\n");
    enable_format_on_save(cx);
    rust_options(cx, |o| o["noFormatting"] = true.into());
    activate(cx);
    type_in(&ws, cx, "mine");
    let before = ws.read_with(cx, |ws, _| ws.menus_built);
    let action = ws.update(cx, |ws, cx| {
        assert!(cx.build_action("workspace::SaveWithoutFormatting", None).is_ok());
        assert!(ws.set_menus_now(cx), "native installation requires the actual active window");
        ws.menus(cx)
            .into_iter()
            .find(|m| m.name == "File")
            .unwrap()
            .items
            .into_iter()
            .find_map(|i| match i {
                gpui::MenuItem::Action { name, action, .. } if name == "Save Without Formatting" => Some(action),
                _ => None,
            })
            .expect("native File command")
    });
    assert!(ws.read_with(cx, |ws, _| ws.menus_built) > before);
    assert!(action.as_any().is::<SaveWithoutFormatting>());
    cx.update(|window, cx| window.dispatch_action(action, cx));
    settle_until(cx, |_| std::fs::read_to_string(&path).unwrap() == "😀primine\n");
    assert!(!ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 0);
}

#[gpui::test]
fn format_on_save_configured_deadline_also_bounds_service_lock_wait(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-lock-deadline", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| cx.global_mut::<Settings>().format_on_save_timeout_ms = 50);
    type_in(&ws, cx, "mine");
    let manager = ws.read_with(cx, |ws, _| ws.language.manager.clone());
    let lock = manager.lock().unwrap();
    let started = std::time::Instant::now();
    save_document(&ws, cx);
    while started.elapsed() < std::time::Duration::from_secs(1) {
        cx.executor().advance_clock(std::time::Duration::from_millis(25));
        cx.run_until_parked();
        if !cx.update(|_, cx| crate::documents::Documents::saving(cx)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        !cx.update(|_, cx| crate::documents::Documents::saving(cx)),
        "service contention cannot extend the formatter deadline"
    );
    assert!(lock.sessions.is_empty(), "a waiting formatter never starts a server");
    drop(lock);
    assert_eq!(text(&ws, cx), "😀primine\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    assert!(cx.has_pending_prompt());
}

#[gpui::test]
fn format_on_save_transport_cancellation_releases_request_without_poisoning_manual_format(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-cancel-wire", "target\n");
    let entered = path.with_extension("format-entered");
    rust_options(cx, |o| {
        o["holdFormatting"] = true.into();
        o["formatEntered"] = json!(entered);
    });
    let (input, request) = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap());
    let cancel = Arc::new(AtomicBool::new(false));
    let token = cancel.clone();
    let observer = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !entered.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(entered.exists(), "the real formatter request entered the fixture");
        token.store(true, Ordering::Relaxed);
        std::time::Instant::now()
    });
    let mut manager = Manager::default();
    let result = manager.request_limited(
        &input,
        &request,
        "textDocument/formatting",
        json!({}),
        Some((&cancel, Some(std::time::Instant::now() + std::time::Duration::from_secs(5)))),
    );
    let canceled = observer.join().unwrap();
    assert!(canceled.elapsed() < std::time::Duration::from_secs(1));
    assert!(result.err().unwrap().contains("cancelled"));
    assert_eq!(count(&state(&manager, &request.document), "notifications", "$/cancelRequest"), 1);
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    rust_options(cx, |o| {
        o["holdFormatting"] = false.into();
    });
    ws.update_in(cx, |ws, window, cx| ws.lsp_format(&LspFormat, window, cx));
    settle_until(cx, |cx| text(&ws, cx) == "😀formatted\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n", "manual formatting never saves");
}

#[gpui::test]
fn format_on_save_failed_close_retains_tab_and_completes_idle_waiter(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-failed-close", "target\n");
    enable_format_on_save(cx);
    type_in(&ws, cx, "mine");
    let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let document = editor.read_with(cx, |e, _| e.document.clone());
    let formatting = hold_formatting(&path);
    ws.update_in(cx, |ws, window, cx| ws.request_close(editor.clone(), After::Close, window, cx));
    cx.simulate_prompt_answer("Save");
    settle_until(cx, |_| formatting.arrived() == 1);
    let idle = std::rc::Rc::new(std::cell::Cell::new(false));
    let finished = idle.clone();
    cx.update(|_, cx| crate::documents::OpenDocument::when_saved(&document, move |_| finished.set(true), cx));
    assert!(!idle.get());
    type_in(&ws, cx, "later");
    formatting.release();
    settle_until(cx, |_| idle.get());
    assert!(ws.read_with(cx, |ws, _| ws.views.find(&editor).is_some()));
    assert!(editor.read_with(cx, |e, cx| e.doc(cx).unsaved()));
    assert_eq!(text(&ws, cx), "😀priminelater\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    assert!(cx.has_pending_prompt());
}

#[gpui::test]
fn format_on_save_multiple_edits_are_one_shared_undo_operation(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-format-atomic", "target\n");
    enable_format_on_save(cx);
    rust_options(cx, |o| {
        o["formatResult"] = json!([
            {"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":3}},"newText":"left"},
            {"range":{"start":{"line":0,"character":4},"end":{"line":0,"character":5}},"newText":"right"}
        ])
    });
    let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    save_document(&ws, cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(text(&ws, cx), "😀leftrright\n");
    assert_eq!(source.read_with(cx, |e, cx| e.text(cx)), "😀leftrright\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀leftrright\n");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert_eq!(source.read_with(cx, |e, cx| e.text(cx)), "😀pri\n");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.redo())));
    assert_eq!(text(&ws, cx), "😀leftrright\n");
}

#[gpui::test]
fn file_autosave_formatter_timeout_keeps_dirty_text_and_pauses_retry(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-auto-format-timeout", "target\n");
    enable_format_on_save(cx);
    rust_options(cx, |o| o["holdFormatting"] = true.into());
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    cx.update(|_, cx| {
        let settings = cx.global_mut::<Settings>();
        settings.file_autosave = true;
        settings.format_on_save_timeout_ms = 50;
    });
    type_in(&ws, cx, "mine");
    auto_format_tick(cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(text(&ws, cx), "😀primine\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    assert!(status(&ws, cx).contains("Automatic save paused"));
    assert!(!cx.has_pending_prompt());
    for _ in 0..4 {
        auto_format_tick(cx);
    }
    let state = captured(&ws, cx, Some(&path));
    assert_eq!(count(&state, "requests", "textDocument/formatting"), 1);
    assert_eq!(count(&state, "notifications", "$/cancelRequest"), 1);
}

#[gpui::test]
fn format_on_save_bypass_preserves_preview_and_save_copy_remains_independent(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, path, _) = open(cx, "l04-format-bypass-composition-copy", "target\n");
    enable_format_on_save(cx);
    rust_options(cx, |o| o["formatError"] = true.into());
    cx.update(|_, cx| cx.global_mut::<Settings>().ensure_newline_at_eof_on_save = true);
    replace_text(&ws, cx, "source", 6);
    let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    editor.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "界", None, window, cx));
    let preview = editor.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).save_point()));
    let copy = path.with_file_name("copy.rs");
    cx.dispatch_action(SaveCopyAs);
    cx.simulate_new_path_selection(|_| Some(copy.clone()));
    settle_until(cx, |_| copy.exists());
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "source界", "copy does no cleanup or source save");
    assert_eq!(preview, editor.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).save_point())));
    assert!(editor.read_with(cx, |e, cx| e.doc(cx).unsaved()));
    // A no-op cleanup preserves the preview exactly, just as ordinary Save does.
    cx.update(|_, cx| cx.global_mut::<Settings>().ensure_newline_at_eof_on_save = false);
    cx.dispatch_action(SaveWithoutFormatting);
    settle_until(cx, |_| std::fs::read_to_string(&path).unwrap() == "source界");
    assert_eq!(preview.0, editor.read_with(cx, |e, cx| e.text(cx)));
    assert_eq!(preview.1, editor.read_with(cx, |e, _| e.marked.clone()));
    assert!(!editor.read_with(cx, |e, cx| e.doc(cx).unsaved()));
    assert!(
        ws.read_with(cx, |ws, _| ws.language.manager.lock().unwrap().sessions.is_empty()),
        "copy and bypass never request a formatter"
    );
}

#[gpui::test]
fn format_on_save_rechecks_identity_version_and_composition_at_capture(cx: &mut TestAppContext) {
    for mode in ["typing", "route", "ime"] {
        let (ws, cx, path, _) = open(cx, &format!("l04-format-capture-{mode}"), "target\n");
        enable_format_on_save(cx);
        let capture = crate::documents::save::gate::pause(&path.join("format-capture"));
        save_document(&ws, cx);
        settle_until(cx, |_| capture.arrived() == 1);
        assert_eq!(text(&ws, cx), "😀formatted\n", "the formatter and cleanup have finished");
        ws.update_in(cx, |ws, window, cx| match mode {
            "typing" => {
                ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("late")));
            }
            "route" => ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx),
            _ => {
                ws.with_active(cx, |e, _| e.marked = std::iter::once(1..2).collect());
            }
        });
        let before = text(&ws, cx);
        capture.release();
        settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
        assert_eq!(text(&ws, cx), before, "{mode}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "😀pri\n",
            "{mode}: capture must validate after preparation yields"
        );
        assert!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
        assert!(cx.has_pending_prompt());
    }
}

#[gpui::test]
fn file_autosave_rejected_formatted_capture_pauses_its_own_edit(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "l04-auto-format-capture-pause", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = true);
    type_in(&ws, cx, "mine");
    let capture = crate::documents::save::gate::pause(&path.join("format-capture"));
    auto_format_tick(cx);
    settle_until(cx, |_| capture.arrived() == 1);
    assert_eq!(text(&ws, cx), "😀formatted\n");
    let lease = cx.update(|_, cx| pipelines::lease(&path, cx));
    capture.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    assert!(status(&ws, cx).contains("Automatic save paused"));
    drop(lease);
    for _ in 0..4 {
        auto_format_tick(cx);
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "😀pri\n",
        "formatter's own edit must not recursively retry a rejected capture"
    );
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 1);
    assert!(!cx.has_pending_prompt());
    type_in(&ws, cx, "later");
    auto_format_tick(cx);
    settle_until(cx, |_| std::fs::read_to_string(&path).unwrap() == "😀formatted\n");
    assert_eq!(
        count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"),
        2,
        "a later user edit still retries after the normal idle delay"
    );
}

/// Add a runtime language pointing at the ordinary fixture, without a Preferences server entry.
fn runtime_manifest(id: &str, target: &Path, options: Value) -> PathBuf {
    let folder = session::support_dir().join("Languages").join(id);
    std::fs::create_dir_all(&folder).unwrap();
    let mut config = config(target).remove("rust").unwrap();
    for (key, value) in options.as_object().into_iter().flatten() {
        config.initialization_options[key] = value.clone();
    }
    let path = folder.join("language.json");
    std::fs::write(
        &path,
        json!({"schema":1, "id":id, "name":"Fixture Runtime",
        "scope":"source.fixture", "files":{"extensions":["orlofixture"]},
        "lsp":{"language_id":"fixture-protocol", "server":"fixture-family",
        "command":config.command, "initialization_options":config.initialization_options,
        "root_markers":["*.fixture-project"]}})
        .to_string(),
    )
    .unwrap();
    path
}

#[gpui::test]
fn u02_template_save_as_redetects_syntax_and_routes_runtime_and_bundled_files(cx: &mut TestAppContext) {
    let root = temp("orlo-template-language-route").canonicalize().unwrap();
    let target = root.join("target.rs");
    std::fs::write(&target, "target\n").unwrap();
    let (ws, cx) = window(cx);
    runtime_manifest("template-fixture", &target, json!({}));
    let snippets = crate::editing::snippet::dir();
    std::fs::create_dir_all(&snippets).unwrap();
    std::fs::write(
        snippets.join("route.sublime-snippet"),
        "<snippet><content>😀pri$0</content><description>Route template</description></snippet>",
    )
    .unwrap();
    cx.update(|_, cx| serve(cx, config(&target)));
    ws.update_in(cx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    for (name, choice, protocol, syntax, extension) in [
        ("Fixture Runtime", "template-fixture", "fixture-protocol", "Plain Text", "orlofixture"),
        // bundled languages are registry languages too, so the choice is the id
        ("Rust", "rust", "rust", "Rust", "rs"),
    ] {
        ws.update_in(cx, |ws, w, cx| ws.new_from_template(&NewFromTemplate, w, cx));
        cx.simulate_input("Route template");
        cx.simulate_keystrokes("enter");
        cx.simulate_input(name);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            let e = editor.read(cx);
            assert_eq!(e.language_choice(), Some(choice));
            assert_eq!(e.highlighter.syntax_name(), syntax);
            assert!(e.doc(cx).path.is_none() && e.doc(cx).dirty);
        });
        // Preserve ordinary Save As: naming the document re-detects its syntax.
        let saved = root.join(format!("{choice}.txt"));
        cx.dispatch_action(Save);
        cx.simulate_new_path_selection(|_| Some(saved.clone()));
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "😀pri");
        ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).language_choice(), None);
            assert_eq!(editor.read(cx).highlighter.syntax_name(), "Plain Text");
            assert!(ws.language_request(cx).is_err(), "an unrelated extension follows normal file detection");
        });
        let saved = root.join(format!("{choice}.{extension}"));
        cx.dispatch_action(SaveAs);
        cx.simulate_new_path_selection(|_| Some(saved.clone()));
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "😀pri");
        ws.read_with(cx, |ws, cx| {
            let (_, request) = ws.language_request(cx).expect("the saved filename must use its ordinary LSP route");
            assert_eq!(request.document.language().unwrap().id, protocol);
        });
        activate(cx);
        ws.update_in(cx, |ws, w, cx| ws.lsp_completion(&LspCompletion, w, cx));
        settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
        let state = captured(&ws, cx, Some(&saved));
        assert_eq!(state["documents"][lsp::uri(&saved).unwrap()]["languageId"], protocol);
        assert!(count(&state, "requests", "textDocument/completion") > 0);
    }
}

#[gpui::test]
fn runtime_language_commands_definition_and_format_on_save(cx: &mut TestAppContext) {
    let root = temp("orlo-runtime-language").canonicalize().unwrap();
    let nested = root.join("src");
    std::fs::create_dir_all(&nested).unwrap();
    let source = nested.join("a.orlofixture");
    let target = nested.join("b.orlofixture");
    std::fs::write(&source, "😀pri\n").unwrap();
    std::fs::write(&target, "😀target\n").unwrap();
    std::fs::write(root.join("app.fixture-project"), "").unwrap();
    let (ws, cx) = window(cx);
    runtime_manifest("fixture", &target, json!({}));
    cx.update(|_, cx| serve(cx, Default::default()));
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.open_path(source.clone(), window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
        assert!(ws.language_request(cx).is_ok(), "the runtime extension must be routable");
    });
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.lsp_completion(&LspCompletion, window, cx));
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
    let state = captured(&ws, cx, Some(&source));
    assert_eq!(state["documents"][lsp::uri(&source).unwrap()]["languageId"], "fixture-protocol");
    assert_eq!(state["documents"][lsp::uri(&source).unwrap()]["text"], "😀pri\n");
    assert_eq!(
        state["requests"].as_array().unwrap().iter().find(|r| r["method"] == "initialize").unwrap()["params"]
            ["rootUri"],
        lsp::uri(&root).unwrap()
    );
    cx.update(|_, cx| {
        let settings = cx
            .global::<Settings>()
            .overlay(
                json!({"overrides":[{
                    "language":"Fixture Runtime", "scope":"source.fixture",
                    "settings":{"format_on_save":true, "tab_size":3}
                }]})
                .as_object()
                .unwrap(),
            )
            .unwrap();
        cx.set_global(settings);
    });
    ws.read_with(cx, |ws, cx| {
        let settings = ws.editor_settings(ws.active_editor().unwrap().read(cx), cx);
        assert!(settings.format_on_save, "manifest name/scope must match file preferences");
        assert_eq!(settings.tab_size, 3);
    });
    save_document(&ws, cx);
    settle_until(cx, |_| std::fs::read_to_string(&source).unwrap() == "😀formatted\n");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| ws.goto_definition(&GotoDefinition, window, cx));
    settle_until(cx, |cx| showing(&ws, cx, &target) || status(&ws, cx).starts_with("Language result discarded"));
    assert!(showing(&ws, cx, &target), "{}", status(&ws, cx));
    assert_eq!(count(&captured(&ws, cx, Some(&source)), "requests", "textDocument/definition"), 1);
}

#[gpui::test]
fn runtime_glob_and_shebang_fixture_support_unicode_diagnostics_completion_and_definition(cx: &mut TestAppContext) {
    let root = temp("orlo-runtime-pattern-assistance").canonicalize().unwrap();
    let nested = root.join("views");
    std::fs::create_dir_all(&nested).unwrap();
    let target = nested.join("target.fixture-template");
    let glob = nested.join("中.fixture-template");
    let shebang = nested.join("runner");
    std::fs::write(&target, "😀target\n").unwrap();
    std::fs::write(&glob, "😀pri\n").unwrap();
    std::fs::write(&shebang, "#!/usr/bin/env fixture-runner 😀\n😀pri\n").unwrap();
    std::fs::write(root.join("app.fixture-project"), "").unwrap();
    let (ws, cx) = window(cx);
    let manifest = runtime_manifest("noncompiled-42", &target, json!({}));
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    value["files"] = json!({"globs":["**/views/*.fixture-template"],"first_line":"^#!.*\\bfixture-runner\\b"});
    std::fs::write(&manifest, value.to_string()).unwrap();
    cx.update(|_, cx| serve(cx, Default::default()));
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    assert!(language::Language::from_registry_id("noncompiled-42").is_none());
    for (source, cursor, line, diagnostic_range) in [(&glob, 4, 0, 1..4), (&shebang, 36, 1, 0..31)] {
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(source.clone(), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(cursor));
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).highlighter.syntax_name(), "Plain Text");
            let (input, request) = ws.language_request(cx).unwrap();
            assert_eq!(
                input.registry.detect(Some(source), &request.document.rope.to_string()).unwrap().id(),
                "noncompiled-42"
            );
            assert_eq!(request.document.language().unwrap().id, "fixture-protocol");
        });
        activate(cx);
        ws.update_in(cx, |ws, window, cx| ws.lsp_completion(&LspCompletion, window, cx));
        settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
        ws.update(cx, |ws, cx| ws.poll_language(cx));
        settle_until(cx, |cx| ws.read_with(cx, |ws, cx| !ws.active_editor().unwrap().read(cx).diagnostics.is_empty()));
        ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).diagnostics[0].range, diagnostic_range);
        });
        let state = captured(&ws, cx, Some(source));
        let completion = state["requests"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|r| r["method"] == "textDocument/completion")
            .unwrap();
        assert_eq!(completion["params"]["position"], json!({"line":line,"character":5}));
        assert_eq!(state["documents"][lsp::uri(source).unwrap()]["languageId"], "fixture-protocol");
        assert_eq!(state["requests"][0]["params"]["rootUri"], lsp::uri(&root).unwrap());
        ws.update_in(cx, |ws, window, cx| ws.goto_definition(&GotoDefinition, window, cx));
        settle_until(cx, |cx| showing(&ws, cx, &target) || status(&ws, cx).starts_with("Language result discarded"));
        assert!(showing(&ws, cx, &target), "{}", status(&ws, cx));
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.selections[0].range(), 1..7));
    }
}

#[gpui::test]
fn runtime_project_server_settings_cannot_supply_executables(cx: &mut TestAppContext) {
    let root = temp("orlo-runtime-explicit-boundary").canonicalize().unwrap();
    let marker = root.join("must-not-run");
    let local = root.join("Languages/hostile");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("language.json"), json!({"schema":1,"id":"hostile","name":"Hostile","scope":"source.hostile",
        "files":{"extensions":["hostile"]},"lsp":{"language_id":"hostile","command":["/bin/sh","-c",format!("touch {}", marker.display())]}}).to_string()).unwrap();
    let (project, warnings) = crate::config::project::Project::parse(&root.join("app.sublime-project"), &json!({
        "settings":{"lsp_enabled":true,"language_servers":{"rust":{"command":["/bin/sh","-c",format!("touch {}", marker.display())]}}}
    }).to_string()).unwrap();
    assert_eq!(warnings.len(), 2);
    assert!(warnings.iter().any(|w| w.contains("language_servers") && w.contains("application-wide")));
    assert!(!project.settings.preferences.contains_key("language_servers"));
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.project = Some(project);
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        assert!(crate::languages::registry::registry().get("hostile").is_none());
    });
    assert!(!marker.exists());
}

#[gpui::test]
fn runtime_language_explicit_syntax_and_shared_view_routing(cx: &mut TestAppContext) {
    let root = temp("orlo-runtime-explicit").canonicalize().unwrap();
    let source = root.join("a.orlofixture");
    std::fs::write(&source, "😀pri\n").unwrap();
    let (ws, cx) = window(cx);
    runtime_manifest("fixture", &source, json!({}));
    cx.update(|_, cx| serve(cx, config(&source)));
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.open_path(source.clone(), window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
        let original = ws.active_editor().unwrap();
        let settings = cx
            .global::<Settings>()
            .overlay(
                json!({"overrides":[
                    {"language":"Fixture Runtime", "settings":{"format_on_save":true,"tab_size":3}},
                    {"language":"Rust", "settings":{"tab_size":7}}
                ]})
                .as_object()
                .unwrap(),
            )
            .unwrap();
        cx.set_global(settings);
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.set_syntax(&SetSyntax { name: "Rust".into() }, window, cx);
        let settings = ws.editor_settings(ws.active_editor().unwrap().read(cx), cx);
        assert_eq!(settings.tab_size, 7);
        assert!(!settings.format_on_save, "explicit Rust must not use the extension manifest's preferences");
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        assert_eq!(state(&manager, &request.document)["documents"][lsp::uri(&source).unwrap()]["languageId"], "rust");
        drop(manager);
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        assert_eq!(state(&manager, &request.document)["documents"][lsp::uri(&source).unwrap()]["languageId"], "rust");
        drop(manager);
        ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx);
        assert!(ws.language_request(cx).is_err(), "Plain Text must not retain the extension's server");
        let (input, request) = ws.language_request_for(&original, cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        assert_eq!(
            state(&manager, &request.document)["documents"][lsp::uri(&source).unwrap()]["languageId"],
            "fixture-protocol"
        );
    });
}

#[gpui::test]
fn modeline_language_routes_without_starting_and_expires_stale_captures(cx: &mut TestAppContext) {
    let root = temp("orlo-modeline-routing").canonicalize().unwrap();
    let source = root.join("a.txt");
    std::fs::write(root.join(".editorconfig"), "root=true").unwrap();
    let original = "# -*- mode: python; -*-\npass\n";
    std::fs::write(&source, original).unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_settings(r#"{"modelines":true}"#, window, cx);
        serve(cx, config(&source));
        ws.open_path(source.clone(), window, cx);
        let editor = ws.active_editor().unwrap();
        let before = ws.language_input(cx).documents[0].clone();
        assert_eq!(before.language().unwrap().id, "python");
        assert!(before.matches(editor.read(cx), cx));
        assert!(ws.language.manager.lock().unwrap().sessions.is_empty());
        ws.poll_language(cx);
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    ws.update_in(cx, |ws, window, cx| {
        assert!(
            ws.language.manager.lock().unwrap().sessions.is_empty(),
            "loading and polling a declaration must not start a server"
        );
        let editor = ws.active_editor().unwrap();
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        assert_eq!(state(&manager, &request.document)["documents"][lsp::uri(&source).unwrap()]["languageId"], "python");
        drop(manager);
        let before = request.document;
        let epoch = cx.global::<Service>().generation.load(Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        editor.update(cx, |e, cx| e.begin_signature(cancelled.clone(), window, cx));
        ws.apply_settings(r#"{"modelines":false}"#, window, cx);
        ws.apply_view(cx);
        assert!(cancelled.load(Ordering::Relaxed), "language changes immediately retire pending help");
        assert!(editor.read(cx).signature_pending.is_none());
        assert!(!before.matches(editor.read(cx), cx), "same text/version must not retain a disabled language identity");
        assert!(cx.global::<Service>().generation.load(Ordering::Relaxed) > epoch);
        assert!(ws.language_input(cx).documents[0].language().is_none());
        ws.apply_settings(r#"{"modelines":true}"#, window, cx);
        ws.apply_view(cx);
        ws.set_syntax(&SetSyntax { name: "Rust".into() }, window, cx);
        assert_eq!(ws.language_input(cx).documents[0].language().unwrap().id, "rust");
        assert_eq!(editor.read(cx).highlighter.language_override(), Some("Rust"));
        assert_eq!(editor.read(cx).text(cx), original);
    });
    assert_eq!(std::fs::read_to_string(&source).unwrap(), original);
}

#[test]
fn modeline_poll_closes_changed_binding_keeps_siblings_and_defers_startup() {
    let root = temp("orlo-modeline-poll-binding").canonicalize().unwrap();
    let mut input = input(
        &root,
        vec![document(&root, "source.c", 1), document(&root, "sibling.c", 2), document(&root, "other.py", 3)],
    );
    let mut manager = Manager::default();
    manager.sync(&input);
    let clang = state(&manager, &input.documents[1])["pid"].clone();
    let python = state(&manager, &input.documents[2])["pid"].clone();
    let uri = lsp::uri(&input.documents[0].path).unwrap();
    input.documents[0].syntax_override = Some("cpp".into());
    input.documents[0].modeline_language = true;
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    let sibling = state(&manager, &input.documents[1]);
    assert_eq!(sibling["pid"], clang, "declaration refresh keeps sibling transport alive");
    assert!(sibling["documents"].get(&uri).is_none(), "old language binding must be closed");
    assert_eq!(state(&manager, &input.documents[2])["pid"], python);
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    assert!(state(&manager, &input.documents[1])["documents"].get(&uri).is_none());
    input.serial += 1;
    let request = request(&input);
    manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
    let admitted = state(&manager, &input.documents[0]);
    assert_eq!(admitted["documents"][&uri]["languageId"], "cpp");
    assert_eq!(admitted["documents"][lsp::uri(&input.documents[1].path).unwrap()]["languageId"], "c");
    assert_ne!(admitted["pid"], clang, "explicit assistance retires old protocol diagnostics");
    assert_eq!(state(&manager, &input.documents[2])["pid"], python);
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    assert_eq!(state(&manager, &input.documents[0])["pid"], admitted["pid"]);
    input.documents[0].syntax_override = Some("go".into());
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    assert!(manager.sessions.values().filter_map(|s| s.as_ref().ok()).any(|s| s.retired_modelines.contains(&uri)));
    input.documents.remove(0);
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    assert!(
        manager.sessions.values().filter_map(|s| s.as_ref().ok()).all(|s| s.retired_modelines.is_empty()),
        "retired identities are bounded by current input membership"
    );
    assert_eq!(state(&manager, &input.documents[0])["pid"], admitted["pid"]);
}

#[gpui::test]
fn modeline_registered_language_and_hostile_contents_remain_data(cx: &mut TestAppContext) {
    let root = temp("orlo-modeline-hostile").canonicalize().unwrap();
    let source = root.join("source.txt");
    let marker = root.join("must-not-exist");
    let original = format!("# -*- eval: (shell-command \"touch {}\"); mode: fixture; -*-\n// kate: syntax Fixture Runtime; command curl https://invalid.invalid/; encoding utf-16;\n// vim: set foldexpr=execute('touch {}') shell=sh:\n", marker.display(), marker.display());
    std::fs::write(root.join(".editorconfig"), "root=true").unwrap();
    std::fs::write(&source, &original).unwrap();
    let (ws, cx) = window(cx);
    runtime_manifest("fixture", &source, json!({}));
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_settings(
            r#"{"modelines":true,"overrides":[{"language":"Fixture Runtime","settings":{"tab_size":3}}]}"#,
            window,
            cx,
        );
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.open_path(source.clone(), window, cx);
        let e = ws.active_editor().unwrap();
        let e = e.read(cx);
        assert_eq!(e.highlighter.syntax_name(), "Plain Text", "registered identity needs no new parser");
        assert_eq!(e.highlighter.language_override(), None);
        assert_eq!(e.doc(cx).tab_size, 3);
        assert_eq!(ws.language_input(cx).documents[0].language().unwrap().id, "fixture-protocol");
        assert_eq!(e.text(cx), original);
        assert!(!e.doc(cx).unsaved());
        assert!(ws.language.manager.lock().unwrap().sessions.is_empty());
        ws.poll_language(cx);
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    ws.read_with(cx, |ws, _| assert!(ws.language.manager.lock().unwrap().sessions.is_empty()));
    assert!(!marker.exists());
    assert_eq!(std::fs::read_to_string(&source).unwrap(), original);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
}

#[test]
fn modeline_pending_route_manual_override_retires_the_old_protocol_transport() {
    let root = temp("orlo-modeline-manual-route").canonicalize().unwrap();
    let mut input = input(&root, vec![document(&root, "source.c", 1), document(&root, "sibling.c", 2)]);
    let mut manager = Manager::default();
    manager.sync(&input);
    let before = state(&manager, &input.documents[1])["pid"].clone();
    let uri = lsp::uri(&input.documents[0].path).unwrap();
    input.documents[0].syntax_override = Some("Plain Text".into());
    input.documents[0].modeline_language = true;
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    assert_eq!(manager.sessions.len(), 1, "a route-less declaration must not start anything");
    let sibling = state(&manager, &input.documents[1]);
    assert_eq!(sibling["pid"], before);
    assert!(sibling["documents"].get(&uri).is_none());
    input.documents[0].modeline_language = false;
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    assert_eq!(state(&manager, &input.documents[1])["pid"], before);
    assert!(
        manager.sessions.values().filter_map(|s| s.as_ref().ok()).any(|s| s.retired_modelines.contains(&uri)),
        "a still-open route-less URI keeps its retired identity"
    );
    input.documents[0].syntax_override = Some("C++".into());
    input.documents[0].modeline_language = false;
    input.serial += 1;
    manager.sync_with_modelines(&input, false);
    let manual = state(&manager, &input.documents[0]);
    assert_eq!(manual["documents"][&uri]["languageId"], "cpp");
    assert_ne!(manual["pid"], before, "manual syntax uses the existing fresh-transport identity policy");
    assert_eq!(manual["documents"][lsp::uri(&input.documents[1].path).unwrap()]["languageId"], "c");
}

#[gpui::test]
fn runtime_language_reload_settings_and_malformed_manifest(cx: &mut TestAppContext) {
    let root = temp("orlo-runtime-reload").canonicalize().unwrap();
    let source = root.join("a.orlofixture");
    std::fs::write(&source, "😀pri\n").unwrap();
    let (ws, cx) = window(cx);
    let manifest = runtime_manifest("fixture", &source, json!({}));
    cx.update(|_, cx| serve(cx, Default::default()));
    let (old_input, old_request) = ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.open_path(source.clone(), window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
        let (input, request) = ws.language_request(cx).unwrap();
        ws.language.manager.lock().unwrap().request(&input, &request, "textDocument/hover", json!({})).unwrap();
        (input, request)
    });
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| !ws.active_editor().unwrap().read(cx).diagnostics.is_empty()));
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    value["lsp"]["language_id"] = json!("fixture-reloaded");
    std::fs::write(&manifest, value.to_string()).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        assert!(!ws.language_request_current(&old_request, cx));
        assert!(
            ws.active_editor().unwrap().read(cx).diagnostics.is_empty(),
            "reload must clear old decorations immediately"
        );
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        assert!(manager.request(&old_input, &old_request, "textDocument/hover", json!({})).is_err());
        manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        let state = state(&manager, &request.document);
        assert_eq!(state["documents"][lsp::uri(&source).unwrap()]["languageId"], "fixture-reloaded");
        assert_eq!(count(&state, "requests", "initialize"), 1);
    });
    cx.update(|_, cx| {
        let mut override_config = config(&source).remove("rust").unwrap();
        override_config.initialization_options["formattedText"] = json!("settings-override");
        cx.global_mut::<Settings>().language_servers.insert("fixture-family".into(), override_config);
    });
    enable_format_on_save(cx);
    save_document(&ws, cx);
    settle_until(cx, |_| std::fs::read_to_string(&source).unwrap() == "😀settings-override\n");
    cx.update(|_, cx| {
        cx.global_mut::<Settings>().language_servers.insert("fixture-family".into(), ServerConfig::default());
    });
    ws.read_with(cx, |ws, cx| {
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        assert!(manager.request(&input, &request, "textDocument/hover", json!({})).is_err());
        assert!(manager.sessions.is_empty());
    });
    std::fs::write(&manifest, r#"{"schema":1,"id":"fixture","typo":true}"#).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        assert!(ws.language_request(cx).is_err());
    });
    let (_, error) = cx.pending_prompt().unwrap();
    assert!(error.contains("unknown field `typo`"));
    cx.simulate_prompt_answer("Ok");
}

#[gpui::test]
fn runtime_reload_keeps_unaffected_processes_documents_and_sessions(cx: &mut TestAppContext) {
    let root = temp("orlo-runtime-selective-reload").canonicalize().unwrap();
    let other = root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let (ws, cx) = window(cx);
    let manifest = runtime_manifest("fixture", &root.join("target.orlofixture"), json!({}));
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    let mut input = input(
        &root,
        vec![
            document(&root, "a.orlofixture", 1),
            document(&root, "b.rs", 2),
            document(&other, "c.orlofixture", 3),
            document(&other, "d.rs", 4),
        ],
    );
    let overrides = input.config.clone();
    input.config = language::runtime_servers(&input.registry);
    input.config.extend(overrides.clone());
    let rebind = |input: &mut Input| {
        input.serial += 1;
        input.registry = crate::languages::registry::registry();
        for document in &mut input.documents {
            document.registry = input.registry.clone();
        }
        input.config = language::runtime_servers(&input.registry);
        input.config.extend(overrides.clone());
    };
    let identity = |manager: &Manager, document: &Document| {
        let state = state(manager, document);
        let session = manager.sessions[&server_key(document).unwrap()].as_ref().unwrap();
        (
            state["pid"].as_u64().unwrap(),
            count(&state, "notifications", "textDocument/didOpen"),
            session.documents[&lsp::uri(&document.path).unwrap()].serial,
        )
    };
    let mut manager = Manager::default();
    manager.sync(&input);
    let before: Vec<_> = input.documents.iter().map(|d| identity(&manager, d)).collect();
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    rebind(&mut input);
    manager.sync(&input);
    for (d, before) in input.documents.iter().zip(&before) {
        assert_eq!(identity(&manager, d), *before, "no-op reload must retain the live process and opening");
    }
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    value["lsp"]["language_id"] = json!("fixture-new-protocol");
    std::fs::write(&manifest, value.to_string()).unwrap();
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    rebind(&mut input);
    manager.sync(&input);
    for (i, d) in input.documents.iter().enumerate() {
        if i % 2 == 0 {
            assert_ne!(identity(&manager, d).0, before[i].0);
            assert_eq!(identity(&manager, d).1, 1);
            assert_eq!(
                state(&manager, d)["documents"][lsp::uri(&d.path).unwrap()]["languageId"],
                "fixture-new-protocol"
            );
        } else {
            assert_eq!(identity(&manager, d), before[i]);
        }
    }
    let fixture_before = identity(&manager, &input.documents[0]);
    input.serial += 1;
    input.config.get_mut("rust").unwrap().initialization_options["changed"] = json!(true);
    manager.sync(&input);
    assert_eq!(
        identity(&manager, &input.documents[0]),
        fixture_before,
        "another family's settings cannot restart this server"
    );
    let rust_before: Vec<_> = [1, 3].map(|i| identity(&manager, &input.documents[i])).into();
    // A missing executable affects only its own family. Installing it and reloading retries
    // the cached failure without restarting either unrelated root's live server.
    let executable = root.join("fixture-python");
    value["lsp"]["command"][0] = json!(executable);
    std::fs::write(&manifest, value.to_string()).unwrap();
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    let changed_rust = input.config["rust"].clone();
    rebind(&mut input);
    input.config.insert("rust".into(), changed_rust.clone());
    let decorations = manager.sync(&input);
    assert_eq!(decorations.iter().filter(|d| d.status.contains("Cannot start")).count(), 2);
    for d in decorations.iter().filter(|d| d.document.path.extension().is_some_and(|e| e == "orlofixture")) {
        assert!(
            d.status.contains("Cannot start") && d.status.contains(&executable.display().to_string()),
            "{}",
            d.status
        );
    }
    for (i, before) in [1, 3].into_iter().zip(&rust_before) {
        assert_eq!(identity(&manager, &input.documents[i]), *before);
    }
    std::fs::write(&executable, "#!/bin/sh\nexec python3 \"$@\"\n").unwrap();
    std::fs::set_permissions(&executable, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    rebind(&mut input);
    input.config.insert("rust".into(), changed_rust.clone());
    manager.sync(&input);
    assert!(state(&manager, &input.documents[0])["pid"].as_u64().is_some());
    for (i, before) in [1, 3].into_iter().zip(&rust_before) {
        assert_eq!(identity(&manager, &input.documents[i]), *before);
    }
    std::fs::write(&manifest, r#"{"schema":1,"id":"fixture","typo":true}"#).unwrap();
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    let (_, detail) = cx.pending_prompt().unwrap();
    assert!(detail.contains("typo") && detail.contains(&manifest.display().to_string()));
    cx.simulate_prompt_answer("Ok");
    rebind(&mut input);
    input.config.insert("rust".into(), changed_rust);
    manager.sync(&input);
    for (i, before) in [1, 3].into_iter().zip(&rust_before) {
        assert_eq!(identity(&manager, &input.documents[i]), *before);
        let mut request = request(&input);
        request.document = input.documents[i].clone();
        let response = manager.request(&input, &request, "textDocument/completion", json!({})).unwrap();
        assert_eq!(response.value[0]["label"], "print");
        assert_eq!(
            state(&manager, &request.document)["documents"][lsp::uri(&request.document.path).unwrap()]["languageId"],
            "rust"
        );
    }
}

#[gpui::test]
fn runtime_server_settings_cancel_held_transport_and_preserve_unrelated_session(cx: &mut TestAppContext) {
    let (ws, cx, source, target) = open(cx, "orlo-runtime-settings-held-signature", "target");
    let sibling = source.parent().unwrap().join("unrelated.go");
    let marker = source.parent().unwrap().join("signature-held");
    std::fs::write(&sibling, "😀pri\n").unwrap();
    rust_options(cx, |o| *o = json!({"target":lsp::uri(&target).unwrap(), "signatureHoldFirst":marker}));
    ws.update_in(cx, |ws, window, cx| ws.open_path(sibling.clone(), window, cx));
    settle_until(cx, |cx| showing(&ws, cx, &sibling));
    ws.update_in(cx, |ws, window, cx| ws.open_path(source.clone(), window, cx));
    activate(cx);
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    let (manager, key, before, other_before) = ws.read_with(cx, |ws, cx| {
        let (input, request) = ws.language_request(cx).unwrap();
        let manager = ws.language.manager.clone();
        let mut guard = manager.lock().unwrap();
        guard.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        let before = state(&guard, &request.document);
        let other = input.documents.iter().find(|d| d.path == sibling).unwrap();
        let other_before = state(&guard, other);
        drop(guard);
        (manager, server_key(&request.document).unwrap(), before, other_before)
    });
    let token = Arc::new(AtomicBool::new(false));
    ws.update_in(cx, |ws, window, cx| {
        ws.language.signature_cancel = Some(token.clone());
        ws.active_editor().unwrap().update(cx, |e, cx| {
            assert!(e.signature_allowed(window));
            e.begin_signature(token.clone(), window, cx);
        });
    });
    let thread_token = token.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut manager = manager.lock().unwrap();
        let client = &mut manager.sessions.get_mut(&key).unwrap().as_mut().unwrap().client;
        let result = client.request_cancellable("textDocument/signatureHelp", json!({}), &thread_token);
        let wire = client.request("test/state", json!({})).unwrap();
        done_tx.send((result, wire)).ok();
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(marker.exists(), "the fixture must hold the actual signature request");
    assert!(!token.load(Ordering::Relaxed));
    rust_options(cx, |o| o["revision"] = json!(2));
    cx.run_until_parked();
    let (result, wire) = done_rx.recv_timeout(std::time::Duration::from_secs(1)).expect("Manager stayed blocked");
    worker.join().unwrap();
    assert!(result.is_err());
    assert_eq!(wire["pid"], before["pid"]);
    assert_eq!(count(&wire, "requests", "textDocument/signatureHelp"), 1);
    assert_eq!(count(&wire, "notifications", "$/cancelRequest"), 1);
    ws.update_in(cx, |ws, window, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert!(e.signature_allowed(window), "no focus loss may provide cancellation");
        assert!(e.signature_help.is_none() && e.signature_pending.is_none() && e.signature_inflight.is_none());
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        manager.request(&input, &request, "textDocument/completion", json!({})).unwrap();
        assert_ne!(state(&manager, &request.document)["pid"], before["pid"]);
        let other = input.documents.iter().find(|d| d.path == sibling).unwrap();
        let after = state(&manager, other);
        assert_eq!(after["pid"], other_before["pid"]);
        assert_eq!(count(&after, "notifications", "textDocument/didOpen"), 1);
    });
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn runtime_server_settings_cancel_assistance_in_an_active_focused_window(cx: &mut TestAppContext) {
    for mode in ["pending-options", "shown-options", "pending-disable", "shown-disable"] {
        let (ws, cx, source, _) = open(cx, &format!("orlo-runtime-active-settings-{mode}"), "target");
        activate(cx);
        if mode.starts_with("shown") {
            ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
            settle_until(cx, |cx| {
                ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some())
            });
        }
        let (token, before) = ws.update_in(cx, |ws, window, cx| {
            let (input, request) = ws.language_request(cx).unwrap();
            let mut manager = ws.language.manager.lock().unwrap();
            manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
            let before = state(&manager, &request.document);
            drop(manager);
            let token = if mode.starts_with("shown") {
                ws.language.signature_cancel.clone().unwrap()
            } else {
                let token = Arc::new(AtomicBool::new(false));
                ws.language.signature_cancel = Some(token.clone());
                ws.active_editor().unwrap().update(cx, |e, cx| e.begin_signature(token.clone(), window, cx));
                token
            };
            let editor = ws.active_editor().unwrap().read(cx);
            assert!(editor.signature_allowed(window), "{mode}: source must stay active and focused");
            assert!(!token.load(Ordering::Relaxed));
            assert!(editor.signature_pending.is_some() || editor.signature_help.is_some());
            ws.language.range_formatting =
                Some((cx.global::<Service>().generation.load(Ordering::Relaxed), vec![request.document.clone()]));
            ws.language.completion = Some((request, Encoding::Utf16, vec![json!({"label":"print"})]));
            (token, before)
        });
        cx.update(|_, cx| {
            let mut settings = cx.global::<Settings>().clone();
            if mode.ends_with("disable") {
                settings.lsp_enabled = false;
            } else {
                // A valid unrelated family change must clear obsolete UI without restarting Rust.
                settings.language_servers.get_mut("go").unwrap().initialization_options["revision"] = json!(2);
            }
            cx.set_global(settings);
        });
        cx.run_until_parked();
        assert!(cx.pending_prompt().is_none(), "a valid setting must not steal focus with a prompt");
        ws.update_in(cx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap().read(cx);
            assert!(editor.signature_allowed(window), "{mode}: cleanup must not come from focus loss");
            assert!(token.load(Ordering::Relaxed), "{mode}: settings must cancel assistance immediately");
            assert!(editor.signature_pending.is_none() && editor.signature_inflight.is_none());
            assert!(editor.signature_help.is_none());
            assert!(ws.language.completion.is_none() && ws.language.range_formatting.is_none());
            let mut manager = ws.language.manager.lock().unwrap();
            if mode.ends_with("disable") {
                let input = ws.language_input(cx);
                manager.sync(&input);
                assert!(manager.sessions.is_empty());
            } else {
                let (input, request) = ws.language_request(cx).unwrap();
                manager.request(&input, &request, "textDocument/completion", json!({})).unwrap();
                let after = state(&manager, &request.document);
                assert_eq!(after["pid"], before["pid"]);
                assert_eq!(count(&after, "notifications", "textDocument/didOpen"), 1);
                assert_eq!(after["documents"][lsp::uri(&source).unwrap()]["languageId"], "rust");
            }
        });
    }
}

#[gpui::test]
fn runtime_settings_invalid_family_keeps_live_siblings_and_cancels_old_assistance(cx: &mut TestAppContext) {
    let (ws, cx, source, _) = open(cx, "orlo-runtime-invalid-settings", "target");
    activate(cx);
    let token = Arc::new(AtomicBool::new(false));
    let before = ws.update_in(cx, |ws, window, cx| {
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        let before = state(&manager, &request.document);
        drop(manager);
        ws.language.signature_cancel = Some(token.clone());
        ws.active_editor().unwrap().update(cx, |e, cx| e.begin_signature(token.clone(), window, cx));
        let mut values = serde_json::to_value(cx.global::<Settings>()).unwrap();
        values["language_servers"]["broken"] = json!({"command":[""],"initialization_options":{}});
        values["language_servers"]["go"]["initialization_options"]["revision"] = json!(2);
        ws.apply_settings(&values.to_string(), window, cx);
        before
    });
    let (_, detail) = cx.pending_prompt().unwrap();
    assert!(detail.contains("language_servers.broken") && detail.contains("command"), "{detail}");
    cx.simulate_prompt_answer("Ok");
    cx.run_until_parked();
    assert!(token.load(Ordering::Relaxed), "server settings must cancel old signature assistance");
    ws.read_with(cx, |ws, cx| {
        let (input, request) = ws.language_request(cx).unwrap();
        let mut manager = ws.language.manager.lock().unwrap();
        let response = manager.request(&input, &request, "textDocument/completion", json!({})).unwrap();
        assert_eq!(response.value[0]["label"], "print");
        let after = state(&manager, &request.document);
        assert_eq!(after["pid"], before["pid"]);
        assert_eq!(after["documents"][lsp::uri(&source).unwrap()]["languageId"], "rust");
        assert_eq!(count(&after, "notifications", "textDocument/didOpen"), 1);
    });
}

#[gpui::test]
fn runtime_reload_cancels_pending_signature_and_clears_assistance_cache(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-runtime-signature-reload", "target");
    let token = Arc::new(AtomicBool::new(false));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.begin_signature(token.clone(), window, cx));
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        let e = ws.active_editor().unwrap();
        assert!(token.load(Ordering::Relaxed), "reload must cancel the in-flight request immediately");
        assert!(e.read(cx).signature_pending.is_none());
        assert!(e.read(cx).signature_inflight.is_none());
        assert!(ws.language.completion.is_none());
        assert!(ws.language.range_formatting.is_none());
    });
}

#[gpui::test]
fn runtime_language_reload_discards_pending_save_formatting(cx: &mut TestAppContext) {
    let root = temp("orlo-runtime-save-reload").canonicalize().unwrap();
    let source = root.join("a.orlofixture");
    std::fs::write(&source, "😀pri\n").unwrap();
    let (ws, cx) = window(cx);
    let manifest = runtime_manifest("fixture", &source, json!({}));
    cx.update(|_, cx| serve(cx, Default::default()));
    ws.update_in(cx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        ws.open_path(source.clone(), window, cx);
    });
    enable_format_on_save(cx);
    let formatting = hold_formatting(&source);
    save_document(&ws, cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    std::fs::remove_file(&manifest).unwrap();
    // Remove the now-empty folder, so the reload is clean rather than an invalid folder.
    std::fs::remove_dir(manifest.parent().unwrap()).unwrap();
    ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "😀pri\n");
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert!(status(&ws, cx).starts_with("Save formatting failed"));
    assert!(cx.has_pending_prompt());
}

#[gpui::test]
fn runtime_language_manual_edits_allow_shared_syntax_choices_but_reject_changed_or_duplicate_buffers(
    cx: &mut TestAppContext,
) {
    let (ws, cx, path, _) = open(cx, "orlo-runtime-shared-manual-edits", "😀target\n");
    let original = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let other = ws.update_in(cx, |ws, window, cx| {
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let other = ws.active_editor().unwrap();
        ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx);
        ws.activate_view(original.entity_id(), window, cx);
        other
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| ws.lsp_format(&LspFormat, window, cx));
    settle_until(cx, |cx| {
        status(&ws, cx).contains("applied")
            || status(&ws, cx).contains("discarded")
            || status(&ws, cx).contains("Different buffers")
    });
    assert_eq!(text(&ws, cx), "😀formatted\n", "{}", status(&ws, cx));
    assert_eq!(other.read_with(cx, |e, cx| e.doc(cx).text()), "😀formatted\n");
    ws.update(cx, |_, cx| original.update(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert_eq!(other.read_with(cx, |e, cx| e.doc(cx).text()), "😀pri\n");

    let prepared = |ws: &Workspace, cx: &App| {
        let (_, request) = ws.language_request_for(&original, cx).unwrap();
        let edits = json!({"changes":{lsp::uri(&path).unwrap():[{
            "range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},
            "newText":"changed"
        }]}});
        prepare_edits(&request, response(edits, path.parent().unwrap())).unwrap()
    };
    ws.update_in(cx, |ws, window, cx| {
        let edits = prepared(ws, cx);
        original.update(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
        assert!(ws.apply_language_edits(edits, window, cx).unwrap_err().contains("changed"));
        assert_eq!(original.read(cx).doc(cx).text(), "😀prix\n");
        original.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        let edits = prepared(ws, cx);
        ws.new_file(&NewFile, window, cx);
        let duplicate = ws.active_editor().unwrap();
        duplicate.update(cx, |e, cx| {
            e.set_text("😀pri\n", cx);
            e.update_doc(cx, |d| d.path = Some(path.clone()));
        });
        assert!(!duplicate.read(cx).same_document(original.read(cx)));
        assert!(
            ws.apply_language_edits(edits, window, cx).is_err(),
            "independent duplicate buffers must still reject edits"
        );
        assert_eq!(original.read(cx).doc(cx).text(), "😀pri\n");
        assert_eq!(duplicate.read(cx).doc(cx).text(), "😀pri\n");
    });
}

fn same_family_switch_isolates_delayed_old_identity_diagnostics(kind: &str) {
    let root = temp(&format!("orlo-runtime-identity-diagnostics-{kind}")).canonicalize().unwrap();
    let gate = root.join("identity-opened");
    let published = root.join("identity-published");
    let mut input = input(&root, vec![document(&root, "source.c", 1)]);
    input.config = sync_config(
        &root.join("target.c"),
        1,
        None,
        json!({
            "identityDiagnostics":kind, "identityGate":gate, "identityPublished":published
        }),
    );
    let mut manager = Manager::default();
    manager.sync(&input);
    let before = state(&manager, &input.documents[0]);
    let uri = lsp::uri(&input.documents[0].path).unwrap();
    assert_eq!(before["documents"][&uri]["languageId"], "c");
    input.documents[0].syntax_override = Some("C++".into());
    input.serial += 1;
    manager.sync(&input);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !published.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let publication: Value = serde_json::from_str(
        &std::fs::read_to_string(&published).expect("old client must actually publish after the new identity opens"),
    )
    .unwrap();
    assert_eq!(publication, json!({"old":"c", "new":"cpp", "kind":kind}));
    let after = state(&manager, &input.documents[0]);
    assert_eq!(after["documents"][&uri]["languageId"], "cpp");
    let decorations = manager.sync(&input);
    let current = decorations.iter().find(|d| d.document.path == input.documents[0].path).unwrap();
    assert_eq!(
        current.diagnostics.iter().map(|d| d.message.as_str()).collect::<Vec<_>>(),
        ["fixture diagnostic"],
        "{kind}: old-client diagnostics must not become new-identity decorations"
    );
}

#[test]
fn runtime_language_same_family_switch_isolates_versioned_old_identity_diagnostics() {
    same_family_switch_isolates_delayed_old_identity_diagnostics("versioned");
}

#[test]
fn runtime_language_same_family_switch_isolates_unversioned_old_identity_diagnostics() {
    same_family_switch_isolates_delayed_old_identity_diagnostics("unversioned");
}

#[gpui::test]
fn runtime_language_manual_edits_discard_changed_source_choice(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "orlo-runtime-changed-source-choice", "😀target\n");
    ws.update_in(cx, |ws, window, cx| {
        let (_, request) = ws.language_request(cx).unwrap();
        let edit = json!({"changes":{lsp::uri(&path).unwrap():[{
            "range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},
            "newText":"must-not-apply"
        }]}});
        ws.stage_language_edits(request, response(edit, path.parent().unwrap()), window, cx);
        ws.set_syntax(&SetSyntax { name: "Plain Text".into() }, window, cx);
    });
    settle_until(cx, |cx| status(&ws, cx).starts_with("Server edits discarded"));
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn runtime_language_conflicting_window_polls_keep_one_stable_owner_and_requests_use_their_source(
    cx: &mut TestAppContext,
) {
    let root = temp("orlo-runtime-stable-poll-owner").canonicalize().unwrap();
    let source = root.join("source.c");
    std::fs::write(&source, "😀pri\n").unwrap();
    let (ws1, _) = window(cx);
    let (ws2, _) = cx.add_window_view(Workspace::new);
    cx.update(|cx| serve(cx, config(&source)));
    let mut cx1 = VisualTestContext::from_window(cx.windows()[0], cx);
    let mut cx2 = VisualTestContext::from_window(cx.windows()[1], cx);
    let owner = ws1.update_in(&mut cx1, |ws, window, cx| {
        ws.open_path(root, window, cx);
        ws.open_path(source.clone(), window, cx);
        ws.active_editor().unwrap()
    });
    let other = ws2.update_in(&mut cx2, |ws, window, cx| {
        ws.open_path(source.clone(), window, cx);
        ws.set_syntax(&SetSyntax { name: "C++".into() }, window, cx);
        ws.active_editor().unwrap()
    });
    assert!(owner.read_with(&cx1, |e, _| e.document.clone()) == other.read_with(&cx2, |e, _| e.document.clone()));
    let manager = ws1.read_with(&cx1, |ws, _| ws.language.manager.clone());
    let poll = |ws: &Entity<Workspace>, cx: &VisualTestContext| {
        let input = ws.read_with(cx, |ws, cx| ws.language_input(cx));
        let document = input.documents.iter().find(|d| d.path == source).unwrap();
        let mut manager = manager.lock().unwrap();
        manager.sync(&input);
        let state = state(&manager, document);
        assert_eq!(state["documents"][lsp::uri(&source).unwrap()]["languageId"], "c");
        state["pid"].as_u64().unwrap()
    };
    let initial_pid = poll(&ws1, &cx1);
    for _ in 0..3 {
        assert_eq!(poll(&ws2, &cx2), initial_pid, "another window must not restart polling's owner");
        assert_eq!(poll(&ws1, &cx1), initial_pid);
    }
    ws2.read_with(&cx2, |ws, cx| {
        let (input, request) = ws.language_request_for(&other, cx).unwrap();
        let mut manager = manager.lock().unwrap();
        manager.request(&input, &request, "textDocument/hover", json!({})).unwrap();
        let state = state(&manager, &request.document);
        assert_eq!(state["documents"][lsp::uri(&source).unwrap()]["languageId"], "cpp");
        assert_ne!(state["pid"].as_u64().unwrap(), initial_pid);
    });
    // Restore polling's owner, then save from the other view: preparation must use C++ again.
    let owner_pid = poll(&ws1, &cx1);
    enable_format_on_save(&mut cx2);
    save_document(&ws2, &mut cx2);
    settle_until(&mut cx2, |_| std::fs::read_to_string(&source).unwrap() == "😀formatted\n");
    ws2.read_with(&cx2, |ws, cx| {
        let (_, request) = ws.language_request_for(&other, cx).unwrap();
        let state = state(&manager.lock().unwrap(), &request.document);
        assert_eq!(state["documents"][lsp::uri(&source).unwrap()]["languageId"], "cpp");
        assert_eq!(count(&state, "requests", "textDocument/formatting"), 1);
        assert_ne!(state["pid"].as_u64().unwrap(), owner_pid);
    });
    assert_eq!(owner.read_with(&cx1, |e, cx| e.doc(cx).text()), "😀formatted\n");
    let final_owner_pid = poll(&ws1, &cx1);
    for _ in 0..3 {
        assert_eq!(poll(&ws2, &cx2), final_owner_pid);
        assert_eq!(poll(&ws1, &cx1), final_owner_pid);
    }
    ws1.update(&mut cx1, |ws, cx| ws.remove_view(&owner, cx));
    ws2.read_with(&cx2, |ws, cx| {
        let input = ws.language_input(cx);
        let document = input.documents.iter().find(|d| d.path == source).unwrap();
        let mut manager = manager.lock().unwrap();
        manager.sync(&input);
        let state = state(&manager, document);
        assert_eq!(state["documents"][lsp::uri(&source).unwrap()]["languageId"], "cpp");
    });
}

// Like formatting, pause the real fixture response before application: GPUI's
// deterministic executor polls blocking transport requests on the test thread.
pub(super) fn completion_gate_path(path: &Path) -> PathBuf {
    path.join("completion-response")
}

#[gpui::test]
fn virtual_space_mouse_move_discards_pending_completion_primary_and_additional_edits(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-lsp-virtual-gap-resolve", "😀target\n");
    activate(cx);
    let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    editor.update(cx, |e, cx| e.set_virtual_space(true, cx));
    lsp_complete(&ws, cx);
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print".into()));
    let pause = crate::documents::save::gate::pause(&completion_gate_path(&a));
    pick(&ws, cx, "print");
    settle_until(cx, |_| pause.arrived() == 1);
    assert_eq!(waiting(&ws, cx), None, "the committed popup has already been taken");
    assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "completionItem/resolve"), 1);
    let pos = editor.read_with(cx, |e, cx| {
        let end = e.char_bounds(4, cx).unwrap();
        let space = end.left() - e.char_bounds(3, cx).unwrap().left();
        gpui::point(end.left() + space * 3., end.center().y)
    });
    cx.simulate_click(pos, gpui::Modifiers::none());
    editor.read_with(cx, |e, _| assert_eq!(e.state.selections, vec![Selection::from_points((4, 3), (4, 3))]));
    pause.release();
    settle_until(cx, |cx| status(&ws, cx).contains("discarded") || text(&ws, cx) != "😀pri\n");
    assert_eq!(text(&ws, cx), "😀pri\n", "neither primary replacement nor additional import may apply");
    assert!(status(&ws, cx).contains("discarded"));
    editor.read_with(cx, |e, _| assert_eq!(e.state.selections, vec![Selection::from_points((4, 3), (4, 3))]));
}

fn auto_format_tick(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
}

#[gpui::test]
fn file_autosave_formatter_failure_pauses_its_own_edit_and_codec_fix_retries(cx: &mut TestAppContext) {
    use crate::editing::encoding::Encoding;
    let (ws, cx, path, _) = open(cx, "orlo-auto-format-failure", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = true);
    let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    editor.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Iso8859_1, false)));
    auto_format_tick(cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(text(&ws, cx), "😀formatted\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    assert!(status(&ws, cx).contains("Automatic save paused"));
    for _ in 0..4 {
        auto_format_tick(cx);
    }
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 1);
    let version = editor.read_with(cx, |e, cx| e.doc(cx).version);
    editor.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Utf8, false)));
    assert_eq!(version, editor.read_with(cx, |e, cx| e.doc(cx).version));
    auto_format_tick(cx);
    settle_until(cx, |_| std::fs::read_to_string(&path).unwrap() == "😀formatted\n");
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 2);
}

#[gpui::test]
fn file_autosave_disable_during_formatting_cancels_edit_and_capture(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "orlo-auto-format-disable", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = true);
    type_in(&ws, cx, "mine");
    let before = text(&ws, cx);
    let formatting = hold_formatting(&path);
    auto_format_tick(cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = false);
    cx.run_until_parked();
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(text(&ws, cx), before);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    auto_format_tick(cx);
    assert_eq!(count(&captured(&ws, cx, Some(&path)), "requests", "textDocument/formatting"), 1);
}

#[gpui::test]
fn file_autosave_composition_started_during_formatting_never_reaches_disk(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, path, _) = open(cx, "orlo-auto-format-ime", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = true);
    type_in(&ws, cx, "mine");
    let formatting = hold_formatting(&path);
    auto_format_tick(cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
    let composing = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    composing.update_in(cx, |e, w, cx| {
        e.state.virtual_space = true.into();
        e.state.overtype = true;
        let end = e.doc(cx).len();
        e.sel(cx).set_selections(vec![Selection::from_points((end, 5), (end, 5))]);
        e.replace_and_mark_text_in_range(None, "界", None, w, cx);
    });
    let preview = composing.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    auto_format_tick(cx);
    assert_eq!(preview, composing.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
    composing.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(None, "", None, w, cx));
    auto_format_tick(cx);
    settle_until(cx, |_| std::fs::read_to_string(&path).unwrap().contains("formatted"));
}

#[gpui::test]
fn file_autosave_external_change_during_formatting_is_not_overwritten(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "orlo-auto-format-external", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = true);
    type_in(&ws, cx, "mine");
    let formatting = hold_formatting(&path);
    auto_format_tick(cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    std::fs::write(&path, "external😀\n").unwrap();
    formatting.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external😀\n");
    assert!(status(&ws, cx).contains("changed on disk"));
}

#[gpui::test]
fn file_autosave_path_change_and_retained_last_view_cancel_pending_formatting(cx: &mut TestAppContext) {
    for close in [false, true] {
        let (ws, cx, path, _) =
            open(cx, if close { "orlo-auto-format-close" } else { "orlo-auto-format-path" }, "target\n");
        enable_format_on_save(cx);
        cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = true);
        type_in(&ws, cx, "mine");
        let retained = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        let original = text(&ws, cx);
        let formatting = hold_formatting(&path);
        auto_format_tick(cx);
        settle_until(cx, |_| formatting.arrived() == 1);
        let destination = path.with_extension("moved");
        std::fs::write(&destination, "destination").unwrap();
        if close {
            ws.update_in(cx, |ws, w, cx| ws.close_editor(&retained, w, cx));
        } else {
            retained.update(cx, |e, cx| e.update_doc(cx, |d| d.path = Some(destination.clone())));
        }
        formatting.release();
        settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
        auto_format_tick(cx);
        assert_eq!(retained.read_with(cx, |e, cx| e.text(cx)), original);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "destination");
    }
}

#[gpui::test]
fn file_autosave_undo_to_clean_during_formatting_cancels_preparation(cx: &mut TestAppContext) {
    let (ws, cx, path, _) = open(cx, "orlo-auto-format-undo-clean", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| cx.global_mut::<Settings>().file_autosave = true);
    type_in(&ws, cx, "mine");
    let formatting = hold_formatting(&path);
    let writes = crate::documents::save::gate::pause(&path);
    auto_format_tick(cx);
    settle_until(cx, |_| formatting.arrived() == 1);
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "😀pri\n");
    formatting.release();
    cx.run_until_parked();
    assert_eq!(writes.arrived(), 0, "a clean document is no longer eligible");
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "😀pri\n");
}

#[gpui::test]
fn declared_signature_trigger_and_retrigger_pass_context_and_allow_signature_navigation(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, _, b) = open(cx, "orlo-signature-triggers", "target");
    rust_options(cx, |o| {
        *o = json!({"target":lsp::uri(&b).unwrap(),
        "signatureProvider":{"triggerCharacters":["("],"retriggerCharacters":[","]},
        "signatureResult":{"signatures":[{"label":"f(a)","documentation":"first"},
            {"label":"f(😀, 中)","parameters":[{"label":[2,4]},{"label":[6,7]}],"documentation":"second"}]}})
    });
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "(", window, cx))
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("popup").is_some());
    cx.simulate_keystrokes("down");
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        let help = &e.signature_help.as_ref().unwrap().help;
        assert_eq!(help.active, 1);
        assert_eq!(help.signatures[1].documentation.as_deref(), Some("second"));
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, ",", window, cx))
    });
    settle_until(cx, |cx| count(&captured(&ws, cx, None), "requests", "textDocument/signatureHelp") == 2);
    let state = captured(&ws, cx, None);
    let requests: Vec<_> =
        state["requests"].as_array().unwrap().iter().filter(|r| r["method"] == "textDocument/signatureHelp").collect();
    assert_eq!(requests[0]["params"]["context"]["triggerCharacter"], "(");
    assert_eq!(requests[1]["params"]["context"]["isRetrigger"], true);
    assert_eq!(requests[1]["params"]["context"]["activeSignatureHelp"]["activeSignature"], 1);
    assert_eq!(
        requests[1]["params"]["context"]["activeSignatureHelp"]["signatures"][1]["parameters"][1]["label"],
        json!([6, 7])
    );
}

#[gpui::test]
fn empty_and_failed_signature_results_do_not_show_help(cx: &mut TestAppContext) {
    for error in [false, true] {
        let (ws, cx, _, b) = open(cx, if error { "orlo-signature-error" } else { "orlo-signature-empty" }, "target");
        rust_options(cx, |o| {
            *o = json!({"target":lsp::uri(&b).unwrap(), "signatureError":error, "signatureResult":null})
        });
        activate(cx);
        ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
        settle_until(cx, |cx| {
            if error {
                status(&ws, cx).contains("fixture signature failure")
            } else {
                status(&ws, cx).contains("No signature help")
            }
        });
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
    }
}

pub(super) fn signature_gate_path(path: &Path) -> PathBuf {
    path.with_extension("signature-gate")
}

#[gpui::test]
fn delayed_signature_after_caret_move_is_discarded(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-signature-delay", "target");
    activate(cx);
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |_| pause.arrived() == 1);
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.jump_to(0, cx)));
    pause.release();
    cx.run_until_parked();
    assert!(!status(&ws, cx).contains("cancelled"));
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
}

#[gpui::test]
fn signature_pending_edits_and_nested_trigger_use_current_context(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, a, b) = open(cx, "orlo-signature-rapid", "target");
    rust_options(cx, |o| {
        *o = json!({"target":lsp::uri(&b).unwrap(),
        "signatureProvider":{"triggerCharacters":["("],"retriggerCharacters":[","]}})
    });
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "(", window, cx))
    });
    settle_until(cx, |_| pause.arrived() == 1);
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "a", window, cx))
    });
    settle_until(cx, |_| pause.arrived() == 2);
    pause.release();
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "(", window, cx))
    });
    settle_until(cx, |cx| count(&captured(&ws, cx, None), "requests", "textDocument/signatureHelp") == 3);
    let state = captured(&ws, cx, None);
    let requests: Vec<_> =
        state["requests"].as_array().unwrap().iter().filter(|r| r["method"] == "textDocument/signatureHelp").collect();
    assert_eq!(requests[1]["params"]["context"]["triggerKind"], 3);
    assert_eq!(requests[2]["params"]["context"]["isRetrigger"], true);
}

fn prefinal_restart_signature_case(cx: &mut TestAppContext, pending: bool) {
    let (ws, cx, source, _) = open(cx, &format!("orlo-explicit-restart-signature-{pending}"), "target");
    activate(cx);
    let pause = pending.then(|| crate::documents::save::gate::pause(&signature_gate_path(&source)));
    ws.update_in(cx, |ws, w, cx| ws.lsp_signature_help(&LspSignatureHelp, w, cx));
    if let Some(pause) = &pause {
        settle_until(cx, |_| pause.arrived() == 1);
    } else {
        settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    }
    let (manager, cancel, before) = ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        let e = editor.read(cx);
        (
            ws.language.manager.clone(),
            ws.language.signature_cancel.clone().unwrap(),
            (e.doc(cx).version, e.doc(cx).dirty, e.read_state(cx).selection_state()),
        )
    });
    let held = manager.lock().unwrap();
    ws.update_in(cx, |ws, w, cx| {
        let editor = ws.active_editor().unwrap();
        assert!(editor.read(cx).signature_allowed(w), "cancellation must not come from lost focus");
        assert!(!cancel.load(Ordering::Relaxed));
        ws.restart_language_servers(&RestartLanguageServers, w, cx);
        assert!(cancel.load(Ordering::Relaxed), "explicit restart must cancel before waiting for Manager");
        editor.update(cx, |e, cx| {
            e.validate_signature_context(w, cx);
            assert!(e.signature_allowed(w));
            assert!(e.signature_help.is_none() && e.signature_pending.is_none() && e.signature_inflight.is_none());
        });
    });
    drop(held);
    if let Some(pause) = pause {
        pause.release();
    }
    cx.run_until_parked();
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
    ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        let e = editor.read(cx);
        assert!(e.signature_help.is_none());
        assert_eq!((e.doc(cx).version, e.doc(cx).dirty), (before.0, before.1));
        assert!(e.read_state(cx).selection_state() == before.2);
    });
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn prefinal_explicit_restart_cancels_pending_signature_before_manager_lock(cx: &mut TestAppContext) {
    prefinal_restart_signature_case(cx, true);
}

#[gpui::test]
fn prefinal_explicit_restart_dismisses_focused_displayed_signature_before_manager_lock(cx: &mut TestAppContext) {
    prefinal_restart_signature_case(cx, false);
}

#[gpui::test]
fn signature_response_rejects_composition_version_and_server_restart(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in ["composition", "version", "server"] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-signature-guard-{mode}"), "target");
        activate(cx);
        let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
        ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
        settle_until(cx, |_| pause.arrived() == 1);
        ws.update_in(cx, |ws, window, cx| match mode {
            "server" => ws.invalidate_language_requests(cx),
            "composition" => ws
                .active_editor()
                .unwrap()
                .update(cx, |e, cx| e.replace_and_mark_text_in_range(None, "(", None, window, cx)),
            _ => {
                ws.with_active(cx, |e, cx| {
                    e.edit(cx, |s| {
                        s.insert("x");
                        s.set_cursor(4);
                    })
                });
            }
        });
        pause.release();
        cx.run_until_parked();
        assert!(!status(&ws, cx).contains("cancelled"));
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
    }
}

#[gpui::test]
fn typing_signature_trigger_never_waits_for_busy_manager(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, _, b) = open(cx, "orlo-signature-manager-busy", "target");
    rust_options(cx, |o| {
        *o = json!({"target":lsp::uri(&b).unwrap(),
        "signatureProvider":{"triggerCharacters":["("]}})
    });
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    let manager = ws.read_with(cx, |ws, _| ws.language.manager.clone());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _lock = manager.lock().unwrap();
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(std::time::Duration::from_secs(2));
    });
    ready_rx.recv().unwrap();
    let start = std::time::Instant::now();
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "(", window, cx))
    });
    let elapsed = start.elapsed();
    release_tx.send(()).ok();
    worker.join().unwrap();
    assert!(elapsed < std::time::Duration::from_secs(1), "typing waited for manager: {elapsed:?}");
}

#[gpui::test]
fn signature_selection_generation_rejects_virtual_and_away_back_moves(cx: &mut TestAppContext) {
    for mode in ["virtual", "away-back"] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-signature-selection-{mode}"), "target");
        activate(cx);
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.set_virtual_space(true, cx)));
        let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
        ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
        settle_until(cx, |_| pause.arrived() == 1);
        ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| {
                if mode == "virtual" {
                    e.sel(cx).reselect(vec![Selection::from_points((4, 2), (4, 2))]);
                } else {
                    e.sel(cx).set_cursor(3);
                    e.sel(cx).set_cursor(4);
                }
                cx.notify();
            });
        });
        pause.release();
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
    }
}

#[gpui::test]
fn superseded_signature_requests_queued_behind_manager_are_cancelled(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, _, b) = open(cx, "orlo-signature-queued", "target");
    rust_options(cx, |o| *o = json!({"target":lsp::uri(&b).unwrap(), "signatureProvider":{"triggerCharacters":["("]}}));
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    let manager = ws.read_with(cx, |ws, _| ws.language.manager.clone());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _lock = manager.lock().unwrap();
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(std::time::Duration::from_secs(2));
    });
    ready_rx.recv().unwrap();
    for text in ["(", "a", "b"] {
        ws.update_in(cx, |ws, window, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, text, window, cx))
        });
    }
    release_tx.send(()).ok();
    worker.join().unwrap();
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    assert_eq!(count(&captured(&ws, cx, None), "requests", "textDocument/signatureHelp"), 1);
}

#[gpui::test]
fn signature_invocation_does_not_add_selection_or_text_undo_steps(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-signature-history", "target");
    activate(cx);
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.sel(cx).sel_reset();
            e.sel(cx).set_cursor(1);
        });
    });
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            assert!(e.sel(cx).undo_selection());
            assert_eq!(e.state.cursor(), 4);
            assert!(!e.sel(cx).undo_selection());
            e.edit(cx, |s| s.insert("x"));
        });
    });
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "😀pri\n");
}

#[gpui::test]
fn cancelled_active_signature_frees_manager_and_late_reply_cannot_reappear(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-signature-active-cancel", "target");
    let entered = a.parent().unwrap().join("signature-entered");
    rust_options(cx, |o| *o = json!({"target":lsp::uri(&b).unwrap(), "signatureHoldFirst":entered}));
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    let cancel = ws.read_with(cx, |ws, _| ws.language.signature_cancel.clone().unwrap());
    let marker = entered.clone();
    let worker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(marker.exists());
        cancel.store(true, Ordering::Relaxed);
    });
    cx.run_until_parked();
    worker.join().unwrap();
    assert!(!status(&ws, cx).contains("cancelled"));
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        let help = &e.signature_help.as_ref().unwrap().help;
        assert_eq!(help.signatures[help.active].label, "f(😀, value: int)");
    });
    assert_eq!(count(&captured(&ws, cx, None), "notifications", "$/cancelRequest"), 1);
}

#[gpui::test]
fn escape_cancels_explicit_signature_while_response_is_pending(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-signature-pending-escape", "target");
    activate(cx);
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |_| pause.arrived() == 1);
    cx.simulate_keystrokes("escape");
    pause.release();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert!(e.signature_help.is_none());
        assert!(e.signature_pending.is_none());
    });
    assert!(!status(&ws, cx).contains("cancelled"));
}

#[gpui::test]
fn pending_signature_does_not_return_when_find_takes_editor_focus(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-signature-find-focus", "target");
    activate(cx);
    let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap().entity_id());
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |_| pause.arrived() == 1);
    cx.simulate_keystrokes("cmd-f");
    ws.read_with(cx, |ws, _| assert_eq!(ws.active_editor().unwrap().entity_id(), source));
    pause.release();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert!(e.signature_help.is_none());
        assert!(e.signature_pending.is_none());
    });
    assert!(!status(&ws, cx).contains("cancelled"));
}

#[gpui::test]
fn palette_signature_help_survives_its_focus_restore(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-palette-signature", "target");
    activate(cx);
    palette_command(&ws, cx, "LSP: Signature Help");
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("popup").is_some());
}

#[gpui::test]
fn nested_signature_returns_to_outer_then_exits_call(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, _, b) = open(cx, "orlo-signature-nested-exit", "target");
    rust_options(cx, |o| {
        *o = json!({"target":lsp::uri(&b).unwrap(), "signatureContextual":true,
        "signatureProvider":{"triggerCharacters":["("]}})
    });
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    let insert = |text: &str, cx: &mut VisualTestContext| {
        ws.update_in(cx, |ws, window, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, text, window, cx))
        });
    };
    let active = |cx: &mut VisualTestContext| {
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap().read(cx);
            e.signature_help.as_ref().map(|p| p.help.signatures[p.help.active].label.clone())
        })
    };
    insert("(", cx);
    settle_until(cx, |cx| active(cx).as_deref() == Some("outer()"));
    insert("(", cx);
    settle_until(cx, |cx| active(cx).as_deref() == Some("inner()"));
    insert(")", cx);
    settle_until(cx, |cx| active(cx).as_deref() == Some("outer()"));
    insert(")", cx);
    settle_until(cx, |cx| status(&ws, cx).contains("No signature help"));
    assert!(active(cx).is_none());
}

#[gpui::test]
fn virtual_caret_does_not_request_signature_for_physical_offset(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-signature-virtual-start", "target");
    activate(cx);
    ws.update(cx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.set_virtual_space(true, cx);
            e.sel(cx).reselect(vec![Selection::from_points((4, 2), (4, 2))]);
        })
    });
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_help.is_none()));
    assert!(status(&ws, cx).contains("beyond the line end"));
}

#[gpui::test]
fn queued_signatures_do_not_starve_unrelated_background_work(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, _, b) = open(cx, "orlo-signature-queue-yields", "target");
    rust_options(cx, |o| *o = json!({"target":lsp::uri(&b).unwrap(), "signatureProvider":{"triggerCharacters":["("]}}));
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    let manager = ws.read_with(cx, |ws, _| ws.language.manager.clone());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _lock = manager.lock().unwrap();
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(std::time::Duration::from_secs(2));
    });
    ready_rx.recv().unwrap();
    for text in std::iter::repeat_n("(", 50) {
        ws.update_in(cx, |ws, window, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, text, window, cx))
        });
    }
    let ran = Arc::new(AtomicBool::new(false));
    let probe = ran.clone();
    cx.update(|_, cx| {
        cx.background_spawn(async move {
            probe.store(true, Ordering::Relaxed);
        })
        .detach()
    });
    let start = std::time::Instant::now();
    cx.run_until_parked();
    let elapsed = start.elapsed();
    release_tx.send(()).ok();
    worker.join().unwrap();
    assert!(ran.load(Ordering::Relaxed));
    assert!(elapsed < std::time::Duration::from_secs(1), "background worker starved for {elapsed:?}");
}

#[test]
fn automatic_signature_sync_keeps_other_sessions_and_does_not_start_unrelated_servers() {
    let root = temp("orlo-signature-sync-scope").canonicalize().unwrap();
    let rust = document(&root, "a.rs", 1);
    let sibling = document(&root, "b.rs", 2);
    let python = document(&root, "c.py", 3);
    let go = document(&root, "d.go", 4);
    let mut initial = input(&root, vec![rust.clone(), sibling.clone(), python.clone()]);
    initial.config.get_mut("rust").unwrap().initialization_options["signatureProvider"] =
        json!({"triggerCharacters":["("]});
    let mut manager = Manager::default();
    manager.sync(&initial);
    let rust_key = server_key(&rust).unwrap();
    let python_key = server_key(&python).unwrap();
    let go_key = server_key(&go).unwrap();
    assert!(manager.sessions.contains_key(&rust_key));
    assert!(manager.sessions.contains_key(&python_key));
    assert!(!manager.sessions.contains_key(&go_key));
    let mut later = input(&root, vec![rust.clone(), sibling.clone(), python.clone(), go]);
    later.config = initial.config.clone();
    let request = request(&later);
    manager.request_with_cancel(&later, &request, "textDocument/signatureHelp", json!({
        "_signatureAuto":true,"_signatureTrigger":"(","context":{"triggerKind":2,"triggerCharacter":"(","isRetrigger":false}
    }), Some(&AtomicBool::new(false))).unwrap();
    assert!(manager.sessions.contains_key(&python_key));
    assert!(!manager.sessions.contains_key(&go_key));
    assert!(manager.sessions[&rust_key].as_ref().unwrap().documents.contains_key(&lsp::uri(&sibling.path).unwrap()));
}

#[test]
fn automatic_signature_waits_for_same_family_language_identity_restart() {
    let root = temp("orlo-signature-language-identity").canonicalize().unwrap();
    let mut input = input(&root, vec![document(&root, "source.c", 1)]);
    for config in input.config.values_mut() {
        config.initialization_options["signatureProvider"] = json!({"triggerCharacters":["("]});
    }
    let mut manager = Manager::default();
    manager.sync(&input);
    let before = state(&manager, &input.documents[0]);
    let uri = lsp::uri(&input.documents[0].path).unwrap();
    assert_eq!(before["documents"][&uri]["languageId"], "c");
    input.documents[0].syntax_override = Some("C++".into());
    input.serial += 1;
    let req = request(&input);
    let extra = json!({"_signatureAuto":true,"_signatureTrigger":"(","context":{"triggerKind":2,"triggerCharacter":"(","isRetrigger":false}});
    assert!(manager
        .request_with_cancel(&input, &req, "textDocument/signatureHelp", extra.clone(), Some(&AtomicBool::new(false)))
        .is_err());
    let still_old = state(&manager, &input.documents[0]);
    assert_eq!(still_old["pid"], before["pid"]);
    assert_eq!(still_old["documents"][&uri]["languageId"], "c");
    input.serial += 1;
    manager.sync(&input);
    let after = state(&manager, &input.documents[0]);
    assert_eq!(after["documents"][&uri]["languageId"], "cpp");
    assert_ne!(after["pid"], before["pid"]);
    input.serial += 1;
    let req = request(&input);
    assert!(manager
        .request_with_cancel(&input, &req, "textDocument/signatureHelp", extra, Some(&AtomicBool::new(false)))
        .is_ok());
}

#[gpui::test]
fn retrigger_only_comma_continues_pending_first_signature(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, a, b) = open(cx, "orlo-signature-pending-comma", "target");
    rust_options(
        cx,
        |o| *o = json!({"target":lsp::uri(&b).unwrap(), "signatureProvider":{"triggerCharacters":["("],"retriggerCharacters":[","]}}),
    );
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "(", window, cx))
    });
    settle_until(cx, |_| pause.arrived() == 1);
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, ",", window, cx))
    });
    settle_until(cx, |_| pause.arrived() == 2);
    pause.release();
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    let state = captured(&ws, cx, None);
    let requests: Vec<_> =
        state["requests"].as_array().unwrap().iter().filter(|r| r["method"] == "textDocument/signatureHelp").collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["params"]["context"]["triggerKind"], 3);
    assert_eq!(requests[1]["params"]["context"]["isRetrigger"], false);
    assert!(requests[1]["params"]["context"].get("triggerCharacter").is_none());
}

#[gpui::test]
fn pending_signature_mouse_selection_and_resize_cancel_and_clear_context(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in ["mouse", "selection", "resize"] {
        let (ws, cx, a, _) = open(cx, &format!("orlo-pending-context-{mode}"), "target");
        activate(cx);
        let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
        ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
        settle_until(cx, |_| pause.arrived() == 1);
        let token = ws.read_with(cx, |ws, _| ws.language.signature_cancel.clone().unwrap());
        match mode {
            "mouse" => {
                let point = ws
                    .read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).char_bounds(0, cx).unwrap().center());
                cx.simulate_mouse_down(point, gpui::MouseButton::Left, gpui::Modifiers::default());
                cx.simulate_mouse_up(point, gpui::MouseButton::Left, gpui::Modifiers::default());
            }
            "resize" => cx.simulate_resize(gpui::size(px(900.), px(500.))),
            _ => {
                ws.update(cx, |ws, cx| {
                    ws.with_active(cx, |e, cx| {
                        e.sel(cx).set_selection(0, 4);
                        cx.notify();
                    });
                });
            }
        }
        cx.run_until_parked();
        assert!(token.load(Ordering::Relaxed), "{mode} did not cancel pending work");
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_pending.is_none()));
        pause.release();
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "z", window, cx))
        });
        cx.run_until_parked();
        assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/signatureHelp"), 1);
    }
}

#[gpui::test]
fn pending_signature_context_exit_cancels_real_held_transport(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for mode in ["mouse", "selection", "resize"] {
        let (ws, cx, a, b) = open(cx, &format!("orlo-held-pending-{mode}"), "target");
        let marker = a.parent().unwrap().join("signature-held");
        rust_options(cx, |o| *o = json!({"target":lsp::uri(&b).unwrap(), "signatureHoldFirst":marker}));
        activate(cx);
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
        ws.update(cx, |ws, cx| ws.poll_language(cx));
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.language.busy));
        let (key, manager) = ws.read_with(cx, |ws, cx| {
            let (_, request) = ws.language_request(cx).unwrap();
            (server_key(&request.document).unwrap(), ws.language.manager.clone())
        });
        let token = Arc::new(AtomicBool::new(false));
        ws.update_in(cx, |ws, window, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.begin_signature(token.clone(), window, cx))
        });
        let thread_token = token.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut manager = manager.lock().unwrap();
            let session = manager.sessions.get_mut(&key).unwrap().as_mut().unwrap();
            let result = session.client.request_cancellable("textDocument/signatureHelp", json!({}), &thread_token);
            done_tx.send(result).ok();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(marker.exists(), "fixture did not receive request: {:?}", done_rx.try_recv().map(|r| r.err()));
        match mode {
            "mouse" => {
                let point = ws
                    .read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).char_bounds(0, cx).unwrap().center());
                cx.simulate_mouse_down(point, gpui::MouseButton::Left, gpui::Modifiers::default());
                cx.simulate_mouse_up(point, gpui::MouseButton::Left, gpui::Modifiers::default());
            }
            "resize" => cx.simulate_resize(gpui::size(px(900.), px(500.))),
            _ => {
                ws.update(cx, |ws, cx| {
                    ws.with_active(cx, |e, cx| {
                        e.sel(cx).set_selection(0, 4);
                        cx.notify();
                    });
                });
            }
        }
        cx.run_until_parked();
        assert!(done_rx.recv_timeout(std::time::Duration::from_secs(1)).expect("manager stayed blocked").is_err());
        worker.join().unwrap();
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_pending.is_none()));
        ws.update_in(cx, |ws, window, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "z", window, cx))
        });
        cx.run_until_parked();
        let state = captured(&ws, cx, Some(&a));
        assert_eq!(count(&state, "requests", "textDocument/signatureHelp"), 1);
        assert_eq!(count(&state, "notifications", "$/cancelRequest"), 1);
    }
}

#[gpui::test]
fn stale_signature_completion_cannot_clear_newer_pending_context(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-signature-pending-token", "target");
    activate(cx);
    let old = Arc::new(AtomicBool::new(false));
    let newer = Arc::new(AtomicBool::new(false));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| {
            e.begin_signature(old.clone(), window, cx);
            e.begin_signature(newer.clone(), window, cx);
            e.finish_signature(&old);
            assert!(e.signature_pending.is_some());
            assert!(Arc::ptr_eq(e.signature_inflight.as_ref().unwrap(), &newer));
            assert!(!newer.load(Ordering::Relaxed));
        });
    });
}

#[gpui::test]
fn signature_retrigger_shares_admitted_context_through_pending_input(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, a, b) = open(cx, "orlo-signature-context-shared", "target");
    rust_options(cx, |o| {
        *o = json!({"target":lsp::uri(&b).unwrap(), "signatureProvider":{"triggerCharacters":["("],"retriggerCharacters":[","]},
        "signatureResult":{"activeSignature":1,"signatures":[{"label":"f(a)"},{"label":"f(😀, 中)","parameters":[{"label":[2,4]},{"label":[6,7]}]}]}})
    });
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.0.contains(&"(".into()))
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "(", window, cx))
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    let raw = ws
        .read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.as_ref().unwrap().help.raw.clone());
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, ",", window, cx))
    });
    settle_until(cx, |_| pause.arrived() == 1);
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        let (pending, selected) = e.signature_pending.as_ref().unwrap().context.as_ref().unwrap();
        assert!(Arc::ptr_eq(&raw, pending));
        assert_eq!(*selected, 1);
        assert_eq!(pending["signatures"][1]["parameters"][1]["label"], json!([6, 7]));
    });
    pause.release();
}

#[gpui::test]
fn visible_signature_does_not_retrigger_after_same_frame_selection_change(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, a, _) = open(cx, "orlo-signature-visible-selection-input", "target");
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| {
            e.sel(cx).set_selection(0, 4);
            e.replace_text_in_range(None, "z", window, cx);
        });
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).signature_pending.is_none()));
    assert_eq!(pause.arrived(), 0);
    assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/signatureHelp"), 1);
    pause.release();
}

#[gpui::test]
fn repeated_explicit_signature_request_survives_old_popup_cancellation(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-signature-repeat-explicit", "target");
    activate(cx);
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |_| pause.arrived() == 1);
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert!(e.signature_pending.is_some());
        assert!(!e.signature_inflight.as_ref().unwrap().load(Ordering::Relaxed));
    });
    pause.release();
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    assert_eq!(count(&captured(&ws, cx, Some(&a)), "requests", "textDocument/signatureHelp"), 2);
}

#[gpui::test]
fn explicit_signature_at_new_selection_does_not_inherit_pending_call_context(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx, a, b) = open(cx, "orlo-signature-explicit-pending-origin", "target");
    rust_options(cx, |o| {
        *o = json!({"target":lsp::uri(&b).unwrap(),
        "signatureProvider":{"triggerCharacters":["("],"retriggerCharacters":[","]},
        "signatureResult":{"activeSignature":1,"signatures":[{"label":"alpha(a)"},{"label":"alpha(a, b)"}]}})
    });
    replace_text(&ws, cx, "alpha(a); beta(b)", 7);
    activate(cx);
    ws.update(cx, |ws, cx| ws.poll_language(cx));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_triggers.1.contains(&",".into()))
    });
    ws.update_in(cx, |ws, window, cx| ws.lsp_signature_help(&LspSignatureHelp, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    let pause = crate::documents::save::gate::pause(&signature_gate_path(&a));
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, ",", window, cx))
    });
    settle_until(cx, |_| pause.arrived() == 1);
    // No paint or input validation may separate the selection change from explicit capture.
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(17));
        ws.lsp_signature_help(&LspSignatureHelp, window, cx);
    });
    settle_until(cx, |_| pause.arrived() == 2);
    ws.update_in(cx, |ws, window, cx| {
        ws.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "z", window, cx))
    });
    settle_until(cx, |_| pause.arrived() == 3);
    pause.release();
    settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).signature_help.is_some()));
    let state = captured(&ws, cx, Some(&a));
    let requests: Vec<_> =
        state["requests"].as_array().unwrap().iter().filter(|r| r["method"] == "textDocument/signatureHelp").collect();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[1]["params"]["context"]["activeSignatureHelp"]["activeSignature"], 1);
    assert_eq!(requests[2]["params"]["context"]["triggerKind"], 1);
    let context = &requests[3]["params"]["context"];
    assert_eq!(context["triggerKind"], 3);
    assert_eq!(context["isRetrigger"], false);
    assert!(context.get("activeSignatureHelp").is_none());
    assert!(context.get("triggerCharacter").is_none());
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.doc(cx).rope.to_string(), "alpha(a,); beta(bz)");
        assert!(e.signature_pending.is_none());
    });
}

#[gpui::test]
fn virtual_space_fix1_server_completion_keeps_origin_through_real_resolve_fields_and_undo(cx: &mut TestAppContext) {
    assert_rectangular_server_completion(false, cx);
}

#[gpui::test]
fn virtual_space_fix1_server_snippet_keeps_origin_through_real_resolve_fields_and_undo(cx: &mut TestAppContext) {
    assert_rectangular_server_completion(true, cx);
}

fn assert_rectangular_server_completion(snippet: bool, cx: &mut TestAppContext) {
    use crate::config::settings::VirtualSpace;
    let (ws, cx, path, _) = open(cx, &format!("orlo-e05-fix1-completion-{snippet}"), "target\n");
    replace_text(&ws, cx, "😀pri", 4);
    cx.update(|_, cx| cx.global_mut::<Settings>().virtual_space = VirtualSpace::Rectangular);
    let body = if snippet { "${1:界}-$1-${2:é}$0" } else { "界X" };
    rust_options(cx, |o| {
        o["completionItems"] = json!([{"label":"print finish", "insertTextFormat":if snippet { 2 } else { 1 },
                "textEdit":{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},"newText":body},
                "data":{"resolve":true}}]);
        o["completionAdditional"] =
            json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"use;\n"}]);
    });
    activate(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::caret(4).in_rectangle()]));
        ws.lsp_completion(&LspCompletion, window, cx);
    });
    settle_until(cx, |cx| listed(&ws, cx).contains(&"print finish".into()));
    pick(&ws, cx, "print finish");
    let expanded = if snippet { "use;\n😀界-界-é" } else { "use;\n😀界X" };
    settle_until(cx, |cx| text(&ws, cx) == expanded);
    let all_rectangular = |cx: &mut VisualTestContext| {
        ws.read_with(cx, |ws, cx| {
            ws.active_editor().unwrap().read(cx).state.selections.iter().all(|sel| sel.rectangular)
        })
    };
    assert!(all_rectangular(cx));
    assert_eq!(selections(&ws, cx).len(), if snippet { 2 } else { 1 });
    if snippet {
        cx.simulate_keystrokes("tab");
        assert_eq!(selections(&ws, cx).len(), 1);
        assert!(all_rectangular(cx));
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(selections(&ws, cx).len(), 2);
        assert!(all_rectangular(cx));
        cx.simulate_keystrokes("tab tab");
    }
    cx.simulate_keystrokes("right");
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.state.selections.len(), 1);
        assert_eq!(e.state.selections[0].head_point(), (expanded.chars().count(), 1));
        for e in ws.open_editors(cx) {
            assert_eq!(e.read(cx).text(cx), expanded);
        }
    });
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, cx), "😀pri");
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).state.selections, [Selection::caret(4).in_rectangle()])
    });
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, cx), expanded);
    assert!(all_rectangular(cx));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "😀pri\n");
}

#[gpui::test]
fn n23_viewer_revalidates_every_language_edit_target_and_on_off_staleness(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-n23-language-batch", "target😀\n");
    ws.update_in(cx, |ws, w, cx| ws.open_path(b.clone(), w, cx));
    let second = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(cx, |ws, w, cx| ws.open_path(a.clone(), w, cx));
    let first = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let captured = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap().1);
    let batch = |request: &Request| {
        vec![(&a, "A"), (&b, "B")]
            .into_iter()
            .map(|(path, text)| PreparedEdit {
                path: path.clone(),
                before: request.documents.iter().find(|d| d.path == *path).cloned(),
                loaded: None,
                edits: vec![(0..1, text.into())],
            })
            .collect()
    };
    let before = (first.read_with(cx, |e, cx| e.text(cx)), second.read_with(cx, |e, cx| e.text(cx)));
    second.read_with(cx, |e, _| e.document.clone()).update(cx, |d, cx| d.set_viewer_mode(true, cx));
    ws.update_in(cx, |ws, w, cx| assert!(ws.apply_language_edits(batch(&captured), w, cx).is_err()));
    assert_eq!((first.read_with(cx, |e, cx| e.text(cx)), second.read_with(cx, |e, cx| e.text(cx))), before);
    // Fresh snapshots of an already protected target also refuse the entire batch.
    let protected = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap().1);
    ws.update_in(cx, |ws, w, cx| assert!(ws.apply_language_edits(batch(&protected), w, cx).is_err()));
    second.read_with(cx, |e, _| e.document.clone()).update(cx, |d, cx| d.set_viewer_mode(false, cx));
    ws.update_in(cx, |ws, w, cx| assert!(ws.apply_language_edits(batch(&captured), w, cx).is_err()));
    assert_eq!((first.read_with(cx, |e, cx| e.text(cx)), second.read_with(cx, |e, cx| e.text(cx))), before);
    let fresh = ws.read_with(cx, |ws, cx| ws.language_request(cx).unwrap().1);
    ws.update_in(cx, |ws, w, cx| ws.apply_language_edits(batch(&fresh), w, cx).unwrap());
    assert_ne!(first.read_with(cx, |e, cx| e.text(cx)), before.0);
    assert_ne!(second.read_with(cx, |e, cx| e.text(cx)), before.1);
    for e in [&first, &second] {
        e.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
    }
    assert_eq!((first.read_with(cx, |e, cx| e.text(cx)), second.read_with(cx, |e, cx| e.text(cx))), before);
}

#[gpui::test]
fn n23_viewer_explicit_save_keeps_exact_dirty_text_and_skips_format_cleanup(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-n23-save", "target\n");
    enable_format_on_save(cx);
    cx.update(|_, cx| {
        let settings = cx
            .global::<Settings>()
            .overlay(
                json!({"trim_trailing_white_space_on_save":"all","ensure_newline_at_eof_on_save":true})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        assert!(settings.format_on_save && settings.lsp_enabled);
        cx.set_global(settings);
    });
    ws.update_in(cx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("dirty😀  ");
            })
        });
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
    });
    let before = text(&ws, cx);
    let hold = hold_formatting(&a);
    save_document(&ws, cx);
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(hold.arrived(), 0, "protected save must not request formatting");
    hold.release();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), before);
    assert_eq!(text(&ws, cx), before);
    assert!(ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        let d = e.read(cx).doc(cx);
        d.viewer_mode() && !d.unsaved()
    }));
    ws.update_in(cx, |ws, w, cx| {
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
    });
    assert_eq!(text(&ws, cx), "😀pri\n");
    assert!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
}

#[gpui::test]
fn n23_viewer_on_off_cannot_revive_pending_format_save(cx: &mut TestAppContext) {
    let (ws, cx, a, _) = open(cx, "orlo-n23-stale-format", "target\n");
    enable_format_on_save(cx);
    let original = std::fs::read(&a).unwrap();
    type_in(&ws, cx, "dirty ");
    let before = text(&ws, cx);
    let paused = hold_formatting(&a);
    save_document(&ws, cx);
    settle_until(cx, |_| paused.arrived() == 1);
    ws.update_in(cx, |ws, w, cx| {
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
    });
    paused.release();
    settle_until(cx, |cx| !cx.update(|_, cx| crate::documents::Documents::saving(cx)));
    assert_eq!(text(&ws, cx), before);
    assert_eq!(std::fs::read(&a).unwrap(), original);
    assert!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
}

#[gpui::test]
fn inline_diagnostics_fixture_publication_settings_and_all_open_picker_keep_full_detail(cx: &mut TestAppContext) {
    let (ws, cx, a, b) = open(cx, "orlo-inline-diagnostics-picker", "😀other\n");
    cx.simulate_resize(gpui::size(px(400.), px(300.)));
    activate(cx);
    cx.update(|_, cx| {
        let mut settings = cx.global::<Settings>().clone();
        settings.inline_diagnostics = true;
        settings.inline_diagnostics_severity = crate::config::settings::DiagnosticSeverity::Error;
        cx.set_global(settings);
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_view(cx);
        ws.open_path(b.clone(), window, cx);
        ws.poll_language(cx);
    });
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.open_editors(cx).iter().filter(|e| e.read(cx).language_current(cx)).count() == 2)
    });
    let full = format!("{}\nN41 FULL MESSAGE ENDPOINT", "full detail é😀 ".repeat(120));
    let (manager, input) = ws.read_with(cx, |ws, cx| (ws.language.manager.clone(), ws.language_input(cx)));
    let doc = input.documents.iter().find(|d| d.path == a).unwrap();
    let uri = lsp::uri(&a).unwrap();
    let range = json!({"start":{"line":0,"character":2},"end":{"line":0,"character":5}});
    let reports = json!([{"range":range,"severity":1,"message":full},
        {"range":range,"severity":2,"message":"suppressed same-line warning"}]);
    {
        let mut manager = manager.lock().unwrap();
        let session = manager.sessions.get_mut(&server_key(doc).unwrap()).unwrap().as_mut().unwrap();
        session
            .client
            .request(
                "test/publishDiagnostics",
                json!({"uri":uri,"version":session.documents[&uri].serial,"diagnostics":reports}),
            )
            .unwrap();
        session.drain();
        assert_eq!(session.documents[&uri].diagnostics[0].message, full);
        session.client.request("test/publishDiagnostics", json!({"uri":uri,"version":0,"diagnostics":[]})).unwrap();
        session.drain();
        assert_eq!(session.documents[&uri].diagnostics.len(), 2, "late numeric report cannot erase current data");
        session.client.request("test/publishDiagnostics", json!({"uri":uri,"diagnostics":reports})).unwrap();
        session.drain();
        assert_eq!(session.documents[&uri].diagnostics.len(), 2, "existing versionless admission remains supported");
    }
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(a.clone(), window, cx);
        ws.poll_language(cx);
    });
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| {
            ws.active_editor().unwrap().read(cx).diagnostics.first().is_some_and(|d| d.message == full)
        })
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        let e = ws.active_editor().unwrap();
        let e = e.read(cx);
        assert!(e.view.inline_diagnostics);
        let drawn = e.painted_inline_diagnostics();
        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].ends_with('…'));
        assert!(!drawn[0].contains("suppressed"));
        ws.lsp_diagnostics(&LspDiagnostics, window, cx);
        let panel = ws.quick_panel.as_mut().unwrap();
        assert_eq!(panel.items.len(), 3, "every diagnostic from both open files remains available");
        assert!(panel.items.iter().any(|i| i.label.contains("suppressed same-line warning")));
        panel.selected = panel.filtered.iter().position(|i| panel.items[*i].label.contains(&full)).unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    let details = cx.debug_bounds("diagnostic-detail").expect("full diagnostics need a readable detail area");
    let text_bounds = cx.debug_bounds("diagnostic-detail-text").unwrap();
    assert!(text_bounds.size.height > details.size.height, "long message must require real scrolling");
    assert!(details.right() <= px(400.) && details.bottom() <= px(300.));
    let before = ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).state.selections.clone());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: details.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-100_000.))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let tail = cx.debug_bounds("diagnostic-detail-text").unwrap();
    assert!(tail.top() < text_bounds.top());
    assert!(
        tail.bottom() <= details.bottom() + px(1.) && tail.bottom() > details.top(),
        "the complete long-message endpoint must scroll into view"
    );
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert!(cx.debug_bounds("diagnostic-detail-text").unwrap().top() >= details.top());
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.quick_panel.as_ref().unwrap().diagnostic_detail().unwrap().1.ends_with("N41 FULL MESSAGE ENDPOINT"));
    });
    assert!(
        cx.debug_bounds("diagnostic-detail-text").unwrap().top() >= details.top(),
        "changing selection resets detail scroll"
    );
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: details.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-100_000.))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let selected_tail = cx.debug_bounds("diagnostic-detail-text").unwrap();
    assert!(
        selected_tail.bottom() <= details.bottom() + px(1.) && selected_tail.bottom() > details.top(),
        "the long-message endpoint remains readable after changing selection"
    );
    cx.simulate_input("suppressed");
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.filtered.len() == 1)));
    ws.read_with(cx, |ws, _| {
        assert!(ws
            .quick_panel
            .as_ref()
            .unwrap()
            .diagnostic_detail()
            .unwrap()
            .1
            .ends_with("suppressed same-line warning"));
    });
    ws.update_in(cx, |ws, window, cx| {
        let panel = ws.quick_panel.as_ref().unwrap();
        assert!(panel.input.read(cx).focus_handle(cx).is_focused(window), "query must retain native focus");
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(ws.read_with(cx, |ws, _| ws.quick_panel.is_none()), "Escape dismisses the diagnostic picker");
    ws.update_in(cx, |ws, window, cx| {
        assert!(ws.quick_panel.is_none());
        let e = ws.active_editor().unwrap();
        assert_eq!(e.read(cx).state.selections, before);
        assert!(e.read(cx).focus_handle(cx).is_focused(window));
        ws.lsp_diagnostics(&LspDiagnostics, window, cx);
        let panel = ws.quick_panel.as_mut().unwrap();
        panel.selected = panel.filtered.iter().position(|i| panel.items[*i].label.contains(&full)).unwrap();
        ws.confirm_quick_panel(window, cx);
    });
    settle_until(cx, |cx| showing(&ws, cx, &a));
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        assert_eq!(e.read(cx).state.selections, [Selection::new(1, 4)]);
        assert_eq!(e.read(cx).text(cx), "😀pri\n");
    });
    ws.update_in(cx, |ws, window, cx| ws.restart_language_servers(&RestartLanguageServers, window, cx));
    ws.read_with(cx, |ws, cx| assert!(!ws.active_editor().unwrap().read(cx).inline_diagnostics_current(cx)));
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| {
            ws.active_editor().unwrap().read(cx).diagnostics.first().is_some_and(|d| d.message == "fixture diagnostic")
        })
    });
}

#[gpui::test]
fn diagnostics_list_each_document_once_however_many_views_show_it(cx: &mut TestAppContext) {
    let (ws, cx, _, _) = open(cx, "orlo-lsp-ui-diagnostics-views", "😀pri\n");
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    // test windows run no poll timer: each settle step runs a pass
    settle_until(cx, |cx| {
        ws.update(cx, |ws, cx| {
            ws.poll_language(cx);
            let views: Vec<_> = ws.views.tabs().collect();
            views.len() == 2
                && (views.iter()).all(|e| e.read(cx).language_current(cx) && !e.read(cx).diagnostics.is_empty())
        })
    });
    ws.update_in(cx, |ws, window, cx| ws.lsp_diagnostics(&LspDiagnostics, window, cx));
    let rows = ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().map(|p| p.items.len()));
    assert_eq!(rows, Some(1));
}

#[test]
fn a_failed_sync_clears_on_the_next_clean_pass() {
    let root = temp("orlo-lsp-transient-sync-error").canonicalize().unwrap();
    let s = EditorState::from_text("😀pri\n");
    let mut input = input(&root, vec![now(&document(&root, "a.rs", 0), &s)]);
    let mut manager = Manager::default();
    let (_, status) = poll(&mut manager, &mut input, &s);
    assert_eq!(status, "LSP: rust");
    // as a pass leaves it when the write queue was full
    let key = server_key(&input.documents[0]).unwrap();
    manager.sessions.get_mut(&key).unwrap().as_mut().unwrap().sync_error =
        Some("Language server write queue: sending on a full channel".into());
    let (_, status) = poll(&mut manager, &mut input, &s);
    assert_eq!(status, "LSP: rust");
}

#[test]
fn documents_find_their_root_by_its_real_path_too() {
    let dir = temp("orlo-lsp-linked-root").canonicalize().unwrap();
    std::fs::create_dir_all(dir.join("real/src")).unwrap();
    std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
    let roots = resolved_roots(vec![dir.join("link")]);
    let file = dir.join("real/src/main.rs");
    assert_eq!(document_root(&roots, &file), Some(dir.join("real")));
    let roots = resolved_roots(vec![dir.join("real"), dir.join("real/src")]);
    assert_eq!(document_root(&roots, &file), Some(dir.join("real/src")));
}
