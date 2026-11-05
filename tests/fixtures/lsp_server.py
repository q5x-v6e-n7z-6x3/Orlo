"""A real stdio peer exercising interleaved requests and fragmented LSP frames."""
import json
import os
import sys
import subprocess
import time

mode = sys.argv[1]
state = {}

def send(value):
    body = json.dumps(value, ensure_ascii=False).encode()
    frame = f"Content-Length: {len(body)}\r\n\r\n".encode() + body
    for start in range(0, len(frame), 7):
        sys.stdout.buffer.write(frame[start:start + 7])
        sys.stdout.buffer.flush()

def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            if mode == "stubborn":
                time.sleep(60)
            raise SystemExit(0)
        if line == b"\r\n":
            break
        name, value = line.split(b":", 1)
        if name.lower() == b"content-length":
            length = int(value)
    return json.loads(sys.stdin.buffer.read(length))

def respond(message, result):
    send({"jsonrpc": "2.0", "id": message["id"], "result": result})

def request(identifier, method, params):
    send({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params})
    reply = read()
    assert reply["id"] == identifier, reply
    return reply

while True:
    message = read()
    method = message.get("method")
    if method == "initialize":
        if mode == "invalid":
            sys.stdout.buffer.write(b"Content-Length: 999999999999\r\n\r\n")
            sys.stdout.buffer.flush()
            continue
        assert message["params"]["capabilities"]["textDocument"]["completion"]["completionItem"]["snippetSupport"] is True
        state["configuration"] = request("cfg", "workspace/configuration", {"items": [{"section": "analysis"}]})["result"]
        state["applyEdit"] = request("edit", "workspace/applyEdit", {"edit": {"changes": {}}})["result"]
        state["unknown"] = request("unknown", "workspace/executeCommand", {"command": "do-not-run"})
        request("register", "client/registerCapability", {"registrations": []})
        request("progress", "window/workDoneProgress/create", {"token": "a"})
        folders = request("folders", "workspace/workspaceFolders", {})["result"]
        assert folders[0]["uri"] == "file:///tmp"
        capabilities = {"textDocumentSync": 1}
        if mode != "default":
            capabilities["positionEncoding"] = "utf-8"
        respond(message, {"capabilities": capabilities})
    elif method == "test/state":
        respond(message, state)
    elif method == "test/lateReply":
        send({"jsonrpc": "2.0", "id": state["cancelled"], "result": "late"})
        respond(message, "current")
    elif method == "test/childPid":
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        respond(message, child.pid)
    elif method == "test/pid":
        respond(message, os.getpid())
    elif method == "test/error":
        send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32603, "message": "fixture failure"}})
    elif method == "test/exit":
        raise SystemExit(0)
    elif method == "$/cancelRequest":
        state["cancelled"] = message["params"]["id"]
    elif method == "test/stderr":
        sys.stderr.buffer.write(b"fixture log\n" * 16384)
        sys.stderr.buffer.flush()
        respond(message, None)
    elif method == "test/silent":
        pass
    elif method == "textDocument/didOpen":
        doc = message["params"]["textDocument"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": doc["uri"], "version": doc["version"], "diagnostics": []}})
    elif method == "shutdown":
        if mode != "stubborn":
            respond(message, None)
    elif method == "exit":
        if mode != "stubborn":
            raise SystemExit(0)
