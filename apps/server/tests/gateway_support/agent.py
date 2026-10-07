#!/usr/bin/env python3
"""A scripted agent for gateway_e2e.py: a stand-in for Claude Code or OpenCode.

storm-runtime starts it as a session's provider command, with exactly the MCP
configuration it would give the real CLI (AM32). It:

- records its argv and the gateway-relevant environment to `agent-env.json`;
- reads its MCP config the way the real CLI would — `--mcp-config <file>`
  (Claude Code), or `$XDG_CONFIG_HOME/opencode/opencode.json` (OpenCode) —
  and starts every stdio server in it, which is `storm-runtime mcp-bridge`;
- copies every line each bridge writes to `transcript-<slug>.jsonl`;
- sends whatever the suite drops into `inbox/` (`NNN.json`:
  `{"slug": …, "message": …}`) to that bridge, in order.

It never sees a credential: there is none to see. A test fixture, not Storm.
"""
import json
import os
import subprocess
import sys
import threading
import time

HERE = os.getcwd()
INBOX = os.path.join(HERE, "inbox")
os.makedirs(INBOX, exist_ok=True)


def record_env():
    keys = ["XDG_CONFIG_HOME", "OPENCODE_DISABLE_PROJECT_CONFIG"]
    with open(os.path.join(HERE, "agent-env.json"), "w") as f:
        json.dump({"argv": sys.argv[1:], "env": {k: os.environ.get(k) for k in keys}}, f)


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
    out = os.path.join(HERE, f"transcript-{slug}.jsonl")
    for line in proc.stdout:
        with open(out, "a") as f:
            f.write(line.decode())


def main():
    record_env()
    bridges = {}
    for slug, (argv, env) in servers().items():
        p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             env={**os.environ, **env})
        bridges[slug] = p
        threading.Thread(target=pump, args=(slug, p), daemon=True).start()
    print(f"scripted agent: {sorted(bridges)}", flush=True)
    while True:
        for name in sorted(os.listdir(INBOX)):
            if not name.endswith(".json"):
                continue
            path = os.path.join(INBOX, name)
            try:
                item = json.load(open(path))
            except json.JSONDecodeError:
                continue  # still being written
            p = bridges.get(item["slug"])
            if p:
                p.stdin.write((json.dumps(item["message"]) + "\n").encode())
                p.stdin.flush()
            os.rename(path, path + ".sent")
        time.sleep(0.05)


if __name__ == "__main__":
    main()
