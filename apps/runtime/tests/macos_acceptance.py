#!/usr/bin/env python3
"""macOS Runtime Host acceptance, on a real Mac: AC-M1–AC-M9 (decision 83).

The vault's *Agent Runtime/macOS Runtime Host* (D14) defines the criteria.
This drives them against the **installed** host: the real `storm-runtime
install`, the real LaunchDaemon under launchd, the real `_stormruntime`
account, and a real storm-server it enrolls with. Nothing reaches into
either process: the server is driven through its REST and SSE surface (as
`apps/server/tests/agent_e2e.py` does), and the host through `launchctl`,
`dscl`, `ps` and the filesystem, as an operator would.

**It changes the machine.** It installs a system service and creates an
account, and removes both at the end (`uninstall --purge`). Run it on a CI
runner or a Mac you are prepared to do that on. It needs passwordless `sudo`.
It refuses to run over an existing install unless
`STORM_ACCEPT_REPLACE=1`.

    SERVER_BIN=…/storm-server RUNTIME_BIN=…/storm-runtime \\
        python3 apps/runtime/tests/macos_acceptance.py

Environment:
- `STORM_ACCEPT_STUBS=1`: where `claude` or `opencode` is not installed, put
  a stub of that name in the Homebrew prefix (outside `/usr/bin`, AC-M3), and
  remove it afterwards. Without it, a missing CLI is a FAIL.
- `STORM_ACCEPT_CLEANUP=1`: also delete `/Library/StormRuntime/workspaces`
  at the end. `uninstall` never does (AM35).
- `STORM_ACCEPT_ARTIFACTS=<dir>`: where the server and host logs are copied.

What it cannot automate is reported `MANUAL`, never `PASS`: a real Claude
Code or OpenCode login and prompt, the Mac → phone handoff, and a reboot.
Exits non-zero on any FAIL.
"""

import base64
import http.client
import json
import os
import pwd
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

SERVER_BIN = os.environ.get("SERVER_BIN", "apps/server/target/debug/storm-server")
RUNTIME_BIN = os.path.abspath(os.environ.get("RUNTIME_BIN", "apps/runtime/target/debug/storm-runtime"))

# AM33–AM35: the layout `install` makes. The harness checks these; it never
# creates them itself.
LABEL = "dev.storm.runtime"
SERVICE = f"system/{LABEL}"
ACCOUNT = "_stormruntime"
ROOT = "/Library/StormRuntime"
BIN = f"{ROOT}/bin/storm-runtime"
CONFIG = f"{ROOT}/runtime.toml"
STATE = f"{ROOT}/state"
HOME = f"{STATE}/home"
WORKSPACES = f"{ROOT}/workspaces"
LOG_DIR = "/Library/Logs/StormRuntime"
LOG = f"{LOG_DIR}/storm-runtime.log"
PLIST = f"/Library/LaunchDaemons/{LABEL}.plist"
# AM38: the PATH a session gets when runtime.toml sets none.
AM38_PATH = f"{HOME}/.local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

OPERATOR = pwd.getpwuid(os.getuid()).pw_name
OPERATOR_HOME = os.path.expanduser("~")
WS = "accept"

results = []  # (ac, name, status, detail)


def check(ac, name, cond, detail=""):
    status = "PASS" if cond else "FAIL"
    results.append((ac, name, status, "" if cond else str(detail)[:300]))
    print(f"  {status}  [{ac}] {name}" + ("" if cond else f"   {str(detail)[:300]}"), flush=True)
    return cond


def manual(ac, name, why):
    results.append((ac, name, "MANUAL", why))
    print(f"  MANUAL [{ac}] {name}   ({why})", flush=True)


def sh(cmd, sudo=False, user=None, input=None, check_rc=False, cwd="/", timeout=60):
    """Runs a command; returns (rc, stdout+stderr). `user` runs it as that
    account through `sudo -u`, from `/`, which every account can read."""
    argv = list(cmd)
    if user:
        argv = ["sudo", "-n", "-u", user] + argv
    elif sudo:
        argv = ["sudo", "-n"] + argv
    p = subprocess.run(argv, input=input, capture_output=True, text=True, cwd=cwd, timeout=timeout)
    out = (p.stdout or "") + (p.stderr or "")
    if check_rc and p.returncode != 0:
        raise RuntimeError(f"{' '.join(argv)} failed ({p.returncode}): {out}")
    return p.returncode, out


# --- launchd -----------------------------------------------------------------

def launchd():
    """The job's top-level properties from `launchctl print`, or None when it
    is not loaded. Top-level keys are indented by exactly one tab; nested
    blocks (environment, event triggers) are deeper and are ignored."""
    rc, out = sh(["launchctl", "print", SERVICE], sudo=True)
    if rc != 0:
        return None
    props = {}
    for line in out.splitlines():
        m = re.match(r"^\t([a-z][a-z ]*?) = (.*)$", line)
        if m and m.group(1) not in props:
            props[m.group(1)] = m.group(2).strip()
    return props


def daemon_pid():
    p = launchd() or {}
    pid = p.get("pid")
    return int(pid) if pid and pid.isdigit() else None


def runs():
    p = launchd() or {}
    return int(p["runs"]) if p.get("runs", "").isdigit() else None


def account_processes():
    """`(pid, pgid, command)` of every process running as the account."""
    rc, out = sh(["ps", "-axo", "user=,pid=,pgid=,command="])
    procs = []
    for line in out.splitlines():
        parts = line.split(None, 3)
        if len(parts) >= 3 and parts[0] == ACCOUNT:
            procs.append((int(parts[1]), int(parts[2]), parts[3] if len(parts) > 3 else ""))
    return procs


def group_members(pgid):
    rc, out = sh(["ps", "-axo", "pid=,pgid="])
    return [int(p) for p, g in (l.split() for l in out.splitlines() if l.strip()) if int(g) == pgid]


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:  # another account's process: it exists
        return True


def stat(path):
    """`(mode, owner, group)` by `stat -f` (BSD), as root so a 0700 parent
    does not hide the answer."""
    rc, out = sh(["stat", "-f", "%Lp %Su %Sg", path], sudo=True)
    if rc != 0:
        return None
    mode, owner, group = out.split()
    return mode, owner, group


def acl(path):
    """The ACL entries `ls -led` prints under the path's line."""
    rc, out = sh(["ls", "-led", path], sudo=True)
    return [l.strip() for l in out.splitlines()[1:] if re.match(r"^\s*\d+: ", l)]


def dscl(kind, name, *keys):
    rc, out = sh(["dscl", ".", "-read", f"/{kind}/{name}", *keys])
    if rc != 0:
        return None
    rec, key = {}, None
    for line in out.splitlines():
        # Attributes outside the standard set print as
        # "dsAttrTypeNative:IsHidden: 1"; key them by their own name.
        line = re.sub(r"^dsAttrType(?:Native|Standard):", "", line)
        if re.match(r"^\S[^:]*:", line):
            key, _, value = line.partition(":")
            rec[key] = value.strip()
        elif key:  # a value on the next line, indented
            rec[key] = (rec[key] + " " + line.strip()).strip()
    return rec


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


# --- the server ----------------------------------------------------------------

def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


PORT = free_port()
BASE = f"http://127.0.0.1:{PORT}"
# Under /private/tmp, not the per-user $TMPDIR: the server's data tree must be
# refused to the account by its *own* modes (0750, P3), not by a 0700 parent
# that would make the AC-M7 check prove nothing.
WORK = SERVER_STATE = VAULTS = SERVER_LOG = ARTIFACTS = None  # set by make_work()


def make_work():
    global WORK, SERVER_STATE, VAULTS, SERVER_LOG, ARTIFACTS
    WORK = tempfile.mkdtemp(prefix="storm-macos-accept-", dir="/private/tmp")
    os.chmod(WORK, 0o755)
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
    os.makedirs(SERVER_STATE, mode=0o750)
    os.chmod(SERVER_STATE, 0o750)  # P3's mode, whatever the umask
    os.chmod(VAULTS, 0o750)
    p = subprocess.Popen(
        [SERVER_BIN, "serve", "--vault-root", VAULTS, "--state", SERVER_STATE, "--port", str(PORT)],
        stdout=open(SERVER_LOG, "a"), stderr=subprocess.STDOUT,
    )
    wait(lambda: _health(), "the server", timeout=60)
    return p


def _health():
    try:
        return call("GET", "/v1/health")[0] == 200
    except Exception:
        return False


class Stream:
    """An SSE reader, one event at a time (agent_e2e.py's)."""

    def __init__(self, path, auth, timeout=20):
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

    def until(self, predicate, limit=2000):
        text, last_id = "", None
        for _ in range(limit):
            ev = self.next()
            if ev is None:
                return text, last_id
            event, eid, data = ev
            if event == "output":
                text += base64.b64decode(data).decode(errors="replace")
                last_id = int(eid)
            if predicate(event, data, text):
                return text, last_id
        raise RuntimeError(f"stream never satisfied the predicate; got {text[-300:]!r}")

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

    def launch(self, host_id, provider, cols=80, rows=24):
        body = {"host_id": host_id, "workspace": WS, "provider": provider,
                "terminal": {"cols": cols, "rows": rows}}
        status, rec = call("POST", "/v1/agent/sessions", body, auth=self.auth)
        if status != 200:
            raise RuntimeError(f"launch {provider}: {status} {rec}")
        wait(lambda: self.session(rec["id"]).get("status") == "running", f"{rec['id']} running", timeout=30)
        return rec["id"]

    def write(self, sid, text):
        return call("POST", f"/v1/agent/sessions/{sid}/terminal/input", auth=self.auth, raw=text.encode())[0]

    def stream(self, sid, offset=0):
        return Stream(f"/v1/agent/sessions/{sid}/terminal/stream?offset={offset}", self.auth)

    def run(self, sid, command):
        """Types `command` into a shell session and returns the output it
        produced. The stream resumes where this session's last `run` stopped,
        so earlier output never answers for this one, and the command ends
        with a marker built by `printf`, so the terminal's echo of the typed
        line can never contain it."""
        self.n += 1
        done = f"DONE{self.n}"
        line = f"{command}; printf '%s%s\\n' DO NE{self.n}"
        s = self.stream(sid, self.offsets.get(sid, 0))
        try:
            s.until(lambda e, d, t: e == "status")
            self.write(sid, line + "\r")
            text, last = s.until(lambda e, d, t: done in t)
        finally:
            s.close()
        if last is not None:
            self.offsets[sid] = last
        return text

    def end(self, sid):
        return call("POST", f"/v1/agent/sessions/{sid}/end", auth=self.auth)[0]

    def enrollment(self):
        status, issued = call("POST", "/v1/agent/hosts/enrollments", {"server_url": BASE}, auth=self.auth)
        if status != 200:
            raise RuntimeError(f"enrollment: {status} {issued}")
        return issued["enrollment"]


# --- steps -------------------------------------------------------------------

def preflight():
    if sys.platform != "darwin":
        sys.exit("macos_acceptance.py runs on macOS only")
    if sh(["true"], sudo=True)[0] != 0:
        sys.exit("needs passwordless sudo (sudo -n true failed)")
    for b in (SERVER_BIN, RUNTIME_BIN):
        if not os.access(b, os.X_OK):
            sys.exit(f"not executable: {b}")
    if os.path.exists(PLIST) or dscl("Users", ACCOUNT):
        if os.environ.get("STORM_ACCEPT_REPLACE") != "1":
            sys.exit(f"a Runtime Host is already installed ({PLIST}); set STORM_ACCEPT_REPLACE=1 to remove it first")
        sh([RUNTIME_BIN, "uninstall", "--purge"], sudo=True)


def stub_providers():
    """AC-M3 needs `claude` and `opencode` outside /usr/bin. Real ones win.
    A stub prints what the session gave it and stays up, so the same launch
    also proves AM38's environment for a non-shell provider."""
    prefix = "/opt/homebrew/bin" if os.path.isdir("/opt/homebrew/bin") else "/usr/local/bin"
    made, real = [], {}
    for name in ("claude", "opencode"):
        found = None
        for d in AM38_PATH.split(":"):
            if os.access(os.path.join(d, name), os.X_OK):
                found = os.path.join(d, name)
                break
        if found:
            real[name] = found
            continue
        if os.environ.get("STORM_ACCEPT_STUBS") != "1":
            continue
        path = os.path.join(prefix, name)
        body = (
            "#!/bin/sh\n"
            "echo \"STUB=$(basename \"$0\") ID=$(id -un) PID=$$\"\n"
            "echo \"ENV_HOME=$HOME\"; echo \"ENV_SHELL=$SHELL\"; echo \"ENV_PATH=$PATH\"\n"
            "echo \"ARGS=$*\"\n"
            "(trap '' HUP; exec sleep 997) &\n"
            "echo \"CHILD=$!\"\n"
            "exec sleep 998\n"
        )
        rc, _ = sh(["sh", "-c", f"cat > '{path}' && chmod 755 '{path}'"], input=body)
        if rc != 0:
            rc, _ = sh(["sh", "-c", f"cat > '{path}' && chmod 755 '{path}'"], sudo=True, input=body)
        if rc == 0:
            made.append(path)
    return prefix, made, real


def main():
    preflight()
    make_work()
    server = None
    stubs = []
    owner = None
    try:
        server = start_server()
        bearer, _, _ = storm_auth.sign_in(BASE, log_path=SERVER_LOG)
        owner = Owner(bearer)

        # ---------------------------------------------------------------- AC-M1
        print("\n=== AC-M1: install ===")
        rc, out = sh([RUNTIME_BIN, "install"], sudo=True, timeout=120)
        check("M1", "sudo storm-runtime install succeeds", rc == 0, out)
        user = dscl("Users", ACCOUNT, "UniqueID", "PrimaryGroupID", "UserShell", "NFSHomeDirectory", "IsHidden")
        group = dscl("Groups", ACCOUNT, "PrimaryGroupID")
        check("M1", "the _stormruntime account exists", user is not None, user)
        check("M1", "and its group", group is not None, group)
        if user and group:
            uid = int(user.get("UniqueID", "0"))
            check("M1", "uid in 200–400, gid the same", 200 <= uid <= 400 and user.get("PrimaryGroupID") == str(uid)
                  and group.get("PrimaryGroupID") == str(uid), (user, group))
            check("M1", "no login shell", user.get("UserShell") == "/usr/bin/false", user.get("UserShell"))
            check("M1", "home is state/home", user.get("NFSHomeDirectory") == HOME, user.get("NFSHomeDirectory"))
            check("M1", "hidden", user.get("IsHidden") == "1", user.get("IsHidden"))
        for g in ("admin", "staff"):
            rc, out = sh(["dsmemberutil", "checkmembership", "-U", ACCOUNT, "-G", g])
            check("M1", f"not a member of {g}", "is not a member" in out, out)
        for path, want in [
            (BIN, ("755", "root", "wheel")),
            (CONFIG, ("644", "root", "wheel")),
            (STATE, ("700", ACCOUNT, None)),
            (HOME, ("700", ACCOUNT, None)),
            (WORKSPACES, ("750", ACCOUNT, ACCOUNT)),
            (LOG_DIR, ("750", ACCOUNT, "admin")),
            (PLIST, ("644", "root", "wheel")),
        ]:
            got = stat(path)
            ok = got is not None and got[0] == want[0] and got[1] == want[1] and (want[2] is None or got[2] == want[2])
            check("M1", f"{path} is {want[0]} {want[1]}" + (f":{want[2]}" if want[2] else ""), ok, got)
        entries = acl(WORKSPACES)
        for who in (ACCOUNT, OPERATOR):
            e = next((e for e in entries if f"user:{who} allow" in e), "")
            check("M1", f"the workspace root's ACL shares it with {who}, inheritably",
                  "file_inherit" in e and "directory_inherit" in e and "add_file" in e, entries)
        props = launchd()
        check("M1", "launchd has the job loaded", props is not None)
        check("M1", "and not running before enrollment", props is not None and props.get("state") != "running"
              and not props.get("pid"), props)
        rc, out = sh([RUNTIME_BIN, "install"], sudo=True, timeout=120)
        user2 = dscl("Users", ACCOUNT, "UniqueID")
        check("M1", "install again is an idempotent upgrade (same account)",
              rc == 0 and user and user2 and user2.get("UniqueID") == user.get("UniqueID"), out)

        # The operator makes a workspace without sudo: the ACL is what allows it.
        try:
            os.mkdir(os.path.join(WORKSPACES, WS))
            made_ws = True
        except OSError as e:
            made_ws = False
            made_err = e
        check("M7", "the operator creates a workspace in the root without sudo", made_ws,
              None if made_ws else made_err)
        if not made_ws:
            sh(["mkdir", os.path.join(WORKSPACES, WS)], user=ACCOUNT)

        prefix, stubs, real = stub_providers()

        # ---------------------------------------------------------------- AC-M2
        print("\n=== AC-M2: enrollment ===")
        enrollment = owner.enrollment()
        token = enrollment.rsplit(":", 1)[-1]
        p = subprocess.Popen(["sudo", "-n", "-u", ACCOUNT, BIN, "enroll", "--name", "mac-accept"],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, cwd="/")
        time.sleep(1.0)  # enroll is now blocked reading stdin
        rc, argv_all = sh(["ps", "-axww", "-o", "command="])
        check("M8", "the enrollment string is in no process's argv", token not in argv_all and "storm-enroll:" not in argv_all)
        out, _ = p.communicate(enrollment + "\n", timeout=60)
        check("M2", "enroll reads the string from stdin and succeeds", p.returncode == 0, out)
        m = re.search(r"\((hst_[0-9A-Z]{26})\)", out)
        host_id = m.group(1) if m else None
        check("M2", "it names the host id", host_id is not None, out)
        host = try_wait(lambda: owner.online(host_id), timeout=60)
        check("M2", "launchd starts it with no further command, and it is online", host is not None,
              (launchd(), owner.host(host_id)))
        props = launchd() or {}
        check("M2", "launchctl reports it running", props.get("state") == "running", props)
        pid = daemon_pid()
        rc, out = sh(["ps", "-o", "user=", "-p", str(pid)]) if pid else (1, "")
        check("M2", "the daemon runs as _stormruntime, not as root or the operator", out.strip() == ACCOUNT, out)
        rc, out = sh([BIN, "check"], user=ACCOUNT)
        check("M2", "storm-runtime check proves the whole path", rc == 0 and "OK" in out, out)
        rc, out = sh(["ps", "-o", "command=", "-p", str(pid)]) if pid else (1, "")
        check("M5", "launchd runs it with --exclusive-account", "--exclusive-account" in out, out)

        # ---------------------------------------------------------------- AC-M3
        print("\n=== AC-M3: providers ===")
        caps = (host or {}).get("capabilities") or {}
        avail = {p["id"]: p.get("available") for p in caps.get("providers", [])}
        for pid_, name in (("claude-code", "claude"), ("opencode", "opencode")):
            where = real.get(name) or next((s for s in stubs if s.endswith("/" + name)), None)
            check("M3", f"{pid_} is available ({where or 'not installed'})", avail.get(pid_) is True and where
                  and not where.startswith("/usr/bin/"), (avail, where))
        check("M3", "shell is available", avail.get("shell") is True, avail)
        check("M3", "the host reports its workspace", WS in caps.get("workspaces", []), caps.get("workspaces"))

        stub_sid = None
        if "claude" not in real and any(s.endswith("/claude") for s in stubs):
            stub_sid = owner.launch(host_id, "claude-code")
            s = owner.stream(stub_sid)
            text, _ = s.until(lambda e, d, t: "CHILD=" in t and t.rstrip().split("CHILD=")[-1][:1].isdigit())
            s.close()
            env = dict(re.findall(r"^(ENV_\w+|STUB|ARGS)=(.*?)\r?$", text, re.M))
            check("M3", "a session's HOME is state/home (AM38)", env.get("ENV_HOME") == HOME, env)
            check("M3", "its SHELL is /bin/zsh", env.get("ENV_SHELL") == "/bin/zsh", env)
            check("M3", "its PATH is AM38's", env.get("ENV_PATH") == AM38_PATH, env.get("ENV_PATH"))
            check("M3", "claude-code is started in default permission mode", "--permission-mode default" in env.get("ARGS", ""), env)
            check("M3", "the agent runs as _stormruntime", f"ID={ACCOUNT}" in text, text[-300:])
        else:
            manual("M3", "a non-shell session's environment", "a real `claude` is installed; checked with the shell only")
        manual("M4", "Claude Code and OpenCode take a prompt and edit a file (F2, F3)",
               "needs a logged-in CLI as _stormruntime")

        # ---------------------------------------------------------------- AC-M4
        print("\n=== AC-M4: a shell session ===")
        sid = owner.launch(host_id, "shell")
        out = owner.run(sid, "echo M4=$((6*7))")
        check("M4", "input reaches the shell and output streams back", "M4=42" in out, out[-200:])
        out = owner.run(sid, 'printf "ID=%s HOME=%s SHELL=%s ARGV0=%s\\n" "$(id -un)" "$HOME" "$SHELL" "$0"')
        check("M3", "the shell runs as _stormruntime with AM38's HOME and SHELL",
              f"ID={ACCOUNT}" in out and f"HOME={HOME}" in out and "SHELL=/bin/zsh" in out, out[-300:])
        check("M3", "the shell provider is zsh", "zsh" in out.split("ARGV0=")[-1][:20], out[-200:])
        out = owner.run(sid, 'printf "P=%s\\n" "$PATH"')
        path_line = (re.search(r"P=(/[^\r\n]*)", out) or re.search("", "")).group(0)
        check("M3", "the login shell's PATH still has the Homebrew prefix (path_helper may reorder it)",
              "/opt/homebrew/bin" in path_line or not os.path.isdir("/opt/homebrew"), path_line)
        out = owner.run(sid, "pwd")
        check("M4", "the working directory is the workspace", f"{WORKSPACES}/{WS}" in out, out[-200:])
        status, _ = call("POST", f"/v1/agent/sessions/{sid}/terminal/resize",
                         {"cols": 100, "rows": 40, "focus": True}, auth=owner.auth)
        check("M4", "resize is accepted", status == 204, status)
        out = owner.run(sid, "echo SZ=$(stty size)")
        check("M4", "the PTY follows the resize", "SZ=40 100" in out, out[-200:])
        # A client disconnects: the session does not notice. Reconnect from the
        # offset the first client had reached, and get exactly what followed.
        s = owner.stream(sid)
        _, last = s.until(lambda e, d, t: "SZ=40 100" in t)
        s.close()
        owner.write(sid, "echo AFTER-$((1+1))\r")
        time.sleep(1.0)
        check("M4", "the session survives the client disconnecting", owner.session(sid).get("status") == "running")
        s = owner.stream(sid, offset=last)
        text, _ = s.until(lambda e, d, t: "AFTER-2" in t)
        s.close()
        check("M4", "reconnecting at an offset resumes exactly there", "AFTER-2" in text and "SZ=40" not in text, text[-200:])
        manual("M4", "Mac → phone handoff (F5)", "needs the app on a Mac and a phone")

        # ---------------------------------------------------------------- AC-M7
        print("\n=== AC-M7: filesystem ===")
        out = owner.run(sid, "echo agent-wrote > agent.txt; printf 'W%s\\n' $?")
        check("M7", "the agent writes in its workspace", re.search(r"W0\b", out) is not None, out[-200:])
        agent_file = os.path.join(WORKSPACES, WS, "agent.txt")
        try:
            with open(agent_file, "a") as f:
                f.write("operator-appended\n")
            op_ok = "agent-wrote" in open(agent_file).read()
        except OSError as e:
            op_ok = e
        check("M7", "the operator reads and writes the agent's file without sudo (inherited ACL)", op_ok is True, op_ok)
        try:
            with open(os.path.join(WORKSPACES, WS, "operator.txt"), "w") as f:
                f.write("from-operator\n")
            wrote = True
        except OSError as e:
            wrote = e
        check("M7", "the operator writes in the workspace", wrote is True, wrote)
        out = owner.run(sid, "cat operator.txt && echo more >> operator.txt; printf 'R%s\\n' $?")
        check("M7", "the agent reads and writes the operator's file", "from-operator" in out and re.search(r"R0\b", out) is not None, out[-200:])
        documents = os.path.join(OPERATOR_HOME, "Documents")
        for label, path in [("the operator's home", OPERATOR_HOME), ("~/Documents", documents),
                            ("the server's state tree", SERVER_STATE), ("the vaults", VAULTS)]:
            out = owner.run(sid, f"ls '{path}' >/dev/null 2>&1; printf 'RC%s\\n' $?")
            rc_ = re.search(r"RC(\d+)", out)
            check("M7", f"the agent is refused {label}", rc_ and rc_.group(1) != "0", out[-200:])
        out = owner.run(sid, f"cat '{SERVER_STATE}/auth.db' >/dev/null 2>&1; printf 'RC%s\\n' $?")
        check("M7", "the agent cannot read auth.db", re.search(r"RC[1-9]", out) is not None, out[-200:])
        manual("M7", "nothing needs Full Disk Access", "inspect System Settings > Privacy & Security > Full Disk Access: storm-runtime is absent")

        # ---------------------------------------------------------------- AC-M5
        print("\n=== AC-M5: process lifecycle ===")
        if stub_sid:
            s = owner.stream(stub_sid)
            text, _ = s.until(lambda e, d, t: "CHILD=" in t)
            s.close()
            leader = int(re.search(r"PID=(\d+)", text).group(1))
            child = int(re.search(r"CHILD=(\d+)", text).group(1))
            check("M5", "the agent and its HUP-ignoring child share the session's group",
                  child in group_members(leader), group_members(leader))
            owner.end(stub_sid)
            wait(lambda: owner.session(stub_sid).get("status") == "stopped", "stopped", timeout=20)
            gone = try_wait(lambda: not group_members(leader) and not alive(child), timeout=15)
            check("M5", "End leaves no process of the session's group", gone, group_members(leader))
        out = owner.run(sid, "echo LEAD=$$; (trap '' HUP; exec sleep 991) & echo BG=$!")
        lead = int(re.search(r"LEAD=(\d+)", out).group(1))
        bg = int(re.search(r"BG=(\d+)", out).group(1))
        check("M5", "the interactive shell put its job in a group of its own (job control)",
              bg not in group_members(lead), (lead, bg))
        owner.end(sid)
        wait(lambda: owner.session(sid).get("status") == "stopped", "stopped", timeout=20)
        gone = try_wait(lambda: not group_members(lead), timeout=15)
        check("M5", "End leaves no process of the shell's own group", gone, group_members(lead))
        gone_job = try_wait(lambda: not alive(bg), timeout=10)
        # Freeze §7.3: nothing the session spawned outlives it. A job the
        # interactive shell moved into its own process group is outside
        # kill(-pgid); only the sweep below reaches it.
        check("M5", "End also ends a job the shell moved to its own group (§7.3)", gone_job,
              f"pid {bg} still alive after End")

        # The host stops while an agent's HUP-ignoring child runs.
        sid = owner.launch(host_id, "shell")
        out = owner.run(sid, "(trap '' HUP; exec sleep 992) & echo BG=$!")
        bg = int(re.search(r"BG=(\d+)", out).group(1))
        before = daemon_pid()
        runs_before = runs()
        sh(["launchctl", "kill", "TERM", SERVICE], sudo=True)
        gone = try_wait(lambda: not alive(bg), timeout=25)
        check("M5", "launchctl kill TERM: the agent's child does not survive the host", gone, bg)
        back = try_wait(lambda: (daemon_pid() or before) != before and owner.online(host_id), timeout=60)
        check("M5", "launchd restarts the enrolled host after it stops", back is not None, launchd())
        rec = try_wait(lambda: (lambda r: r if r.get("status") == "failed" else None)(owner.session(sid)), timeout=60) or {}
        check("M5", "its session is failed (host_restart), and nothing restarted it",
              (rec.get("status"), rec.get("end_reason")) == ("failed", "host_restart"), rec)
        procs = account_processes()
        check("M5", "after the restart the account runs only the host", [p for p in procs if p[0] != daemon_pid()] == [], procs)
        last_exit = (launchd() or {}).get("last exit code", "")
        check("M5", "the stopped host exited cleanly (0)", last_exit.startswith("0"), last_exit)
        del runs_before

        # The host is killed outright: only the startup sweep can end the child.
        sid = owner.launch(host_id, "shell")
        out = owner.run(sid, "(trap '' HUP; exec sleep 993) & echo BG=$!")
        bg = int(re.search(r"BG=(\d+)", out).group(1))
        before = daemon_pid()
        sh(["kill", "-9", str(before)], sudo=True)
        time.sleep(1)
        check("M5", "with the host killed (-9), the HUP-ignoring child is still there", alive(bg), bg)
        back = try_wait(lambda: (daemon_pid() or before) != before and owner.online(host_id), timeout=60)
        check("M5", "launchd restarts a host killed with SIGKILL", back is not None, launchd())
        gone = try_wait(lambda: not alive(bg), timeout=20)
        check("M5", "the restarted host's sweep ends the survivor", gone, bg)
        rec = try_wait(lambda: (lambda r: r if r.get("status") == "failed" else None)(owner.session(sid)), timeout=60) or {}
        check("M5", "its session is failed (host_restart)", (rec.get("status"), rec.get("end_reason")) == ("failed", "host_restart"), rec)

        # ---------------------------------------------------------------- AC-M2
        before = daemon_pid()
        sh(["launchctl", "kickstart", "-k", SERVICE], sudo=True)
        back = try_wait(lambda: (daemon_pid() or before) != before and owner.online(host_id), timeout=60)
        check("M2", "online again after launchctl kickstart -k", back is not None, launchd())
        manual("M2", "online again after a reboot", "reboot the Mac and look at Hosts")

        # ---------------------------------------------------------------- AC-M7
        print("\n=== AC-M7: refused workspace roots ===")
        _, original = sh(["cat", CONFIG], sudo=True, check_rc=True)
        for bad in (os.path.join(OPERATOR_HOME, "storm-accept-ws"), "/Volumes/StormAccept/ws", "/Network/StormAccept"):
            cfg = re.sub(r"(?m)^workspace_roots\s*=.*$", f'workspace_roots = ["{bad}"]', original)
            if cfg == original:
                cfg = f'workspace_roots = ["{bad}"]\n' + original
            sh(["sh", "-c", f"cat > '{CONFIG}'"], sudo=True, input=cfg, check_rc=True)
            mark = sh(["sh", "-c", f"wc -c < '{LOG}'"], sudo=True)[1].strip() or "0"
            sh(["launchctl", "kickstart", "-k", SERVICE], sudo=True)
            refused = try_wait(lambda: bad in sh(["sh", "-c", f"tail -c +{int(mark) + 1} '{LOG}'"], sudo=True)[1]
                               and (launchd() or {}).get("state") != "running", timeout=30)
            check("M7", f"a root at {bad} is refused at startup", refused,
                  sh(["sh", "-c", f"tail -c +{int(mark) + 1} '{LOG}' | tail -5"], sudo=True)[1])
        sh(["sh", "-c", f"cat > '{CONFIG}'"], sudo=True, input=original, check_rc=True)
        sh(["launchctl", "kickstart", "-k", SERVICE], sudo=True)
        check("M7", "the host comes back with the original config",
              try_wait(lambda: owner.online(host_id), timeout=60) is not None, launchd())

        # ---------------------------------------------------------------- AC-M8
        print("\n=== AC-M8: secrets ===")
        _, host_log = sh(["cat", LOG], sudo=True)
        logs = host_log + open(SERVER_LOG).read()
        check("M8", "no enrollment token in any log", token not in logs and not re.search(r"sen_[0-9A-Z]{26}\.[A-Za-z0-9_-]{43}", logs))
        check("M8", "no host token in any log", not re.search(r"sht_[A-Za-z0-9_-]{43}", logs))
        ident = f"{STATE}/identity"
        check("M8", "identity/ is 0700 _stormruntime", (stat(ident) or ("",))[:2] == ("700", ACCOUNT), stat(ident))
        _, keys = sh(["sh", "-c", f"ls '{ident}'"], sudo=True)
        modes = [stat(f"{ident}/{k}") for k in keys.split()]
        check("M8", "every host key is 0600 _stormruntime", modes and all(m and m[:2] == ("600", ACCOUNT) for m in modes), modes)
        try:
            os.listdir(STATE)
            hidden = False
        except PermissionError:
            hidden = True
        check("M8", "the operator cannot read the host's state without sudo", hidden)

        # ---------------------------------------------------------------- AC-M6
        print("\n=== AC-M6: revocation ===")
        doomed = owner.launch(host_id, "shell")
        status, _ = call("DELETE", f"/v1/agent/hosts/{host_id}", auth=owner.auth)
        check("M6", "the owner revokes the host", status == 204, status)
        rec = owner.session(doomed)
        check("M6", "its sessions fail as host_revoked", (rec.get("status"), rec.get("end_reason")) == ("failed", "host_revoked"), rec)
        stopped = try_wait(lambda: (lambda p: p if p.get("state") != "running" and not p.get("pid") else None)(launchd() or {}), timeout=90)
        check("M6", "the host stops", stopped is not None, launchd())
        check("M6", "and exited 3", (launchd() or {}).get("last exit code", "").startswith("3"), launchd())
        check("M6", "host.json is now host.json.revoked",
              sh(["test", "-f", f"{STATE}/host.json.revoked"], sudo=True)[0] == 0
              and sh(["test", "-e", f"{STATE}/host.json"], sudo=True)[0] != 0)
        r0 = runs()
        time.sleep(30)
        p = launchd() or {}
        check("M6", "launchd does not restart it (30 s, no new runs)", runs() == r0 and p.get("state") != "running", (r0, p))
        check("M6", "its agents are gone", account_processes() == [], account_processes())
        enrollment = owner.enrollment()
        p = subprocess.run(["sudo", "-n", "-u", ACCOUNT, BIN, "enroll", "--name", "mac-accept-2"],
                           input=enrollment + "\n", capture_output=True, text=True, cwd="/", timeout=60)
        check("M6", "re-enrolling needs no --force", p.returncode == 0, p.stdout + p.stderr)
        m = re.search(r"\((hst_[0-9A-Z]{26})\)", p.stdout)
        new_id = m.group(1) if m else None
        check("M6", "and launchd brings the host back online", new_id and try_wait(lambda: owner.online(new_id), timeout=60),
              launchd())

        # ---------------------------------------------------------------- uninstall
        print("\n=== AC-M1: uninstall ===")
        if os.path.isdir(ARTIFACTS):
            sh(["sh", "-c", f"cp '{LOG}' '{ARTIFACTS}/storm-runtime.log' && chmod 644 '{ARTIFACTS}/storm-runtime.log'"], sudo=True)
        rc, out = sh([BIN, "uninstall", "--purge"], sudo=True, timeout=120)
        check("M1", "uninstall --purge succeeds", rc == 0, out)
        check("M1", "the job is gone", launchd() is None)
        check("M1", "the plist is gone", not os.path.exists(PLIST))
        check("M1", "the binary is gone", not os.path.exists(BIN))
        check("M1", "the account and group are gone", dscl("Users", ACCOUNT) is None and dscl("Groups", ACCOUNT) is None)
        check("M1", "the state is gone", sh(["test", "-e", STATE], sudo=True)[0] != 0)
        check("M1", "the workspaces are kept", os.path.exists(agent_file))
        manual("M9", "Linux unchanged", "agent_e2e.py and the runtime suite run in Linux CI")
    except Exception as e:  # a crash is a FAIL, with what it was doing
        check("--", "the harness ran to completion", False, f"{type(e).__name__}: {e}")
    finally:
        for path in stubs:
            if sh(["rm", "-f", path])[0] != 0:
                sh(["rm", "-f", path], sudo=True)
        if os.path.exists(PLIST) or dscl("Users", ACCOUNT):
            if os.path.isdir(ARTIFACTS):
                sh(["sh", "-c", f"cp '{LOG}' '{ARTIFACTS}/storm-runtime.log' && chmod 644 '{ARTIFACTS}/storm-runtime.log'"], sudo=True)
                # launchd's own view: state, runs, last exit code or signal.
                sh(["sh", "-c", f"launchctl print system/{LABEL} > '{ARTIFACTS}/launchctl-print.txt' 2>&1; "
                    f"log show --last 15m --style compact --predicate 'process == \"launchd\" AND eventMessage CONTAINS \"{LABEL}\"' "
                    f"> '{ARTIFACTS}/launchd-log.txt' 2>&1; chmod 644 '{ARTIFACTS}'/*.txt"], sudo=True)
            for b in (BIN, RUNTIME_BIN):
                if sh([b, "uninstall", "--purge"], sudo=True)[0] == 0:
                    break
        if os.environ.get("STORM_ACCEPT_CLEANUP") == "1":
            sh(["rm", "-rf", WORKSPACES], sudo=True)
            sh(["rmdir", ROOT], sudo=True)
        if server and server.poll() is None:
            server.kill()
        if ARTIFACTS != WORK and os.path.isdir(ARTIFACTS):
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
    print(f"logs: {SERVER_LOG}" + (f" (copied to {ARTIFACTS})" if ARTIFACTS != WORK else ""))
    sys.exit(1 if counts["FAIL"] else 0)


if __name__ == "__main__":
    main()
