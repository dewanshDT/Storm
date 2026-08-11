# storm-server

The Storm sync server: a single Rust binary that owns the canonical vault.

```
/srv/storm/
├── vaults/                 the storage root: one directory per vault
│   ├── personal/           canonical markdown — greppable, rsync-able,
│   │   ├── Daily/2026-08-07.md      Obsidian-readable
│   │   └── Projects/Ideas.md
│   └── work/
└── state/
    ├── vaults.json         registry: root + id/name/directory per vault
    └── <vault-id>/index.db derived index + version history, one per vault
```

`state/` is a **sibling** of `vault/`, never inside it, so the vault directory
contains nothing but your notes.

## Running it

Point it at a **copy** of your vaults first. The `--dry-run` pass writes
nothing and reports what an import would change, per vault:

```sh
cargo run -- dry-run --vault-root /srv/storm/vaults --state /srv/storm/state
```

```
Dry run — nothing was written.

  storage root : /srv/storm/vaults
  vaults       : 2

  personal
      markdown files found : 1284
      would add `id` to    : 1284
          Daily/2026-08-05.md
          ...

Run again without --dry-run to apply.
```

Then for real:

```sh
cargo run --release -- serve \
  --vault-root /srv/storm/vaults \
  --state /srv/storm/state \
  --host 0.0.0.0 --port 8484 \
  --token "$(openssl rand -hex 32)" \
  --web /usr/share/storm/web   # or a local Flutter build/web
```

Omit `--token` and one is generated and printed for that run. Set `STORM_TOKEN`
in the environment to keep it stable.

**v1 is LAN-only.** A single shared bearer token with no TLS is only defensible
inside the LAN. TLS and per-device token rotation must land *before* this is
ever reachable from outside it.

## Importing an existing Obsidian vault

There is no separate importer — the startup scan *is* the importer, and it runs
per vault. It walks the
vault, adds `id`/`created`/`modified` frontmatter to any note missing it, and
builds the index. Running it again is a no-op.

Your own frontmatter is preserved byte-for-byte. Storm never round-trips your
YAML through a serializer, because doing so would reorder keys, drop comments,
and normalise quoting — rewriting effectively every file in the vault. Instead
it replaces or inserts individual lines and passes everything else through
untouched:

```yaml
---
id: 9f3250ea-13af-4aa3-8546-1971915586a6   # added
created: 2026-08-05T10:00:00Z              # added
modified: 2026-08-05T10:04:12Z             # added
aliases: [storm, sync-design]              # yours, untouched
cssclass: wide-page                        # yours, untouched
# a comment I wrote by hand                # yours, untouched
tags: [homelab, project]                   # yours, untouched
---
```

`.obsidian/`, `.git/`, `.trash/` and every other dotted directory are skipped.

## API

All routes need `Authorization: Bearer <token>`, except `/v1/health`.
WebSocket clients may pass `?token=` instead, since browsers can't set headers
on a handshake.

| | |
|---|---|
| `GET /v1/health` | liveness, unauthenticated |
| `GET /v1/vaults` | every vault: id, name, directory, note count, missing |
| `POST /v1/vaults` | `{name}` — creates the directory under the root |
| `PATCH /v1/vaults/{v}` | `{name}` — display name only, nothing moves |
| `DELETE /v1/vaults/{v}` | unregisters. **Never deletes files.** |
| `GET /v1/config` | storage root, state directory, vault count, MCP state |
| `PUT /v1/config` | `{vault_root, orphan_ok?}` — see below |
| `PUT /v1/config/mcp` | `{enabled, writable?}` — persisted, see MCP below |
| `GET /v1/recents?limit=` | recently opened notes, across every vault |
| `WS /v1/stream` | change events for every vault, each tagged `vault_id` |

Everything note-shaped is scoped to a vault:

| | |
|---|---|
| `GET /v1/vaults/{v}/tree` | all notes + folders + current `seq` |
| `GET /v1/vaults/{v}/sync?since=&limit=` | changes after `seq` — the delta-sync primitive |
| `GET /v1/vaults/{v}/notes/{id}` | metadata + content |
| `POST /v1/vaults/{v}/notes` | `{path, content}` |
| `PUT /v1/vaults/{v}/notes/{id}` | `{base_version, content, device_id?}` — see below |
| `POST /v1/vaults/{v}/notes/{id}/move` | `{new_path}` |
| `POST /v1/vaults/{v}/notes/{id}/opened` | records an open, for the recents list |
| `DELETE /v1/vaults/{v}/notes/{id}` | |
| `POST /v1/vaults/{v}/folders` | `{path}` — creates an explicit folder |
| `POST /v1/vaults/{v}/folders/rename` | `{from, to}` — moves every note under it |
| `DELETE /v1/vaults/{v}/folders/{path}` | refused unless empty |
| `GET /v1/vaults/{v}/search?q=&limit=` | FTS5 with highlighted snippets |
| `GET /v1/vaults/{v}/notes/{id}/backlinks` | linked mentions |
| `GET /v1/vaults/{v}/tags`, `/tags/{tag}` | tag browser |

A vault id that was never registered is a `404`; one whose directory has gone
is a `409`, and stays in the registry rather than disappearing.

### Vaults, folders, and the storage root

`--vault-root` is a directory *containing* vaults. Each subdirectory is one;
`state/vaults.json` maps a stable UUID to the directory and a display name, so
a rename does not orphan the index. The root is rescanned at startup, on a root
change, and on vault create/delete — not continuously.

**The stored root wins.** `state/vaults.json` holds the storage root, and that
is the setting; `--vault-root` only seeds a registry that does not exist yet. If
both are given and disagree, the server logs a warning naming each and uses the
stored one. Change it in the app under Server, or by editing `vaults.json` —
either way Storm still never moves the directories.

**Folders are recorded, not only derived.** The tree still derives folders from
note paths, but a folder created through the API is also written to a `folders`
table and exempted from the empty-directory pruning that runs after a delete or
move. Without that exemption a folder made on purpose would vanish the moment
its last note left.

**`PUT /v1/config` never moves files.** It points the server at directories
someone has already moved. If none of the registered vaults are found under the
new root it returns `409` naming what would be orphaned, unless the caller
passes `orphan_ok` — because a server that boots healthy with zero vaults reads
as "my notes are gone".

`--vault` is accepted for one release and means "the parent of this is the
storage root". It refuses to start if that path is missing, rather than coming
up empty.

### The PUT is the whole sync design

Clients send the version they edited from. The server reconciles:

```
base_version == current  ->  fast-forward, take the client's text
base_version <  current  ->  3-way merge of (base, server, client)
                               clean    -> 200 {merged: true}
                               conflict -> 200 {conflict: true}, markers in the file
```

A conflict is **never** rejected and never spawns a `.sync-conflict-*` sibling.
The marked text is written into the note, and the pre-merge server version stays
in `note_versions`, so nothing is lost. You resolve it by deleting four lines.

Whenever the response carries `merged` or `conflict`, the client **must** adopt
the returned `content` — otherwise its next save races a version it never saw.

Two behaviours worth knowing:

- The server owns `modified:`. Clients must not write it. It is normalised out
  of all three sides before merging, or it would conflict on every concurrent
  write (the server rewrites it on every save).
- diff3 is line-based with context, so *adjacent* edits conflict even though
  they don't overlap — deleting a paragraph while another device edits the next
  one is reported as a conflict. Rare for a single-user vault. If it turns out
  not to be, that's the signal to revisit `yrs`.

## MCP

The Model Context Protocol is served at `/mcp`, so an agent can work with the
vaults. Nine read tools — `list_vaults`, `get_vault`, `search`, `get_note`,
`get_related_notes`, `list_tags`, `recent_notes`, `get_note_history`,
`get_note_version` — and three write tools, `create_note`, `update_note` and
`delete_note`, served only in read-write mode.

**Read-only is a mode, not a disabled state.** The write tools are filtered out
of the router when `mcp_writable` is off, so a read-only server does not
advertise them at all — an agent cannot choose a tool it was never shown.
`update_note` carries `base_version` into the same diff3 merge the Flutter
client uses, and `create_note` stamps `source: ai` through the frontmatter
line-splicer. There is no trash: `delete_note` removes the file at once, exactly
as the client's delete does, and only `note_versions` still holds the text.

**Off by default, and switched at runtime.** The setting lives in
`state/vaults.json` beside the storage root, so it survives a restart, and the
Flutter app can change it under **Server ▸ AI access**. While off, `/mcp`
answers 404 to every request.

```sh
# Turn it on from the app, or from the API:
curl -X PUT http://host:8484/v1/config/mcp -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' -d '{"enabled":true}'

# --mcp turns it on at boot. It is an override, not the source of truth:
# switching it off in the app later sticks, until the flag is passed again.
storm-server serve --vault-root ~/vaults --state ~/state --token "$STORM_TOKEN" --mcp
```

`GET /v1/config` reports `mcp_enabled` and `mcp_writable`.

MCP resolves vaults through the same registry the REST API uses — the same root,
the same `vault_of`, no path handling of its own — so it always sees whatever
storage root is currently in force.

Same bearer token as everything else, because `/mcp` is nested above the auth
middleware in `api.rs`. That ordering is load-bearing: axum applies a layer only
to routes registered above it, so mounting `/mcp` after it would leave the one
unauthenticated route on the server. `tests/mcp_e2e.py` checks it.

Point a client at `http://<host>:<port>/mcp` with `Authorization: Bearer
<token>`. If it is reached at a LAN address and every call fails, suspect the
`Host` header before the token: the SDK restricts hosts to loopback by default,
and `mcp::allowed_hosts` widens that to the address the server was bound to. A
wildcard bind (`--host 0.0.0.0`) cannot enumerate them, so the check is disabled
and logged at startup.

## External edits

The vault stays greppable and can be exported read-only over the NAS share, so
the server doesn't assume it's the only writer. A `notify` watcher (500 ms
debounce) reindexes anything that changes on disk and pushes it to clients — so
editing a note with `nvim` on the server shows up everywhere.

The server's own writes are filtered by content hash rather than bookkeeping,
which stays correct even when events arrive late, coalesced, or out of order.

## Durability

- Writes are atomic: temp file in the same directory, `fsync`, rename. A crash
  can leave a stray temp file but never a truncated note.
- Paths from the API cannot escape the vault (`..`, absolute paths, symlinks,
  and dotted segments are all rejected).
- `state/` is rebuildable from `vault/` at any time — but it holds version
  history, which the merge needs, so back up both.

## Tests

```sh
cargo test      # 91 unit tests
cargo clippy --all-targets
```

The unit tests cover the sharp edges: frontmatter byte-preservation, merge
outcomes, path traversal, tag/link extraction against code blocks, and index
reconciliation. There is also an end-to-end script that drives a live server
through the sync matrix (`apps/server/tests/e2e.py`).
