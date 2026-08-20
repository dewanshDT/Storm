#!/usr/bin/env python3
"""End-to-end exercise of the MCP endpoint — M13's exit criterion.

Companion to `e2e.py`, same style and no dependencies: this drives `/mcp` as a
real client would, over HTTP, against a live server started with `--mcp`.

Usage:

    cargo run -- --vault-root /tmp/vaults --state /tmp/s --mcp &
    VAULT_ROOT=/tmp/vaults python3 server/tests/mcp_e2e.py

What it is really for, beyond "the tools answer":

  * the endpoint is behind a credential, which depends on `/mcp` being
    nested *above* the auth layer in `api.rs` — an ordering nothing else would
    catch;
  * `structuredContent` is a JSON object everywhere, which the spec requires
    and `rmcp` does not enforce;
  * a bad vault id comes back as a tool error the model can act on, not a
    protocol error clients may never show it;
  * no absolute filesystem path is ever in a payload.

Exits non-zero if any check fails.
"""
import json
import os
import sys
import urllib.error
import urllib.request

BASE = os.environ.get("STORM_BASE", "http://127.0.0.1:8484")
# **Two different credentials, deliberately.** REST fixtures go through a
# *session*; `/mcp` goes through an `stk_` **MCP key**, which is what a real
# MCP client holds and what this suite should therefore be proving. A key is
# refused on REST by design (A14.2), so one credential could not do both.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import storm_auth  # noqa: E402

SESSION = None
KEY = None
VAULT_ROOT = os.path.expanduser(os.environ["VAULT_ROOT"])

# Every tool this server is expected to expose. Asserted exactly, so adding one
# without deciding to is a failure rather than a surprise in production.
EXPECTED_TOOLS = {
    "get_note",
    "get_note_history",
    "get_note_version",
    "get_related_notes",
    "get_vault",
    "list_tags",
    "list_vaults",
    "recent_notes",
    "search",
}

# Only served when the server is in read-write mode.
WRITE_TOOLS = {"create_note", "update_note", "delete_note"}

ok = 0
fail = 0
_id = 0

# Every payload seen, so the absolute-path check covers all of them at the end
# rather than needing a line per tool.
seen_payloads = []


def check(label, cond, detail=""):
    global ok, fail
    if cond:
        ok += 1
        print(f"  PASS  {label}")
    else:
        fail += 1
        print(f"  FAIL  {label}   {detail}")


def rest(method, path, body=None):
    """The REST API, for setting up fixtures MCP can only read."""
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method)
    req.add_header("Authorization", SESSION)
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        # Not every rejection is JSON — axum answers a malformed body with
        # plain text, and a traceback here would hide which fixture broke.
        raw = e.read()
        try:
            return e.code, json.loads(raw or b"{}")
        except json.JSONDecodeError:
            return e.code, {"error": raw.decode(errors="replace")}


def rpc(method, params=None, token=None, bearer=True):
    """One JSON-RPC call to /mcp. Returns (status, body)."""
    global _id
    _id += 1
    payload = {"jsonrpc": "2.0", "id": _id, "method": method}
    if params is not None:
        payload["params"] = params

    req = urllib.request.Request(
        f"{BASE}/mcp", data=json.dumps(payload).encode(), method="POST"
    )
    presented = KEY if (token is None and bearer) else token
    if presented:
        req.add_header(
            "Authorization",
            presented if presented.startswith("Bearer ") else f"Bearer {presented}",
        )
    req.add_header("Content-Type", "application/json")
    # Streamable HTTP requires the client to accept both, even when the server
    # is configured to answer in plain JSON.
    req.add_header("Accept", "application/json, text/event-stream")
    try:
        with urllib.request.urlopen(req) as r:
            body = json.loads(r.read())
            seen_payloads.append(body)
            return r.status, body
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def structured(result):
    """The tool's structured content, as a dict.

    Returns `{}` when it is anything else, so an out-of-spec payload fails the
    assertion that checks for it rather than crashing every check after it on
    `list.get`. The shape itself is asserted explicitly where it matters.
    """
    value = (result or {}).get("structuredContent")
    return value if isinstance(value, dict) else {}


def call(tool, args=None):
    """A tools/call, returning the `result` object."""
    status, body = rpc("tools/call", {"name": tool, "arguments": args or {}})
    if status != 200 or "result" not in body:
        return None, body
    return body["result"], body


# Authenticate before the handshake — the very first JSON-RPC call needs a
# credential now, where it used to need a constant.
try:
    SESSION, _DEVICE, _USER = storm_auth.sign_in(BASE)
    KEY = storm_auth.mint_mcp_key(BASE, SESSION)
except RuntimeError as e:
    print(f"could not authenticate: {e}")
    sys.exit(1)

print("== handshake ==")
status, body = rpc(
    "initialize",
    {
        "protocolVersion": "2025-11-25",
        "capabilities": {},
        "clientInfo": {"name": "storm-e2e", "version": "1"},
    },
)
check("initialize succeeds", status == 200 and "result" in body, body)
info = body.get("result", {})
check(
    "identifies itself as storm",
    info.get("serverInfo", {}).get("name") == "storm",
    info.get("serverInfo"),
)
check("declares the tools capability", "tools" in info.get("capabilities", {}), info)
check(
    "sends instructions for the model",
    bool(info.get("instructions")),
    "an agent with no instructions has to guess how vaults and ids relate",
)

print("== the tool surface ==")
status, body = rpc("tools/list")
tools = body.get("result", {}).get("tools", [])
names = {t["name"] for t in tools}
check("exactly the expected tools", names == EXPECTED_TOOLS, sorted(names ^ EXPECTED_TOOLS))
check(
    "every tool has an object input schema",
    all(t.get("inputSchema", {}).get("type") == "object" for t in tools),
    [t["name"] for t in tools if t.get("inputSchema", {}).get("type") != "object"],
)
check(
    "every tool describes itself",
    all(t.get("description") for t in tools),
    [t["name"] for t in tools if not t.get("description")],
)

print("== fixtures ==")
_, vaults_body = rest("GET", "/v1/vaults")
vault = vaults_body["vaults"][0]["id"]
# A description to read back, written the way a client would — through the API,
# into the config note the design already uses.
rest(
    "POST",
    f"/v1/vaults/{vault}/notes",
    {"path": "_storm/vault.md", "content": "---\nstorm.description: E2E vault\n---\n"},
)
status, created = rest(
    "POST",
    f"/v1/vaults/{vault}/notes",
    {
        "path": "MCP/Tornado.md",
        "content": "---\ntags:\n  - weather\n---\n\n# Tornado\n\nA rotating column of air.\n",
    },
)
# `WriteResult` wraps the row: {"note": {...}, "content": ...}.
note = created.get("note", {})
note_id = note.get("id")
# A second version, so history has something to list.
rest(
    "PUT",
    f"/v1/vaults/{vault}/notes/{note_id}",
    {
        "content": "---\ntags:\n  - weather\n---\n\n# Tornado\n\nA violently rotating column of air.\n",
        "base_version": note.get("version"),
    },
)
rest("POST", f"/v1/vaults/{vault}/notes/{note_id}/opened")
check("fixture note created", bool(note_id), created)
check("fixture note updated to v2", bool(note.get("version")), created)

print("== reading ==")
result, raw = call("list_vaults")
raw_sc = (result or {}).get("structuredContent")
check("list_vaults returns an object", isinstance(raw_sc, dict), raw_sc)
sc = structured(result)
check("list_vaults lists the vault", any(v["id"] == vault for v in sc.get("vaults", [])), sc)

result, raw = call("get_vault", {"vault": vault})
sc = structured(result)
check("get_vault reads the description from _storm/vault.md", sc.get("description") == "E2E vault", sc)
check("get_vault lists folders", isinstance(sc.get("folders"), list), sc)

result, raw = call("search", {"vault": vault, "query": "rotating"})
sc = structured(result)
hits = sc.get("hits", [])
check("search finds the note", any(h["id"] == note_id for h in hits), sc)
check("search returns a snippet", all("snippet" in h for h in hits), hits)

# Punctuation that is FTS5 syntax — the sanitiser is shared with REST, and this
# proves MCP gets it rather than having its own.
result, _ = call("search", {"vault": vault, "query": "rotating-column"})
check(
    "punctuation does not blow up the query",
    (result or {}).get("isError") is not True,
    result,
)

result, raw = call("get_note", {"vault": vault, "note_id": note_id})
sc = structured(result)
check("get_note returns the content", "violently rotating" in sc.get("content", ""), sc)
check("get_note returns the vault-relative path", sc.get("path") == "MCP/Tornado.md", sc)

result, raw = call("get_related_notes", {"vault": vault, "note_id": note_id})
sc = structured(result)
check("get_related_notes answers with both signals", "backlinks" in sc and "shared_tags" in sc, sc)

result, raw = call("list_tags", {"vault": vault})
sc = structured(result)
check("list_tags includes the fixture's tag", any(t["tag"] == "weather" for t in sc.get("tags", [])), sc)

result, raw = call("recent_notes")
sc = structured(result)
check("recent_notes includes the opened note", any(r["note_id"] == note_id for r in sc.get("recents", [])), sc)
check("recent_notes names the vault it came from", all("vault_name" in r for r in sc.get("recents", [])), sc)

print("== history ==")
result, raw = call("get_note_history", {"vault": vault, "note_id": note_id})
sc = structured(result)
versions = sc.get("versions", [])
check("history lists both versions", len(versions) >= 2, sc)
check("history is newest first", [v["version"] for v in versions] == sorted((v["version"] for v in versions), reverse=True), versions)
check(
    "history carries no content",
    all("content" not in v for v in versions),
    "a full snapshot per version would make a history of a long note megabytes",
)

result, raw = call("get_note_version", {"vault": vault, "note_id": note_id, "version": 1})
sc = structured(result)
check("an old version is readable", "A rotating column" in json.dumps(sc), sc)

print("== errors and auth ==")
result, raw = call("get_note", {"vault": "no-such-vault", "note_id": note_id})
check(
    "an unknown vault is a tool error, not a protocol error",
    raw.get("error") is None and (result or {}).get("isError") is True,
    raw,
)
check(
    "the tool error says what was wrong",
    "vault" in json.dumps(structured(result)).lower(),
    result,
)

result, raw = call("get_note", {"vault": vault, "note_id": "00000000-0000-0000-0000-000000000000"})
check("an unknown note is a tool error too", (result or {}).get("isError") is True, raw)

status, _ = rpc("tools/list", bearer=False)
check("no credential is refused", status == 401, status)
status, _ = rpc("tools/list", token="stk_wrong")
check("a wrong key is refused", status == 401, status)
status, _ = rpc("tools/list", token="testtoken")
check("the retired shared token is refused", status == 401, status)

print("== the config note is not one of the user's notes ==")
# `_storm/vault.md` is Storm's own per-vault configuration, kept as ordinary
# markdown. The Flutter client filtered it at five separate call sites, so MCP —
# a new caller — saw it. The rule now lives in the query.
rest(
    "POST",
    f"/v1/vaults/{vault}/notes",
    {"path": "_storm/scratch.md", "content": "---\ntags:\n  - internal\n---\n\nrotating internals\n"},
)
result, _ = call("search", {"vault": vault, "query": "rotating"})
paths = [h["path"] for h in structured(result).get("hits", [])]
check("search does not return _storm/", not any(p.startswith("_storm/") for p in paths), paths)
check("search still returns the user's note", "MCP/Tornado.md" in paths, paths)

result, _ = call("list_tags", {"vault": vault})
tags = [t["tag"] for t in structured(result).get("tags", [])]
check("tags from _storm/ are not listed", "internal" not in tags, tags)

result, _ = call("recent_notes")
rpaths = [r["path"] for r in structured(result).get("recents", [])]
check("recents does not return _storm/", not any(p.startswith("_storm/") for p in rpaths), rpaths)

print("== the endpoint can be switched off and on ==")
status, _ = rest("PUT", "/v1/config/mcp", {"enabled": False})
check("toggling off succeeds", status == 200, status)
status, body = rpc("tools/list")
check("mcp refuses while off", status == 404, status)
_, cfg = rest("GET", "/v1/config")
check("config reports it off", cfg.get("mcp_enabled") is False, cfg)

status, _ = rest("PUT", "/v1/config/mcp", {"enabled": True})
status, body = rpc("tools/list")
check("mcp answers again once on", status == 200 and "result" in body, status)
_, cfg = rest("GET", "/v1/config")
check("config reports it on", cfg.get("mcp_enabled") is True, cfg)

print("== read-only mode hides the write tools ==")
rest("PUT", "/v1/config/mcp", {"enabled": True, "writable": False})
_, body = rpc("tools/list")
names = {t["name"] for t in body["result"]["tools"]}
check("no write tools are advertised", names == EXPECTED_TOOLS, sorted(names - EXPECTED_TOOLS))
result, raw = call("delete_note", {"vault": vault, "note_id": note_id})
check(
    "and calling one is refused",
    raw.get("error") is not None or (result or {}).get("isError") is True,
    raw,
)
_, cfg = rest("GET", "/v1/config")
check("config reports read-only", cfg.get("mcp_writable") is False, cfg)

print("== read-write mode ==")
rest("PUT", "/v1/config/mcp", {"enabled": True, "writable": True})
_, body = rpc("tools/list")
names = {t["name"] for t in body["result"]["tools"]}
check("write tools appear", names == EXPECTED_TOOLS | WRITE_TOOLS, sorted(names ^ (EXPECTED_TOOLS | WRITE_TOOLS)))

result, raw = call(
    "create_note",
    {"vault": vault, "path": "MCP/Written.md", "content": "# Written\n\nBy an agent.\n"},
)
# `WriteResult` wraps the row: {"note": {...}, "content", "seq", "merged"}.
sc = structured(result)
row = sc.get("note", {})
check("create_note creates it", row.get("path") == "MCP/Written.md", sc)
written_id = row.get("id")
check("and stamps source: ai in the frontmatter", "source: ai" in sc.get("content", ""), sc.get("content"))

# The stamp must be a spliced line, not a re-serialised document: the body has
# to survive byte for byte.
check("the body is untouched", "By an agent." in sc.get("content", ""), sc.get("content"))

result, _ = call(
    "update_note",
    {
        "vault": vault,
        "note_id": written_id,
        "content": "# Written\n\nEdited by an agent.\n",
        "base_version": row.get("version"),
    },
)
sc2 = structured(result)
check(
    "update_note bumps the version",
    sc2.get("note", {}).get("version", 0) > row.get("version", 0),
    sc2,
)
result, _ = call("get_note", {"vault": vault, "note_id": written_id})
check("and the edit is readable", "Edited by an agent." in structured(result).get("content", ""), structured(result))

# A stale base_version must merge rather than clobber — the same contract the
# Flutter client obeys.
result, _ = call(
    "update_note",
    {
        "vault": vault,
        "note_id": written_id,
        "content": "# Written\n\nA third edit from a stale read.\n",
        "base_version": 1,
    },
)
sc3 = structured(result)
check(
    "a stale base_version is merged, not silently accepted",
    sc3.get("merged") is True or sc3.get("conflict") is True or "error" in sc3,
    sc3,
)

result, _ = call("delete_note", {"vault": vault, "note_id": written_id})
check("delete_note removes it", "seq" in structured(result), structured(result))
# Guarded, because an id of None would make this pass for the wrong reason.
check("the note existed to delete", bool(written_id), "create_note returned no id")
result, _ = call("get_note", {"vault": vault, "note_id": written_id})
check("and it is gone", (result or {}).get("isError") is True, result)

print("== writes cannot outlive the endpoint ==")
rest("PUT", "/v1/config/mcp", {"enabled": False, "writable": True})
_, cfg = rest("GET", "/v1/config")
check(
    "switching MCP off also disarms writes",
    cfg.get("mcp_writable") is False,
    'otherwise turning MCP back on would silently restore write access',
)
rest("PUT", "/v1/config/mcp", {"enabled": True, "writable": True})

print("== nothing leaks a filesystem path ==")
blob = json.dumps(seen_payloads)
check(
    "no absolute vault path in any payload",
    VAULT_ROOT not in blob,
    f"{VAULT_ROOT} appeared in a response",
)
check(
    "every structuredContent is an object",
    all(
        isinstance(p.get("result", {}).get("structuredContent"), (dict, type(None)))
        for p in seen_payloads
    ),
    "the spec requires an object; rmcp will happily send an array",
)

print(f"\n{ok} passed, {fail} failed")
sys.exit(1 if fail else 0)
