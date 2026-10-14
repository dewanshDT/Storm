#!/usr/bin/env python3
"""`storm-runtime mcp-connect` against a real storm-server and a real
storm-relay (decisions 105c, 105b, 105d).

Starts its own server, relay and impostor server on ten ports from
`MCP_LIVE_PORT` (default 18870; never 8484–8487). The server binds `127.0.0.1`, not a wildcard, so rmcp's
`allowed_hosts` is enforced and a relayed call proves its `Host` is accepted.
It claims the server, mints an `stk_` key, registers the relay, and drives the
real binary over stdio twice: direct, and through the relay with the address
a closed port. Then the relay-only and cached-relay cases, a `base_version`
merge against a concurrent LAN edit over the relay, and impostors.

Usage:

    SERVER_BIN=apps/server/target/debug/storm-server \\
    RELAY_BIN=apps/relay/target/debug/storm-relay \\
    RUNTIME_BIN=apps/runtime/target/debug/storm-runtime \\
        python3 apps/runtime/tests/mcp_connect_live.py

With `RELAY_IMPL=cf` and `RELAY_CF_DEV=apps/relay-cf/scripts/dev-relay.sh`,
both relays are the Worker relay under `wrangler dev`, in allowlist mode with
this server's key, at its default `HELLO` limit (decision 108c).

Exits non-zero if any check fails.
"""
import json
import os
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

SERVER_BIN = os.environ["SERVER_BIN"]
RELAY_BIN = os.environ["RELAY_BIN"]
RUNTIME_BIN = os.environ["RUNTIME_BIN"]
RELAY_IMPL = os.environ.get("RELAY_IMPL", "rust")
PORT_BASE = int(os.environ.get("MCP_LIVE_PORT", "18870"))

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


_taken = set()


def free_port():
    for port in range(PORT_BASE, PORT_BASE + 10):
        if port in _taken:
            continue
        with socket.socket() as s:
            try:
                s.bind(("127.0.0.1", port))
            except OSError:
                continue
        _taken.add(port)
        return port
    raise RuntimeError(f"no free port in {PORT_BASE}–{PORT_BASE + 9}")


def wait_for(url, what, log_path):
    for _ in range(120):
        try:
            urllib.request.urlopen(url, timeout=1)
            return
        except urllib.error.HTTPError:
            return
        except Exception:
            time.sleep(0.25)
    with open(log_path) as f:
        sys.stderr.write(f.read())
    raise RuntimeError(f"{what} did not start")


def rest(base, method, path, auth, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{base}{path}", data=data, method=method)
    req.add_header("Authorization", auth)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read()
            return r.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace")


def connect(args, key, lines, state=None):
    """Runs mcp-connect with `args`, writes `lines`, closes stdin, returns
    (exit code, stdout messages, stderr)."""
    # A fresh state directory unless a check is about the cache: a cached
    # relay set would add every registered relay to every race.
    state = state or tempfile.mkdtemp(prefix="mcp-connect-state-")
    p = subprocess.run(
        [RUNTIME_BIN, "mcp-connect", *args],
        input="".join(json.dumps(l) + "\n" for l in lines).encode(),
        capture_output=True,
        env={**os.environ, "STORM_MCP_KEY": key, "XDG_STATE_HOME": state,
             "HOME": state, "LOCALAPPDATA": state},
        timeout=60,
    )
    out = [json.loads(l) for l in p.stdout.decode().splitlines() if l.strip()]
    return p.returncode, out, p.stderr.decode()


INIT = {"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "live", "version": "0"}}}


def storm_error(m):
    return (m or {}).get("error", {}).get("data", {}).get("storm_error")


def the_suite(mode, path, sid, pk, key):
    """S3's checks, over `path` (the flags that pick a route)."""
    pin = ["--server", sid, "--pk", pk]

    print(f"\n=== {mode}: initialize, tools/list, get_vault ===")
    code, out, err = connect(pin + path, key, [
        INIT,
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
         "params": {"name": "list_vaults", "arguments": {}}},
    ])
    check(f"{mode}: it exits cleanly when stdin closes", code == 0, err)
    by_id = {m.get("id"): m for m in out}
    check(f"{mode}: one answer per request, none for the notification", len(out) == 3, out)
    check(f"{mode}: initialize answers", "result" in by_id.get(0, {}), by_id.get(0))
    tools = {t["name"] for t in by_id.get(1, {}).get("result", {}).get("tools", [])}
    check(f"{mode}: tools/list lists get_vault", "get_vault" in tools, by_id.get(1))
    vaults = by_id.get(2, {}).get("result", {}).get("structuredContent", {}).get("vaults", [])
    check(f"{mode}: list_vaults finds the vault", len(vaults) >= 1, by_id.get(2))
    primary = [v for v in vaults if v.get("name") == "primary"] or vaults
    vault_id = primary[0]["id"] if primary else "none"

    code, out, err = connect(pin + path, key, [
        INIT,
        {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
         "params": {"name": "get_vault", "arguments": {"vault": vault_id}}},
        {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
         "params": {"name": "get_vault", "arguments": {"vault": "no-such-vault"}}},
    ])
    by_id = {m.get("id"): m for m in out}
    got = by_id.get(3, {}).get("result", {})
    check(f"{mode}: get_vault round-trips",
          got.get("isError") is not True and "content" in got, by_id.get(3))
    bad = by_id.get(4, {})
    check(f"{mode}: a tool error is a tool error, not a transport error",
          bad.get("result", {}).get("isError") is True and "error" not in bad, bad)
    check(f"{mode}: the key is not in the log", key not in err, err)

    print(f"\n=== {mode}: a 2026-07-28 request, and a batch ===")
    # SEP-2243: from 2026-07-28 rmcp refuses a request without
    # Mcp-Method / Mcp-Name headers, which only the adapter can add.
    meta = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}}
    code, out, err = connect(pin + path, key, [
        {"jsonrpc": "2.0", "id": 5, "method": "tools/call",
         "params": {"name": "get_vault", "arguments": {"vault": vault_id}, "_meta": meta}},
        [{"jsonrpc": "2.0", "id": 6, "method": "tools/list"},
         {"jsonrpc": "2.0", "method": "notifications/initialized"},
         {"jsonrpc": "2.0", "id": 7, "method": "ping"}],
    ])
    by_id = {m.get("id"): m for m in out if isinstance(m, dict)}
    check(f"{mode}: a 2026-07-28 call passes the header checks",
          by_id.get(5, {}).get("result", {}).get("isError") is not True
          and "result" in by_id.get(5, {}), by_id.get(5))
    batch = [m for m in out if isinstance(m, list)]
    check(f"{mode}: a batch is answered as one array of its two requests",
          len(batch) == 1 and sorted(m.get("id") for m in batch[0]) == [6, 7], out)

    print(f"\n=== {mode}: a wrong pinned key, a bad key ===")
    other = "Kay64UG8yvCyLhqU000LxzYeUm0L_hLIl5S8kyKWbdc"
    code, out, err = connect(["--server", sid, "--pk", other] + path, key, [INIT])
    # A relay routes a HELLO only to the key it names (SRP §5, decision 116n),
    # so over the relay a wrong pin finds no server rather than a wrong one.
    wrong_pin = "identity_failed" if mode == "direct" else "server_offline"
    check(f"{mode}: {wrong_pin}", len(out) == 1 and storm_error(out[0]) == wrong_pin, out)
    code, out, err = connect(pin + path, "stk_notakey", [INIT])
    check(f"{mode}: unauthorized, from the server",
          len(out) == 1 and storm_error(out[0]) == "unauthorized", out)
    return vault_id


def start_relay(port, name, sid, pk, start, work):
    """storm-relay, or with RELAY_IMPL=cf the Worker relay under wrangler dev
    with this server allowlisted (decision 108c)."""
    if RELAY_IMPL != "cf":
        return start([RELAY_BIN, "--bind", f"127.0.0.1:{port}"], f"{name}.log")
    allowlist = os.path.join(work, f"{name}.allowlist")
    with open(allowlist, "w") as f:
        f.write(f"{sid} {pk}\n")
    return start([os.environ["RELAY_CF_DEV"], str(port), "allowlist", allowlist,
                  os.path.join(work, f"{name}-cf")], f"{name}.log")


def main():
    work = tempfile.mkdtemp(prefix="mcp-connect-live-")
    procs = []

    def start(argv, log_name):
        log_path = os.path.join(work, log_name)
        procs.append(subprocess.Popen(argv, stdout=open(log_path, "w"),
                                      stderr=subprocess.STDOUT))
        return log_path

    try:
        os.makedirs(os.path.join(work, "vaults", "primary"))
        with open(os.path.join(work, "vaults", "primary", "Seed.md"), "w") as f:
            f.write("# Seed\n\nA starter note.\n")
        # Two relays, because a relay allows one address ten HELLOs a minute
        # and every connect through a relay is one.
        port, relay_port, other_port, impostor_port, closed = (free_port() for _ in range(5))
        base = f"http://127.0.0.1:{port}"
        relay = f"ws://127.0.0.1:{relay_port}"
        other = f"ws://127.0.0.1:{other_port}"
        impostor = f"http://127.0.0.1:{impostor_port}"
        unreachable = f"http://127.0.0.1:{closed}"

        log_path = start([SERVER_BIN, "serve", "--host", "127.0.0.1", "--port", str(port),
                          "--vault-root", os.path.join(work, "vaults"),
                          "--state", os.path.join(work, "state"), "--mcp"], "server.log")
        os.makedirs(os.path.join(work, "impostor-vaults", "primary"))
        impostor_log = start([SERVER_BIN, "serve", "--host", "127.0.0.1",
                              "--port", str(impostor_port),
                              "--vault-root", os.path.join(work, "impostor-vaults"),
                              "--state", os.path.join(work, "impostor-state"), "--mcp"],
                             "impostor.log")
        wait_for(f"{base}/v1/health", "the server", log_path)
        wait_for(f"{impostor}/v1/health", "the impostor", impostor_log)

        os.environ.pop("STORM_SESSION", None)
        session, _device, _user = storm_auth.sign_in(base, log_path=log_path)
        key = storm_auth.mint_mcp_key(base, session).removeprefix("Bearer ")
        with urllib.request.urlopen(f"{base}/v1/server") as r:
            info = json.loads(r.read())
        sid, pk = info["server_id"], info["public_key"]

        relay_log = start_relay(relay_port, "relay", sid, pk, start, work)
        other_log = start_relay(other_port, "other-relay", sid, pk, start, work)
        wait_for(f"http://127.0.0.1:{relay_port}/", "the relay", relay_log)
        wait_for(f"http://127.0.0.1:{other_port}/", "the other relay", other_log)

        status, _ = rest(base, "PUT", "/v1/config/relays", session, {"relays": [relay, other]})
        check("the relays are configured", status == 200, status)
        for _ in range(80):
            with urllib.request.urlopen(f"{base}/v1/server") as r:
                if sorted(x["url"] for x in json.loads(r.read())["relays"]) == sorted([relay, other]):
                    break
            time.sleep(0.25)
        else:
            raise RuntimeError("the server never registered with both relays")

        the_suite("direct", ["--addr", base], sid, pk, key)
        vault_id = the_suite("relayed", ["--addr", unreachable, "--relay", relay], sid, pk, key)

        pin = ["--server", sid, "--pk", pk]
        list_vaults = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": "list_vaults", "arguments": {}}}

        print("\n=== the relay alone, and the relay set it caches ===")
        fresh = tempfile.mkdtemp(prefix="mcp-connect-cache-")
        code, out, err = connect(pin + ["--relay", other], key, [INIT, list_vaults], state=fresh)
        check("a relay alone reaches the server", len(out) == 2 and all("result" in m for m in out), out)
        check("it says the relay carried it", f"relay {other}" in err, err)
        cache = os.path.join(fresh, "storm", "mcp-connect-relays.json")
        try:
            with open(cache) as f:
                cached = json.load(f)
        except OSError:
            cached = {}
        check("the registered relay set is cached for this server", sorted(cached.get(sid, [])) == sorted([relay, other]), cached)
        dead = f"ws://127.0.0.1:{closed}"
        code, out, err = connect(pin + ["--relay", dead], key, [INIT, list_vaults], state=fresh)
        check("a dead seed relay still connects through the cached one",
              len(out) == 2 and all("result" in m for m in out), out)

        print("\n=== a base_version merge over the relay, against a LAN edit ===")
        status, created = rest(base, "POST", f"/v1/vaults/{vault_id}/notes", session,
                               {"path": "Relay/Merge.md",
                                "content": "# Merge\n\nline one\n\nline two\n\nline three\n"})
        check("the note is created on the LAN", status in (200, 201), created)
        note = created.get("note", {}) if isinstance(created, dict) else {}
        base_version = note.get("version")
        status, lan = rest(base, "PUT", f"/v1/vaults/{vault_id}/notes/{note.get('id')}", session,
                           {"base_version": base_version,
                            "content": "# Merge\n\nline one, edited on the LAN\n\nline two\n\nline three\n"})
        check("the LAN edit lands", status == 200, lan)
        rest(base, "PUT", "/v1/config/mcp", session, {"enabled": True, "writable": True})
        code, out, err = connect(["--addr", unreachable, "--relay", other] + pin, key, [
            INIT,
            {"jsonrpc": "2.0", "id": 9, "method": "tools/call",
             "params": {"name": "update_note", "arguments": {
                 "vault": vault_id, "note_id": note.get("id"), "base_version": base_version,
                 "content": "# Merge\n\nline one\n\nline two\n\nline three, edited by the agent\n"}}},
        ])
        by_id = {m.get("id"): m for m in out}
        sc = by_id.get(9, {}).get("result", {}).get("structuredContent", {})
        check("the agent's stale write is merged, not refused or clobbered",
              sc.get("merged") is True, by_id.get(9))
        merged = sc.get("content", "")
        check("the merge keeps both edits",
              "edited on the LAN" in merged and "edited by the agent" in merged, merged)
        status, now = rest(base, "GET", f"/v1/vaults/{vault_id}/notes/{note.get('id')}", session)
        check("the LAN sees the merged note",
              isinstance(now, dict) and "edited by the agent" in now.get("content", "")
              and "edited on the LAN" in now.get("content", ""), now)
        rest(base, "PUT", "/v1/config/mcp", session, {"enabled": True, "writable": False})

        print("\n=== impostors ===")
        code, out, err = connect(pin + ["--addr", impostor, "--relay", other], key, [INIT, list_vaults])
        vaults = (out[1] if len(out) == 2 else {}).get("result", {}).get(
            "structuredContent", {}).get("vaults", [])
        check("an impostor at the address loses to the relay, which proves the server",
              vault_id in [v["id"] for v in vaults] and f"relay {other}" in err, out)
        code, out, err = connect(pin + ["--addr", impostor], key, [INIT])
        check("an impostor alone is identity_failed, never offline",
              len(out) == 1 and storm_error(out[0]) == "identity_failed", out)
        code, out, err = connect(pin + ["--addr", impostor, "--relay", dead],
                                 key, [INIT])
        check("an impostor and a dead relay is still identity_failed",
              len(out) == 1 and storm_error(out[0]) == "identity_failed", out)

        print("\n=== a relay without the server, and no relay at all ===")
        with urllib.request.urlopen(f"{impostor}/v1/server") as r:
            unregistered = json.loads(r.read())["server_id"]
        code, out, err = connect(["--server", unregistered, "--pk", pk, "--relay", other],
                                 key, [INIT])
        check("a relay without the server is server_offline",
              len(out) == 1 and storm_error(out[0]) == "server_offline", out)
        # SRP §8.10 rule 3 (116i): initialize (handled in order) races once;
        # the calls after it are answered from the backoff.
        code, out, err = connect(["--server", unregistered, "--pk", pk, "--relay", other],
                                 key, [INIT] + [{**list_vaults, "id": n} for n in (1, 2, 3)])
        check("messages after a failed race fail from the backoff, not a new race",
              len(out) == 4 and all(storm_error(m) == "server_offline" for m in out)
              and [("not retrying" in m["error"]["message"]) for m in out]
              == [False, True, True, True], out)
        code, out, err = connect(pin + ["--relay", dead], key, [INIT])
        check("no relay is relay_down", len(out) == 1 and storm_error(out[0]) == "relay_down", out)
    finally:
        for p in procs:
            p.terminate()
        for p in procs:
            p.wait(timeout=10)

    print(f"\n{ok} passed, {fail} failed")
    sys.exit(1 if fail else 0)


if __name__ == "__main__":
    main()
