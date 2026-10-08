#!/usr/bin/env python3
"""The acceptance harness's agent: a stand-in for Claude Code or OpenCode.

`storm-runtime` starts it as a session's provider command with exactly the
MCP configuration it gives the real CLI (AM32), as
`apps/server/tests/gateway_support/agent.py` does for the live suite. It
starts every bridge in that config, and then does what the harness drops
into `inbox-<session>/` (`NNNNN.json`), in order:

- `{"slug": …, "message": …}` — send an MCP message to that bridge; every
  line a bridge answers is appended to `transcript-<session>-<slug>.jsonl`;
- `{"say": "…"}` — print a line to its terminal: what it just did;
- `{"exit": 0}` — leave, with that status.

Its writes are real calls through the gateway, so Wrote rows, provenance
and versions are the server's. A test fixture, not Storm.
"""
import json
import os
import re
import subprocess
import sys
import threading
import time

HERE = os.getcwd()


def session_id():
    blob = " ".join(sys.argv[1:]) + " " + os.environ.get("XDG_CONFIG_HOME", "")
    m = re.search(r"ags_[A-Za-z0-9_-]+", blob)
    return m.group(0) if m else "unknown"


SID = session_id()
INBOX = os.path.join(HERE, f"inbox-{SID}")
os.makedirs(INBOX, exist_ok=True)


def servers():
    """{slug: (argv, env)} from whichever config this launch was given."""
    if "--mcp-config" in sys.argv:
        path = sys.argv[sys.argv.index("--mcp-config") + 1]
        cfg = json.load(open(path))["mcpServers"]
        return {s: ([c["command"], *c["args"]], c.get("env", {})) for s, c in cfg.items()}
    xdg = os.environ.get("XDG_CONFIG_HOME")
    if xdg:
        cfg = json.load(open(os.path.join(xdg, "opencode", "opencode.json")))["mcp"]
        return {s: (c["command"], c.get("environment", {})) for s, c in cfg.items()}
    return {}


def pump(slug, proc):
    out = os.path.join(HERE, f"transcript-{SID}-{slug}.jsonl")
    for line in proc.stdout:
        with open(out, "a") as f:
            f.write(line.decode())


def say(text):
    # Truecolor, as an agent CLI colours its own lines: writes stand out.
    if text.startswith("›") and re.search(r"updated|created", text):
        text = f"\x1b[38;2;157;132;240m{text}\x1b[0m"
    elif text.startswith("›"):
        text = f"\x1b[38;2;178;172;168m{text}\x1b[0m"
    sys.stdout.write(text + "\r\n")
    sys.stdout.flush()


def main():
    claude = "--mcp-config" in sys.argv
    say("$ " + ("claude" if claude else "opencode"))
    bridges = {}
    for slug, (argv, env) in servers().items():
        p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             env={**os.environ, **env})
        bridges[slug] = p
        threading.Thread(target=pump, args=(slug, p), daemon=True).start()
    say(f"› starting in {os.path.basename(HERE)}")
    sys.stdout.write("› ")
    sys.stdout.flush()
    while True:
        for name in sorted(os.listdir(INBOX)):
            if not name.endswith(".json"):
                continue
            path = os.path.join(INBOX, name)
            try:
                item = json.load(open(path))
            except json.JSONDecodeError:
                continue  # still being written
            os.rename(path, path + ".done")
            if "exit" in item:
                sys.stdout.write("\r\x1b[K")
                sys.stdout.flush()
                sys.exit(item["exit"])
            if "say" in item:
                sys.stdout.write("\r\x1b[K")
                for line in item["say"].split("\n"):
                    say(line)
                sys.stdout.write("› ")
                sys.stdout.flush()
                continue
            p = bridges.get(item.get("slug"))
            if p:
                p.stdin.write((json.dumps(item["message"]) + "\n").encode())
                p.stdin.flush()
        time.sleep(0.05)


if __name__ == "__main__":
    main()
