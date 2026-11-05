//! Bounded stdio LSP transport and document-coordinate validation.
#[cfg(test)]
mod real_servers;
use orlo_editor::text::{ByteOffset, CharOffset, Coords, Point, Utf16Offset};
use orlo_utils::text::diff;
use ropey::{Rope, RopeSlice};
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_MESSAGE: usize = 16 << 20;
const MAX_HEADER: usize = 8192;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}
#[derive(Debug)]
pub enum Event {
    Notification { method: String, params: Value },
    Error(String),
}

struct Transport {
    outgoing: Mutex<Option<mpsc::SyncSender<Value>>>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>,
    events: Mutex<VecDeque<(usize, Event)>>,
    stderr: Mutex<VecDeque<u8>>,
    next_id: AtomicU64,
    closed: AtomicBool,
}
impl Transport {
    fn send(&self, message: Value) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("Language server connection closed".into());
        }
        self.outgoing
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("Language server connection closed")?
            .try_send(message)
            .map_err(|e| format!("Language server write queue: {e}"))
    }
    fn event(&self, event: Event, bytes: usize) {
        let mut events = self.events.lock().unwrap();
        let mut total: usize = events.iter().map(|(size, _)| size).sum();
        while events.len() >= 256 || total.saturating_add(bytes) > MAX_MESSAGE * 2 {
            let Some((size, _)) = events.pop_front() else {
                break;
            };
            total -= size;
        }
        events.push_back((bytes, event));
    }
    fn fail(&self, message: String) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let stderr: Vec<u8> = self.stderr.lock().unwrap().iter().copied().collect();
        let message =
            if stderr.is_empty() { message } else { format!("{message}: {}", String::from_utf8_lossy(&stderr).trim()) };
        for (_, sender) in self.pending.lock().unwrap().drain() {
            let _ = sender.send(Err(message.clone()));
        }
        self.event(Event::Error(message), 8192);
    }
    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        self.request_with_cancel(method, params, timeout, None)
    }

    fn request_with_cancel(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if self.closed.load(Ordering::Acquire) {
                return Err("Language server connection closed".into());
            }
            if pending.len() >= 128 {
                return Err("Too many pending language server requests".into());
            }
            pending.insert(id, sender);
        }
        if let Err(error) = self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})) {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let cancelled = cancel.is_some_and(|flag| flag.load(Ordering::Relaxed));
            let remaining = deadline.saturating_duration_since(Instant::now());
            if cancelled || remaining.is_zero() {
                self.pending.lock().unwrap().remove(&id);
                let _ = self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":id}}));
                return Err(if cancelled {
                    "Language request cancelled".into()
                } else {
                    format!("Language server {method}: timed out")
                });
            }
            let wait = if cancel.is_some() { remaining.min(Duration::from_millis(25)) } else { remaining };
            match receiver.recv_timeout(wait) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => return Err(format!("Language server {method}: {error}")),
            }
        }
    }
}
struct Process {
    child: Child,
    threads: Vec<JoinHandle<()>>,
}

pub struct Client {
    pub capabilities: Value,
    pub encoding: Encoding,
    transport: Arc<Transport>,
    process: Option<Process>,
}
impl Client {
    /// Call on a worker thread: initialization waits for the server, up to ten seconds.
    pub fn start(command: &[String], root: &Path, initialization_options: Value) -> Result<Self, String> {
        let executable = command.first().filter(|s| !s.is_empty()).ok_or("Empty language server command")?;
        let root_uri = uri(root)?;
        let folders = json!([{"uri":root_uri,"name":root.file_name().unwrap_or_default().to_string_lossy()}]);
        let mut process = Command::new(executable);
        process
            .args(&command[1..])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            process.process_group(0);
        }
        let mut child = process.spawn().map_err(|e| format!("Cannot start {executable}: {e}"))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (sender, receiver) = mpsc::sync_channel::<Value>(128);
        let transport = Arc::new(Transport {
            outgoing: Mutex::new(Some(sender)),
            pending: Mutex::new(HashMap::new()),
            events: Mutex::new(VecDeque::new()),
            stderr: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        });
        let writer_transport = transport.clone();
        let writer = thread::spawn(move || {
            for message in receiver {
                let result = (|| {
                    let body = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
                    if body.len() > MAX_MESSAGE {
                        return Err("Outgoing LSP message exceeds 16 MiB".into());
                    }
                    write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).map_err(|e| e.to_string())?;
                    stdin.write_all(&body).map_err(|e| e.to_string())?;
                    stdin.flush().map_err(|e| e.to_string())
                })();
                if let Err(error) = result {
                    writer_transport.fail(format!("Language server write failed: {error}"));
                    break;
                }
            }
        });
        let stderr_transport = transport.clone();
        let stderr_thread = thread::spawn(move || {
            let mut buffer = [0u8; 2048];
            while let Ok(size) = stderr.read(&mut buffer) {
                if size == 0 {
                    break;
                }
                let mut tail = stderr_transport.stderr.lock().unwrap();
                tail.extend(&buffer[..size]);
                while tail.len() > 8192 {
                    tail.pop_front();
                }
            }
        });
        let reader_transport = transport.clone();
        let config = initialization_options.clone();
        let reader_folders = folders.clone();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let (message, size) = match read_message(&mut reader) {
                    Ok(message) => message,
                    Err(error) => {
                        reader_transport.fail(error);
                        break;
                    }
                };
                if let Err(error) = receive(&reader_transport, message, size, &config, &reader_folders) {
                    reader_transport.fail(error);
                    break;
                }
            }
        });
        let mut client = Self {
            capabilities: Value::Null,
            encoding: Encoding::Utf16,
            transport,
            process: Some(Process { child, threads: vec![writer, reader, stderr_thread] }),
        };
        let initialized = client.request(
            "initialize",
            json!({
                "processId":std::process::id(), "clientInfo":{"name":"Orlo","version":env!("CARGO_PKG_VERSION")},
                "rootUri":root_uri,"workspaceFolders":folders,"initializationOptions":initialization_options,
                "capabilities":capabilities()
            }),
        )?;
        client.capabilities = initialized
            .get("capabilities")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or("Language server initialize omitted capabilities")?;
        client.encoding = match client.capabilities.get("positionEncoding").and_then(Value::as_str) {
            None | Some("utf-16") => Encoding::Utf16,
            Some("utf-8") => Encoding::Utf8,
            Some("utf-32") => Encoding::Utf32,
            Some(other) => return Err(format!("Unsupported language server position encoding: {other}")),
        };
        client.notify("initialized", json!({}))?;
        Ok(client)
    }
    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.transport.request(method, params, REQUEST_TIMEOUT)
    }
    pub fn request_cancellable(&self, method: &str, params: Value, cancel: &AtomicBool) -> Result<Value, String> {
        self.transport.request_with_cancel(method, params, REQUEST_TIMEOUT, Some(cancel))
    }
    /// A caller-owned deadline, without changing the normal language request timeout.
    pub fn request_cancellable_for(
        &self,
        method: &str,
        params: Value,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.transport.request_with_cancel(method, params, timeout, Some(cancel))
    }
    pub fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.transport.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    pub fn drain_events(&self) -> Vec<Event> {
        self.transport.events.lock().unwrap().drain(..).map(|(_, event)| event).collect()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let Some(mut process) = self.process.take() else {
            return;
        };
        let transport = self.transport.clone();
        // Reaping and an unresponsive server must never stall the editor thread.
        thread::spawn(move || {
            let _ = transport.request("shutdown", Value::Null, Duration::from_millis(500));
            let _ = transport.send(json!({"jsonrpc":"2.0","method":"exit","params":null}));
            transport.outgoing.lock().unwrap().take();
            let deadline = Instant::now() + Duration::from_millis(500);
            loop {
                match process.child.try_wait() {
                    Ok(Some(_)) => break,
                    _ if Instant::now() >= deadline => {
                        let _ = process.child.kill();
                        let _ = process.child.wait();
                        break;
                    }
                    _ => thread::sleep(Duration::from_millis(10)),
                }
            }
            // Server subprocesses can inherit the pipes even after their parent exits.
            #[cfg(unix)]
            unsafe {
                libc::kill(-(process.child.id() as libc::pid_t), libc::SIGKILL);
            }
            transport.fail("Language server stopped".into());
            for thread in process.threads {
                let _ = thread.join();
            }
        });
    }
}

fn capabilities() -> Value {
    json!({
        "general":{"positionEncodings":["utf-8","utf-16","utf-32"]},
        "workspace":{"applyEdit":false,"workspaceEdit":{"documentChanges":true,"resourceOperations":[]},"configuration":true,"workspaceFolders":true},
        "textDocument":{
            "synchronization":{"didSave":true},
            "publishDiagnostics":{"versionSupport":true},
            "completion":{"completionItem":{"snippetSupport":true,"documentationFormat":["plaintext"],
                "resolveSupport":{"properties":["detail","documentation","additionalTextEdits"]}}},
            "definition":{"linkSupport":false},"typeDefinition":{"linkSupport":true},
            "implementation":{"linkSupport":true},"references":{},"hover":{"contentFormat":["plaintext"]},
            "signatureHelp":{"contextSupport":true,"signatureInformation":{"documentationFormat":["plaintext"],"parameterInformation":{"labelOffsetSupport":true},"activeParameterSupport":true}},
            "formatting":{},"rangeFormatting":{},"rename":{"prepareSupport":false},
            "codeAction":{"dataSupport":true,"resolveSupport":{"properties":["edit"]},
                "codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix","refactor","refactor.extract","refactor.inline","refactor.rewrite","source","source.organizeImports","source.fixAll"]}}},
            "semanticTokens":{"requests":{"range":false,"full":true},"formats":["relative"],
                "tokenTypes":["namespace","type","class","enum","interface","struct","typeParameter","parameter","variable","property","enumMember","event","function","method","macro","keyword","modifier","comment","string","number","regexp","operator","decorator"],
                "tokenModifiers":["declaration","definition","readonly","static","deprecated","abstract","async","modification","documentation","defaultLibrary"],
                "multilineTokenSupport":false,"overlappingTokenSupport":false}
        }
    })
}

fn receive(transport: &Transport, message: Value, bytes: usize, config: &Value, folders: &Value) -> Result<(), String> {
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !message.is_object() {
        return Err("Invalid LSP JSON-RPC message".into());
    }
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = message.get("id") {
            if !id.is_string() && id.as_i64().is_none() {
                return Err("Invalid LSP server request id".into());
            }
            let mut response = match method {
                "workspace/configuration" => {
                    let items =
                        params.get("items").and_then(Value::as_array).ok_or("Invalid workspace/configuration items")?;
                    let values: Vec<_> = items
                        .iter()
                        .map(|item| {
                            item.get("section")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .and_then(|section| section.split('.').try_fold(config, |v, key| v.get(key)))
                                .unwrap_or(config)
                                .clone()
                        })
                        .collect();
                    json!({"result":values})
                }
                "workspace/workspaceFolders" => json!({"result":folders}),
                "client/registerCapability" | "client/unregisterCapability" | "window/workDoneProgress/create" => {
                    json!({"result":null})
                }
                "workspace/applyEdit" => {
                    json!({"result":{"applied":false,"failureReason":"Orlo applies edits only after an explicit editor command"}})
                }
                _ => {
                    json!({"error":{"code":-32601,"message":format!("Unsupported client request: {method}")}})
                }
            };
            response["jsonrpc"] = json!("2.0");
            response["id"] = id.clone();
            transport.send(response)?;
        } else {
            transport.event(Event::Notification { method: method.to_owned(), params }, bytes);
        }
    } else {
        let id = message.get("id").and_then(Value::as_u64).ok_or("Invalid LSP response id")?;
        let result = match (message.get("result"), message.get("error")) {
            (Some(result), None) => Ok(result.clone()),
            (None, Some(error)) => Err(format!(
                "Language server error {}: {}",
                error.get("code").unwrap_or(&Value::Null),
                error.get("message").and_then(Value::as_str).unwrap_or("Unknown error")
            )),
            _ => return Err("Invalid LSP response result/error".into()),
        };
        if let Some(sender) = transport.pending.lock().unwrap().remove(&id) {
            let _ = sender.send(result);
        }
    }
    Ok(())
}

/// A message and its body's length in bytes.
fn read_message(reader: &mut impl BufRead) -> Result<(Value, usize), String> {
    let mut length = None;
    let mut header_bytes = 0usize;
    loop {
        let mut line = Vec::new();
        reader
            .take((MAX_HEADER - header_bytes + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("Language server read failed: {e}"))?;
        if line.is_empty() {
            return Err("Language server closed stdout".into());
        }
        header_bytes += line.len();
        if header_bytes > MAX_HEADER || !line.ends_with(b"\r\n") || !line.is_ascii() {
            return Err("Invalid or oversized LSP header".into());
        }
        if line == b"\r\n" {
            break;
        }
        let line = std::str::from_utf8(&line[..line.len() - 2]).map_err(|_| "Invalid LSP header")?;
        let (key, value) = line.split_once(':').ok_or("Invalid LSP header")?;
        if key.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err("Duplicate LSP Content-Length".into());
            }
            // digits only: `parse` would also take a sign
            let size = Some(value.trim())
                .filter(|v| v.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or("Invalid LSP Content-Length")?;
            if size == 0 || size > MAX_MESSAGE {
                return Err("LSP message exceeds 16 MiB or is empty".into());
            }
            length = Some(size);
        } else if key.eq_ignore_ascii_case("Content-Type") {
            for parameter in value.split(';').skip(1) {
                if let Some((name, value)) = parameter.trim().split_once('=') {
                    if name.eq_ignore_ascii_case("charset")
                        && !matches!(value.trim().to_ascii_lowercase().as_str(), "utf-8" | "utf8")
                    {
                        return Err("Unsupported LSP content encoding".into());
                    }
                }
            }
        }
    }
    let mut body = vec![0; length.ok_or("Missing LSP Content-Length")?];
    reader.read_exact(&mut body).map_err(|e| format!("Incomplete LSP message: {e}"))?;
    let message = serde_json::from_slice(&body).map_err(|e| format!("Invalid LSP JSON: {e}"))?;
    Ok((message, body.len()))
}

/// A protocol position: a 0-based line, and a column in the code units of the encoding agreed
/// with that server (`Client::encoding`): UTF-8 bytes, UTF-16 units or chars. It means
/// something only in that encoding and in the text it was taken from, so it becomes a
/// `CharOffset` only through `offset`, against that text's `Coords`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}

/// A protocol range: two `Position`s in the same encoding and text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

impl Position {
    pub fn new(line: usize, character: usize) -> Self {
        Self { line, character }
    }

    /// A server's `{"line", "character"}`: both non-negative integers.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let number = |key| {
            value
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| format!("Invalid LSP position {key}"))
        };
        Ok(Self::new(number("line")?, number("character")?))
    }
}

impl Span {
    /// A server's `{"start", "end"}`; their order is checked against the text, by `range`.
    pub fn parse(value: &Value) -> Result<Self, String> {
        Ok(Self {
            start: Position::parse(value.get("start").ok_or("Missing LSP range start")?)?,
            end: Position::parse(value.get("end").ok_or("Missing LSP range end")?)?,
        })
    }
}

/// `encoding`'s code units from the start of `text` to `offset`.
fn units(text: Coords, offset: CharOffset, encoding: Encoding) -> usize {
    match encoding {
        Encoding::Utf8 => text.char_to_byte(offset).0,
        Encoding::Utf16 => text.char_to_utf16(offset).0,
        Encoding::Utf32 => offset.0,
    }
}

/// The char that starts at code unit `unit`; `None` when `unit` falls inside a char.
fn char_at(text: Coords, unit: usize, encoding: Encoding) -> Option<CharOffset> {
    let offset = match encoding {
        Encoding::Utf8 => text.byte_to_char(ByteOffset(unit)),
        Encoding::Utf16 => text.utf16_to_char(Utf16Offset(unit)),
        Encoding::Utf32 => CharOffset(unit),
    };
    (units(text, offset, encoding) == unit).then_some(offset)
}

/// The protocol's lines of a text with a `\r` not before a `\n`, which ends a line there while
/// the rope's lines go on: each line's start, and its end before its break.
struct CrLines {
    starts: Vec<usize>,
    ends: Vec<usize>,
}

impl CrLines {
    /// `None` for a text with no lone `\r`, whose lines are the rope's.
    fn of(rope: &Rope) -> Option<Self> {
        // a byte scan first, skipping chunks with no `\r`: most texts have none
        let mut after_cr = false;
        let lone = rope.chunks().any(|chunk| {
            let bytes = chunk.as_bytes();
            if !after_cr && !bytes.contains(&b'\r') {
                return false;
            }
            bytes.iter().any(|&b| {
                let lone = after_cr && b != b'\n';
                after_cr = b == b'\r';
                lone
            })
        }) || after_cr;
        if !lone {
            return None;
        }
        let mut chars = rope.chars().enumerate().peekable();
        let (mut starts, mut ends) = (vec![0], Vec::new());
        while let Some((i, c)) = chars.next() {
            let next = match c {
                '\r' if chars.next_if(|(_, c)| *c == '\n').is_some() => i + 2,
                '\r' | '\n' => i + 1,
                _ => continue,
            };
            ends.push(i);
            starts.push(next);
        }
        ends.push(rope.len_chars());
        Some(Self { starts, ends })
    }

    /// The lines of `rope`, for the last few ropes asked about: one response converts many
    /// positions in one text, and a rope's clone is the same text until either is edited.
    fn get(rope: &Rope) -> Option<Rc<Self>> {
        thread_local! {
            static SEEN: RefCell<VecDeque<(Rope, Option<Rc<CrLines>>)>> = RefCell::default();
        }
        SEEN.with_borrow_mut(|seen| {
            if let Some((_, lines)) = seen.iter().find(|(r, _)| r.is_instance(rope)) {
                return lines.clone();
            }
            let lines = Self::of(rope).map(Rc::new);
            if seen.len() == 4 {
                seen.pop_front();
            }
            seen.push_back((rope.clone(), lines.clone()));
            lines
        })
    }
}

/// The protocol's line `offset` is on: a `\n`, a `\r\n` and a lone `\r` each end one.
fn line_of(text: Coords, offset: CharOffset) -> usize {
    match CrLines::get(text.rope()) {
        Some(lines) => lines.starts.partition_point(|&s| s <= offset.0) - 1,
        None => text.char_to_point(offset).line,
    }
}

/// The protocol's last line.
fn last_line(text: Coords) -> usize {
    line_of(text, text.len())
}

/// Where the protocol's `line` starts, and where it ends before its `\n`, `\r\n` or `\r`.
fn line_bounds(text: Coords, line: usize) -> Range<CharOffset> {
    match CrLines::get(text.rope()) {
        Some(lines) => CharOffset(lines.starts[line])..CharOffset(lines.ends[line]),
        None => text.point_to_char(Point::new(line, 0))..text.point_to_char(Point::new(line, usize::MAX)),
    }
}

/// `offset` in `text` as a position in `encoding`. Inside a CRLF it is the line's end.
pub fn position(text: Coords, offset: CharOffset, encoding: Encoding) -> Position {
    assert!(offset <= text.len(), "LSP offset outside document");
    let line = line_of(text, offset);
    let bounds = line_bounds(text, line);
    let column = units(text, offset.min(bounds.end), encoding) - units(text, bounds.start, encoding);
    Position::new(line, column)
}

/// The char at `position` in `text`. A column past its line's end is the line's end, as the
/// protocol has it. A line past the last and a column inside a char are refused: the server is
/// describing other text.
pub fn offset(text: Coords, position: Position, encoding: Encoding) -> Result<CharOffset, String> {
    if position.line > last_line(text) {
        return Err("LSP line outside document".into());
    }
    let bounds = line_bounds(text, position.line);
    let end = units(text, bounds.end, encoding);
    let unit = units(text, bounds.start, encoding).saturating_add(position.character);
    if unit >= end {
        return Ok(bounds.end);
    }
    char_at(text, unit, encoding).ok_or_else(|| "LSP position splits a character".into())
}

/// `range` in `text` as a span in `encoding`.
pub fn span(text: Coords, range: Range<CharOffset>, encoding: Encoding) -> Span {
    Span { start: position(text, range.start, encoding), end: position(text, range.end, encoding) }
}

/// The chars `span` covers in `text`; refused as `offset` refuses, or when it is reversed.
pub fn range(text: Coords, span: Span, encoding: Encoding) -> Result<Range<CharOffset>, String> {
    let start = offset(text, span.start, encoding)?;
    let end = offset(text, span.end, encoding)?;
    if start > end {
        return Err("Reversed LSP range".into());
    }
    Ok(start..end)
}

/// A server's JSON range in `text`, as the char offsets the editor's own ranges use.
pub fn char_range(text: Coords, value: &Value, encoding: Encoding) -> Result<Range<usize>, String> {
    let r = range(text, Span::parse(value)?, encoding)?;
    Ok(r.start.0..r.end.0)
}

/// One change's JSON beyond its text: its range and keys, for weighing ranges against the whole
/// text.
const CHANGE_OVERHEAD: usize = 100;

/// The `contentChanges` that bring a server from `sent`, the text it last received, to `text`;
/// empty when they are the same text.
///
/// A server that syncs in full gets the whole text. One that syncs incrementally gets the
/// replacements between them, the last first: each range is in `sent`'s positions, which stay
/// right because a replacement never moves the text before it. It gets the whole text instead
/// when that is no longer than the ranges, or when either text has a `\r`, which ends a line
/// for the protocol but not for the rope's lines that positions count.
pub fn content_changes(sent: Coords, text: Coords, incremental: bool, encoding: Encoding) -> Vec<Value> {
    let (old, new) = (sent.rope(), text.rope());
    if old.is_instance(new) {
        return Vec::new();
    }
    let whole = || vec![json!({ "text": new.to_string() })];
    let has_cr = |s: RopeSlice| s.chunks().any(|c| c.as_bytes().contains(&b'\r'));
    if !incremental || has_cr(old.slice(..)) {
        return whole();
    }
    let changes = diff(old, new);
    if changes.is_empty() {
        return Vec::new();
    }
    // each change is where the ones before it left the text: back to `sent`'s offsets
    let (mut shift, mut bytes) = (0isize, 0usize);
    let mut ranges = Vec::with_capacity(changes.len());
    for c in &changes {
        let inserted = new.slice(c.start..c.start + c.inserted);
        if has_cr(inserted) {
            return whole();
        }
        bytes += inserted.len_bytes() + CHANGE_OVERHEAD;
        if bytes >= new.len_bytes() {
            return whole();
        }
        let start = c.start.wrapping_add_signed(-shift);
        ranges.push((CharOffset(start)..CharOffset(start + c.removed), inserted));
        shift += c.inserted as isize - c.removed as isize;
    }
    ranges
        .into_iter()
        .rev()
        .map(|(range, inserted)| json!({ "range": span(sent, range, encoding), "text": inserted.to_string() }))
        .collect()
}

pub fn uri(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("LSP file path must be absolute".into());
    }
    let value = path.to_str().ok_or("LSP file path is not UTF-8")?;
    if value.contains('\0') {
        return Err("NUL in LSP file path".into());
    }
    let mut result = String::from("file://");
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(result, "%{byte:02X}").unwrap();
        }
    }
    Ok(result)
}
pub fn path(uri: &str) -> Result<PathBuf, String> {
    let value = uri.strip_prefix("file://").ok_or("Only local file URIs are supported")?;
    let value = (value.strip_prefix("localhost/")).map_or_else(|| value.to_owned(), |v| format!("/{v}"));
    if !value.starts_with('/') || value.contains(['?', '#']) {
        return Err("Invalid or nonlocal file URI".into());
    }
    let mut bytes = Vec::new();
    let mut source = value.bytes();
    while let Some(byte) = source.next() {
        if byte == b'%' {
            let mut digit = || (source.next()).and_then(|b| (b as char).to_digit(16)).ok_or("Invalid file URI escape");
            bytes.push((digit()? * 16 + digit()?) as u8);
        } else {
            bytes.push(byte);
        }
    }
    if bytes.contains(&0) {
        return Err("NUL in file URI".into());
    }
    let path = PathBuf::from(String::from_utf8(bytes).map_err(|_| "File URI is not UTF-8")?);
    if !path.is_absolute() {
        return Err("File URI must be absolute".into());
    }
    Ok(path)
}

/// Validate the entire batch before mutation; output is in descending offset order.
pub fn text_edits(text: Coords, edits: &Value, encoding: Encoding) -> Result<Vec<(Range<usize>, String)>, String> {
    let mut result = Vec::new();
    for edit in edits.as_array().ok_or("LSP edits must be an array")? {
        if edit.get("annotationId").is_some() {
            return Err("Annotated LSP edits are unsupported".into());
        }
        result.push((
            char_range(text, edit.get("range").ok_or("Missing LSP edit range")?, encoding)?,
            edit.get("newText").and_then(Value::as_str).ok_or("Missing LSP edit text")?.to_owned(),
        ));
    }
    result.sort_by_key(|(range, _)| range.start);
    for pair in result.windows(2) {
        if pair[0].0.end > pair[1].0.start {
            return Err("Overlapping LSP edits".into());
        }
    }
    let mut merged: Vec<(Range<usize>, String)> = Vec::new();
    for (range, text) in result {
        if let Some((previous, inserted)) = merged.last_mut().filter(|(r, _)| r.start == range.start) {
            previous.end = range.end;
            inserted.push_str(&text);
        } else {
            merged.push((range, text));
        }
    }
    merged.reverse();
    Ok(merged)
}
/// Preserve document versions so the workspace can reject stale edits atomically.
pub fn workspace_edits(value: &Value) -> Result<Vec<(String, Option<i64>, Value)>, String> {
    let object = value.as_object().ok_or("LSP workspace edit must be an object")?;
    if object.get("changeAnnotations").is_some_and(|v| v.as_object().is_none_or(|m| !m.is_empty())) {
        return Err("Annotated workspace edits are unsupported".into());
    }
    if object.contains_key("changes") && object.contains_key("documentChanges") {
        return Err("Ambiguous workspace edit representations".into());
    }
    let mut result = Vec::new();
    if let Some(changes) = object.get("changes") {
        for (uri, edits) in changes.as_object().ok_or("Invalid LSP changes")? {
            result.push((uri.clone(), None, edits.clone()));
        }
    }
    if let Some(changes) = object.get("documentChanges") {
        for change in changes.as_array().ok_or("Invalid LSP documentChanges")? {
            if change.get("kind").is_some() {
                return Err("LSP file create, rename, and delete operations are unsupported".into());
            }
            let document = change.get("textDocument").ok_or("Missing LSP textDocument")?;
            let uri = document.get("uri").and_then(Value::as_str).ok_or("Missing LSP document URI")?;
            let version = match document.get("version").ok_or("Missing LSP document version")? {
                Value::Null => None,
                value => {
                    Some(value.as_i64().filter(|v| i32::try_from(*v).is_ok()).ok_or("Invalid LSP document version")?)
                }
            };
            result.push((uri.to_owned(), version, change.get("edits").ok_or("Missing LSP document edits")?.clone()));
        }
    }
    let mut documents = HashSet::new();
    for (uri, _, edits) in &result {
        if !documents.insert(path(uri)?) {
            return Err("Duplicate document in workspace edit".into());
        }
        for edit in edits.as_array().ok_or("LSP edits must be an array")? {
            if edit.get("annotationId").is_some() {
                return Err("Annotated LSP edits are unsupported".into());
            }
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// The fixture server's command line in `mode`.
    fn fixture(mode: &str) -> [String; 3] {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/lsp_server.py");
        ["python3".into(), script.into(), mode.into()]
    }

    fn server(mode: &str) -> Client {
        let options = json!({"analysis":{"enabled":true}});
        Client::start(&fixture(mode), Path::new("/tmp"), options).unwrap()
    }

    /// The process id the fixture answers `method` with.
    fn pid(client: &Client, method: &str) -> u64 {
        client.request(method, json!({})).unwrap().as_u64().unwrap()
    }

    fn alive(pid: u64) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
    const ENCODINGS: [Encoding; 3] = [Encoding::Utf8, Encoding::Utf16, Encoding::Utf32];

    /// `s` measured in `encoding`'s code units by the standard library, not the rope.
    fn units_of(s: &str, encoding: Encoding) -> usize {
        match encoding {
            Encoding::Utf8 => s.len(),
            Encoding::Utf16 => s.encode_utf16().count(),
            Encoding::Utf32 => s.chars().count(),
        }
    }

    #[test]
    fn positions_count_the_agreed_code_units_and_refuse_split_chars() {
        let texts = [
            "plain ascii\nsecond\n",
            // precomposed accents: 2 bytes, 1 UTF-16 unit
            "café naïve\nÅngström",
            // combining marks are chars of their own: a position may fall between them
            "e\u{301}cole n\u{303}o\n",
            // outside the basic plane: 4 bytes, 2 UTF-16 units, 1 char
            "😀 grin 𝄞 clef\n🦀\n",
            // a tab is one unit, however wide it is drawn
            "\tindent\t\tcolumn\n\t\n",
            "日本語\tテキスト\n한국어 中文\n",
            "\n\nempty lines around\n\n",
            "",
            "no trailing newline 🦀",
        ];
        for text in texts {
            let rope = Rope::from_str(text);
            let coords = Coords::new(&rope, 7);
            let chars: Vec<char> = text.chars().collect();
            let lines: Vec<&str> = text.split('\n').collect();
            for encoding in ENCODINGS {
                for o in 0..=chars.len() {
                    let before: String = chars[..o].iter().collect();
                    let column = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
                    let want = Position::new(before.matches('\n').count(), units_of(column, encoding));
                    let got = position(coords, CharOffset(o), encoding);
                    assert_eq!(got, want, "{text:?} char {o} in {encoding:?}");
                    assert_eq!(offset(coords, got, encoding), Ok(CharOffset(o)));
                }
                // every column on every line: accepted at char boundaries, refused inside a
                // char, and past the line's end its end (the protocol's rule)
                let mut start = 0;
                for (n, line) in lines.iter().enumerate() {
                    let boundaries: HashSet<usize> = line
                        .char_indices()
                        .map(|(i, _)| units_of(&line[..i], encoding))
                        .chain([units_of(line, encoding)])
                        .collect();
                    let end = CharOffset(start + line.chars().count());
                    for unit in 0..=units_of(line, encoding) + 2 {
                        let result = offset(coords, Position::new(n, unit), encoding);
                        let what = format!("{text:?} line {n} unit {unit} in {encoding:?}");
                        match unit > units_of(line, encoding) {
                            true => assert_eq!(result, Ok(end), "{what}"),
                            false => {
                                assert_eq!(result.is_ok(), boundaries.contains(&unit), "{what}")
                            }
                        }
                    }
                    start = end.0 + 1;
                }
                let past = Position::new(lines.len(), 0);
                assert!(offset(coords, past, encoding).is_err(), "{text:?}");
            }
        }
    }

    #[test]
    fn a_position_inside_a_crlf_is_its_line_end() {
        let rope = Rope::from_str("a😀é\r\nx\n");
        let coords = Coords::new(&rope, 1);
        for (encoding, column) in [(Encoding::Utf8, 7), (Encoding::Utf16, 4), (Encoding::Utf32, 3)] {
            let end = Position::new(0, column);
            // before the \r, and between the \r and the \n
            assert_eq!(position(coords, CharOffset(3), encoding), end);
            assert_eq!(position(coords, CharOffset(4), encoding), end);
            assert_eq!(offset(coords, end, encoding), Ok(CharOffset(3)));
            assert_eq!(offset(coords, Position::new(0, column + 1), encoding), Ok(CharOffset(3)));
            assert_eq!(position(coords, CharOffset(5), encoding), Position::new(1, 0));
        }
    }

    #[test]
    fn a_lone_cr_ends_a_line_for_the_protocol() {
        // the rope keeps a \r that is not before a \n as content; the protocol ends a line
        // there. Chars: a \r b 😀 \n c \r \n d
        let rope = Rope::from_str("a\rb😀\nc\r\nd");
        let coords = Coords::new(&rope, 1);
        for encoding in ENCODINGS {
            let emoji = units_of("😀", encoding);
            let want = [
                (0, 0),
                (0, 1),
                (1, 0),
                (1, 1),
                (1, 1 + emoji),
                (2, 0),
                // the \r of a \r\n, and between them: the line's end
                (2, 1),
                (2, 1),
                (3, 0),
                (3, 1),
            ];
            for (o, (line, column)) in want.into_iter().enumerate() {
                let got = position(coords, CharOffset(o), encoding);
                assert_eq!(got, Position::new(line, column), "char {o} in {encoding:?}");
                if o != 7 {
                    assert_eq!(offset(coords, got, encoding), Ok(CharOffset(o)));
                }
            }
            // a line's end stops at its \r, and nothing is past the fourth line
            assert_eq!(offset(coords, Position::new(0, 5), encoding), Ok(CharOffset(1)));
            assert!(offset(coords, Position::new(4, 0), encoding).is_err());
        }
        // ropes without a lone \r, and edited ones, keep their own lines
        let edited = {
            let mut r = rope.clone();
            r.remove(1..2);
            r
        };
        let coords = Coords::new(&edited, 2);
        assert_eq!(position(coords, CharOffset(1), Encoding::Utf32), Position::new(0, 1));
    }

    #[test]
    fn protocol_positions_parse_strictly_and_serialize_as_the_protocol_names_them() {
        assert_eq!(
            serde_json::to_value(Span { start: Position::new(1, 2), end: Position::new(3, 4) }).unwrap(),
            json!({"start":{"line":1,"character":2},"end":{"line":3,"character":4}})
        );
        assert_eq!(Position::parse(&json!({"line":2,"character":5})), Ok(Position::new(2, 5)));
        for p in [
            json!({"line":-1,"character":0}),
            json!({"line":0,"character":1.5}),
            json!({"line":"0","character":0}),
            json!({"character":0}),
            json!(null),
        ] {
            assert!(Position::parse(&p).is_err(), "{p}");
        }
        assert!(Span::parse(&json!({"start":{"line":0,"character":0}})).is_err());
        let rope = Rope::from_str("a😀b\nc");
        let coords = Coords::new(&rope, 3);
        for encoding in ENCODINGS {
            let s = span(coords, CharOffset(1)..CharOffset(4), encoding);
            assert_eq!(range(coords, s, encoding), Ok(CharOffset(1)..CharOffset(4)));
            let reversed = Span { start: s.end, end: s.start };
            assert!(range(coords, reversed, encoding).is_err());
        }
    }
    #[test]
    fn file_uris_roundtrip_unicode_reserved_bytes_and_reject_other_schemes() {
        let file = Path::new("/tmp/a b#?%/😀.rs");
        assert_eq!(uri(file).unwrap(), "file:///tmp/a%20b%23%3F%25/%F0%9F%98%80.rs");
        assert_eq!(path("file:///tmp/a%20b%23%3F%25/%F0%9F%98%80.rs").unwrap(), file);
        assert_eq!(path("file://localhost/tmp/a").unwrap(), Path::new("/tmp/a"));
        for invalid in [
            "https://example.org/a",
            "file://remote/tmp/a",
            "file:///tmp/%GG",
            "file:///tmp/%00",
            "file:///tmp/a#fragment",
            "file:relative",
        ] {
            assert!(path(invalid).is_err(), "{invalid}");
        }
        assert!(uri(Path::new("relative")).is_err());
        assert!(uri(Path::new("/tmp/has\0nul")).is_err());
    }
    #[test]
    fn edits_validate_the_whole_batch_before_returning_descending_ranges() {
        let rope = Rope::from_str("a😀bc\n");
        let text = Coords::new(&rope, 1);
        let edits = json!([
            {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"A"},
            {"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":4}},"newText":"B"}
        ]);
        assert_eq!(text_edits(text, &edits, Encoding::Utf16).unwrap(), vec![(2..3, "B".into()), (0..1, "A".into())]);
        let overlap = json!([
            {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"x"},
            {"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":4}},"newText":"y"}
        ]);
        assert!(text_edits(text, &overlap, Encoding::Utf16).is_err());
        assert!(text_edits(text, &json!([{"range":edits[0]["range"],"newText":12}]), Encoding::Utf16).is_err());
        assert!(text_edits(
            text,
            &json!([{"range":edits[0]["range"],"newText":"x","annotationId":"confirm"}]),
            Encoding::Utf16
        )
        .is_err());
    }
    #[test]
    fn edits_preserve_same_position_insert_order_before_a_replacement() {
        let mut rope = Rope::from_str("ab");
        let edits = json!([
            {"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":1}},"newText":"X"},
            {"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":1}},"newText":"Y"},
            {"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":2}},"newText":"B"}
        ]);
        let edits = text_edits(Coords::new(&rope, 1), &edits, Encoding::Utf16).unwrap();
        assert_eq!(edits, vec![(1..2, "XYB".to_owned())]);
        for (range, text) in edits {
            let start = range.start;
            rope.remove(range);
            rope.insert(start, &text);
        }
        assert_eq!(rope.to_string(), "aXYB");
    }
    #[test]
    fn workspace_edits_preserve_versions_and_reject_resource_or_duplicate_changes() {
        assert!(
            workspace_edits(&json!({"documentChanges":[{"textDocument":{"uri":"file:///tmp/a"},"edits":[]}]})).is_err()
        );
        let edit = json!({"documentChanges":[{"textDocument":{"uri":"file:///tmp/a","version":7},"edits":[]}]});
        assert_eq!(workspace_edits(&edit).unwrap(), vec![("file:///tmp/a".into(), Some(7), json!([]))]);
        assert!(workspace_edits(
            &json!({"documentChanges":[{"kind":"rename","oldUri":"file:///tmp/a","newUri":"file:///tmp/b"}]})
        )
        .is_err());
        assert!(workspace_edits(&json!({"documentChanges":[edit["documentChanges"][0],edit["documentChanges"][0]]}))
            .is_err());
        assert!(workspace_edits(
            &json!({"documentChanges":[{"textDocument":{"uri":"file:///tmp/a","version":"7"},"edits":[]}]})
        )
        .is_err());
        assert!(workspace_edits(&json!({"changeAnnotations":{"x":{"label":"confirm"}},"changes":{}})).is_err());
    }
    #[test]
    fn fragmented_stdio_services_server_requests_during_initialize_and_routes_diagnostics() {
        let client = server("normal");
        assert_eq!(client.encoding, Encoding::Utf8);
        let result = client.request("test/state", json!({})).unwrap();
        assert_eq!(result["configuration"], json!([{"enabled":true}]));
        assert_eq!(result["applyEdit"]["applied"], false);
        assert!(result["unknown"]["error"]["code"].is_number());
        client
            .notify("textDocument/didOpen", json!({"textDocument":{"uri":"file:///tmp/a","version":7,"text":"a"}}))
            .unwrap();
        client.request("test/state", json!({})).unwrap();
        assert!(client.drain_events().iter().any(|e| matches!(e, Event::Notification { method, params } if method == "textDocument/publishDiagnostics" && params["version"] == 7)));
        assert!(client.request("test/error", json!({})).unwrap_err().contains("fixture failure"));
        assert!(client.request("test/exit", json!({})).is_err());
        assert!(client.request("test/state", json!({})).is_err());
    }
    #[test]
    fn process_failures_invalid_framing_and_default_encoding_are_reported() {
        assert!(Client::start(&["/does/not/exist".into()], Path::new("/tmp"), Value::Null).is_err());
        assert!(Client::start(&fixture("invalid"), Path::new("/tmp"), Value::Null).is_err());
        assert_eq!(server("default").encoding, Encoding::Utf16);
    }
    #[test]
    fn timeout_cancels_one_request_without_poisoning_the_connection() {
        let client = server("normal");
        let start = Instant::now();
        assert!(client.transport.request("test/silent", json!({}), Duration::from_millis(30)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        let state = client.request("test/state", json!({})).unwrap();
        assert!(state["cancelled"].is_number());
        client.request("test/stderr", json!({})).unwrap();
    }
    #[test]
    fn signature_cancellation_releases_request_and_ignores_late_reply() {
        let client = server("normal");
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            signal.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        assert!(client.request_cancellable("textDocument/signatureHelp", json!({}), &cancel).is_err());
        thread.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(client.request("test/lateReply", json!({})).unwrap(), "current");
        assert!(client.request("test/state", json!({})).unwrap()["cancelled"].is_number());
    }

    #[test]
    fn framing_rejects_duplicate_lengths_oversized_headers_and_truncated_bodies() {
        for frame in [
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
            b"Content-Length: -1\r\n\r\n".to_vec(),
            b"Content-Length: 2\n\n{}".to_vec(),
            b"Content-Length: 9\r\n\r\n{}".to_vec(),
            b"Content-Length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=latin1\r\n\r\n{}".to_vec(),
            b"x".repeat(MAX_HEADER + 1),
        ] {
            assert!(read_message(&mut BufReader::new(frame.as_slice())).is_err());
        }
        assert_eq!(
            read_message(&mut BufReader::new(b"content-length: 4\r\n\r\nnull".as_slice())).unwrap(),
            (Value::Null, 4)
        );
    }
    #[test]
    fn shutdown_also_stops_children_holding_server_pipes_open() {
        let client = server("normal");
        let pid = pid(&client, "test/childPid");
        drop(client);
        std::thread::sleep(Duration::from_millis(800));
        let alive = alive(pid);
        if alive {
            let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
        assert!(!alive, "language server child process leaked");
    }
    #[test]
    fn shutdown_kills_servers_that_ignore_the_exit_notification() {
        let client = server("stubborn");
        let pid = pid(&client, "test/pid");
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pid) {
            assert!(Instant::now() < deadline, "stubborn server process leaked");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn drop_is_nonblocking_and_gracefully_reaps_server() {
        let client = server("normal");
        let pid = pid(&client, "test/pid");
        let start = Instant::now();
        drop(client);
        assert!(start.elapsed() < Duration::from_millis(100));
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pid) {
            assert!(Instant::now() < deadline, "server process leaked");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
