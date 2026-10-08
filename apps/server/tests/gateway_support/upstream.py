#!/usr/bin/env python3
"""A mock upstream MCP server (Streamable HTTP) for gateway_e2e.py.

Ported from the gateway gates' harness (decision 81). It is a test fixture,
not part of Storm.

- Every request must carry `Authorization: Bearer <canary>`, or it is 401.
- `initialize` mints an `Mcp-Session-Id`; an unknown id is 404, the spec's
  signal to re-initialize. A restart of this process forgets them all.
- Tools: `echo`, `slow` (progress, then done), `ask` (a form elicitation on the
  call's own stream), `ask_url` (a URL elicitation, which must never reach the
  agent), `caps` (what the client declared).
- Every request is logged to a JSONL file so the suite can count executions.
- `extra_tools.json` beside the log file, when present, adds tools to every
  `tools/list` from then on: the suite's way to make a tool appear later.

    upstream.py <port> <canary-file> <log-file>
"""
import json
import os
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
CANARY = open(sys.argv[2]).read().strip()
LOG = sys.argv[3]
SESSIONS = {}
LOCK = threading.Lock()

TOOLS = [{"name": n, "description": d, "inputSchema": {"type": "object"}} for n, d in [
    ("echo", "Echo text back."),
    ("slow", "Report progress, then return."),
    ("ask", "Ask the human a question (form elicitation)."),
    ("ask_url", "Ask the human to visit a URL (URL elicitation)."),
    ("caps", "Return the client's declared capabilities."),
]]


def extra_tools():
    try:
        with open(os.path.join(os.path.dirname(LOG), "extra_tools.json")) as f:
            return [{"name": n, "description": "added later", "inputSchema": {"type": "object"}}
                    for n in json.load(f)]
    except OSError:
        return []


def log(entry):
    entry["t"] = time.time()
    with LOCK, open(LOG, "a") as f:
        f.write(json.dumps(entry) + "\n")


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *a):
        pass

    def _json(self, code, obj=None, headers=None):
        body = b"" if obj is None else json.dumps(obj).encode()
        self.send_response(code)
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        if obj is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _sse_start(self, sid):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Mcp-Session-Id", sid)
        self.end_headers()

    def _sse(self, msg):
        self.wfile.write(b"event: message\ndata: " + json.dumps(msg).encode() + b"\n\n")
        self.wfile.flush()

    def do_GET(self):
        # No standalone stream.
        self._json(405)

    def do_POST(self):
        if self.headers.get("Authorization", "") != "Bearer " + CANARY:
            log({"kind": "unauthorized"})
            return self._json(401, {"error": "invalid_token"},
                              {"WWW-Authenticate": 'Bearer realm="mock"'})
        msg = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        sid = self.headers.get("Mcp-Session-Id")
        method = msg.get("method")

        if method == "initialize":
            sid = "up-" + uuid.uuid4().hex[:12]
            caps = msg["params"].get("capabilities") or {}
            with LOCK:
                SESSIONS[sid] = {"pending": {}, "caps": caps}
            log({"kind": "initialize", "session": sid, "client_capabilities": caps})
            return self._json(200, {"jsonrpc": "2.0", "id": msg["id"], "result": {
                "protocolVersion": msg["params"].get("protocolVersion", "2025-11-25"),
                "capabilities": {"tools": {"listChanged": True}},
                "serverInfo": {"name": "mock-upstream", "version": "1"}}}, {"Mcp-Session-Id": sid})

        with LOCK:
            sess = SESSIONS.get(sid)
        if sess is None:
            log({"kind": "unknown_session", "method": method})
            return self._json(404, {"jsonrpc": "2.0", "id": msg.get("id"),
                                    "error": {"code": -32001, "message": "session not found"}})

        if method is None and "id" in msg:
            with LOCK:
                slot = sess["pending"].get(msg["id"])
            log({"kind": "client_response", "id": msg["id"], "known": slot is not None})
            if slot is None:
                return self._json(404)
            slot[1] = msg
            slot[0].set()
            return self._json(202)

        if method and method.startswith("notifications/"):
            return self._json(202)
        if method == "ping":
            return self._json(200, {"jsonrpc": "2.0", "id": msg["id"], "result": {}})
        if method == "tools/list":
            return self._json(200, {"jsonrpc": "2.0", "id": msg["id"],
                                    "result": {"tools": TOOLS + extra_tools()}})

        if method == "tools/call":
            name = msg["params"]["name"]
            args = msg["params"].get("arguments") or {}
            token = (msg["params"].get("_meta") or {}).get("progressToken")
            log({"kind": "tools/call", "tool": name, "call_id": args.get("call_id")})
            done = lambda text: {"jsonrpc": "2.0", "id": msg["id"],  # noqa: E731
                                 "result": {"content": [{"type": "text", "text": text}]}}
            if name == "echo":
                return self._json(200, done("echo: " + str(args.get("text"))))
            if name == "caps":
                return self._json(200, done(json.dumps(sess["caps"])))
            if name == "slow":
                self._sse_start(sid)
                secs = float(args.get("seconds", 2))
                steps = max(1, int(secs))
                try:
                    for i in range(steps):
                        if token is not None:
                            self._sse({"jsonrpc": "2.0", "method": "notifications/progress",
                                       "params": {"progressToken": token, "progress": i, "total": steps}})
                        time.sleep(secs / steps)
                    self._sse(done("slow done: " + str(args.get("call_id"))))
                    log({"kind": "slow_completed", "call_id": args.get("call_id")})
                except (BrokenPipeError, ConnectionResetError):
                    log({"kind": "slow_stream_broken", "call_id": args.get("call_id")})
                return
            if name in ("ask", "ask_url"):
                elic_id = "elic-" + uuid.uuid4().hex[:6]
                ev = threading.Event()
                with LOCK:
                    sess["pending"][elic_id] = [ev, None]
                if name == "ask":
                    params = {"mode": "form", "message": "lang?", "requestedSchema": {
                        "type": "object", "properties": {"answer": {"type": "string"}}}}
                else:
                    params = {"mode": "url", "message": "log in", "url": "https://example.com/login",
                              "elicitationId": elic_id}
                self._sse_start(sid)
                try:
                    self._sse({"jsonrpc": "2.0", "id": elic_id, "method": "elicitation/create", "params": params})
                    log({"kind": "elicitation_sent", "id": elic_id, "mode": params["mode"]})
                    got = ev.wait(float(args.get("wait", 30)))
                    answer = sess["pending"][elic_id][1] if got else None
                    self._sse(done("answer: " + json.dumps(answer.get("result") if answer else None)))
                except (BrokenPipeError, ConnectionResetError):
                    log({"kind": "ask_stream_broken", "id": elic_id})
                finally:
                    with LOCK:
                        sess["pending"].pop(elic_id, None)
                return
        return self._json(200, {"jsonrpc": "2.0", "id": msg.get("id"),
                                "error": {"code": -32601, "message": "method not found"}})

    def do_DELETE(self):
        with LOCK:
            SESSIONS.pop(self.headers.get("Mcp-Session-Id"), None)
        self._json(204)


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
