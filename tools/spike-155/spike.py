#!/usr/bin/env python3
"""Decision 155 spike dev stack: a storm-server + storm-runtime from the spike
branch, own state/ports, a throwaway HOME. Sessions are launched and typed
into through the server's public API, as a client does."""
import base64, http.client, json, os, shutil, signal, socket, subprocess, sys, time, urllib.request, urllib.error, threading

WT = "/var/lib/storm-runtime/workspaces/scratch/Storm/.claude/worktrees/155-spike"
SERVER_BIN = "/tmp/s155/bin/storm-server"
RUNTIME_BIN = "/tmp/s155-target/debug/storm-runtime"
WORK = os.path.dirname(os.path.abspath(__file__))
ST = os.path.join(WORK, "stack.json")
VERS = "/var/lib/storm-runtime/home/.local/share/claude/versions"
sys.path.insert(0, f"{WT}/apps/server/tests")

def load():
    return json.load(open(ST)) if os.path.exists(ST) else {}

def save(d):
    json.dump(d, open(ST, "w"), indent=1)

def call(base, method, path, body=None, auth=None, raw=None, timeout=10):
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    req = urllib.request.Request(f"{base}{path}", data=data, method=method)
    if body is not None and raw is None:
        req.add_header("content-type", "application/json")
    if auth:
        req.add_header("authorization", auth)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            t = r.read()
            return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e:
        t = e.read()
        try:
            return e.code, json.loads(t)
        except Exception:
            return e.code, t

def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p

def make_home(home, ws, creds="real"):
    os.makedirs(os.path.join(home, ".claude"), exist_ok=True)
    src = json.load(open("/var/lib/storm-runtime/home/.claude/.credentials.json"))
    oauth = dict(src["claudeAiOauth"])
    oauth.pop("refreshToken", None)  # never let a throwaway copy rotate the real login
    oauth.pop("refreshTokenExpiresAt", None)
    if creds == "bogus":
        oauth["accessToken"] = "sk-ant-oat01-" + "x" * 80
    p = os.path.join(home, ".claude", ".credentials.json")
    with open(p, "w") as f:
        json.dump({"claudeAiOauth": oauth}, f)
    os.chmod(p, 0o600)
    cfg = {"hasCompletedOnboarding": True, "lastOnboardingVersion": "2.1.325", "theme": "dark",
           "numStartups": 5, "hasSeenTasksHint": True, "autoUpdates": False,
           "projects": {ws: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True,
                             "allowedTools": [], "projectOnboardingSeenCount": 3}}}
    json.dump(cfg, open(os.path.join(home, ".claude.json"), "w"))

def up(version="2.1.325"):
    d = {}
    port = free_port()
    base = f"http://127.0.0.1:{port}"
    for sub in ("state", "host", "vaults/primary", "workspaces/spike", "home", "screens", "debug"):
        os.makedirs(os.path.join(WORK, sub), exist_ok=True)
    open(os.path.join(WORK, "vaults/primary/Seed.md"), "w").write("# Seed\n")
    ws = os.path.join(WORK, "workspaces/spike")
    make_home(os.path.join(WORK, "home"), ws)
    open(os.path.join(WORK, "claude-bin"), "w").write(f"{VERS}/{version}")
    wrap = os.path.join(WORK, "claude-wrap.sh")
    open(wrap, "w").write(f"""#!/bin/sh
exec "$(cat {WORK}/claude-bin)" --permission-mode default --debug-file "{WORK}/debug/claude-$$.log" "$@"
""")
    os.chmod(wrap, 0o755)
    open(os.path.join(WORK, "runtime.toml"), "w").write(f"""workspace_roots = ["{WORK}/workspaces"]
forbidden_roots = ["{WORK}/state", "{WORK}/vaults"]
max_sessions = 6
program_status = true

[[providers]]
id = "claude-code"
command = "{wrap}"

[[providers]]
id = "shell"
command = "/bin/sh"
args = ["-i"]

[[providers]]
id = "prober"
command = "/usr/bin/python3"
args = ["{WORK}/prober.py"]
""")
    slog = open(os.path.join(WORK, "server.log"), "a")
    srv = subprocess.Popen([SERVER_BIN, "serve", "--vault-root", os.path.join(WORK, "vaults"),
                            "--state", os.path.join(WORK, "state"), "--port", str(port)],
                           stdout=slog, stderr=subprocess.STDOUT, start_new_session=True)
    for _ in range(120):
        try:
            if call(base, "GET", "/v1/health")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)
    for k in ("STORM_SESSION", "STORM_DEVICE"):
        os.environ.pop(k, None)
    import storm_auth
    owner, device, _ = storm_auth.sign_in(base, log_path=os.path.join(WORK, "server.log"))
    st, issued = call(base, "POST", "/v1/agent/hosts/enrollments", {"server_url": base}, auth=owner)
    assert st == 200, issued
    e = subprocess.run([RUNTIME_BIN, "enroll", "--state", os.path.join(WORK, "host"), "--name", "spike-host"],
                       input=(issued["enrollment"] + "\n").encode(), capture_output=True)
    assert e.returncode == 0, e.stderr
    host_id = json.load(open(os.path.join(WORK, "host", "host.json")))["host_id"]
    d.update(base=base, owner=owner, device=device, host_id=host_id, server_pid=srv.pid, port=port)
    save(d)
    start_runtime()
    print("up", base, host_id)

def start_server():
    d = load()
    slog = open(os.path.join(WORK, "server.log"), "a")
    srv = subprocess.Popen([SERVER_BIN, "serve", "--vault-root", os.path.join(WORK, "vaults"),
                            "--state", os.path.join(WORK, "state"), "--port", str(d["port"])],
                           stdout=slog, stderr=subprocess.STDOUT, start_new_session=True)
    d["server_pid"] = srv.pid
    save(d)
    for _ in range(120):
        try:
            if call(d["base"], "GET", "/v1/health")[0] == 200:
                print("server up"); return
        except Exception:
            pass
        time.sleep(0.25)

def runtime_env():
    env = {"HOME": os.path.join(WORK, "home"), "PATH": "/usr/local/bin:/usr/bin:/bin",
           "USER": os.environ.get("USER", "storm-runtime"), "LANG": "C.UTF-8",
           "RUST_LOG": "info,program_status_raw=debug",
           "STORM_PROGRAM_STATUS_LOG": os.path.join(WORK, "program-status.jsonl"),
           "STORM_SPIKE_SCREEN_DIR": os.path.join(WORK, "screens"),
           "STORM_SPIKE_ATTENTION_HOOKS": "1",
           "STORM_SPIKE_HOOK_LOG": os.path.join(WORK, "hooks.log")}
    return env

def start_runtime():
    d = load()
    rlog = open(os.path.join(WORK, "runtime.log"), "a")
    rt = subprocess.Popen([RUNTIME_BIN, "serve", "--state", os.path.join(WORK, "host"),
                           "--config", os.path.join(WORK, "runtime.toml")],
                          stdout=rlog, stderr=subprocess.STDOUT, env=runtime_env(), start_new_session=True)
    d["runtime_pid"] = rt.pid
    save(d)
    for _ in range(100):
        st, hosts = call(d["base"], "GET", "/v1/agent/hosts", auth=d["owner"])
        h = next((h for h in hosts or [] if h["id"] == d["host_id"]), None)
        if h and h["status"] == "online" and h.get("capabilities"):
            print("host online:", [p["id"] for p in h["capabilities"]["providers"]])
            return
        time.sleep(0.2)
    print("host did not come online")

def launch(provider="claude-code", model="haiku"):
    model = os.environ.get("SPIKE_MODEL", model)
    d = load()
    body = {"host_id": d["host_id"], "workspace": "spike", "provider": provider,
            "terminal": {"cols": 120, "rows": 40}}
    if model and provider == "claude-code":
        body["model"] = model
    st, rec = call(d["base"], "POST", "/v1/agent/sessions", body, auth=d["owner"])
    print(st, json.dumps(rec)[:400] if not isinstance(rec, dict) else rec.get("id"), time.time())
    return rec

def client(sid, seconds=20):
    """A fast client: reads the stream and answers DA1 at once, logging when."""
    d = load()
    out = open(os.path.join(WORK, f"client-{sid}.log"), "a")
    conn = http.client.HTTPConnection("127.0.0.1", d["port"], timeout=seconds + 5)
    conn.request("GET", f"/v1/agent/sessions/{sid}/terminal/stream?offset=0",
                 headers={"authorization": d["owner"], "accept": "text/event-stream"})
    resp = conn.getresponse()
    end = time.time() + seconds
    buf = b""; answered = False
    while time.time() < end:
        line = resp.readline()
        if not line:
            break
        if line.startswith(b"data:"):
            try:
                chunk = base64.b64decode(line[5:].strip())
            except Exception:
                continue
            buf += chunk
            if b"\x1b]7501;?" in chunk:
                print(time.time(), "client saw the 7501 probe", file=out, flush=True)
            if not answered and b"\x1b[c" in buf:
                answered = True
                t = time.time()
                st, _ = call(d["base"], "POST", f"/v1/agent/sessions/{sid}/terminal/input", auth=d["owner"], raw=b"\x1b[?62;22c")
                print(t, "client saw DA1, answered", st, file=out, flush=True)

def keys(sid, text):
    d = load()
    st, _ = call(d["base"], "POST", f"/v1/agent/sessions/{sid}/terminal/input", auth=d["owner"], raw=text.encode())
    print("keys", repr(text), st, f"{time.time():.3f}")

def prompt(sid, text):
    keys(sid, text); time.sleep(0.8); keys(sid, "\r")

def screen(sid, rows=40, cols=120):
    import pyte
    s = pyte.Screen(cols, rows); stream = pyte.ByteStream(s)
    stream.feed(open(os.path.join(WORK, "screens", f"{sid}.out"), "rb").read())
    lines = [l.rstrip() for l in s.display]
    while lines and not lines[-1]:
        lines.pop()
    print("\n".join(lines))

def log(sid=None, since=0.0, posts=False):
    since = float(since); posts = posts in (True, "1", "True")
    t0 = None
    for line in open(os.path.join(WORK, "program-status.jsonl")):
        e = json.loads(line)
        if e["t"] < since:
            continue
        if sid and e.get("session") not in (sid, None) and e.get("body", {}).get("x") is None and e["ev"] != "post":
            continue
        if e["ev"] == "post" and not posts:
            continue
        b = e.get("body")
        if b:
            print(f'{e["t"]:.3f} post activity={b.get("activity")} prog={b.get("program_status")} src={b.get("program_source")} attn={b.get("program_attention")} title={b.get("title")!r}')
        else:
            print(f'{e["t"]:.3f} {e["ev"]} {e.get("raw")!r} {e.get("seen","")}')

def hooks(since=0.0):
    since = float(since)
    p = os.path.join(WORK, "hooks.log")
    if not os.path.exists(p):
        return
    for line in open(p):
        ts, ev, rest = (line.split(" ", 2) + ["", ""])[:3]
        try:
            if float(ts) < since:
                continue
        except ValueError:
            continue
        j = {}
        try:
            j = json.loads(rest)
        except Exception:
            pass
        extra = {k: j.get(k) for k in ("notification_type", "message", "tool_name", "stop_hook_active") if j.get(k) is not None}
        print(f"{float(ts):.3f} HOOK {ev} {json.dumps(extra)[:160]}")

def session(sid):
    d = load()
    st, rec = call(d["base"], "GET", f"/v1/agent/sessions/{sid}", auth=d["owner"])
    keys_ = ("status", "activity", "needs_attention", "title")
    print({k: rec.get(k) for k in keys_} if isinstance(rec, dict) else rec)

def end(sid):
    d = load()
    print(call(d["base"], "POST", f"/v1/agent/sessions/{sid}/end", {}, auth=d["owner"]))

def down():
    d = load()
    for k in ("runtime_pid", "server_pid"):
        if d.get(k):
            try:
                os.killpg(d[k], signal.SIGTERM)
            except ProcessLookupError:
                pass
    print("down")

def when(sid, text, since=0.0):
    since = float(since)
    raw = open(os.path.join(WORK, "screens", f"{sid}.out"), "rb").read()
    needle = text.encode()
    for line in open(os.path.join(WORK, "screens", f"{sid}.idx")):
        t, at, n = line.split(); t = float(t); at = int(at); n = int(n)
        if t < since:
            continue
        if needle in raw[max(0, at - len(needle)):at + n]:
            print(f"{t:.3f} first chunk with {text!r}")
            return
    print("not found", text)


def resize(sid, cols, rows):
    d = load()
    print(call(d["base"], "POST", f"/v1/agent/sessions/{sid}/terminal/resize", {"cols": int(cols), "rows": int(rows)}, auth=d["owner"]))


def launch_client(provider="claude-code", seconds="12"):
    rec = launch(provider)
    open(os.path.join(WORK, "sidB"), "w").write(rec["id"])
    client(rec["id"], int(seconds))

if __name__ == "__main__":
    cmd, *a = sys.argv[1:]
    a = [bytes(x, "utf-8").decode("unicode_escape") if cmd in ("keys",) else x for x in a]
    globals()[cmd](*a)
