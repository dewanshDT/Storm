# Storm

Self-hosted, markdown-first notes. Flutter clients talking to a small Rust
server that owns the canonical vault — replacing Obsidian + Syncthing with
something whose format and protocol you control.

The vault stays a plain directory of `.md` files: greppable, rsync-able, and
readable by Obsidian if you ever want out.

```
┌──── clients (one Dart codebase) ────┐
│  macOS · Linux · Android · Web      │
│  editor · cache · outbox · sync     │
└──────────────┬──────────────────────┘
        REST + WebSocket (LAN)
┌──────────────┴──────────────────────┐
│  storm-server (Rust, axum)          │
│  3-way merge · FTS5 · file watcher  │
└──────┬───────────────────┬──────────┘
  vault/                state/
  plain markdown        index + history
```

## Layout

| Path | What |
|---|---|
| `apps/server` | Rust sync server — [README](apps/server/README.md) |
| `apps/client` | Flutter app, all four targets — [README](apps/client/README.md) |
| `spike/editor_spike` | Frozen M0 prototype, deleted after M5 |
| `docs/prd.md` | Original brief, not maintained |
| `PLAN.md` | **Living plan, status and decision log** |

## Getting started

Requires Rust and Flutter. Point the server at a **copy** of your vault first —
`--dry-run` writes nothing and reports what an import would change:

```sh
make dry-run VAULT=~/vault-copy
make server  VAULT=~/vault-copy      # http://127.0.0.1:8484, token: testtoken
make client                          # or: make web
```

`make help` lists everything.

## Testing

```sh
make check        # analyze + clippy + both unit suites
make test-live    # starts a server, runs the integration suites, cleans up
```

The unit suites need nothing running. The live suites drive the real client
against a real server, which is where a protocol mismatch actually shows up.

## Status

v1 is LAN-only with a single shared bearer token — defensible only inside the
LAN. TLS and per-device tokens must land **before** this is reachable from
anywhere else.

See [PLAN.md](PLAN.md) for milestone status, the decision log, and open
blockers.
