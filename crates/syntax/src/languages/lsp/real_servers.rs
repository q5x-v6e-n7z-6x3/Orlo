//! Opt-in integration tests against installed language servers; ordinary tests need no tools.
use super::*;
use std::fs;

struct Project(PathBuf);
impl Project {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("orlo-lsp-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn supported(client: &Client, name: &str) -> bool {
    client.capabilities.get(name).is_some_and(|v| !v.is_null() && v != false)
}

fn request_until(client: &Client, method: &str, params: Value, useful: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let result = client.request(method, params.clone());
        if let Ok(value) = &result {
            if useful(value) {
                return value.clone();
            }
        }
        assert!(Instant::now() < deadline, "{method} never returned useful data: {result:?}");
        thread::sleep(Duration::from_millis(200));
    }
}

fn diagnostics_until(client: &Client, document: &str, version: i64, errors: bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = Value::Null;
    loop {
        for event in client.drain_events() {
            match event {
                Event::Error(error) => panic!("{error}"),
                Event::Notification { method, params }
                    if method == "textDocument/publishDiagnostics"
                        && params["uri"] == document
                        && params["version"].as_i64().is_none_or(|v| v >= version) =>
                {
                    let values = params["diagnostics"].as_array().expect("diagnostic array");
                    let has_errors = values.iter().any(|d| d["severity"].as_u64().unwrap_or(1) == 1);
                    if has_errors == errors {
                        return params["diagnostics"].clone();
                    }
                    last = params;
                }
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "diagnostics errors={errors}, version={version} never arrived: {last}");
        thread::sleep(Duration::from_millis(50));
    }
}

fn change(client: &Client, file: &Path, document: &str, version: i64, text: &str) {
    fs::write(file, text).unwrap();
    client
        .notify(
            "textDocument/didChange",
            json!({
                "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
            }),
        )
        .unwrap();
    client.notify("textDocument/didSave", json!({"textDocument":{"uri":document},"text":text})).unwrap();
}

/// The completion item labelled `double…`, from a list or a `CompletionList`.
fn double_item(completion: &Value) -> Option<&Value> {
    let items = completion.as_array().or_else(|| completion["items"].as_array())?;
    (items.iter()).find(|item| item["label"].as_str().is_some_and(|s| s.starts_with("double")))
}

fn document(client: &Client, project: &Project, name: &str, language: &str, text: &str, invalid: &str) {
    let file = project.write(name, text);
    let document = uri(&file).unwrap();
    let rope = Rope::from_str(text);
    let coords = Coords::new(&rope, 1);
    let use_byte = text.rfind("double").unwrap();
    let cursor = text[..use_byte].chars().count() + 2;
    let point = position(coords, CharOffset(cursor), client.encoding);
    let params = json!({"textDocument":{"uri":document},"position":point});
    client
        .notify(
            "textDocument/didOpen",
            json!({
                "textDocument":{"uri":document,"languageId":language,"version":1,"text":text}
            }),
        )
        .unwrap();

    let hover = request_until(client, "textDocument/hover", params.clone(), |v| {
        crate::language::hover_text(v).contains("double")
    });
    if !hover["range"].is_null() {
        char_range(coords, &hover["range"], client.encoding).unwrap();
    }
    let completion = request_until(
        client,
        "textDocument/completion",
        json!({
            "textDocument":{"uri":document},"position":position(coords, CharOffset(cursor+1), client.encoding),
            "context":{"triggerKind":1}
        }),
        |v| double_item(v).is_some(),
    );
    let item = double_item(&completion).unwrap();
    assert!(!crate::language::completion_edits(coords, cursor + 1, item, client.encoding).unwrap().is_empty());
    let definition = request_until(client, "textDocument/definition", params.clone(), |v| {
        crate::language::locations(v).is_ok_and(|v| !v.is_empty())
    });
    let locations = crate::language::locations(&definition).unwrap();
    assert!(
        locations.iter().any(|(u, r)| u == &document && range(coords, *r, client.encoding).is_ok()),
        "{definition}"
    );

    let mut checks = vec!["hover", "completion", "definition"];
    if supported(client, "referencesProvider") {
        let refs = request_until(
            client,
            "textDocument/references",
            json!({
                "textDocument":{"uri":document},"position":point,"context":{"includeDeclaration":true}
            }),
            |v| v.as_array().is_some_and(|v| v.len() >= 2),
        );
        assert!(crate::language::locations(&refs).unwrap().len() >= 2);
        checks.push("references");
    }
    if supported(client, "renameProvider") {
        let renamed = client
            .request(
                "textDocument/rename",
                json!({
                    "textDocument":{"uri":document},"position":point,"newName":"triple"
                }),
            )
            .unwrap();
        let edits = workspace_edits(&renamed).unwrap();
        assert!(!edits.is_empty(), "rename returned no edits: {renamed}");
        for (u, _, edits) in edits {
            assert_eq!(u, document);
            let mut changed = rope.clone();
            for (range, text) in text_edits(coords, &edits, client.encoding).unwrap() {
                let start = range.start;
                changed.remove(range);
                changed.insert(start, &text);
            }
            assert_eq!(changed.to_string(), text.replace("double", "triple"));
        }
        checks.push("rename");
    }
    if supported(client, "documentFormattingProvider") {
        let edits = client
            .request(
                "textDocument/formatting",
                json!({
                    "textDocument":{"uri":document},"options":{"tabSize":4,"insertSpaces":true}
                }),
            )
            .unwrap();
        assert!(
            !text_edits(coords, &edits, client.encoding).unwrap().is_empty(),
            "formatting changed nothing: {edits}"
        );
        checks.push("formatting");
    }
    if supported(client, "semanticTokensProvider") && !client.capabilities["semanticTokensProvider"]["full"].is_null() {
        let tokens =
            client.request("textDocument/semanticTokens/full", json!({"textDocument":{"uri":document}})).unwrap();
        let spans = crate::language::semantic_tokens(
            coords,
            &tokens,
            &client.capabilities["semanticTokensProvider"]["legend"],
            client.encoding,
        )
        .unwrap();
        assert!(!spans.is_empty(), "advertised semantic tokens were empty: {tokens}");
        checks.push("semantic tokens");
    }

    client.drain_events();
    change(client, &file, &document, 2, invalid);
    let diagnostics = diagnostics_until(client, &document, 2, true);
    assert!(!crate::language::diagnostics(Coords::new(&Rope::from_str(invalid), 2), &diagnostics, client.encoding)
        .is_empty());
    change(client, &file, &document, 3, text);
    diagnostics_until(client, &document, 3, false);
    checks.push("diagnostics appear/clear");
    client.notify("textDocument/didClose", json!({"textDocument":{"uri":document}})).unwrap();
    eprintln!("{language}: {} ({:?})", checks.join(", "), client.encoding);
}

fn stop(mut client: Client) {
    assert!(client.request("shutdown", Value::Null).unwrap().is_null());
    client.notify("exit", Value::Null).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = client.process.as_mut().unwrap().child.try_wait().unwrap() {
            assert!(status.success(), "language server exit: {status}");
            break;
        }
        assert!(Instant::now() < deadline, "language server did not exit after shutdown");
        thread::sleep(Duration::from_millis(20));
    }
}

fn start(tools: &Path, project: &Project, name: &str) -> Client {
    let mut config = crate::language::default_servers().remove(name).unwrap();
    let executable = match name {
        "rust" => "bin/rust-analyzer",
        "go" => "bin/gopls",
        "typescript" => "npm/node_modules/.bin/typescript-language-server",
        "python" => "python/bin/pylsp",
        _ => unreachable!(),
    };
    config.command[0] = tools.join(executable).to_string_lossy().into_owned();
    Client::start(&config.command, &project.0, config.initialization_options).unwrap()
}

/// Set ORLO_LSP_TOOLS to a directory containing bin/{rust-analyzer,gopls},
/// npm/node_modules/.bin/typescript-language-server, python/bin/pylsp, and go/bin/go.
#[test]
#[ignore = "requires real language servers under ORLO_LSP_TOOLS or target/lsp-tools"]
fn real_language_servers_smoke() {
    let tools = std::env::var_os("ORLO_LSP_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lsp-tools"));
    let tools = fs::canonicalize(tools).expect("install test-local language servers first");
    let mut paths =
        vec![tools.join("bin"), tools.join("go/bin"), tools.join("npm/node_modules/.bin"), tools.join("python/bin")];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    std::env::set_var("GOPATH", tools.join("gopath"));
    std::env::set_var("GOCACHE", tools.join("go-cache"));
    std::env::set_var("GOTELEMETRY", "off");
    std::env::set_var("GOENV", "off");

    let rust = Project::new("rust");
    rust.write("Cargo.toml", "[package]\nname = \"orlo_lsp_smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    let rust_text = "pub fn double(value:i32)->i32{value*2}\npub fn answer()->i32{let _ = \"😀\";double(21)}\n";
    rust.write("src/lib.rs", rust_text);
    let client = start(&tools, &rust, "rust");
    document(&client, &rust, "src/lib.rs", "rust", rust_text, &format!("{rust_text}pub fn broken( {{\n"));
    stop(client);

    let go = Project::new("go");
    go.write("go.mod", "module example.com/orlo-lsp-smoke\n\ngo 1.25\n");
    let go_text =
        "package smoke\nfunc double(value int)int{return value*2}\nfunc Answer()int{_ = \"😀\";return double(21)}\n";
    go.write("main.go", go_text);
    let client = start(&tools, &go, "go");
    document(&client, &go, "main.go", "go", go_text, &format!("{go_text}func broken( {{\n"));
    stop(client);

    let ts = Project::new("typescript");
    ts.write("package.json", "{\"name\":\"orlo-lsp-smoke\",\"version\":\"1.0.0\",\"type\":\"module\"}\n");
    ts.write(
        "tsconfig.json",
        "{\"compilerOptions\":{\"allowJs\":true,\"checkJs\":true,\"strict\":true},\"include\":[\"*.js\",\"*.ts\"]}\n",
    );
    let client = start(&tools, &ts, "typescript");
    let js_text = "/** @param {number} value */\nexport function double(value){return value*2;}\nexport const answer=[\"😀\",double(21)];\n";
    document(&client, &ts, "main.js", "javascript", js_text, &format!("{js_text}function broken( {{\n"));
    let ts_text =
        "export function double(value:number):number{return value*2;}\nexport const answer=[\"😀\",double(21)];\n";
    document(&client, &ts, "main.ts", "typescript", ts_text, &format!("{ts_text}function broken( {{\n"));
    stop(client);

    let python = Project::new("python");
    python.write("pyproject.toml", "[project]\nname = \"orlo-lsp-smoke\"\nversion = \"0.1.0\"\n");
    let py_text = "def double(value):\n    return value*2\n\nanswer=(\"😀\",double(21))\n";
    let client = start(&tools, &python, "python");
    document(&client, &python, "main.py", "python", py_text, &format!("{py_text}def broken(\n"));
    stop(client);
}

/// Uses registered defaults on PATH; npm servers may be installed in a test-local prefix.
#[test]
#[ignore = "requires clangd, SourceKit-LSP, Bash Language Server and Intelephense"]
fn real_language_servers_smoke_added_families() {
    for (file, text) in [
        ("main.c", "int double_value(int value) { return value * 2; }\nint main(void) { return double_value(21); }\n"),
        ("main.cpp", "int double_value(int value) { return value * 2; }\nint main() { return double_value(21); }\n"),
        ("main.swift", "func doubleValue(_ value: Int) -> Int { value * 2 }\nlet answer = doubleValue(21)\n"),
        ("main.sh", "#!/bin/bash\ndouble_value() { echo $(($1 * 2)); }\ndouble_value 21\n"),
        ("main.php", "<?php\nfunction doubleValue($value) { return $value * 2; }\n$answer = doubleValue(21);\n"),
    ] {
        let project = Project::new(file);
        let path = project.write(file, text);
        let language = crate::language::Language::detect(&path).unwrap();
        let config = crate::language::default_servers().remove(language.server()).unwrap();
        let client = Client::start(&config.command, &project.0, config.initialization_options)
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let document = uri(&path).unwrap();
        client
            .notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {"uri": document, "languageId": language.id(), "version": 1, "text": text}
                }),
            )
            .unwrap();
        let rope = Rope::from_str(text);
        let coords = Coords::new(&rope, 1);
        let cursor = text[..text.rfind("double").unwrap()].chars().count() + 3;
        let params = json!({"textDocument": {"uri": document}, "position": position(coords, CharOffset(cursor), client.encoding)});
        assert!(supported(&client, "hoverProvider"), "{file}: hover advertised");
        assert!(supported(&client, "completionProvider"), "{file}: completion advertised");
        let hover = request_until(&client, "textDocument/hover", params.clone(), |v| {
            crate::language::hover_text(v).contains("double")
        });
        if !hover["range"].is_null() {
            char_range(coords, &hover["range"], client.encoding).unwrap();
        }
        request_until(&client, "textDocument/completion", params, |v| {
            v.as_array().or_else(|| v["items"].as_array()).is_some_and(|items| {
                items.iter().any(|item| {
                    item["filterText"]
                        .as_str()
                        .or_else(|| item["label"].as_str())
                        .is_some_and(|label| label.trim_start().starts_with("double"))
                })
            })
        });
        client.notify("textDocument/didClose", json!({"textDocument": {"uri": document}})).unwrap();
        eprintln!("{file}: {} hover and completion passed", language.server());
        stop(client);
    }
}
