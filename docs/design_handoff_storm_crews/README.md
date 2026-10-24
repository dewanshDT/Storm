# Handoff: Agent crews (Storm v2 Agents page)

## Overview
Crews let you group several agent sessions working toward one goal, for example "Gateway release" with an implementer, a reviewer and a release-notes session. A crew is a **label on the server**: it changes how the Agents page is organised. It doesn't change what a session can access, which host it runs on, or how it runs.

This builds on the approved Storm v2 design (`docs/design_handoff_storm_v2/` or wherever it was committed). Read that README first. Everything not mentioned here (the Activity Rail, session view, terminal, Details panel, launcher, Settings) stays as v2 specifies.

**Scope**
- **V1, build this:** crews, membership, optional roles, collapsible crew sections, the crew page, drag and drop, multi-select, the phone long-press sheet, and the Crew and Role rows in the launcher.
- **Phase 3, do NOT build:** agents starting other agents, parent/child lineage, the approval prompt, and the "Let agents start agents" setting. These appear only in frames labelled "Phase 3 preview" and must not ship in any form, including disabled or hidden-behind-a-flag UI.

## About the design files
The `.dc.html` files are **design references built in HTML**. They're not code to port. Recreate the design in the Flutter client using its existing widgets, tokens (`lib/ui/tokens.dart`) and patterns. Open `Storm Agent Crews.dc.html` in a browser (it needs `support.js` next to it):
- **Round 3** (top of the page) is the **approved direction**. Frames 3a–3p.
- Rounds 2 and 1 below it are **superseded explorations**. Don't implement them: no board, no tree.

`Agents Crews Frame.dc.html` renders each frame. Its `scene` prop selects the state. It's read-only sample data, not a working prototype.

## Fidelity
**High fidelity.** The frames use the real v2 tokens (Storm dark), type, spacing and components. Match them, and use the token names below rather than literals (decision 43: no literals in `lib/ui/`).

---

## Product model

| Concept | Rule |
|---|---|
| **Crew** | A named, coloured group of sessions. Server-backed, so every device sees the same crews. It owns no notes, tasks or plans. |
| **Membership** | A session belongs to **at most one** crew, or none. You can move it in, out or between crews at any time. Any sessions can be grouped, whether or not they're related. |
| **Role** | Optional. A session is a **plain agent** unless a role is set. Roles are **not unique seats**: a crew can have several sessions with the same role. |
| **Role source** | The role list is **every note in Kit's `agents/` folder, read live**. Nothing is built in. Lead, Coder and Reviewer exist only because Kit is seeded with those notes. A role is stored as the **note id**, so renaming the note renames the role everywhere. |
| **Role at launch** | Choosing a role in the launcher gives the agent that role's Kit note as **extra context**, alongside any context note. |
| **Role after launch** | Changing a role later only changes the **label**. The running agent keeps its original context. |
| **Who set the role** | Stored as `me` or `agent`. In V1 it's always `me`; `agent` only exists once Phase 3 ships. |
| **Deleted role note** | Sessions keep the tag, struck through and dimmed, with "Role note was deleted" in About. You can clear it or choose another role. |
| **Ended sessions** | Stay in their crew, after the running ones and dimmed, until you dismiss them. |
| **Lineage** (Phase 3) | Who started whom. This is a **different relationship** from crew membership; don't model one as the other. |

### Server/API (new work)
- `Crew { id, name, color (vault palette word, e.g. "sage"), order }`
- `CrewMember { crew_id, session_id (unique), order, role_note_id?, role_set_by: "me"|"agent" }`
- Endpoints:
  - list crews and members (synced like sessions);
  - create (with initial members);
  - rename, recolour, reorder;
  - add, move or remove a member;
  - reorder members;
  - set or clear a role;
  - delete a crew.
- Launch: accept an optional `crew_id` (or a new-crew name) and `role_note_id`. With a role, the runtime adds the role note to the agent's context the same way as the context note. This depends on the v2 "note-as-launch-context" work.
- End all running and Dismiss all ended can loop over the existing per-session calls.
- The crew page's "Started from" and "Wrote" lists are **assembled from each member's existing context note and Wrote list**. They depend on the v2 server work for those; a crew stores nothing extra.
- Owner-only, like all agent features.

---

## Screens

Desktop shell, unchanged from v2: Activity Rail 56px · sidebar 260px (220px at the 900px breakpoint) · main pane. Phone: corner bubbles 44px at 20px insets, and the pill.

### 3a · Agents overview (desktop): mixed state
**Sidebar**, top to bottom:
- "Agents" title, the "＋ New session" primary button, the Overview row (all unchanged).
- **CREWS** section label: Plex Mono 11px, letter-spacing .08em, uppercase, `text3`, padding 12/8/6.
- **Crew header row:** padding 7px 8px, radius `r-control`, gap 8px. Contents:
  - chevron ▾/▸ (12px wide, `text3`, 10px);
  - colour square 10×10, radius 3, `oklch(0.62 0.085 <palette hue>)`;
  - name (13px/500, `text`, ellipsis, takes priority);
  - count (Mono 11px, `text3`, never shrinks). Open: "2 running", or "2 run" at 900px. Collapsed: "1 · 2", meaning running · ended, at every width.

  Selected header: `surface2` background.
- **Members** (when open): a column with gap 2px, margin-left 13px, padding-left 6px, and a **1px solid `border` left edge** (this is the membership mark).
- **Session row** (the same row everywhere, as in v2):
  - padding 6px 8px, gap 10px, radius `r-control`;
  - agent mark 28px (rounded square `surface2` with a provider glyph, and a status badge dot bottom-right: `accent` running, `danger` failed, none when ended);
  - name 13px `text`, plus an optional role tag;
  - sub-line Mono 11px `text3`: **`{workspace} · {age or status}`**, e.g. "storm · 40m", "site · completed".
  - Ended rows: opacity .62. Selected: `surface2`.
- Then the **RUNNING** and **ENDED** sections for sessions that aren't in a crew (the v2 list, unchanged). Gap between rows is 2px.

**Main pane** (max-width 820, padding 28/40):
- "Agents" title (25/600) and the v2 intro line.
- **CREWS** label, then one card per crew. Card: `surface`, 1px `border`, radius `r-card`, padding 14/18.
  - Header: 12px colour square, name 15/600, Mono 12 summary "2 running · 1 ended" (the **long** form, on cards only), and ··· on the right (`text3`) that opens the crew menu.
  - **Open card:** member rows with mark 28, name 14, role tag, Mono 12 meta "`{workspace} · {age}`", and on the right the context-note chip (note icon + title, 12px, 1px border) and "wrote n" (Mono 11, `accent`).
  - **Collapsed card:** overlapping member marks (24px, -8px overlap) and "Collapsed · name, name, …" (13px `text3`).
- **NOT IN A CREW** label, then one card holding the loose session rows (same row spec).
- The v2 infrastructure footer line, unchanged.
- This **replaces the v2 overview's grouping by workspace**; the workspace now shows on each row.

### 3c · Crew page (desktop)
Opened by clicking a crew header in the sidebar, or a crew card's title.
- **Header:** colour square 18px, name 25/600, then on the right "＋ New session in crew" (`accent-soft` background, `accent` text, 13/500) and ··· (1px border, radius `r-control`). Under it, Mono 12 `text3`: "2 running · 1 ended · build-vm · storm, site" (counts, distinct hosts, distinct workspaces).
- **Members card:** `surface` card with rows, meta "`{workspace} on {host} · {age}`", running first and ended dimmed.
- **Two columns** (1fr / 1.3fr, gap 28):
  - **STARTED FROM:** the distinct context notes of the members, each a bordered row with a note icon, title, and "n sessions".
  - **WROTE:** every note the members wrote: title 14, crumb Mono 11, and on the right "v53 · gateway-impl" or "new · release-notes".
- Footnote (12px `text3`): "Collected from each member's context note and Wrote list. The crew itself holds no notes."
- **Crew menu (···):** popover 230px, `surface2`, `r-card`, shadow. Items, in order:
  - Rename;
  - Colour (6 palette swatches; current one ringed in `accent`);
  - divider;
  - Collapse in sidebar;
  - divider;
  - Dismiss ended (count);
  - End all running… (count; `danger` text).

  End all running opens a confirmation that **lists every session it will stop**, using the v2 copy: "The agent and everything it started are stopped on the host."

### 3e · A crew member's session (desktop)
The v2 session view, unchanged except:
- The meta line under the title adds a **crew chip** (9px colour square + crew name, 12px `text2`, 1px border, radius `r-control`) and the **role tag**.
- **About** gains:
  - **Crew** ("Gateway release", with "Move to crew ▾" in `accent`);
  - **Role** ("reviewer · set by you at launch. The agent read kit / agents / Reviewer.", with "Change role ▾").
- The Details panel is still the closable, resizable 320px panel with the toggle in the header. **No third column.**

### Role tag
- Plex Mono 10.5px, line-height 16px, `text2`, padding 0 5px, radius 5px, 1px outline. Text is the role note's title, lowercased (e.g. `reviewer`).
- **Solid `border` outline:** you set it. **Dashed `text3` outline:** an agent set it (Phase 3 only).
- **Neutral only.** No colour from Kit notes. The crew colour square is the only colour a crew adds.
- Shown only when a role is set. Plain sessions show nothing.

### 3g · Drag and drop (desktop)
- **Drop target is a loose session:** creates a crew with both sessions. The target row gets an `accent-soft` background with a 1.5px `accent` inset ring, plus a floating "Make crew" pill (11px/500, `accent` on `on-accent`).
  - The dragged row's original spot dims to .3. The ghost is the row on `surface2`, rotated -2°, with a shadow.
  - After the drop, the crew name field opens inline for editing. It's prefilled with the shared context note's title, otherwise "New crew".
- **Drop on a crew header or member:** joins that crew.
- **Drop between rows in a crew:** reorders.
- **Drop on the Running or Ended section:** leaves the crew.
- **Crew headers** can be dragged to reorder crews.
- Dropping **never sets a role**.

### 3i · Multi-select (desktop)
⌘-click or Shift-click rows to select them (`accent-soft` background, 18px `accent` check on the right). A floating bar appears at the foot of the sidebar: "2 sessions selected", then **Make crew** (primary), **Move to…** and **Esc**. Esc clears the selection.

### 3o · Launcher: Start work from a note
This is the v2 launcher with **two new rows** after "Can write to":
- **Crew:** options are None, "New crew: {context note title}" (only when there's a context note), then the existing crews, each with its colour square. **Default: None.**
- **Role:** None · a plain agent (default), then every Kit `agents/` note. If Kit has no role notes, the row shows only None, plus "Add roles in Kit ›".

Opened from a crew page, Crew is preset to that crew. With a role chosen, the footnote reads "The agent reads {note} and the {Role} role note from Kit."

**Every dropdown shows a trailing chevron** (14px `text3`, right-aligned) so it reads as a picker. This also applies to the existing v2 launcher fields.

### 3j · No crews · 3k · Loading · 3l · 900px
- **No crews:**
  - The sidebar is the plain v2 Running / Ended list.
  - The overview shows one hint above the sessions (13px `text2`, 1px dashed border, radius `r-control`): "Working on one thing across several sessions? Drag one session onto another to make a crew, or pick a crew when you launch." The hint disappears after the first crew is made.
  - With no sessions at all, the v2 first-session and no-host states apply unchanged.
- **Loading:** skeletons without shimmer. **Crew headers render only after crews have loaded**, so sessions never jump from the loose list into a crew on screen.
- **900px:** the sidebar is 220px and the open count reads "2 run". Names ellipsize, while marks and tags keep their size. The Details panel can be closed.

### Phone
- **3b · List:** the same order as desktop.
  - Crew rows: 12px square, name 15/500, Mono 11 "2 running · 1 ended", ▾/▸.
  - Members are indented with a **5px margin and a 1px solid left edge**.
  - Rows: mark 32, name 15, sub-line Mono 11.
  - The pill shows "＋ New session".
- **3d · Crew screen:** pushed onto the stack, and the **left bubble becomes a back chevron** (as on the v2 session screen). Header: 16px square, name 23/600, ··· (40×36 bordered) that opens the crew menu as a sheet. Below: member rows, STARTED FROM chips, the WROTE list. The pill shows "＋ New session in crew".
- **3f · Long-press a session** (the phone's replacement for dragging), which opens a sheet:
  - the name and status;
  - **CREW:** existing crews, "Not in a crew" (✓ when current), and "＋ New crew…";
  - **ROLE · FROM KIT:** chips, with None preselected;
  - the note "Setting a role now only changes the label. The agent keeps the context it started with.";
  - End session.

  The same Crew and Role controls are also in the v2 session details sheet.
- **3h · Launcher sheet:** the v2 sheet plus the Crew and Role rows, both with chevrons.
- **No drag and drop on the phone.**

### Phase 3 preview: do not build (3m, 3n, 3p)
These are shown only for future-proofing; each frame carries a "Phase 3 preview · not in V1" badge.
- **Lineage** looks different from membership: a child row sits on a **dashed `text3` left indent** under its parent, with the sub-line "↳ from {parent}" and "↳ started by {parent} ›" in the session header. Its role tag is dashed.
- **The "Let agents start agents" setting** in Settings › AI access is global, off by default, and separate from any future per-session setting (which could only narrow it). **In V1 this row must not exist.**

Leave room in the data model for a nullable `parent_session_id` on sessions, but add no UI for it.

---

## Design tokens (Storm dark; derive the others with `tokens.dart`)
| Token | Value |
|---|---|
| bg | `oklch(0.22 0.008 55)` |
| surface | `oklch(0.27 0.008 55)` |
| surface2 | `oklch(0.31 0.008 55)` |
| border | `oklch(0.36 0.008 55)` |
| text / text2 / text3 | `oklch(0.95 …)` / `oklch(0.74 …)` / `oklch(0.66 0.008 55)` |
| accent / accent-soft / on-accent | `oklch(0.68 0.15 293)` / `oklch(0.30 0.05 293)` / `oklch(0.16 0.03 293)` |
| green / danger | `oklch(0.74 0.13 148)` / `oklch(0.70 0.17 25)` |
| r-card / r-control | 16px / 10px |
| shadow | `0 12px 42px -8px rgba(0,0,0,.35)` |
| Crew colours | Vault palette words. Swatch `oklch(0.62 0.085 <hue>)`, e.g. sage 135, sky 210, rose 25, sand 70, violet 290, berry 340 |

Fonts: IBM Plex Sans (UI), IBM Plex Mono (labels, meta, tags, counts), Newsreader (notes only).

## Acceptance checklist
- [ ] Crews persist on the server and appear identically on two devices.
- [ ] A session can't be in two crews. Moving it removes it from the old crew.
- [ ] Sidebar order: CREWS (running before ended inside each) → RUNNING → ENDED. Rows have a 2px gap.
- [ ] Collapsed headers show "r · e" and never truncate a name shorter than about 18 characters at 260px. Cards show the long form.
- [ ] The workspace is visible on every session row.
- [ ] The role list matches Kit `agents/` notes live: add, rename or delete a note and the list updates. A deleted role shows struck through.
- [ ] Launching with a role passes the role note as context. Changing it later doesn't affect the running agent.
- [ ] Role tags are neutral (no colour). The solid outline is the only V1 style.
- [ ] Drag: onto a loose session makes a crew; onto a crew joins it; onto Running or Ended leaves it; drag to reorder members and crews. Dropping never sets a role.
- [ ] Multi-select bar: Make crew, Move to…, Esc.
- [ ] Crew menu: Rename, Colour, Collapse, Dismiss ended, End all running (confirmation lists the sessions).
- [ ] The crew page shows Started from and Wrote, assembled from the members.
- [ ] Phone: long-press sheet for crew and role, the back chevron on the crew screen, no drag.
- [ ] Every launcher dropdown has a chevron.
- [ ] No Phase 3 UI anywhere: no lineage indent, no "started by", no spawning setting.
- [ ] Everything else in the Agents page matches v2.

## Open questions (not designed; ask before building)
1. **Collapse state:** remembered per device, like the Details panel width, or synced? The frames assume per device.
2. **Empty crew:** when every member is dismissed, delete the crew automatically (with a few seconds of Undo) or keep it?
3. **Crew name when dragging** two sessions with different context notes: use the target's note title, or "New crew"?
4. **Sidebar crew header:** there's no ··· there yet, so crew actions need the crew page or the overview card. Proposed: a ··· that appears on hover, plus right-click, opening the same menu. Not drawn.

## Files
- `Storm Agent Crews.dc.html`: the design page. Round 3 (frames 3a–3p) is approved; Rounds 2 and 1 are history.
- `Agents Crews Frame.dc.html`: the frame renderer (`scene` prop: overview, crew, session, drag, select, launch, nocrews, loading, narrow, phase3, settings, p-list, p-crew, p-sheet, p-launch, p-phase3).
- `support.js`: the runtime needed to open the `.dc.html` files.
