#!/usr/bin/env python3
"""Token sync, end to end (decision 124; token sync spec §10, AC-TS1–TS5,
TS7, TS10–TS13, TS20).

A real storm-server, a real storm-runtime (through a loopback proxy that lets
the suite see the host's side of the wire), a host that predates token sync,
and a stand-in `claude` that records its argv, its environment and its
`--settings` file. Canary tokens only; never a real one.

What it proves, with the real build:
- the owner sets, replaces and removes the token, and no route returns it;
  `stk_` keys and host tokens get 401;
- the scope defaults to none: a token synced and no host ticked changes no
  launch;
- a ticked host's Claude Code gets `CLAUDE_CODE_OAUTH_TOKEN` and nothing that
  competes with it, and its `--settings` file carries `apiKeyHelper: ""` and
  every G-TS pin; an unticked host's launch is the native one, unchanged;
- the pull is refused once the session runs, for another host's session, and
  for a launch that was not told to pull; a host too old to pull runs on its
  own login with the notice;
- narrowing the scope while a pull is in flight fails the start closed: the
  CLI never runs;
- a backup restores a server that still delivers the token; one without its
  key reports `needs_resync`;
- removal stops new sessions, and a running one keeps its token;
- the canary is in no log, argv, state directory, workspace or scrollback,
  and only in the synced CLIs' environments (positive controls for both).

    SERVER_BIN=… RUNTIME_BIN=… python3 token_sync_e2e.py
"""

import base64
import hashlib
import http.client
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

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "token_sync_support"))
import storm_auth  # noqa: E402
from link_proxy import LinkProxy  # noqa: E402
from old_host import OldHost  # noqa: E402

SERVER_BIN = os.path.abspath(os.environ.get("SERVER_BIN", "target/debug/storm-server"))
RUNTIME_BIN = os.path.abspath(os.environ.get("RUNTIME_BIN", "../runtime/target/debug/storm-runtime"))

KIND = "claude_subscription_token"
CANARY = "sk-ant-oat01-STORMCANARY-SYNC-" + os.urandom(12).hex()
CANARY2 = "sk-ant-oat01-STORMCANARY-RESYNC-" + os.urandom(12).hex()
HOST_OWN = "sk-ant-oat01-HOSTOWN-" + os.urandom(8).hex()
DAEMON_OWN = "sk-ant-oat01-DAEMONOWN-" + os.urandom(8).hex()
SUB = "/v1/provider-credentials/claude-subscription"
COMPETING = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
             "CLAUDE_CODE_USE_BEDROCK", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"]
EMPTY_PINS = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK",
              "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY"]
NETWORK_PINS = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy",
                "NO_PROXY", "no_proxy", "NODE_EXTRA_CA_CERTS", "NODE_TLS_REJECT_UNAUTHORIZED",
                "SSL_CERT_FILE", "SSL_CERT_DIR"]

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
BASE = f"http://127.0.0.1:{PORT}"
WORK = tempfile.mkdtemp(prefix="storm-token-sync-e2e-")
VAULTS = os.path.join(WORK, "vaults")
STATE = os.path.join(WORK, "state")
BACKUP = os.path.join(WORK, "backup")
RESTORED = os.path.join(WORK, "restored")
NOKEY = os.path.join(WORK, "nokey")
HOST_STATE = os.path.join(WORK, "host")
WORKSPACES = os.path.join(WORK, "workspaces")
SERVER_LOG = os.path.join(WORK, "server.log")
RUNTIME_LOG = os.path.join(WORK, "runtime.log")
RUNTIME_ENV = {
    **os.environ,
    "RUST_LOG": "info",
    "ANTHROPIC_API_KEY": "daemon-api-key",
    "ANTHROPIC_AUTH_TOKEN": "daemon-auth-token",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN": "daemon-refresh-token",
    "CLAUDE_CODE_USE_BEDROCK": "1",
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:9/decoy",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST": "1",
    "CLAUDE_CODE_OAUTH_TOKEN": DAEMON_OWN,
    "NODE_EXTRA_CA_CERTS": "/etc/ssl/storm-e2e-daemon-ca.pem",
}
for proxy_var in ("HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"):
    RUNTIME_ENV.pop(proxy_var, None)


def sha(s):
    return hashlib.sha256(s.encode()).hexdigest()


def call(method, path, body=None, auth=None, base=BASE, timeout=15, headers=False):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{base}{path}", data=data, method=method)
    if body is not None:
        req.add_header("content-type", "application/json")
    if auth:
        req.add_header("authorization", auth)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            text = r.read()
            out = (r.status, (json.loads(text) if text else None))
            return out + (dict(r.headers),) if headers else out
    except urllib.error.HTTPError as e:
        text = e.read()
        try:
            out = (e.code, json.loads(text))
        except Exception:
            out = (e.code, text)
        return out + (dict(e.headers),) if headers else out


def wait(predicate, what, timeout=30):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {what} (last: {str(last)[:300]})")


def start_server(state, port=PORT):
    log = open(SERVER_LOG, "a")
    p = subprocess.Popen([SERVER_BIN, "serve", "--vault-root", VAULTS, "--state", state, "--port", str(port)],
                         stdout=log, stderr=subprocess.STDOUT)
    base = f"http://127.0.0.1:{port}"

    def health():
        try:
            return call("GET", "/v1/health", base=base)[0] == 200
        except Exception:
            return False
    wait(health, "the server")
    return p


def stop(p):
    if p and p.poll() is None:
        p.send_signal(signal.SIGTERM)
        try:
            p.wait(10)
        except subprocess.TimeoutExpired:
            p.kill()


def stub(workspace):
    path = os.path.join(WORKSPACES, workspace, "claude-stub.json")
    return json.load(open(path)) if os.path.exists(path) else None


def scrollback(owner, sid, until, timeout=10):
    conn = http.client.HTTPConnection("127.0.0.1", PORT, timeout=timeout)
    conn.request("GET", f"/v1/agent/sessions/{sid}/terminal/stream?offset=0",
                 headers={"authorization": owner, "accept": "text/event-stream"})
    resp = conn.getresponse()
    text, event = "", None
    deadline = time.time() + timeout
    try:
        while time.time() < deadline and until not in text:
            line = resp.readline()
            if not line:
                break
            line = line.decode().rstrip("\r\n")
            if line.startswith("event:"):
                event = line[6:].strip()
            elif line.startswith("data:") and event == "output":
                text += base64.b64decode(line[5:].strip()).decode(errors="replace")
    except (socket.timeout, OSError):
        pass
    finally:
        conn.close()
    return text


def main():
    os.makedirs(os.path.join(VAULTS, "primary"))
    with open(os.path.join(VAULTS, "primary", "Seed.md"), "w") as f:
        f.write("# Seed\n")
    workspaces = ["native", "synced", "unticked", "failclosed", "resynced", "restored", "removed", "old", "old2"]
    for ws in workspaces:
        os.makedirs(os.path.join(WORKSPACES, ws))
    env_file = os.path.join(WORK, "claude.env")
    with open(env_file, "w") as f:
        f.write(f"ANTHROPIC_API_KEY=envfile-api-key\nCLAUDE_CODE_OAUTH_TOKEN={HOST_OWN}\nDISABLE_TELEMETRY=1\n")
    os.chmod(env_file, 0o600)
    stub_py = os.path.join(HERE, "token_sync_support", "claude_stub.py")
    with open(os.path.join(WORK, "runtime.toml"), "w") as f:
        f.write(f"""workspace_roots = ["{WORKSPACES}"]
forbidden_roots = ["{STATE}", "{VAULTS}"]
max_sessions = 8

[[providers]]
id = "claude-code"
command = "{sys.executable}"
args = ["{stub_py}"]
env_file = "{env_file}"

[[providers]]
id = "shell"
command = "/bin/sh"
args = ["-i"]
""")
    os.environ.pop("STORM_SESSION", None)
    os.environ.pop("STORM_DEVICE", None)

    server = start_server(STATE)
    proxy = LinkProxy(PORT)
    runtime = None
    old = None
    sessions = {}
    try:
        owner, _, _ = storm_auth.sign_in(BASE, log_path=SERVER_LOG)

        print("\n=== the owner's API (§0.3) ===")
        status, view = call("GET", SUB, auth=owner)
        check("nothing synced: present false, no hosts", status == 200 and view == {
            "kind": KIND, "present": False, "hosts": []}, view)
        for bad in ["sk-ant-api03-" + "x" * 30, "sk-ant-oat01-short", "sk-ant-oat01-has inner space-" + "x" * 20,
                    "sk-ant-oat01-" + "x" * 401]:
            status, body = call("PUT", SUB, {"token": bad}, auth=owner)
            check(f"a badly shaped token is 400 bad_token_shape ({bad[:16]}…)",
                  status == 400 and body.get("error") == "bad_token_shape" and bad not in json.dumps(body), body)
        status, view = call("PUT", SUB, {"token": f"\x1b[200~ {CANARY}\r\n\x1b[201~"}, auth=owner)
        check("a pasted token is stored once its paste markers are stripped",
              status == 200 and view["present"] and view["status"] == "active", view)
        check("the answer never holds the token", CANARY not in json.dumps(view), view)
        check("its fingerprint is 8 hex characters",
              len(view["fingerprint"]) == 8 and all(c in "0123456789abcdef" for c in view["fingerprint"]), view)
        check("expiry is a year after minting, with no warning",
              view["days_left"] in (364, 365) and view["warn"] is None, view)
        check("scope defaults to none (AC-TS3)", view["hosts"] == [], view)
        status, view = call("GET", SUB, auth=owner)
        check("GET never holds the token", status == 200 and CANARY not in json.dumps(view), view)
        first_fp = view["fingerprint"]
        key = storm_auth.mint_mcp_key(BASE, owner)
        for method, path, body in [("GET", SUB, None), ("PUT", SUB, {"token": CANARY}), ("DELETE", SUB, None),
                                   ("PATCH", f"{SUB}/hosts", {"hosts": []})]:
            status, _ = call(method, path, body, auth=f"Bearer {key}")
            check(f"an stk_ key gets 401 on {method} {path}", status == 401, status)

        print("\n=== a host that can pull, and one that predates token sync ===")
        _, issued = call("POST", "/v1/agent/hosts/enrollments",
                         {"server_url": f"http://127.0.0.1:{proxy.port}"}, auth=owner)
        enrolled = subprocess.run([RUNTIME_BIN, "enroll", "--state", HOST_STATE, "--name", "sync-host"],
                                  input=(issued["enrollment"] + "\n").encode(), capture_output=True)
        check("storm-runtime enrolls", enrolled.returncode == 0, enrolled.stderr.decode())
        host_a = json.load(open(os.path.join(HOST_STATE, "host.json")))["host_id"]
        runtime = subprocess.Popen(
            [RUNTIME_BIN, "serve", "--state", HOST_STATE, "--config", os.path.join(WORK, "runtime.toml")],
            stdout=open(RUNTIME_LOG, "a"), stderr=subprocess.STDOUT, env=RUNTIME_ENV)

        def online(host_id):
            def ready():
                _, hosts = call("GET", "/v1/agent/hosts", auth=owner)
                h = next((h for h in hosts or [] if h["id"] == host_id), None)
                return h if h and h["status"] == "online" and h.get("capabilities") else None
            return ready
        host = wait(online(host_a), "the host")
        check("the host advertises provider_credentials (AM70)",
              host["capabilities"].get("provider_credentials") == [KIND], host["capabilities"])
        _, issued = call("POST", "/v1/agent/hosts/enrollments", {"server_url": BASE}, auth=owner)
        old = OldHost(issued["enrollment"], "old-host", ["old", "old2"])
        old_view = wait(online(old.host_id), "the old host")
        check("an old host advertises none", "provider_credentials" not in old_view["capabilities"] or
              old_view["capabilities"]["provider_credentials"] == [], old_view["capabilities"])
        for method, path, body in [("GET", SUB, None), ("PATCH", f"{SUB}/hosts", {"hosts": []})]:
            status, _ = call(method, path, body, auth=old.bearer)
            check(f"a host token gets 401 on {method} {path}", status == 401, status)

        def launch(workspace, host_id=host_a, provider="claude-code", run=True):
            status, rec = call("POST", "/v1/agent/sessions", {
                "host_id": host_id, "workspace": workspace, "provider": provider,
                "terminal": {"cols": 80, "rows": 24}}, auth=owner)
            assert status == 200, rec
            sessions[workspace] = rec["id"]
            if run:
                wait(lambda: call("GET", f"/v1/agent/sessions/{rec['id']}", auth=owner)[1]["status"] == "running",
                     f"{workspace} running")
                wait(lambda: stub(workspace), f"the stub's record in {workspace}")
            return rec

        def pull(sid, bearer=None):
            return call("GET", f"/v1/runtime/sessions/{sid}/provider-credential",
                        auth=bearer or proxy.bearer, headers=True)

        print("\n=== a token synced, no host ticked: the native launch (AC-TS3, AC-TS2) ===")
        rec = launch("native")
        check("the launch says native", rec["credential"] == "native" and rec["credential_notice"] is None, rec)
        native = stub("native")
        check("the CLI runs on the env file's login", native["token_sha256"] == sha(HOST_OWN), native["env"])
        check("the env file's API key applies as before", native["env"]["ANTHROPIC_API_KEY"] == "envfile-api-key",
              native["env"])
        check("the daemon's other variables are inherited as before",
              native["env"]["ANTHROPIC_AUTH_TOKEN"] == "daemon-auth-token" and
              native["env"]["ANTHROPIC_BASE_URL"] == "http://127.0.0.1:9/decoy", native["env"])
        check("its settings file has no pins and no apiKeyHelper",
              native["settings"] and "env" not in native["settings"]["json"] and
              "apiKeyHelper" not in native["settings"]["json"], native["settings"])
        status, body, _ = pull(sessions["native"])
        check("its pull is refused once it runs", status == 409 and body.get("error") == "session_not_starting",
              body)
        status, body, _ = pull(sessions["native"], bearer=owner)
        check("a session token gets 401 on the pull route", status == 401, status)

        print("\n=== the host ticked: a synced launch (AC-TS7, G-TS pins) ===")
        status, view = call("PATCH", f"{SUB}/hosts", {"hosts": [host_a, old.host_id, host_a]}, auth=owner)
        check("the scope takes the ticked hosts, duplicates collapsed",
              status == 200 and sorted(view["hosts"]) == sorted([host_a, old.host_id]), view)
        status, body = call("PATCH", f"{SUB}/hosts", {"hosts": ["hst_00000000000000000000000000"]}, auth=owner)
        check("an unknown host is 400 unknown_host", status == 400 and body.get("error") == "unknown_host", body)
        rec = launch("synced")
        check("the launch says synced", rec["credential"] == "synced" and CANARY not in json.dumps(rec), rec)
        synced = stub("synced")
        check("the CLI gets the synced token as CLAUDE_CODE_OAUTH_TOKEN", synced["token_sha256"] == sha(CANARY))
        present = [n for n in COMPETING if n in synced["env_names"]]
        check("no competing credential or routing variable reaches it, from the daemon or the env file",
              present == [], present)
        check("the env file's other variables still apply", synced["env"]["DISABLE_TELEMETRY"] == "1",
              synced["env"])
        settings = (synced["settings"] or {}).get("json", {})
        pins = settings.get("env", {})
        check("the settings file sets apiKeyHelper to \"\"", settings.get("apiKeyHelper") == "", settings)
        check("the settings file pins the base URL", pins.get("ANTHROPIC_BASE_URL") == "https://api.anthropic.com",
              pins)
        check("it pins the competing credentials and cloud switches to \"\"",
              all(pins.get(n) == "" for n in EMPTY_PINS), pins)
        expected = {n: RUNTIME_ENV.get(n, "") for n in NETWORK_PINS}
        check("it pins every proxy and CA variable to the daemon's own value or \"\"",
              {n: pins.get(n) for n in NETWORK_PINS} == expected, pins)
        check("every settings value is a string (an invalid file fails open)",
              all(isinstance(v, str) for v in pins.values()) and "null" not in json.dumps(settings), settings)
        check("the SessionStart hook is still there", "SessionStart" in json.dumps(settings.get("hooks")), settings)
        check("the settings file is private", synced["settings"]["mode"] == "0o600", synced["settings"])
        check("the token is in no argument", not any(CANARY in a for a in synced["argv"]), synced["argv"])
        log = open(RUNTIME_LOG).read()
        check("the host logs the names it ignored, never a value",
              "synced login: ignoring" in log and "ANTHROPIC_API_KEY" in log and "envfile-api-key" not in log)

        print("\n=== the pull's rules, live (AC-TS14) ===")
        status, body, hdrs = pull(sessions["synced"])
        check("a running session's pull is 409 session_not_starting",
              status == 409 and body.get("error") == "session_not_starting" and CANARY not in json.dumps(body), body)
        status, body, _ = pull(sessions["synced"], bearer=old.bearer)
        check("another host's pull is 404 not_your_session",
              status == 404 and body.get("error") == "not_your_session", body)
        status, body, _ = pull("ags_00000000000000000000000000")
        check("an unknown session is 404 not_your_session",
              status == 404 and body.get("error") == "not_your_session", body)

        print("\n=== an old host, ticked (AC-TS10) ===")
        rec = launch("old", host_id=old.host_id, run=False)
        check("the launch says old_host, with the update notice",
              rec["credential"] == "old_host" and "old-host" in (rec["credential_notice"] or "") and
              "update storm-runtime" in rec["credential_notice"], rec)
        start = wait(lambda: next((c for c in old.starts() if c["session"] == rec["id"]), None), "the old start")
        check("its start names no credential", "provider_credential" not in start, start)
        status, body, _ = pull(rec["id"], bearer=old.bearer)
        check("and its pull is refused", status == 403 and body.get("error") == "host_not_in_scope", body)

        print("\n=== unticked: native and unchanged, and widening waits for a new launch (AC-TS2, AC-TS5) ===")
        call("PATCH", f"{SUB}/hosts", {"hosts": []}, auth=owner)
        rec = launch("old2", host_id=old.host_id, run=False)
        check("an unticked launch says native", rec["credential"] == "native", rec)
        call("PATCH", f"{SUB}/hosts", {"hosts": [old.host_id]}, auth=owner)
        status, body, _ = pull(rec["id"], bearer=old.bearer)
        check("ticked afterwards, its starting session's pull is still 403 host_not_in_scope",
              status == 403 and body.get("error") == "host_not_in_scope", body)
        rec = launch("unticked")
        unticked = stub("unticked")
        check("the launch says native", rec["credential"] == "native", rec)
        check("its environment is the native launch's", unticked["env"] == native["env"] and
              unticked["token_sha256"] == native["token_sha256"] and
              unticked["env_names"] == native["env_names"], (unticked["env"], native["env"]))
        check("its settings file is the native launch's",
              sorted(unticked["settings"]["json"]) == sorted(native["settings"]["json"]), unticked["settings"])

        print("\n=== narrowed during the pull: the start fails closed (AC-TS5, AC-TS11) ===")
        call("PATCH", f"{SUB}/hosts", {"hosts": [host_a]}, auth=owner)
        proxy.hold()
        rec = launch("failclosed", run=False)
        check("the launch was told to pull", rec["credential"] == "synced", rec)
        check("the host's pull is in flight", proxy.held.wait(20))
        call("PATCH", f"{SUB}/hosts", {"hosts": []}, auth=owner)
        proxy.release()
        final = wait(lambda: (lambda s: s if s in ("failed", "completed") else None)(
            call("GET", f"/v1/agent/sessions/{rec['id']}", auth=owner)[1]["status"]), "the start to fail")
        check("the session fails", final == "failed", final)
        time.sleep(1)
        check("the CLI never ran", stub("failclosed") is None)
        check("the host says why", "could not fetch the synced Claude login" in open(RUNTIME_LOG).read())

        print("\n=== replace: a new token, the scope kept (§3) ===")
        call("PATCH", f"{SUB}/hosts", {"hosts": [host_a]}, auth=owner)
        status, view = call("PUT", SUB, {"token": CANARY2, "minted_at": "2026-10-01T00:00:00Z"}, auth=owner)
        check("the replace keeps the scope and changes the fingerprint",
              status == 200 and view["hosts"] == [host_a] and view["fingerprint"] != first_fp and
              view["minted_at"] == "2026-10-01T00:00:00Z" and view["expires_at"] == "2027-10-01T00:00:00Z", view)
        launch("resynced")
        check("the next session gets the new token", stub("resynced")["token_sha256"] == sha(CANARY2))
        status, body, hdrs = pull(sessions["resynced"])
        check("no answer to a refused pull is cacheable or holds the token",
              status == 409 and CANARY2 not in json.dumps(body))

        print("\n=== the scrollback ===")
        for ws in ("synced", "resynced"):
            text = scrollback(owner, sessions[ws], "claude stub: ready")
            check(f"{ws}: the CLI's output is replayed", "claude stub: ready" in text, text[-200:])
            check(f"{ws}: and holds no token", CANARY not in text and CANARY2 not in text)

        print("\n=== a backup round-trip (AC-TS12) ===")
        backed = subprocess.run([SERVER_BIN, "backup-db", "--state", STATE, BACKUP], capture_output=True)
        check("backup-db snapshots the state", backed.returncode == 0, backed.stderr.decode()[-400:])
        keys = os.path.join(BACKUP, "gateway", "keys")
        check("the backup carries gateway.db and its key",
              os.path.exists(os.path.join(BACKUP, "gateway", "gateway.db")) and os.listdir(keys), BACKUP)
        stop(server)
        shutil.copytree(BACKUP, RESTORED)
        shutil.copytree(BACKUP, NOKEY)
        shutil.rmtree(os.path.join(NOKEY, "gateway", "keys"))
        nokey_port = free_port()
        nokey = start_server(NOKEY, nokey_port)
        status, view = call("GET", SUB, auth=owner, base=f"http://127.0.0.1:{nokey_port}")
        check("restored without its key: the server starts and the token is needs_resync",
              status == 200 and view.get("status") == "needs_resync" and view["hosts"] == [host_a], view)
        stop(nokey)
        server = start_server(RESTORED)
        wait(online(host_a), "the host to reconnect", timeout=120)
        status, view = call("GET", SUB, auth=owner)
        check("restored: the token is active, its scope kept",
              view.get("status") == "active" and view["hosts"] == [host_a], view)
        rec = launch("restored")
        check("restored: a ticked launch is synced", rec["credential"] == "synced", rec)
        check("restored: the CLI gets the token", stub("restored")["token_sha256"] == sha(CANARY2))

        print("\n=== removal stops new sessions (AC-TS4) ===")
        status, _ = call("DELETE", SUB, auth=owner)
        check("DELETE is 204", status == 204, status)
        status, view = call("GET", SUB, auth=owner)
        check("nothing synced, and the scope went with it", view == {"kind": KIND, "present": False, "hosts": []},
              view)
        call("PATCH", f"{SUB}/hosts", {"hosts": [host_a]}, auth=owner)
        rec = launch("removed")
        check("a new launch on a ticked host is native", rec["credential"] == "native", rec)
        check("and runs on the host's own login", stub("removed")["token_sha256"] == sha(HOST_OWN))
        pid = stub("restored")["pid"]
        check("a session launched before the removal keeps its token (stated behaviour)",
              CANARY2.encode() in open(f"/proc/{pid}/environ", "rb").read())

        print("\n=== the canary is nowhere it should not be (AC-TS1) ===")
        spellings = []
        for c in (CANARY, CANARY2):
            spellings += [c, f"Bearer {c}", base64.b64encode(c.encode()).decode(),
                          base64.urlsafe_b64encode(c.encode()).decode().rstrip("=")]

        def scan_tree(root):
            hits = []
            for dirpath, _, files in os.walk(root):
                for name in files:
                    path = os.path.join(dirpath, name)
                    try:
                        data = open(path, "rb").read()
                    except OSError:
                        continue
                    if any(s.encode() in data for s in spellings):
                        hits.append(path)
            return hits

        planted = os.path.join(tempfile.mkdtemp(prefix="storm-ts-control-"), "planted")
        with open(planted, "w") as f:
            f.write(base64.b64encode(CANARY.encode()).decode())
        check("positive control: the scanner finds a planted canary",
              scan_tree(os.path.dirname(planted)) == [planted])
        shutil.rmtree(os.path.dirname(planted))
        hits = scan_tree(WORK)
        check("no log, state directory, backup, session file or workspace holds it", hits == [], hits)

        synced_pids = {str(stub(ws)["pid"]) for ws in ("synced", "resynced", "restored")}
        in_env, in_argv = set(), set()
        for p in os.listdir("/proc"):
            if not p.isdigit():
                continue
            try:
                environ = open(f"/proc/{p}/environ", "rb").read()
                cmdline = open(f"/proc/{p}/cmdline", "rb").read()
            except OSError:
                continue
            if any(s.encode() in environ for s in spellings):
                in_env.add(p)
            if any(s.encode() in cmdline for s in spellings):
                in_argv.add(p)
        check("positive control: every synced CLI's environment holds its token", synced_pids <= in_env,
              (synced_pids, in_env))
        check("no other process's environment holds it (the daemon included)", in_env == synced_pids, in_env)
        check("no process's command line holds it", in_argv == set(), in_argv)

        for sid in sessions.values():
            call("POST", f"/v1/agent/sessions/{sid}/end", auth=owner)
    finally:
        for p in (runtime, server):
            stop(p)
        proxy.close()
        if old:
            old.close()

    print(f"\n{'=' * 52}\n  {ok} passed, {fail} failed\n{'=' * 52}")
    if fail:
        print(f"  logs kept in {WORK}")
        sys.exit(1)
    shutil.rmtree(WORK, ignore_errors=True)


if __name__ == "__main__":
    main()
