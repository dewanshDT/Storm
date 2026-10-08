# Storm UI/UX Discovery & Product Model

> **What this is.** A discovery document for a future product-design and
> information-architecture effort. It describes what Storm is, what the client
> does today, why it is shaped the way it is, where the experience is breaking
> down, and which questions must be settled before redesigning anything.
>
> **What this is not.** It is not a redesign, not a proposed navigation
> structure, and not a critique of visual polish. Where a sentence reads like
> a recommendation, it is a question for the designer, not an answer.

**Prepared:** 2026-10-08.

**Baseline inspected:**
- The client, server and runtime code on **`origin/staging` at `d864f8d`**
  (MCP Gateway V1 merged, PR #93). This is the development trunk.
- **`main` (v0.3.1) does not yet contain Integrations.** The `staging` diff
  over `main` in `apps/client/lib` is entirely the gateway client: the
  `integrations_*`, `oauth_*` files, the launcher toggle and the Server
  settings entry.
- The product and specification notes in the operator's Storm vault
  (`personal` → `Storm/…`), read through Storm MCP. Per `CLAUDE.md`, that
  vault holds the authoritative product and spec material, including the
  frozen Agent Runtime and MCP Gateway specifications.
- `PLAN.md` (on `staging`), `CLAUDE.md`, `README.md`, `kit/`.
- The screenshots in `docs/screenshots/`.

**Not consulted.** The repo's older design briefs (`docs/storm-ui.md`,
`docs/storm-ui-refactor.md`, `docs/storm-adaptive.md`,
`docs/storm-multi-vault.md`, `docs/storm-properties.md`) and the M14
design-system handoff (`docs/design_handoff_storm_design_system/`). The
operator directed this pass to the vault's documentation instead. These
briefs predate Agents and Integrations, but `docs/storm-ui.md` ("what every
screen does today, for designing against") is still worth the designer's
time.

**Screenshot caveat.** Everything in `docs/screenshots/` predates the Agents
space and Integrations: the README still says v0.2.9. None of them shows
agent surfaces.

## How to read the labels

| Label | Meaning |
|---|---|
| **[FACT]** | Explicitly supported by code or docs. A reference is given. |
| **[INTENT]** | Product intent, stated in product or spec documents. |
| **[PROBLEM]** | An observed UX or IA problem, evident in the implementation. |
| **[INFERENCE]** | A reasonable interpretation that nothing states outright. |
| **[OPEN]** | A question for the design process. |
| **[CONSTRAINT]** | Something a design must respect. Section 19 separates hard constraints from preferences. |

Vault notes are cited by title, e.g. *Vision*, or by full path,
e.g. *Agent Runtime/V1 Specification Freeze*. Code is cited by path under
`apps/client/lib/` unless stated otherwise.

---

## 1. Executive Summary

**Storm began as a self-hosted replacement for Obsidian plus Syncthing.** That
is a Markdown notes app with a Rust sync server. It has since become the
first layer of a much broader product. [FACT: `docs/prd.md` §1; `CLAUDE.md`
first line; *Vision*]

The stated long-term thesis: **"Storm becomes the persistent workspace where
humans and AI agents work together"**, and for developers specifically, "a
developer workspace and control plane for persistent, knowledge-aware AI
agents". [INTENT: *Vision*]

Today the product has four functional layers, all shipped or merged:

1. **Knowledge.** Vaults of plain Markdown, sync, offline, search, tags,
   backlinks and typed properties. Mature: M0–M18.
2. **AI access to knowledge.** Storm as an MCP server, with per-account MCP
   keys. M13, M19.
3. **Agents.** Persistent coding-agent sessions (Claude Code, OpenCode, a
   shell) running on enrolled Runtime Hosts and driven through a terminal
   from any device. M20, accepted 2026-10-05, owner-only.
4. **Integrations.** The MCP Gateway: Storm holds third-party MCP credentials
   (Notion, Linear, GitHub…), and agents use them without seeing them.
   M21, merged to `staging` 2026-10-08, awaiting acceptance and release.

**The core IA finding.** The client's navigation was designed for layer 1.
Layers 2–4 were attached where there was room, each time with a careful
local rationale:
- MCP and its keys live in **Server settings**.
- Agents became a "space" beside Notes (decision 78), but differently on
  phone and desktop.
- Integrations sit in **Server settings ▸ Integrations**, below a settings
  screen and outside every shell.
- Kit, Storm's reusable agent-tooling layer, has **no UI identity at all**.
  It is a vault that happens to be named `kit`.

The result is a product whose architecture is much clearer than its
navigation:
- "Agent" means three different things.
- "MCP" means four.
- "Session" means two.
- Server-wide configuration is reached through a vault switcher.
- The desktop and phone layouts expose different top-level capabilities.

**Highest-impact problems** (details in §11–§12):
- **There is no global level in the IA.** The phone dashboard is the only
  cross-cutting home, and it does not exist at desktop width. As a result,
  recents and the Agents band are phone-only.
- **Server settings has become a catch-all.** It mixes infrastructure
  (storage root), security policy (MCP switches, registration), personal
  credentials (MCP keys), agent infrastructure (Hosts), third-party services
  (Integrations) and workspace content (the vault list).
- **Overloaded vocabulary.** Agent / session / provider / host / workspace,
  and four different "MCP"s.
- **Kit and the agent-role definitions are invisible**, though the product
  treats them as core (decision 64).
- **The two-way Notes | Agents switch** is explicitly marked in `PLAN.md` as
  the structure to revisit when a third space appears. The roadmap contains
  several candidates.

**Biggest open design questions** (§22):
- What is Storm's top-level object model? Workspace? Server? Vaults plus
  agents?
- Should the IA be organised by *activity* (Notes, Agents) or by *object*
  (Vaults, Sessions, Hosts, Integrations)?
- Where does the boundary between "using Storm" and "operating Storm" fall?
- How should owner-only power and member simplicity coexist?

---

## 2. Storm Product Thesis

### 2.1 In the product's own words

> **"Your knowledge. On your infrastructure."** [FACT: `README.md`; *Storm
> Website Home*]

> "Storm is a self-hosted, Markdown-native knowledge system with
> cross-platform clients and first-class access for AI agents through MCP."
> [FACT: *Storm Website Home*, the current public positioning]

> "Storm becomes the persistent workspace where humans and AI agents work
> together." … "The product is the workspace, not the agent." [INTENT:
> *Vision*, `status: proposed`]

### 2.2 The thesis, distilled [INTENT: *Vision*, *Problem*, *Product Strategy*]

1. **Knowledge you own, as plain files.** Markdown on infrastructure you
   control, readable without Storm. This is foundational and non-negotiable
   (`docs/prd.md` §2, `CLAUDE.md` invariants).
2. **Devices become control surfaces.** "The phone and laptop stop being the
   thing that must stay online for work to continue." Agent work belongs to
   the Storm Server, not to whichever device was looking at it (*Problem* 1
   and 4).
3. **Storm sits *above* coding agents.** It is agent- and model-agnostic.
   Claude Code, OpenCode, Codex, Cursor and others are pluggable workers.
   Storm provides persistence, workspace, knowledge, permissions, execution,
   sessions, terminal, orchestration, observability and the client UI
   (*Vision*).
4. **Shared context across agents.** One knowledge layer that every agent
   reads and writes with the same merge rules as a human device (*Problem* 3,
   *Shared Knowledge*).
5. **Long-term:** the "engineering graph". Requirements → decisions →
   implementation → tests → reviews → outcomes, traceable, with agent work as
   nodes. "The moat is accumulated context" (*Engineering Graph*,
   *Product Strategy*; `HYPOTHESIS` / `PROPOSED`).

### 2.3 What Storm explicitly is not [INTENT: *Vision*]

- Not an AI model. No inference, no weights.
- Not a personal AI assistant. It does not compete on "chat, messaging
  integrations, personality, voice assistants, generic automation".
- Not "Obsidian with AI", "Claude Code in a browser", "another AI assistant"
  or "another coding agent".
- Not a remote IDE, file editor, Git UI or server-shell product, in V1
  (*V1 Specification Freeze* §2).
- Not a marketplace or catalogue of integrations (*MCP Gateway V1
  Specification* §17).

### 2.4 Three planes [INTENT: *Vision*: "Vision, MVP, implementation"]

The docs are disciplined about three layers, and a designer should be too:

| Plane | Where it lives | Status |
|---|---|---|
| **Vision** | *Vision*, *Engineering Graph*, *Product Strategy* | Mostly `PROPOSED` / `HYPOTHESIS` |
| **MVP / V1** | *Roadmap*, the frozen V1 specs | Built |
| **Implementation** | The code on `staging` | What actually exists |

[CONSTRAINT] Every proposed feature is tested against one question: "Does
this make the core developer workflow meaningfully better?" Enterprise
features (SSO, approval workflows, governance) are "long-term possibilities.
**Do not implement them prematurely.**" (*Product Strategy*)

---

## 3. Product Mental Model

### 3.1 The five pillars [INTENT: *Vision*]

```
                    STORM
      ┌───────────────┼───────────────┐
  KNOWLEDGE        WORKSPACE        AGENTS
      └───────────────┼───────────────┘
                 ORCHESTRATION
                      │
                   CONTROL
```

| Pillar | Contents | Status |
|---|---|---|
| Knowledge | Vaults, search, metadata, links, MCP | `IMPLEMENTED` |
| Workspace | Projects, files, Git, shell, infrastructure | `PROPOSED`. Today only the bare "workspace = a directory on a Runtime Host". |
| Agents | Claude Code, OpenCode, shell | V1 built |
| Orchestration | Delegation, reviewers, parent/child sessions | `PROPOSED`, Phase 3 |
| Control | Clients, live sessions, terminal, permissions, observability | Partly built |

The relationship the Vision calls "the one that matters":

```
Knowledge  ↕  Agents  ↕  Workspace
```

Agents use knowledge. Agents modify the workspace. Workspace changes become
knowledge. Humans observe and control everything.

### 3.2 The system model the user operates inside

```
 ┌─────────────── Storm Clients (one Flutter codebase) ──────────────┐
 │ Android · macOS · Web  (Linux built, desktop "deferred"; no iOS)  │
 │ local cache + outbox (offline notes) · terminal views · settings  │
 └───────────────┬──────────────────────────────┬────────────────────┘
       REST + WS (notes)              REST + SSE (agents)
       direct LAN, or via optional storm-relay (notes; agents later)
 ┌───────────────┴──────────────────────────────┴────────────────────┐
 │ Storm Server (storm-server, Rust) — the ONLY authority            │
 │  vaults registry · per-vault index · 3-way merge · search         │
 │  auth (users, devices, sessions, MCP keys)                        │
 │  MCP server at /mcp (for external AI clients)                     │
 │  Agent Manager (sessions, hosts)  ·  MCP Gateway (integrations)   │
 └───────┬───────────────────────────────┬───────────────────────────┘
   plain .md vaults + state/        host link (host dials out)
                                 ┌───────┴──────────────────────────┐
                                 │ Runtime Host(s) (storm-runtime)  │
                                 │ executes agents in workspaces;   │
                                 │ no user data, no authority       │
                                 │ PTY · per-session MCP bridge     │
                                 └───────┬──────────────────────────┘
                                   Claude Code / OpenCode / shell
                                         │ (via gateway)
                                   third-party MCP upstreams
                                   (Notion, Linear, GitHub…)
```

[FACT: `README.md` diagram; *V1 Specification Freeze* §3; *MCP Gateway V1
Specification* §4]

### 3.3 The user's intended mental model

This is an [INFERENCE] assembled from [INTENT] statements.

- **"My Storm" is a server I own**, holding my knowledge and running my
  agents. Clients are windows onto it, and any device can pick up where
  another left off.
- **Knowledge is the durable thing.** Agents are workers that come and go.
  "If an agent disappears, Storm keeps the workspace, the knowledge and the
  record." (*Vision*)
- **Infrastructure exists but should stay out of the way.** Hosts, relays,
  storage roots, keys. The docs repeatedly push configuration away from the
  places where work happens. Decision 78: "Sessions are something you *do*;
  settings is where you configure."

[OPEN] The docs never name a single top-level container for "everything this
server holds for me". Is it "the server", "the workspace", "my Storm"? The
Vision uses "Storm Workspace" for exactly this, but "workspace" is already
taken by a host directory (§5).

---

## 4. Users and Core Jobs

### 4.1 Who [INTENT]

- **First user: the builder.** "Build Storm for the creator's own development
  workflow first." Milestone: "I genuinely don't want to go back to my old
  development workflow." (*Product Strategy*)
- **First audience: software developers.** Full-stack, backend, DevOps,
  ML/AI, security engineers, OSS maintainers, technical founders. "It does
  not attempt to serve every kind of knowledge worker." (*Vision*)
- **Hypothesised later:** Pro (managed Storm Cloud), Teams (shared workspaces
  and agent execution), Enterprise. (*Product Strategy*, `HYPOTHESIS`)

### 4.2 Roles that exist in the product today [FACT]

| Role | Source | What they can reach in the client |
|---|---|---|
| **Operator** | Runs `storm-server` / `storm-runtime`, edits `runtime.toml`, uses the CLI | Mostly outside the client: install, `pair`, `user add`, provider install and login on hosts |
| **Owner** | The first account (M19) | Everything, including Agents, Hosts, Integrations and server-wide config (decision 80) |
| **Admin** | Auth data model | Today the same as a member in the client. The A9 authorization release is pending, Q19–Q25 unanswered. |
| **Member** | Self-registered or CLI-created | Notes and vaults. Vault create, rename and remove are still open to them (decision 80 notes). **No agent entry point at all** (AC-S1). |
| **MCP client / external agent** | Holds an `stk_` MCP key | `/mcp` only. Never `/v1/agent/*`. |
| **Agent in a session** | `Actor::Agent` via the gateway | Owner's vaults (read; write only if flagged) and the owner's integrations |

[INFERENCE] In practice today, owner, operator and sole user are the same
person. The IA must still serve a member who sees no Agents, Hosts or
Integrations at all. That is a separate, simpler product view.

### 4.3 Core jobs

These come from the specs, roadmap and acceptance criteria.

| # | Job | Source |
|---|---|---|
| J1 | Write, read, find and organise notes on any device, offline included | PRD §3, M0–M18 |
| J2 | Keep several knowledge bases (vaults) separate but quickly switchable | M9/M10 |
| J3 | Let AI tools (external MCP clients) read, and optionally write, my notes | M13, `README` §MCP |
| J4 | Start a coding agent on my infrastructure, leave it, resume from another device | *V1 Freeze* §1 proof: "laptop → phone" |
| J5 | Run several agent sessions at once, across hosts and workspaces | AC-F6 |
| J6 | Give agents my knowledge and my third-party services safely, once | *MCP Gateway* §1 |
| J7 | Supply agents with reusable roles and conventions (Kit) | `kit/README.md`, decision 64 |
| J8 | Administer my server: storage, accounts, devices, keys, hosts, relay | M15, M19, relay |
| J9 | Pair new devices; recover from sign-out and disconnects | M19 |
| J10 (later) | Orchestrate related agents, approve risky actions, trace work back to requirements | Roadmap Phases 2–4, *Engineering Graph* |

---

## 5. Product Concepts

A glossary with ownership and status. Terms in **bold** are user-facing
words in the current UI.

| Concept | Definition | Owned by | Lifetime / identity | Status | User-facing word(s) today |
|---|---|---|---|---|---|
| **Storm Server** | `storm-server`: control plane, the sole authority | Operator / owner | `server_id`, random; Ed25519 identity | Shipped | "Server settings", the host shown under the vault bubble |
| Storage root | The directory containing vault directories | Server | Stored in `state/vaults.json`; stored value wins | Shipped | "**Vault storage root**", "**Vault root**" |
| **Vault** | A directory under the root, tracked by UUID; a knowledge base | Server (all vaults visible to all accounts until A9) | UUID; may be `missing` | Shipped | "Vaults", vault cards, vault switcher |
| **Note** | A `.md` file with YAML frontmatter, tracked by UUID | Vault | UUID in `id:`; versioned | Shipped | Note |
| Folder | A directory in a vault; explicitly created ones are recorded | Vault | Path | Shipped | Folder, Directory |
| Property | A frontmatter key; types live in hidden `_storm/` config | Note / vault | — | Shipped | "Properties" |
| Tag, wikilink, mention | `#tag` / `tags:`; `[[Title]]`; backlinks | Vault | Resolved by title | Shipped | Tags, **Mentions** / "linked mentions" |
| Recent | A note opened recently, recorded server-side, **cross-vault** | User/server | — | Shipped | "**Recently opened**" |
| Pin | A note kept offline on this device | Device | — | Shipped | Pin (long-press action) |
| **Kit** | The `kit` vault: agent role definitions, project layout spec, canonical scripts, skills | Server; seeded on first run (decision 64) | An ordinary vault named `kit`; deletable, never re-seeded | Shipped | Just a vault card called "kit". **No dedicated UI.** |
| Kit "agents" (roles) | `kit/vault/agents/*.md`: Architect, Lead, Researcher, Coder, Reviewer | Kit vault | Notes | Shipped (content) | None in the app. Used by external agents through adapters. |
| Canonical script | Runnable tooling stored only under kit's `scripts/` | Kit vault | Name | Shipped (MCP tools only) | None in the app |
| **MCP (server)** | Storm serving its vault tools at `/mcp` to external AI clients | Server-wide switches | `mcp_enabled`, `mcp_writable` | Shipped | "**AI access (MCP)**", "Let AI assistants read your notes", "Allow it to create, edit and delete" |
| **MCP key** | `stk_` credential for an external MCP client; acts as its account | **Account** (not server) | Shown once; revocable | Shipped | "**MCP keys**", "Manage MCP keys", inside *Server settings* |
| User / account | Server-local identity `(server_id, user_id)`, with a role | Server | — | Shipped | Login picker, "Allow new accounts" |
| Device | A paired client install with device credentials | Account | — | Shipped | "**Add a device**", "Disconnect" |
| Auth session (`ses_`) | A signed-in client session | Device/account | Revocable | Shipped | "Sign out" |
| Relay | Optional `storm-relay` that tunnels clients to a server without port forwarding | Operator | — | Shipped (no client settings screen) | Only the "Relayed" sync status |
| **Runtime Host** (`hst_`) | Enrolled `storm-runtime`: the execution plane; no authority | Owner enrolls; operator configures | Ed25519 key; online / offline / revoked | Shipped | "**Hosts**", "Enroll a host" |
| **Workspace** (agent) | A directory directly under a host's workspace root; `(host_id, name)` | Host | No UI to create or delete | Shipped | "Workspace" in the launcher |
| **Provider** | An agent integration: `claude-code`, `opencode`, `shell`; has a *kind* (`cli`) and *interactions* (`terminal`) | Host declares, owner picks the default | — | Shipped | "Provider", "**Default provider**" |
| Agent Definition | In V1, a provider entry. User-defined named agents are deferred. | Host | — | V1-minimal | Not shown |
| **Agent session** (`ags_`) | A persistent provider instance in one workspace on one host | Server (Agent Manager) | Status vocabulary §14; never migrates or auto-restarts | Shipped | "Session", "New session", "Running", "Ended" |
| Tab | A client-local view of a session, per device | Device | Persisted by `ags_` | Shipped | Tab strip (wide); switcher sheet (phone) |
| Interaction | The channel to a session. V1 has `terminal` only; structured kinds later. | Provider/runtime | — | V1 | Terminal |
| **Integration** / Connection (`mcc_`) | One owner's authenticated link to one upstream MCP service | **Owner**; credentials on the server, encrypted | `pending_auth` → `connected` ⇄ `needs_reauth`, or `disabled` | Merged to `staging` | "**Integrations**", "Add integration" |
| Built-in `storm` connection | Storm's own vaults offered to agents through the gateway | Implicit, cannot be deleted | — | Merged | "Storm vaults" ("Built in") |
| Grant | (session, connection) snapshotted at launch, plus the vault-write flag | Agent Manager | Fixed for the session's life | Merged | "**Allow vault writes**" (launcher toggle) |
| Engineering graph, tasks, relationships, approvals | Future concepts | — | — | `PROPOSED` | — |

### 5.1 Terminology collisions [PROBLEM]

These are the collisions a designer most needs to know about.

- **"Agent":**
  1. Kit's role definitions: `kit/vault/agents/Storm Lead.md`.
  2. The *Agents* space, which lists **sessions**, not agents.
  3. A provider such as Claude Code (*Vision*: "Agents — Claude Code,
     Cursor…").
  4. An external MCP client using an `stk_` key (`mcp_keys_screen.dart`:
     "an MCP client on a laptop, a script, an agent").

  V1 has no user-visible "agent" object at all (*V1 Freeze* §6).
- **"MCP":**
  1. Storm as an MCP **server**: "AI access (MCP)".
  2. **MCP keys** for external clients.
  3. "**MCP writes**", the server switch the launcher cites: "Needs MCP
     writes on in Server settings". The switch is labelled for external
     assistants, but it also gates agents' vault writes.
  4. The MCP **Gateway**, where Storm is an MCP **client** to upstreams. The
     UI calls it "Integrations" (G-D21), but its intro and copy reference
     MCP.
- **"Session":**
  1. The auth session (`ses_`, "Sign out").
  2. The agent session (`ags_`).

  The spec itself flags this: "Called 'agent session' wherever the auth
  meaning … could be confused" (*V1 Freeze* §4).
- **"Host":**
  1. Runtime Host.
  2. The server's hostname, shown in the vault bubble popover
     (`hostOf(baseUrl)`, `corner_bubbles.dart`).
- **"Workspace":**
  1. A directory on a Runtime Host.
  2. The Vision's "Storm Workspace", the whole product.
  3. The Vision's "Workspace" pillar: files, Git, shell.
- **"Provider":** an agent integration (AM20). The gateway spec explicitly
  forbids using it for upstream services (*MCP Gateway* §3), but users will
  hear "AI provider" as a model vendor.
- **"Settings":**
  - "Server settings" holds personal items: MCP keys.
  - "Client settings" holds account actions: Sign out, Add a device.
  - The phone opens client settings from a bubble labelled "**A**".

---

## 6. Current Product Surfaces

Each entry gives:
- **Purpose** and **Status**
- **Objects**
- **Actions**
- **Entry points**
- **Phone / Desktop** behaviour
- **Depends on**
- **Future relevance**
- **Constraints**

Desktop means the wide layout, ≥ 900px window width. That applies to macOS,
wide web and Linux. Phone means everything narrower, including tablets in
portrait (`breakpoints.dart`).

### 6.1 Dashboard (home), `/` [FACT: `ui/shell/dashboard.dart`]

- **Purpose:** the cross-vault home. "A grid of vaults over the notes you
  opened most recently — and, for the server's owner, the agents running
  now."
- **Objects:**
  - Masthead stats: total notes and "last synced"; no brand mark.
  - **Agents band** (owner only).
  - **Recently opened**: 5 rows, cross-vault, "vault · folder".
  - **Vaults** grid of colour-tile cards with note count and sync dot.
- **Actions:**
  - Open a recent or a vault.
  - Long-press a vault card to set its colour (written into the vault's
    `_storm/vault.md`).
  - Pull to refresh.
  - Pill: **New vault** (primary) and **Server settings**.
  - Corner bubbles: vault bubble (top left), "A" bubble (top right).
- **Phone:** the home screen and the bottom of the back stack.
- **Desktop:** **the dashboard does not exist at desk width.** It renders
  blank and forwards to the last-used vault's browse route. It appears only
  when there are no vaults. [FACT: dashboard.dart comments; `Storm Client`
  note, routing rule 4]
- **Status:** shipped (M7/M8, extended by decision 78).
- **Constraint:** the forward's stale-location read is "load-bearing, not a
  bug to tidy". It is what makes system back at desk width leave the vault
  (decision 78; `notesHome()`). Changing desktop home behaviour is therefore
  also a back-navigation decision.

### 6.2 Vault browsing (Directory), `/v/:vault/browse[/path]` [FACT: `ui/browse_screen.dart`, `ui/shell/vault_sidebar.dart`]

- **Phone:** a drill-down list, one folder at a time, with breadcrumbs.
  "Trees compress badly at phone width."
- **Desktop:** an expandable **folder tree** in the sidebar, with the pane
  beside it showing "Select a note". The two share row widgets and data,
  not one widget (decision 32).
- **Actions:**
  - Open a folder or note.
  - Long-press for rename, delete and move.
  - New note, new folder.
  - Pinned notes show a marker.
- **Constraint:** the folder path *is* the URL. Deep links reveal the open
  note in the tree.

### 6.3 Note (read / edit / properties / mentions), `/v/:vault/note/:id` [FACT: `ui/note_screen.dart`, `note_properties.dart`, `mentions_section.dart`, `note_mode_toggle.dart`, `editor/`]

- **Objects:**
  - Markdown body, read or edit.
  - Version line ("v51 · Saved").
  - Properties (typed frontmatter, colour swatches, tags).
  - Linked mentions, collapsed, at the end of the scroll.
  - Attachments strip.
- **Actions:**
  - Read / Edit toggle (Read mode is a client setting).
  - Formatting toolbar above the keyboard.
  - Wikilink autocomplete.
  - Find in note.
  - Note actions: pin, attach, rename/move, delete. Reached through a header
    "Note actions" button and long-press. Decision 42 originally made these
    long-press only; the current header carries a "Note actions" button.
  - Properties drawer: a sheet on phone, a 280-ratio right drawer with a
    narrow rail on desktop.
- **Phone:** the header has back, folder path and actions. No app bar
  (decision 40). The pill shows a **Mentions** badge with a count.
- **Desktop:** three columns: sidebar, note at a 640px measure, properties
  drawer (see `desktop-wide.png`).
- **Status:** the most mature surface (M0–M18).
- **Constraints:** the editor dims Markdown markers rather than hiding them,
  and cannot render inline images (`editor-findings.md`, decision 5).
  Conflicts are written into the note with markers (decision 6). No trash:
  delete is final (decision 38a).

### 6.4 Search, `/v/:vault/search` [FACT: `ui/search_screen.dart`, `search_panel.dart`]

- Full-text search **within one vault**, drawn as a sheet over the screen it
  came from, and still a route (decision 41).
- **Entry:** the pill Search slot (phone), the sidebar field (desktop), ⌘K.
- **[FACT] There is no cross-vault search** in the client or in MCP. Every
  MCP `search` takes one vault.

### 6.5 Tags, `/v/:vault/tags` [FACT: `ui/tags_screen.dart`]

- A tag browser sheet, per vault. Tags group on the first path segment
  (*Tag Taxonomy*).

### 6.6 Vault switching [FACT: `corner_bubbles.dart` `VaultBubble`; `vault_sidebar.dart` `_VaultSwitcher`]

- **Phone:** the top-left bubble shows the active vault's initial and a sync
  dot. Its popover lists:
  1. vaults, with note count and status;
  2. the sync line ("Synced 2m ago", "Offline · 3 edits queued", "Relayed ·
     …", "Server identity failed");
  3. the server host;
  4. **Sync now**;
  5. **Server settings ›**.
- **Desktop:** the vault name at the top of the sidebar opens a popover with
  the vault list, **Sync now** and **Server settings ›**. There is
  deliberately no "All vaults" entry, because there is no desktop dashboard.
- **Note:** only the *active* vault has a sync engine. Other vaults show
  "offline" dots by construction (dashboard.dart `_VaultCard`).

### 6.7 Client settings [FACT: `ui/client_settings_screen.dart`, `corner_bubbles.dart` `ClientSettingsBody`]

- **Sections:**
  - **Appearance:** theme preset (Storm dark, Storm light, SlowFlow earth),
    text size, note font.
  - **Notes:** Read mode, Show note id.
  - **Connection:** Add a device, Sign out, Disconnect ("Forget this server
    and its token").
  - **About:** version.
- **Phone:** a **popover** from the top-right "**A**" bubble ("Settings
  today, account when multi-user ships").
- **Desktop:** a **page** in the pane, from the sidebar footer gear
  (`/v/:vault/settings/client`). A popover anchored at the foot of the
  window opened off-screen.
- **[FACT]** Only mounted inside a vault on desktop. On the phone dashboard
  it is the bubble popover.

### 6.8 Server settings, `/settings/server` and `/v/:vault/settings/server` [FACT: `ui/server_settings_screen.dart`]

A Material `AppBar` page. Sections, in order:

1. **Vault storage root.** Root path, vault count, **Change** (with an "it
   doesn't move files" warning, and refusal if it would orphan vaults).
2. **AI access (MCP).**
   - "Let AI assistants read your notes" ("Serving at /mcp").
   - "Allow it to create, edit and delete", with a no-trash warning.
3. **Accounts.** "Allow new accounts" (registration).
4. **MCP keys.** A button to `/settings/mcp-keys`. Sign-in required.
5. **Agents** (owner only). "The machines agents run on, and which agent
   starts by default. Sessions are on the dashboard." A button to **Hosts**.
6. **Integrations** (owner only, `staging`). "Services your agents can use,
   like Notion or GitHub…" A button to **Integrations**.
7. **Vaults.** Rows with Rename / Remove from Storm ("its directory and
   notes stay"), and **New vault**.

- **Entry:**
  - Phone: vault bubble popover; the dashboard pill's server glyph.
  - Desktop: vault switcher popover.
- **Mounted twice:** inside the vault shell (desktop keeps the sidebar) and
  at top level (reachable with no vault).
- **Access:** every `PUT /v1/config*` is owner-only (decision 80), but the
  screen itself is shown to members. A member's writes are refused with
  403.

### 6.9 MCP keys, `/settings/mcp-keys` [FACT: `ui/mcp_keys_screen.dart`]

- Per **account**: list, **New key** (named), revoke. The secret is shown
  once in a deliberately hard-to-dismiss dialog.
- Top-level route, **outside every shell**. On desktop the sidebar
  disappears.

### 6.10 Agents space, `/agents` [FACT: `agent/agents_screen.dart`, `agents_shell.dart`, `agents_band.dart`; decision 78; *Client UX*]

- **Owner only.** The router redirects non-owners to the dashboard, and every
  entry point is hidden (AC-S1).
- **Phone:**
  - **Agents band** on the dashboard: up to N live sessions as rows
    (provider mark, session name, host, status, age), and "All ›". When idle,
    one row follows setup progress: *Set up a host* → *Your hosts are
    offline* → *No agents running · New session*. The band "always sits in
    the same place".
  - **Agents screen:** an AppBar "Agents" with a Hosts icon, a list
    (**Running**, then **Ended**) and a **New session** pill.
  - **Open session:** fills the screen, with a status chip, End or Dismiss,
    a tab-count switcher button, and an **extra-keys row** (Esc, Tab, sticky
    Ctrl and Shift, arrows, Paste, Done) while the keyboard is up. Android
    back returns to the list.
- **Desktop:**
  - The **Notes | Agents** segmented switch tops the sidebar.
  - The Agents sidebar lists sessions where the tree would be, with
    **New session** and the Hosts icon at the foot.
  - The pane shows a **tab strip** over the terminal, or "No session open".
- **Launcher** (a sheet), in order:
  1. **Host**, preselecting the last used and online one.
  2. **Workspace**, warning "Another session is already working here".
  3. **Provider**, default preselected.
  4. **Allow vault writes** toggle (non-shell; off by default; "Needs MCP
     writes on in Server settings").
  5. Network line: "Network: inherits <host>'s policy", plus "Agents here
     can use your integrations and read your vaults, with that network
     access."
  6. **Launch**.

  Any fallback is announced: "Claude Code isn't installed on build-vm —
  using OpenCode".
- **Freshness:** reloads on appear, pull-to-refresh, and every 15s while
  visible. Offline is a calm state ("Agents need the server").
- **Status:** shipped in v0.3.0, accepted 2026-10-05.
- **Not built** (deferred in decision 78): sessions as routes (no
  per-session deep link), switch keyboard shortcuts, a shared sidebar frame
  (⌘\ collapse doesn't apply on the Agents side).

### 6.11 Hosts, `/agents/hosts` [FACT: `agent/hosts_screen.dart`]

- **Shows:**
  - Hosts with a status chip (Online / Offline / Revoked), last seen,
    providers, and "Network: inherits this host's policy".
  - The **Default provider** selector.
- **Actions:**
  - **Enroll a host:** a one-time string shown once, with the `storm-runtime
    enroll` instructions.
  - Rename, Revoke.
- **Entry:**
  - Server settings ▸ Agents ▸ Hosts.
  - The Agents screen AppBar icon (phone).
  - The Agents sidebar footer (desktop).
  - The band's "Set up a host" row.
- **Mounted inside AgentsShell.** On desktop, arriving from Server settings
  swaps the sidebar from Notes to Agents.

### 6.12 Integrations, `/settings/integrations` (`staging` only) [FACT: `agent/integrations_screen.dart`; PLAN 81h, 81k, 81n; *MCP Gateway V1 Specification* §14]

- **Owner only**, by the server's 403. The entry is shown only when the
  owner check passes.
- **Intro states the blast radius:** "Connect a service once, here. Every
  agent session except a shell can then use it … anything an agent can read,
  it can send elsewhere."
- **List:** the built-in "Storm vaults" first ("Your vaults, read only. Turn
  on MCP writes to let a session allow writing."), then each connection with
  slug, auth kind, status label and last error.
- **Row actions:** Test, Choose tools (new tools start off), Replace token or
  Sign in again, Enable/Disable, Disconnect.
- **Add integration** (a Material extended FAB):
  - Name, https URL.
  - Mode: Sign in (OAuth), Token, or None.
  - Web: no OAuth, and the dialog says so.
  - GitHub PAT advice.
- **Entry: Server settings ▸ Integrations only.** Also opened automatically
  when an OAuth redirect arrives for an app that was killed (`main.dart`
  `_showIntegrations`).
- **Top-level route outside every shell.** On desktop the sidebar
  disappears. Its back button falls back to `/settings/server`.

### 6.13 Authentication and pairing [FACT: `router.dart` redirect; `pairing_screen.dart`, `login_screen.dart`, `signup_screen.dart`, `add_device_screen.dart`, `scan_pairing_screen.dart`, `starting_screen.dart`]

- **First run:**
  1. Scan or paste a `storm://pair` QR.
  2. Verify the server identity.
  3. Create the owner account.
  4. Sign in.
- **Web:** bootstraps its own device from a nonce in the served page, so
  there is no QR. If the bootstrap fails, it falls back to pairing.
- **Paired but no session:** `/login`, an account picker plus password.
  Optional **Create an account** when registration is open. "Use a different
  server".
- **Add a device:** a QR plus a copyable URI, from Client settings.
- **Session renewal:** on app resume; a lapsed session goes to `/login`.
- **Disconnect:** forgets the server. Sign out: stays paired.

### 6.14 Gallery, `/gallery` [FACT]

- Every shared widget in all three themes. Not linked from the app; a
  design-judging surface.

### 6.15 Surfaces that do not exist

These are gaps worth knowing, not recommendations.

- **No accounts / users management UI.** Only the registration toggle. Users
  are added with `storm-server user add`; there is no disable or role change
  from the app. [FACT: grep finds only the login picker's `GET /v1/users`]
- **No devices or auth-sessions list.** "Client: sessions screen
  improvements" is open in *Global Todo*.
- **No relay settings screen.** *Global Todo*: "Relay settings screen in the
  client"; "Still no client screen to set the relay list".
- **No Kit UI.** Roles, scripts and skills are browsable only as ordinary
  notes in the `kit` vault.
- **No cross-vault search, no global "everything" view, no notifications,
  no activity feed.**
- **No surface for what an agent did to the knowledge.** Agent writes carry
  `source: ai` frontmatter by convention and merge like any device. There is
  no filter or view for them.
- **No version-compatibility surface.** A client too old for its server
  "hangs on load or blames the network" (*Global Todo* backlog).
- **No in-app update path** (*Global Todo*, parking lot).

---

## 7. Current Information Architecture

Descriptive only. Problems follow in §11–§12.

### 7.1 Route tree [FACT: `router.dart`]

```
/starting                 waiting room (auth state loading)
/pairing  /login  /signup auth (redirect-driven)
/add-device               top level, no shell
/settings/mcp-keys        top level, no shell
/gallery                  unlinked
/                         Dashboard (phone) | forwards to last vault (desktop)
├── settings/server              Server settings (no shell)
├── settings/integrations        Integrations (no shell, owner)
├── [AgentsShell]                owner only
│   └── agents                   sessions list / tabs / terminal
│       └── hosts                Hosts + default provider
└── [VaultShell + VaultGate]     makes :vault active
    ├── v/:vault/browse[/path]
    ├── v/:vault/note/:id
    ├── v/:vault/search          (sheet)
    ├── v/:vault/tags            (sheet)
    ├── v/:vault/settings/server (same screen, sidebar kept)
    └── v/:vault/settings/client (desktop page)
```

Everything is a child of `/`, so Android back always unwinds to the
dashboard (decision 17). Sessions are **not** routes.

### 7.2 Phone IA (as built)

```
DASHBOARD  ─ corner: [Vault bubble ▾]                       [A bubble ▾]
  │            ├ vault list → /v/x/browse (go)            Appearance · Notes ·
  │            ├ sync line · server host                   Connection (Add device,
  │            ├ Sync now                                  Sign out, Disconnect) · About
  │            └ Server settings ›
  ├ Agents band (owner) → session (terminal) | All › → /agents | Set up a host → Hosts
  ├ Recently opened (5, cross-vault) → note
  ├ Vaults grid → /v/x/browse ; long-press → colour
  └ pill: [+ New vault] [Server settings]

VAULT SCREENS (browse/note/search/tags) ─ same corner bubbles
  └ pill: Directory · Search · (+ New note) · New folder* · Mentions(badge) · Tags

AGENTS (/agents, AppBar)  ─ Hosts icon ; list Running/Ended ; pill [New session]
  └ session: full-screen terminal, chip, End, switcher, extra-keys row

SERVER SETTINGS (AppBar) → Storage root · AI access · Accounts · MCP keys →
                            Agents → Hosts · Integrations → · Vaults
```

### 7.3 Desktop IA (as built)

```
┌ Sidebar (Notes) ──────────────┐┌ Pane ───────────────────────┐┌ Drawer ┐
│ [Notes | Agents] (owner)      ││ browse: "Select a note"      ││ Props  │
│ [■ vault name ▾] → vault list,││ note: read/edit              ││        │
│   Sync now, Server settings › ││ search/tags sheets           ││        │
│ [Search…] → search sheet      ││ server settings (in pane)    ││        │
│ folder tree                   ││ client settings (in pane)    ││        │
│ footer: + note · + folder ·   ││                              ││        │
│   mentions · tags · ⚙ client  ││                              ││        │
└───────────────────────────────┘└──────────────────────────────┘└────────┘

┌ Sidebar (Agents) ─────────────┐┌ Pane ───────────────────────┐
│ [Notes | Agents]              ││ tab strip + terminal         │
│ sessions (Running / Ended)    ││ or Hosts                     │
│ footer: + New session · Hosts ││                              │
└───────────────────────────────┘└──────────────────────────────┘

Full-window, no sidebar:  Integrations · MCP keys · Add a device · /settings/server (from no-vault)
```

### 7.4 Global vs contextual (as built)

| Thing | Conceptual scope | Where the UI places it |
|---|---|---|
| Vault list / switching | Server | Vault bubble (phone), sidebar head (desktop), dashboard grid (phone) |
| Recents | Account, cross-vault | Phone dashboard only |
| Search | Per vault (by design) | Vault screens |
| Sync status / Sync now | Engine for the active vault | Vault bubble / switcher |
| Server settings | Server | Inside the vault switcher popover, and the dashboard pill |
| Client settings | Device | Top-right bubble (phone), sidebar gear (desktop) |
| MCP keys | Account | Server settings |
| Add device / Sign out | Account / device | Client settings ▸ Connection |
| Agents (sessions) | Owner, server | Dashboard band (phone), sidebar switch (desktop) |
| Hosts | Server infrastructure | Server settings *and* Agents space |
| Default provider | Server | Hosts screen |
| Integrations | Owner | Server settings |
| Vault-write permission for agents | Per launch, AND server switch | Launcher toggle, plus the Server settings MCP switch |
| Kit | Server (a vault) | Vault grid, like any vault |

### 7.5 How users move between areas [FACT]

- **Notes → Agents:**
  - Phone: back to the dashboard, then the band. There is no Agents entry in
    the vault pill or bubbles.
  - Desktop: the sidebar switch.
- **Agents → Notes:**
  - Phone: back.
  - Desktop: the switch goes to `notesHome()`, the last vault used.
- **Vault → vault:** the bubble or switcher popover (`go`, which replaces
  the location rather than stacking).
- **Anywhere → configuration:** through the vault bubble or switcher
  ("Server settings ›"), the dashboard pill (phone), or the A bubble or gear
  (client).
- **Discovering existing agents/sessions:**
  - Phone: dashboard band (live first, then "All ›").
  - Desktop: only after choosing the Agents side of the switch. Nothing on
    the Notes side indicates running sessions.

---

## 8. Desktop Experience

**[FACT]**
- **One breakpoint at 900px.** "The phone layout is the default;
  wide-screen behaviour is additive" (decision 34, `CLAUDE.md` invariant).
  Every adaptive test asserts both sides.
- **Column sizes are ratios** of a 1200px design frame (sidebar 260,
  drawer 280), floored and capped (`breakpoints.dart`). The editor measure
  is 640px.
- **Corner bubbles and the nav pill are hidden at desk width.** Their
  actions move to the sidebar head and footer, both built from the same
  `vaultActions()` data so they can't drift.
- **Keyboard shortcuts (M18):**
  - ⌘K search, ⌘N new note, ⌘⇧N new folder, ⌘\ toggle sidebar.
  - Note-level: read/edit, save, find, escape.
  - Editor: bold, italic.
  - All suppressed while a terminal has focus; Ctrl+C always belongs to the
    agent.
- **No desktop dashboard.** Launch lands in the last vault.
- **The desktop sidebar is two different shells** (`VaultSidebar`,
  `AgentsSidebar`) that are drawn alike. A shared frame is deferred.

**Platforms:**
- macOS: sandboxed; zip release; no notarisation.
- Web: served by the server itself; static-token-only Integrations; a
  service-worker cache.
- Linux: built for the runtime/gateway work (`oauth_flow_io` mentions Linux
  and Windows). The README calls the Linux desktop "deferred", and
  [INFERENCE] it is not a released client.

**[INFERENCE]** Desktop is where developers will run several sessions in
tabs beside their notes. The current desktop IA cannot show notes and a
terminal at the same time: the switch swaps the whole sidebar and pane.

---

## 9. Mobile Experience

**[FACT]**
- **Android** is the primary phone target; there is no iOS (decision 3).
- **Navigation grammar:**
  - Two **corner bubbles**, rounded squares: "places".
  - A floating **pill**: "actions", with one filled primary circle.
  - Screen-specific headers below the bubbles.
  - No app bar inside a vault (decision 40).
- **Keyboard-aware chrome:** the pill hides when the soft keyboard is up,
  and the formatting toolbar or terminal extra-keys row rides on the
  keyboard. The two are never on screen together (decision 11).
- **Sheets** for search, tags, mentions, the launcher and the session
  switcher.
- **Long-press** reveals secondary actions on rows (decision 42).
- **Agents on the phone:**
  - The dashboard band is the only entry.
  - An open session is edge-to-edge (commit `856a5d6`).
  - The extra-keys row matches the editor bar (commit `50db59a`).
  - Android back returns from session to list to dashboard.
  - Agent controls "must be reachable one-handed" (*Client UX*).
- **Offline:** notes are readable and editable from cache, with edits queued
  in the outbox. Agents require the server: "Agents need the server".
- **Native OAuth:**
  - Android and macOS: through `storm://oauth`.
  - Every other native platform: a loopback listener.
  - Android native OAuth acceptance passed 2026-10-08 (*Global Todo*).

**[INFERENCE]**
- The phone is explicitly the "control surface" in the Vision: the device
  you continue agent work from. Yet on the phone, Agents are one step
  further from a vault screen than on desktop.
- Phone settings are split across at least four entry shapes:
  1. the bubble popover (client);
  2. a dashboard pill glyph (server);
  3. the vault bubble's "Server settings ›";
  4. pages with Material AppBars.

---

## 10. Important User Flows

Each flow lists entry, steps, dependencies, friction, assumptions, unclear
navigation and future implications. "Friction" is an observation, not a
judgement about the right fix.

### F1. Open Storm → access knowledge

- **Entry:** cold launch.
- **Steps:** `/starting` → (auth redirect) →
  - phone: the dashboard (recents and vaults);
  - desktop: the last vault's browse pane with the tree.
- **Depends on:** a session and settings loaded. Recents need the server or
  the cache mirror.
- **Friction:**
  - On desktop the user lands in *a vault*, not in "their Storm". Recents
    (cross-vault) are unavailable on desktop.
  - The phone masthead's "last synced" reads "—" until this run syncs.
- **Future:** whatever "home" becomes must also carry agent state and
  possibly approvals and notifications (*Client UX* "Later: Overview").

### F2. Switch vault

- **Steps:**
  - Phone: vault bubble → popover → vault, or the dashboard grid.
  - Desktop: sidebar head → popover → vault.
- **Friction:**
  - The switcher also carries Sync now and **Server settings**, so a
    server-wide entry hides inside a vault control.
  - Only the active vault reports real sync status; the others show offline
    dots by construction.
  - The phone's vault bubble shows the *active* vault even on the
    cross-vault dashboard.
- **Future:** A9 will filter the vault list per user. Kit is listed as a
  peer of personal vaults.

### F3. Find / create / edit a note

- **Find:** tree or drill-down, per-vault search (⌘K or pill), tags,
  mentions, recents (phone).
- **Create:** the pill's primary **+** or ⌘N. The folder defaults to the
  current location, and a colour can be chosen up front.
- **Edit:**
  - Read/Edit toggle.
  - The properties list is the only way to edit frontmatter (decision 30).
  - Saves merge 3-way; conflicts show inline markers.
- **Friction:**
  - Finding a note whose vault you don't remember needs a vault-by-vault
    search.
  - Note actions are low-discoverability (long-press; decision 42 accepted
    the trade).
- **Assumptions:** the user knows the vault. Search is the fastest path.

### F4. Use Kit / reusable AI resources

- **As built:**
  - Open the `kit` vault like any other and read or edit role notes,
    `Project Architecture Guidelines`, skills and scripts.
  - External agents consume kit through MCP after adapter install
    (`kit/install/*.md`), or through `BOOTSTRAP.md`.
- **Friction:**
  - No in-app explanation of what Kit is or why it is special.
  - The script tools only work in the vault named `kit` (`mcp.rs`).
  - Deleting `kit` is allowed and not reversed (decision 64).
  - **Agent sessions launched from Storm do not use Kit roles.** The
    launcher picks a provider, not a role.
- **[OPEN]** Is Kit a vault, a library, a configuration layer for agents, or
  all three? Should launched sessions be able to start "as" a Kit role?

### F5. Find an existing agent

- **As built:** there are no persistent "agent" objects. The user finds
  **sessions**:
  - Phone: dashboard band → session, or "All ›".
  - Desktop: switch to Agents → sidebar list.
- **Friction:**
  - The word "Agents" leads to a list of sessions.
  - Ended sessions accumulate until individually dismissed.
  - Sessions can't be deep-linked (no route).
- **Future:** named agents and Agent Definitions (deferred), orchestration
  trees and tasks will each create a real "agent" or "task" object distinct
  from a session.

### F6. Start an agent session

- **Steps:**
  1. New session (band row, pill, or sidebar footer).
  2. Launcher: Host → Workspace → Provider → [Allow vault writes] → Launch.
  3. The terminal opens as the active tab.
- **Dependencies:**
  - An enrolled, online host with a resolvable provider, logged in on the
    host as `storm-runtime`, out of band.
  - Workspace directories created on the host, out of band ("There is no UI
    to create, clone or delete workspaces").
  - For vault writes: the server's MCP-writes switch.
  - For integrations: connections set up beforehand in Server settings.
- **Friction:**
  - Prerequisites are spread across Hosts (enroll), the host's shell
    (provider login, workspace dirs), Server settings (MCP writes) and
    Integrations.
  - The launcher shows consequences (egress, integrations) as text lines,
    not as choices.
- **Future:** Phase 2 adds per-session policies (egress, budgets,
  isolation). Per-workspace defaults are LATER in the gateway spec. Roles
  and orchestration add more launch-time parameters.

### F7. Return to an existing session

- **Steps:**
  - Phone: dashboard band → row → terminal; the switcher sheet for other
    tabs.
  - Desktop: Agents side → list or tab strip.
- **Behaviour:** the stream resumes by byte offset; a gap is explicit. Tabs
  are per device and reconciled with the server at launch.
- **Friction:**
  - Tabs on device A are not tabs on device B, by design (*V1 Freeze* §10).
  - There is no notification when a session completes or needs input; the
    user must come back and look.
- **Future:** notifications ("only state changes that need a human —
  approvals, failures, completion"); waiting and approval statuses are
  reserved (`waiting`, `waiting_on_approval`).

### F8. Move between Notes and Agents

- **Phone:** vault → back to the dashboard → band. Agents → back to the
  dashboard → vault.
- **Desktop:** the sidebar switch. The whole sidebar and pane swap, and only
  one is visible at a time.
- **Friction:**
  - The core relationship (Knowledge ↕ Agents) has no shared surface.
  - A note an agent is writing cannot sit beside its terminal.
- **[OPEN]** Should notes and sessions co-exist on screen (desktop)? Should
  a note show which sessions touched it?

### F9. Configure Storm / client settings

- **Phone:** the "A" bubble popover.
- **Desktop:** the sidebar gear → page in the pane.
- **Friction:**
  - The "A" bubble label doesn't say settings.
  - Account actions (sign out, add device) sit in a device-settings menu.
  - On the phone dashboard and in vaults the same popover appears, but on
    the Agents screen (AppBar) there is no settings entry.

### F10. Configure server settings

- **Entry:** the vault bubble or switcher → "Server settings ›", or the
  dashboard pill.
- **Friction:**
  - One long page mixes seven domains (§6.8).
  - Members see controls they can't change (403 on write).
  - MCP keys (personal) and Integrations (owner's third-party credentials)
    live beside storage-root administration.
- **Future:** the relay list, users and roles (A9), devices, backups,
  version and update, Phase 2 policies and audit could all plausibly be
  "server settings". There is no stated rule for what goes there.

### F11. Add an MCP integration

- **Steps:**
  1. Server settings.
  2. Scroll to Integrations → Integrations.
  3. Add integration (FAB).
  4. Name + URL + mode.
  5. Sign in (the browser opens; loopback or `storm://oauth`), Token, or
     None.
  6. Test.
  7. Choose tools.
- **Friction:**
  - Reached via the vault switcher → Server settings → scroll.
  - On desktop, the sidebar disappears on entry.
  - Web can't do OAuth.
  - A killed-app OAuth return auto-navigates here.
- **Assumptions:** the owner knows the service's MCP URL. There is no
  catalogue, explicitly out of scope.
- **Future:** per-workspace defaults, admin-shared connections (after A9),
  device-code grants, notifications when new tools appear (81k is a
  snackbar plus "off until reviewed").

### F12. Use an integration from an agent

- **As built:**
  - Every non-shell session launched after the connection was added gets
    all of the owner's connections plus the built-in `storm` connection,
    automatically.
  - The agent CLI prompts before each tool (Claude Code natively; OpenCode
    because Storm writes `ask`).
  - All of this happens **inside the terminal**. Storm's UI shows nothing
    per call: audit is server-side metadata, with no UI.
- **Friction:**
  - The user cannot see from Storm which integrations a session has, or
    what it called.
  - Grants are fixed at launch, so adding an integration mid-session needs
    a new session.
  - A provider's "always allow" persists in the agent's own settings
    (accepted risk 5).
- **Future:** Observability, approvals queue, per-session grants UI.

### F13. Manage Runtime Hosts

- **Steps:**
  1. Hosts (from Settings or the Agents space).
  2. Enroll a host: copy the one-time string.
  3. On the host, run `storm-runtime enroll` and paste the string.
  4. The host appears online.
  5. Rename, Revoke, set the default provider.
- **Friction:**
  - Half the flow is a terminal on another machine.
  - Provider availability is shown but not fixable from the UI.
  - Workspaces are not managed in the UI.
- **Future:** resource metrics (Phase 2), Cloud-operated hosts ("the client
  must not need to know which one it talks to"), host-over-relay.

### F14. Use Storm on mobile

Covered across F1–F13. The phone-specific points:
- The dashboard is the hub for both spaces.
- Agents are dashboard-only.
- Settings are reached through popovers plus AppBar pages.
- The terminal gets the extra-keys row.
- OAuth sign-in for Integrations works natively.

### F15. Disconnect / reconnect

- **Notes:**
  - Offline is inferred from request success.
  - The vault bubble dot and line say "Offline · showing your cached copy".
  - Edits queue in the outbox; reconnect uses 1→60s backoff.
  - "Server identity failed · not syncing" is distinct from offline.
  - "Relayed" is its own status.
- **Agents:**
  - "Agents need the server".
  - Host offline means sessions go `unknown` and input is refused (503).
- **Auth:**
  - On resume, sessions are renewed or the user is sent to `/login`.
  - "Disconnect" forgets the server entirely; "Use a different server" on
    login.
- **Friction:**
  - Connection state is spread across the vault bubble (notes), the band
    and Agents (agents), and Hosts (hosts).
  - There is no single server-health view.
  - Client/server version mismatch is invisible (backlog).

---

## 11. Current UX / IA Problems

Severity scale: **High**, meaning it blocks or seriously distorts a core job,
or worsens with the next features; **Medium**; **Low**.

### P1. There is no global level; the only home is phone-only

- **Problem:** cross-cutting information (recents across vaults, running
  agents) lives on the dashboard, and the dashboard does not exist at desk
  width.
- **Evidence:** `dashboard.dart` (desk-width forward; `_Recents`,
  `AgentsBand` used only there); grep shows `recentsProvider` and
  `AgentsBand` consumed only by the dashboard.
- **Why it matters:**
  - Desktop users lose recents entirely.
  - Desktop users see nothing about running agents unless they switch
    spaces.
  - The Vision's "control surface" and "observe everything" have no home.
- **Affected:** all desktop users; owners with agents.
- **Likely IA issue:** "home" was defined as "a vault picker", which a
  sidebar replaces on desktop. It was never defined as the server-level
  overview.
- **Severity:** High. **Confidence:** High.

### P2. Server settings is a catch-all mixing four kinds of thing

- **Problem:** one page holds:
  - infrastructure: storage root;
  - security policy: MCP switches, registration;
  - personal credentials: MCP keys;
  - agent infrastructure: Hosts and default provider;
  - third-party services: Integrations;
  - workspace content: the vault list and New vault.
- **Evidence:** `server_settings_screen.dart` sections; screenshot
  `phone-server-settings.png`.
- **Why it matters:**
  - Each new feature has defaulted to a new section.
  - Members see an admin page whose writes 403.
  - Personal items are hidden under "Server".
- **Affected:** owners (finding things), members (confusion).
- **Likely IA issue:** "settings" is being used as the overflow bin for
  anything that isn't a note.
- **Severity:** High. **Confidence:** High.

### P3. Integrations are buried and leave the shell

- **Problem:** Integrations, a product capability central to the gateway
  thesis, are reached only via the vault switcher → Server settings →
  scroll → Integrations. They render full-window, outside every shell.
- **Evidence:** `router.dart` (`settings/integrations` outside the
  ShellRoutes); `server_settings_screen.dart` (the only entry);
  `main.dart` (auto-navigation on an OAuth orphan).
- **Why it matters:**
  - The feature is hard to find.
  - The desktop sidebar vanishes on entry.
  - Integrations relate to *agents*, not to vault storage, yet the path to
    them runs through a vault control.
- **Affected:** owners.
- **Likely IA issue:** the configuration location was chosen by analogy
  ("connect once, here") without a home for agent-related configuration.
- **Severity:** High. **Confidence:** High.

### P4. "Agent" has no object; the Agents space is a sessions list

- **Problem:** users meet "Agents" (the space), "agents" (Kit roles) and
  "agents" (Claude Code), while the only manipulable object is a
  **session**.
- **Evidence:** §5.1; *V1 Freeze* §6 ("Users cannot define named agents in
  V1"); `kit/vault/agents/`.
- **Why it matters:**
  - Future named agents, roles and orchestration will need the word
    "agent" for a real object.
  - The current label is already spent on a list of sessions.
- **Affected:** owners; every future agent feature.
- **Likely IA issue:** the space is named for the activity, not the object
  (decision 78's explicit choice: "'Agents' is an activity").
- **Severity:** High (it is structural for future work). **Confidence:**
  High.

### P5. Kit is foundational in intent but invisible in the UI

- **Problem:** Kit is "core … every Storm server has one" (decision 64), yet
  it appears as an ordinary vault card. Its roles are unconnected to the
  launcher, and its scripts are MCP-only.
- **Evidence:** `apps/server/src/kit.rs`; `mcp.rs` (scripts restricted to
  kit); no client references to kit.
- **Why it matters:** the "reusable AI resources" layer has no surface. It
  competes visually with personal vaults, and nothing explains it.
- **Affected:** owners using agents.
- **Likely IA issue:** Kit was implemented as content, with no product
  concept around it.
- **Severity:** Medium. Could become High once roles or agent definitions
  arrive. **Confidence:** High.

### P6. Phone and desktop expose different top-level capabilities

- **Problem:**

  | | Phone | Desktop |
  |---|---|---|
  | Recents | Yes | No |
  | Agents entry | Dashboard band only | Sidebar switch only |
  | Client settings | Popover | Page |
  | Server settings | Pill or bubble | Switcher |
  | Collapse (⌘\\) | — | Works only on the Notes side |

- **Evidence:** §7.2 and §7.3.
- **Why it matters:** the Vision's cross-device continuity ("start on the
  laptop, finish on the phone") is undermined when the two devices organise
  the product differently.
- **Affected:** everyone using more than one device. That is Storm's
  premise.
- **Likely IA issue:** "phone default, wide additive" was applied to
  *layout* and has drifted into *structure*.
- **Severity:** Medium–High. **Confidence:** High.

### P7. Global controls look contextual

- **Problem:**
  - Server settings appears inside the vault switcher.
  - The phone vault bubble sits on the cross-vault dashboard and shows the
    active vault.
  - "Sync now" sits in the vault menu.
- **Evidence:** `corner_bubbles.dart`, `vault_sidebar.dart` (popover
  contents). The code comment says "Server settings are in the vault
  switcher, next to the vault they configure", but the page configures the
  whole server.
- **Why it matters:** users can't predict scope. Is this setting for this
  vault, this device, my account, or the server?
- **Severity:** Medium. **Confidence:** High.

### P8. "MCP" is four things, and the switches couple them

- **Problem:**
  - Agent vault writes require the server "MCP writes" switch. Its copy is
    about external "AI assistants", and it says "create, edit and delete",
    but agents never get delete.
  - Integrations (the MCP gateway) are presented apart from "AI access
    (MCP)".
  - Whether gateway *reads* depend on the "Let AI assistants read your
    notes" switch is not visible in the client. [OPEN: the spec gates
    writes on `mcp_writable`; read dependence on `mcp_enabled` was not
    determined in this pass.]
- **Evidence:** the launcher subtitle "Needs MCP writes on in Server
  settings"; the integrations built-in row copy; G-D5; Server settings
  copy.
- **Why it matters:** the security model is good, but its controls are
  scattered across three screens with inconsistent language. That is a
  trust problem for exactly the feature with the largest accepted blast
  radius (*MCP Gateway* §18 risk 1).
- **Severity:** High. **Confidence:** High.

### P9. Two visual grammars

- **Problem:** the notes surfaces use the bespoke bubbles/pill/sheet
  grammar. Settings, Agents, Hosts, Integrations, MCP keys and Add-device
  use Material AppBars, OutlinedButtons, SwitchListTiles and
  (Integrations) a Material FAB. Decision 78 replaced the FAB on Agents
  "because the space is a peer of the vault screens"; Integrations
  reintroduced one.
- **Evidence:** the files cited; decision 78; `phone-server-settings.png`.
- **Why it matters:** the product reads as a polished notes app with admin
  screens bolted on. This is the visual symptom of P2 and P3.
- **Severity:** Medium. **Confidence:** High.

### P10. Hosts are double-mounted, and entering from Settings switches space

- **Problem:** Hosts lives in the Agents shell. From Server settings (Notes
  side, desktop) it swaps the sidebar to Agents.
- **Evidence:** `router.dart` (`agents/hosts` in `AgentsShell`);
  `server_settings_screen.dart` (`push(Routes.agentHosts)`).
- **Why it matters:** Hosts is infrastructure *and* part of the agent
  workflow. The IA hasn't decided which, so it is both.
- **Severity:** Low–Medium. **Confidence:** High.

### P11. Sessions are not addressable, and tabs are device-local

- **Problem:**
  - No session route or deep link.
  - Tabs are per device.
  - Ended sessions persist until dismissed.
  - The list mixes running and ended.
- **Evidence:** decision 78 ("Not in this pass: per-session deep link");
  *V1 Freeze* §7.4 and §10.
- **Why it matters:**
  - Notifications, links from notes to sessions, and orchestration trees all
    need addressable sessions.
  - Continuity across devices relies on the list, not on "where I was".
- **Severity:** Medium (it grows with every agent feature).
  **Confidence:** High.

### P12. Knowledge ↔ agent relationship has no surface

- **Problem:**
  - Notes don't show agent provenance beyond the raw `source: ai` property.
  - Sessions don't show what they read or wrote.
  - No view combines a note and a session.
- **Evidence:** *Vision* ("the relationship that matters"); absence in the
  code; *Shared Knowledge* open question (staged vs direct writes).
- **Why it matters:** this relationship *is* the product thesis. The UI
  presents two disconnected apps.
- **Severity:** High for the thesis; Medium for current use.
  **Confidence:** Medium. It is a gap rather than a defect, but the docs
  make it central.

### P13. Account, device and server-admin surfaces are missing or misfiled

- **Problem:**
  - No users, roles or devices UI.
  - MCP keys (per account) sit under Server.
  - Sign out and Add device sit under Client (device) settings.
  - There is no relay screen.
- **Evidence:** §6.15; *Global Todo*.
- **Why it matters:** A9 (roles, grants) and the relay will need homes, and
  the current split doesn't separate device, account and server.
- **Severity:** Medium. **Confidence:** High.

### P14. The two-way space switch doesn't scale

- **Problem:** "Notes | Agents" is a binary toggle, owner-gated. Decision
  78's own revisit trigger: "a third space appears, at which point the
  switch wants to become a real space picker".
- **Evidence:** `space_switch.dart`; decision 78.
- **Why it matters:** the roadmap names Workspace (files, Git, shell),
  Orchestration (tasks, reviews), Approvals, Overview and Observability.
  Several are space-sized.
- **Severity:** High (future). **Confidence:** High.

### P15. Owner-only gating hides the IA's shape from members, and vice versa

- **Problem:** a member's app is the pre-agents app. An owner's app adds a
  band, a switch and settings sections. The IA has two variants decided by
  one boolean.
- **Evidence:** `agentAccessProvider` gates; AC-S1; A9 pending.
- **Why it matters:** A9 may give admins or members partial agent or
  integration access, and the IA would need graded visibility.
- **Severity:** Medium. **Confidence:** Medium.

### P16. Search, recents and status don't scale across vaults

- **Problem:**
  - Search is per vault.
  - Recents cap at 5 on the phone dashboard.
  - Sync status is only real for the active vault.
  - There is no "all content" view.
- **Evidence:** `dashboard.dart` (`limit = 5`); MCP `search` is per vault;
  the `_VaultCard` status logic.
- **Why it matters:** this was fine with one vault (PRD v1: "one vault per
  app install"). The server already hosts four, kit included, and agents
  will create more content.
- **Severity:** Medium. **Confidence:** High.

### P17. The dashboard's "+" means New vault

- **Problem:** the primary action on the home screen creates a rare,
  server-level object, while everywhere else "+" creates a note.
- **Evidence:** `vault_actions.dart` (`New vault`, primary);
  `dashboard.dart`.
- **Why it matters:** the most prominent action on home is the least
  frequent.
- **Severity:** Low. **Confidence:** High.

### P18. Infrastructure prerequisites surface as dead ends inside product flows

- **Problem:** the agent launcher depends on:
  - host enrollment, done partly in a terminal;
  - provider login on the host;
  - workspace directories created on the host;
  - the server MCP-writes switch;
  - Integrations set up beforehand.

  The UI states these as text ("Start storm-runtime…", "Add a directory
  under one of…") without a path to fix them.
- **Evidence:** `agents_screen.dart` launcher strings; `hosts_screen.dart`;
  *V1 Freeze* §8.
- **Why it matters:** first-run of the core agent job crosses three
  machines and four screens.
- **Severity:** Medium. Partly inherent to self-hosting. **Confidence:**
  High.

### P19. Status and health information is fragmented

- **Problem:** health is split across:
  - sync (vault bubble dot and line);
  - relayed vs direct (a dot colour);
  - server identity failure (a dot state);
  - hosts (chips in Hosts);
  - sessions (chips);
  - integrations (row status labels);
  - version mismatch (nowhere).
- **Why it matters:** the Vision's "observability" and the self-hosted
  operator's needs ("is my Storm OK?") have no shared place.
- **Severity:** Medium. **Confidence:** High.

### P20. The Mentions slot is always present

- **Problem:** outside a note, the pill's Mentions slot opens a sheet saying
  "Open a note to see what links to it".
- **Evidence:** `vault_actions.dart` ("Six slots, always").
- **Severity:** Low. This is a deliberate trade for a stable pill.
  **Confidence:** High.

---

## 12. Navigation and Hierarchy Problems (summary)

This section summarises §11 by structural cause.

1. **Missing levels.** The hierarchy as built is Vault → Folder → Note, plus
   a sibling Agents space. There is no explicit **server/workspace** level
   above vaults and agents, and no **account** level. Their contents
   (settings, keys, hosts, integrations, recents) are scattered into the
   vault level. (P1, P2, P7, P13)
2. **Activity-vs-object naming.** Spaces are named by activity ("Notes",
   "Agents"). Their contents are objects of different kinds: vaults and
   notes; sessions. Future objects (agents, tasks, workspaces, reviews)
   have no slot. (P4, P14)
3. **Settings as the overflow.** Every non-note capability after M13 went
   into Server settings first. (P2, P3, P8)
4. **Layout-driven structure.** The phone and desktop structures differ
   because components (dashboard, bubbles, pill) were removed at desk width
   rather than re-homed. (P1, P6)
5. **Dead ends and exits from shells.** Integrations, MCP keys and Add
   device leave the shell on desktop. The Agents AppBar has no settings
   entry. (P3, P9)
6. **Back-stack coupling.** Desktop home behaviour and system-back behaviour
   are coupled through the dashboard's stale-read forward (decision 78).
   Any change to "home" must be designed with back navigation.
   [CONSTRAINT, technical]

---

## 13. Settings / Configuration Model

### 13.1 Where configuration actually lives [FACT]

| Scope | Examples | Stored | Changed from |
|---|---|---|---|
| **Device** | Theme, text size, note font, read mode, show note id, pinned notes, tabs, last-used host | Client prefs / cache | Client settings; pins on notes; tabs implicitly |
| **Account** | Password, MCP keys, devices, auth sessions | `state/auth.db` | MCP keys screen; sign out / disconnect. **No device list.** No password-change screen was found in this pass (the A9 matrix lists "change own password" as a role right). |
| **Vault** | Name, colour (`storm.color` in `_storm/vault.md`), property types | Registry + in-vault `_storm/` | Server settings (rename/remove); dashboard long-press (colour); properties |
| **Server (owner)** | Storage root, `mcp_enabled`, `mcp_writable`, registration, relay list, default provider | `state/vaults.json`, `state/…`, `state/agent/config.json` | Server settings; Hosts (default provider); **relay: no client UI** |
| **Owner's integrations** | Connections, credentials, tool allowlists | `state/gateway/gateway.db` (encrypted) | Integrations |
| **Per launch** | Host, workspace, provider, allow vault writes | `agent.db` session record and grants | Launcher |
| **Runtime Host (operator)** | Workspace roots, providers, `max_sessions`, scrollback, provider secrets | `/etc/storm-runtime/runtime.toml`, env files | **On the host only** |
| **Server process (operator)** | Data root, units, backups | `storm.env`, systemd | CLI (`storm-server up/pair/user`) |

### 13.2 Principles stated in the docs

- [INTENT] "Settings keep configuration only" (decision 78). Sessions are
  done, not configured.
- [INTENT] Client vs server: "The distinction that matters to the user is
  which side of the wire a setting lives on" (`client_settings_screen.dart`
  doc comment). A server switch's subtitle must say it affects every client
  (`_McpCard` comment).
- [CONSTRAINT] "A setting that does not survive a restart is not a setting"
  (`CLAUDE.md`).
- [CONSTRAINT] Server-wide settings are owner-only (decision 80). The UI
  must never infer a role; the server's 403 decides (decision 77d,
  "the 77d rule").
- [CONSTRAINT] Configuration is split by plane: the server holds
  control-plane config, each host holds execution-plane config, and
  "invalid config surfaces in the client as a visible error, not a log
  line" (*Configuration*).
- [CONSTRAINT] Provider (model) secrets never enter Storm. Integration
  credentials do, encrypted, and never leave the server.

### 13.3 Observed gaps

- [PROBLEM] There is no explicit **account** settings home (see P13).
- [PROBLEM] Operator-only config (`runtime.toml`) is invisible in the
  client, though its effects (providers, workspaces) drive the launcher.
- [OPEN] Should the client ever *edit* host-side config, or only display
  it? The spec places it with the operator.

---

## 14. Agents / Sessions Model

### 14.1 Objects and relationships [FACT: *V1 Specification Freeze* §4–§9]

```
Owner ─owns─▶ Agent session (ags_) ─runs on─▶ Runtime Host (hst_)
                 │  provider (claude-code | opencode | shell; kind=cli)
                 │  workspace (host_id, name)  ← dir under host root
                 │  interaction = terminal (PTY on host)
                 │  grants: all owner connections + storm (non-shell), allow_vault_writes
                 └─ viewed through: Tab (per device)
```

### 14.2 Status vocabulary (one set; "Each state is drawn differently") [CONSTRAINT]

```
creating → starting → running → completed | stopped | failed
                    └── unknown  (host unreachable; reconciles)
```

- `failed` shows an `end_reason`: `host_restart`, `host_revoked`, `lost`,
  start failure, or a signal.
- Reserved for later: `waiting`, `waiting_on_approval`, `paused`,
  `interrupted`, `idle`.
- `crashed` and `restarting` are removed.
- Nothing auto-restarts (D5). A session never migrates hosts.

### 14.3 Behavioural rules a design must respect [CONSTRAINT]

- **Ending a session needs confirmation.** "The agent and everything it
  started are stopped on the host."
- **Ended sessions stay listed until dismissed.** Dismissal is allowed only
  once ended.
- **Several clients may attach to one session and all may type.** The
  last-active client sets the PTY size.
- **Input is at-most-once and never retried.** It is refused while the host
  is offline: "Inline error, no retry. Re-type."
- **Fallbacks are announced, never silent.**
- **Shared workspaces warn, they don't block.**
- **Ctrl+C belongs to the agent** while the terminal is focused.
- **Tabs are views.** Closing one doesn't end the session.

### 14.4 Principles [INTENT: *Client UX*]

- "Agents are collaborators, not background tasks. Each visible session has
  a provider, a host, a status, and a place where its work is visible."
- "Everything an agent does is inspectable. No 'the agent is thinking'
  spinner without a trace. In V1 the trace is the terminal."
- "Humans stay in control. … The UI must make the current control point
  obvious."

### 14.5 Session vs agent vs task [OPEN]

The product model distinguishes four things:
- **Agent Definition:** V1 is a provider entry; named agents later.
- **Agent Session:** machinery.
- **Agent Task** (later): "what the agent was asked to do; a session may
  carry several tasks; a task may span sessions".
- **Agent Relationship** (later): parent/child.

The current UI only has sessions. The IA should leave room for all four
without renaming everything (see P4).

---

## 15. Vault / Notes / Kit Model

### 15.1 Vaults [FACT: decisions 20–25; `CLAUDE.md`]

- One server hosts many vaults.
- A vault is a directory under the storage root, identified by UUID.
- A vault can be `missing`. It is shown greyed, never hidden: "One that
  vanished from the list would look exactly like one that never existed".
- Each vault has its own index and sync cursor. The change stream is
  global.
- Recents are server-side and cross-vault.
- The active vault is routed (`/v/:vault/…`) with a persisted mirror.
  `VaultGate` blocks a stale frame.
- Removing a vault only unregisters it. Storm never moves directories.
- [FACT] Until A9, **every account sees every vault**. Per-vault grants are
  designed, not built.

### 15.2 Notes [FACT]

- Plain Markdown plus YAML frontmatter.
- `id`, `created` and `modified` are Storm-owned; `modified` is owned by the
  server.
- 3-way merge on `base_version`; conflicts become in-note markers.
- No trash.
- Properties are typed; their types live in hidden vault config.
- Colours are stored as words (`sage`, `mint`…), never hex.
- Wikilinks resolve by **title**.
- Attachments sit in an attachment strip; no inline images yet.

### 15.3 Notes conventions that agents rely on [FACT: `CLAUDE.md`; *How This Vault Works*; `kit/README.md`]

- Agent-edited notes carry `summary:` and `source: ai`.
- `spec/` · `work/` · `log/` project layout. The "four notes or fewer"
  principle exists because `get_note` has no partial read and
  `update_note` replaces the whole body.
- [INFERENCE] These conventions are invisible in the UI but shape vault
  content. A designer should expect machine-written notes, `INDEX`, `BOARD`
  and `CONVENTIONS` notes, and many `summary` properties.

### 15.4 Kit [FACT: decision 64; `kit/README.md`; `apps/server/src/kit.rs`; `mcp.rs`; kit vault folders]

- **Seeded on first boot** (only when no registry exists) from `kit/vault/`
  in the binary:
  - `README`
  - `Project Architecture Guidelines`
  - five role notes: Architect, Lead, Researcher, Coder, Reviewer.
- **The operator's kit vault** also holds `skills/`, `projects/` and
  `workk-skills/` folders. Those are user-grown.
- **Scripts:** `list_scripts`, `get_script`, `create_script` and
  `update_script` work only in the vault named `kit` ("the one place agents
  may store runnable code").
- **Adapters** (`kit/install/claude-code.md`, `opencode.md`, `cursor.md`)
  are "thin loaders": "Editing a note in Storm changes behaviour everywhere,
  with no reinstall."
- [PROBLEM] There is no Kit concept in the client (P5).
- [OPEN] Kit's relationship to:
  - Agent Definitions: the deferred "user-defined named agents";
  - the launcher;
  - the gateway's built-in `storm` connection, through which session agents
    can now read Kit;
  - multi-user: is Kit per server or per user?

---

## 16. MCP / Integrations Model

### 16.1 Two directions of MCP [FACT]

| | Storm as **MCP server** (M13) | Storm as **MCP client / gateway** (M21) |
|---|---|---|
| Who calls | External AI clients (any MCP client), authenticated with an `stk_` key | Agents in Storm sessions, through the host bridge |
| What they reach | Storm's vault tools: eleven read tools always (incl. kit scripts), plus five write tools when writable (`README.md` §MCP) | Upstream services (Notion, Linear, GitHub…) **and** Storm's vaults (built-in `storm`, no `delete_note`) |
| Switches | `mcp_enabled`, `mcp_writable` (server-wide, owner) | Per connection: enable, tool allowlist, resources/prompts toggles; per launch: allow vault writes (AND `mcp_writable`) |
| Credentials | `stk_` keys, per account, shown once | Upstream tokens, encrypted on the server, never sent to hosts or agents |
| UI | Server settings ▸ AI access; Server settings ▸ MCP keys | Server settings ▸ Integrations; launcher toggle |
| Audit | `security_events` (keys) | `security_events` + `gateway.db.calls` (metadata only, 30 days). **No UI.** |

### 16.2 Gateway rules that shape UX [CONSTRAINT: *MCP Gateway V1 Specification*]

- **Every non-shell session gets every connection** (G-D9). There is no
  per-session selection in V1; per-workspace defaults are LATER.
- **Grants are fixed at launch.** Disconnecting or disabling takes effect
  immediately; adding applies to new sessions only.
- **Tools that appear later default off**, and the owner is notified (81k,
  a snackbar in the current client).
- **Web clients:** static tokens only, no OAuth (G-D13).
- **Naming is "Integrations"** in the UI (G-D21). "Provider" must never mean
  an upstream.
- **Out of scope:** a catalogue or marketplace; stdio upstreams in the
  server; non-owner use; an aggregated endpoint.
- **Accepted risk, shown at launch:** blast radius plus host-inherited
  egress (risk 1). The UI copy is part of the risk acceptance, so a redesign
  must keep it equally visible.

### 16.3 [OPEN]

- Are Integrations a property of the *agents* system, of the *server*, or a
  peer top-level concept?
- Should external MCP access (keys) and the gateway (integrations) be
  presented together as "AI access", or apart?

---

## 17. Runtime / Infrastructure Model

### 17.1 Components [FACT]

| Component | Role | Authority | User-visible? |
|---|---|---|---|
| storm-server | Control plane: vaults, auth, Agent Manager, gateway | **Sole authority** | Indirectly (Server settings, sync status, host label) |
| storm-relay | Optional tunnel; "never an authority … authenticates no clients" | None | Only "Relayed" status |
| storm-runtime (Runtime Host) | Execution plane: PTYs, providers, workspaces, MCP bridge | None ("never learns who owns a session") | Hosts screen, launcher |
| Providers (agent CLIs) | The intelligence | — | Launcher, terminal |
| Upstream MCP servers | Third-party services | — | Integrations |

### 17.2 Boundaries [CONSTRAINT]

- **Clients talk only to the Storm Server**, never to hosts or agents
  (layering rule 1).
- **A Runtime Host may be the server's own machine**, under a separate OS
  user that cannot read the vaults (P3, AC-S3), **or another machine**.
- **The relay must never become mandatory** (R6). The transport "is not
  part of the product model, and an entity … must never mean something
  different because the connection was relayed" (*Product Model*).
- **Self-hosted vs Cloud:** "the client must not need to know which one it
  talks to" (*Product Model*).
- **Agent V1 connectivity is direct network only.** Relay for agents is
  later.

### 17.3 Foundational vs replaceable

| Foundational (do not design around removing) | Replaceable / pluggable |
|---|---|
| Plain Markdown vaults; server as the copy of record | Agent providers (Claude Code, OpenCode, shell, future native/API/SDK) |
| Server as the sole authority | Terminal emulator (`xterm2`, behind a surface boundary) |
| UUID identities (notes, vaults); per-vault index | Upstream integrations |
| Owner-controlled infrastructure | Relay (optional), hosting (self vs Cloud) |
| MCP as the knowledge interface | Interaction kind (terminal today; structured later) |

[FACT/INTENT: *Vision*: "a better agent should be integrable without a
redesign"; AM20; *Shared Knowledge*]

**What Storm must stay independent of** [INTENT]:
- any single agent vendor;
- any model;
- any cloud;
- the relay;
- the client device being online;
- Storm itself, for reading notes ("escapable" vault, PRD §2).

---

## 18. Existing Design Language

### 18.1 Foundations [FACT: `ui/tokens.dart`, `ui/theme.dart`, `ui/oklch.dart`; *Storm Client*; decisions 43–44]

- **Token layer:** ~15 numeric inputs derive every colour, size, radius and
  duration.
- **Colour space: OKLCH**, for perceptually even steps.
- **A literal in `lib/ui/` fails a test:** `Colors.*`, a bare `fontSize:`,
  or a numeric `circular()` other than the 999 pill.
- **Three identities:**
  - **Storm dark** (default; "dark-first … a notes app is mostly read at
    night");
  - **Storm light**;
  - **SlowFlow earth** (warm paper, also the marketing site).

  The user chooses explicitly; the OS setting does not override it.
- **Typography:**
  - IBM Plex Sans: UI.
  - IBM Plex Mono: metadata, counts, paths, the vault bubble initial.
  - Newsreader (serif): the default note body; user-selectable note font.
- **Semantic colour is fixed:**
  - accent = interactive (links, active, primary action);
  - amber = tags and highlights only;
  - green = synced;
  - danger = failure or conflict;
  - `text3` = offline or inactive.

  "A colour used for a second purpose stops working as a signal."
- **Contrast** is measured against `surface`, not `bg`.
- **Vault and note accents** are a named palette, stored as words. Vault
  tiles are tinted squares with an initial.

### 18.2 Components and patterns

- **Corner bubbles** (rounded squares = places) and the **pill** (= an
  action bar) with one **PrimaryCircle**. "The shape difference is
  deliberate grammar."
- **One content inset**, `StormChrome.contentInset`. Bubbles, headers and
  prose share one left edge.
- **No app bar inside a vault.** Each screen has its own one-row header
  (decision 40).
- **Sheets** for search, tags, mentions, the launcher and the switcher.
  **Popovers** for the vault switcher and appearance.
- **Section labels:** small, uppercase, letter-spaced ("RECENTLY OPENED",
  "VAULTS"). Settings are grouped by labels and spacing, "not cards".
- **Recents are text rows, not cards** ("eight cards stacked is eight
  competing rectangles"). Vaults are cards.
- **Status:** `StatusDot` (synced, syncing, offline, untrusted, relayed);
  `StatusChip` ("carries a real state, never decoration", 77d); the agent
  provider "mark".
- **Empty states** always say what is true and offer the next action ("No
  vaults yet · New vault", "Set up a host").
- **Skeletons without shimmer** (decision 44).
- **"Offline is a state, not an error."**
- **Confirmation copy** names the real consequence: "Remove from Storm … its
  directory and every note stay"; "Storm has no trash"; "Close this and the
  string is gone".
- **Secrets are shown once**, in deliberately hard-to-dismiss dialogs (MCP
  keys, host enrollment).
- **Keyboard-riding bars:** the editor toolbar and the terminal extra-keys
  row share one family style.
- **Icons:** Lucide, plus six hand-drawn nav glyphs matching the prototype.
- **Read / Edit toggle** and **Notes | Agents switch** share a segmented
  language (`accentSoft` active segment).

### 18.3 What appears successful (probably preserve) [INFERENCE, supported by decision rationale and screenshots]

- **The Notes experience:**
  - read mode typography (serif body, 640 measure);
  - the properties drawer with typed chips and colour swatches;
  - the calm, chrome-light phone screens;
  - the stable six-slot pill;
  - the folder tree vs drill-down split;
  - honest sync language.

  Most of the M0–M18 decisions carry a recorded "why" and a regression
  test.
- **The theme system** and fixed semantic colours.
- **Status honesty:** missing vaults shown greyed, `unknown` rather than a
  guessed `running`, announced fallbacks, explicit gaps.
- **Copy voice:** plain, consequence-first, never alarmist.

### 18.4 Where the language breaks down [PROBLEM]

- Material AppBar, OutlinedButton, SwitchListTile and FAB screens (§11 P9).
- Settings as one long scrolling page with mixed control types (buttons
  that navigate, switches that act, rows with overflow menus).
- The phone Agents screen uses an AppBar while being declared a peer of the
  vault screens.

---

## 19. Product and Technical Constraints

### 19.1 Hard constraints (violating these is a defect or a security issue)

| # | Constraint | Source |
|---|---|---|
| H1 | Vaults stay plain Markdown. Storm state never goes inside a vault. Colours are stored as words. | `CLAUDE.md` invariants |
| H2 | The server is the only authority. Hosts and relays never are. Clients talk only to the server. | *V1 Freeze* §3; relay R5/R12 |
| H3 | The UI never infers authorization. The server's 403 decides. Non-owners see **no** agent or integration entry point: not disabled, absent. | Decisions 77d, 78; AC-S1; `space_switch.dart` |
| H4 | Agents, Hosts, Integrations and server-wide config are **owner-only** in V1. | AM5, G-D20, decision 80 |
| H5 | **The phone layout is the default; wide is additive.** One breakpoint (900px). A change to what renders below it is a defect. | Decision 34; `CLAUDE.md` |
| H6 | Every location is a child of `/`, so Android back unwinds rather than exiting. Deeper means `push`; switching means `go`. | Decision 17 |
| H7 | The desktop home forward and system back are coupled. | Decision 78 |
| H8 | Search and Tags (and other overlays) must remain **routes** for web deep links. | Decision 41 |
| H9 | Secrets are shown once: MCP keys, enrollment strings, integration tokens. Credentials are never re-displayed. | A5, 81h |
| H10 | Session status is drawn from the one vocabulary. `unknown` and `failed` reasons must be shown honestly. Fallbacks are announced. | *V1 Freeze* §6, §7.2 |
| H11 | Ending a session needs confirmation. Input is never auto-retried. | *V1 Freeze* §11.3 |
| H12 | The blast radius (integrations, vault access, host egress) must be **shown at launch** and on Integrations. | AM6/AM27, G-D10, *Gateway* risk 1 |
| H13 | Terminal focus suppresses app shortcuts. Ctrl+C belongs to the agent. | *Client UX* |
| H14 | Web cannot do integration OAuth. Web clients use static tokens. | G-D13 |
| H15 | A merged or conflicted save means the client adopts the server text. Conflicts are shown in the note. | `CLAUDE.md` |
| H16 | No trash exists. Delete copy must say so. | Decision 38a |
| H17 | Token conformance: no literal colours, sizes or radii in `lib/ui/`. | Decision 43 |
| H18 | "Provider" means an agent integration only, never an upstream service. | *Gateway* §3 |

### 19.2 Strong preferences (documented, revisitable with reason)

- "Sessions are something you *do*; settings is where you configure."
  (decision 78)
- No app bar inside a vault; the bubbles and pill grammar (decision 40).
- Name spaces by activity ("Notes", not "Vaults") (decision 78). This is
  explicitly up for revisit with a third space.
- One entry point per intent; no duplicate "N live" stats (decision 78).
- The Agents band never moves (muscle memory) (decision 78).
- "Don't overbuild"; vision ≠ MVP ≠ implementation (*Product Strategy*).
- No competitor comparisons; don't market what isn't released (*Storm
  Website Home*).

### 19.3 Explicitly out of scope (V1 / near term)

**Agent Runtime V1 non-goals** (*V1 Freeze* §2):
- remote IDE and file editing;
- Git UI;
- server shell;
- orchestration;
- approvals;
- user-defined named agents;
- pause and resume;
- resource monitoring;
- egress control;
- relay for agents;
- Cloud.

**Gateway out of scope:**
- an integrations marketplace;
- non-owner use;
- aggregated endpoints.

**Platform:**
- iOS;
- Linux desktop release (deferred).

---

## 20. Future Roadmap / Expected Expansion

Certainty levels:
- **Committed:** spec'd and approved.
- **Planned:** on the roadmap with exit criteria.
- **Proposed:** in the pack, not scheduled.
- **Hypothesis.**

| Capability | What is planned | Certainty | Source |
|---|---|---|---|
| MCP Gateway release | On-device acceptance, then release | Committed | *Global Todo*, *Gateway* §17 |
| Authorization (A9) | Per-vault grants, roles (owner/admin/member), filtered vault lists, 403 on named vaults | Planned; blocked on Q19–Q25 | *Auth Authorization Review* |
| Relay in production | VPS relay; relay settings screen in the client; agents over relay later | Planned | *Global Todo*, R11 |
| Phase 2: Boundaries | Resource budgets, process isolation, **egress policy (default none)**, closing the "host-inherited" exception | Planned | *Roadmap* |
| Phase 3: Orchestration | Parent/child sessions, delegation, cascade-stop, workspace policies, tasks, roles, observability ("the three questions") | Planned | *Roadmap*, *Agent Orchestration*, *Agent Roles* |
| Phase 4: Server execution | Opt-in, audited, deny-by-default command capability | Planned | *Roadmap*, *Server Execution* |
| Approvals queue | "A queue of actions waiting on the human" | Proposed (Later) | *Client UX*, *Security*, *Gateway* LATER |
| Overview / resource monitor | Active agents, recent activity, CPU, memory, disk per agent | Proposed (Later) | *Client UX* |
| Richer session view | Chat, a log/trace pane, files touched, pause/resume | Proposed (Later) | *Client UX* |
| Non-terminal interactions | `native` / `api` / `sdk` provider kinds with structured interactions | Designed boundary, not built | AM20, *V1 Freeze* §9 |
| More providers | Codex, Cursor Agent, Gemini CLI, "any CLI agent" | Proposed | *Vision* |
| Named agents / Agent Definitions | User-defined agents with capabilities and policies | Deferred | *V1 Freeze* §6, *Configuration* "Later" |
| Notifications | Approvals, failures, completion only | Proposed | *Client UX* |
| Sessions as routes, deep links, switch shortcuts, shared sidebar frame | Navigation follow-ups | Deferred (decision 78) | PLAN 78, *Global Todo* parking lot |
| Per-workspace integration defaults, admin-shared connections, device-code grant, data-key rotation | Gateway LATER | Proposed | *Gateway* §17 |
| Staged agent writes | Agent scratch space a human promotes | Open question | *Shared Knowledge* |
| Context/retrieval (`get_context`), embeddings | Bounded context delivery | Open | *Context and Retrieval*, *Open Questions* |
| Engineering graph | Requirements ↔ code ↔ tests ↔ agent work traceability | Proposed / Hypothesis | *Engineering Graph* |
| Workspace pillar | Projects, files, Git, shell, infrastructure | Proposed | *Vision* |
| Storm Cloud, Teams, Enterprise | Managed hosts; shared workspaces; governance | Hypothesis | *Product Strategy*, *Cloud Architecture* |
| Platforms | iOS; Linux desktop; Windows (mentioned in the gateway OAuth copy) | Backlog / parking | *Global Todo* |
| Editor | Block-based editor, inline images | Backlog | *Global Todo* |
| Client/server version check, in-app update | Compatibility screen | Backlog (needs a decision) | *Global Todo* |
| Users, devices and sessions management UI | "Client: sessions screen improvements", rename server, credential rotation | Planned (auth program) | *Global Todo* |

---

## 21. Future-Proofing Considerations

For each item: what it would stress in the current IA, and what a design
should avoid making structurally impossible. These are cautions, not
answers.

1. **A third, fourth or fifth space** (Workspace, Tasks/Orchestration,
   Approvals, Overview).
   - **Stresses:** the binary Notes | Agents switch; the phone's
     dashboard-band entry.
   - **Avoid:** a navigation model that can only hold two peers, or that
     differs fundamentally between phone and desktop.
2. **Agents as objects** (named agents, Kit roles, orchestration trees).
   - **Stresses:** "Agents" currently labels a sessions list.
   - **Avoid:** spending the word "Agent" on sessions in a way that forces a
     rename later; one flat list that cannot show parent/child.
3. **Addressable sessions and notifications.**
   - **Stresses:** sessions aren't routes; tabs are per device.
   - **Avoid:** designs that assume a session is only reachable from a list.
     Notifications, note links and approvals will deep-link.
4. **Approvals and human-in-the-loop.**
   - **Stresses:** no inbox-like surface exists; prompts currently live
     inside the terminal.
   - **Avoid:** a home with no place for "things waiting on you"; burying
     approvals inside individual sessions.
5. **Structured interactions** (non-terminal providers).
   - **Stresses:** the session view is "chosen by interaction kind".
   - **Avoid:** making the terminal the definition of a session in the IA.
     It is one view (AM20).
6. **Phase 2 policies** (egress, budgets, isolation) and per-session grants.
   - **Stresses:** the launcher is a short sheet; Integrations is global.
   - **Avoid:** a launcher or session model with no room for
     policy/permission summaries; hiding the blast-radius line.
7. **A9 multi-user.**
   - **Stresses:** a single owner boolean gates whole areas; Server settings
     is shown to members.
   - **Avoid:** an IA that only has "owner sees everything" vs "member sees
     notes". Admins, grants and shared connections will need graded
     visibility.
8. **More infrastructure** (relay list, hosts on Cloud, backups, versions,
   server identity rotation).
   - **Stresses:** Server settings is already overloaded.
   - **Avoid:** letting operational surfaces crowd the product surfaces.
     The Vision explicitly does not want an "infrastructure dashboard" as
     the experience. Equally, avoid hiding health problems.
9. **Many vaults and agent-generated content.**
   - **Stresses:** per-vault search; a 5-item recents list; a vault grid as
     home.
   - **Avoid:** a model where finding anything requires first knowing its
     vault; making Kit indistinguishable from personal content.
10. **Knowledge ↔ agent linkage** (provenance, engineering graph).
    - **Stresses:** two disconnected spaces.
    - **Avoid:** structures where a note and the session that wrote it can
      never be shown together or cross-linked.
11. **Cloud and relay transparency.**
    - **Avoid:** UI that distinguishes self-hosted from Cloud, or relayed
      from direct, as different products. Status may differ; the model may
      not.
12. **Platform growth** (iOS, Linux, Windows; tablets stay on phone layout).
    - **Avoid:** a desktop-only or phone-only home for any core capability.

---

## 22. Open Design Questions

These are for discussion with the designer. The repository does not answer
them unless a citation says so.

### Object model and hierarchy

1. What is the top-level container the user thinks they are in: "my
   server", "my Storm", "a workspace"? Should it be visible in the UI at
   all?
2. Should the IA be organised by **activity** (Notes, Agents) or by
   **object** (Vaults, Sessions, Hosts, Integrations)? Decision 78 chose
   activity for two spaces.
3. What deserves first-class navigation today, and what will in 6 months
   (§20)?
4. Is there a **server-level home** (overview), and is it the same on phone
   and desktop?

### Knowledge

5. How should Vaults relate to Notes in navigation: a container to pick
   first, or a facet of a unified notes space?
6. Should search and recents be cross-vault by default?
7. Is Kit a vault, a library, agent configuration, or a separate concept?
   Should it be visually distinct from personal vaults? Can a user have
   several kits?
8. How should agent-written content be distinguished, reviewed or promoted
   (the *Shared Knowledge* open question)?

### Agents

9. Should "Agents" remain the name of the space whose contents are
   sessions? What will "an agent" be once named agents or roles exist?
10. How should sessions relate to agents, tasks and workspaces in the
    hierarchy?
11. Should running agent state be visible from everywhere (status, badge),
    including inside Notes?
12. Should notes and sessions be viewable side by side on desktop?
13. What does "return to where I was" mean across devices, given that tabs
    are per device?

### Integrations, MCP, infrastructure

14. Where should Integrations live: with Agents, with server settings, or
    as their own area?
15. Should external MCP access (keys, switches) and the gateway be presented
    as one "AI access" concept?
16. Where should Runtime Hosts live: agent workflow, infrastructure, or
    both?
17. How should the boundary between *using* Storm and *operating* Storm be
    drawn? Which operator concerns appear in the client at all?
18. Is there a single health/status surface for server, relay, hosts,
    integrations and version?

### Settings

19. How should device, account, vault, server and owner-only settings be
    separated and named?
20. Where do MCP keys, devices, sign-out and the future users/roles
    management belong?
21. What should a member see of server configuration they can't change?

### Platform

22. What is the phone's primary navigation once there are more than two
    spaces?
23. Which differences between phone and desktop are layout (acceptable), and
    which are structure (to avoid)?
24. How should the one-handed constraint shape agent controls as they grow
    (approvals, end, switch)?

### Scale and restraint

25. What must stay secondary so Storm does not become an infrastructure
    dashboard?
26. How will the IA absorb approvals, notifications and orchestration
    without restructuring again?
27. How should owner-only vs member experiences relate after A9?

---

## 23. Things the Designer Should Preserve

- **The Notes reading and editing experience:**
  - read mode typography;
  - the properties drawer;
  - the honest sync language;
  - the calm, chrome-light phone screens;
  - the version line;
  - the conflict model.

  These are the product's most mature, most tested surfaces (M0–M18).
- **The token system and three identities.** OKLCH tokens, fixed semantic
  colours, the theme choice owned by the user, colours stored as words.
- **The status-honesty principles:**
  - missing is shown, not hidden;
  - `unknown` over a guessed `running`;
  - announced fallbacks;
  - explicit gaps;
  - offline as a calm state;
  - the shown-once secret dialogs.
- **The copy voice:** consequence-first, plain, and exact about what Storm
  does and does not do ("Storm has no trash", "Storm cannot revoke a token
  itself").
- **The security disclosures:** the launcher's network and integrations
  lines; the Integrations intro.
- **Phone-first behaviour:**
  - keyboard-aware bars;
  - the extra-keys row;
  - Android back semantics;
  - one-handed agent controls.
- **The owner-only invisibility rule:** absent, not disabled.
- **Deep-linkable routes**, including the overlays.

## 24. Things the Designer Should Challenge

- **Server settings as the home for every non-note capability** (P2, P3,
  P8).
- **The absence of a server-level home on desktop**, and the phone-only
  recents and agent band (P1, P6).
- **The binary Notes | Agents switch** as the long-term top-level structure
  (P14; decision 78's own revisit trigger).
- **The vocabulary:** Agents/sessions, the four "MCP"s, the two "sessions",
  the two "hosts", "workspace" (§5.1).
- **Kit's invisibility** (P5).
- **The two visual grammars** (P9).
- **The placement of account-level items:** MCP keys under Server; Sign out
  under Client (P13).
- **The "+" on home creating a vault** (P17).
- **The lack of any surface joining knowledge and agent work** (P12).

## 25. Things the Designer Must Not Assume

- **That `main` shows the current product.** Integrations exist only on
  `staging`. The screenshots predate Agents.
- **That Storm is a notes app with AI added, a remote terminal, or an MCP
  client.** The thesis is a persistent workspace and control plane (§2).
- **That "Agents" are configurable objects.** In V1 they are sessions of
  host-declared providers.
- **That members see agents, hosts or integrations.** They don't, and must
  not see disabled versions.
- **That host-side things are editable from the client:** workspaces,
  provider install and login, `runtime.toml`.
- **That integrations can be chosen per session.** All non-shell sessions
  get all of them in V1.
- **That agents can delete notes.** Never through the gateway.
- **That Storm stores model or provider API keys.** It never does. Only
  integration credentials, encrypted.
- **That a relay, Cloud or a specific agent vendor is present or
  required.**
- **That the desktop dashboard exists, or that removing the forward is
  harmless** (H7).
- **That a vault equals a project or workspace.** Agent workspaces are host
  directories, unrelated to vaults.
- **That every account can change server settings.** Writes are
  owner-only.
- **That roadmap items are committed.** Check §20's certainty column; much
  of the Vision is `PROPOSED` or `HYPOTHESIS`.

---

## 26. Evidence / Repository References

### Vault (operator's `personal` vault, read via Storm MCP)

**`Storm/Agent Runtime/`:**
- *Vision*
- *Problem*
- *Product Model*
- *Product Strategy*
- *Client UX* (navigation, decision 78)
- *V1 Specification Freeze* (rev 4, approved; amended by D13)
- *Roadmap*
- *Configuration*
- *Shared Knowledge*
- *Agent Orchestration*
- *Engineering Graph*
- *Open Questions*

**`Storm/MCP Gateway/`:**
- *V1 Specification* (frozen rev 3; G-D1–G-D25, risks §18, scope §17)

**`Storm/Remote/`:**
- *Auth Authorization Review (A9)* (role matrix, Q19–Q25)

**`Storm/`:**
- *Client* (layers, design system, routing rules, UI shell)
- *Global Todo* (status as of 2026-10-08, backlog, parking lot)
- *Website/Home* (public positioning)

### Repository (`origin/staging` `d864f8d`, unless noted)

**Top-level docs:**
- `README.md`: thesis line, architecture diagram, MCP summary. It says
  v0.2.9 and is stale relative to v0.3.1.
- `CLAUDE.md`: invariants (vault, M9/M10, M19, relay, M20).
- `PLAN.md` decision log:
  - 3, 17, 20–25, 30–34 (layout), 38–44 (MCP, chrome, sheets, note
    actions, tokens);
  - 64 (kit);
  - 76 (permissions);
  - 77–77e (Agent Runtime);
  - **78** (Agents space);
  - **80** (owner-only config);
  - **81, 81h, 81k–81n** (gateway client).
- `docs/prd.md`: the original brief, superseded.

**Kit:**
- `kit/README.md`, `kit/vault/agents/*`
- `apps/server/src/kit.rs`
- `apps/server/src/mcp.rs` (script tools and kit restriction)

**Client, routing and shell:**
- `apps/client/lib/router.dart`: route tree, redirect, owner guard.
- `apps/client/lib/main.dart`: theme default, OAuth orphan → Integrations.
- `apps/client/lib/ui/shell/`:
  - `dashboard.dart` (`notesHome`, desk-width forward, recents limit);
  - `vault_shell.dart`;
  - `vault_sidebar.dart` (switcher contents, footer, gear);
  - `nav_bubble.dart` (pill grammar);
  - `vault_actions.dart` (pill slots, dashboard "+");
  - `corner_bubbles.dart` (vault bubble, "A" bubble, `ClientSettingsBody`);
  - `storm_scaffold.dart`;
  - `space_switch.dart`.
- `apps/client/lib/ui/breakpoints.dart`: one breakpoint, ratios.

**Client, screens:**
- `apps/client/lib/ui/server_settings_screen.dart`
- `client_settings_screen.dart`
- `mcp_keys_screen.dart`
- `browse_screen.dart`
- `note_screen.dart`
- `note_properties.dart`
- `mentions_section.dart`
- `search_screen.dart`
- `tags_screen.dart`
- `pairing_screen.dart`
- `login_screen.dart`
- `add_device_screen.dart`

**Client, agents and integrations:**
- `apps/client/lib/agent/agents_shell.dart`
- `agents_screen.dart` (list, tabs, session view, extra keys, launcher)
- `agents_band.dart`
- `hosts_screen.dart`
- `integrations_screen.dart`
- `oauth_flow_io.dart`

**Client, design system:**
- `apps/client/lib/ui/tokens.dart` (font families, presets)
- `theme.dart`

**Recent commits:**
- `ae56abe` (session rows)
- `856a5d6` (phone session layout)
- `50db59a` (keys row)

**Screenshots:**
- `docs/screenshots/desktop-wide.png`
- `phone-dashboard.png`
- `phone-server-settings.png`

All pre-Agents.
