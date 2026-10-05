#!/usr/bin/env python3
"""MCP Gateway V1, end to end (PLAN.md decisions 81, 81f; spec §17).

A real storm-server, a real storm-runtime, a mock upstream MCP server and a
scripted agent that the runtime launches exactly as it would Claude Code or
OpenCode. The agent talks MCP to `storm-runtime mcp-bridge`, which forwards
through the host daemon and the host link to the gateway, which is the only
thing that holds the upstream credential.

What it proves, with the real build:
- the launch gives the agent Storm's servers only (AM32), and the host never
  sees a credential;
- calls work, the allowlist and the built-in `storm` connection apply, and
  `sampling`, `roots` and `elicitation.url` never reach the upstream;
- progress and a form elicitation ride their call; a URL elicitation never
  reaches the agent;
- R2–R8: a server restart re-initializes transparently with exactly one
  initialize result for the agent; an in-flight call fails once and is not
  re-sent; an open elicitation is cancelled and a late answer dropped; an
  upstream restart (404) re-initializes and runs the call once;
- disconnecting refuses the next call; `shell` gets nothing;
- the credential boundary (C1, C2, C4, C5, C7) on this host, with positive
  controls; C3 (the journal) is the operator's.

    SERVER_BIN=… RUNTIME_BIN=… python3 gateway_e2e.py
"""

import base64
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import storm_auth  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
SERVER_BIN = os.path.abspath(os.environ.get("SERVER_BIN", "target/debug/storm-server"))
RUNTIME_BIN = os.path.abspath(os.environ.get("RUNTIME_BIN", "../runtime/target/debug/storm-runtime"))

ok = 0
fail = 0


def check(name, cond, detail=""):
    global ok, fail
    if cond:
        ok += 1
        print(f"  PASS  {name}")
    else:
        fail += 1
        print(f"  FAIL  {name}   {str(detail)[:600]}")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


PORT = free_port()
UP_PORT = free_port()
BASE = f"http://127.0.0.1:{PORT}"
WORK = tempfile.mkdtemp(prefix="storm-gateway-e2e-")
STATE = os.path.join(WORK, "state")
HOST_STATE = os.path.join(WORK, "host")
WORKSPACES = os.path.join(WORK, "workspaces")
UP_STATE = os.path.join(WORK, "upstream")
SERVER_LOG = os.path.join(WORK, "server.log")
RUNTIME_LOG = os.path.join(WORK, "runtime.log")
UP_LOG = os.path.join(UP_STATE, "calls.jsonl")
CANARY_FILE = os.path.join(UP_STATE, "canary")
CANARY = "upstream-canary-" + os.urandom(12).hex()


def call(method, path, body=None, auth=None, timeout=15):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method)
    if body is not None:
        req.add_header("content-type", "application/json")
    if auth:
        req.add_header("authorization", auth)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            text = r.read()
            return r.status, (json.loads(text) if text else None)
    except urllib.error.HTTPError as e:
        text = e.read()
        try:
            return e.code, json.loads(text)
        except Exception:
            return e.code, text


def wait(predicate, what, timeout=30):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {what} (last: {str(last)[:300]})")


def start_server():
    log = open(SERVER_LOG, "a")
    p = subprocess.Popen(
        [SERVER_BIN, "serve", "--vault-root", os.path.join(WORK, "vaults"), "--state", STATE,
         "--port", str(PORT), "--gateway-allow-http-upstreams"],
        stdout=log, stderr=subprocess.STDOUT,
    )
    wait(lambda: _health(), "the server")
    return p


def _health():
    try:
        return call("GET", "/v1/health")[0] == 200
    except Exception:
        return False


def start_upstream():
    p = subprocess.Popen([sys.executable, os.path.join(HERE, "gateway_support", "upstream.py"),
                          str(UP_PORT), CANARY_FILE, UP_LOG],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def up():
        try:
            socket.create_connection(("127.0.0.1", UP_PORT), timeout=0.5).close()
            return True
        except OSError:
            return False
    wait(up, "the upstream")
    return p


def upstream_log():
    if not os.path.exists(UP_LOG):
        return []
    return [json.loads(line) for line in open(UP_LOG) if line.strip()]


class Agent:
    """The scripted agent of one session, through its workspace files."""

    def __init__(self, workspace):
        self.dir = os.path.join(WORKSPACES, workspace)
        self.n = 0

    def send(self, slug, message):
        self.n += 1
        inbox = os.path.join(self.dir, "inbox")
        tmp = os.path.join(inbox, f"{self.n:05d}.tmp")
        with open(tmp, "w") as f:
            json.dump({"slug": slug, "message": message}, f)
        os.rename(tmp, os.path.join(inbox, f"{self.n:05d}.json"))

    def transcript(self, slug):
        path = os.path.join(self.dir, f"transcript-{slug}.jsonl")
        if not os.path.exists(path):
            return []
        return [json.loads(line) for line in open(path) if line.strip()]

    def answer(self, slug, rid, timeout=30):
        return wait(lambda: next((m for m in self.transcript(slug) if m.get("id") == rid and
                                  ("result" in m or "error" in m)), None),
                    f"the answer to {rid} on {slug}", timeout)

    def env(self):
        path = os.path.join(self.dir, "agent-env.json")
        return json.load(open(path)) if os.path.exists(path) else None

    def initialize(self, slug, rid):
        self.send(slug, {"jsonrpc": "2.0", "id": rid, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {"roots": {"listChanged": True}, "sampling": {},
                             "elicitation": {"form": {}, "url": {}}},
            "clientInfo": {"name": "scripted-agent", "version": "1"}}})
        result = self.answer(slug, rid)
        self.send(slug, {"jsonrpc": "2.0", "method": "notifications/initialized"})
        return result

    def tool(self, slug, rid, name, args=None, meta=None):
        params = {"name": name, "arguments": args or {}}
        if meta:
            params["_meta"] = meta
        self.send(slug, {"jsonrpc": "2.0", "id": rid, "method": "tools/call", "params": params})


def text_of(answer):
    try:
        return answer["result"]["content"][0]["text"]
    except (KeyError, IndexError, TypeError):
        return ""


def main():
    os.makedirs(os.path.join(WORK, "vaults", "primary"))
    with open(os.path.join(WORK, "vaults", "primary", "Seed.md"), "w") as f:
        f.write("# Seed\n")
    for ws in ("gw", "oc", "sh"):
        os.makedirs(os.path.join(WORKSPACES, ws))
    os.makedirs(UP_STATE)
    with open(CANARY_FILE, "w") as f:
        f.write(CANARY)
    agent_py = os.path.join(HERE, "gateway_support", "agent.py")
    with open(os.path.join(WORK, "runtime.toml"), "w") as f:
        f.write(f"""workspace_roots = ["{WORKSPACES}"]
forbidden_roots = ["{STATE}", "{os.path.join(WORK, 'vaults')}"]
max_sessions = 6

[[providers]]
id = "claude-code"
command = "{sys.executable}"
args = ["{agent_py}"]

[[providers]]
id = "opencode"
command = "{sys.executable}"
args = ["{agent_py}"]

[providers.settings]
model = "opencode/big-pickle"

[[providers]]
id = "shell"
command = "/bin/sh"
args = ["-i"]
""")
    os.environ.pop("STORM_SESSION", None)
    os.environ.pop("STORM_DEVICE", None)

    upstream = start_upstream()
    server = start_server()
    runtime = None
    try:
        owner, device, _ = storm_auth.sign_in(BASE, log_path=SERVER_LOG)

        print("\n=== an integration, the owner's alone ===")
        status, conn = call("POST", "/v1/integrations/connections", {
            "display_name": "Mock", "url": f"http://127.0.0.1:{UP_PORT}/mcp", "auth_kind": "static",
            "credential": {"value": f"Bearer {CANARY}"}}, auth=owner)
        check("the owner connects a static integration", status == 201, conn)
        cid, slug = conn["id"], conn["slug"]
        status, test = call("POST", f"/v1/integrations/connections/{cid}/test", {}, auth=owner)
        check("its test lists the upstream's tools", status == 200 and test["tool_count"] == 5, test)
        subprocess.run([SERVER_BIN, "user", "add", "member", "--role", "member", "--state", STATE,
                        "--password-stdin"], input=storm_auth.PASSWORD.encode(), check=True,
                       capture_output=True)
        _, login = call("POST", "/v1/auth/login", {"username": "member", "password": storm_auth.PASSWORD},
                        auth=device)
        member = f"Bearer {login['access_token']}"
        for method, path in [("GET", "/v1/integrations/connections"),
                             ("GET", f"/v1/integrations/connections/{cid}"),
                             ("POST", f"/v1/integrations/connections/{cid}/test"),
                             ("DELETE", f"/v1/integrations/connections/{cid}")]:
            status, _ = call(method, path, {} if method == "POST" else None, auth=member)
            check(f"a member gets 403 on {method} {path.replace(cid, '{id}')}", status == 403, status)
        key = storm_auth.mint_mcp_key(BASE, owner)
        status, _ = call("GET", "/v1/integrations/connections", auth=f"Bearer {key}")
        check("an stk_ key is refused on the integration routes", status == 401, status)

        print("\n=== a host that can bridge ===")
        _, issued = call("POST", "/v1/agent/hosts/enrollments", {"server_url": BASE}, auth=owner)
        enrolled = subprocess.run([RUNTIME_BIN, "enroll", "--state", HOST_STATE, "--name", "gw-host"],
                                  input=(issued["enrollment"] + "\n").encode(), capture_output=True)
        check("storm-runtime enrolls", enrolled.returncode == 0, enrolled.stderr.decode())
        host_id = json.load(open(os.path.join(HOST_STATE, "host.json")))["host_id"]
        runtime = subprocess.Popen(
            [RUNTIME_BIN, "serve", "--state", HOST_STATE, "--config", os.path.join(WORK, "runtime.toml")],
            stdout=open(RUNTIME_LOG, "a"), stderr=subprocess.STDOUT, env={**os.environ, "RUST_LOG": "info"})

        def host_online():
            _, hosts = call("GET", "/v1/agent/hosts", auth=owner)
            h = next((h for h in hosts or [] if h["id"] == host_id), None)
            return h if h and h["status"] == "online" and h.get("capabilities") else None
        host = wait(host_online, "the host")
        check("the host reports mcp_bridge", host["capabilities"].get("mcp_bridge") is True, host)
        sock = os.path.join(HOST_STATE, "run")
        check("the daemon's socket directory is 0700", oct(os.stat(sock).st_mode & 0o777) == "0o700",
              oct(os.stat(sock).st_mode))

        def launch(provider, workspace, writes=False):
            status, rec = call("POST", "/v1/agent/sessions", {
                "host_id": host_id, "workspace": workspace, "provider": provider,
                "terminal": {"cols": 80, "rows": 24}, "allow_vault_writes": writes}, auth=owner)
            assert status == 200, rec
            wait(lambda: call("GET", f"/v1/agent/sessions/{rec['id']}", auth=owner)[1]["status"] == "running",
                 "running")
            return rec

        print("\n=== Claude Code's launch (AM32) ===")
        rec = launch("claude-code", "gw")
        sid = rec["id"]
        granted = sorted(g["slug"] for g in rec["mcp"]["connections"])
        check("the launch grants storm and the integration", granted == sorted(["storm", slug]), rec["mcp"])
        a = Agent("gw")
        env = wait(a.env, "the agent's record of its launch")
        argv = env["argv"]
        check("it gets --mcp-config <session>/mcp.json --strict-mcp-config",
              "--strict-mcp-config" in argv and argv[argv.index("--mcp-config") + 1].endswith(f"{sid}/mcp.json"),
              argv)
        cfg_path = argv[argv.index("--mcp-config") + 1]
        cfg = json.load(open(cfg_path))
        check("its config holds a bridge per granted connection, and nothing else",
              sorted(cfg["mcpServers"]) == sorted(["storm", slug]) and
              all(s["args"][0] == "mcp-bridge" for s in cfg["mcpServers"].values()), cfg)
        check("the session directory is 0700",
              oct(os.stat(os.path.dirname(cfg_path)).st_mode & 0o777) == "0o700")

        print("\n=== calls through the bridge ===")
        init = a.initialize(slug, 1)
        check("initialize answers with the upstream's server", init.get("result", {}).get("serverInfo", {})
              .get("name") == "mock-upstream", init)
        a.send(slug, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        tools = sorted(t["name"] for t in a.answer(slug, 2)["result"]["tools"])
        check("tools/list shows the allowlist", tools == ["ask", "ask_url", "caps", "echo", "slow"], tools)
        a.tool(slug, 3, "echo", {"text": "hi"})
        check("a tool call runs upstream", text_of(a.answer(slug, 3)) == "echo: hi")
        a.tool(slug, 4, "caps")
        upcaps = json.loads(text_of(a.answer(slug, 4)))
        check("sampling, roots and elicitation.url never reach the upstream (R1, G-D23)",
              upcaps == {"elicitation": {"form": {}}}, upcaps)
        a.tool(slug, 5, "slow", {"call_id": "p1", "seconds": 2}, meta={"progressToken": "tok-1"})
        a.answer(slug, 5)
        progress = [m for m in a.transcript(slug) if m.get("method") == "notifications/progress"]
        check("progress arrives with the agent's own token",
              len(progress) >= 2 and all(p["params"]["progressToken"] == "tok-1" for p in progress), progress)
        a.tool(slug, 6, "ask", {"wait": 20})
        ask = wait(lambda: next((m for m in a.transcript(slug) if m.get("method") == "elicitation/create"),
                                None), "the elicitation")
        a.send(slug, {"jsonrpc": "2.0", "id": ask["id"], "result": {"action": "accept", "content": {"answer": "rust"}}})
        got = text_of(a.answer(slug, 6))
        check("a form elicitation is answered by the agent", '"rust"' in got and "accept" in got, got)
        a.tool(slug, 7, "ask_url", {"wait": 2})
        got = text_of(a.answer(slug, 7))
        url_reached = [m for m in a.transcript(slug) if m.get("method") == "elicitation/create"
                       and m.get("params", {}).get("mode") == "url"]
        check("a URL elicitation never reaches the agent (G-D23)", not url_reached and "decline" in got, got)

        a.initialize("storm", 101)
        a.send("storm", {"jsonrpc": "2.0", "id": 102, "method": "tools/list"})
        stools = [t["name"] for t in a.answer("storm", 102)["result"]["tools"]]
        check("the built-in storm connection is read-only without the flags, and never deletes",
              "list_vaults" in stools and "create_note" not in stools and "delete_note" not in stools, stools)
        a.tool("storm", 103, "list_vaults")
        check("an agent reads the vault through the gateway", "result" in a.answer("storm", 103))

        print("\n=== R2–R6: the server restarts ===")
        inits_before = len([e for e in upstream_log() if e["kind"] == "initialize"])
        server.send_signal(signal.SIGTERM)
        server.wait(10)
        server = start_server()
        wait(host_online, "the host to reconnect")
        wait(lambda: call("GET", f"/v1/agent/sessions/{sid}", auth=owner)[1]["status"] == "running",
             "the session to be running again")
        a.tool(slug, 20, "echo", {"text": "after-restart"})
        check("after a restart the next call succeeds (R6)", text_of(a.answer(slug, 20)) == "echo: after-restart")
        init_results = [m for m in a.transcript(slug) if m.get("result", {}).get("serverInfo")]
        check("the agent saw exactly one initialize result (R5)", len(init_results) == 1, len(init_results))
        inits_after = len([e for e in upstream_log() if e["kind"] == "initialize"])
        check("a new upstream session was opened (R4)", inits_after == inits_before + 1, (inits_before, inits_after))
        echoes = [e for e in upstream_log() if e.get("kind") == "tools/call" and e.get("tool") == "echo"]
        check("it executed once", len(echoes) == 2, echoes)

        print("\n=== R7: an in-flight call when the server restarts ===")
        a.tool(slug, 30, "slow", {"call_id": "r7", "seconds": 6})
        wait(lambda: any(e.get("call_id") == "r7" for e in upstream_log()), "the slow call upstream")
        server.send_signal(signal.SIGKILL)
        server.wait(10)
        # Straight back up: a bridge that retried "until the server is back"
        # (the gates' `retry` mutation) would now run the call a second time.
        server = start_server()
        wait(host_online, "the host to reconnect")
        answer = a.answer(slug, 30, timeout=60)
        check("the in-flight call fails once", "error" in answer, answer)
        time.sleep(8)  # longer than the call; a retry would have run by now
        r7 = [e for e in upstream_log() if e.get("kind") == "tools/call" and e.get("call_id") == "r7"]
        check("the upstream executed it once, never retried (R7)", len(r7) == 1, r7)
        errors = [m for m in a.transcript(slug) if m.get("id") == 30]
        check("the agent got one answer for it", len(errors) == 1, errors)

        print("\n=== R8: an open elicitation when the server restarts ===")
        wait(lambda: call("GET", f"/v1/agent/sessions/{sid}", auth=owner)[1]["status"] == "running", "running")
        seen = {m["id"] for m in a.transcript(slug) if m.get("method") == "elicitation/create"}
        a.tool(slug, 40, "ask", {"wait": 30})
        ask = wait(lambda: next((m for m in a.transcript(slug) if m.get("method") == "elicitation/create"
                                 and m["id"] not in seen), None), "the second elicitation")
        server.send_signal(signal.SIGKILL)
        server.wait(10)
        a.answer(slug, 40, timeout=30)
        cancelled = [m for m in a.transcript(slug) if m.get("method") == "notifications/cancelled"
                     and m["params"]["requestId"] == ask["id"]]
        check("the open elicitation is cancelled toward the agent (R8)", len(cancelled) == 1, cancelled)
        server = start_server()
        wait(host_online, "the host to reconnect")
        responses_before = len([e for e in upstream_log() if e["kind"] == "client_response"])
        a.send(slug, {"jsonrpc": "2.0", "id": ask["id"], "result": {"action": "accept", "content": {"answer": "late"}}})
        time.sleep(2)
        responses_after = len([e for e in upstream_log() if e["kind"] == "client_response"])
        check("a late answer is dropped, never sent upstream", responses_after == responses_before)

        print("\n=== R3b: the upstream restarts (404) ===")
        wait(lambda: call("GET", f"/v1/agent/sessions/{sid}", auth=owner)[1]["status"] == "running", "running")
        a.tool(slug, 50, "echo", {"text": "warm"})
        a.answer(slug, 50)
        upstream.send_signal(signal.SIGTERM)
        upstream.wait(10)
        upstream = start_upstream()
        a.tool(slug, 51, "echo", {"text": "after-404"})
        check("the call succeeds through a transparent re-init", text_of(a.answer(slug, 51)) == "echo: after-404")
        after = [e for e in upstream_log() if e.get("kind") == "tools/call" and e.get("tool") == "echo"]
        check("and runs once", len(after) == 4 and
              len([m for m in a.transcript(slug) if m.get("id") == 51]) == 1, after)

        print("\n=== OpenCode's launch (AM32) ===")
        oc = Agent("oc")
        rec = launch("opencode", "oc")
        env = wait(oc.env, "the agent's record of its launch")
        xdg = env["env"]["XDG_CONFIG_HOME"]
        check("it gets XDG_CONFIG_HOME=<session>/xdg", xdg and xdg.endswith(f"{rec['id']}/xdg"), env)
        check("it gets OPENCODE_DISABLE_PROJECT_CONFIG=1", env["env"]["OPENCODE_DISABLE_PROJECT_CONFIG"] == "1")
        occfg = json.load(open(os.path.join(xdg, "opencode", "opencode.json")))
        check("every connection asks before a tool",
              occfg["permission"].get(f"{slug}_*") == "ask" and occfg["permission"].get("storm_*") == "ask", occfg)
        check("the host's settings are merged in", occfg.get("model") == "opencode/big-pickle", occfg)
        oc.initialize(slug, 1)
        oc.tool(slug, 2, "echo", {"text": "oc"})
        check("OpenCode's session calls through its bridge", text_of(oc.answer(slug, 2)) == "echo: oc")

        print("\n=== shell, and disconnecting mid-session ===")
        rec = launch("shell", "sh")
        check("shell gets no grants (G-D9)", rec["mcp"]["connections"] == [], rec["mcp"])
        check("and no session directory", not os.path.exists(os.path.join(HOST_STATE, "sessions", rec["id"])))
        status, _ = call("DELETE", f"/v1/integrations/connections/{cid}", auth=owner)
        check("the owner disconnects", status == 204, status)
        a.tool(slug, 60, "echo", {"text": "after-disconnect"})
        answer = a.answer(slug, 60)
        check("the next call is refused", answer.get("error", {}).get("data", {}).get("storm_error") == "not_granted",
              answer)

        print("\n=== the credential boundary (C1, C2, C4, C5, C7) ===")
        spellings = [CANARY, f"Bearer {CANARY}"]
        spellings += [base64.b64encode(s.encode()).decode() for s in list(spellings)]
        spellings += [base64.urlsafe_b64encode(s.encode()).decode().rstrip("=") for s in spellings[:2]]

        def scan_tree(root, skip=()):
            hits = []
            for dirpath, _, files in os.walk(root):
                if any(dirpath.startswith(s) for s in skip):
                    continue
                for name in files:
                    path = os.path.join(dirpath, name)
                    try:
                        data = open(path, "rb").read()
                    except OSError:
                        continue
                    if any(s.encode() in data for s in spellings):
                        hits.append(path)
            return hits

        check("positive control: the scanner finds the canary where it is",
              scan_tree(UP_STATE) == [CANARY_FILE], scan_tree(UP_STATE))
        # Everything under the suite's directory except the two places the
        # credential is meant to be: the upstream's own files, and the server's
        # state (where it is sealed — checked separately below).
        hits = [h for h in scan_tree(WORK) if not h.startswith(UP_STATE) and not h.startswith(STATE)]
        check("C1/C5/C7: no host file, session config, transcript or log holds it", hits == [], hits)
        check("and the server's own state holds it only sealed (gateway.db)",
              scan_tree(STATE) == [], scan_tree(STATE))

        # A planted control: a process carrying the canary in its environment
        # and its arguments, which the scan must find.
        control = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)", CANARY],
                                   env={**os.environ, "CONTROL": CANARY})
        wait(lambda: b"time.sleep" in open(f"/proc/{control.pid}/cmdline", "rb").read(),
             "the control process to exec")
        mine = []
        for pid in os.listdir("/proc"):
            if not pid.isdigit():
                continue
            try:
                environ = open(f"/proc/{pid}/environ", "rb").read()
                cmdline = open(f"/proc/{pid}/cmdline", "rb").read()
            except OSError:
                continue
            mine.append((pid, environ, cmdline))
        names = [c.split(b"\0")[0] for _, _, c in mine]
        check("positive control: the bridges are among the processes read",
              any(b"storm-runtime" in n for n in names))
        leaked = [pid for pid, environ, cmdline in mine
                  if any(s.encode() in environ or s.encode() in cmdline for s in spellings)
                  and pid != str(upstream.pid)]
        check("positive control: the planted process is found", str(control.pid) in leaked, leaked)
        control.kill()
        leaked = [pid for pid in leaked if pid != str(control.pid)]
        check("C2/C4: no other process's environment or arguments hold it", leaked == [], leaked)

        sessions_dir = os.path.join(HOST_STATE, "sessions")
        call("POST", f"/v1/agent/sessions/{sid}/end", auth=owner)
        wait(lambda: not os.path.exists(os.path.join(sessions_dir, sid)), "the session directory to go")
        check("an ended session's directory is removed", True)
    finally:
        for p in (runtime, server, upstream):
            if p and p.poll() is None:
                p.send_signal(signal.SIGTERM)
                try:
                    p.wait(10)
                except subprocess.TimeoutExpired:
                    p.kill()

    print(f"\n{'=' * 52}\n  {ok} passed, {fail} failed\n{'=' * 52}")
    if fail:
        print(f"  logs kept in {WORK}")
        sys.exit(1)
    shutil.rmtree(WORK, ignore_errors=True)


if __name__ == "__main__":
    main()
