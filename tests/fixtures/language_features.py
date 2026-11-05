"""Deterministic language peer used through Orlo's real workspace manager."""
import json
import os
import re
import time
import sys

source = sys.stdin.buffer
sink = sys.stdout.buffer
documents = {}
notifications = []
requests = []
options = {}
client_capabilities = {}
semantic_failed = False
signature_held_id = None
range_requests = 0
range_snapshots = []
pending_identity_diagnostics = []
# what each didChange broke: a range the agreed encoding cannot place, or the wrong sync kind
errors = []


def send(value):
    body = json.dumps(value, ensure_ascii=False).encode()
    sink.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sink.flush()


def publish_marker(path, text):
    # Existence is the reader's ready signal, so expose only the completed payload.
    temporary = f"{path}.{os.getpid()}.tmp"
    with open(temporary, "w", encoding="utf-8") as marker:
        marker.write(text)
    os.replace(temporary, path)


def read():
    length = None
    while True:
        line = source.readline()
        if not line:
            raise SystemExit(0)
        if line == b"\r\n":
            break
        name, value = line.split(b":", 1)
        if name.lower() == b"content-length":
            length = int(value)
    return json.loads(source.read(length))


def units(text):
    """`text`'s length in the position encoding agreed at initialize (UTF-16 unless chosen)."""
    encoding = options.get("encoding", "utf-16")
    if encoding == "utf-8":
        return len(text.encode())
    if encoding == "utf-32":
        return len(text)
    return len(text.encode("utf-16-le")) // 2


def word_range(uri):
    text = (documents.get(uri, {"text": "😀target\n"})["text"].splitlines() or [""])[0]
    return {"start": {"line": 0, "character": units("😀") if text.startswith("😀") else 0},
            "end": {"line": 0, "character": units(text)}}


def index(text, position):
    """The str index of an LSP position, by the protocol's own rules: lines end at \\n, \\r\\n or
    \\r, and a column must land on a char boundary within its line."""
    starts = [0] + [m.end() for m in re.finditer("\r\n|\r|\n", text)]
    line, column = position["line"], position["character"]
    if not 0 <= line < len(starts):
        raise ValueError(f"line {line} outside {len(starts)} lines")
    end = starts[line + 1] if line + 1 < len(starts) else len(text)
    content = text[starts[line]:end].rstrip("\r\n")
    at, counted = starts[line], 0
    for char in content:
        if counted >= column:
            break
        counted += units(char)
        at += 1
    if counted != column:
        raise ValueError(f"column {column} splits a char or passes the end of line {line}")
    return at


def apply(doc, changes):
    """`changes` applied in order, each against the text the one before it left."""
    text = doc["text"]
    for change in changes:
        if "range" not in change:
            text = change["text"]
            continue
        if options.get("sync", 1) != 2:
            raise ValueError("ranged change sent to a full-sync server")
        start = index(text, change["range"]["start"])
        end = index(text, change["range"]["end"])
        if start > end:
            raise ValueError("reversed range")
        text = text[:start] + change["text"] + text[end:]
    return text


def edit(uri, text):
    return {"range": word_range(uri), "newText": text}


def diagnostic(uri, version=None):
    doc = documents[uri]
    send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
        "uri": uri, "version": doc["version"] if version is None else version,
        "diagnostics": [{"range": word_range(uri), "severity": 1, "message": "fixture diagnostic"}]}})


def queue_identity_diagnostics(uri):
    if not options.get("identityDiagnostics"):
        return
    doc = documents[uri]
    params = {"uri": uri, "diagnostics": [{"range": word_range(uri), "severity": 1,
              "message": f"late diagnostic from {doc['languageId']}"}]}
    if options["identityDiagnostics"] == "versioned":
        params["version"] = doc["version"]
    pending_identity_diagnostics.append((doc["languageId"], params))


def publish_old_identity_diagnostics():
    # Release publications only once the replacement identity has actually been opened.
    # Same-client reopen and retired-client shutdown both exercise the same delayed payload.
    for old_identity, params in pending_identity_diagnostics:
        deadline = time.monotonic() + 0.4
        opened = old_identity
        while time.monotonic() < deadline:
            try:
                with open(options["identityGate"], encoding="utf-8") as gate:
                    opened = gate.read()
            except FileNotFoundError:
                pass
            if opened != old_identity:
                break
            time.sleep(0.005)
        if opened == old_identity:
            continue
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": params})
        publish_marker(options["identityPublished"], json.dumps({
            "old": old_identity, "new": opened, "kind": options["identityDiagnostics"]}))
    pending_identity_diagnostics.clear()


while True:
    message = read()
    method = message.get("method")
    params = message.get("params") or {}
    result = None
    if method in ("completionItem/resolve", "codeAction/resolve") and options.get("resolveProvider") is False:
        requests.append({"method": method, "params": params})
        send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "fixture resolve unsupported"}})
        continue
    if method == "initialize":
        options = params.get("initializationOptions") or {}
        client_capabilities = params["capabilities"]
        result = {"capabilities": {
            "textDocumentSync": {"openClose": True, "change": options.get("sync", 1), "save": {"includeText": True}},
            "completionProvider": {"resolveProvider": options.get("resolveProvider", True)}, "definitionProvider": True,
            "referencesProvider": True, "hoverProvider": True, "signatureHelpProvider": options.get("signatureProvider", {}), "renameProvider": True,
            "documentFormattingProvider": not options.get("noFormatting", False), "documentRangeFormattingProvider": not options.get("noRangeFormatting", False), "codeActionProvider": True if options.get("booleanCodeActions") else {"resolveProvider": options.get("resolveProvider", True)},
            "executeCommandProvider": {"commands": ["fixture.noop", "fixture.afterEdit"]},
            "semanticTokensProvider": {"full": True, "legend": {"tokenTypes": ["function"], "tokenModifiers": []}}}}
        if "encoding" in options:
            result["capabilities"]["positionEncoding"] = options["encoding"]
        for destination, provider in options.get("navigationProviders", {"typeDefinition": True, "implementation": True}).items():
            result["capabilities"][destination + "Provider"] = provider
    elif method == "textDocument/didOpen":
        doc = params["textDocument"].copy()
        documents[doc["uri"]] = doc
        diagnostic(doc["uri"])
        if options.get("identityDiagnostics"):
            publish_marker(options["identityGate"], doc["languageId"])
            publish_old_identity_diagnostics()
    elif method == "textDocument/didChange":
        doc = documents[params["textDocument"]["uri"]]
        assert params["textDocument"]["version"] > doc["version"]
        try:
            doc["text"] = apply(doc, params["contentChanges"])
        except ValueError as error:
            errors.append(f"version {params['textDocument']['version']}: {error}")
        doc.update(params["textDocument"])
        diagnostic(doc["uri"])
        if options.get("exitAfterChanges") and sum(n["method"] == "textDocument/didChange" for n in notifications) + 1 >= options["exitAfterChanges"]:
            raise SystemExit(0)
    elif method == "textDocument/didClose":
        queue_identity_diagnostics(params["textDocument"]["uri"])
        documents.pop(params["textDocument"]["uri"], None)
    elif method == "shutdown":
        for uri in documents:
            queue_identity_diagnostics(uri)
    elif method == "$/cancelRequest":
        notifications.append({"method": method, "params": params})
    elif method == "test/state":
        result = {"pid": os.getpid(), "documents": documents, "notifications": notifications, "requests": requests, "errors": errors, "rangeSnapshots": range_snapshots, "clientCapabilities": client_capabilities}
    elif method == "test/publishDiagnostics":
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": params})
    elif method == "test/staleDiagnostics":
        diagnostic(params["uri"], 1)
    elif method == "textDocument/completion":
        uri = params["textDocument"]["uri"]
        result = [{"label": "print", "kind": 3, "detail": "fn print()", "insertTextFormat": 1, "textEdit": edit(uri, "print"), "data": {"resolve": True}},
                  {"label": "print snippet", "insertTextFormat": 2, "insertText": "f(${1:中}, $1, ${2|one,two|})$0", "data": {"resolve": True}}]
        result = options.get("completionItems", result)
    elif method == "completionItem/resolve":
        result = dict(params)
        # the import goes below the word's line, or above it
        line = 0 if options.get("importFirst") else 1
        result["additionalTextEdits"] = [{"range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 0}}, "newText": "imported\n"}]
        result["additionalTextEdits"] = options.get("completionAdditional", result["additionalTextEdits"])
    elif method == "textDocument/signatureHelp":
        if options.get("signatureHoldFirst") and signature_held_id is None:
            signature_held_id = message["id"]
            requests.append({"method": method, "params": params})
            publish_marker(options["signatureHoldFirst"], "entered")
            continue
        if signature_held_id is not None:
            send({"jsonrpc": "2.0", "id": signature_held_id, "result": {"signatures": [{"label": "stale()"}]}})
        if options.get("signatureError"):
            requests.append({"method": method, "params": params})
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32603, "message": "fixture signature failure"}})
            continue
        if options.get("signatureContextual"):
            doc = documents[params["textDocument"]["uri"]]
            before = doc["text"][:index(doc["text"], params["position"])]
            depth = before.count("(") - before.count(")")
            result = {"signatures": [{"label": "inner()" if depth > 1 else "outer()"}]} if depth > 0 else None
        else:
            result = options.get("signatureResult", {"activeParameter": 1, "signatures": [{"label": "f(😀, value: int)", "parameters": [{"label": [2, 4]}, {"label": [6, 16]}]}]})
    elif method == "textDocument/definition":
        if options.get("definitionEntered"):
            with open(options["definitionEntered"], "w") as marker:
                marker.write("entered")
            end = time.monotonic() + 8
            while not os.path.exists(options["definitionRelease"]) and time.monotonic() < end:
                time.sleep(0.005)
            if options.get("definitionEmpty"):
                send({"jsonrpc": "2.0", "id": message["id"], "result": None})
                continue
        result = {"uri": options.get("target", params["textDocument"]["uri"]),
                  "range": {"start": {"line": 0, "character": 2}, "end": {"line": 0, "character": 8}}}
    elif method == "textDocument/references":
        uri = params["textDocument"]["uri"]
        result = [{"uri": uri, "range": word_range(uri)},
                  {"uri": options.get("target", uri), "range": {"start": {"line": 0, "character": 2}, "end": {"line": 0, "character": 8}}}]
    elif method in ("textDocument/typeDefinition", "textDocument/implementation"):
        destination = method.split("/")[1]
        if options.get("navigationHoldMethod") == destination:
            publish_marker(options["navigationEntered"], "entered")
            deadline = time.monotonic() + 8
            while not os.path.exists(options["navigationRelease"]) and time.monotonic() < deadline:
                time.sleep(0.005)
        if options.get("navigationError"):
            requests.append({"method": method, "params": params})
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "fixture navigation unsupported"}})
            continue
        result = options.get("navigationResults", {}).get(destination, {
            "uri": options.get("target", params["textDocument"]["uri"]),
            "range": {"start": {"line": 0, "character": units("😀")},
                      "end": {"line": 0, "character": units("😀target")}}})
    elif method == "textDocument/hover":
        result = {"contents": {"kind": "plaintext", "value": "fixture docs 😀"}}
    elif method == "textDocument/rangeFormatting":
        range_requests += 1
        doc = documents[params["textDocument"]["uri"]]
        range_snapshots.append({"text": doc["text"], "version": doc["version"]})
        if options.get("rangeHoldAt") == range_requests:
            publish_marker(options["rangeEntered"], "entered")
            deadline = time.monotonic() + 8
            while not os.path.exists(options["rangeRelease"]) and time.monotonic() < deadline:
                time.sleep(0.005)
        if options.get("rangeErrorAt") == range_requests:
            requests.append({"method": method, "params": params})
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32603, "message": "fixture range failure"}})
            continue
        replies = options.get("rangeResults")
        result = replies[range_requests - 1] if replies else [{"range": params["range"], "newText": "selected"}]
    elif method == "textDocument/formatting":
        if options.get("holdFormatting"):
            requests.append({"method": method, "params": params})
            if options.get("formatEntered"):
                publish_marker(options["formatEntered"], "entered")
            continue
        if options.get("formatError"):
            requests.append({"method": method, "params": params})
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32603, "message": "fixture format failed"}})
            continue
        result = options.get("formatResult", [edit(params["textDocument"]["uri"], options.get("formattedText", "formatted"))])
    elif method == "textDocument/rename":
        result = {"documentChanges": [{"textDocument": {"uri": uri, "version": doc["version"]},
                                        "edits": [edit(uri, params["newName"])]}
                                       for uri, doc in documents.items()]}
    elif method == "textDocument/codeAction":
        uri = params["textDocument"]["uri"]
        assert params["context"]["diagnostics"][0]["message"] == "fixture diagnostic"
        result = [{"title": "Fix now", "kind": "quickfix", "edit": {"changes": {uri: [edit(uri, "fixed")]}}},
                  {"title": "Resolve fix", "kind": "quickfix", "data": {"uri": uri}},
                  {"title": "Disabled fix", "disabled": {"reason": "fixture"}}]
        if options.get("commandOnlyAction"):
            result = [{"title": "Run command", "kind": "quickfix", "data": {"uri": uri},
                       "command": {"title": "Observe document", "command": "fixture.afterEdit", "arguments": [uri, "resolved" if options.get("resolveProvider", True) else "pri"]}}]
        elif options.get("commandMarker"):
            result[0]["command"] = {"title": "Observe applied fix", "command": "fixture.afterEdit", "arguments": [uri, "fixed"]}
    elif method == "workspace/executeCommand":
        uri, expected = params["arguments"]
        doc = documents[uri]
        assert expected in doc["text"], "command ran before edited document was synchronized"
        with open(options["commandMarker"], "w", encoding="utf-8") as marker:
            json.dump({"text": doc["text"], "version": doc["version"]}, marker)
    elif method == "codeAction/resolve":
        uri = params["data"]["uri"]
        result = dict(params, edit={"changes": {uri: [edit(uri, "resolved")]}})
    elif method == "textDocument/semanticTokens/full":
        if options.get("semantic_fail_once") and not semantic_failed:
            semantic_failed = True
            requests.append({"method": method, "params": params})
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32603, "message": "fixture transient semantic failure"}})
            continue
        uri = params["textDocument"]["uri"]
        span = word_range(uri)
        result = {"data": [0, span["start"]["character"], span["end"]["character"] - span["start"]["character"], 0, 0]}
    elif method == "exit":
        publish_old_identity_diagnostics()
        raise SystemExit(0)
    if method and method.startswith("textDocument/did"):
        notifications.append({"method": method, "params": params})
    if "id" in message:
        requests.append({"method": method, "params": params})
        send({"jsonrpc": "2.0", "id": message["id"], "result": result})
