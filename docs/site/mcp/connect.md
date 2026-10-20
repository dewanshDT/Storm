---
title: Connect an AI app
description: Let Claude Code, Claude Desktop, Cursor and other MCP clients read — and, if you allow it, write — your notes.
---

## Turn it on

MCP is **off by default**. In **Settings › AI access**, turn on **Let AI apps
read your notes**. Writing needs **Let them create, edit and delete** as well;
without it the write tools are not even offered. The tools are listed in
[MCP tools](/docs/reference/mcp-tools/).

Each AI app gets its own **key**: **Settings › Devices & access › New key**.
A key is shown once, revocable, and accepted on `/mcp` and nowhere else —
your password and your devices' sessions never go into a config file.

## `storm-runtime mcp-connect` (recommended)

An AI app on another computer (Claude Code, Claude Desktop, Cursor) reaches
Storm through `storm-runtime mcp-connect`, a stdio adapter the app starts
itself. It is the same `storm-runtime` binary as a Runtime Host, but needs
none of a host's setup: no account, no service, no enrollment.

- **Direct first, relay as fallback.** It dials `--addr` (the server's LAN
  address) and, when that is unreachable, each `--relay` in turn, so one
  config works at home and away.
- **Identity before the key.** It runs the server challenge against `--pk`
  for `--server` before it sends anything, on either route, so the key is
  never sent to a server that is not yours. (A relay still carries the
  traffic, as it does the app's.)
- **The key from the environment.** `STORM_MCP_KEY`, or `--key-file`; never an
  argument, where every user on the machine could read it in `ps`.
- **Errors say why.** A request it cannot deliver is answered with a JSON-RPC
  error carrying a `storm_error` code, so the app shows the reason instead of
  timing out.

The app writes the configuration: **Settings ▸ Devices & access ▸ New key**
shows a `claude mcp add` line and a JSON entry, filled in with the server id,
its pinned key, its LAN address and its registered relays. The raw HTTP entry
is under *Other clients (HTTP, LAN only)*.

To put the binary on a computer that is not a Runtime Host, the installer has
an **MCP only** choice (`--mcp`; `install.ps1 -Mcp` on Windows), from
v0.11.0. It downloads `storm-runtime` from the release (the Linux x86_64 binary, or the
macOS universal tarball), checks it against `checksums.txt` and copies it to
`/usr/local/bin/storm-runtime`, the one `sudo` step. On Linux a machine with
the `storm-runtime` package already has it on PATH, so there is nothing to do;
on macOS a Homebrew link at that path is left alone. `install.sh uninstall
--mcp` removes the copy. On Windows, `install.ps1 -Mcp` copies
`storm-runtime.exe` into `%LOCALAPPDATA%\Programs\Storm` and adds that to your
user PATH, without administrator.

Apps started from a desktop (Claude Desktop on macOS in particular) may not
see `/usr/local/bin` on their PATH; give `"command"` the full path then.

## The plain HTTP entry

On your home network, an MCP client can also talk to `/mcp` directly, with no
adapter and no identity check:

```json
{"mcpServers": {"storm": {"type": "http", "url": "http://<server>:8484/mcp",
  "headers": {"Authorization": "Bearer stk_…"}}}}
```

The key dialog shows this too, under *Other clients (HTTP, LAN only)*.
