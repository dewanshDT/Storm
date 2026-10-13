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
- **Description:** Storm architecture: Flutter clients, a Rust sync server, plain Markdown vaults, Runtime Hosts for agents, and MCP.

## Page hero

**Eyebrow:** 04 · Architecture

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

Agents run on **Runtime Hosts** — `storm-runtime` on Linux, macOS or Windows,
on the server's machine or another. A host dials out to the server, so it needs
no inbound port, and it holds no notes: the agent reads and writes through the
server like any other client. Integrations connected in the app are reached
through the server too, which keeps their credentials.

```
macOS · Android · Web  AI applications
       │ REST + WS        │ MCP
       ▼                  ▼
  storm-server (Rust / axum)  ◄──────  storm-runtime
       │                     dials out Claude Code ·
  ┌────┴─────┐                         OpenCode · shell
  ▼          ▼
vaults/    state/
*.md       indexes · auth.db ·
           agent.db · gateway.db
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
- **One account, and it is yours.** Storm is single-user (decision 82): one
  password, per-device pairing and sessions, and revocable MCP keys — all in
  `state/auth.db` on your server. No cloud account, no third-party identity
  provider.
- **A Runtime Host is never an authority.** It runs agents and nothing else;
  the server decides who may start a session, which vault an agent may write
  to, and which integration tools it may call.

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
