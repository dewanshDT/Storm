#!/usr/bin/env python3
"""`storm-runtime mcp-connect` against a real storm-server (decision 105c).

Starts its own server on a port in 18870–18879 (never 8484–8487), claims it,
mints an `stk_` key, and drives the real binary over stdio: `initialize`,
`tools/list` and a `get_vault` round trip, then a wrong pinned key, which must
be `identity_failed` with nothing sent.

Usage:

    SERVER_BIN=apps/server/target/debug/storm-server \\
    RUNTIME_BIN=apps/runtime/target/debug/storm-runtime \\
        python3 apps/runtime/tests/mcp_connect_live.py

Exits non-zero if any check fails.
"""
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "server", "tests"))
import storm_auth  # noqa: E402

SERVER_BIN = os.environ["SERVER_BIN"]
RUNTIME_BIN = os.environ["RUNTIME_BIN"]

ok = 0
fail = 0


def check(label, cond, detail=""):
    global ok, fail
    if cond:
        ok += 1
        print(f"  ok   {label}")
    else:
        fail += 1
        print(f"  FAIL {label} {detail}")


def free_port():
    for port in range(18870, 18880):
        with socket.socket() as s:
            try:
                s.bind(("127.0.0.1", port))
                return port
            except OSError:
                continue
    raise RuntimeError("no free port in 18870–18879")


def connect(base, sid, pk, key, lines):
    """Runs mcp-connect, writes `lines`, closes stdin, returns (stdout messages, stderr)."""
    p = subprocess.run(
        [RUNTIME_BIN, "mcp-connect", "--server", sid, "--pk", pk, "--addr", base],
        input="".join(json.dumps(l) + "\n" for l in lines).encode(),
        capture_output=True,
        env={**os.environ, "STORM_MCP_KEY": key},
        timeout=30,
    )
    out = [json.loads(l) for l in p.stdout.decode().splitlines() if l.strip()]
    return p.returncode, out, p.stderr.decode()


def main():
    work = tempfile.mkdtemp(prefix="mcp-connect-live-")
    os.makedirs(os.path.join(work, "vaults", "primary"))
    with open(os.path.join(work, "vaults", "primary", "Seed.md"), "w") as f:
        f.write("# Seed\n\nA starter note.\n")
    port = free_port()
    base = f"http://127.0.0.1:{port}"
    log_path = os.path.join(work, "server.log")
    log = open(log_path, "w")
    server = subprocess.Popen(
        [SERVER_BIN, "serve", "--vault-root", os.path.join(work, "vaults"),
         "--state", os.path.join(work, "state"), "--port", str(port), "--mcp"],
        stdout=log, stderr=subprocess.STDOUT,
    )
    try:
        for _ in range(120):
            try:
                urllib.request.urlopen(f"{base}/v1/health", timeout=1)
                break
            except Exception:
                time.sleep(0.25)
        else:
            raise RuntimeError("server did not start")

        os.environ.pop("STORM_SESSION", None)
        session, _device, _user = storm_auth.sign_in(base, log_path=log_path)
        key = storm_auth.mint_mcp_key(base, session).removeprefix("Bearer ")
        with urllib.request.urlopen(f"{base}/v1/server") as r:
            info = json.loads(r.read())
        sid, pk = info["server_id"], info["public_key"]

        print("\n=== initialize, tools/list, get_vault ===")
        init = {"jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "live", "version": "0"}}}
        code, out, err = connect(base, sid, pk, key, [
            init,
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
             "params": {"name": "list_vaults", "arguments": {}}},
        ])
        check("it exits cleanly when stdin closes", code == 0, err)
        by_id = {m.get("id"): m for m in out}
        check("one answer per request, none for the notification", len(out) == 3, out)
        check("initialize answers", "result" in by_id.get(0, {}), by_id.get(0))
        tools = {t["name"] for t in by_id.get(1, {}).get("result", {}).get("tools", [])}
        check("tools/list lists get_vault", "get_vault" in tools, tools)
        vaults = by_id.get(2, {}).get("result", {}).get("structuredContent", {}).get("vaults", [])
        check("list_vaults finds the vault", len(vaults) >= 1, by_id.get(2))
        vault_id = vaults[0]["id"] if vaults else "none"

        code, out, err = connect(base, sid, pk, key, [
            init,
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
             "params": {"name": "get_vault", "arguments": {"vault": vault_id}}},
            {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
             "params": {"name": "get_vault", "arguments": {"vault": "no-such-vault"}}},
        ])
        by_id = {m.get("id"): m for m in out}
        got = by_id.get(3, {}).get("result", {})
        check("get_vault round-trips", got.get("isError") is not True and "content" in got, by_id.get(3))
        bad = by_id.get(4, {})
        check("a tool error is a tool error, not a transport error",
              bad.get("result", {}).get("isError") is True and "error" not in bad, bad)
        check("the key is not in the log", key not in err, err)

        print("\n=== a 2026-07-28 request, and a batch ===")
        # SEP-2243: from 2026-07-28 rmcp refuses a request without
        # Mcp-Method / Mcp-Name headers, which only the adapter can add.
        meta = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {}}
        code, out, err = connect(base, sid, pk, key, [
            {"jsonrpc": "2.0", "id": 5, "method": "tools/call",
             "params": {"name": "get_vault", "arguments": {"vault": vault_id}, "_meta": meta}},
            [{"jsonrpc": "2.0", "id": 6, "method": "tools/list"},
             {"jsonrpc": "2.0", "method": "notifications/initialized"},
             {"jsonrpc": "2.0", "id": 7, "method": "ping"}],
        ])
        by_id = {m.get("id"): m for m in out if isinstance(m, dict)}
        check("a 2026-07-28 call passes the header checks",
              by_id.get(5, {}).get("result", {}).get("isError") is not True
              and "result" in by_id.get(5, {}), by_id.get(5))
        batch = [m for m in out if isinstance(m, list)]
        check("a batch is answered as one array of its two requests",
              len(batch) == 1 and sorted(m.get("id") for m in batch[0]) == [6, 7], out)

        print("\n=== a wrong pinned key ===")
        other = "Kay64UG8yvCyLhqU000LxzYeUm0L_hLIl5S8kyKWbdc"
        code, out, err = connect(base, sid, other, key, [init])
        check("identity_failed", len(out) == 1 and
              out[0].get("error", {}).get("data", {}).get("storm_error") == "identity_failed", out)

        print("\n=== a bad key ===")
        code, out, err = connect(base, sid, pk, "stk_notakey", [init])
        check("unauthorized, from the server", len(out) == 1 and
              out[0].get("error", {}).get("data", {}).get("storm_error") == "unauthorized", out)
    finally:
        server.terminate()
        server.wait(timeout=10)

    print(f"\n{ok} passed, {fail} failed")
    sys.exit(1 if fail else 0)


if __name__ == "__main__":
    main()
