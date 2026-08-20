#!/usr/bin/env python3
"""End-to-end exercise of the Storm server API — M1's exit criterion.

Drives scenarios 3-7 of the sync matrix in PLAN.md against a live server,
checking behaviour the Rust unit tests can't: real HTTP, real files on disk,
and the file watcher reacting to an external edit.

Usage:

    cargo run -- --vault-root /tmp/vaults --state /tmp/s &
    VAULT_ROOT=/tmp/vaults python3 server/tests/e2e.py

The vault under test is the first one the server reports, so this runs against
whatever the harness seeded. Exits non-zero if any check fails.
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
# **No shared token.** The cutover removed it, so this suite now signs in the
# way a real client does — pair a device from the boot nonce, create the first
# account, log in — via the one helper every suite shares.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import storm_auth  # noqa: E402

AUTH = None
VAULT_ROOT = os.path.expanduser(os.environ["VAULT_ROOT"])

ok = 0
fail = 0


def call(method, path, body=None):
    url = f"{BASE}{path}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", AUTH)
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


# Sign in before anything else. Every check below needs a real session now;
# there is no constant that opens the server.
try:
    AUTH, _DEVICE, _USER = storm_auth.sign_in(BASE)
except RuntimeError as e:
    print(f"could not authenticate: {e}")
    sys.exit(1)

# Every note-shaped route is scoped to a vault. Resolved once, from the
# server, so the script does not have to know how the harness seeded it.
_, _vaults = call("GET", "/v1/vaults")
if not _vaults["vaults"]:
    print("no vaults registered — the harness should have seeded one")
    sys.exit(1)
VAULT_ID = _vaults["vaults"][0]["id"]
VAULT = os.path.join(VAULT_ROOT, _vaults["vaults"][0]["dir"])


def vp(path):
    """Vault-scoped API path."""
    return f"/v1/vaults/{VAULT_ID}{path}"


def enc(path):
    """Percent-encodes a vault path for a URL, keeping its separators."""
    return urllib.parse.quote(path, safe="/")


def note_id(path):
    _, tree = call("GET", vp("/tree"))
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
# Everything under E2E/, not just the fixture path: a run that died partway
# leaves notes behind, and the next run would then fail on debris rather than
# on anything real.
_, existing = call("GET", vp("/tree"))
for stale in [n for n in existing["notes"] if n["path"].startswith("E2E/")]:
    call("DELETE", vp(f"/notes/{stale['id']}"))
for folder in [f for f in existing.get("folders", []) if f.startswith("E2E")]:
    call("DELETE", vp("/folders/" + enc(folder)))
st, created = call("POST", vp("/notes"), {"path": FIXTURE_PATH, "content": FIXTURE_BODY})
check("fixture note created", st == 200, created)
nid = created["note"]["id"]

print("\n=== scenario 3: fast-forward write (client is up to date) ===")
_, note = call("GET", vp(f"/notes/{nid}"))
base_content, base_version = note["content"], note["version"]

ff = base_content.replace("Gamma line.", "Gamma line. FAST-FORWARD.")
st, r = call("PUT", vp(f"/notes/{nid}"), {"base_version": base_version, "content": ff})
check("accepted", st == 200, r)
check("version bumped", r["note"]["version"] == base_version + 1)
check("not flagged as merged", r["merged"] is False)
check("edit present", "FAST-FORWARD" in r["content"])

print("\n=== scenario 4: stale write, non-overlapping edits -> clean merge ===")
_, note = call("GET", vp(f"/notes/{nid}"))
shared_base, shared_ver = note["content"], note["version"]

# Device A (online) edits near the top and lands first.
a = shared_base.replace("Alpha line, near the top.", "Alpha line, edited on desktop.")
st, ra = call("PUT", vp(f"/notes/{nid}"), {"base_version": shared_ver, "content": a})
check("device A accepted", st == 200, ra)

# Device B was offline; it still holds the older base and edits a far region.
b = shared_base.replace("Omega line, near the bottom.", "Omega line, edited on phone.")
st, rb = call("PUT", vp(f"/notes/{nid}"), {"base_version": shared_ver, "content": b})
check("device B accepted", st == 200, rb)
check("flagged merged", rb["merged"] is True)
check("no conflict", rb["conflict"] is False, rb["content"])
check("desktop edit survived", "edited on desktop" in rb["content"], rb["content"])
check("phone edit survived", "edited on phone" in rb["content"], rb["content"])
check("no markers", "<<<<<<<" not in rb["content"])

print("\n=== scenario 5: stale write, same line -> conflict, nothing lost ===")
_, note = call("GET", vp(f"/notes/{nid}"))
cbase, cver = note["content"], note["version"]

a = cbase.replace("Beta line, in the middle.", "Beta line, DESKTOP version.")
st, ra = call("PUT", vp(f"/notes/{nid}"), {"base_version": cver, "content": a})
pre_merge_version = ra["note"]["version"]

b = cbase.replace("Beta line, in the middle.", "Beta line, PHONE version.")
st, rb = call("PUT", vp(f"/notes/{nid}"), {"base_version": cver, "content": b})
check("still accepted (never rejected)", st == 200, rb)
check("flagged conflict", rb["conflict"] is True)
check("markers present", "<<<<<<<" in rb["content"])
check("desktop text kept", "DESKTOP version." in rb["content"])
check("phone text kept", "PHONE version." in rb["content"])
check("client's own edit is the `ours` side",
      rb["content"].index("PHONE version.") < rb["content"].index("======="))

print("\n=== scenario 6: rename + edit survive together (the UUID payoff) ===")
st, r = call("POST", vp(f"/notes/{nid}/move"), {"new_path": "E2E/Archive/Merge Fixture.md"})
check("moved", st == 200, r)
check("path updated", r["note"]["path"] == "E2E/Archive/Merge Fixture.md")
check("id unchanged", r["note"]["id"] == nid)
check("file moved on disk", os.path.exists(f"{VAULT}/E2E/Archive/Merge Fixture.md"))
check("old path gone", not os.path.exists(f"{VAULT}/{FIXTURE_PATH}"))

_, note = call("GET", vp(f"/notes/{nid}"))
st, r = call(
    "PUT",
    vp(f"/notes/{nid}"),
    {"base_version": note["version"], "content": note["content"] + "\nAfter move.\n"},
)
check("edit after move still resolves by id", st == 200 and "After move." in r["content"])

print("\n=== delta sync via change_log ===")
_, s = call("GET", vp("/sync?since=0"))
check("changes returned", len(s["changes"]) > 0)
mid = s["changes"][len(s["changes"]) // 2]["seq"]
_, s2 = call("GET", vp(f"/sync?since={mid}"))
check("since= filters correctly", all(c["seq"] > mid for c in s2["changes"]))
check("seq is the latest", s2["seq"] == s["seq"])

print("\n=== create + delete ===")
st, r = call("POST", vp("/notes"), {"path": "Temp/Scratch.md", "content": "# Scratch\n\n#temp\n"})
check("created", st == 200, r)
new_id = r["note"]["id"]
check("file on disk", os.path.exists(f"{VAULT}/Temp/Scratch.md"))
check("id in frontmatter", "id:" in open(f"{VAULT}/Temp/Scratch.md").read())

st, _ = call("POST", vp("/notes"), {"path": "Temp/Scratch.md", "content": "dupe"})
check("refuses to clobber an existing path", st == 400)

st, _ = call("DELETE", vp(f"/notes/{new_id}"))
check("deleted", st == 200)
check("file gone", not os.path.exists(f"{VAULT}/Temp/Scratch.md"))
check("empty folder pruned", not os.path.exists(f"{VAULT}/Temp"))
st, _ = call("GET", vp(f"/notes/{new_id}"))
check("404 after delete", st == 404)

print("\n=== scenario 7: external edit picked up by the watcher ===")
# Its own note again, so this doesn't depend on the vault's contents.
watch_path = "E2E/Watched.md"
st, watched = call("POST", vp("/notes"), {"path": watch_path, "content": "# Watched\n"})
check("watch fixture created", st == 200, watched)
wid = watched["note"]["id"]
target = f"{VAULT}/{watch_path}"
_, before = call("GET", vp(f"/notes/{wid}"))

with open(target, "a") as f:
    f.write("\nEdited directly with an external tool. #external\n")

deadline = time.time() + 15
seen = False
while time.time() < deadline:
    time.sleep(0.5)
    _, after = call("GET", vp(f"/notes/{wid}"))
    if after["version"] > before["version"]:
        seen = True
        break
check("watcher noticed the external edit", seen)
if seen:
    check("content reindexed", "external tool" in after["content"])
    _, tags = call("GET", vp("/tags"))
    check("new tag indexed", any(t["tag"] == "external" for t in tags["tags"]))

print("\n=== folders are real, not just derived from note paths ===")
st, r = call("POST", vp("/folders"), {"path": "E2E/Empty Folder"})
check("folder created", st == 200, r)
check("directory on disk", os.path.isdir(f"{VAULT}/E2E/Empty Folder"))
_, tree = call("GET", vp("/tree"))
check("an empty folder appears in the tree", "E2E/Empty Folder" in tree["folders"])

# The regression the `folders` table exists for: a folder created on purpose
# must survive the delete that empties it.
st, kept = call("POST", vp("/notes"), {"path": "E2E/Empty Folder/Temp.md", "content": "# T\n"})
check("note inside the folder", st == 200, kept)
call("DELETE", vp(f"/notes/{kept['note']['id']}"))
check("recorded folder survives being emptied",
      os.path.isdir(f"{VAULT}/E2E/Empty Folder"))

st, r = call("POST", vp("/notes"), {"path": "E2E/Derived/Only.md", "content": "# O\n"})
call("DELETE", vp(f"/notes/{r['note']['id']}"))
check("a merely-derived folder is still pruned",
      not os.path.exists(f"{VAULT}/E2E/Derived"))

st, r = call("POST", vp("/notes"), {"path": "E2E/Empty Folder/Held.md", "content": "# H\n"})
held = r["note"]["id"]
st, r = call("DELETE", vp("/folders/" + enc("E2E/Empty Folder")))
check("deleting a folder with notes is refused", st == 409, r)
check("its notes are untouched", os.path.exists(f"{VAULT}/E2E/Empty Folder/Held.md"))

st, r = call("POST", vp("/folders/rename"), {"from": "E2E/Empty Folder", "to": "E2E/Renamed"})
check("folder renamed", st == 200, r)
check("one `moved` change per contained note", r["moved"] == 1, r)
_, moved = call("GET", vp(f"/notes/{held}"))
check("contained note's path rewritten", moved["path"] == "E2E/Renamed/Held.md", moved)
check("note kept its id", moved["id"] == held)
check("directory moved on disk", os.path.isdir(f"{VAULT}/E2E/Renamed"))

call("DELETE", vp(f"/notes/{held}"))
st, _ = call("DELETE", vp("/folders/" + enc("E2E/Renamed")))
check("an empty folder can be deleted", st == 200)
check("gone from disk", not os.path.exists(f"{VAULT}/E2E/Renamed"))

print("\n=== recents are recorded server-side and cross-vault ===")
st, r = call("POST", vp("/notes"), {"path": "E2E/Opened.md", "content": "# Opened\n"})
opened_id = r["note"]["id"]
_, before = call("GET", vp(f"/notes/{opened_id}"))

st, _ = call("POST", vp(f"/notes/{opened_id}/opened"))
check("open recorded", st == 200)

_, after = call("GET", vp(f"/notes/{opened_id}"))
check("opening is not an edit", after["version"] == before["version"], after)

_, rec = call("GET", "/v1/recents?limit=10")
mine = [x for x in rec["recents"] if x["note_id"] == opened_id]
check("appears in recents", len(mine) == 1, rec)
if mine:
    check("carries the vault it came from", mine[0]["vault_id"] == VAULT_ID)
    check("carries the vault's name", bool(mine[0]["vault_name"]))

st, _ = call("POST", vp("/notes/does-not-exist/opened"))
check("opening an unknown note is a 404", st == 404)
call("DELETE", vp(f"/notes/{opened_id}"))

print("\n=== a second vault is genuinely separate ===")
st, second = call("POST", "/v1/vaults", {"name": "E2E Second"})
check("vault created", st == 200, second)
second_id = second["id"]
check("directory created under the root",
      os.path.isdir(os.path.join(VAULT_ROOT, second["dir"])))

# The collision one shared index could not have represented.
shared = "Daily/2026-08-07.md"
st, a = call("POST", vp("/notes"), {"path": f"E2E/{shared}", "content": "# first\n"})
st2, b = call("POST", f"/v1/vaults/{second_id}/notes",
              {"path": shared, "content": "# second\n"})
check("the same note path exists in both vaults", st == 200 and st2 == 200, (a, b))
check("with different ids", a["note"]["id"] != b["note"]["id"])

_, one = call("GET", vp("/tree"))
_, two = call("GET", f"/v1/vaults/{second_id}/tree")
check("each tree holds only its own notes",
      not any(n["id"] == b["note"]["id"] for n in one["notes"]))
check("seq counters are independent", two["seq"] < one["seq"], (one["seq"], two["seq"]))

st, r = call("PATCH", f"/v1/vaults/{second_id}", {"name": "E2E Renamed Vault"})
check("vault renamed", st == 200 and r["name"] == "E2E Renamed Vault", r)

st, _ = call("GET", "/v1/vaults/nope/tree")
check("an unknown vault is a 404", st == 404)

call("DELETE", vp(f"/notes/{a['note']['id']}"))
call("DELETE", f"/v1/vaults/{second_id}/notes/{b['note']['id']}")

st, removed = call("DELETE", f"/v1/vaults/{second_id}")
check("vault unregistered", st == 200, removed)
check("but its directory is left on disk",
      os.path.isdir(os.path.join(VAULT_ROOT, second["dir"])), removed)
os.rmdir(os.path.join(VAULT_ROOT, second["dir"]))

print("\n=== the storage root cannot be moved somewhere that loses vaults ===")
_, cfg = call("GET", "/v1/config")
check("config reports the root", cfg["vault_root"].endswith(os.path.basename(VAULT_ROOT)), cfg)
original_root = cfg["vault_root"]

st, r = call("PUT", "/v1/config", {"vault_root": "relative/path"})
check("a relative root is refused", st == 400, r)
st, r = call("PUT", "/v1/config", {"vault_root": "/definitely/not/here"})
check("a missing root is refused", st == 400, r)

empty_root = os.path.join(os.path.dirname(VAULT_ROOT.rstrip("/")), "e2e-empty-root")
os.makedirs(empty_root, exist_ok=True)
st, r = call("PUT", "/v1/config", {"vault_root": empty_root})
check("a root that would orphan every vault is refused", st == 409, r)
check("the refusal explains itself", "orphan_ok" in r.get("error", ""), r)

_, cfg = call("GET", "/v1/config")
check("the root did not move", cfg["vault_root"] == original_root, cfg)
_, still = call("GET", vp("/tree"))
check("the vault still serves its notes", "notes" in still)
os.rmdir(empty_root)

print("\n=== cleanup ===")
_, tree = call("GET", vp("/tree"))
removed = 0
for n in tree["notes"]:
    if n["path"].startswith("E2E/"):
        call("DELETE", vp(f"/notes/{n['id']}"))
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
