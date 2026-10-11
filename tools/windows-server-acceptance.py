#!/usr/bin/env python3
"""storm-server as a per-user Scheduled Task on Windows (decision 99,
W-S1–S3), on a disposable Windows machine.

Run by .github/workflows/windows-acceptance.yml on windows-latest. It installs
the branch's storm-server the way a release zip does — from a directory
holding `storm-server.exe`, `storm-serverw.exe` and a `web/` — as the runner's
own user, then checks AC-WS1, WS2, WS5 and WS6 (the vault's *Installer/Windows
Server*):

  WS1  up: copies both binaries and the web client, registers the logon task
       as this user, runs it windowless, and the server answers
  WS2  the data root's DACL is protected: this user, SYSTEM, Administrators
  WS5  `backup` makes a dated folder whose snapshots open, and the nightly
       task is registered
  WS6  status; down stops it and it stays stopped; up again (an upgrade);
       uninstall removes the tasks and the install, and keeps the data

Env: SERVER_BIN (the built storm-server.exe; storm-serverw.exe must be beside
it), STORM_ACCEPT_ARTIFACTS (logs copied there).
"""

import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

LOCAL = pathlib.Path(os.environ["LOCALAPPDATA"])
SUPPORT = LOCAL / "Storm"
DATA = pathlib.Path(os.environ["USERPROFILE"]) / "Storm"
PORT = 18484

results = []


def check(cid, what, ok, detail=""):
    results.append((cid, what, bool(ok)))
    print(f"  {'PASS' if ok else 'FAIL'}  [{cid}] {what}" + ("" if ok else f"   {str(detail)[:400]}"), flush=True)
    return ok


def sh(cmd, input=None, timeout=120):
    p = subprocess.run(cmd, input=input, capture_output=True, text=True, timeout=timeout,
                       encoding="utf-8", errors="replace")
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def healthy():
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/v1/health", timeout=2) as r:
            return r.status == 200
    except Exception:
        return False


def wait(pred, timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(0.5)
    return False


def task(name):
    rc, out = sh(["schtasks", "/query", "/tn", name, "/xml"])
    return out if rc == 0 else None


def main():
    if sys.platform != "win32":
        sys.exit("windows-server-acceptance.py runs on Windows only")
    built = pathlib.Path(os.environ["SERVER_BIN"]).resolve()
    if DATA.exists() or SUPPORT.exists():
        sys.exit(f"{DATA} or {SUPPORT} already exists; this harness wants a clean profile")

    # What a release zip unpacks to.
    release = pathlib.Path(tempfile.mkdtemp(prefix="storm-server-release-"))
    for name in ("storm-server.exe", "storm-serverw.exe"):
        shutil.copy(built.parent / name, release / name)
    (release / "web").mkdir()
    (release / "web" / "index.html").write_text("<!doctype html><title>Storm</title>\n")
    server = str(release / "storm-server.exe")

    try:
        print("\n=== WS1: up ===")
        rc, out = sh([server, "up", "--port", str(PORT)], timeout=180)
        print(out)
        check("WS1", "up succeeds as a standard user's own install", rc == 0, out[-400:])
        for path in (SUPPORT / "bin" / "storm-server.exe", SUPPORT / "bin" / "storm-serverw.exe",
                     SUPPORT / "web" / "index.html", SUPPORT / "server.json"):
            check("WS1", f"{path} installed", path.exists())
        xml = task("Storm Server") or ""
        user = os.environ.get("USERNAME", "")
        check("WS1", "the logon task exists", "<LogonTrigger>" in xml, xml[:300])
        check("WS1", "it runs as this user, not elevated",
              user.lower() in xml.lower() and "<RunLevel>LeastPrivilege</RunLevel>" in xml, xml[:600])
        check("WS1", "it runs the windowless twin", "storm-serverw.exe" in xml, xml[:600])
        check("WS1", "the server answers", wait(healthy, 60))
        rc, out = sh(["powershell", "-NoProfile", "-Command",
                      "Get-Process storm-serverw -ErrorAction SilentlyContinue | "
                      "ForEach-Object { $_.MainWindowHandle }"])
        handles = [h for h in out.split() if h.strip().isdigit()]
        check("WS1", "storm-serverw runs, with no window", handles and all(h == "0" for h in handles), out)
        record = json.loads((SUPPORT / "server.json").read_text())
        check("WS1", "server.json records the data root", pathlib.Path(record["data_root"]) == DATA, record)
        log = SUPPORT / "logs" / "storm-server.log"
        check("WS1", "it logs to its file", wait(lambda: log.exists() and log.stat().st_size > 0, 20))

        print("\n=== WS2: owner-only data ===")
        rc, acl = sh(["icacls", str(DATA)])
        check("WS2", "the data root's DACL names this user, SYSTEM and Administrators",
              user.lower() in acl.lower() and "NT AUTHORITY\\SYSTEM" in acl and "BUILTIN\\Administrators" in acl, acl)
        check("WS2", "and no one else", "BUILTIN\\Users" not in acl and "Everyone" not in acl
              and "Authenticated Users" not in acl and "(I)" not in acl, acl)

        print("\n=== WS6: status, down, up ===")
        rc, out = sh([server, "status"])
        check("WS6", "status says healthy", rc == 0 and "health  : ok" in out, out)
        rc, out = sh([server, "down"])
        check("WS6", "down stops it", rc == 0 and wait(lambda: not healthy(), 30), out)
        time.sleep(5)
        check("WS6", "and it stays stopped", not healthy())
        xml = task("Storm Server") or ""
        check("WS6", "its task is disabled", "<Enabled>false</Enabled>" in xml, xml[-400:])
        rc, out = sh([server, "up", "--port", str(PORT)], timeout=180)
        check("WS6", "up again brings it back (an upgrade)", rc == 0 and wait(healthy, 60), out[-400:])

        print("\n=== WS5: backup ===")
        check("WS5", "the nightly backup task exists", "<CalendarTrigger>" in (task("Storm Backup") or ""))
        binw = str(SUPPORT / "bin" / "storm-server.exe")
        rc, out = sh([binw, "backup", "--vault-root", record["vaults"], "--state", record["state"],
                      "--dest", record["backups"]], timeout=180)
        dated = [d for d in pathlib.Path(record["backups"]).iterdir() if re.fullmatch(r"\d{4}-\d\d-\d\d", d.name)]
        check("WS5", "backup makes a dated folder", rc == 0 and len(dated) == 1, out[-400:])
        if dated:
            check("WS5", "with the vaults and the index", (dated[0] / "vaults").is_dir()
                  and (dated[0] / "index").is_dir(), list(dated[0].iterdir()))
        old = pathlib.Path(record["backups"]) / "2000-01-01"
        old.mkdir()
        rc, out = sh([binw, "backup", "--vault-root", record["vaults"], "--state", record["state"],
                      "--dest", record["backups"]], timeout=180)
        check("WS5", "and prunes a dated folder past 30 days", rc == 0 and not old.exists(), out[-300:])

        print("\n=== WS6: uninstall ===")
        installed = str(SUPPORT / "bin" / "storm-server.exe")
        rc, out = sh([server, "uninstall"], timeout=120)
        check("WS6", "uninstall succeeds", rc == 0, out)
        check("WS6", "the tasks are gone", task("Storm Server") is None and task("Storm Backup") is None)
        check("WS6", "the server stopped", wait(lambda: not healthy(), 30))
        check("WS6", "the install is gone", not pathlib.Path(installed).exists())
        check("WS6", "the notes are kept", (DATA / "vaults").is_dir() and (DATA / "state").is_dir())
    finally:
        artifacts = os.environ.get("STORM_ACCEPT_ARTIFACTS")
        if artifacts:
            os.makedirs(artifacts, exist_ok=True)
            for f in (SUPPORT / "logs").glob("*.log") if (SUPPORT / "logs").exists() else []:
                shutil.copy(f, artifacts)
        sh([server, "uninstall"])

    passed = sum(1 for r in results if r[2])
    failed = len(results) - passed
    print(f"\n{passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
