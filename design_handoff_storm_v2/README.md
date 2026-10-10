# Handoff: Storm v2 — activity rail, Agents as an activity, global Settings

## Overview

Storm v2 restructures the Flutter client around **activities** rather than vaults:

- a lightweight **Activity Rail** (desktop) / **place picker** (phone) with **Notes** and **Agents**, plus **Status** and **Settings** at its foot;
- **Settings** becomes one global destination, reachable from anywhere and never inside a vault control;
- the **knowledge → agent work → session → resulting knowledge** loop is made visible: start a session from a note, see what it wrote, and see on the note which session edited it.

Storm is now designed as a **single-user personal workspace**. There are no accounts, members, roles, registration or invitation surfaces.

The approved prototype is the source of truth. Wherever this README and the prototype disagree, the prototype wins. Where something is genuinely undecided, it is listed under **Open implementation questions (§12)**; do not resolve those silently.

## About the design files

The files in this bundle are **design references built in HTML**. They show the intended look and behaviour; they are not production code to copy. The task is to **recreate these designs in the existing Flutter client** (`apps/client/lib/`), using its established patterns: `tokens.dart`, `theme.dart`, `widgets.dart`, `states.dart`, `accents.dart`, `breakpoints.dart`, go_router and Riverpod.

| File | What it is |
|---|---|
| `Storm v2.dc.html` | The presentation page. It shows a live desktop frame (1280 × 800) and a live phone frame (390 × 844) side by side. Its Tweaks switch the theme (`storm-dark`, `storm-light`, `slowflow-earth`) and the Agents state (`running`, `first`, `empty`). |
| `Storm App.dc.html` | The prototype itself. Template and logic are in one file. The `device` prop is `desktop` or `phone`; both share one state model. |
| `support.js` | The runtime the `.dc.html` files need. Open `Storm v2.dc.html` in a browser. |
| `screenshots/` | Static PNGs of the approved states, at 2× and in Storm dark. Use them as visual references; see §13 for the index. |

**Fidelity: high.** Colours, type, spacing, radii and interactions are final. The colours are the **existing token derivation**: the prototype re-implements `tokens.dart` and `accents.dart` formulas in JS. Implement them with the real `StormTokens`, not new literals. The token conformance test (no literal colours, sizes or radii in `lib/ui/`) still applies. §7 maps every prototype size to a token role.

Baseline: `dewanshDT/Storm@staging` `d864f8d`.

---

## 1. Information architecture

### 1.1 Scopes

Every capability belongs to exactly one scope:

| Scope | Contents | Lives in |
|---|---|---|
| **Storm workspace** | Notes (vaults, folders, notes), Agents (work, sessions) | Primary navigation: rail / place picker |
| **This device** | Theme, text size, note font, read mode, show note id, keyboard | Settings › This device |
| **Terminal** (this client) | Terminal text size, line spacing, padding | Settings › Terminal |
| **You (access)** | Signed-in devices, add a device, access keys, sign out | Settings › Devices & access |
| **Storm configuration** | Vaults, AI access, Integrations, Hosts & default agent, Storage, Connection, Advanced | Settings › Storm group |
| **Health** | Sync, hosts, integrations, compatibility | Rail status dot → popover; Settings › About & health |

These never appear in primary navigation: MCP endpoints, the storage root, relays, server identity and hosts. Infrastructure surfaces only as status lines that link into Settings.

### 1.2 Primary activities

| Activity | What it contains | Desktop entry | Phone entry |
|---|---|---|---|
| **Notes** | The selected vault: Recent (cross-vault), folders, notes | Rail item "Notes" | Place picker › a vault |
| **Agents** | Overview (work), Running and Ended sessions, New session, session detail | Rail item "Agents", with a running-count badge | Place picker › "Agents" (shows "N running") |
| **Settings** | Global settings | Rail gear at the foot | Top-right corner bubble |

There is **no Home or dashboard**. The phone dashboard (`dashboard.dart`, vault grid, masthead, agents band) is **removed**. Its jobs move as follows:
- recents → the top of the Notes sidebar (desktop) or the top of the vault root (phone);
- vaults → the vault switcher or place picker, and Settings › Vaults;
- running agents → the rail badge, the place picker count, and Agents › Overview.

The rail is built to take more activities later, but **no placeholder slot is drawn** today.

### 1.3 Vault context

- The **selected vault scopes Notes**: the tree, search ("Search personal"), new note and new folder.
- Vaults are **not** a navigation level above activities. The vault is shown as context:
  - desktop: the vault header at the top of the Notes sidebar (tile, name, sync line), which opens the vault switcher;
  - phone: the left corner bubble shows the vault tile, and the place picker lists the vaults.
- **Cross-vault items carry a vault tag**: Recent rows (desktop), "vault · folder" lines (phone), the launcher context, and session panels (a "vault / folder" crumb).
- **Kit is a plain vault.** It gets no special treatment, badge or section. Kit's `agents/*.md` files appear only as ordinary notes; the UI never calls them agents (§3).

### 1.4 Routes and deep links

The prototype has no URLs. The mapping below fits the existing router and the constraints H6 (every location is a child of `/`), H7 (desktop home and back-navigation are coupled) and H8 (overlays remain routes). **The route scheme itself is open question Q1.**

| Location | Route (proposed) | Notes |
|---|---|---|
| Notes · vault root / folder | `/v/:vault/browse[/path]` | Unchanged. On the phone this is the vault root list; on the desktop, the tree plus the empty pane. |
| Note | `/v/:vault/note/:id` | Unchanged |
| Search, Tags | `/v/:vault/search`, `/v/:vault/tags` | Unchanged (sheets, still routes) |
| Agents · overview (desktop) / list (phone) | `/agents` | Exists |
| **Session detail** | `/agents/s/:sessionId[?tab=context\|wrote\|about]` | **New.** The "Edited by session" link and Run again need sessions to be addressable. |
| Settings list (phone) / first page (desktop) | `/settings` | Desktop redirects to `/settings/device` |
| Settings page | `/settings/:page` | `device`, `access`, `vaults`, `ai`, `integrations`, `hosts`, `storage`, `connection`, `advanced`, `health` |
| New-session launcher | — | A sheet, not a route (as today) |

These routes are retired and should redirect:
- `/settings/server` → `/settings/vaults`
- `/v/:vault/settings/server` → `/settings/vaults`
- `/settings/mcp-keys` → `/settings/access`
- `/agents/hosts` → `/settings/hosts`
- `/v/:vault/settings/client` → `/settings/device`
- `/settings/integrations` keeps its path, so the OAuth-orphan auto-navigation in `main.dart` still works.

### 1.5 Launch behaviour

- Launch **opens the last activity and location** (persisted per device).
  - Notes: the last vault and note. In the prototype, the desktop starts on a note and the phone starts at the vault root.
  - Agents: the overview, or the last open session.
  - Settings: the last page.
- With no vaults at all, Notes shows the "No vaults yet · New vault" empty state, which links to Settings › Vaults.
- What `/` resolves to, and where Android back finally exits, are open question Q2.

---

## 2. Screens

Desktop frame measurements:
- rail 56;
- sidebar 260 (use `context.sidebarWidth`; it is 260 at the 1200 design frame);
- main pane flexes;
- properties drawer 280 (`context.drawerWidth`).

Phone frame measurements:
- corner bubbles 44 × 44 at 20 / 20;
- content top padding 80;
- side insets 20;
- pill 26 from the bottom.

### 2.1 Activity rail (desktop)

- **Layout.**
  - Column, 56 wide, `bg`, 1px `border` on the right.
  - Padding 16 top and 14 bottom; gap 8.
  - From top to bottom: Notes, Agents, a flexible spacer, Status, Settings.
- **Rail item.**
  - 46 wide; padding 7 / 5; radius `rControl`.
  - A 20px icon (stroke 1.75) over a 10px label, with a gap of 3.
  - Inactive: `text3`, transparent background.
  - Active: `accent` icon and label on `accentSoft`.
  - Settings is icon-only (padding 8).
- **Icons** are Lucide-style: `file-text` (Notes), `square-terminal` (Agents), `settings-2` (Settings).
- **Agents badge.**
  - Shown only when sessions are live (running, starting or unknown).
  - Position top 3, right 6; min 15 × 15; pill-shaped; `accent` fill.
  - Text: `onAccent`, mono 10 / 500, the live-session count.
- **Status dot.**
  - A 9px dot inside a 40 × 32 hit area. Colour: `danger` if any health row is danger, otherwise `green`. (`green` here means "synced and nothing wrong"; see Q9.)
  - Click opens the **health popover**: bottom 58, left 64, width 280, `surface2`, 1px `border`, radius `rCard`, `shadow`, padding 12.
  - The popover lists the health rows (8px dot plus 13px text), then "About & health ›" in `accent`, which goes to Settings › About & health.
- **Navigation.**
  - Clicking an activity goes to that activity's last location.
  - The rail is always present at desk width, including in Settings.

### 2.2 Notes · sidebar (desktop)

Top block (padding 16 12 8, gap 8):

1. **Vault header.**
   - A row with padding 6 / 8, radius `rControl`, and `surface2` on hover.
   - Vault tile 30 × 30, then the name (14 / 600 `text`) over a sync line (mono 11 `text3`, "synced 2m ago"), then "▾".
   - Click opens the **vault switcher popover**:
     - position top 62, left 68, width 240; `surface2`, border, `rCard`, `shadow`, padding 8;
     - rows: tile 28, name 14, mono note count, and "✓" in `accent` on the current vault;
     - a divider;
     - the sync line "Synced 2m ago · Sync now" (mono 11 `text3`);
     - "Manage vaults ›" in `accent`, which goes to Settings › Vaults.
   - **"Server settings ›" is removed from this popover.**
2. **Search field.**
   - `surface2`, 1px border, `rControl`, padding 8 / 10.
   - A search icon, "Search {vault}" (13 `text3`) and "⌘K" (mono 11).
   - Opens the existing search sheet.

Scroll body (padding 4 8 12):

- **RECENT** (section label).
  - Up to 4 rows. These are the **cross-vault** recents from `recentsProvider`, most recent first.
  - Row: padding 6 / 8, radius `rControl`; the title is 13px, single-line with ellipsis; a **vault tag** on the right.
  - Selected (the open note): `text` on `surface2`. Otherwise: `text2`.
- **FOLDERS** (section label, padding-top 16). The folder tree of the selected vault.
  - Row: padding 5 / 8, with a left indent of `8 + 14 × depth`; 13px.
  - Folder rows: a chevron ▸/▾ in a 12-wide column, colour `text3`.
  - Note rows: the title. Selected: `text` on `surface2`.
  - **Unseen dot**: a 6px `accent` dot on any note changed by an agent since you last opened it, and on any collapsed folder that contains one. Tooltip: "Changed by an agent since you last opened it". Opening the note clears it.

Footer (1px top border, padding 10 / 12, gap 6):
- "＋ New note": primary, flexible width, 13 / 500.
- New folder: 36 wide, outline, folder icon.
- Tags: 36 wide, outline, "#".

### 2.3 Notes · main pane (desktop)

- **No note open:** a centred "Select a note, or press ⌘K to search" (14 `text3`).
- **Note view:**
  - The pane runs from the sidebar edge to the drawer (or to the window edge when the drawer is closed).
  - Inner column: max-width 720, centred, padding 28 40 80. That gives the 640 measure (`kEditorMeasure`, `kEditorInset`, `kEditorTopInset`).
  1. **Header row** (min-height 32, gap 10).
     - Crumb: `vault / folder / folder`, mono 12 `text3`.
     - Spacer.
     - **Read | Edit** segmented control (`NoteModeToggle`):
       - container padding 3, `surface2`, 1px border, radius `rControl × 0.8`;
       - segments padding 4 / 11, radius `rControl × 0.6`, 12.5px;
       - active segment: `accent` 600 on `accentSoft`; inactive: `text3` 500.
     - **Start session**: a soft action button.
       - Padding 6 / 11, `accentSoft` fill, `accent` text 13 / 500.
       - A filled 12px play icon and the label "Start session" (the phone label is "Session").
       - If no host is online, it opens Agents, which shows the first-session or no-host state, instead of the launcher.
     - Properties toggle: a 17px panel icon in `text3`.
  2. **Title.** Newsreader 34 / 600, line-height 1.15, padding-top 22, `text-wrap: pretty`.
  3. **Version line** (mono 12 `text3`, gap 10, wraps, padding 8 0 18).
     - `v{version} · Saved`, plus ` · id {id}` when "Show note id" is on.
     - When the note has provenance, add the **provenance link** in `accent`:
       - "Edited by session {name}, {age} ›", or "Created by session {name}, {age} ›" for a note the session created;
       - clicking opens that session with the **Wrote** tab selected.
  4. **Body.** Uses the existing editor and markdown view (gap 12).
     - Paragraphs: Newsreader 18, line-height 1.6.
     - H2: 22 / 600, padding-top 10.
     - List items: 18, line-height 1.55, disc bullets.
     - In Edit mode the markers (`## `, `- `) show dimmed in mono `text3`. The prototype only approximates the existing dim-markers editor; keep the real one.
  5. **Linked mentions.** Margin-top 40, 1px top border, padding-top 14. "Linked mentions {n} ▸" (13 `text3`), collapsed. This is the existing `mentions_section.dart`.
- **Properties drawer** (open by default; toggled by the header icon and the drawer's ×).
  - 280 wide, `surface`, 1px left border, padding 24 / 20, gap 16.
  - "PROPERTIES" label and "×".
  - Rows, each a mono 11 `text3` key over its value:
    - status (14 `text`);
    - tags (amber chips: mono 12 `amber` on `amberSoft`, radius `rControl`, padding 2 / 8, `#tag`);
    - color (the accent name);
    - **source** (`ai`) when present;
    - modified.
  - Use the existing `properties_panel.dart` and `note_properties.dart`.

### 2.4 Notes · phone

- **Vault root** (`/v/:vault/browse`).
  - Title: the vault name, 25 / 600.
  - RECENT section: cross-vault rows (padding 11 / 0, 1px bottom border).
    - Title 15 / 500 over "vault · folder · folder" (12 `text3`).
    - Age on the right (mono 11 `text3`).
    - Up to 4 rows.
  - FOLDERS section: the vault's top-level rows.
- **Folder** (`/browse/path`).
  - "‹ {parent}" back link in `accent` 14, then the folder name as the title.
  - Rows (padding 12 / 0, bottom border):
    - folders first: "▸", name 15, mono child count;
    - then notes: "·", title, mono age.
  - The unseen dot shows on both.
- **Note.**
  - Uses the same note view as the desktop, with padding 80 20 130.
  - The header has "‹ {parent folder or vault}" in `accent` in place of the crumb.
  - No drawer toggle. Properties keep using the existing phone sheet (see Q6).
  - The provenance link and Start session behave as on the desktop.
- **Pill** (shown on the vault root and folder screens only; see Q5 for the note screen).
  - Floating and centred; `surface`, 1px border, fully rounded, `shadow`, padding 6, gap 4.
  - Slots: Directory (44, folder icon; goes to the vault root), Search (44), **New note** (48, `accent` primary circle, "＋"), Tags (44, "#").

### 2.5 Phone chrome · corner bubbles and place picker

- **Left bubble** ("places").
  - 44 × 44, `surface`, 1px border, radius `rControl`, `text2`.
  - Content: the current vault tile (30) in Notes and Settings; the agents icon (20) in Agents.
  - A 10px dot at top −3, right −3, with a 2px `bg` ring:
    - `green` (synced) in Notes;
    - `accent` in Agents when sessions are live, otherwise `text3`.
  - Tap opens the **place picker** popover: top 72, left 20, width 250, `surface2`, border, `rCard`, `shadow`, padding 8.
    - NOTES label, then the vault rows: tile 28, name 15, and "✓" on the current vault when in Notes. Tapping a vault opens Notes at that vault's root.
    - A divider, then the **Agents** row: an agents icon tile 28 on `surface`, "Agents" 15, "{n} running" mono 11 `accent` when live, and "✓" when in Agents.
    - A divider, then the sync line "Synced 2m ago · Sync now".
- **Right bubble**: the Settings gear. The same bubble, with an `accent` icon and border while in Settings. It replaces the "A" bubble.

### 2.6 Agents · desktop sidebar

- Padding 16 12 8.
- Title "Agents" (14 / 600).
- **＋ New session**: primary, full width, padding 9. Shown only when at least one host is online.
- **Overview** row: padding 7 / 10, 13px. Selected (`text` on `surface2`) while no session is open.
- **RUNNING** section: live sessions (running, starting, unknown). **ENDED** section: completed, stopped and failed sessions. Each section is hidden when empty.
- **Session row.**
  - Padding 7 / 8, radius `rControl`, gap 10. Selected: `surface2`.
  - An 8px status dot.
  - Name: 13 `text` for live sessions, 13 `text2` for ended ones.
  - Sub-line: mono 11 `text3`, "{workspace} · {agent} · {age or status}", single-line with ellipsis.

### 2.7 Agents · main pane (desktop)

Which state shows:
1. No hosts enrolled → **No host**.
2. Hosts exist but there are no sessions → **First session**.
3. A session is selected → **Session detail**.
4. Otherwise → **Overview**.

- **Overview** (max-width 760, padding 28 40 60, gap 24).
  - Title "Agents" (25 / 600).
  - Intro (14 `text2`): "What your agents are working on. Sessions keep running on your hosts when you close Storm."
  - **WORK**: one card per (workspace, host).
    - Card: `surface`, 1px border, radius `rCard`, padding 16 / 18.
    - Header: workspace 15 / 600 and "on {host}" (mono 12 `text3`).
    - Session rows: padding 8, bleeding −8 so the hover fill (`surface2`) reaches the card edge.
      - Dot, name 14, meta (mono 12 `text3`, "{agent} · {age or status}").
      - Right side: a **context chip** when the session started from a note (12 `text2`, 1px border, radius `rControl`, padding 2 / 8, file icon and note title).
      - Then "wrote {n}" (mono 11 `accent`) when n > 0.
    - Clicking a row opens the session.
  - **START AN AGENT**: one card per agent (Claude Code, OpenCode, Shell).
    - Card: min-width 150, `surface`, 1px border, radius `rControl`, padding 10 / 14; `accent` border on hover.
    - Name 14 / 500 over a "where" line (mono 11 `text3`): the online host ids that offer it, or "{n} host offline".
    - Click opens the launcher with that agent preselected.
  - **Infrastructure line** (mono 12 `text3`): "{online} of {total} hosts online · {n} integrations, {m} needs sign-in · Settings ›". "Settings ›" is in `accent`; the whole line goes to Settings › Hosts & default agent.
- **First session** (vertically centred, left padding 64, max-width 460, gap 14).
  - "Start your first session" (25 / 600).
  - Copy (15 `text2`, line-height 1.55): "{host} is online with Claude Code and OpenCode. A session runs in one of its workspaces and keeps going after you close Storm."
  - START FROM A NOTE: up to 3 recent notes, each a card row (title 14 and vault tag) that opens the launcher with that note as context.
  - "New session" primary button, then "or start without a note" (13 `text3`).
- **No host** (same layout, max-width 480).
  - "Agents run on a machine you own" (25 / 600).
  - Copy: "Storm keeps each session running on that machine, so you can leave and pick it up from any device."
  - Three numbered steps. Each number is a 26px circle in mono 12; step 1 is in `accent`, steps 2–3 in `border` and `text3`.
    1. "Enroll a host", with an **Enroll a host** primary button that goes to Settings › Hosts.
    2. "Sign Claude Code or OpenCode in on that host".
    3. "Start a session".
- **Session detail.** See §3.6.

### 2.8 Agents · phone

- **List** (`/agents`).
  - Title "Agents" (25 / 600).
  - RUNNING and ENDED sections. Rows: padding 12 / 0, bottom border; dot, name 15 (ended rows in `text2`), sub-line mono 11.
  - Infrastructure line at the foot (mono 11), linking to Settings › Hosts.
  - **There is no work grouping on the phone**: the rows already carry the workspace.
  - Pill: a single labelled primary, "＋ New session" (height 48, padding 0 / 22). Shown only when a host is online.
- **No host**: the title, the copy "Agents run on a machine you own. Sessions keep going when this phone is off.", and the three steps without a button.
- **First session**: "{host} is online. Start from a note, or tap ＋.", then rows of "▶ {title}" with a vault tag, which open the launcher.
- **Session detail.** See §3.6.

### 2.9 Settings

See §5.

---

## 3. Agents model

### 3.1 Vocabulary (UI-facing)

| Term | Meaning | Today's code term |
|---|---|---|
| **Agent** | The AI worker you start: Claude Code, OpenCode, Shell. | *Provider* (`claude-code`, `opencode`, `shell`). The UI says "Agent"; keep "provider" in code and API. |
| **Session** | One run of an agent, in one workspace, on one host. | Agent session `ags_` |
| **Work** | Sessions grouped by (workspace, host) on the Agents overview. Derived; there is no new object. | — |
| **Workspace** | A directory on a host that a session runs in. | Workspace `(host_id, name)` |
| **Host** | A machine that runs agents. Lives in Settings. | Runtime Host `hst_` |
| **Kit agent files** | `kit/vault/agents/*.md` (Storm Lead, Coder…). Plain notes in the plain `kit` vault. **Never labelled "agent" in the UI**, and not offered in the launcher. | — |

Other user-facing renames:
- MCP keys → **Access keys**;
- auth sessions → **Signed-in devices**;
- the server host shown in the vault bubble → **Server address**.

"Session" in the UI only ever means an agent session.

### 3.2 Session status vocabulary

The status vocabulary is unchanged. This is how each status is drawn:

| Status | Label | Dot | Group |
|---|---|---|---|
| starting | Starting | `accent` **ring** (2px) | Running |
| running | Running | `accent` fill | Running |
| unknown | Unknown | `text3` fill | Running (live, input refused) |
| completed | Completed | `text3` | Ended |
| stopped | Stopped | `text3` | Ended |
| failed | Failed (with the end reason in the meta line) | `danger` | Ended |

Status chip: mono 11, the status colour, 1px border, fully rounded, padding 2 / 9, a 7px dot and the label.

Q8 asks whether `accent` is right for running.

### 3.3 Running and ended

- Live sessions are listed first, then ended ones.
- Ended sessions stay listed until dismissed.
- Nothing ever restarts on its own.

### 3.4 First-session and no-host states

See §2.7 and §2.8.

### 3.5 New-session launcher

Container:
- desktop: a modal 460 wide at top 96, centred horizontally, over a `rgba(0,0,0,.4)` scrim;
- phone: a bottom sheet with radius `rCard` on the top corners and padding 20 20 32;
- both: `surface`, 1px border, `shadow`, gap 14.

Contents, in order:

1. "New session" (18 / 600) and "×".
2. **Context** (only when started from a note).
   - Label CONTEXT, then a box: padding 10 / 12, `accentSoft` fill, 1px border, radius `rControl`.
   - The box holds an `accent` file icon, the note title (14), the crumb (mono 11 `text3`), and "×" to remove the context.
3. **Fields** in a 2-column grid (96px label column, gap 10). Labels are 13 `text2`; selects use `surface2`, 1px border, `rControl`, padding 9 / 12, 14px.
   - **Host**: online hosts only, shown as "{id} · online". The last used online host is preselected.
   - **Workspace**: that host's workspaces. If a live session already uses that workspace on that host, the option reads "{name}  (another session is working here)". This warns but does not block.
   - **Agent**: that host's agents, with the default marked "· default".
   - Changing the host resets the workspace to the first one, and the agent if the new host doesn't offer it.
4. **Can write to** (hidden for Shell).
   - A toggle; when it is on, a vault select appears.
   - The vault defaults to **the context note's vault**, or to the first vault when there is no context.
   - Toggle off: "Read only".
   - If Settings › AI access › "Allow writes when chosen at launch" is off, the toggle is disabled and shows "Off in Settings › AI access".
5. **Risk box** (12 `text3` on `surface2`, padding 10 / 12, radius `rControl`). Must stay visible (H12).
   - Non-shell: "Network: inherits {host}’s policy. This session can use your integrations (Linear, GitHub) and read your vaults, with that network access."
   - Shell: "Network: inherits {host}’s policy. A shell has no access to your vaults or integrations."
6. **Footer**: "The agent reads the note through Storm." (12 `text3`, only with context), then the **Launch** primary button.
   - Launch creates the session and opens its detail view: the Context tab if it has a note, otherwise About.
   - Any provider fallback must still be announced (H10).

Entry points: Start session on a note; ＋ New session (sidebar or pill); the agent cards on the overview; the first-session note rows; Run again.

### 3.6 Session detail

**Desktop.** A two-column grid (`1.1fr | 1fr`) with a 1px border between the columns.

- **Left column: the session.**
  - **Header** (padding 14 / 20, bottom border).
    - Name 16 / 600 and the status chip.
    - Live sessions: an **End** button (outline, 13 `text2`).
    - Ended sessions: **Run again** (outline, `accent` text) and **Dismiss** (text-only `text2`).
    - Meta line (mono 12 `text3`): "{agent} · {workspace} on {host} · started {age} ago", or the ended line for an ended session.
  - **Inline end confirmation** (H11). A box: margin 12 16 0, `surface2`, border, `rControl`, padding 12 / 14.
    - Text: "End {name}? The agent and everything it started are stopped on the host."
    - Buttons: Cancel, then **End session** (`danger` fill, `onAccent` text).
    - Confirming sets the status to stopped.
  - **Terminal.** The real terminal surface (xterm), mono 13, line-height 1.25, on `bg`, padding 10 on every side, with the height left under the last whole row split above and below it. Settings › Terminal can change all three (D15 AM47; was line-height 1.7, padding 16 / 20). The prototype's coloured lines are illustrative (§10).
  - **Ended footer** (mono 12 `text3`, top border): "{ended line} · scrollback kept until you dismiss it", for example "Completed 14:02 · ran 38 min".
- **Right column: the panel** (`bg`).
  - **Tabs** (padding 10 / 14, bottom border): **Context | Wrote {n} | About**. Each tab: padding 6 / 12, radius `rControl`, 13px. Active: `accent` 600 on `accentSoft`. Inactive: `text3` 500.
  - **Context**: the source note, read-only.
    - Crumb (mono 12), and "Open in Notes ›" (`accent`), which opens the note in Notes.
    - Title: Newsreader 26 / 600.
    - Version line, plus " · edited by this session" or " · created by this session" when this session wrote the note.
    - Body: the same styles as the note view.
    - With no context note: "Started without a note. Notes this session writes appear under Wrote." (14 `text3`).
  - **Wrote**: the notes this session created or edited.
    - Row: padding 10 / 12, radius `rControl`, `surface` on hover. Title 14 over the crumb (mono 11). Right side: "new", or "v{version}".
    - Clicking a row shows that note in the panel, with a "‹ Wrote" back link.
    - Empty, with writes off: "This session can’t write to your vaults."
    - Empty, with writes on: "Nothing written yet. Notes appear here as the session writes them."
  - **About**: key/value rows. Each key is a mono 11 uppercase label; each value is 14 `text`.

    | Row | Value |
    |---|---|
    | Agent | The agent name |
    | Workspace | "{workspace} on {host}" |
    | Vault access | "Reads all vaults. Writes to {vault}. Never deletes." / "Reads all vaults. No writes." / Shell: "None. Shell sessions have no Storm access." |
    | Integrations | "Linear, GitHub. Fixed when the session started." / "None" (Shell) |
    | Network | "Inherits {host}’s policy" |

**Phone.** Content padding-top 76.

- Header block (padding 0 20 10):
  - "‹ Agents" (`accent`) and the status chip;
  - the name (20 / 600);
  - a chip row:
    - context chip "▤ {note}": pushes the note full-screen, with "‹ {session}" as its back link;
    - "Wrote {n}": opens the details sheet;
    - "Details": opens the details sheet.
- Terminal: fills the screen, mono 12, line-height 1.25, padding 10, on `surface`, with a top border.
- **Extra-keys row** while the session is live: Esc, Tab, Ctrl, ←, →, a spacer, Paste. Keys are on `surface`, inside a `surface2` bar. Keep the existing row from `agents_screen.dart`.
- For an ended session: a bottom bar with **Run again** (primary) and **Dismiss** (outline), both 50% width.
- **Details bottom sheet**: a 36 × 4 grabber, max-height 75%, scrolls.
  - STARTED FROM (when present): a note row that pushes the note.
  - WROTE: rows that push the note.
  - The About rows.
  - **End session**: outline, `danger` text. Tapping it shows the confirmation copy with Cancel and End session (`danger` fill).
- There is **no split view on the phone**.

### 3.7 Run again and Dismiss

- **Run again** (ended sessions only) opens the launcher **prefilled** with the original session's context note, host, workspace, agent, write toggle and write vault. If the original host is offline, the first online host is used. The user still presses Launch.
- **Dismiss** (ended sessions only) removes the session from the lists and returns to the overview (desktop) or the list (phone). It is allowed only once a session has ended.

---

## 4. The knowledge → agent → knowledge loop

1. **Note.** The user reads a note in Notes, for example `personal / projects / storm / Gateway spec`.
2. **Start session.**
   - Desktop: the header button. Phone: "Session".
   - With no online host, it routes to Agents (first-session or no-host state).
3. **Launcher with context.** The CONTEXT box shows the note and its crumb, and can be removed with ×.
4. **Host / Workspace / Agent.** Defaults: the last online host, its first workspace, and the default agent.
5. **Write vault.** "Can write to" is on when AI access allows it, defaulting to the note's vault. It can be switched to another vault or turned off.
6. **Launch.** The session is created with its context, and the app navigates to the session detail with the **Context** tab open, showing the source note beside the terminal.
7. **Session runs.** The status goes from Starting (ring) to Running. The rail badge and the place-picker count update.
8. **Wrote.** As the session creates or edits notes **in its write vault**, they appear in **Wrote {n}**, in newest-write order. The overview row shows "wrote {n}".
9. **Resulting note.** Each written note:
   - shows the **unseen dot** in the tree (and phone rows) until you open it;
   - shows "**Edited by session {name}, {age} ›**" (or "Created by…") on its version line;
   - in the drawer, shows `source: ai` when the session created it.
10. **Back to the session.** The provenance link opens the session with the Wrote tab. "Open in Notes ›" in the panel goes the other way.

---

## 5. Settings

### 5.1 Hierarchy

```
This device            ← device-level (this client only)
Terminal               ← this client's terminals (D15 AM47)
Devices & access       ← you: devices and keys (server-stored, per account)
STORM                  ← Storm / server-level configuration
  Vaults
  AI access
  Integrations •       (dot = a connection needs attention)
  Hosts & default agent
  Storage
  Connection           (mixed: server address and identity, plus this device's route and disconnect)
  Advanced
──
About & health
```

The Accounts section is **removed**, including the registration toggle (single user).

### 5.2 Shell and layout

- **Desktop.**
  - The rail stays. The sidebar becomes the settings navigation:
    - padding 22 12 12;
    - title "Settings" (14 / 600);
    - items: padding 7 / 10, 13px; selected is `text` on `surface2`;
    - the group header "STORM" is a mono 11 uppercase label;
    - an unlabelled gap before About & health.
  - Page area: max-width 680, padding 28 40 60, gap 18.
  - Each page starts with its title (25 / 600) and an intro (14 `text2`).
- **Phone.**
  - The right bubble opens the **settings list**: title "Settings", the same items and order, rows 15px with padding 13 / 0, bottom borders and "›".
  - Each page pushes onto the stack with "‹ Settings" at the top, using the same content. **No Material AppBar.**
- **Rows.** Shared pattern: padding 12 / 0, 1px bottom border, gap 12. A label (14 `text`) with an optional sub-line (12 `text3`), and a control or action on the right. Actions are 13 `text2`.

### 5.3 Pages (exact copy)

**This device** · *Only affects this device.*
- APPEARANCE: three preset buttons (Storm dark, Storm light, SlowFlow earth).
  - Each button: an 18px swatch (`bg` fill with a 3px `accent` ring) and the label.
  - Selected: `accent` border on `accentSoft`. Otherwise: border on `surface`.
- "Text size" … "16 px". "Note font" … "Newsreader" (set in Newsreader).
- NOTES:
  - "Open notes in Read mode" / "Switch to Edit with ⌘E." (toggle);
  - "Show note id" / "Shown on the version line." (toggle).
- KEYBOARD: ⌘K Search · ⌘N New note · ⌘\ Sidebar · ⌘E Read / Edit.

**Terminal** · *Every agent session in this app. Kept on this device.* (D15 AM47)
- "Text size" … a stepper showing "Default" (each layout's own size) or "14 px", 10–20.
- LINE SPACING: chips Compact (1.15) · Default (1.25) · Relaxed (1.5).
- PADDING: chips Tight (×0.5) · Default · Roomy (×1.5).
- PREVIEW: a read-only terminal at the chosen values, on the terminal's own colours.
- "Reset to defaults", only once something differs.

**Devices & access** · *Where you’re signed in, and keys for AI apps outside Storm.*
- SIGNED-IN DEVICES: rows of name and meta (mono 11), for example "MacBook (this device)" / "active now" with **Sign out**, and other devices with **Revoke**. Then "＋ Add a device" (outline; shows the existing QR).
- ACCESS KEYS: "For AI apps outside Storm, such as Claude on a laptop. A key acts as you, and is shown once."
  - Rows: name and "stk_••••3f9a · used 1h ago", with **Revoke**.
  - "＋ New key" (outline). Uses the existing shown-once dialog.

**Vaults** · *Each vault is a folder of Markdown under the storage root.*
- Rows: tile 28, name, "{n} notes · {path}" (mono 11), **Rename**, **Remove**.
- A missing vault is greyed (opacity .7, dashed tile): "Directory not found. Nothing was deleted."
- Footnote: "Removing a vault takes it out of Storm. Its directory and every note stay where they are."
- "＋ New vault" (primary).
- Vault colour is still set here or by the existing long-press. See Q7.

**AI access** · *What AI apps outside Storm, and agents inside it, can do with your notes.*
- AI APPS OUTSIDE STORM:
  - "Let AI apps read your notes" / "Serves your vaults to apps that hold an access key." (toggle → `mcp_enabled`);
  - "Let them create, edit and delete" / "Storm has no trash. A deleted note is gone." (toggle → `mcp_writable`, external apps only);
  - "Access keys are in Devices & access ›" (`accent`).
- STORM AGENTS:
  - "Sessions can read your vaults" / "Every session except a shell." … "always";
  - "Allow writes when chosen at launch" / "You pick one vault per session. Agents never delete notes." (toggle → **new agent-write setting**, §9).

**Integrations** · *Services your agents can use, such as Linear or GitHub.*
- Info box (`surface`, border, `rControl`): "Connect a service once, here. Every agent session except a shell can then use it. Anything an agent can read, it can send elsewhere."
- Rows: name / detail, a status pill (mono 11, 1px border), and an action:
  - "Storm vaults" · "Built in. Read only unless a session is allowed to write." · Built in;
  - "Linear" · "Signed in · 14 tools on" · Connected · **Choose tools**;
  - "GitHub" · "Token expired 2 days ago" · **Needs sign-in** (`danger`) · **Sign in again**.
- "＋ Add integration" (primary). It replaces the Material FAB. The existing add, test, tools, enable and disconnect flows stay.

**Hosts & default agent** · *The machines agents run on.*
- Host rows: an 8px dot (`green` online, `text3` offline), the name, and the meta "{online now | offline · last seen 3h ago} · {agents} · network: host policy", with **Rename** and **Revoke**.
- With no hosts: "No hosts enrolled yet."
- "＋ Enroll a host" (primary). Uses the existing shown-once enrollment string. Followed by: "Enrolling shows a one-time string. Run `storm-runtime enroll` on the machine and paste it."
- DEFAULT AGENT: choice chips (Claude Code, OpenCode, Shell). Selected: `accent` border and text on `accentSoft`.

**Storage** · *Where your vaults live on the server.*
- VAULT STORAGE ROOT: the path in a mono box (`surface2`).
- "4 vaults live here. Changing the root doesn't move any files. Storm refuses a change that would leave vaults behind."
- "Change…" (outline).

**Connection** · *How this device reaches your Storm.*
- Rows: "Server address" … `storm.home:7420`; "Route" … `direct` or `relayed`; "Server identity" … `verified · 4F:2A:91…`.
- RELAYS: "Used when this device can't reach the server directly. A relay can't read your notes."
  - Rows with **Remove**, then "＋ Add relay".
- Divider, then "Disconnect this device" (`danger`) / "Forgets this server and its sign-in. Your notes stay on the server."

**Advanced** · *Details for troubleshooting and for connecting other tools.*
- "MCP endpoint" / "For AI apps that connect with an access key." … `{server}/mcp`.
- "Versions" / "Client and server." … `0.4.0 · 0.4.0`.
- "Re-pair this device" / "Scan a new pairing code without signing out." ›

The word "MCP" appears only here and in help text.

**About & health** · *Whether everything is working.*
- Health rows (8px dot, 14 text, `accent` action):
  - "Notes synced 2m ago" · Sync now;
  - "{n} of {m} hosts online" · Hosts (or "No hosts enrolled" · Enroll);
  - "GitHub needs you to sign in again" (`danger`) · Integrations;
  - "This client and the server are compatible".
- Footer: "Storm 0.4.0 · storm.home:7420" (mono 12).

### 5.4 Device-level vs Storm-level

| Device-level (client prefs, this device only) | Storm / server-level |
|---|---|
| This device (all of it); Connection › Route and Disconnect; the per-device last location and tabs; unseen state | Devices & access (stored per account on the server); Vaults; AI access; Integrations; Hosts & default agent; Storage; Connection › Server address, Identity, Relays; Advanced › MCP endpoint |

---

## 6. Responsive behaviour

There is one breakpoint, `kExpandedWidth = 900`. The phone layout is the default, and wide is additive (H5). The **same places exist on both**; only their layout differs.

| Concept | Desktop (≥ 900) | Phone (< 900) |
|---|---|---|
| Activity switch | Rail | Left corner bubble → place picker |
| Settings entry | Rail gear | Right corner bubble |
| Health | Rail dot → popover | Settings › About & health; bubble dot (sync) |
| Vault switch | Sidebar header → popover | Place picker › Notes |
| Recent | Sidebar, top | Vault root, top |
| Folders | Tree in the sidebar | Drill-down list with "‹ parent" |
| Note | Pane at a 640 measure, with the properties drawer | Full screen; properties use the existing sheet |
| Agents overview | Work cards grouped by workspace, plus agent cards | A flat Running / Ended list (rows carry the workspace) |
| New session | Sidebar button, agent cards | Pill "＋ New session" |
| Launcher | Centred modal, 460 | Bottom sheet |
| Session | Terminal \| panel split with Context / Wrote / About tabs | Full-screen terminal, a chip row, and a details sheet; the note is pushed full screen |
| End confirm | Inline bar | Inside the details sheet |
| Settings | Sidebar navigation and page | List → pushed page |

Tablets in portrait stay on the phone layout.

---

## 7. Design system (as used by the prototype)

**Use `tokens.dart` as-is. No new tokens are introduced.** The values below are what the prototype renders.

### 7.1 Colour tokens

| Token | Storm dark | Storm light | SlowFlow earth |
|---|---|---|---|
| bg | oklch(0.220 0.008 55) | oklch(0.920 0.008 55) | oklch(0.840 0.022 62) |
| surface | oklch(0.270 0.008 55) | oklch(0.870 0.008 55) | oklch(0.790 0.022 62) |
| surface2 | oklch(0.310 0.008 55) | oklch(0.830 0.008 55) | oklch(0.750 0.022 62) |
| border | oklch(0.360 0.008 55) | oklch(0.780 0.008 55) | oklch(0.700 0.022 62) |
| text / text2 / text3 | L 0.95 / 0.74 / 0.66 | L 0.18 / 0.34 / 0.40 | L 0.18 / 0.34 / 0.40 |
| accent | oklch(0.68 0.15 293) | oklch(0.40 0.15 293) | oklch(0.40 0.06 55) |
| accentSoft | oklch(0.300 0.050 293) | oklch(0.840 0.050 293) | oklch(0.760 0.020 55) |
| onAccent | oklch(0.16 0.03 293) | #ffffff | #ffffff |
| amber / amberSoft | 0.74 0.14 68 / 0.30 0.05 68 | 0.38 0.14 68 / 0.84 0.05 68 | 0.38 0.14 68 / 0.76 0.05 68 |
| green | oklch(0.74 0.13 148) | oklch(0.38 0.13 148) | same as light |
| danger | oklch(0.70 0.17 25) | oklch(0.38 0.17 25) | same as light |
| rCard / rControl | 16 / 10 | 16 / 10 | 2 / 10 |
| shadow | 0 12 42 −8 rgba(0,0,0,.35) | 0 7 24 −8 rgba(0,0,0,.20) | 0 5 19 −8 rgba(0,0,0,.16) |

**Vault tiles** use `Accent.tile(t)` from `accents.dart`: personal = sage, work = lavender, kit = clay. The initial is in mono, in a dark ink of the same hue. Tile sizes are 28 (lists) and 30 (headers), with radius `rControl × 0.8`.

**Semantic rules** (unchanged):
- accent = interactive, active and running;
- amber = tags only;
- green = synced / healthy;
- danger = failure / needs attention;
- text3 = offline, inactive or ended.

### 7.2 Typography

- **IBM Plex Sans** for chrome.
- **IBM Plex Mono** for labels, metadata, crumbs, counts and the terminal.
- **Newsreader** for note titles and bodies.

| Prototype size | Use | Token role |
|---|---|---|
| mono 11, uppercase, letter-spacing .08em, `text3` | Section labels (RECENT, WORK…) | `labelSmall` (`labelSize`, floor 11) |
| mono 10.5–12 | Tags, meta lines, crumbs, chips | `labelMedium` / `codeSize` |
| 12.5–13 | Sidebar rows, buttons, segmented controls, secondary copy | `bodySmall` (`codeSize` 12.8) |
| 14–15 | UI body, settings rows, phone rows | `bodyMedium` (`bodySize` 16 at default; see Q10) |
| 16–20 | Session name, sheet titles | `titleMedium` / `headingSize` 20 |
| 25 / 600 | Page titles (Agents, Settings pages, phone screens) | `headlineSmall`, nearest step (Q10) |
| Newsreader 34 / 600, line-height 1.15 | Note title | `displaySize` 31.25 (Q10) |
| Newsreader 26 / 600 | Note title in the session panel | `headingSize × scale` |
| Newsreader 18, line-height 1.6; H2 22 / 600 | Note body | The existing editor and markdown style |

### 7.3 Spacing, borders and radii

- Spacing is based on `sp` = 8. Common gaps are 4, 6, 8, 10, 12, 14, 16, 18, 24 and 28; desktop page insets are 28 / 40; phone insets are 20.
- Every border is 1px (`bw`) in `border`.
- `rCard` is for cards, popovers, the launcher, sheets and work cards.
- `rControl` is for inputs, buttons, rows, chips and corner bubbles.
- Fully rounded (999) is reserved for the pill, toggles, badges and status chips.

### 7.4 Controls

| Control | Spec |
|---|---|
| Primary button | `accent` fill, `onAccent` text 13–14 / 500, radius `rControl`, padding 8–10 / 14–20 |
| Outline button | 1px `border`, 13 `text` (or `text2`), radius `rControl`, padding 8 / 14 |
| Soft action (Start session) | `accentSoft` fill, `accent` 13 / 500, play icon, padding 6 / 11 |
| Danger button | `danger` fill, `onAccent` text |
| Text action | 13 `text2`, or `accent` for navigation actions ending in "›" |
| Toggle | Track 38 × 22, fully rounded. On: `accent` fill and border. Off: `surface2` with a `border` border. Knob 16 (`onAccent` on, `text3` off), slides in 160ms |
| Segmented / tabs | Active `accentSoft` with `accent` 600; inactive transparent with `text3` 500 |
| Select | `surface2`, 1px border, `rControl`, padding 9 / 12, 14px |
| Choice chip | Outline. Selected: `accent` border and text on `accentSoft` |

### 7.5 Navigation states

- Rail item: inactive `text3` on transparent; active `accent` on `accentSoft`.
- Sidebar row: inactive `text2`; selected `text` on `surface2`.
- Phone right bubble while in Settings: `accent` icon and border.

### 7.6 Status indicators

- Session dot: 8px, or a 2px ring for starting.
- Health dot: 8–9px.
- Unseen dot: 6px `accent`.
- Bubble dot: 10px with a 2px `bg` ring.
- Status chip: see §3.2.
- Integration status pill: mono 11, 1px border, fully rounded, coloured by state.

### 7.7 Panels and sheets

- Popover: `surface2`, border, `rCard`, `shadow`, padding 8–12. A transparent full-frame layer behind it closes it on outside click.
- Modal: scrim `rgba(0,0,0,.4)`.
- Bottom sheet: scrim `rgba(0,0,0,.35)`, a 36 × 4 grabber in `border`, max-height 75%.
- Properties drawer: `surface` with a left border.

### 7.8 Empty states

- A title (25 / 600), one or two sentences of copy (15 `text2`, line-height 1.55), and the next action.
- Numbered steps use 26px circles; only the current step is in `accent`.
- No illustrations. Follow the `states.dart` `EmptyState` voice.

---

## 8. Component inventory

Build these once and reuse them. ✓ marks an existing widget to extend rather than replace.

**Shell**
- `ActivityRail`, `RailItem` (icon, label, active state, badge)
- `RailStatusDot` and `HealthPopover`
- A shared **sidebar frame** for the Notes, Agents and Settings shells. This was deferred in decision 78; it is now needed by three shells.
- `CornerBubble` ✓ (`StormBubble`)
- `PlacePicker` popover
- `FloatingPill` ✓ (`nav_bubble.dart`) with `PrimaryCircle` and a labelled-primary variant

**Notes**
- `VaultHeader` and `VaultSwitcherPopover`
- `VaultTile` ✓ (`Accent.tile`)
- `VaultTag`
- `SectionLabel` ✓
- `SidebarRow` (recent / tree / session / settings variants, with `UnseenDot`)
- `NoteHeader` (crumb or back link, `NoteModeToggle` ✓, `StartSessionButton`, drawer toggle)
- `VersionLine` with `ProvenanceLink`
- `NoteBody` ✓
- `PropertiesDrawer` ✓
- `LinkedMentionsRow` ✓

**Agents**
- `SessionStatusDot` and `StatusChip` ✓
- `SessionRow`
- `WorkGroupCard`
- `AgentStartCard`
- `NoteContextChip`
- `InfraLine`
- `SessionHeader`
- `InlineConfirm`
- `TerminalSurface` ✓ and `ExtraKeysRow` ✓
- `SessionPanelTabs`
- `WroteList`
- `KeyValueList` (About)
- `SessionDetailsSheet` (phone)
- `NewSessionLauncher` (modal / sheet), with `LauncherField`, `ContextBox`, `WriteVaultField` and `RiskNote`

**Settings**
- `SettingsNav`, used for both the desktop sidebar and the phone list
- `SettingsPage` (title and intro)
- `SettingsRow` (label, sub-line, control or action)
- `StormToggle`
- `ChoiceChips`
- `PresetPicker`
- `InfoBox`
- `HealthRow`

**Shared**
- `PrimaryButton`, `OutlineButton`, `SoftActionButton`, `DangerButton`
- `Popover`, `BottomSheet`
- `EmptyState` ✓ with `NumberedSteps`

---

## 9. Implementation boundaries

**Legend**
- **B**: already supported by the current backend (staging `d864f8d`)
- **U**: UI-only (client work on existing data)
- **S**: requires new server/API work
- **P**: prototype-only placeholder

| Behaviour | Class | Notes |
|---|---|---|
| Vault list, notes, folders, tree, search, tags, mentions, properties | B | Existing |
| Cross-vault recents in the sidebar / vault root | U | `recentsProvider` already returns cross-vault rows |
| Activity rail, place picker, Settings as a rail destination, removing the dashboard | U | Router and shell work; mind H6 and H7 (Q2) |
| Vault switcher without "Server settings ›" | U | |
| Re-homing Server settings content into the Settings pages | U | Same endpoints. Owner-only writes still 403 for a non-owner; the single user is the owner |
| Removing Accounts / registration UI | U | The server's registration flag can stay (Q4) |
| Agents overview grouped by (workspace, host) | U | Derived from session records |
| "Start an agent" cards | U | Derived from hosts' providers |
| Session list, statuses, End, Dismiss, terminal, launcher host/workspace/provider | B | Existing |
| **Session routes** `/agents/s/:id` | U | Client router (deferred in decision 78) |
| Run again (prefill from the original session) | U / S | Host, workspace and provider are on the session record. Whether the grant's write flag and **write vault** are readable from the session API is unverified (Q3) |
| Workspace "another session is working here" | B | The existing shared-workspace warning |
| **Note-as-launch-context** | **S** | The launch API needs a `context_note` (vault id, note id). The server must record it on the session and expose it in session reads. How the agent receives it (initial prompt, env var, or an MCP resource through the gateway) is open (Q11) |
| **Vault write destination (one vault per session)** | **S** | Today the grant is a boolean `allow_vault_writes` across vaults. It needs `write_vault_id` on the grant, enforced by the gateway's built-in `storm` connection |
| **Wrote list per session** | **S** | Needs an API listing note ids created or edited by a session. The gateway records call metadata (`gateway.db.calls`, 30 days), but no per-session written-notes endpoint is known |
| **"Edited by session" provenance** | **S** | Needs the last agent writer (session id, created or edited, timestamp) per note, exposed on note reads. Today only the `source: ai` frontmatter convention exists. It must not add Storm state inside the vault (H1) |
| Unseen dot | U + S | A client-side last-opened version per note, compared with the provenance write. Depends on provenance |
| **Split AI-write permissions** | **S** | Today `mcp_writable` gates both external apps and agent vault writes. Add a separate agent-write setting, with `mcp_writable` covering external apps only |
| **Relay list in Connection** | **S** (likely) | No client screen exists. Whether the server exposes a relay-list read/write API is unverified; the PLAN lists "relay settings screen" as planned |
| Signed-in devices list with Revoke | **S** (likely) | There is no devices / auth-sessions list in the client today. An endpoint is unverified |
| Access keys, Add a device, Sign out, Disconnect | B | Existing |
| Integrations (add, test, tools, re-auth, disable, disconnect) | B | Existing on staging; restyle only |
| Hosts (list, enroll, rename, revoke) and default provider | B | Existing |
| Storage root, server identity, server address, route (direct / relayed) | B | Existing |
| Health aggregation (rail dot, About & health) | U | Aggregates existing sync, host and integration states. A **version-compatibility** signal does not exist (S, backlog) |
| Advanced › Versions | U / S | The client version exists; reading the server version needs an endpoint if one isn't exposed |
| Session naming (`gateway-spec` from the note title) | U / S | Whether the server or the client assigns the name is open (Q12) |

---

## 10. Prototype-only behaviour (do not mistake for backend capability)

- **Simulated session progression.** On Launch, timers move the session from starting to running at about 0.7s. They then append terminal lines ("read …", "updated BOARD: 2 items moved to Done", "created …/log/2026-10-08"), **edit BOARD** and **create a log note** at about 3.3s and 4.9s. Real sessions stream PTY output, and writes happen only when the agent chooses to make them.
- **Terminal content** is a list of styled text lines. Colours by prefix (`$`, `›`, `✕`, `✓`, and accent for "updated / created") are illustrative. The real view is the xterm surface.
- **Sample data:** the vaults (personal / work / kit), notes, sessions (docs-pass, test-sweep, lint-fix), hosts (build-vm online, mac-mini offline), integrations, devices, the access key, the relay, the missing `archive` vault, paths, versions, and every age ("2m ago").
- **Health rows** are static, except for the host counts.
- **No-op controls:** New note, New folder, Tags, Search, Sync now, Add a device, New key, Revoke, Rename, Remove, New vault, Add integration, Choose tools, Sign in again, Enroll a host, Change…, Add relay, Disconnect, Re-pair, the extra keys, and Linked mentions.
- **Text size and note font** are static labels.
- The theme picker in This device changes **only that frame**. The Tweaks "Agents state" is a demo switch, not a product state.
- In Edit mode, the note body shows literal `## ` and `- ` markers. Use the real editor.
- The default-agent chips and the AI-access toggles only change local state.

---

## 11. Acceptance checklist

**Navigation**
- [ ] Desktop shows the 56px rail on every screen: Notes, Agents, flexible spacer, status dot, Settings.
- [ ] Rail active state is `accent` on `accentSoft`. The Agents badge shows the live-session count only when it is above 0.
- [ ] Phone: the left bubble opens the place picker (vaults, Agents with "{n} running", sync line). The right bubble opens Settings. The "A" bubble is gone.
- [ ] No Home or dashboard exists on either device. Launch restores the last activity and location.
- [ ] No vault control contains "Server settings". The vault switcher ends with "Manage vaults ›".
- [ ] All old settings routes redirect (§1.4). The OAuth return still lands on Integrations.
- [ ] Android back unwinds pushed pages (note, folder, session, settings page, pushed note) without exiting early (Q2).

**Notes**
- [ ] The sidebar shows the vault header (tile, name, sync line), "Search {vault}", up to 4 cross-vault Recent rows with vault tags, and the folder tree.
- [ ] The phone vault root shows Recent (title, "vault · folder" line, age) above Folders.
- [ ] The note view keeps the existing editor, the 640 measure, Read | Edit, the properties drawer and Linked mentions.
- [ ] The note header shows **Start session**. With no online host, it opens Agents instead of the launcher.
- [ ] A note written by a session shows "Edited by session {name}, {age} ›" (or "Created by…") in `accent` on the version line. It opens that session's Wrote tab.
- [ ] The unseen dot appears on agent-changed notes and their collapsed folders, and clears when the note is opened.

**Agents**
- [ ] The sidebar shows ＋ New session (only with an online host), Overview, and Running / Ended sections, each hidden when empty.
- [ ] The overview shows WORK cards grouped by (workspace, host). Rows carry a context chip and "wrote {n}". START AN AGENT cards open the launcher with that agent. The infrastructure line links to Settings › Hosts.
- [ ] The no-host and first-session states match §2.7 / §2.8 copy. The phone shows a flat list with no work cards.
- [ ] Status dots and chips follow §3.2, including the starting ring and `danger` for failed.
- [ ] The UI never labels Kit files as agents. Claude Code, OpenCode and Shell are "Agents".

**Launcher**
- [ ] It opens as a modal (desktop, 460 wide) or a bottom sheet (phone).
- [ ] It shows the CONTEXT box when started from a note, removable with ×.
- [ ] Host (online only), Workspace (with the shared-workspace warning) and Agent (with the default marked) behave as in §3.5.
- [ ] "Can write to" defaults to the note's vault. It shows "Read only" when off, is disabled with "Off in Settings › AI access" when the setting is off, and is hidden for Shell.
- [ ] The risk box text matches §3.5 for shell and non-shell.

**Session**
- [ ] Desktop shows the split: the terminal column (header, meta line, End or Run again / Dismiss) and the panel with Context | Wrote {n} | About.
- [ ] End needs inline confirmation with the exact copy, and ends as stopped.
- [ ] Ended sessions show the ended footer. Run again prefills the launcher; Dismiss removes the session and returns to the overview.
- [ ] Context shows the source note read-only, with "Open in Notes ›". Without one, it shows the "Started without a note…" copy.
- [ ] Wrote lists created and edited notes ("new" or "v{n}"), opens them in the panel with "‹ Wrote", and shows the two empty-state copies correctly.
- [ ] About shows Agent, Workspace, Vault access, Integrations and Network, with the shell variants.
- [ ] Phone: a full-screen terminal with the chip row, the extra-keys row while live, the details sheet (Started from, Wrote, About, End with confirmation), and Run again / Dismiss bars when ended. A pushed note returns to the terminal.

**Settings**
- [ ] The hierarchy and order match §5.1. There is no Accounts section.
- [ ] Desktop keeps the rail, with the settings navigation in the sidebar. Phone has a list, then pushed pages with "‹ Settings" and no AppBar.
- [ ] Every page matches the §5.3 copy and controls.
- [ ] The AI access page shows two groups, and the agent-write toggle is independent of `mcp_writable`.
- [ ] The Integrations navigation item shows a `danger` dot when any connection needs attention.
- [ ] The theme preset change applies immediately and persists on this device.

**Responsive**
- [ ] One breakpoint at 900. Every place in §6 exists on both layouts; only presentation differs.
- [ ] Phone hit targets are at least 44 (bubbles, pill slots, rows).

**Core loop (end to end, on a real backend)**
- [ ] Note → Start session → launcher shows the note as context → Launch → session opens on Context.
- [ ] The agent receives the note context (Q11).
- [ ] With writes on to vault X, the notes the agent writes appear in Wrote, and only in vault X.
- [ ] Each written note shows its provenance link and the unseen dot, and the link returns to that session's Wrote tab.
- [ ] With writes off, Wrote stays empty with the "can’t write" copy, and no vault changes.

**Tokens**
- [ ] No literal colours, sizes or radii in `lib/ui/`; the token conformance test passes in all three presets.

---

## 12. Open implementation questions

These are deliberately not decided by the prototype.

1. **Q1 · Route scheme.** Confirm `/agents/s/:id` and `/settings/:page`, and the redirects from the retired routes.
2. **Q2 · Root and back-navigation.** Without a dashboard, what does `/` render on the phone? Is the last vault root the bottom of the back stack, and does back from there exit the app? This also replaces the desktop `notesHome()` forward and its back-navigation coupling (H7).
3. **Q3 · Run again data.** Does the session API expose the grant's write flag, the write vault and the context note, so Run again can prefill them exactly?
4. **Q4 · Single-user cleanup.** Should the server's registration flag, account picker and owner checks be removed, or only hidden in the client?
5. **Q5 · Phone pill on the note screen.** The prototype hides the pill on the phone note view. The current app shows a pill with a Mentions badge there. Keep the existing note pill or remove it?
6. **Q6 · Phone properties.** The prototype doesn't show how properties open on the phone. Keep the existing properties sheet and its entry point?
7. **Q7 · Vault colour entry.** The long-press-to-colour on dashboard vault cards disappears with the dashboard. Settings › Vaults has Rename and Remove but no colour action. Add "Colour" there, or put it in the switcher's long-press?
8. **Q8 · Running colour.** The prototype draws running and starting in `accent`. Confirm this against the existing `StatusChip` and `StatusDot` semantics.
9. **Q9 · Rail dot when healthy.** The prototype uses `green`, though green formally means "synced". Keep it, or use `text3` when everything is healthy and show colour only on problems?
10. **Q10 · Type sizes.** The prototype uses fixed sizes (13, 14, 15, 25, 34). Map them to the token roles in §7.2; where no role matches (25 page titles, 34 note title), confirm whether to use the nearest role or add a step.
11. **Q11 · Context delivery.** How does a launched agent receive the context note: an initial prompt, an env var, or a gateway resource?
12. **Q12 · Session naming.** Is the name derived from the note title (the client's slug) or assigned by the server? What should a session without a note be called?
13. **Q13 · Wrote ordering and edits by other sessions.** If two sessions edit the same note, the provenance link shows the latest writer only. Confirm this is intended.

---

## 13. Screenshots

All screenshots are in `screenshots/`. They were captured from the approved prototype with no changes, in Storm dark, at 2×: desktop 2560 × 1600 (a 1280 × 800 frame) and phone 780 × 1688 (a 390 × 844 frame). The data is sample data (§10).

**Desktop: Notes**

| File | State | README section |
|---|---|---|
| `desktop-01-notes-provenance.png` | BOARD open. The version line shows "Edited by session test-sweep, 2h ago ›"; Recent and the tree are visible | §2.2, §2.3 |
| `desktop-02-note-start-session.png` | Gateway spec open, with Read \| Edit, **Start session** and the properties drawer | §2.3 |
| `desktop-03-vault-switcher.png` | Vault switcher popover (vaults, sync line, Manage vaults ›) | §2.2 |
| `desktop-04-health-popover.png` | Rail status dot → health popover | §2.1 |

**Desktop: the loop and Agents**

| File | State | README section |
|---|---|---|
| `desktop-05-new-session-launcher.png` | Launcher with Gateway spec as context; Host, Workspace, Agent; Can write to: personal; risk box | §3.5 |
| `desktop-06-agents-overview.png` | Agents overview: WORK cards by workspace, context chips, "wrote n", START AN AGENT, infrastructure line | §2.7 |
| `desktop-06b-agents-first-session.png` | First-session state (host online, no sessions) | §2.7 |
| `desktop-06c-agents-no-host.png` | No-host state (three steps, Enroll a host) | §2.7 |
| `desktop-07-session-running.png` | Running session with no context note (docs-pass); the Context tab shows the "Started without a note" copy | §3.6 |
| `desktop-08-session-context.png` | Running session launched from Gateway spec, **Context** tab | §3.6, §4 |
| `desktop-09-session-wrote.png` | The same session, **Wrote 2** tab (BOARD, log/2026-10-08) | §3.6, §4 |
| `desktop-10-session-about.png` | The same session, **About** tab | §3.6 |
| `desktop-11a-session-completed.png` | Completed session (test-sweep): Run again, Dismiss, ended footer | §3.6, §3.7 |
| `desktop-11b-session-failed.png` | Failed session (lint-fix): `danger` status, failure line | §3.2, §3.6 |
| `desktop-11c-session-end-confirm.png` | Inline End confirmation on a running session | §3.6 |
| `desktop-loop-a-unseen-dots.png` | Back in Notes after the session: unseen dots on the written notes | §2.2, §4 |
| `desktop-loop-b-edited-by-session.png` | The resulting note (BOARD, v52): "Edited by session gateway-spec, just now ›" | §2.3, §4 |

**Desktop: Settings**

| File | State | README section |
|---|---|---|
| `desktop-12-settings-this-device.png` | Settings overview: the settings navigation and This device | §5.2, §5.3 |
| `desktop-13-settings-devices-access.png` | Devices & access | §5.3 |
| `desktop-14-settings-vaults.png` | Vaults, including the missing `archive` vault | §5.3 |
| `desktop-15-settings-ai-access.png` | AI access: AI apps outside Storm and Storm agents, as two groups | §5.3 |
| `desktop-16-settings-integrations.png` | Integrations, including the needs-sign-in row and the navigation dot | §5.3 |
| `desktop-17-settings-hosts.png` | Hosts & default agent | §5.3 |
| `desktop-18-settings-storage.png` | Storage | §5.3 |
| `desktop-19-settings-connection.png` | Connection, including Relays and Disconnect | §5.3 |
| `desktop-20-settings-advanced.png` | Advanced | §5.3 |
| `desktop-21-settings-about-health.png` | About & health | §5.3 |

**Phone**

| File | State | README section |
|---|---|---|
| `phone-01-notes-vault-root.png` | Notes vault root: Recent above Folders; corner bubbles and pill | §2.4, §2.5 |
| `phone-02-place-picker.png` | Place picker (vaults, Agents, sync line) | §2.5 |
| `phone-03-note-start-session.png` | Note with the "Session" button | §2.4 |
| `phone-04-new-session-sheet.png` | Launcher as a bottom sheet, with context | §3.5 |
| `phone-05-agents-list.png` | Agents list: Running and Ended, the infrastructure line, the New session pill | §2.8 |
| `phone-06-session-running.png` | Running session: chip row, terminal, extra-keys row | §3.6 |
| `phone-07-session-details-sheet.png` | Session details sheet: Started from, Wrote, About, End session | §3.6 |
| `phone-08-session-context-note.png` | Context note pushed over the session, with "‹ gateway-spec" as the back link | §3.6 |
| `phone-09-session-ended.png` | Completed session with the Run again and Dismiss bar | §3.6, §3.7 |
| `phone-10-settings-list.png` | Settings list (right bubble active) | §5.2 |
| `phone-11-settings-page-ai-access.png` | A pushed settings page (AI access), with "‹ Settings" | §5.2, §5.3 |

These screenshots were not taken: the light themes, the phone first-session and no-host states, the phone folder drill-down, and the phone Edited-by-session note. To see them, use the live prototype with its Tweaks (`Storm v2.dc.html`).
