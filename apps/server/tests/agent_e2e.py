#!/usr/bin/env python3
"""Agent Runtime V1, end to end: a real storm-server and a real storm-runtime.

PLAN.md decisions 77 and 77c. The fake provider drives the whole path through
the Agent Manager and the host link (AC-A1). A real shell then proves the PTY,
and the recovery criteria (AC-R1/R2/R3) restart and revoke for real. Nothing
here reaches into either process: everything goes through the public REST and
SSE surface, as the app does.

Self-contained, because the recovery checks restart the server. It needs a
fresh state directory, which it makes. It reads `SERVER_BIN` and `RUNTIME_BIN`.

    SERVER_BIN=…/storm-server RUNTIME_BIN=…/storm-runtime python3 agent_e2e.py
"""

import base64
import http.client
import json
import os
import re
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

SERVER_BIN = os.environ.get("SERVER_BIN", "target/debug/storm-server")
RUNTIME_BIN = os.environ.get("RUNTIME_BIN", "../runtime/target/debug/storm-runtime")

ok = 0
fail = 0


def check(name, cond, detail=""):
    global ok, fail
    if cond:
        ok += 1
        print(f"  PASS  {name}")
    else:
        fail += 1
        print(f"  FAIL  {name}   {detail}")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


PORT = free_port()
BASE = f"http://127.0.0.1:{PORT}"
WORK = tempfile.mkdtemp(prefix="storm-agent-e2e-")
STATE = os.path.join(WORK, "state")
HOST_STATE = os.path.join(WORK, "host")
WORKSPACES = os.path.join(WORK, "workspaces")
SERVER_LOG = os.path.join(WORK, "server.log")
RUNTIME_LOG = os.path.join(WORK, "runtime.log")


def call(method, path, body=None, auth=None, raw=None, timeout=10):
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method)
    if body is not None and raw is None:
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


def start_server():
    log = open(SERVER_LOG, "a")
    p = subprocess.Popen(
        [SERVER_BIN, "serve", "--vault-root", os.path.join(WORK, "vaults"),
         "--state", STATE, "--port", str(PORT)],
        stdout=log, stderr=subprocess.STDOUT,
    )
    for _ in range(120):
        try:
            if call("GET", "/v1/health")[0] == 200:
                return p
        except Exception:
            pass
        time.sleep(0.25)
    raise RuntimeError("server did not start")


def start_runtime():
    log = open(RUNTIME_LOG, "a")
    return subprocess.Popen(
        [RUNTIME_BIN, "serve", "--state", HOST_STATE, "--config", os.path.join(WORK, "runtime.toml")],
        stdout=log, stderr=subprocess.STDOUT, env={**os.environ, "RUST_LOG": "info"},
    )


def wait(predicate, what, timeout=20):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {what} (last: {last})")


class Stream:
    """An SSE reader over a raw HTTP connection, one event at a time."""

    def __init__(self, path, auth, timeout=10):
        self.conn = http.client.HTTPConnection("127.0.0.1", PORT, timeout=timeout)
        self.conn.request("GET", path, headers={"authorization": auth, "accept": "text/event-stream"})
        self.resp = self.conn.getresponse()
        self.status = self.resp.status

    def next(self):
        """Returns (event, id, data) or None when the stream ends."""
        event, eid, data = "message", None, []
        while True:
            line = self.resp.readline()
            if not line:
                return None
            line = line.decode().rstrip("\r\n")
            if line == "":
                if data or event != "message":
                    return event, eid, "\n".join(data)
                continue
            if line.startswith(":"):
                continue
            field, _, value = line.partition(":")
            value = value[1:] if value.startswith(" ") else value
            if field == "event":
                event = value
            elif field == "id":
                eid = value
            elif field == "data":
                data.append(value)

    def until(self, predicate, limit=400):
        """Reads until `predicate(event, id, data, text_so_far)`; returns
        (collected output text, last output id)."""
        text, last_id = "", None
        for _ in range(limit):
            ev = self.next()
            if ev is None:
                return text, last_id, True
            event, eid, data = ev
            if event == "output":
                text += base64.b64decode(data).decode(errors="replace")
                last_id = int(eid)
            if predicate(event, eid, data, text):
                return text, last_id, False
        raise RuntimeError(f"stream never satisfied the predicate; got {text[-200:]!r}")

    def close(self):
        self.conn.close()


def session(owner, sid):
    return call("GET", f"/v1/agent/sessions/{sid}", auth=owner)[1]


def launch(owner, host_id, workspace="storm", provider=None):
    body = {"host_id": host_id, "workspace": workspace, "terminal": {"cols": 80, "rows": 24}}
    if provider:
        body["provider"] = provider
    return call("POST", "/v1/agent/sessions", body, auth=owner)


def running(owner, sid):
    return wait(lambda: (session(owner, sid) or {}).get("status") == "running", f"{sid} running")


def write(owner, sid, text):
    return call("POST", f"/v1/agent/sessions/{sid}/terminal/input", auth=owner, raw=text.encode())[0]


def main():
    os.makedirs(os.path.join(WORK, "vaults", "primary"))
    with open(os.path.join(WORK, "vaults", "primary", "Seed.md"), "w") as f:
        f.write("# Seed\n")
    for ws in ("storm", "site"):
        os.makedirs(os.path.join(WORKSPACES, ws))
    with open(os.path.join(WORK, "runtime.toml"), "w") as f:
        f.write(f"""workspace_roots = ["{WORKSPACES}"]
forbidden_roots = ["{STATE}", "{os.path.join(WORK, 'vaults')}"]
max_sessions = 6

[[providers]]
id = "fake"
kind = "fake"

[[providers]]
id = "shell"
command = "/bin/sh"
args = ["-i"]
""")

    # This suite claims its own server. A session inherited from `make
    # test-live`'s environment belongs to a different one.
    os.environ.pop("STORM_SESSION", None)
    os.environ.pop("STORM_DEVICE", None)

    server = start_server()
    runtime = None
    try:
        owner, device, _ = storm_auth.sign_in(BASE, log_path=SERVER_LOG)

        print("\n=== enrollment ===")
        status, issued = call("POST", "/v1/agent/hosts/enrollments", {"server_url": BASE}, auth=owner)
        check("the owner issues an enrollment", status == 200, issued)
        enrolled = subprocess.run(
            [RUNTIME_BIN, "enroll", "--state", HOST_STATE, "--name", "e2e-host"],
            input=(issued["enrollment"] + "\n").encode(), capture_output=True,
        )
        check("storm-runtime enrolls", enrolled.returncode == 0, enrolled.stderr.decode())
        host_id = json.load(open(os.path.join(HOST_STATE, "host.json")))["host_id"]

        runtime = start_runtime()

        def host_online():
            status, hosts = call("GET", "/v1/agent/hosts", auth=owner)
            h = next((h for h in hosts or [] if h["id"] == host_id), None)
            return h if h and h["status"] == "online" and h.get("capabilities") else None

        host = wait(host_online, "the host to come online")
        check("the host is online (AC-F1)", host["status"] == "online")
        caps = host["capabilities"]
        check("it offers fake and shell", {p["id"] for p in caps["providers"]} == {"fake", "shell"}, caps)
        check("it lists its workspaces", sorted(caps["workspaces"]) == ["site", "storm"], caps)
        check("egress is the host's", host["egress"] == "host")
        status, wss = call("GET", f"/v1/agent/hosts/{host_id}/workspaces", auth=owner)
        check("workspaces carry live counts", status == 200 and all("live_sessions" in w for w in wss), wss)

        print("\n=== the boundary (AC-S1, AC-S2; single user since decision 82) ===")
        # One account; what remains to prove is that only a signed-in session
        # reaches agents. An access key acts as the account too, but only on
        # /mcp, so every agent route refuses it at the tier.
        key = storm_auth.mint_mcp_key(BASE, owner)
        for method, path in [("GET", "/v1/agent/hosts"), ("GET", "/v1/agent/sessions"),
                             ("GET", "/v1/config/agent"), ("POST", "/v1/agent/sessions")]:
            body = {"host_id": host_id, "workspace": "storm", "terminal": {"cols": 80, "rows": 24}} if method == "POST" else None
            status, _ = call(method, path, body, auth=f"Bearer {key}")
            check(f"an access key gets 401 on {method} {path}", status == 401, status)
        status, _ = call("GET", "/v1/runtime/whoami", auth=owner)
        check("a session token is refused on /v1/runtime/*", status == 401, status)

        print("\n=== AC-A1: the fake provider through the manager and the link ===")
        status, rec = launch(owner, host_id, provider="fake")
        check("launch answers with the record", status == 200 and rec["status"] in ("starting", "running"), rec)
        sid = rec["id"]
        running(owner, sid)
        check("the host reports it running", session(owner, sid)["status"] == "running")
        s = Stream(f"/v1/agent/sessions/{sid}/terminal/stream?offset=0", owner)
        check("the stream opens", s.status == 200, s.status)
        first = s.next()
        check("every stream starts with a status event", first and first[0] == "status", first)
        text, last_id, _ = s.until(lambda e, i, d, t: "fake session" in t)
        check("the start line names the session and the workspace",
              sid in text and text.rstrip().endswith("/storm"), text)
        check("input is accepted", write(owner, sid, "hello\r") == 204)
        text, _, _ = s.until(lambda e, i, d, t: "hello\r" in t)
        check("input comes back as output", "hello" in text, text)
        status, _ = call("POST", f"/v1/agent/sessions/{sid}/terminal/resize",
                         {"cols": 100, "rows": 40, "focus": True}, auth=owner)
        check("resize is accepted", status == 204, status)
        text, _, _ = s.until(lambda e, i, d, t: "[resize 100x40]" in t)
        check("the resize reaches the terminal", "[resize 100x40]" in text)
        check("the record carries the size", session(owner, sid)["cols"] == 100)
        status, _ = call("POST", f"/v1/agent/sessions/{sid}/end", auth=owner)
        check("end is accepted", status == 202, status)
        _, _, ended = s.until(lambda e, i, d, t: e == "status" and json.loads(d)["status"] == "stopped")
        check("the stream reports stopped", True)
        check("the stream closes once the session has ended", s.next() is None)
        s.close()
        check("the record is stopped", session(owner, sid)["status"] == "stopped")
        status, _ = call("POST", f"/v1/agent/sessions/{sid}/terminal/input", auth=owner, raw=b"x")
        check("input to an ended session is refused", status == 409, status)

        print("\n=== offset resume and gap ===")
        status, rec = launch(owner, host_id, provider="fake")
        sid2 = rec["id"]
        running(owner, sid2)
        s = Stream(f"/v1/agent/sessions/{sid2}/terminal/stream?offset=0", owner)
        _, last_id, _ = s.until(lambda e, i, d, t: "fake session" in t)
        s.close()
        write(owner, sid2, "abc")
        s = Stream(f"/v1/agent/sessions/{sid2}/terminal/stream?offset={last_id}", owner)
        text, _, _ = s.until(lambda e, i, d, t: "abc" in t)
        check("resuming from an offset sends exactly what followed", text == "abc", repr(text))
        s.close()
        chunk = "x" * 65536
        for _ in range(70):
            if write(owner, sid2, chunk) != 204:
                break
        def past_capacity():
            s = Stream(f"/v1/agent/sessions/{sid2}/terminal/stream?offset=0", owner)
            s.next()
            ev = s.next()
            s.close()
            return ev if ev and ev[0] == "gap" else None
        gap = wait(past_capacity, "the cache to pass its capacity", timeout=60)
        g = json.loads(gap[2])
        check("a reader below the floor gets an explicit gap", g["from"] == 0 and g["to"] > 0, g)
        call("POST", f"/v1/agent/sessions/{sid2}/end", auth=owner)
        wait(lambda: session(owner, sid2)["status"] == "stopped", "stopped")
        status, _ = call("DELETE", f"/v1/agent/sessions/{sid2}", auth=owner)
        check("an ended session is dismissed", status == 204, status)
        check("and is gone", call("GET", f"/v1/agent/sessions/{sid2}", auth=owner)[0] == 404)

        print("\n=== a real shell in a PTY ===")
        status, rec = launch(owner, host_id, workspace="site", provider="shell")
        sid3 = rec["id"]
        running(owner, sid3)
        status, _ = call("DELETE", f"/v1/agent/sessions/{sid3}", auth=owner)
        check("a live session cannot be dismissed", status == 409, status)
        s = Stream(f"/v1/agent/sessions/{sid3}/terminal/stream?offset=0", owner)
        write(owner, sid3, "echo $((6*7))\r")
        text, _, _ = s.until(lambda e, i, d, t: "42" in t.replace("6*7", ""))
        check("the shell runs commands", "42" in text)
        write(owner, sid3, "pwd\r")
        text, _, _ = s.until(lambda e, i, d, t: "/site" in t)
        check("its working directory is the workspace", "/site" in text)
        write(owner, sid3, "exit 7\r")
        s.until(lambda e, i, d, t: e == "status" and json.loads(d)["status"] == "completed")
        s.close()
        rec = session(owner, sid3)
        check("exiting completes it with the exit code", (rec["status"], rec["exit_code"]) == ("completed", 7), rec)

        print("\n=== fallback and the default provider (AC-F7) ===")
        status, rec = launch(owner, host_id, provider="claude-code")
        fb = rec.get("provider_fallback")
        check("a missing provider falls back, and says so",
              rec["provider"] == "shell" and fb == {"requested": "claude-code", "used": "shell", "reason": "not_installed"}, rec)
        running(owner, rec["id"])
        call("POST", f"/v1/agent/sessions/{rec['id']}/end", auth=owner)
        status, cfg = call("PUT", "/v1/config/agent", {"default_provider": "fake"}, auth=owner)
        check("the owner sets the default provider", status == 200 and cfg["default_provider"] == "fake", cfg)
        status, rec = launch(owner, host_id)
        check("a launch without a provider uses the default", rec["provider"] == "fake" and rec["provider_fallback"] is None, rec)
        survivor = rec["id"]
        running(owner, survivor)

        print("\n=== AC-R1: the server restarts mid-session ===")
        server.send_signal(signal.SIGTERM)
        server.wait(timeout=15)
        server = start_server()
        rec = session(owner, survivor)
        check("after a restart the session is unknown until the host reports", rec["status"] in ("unknown", "running"), rec)
        wait(lambda: session(owner, survivor)["status"] == "running", "the host to reconcile it", timeout=90)
        check("the host reconnects and the session is running again", True)
        s = Stream(f"/v1/agent/sessions/{survivor}/terminal/stream?offset=0", owner)
        text, _, _ = s.until(lambda e, i, d, t: "fake session" in t)
        check("scrollback is replayed from the host", survivor in text)
        write(owner, survivor, "still here\r")
        text, _, _ = s.until(lambda e, i, d, t: "still here" in t)
        check("the agent was never interrupted", "still here" in text)
        s.close()

        print("\n=== offline input, and AC-R2: the host restarts ===")
        runtime.send_signal(signal.SIGKILL)
        runtime.wait(timeout=10)
        wait(lambda: session(owner, survivor)["status"] == "unknown", "the session to go unknown", timeout=60)
        check("a dropped link makes the session unknown", True)
        check("input is refused while the host is offline", write(owner, survivor, "x") == 503)
        status, _ = launch(owner, host_id, provider="fake")
        check("a launch is refused while the host is offline", status == 503, status)
        runtime = start_runtime()
        wait(lambda: session(owner, survivor)["status"] == "failed", "host_restart", timeout=60)
        rec = session(owner, survivor)
        check("its sessions became failed (host_restart), none restarted",
              rec["end_reason"] == "host_restart", rec)
        wait(host_online, "the host back online", timeout=60)

        print("\n=== AC-S4: a mismatched pinned key ===")
        bogus = os.path.join(WORK, "bogus")
        os.makedirs(bogus)
        cfg = json.load(open(os.path.join(HOST_STATE, "host.json")))
        cfg["server_pubkey"] = "Kay64UG8yvCyLhqU000LxzYeUm0L_hLIl5S8kyKWbdc"  # some other key
        json.dump(cfg, open(os.path.join(bogus, "host.json"), "w"))
        os.makedirs(os.path.join(bogus, "identity"))
        for f in os.listdir(os.path.join(HOST_STATE, "identity")):
            open(os.path.join(bogus, "identity", f), "wb").write(open(os.path.join(HOST_STATE, "identity", f), "rb").read())
        checked = subprocess.run([RUNTIME_BIN, "check", "--state", bogus], capture_output=True)
        check("a host refuses a server that cannot prove the pinned key",
              checked.returncode != 0 and b"could not prove" in checked.stderr, checked.stderr)

        print("\n=== AC-F6: three sessions across two hosts ===")
        host2_state = os.path.join(WORK, "host2")
        status, issued = call("POST", "/v1/agent/hosts/enrollments", {"server_url": BASE}, auth=owner)
        enrolled = subprocess.run(
            [RUNTIME_BIN, "enroll", "--state", host2_state, "--name", "second-host"],
            input=(issued["enrollment"] + "\n").encode(), capture_output=True,
        )
        check("a second host enrolls", enrolled.returncode == 0, enrolled.stderr.decode())
        host2_id = json.load(open(os.path.join(host2_state, "host.json")))["host_id"]
        runtime2 = subprocess.Popen(
            [RUNTIME_BIN, "serve", "--state", host2_state, "--config", os.path.join(WORK, "runtime.toml")],
            stdout=open(RUNTIME_LOG, "a"), stderr=subprocess.STDOUT,
        )
        try:
            def both_online():
                _, hosts = call("GET", "/v1/agent/hosts", auth=owner)
                return all(any(h["id"] == i and h["status"] == "online" for h in hosts) for i in (host_id, host2_id))
            wait(both_online, "both hosts online", timeout=60)
            a = launch(owner, host_id, workspace="storm", provider="fake")[1]["id"]
            b = launch(owner, host_id, workspace="storm", provider="fake")[1]["id"]
            c = launch(owner, host2_id, workspace="site", provider="fake")[1]["id"]
            for sid in (a, b, c):
                running(owner, sid)
            check("three sessions run at once, across two hosts", True)
            _, wss = call("GET", f"/v1/agent/hosts/{host_id}/workspaces", auth=owner)
            shared = next(w for w in wss if w["name"] == "storm")
            check("the shared workspace reports two live sessions (the warning)", shared["live_sessions"] == 2, wss)
            for sid, word in ((a, "alpha"), (b, "bravo"), (c, "charlie")):
                write(owner, sid, word)
            for sid, word, others in ((a, "alpha", ("bravo", "charlie")), (b, "bravo", ("alpha", "charlie")), (c, "charlie", ("alpha", "bravo"))):
                s = Stream(f"/v1/agent/sessions/{sid}/terminal/stream?offset=0", owner)
                text, _, _ = s.until(lambda e, i, d, t: word in t)
                s.close()
                check(f"each session sees only its own input ({word})", not any(o in text for o in others), text)
            for sid in (a, b, c):
                call("POST", f"/v1/agent/sessions/{sid}/end", auth=owner)
            wait(lambda: all(session(owner, sid)["status"] == "stopped" for sid in (a, b, c)), "all ended")
            check("each ends on its own", True)
        finally:
            runtime2.kill()

        print("\n=== AC-R3: revocation ===")
        status, rec = launch(owner, host_id, provider="fake")
        doomed = rec["id"]
        running(owner, doomed)
        status, _ = call("DELETE", f"/v1/agent/hosts/{host_id}", auth=owner)
        check("the owner revokes the host", status == 204, status)
        rec = session(owner, doomed)
        check("its sessions fail as host_revoked", (rec["status"], rec["end_reason"]) == ("failed", "host_revoked"), rec)
        try:
            code = runtime.wait(timeout=90)
        except subprocess.TimeoutExpired:
            code = None
        check("the revoked host ends its sessions and exits 3", code == 3, code)
        checked = subprocess.run([RUNTIME_BIN, "check", "--state", HOST_STATE], capture_output=True)
        check("and cannot authenticate again", checked.returncode != 0, checked.stderr)

        print("\n=== AC-S5: no secret in any log ===")
        logs = open(SERVER_LOG).read() + open(RUNTIME_LOG).read()
        check("no enrollment token", not re.search(r"sen_[0-9A-Z]{26}\.[A-Za-z0-9_-]{43}", logs))
        check("no host token", not re.search(r"sht_[A-Za-z0-9_-]{43}", logs))
    finally:
        for p in (runtime, server):
            if p and p.poll() is None:
                p.kill()

    print(f"\n{'=' * 52}\n  {ok} passed, {fail} failed\n{'=' * 52}")
    if fail:
        print(f"logs: {SERVER_LOG} {RUNTIME_LOG}")
    else:
        import shutil
        shutil.rmtree(WORK, ignore_errors=True)
    sys.exit(1 if fail else 0)


if __name__ == "__main__":
    main()
