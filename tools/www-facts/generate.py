#!/usr/bin/env python3
"""Generate apps/www/src/data/facts.json: what the site and the docs quote.

The site has twice described a Storm that no longer existed, once for the
token and once for accounts (decisions 65, 107), because numbers and commands
were typed by hand. These are read from the source and the built binaries
instead, and CI fails when the committed file differs.

  generate.py                      regenerate the MCP tools from mcp.rs
  generate.py --server BIN --runtime BIN   also the two CLIs' --help
  generate.py --check [...]        compare instead of writing; exit 1 on drift

Each section is only touched when its input is given, so the www job (no
Rust) checks the tools and the Rust jobs check their own CLI.
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "apps/www/src/data/facts.json"
MCP_RS = ROOT / "apps/server/src/mcp.rs"


def const_list(src, name):
    m = re.search(rf"pub const {name}: &\[&str\] = &\[(.*?)\];", src, re.S)
    if not m:
        sys.exit(f"www-facts: {name} not found in {MCP_RS}")
    return re.findall(r'"([a-z_]+)"', m.group(1))


def mcp_tools():
    src = MCP_RS.read_text()
    tools = []
    for m in re.finditer(r"#\[tool\((.*?)\)\]\s*async fn ([a-z_]+)", src, re.S):
        attr, name = m.groups()
        parts = re.findall(r'"((?:[^"\\]|\\.)*)"', attr)
        if not parts:
            sys.exit(f"www-facts: no description for {name}; the parser needs updating")
        desc = "".join(parts).encode().decode("unicode_escape").encode("latin-1").decode()
        tools.append({"name": name, "description": " ".join(desc.split())})
    if not tools:
        sys.exit(f"www-facts: no #[tool] functions in {MCP_RS}")
    # Decision 134's tools are offered to no one until 134-11 enables a host,
    # so the site does not describe them yet.
    dark = set(const_list(src, "SPAWN_TOOLS")) | set(const_list(src, "SPAWNED_TOOLS"))
    tools = [t for t in tools if t["name"] not in dark]
    write = set(const_list(src, "WRITE_TOOLS"))
    only_agents = set(const_list(src, "ONLY_FOR_AGENTS"))
    approval = set(const_list(src, "DELETE_TOOLS"))
    for t in tools:
        n = t["name"]
        t["access"] = "agents" if n in only_agents else "write" if n in write else "read"
        t["agents"] = True
        t["approval"] = n in approval
    return tools


def run_help(binary, *args):
    # A fixed environment, so the text does not depend on the terminal or on
    # whoever ran it: clap wraps to COLUMNS, and some defaults mention HOME.
    env = {"PATH": os.environ.get("PATH", ""), "COLUMNS": "100", "HOME": "/home/user", "LANG": "C.UTF-8"}
    out = subprocess.run([binary, *args, "--help"], capture_output=True, text=True, env=env, check=True)
    return out.stdout.rstrip() + "\n"


def cli(binary):
    top = run_help(binary)
    block = top.split("Commands:\n", 1)[1].split("\n\n", 1)[0]
    names = [l.split()[0] for l in block.splitlines() if l.startswith("  ") and not l.startswith("   ")]
    commands = []
    for name in names:
        if name == "help":
            continue
        line = next(l for l in block.splitlines() if l.split() and l.split()[0] == name)
        commands.append({
            "name": name,
            "summary": line.strip()[len(name):].strip(),
            "help": run_help(binary, name),
        })
    return {"help": top, "commands": commands}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", help="a built storm-server")
    ap.add_argument("--runtime", help="a built storm-runtime")
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()

    current = json.loads(OUT.read_text()) if OUT.exists() else {}
    fresh = dict(current)
    fresh["mcp_tools"] = mcp_tools()
    if a.server:
        fresh["storm_server"] = cli(a.server)
    if a.runtime:
        fresh["storm_runtime"] = cli(a.runtime)
    text = json.dumps(fresh, indent=2, ensure_ascii=False) + "\n"

    if a.check:
        stale = [k for k in fresh if fresh[k] != current.get(k)]
        if stale:
            print(f"www-facts: {OUT.relative_to(ROOT)} is stale for: {', '.join(stale)}", file=sys.stderr)
            print("  Run `make www-facts` and commit the result; the site and docs quote it.", file=sys.stderr)
            sys.exit(1)
        print(f"www-facts: ok ({', '.join(k for k in fresh)})")
        return
    OUT.write_text(text)
    print(f"www-facts: wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
