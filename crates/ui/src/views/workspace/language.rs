//! Language server sessions in the background, and the editor and panel commands that use them.
#[cfg(test)]
#[path = "manifest_behaviour_tests.rs"]
mod manifest_behaviour_tests;
#[cfg(test)]
#[path = "language_tests.rs"]
mod tests;

use super::automation::FormatTarget;
use super::*;
use crate::config::automation;
use crate::languages::complete::{self, Candidate, Insert};
use crate::languages::language::{self, Diagnostic, SemanticSpan, ServerConfig, ServerLanguage};
use crate::languages::lsp::{self, Client, Encoding, Event};
use orlo_editor::text::{CharOffset, Coords, SelState};
use ropey::Rope;
use serde_json::{json, Value};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// How a document's language status begins while its server waits for the folder holding its
/// project to be trusted; the folder follows.
pub(crate) const LSP_WAITING: &str = "LSP: waiting for this folder to be trusted: ";

actions!(
    workspace,
    [
        LspCompletion,
        LspReferences,
        LspTypeDefinition,
        LspTypeDefinitionToSide,
        LspImplementation,
        LspImplementationToSide,
        LspHover,
        LspRename,
        LspFormat,
        LspFormatSelection,
        LspSignatureHelp,
        LspCodeActions,
        LspDiagnostics,
        RestartLanguageServers
    ]
);

#[derive(Clone)]
pub struct Document {
    path: PathBuf,
    registry: Arc<crate::languages::registry::Registry>,
    syntax_override: Option<String>,
    /// Derived file declaration: polling may sync an admitted route, never start one for it.
    modeline_language: bool,
    root: Option<PathBuf>,
    buffer: u64,
    version: u64,
    rope: Rope,
    disk: Option<crate::files::watch::Stamp>,
    read_only: bool,
}

impl Document {
    /// The text as this document has it, for converting positions taken from it.
    fn coords(&self) -> Coords<'_> {
        Coords::new(&self.rope, self.version)
    }

    fn language(&self) -> Option<ServerLanguage> {
        if let Some(name) = &self.syntax_override {
            return if self.modeline_language {
                ServerLanguage::declared(&self.registry, name)
            } else {
                ServerLanguage::named(&self.registry, name)
            };
        }
        let head = if self.registry.languages().is_empty() {
            String::new()
        } else {
            self.rope.slice(..self.rope.len_chars().min(crate::languages::registry::CONTENT_BYTES)).to_string()
        };
        ServerLanguage::detect(&self.registry, &self.path, &head)
    }

    fn matches(&self, e: &Editor, cx: &App) -> bool {
        self.matches_route(e, cx) && e.doc(cx).version == self.version
    }

    fn matches_route(&self, e: &Editor, cx: &App) -> bool {
        Arc::ptr_eq(&self.registry, &crate::languages::registry::registry())
            && self.syntax_override.as_deref() == e.language_choice()
            && self.modeline_language == e.modeline_language.is_some()
            && e.doc(cx).path.as_ref() == Some(&self.path)
            && e.doc(cx).id() == self.buffer
    }

    // Other views may render this shared buffer with a different syntax.
    fn matches_buffer(&self, e: &Editor, cx: &App) -> bool {
        e.doc(cx).path.as_ref() == Some(&self.path)
            && e.doc(cx).id() == self.buffer
            && e.doc(cx).version == self.version
    }
}

type ServerKey = (PathBuf, String);

struct OpenDocument {
    document: Document,
    /// The text the server last received, exactly, and the editor version it was: the next
    /// change is made from it, so edits between polls all arrive.
    sent: (Rope, u64),
    serial: i64,
    diagnostics: Vec<Diagnostic>,
    semantics: Vec<SemanticSpan>,
    semantic_version: Option<u64>,
}

struct Session {
    client: Client,
    documents: HashMap<String, OpenDocument>,
    error: Option<String>,
    /// A declaration retired a protocol identity. Keep siblings live until assistance admits
    /// the new route, then use the existing transport reset to discard unversioned diagnostics.
    retired_modelines: HashSet<String>,
    /// The last pass's failed sync, such as a full write queue: the next clean pass clears it.
    sync_error: Option<String>,
}

#[derive(Default)]
struct Manager {
    registry: Option<Arc<crate::languages::registry::Registry>>,
    sessions: HashMap<ServerKey, Result<Session, String>>,
    config: std::collections::BTreeMap<String, ServerConfig>,
    serial: u64,
}

#[derive(Default)]
struct Service {
    manager: Arc<Mutex<Manager>>,
    serial: AtomicU64,
    generation: Arc<AtomicU64>,
}
impl gpui::Global for Service {}

#[derive(Default)]
pub(crate) struct State {
    signature_cancel: Option<Arc<AtomicBool>>,
    navigation_cancel: Option<Arc<AtomicBool>>,
    manager: Arc<Mutex<Manager>>,
    timer: Option<Task<()>>,
    busy: bool,
    generation: u64,
    /// Bumped by each request and each panel or picker that opens: a navigation or
    /// hover reply for an older one is dropped. Edit requests carry none and keep their own
    /// checks.
    pub(crate) navigation_generation: u64,
    /// The language server's completion items the list offers, and the request they answer.
    completion: Option<(Request, Encoding, Vec<Value>)>,
    /// Advertised routes from the last admitted poll, invalidated with the shared service.
    range_formatting: Option<(u64, Vec<Document>)>,
    navigation: Option<(u64, Vec<(Document, NavigationCapabilities)>)>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NavigationCapabilities {
    pub(crate) type_definition: bool,
    pub(crate) implementation: bool,
}

impl State {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self { manager: cx.default_global::<Service>().manager.clone(), ..Default::default() }
    }

    pub(super) fn invalidate_caches(&mut self) {
        self.completion = None;
        self.range_formatting = None;
        self.navigation = None;
    }
}

struct Input {
    registry: Arc<crate::languages::registry::Registry>,
    serial: u64,
    documents: Vec<Document>,
    config: std::collections::BTreeMap<String, ServerConfig>,
    enabled: bool,
    active: Option<PathBuf>,
}

struct Decoration {
    document: Document,
    diagnostics: Vec<Diagnostic>,
    semantics: Vec<SemanticSpan>,
    status: String,
    signature_triggers: (Vec<String>, Vec<String>),
    range_formatting: bool,
    navigation: NavigationCapabilities,
}

impl Decoration {
    /// `document` with no results, only `status`.
    fn status(document: &Document, status: String) -> Self {
        Self {
            document: document.clone(),
            diagnostics: Vec::new(),
            semantics: Vec::new(),
            status,
            signature_triggers: Default::default(),
            range_formatting: false,
            navigation: NavigationCapabilities::default(),
        }
    }
}

fn signature_triggers(provider: &Value) -> (Vec<String>, Vec<String>) {
    let read = |key| {
        provider[key]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).filter(|s| s.chars().count() == 1).map(String::from).collect())
            .unwrap_or_default()
    };
    (read("triggerCharacters"), read("retriggerCharacters"))
}

fn root_for(document: &Document, language: &ServerLanguage) -> PathBuf {
    if let Some(root) = &document.root {
        return root.clone();
    }
    let parent = document.path.parent().unwrap_or(Path::new("/"));
    parent.ancestors().find(|p| language.is_project_root(p) || p.join(".git").exists()).unwrap_or(parent).to_path_buf()
}

/// Each root as spelled and as its real path: documents are opened by their real paths, while a
/// project's roots may name `/tmp` or a symlink.
fn resolved_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut all = Vec::with_capacity(roots.len() * 2);
    for root in roots {
        let real = crate::editing::text::resolve(&root);
        if real != root {
            all.push(real);
        }
        all.push(root);
    }
    all
}

/// The innermost root `path` is under.
fn document_root(roots: &[PathBuf], path: &Path) -> Option<PathBuf> {
    (roots.iter()).filter(|root| path.starts_with(root)).max_by_key(|root| root.components().count()).cloned()
}

fn server_key(document: &Document) -> Option<ServerKey> {
    let language = document.language()?;
    Some((root_for(document, &language), language.server))
}

impl Manager {
    fn sync(&mut self, input: &Input) -> Vec<Decoration> {
        self.sync_with_modelines(input, true)
    }

    fn sync_with_modelines(&mut self, input: &Input, start_modelines: bool) -> Vec<Decoration> {
        if input.serial < self.serial {
            return Vec::new();
        }
        self.serial = input.serial;
        let reloaded = self.registry.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &input.registry));
        if self.config != input.config || reloaded {
            // A new catalog generation invalidates request snapshots, not unchanged transports.
            // Retry failed startups on explicit reload, e.g. after installing a missing tool.
            self.sessions.retain(|(_, family), session| {
                self.config.get(family) == input.config.get(family) && (!reloaded || session.is_ok())
            });
            self.config = input.config.clone();
            self.registry = Some(input.registry.clone());
        }
        if !input.enabled {
            self.sessions.clear();
            return Vec::new();
        }
        if self.sessions.values().filter_map(|s| s.as_ref().ok()).any(|s| !s.retired_modelines.is_empty()) {
            let live: HashSet<_> = input.documents.iter().filter_map(|d| lsp::uri(&d.path).ok()).collect();
            for session in self.sessions.values_mut().filter_map(|s| s.as_mut().ok()) {
                session.retired_modelines.retain(|uri| live.contains(uri));
            }
        }
        let mut wanted: HashMap<ServerKey, Vec<&Document>> = HashMap::new();
        let mut out = Vec::new();
        for document in &input.documents {
            // very large documents keep tree/syntect editing; raise after measuring LSP payloads.
            if document.rope.len_bytes() > 8 * 1024 * 1024 {
                if document.language().is_some() {
                    out.push(Decoration::status(document, "LSP: file exceeds 8 MiB limit".into()));
                }
                continue;
            }
            let key = server_key(document);
            if !start_modelines && document.modeline_language {
                let Ok(uri) = lsp::uri(&document.path) else { continue };
                let admitted = key
                    .as_ref()
                    .and_then(|key| self.sessions.get(key))
                    .and_then(|s| s.as_ref().ok())
                    .and_then(|s| s.documents.get(&uri))
                    .is_some_and(|open| open.document.language() == document.language());
                if !admitted {
                    // Retain the old URI's identity across didClose, including a switch to
                    // a missing server, so later manual/explicit admission resets old diagnostics.
                    for session in self.sessions.values_mut().filter_map(|s| s.as_mut().ok()) {
                        if session.documents.contains_key(&uri) {
                            session.retired_modelines.insert(uri.clone());
                        }
                    }
                    continue;
                }
            }
            if let Some(key) = key {
                wanted.entry(key).or_default().push(document);
            }
        }
        self.sessions.retain(|key, _| wanted.contains_key(key));
        for (key, documents) in wanted {
            let config = input.config.get(&key.1).cloned().unwrap_or_default();
            if config.command.is_empty() {
                continue;
            }
            // Diagnostics have no opening identity (and may omit a version), so a
            // protocol-ID transition needs a fresh transport, not a URI close/reopen.
            let identity_changed = self.sessions.get(&key).and_then(|s| s.as_ref().ok()).is_some_and(|session| {
                documents.iter().any(|document| {
                    lsp::uri(&document.path).ok().is_some_and(|uri| {
                        session.retired_modelines.contains(&uri)
                            || session
                                .documents
                                .get(&uri)
                                .is_some_and(|open| open.document.language() != document.language())
                    })
                })
            });
            if identity_changed {
                self.sessions.remove(&key);
            }
            // a server may build the project it serves (rust-analyzer runs build.rs and proc
            // macros, Gradle's runs build scripts): it starts once the user trusts the folder
            if !self.sessions.contains_key(&key) && !orlo_utils::trust::trusted(&key.0) {
                for document in documents {
                    out.push(Decoration::status(document, format!("{LSP_WAITING}{}", key.0.display())));
                }
                continue;
            }
            let result = self.sessions.entry(key.clone()).or_insert_with(|| {
                Client::start(&config.command, &key.0, config.initialization_options.clone()).map(|client| {
                    let (documents, retired_modelines) = (HashMap::new(), HashSet::new());
                    Session { client, documents, error: None, retired_modelines, sync_error: None }
                })
            });
            let session = match result {
                Ok(session) => session,
                Err(error) => {
                    for document in documents {
                        out.push(Decoration::status(document, format!("LSP: {error}")));
                    }
                    continue;
                }
            };
            session.drain();
            let uris: HashSet<_> = documents.iter().filter_map(|d| lsp::uri(&d.path).ok()).collect();
            session.documents.retain(|uri, _| {
                if uris.contains(uri) {
                    return true;
                }
                let _ = session.client.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
                false
            });
            session.sync_error = None;
            for document in documents {
                if let Err(error) = session.sync(document) {
                    session.sync_error = Some(error);
                }
            }
            if let Some(path) = &input.active {
                if let Ok(uri) = lsp::uri(path) {
                    session.semantic(&uri);
                }
            }
            session.drain();
            for open in session.documents.values() {
                out.push(Decoration {
                    document: open.document.clone(),
                    diagnostics: open.diagnostics.clone(),
                    semantics: open.semantics.clone(),
                    status: (session.error.as_ref())
                        .or(session.sync_error.as_ref())
                        .map_or_else(|| format!("LSP: {}", key.1), |e| format!("LSP: {e}")),
                    signature_triggers: signature_triggers(&session.client.capabilities["signatureHelpProvider"]),
                    range_formatting: !session.client.capabilities["documentRangeFormattingProvider"].is_null()
                        && session.client.capabilities["documentRangeFormattingProvider"] != false,
                    navigation: NavigationCapabilities {
                        type_definition: !session.client.capabilities["typeDefinitionProvider"].is_null()
                            && session.client.capabilities["typeDefinitionProvider"] != false,
                        implementation: !session.client.capabilities["implementationProvider"].is_null()
                            && session.client.capabilities["implementationProvider"] != false,
                    },
                });
            }
        }
        out
    }

    fn request(&mut self, input: &Input, request: &Request, method: &str, extra: Value) -> Result<Response, String> {
        self.request_with_cancel(input, request, method, extra, None)
    }

    fn request_with_cancel(
        &mut self,
        input: &Input,
        request: &Request,
        method: &str,
        extra: Value,
        cancel: Option<&AtomicBool>,
    ) -> Result<Response, String> {
        self.request_limited(input, request, method, extra, cancel.map(|c| (c, None)))
    }

    fn request_limited(
        &mut self,
        input: &Input,
        request: &Request,
        method: &str,
        extra: Value,
        limit: Option<(&AtomicBool, Option<std::time::Instant>)>,
    ) -> Result<Response, String> {
        if request.epoch.load(Ordering::Relaxed) != request.service_generation {
            return Err("Language server restarted; request the action again".into());
        }
        let ranges = if method == "textDocument/rangeFormatting" {
            let mut ranges = request.format_selection.as_ref().map_or_else(
                || vec![request.selection.clone()],
                |origin| origin.state.selections.iter().map(|s| s.range()).filter(|r| !r.is_empty()).collect(),
            );
            if ranges.is_empty() || ranges.iter().any(|r| r.is_empty() || r.end > request.document.rope.len_chars()) {
                return Err("Select a range to format".into());
            }
            ranges.sort_by_key(|r| r.start);
            if ranges.windows(2).any(|p| p[0].end > p[1].start) {
                return Err("Overlapping formatting selections".into());
            }
            Some(ranges)
        } else {
            None
        };
        if !Arc::ptr_eq(&input.registry, &crate::languages::registry::registry())
            || !Arc::ptr_eq(&input.registry, &request.document.registry)
        {
            return Err("Language definitions changed; request the action again".into());
        }
        let key = server_key(&request.document).ok_or("Unsupported language")?;
        if extra.get("_signatureAuto").is_some() {
            if !input.enabled
                || self.config != input.config
                || self.registry.as_ref().is_none_or(|r| !Arc::ptr_eq(r, &input.registry))
            {
                return Err("Language server configuration changed".into());
            }
            let provider = self
                .sessions
                .get(&key)
                .and_then(|s| s.as_ref().ok())
                .map(|s| &s.client.capabilities["signatureHelpProvider"])
                .ok_or("Language server unavailable")?;
            let (triggers, retriggers) = signature_triggers(provider);
            let active = extra["context"]["isRetrigger"] == true;
            if let Some(trigger) = extra.get("_signatureTrigger").and_then(Value::as_str) {
                if !triggers.iter().any(|s| s == trigger) && !(active && retriggers.iter().any(|s| s == trigger)) {
                    return Err("Signature trigger no longer supported".into());
                }
            }
        }
        if extra.get("_signatureAuto").is_some() {
            if input.serial < self.serial {
                return Err("Language input superseded".into());
            }
            let session =
                self.sessions.get_mut(&key).ok_or("Language server is disabled")?.as_mut().map_err(|e| e.clone())?;
            let uri = lsp::uri(&request.document.path)?;
            if session.documents.get(&uri).is_some_and(|open| open.document.language() != request.document.language()) {
                return Err("Language identity changed; waiting for synchronization".into());
            }
            self.serial = input.serial;
            session.drain();
            session.sync(&request.document)?;
            session.drain();
        } else {
            self.sync(input);
        }
        let session =
            self.sessions.get_mut(&key).ok_or("Language server is disabled")?.as_mut().map_err(|e| e.clone())?;
        let capability = match method {
            "textDocument/completion" | "completionItem/resolve" => "completionProvider",
            "textDocument/definition" => "definitionProvider",
            "textDocument/typeDefinition" => "typeDefinitionProvider",
            "textDocument/implementation" => "implementationProvider",
            "textDocument/references" => "referencesProvider",
            "textDocument/hover" => "hoverProvider",
            "textDocument/signatureHelp" => "signatureHelpProvider",
            "textDocument/rename" => "renameProvider",
            "textDocument/formatting" => "documentFormattingProvider",
            "textDocument/rangeFormatting" => "documentRangeFormattingProvider",
            "textDocument/codeAction" | "codeAction/resolve" => "codeActionProvider",
            "workspace/executeCommand" => "executeCommandProvider",
            _ => return Err("Unsupported language request".into()),
        };
        let supported = &session.client.capabilities[capability];
        if supported.is_null() || supported == false {
            return Err(format!("Server does not support {method}"));
        }
        let uri = lsp::uri(&request.document.path)?;
        let open = session.documents.get(&uri).ok_or("Document is not synchronized")?;
        if open.document.version != request.document.version
            || open.document.buffer != request.document.buffer
            || open.document.language() != request.document.language()
        {
            return Err("Document changed; request the language feature again".into());
        }
        let encoding = session.client.encoding;
        let mut params = if method.starts_with("workspace/") || method.ends_with("/resolve") {
            extra
        } else {
            let text = request.document.coords();
            let mut params = json!({"textDocument":{"uri":uri},
                "position":lsp::position(text,CharOffset(request.cursor),encoding)});
            if let Some(fields) = extra.as_object() {
                for (key, value) in fields {
                    if key.starts_with("_signature") {
                        continue;
                    }
                    params[key] = value.clone();
                }
            }
            if method == "textDocument/rangeFormatting" {
                params["range"] = json!(lsp::span(
                    text,
                    CharOffset(request.selection.start)..CharOffset(request.selection.end),
                    encoding
                ));
                params.as_object_mut().unwrap().remove("position");
            }
            if method == "textDocument/codeAction" {
                let selection = &request.selection;
                params["range"] =
                    json!(lsp::span(text, CharOffset(selection.start)..CharOffset(selection.end), encoding));
                let diagnostics: Vec<_> = open
                    .diagnostics
                    .iter()
                    .filter(|d| d.range.start <= request.selection.end && d.range.end >= request.selection.start)
                    .map(|d| d.raw.clone())
                    .collect();
                params["context"] = json!({"diagnostics":diagnostics,"triggerKind":1});
            }
            params
        };
        let ask = |params| match limit {
            Some((cancel, Some(deadline))) => session.client.request_cancellable_for(
                method,
                params,
                cancel,
                deadline.saturating_duration_since(std::time::Instant::now()),
            ),
            Some((cancel, None)) => session.client.request_cancellable(method, params, cancel),
            None => session.client.request(method, params),
        };
        let value = if let Some(ranges) = ranges {
            let mut batch = Vec::new();
            let mut occupied: Vec<Range<usize>> = Vec::new();
            for range in ranges {
                if request.epoch.load(Ordering::Relaxed) != request.service_generation {
                    return Err("Language server restarted; request the action again".into());
                }
                params["range"] = json!(lsp::span(
                    request.document.coords(),
                    CharOffset(range.start)..CharOffset(range.end),
                    encoding
                ));
                let value = ask(params.clone())?;
                if value.is_null() {
                    continue;
                }
                // Every reply describes the original rope; never sync or apply between ranges.
                let edits = lsp::text_edits(request.document.coords(), &value, encoding)?;
                if edits.iter().any(|(r, _)| {
                    occupied.iter().any(|old| r.start == old.start || (r.start < old.end && old.start < r.end))
                }) {
                    return Err("Overlapping LSP edits across formatting ranges".into());
                }
                occupied.extend(edits.into_iter().map(|(r, _)| r));
                batch.extend(value.as_array().unwrap().iter().cloned());
            }
            Value::Array(batch)
        } else {
            ask(params)?
        };
        Ok(Response { value, encoding, versions: session.versions(), root: key.0, command: None })
    }

    /// Whether the server of `document` resolves the items of `provider`.
    fn can_resolve(&self, document: &Document, provider: &str) -> bool {
        server_key(document)
            .and_then(|key| self.sessions.get(&key))
            .and_then(|session| session.as_ref().ok())
            .is_some_and(|session| session.client.capabilities[provider]["resolveProvider"] == true)
    }
}

impl Session {
    /// The buffer, editor version and server version of each open document, by URI.
    fn versions(&self) -> HashMap<String, (u64, u64, i64)> {
        (self.documents.iter())
            .map(|(uri, o)| {
                let d = &o.document;
                (uri.clone(), (d.buffer, d.version, o.serial))
            })
            .collect()
    }

    fn sync(&mut self, document: &Document) -> Result<(), String> {
        let uri = lsp::uri(&document.path)?;
        let language = document.language().ok_or("Unsupported language")?;
        if self.documents.get(&uri).is_some_and(|o| o.document.buffer != document.buffer) {
            self.client.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}))?;
            self.documents.remove(&uri);
        }
        if let Some(open) = self.documents.get_mut(&uri) {
            let sync = &self.client.capabilities["textDocumentSync"];
            let kind = sync.as_u64().or_else(|| sync["change"].as_u64()).unwrap_or(0);
            // a server that takes no changes (kind 0) still hears of opens and saves
            if open.document.version != document.version && kind != 0 {
                let sent = Coords::new(&open.sent.0, open.sent.1);
                let changes = lsp::content_changes(sent, document.coords(), kind == 2, self.client.encoding);
                if !changes.is_empty() {
                    // one version for the notification: the text after all its changes
                    let serial = open.serial + 1;
                    self.client.notify(
                        "textDocument/didChange",
                        json!({"textDocument":{"uri":uri,"version":serial},
                        "contentChanges":changes}),
                    )?;
                    open.serial = serial;
                    open.sent = (document.rope.clone(), document.version);
                    open.diagnostics.clear();
                    open.semantics.clear();
                    open.semantic_version = None;
                }
            }
            // the text is the server's now: what its results describe, even if the save
            // notification below fails and is sent again next time
            let saved = open.document.disk;
            open.document = Document { disk: saved, ..document.clone() };
            if saved != document.disk {
                let save = &self.client.capabilities["textDocumentSync"]["save"];
                if !save.is_null() && save != false {
                    let mut params = json!({"textDocument":{"uri":uri}});
                    if save["includeText"] == true {
                        params["text"] = document.rope.to_string().into();
                    }
                    self.client.notify("textDocument/didSave", params)?;
                }
                open.document.disk = document.disk;
            }
        } else {
            self.client.notify(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":uri,"languageId":language.id,
                "version":1,"text":document.rope.to_string()}}),
            )?;
            self.documents.insert(
                uri,
                OpenDocument {
                    document: document.clone(),
                    sent: (document.rope.clone(), document.version),
                    serial: 1,
                    diagnostics: Vec::new(),
                    semantics: Vec::new(),
                    semantic_version: None,
                },
            );
        }
        Ok(())
    }

    fn drain(&mut self) {
        for event in self.client.drain_events() {
            match event {
                Event::Notification { method, params } if method == "textDocument/publishDiagnostics" => {
                    let Some(uri) = params["uri"].as_str() else {
                        continue;
                    };
                    let Some(open) = self.documents.get_mut(uri) else {
                        continue;
                    };
                    if params["version"].as_i64().is_some_and(|v| v != open.serial) {
                        continue;
                    }
                    open.diagnostics =
                        language::diagnostics(open.document.coords(), &params["diagnostics"], self.client.encoding);
                }
                Event::Error(error) => self.error = Some(error),
                _ => {}
            }
        }
    }

    fn semantic(&mut self, uri: &str) {
        let provider = &self.client.capabilities["semanticTokensProvider"];
        if provider["full"].is_null() || provider["full"] == false {
            return;
        }
        let Some(open) = self.documents.get_mut(uri) else {
            return;
        };
        if open.semantic_version == Some(open.document.version) {
            return;
        }
        if let Ok(value) = self.client.request("textDocument/semanticTokens/full", json!({"textDocument":{"uri":uri}}))
        {
            open.semantic_version = Some(open.document.version);
            open.semantics =
                language::semantic_tokens(open.document.coords(), &value, &provider["legend"], self.client.encoding)
                    .unwrap_or_default();
        }
    }
}

#[derive(Clone)]
pub struct Request {
    document: Document,
    documents: Vec<Document>,
    cursor: usize,
    selection: Range<usize>,
    format_selection: Option<FormatSelection>,
    generation: (u64, u64),
    navigation_generation: Option<u64>,
    navigation_source: Option<(EntityId, u64, SelState)>,
    epoch: Arc<AtomicU64>,
    service_generation: u64,
    started: std::time::SystemTime,
}

#[derive(Clone)]
struct FormatSelection {
    editor: EntityId,
    generation: u64,
    state: SelState,
}

struct Response {
    value: Value,
    encoding: Encoding,
    versions: HashMap<String, (u64, u64, i64)>,
    root: PathBuf,
    command: Option<Value>,
}

impl Response {
    fn has_edits(&self) -> bool {
        self.value.get("changes").is_some() || self.value.get("documentChanges").is_some()
    }
}

#[derive(Clone)]
pub enum Choice {
    Location { uri: String, range: lsp::Span, encoding: Encoding, side: bool },
    CodeAction { request: Arc<Request>, action: Value },
}

#[derive(Clone, Copy)]
enum Feature {
    Definition,
    DefinitionToSide,
    References,
    TypeDefinition,
    TypeDefinitionToSide,
    Implementation,
    ImplementationToSide,
    Hover,
    SignatureHelp,
    Rename,
    Format,
    RangeFormat,
    CodeActions,
}

impl Feature {
    fn method(self) -> &'static str {
        match self {
            Self::Definition | Self::DefinitionToSide => "textDocument/definition",
            Self::References => "textDocument/references",
            Self::TypeDefinition | Self::TypeDefinitionToSide => "textDocument/typeDefinition",
            Self::Implementation | Self::ImplementationToSide => "textDocument/implementation",
            Self::Hover => "textDocument/hover",
            Self::SignatureHelp => "textDocument/signatureHelp",
            Self::Rename => "textDocument/rename",
            Self::Format => "textDocument/formatting",
            Self::RangeFormat => "textDocument/rangeFormatting",
            Self::CodeActions => "textDocument/codeAction",
        }
    }

    fn is_destination(self) -> bool {
        matches!(
            self,
            Self::TypeDefinition | Self::TypeDefinitionToSide | Self::Implementation | Self::ImplementationToSide
        )
    }

    fn to_side(self) -> bool {
        matches!(self, Self::DefinitionToSide | Self::TypeDefinitionToSide | Self::ImplementationToSide)
    }
}

impl Workspace {
    pub(crate) fn invalidate_navigation(&mut self) {
        if let Some(cancel) = self.language.navigation_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(cancel) = self.language.signature_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.spelling_job = None;
        self.language.navigation_generation += 1;
    }

    pub(crate) fn invalidate_language_requests(&self, cx: &App) {
        cx.global::<Service>().generation.fetch_add(1, Ordering::Relaxed);
    }
    fn execute_language_command(&mut self, command: Value, window: &mut Window, cx: &mut Context<Self>) {
        let (input, request) = match self.language_request(cx) {
            Ok(r) => r,
            Err(e) => return self.status_message(e, cx),
        };
        self.run_language(
            (input, request),
            "workspace/executeCommand",
            command,
            false,
            None,
            window,
            cx,
            |ws, response, request, window, cx| match response {
                Some(response) if response.has_edits() => ws.stage_language_edits(request, response, window, cx),
                Some(_) => ws.status_message("Language server command completed", cx),
                None => {}
            },
        );
    }

    fn language_input(&self, cx: &App) -> Input {
        self.language_input_for(None, cx)
    }

    fn language_input_for(&self, source: Option<&Entity<Editor>>, cx: &App) -> Input {
        let registry = crate::languages::registry::registry();
        let mut roots: Vec<_> = self.roots.iter().map(|r| r.path.clone()).collect();
        for window in self.other_workspace_handles(cx) {
            if let Ok(ws) = window.read(cx) {
                roots.extend(ws.roots.iter().map(|r| r.path.clone()));
            }
        }
        let roots = resolved_roots(roots);
        let mut seen = HashSet::new();
        let mut editors = self.open_editors(cx);
        // Polls in different windows must select the same view of each shared buffer.
        // Explicit requests/save preparation keep their source ahead of that stable order.
        editors.sort_by_key(|e| (Some(e) != source, e.entity_id()));
        let documents = editors
            .into_iter()
            .filter_map(|editor| {
                let e = editor.read(cx);
                let path = e.doc(cx).path.clone()?;
                if e.results || e.doc(cx).scratch || !seen.insert(e.doc(cx).id()) {
                    return None;
                }
                let root = document_root(&roots, &path);
                Some(Document {
                    registry: registry.clone(),
                    syntax_override: e.language_choice().map(String::from),
                    modeline_language: e.modeline_language.is_some(),
                    path,
                    root,
                    buffer: e.doc(cx).id(),
                    version: e.doc(cx).version,
                    rope: e.doc(cx).rope.clone(),
                    disk: e.doc(cx).disk,
                    read_only: e.doc(cx).read_only || e.doc(cx).edits_blocked(),
                })
            })
            .collect();
        let settings = cx.global::<Settings>();
        let mut config = language::runtime_servers(&registry);
        config.extend(settings.language_servers.clone());
        Input {
            registry,
            serial: cx.global::<Service>().serial.fetch_add(1, Ordering::Relaxed) + 1,
            documents,
            config,
            enabled: settings.lsp_enabled,
            active: self.active_editor().and_then(|e| e.read(cx).doc(cx).path.clone()),
        }
    }

    pub(crate) fn start_language_polling(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.language.timer = Some(cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
            if this.update_in(cx, |ws, _, cx| ws.poll_language(cx)).is_err() {
                break;
            }
        }));
    }

    fn poll_language(&mut self, cx: &mut Context<Self>) {
        if self.language.busy {
            return;
        }
        let input = self.language_input(cx);
        self.language.busy = true;
        let generation = (self.restore_gen, self.language.generation);
        let epoch = cx.global::<Service>().generation.clone();
        let service_generation = epoch.load(Ordering::Relaxed);
        let manager = self.language.manager.clone();
        let task = cx.background_spawn(async move {
            let mut manager = manager.lock().unwrap_or_else(|e| e.into_inner());
            (input.serial >= manager.serial).then(|| manager.sync_with_modelines(&input, false))
        });
        cx.spawn(async move |this, cx| {
            let decorations = task.await;
            this.update(cx, |ws, cx| {
                if generation.1 == ws.language.generation {
                    ws.language.busy = false;
                }
                if generation != (ws.restore_gen, ws.language.generation) {
                    return;
                }
                if epoch.load(Ordering::Relaxed) != service_generation {
                    return;
                }
                let Some(decorations) = decorations else {
                    return;
                };
                ws.language.range_formatting = Some((
                    service_generation,
                    decorations.iter().filter(|d| d.range_formatting).map(|d| d.document.clone()).collect(),
                ));
                ws.language.navigation = Some((
                    service_generation,
                    decorations.iter().map(|d| (d.document.clone(), d.navigation)).collect(),
                ));
                if cx.active_window() == Some(ws.window)
                    && (ws.menu_range_formatting != ws.has_range_formatting(cx)
                        || ws.menu_language_navigation != ws.navigation_capabilities(cx))
                {
                    ws.refresh_menus(cx);
                }
                let editors = ws.open_editors(cx);
                for editor in editors {
                    let decoration = decorations.iter().find(|d| d.document.matches(editor.read(cx), cx));
                    editor.update(cx, |e, cx| {
                        if let Some(d) = decoration {
                            let key = Some((d.document.path.clone(), d.document.version));
                            if e.language_version != key
                                || e.diagnostics != d.diagnostics
                                || !e.inline_diagnostics_current(cx)
                                || e.semantic_spans != d.semantics
                                || e.lsp_status != d.status
                                || e.signature_triggers != d.signature_triggers
                            {
                                e.language_version = key;
                                e.install_diagnostics(
                                    d.diagnostics.clone(),
                                    Some((epoch.clone(), service_generation)),
                                    cx,
                                );
                                e.semantic_spans = d.semantics.clone();
                                e.lsp_status = d.status.clone();
                                e.signature_triggers = d.signature_triggers.clone();
                                cx.notify();
                            }
                        } else if e.language_version.take().is_some() {
                            e.install_diagnostics(Vec::new(), None, cx);
                            e.semantic_spans.clear();
                            e.lsp_status.clear();
                            e.signature_triggers = Default::default();
                            cx.notify();
                        }
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn restart_language_servers(
        &mut self,
        _: &RestartLanguageServers,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_navigation();
        self.invalidate_language_requests(cx);
        self.refresh_menus(cx);
        self.language.generation += 1;
        self.language.busy = true;
        let manager = self.language.manager.clone();
        let task = cx.background_spawn(async move {
            manager.lock().unwrap_or_else(|e| e.into_inner()).sessions.clear();
        });
        cx.spawn(async move |this, cx| {
            task.await;
            this.update(cx, |ws, cx| {
                ws.language.busy = false;
                ws.poll_language(cx);
            })
            .ok();
        })
        .detach();
        self.status_message("Restarting language servers…", cx);
    }

    pub(super) fn has_language_server(&self, cx: &App) -> bool {
        self.served_language_request(cx).is_ok()
    }

    pub(super) fn has_range_formatting(&self, cx: &App) -> bool {
        cx.global::<Settings>().lsp_enabled
            && self.language.range_formatting.as_ref().is_some_and(|(generation, documents)| {
                *generation == cx.global::<Service>().generation.load(Ordering::Relaxed)
                    && self.active_editor().is_some_and(|e| documents.iter().any(|d| d.matches_route(e.read(cx), cx)))
            })
    }

    pub(super) fn navigation_capabilities(&self, cx: &App) -> NavigationCapabilities {
        if !cx.global::<Settings>().lsp_enabled {
            return NavigationCapabilities::default();
        }
        self.language
            .navigation
            .as_ref()
            .filter(|(generation, _)| *generation == cx.global::<Service>().generation.load(Ordering::Relaxed))
            .and_then(|(_, documents)| {
                let editor = self.active_editor()?;
                documents
                    .iter()
                    .find(|(document, _)| document.matches_route(editor.read(cx), cx))
                    .map(|(_, capabilities)| *capabilities)
            })
            .unwrap_or_default()
    }

    pub(super) fn formatting_composition(&self, buffer: u64, cx: &App) -> bool {
        self.open_editors(cx).iter().any(|e| e.read(cx).doc(cx).id() == buffer && !e.read(cx).marked.is_empty())
    }

    fn language_request(&self, cx: &App) -> Result<(Input, Request), String> {
        let editor = self.active_editor().ok_or("Open a source file first")?;
        self.language_request_for(&editor, cx)
    }

    fn no_server(&self, e: &Editor, cx: &App) -> String {
        let (_, name, _) = self.file_language(e, e.doc(cx).path.as_deref(), true, cx);
        format!("No language server is configured for {name}")
    }

    /// `language_request` for a command to send: the buffer's language must have a server
    /// with a command (an empty one is turned off).
    fn served_language_request(&self, cx: &App) -> Result<(Input, Request), String> {
        let (input, request) = self.language_request(cx)?;
        let server = request.document.language().map(|language| language.server);
        if !server.and_then(|server| input.config.get(&server)).is_some_and(|c| !c.command.is_empty()) {
            let editor = self.active_editor().ok_or("Open a source file first")?;
            return Err(self.no_server(editor.read(cx), cx));
        }
        Ok((input, request))
    }

    fn language_request_for(&self, editor: &Entity<Editor>, cx: &App) -> Result<(Input, Request), String> {
        let input = self.language_input_for(Some(editor), cx);
        if !input.enabled {
            return Err("Language servers are disabled in Preferences".into());
        }
        let e = editor.read(cx);
        let document = input
            .documents
            .iter()
            .find(|d| d.matches(e, cx))
            .cloned()
            .ok_or("Save the source file before using LSP")?;
        if document.language().is_none() {
            return Err(self.no_server(e, cx));
        }
        let documents = input.documents.clone();
        let epoch = cx.global::<Service>().generation.clone();
        Ok((
            input,
            Request {
                document,
                documents,
                cursor: e.state.cursor(),
                selection: e.state.selections[e.state.primary()].range(),
                format_selection: None,
                generation: (self.restore_gen, self.language.generation),
                navigation_generation: None,
                navigation_source: None,
                service_generation: epoch.load(Ordering::Relaxed),
                epoch,
                started: std::time::SystemTime::now(),
            },
        ))
    }

    pub(super) fn language_request_current(&self, request: &Request, cx: &App) -> bool {
        request.generation == (self.restore_gen, self.language.generation)
            && request.navigation_generation.is_none_or(|generation| generation == self.language.navigation_generation)
            && request.epoch.load(Ordering::Relaxed) == request.service_generation
            && cx.global::<Settings>().lsp_enabled
            && self.active_editor().is_some_and(|e| {
                let view = e.read(cx);
                request.document.matches(view, cx)
                    && view.state.cursor() == request.cursor
                    && request.navigation_source.as_ref().is_none_or(|(editor, generation, state)| {
                        *editor == e.entity_id()
                            && *generation == view.read_state(cx).selection_generation()
                            && *state == view.read_state(cx).selection_state()
                    })
                    && request.format_selection.as_ref().is_none_or(|origin| {
                        origin.editor == e.entity_id()
                            && origin.generation == view.read_state(cx).selection_generation()
                            && origin.state == view.read_state(cx).selection_state()
                            && view.input_enabled
                            && !view.doc(cx).read_only
                            && !view.doc(cx).edits_blocked()
                            && !self.formatting_composition(request.document.buffer, cx)
                    })
            })
    }

    fn request_language(&mut self, feature: Feature, extra: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_navigation();
        if matches!(feature, Feature::SignatureHelp)
            && self.active_editor().is_none_or(|e| !e.read(cx).signature_allowed(window))
        {
            if extra.get("_signatureAuto").is_none()
                && self.active_editor().is_some_and(|e| e.read(cx).state.selections.iter().any(|s| s.has_virtual()))
            {
                self.status_message("Signature help unavailable: the caret is beyond the line end", cx);
            }
            if let Some(editor) = self.active_editor() {
                editor.update(cx, |e, _| e.dismiss_signature());
            }
            return;
        }
        if feature.is_destination() || matches!(feature, Feature::SignatureHelp | Feature::RangeFormat) {
            if let Some(editor) = self.active_editor() {
                editor.update(cx, |e, cx| {
                    e.catch_up(cx);
                    e.sel(cx).sel_checkpoint();
                    if matches!(feature, Feature::SignatureHelp) && extra.get("_signatureAuto").is_none() {
                        // Explicit invocation starts fresh; automatic input supplies its own context.
                        e.dismiss_signature();
                    }
                });
            }
        }
        let (input, mut request) = match self.served_language_request(cx) {
            Ok(r) => r,
            Err(e) => {
                if matches!(feature, Feature::SignatureHelp) {
                    self.with_active(cx, |e, _| e.dismiss_signature());
                }
                if !self.lexical_fallback(feature, window, cx) {
                    self.status_message(e, cx);
                }
                return;
            }
        };
        if matches!(feature, Feature::RangeFormat) {
            request.format_selection = self.active_editor().map(|editor| {
                let view = editor.read(cx);
                let state = view.read_state(cx);
                FormatSelection {
                    editor: editor.entity_id(),
                    generation: state.selection_generation(),
                    state: state.selection_state(),
                }
            });
        }
        if feature.is_destination()
            || matches!(
                feature,
                Feature::Definition
                    | Feature::DefinitionToSide
                    | Feature::References
                    | Feature::Hover
                    | Feature::SignatureHelp
            )
        {
            request.navigation_generation = Some(self.language.navigation_generation);
        }
        let navigation_cancel = feature.is_destination().then(|| {
            request.navigation_source = self.active_editor().map(|editor| {
                let view = editor.read(cx);
                (editor.entity_id(), view.read_state(cx).selection_generation(), view.read_state(cx).selection_state())
            });
            let cancel = Arc::new(AtomicBool::new(false));
            self.language.navigation_cancel = Some(cancel.clone());
            cancel
        });
        let signature_origin = matches!(feature, Feature::SignatureHelp).then(|| {
            self.active_editor().map(|e| {
                let view = e.read(cx);
                (
                    e.entity_id(),
                    view.completion_epoch,
                    view.read_state(cx).selection_generation(),
                    view.state.selections.clone(),
                    view.state.primary(),
                )
            })
        });
        let signature_cancel = matches!(feature, Feature::SignatureHelp).then(|| {
            let cancel = Arc::new(AtomicBool::new(false));
            self.language.signature_cancel = Some(cancel.clone());
            if let Some(editor) = self.active_editor() {
                editor.update(cx, |e, cx| e.begin_signature(cancel.clone(), window, cx));
            }
            cancel
        });
        let signature_viewport = window.viewport_size();
        let signature_editor = signature_cancel.as_ref().and_then(|_| self.active_editor());
        let signature_token = signature_cancel.clone();
        let quiet = extra.get("_signatureAuto").is_some();
        self.run_language(
            (input, request),
            feature.method(),
            extra,
            quiet,
            signature_cancel.or(navigation_cancel),
            window,
            cx,
            move |ws, response, request, window, cx| {
                let invalid = (signature_origin.is_some()
                    && ws.active_editor().is_none_or(|e| !e.read(cx).signature_allowed(window)))
                    || (signature_origin.is_some() && window.viewport_size() != signature_viewport)
                    || signature_origin.is_some_and(|origin| {
                        origin
                            != ws.active_editor().map(|e| {
                                let view = e.read(cx);
                                (
                                    e.entity_id(),
                                    view.completion_epoch,
                                    view.read_state(cx).selection_generation(),
                                    view.state.selections.clone(),
                                    view.state.primary(),
                                )
                            })
                    });
                if invalid {
                    if let (Some(editor), Some(token)) = (&signature_editor, &signature_token) {
                        editor.update(cx, |e, _| e.finish_signature(token));
                    }
                    return;
                }
                if let Some(response) = response {
                    ws.language_response(feature, response, request, window, cx);
                } else {
                    if let (Some(editor), Some(token)) = (&signature_editor, &signature_token) {
                        editor.update(cx, |e, _| e.finish_signature(token));
                    }
                    if ws.language_request_current(&request, cx) {
                        ws.lexical_fallback(feature, window, cx);
                    }
                }
            },
        );
    }

    /// Ask the server for `method` in the background. `done` gets the response once it comes for
    /// the same document and caret, else `None`: it failed or was discarded, which the status bar
    /// says unless `quiet` (`ctrl-space` is an everyday key and its local list is already open).
    #[allow(clippy::too_many_arguments)]
    fn run_language(
        &mut self,
        (input, request): (Input, Request),
        method: &'static str,
        extra: Value,
        quiet: bool,
        cancel: Option<Arc<AtomicBool>>,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Option<Response>, Request, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !quiet {
            self.status_message("Language server: working…", cx);
        }
        let manager = self.language.manager.clone();
        let snapshot = request.clone();
        let cancelled = cancel.clone();
        let executor = cx.background_executor().clone();
        let task = cx.background_spawn(async move {
            if let Some(cancel) = cancel {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Language request cancelled".into());
                    }
                    match manager.try_lock() {
                        Ok(mut manager) => {
                            return manager.request_with_cancel(&input, &snapshot, method, extra, Some(&cancel))
                        }
                        Err(std::sync::TryLockError::Poisoned(error)) => {
                            return error.into_inner().request_with_cancel(
                                &input,
                                &snapshot,
                                method,
                                extra,
                                Some(&cancel),
                            )
                        }
                        Err(std::sync::TryLockError::WouldBlock) => {}
                    }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
            }
            manager.lock().unwrap_or_else(|e| e.into_inner()).request(&input, &snapshot, method, extra)
        });
        cx.spawn_in(window, async move |this, cx| {
            let response = task.await;
            #[cfg(test)]
            if method == "textDocument/signatureHelp" {
                crate::documents::save::gate::wait(&tests::signature_gate_path(&request.document.path)).await;
            }
            #[cfg(test)]
            if method == "textDocument/rangeFormatting" {
                crate::documents::save::gate::wait(&tests::range_gate_path(&request.document.path)).await;
            }
            #[cfg(test)]
            if matches!(method, "textDocument/typeDefinition" | "textDocument/implementation") {
                crate::documents::save::gate::wait(&tests::navigation_gate_path(&request.document.path)).await;
            }
            this.update_in(cx, |ws, window, cx| {
                let say = |ws: &mut Self, text: String, cx: &mut Context<Self>| {
                    if !quiet {
                        ws.status_message(text, cx)
                    }
                };
                let response = match response {
                    _ if cancelled.as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed)) => None,
                    _ if !ws.language_request_current(&request, cx) => {
                        let discarded = "Language result discarded: document, caret or navigation changed";
                        say(ws, discarded.into(), cx);
                        None
                    }
                    Ok(response) => Some(response),
                    Err(error) => {
                        say(ws, format!("LSP: {error}"), cx);
                        None
                    }
                };
                done(ws, response, request, window, cx);
            })
            .ok();
        })
        .detach();
    }

    forward_actions! {
    pub(crate) lsp_completion(LspCompletion, window, cx) => server_completions(false, window, cx);
    }

    /// The completion list in the active view with the language server's items, when asked for
    /// (`ctrl-space`, LSP: Complete) in a buffer a server serves; `quiet` (ctrl-space) says
    /// nothing where none does.
    pub(crate) fn server_completions(&mut self, quiet: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (input, request) = match self.served_language_request(cx) {
            Ok(r) => r,
            Err(_) if quiet => return,
            Err(e) => return self.status_message(e, cx),
        };
        let Some(editor) = self.active_editor() else {
            return;
        };
        if !editor.update(cx, |e, cx| e.expect_server_items(window, cx)) {
            if let Some(why) = editor.read(cx).cannot_complete(cx).filter(|_| !quiet) {
                self.status_message(why, cx);
            }
            return;
        }
        self.invalidate_navigation();
        let view = editor.downgrade();
        self.run_language(
            (input, request),
            "textDocument/completion",
            json!({"context":{"triggerKind":1}}),
            quiet,
            None,
            window,
            cx,
            move |ws, response, request, _, cx| match response {
                Some(response) => ws.completion_items(response, request, &view, quiet, cx),
                // nothing is coming: the list stops waiting for it
                None => {
                    view.update(cx, |e, cx| e.end_server_wait(cx)).ok();
                }
            },
        );
    }

    /// The server's plain-text items (at most 5,000) into the list of `view`, which asked for them;
    /// `quiet` (ctrl-space) says nothing about them.
    fn completion_items(
        &mut self,
        response: Response,
        request: Request,
        view: &gpui::WeakEntity<Editor>,
        quiet: bool,
        cx: &mut Context<Self>,
    ) {
        let values = response.value.as_array().or_else(|| response.value["items"].as_array());
        let defaults = &response.value["itemDefaults"];
        let (mut items, mut offered) = (Vec::new(), Vec::new());
        for item in values.into_iter().flatten().take(5000) {
            let mut item = item.clone();
            if let Some(defaults) = defaults.as_object() {
                for key in ["insertTextFormat", "data"] {
                    if item.get(key).is_none() {
                        if let Some(value) = defaults.get(key) {
                            item[key] = value.clone();
                        }
                    }
                }
            }
            if item.get("textEdit").is_none() && defaults["editRange"].is_object() {
                let range = &defaults["editRange"];
                item["textEdit"] = json!({"range":range.get("replace").unwrap_or(range),
                    "newText":item.get("textEditText").or_else(||item.get("insertText")).unwrap_or(&item["label"])});
            }
            let Some(label) = item["label"].as_str().map(String::from) else {
                continue;
            };
            let detail = item["detail"].as_str().and_then(|d| d.lines().next());
            offered.push(Candidate {
                label,
                kind: complete::Kind::Symbol(language::completion_kind(&item).into()),
                detail: detail.unwrap_or_default().to_string(),
                insert: Insert::Server(items.len()),
            });
            items.push(item);
        }
        if !quiet {
            match items.is_empty() {
                true => self.status_message("No completions", cx),
                // the list shows them: the "working…" message is done
                false => self.status = None,
            }
        }
        self.language.completion = Some((request, response.encoding, items));
        view.update(cx, |e, cx| e.add_server_items(offered, cx)).ok();
        cx.notify();
    }
    pub(crate) fn lsp_definition(&mut self, side: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.request_language(
            if side { Feature::DefinitionToSide } else { Feature::Definition },
            json!({}),
            window,
            cx,
        );
    }

    fn lexical_fallback(&mut self, feature: Feature, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match feature {
            Feature::Definition | Feature::DefinitionToSide => {
                self.local_definition(matches!(feature, Feature::DefinitionToSide), window, cx)
            }
            _ => return false,
        }
        true
    }
    pub(crate) fn lsp_references(&mut self, _: &LspReferences, window: &mut Window, cx: &mut Context<Self>) {
        self.request_language(Feature::References, json!({"context":{"includeDeclaration":true}}), window, cx);
    }
    forward_actions! {
        pub(crate) lsp_type_definition(LspTypeDefinition, window, cx) => request_language(Feature::TypeDefinition, json!({}), window, cx);
        pub(crate) lsp_type_definition_to_side(LspTypeDefinitionToSide, window, cx) => request_language(Feature::TypeDefinitionToSide, json!({}), window, cx);
        pub(crate) lsp_implementation(LspImplementation, window, cx) => request_language(Feature::Implementation, json!({}), window, cx);
        pub(crate) lsp_implementation_to_side(LspImplementationToSide, window, cx) => request_language(Feature::ImplementationToSide, json!({}), window, cx);
    }
    pub(crate) fn lsp_hover(&mut self, _: &LspHover, window: &mut Window, cx: &mut Context<Self>) {
        self.with_active(cx, |e, cx| e.close_definition_hover(cx));
        self.request_language(Feature::Hover, json!({}), window, cx);
    }
    pub(crate) fn signature_input(
        &mut self,
        text: String,
        active: Option<(Arc<Value>, usize)>,
        trigger: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let retrigger = active.is_some();
        let mut context = json!({"triggerKind":if trigger {2} else {3},"isRetrigger":retrigger});
        if trigger {
            context["triggerCharacter"] = json!(text);
        }
        if let Some((active, selected)) = active {
            let mut active = (*active).clone();
            active["activeSignature"] = json!(selected);
            context["activeSignatureHelp"] = active;
        }
        let mut extra = json!({"_signatureAuto":true,"context":context});
        if trigger {
            extra["_signatureTrigger"] = json!(text);
        }
        self.request_language(Feature::SignatureHelp, extra, window, cx);
    }

    pub(crate) fn lsp_signature_help(&mut self, _: &LspSignatureHelp, window: &mut Window, cx: &mut Context<Self>) {
        self.request_language(
            Feature::SignatureHelp,
            json!({"context":{"triggerKind":1,"isRetrigger":false}}),
            window,
            cx,
        );
    }

    /// Format Document: the language's configured formatter, else its language server.
    pub(crate) fn lsp_format(&mut self, _: &LspFormat, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let e = editor.read(cx);
        if let Some(command) = self.formatter_for(e, e.doc(cx).path.as_deref(), cx) {
            self.invalidate_navigation();
            return self.format_with(editor, command, window, cx);
        }
        let s = editor.read(cx).read_state(cx);
        self.request_language(
            Feature::Format,
            json!({"options":{"tabSize":s.doc.tab_size,"insertSpaces":s.doc.translate_tabs}}),
            window,
            cx,
        );
    }

    /// A save's optional preparation. The document save queue calls this only after reserving
    /// its slot; the task covers the request, validation, shared edit and final cleanup.
    pub(crate) fn format_for_save(
        &self,
        editor: &Entity<Editor>,
        target: Option<&Path>,
        tidy: impl FnOnce(&mut App) + 'static,
        cx: &Context<Self>,
    ) -> impl FnOnce(&mut App) -> Option<Task<()>> + 'static {
        self.format_for_save_guarded(editor, target, tidy, None, cx)
    }

    /// Document freshness for save formatting is independent of caret/selection movement.
    fn save_format_current(&self, editor: &Entity<Editor>, request: &Request, cx: &App) -> Result<(), String> {
        let e = editor.read(cx);
        if request.generation != (self.restore_gen, self.language.generation)
            || request.epoch.load(Ordering::Relaxed) != request.service_generation
            || !cx.global::<Settings>().lsp_enabled
            || !request.document.matches(e, cx)
            || !e.input_enabled
            || e.doc(cx).read_only
            || e.doc(cx).edits_blocked()
            || self.formatting_composition(request.document.buffer, cx)
        {
            return Err("document, composition or language server changed".into());
        }
        self.save_file_current(&request.document.path, request.document.buffer, cx)
    }

    /// What formatting before a save needs of the file: not being written, and only one buffer
    /// holding it.
    fn save_file_current(&self, path: &Path, buffer: u64, cx: &App) -> Result<(), String> {
        if super::pipelines::reading_blocked(path, cx) {
            return Err("file is being written".into());
        }
        let file = super::pipelines::file_id(path);
        if self.open_editors(cx).iter().any(|other| {
            let other = other.read(cx);
            other.doc(cx).id() != buffer
                && other.doc(cx).path.as_ref().is_some_and(|other| {
                    crate::editing::text::resolve(other) == path
                        || file.is_some_and(|id| super::pipelines::file_id(other) == Some(id))
                })
        }) {
            return Err("Different buffers have the same file; close the duplicate before formatting".into());
        }
        Ok(())
    }

    /// A save's formatter may still replace the text it was given.
    fn save_formatter_current(&self, editor: &Entity<Editor>, target: &FormatTarget, cx: &App) -> Result<(), String> {
        self.format_admission(editor, Some(target), cx).map_err(|_| "document or composition changed".to_string())?;
        target.path().map_or(Ok(()), |path| self.save_file_current(path, target.buffer(), cx))
    }

    pub(crate) fn format_for_save_guarded(
        &self,
        editor: &Entity<Editor>,
        target: Option<&Path>,
        tidy: impl FnOnce(&mut App) + 'static,
        valid: Option<crate::documents::save::SaveValidation>,
        cx: &Context<Self>,
    ) -> impl FnOnce(&mut App) -> Option<Task<()>> + 'static {
        let e = editor.read(cx);
        let path = target.or(e.doc(cx).path.as_deref());
        let settings = self.settings_for_path(e, path, cx);
        let enabled = settings.format_on_save;
        let formatter = enabled.then(|| self.formatter_for(e, path, cx)).flatten();
        let timeout = std::time::Duration::from_millis(u64::from(settings.format_on_save_timeout_ms));
        let editor = editor.clone();
        let workspace = cx.entity().downgrade();
        move |cx| {
            if valid.as_ref().is_some_and(|valid| !valid(cx)) {
                return None;
            }
            let doc = editor.read(cx).doc(cx);
            if !enabled || doc.read_only || doc.edits_blocked() {
                tidy(cx);
                return None;
            }
            let document = editor.read(cx).document.clone();
            #[cfg(test)]
            let prepared_path = doc.path.clone();
            let deadline = std::time::Instant::now() + timeout;
            if let Some(command) = formatter {
                return Some(Self::formatter_for_save(editor, workspace, command, document, deadline, tidy, valid, cx));
            }
            Some(cx.spawn(async move |cx| {
                use std::future::Future;
                use std::task::Poll;
                // Read at this save's turn, without depending on the active tab or caret.
                let result = async {
                    let (input, request, options, manager) = cx
                        .update(|cx| {
                            workspace
                                .update(cx, |ws, cx| {
                                    let (input, request) = ws.language_request_for(&editor, cx)?;
                                    ws.save_format_current(&editor, &request, cx)?;
                                    let doc = editor.read(cx).doc(cx);
                                    let options =
                                        json!({"options":{"tabSize":doc.tab_size,"insertSpaces":doc.translate_tabs}});
                                    Ok::<_, String>((input, request, options, ws.language.manager.clone()))
                                })
                                .map_err(|e| e.to_string())?
                        })
                        .map_err(|e| e.to_string())??;
                    let cancel = Arc::new(AtomicBool::new(false));
                    let token = cancel.clone();
                    let before = request.clone();
                    let executor = cx.background_executor().clone();
                    let mut task = std::pin::pin!(cx.background_spawn(async move {
                        let mut response = loop {
                            if token.load(Ordering::Relaxed) {
                                return Err("Save formatting cancelled".into());
                            }
                            if std::time::Instant::now() >= deadline {
                                return Err("Save formatting timed out".into());
                            }
                            match manager.try_lock() {
                                Ok(mut manager) => {
                                    break manager.request_limited(
                                        &input,
                                        &before,
                                        "textDocument/formatting",
                                        options,
                                        Some((&token, Some(deadline))),
                                    )?
                                }
                                Err(std::sync::TryLockError::Poisoned(error)) => {
                                    break error.into_inner().request_limited(
                                        &input,
                                        &before,
                                        "textDocument/formatting",
                                        options,
                                        Some((&token, Some(deadline))),
                                    )?
                                }
                                Err(std::sync::TryLockError::WouldBlock) => {}
                            }
                            executor.timer(std::time::Duration::from_millis(10)).await;
                        };
                        let uri = lsp::uri(&before.document.path)?;
                        if response.value.is_null() {
                            response.value = json!([]);
                        }
                        response.value = json!({"changes":{uri:response.value}});
                        prepare_edits(&before, response)
                    }));
                    let edits = loop {
                        let mut tick =
                            std::pin::pin!(cx.background_executor().timer(std::time::Duration::from_millis(25)));
                        let ready = std::future::poll_fn(|context| {
                            if let Poll::Ready(result) = task.as_mut().poll(context) {
                                return Poll::Ready(Some(result));
                            }
                            tick.as_mut().poll(context).map(|()| None)
                        })
                        .await;
                        let current = cx
                            .update(|cx| {
                                if valid.as_ref().is_some_and(|valid| !valid(cx)) {
                                    return Err("Automatic save cancelled".into());
                                }
                                workspace
                                    .update(cx, |ws, cx| ws.save_format_current(&editor, &request, cx))
                                    .map_err(|e| e.to_string())?
                            })
                            .map_err(|e| e.to_string())
                            .and_then(|r| r);
                        if let Err(error) = current {
                            cancel.store(true, Ordering::Relaxed);
                            break Err(error);
                        }
                        if std::time::Instant::now() >= deadline {
                            cancel.store(true, Ordering::Relaxed);
                            break Err("Save formatting timed out".into());
                        }
                        if let Some(result) = ready {
                            break result;
                        }
                    }?;
                    #[cfg(test)]
                    crate::documents::save::gate::wait(&tests::format_gate_path(&request.document.path)).await;
                    cx.update(|cx| {
                        if std::time::Instant::now() >= deadline {
                            return Err("Save formatting timed out".into());
                        }
                        if valid.as_ref().is_some_and(|valid| !valid(cx)) {
                            return Err("Automatic save cancelled".into());
                        }
                        workspace
                            .update(cx, |ws, cx| {
                                ws.save_format_current(&editor, &request, cx)?;
                                // The formatter can edit only this original document, atomically.
                                for edit in &edits {
                                    if edit.before.as_ref().is_none_or(|before| !before.matches(editor.read(cx), cx))
                                        || edit.path != request.document.path
                                    {
                                        return Err("Formatter edited another document".into());
                                    }
                                }
                                for edit in edits {
                                    editor.update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(edit.edits)));
                                }
                                tidy(cx);
                                let mut prepared = request.clone();
                                prepared.document.version = editor.read(cx).doc(cx).version;
                                let origin = editor.clone();
                                let workspace = workspace.clone();
                                let valid = std::rc::Rc::new(move |cx: &App| {
                                    std::time::Instant::now() < deadline
                                        && workspace.upgrade().is_some_and(|ws| {
                                            ws.read(cx).save_format_current(&origin, &prepared, cx).is_ok()
                                        })
                                });
                                crate::documents::OpenDocument::validate_before_capture(&document, valid, cx);
                                Ok::<_, String>(())
                            })
                            .map_err(|e| e.to_string())?
                    })
                    .map_err(|e| e.to_string())?
                }
                .await;
                #[cfg(test)]
                if result.is_ok() {
                    if let Some(path) = prepared_path {
                        crate::documents::save::gate::wait(&path.join("format-capture")).await;
                    }
                }
                if let Err(error) = result {
                    cx.update(|cx| {
                        let error = format!(
                            "Save formatting failed: {error}. Use File > Save Without Formatting to save explicitly."
                        );
                        workspace.update(cx, |ws, cx| ws.status_message(&error, cx)).ok();
                        crate::documents::OpenDocument::fail_preparation(&document, error, cx);
                    })
                    .ok();
                }
            }))
        }
    }
    /// `format_for_save_guarded` with a formatter in place of the language server, with the same
    /// guarantees: one deadline, the text left as it was on any failure, and a capture that
    /// rechecks the formatted text.
    #[allow(clippy::too_many_arguments)]
    fn formatter_for_save(
        editor: Entity<Editor>,
        workspace: gpui::WeakEntity<Self>,
        command: Vec<String>,
        document: Entity<crate::documents::OpenDocument>,
        deadline: std::time::Instant,
        tidy: impl FnOnce(&mut App) + 'static,
        valid: Option<crate::documents::save::SaveValidation>,
        cx: &mut App,
    ) -> Task<()> {
        cx.spawn(async move |cx| {
            use std::future::Future;
            use std::task::Poll;
            let result = async {
                let (process, snapshot) = cx
                    .update(|cx| {
                        workspace
                            .update(cx, |ws, cx| {
                                let snapshot = FormatTarget::of(&editor, cx);
                                ws.save_formatter_current(&editor, &snapshot, cx)?;
                                let left = deadline.saturating_duration_since(std::time::Instant::now());
                                Ok::<_, String>((ws.formatter_process(&editor, left, cx), snapshot))
                            })
                            .map_err(|e| e.to_string())?
                    })
                    .map_err(|e| e.to_string())??;
                let cancel = Arc::new(AtomicBool::new(false));
                let token = cancel.clone();
                let program = command.clone();
                let mut task = std::pin::pin!(
                    cx.background_spawn(async move { automation::run_program(&program, process, &token) })
                );
                let output = loop {
                    let mut tick = std::pin::pin!(cx.background_executor().timer(std::time::Duration::from_millis(25)));
                    let ready = std::future::poll_fn(|context| {
                        if let Poll::Ready(output) = task.as_mut().poll(context) {
                            return Poll::Ready(Some(output));
                        }
                        tick.as_mut().poll(context).map(|()| None)
                    })
                    .await;
                    let current = cx
                        .update(|cx| {
                            if valid.as_ref().is_some_and(|valid| !valid(cx)) {
                                return Err("Automatic save cancelled".into());
                            }
                            workspace
                                .update(cx, |ws, cx| ws.save_formatter_current(&editor, &snapshot, cx))
                                .map_err(|e| e.to_string())?
                        })
                        .map_err(|e| e.to_string())
                        .and_then(|r| r);
                    if let Err(error) = current {
                        cancel.store(true, Ordering::Relaxed);
                        break Err(error);
                    }
                    if let Some(output) = ready {
                        break Ok(output);
                    }
                }?;
                if output.problem.as_deref() == Some("Command timed out") {
                    return Err("Save formatting timed out".into());
                }
                cx.update(|cx| {
                    if std::time::Instant::now() >= deadline {
                        return Err("Save formatting timed out".into());
                    }
                    if valid.as_ref().is_some_and(|valid| !valid(cx)) {
                        return Err("Automatic save cancelled".into());
                    }
                    workspace
                        .update(cx, |ws, cx| {
                            ws.save_formatter_current(&editor, &snapshot, cx)?;
                            let edit =
                                super::automation::formatted_edit(&command, &output, &editor.read(cx).doc(cx).rope)?;
                            if let Some(edit) = edit {
                                editor.update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(vec![edit])));
                            }
                            tidy(cx);
                            let prepared = FormatTarget::of(&editor, cx);
                            let origin = editor.clone();
                            let workspace = workspace.clone();
                            let valid = std::rc::Rc::new(move |cx: &App| {
                                std::time::Instant::now() < deadline
                                    && workspace.upgrade().is_some_and(|ws| {
                                        ws.read(cx).save_formatter_current(&origin, &prepared, cx).is_ok()
                                    })
                            });
                            crate::documents::OpenDocument::validate_before_capture(&document, valid, cx);
                            Ok::<_, String>(())
                        })
                        .map_err(|e| e.to_string())?
                })
                .map_err(|e| e.to_string())?
            }
            .await;
            if let Err(error) = result {
                cx.update(|cx| {
                    let error = format!(
                        "Save formatting failed: {error}. Use File > Save Without Formatting to save explicitly."
                    );
                    workspace.update(cx, |ws, cx| ws.status_message(&error, cx)).ok();
                    crate::documents::OpenDocument::fail_preparation(&document, error, cx);
                })
                .ok();
            }
        })
    }

    pub(crate) fn lsp_format_selection(&mut self, _: &LspFormatSelection, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let view = editor.read(cx);
        if !view.input_enabled || view.doc(cx).read_only || view.doc(cx).edits_blocked() {
            return self.status_message("Document is read only", cx);
        }
        if self.formatting_composition(view.doc(cx).id(), cx) {
            return self.status_message("Finish text composition before formatting", cx);
        }
        let s = view.read_state(cx);
        if s.view.selections.iter().all(|s| s.range().is_empty()) {
            return self.status_message("Select a range to format", cx);
        }
        self.request_language(
            Feature::RangeFormat,
            json!({"options":{"tabSize":s.doc.tab_size,"insertSpaces":s.doc.translate_tabs}}),
            window,
            cx,
        );
    }

    pub(crate) fn lsp_code_actions(&mut self, _: &LspCodeActions, window: &mut Window, cx: &mut Context<Self>) {
        self.request_language(Feature::CodeActions, json!({}), window, cx);
    }
    pub(crate) fn lsp_rename(&mut self, _: &LspRename, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let s = editor.read(cx).read_state(cx);
        let word = s.doc.rope.slice(s.word_range_at(s.cursor())).to_string();
        let done: OnDone = Box::new(|values, _, ws, window, cx| {
            let name = values.text("text");
            if name.trim().is_empty() || name.contains(['\n', '\r', '\0']) {
                return Err("Enter a nonempty name on one line".into());
            }
            ws.request_language(Feature::Rename, json!({"newName":name}), window, cx);
            Ok(())
        });
        self.show_input("Rename symbol", &word, done, window, cx);
    }

    fn language_response(
        &mut self,
        feature: Feature,
        response: Response,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let location =
            |uri, range| Choice::Location { uri, range, encoding: response.encoding, side: feature.to_side() };
        match feature {
            Feature::Definition
            | Feature::DefinitionToSide
            | Feature::References
            | Feature::TypeDefinition
            | Feature::TypeDefinitionToSide
            | Feature::Implementation
            | Feature::ImplementationToSide => match language::locations(&response.value) {
                Ok(locations) if locations.len() == 1 && !matches!(feature, Feature::References) => {
                    let (uri, range) = locations.into_iter().next().unwrap();
                    self.choose_language(location(uri, range), window, cx);
                }
                Ok(locations) => {
                    let items = locations
                        .into_iter()
                        .map(|(uri, range)| {
                            let path = lsp::path(&uri).map_or(uri.clone(), |p| tilde(&p));
                            let (line, col) = (range.start.line + 1, range.start.character + 1);
                            super::pickers::choice(
                                &format!("{path}:{line}:{col}"),
                                "Language server location",
                                ItemAction::Language(location(uri, range)),
                            )
                        })
                        .collect::<Vec<_>>();
                    if items.is_empty() {
                        if !self.lexical_fallback(feature, window, cx) {
                            self.status_message("No locations found", cx);
                        }
                    } else {
                        self.open_quick_panel(Kind::Choices, items, window, cx);
                        if feature.is_destination() {
                            let mut request = request;
                            request.navigation_generation = Some(self.language.navigation_generation);
                            self.quick_panel.as_mut().unwrap().language_navigation = Some(request);
                        }
                    }
                }
                Err(error) => self.status_message(error, cx),
            },
            Feature::SignatureHelp => match orlo_syntax::signature::signature_help(&response.value) {
                Some(signature) => {
                    let cancel = Arc::new(AtomicBool::new(false));
                    self.language.signature_cancel = Some(cancel.clone());
                    self.with_active(cx, |e, cx| e.show_signature(signature, cancel, window, cx));
                }
                None => {
                    self.with_active(cx, |e, _| e.dismiss_signature());
                    self.status_message("No signature help available", cx);
                }
            },
            Feature::Hover => {
                self.with_active(cx, |e, cx| e.close_definition_hover(cx));
                let text = language::hover_text(&response.value);
                if text.is_empty() {
                    self.status_message("No documentation found", cx);
                } else {
                    self.inform(window, cx, "Language documentation", &text);
                }
            }
            Feature::Rename | Feature::Format | Feature::RangeFormat => {
                let mut response = response;
                if matches!(feature, Feature::Format | Feature::RangeFormat) {
                    if response.value.is_null() {
                        response.value = json!([]);
                    }
                    let uri = lsp::uri(&request.document.path).unwrap_or_default();
                    response.value = json!({"changes":{uri:response.value}});
                }
                self.stage_language_edits(request, response, window, cx);
            }
            Feature::CodeActions => {
                let request = Arc::new(request);
                let items = response
                    .value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|a| a.get("disabled").is_none())
                    .filter_map(|action| {
                        Some(super::pickers::choice(
                            action["title"].as_str()?,
                            action["kind"].as_str().unwrap_or("Language server code action"),
                            ItemAction::Language(Choice::CodeAction {
                                request: request.clone(),
                                action: action.clone(),
                            }),
                        ))
                    })
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    self.status_message("No code actions available", cx);
                } else {
                    self.open_quick_panel(Kind::Choices, items, window, cx);
                }
            }
        }
    }

    pub(crate) fn lsp_diagnostics(&mut self, _: &LspDiagnostics, window: &mut Window, cx: &mut Context<Self>) {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for editor in self.views.tabs() {
            let e = editor.read(cx);
            // views of one document share its diagnostics
            if !e.language_current(cx) || !seen.insert(e.doc(cx).id()) {
                continue;
            }
            let Some(path) = &e.doc(cx).path else {
                continue;
            };
            for diagnostic in &e.diagnostics {
                let (line, col) = e.doc(cx).point(diagnostic.range.start);
                let uri = lsp::uri(path).unwrap_or_default();
                let range = lsp::span(
                    e.doc(cx).coords(),
                    CharOffset(diagnostic.range.start)..CharOffset(diagnostic.range.end),
                    Encoding::Utf16,
                );
                let severity = if diagnostic.severity == 1 { "error" } else { "warning" };
                items.push(super::pickers::choice(
                    &format!("{}:{}:{} · {severity}: {}", path.display(), line + 1, col + 1, diagnostic.message),
                    "Language server diagnostic",
                    ItemAction::Language(Choice::Location { uri, range, encoding: Encoding::Utf16, side: false }),
                ));
            }
        }
        if items.is_empty() {
            let status = self.active_editor().map(|e| e.read(cx).lsp_status.clone()).unwrap_or_default();
            self.status_message(format!("No diagnostics. {status}"), cx);
        } else {
            self.open_quick_panel(Kind::Choices, items, window, cx);
            if let Some(panel) = &mut self.quick_panel {
                panel.diagnostic_details = true;
            }
        }
    }

    pub(crate) fn choose_language(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        match choice {
            Choice::Location { uri, range, encoding, side } => {
                self.language_location(uri, range, encoding, side, window, cx);
            }
            Choice::CodeAction { request, action } => {
                if !self.language_request_current(&request, cx) {
                    return self.status_message("Code action discarded: document changed", cx);
                }
                let input = self.language_input(cx);
                let manager = self.language.manager.clone();
                let snapshot = request.clone();
                let task = cx.background_spawn(async move {
                    let mut manager = manager.lock().unwrap_or_else(|e| e.into_inner());
                    let mut action = action;
                    if action.get("edit").is_none()
                        && !action.get("command").is_some_and(Value::is_string)
                        && action.get("data").is_some()
                        && manager.can_resolve(&snapshot.document, "codeActionProvider")
                    {
                        action = manager.request(&input, &snapshot, "codeAction/resolve", action)?.value;
                    }
                    if let Some(edit) = action.get("edit") {
                        let key = server_key(&snapshot.document).ok_or("Unsupported language")?;
                        let session =
                            manager.sessions.get(&key).ok_or("Server closed")?.as_ref().map_err(|e| e.clone())?;
                        return Ok(Response {
                            value: edit.clone(),
                            encoding: session.client.encoding,
                            versions: session.versions(),
                            root: key.0,
                            command: action.get("command").cloned(),
                        });
                    }
                    let command = action.get("command").ok_or("Code action has no edit or command")?;
                    let command = if command.is_string() { &action } else { command };
                    manager.request(&input, &snapshot, "workspace/executeCommand", command.clone())
                });
                cx.spawn_in(window, async move |this, cx| {
                    let response = task.await;
                    this.update_in(cx, |ws, window, cx| {
                        if !ws.language_request_current(&request, cx) {
                            return ws.status_message("Code action discarded: document changed", cx);
                        }
                        match response {
                            Ok(response) if response.has_edits() => {
                                ws.stage_language_edits((*request).clone(), response, window, cx)
                            }
                            Ok(_) => ws.status_message("Language server command completed", cx),
                            Err(e) => ws.status_message(e, cx),
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    /// Commit the language server's item `index` from the list: resolved first when the
    /// server can, then its edit (the server's range, else the word before the caret) and its
    /// additional ones, as one undo step, the caret after the item's text. It says nothing when
    /// it goes in. They are for the text the items came for, so they go through `since`, what
    /// the list's typing did after, and the word's end to the caret.
    pub(crate) fn commit_server_item(
        &mut self,
        index: usize,
        since: Vec<crate::editing::text::Change>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let discarded = "Completion discarded: document changed";
        let Some((request, encoding, items)) = self.language.completion.take() else {
            return;
        };
        // the same server and document as the items', at the commit's text and caret
        let now = self.language_request(cx).ok().filter(|(_, now)| {
            now.generation == request.generation
                && now.service_generation == request.service_generation
                && now.document.path == request.document.path
                && now.document.buffer == request.document.buffer
        });
        let (Some((input, now)), Some(item)) = (now, items.into_iter().nth(index)) else {
            return self.status_message(discarded, cx);
        };
        let origin = self.active_editor().map(|e| {
            let view = e.read(cx);
            (e.entity_id(), view.completion_epoch, view.state.selections.clone(), view.state.primary())
        });
        let manager = self.language.manager.clone();
        let snapshot = now.clone();
        let task = cx.background_spawn(async move {
            let mut manager = manager.lock().unwrap_or_else(|e| e.into_inner());
            let mut item = item;
            if item.get("data").is_some() && manager.can_resolve(&snapshot.document, "completionProvider") {
                if let Ok(response) = manager.request(&input, &snapshot, "completionItem/resolve", item.clone()) {
                    item = response.value;
                }
            }
            // the edits are for the text the items came for: what was typed since moves them,
            // and what was typed at the caret is part of the word they replace
            // applied at the primary caret only; repeat the edit at the other carets that typed
            // the same word if multi-caret LSP commits are wanted
            let (text, caret) = (request.document.coords(), request.cursor);
            let map = |offset| crate::editing::text::map_offset(offset, &since);
            let edits = language::completion_edits(text, caret, &item, encoding)?;
            let snippet = if item["insertTextFormat"] == 2 {
                let mut primary_item = item.clone();
                primary_item.as_object_mut().ok_or("Invalid completion item")?.remove("additionalTextEdits");
                let primary = language::completion_edits(text, caret, &primary_item, encoding)?.remove(0);
                Some(edits.iter().position(|e| *e == primary).ok_or("Missing primary completion edit")?)
            } else {
                None
            };
            let to = |end| if end == caret { snapshot.cursor } else { map(end) };
            let edits = edits.into_iter().map(|(r, text)| (map(r.start)..to(r.end), text)).collect::<Vec<_>>();
            Ok::<_, String>((edits, snippet))
        });
        cx.spawn_in(window, async move |this, cx| {
            let edits = task.await;
            #[cfg(test)]
            crate::documents::save::gate::wait(&tests::completion_gate_path(&now.document.path)).await;
            this.update_in(cx, |ws, _, cx| {
                // A mouse move into a gap can leave the real cursor and document unchanged
                // after the popup was taken. Only completion commits require this full view match.
                let current = ws.active_editor().is_some_and(|e| {
                    let view = e.read(cx);
                    origin.as_ref().is_some_and(|(id, epoch, selections, primary)| {
                        *id == e.entity_id()
                            && *epoch == view.completion_epoch
                            && *selections == view.state.selections
                            && *primary == view.state.primary()
                    })
                });
                if !current || !ws.language_request_current(&now, cx) {
                    return ws.status_message(discarded, cx);
                }
                match edits {
                    Ok((mut edits, snippet)) => {
                        let editor = ws.active_editor().unwrap();
                        if editor.read(cx).doc(cx).read_only || editor.read(cx).doc(cx).edits_blocked() {
                            return ws.status_message("Document is read only", cx);
                        }
                        let result = editor.update(cx, |e, cx| {
                            let vars = snippet.map(|_| e.snippet_variables(cx));
                            let mut result = Ok(());
                            e.edit(cx, |s| {
                                if let Some(index) = snippet {
                                    let (range, body) = edits.remove(index);
                                    result = s
                                        .complete_snippet_ranges_with(range, &body, edits, &|name| {
                                            vars.as_ref().and_then(|v| v.resolve(name))
                                        })
                                        .map(|_| ());
                                } else {
                                    s.complete_ranges(edits);
                                }
                            });
                            result
                        });
                        if let Err(error) = result {
                            ws.status_message(error, cx);
                        }
                    }
                    Err(error) => ws.status_message(error, cx),
                }
            })
            .ok();
        })
        .detach();
    }
}

struct PreparedEdit {
    path: PathBuf,
    before: Option<Document>,
    loaded: Option<Loaded>,
    edits: Vec<(Range<usize>, String)>,
}

fn prepare_edits(request: &Request, response: Response) -> Result<Vec<PreparedEdit>, String> {
    if response.value.is_null() {
        return Ok(Vec::new());
    }
    let mut prepared = Vec::new();
    let started = request.started.duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?;
    let started = (started.as_secs() as i64, started.subsec_nanos() as i64);
    let mut seen = HashSet::new();
    for (uri, version, edits) in lsp::workspace_edits(&response.value)? {
        let path = crate::editing::text::resolve(&lsp::path(&uri)?);
        if !path.starts_with(&response.root) {
            return Err(format!("Server edit is outside the project: {}", path.display()));
        }
        if !seen.insert(path.clone()) {
            return Err("Server edited the same file through several URIs".into());
        }
        let before = request.documents.iter().find(|d| d.path == path).cloned();
        if let Some(version) = version {
            let (buffer, editor_version, serial) =
                response.versions.get(&uri).ok_or("Server supplied a version for an unsynchronized document")?;
            if *serial != version || before.as_ref().is_none_or(|d| d.buffer != *buffer || d.version != *editor_version)
            {
                return Err("Server edit has a stale document version".into());
            }
        }
        let loaded = if before.is_none() {
            let loaded = Loaded::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if loaded.disk.is_none_or(|stamp| stamp.ctime > started || stamp.mtime > started)
                || loaded.disk != crate::files::watch::stamp(&path)
            {
                return Err(format!("File changed during language request: {}", path.display()));
            }
            Some(loaded)
        } else {
            None
        };
        let (text, read_only) = if let Some(before) = &before {
            (before.coords(), before.read_only)
        } else {
            // the file as read: no editor version of it exists
            let loaded = loaded.as_ref().unwrap();
            (Coords::new(&loaded.rope, 0), loaded.read_only)
        };
        if read_only {
            return Err(format!("Document is read only: {}", path.display()));
        }
        let edits = lsp::text_edits(text, &edits, response.encoding)?;
        if !edits.is_empty() {
            prepared.push(PreparedEdit { path, before, loaded, edits });
        }
    }
    Ok(prepared)
}

impl Workspace {
    fn stage_language_edits(
        &mut self,
        request: Request,
        mut response: Response,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = response.command.take();
        let snapshot = request.clone();
        let task = cx.background_spawn(async move { prepare_edits(&snapshot, response) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, window, cx| {
                if !ws.language_request_current(&request, cx) {
                    return ws.status_message("Server edits discarded: document changed", cx);
                }
                match result {
                    Ok(edits) => match ws.apply_language_edits(edits, window, cx) {
                        Err(e) => ws.status_message(e, cx),
                        Ok(()) => {
                            if let Some(command) = command {
                                cx.defer_in(window, move |ws, window, cx| {
                                    ws.execute_language_command(command, window, cx)
                                });
                            }
                        }
                    },
                    Err(e) => ws.status_message(e, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_language_edits(
        &mut self,
        prepared: Vec<PreparedEdit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let editors = self.open_editors(cx);
        // every edit is checked before any buffer is opened or changed
        for edit in &prepared {
            if super::pipelines::reading_blocked(&edit.path, cx) {
                return Err("A file is being written; request the edit again".into());
            }
            let current = editors.iter().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&edit.path));
            match (&edit.before, current) {
                (Some(before), Some(editor))
                    if before.matches_buffer(editor.read(cx), cx)
                        && !editor.read(cx).doc(cx).read_only
                        && !editor.read(cx).doc(cx).edits_blocked() => {}
                (None, None)
                    if edit
                        .loaded
                        .as_ref()
                        .is_some_and(|loaded| loaded.disk == crate::files::watch::stamp(&edit.path)) => {}
                _ => return Err(format!("Server edits discarded: {} changed, opened or closed", edit.path.display())),
            }
            if editors.iter().filter(|e| e.read(cx).doc(cx).path.as_ref() == Some(&edit.path)).any(|e| {
                e.read(cx).doc(cx).read_only
                    || e.read(cx).doc(cx).edits_blocked()
                    || edit.before.as_ref().is_none_or(|before| !before.matches_buffer(e.read(cx), cx))
            }) {
                return Err(
                    "Different buffers have the same file path; close the duplicate before applying server edits"
                        .into(),
                );
            }
        }
        let count = prepared.len();
        let origin = self.active_editor();
        for mut edit in prepared {
            let editor =
                if let Some(editor) = editors.iter().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&edit.path)) {
                    editor.clone()
                } else {
                    let loaded = edit.loaded.take().ok_or("Missing file contents")?;
                    self.attach_loaded(&edit.path, loaded, window, cx)
                };
            editor.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.replace_ranges(edit.edits);
                })
            });
        }
        if let Some(origin) = origin {
            if let Some((group, index)) = self.views.find(&origin) {
                self.views.select(group, index);
                self.focus_editor(window, cx);
            }
        }
        self.status_message(
            if count == 0 {
                "No edits returned".into()
            } else {
                format!("Language edits applied to {count} file(s); save to write them")
            },
            cx,
        );
        Ok(())
    }

    fn language_location(
        &mut self,
        uri: String,
        range: lsp::Span,
        encoding: Encoding,
        side: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_active(cx, |editor, cx| {
            editor.catch_up(cx);
            editor.sel(cx).sel_checkpoint();
        });
        let request = self.language_request(cx).ok().map(|(_, mut request)| {
            request.navigation_source = self.active_editor().map(|editor| {
                let view = editor.read(cx);
                (editor.entity_id(), view.read_state(cx).selection_generation(), view.read_state(cx).selection_state())
            });
            request
        });
        let path = match lsp::path(&uri) {
            Ok(p) => crate::editing::text::resolve(&p),
            Err(e) => return self.status_message(e, cx),
        };
        if let Some(editor) = self.editor_at(&path, cx) {
            let range = match lsp::range(editor.read(cx).doc(cx).coords(), range, encoding) {
                Ok(range) => range.start.0..range.end.0,
                Err(error) => return self.status_message(error, cx),
            };
            if side {
                self.definition_side(window, cx);
            }
            let group = side.then_some(self.views.active);
            return self.reveal_language_location(editor, range, group, window, cx);
        }
        if super::pipelines::reading_blocked(&path, cx) {
            return self.status_message("File is being written; try navigation again", cx);
        }
        let generation = (self.restore_gen, self.language.navigation_generation);
        let active = self.active_editor();
        let observed = super::pipelines::observe_read(&path, cx);
        let p = path.clone();
        let task = cx.background_spawn(async move { Loaded::read(&p) });
        cx.spawn_in(window, async move |this, cx| {
            let loaded = task.await;
            this.update_in(cx, |ws, window, cx| {
                if generation != (ws.restore_gen, ws.language.navigation_generation)
                    || active != ws.active_editor()
                    || request.as_ref().is_some_and(|request| !ws.language_request_current(request, cx))
                {
                    return;
                }
                let loaded = match loaded {
                    Ok(l) => l,
                    Err(e) => return ws.status_message(format!("{}: {e}", path.display()), cx),
                };
                if super::pipelines::reading_blocked(&path, cx) || observed.changed() {
                    return ws.status_message("File changed or is being written; try navigation again", cx);
                }
                let editor = ws.editor_at(&path, cx);
                let target = match &editor {
                    Some(editor) => lsp::range(editor.read(cx).doc(cx).coords(), range, encoding),
                    None => lsp::range(Coords::new(&loaded.rope, 0), range, encoding),
                };
                let range = match target {
                    Ok(range) => range.start.0..range.end.0,
                    Err(error) => return ws.status_message(error, cx),
                };
                // moving to the side group changes the focus, which drops replies: only after
                // the check above
                if side {
                    ws.definition_side(window, cx);
                }
                let group = side.then_some(ws.views.active);
                let editor = editor.unwrap_or_else(|| ws.attach_loaded(&path, loaded, window, cx));
                ws.reveal_language_location(editor, range, group, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The open editor of `path`.
    fn editor_at(&self, path: &Path, cx: &App) -> Option<Entity<Editor>> {
        (self.open_editors(cx).into_iter()).find(|e| e.read(cx).doc(cx).path.as_deref() == Some(path))
    }

    /// A new editor of the file at `path`, as read, in a new tab.
    fn attach_loaded(
        &mut self,
        path: &Path,
        loaded: Loaded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let editor = self.new_editor(EditorState::from_loaded(path, loaded), cx);
        self.drop_blank_untitled(cx);
        self.attach(editor.clone(), false, window, cx);
        editor
    }

    fn reveal_language_location(
        &mut self,
        editor: Entity<Editor>,
        range: Range<usize>,
        group: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(group) = group {
            if group >= self.views.groups.len() {
                return self.status_message("Destination group closed", cx);
            }
            self.views.focus(group);
        }
        let here = self.views.find(&editor).filter(|(g, _)| group.is_none_or(|wanted| *g == wanted));
        let local = group.and_then(|g| {
            self.views.groups[g].tabs.iter().find(|view| view.read(cx).same_document(editor.read(cx))).cloned()
        });
        let editor = if let Some(local) = local {
            let (g, i) = self.views.find(&local).unwrap();
            self.views.select(g, i);
            local
        } else if let Some((group, index)) = here {
            self.views.select(group, index);
            editor
        } else {
            let view = editor.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
            self.drop_blank_untitled(cx);
            self.attach(view.clone(), false, window, cx);
            view
        };
        editor.update(cx, |e, cx| {
            e.jump_to(range.start, cx);
            e.select_range(range, cx);
        });
        self.focus_editor(window, cx);
        self.status = None;
        cx.notify();
    }
}
