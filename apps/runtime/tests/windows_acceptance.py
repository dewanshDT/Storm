#!/usr/bin/env python3
"""Windows Runtime Host acceptance, on a real Windows machine: AC-W1–AC-W9
(decision 99).

The vault's *Agent Runtime/Windows Runtime Host* (D16) defines the criteria.
This drives them against the **installed** host: the real `storm-runtime
install`, the real `StormRuntime` service under the Service Control Manager,
its real virtual account, and a real storm-server it enrolls with. The server
is driven through its REST and SSE surface, the host through `sc.exe`,
`icacls`, PowerShell and the filesystem, as an operator would.

**It changes the machine.** It installs a service, and removes it at the end
(`uninstall --purge`). Run it on a CI runner or a machine you are prepared to
do that on, from an elevated shell. It refuses to run over an existing
install unless `STORM_ACCEPT_REPLACE=1`.

    set SERVER_BIN=…\\storm-server.exe
    set RUNTIME_BIN=…\\storm-runtime.exe
    python apps\\runtime\\tests\\windows_acceptance.py

`STORM_ACCEPT_ARTIFACTS=<dir>`: where the server and host logs are copied.
What it cannot automate is `MANUAL`, never `PASS`: a real Claude Code or
OpenCode login and prompt, the handoff to a phone, and a reboot. Exits
non-zero on any FAIL.
"""

import base64
import ctypes
import http.client
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "server", "tests"))
import storm_auth  # noqa: E402

SERVER_BIN = os.path.abspath(os.environ.get("SERVER_BIN", r"apps\server\target\debug\storm-server.exe"))
RUNTIME_BIN = os.path.abspath(os.environ.get("RUNTIME_BIN", r"apps\runtime\target\debug\storm-runtime.exe"))

# AM48–AM50: what `install` makes. The harness checks these; it never creates
# them itself.
SERVICE = "StormRuntime"
ACCOUNT = r"NT SERVICE\StormRuntime"
PROGRAM_DATA = os.environ.get("ProgramData", r"C:\ProgramData")
ROOT = os.path.join(PROGRAM_DATA, "StormRuntime")
BIN = os.path.join(os.environ.get("ProgramFiles", r"C:\Program Files"), "StormRuntime", "storm-runtime.exe")
CONFIG = os.path.join(ROOT, "runtime.toml")
STATE = os.path.join(ROOT, "state")
HOME = os.path.join(STATE, "home")
WORKSPACES = os.path.join(ROOT, "workspaces")
LOG = os.path.join(ROOT, "logs", "storm-runtime.log")
OPERATOR_PROFILE = os.environ.get("USERPROFILE", "")
WS = "accept"

results = []  # (ac, name, status, detail)


def check(ac, name, cond, detail=""):
    status = "PASS" if cond else "FAIL"
    results.append((ac, name, status, "" if cond else str(detail)[:400]))
    print(f"  {status}  [{ac}] {name}" + ("" if cond else f"   {str(detail)[:400]}"), flush=True)
    return cond


def manual(ac, name, why):
    results.append((ac, name, "MANUAL", why))
    print(f"  MANUAL [{ac}] {name}   ({why})", flush=True)


def sh(cmd, input=None, timeout=120):
    try:
        p = subprocess.run(cmd, input=input, capture_output=True, text=True, timeout=timeout,
                           encoding="utf-8", errors="replace")
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except subprocess.TimeoutExpired as e:
        return -1, f"timed out: {e}"


def ps(script, timeout=60):
    return sh(["powershell", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script], timeout=timeout)


def wait(predicate, what, timeout=30, every=0.5):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(every)
    raise TimeoutError(f"timed out waiting for {what} (last: {last!r})")


def try_wait(predicate, timeout=30, every=0.5):
    try:
        return wait(predicate, "", timeout, every)
    except TimeoutError:
        return None


# --- the service ---------------------------------------------------------------

def sc(*args):
    return sh(["sc.exe", *args])


def query():
    """`sc queryex` as a dict: STATE (a word), PID, and the exit codes."""
    rc, out = sc("queryex", SERVICE)
    if rc != 0:
        return None
    props = {}
    for line in out.splitlines():
        if ":" not in line:
            continue
        key, _, value = line.partition(":")
        props[key.strip()] = value.strip()
    state = props.get("STATE", "")
    props["state"] = state.split()[-1] if state else ""
    return props


def config():
    rc, out = sc("qc", SERVICE)
    if rc != 0:
        return None
    return {k.strip(): v.strip() for k, _, v in (l.partition(":") for l in out.splitlines()) if v}


def running():
    q = query()
    return q if q and q["state"] == "RUNNING" else None


def stopped():
    q = query()
    return q if q and q["state"] == "STOPPED" else None


def alive(pid):
    rc, out = sh(["tasklist", "/FI", f"PID eq {pid}", "/NH", "/FO", "CSV"])
    return f'"{pid}"' in out


def icacls(path):
    return sh(["icacls", path])[1]


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


# --- the server ----------------------------------------------------------------

PORT = free_port()
BASE = f"http://127.0.0.1:{PORT}"
WORK = SERVER_STATE = VAULTS = SERVER_LOG = ARTIFACTS = None


def make_work():
    global WORK, SERVER_STATE, VAULTS, SERVER_LOG, ARTIFACTS
    WORK = tempfile.mkdtemp(prefix="storm-windows-accept-")
    SERVER_STATE = os.path.join(WORK, "state")
    VAULTS = os.path.join(WORK, "vaults")
    SERVER_LOG = os.path.join(WORK, "server.log")
    ARTIFACTS = os.environ.get("STORM_ACCEPT_ARTIFACTS", WORK)
    os.makedirs(ARTIFACTS, exist_ok=True)


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
    os.makedirs(os.path.join(VAULTS, "primary"))
    with open(os.path.join(VAULTS, "primary", "Seed.md"), "w") as f:
        f.write("# Seed\n")
    os.makedirs(SERVER_STATE)
    p = subprocess.Popen(
        [SERVER_BIN, "serve", "--vault-root", VAULTS, "--state", SERVER_STATE, "--port", str(PORT)],
        stdout=open(SERVER_LOG, "a"), stderr=subprocess.STDOUT,
    )
    wait(_health, "the server", timeout=90)
    return p


def _health():
    try:
        return call("GET", "/v1/health")[0] == 200
    except Exception:
        return False


CSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|[\r\n]")


def plain(text):
    """ConPTY's output without its escape sequences and line breaks."""
    return CSI.sub("", text)


class Stream:
    """An SSE reader, one event at a time (agent_e2e.py's)."""

    def __init__(self, path, auth, timeout=30):
        self.conn = http.client.HTTPConnection("127.0.0.1", PORT, timeout=timeout)
        self.conn.request("GET", path, headers={"authorization": auth, "accept": "text/event-stream"})
        self.resp = self.conn.getresponse()

    def next(self):
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

    def until(self, predicate, limit=5000):
        text, last_id = "", None
        for _ in range(limit):
            ev = self.next()
            if ev is None:
                return text, last_id
            event, eid, data = ev
            if event == "output":
                text += base64.b64decode(data).decode(errors="replace")
                last_id = int(eid)
            if predicate(event, data, plain(text)):
                return text, last_id
        raise RuntimeError(f"stream never satisfied the predicate; got {plain(text)[-300:]!r}")

    def close(self):
        self.conn.close()


class Owner:
    def __init__(self, bearer):
        self.auth = bearer
        self.n = 0
        self.offsets = {}

    def host(self, host_id):
        _, hosts = call("GET", "/v1/agent/hosts", auth=self.auth)
        return next((h for h in hosts or [] if h["id"] == host_id), None)

    def online(self, host_id):
        h = self.host(host_id)
        return h if h and h["status"] == "online" and h.get("capabilities") else None

    def session(self, sid):
        return call("GET", f"/v1/agent/sessions/{sid}", auth=self.auth)[1] or {}

    def launch(self, host_id, provider, cols=120, rows=30):
        body = {"host_id": host_id, "workspace": WS, "provider": provider,
                "terminal": {"cols": cols, "rows": rows}}
        status, rec = call("POST", "/v1/agent/sessions", body, auth=self.auth)
        if status != 200:
            raise RuntimeError(f"launch {provider}: {status} {rec}")
        wait(lambda: self.session(rec["id"]).get("status") == "running", f"{rec['id']} running", timeout=60)
        return rec["id"]

    def write(self, sid, text):
        return call("POST", f"/v1/agent/sessions/{sid}/terminal/input", auth=self.auth, raw=text.encode())[0]

    def stream(self, sid, offset=0):
        return Stream(f"/v1/agent/sessions/{sid}/terminal/stream?offset={offset}", self.auth)

    def output(self, sid, needle, timeout=60):
        s = self.stream(sid, 0)
        try:
            deadline = time.time() + timeout
            text, _ = s.until(lambda e, d, t: needle in t or time.time() > deadline)
            return plain(text)
        finally:
            s.close()

    def run(self, sid, command):
        """Types a PowerShell `command` into a shell session and returns the
        output after it. The marker is built by concatenation, so the
        terminal's echo of the typed line never contains it."""
        self.n += 1
        done = f"DONE{self.n}"
        line = f"{command}; Write-Output ('DO' + 'NE{self.n}')"
        s = self.stream(sid, self.offsets.get(sid, 0))
        try:
            s.until(lambda e, d, t: e == "status")
            self.write(sid, line + "\r")
            text, last = s.until(lambda e, d, t: done in t.split(line[-12:])[-1])
        finally:
            s.close()
        if last is not None:
            self.offsets[sid] = last
        return plain(text)

    def end(self, sid):
        return call("POST", f"/v1/agent/sessions/{sid}/end", auth=self.auth)[0]

    def enrollment(self):
        status, issued = call("POST", "/v1/agent/hosts/enrollments", {"server_url": BASE}, auth=self.auth)
        if status != 200:
            raise RuntimeError(f"enrollment: {status} {issued}")
        return issued["enrollment"]


# --- steps -------------------------------------------------------------------

def preflight():
    if sys.platform != "win32":
        sys.exit("windows_acceptance.py runs on Windows only")
    if not ctypes.windll.shell32.IsUserAnAdmin():
        sys.exit("run it from an elevated shell")
    for b in (SERVER_BIN, RUNTIME_BIN):
        if not os.path.isfile(b):
            sys.exit(f"missing: {b}")
    if query() is not None:
        if os.environ.get("STORM_ACCEPT_REPLACE") != "1":
            sys.exit("a Runtime Host is already installed; set STORM_ACCEPT_REPLACE=1 to remove it first")
        sh([RUNTIME_BIN, "uninstall", "--purge"])


STUB = "@echo off\r\necho STUB=%~n0 HOME=%USERPROFILE%\r\necho ENV_PATH=%PATH%\r\necho ARGS=%*\r\nping -n 998 127.0.0.1 >nul\r\n"


def stub_providers():
    """AC-W3: `claude` and `opencode` as `.cmd` shims in the account's own
    `.local\\bin`, which AM53's default PATH puts first. Real ones are not
    needed: the stub proves the lookup, the batch path and the environment."""
    bindir = os.path.join(HOME, ".local", "bin")
    os.makedirs(bindir, exist_ok=True)
    made = []
    for name in ("claude", "opencode"):
        path = os.path.join(bindir, f"{name}.cmd")
        with open(path, "w", newline="") as f:
            f.write(STUB)
        made.append(path)
    return made


def ps_pid_of(text, tag):
    m = re.search(tag + r":(\d+):", text)
    return int(m.group(1)) if m else None


DETACHED = ("$p = Start-Process -PassThru -WindowStyle Hidden -FilePath ping "
            "-ArgumentList '-n','999','127.0.0.1'; Write-Output ('child:' + $p.Id + ':')")


def main():
    preflight()
    make_work()
    server = None
    owner = None
    host_id = None
    try:
        server = start_server()
        bearer, _, _ = storm_auth.sign_in(BASE, log_path=SERVER_LOG)
        owner = Owner(bearer)

        # ---------------------------------------------------------------- AC-W1
        print("\n=== AC-W1: install ===")
        rc, out = sh([RUNTIME_BIN, "install"])
        check("W1", "storm-runtime install succeeds elevated", rc == 0, out)
        cfg = config() or {}
        check("W1", f"the service runs as {ACCOUNT}",
              cfg.get("SERVICE_START_NAME", "").lower() == ACCOUNT.lower(), cfg)
        check("W1", "manual start until enrolled", "DEMAND_START" in cfg.get("START_TYPE", ""), cfg)
        check("W1", "its binary is the installed copy, run as `service`",
              BIN.lower() in cfg.get("BINARY_PATH_NAME", "").lower()
              and cfg.get("BINARY_PATH_NAME", "").rstrip('"').endswith("service"), cfg)
        check("W1", "and it is not running before enrollment", stopped() is not None, query())
        for path in (BIN, CONFIG, STATE, HOME, WORKSPACES, os.path.dirname(LOG)):
            check("W1", f"{path} exists", os.path.exists(path))
        for path in (STATE, os.path.dirname(LOG)):
            acl = icacls(path)
            check("W1", f"{path}: the account may write, Users may not",
                  "NT SERVICE\\StormRuntime:" in acl and "BUILTIN\\Users" not in acl
                  and "Everyone" not in acl, acl)
        acl = icacls(ROOT)
        check("W1", "the root's DACL is protected (nothing from ProgramData)",
              "BUILTIN\\Users" not in acl and "CREATOR OWNER" not in acl, acl)
        acl = icacls(WORKSPACES)
        user = os.environ.get("USERNAME", "")
        check("W1", "the workspace root is shared with the operator, inheritably",
              re.search(rf"\\{re.escape(user)}:\(OI\)\(CI\)\(M\)", acl, re.I) is not None, acl)
        with open(os.path.join(HOME, ".gitconfig")) as f:
            check("W1", "git trusts the workspace root", "ProgramData/StormRuntime/workspaces/*" in f.read())
        rc, out = sh([RUNTIME_BIN, "install"])
        check("W1", "install again is an idempotent upgrade", rc == 0 and stopped() is not None, out)

        try:
            os.mkdir(os.path.join(WORKSPACES, WS))
            made_ws = True
        except OSError as e:
            made_ws = e
        check("W7", "the operator creates a workspace in the root", made_ws is True, made_ws)

        stubs = stub_providers()

        # ---------------------------------------------------------------- AC-W2
        print("\n=== AC-W2: enrollment ===")
        enrollment = owner.enrollment()
        token = enrollment.rsplit(":", 1)[-1]
        p = subprocess.Popen([BIN, "enroll", "--name", "windows-accept"], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        time.sleep(1.5)  # enroll is now blocked reading stdin
        rc, argv_all = ps("Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }")
        check("W8", "the enrollment string is in no process's command line",
              rc == 0 and token not in argv_all and "storm-enroll:" not in argv_all, rc)
        out, _ = p.communicate(enrollment + "\n", timeout=120)
        check("W2", "enroll reads the string from stdin and succeeds", p.returncode == 0, out)
        m = re.search(r"\((hst_[0-9A-Z]{26})\)", out)
        host_id = m.group(1) if m else None
        check("W2", "it names the host id", host_id is not None, out)
        check("W2", "and says it started the service", "Started the StormRuntime service" in out, out)
        check("W2", "the service is running", try_wait(running, timeout=60) is not None, query())
        cfg = config() or {}
        check("W2", "and starts with Windows (delayed)", "AUTO_START" in cfg.get("START_TYPE", ""), cfg)
        host = try_wait(lambda: owner.online(host_id), timeout=90)
        check("W2", "the host is online", host is not None, owner.host(host_id) if host_id else None)
        caps = (host or {}).get("capabilities") or {}
        check("W2", "it reports its workspace root",
              any(WORKSPACES.lower() in r.lower() for r in caps.get("workspace_roots", [])), caps)

        # ---------------------------------------------------------------- AC-W3
        print("\n=== AC-W3: providers ===")
        providers = {p["id"]: p for p in caps.get("providers", [])}
        for pid in ("claude-code", "opencode", "shell"):
            check("W3", f"{pid} is available", providers.get(pid, {}).get("available") is True,
                  providers.get(pid))

        sid = owner.launch(host_id, "claude-code")
        text = owner.output(sid, "ARGS=")
        # cmd.exe hands a batch file each argument quoted (wincmd).
        check("W3", "a .cmd CLI runs through PATHEXT, with its arguments", "STUB=claude" in text
              and '"--permission-mode" "default"' in text, text[-400:])
        check("W3", "its USERPROFILE is the account's home", f"HOME={HOME}".lower() in text.lower(), text[-400:])
        m = re.search(r"ENV_PATH=([^\s]+)", text)
        check("W3", "its PATH starts with the account's own bins",
              m is not None and m.group(1).lower().startswith(os.path.join(HOME, ".local", "bin").lower()),
              m.group(1) if m else text[-300:])
        owner.end(sid)

        # ---------------------------------------------------------------- AC-W4
        print("\n=== AC-W4: sessions ===")
        sid = owner.launch(host_id, "shell")
        text = owner.run(sid, "whoami")
        check("W4", "the shell runs as the virtual account", "nt service\\stormruntime" in text.lower(), text[-300:])
        text = owner.run(sid, "$env:USERPROFILE")
        check("W4", "with the account's profile", HOME.lower() in text.lower(), text[-300:])
        text = owner.run(sid, "(Get-Location).Path")
        check("W4", "in its workspace", os.path.join(WORKSPACES, WS).lower() in text.lower(), text[-300:])
        status = call("POST", f"/v1/agent/sessions/{sid}/terminal/resize",
                      {"cols": 140, "rows": 40}, auth=owner.auth)[0]
        text = owner.run(sid, "[Console]::WindowWidth")
        check("W4", "a resize reaches ConPTY", status in (200, 204) and "140" in text, (status, text[-200:]))
        manual("W4", "a real Claude Code / OpenCode login and prompt", "needs credentials")
        manual("W4", "the handoff to a phone", "needs a second device")

        # ---------------------------------------------------------------- AC-W7
        print("\n=== AC-W7: filesystem ===")
        text = owner.run(sid, "Set-Content -Path agent.txt -Value from-agent; 'wrote'")
        shared = os.path.join(WORKSPACES, WS, "agent.txt")
        try:
            with open(shared) as f:
                agent_wrote = f.read().strip()
            with open(shared, "a") as f:
                f.write("from-operator\n")
            operator_ok = True
        except OSError as e:
            agent_wrote, operator_ok = str(e), False
        check("W7", "the operator reads and writes what the agent wrote",
              agent_wrote == "from-agent" and operator_ok, agent_wrote)
        if OPERATOR_PROFILE:
            text = owner.run(sid, f"try {{ Get-ChildItem -Force '{OPERATOR_PROFILE}' -ErrorAction Stop | "
                                  f"Out-Null; 'READABLE' }} catch {{ 'DENIED' }}")
            check("W7", "the account is refused the operator's profile",
                  re.search(r"DENIED\s*DONE\d+", text) is not None, text[-300:])
        text = owner.run(sid, f"try {{ Get-ChildItem '{SERVER_STATE}' -ErrorAction Stop | Out-Null; 'READABLE' }} "
                              f"catch {{ 'DENIED' }}")
        check("W7", "and a Storm server's state in a profile",
              re.search(r"DENIED\s*DONE\d+", text) is not None, text[-300:])
        bad = os.path.join(WORK, "bad.toml")
        with open(bad, "w") as f:
            f.write(f"workspace_roots = [{json.dumps(os.path.join(OPERATOR_PROFILE or 'C:/Users/x', 'src'))}]\n")
        rc, out = sh([RUNTIME_BIN, "serve", "--state", os.path.join(WORK, "bad-state"), "--config", bad],
                     timeout=30)
        check("W7", "a root under C:\\Users is refused at startup", rc != 0 and "AM54" in out, out[-300:])

        # ---------------------------------------------------------------- AC-W5
        print("\n=== AC-W5: process lifecycle ===")
        text = owner.run(sid, DETACHED)
        child = ps_pid_of(text, "child")
        check("W5", "a session starts a detached child", child is not None and alive(child), text[-200:])
        owner.end(sid)
        gone = try_wait(lambda: owner.session(sid).get("status") in ("stopped", "completed", "failed"), timeout=30)
        check("W5", "End ends the session", gone is not None, owner.session(sid))
        check("W5", "and the detached child, through the job",
              child is not None and try_wait(lambda: not alive(child), timeout=15) is not None, child)

        sid = owner.launch(host_id, "shell")
        child = ps_pid_of(owner.run(sid, DETACHED), "child")
        rc, out = ps("Stop-Service StormRuntime", timeout=90)
        check("W5", "Stop-Service stops the host", rc == 0 and stopped() is not None, out)
        check("W5", "and nothing the session started survives it",
              child is not None and try_wait(lambda: not alive(child), timeout=15) is not None, child)
        q = query() or {}
        check("W5", "a stop is a clean exit", q.get("WIN32_EXIT_CODE", "").startswith("0"), q)
        ps("Start-Service StormRuntime", timeout=90)
        check("W2", "Start-Service brings it back online", try_wait(lambda: owner.online(host_id), timeout=90)
              is not None, owner.host(host_id))

        sid = owner.launch(host_id, "shell")
        child = ps_pid_of(owner.run(sid, DETACHED), "child")
        pid = (running() or {}).get("PID")
        rc, out = sh(["taskkill", "/F", "/PID", str(pid)])
        check("W5", "taskkill /f of the host", rc == 0, out)
        check("W5", "kills the job and everything in it",
              child is not None and try_wait(lambda: not alive(child), timeout=15) is not None, child)
        check("W5", "and recovery restarts it within 30 s",
              try_wait(running, timeout=45) is not None, query())
        check("W5", "online again", try_wait(lambda: owner.online(host_id), timeout=90) is not None,
              owner.host(host_id))
        rec = try_wait(lambda: owner.session(sid) if owner.session(sid).get("status") == "failed" else None,
                       timeout=30) or owner.session(sid)
        check("W5", "the session it ran is failed (host_restart)",
              (rec.get("status"), rec.get("end_reason")) == ("failed", "host_restart"), rec)
        manual("W2", "online again after a reboot", "a runner cannot reboot")

        # ---------------------------------------------------------------- AC-W9
        print("\n=== AC-W9: the local pipe ===")
        pipes = [p for p in os.listdir("\\\\.\\pipe\\") if p.startswith("storm-runtime-") and "conin" not in p]
        check("W9", "the host has its named pipe", len(pipes) >= 1, pipes)
        if pipes:
            try:
                open("\\\\.\\pipe\\" + pipes[0], "r+b").close()
                opened = "opened"
            except OSError as e:
                opened = e
            check("W9", "another account (even an administrator) cannot open it",
                  isinstance(opened, PermissionError), opened)

        # ---------------------------------------------------------------- AC-W8
        print("\n=== AC-W8: secrets ===")
        identity = os.path.join(STATE, "identity")
        keys = [os.path.join(identity, k) for k in os.listdir(identity)] if os.path.isdir(identity) else []
        check("W8", "the host has a key", len(keys) >= 1, keys)
        for k in keys + [os.path.join(STATE, "host.json")]:
            acl = icacls(k)
            check("W8", f"{os.path.basename(k)} is unreadable by Users and Everyone",
                  "BUILTIN\\Users" not in acl and "Everyone" not in acl
                  and "Authenticated Users" not in acl, acl)
        with open(LOG, encoding="utf-8", errors="replace") as f:
            log = f.read()
        check("W8", "the host's log holds no enrollment secret", token not in log and "sht_" not in log)

        # ---------------------------------------------------------------- AC-W6
        print("\n=== AC-W6: revocation ===")
        sid = owner.launch(host_id, "shell")
        status, _ = call("DELETE", f"/v1/agent/hosts/{host_id}", auth=owner.auth)
        check("W6", "the owner revokes the host", status == 204, status)
        rec = try_wait(lambda: owner.session(sid) if owner.session(sid).get("status") == "failed" else None,
                       timeout=30) or owner.session(sid)
        check("W6", "its sessions fail as host_revoked",
              (rec.get("status"), rec.get("end_reason")) == ("failed", "host_revoked"), rec)
        q = try_wait(stopped, timeout=60) or query() or {}
        check("W6", "the service stops", q.get("state") == "STOPPED", q)
        check("W6", "with service-specific code 3",
              q.get("SERVICE_EXIT_CODE", "").startswith("3"), q)
        time.sleep(20)
        check("W6", "and recovery does not restart it", stopped() is not None, query())
        check("W6", "host.json is now host.json.revoked",
              os.path.exists(os.path.join(STATE, "host.json.revoked"))
              and not os.path.exists(os.path.join(STATE, "host.json")))
        enrollment = owner.enrollment()
        rc, out = sh([BIN, "enroll", "--name", "windows-accept-2"], input=enrollment + "\n")
        m = re.search(r"\((hst_[0-9A-Z]{26})\)", out)
        host_id = m.group(1) if m else None
        check("W6", "re-enrolling needs no --force and starts it again",
              rc == 0 and host_id is not None and try_wait(lambda: owner.online(host_id), timeout=90)
              is not None, out)

        for path in stubs:
            os.remove(path)
    except Exception as e:  # a step that cannot continue is a FAIL, never a crash
        check("--", "the harness ran to the end", False, repr(e))
    finally:
        print("\n=== cleanup ===")
        if ARTIFACTS and os.path.exists(LOG):
            shutil.copy(LOG, os.path.join(ARTIFACTS, "storm-runtime.log"))
        rc, out = sh([RUNTIME_BIN, "uninstall", "--purge"], timeout=120)
        check("W1", "uninstall --purge removes the service", rc == 0 and query() is None, out)
        check("W1", "and keeps the workspaces", os.path.isdir(os.path.join(WORKSPACES, WS)))
        check("W1", "and the state", not os.path.exists(STATE))
        if server and server.poll() is None:
            server.kill()
        if ARTIFACTS != WORK and SERVER_LOG and os.path.exists(SERVER_LOG):
            shutil.copy(SERVER_LOG, os.path.join(ARTIFACTS, "server.log"))

    report()


def report():
    print("\n" + "=" * 78)
    print(f"{'AC':<5}{'result':<8}criterion")
    print("-" * 78)
    for ac, name, status, detail in results:
        print(f"{ac:<5}{status:<8}{name}")
        if status == "FAIL" and detail:
            print(f"{'':<13}{detail}")
    counts = {s: sum(1 for r in results if r[2] == s) for s in ("PASS", "FAIL", "MANUAL")}
    print("-" * 78)
    print(f"{counts['PASS']} passed, {counts['FAIL']} failed, {counts['MANUAL']} manual")
    sys.exit(1 if counts["FAIL"] else 0)


if __name__ == "__main__":
    main()
