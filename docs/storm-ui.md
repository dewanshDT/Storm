# Storm — what the app does today

A functional inventory of every surface, written for someone designing against
it. Rewritten for Storm v2 (M22, decision 82) on 2026-10-08 from the built app
on `feat/v2-acceptance`; routes, state strings and copy are literal. The design
source is `design_handoff_storm_v2/` (the prototype wins over its README);
`docs/design/acceptance/storm-v2/` holds a screenshot of every state below.

Companion to the older design briefs — `storm-multi-vault.md` (M9/M10),
`storm-properties.md` (M11), `storm-adaptive.md` (M12). `storm-ui-refactor.md`
(M7/M8: the dashboard and the vault bubble) describes surfaces v2 removed.

---

## 1. What Storm is

A notes app that keeps your notes on **your** server, and runs coding agents
next to them. Notes are plain markdown files in a folder — greppable,
backup-able, openable in Obsidian if Storm ever goes away. A small Rust server
in the homelab owns the canonical copy; the phone, the Mac and the browser are
clients that sync to it.

- **Single user.** One account, a password, several devices (decision 82). No
  members, roles, registration or account picker.
- **Activities, not vaults.** The app has two activities — **Notes** and
  **Agents** — and one global **Settings**. Vaults (`personal`, `work`, the
  server-seeded `kit`) are the context inside Notes.
- **The loop.** Start an agent session from a note, let it write to one vault
  you chose, see what it wrote (**Wrote**) and, on each note it touched, which
  session did (**provenance**, an unseen dot).
- Platforms: **macOS, Android, web, Linux.** One Flutter codebase.

---

## 2. Ground rules

**The vault is plain markdown, always.** Anything a feature stores goes in the
note's frontmatter in words a human would write (`color: sage`), never a hex.

**Offline is a normal state, not an error.** Every Notes screen is reachable
with no server; edits queue and replay. Agents need the server and say so.

**The server decides, and conflicts are visible.** The server merges; when it
can't, the conflict is written into the note as marked-up text.

**One breakpoint, at 900px.** Below it the phone layout (the default); at and
above it the activity rail and a sidebar. The same places exist on both; only
their layout differs. Portrait tablets get the phone layout.

**Dark first.** Three presets — Storm dark (default), Storm light, SlowFlow
earth — all derived from one token layer (`lib/ui/tokens.dart`); no screen
picks its own colour, size or radius (`test/token_conformance_test.dart`).

**There is no trash.** Deleting a note removes the file immediately.

**Unseen is per device.** A dot means "an agent changed this since *this
device* last opened it"; the server never stores it. A device's first look at
a vault takes what agents had already written as seen, so a new device does
not dot every note an agent ever wrote.

---

## 3. The map

Every route is also the web client's URL.

```
/                          not a screen: this device's last location
/pairing · /login          pair a device · sign in (password only)
/notes                     the last vault, or "No vaults yet"
/v/:vault/browse[/path]    vault root / folder (phone); "Select a note" (desk)
/v/:vault/note/:id         a note   (?session=<id>: pushed from a phone session)
/v/:vault/search · /tags   search · tags
/agents                    overview (desk) · Running/Ended list (phone)
/agents/s/:id?tab=…        a session; tab = context | wrote | about
/settings                  the list (phone) · This device (desk)
/settings/:page            device · access · vaults · ai · integrations ·
                           claude-login · hosts · storage · connection ·
                           advanced · health
/add-device                a pairing QR for another device
/gallery                   the token gallery, by typed URL only
```

Retired routes redirect: `/settings/server`, `/settings/client`,
`/v/:vault/settings/*`, `/settings/mcp-keys`, `/agents/hosts`. The OAuth
return still lands on `/settings/integrations`.

**Back (Q2):** the router pops first; then a note goes to its folder, a folder
to its parent, Agents and Settings to the last Notes location; only a vault
root exits the app. Launch restores the last activity and location.

---

## 4. Chrome

**Desktop (≥ 900): the activity rail**, 56 wide, on every screen: Notes,
Agents (badge = live sessions, only above 0), a spacer, the **health dot**
(green when healthy; its popover lists sync, hosts and integrations rows that
link into Settings) and the Settings gear. Active item: `accent` on
`accentSoft`. The health dot and the gear share one 40 px square on the
rail's axis, 8 apart and 8 from the bottom.

**Phone: two corner bubbles** (44 × 44 at 20 / 20). Left opens the **place
picker** — the vaults (tile, name, count, ✓), Agents with "{n} running", and
the sync line; long-press a vault to colour it. Right opens Settings (`accent`
while in Settings). The **pill** at the bottom is Directory · Search · ＋ New
note · Tags on Notes screens (long-press ＋ for a folder; Lucide outlines, as
the prototype draws them), "＋ New session" on the Agents list, and absent on
a note (Q5). Content starts at the 20 inset the bubbles sit on.

---

## 5. Notes

### Sidebar (desktop)

Vault header (tile, name, sync line) → **vault switcher** popover (vaults with
counts and ✓, "Synced … · Sync now", Manage vaults ›). "Search {vault}"
(the old per-vault search, until 156-3; ⌘K opens the palette). **RECENT**: the four newest notes across vaults, each with a vault
tag. **FOLDERS**: the tree (notes by name, drawn twisties). A note an agent
changed carries a 6px `accent` **unseen dot**, and so does a *collapsed*
folder holding one; screen readers hear the name, then "Changed by an agent
since you last opened it". Footer: New note, New folder, Tags.

### Vault root and folders (phone)

The vault's name, **RECENT** (title, "vault · folder", age) and **FOLDERS**.
A folder is "‹ parent" over its name, then its folders and notes; rows carry
the unseen dot.

### Note

Header: the crumb (desk) or "‹ parent" (phone), **Read | Edit**, a soft
**Start session** (phone "Session"), then the properties toggle (desk) or ⋯ and
Properties (phone). Then the title (Newsreader at `displaySize`, omitted when
the body opens with its own `# heading`), the **version line** (`v14 · Saved`,
the id when asked for) with the **provenance link** — "Edited by session
{name}, {age} ›" or "Created by session …" in `accent`, opening that session's
Wrote; a dismissed session's name stays as plain text. The body is the
existing editor (Edit) or the rendered markdown (Read): Newsreader at about 18
(`proseSize = fs·√scale`), line height 1.6, H2 about 22 / 600. The editor
surface fills everything between the sidebar and the Properties drawer (or the
window's edge when it is shut): the header row spans it, and the prose wraps
at the surface's width up to a 900 px measure. Below: attachments (Edit) and
**Linked mentions**.

Start session opens the launcher with the note as context while a host is
online, otherwise Agents (which explains what is missing). A note pushed from a
phone session reads "‹ {session name}" and returns to the terminal; when the
header cannot fit it beside the controls, the controls move to a second line
rather than cutting the name.

**Properties:** a drawer (280, `surface`) beside the note at desk width — open
by default from 1200 px, shut below it (where it would squeeze the prose), and
a toggle that sticks between notes; a sheet on the phone (Q6). The typed
frontmatter list (nine types; `created`/`modified` read only) is the only way
to edit frontmatter.

**States:** `Unsaved` · `Saving…` · `Saved` · `Queued — offline` · `Failed` ·
merged · conflict (markers in the body) · not available offline yet.

### Search, Tags

Full-text search in the current vault (title, path, snippet with marks); tags
with counts, nested under their parents.

---

## 6. Agents

"Agent" is Claude Code, OpenCode or Shell; a **session** is one run of an
agent in a workspace on a host. Kit's `agents/*.md` are notes, never agents.

### Overview (desktop) and list (phone)

Desk sidebar: ＋ New session (only with a host online), Overview, **RUNNING**
and **ENDED** rows with status dots (§3.2: `accent` running, a ring while
starting, `danger` failed, `text3` ended). The overview pane has **WORK**
cards grouped by (workspace, host) — each row a session with its context chip
and "wrote n" — **START AN AGENT** cards that open the launcher with that
agent, and the infrastructure line linking to Settings › Hosts. With no host:
the no-host steps; with hosts and no sessions: the first-session steps. The
phone is a flat Running / Ended list (rows carry the workspace) and the
"＋ New session" pill.

### Launcher

A 460 modal (desk) or bottom sheet (phone): the **CONTEXT** box when started
from a note (× removes it); Host (online only), Workspace (with the shared
workspace warning), Agent (default marked); **Can write to** — defaults to the
note's vault, "Read only" when off, disabled with "Off in Settings › AI
access" when agent writes are off, absent for Shell; for Claude Code, a line
under Agent saying "Claude: your synced login" or "Claude: this host's own
login" (with why, when a ticked host can't get it) by the server's rules;
the risk box. Launch
opens the session on Context (with a note) or About.

### Session

Desk: the session workspace. Its header — name, status chip, the Details
toggle and End, and a meta line — spans the whole workspace; under it the real
xterm surface fills the width (a 10 px inset) beside an optional **Details
inspector** with **Context | Wrote {n} | About** and a close button. The
inspector behaves like Properties: open by default from 1200 px, shut below,
toggled from the header; 320 px wide, its left edge drags it between 260 and
500 while the terminal keeps at least 360; open state and width last for the
run, not across launches. End asks inline in a compact banner — "End {name}"
in bold, the consequence, then Cancel and the red End session on one row (the
message wraps under the title when the banner is narrow) — and ends as
stopped; an ended session shows its footer with **Run again** (the launcher,
prefilled) and **Dismiss**.
**Context** is the source note read only with "Open in Notes ›", or "Started
without a note…". **Wrote** lists created and edited notes ("new" / "v{n}")
and kit scripts, opening a note in the panel with "‹ Wrote"; a note this
session last wrote says "· edited by this session". **About**: Agent,
Workspace, Vault access, Integrations (as granted at launch), Network.

Phone: a full-screen terminal (a 10 px inset), a chip row (Context, Details),
the extra-keys row as a keyboard accessory — only while the software keyboard
is open (`viewInsets.bottom > 0`), above it, with Done (Esc, Tab, Ctrl, ⇧,
arrows, Paste) — the **details sheet**
(Started from, Wrote, About, End with confirmation) and Run again / Dismiss
bars once ended. The context note is pushed full screen.

---

## 7. Settings

One global destination: the settings navigation in the sidebar beside a 680
column (desk, the rail stays), or a list and pushed pages with "‹ Settings"
and no AppBar (phone). Order (§5.1): **This device** (preset, text size, note
font, Read mode) · **Devices & access** (signed-in devices with revoke; access
keys, shown once with a `claude mcp add` line and a stdio JSON entry for
`storm-runtime mcp-connect`, the HTTP entry under a disclosure) · **Storm** › **Vaults** (tile → colour, count, path; a
missing vault greyed and removable) · **AI access** (MCP read / write, and
"Storm agents": allow writes when chosen at launch — independent of
`mcp_writable`) · **Integrations** (connections, tests, tools review; a
`danger` nav dot when one needs attention) · **Claude login** (decision
124: sync one `claude setup-token` into an obscured field that is cleared on
submit, with an optional mint date; status Not synced / Synced / Expires in n
days / Expired / Needs re-sync, a banner at 30 and 7 days and a nav dot;
Replace and Remove; one checkbox per enrolled host, all unticked by default,
no "all hosts"; old hosts marked "update storm-runtime"; the single-user,
terms and claude.ai-revocation notice) · **Hosts & default agent** ·
**Storage** · **Connection** (address, route, pinned key, relays, Disconnect)
· **Advanced** (MCP endpoint, versions, Re-pair) · **About & health** (the
health rows with actions, compatibility). There is no Accounts section. A
preset change applies at once and persists on this device.

---

## 8. Keyboard (desktop, M18)

Platform-aware (⌘ on macOS / Mac web, Ctrl elsewhere). One app-wide layer
in `AppShell` (decision 156), armed on every signed-in screen at every width
wherever there is a physical keyboard; phones arm nothing. The chords come
from one command registry (`lib/keyboard/storm_commands.dart`).

| Action | macOS | Win / Linux |
|---|---|---|
| Command palette | ⌘ K, ⌘ P | Ctrl K, Ctrl P, Ctrl ⇧ K |
| Palette, commands only (`>`) | ⌘ ⇧ P | Ctrl ⇧ P |
| New note (any screen, active vault) | ⌘ N | Ctrl N |
| New folder (Notes) | ⌘ ⇧ N | Ctrl ⇧ N |
| New session (Agents) | ⌘ ⇧ N | Ctrl ⇧ N |
| Toggle sidebar (≥900, any activity) | ⌘ \\ | Ctrl \\ |
| Back / Forward (not on web) | ⌘ [ / ⌘ ] | Alt ← / Alt → |
| Read ↔ Edit | ⌘ E | Ctrl E |
| Save | ⌘ S | Ctrl S |
| Find in note | ⌘ F | Ctrl F |
| Bold / Italic | ⌘ B / I | Ctrl B / I |
| Dismiss / leave | Esc | Esc |

With the terminal focused, every app chord stops there except ⌘K, ⌘P, ⇧⌘P,
⌘[ and ⌘] on macOS and Ctrl ⇧ K / Ctrl ⇧ P elsewhere (AM111).

### Command palette (decision 156)

`lib/ui/palette/`. At ≥900 a top-aligned dialog on the root navigator, 640
wide; below 900 a full-screen route. Focus stays in the field; closing gives
focus back, or leaves it to the destination.

- **Groups**, in a fixed order, empty ones hidden, five rows each and then
  "Show all n": Notes (titles in every vault, the active vault first, a
  vault tag on each), Sessions (needs you, then running, then ended; the
  status in words), Commands (the registry's, with their chord on the
  right), Settings, Folders & tags (tags in amber, the active vault's only),
  Hosts, then **Create note "{q}" in {vault}** last when no title matches
  exactly.
- **Empty query**: NEEDS YOU, RECENT (every vault), SUGGESTED (three
  commands). **No match**: "No matches for '{q}'" and the create row.
  **Offline**: one `text3` line, "Offline · searching this device", over
  cached notes and recents.
- **Keys**: ↑/↓ move and wrap; Enter runs the row; ⇧Enter creates a note
  from the query; ⌘Enter / Ctrl Enter on a note opens the launcher with it
  as context; Esc clears, then closes. Enter and the arrows wait while an
  IME composes.
- **Prefixes**: `>` commands, `@` sessions, `#` tags, `/` folders, `?` the
  list of prefixes.
- **Rows**: `surface`, or `surface2` when selected, where every quiet word
  steps up from `text3` to `text2`. Screen readers hear a button with its
  selected state and the chord as its hint, and a polite "{n} results".

The sidebar's "Search {vault}" field and `/v/:vault/search` stay until
156-3 retires them; nothing new links to them.

---

## 9. State vocabulary

| Shown | Means | Where |
|---|---|---|
| `Unsaved` · `Saving…` · `Saved` | Save progress | Version line |
| `Queued — offline` | Held locally, will replay | Version line |
| `Failed` | Rejected; the edit is still here | Version line |
| `v12` | Which version you are editing from | Version line |
| Edited / Created by session {name}, {age} › | Latest agent writer | Version line |
| unseen dot | Changed by an agent since this device opened it | Tree, folders, phone rows |
| Starting · Running · Unknown | A live session | Status chips |
| Completed · Stopped · Failed | An ended session | Status chips |
| `{n} running` | Live sessions | Place picker, rail badge |
| `Directory not found` | The vault's folder is gone | Settings › Vaults |

---

## 10. Colour and type

### Accents

Notes and vaults can carry one of ten accents. **The name is what gets written
into the file** (`color: sage`), so the set is fixed and the words are part of
the product. Each has a light and a dark value; a card uses it at full strength,
a page tints with it at ~40%.

| Name | Light | Dark |
|---|---|---|
| `none` | — | — |
| `coral` | `#FAD2CF` | `#5C2B29` |
| `peach` | `#FDE2CE` | `#614A19` |
| `sand` | `#FFF8B8` | `#635D19` |
| `sage` | `#E6F4D7` | `#345920` |
| `mint` | `#D4E4ED` | `#16504B` |
| `sky` | `#D3E3FD` | `#2D555E` |
| `lavender` | `#E9D9FB` | `#42275E` |
| `blossom` | `#FDCFE8` | `#6C394F` |
| `clay` | `#E9E3D4` | `#4B443A` |

The product mark is a hand-drawn tornado on a mint card: `#96F2D7` ground,
`#343A40` strokes.

### Type

- **Note bodies** — Newsreader, a serif, bundled with the app so it works
  offline. The default. About 18 / 1.6 at the default text size, H2 about
  22 / 600 (handoff §7.2), scaling with the setting.
- **Alternatives** — the platform's own interface face, or its monospace.
- **App chrome** — IBM Plex Sans; labels, metadata and the terminal IBM Plex
  Mono. Both bundled.
- **Size** — adjustable, default 16.

Only faces that ship with the app or the platform: downloading a font at runtime
is wrong for something that must work with no network, and it would make the
editor's text metrics depend on the connection.

---

## 11. Gaps and open questions

Known holes, and the decisions most worth a designer's answer.

**Tables render only in Read mode.** The editor is one text field whose styling
must map to the underlying characters exactly, so in Edit a table stays pipes.
Read mode (the default) renders it.

**Conflicts are raw.** A banner, then git-style markers to delete by hand.
→ *What should choosing between two versions look like on a phone?*

**Properties are a sheet on the phone.** The typed list is the only way to
edit frontmatter; on a phone it is one tap away rather than above the prose.

**Delete has no undo.** No trash anywhere in the product.
→ *Design a trash, or design a confirmation that earns the risk?*

**Agent writes are marked per session, not per assistant.** A note a Storm
agent session wrote carries provenance and an unseen dot; a write through an
access key (`/mcp`) carries neither.

**Empty states follow `EmptyState`.** Agents' no-host and first-session states
are designed (numbered steps); the Notes ones (empty folder, no matches, no
tags) use the same voice without a design of their own.
