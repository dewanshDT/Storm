# storm-server

The Storm sync server: a single Rust binary that owns the canonical vault.

```
/srv/storm/
├── vault/          canonical markdown — greppable, rsync-able, Obsidian-readable
│   ├── Daily/2026-08-05.md
│   └── Projects/Ideas.md
└── state/
    └── index.db    derived index + version history (rebuildable from vault/)
```

`state/` is a **sibling** of `vault/`, never inside it, so the vault directory
contains nothing but your notes.

## Running it

Point it at a **copy** of your vault first. The `--dry-run` pass writes nothing
and reports what an import would change:

```sh
cargo run -- --vault /srv/storm/vault --state /srv/storm/state --dry-run
```

```
Dry run — nothing was written.

  markdown files found : 1284
  would add `id` to    : 1284
      Daily/2026-08-05.md
      ...

Run again without --dry-run to apply.
```

Then for real:

```sh
cargo run --release -- \
  --vault /srv/storm/vault \
  --state /srv/storm/state \
  --host 0.0.0.0 --port 8484 \
  --token "$(openssl rand -hex 32)" \
  --web /srv/storm/web        # optional: serves the Flutter web client
```

Omit `--token` and one is generated and printed for that run. Set `STORM_TOKEN`
in the environment to keep it stable.

**v1 is LAN-only.** A single shared bearer token with no TLS is only defensible
inside the LAN. TLS and per-device token rotation must land *before* this is
ever reachable from outside it.

## Importing an existing Obsidian vault

There is no separate importer — the startup scan *is* the importer. It walks the
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
| `GET /v1/tree` | all notes + derived folder list + current `seq` |
| `GET /v1/sync?since=&limit=` | changes after `seq` — the delta-sync primitive |
| `GET /v1/notes/{id}` | metadata + content |
| `POST /v1/notes` | `{path, content}` |
| `PUT /v1/notes/{id}` | `{base_version, content, device_id?}` — see below |
| `POST /v1/notes/{id}/move` | `{new_path}` |
| `DELETE /v1/notes/{id}` | |
| `GET /v1/search?q=&limit=` | FTS5 with highlighted snippets |
| `GET /v1/notes/{id}/backlinks` | linked mentions |
| `GET /v1/tags`, `GET /v1/tags/{tag}` | tag browser |
| `WS /v1/stream` | change events pushed to connected clients |

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
cargo test      # 86 unit tests
cargo clippy --all-targets
```

The unit tests cover the sharp edges: frontmatter byte-preservation, merge
outcomes, path traversal, tag/link extraction against code blocks, and index
reconciliation. There is also an end-to-end script that drives a live server
through the sync matrix (`apps/apps/server/tests/e2e.py`).
