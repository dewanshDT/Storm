---
title: Configuration
description: storm.env for the server, runtime.toml for a Runtime Host.
---

Most people never edit these: `storm-server up` and the installers write
them. Neither ever holds a credential.

## `storm.env` (server)

On Linux, `/etc/storm/storm.env` (mode 600), read by the systemd unit.

| Variable | Default | Meaning |
|---|---|---|
| `STORM_VAULT_ROOT` | `/srv/storm/vaults` | The storage root: a folder of vault folders, never a single vault. Only seeds a first run — after that the root lives in the server's records and is changed in **Settings › Storage**. |
| `STORM_STATE` | `/srv/storm/state` | Storm's own records: indexes, the account, agents, integrations. |
| `STORM_WEB` | `/usr/share/storm/web` | The web app, from the package. |
| `STORM_HOST` | `0.0.0.0` | Listen address. All interfaces, so your phone can reach it; never port-forward it. |
| `STORM_PORT` | `8484` | Listen port. Change it with `storm-server up --port N`, which also moves the firewall rule. |
| `STORM_DISCOVERY` | on | `0` stops the server announcing itself on the network and refuses password pairing, whatever the app's setting says. QR pairing still works. |
| `STORM_BACKUP_DEST` | `/srv/storm/backups` | Where nightly backups go. Use another disk. |
| `STORM_BACKUP_KEEP_DAYS` | `30` | How long dated backups are kept. |

`storm-server up --data-root DIR` changes the vault, state and backup paths
together. All flags: [storm-server](/docs/reference/cli/storm-server/).

## `runtime.toml` (Runtime Host)

At `/etc/storm-runtime/runtime.toml`, `/Library/StormRuntime/runtime.toml` or
`C:\ProgramData\StormRuntime\runtime.toml`. Every value is an operational
default, not a limit.

```toml
# Each subdirectory of a root is a workspace, named after its folder.
workspace_roots = ["/var/lib/storm-runtime/workspaces"]
max_sessions = 8
scrollback_bytes = 4194304

# Where agent CLIs are looked for, and the PATH every session gets.
# path = ["/usr/local/bin", "/usr/bin", "/bin"]

# Omit [[providers]] to offer every built-in: claude-code (the default),
# opencode, codex, cursor (cursor-agent), command-code and shell. Cursor and
# Command Code are terminals only: they get no notes or integrations.
# Listing any replaces the defaults, so list every one you want.
# [[providers]]
# id = "claude-code"
# env_file = "/etc/storm-runtime/claude.env"   # secrets go here, mode 0600

# [[providers]]
# id = "opencode"
# [providers.settings]                          # merged into each session's OpenCode config
# model = "anthropic/claude-sonnet-4-5"

# [[providers]]
# id = "shell"
# command = "/bin/bash"
# args = ["-l"]
```

A workspace root inside, or around, a Storm data root is refused, and so are
roots under home folders on macOS and Windows; see
[Runtime Hosts](/docs/agents/hosts/). On Linux, a root outside
`/var/lib/storm-runtime` also needs the unit widened with `ReadWritePaths`.
OpenCode's own `mcp` settings are ignored in sessions: only Storm's
integrations are ever offered to an agent.
