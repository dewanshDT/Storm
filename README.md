# Storm

Self-hosted, Markdown-native knowledge system. A Rust sync server owns the
canonical vaults on infrastructure you control; Flutter clients on macOS,
Android, and web keep pace. The same knowledge is available to AI agents
through MCP.

> **Your knowledge. On your infrastructure.**

Site: [storm.dewansh.space](https://storm.dewansh.space) ·
Clients: [storm.dewansh.space/clients](https://storm.dewansh.space/clients) ·
Releases: [GitHub Releases](https://github.com/dewanshDT/Storm/releases)

```
┌──── clients (one Dart codebase) ────┐
│  macOS · Android · Web              │
│  editor · cache · outbox · sync     │
└──────────────┬──────────────────────┘
        REST + WebSocket
┌──────────────┴──────────────────────┐
│  storm-server (Rust, axum)          │
│  3-way merge · FTS5 · MCP · watcher │
└──────┬───────────────────┬──────────┘
  vaults/*.md           state/
  plain markdown        registry + indexes
```

Notes stay ordinary `.md` files under a storage root you control. Storm’s own
state lives in a sibling `state/` directory — never inside the vaults.

## Install (server)

Debian / Ubuntu — apt bootstrap, then start:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh
sudo storm-server up
sudo storm-server status
```

That URL is the **apt repository** root (not the marketing site). Details:
[`deploy/README.md`](deploy/README.md). Clients (macOS zip, Android APK, web
UI) are on [Releases](https://github.com/dewanshDT/Storm/releases) and the
[Clients page](https://storm.dewansh.space/clients).

### Update

```sh
sudo apt update
sudo apt install --only-upgrade storm-server
sudo systemctl restart storm-server
sudo storm-server status
```

Current release: **v0.2.3** (pre-release).

## Layout

| Path | What |
|---|---|
| [`apps/server`](apps/server/README.md) | Rust sync server (REST, WebSocket, MCP) |
| [`apps/client`](apps/client/README.md) | Flutter app — macOS, Android, web (Linux desktop deferred) |
| [`apps/www`](apps/www/README.md) | Marketing site (Astro) → [storm.dewansh.space](https://storm.dewansh.space) |
| [`deploy/`](deploy/README.md) | systemd units, apt bootstrap, backup |
| [`PLAN.md`](PLAN.md) | Living plan, status, and decision log |
| `docs/` | Design briefs and findings (see links in `PLAN.md`) |

## Development

Requires Rust and Flutter. Point the server at a directory that **contains**
vaults (`VAULT_ROOT`), not at a single vault:

```sh
make dry-run VAULT_ROOT=~/vaults-copy   # report only — writes nothing
make server  VAULT_ROOT=~/vaults-copy   # http://127.0.0.1:8484, token: testtoken
make client                             # or: make web / make serve-web
make www-dev                            # marketing site locally
```

`make help` lists every target.

## Testing

```sh
make check        # clippy + analyze + both unit suites
make test-live    # starts a server, runs integration suites, tears down
```

Unit suites need nothing running. Live suites drive the real client against a
real server.

## MCP

When enabled, the server exposes twelve tools over Streamable HTTP at `/mcp`
(nine read tools always; create / update / delete when writable). Off by
default. See [`apps/server/README.md`](apps/server/README.md).

## Status

M0–M15 are done and deployed. M16 (marketing site) is live at
[storm.dewansh.space](https://storm.dewansh.space). v1 is intended for a
trusted network with a shared bearer token — TLS and per-device tokens before
exposing it more widely.

See [`PLAN.md`](PLAN.md) for milestone status, the decision log, and open
items. License: [MIT](LICENSE).
