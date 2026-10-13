---
tags: [storm, www, docs]
route: /
---

# Storm Website Home

Content for route **`/`**. Parent: [[Storm Website]].

## Meta

- **Title:** Storm
- **Description:** Storm is a self-hosted, Markdown-native knowledge system with cross-platform clients, MCP access for AI agents, and coding agents that run on your own machines.
- **Theme:** SlowFlow earth (warm paper) — see `apps/www/src/styles/tokens.css`

## Hero

**Eyebrow:** Self-hosted knowledge

**Headline**

> Your knowledge.
> On your infrastructure.

**Support**

Storm is a self-hosted, Markdown-native knowledge system with cross-platform
clients, first-class access for AI agents through MCP, and coding agents that
run on your own machines.

**CTAs**

| Label | Target |
|---|---|
| Install Storm | `/install` |
| Read the docs | `#docs` |
| GitHub | `https://github.com/dewanshDT/Storm` |

**Install command (real)**

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh
```

**Install note:** an interactive installer for Linux (Debian/Ubuntu) and
macOS — the server, the app, a Runtime Host, or everything. Links: Windows
(`/install#windows`), full installation.

**Metadata row:** Rust server · Flutter client · Markdown · MCP · Agents · Self-hosted

## Sections

1. **01 · Foundation** — One source of truth. Storage-root tree; `state/` sibling.
2. **02 · AI access** — every MCP tool, named: the read and write lists in
   `index.astro`, which `make www-check` compares with `mcp.rs`; agents also
   get `session_context` and never `delete_note`. Static conceptual demo. Off
   by default, the write tools need a second switch, and an agent
   authenticates with an MCP key minted in the app (`stk_…`, `/mcp` only).
3. **03 · Agents** — Runtime Hosts run Claude Code, OpenCode or a shell on
   Linux, macOS or Windows; start a session from a note, drive the terminal
   from any device, paste images, resume an ended Claude Code / OpenCode
   session; the agent writes only to the vault chosen at launch.
   Integrations (Notion, Linear, …) are connected once in the app; the server
   keeps their credentials, encrypted, and an agent gets only allowed tools.
4. **04 · Architecture** — Flutter clients ↔ Rust server ↔ Markdown vaults;
   MCP; Runtime Hosts dial out.
5. **05 · Storage** — Ordinary `.md` files.
6. **06 · Client** — macOS · Android · Web only (no fake screenshots).
7. **07 · Synchronization** — Synced / Offline / Conflict status language.
8. **08 · Installation** — Links into `/install`. Four steps: run the
   installer (Linux, macOS, Windows), get a client, **sign in** (pair the
   device, set your Storm's password — one account, no cloud account),
   update by re-running the installer.
9. **09 · Documentation** — Repo deep links only (no docs portal yet; see
   *Website v2 — Plan* in the vault).

## Design notes

- SlowFlow tokens; mint BrandMark ground does not retheme the page.
- No Obsidian, Syncthing, or competitor comparisons.
- Claims must match the live app — do not invent features.
- Prototype screenshots are design reference only; do not ship as product photos.
