#!/usr/bin/env python3
"""storm-server as a macOS LaunchAgent (decision 84d), on a disposable Mac.

Run by .github/workflows/macos-acceptance.yml on macos-latest. It installs the
branch's storm-server the way the installer will — from a directory holding the
binary, `storm-backup.sh` and a `web/` — as the runner's own user, then checks:

  S1  up: refuses root; installs the binary, web client and agents; healthy
  S2  the account commands the installer drives (has-account, pair, passwd,
      host-enrollment into a pipe, qr)
  S3  status reports the agent and health; server.json records the layout
  S4  the nightly backup agent runs the backup script into ~/Storm/backups
  S5  down stops it and it stays stopped; up brings it back (an upgrade)
  S6  uninstall removes the agents and the install, and keeps ~/Storm

Env: SERVER_BIN, BACKUP_SCRIPT, STORM_ACCEPT_ARTIFACTS (logs copied there).
"""

import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

HOME = pathlib.Path.home()
SUPPORT = HOME / "Library/Application Support/Storm"
AGENT = HOME / "Library/LaunchAgents/dev.storm.server.plist"
BACKUP_AGENT = HOME / "Library/LaunchAgents/dev.storm.server.backup.plist"
LOGS = HOME / "Library/Logs/Storm"
DATA = HOME / "Storm"
PORT = 18484
UID = os.getuid()

results = []


def check(cid, what, ok, detail=""):
    results.append((cid, ok, what, detail))
    mark = "PASS" if ok else "FAIL"
    print(f"{cid:4} {mark}  {what}" + (f" — {detail}" if detail and not ok else ""), flush=True)
    return ok


def run(*args, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    return subprocess.run([str(a) for a in args], **kw)


def healthy():
    return run("curl", "-sf", "--max-time", "2", f"http://127.0.0.1:{PORT}/v1/health").returncode == 0


def wait(pred, secs=30):
    end = time.time() + secs
    while time.time() < end:
        if pred():
            return True
        time.sleep(0.5)
    return pred()


def domain():
    return f"gui/{UID}" if run("launchctl", "print", f"gui/{UID}").returncode == 0 else f"user/{UID}"


def main():
    server_bin = pathlib.Path(os.environ["SERVER_BIN"])
    backup = pathlib.Path(os.environ["BACKUP_SCRIPT"])
    artifacts = pathlib.Path(os.environ.get("STORM_ACCEPT_ARTIFACTS", tempfile.mkdtemp()))
    artifacts.mkdir(parents=True, exist_ok=True)

    # What a release tarball extracts to.
    pkg = pathlib.Path(tempfile.mkdtemp(prefix="storm-server-pkg-"))
    shutil.copy2(server_bin, pkg / "storm-server")
    shutil.copy2(backup, pkg / "storm-backup.sh")
    (pkg / "web").mkdir()
    (pkg / "web/index.html").write_text("<!doctype html><title>Storm</title>acceptance web\n")
    exe = pkg / "storm-server"
    installed = SUPPORT / "bin/storm-server"

    try:
        # S1
        r = run("sudo", "-n", exe, "up", "--port", PORT)
        check("S1a", "up refuses root", r.returncode != 0 and "not root" in (r.stderr + r.stdout), r.stderr[-300:])
        r = run(exe, "up", "--port", PORT)
        (artifacts / "up.txt").write_text(r.stdout + r.stderr)
        check("S1b", "up succeeds as the user", r.returncode == 0, (r.stdout + r.stderr)[-600:])
        check("S1c", "binary, backup script and web are installed",
              installed.exists() and (SUPPORT / "bin/storm-backup.sh").exists()
              and (SUPPORT / "web/index.html").exists())
        check("S1d", "both launch agents are written", AGENT.exists() and BACKUP_AGENT.exists())
        check("S1e", "the server answers /v1/health", wait(healthy))
        page = run("curl", "-sf", f"http://127.0.0.1:{PORT}/").stdout
        check("S1f", "the web client is served", "acceptance web" in page, page[:200])
        check("S1g", "the data root is ~/Storm",
              all((DATA / d).is_dir() for d in ("vaults", "state", "backups")))
        mode = (DATA / "state").stat().st_mode & 0o777
        check("S1h", "state is private", mode & 0o077 == 0, oct(mode))
        ps = run("ps", "-axo", "user,command").stdout
        line = next((l for l in ps.splitlines() if str(installed) in l and " serve " in l), "")
        check("S1i", "the server runs as the user, from the installed copy",
              bool(line) and line.split()[0] == os.environ.get("USER", ""), line)

        # S2
        state = DATA / "state"
        check("S2a", "has-account says no on a fresh Storm",
              run(installed, "has-account", "--state", state).returncode == 1)
        r = run(installed, "pair", "--state", state, "--addr", f"127.0.0.1:{PORT}", "--qr")
        check("S2b", "pair draws a first-device QR", r.returncode == 0 and "storm://pair" in r.stdout, r.stderr[-300:])
        r = run(installed, "host-enrollment", "--state", state)
        check("S2c", "host-enrollment refuses without an account", r.returncode != 0 and "account" in r.stderr)
        r = run(installed, "passwd", "--state", state, "--password-stdin", input="acceptance-pass-123\n")
        check("S2d", "passwd creates the account", r.returncode == 0, r.stderr[-300:])
        check("S2e", "has-account says yes",
              run(installed, "has-account", "--state", state).returncode == 0)
        r = run(installed, "host-enrollment", "--state", state, "--url", f"http://127.0.0.1:{PORT}")
        check("S2f", "host-enrollment prints a string into a pipe",
              r.returncode == 0 and r.stdout.startswith(f"storm-enroll:v1:http://127.0.0.1:{PORT}:"),
              f"rc={r.returncode} len={len(r.stdout)} {r.stderr[-200:]}")
        r = run(installed, "qr", "https://example.com/storm.apk")
        check("S2g", "qr draws a block", r.returncode == 0 and "█" in r.stdout)

        # S3
        r = run(installed, "status")
        check("S3a", "status reports the agent and health", r.returncode == 0 and "health  : ok" in r.stdout, r.stdout)
        rec = json.loads((SUPPORT / "server.json").read_text())
        check("S3b", "server.json records the layout",
              rec["port"] == PORT and rec["state"] == str(state), json.dumps(rec))

        # S4
        d = domain()
        run("launchctl", "kickstart", f"{d}/dev.storm.server.backup")
        today = time.strftime("%Y-%m-%d", time.gmtime())
        ok = wait(lambda: (DATA / "backups" / today / "index").is_dir(), 60)
        log = (LOGS / "storm-backup.log")
        if log.exists():
            shutil.copy2(log, artifacts / "storm-backup.log")
        check("S4", "the backup agent writes a dated backup", ok,
              log.read_text()[-600:] if log.exists() else "no backup log")

        # S5
        r = run(installed, "down")
        check("S5a", "down stops the server", r.returncode == 0 and wait(lambda: not healthy(), 20))
        time.sleep(5)
        check("S5b", "it stays stopped (not restarted by KeepAlive)", not healthy())
        r = run(exe, "up", "--port", PORT)
        check("S5c", "up again (an upgrade from the package) brings it back",
              r.returncode == 0 and wait(healthy), r.stderr[-300:])
        check("S5d", "the account survived the upgrade",
              run(installed, "has-account", "--state", state).returncode == 0)

        # S6
        (DATA / "vaults/keep").mkdir(parents=True, exist_ok=True)
        (DATA / "vaults/keep/note.md").write_text("# kept\n")
        r = run(installed, "uninstall")
        check("S6a", "uninstall succeeds", r.returncode == 0, r.stderr[-300:])
        check("S6b", "the server is gone", wait(lambda: not healthy(), 20))
        check("S6c", "agents and the install are removed",
              not AGENT.exists() and not BACKUP_AGENT.exists() and not SUPPORT.exists())
        check("S6d", "the notes are kept", (DATA / "vaults/keep/note.md").exists())
        check("S6e", "launchd no longer knows the agent",
              run("launchctl", "print", f"{d}/dev.storm.server").returncode != 0)
    finally:
        if (LOGS / "storm-server.log").exists():
            shutil.copy2(LOGS / "storm-server.log", artifacts / "storm-server.log")
        if installed.exists():
            run(installed, "uninstall")

    failed = [c for c in results if not c[1]]
    print(f"\n{len(results) - len(failed)} passed, {len(failed)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
