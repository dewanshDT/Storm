#!/usr/bin/env python3
"""End-to-end exercise of the Storm server API — M1's exit criterion.

Drives scenarios 3-7 of the sync matrix in PLAN.md against a live server,
checking behaviour the Rust unit tests can't: real HTTP, real files on disk,
and the file watcher reacting to an external edit.

Usage:

    cargo run -- --vault /tmp/v --state /tmp/s --token testtoken --port 8484 &
    VAULT=/tmp/v python3 server/tests/e2e.py

Exits non-zero if any check fails.
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

BASE = "http://127.0.0.1:8484"
TOKEN = "testtoken"
VAULT = os.path.expanduser(os.environ["VAULT"])

ok = 0
fail = 0


def call(method, path, body=None):
    url = f"{BASE}{path}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bearer {TOKEN}")
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def check(label, cond, detail=""):
    global ok, fail
    if cond:
        ok += 1
        print(f"  PASS  {label}")
    else:
        fail += 1
        print(f"  FAIL  {label}   {detail}")


def note_id(path):
    _, tree = call("GET", "/v1/tree")
    return next(n["id"] for n in tree["notes"] if n["path"] == path)


# The merge scenarios need a note whose exact text is known. Creating it here
# rather than reusing whatever is in the vault keeps this script runnable
# against any vault, including an empty one.
FIXTURE_PATH = "E2E/Merge Fixture.md"
FIXTURE_BODY = (
    "# Merge Fixture\n\n"
    "Alpha line, near the top.\n\n"
    "Beta line, in the middle.\n\n"
    "Gamma line.\n\n"
    "Delta line.\n\n"
    "Omega line, near the bottom.\n"
)

print("\n=== fixture ===")
_, existing = call("GET", "/v1/tree")
for stale in [n for n in existing["notes"] if n["path"] == FIXTURE_PATH]:
    call("DELETE", f"/v1/notes/{stale['id']}")
st, created = call("POST", "/v1/notes", {"path": FIXTURE_PATH, "content": FIXTURE_BODY})
check("fixture note created", st == 200, created)
nid = created["note"]["id"]

print("\n=== scenario 3: fast-forward write (client is up to date) ===")
_, note = call("GET", f"/v1/notes/{nid}")
base_content, base_version = note["content"], note["version"]

ff = base_content.replace("Gamma line.", "Gamma line. FAST-FORWARD.")
st, r = call("PUT", f"/v1/notes/{nid}", {"base_version": base_version, "content": ff})
check("accepted", st == 200, r)
check("version bumped", r["note"]["version"] == base_version + 1)
check("not flagged as merged", r["merged"] is False)
check("edit present", "FAST-FORWARD" in r["content"])

print("\n=== scenario 4: stale write, non-overlapping edits -> clean merge ===")
_, note = call("GET", f"/v1/notes/{nid}")
shared_base, shared_ver = note["content"], note["version"]

# Device A (online) edits near the top and lands first.
a = shared_base.replace("Alpha line, near the top.", "Alpha line, edited on desktop.")
st, ra = call("PUT", f"/v1/notes/{nid}", {"base_version": shared_ver, "content": a})
check("device A accepted", st == 200, ra)

# Device B was offline; it still holds the older base and edits a far region.
b = shared_base.replace("Omega line, near the bottom.", "Omega line, edited on phone.")
st, rb = call("PUT", f"/v1/notes/{nid}", {"base_version": shared_ver, "content": b})
check("device B accepted", st == 200, rb)
check("flagged merged", rb["merged"] is True)
check("no conflict", rb["conflict"] is False, rb["content"])
check("desktop edit survived", "edited on desktop" in rb["content"], rb["content"])
check("phone edit survived", "edited on phone" in rb["content"], rb["content"])
check("no markers", "<<<<<<<" not in rb["content"])

print("\n=== scenario 5: stale write, same line -> conflict, nothing lost ===")
_, note = call("GET", f"/v1/notes/{nid}")
cbase, cver = note["content"], note["version"]

a = cbase.replace("Beta line, in the middle.", "Beta line, DESKTOP version.")
st, ra = call("PUT", f"/v1/notes/{nid}", {"base_version": cver, "content": a})
pre_merge_version = ra["note"]["version"]

b = cbase.replace("Beta line, in the middle.", "Beta line, PHONE version.")
st, rb = call("PUT", f"/v1/notes/{nid}", {"base_version": cver, "content": b})
check("still accepted (never rejected)", st == 200, rb)
check("flagged conflict", rb["conflict"] is True)
check("markers present", "<<<<<<<" in rb["content"])
check("desktop text kept", "DESKTOP version." in rb["content"])
check("phone text kept", "PHONE version." in rb["content"])
check("client's own edit is the `ours` side",
      rb["content"].index("PHONE version.") < rb["content"].index("======="))

print("\n=== scenario 6: rename + edit survive together (the UUID payoff) ===")
st, r = call("POST", f"/v1/notes/{nid}/move", {"new_path": "E2E/Archive/Merge Fixture.md"})
check("moved", st == 200, r)
check("path updated", r["note"]["path"] == "E2E/Archive/Merge Fixture.md")
check("id unchanged", r["note"]["id"] == nid)
check("file moved on disk", os.path.exists(f"{VAULT}/E2E/Archive/Merge Fixture.md"))
check("old path gone", not os.path.exists(f"{VAULT}/{FIXTURE_PATH}"))

_, note = call("GET", f"/v1/notes/{nid}")
st, r = call(
    "PUT",
    f"/v1/notes/{nid}",
    {"base_version": note["version"], "content": note["content"] + "\nAfter move.\n"},
)
check("edit after move still resolves by id", st == 200 and "After move." in r["content"])

print("\n=== delta sync via change_log ===")
_, s = call("GET", "/v1/sync?since=0")
check("changes returned", len(s["changes"]) > 0)
mid = s["changes"][len(s["changes"]) // 2]["seq"]
_, s2 = call("GET", f"/v1/sync?since={mid}")
check("since= filters correctly", all(c["seq"] > mid for c in s2["changes"]))
check("seq is the latest", s2["seq"] == s["seq"])

print("\n=== create + delete ===")
st, r = call("POST", "/v1/notes", {"path": "Temp/Scratch.md", "content": "# Scratch\n\n#temp\n"})
check("created", st == 200, r)
new_id = r["note"]["id"]
check("file on disk", os.path.exists(f"{VAULT}/Temp/Scratch.md"))
check("id in frontmatter", "id:" in open(f"{VAULT}/Temp/Scratch.md").read())

st, _ = call("POST", "/v1/notes", {"path": "Temp/Scratch.md", "content": "dupe"})
check("refuses to clobber an existing path", st == 400)

st, _ = call("DELETE", f"/v1/notes/{new_id}")
check("deleted", st == 200)
check("file gone", not os.path.exists(f"{VAULT}/Temp/Scratch.md"))
check("empty folder pruned", not os.path.exists(f"{VAULT}/Temp"))
st, _ = call("GET", f"/v1/notes/{new_id}")
check("404 after delete", st == 404)

print("\n=== scenario 7: external edit picked up by the watcher ===")
# Its own note again, so this doesn't depend on the vault's contents.
watch_path = "E2E/Watched.md"
st, watched = call("POST", "/v1/notes", {"path": watch_path, "content": "# Watched\n"})
check("watch fixture created", st == 200, watched)
wid = watched["note"]["id"]
target = f"{VAULT}/{watch_path}"
_, before = call("GET", f"/v1/notes/{wid}")

with open(target, "a") as f:
    f.write("\nEdited directly with an external tool. #external\n")

deadline = time.time() + 15
seen = False
while time.time() < deadline:
    time.sleep(0.5)
    _, after = call("GET", f"/v1/notes/{wid}")
    if after["version"] > before["version"]:
        seen = True
        break
check("watcher noticed the external edit", seen)
if seen:
    check("content reindexed", "external tool" in after["content"])
    _, tags = call("GET", "/v1/tags")
    check("new tag indexed", any(t["tag"] == "external" for t in tags["tags"]))

print("\n=== cleanup ===")
_, tree = call("GET", "/v1/tree")
removed = 0
for n in tree["notes"]:
    if n["path"].startswith("E2E/"):
        call("DELETE", f"/v1/notes/{n['id']}")
        removed += 1
check("removed the notes this run created", removed >= 2)
check("E2E folder pruned", not os.path.exists(f"{VAULT}/E2E"))

print("\n=== the vault is still just markdown ===")
stray = []
for root, dirs, files in os.walk(VAULT):
    dirs[:] = [d for d in dirs if not d.startswith(".")]
    for fn in files:
        if not fn.endswith(".md") and not fn.endswith(".txt"):
            stray.append(os.path.join(root, fn))
check("no stray files in the vault", not stray, stray)
check("state db lives outside the vault", not os.path.exists(f"{VAULT}/state"))

print(f"\n{'=' * 52}\n  {ok} passed, {fail} failed\n{'=' * 52}")
sys.exit(1 if fail else 0)
