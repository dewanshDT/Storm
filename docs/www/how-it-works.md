---
tags: [storm, www, docs]
route: /how-it-works
---

# Storm Website How it works

Content for route **`/how-it-works`**. Parent: [[Storm Website]].

Thin sketch only — link out for depth. Do not turn this into an API reference
or second `PLAN.md`.

## Meta

- **Title:** How it works · Storm
- **Description:** Storm architecture: Flutter clients, a Rust sync server, plain Markdown vaults, and MCP for AI agents.

## Page hero

**Eyebrow:** 03 · Architecture

**Heading:** One knowledge layer. Many interfaces.

**Lede**

Storm is intentionally small. This page is the sketch; depth lives in the
repository.

## Shape

Clients on macOS, Android, and web — one Flutter codebase — talk REST and a
WebSocket to **storm-server**, a Rust binary you run. The server owns merge,
search, tags, attachments, and version history. Notes remain plain Markdown
under a storage root you control. Compatible AI agents reach the same vault
through MCP.

```
macOS · Android · Web
          │  REST + WebSocket
          ▼
   storm-server (Rust / axum)
          │
   ┌──────┴──────┐
   ▼             ▼
vaults/*.md   state/
plain md      indexes · registry · auth.db
                     ▲
                     │ MCP
              AI applications
```

## Invariants that matter

- **The vault is plain Markdown.** Storm state lives in a sibling `state/`
  directory, never mixed into notes.
- **Notes are tracked by UUID,** so renames and moves are metadata, not new
  identity.
- **The server owns the canonical vault.** Clients cache and queue; a merged
  or conflict response means the client adopts the server’s text.
- **Conflicts stay in the file.** Markers in the note — not a hidden sibling.
- **Offline is normal.** Creates and edits queue and replay. Search is
  server-side.
- **Accounts are local and yours.** An owner account, per-device pairing,
  sessions, and revocable MCP keys — all in `state/auth.db` on your server. No
  cloud account, no third-party identity provider. One person with several
  devices is the default, not the limit.

Full invariant list for agents: [[Storm Invariants]].

## Go deeper

| Topic | Where |
|---|---|
| Living plan + decisions | [`PLAN.md`](https://github.com/dewanshDT/Storm/blob/main/PLAN.md) |
| Deploy + apt | [`deploy/`](https://github.com/dewanshDT/Storm/tree/main/deploy) |
| Server + MCP | [`apps/server/README.md`](https://github.com/dewanshDT/Storm/blob/main/apps/server/README.md) |

**CTAs:** Install → `/install` · Repository → `https://github.com/dewanshDT/Storm`

## Related

[[Storm Website]] · [[Storm Website Install]] · [[Storm Codebase Map]]
