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
- **Description:** A short sketch of Storm's self-hosted layout: clients, sync server, plain markdown vaults.

## Page hero

**Eyebrow:** How it works

**Heading:** One server. Many devices. Files you can still open.

**Lede**

Storm is intentionally small. This page is the sketch; depth lives in the repo.

## Shape

Clients (macOS, Android, web — one Flutter codebase) talk REST and a WebSocket
to **storm-server**, a Rust binary in the homelab. The server owns merge,
search, tags, attachments, and version history. Notes remain plain markdown
under a storage root you control.

```
clients  ──►  storm-server (axum)
                    │
          ┌─────────┴──────────┐
          ▼                    ▼
   vaults/*.md           state/ (indexes)
   plain markdown        never inside vaults
```

## Invariants that matter

- **The vault is plain markdown.** Storm state lives in a sibling `state/`
  directory, never mixed into notes.
- **Notes are tracked by UUID,** so renames and moves are metadata, not new
  files.
- **Offline is normal.** Edits queue on the device and replay when the server
  returns; conflicts that cannot merge are written into the note as visible
  markers.
- **No accounts, no sharing product.** One person, several devices, one token
  on the home network.

Full invariant list for agents: [[Storm Invariants]].

## Go deeper

| Topic | Where |
|---|---|
| Living plan + decisions | [`PLAN.md`](https://github.com/dewanshDT/Storm/blob/main/PLAN.md) |
| Deploy + apt | [`deploy/`](https://github.com/dewanshDT/Storm/tree/main/deploy) |
| Design system | `docs/design_handoff_storm_design_system/` in the repo |

**CTAs:** Install → `/install` · Repository → `https://github.com/dewanshDT/Storm`

## Related

[[Storm Website]] · [[Storm Website Install]] · [[Storm Codebase Map]]
