# Handoff: Agent crews (Round 4)

> **Locked as decision 138 (2026-10-25).** Where this README differs from the decision, the decision and `docs/design/AGENT_CREWS_IMPLEMENTATION_PLAN.md` §2 win: the open questions in §7 are decided, roles live on the session (not the membership), the Role row follows the agent's Storm connection rather than its name, crew swatches use `Accent.tile`, V1 adds no Phase 3 schema, and §6 is corrected by 138p to match decisions 134–137.

## Overview
A **crew** groups agent sessions working toward one goal. It is a **server-side label**: it changes how the Agents page is organised, and nothing about what a session can access or where it runs.

This builds on the shipped Storm v2 client and its handoff at `design_handoff_storm_v2/` (repo root). Everything not mentioned here stays as built.

**Scope**
- **V1, build this:**
  - crews and membership;
  - optional roles from Kit;
  - collapsible crew sections in the sidebar, crew cards on the overview, and the crew page;
  - drag and drop and multi-select on desktop; long-press on phone;
  - Crew and Role in the launcher;
  - every state in §5.
- **Phase 3, build after crews V1 ships** (§6, frames 4m–4w, badged "Phase 3 · ships after crews V1"): agents asking to start, message and end sessions; approvals; lineage; Apply. Phase 3 is a separate PR series and must not be merged into crews V1. The only V1 preparation is a nullable `parent_session_id`.

## About the design files
The `.dc.html` files are **design references in HTML**, not code to port. Rebuild the design in the Flutter client with its existing widgets and `lib/ui/tokens.dart`. Open `Storm Agent Crews.dc.html` in a browser, with `support.js` next to it.
- **Round 4 (frames 4a–4s) is the spec.** `Crews R4 Frame.dc.html` renders them; its `scene` prop picks the state.
- Rounds 3, 2 and 1 are history. `Agents Crews Frame.dc.html` renders Round 3 and contains values the shipped app doesn't use; don't build from it.

## Fidelity
High fidelity, in the shipped tokens. Use token names (decision 43): `surface, surface2, border, text, text2, text3, accent, accentSoft, onAccent, danger, green, amber, rCard, rControl`. 4a, 4b and 4c show the same screen in Storm dark, Storm light and SlowFlow earth.

---

## 1. Model
| Concept | Rule |
|---|---|
| Crew | `{ id, name, color (palette word), order }`. Owns no notes or tasks. Owner-only. |
| Membership | A session is in **at most one** crew. Move it in, out or between crews at any time. Unrelated sessions can share a crew. |
| Role | Optional, on **any** session, in a crew or not. Not a seat: several sessions can share a role. |
| Role source | Notes in Kit (see open question 5). Stored by **note id**: a renamed note's tag follows the new title. A deleted note leaves the tag **struck through**. |
| Launch role vs label | The agent receives the role note **only at launch**. A later change only sets the label. A resumed session keeps its launch role. When the two differ, About shows both: "Launched as storm reviewer (the agent read this note) · Label: sop reviewer". |
| Agents that can't read notes | Shell, Cursor and Command Code. In the launcher the Role row is disabled, with "Shell can't read Storm notes. A role would only be a label." "Can write to" stays hidden, as today. A role set later is a label only, and About says so. |
| Who sets a role | Only a person. No agent sets roles, so every tag is drawn the same, with a solid outline. |
| Dismiss | Deletes the session, so it also leaves its crew. |

### Server/API (new)
- `Crew` and `CrewMember { crew_id, session_id (unique), order, role_note_id?, launch_role_note_id? }`.
- Endpoints: list (polled with sessions, about every 15 s); create (with members); rename, recolour, reorder; add, move or remove a member; reorder members; set or clear the role label; delete.
- Launch accepts `crew_id` (or a new crew name) and `role_note_id`. With a role, the role note joins the agent's context the same way as the context note.
- End all running and Dismiss ended loop over the existing per-session calls.
- The crew page's **Started from** and **Wrote** are assembled from each member's existing context note and Wrote list. Both are already built; a crew stores nothing extra.

## 2. Shipped values these frames follow
- **Session row sub-line:** `{host} · {workspace} · {age or status}`, e.g. "build-vm · storm · 40m". Host and age never shrink; the workspace truncates first.
- **Agent mark:** 32px in the desktop sidebar, 36px on phone, 28px in overview and crew-page card rows.
- **Status badge,** bottom-right of the mark:

  | State | Badge |
  |---|---|
  | working | `accent`, pulsing (opacity 1 → .3, 1.6 s) |
  | idle | none |
  | starting | `text3` |
  | unknown (host offline) | `amber` |
  | failed | `danger` |

- **Ended rows:** only the mark fades (opacity .5), and the name turns `text2`. The rest of the row is unchanged.
- **Sidebar width:** `clamp(260, window × 260/1200, 400)`, so 277 at 1280 and 260 at 900. There's no narrower variant.
- **Overview:** max width 760.
- **Details panel:** 320 by default, resizable from 260 to 500. Open state and width are kept in memory only.
- **End:** ending one session shows an inline banner in the session header. End all running uses the same banner on the crew page, or a sheet on phone.
- **Crew colours:** the vault palette words, with hues coral 25, peach 50, clay 70, sand 85, sage 135, mint 165, sky 230, lavender 290, blossom 340. The swatch is `oklch(L C hue)`:
  - L = .70 on dark, .62 on light, .58 on earth;
  - C = .10, or .05 for clay; ×.85 on earth.

  The colour menu shows all nine.

## 3. Components
### Role tag
Mono 10.5/16, `text2`, 1px `border` outline, radius 5, padding 0 5. Text is the note title, lowercased (e.g. `storm reviewer`).
- **Width:** 5ch minimum, about 14ch maximum (`max-width: 15ch`), with an ellipsis.
- **Shrinks before the name:** the tag has `flex-shrink: 1000` and the name has 1.
- **Deleted note:** line-through, opacity .6.
- **No colour,** whatever the role note says.

### Sidebar crew header
padding 7/8, radius `rControl`, gap 8. Contents:
- chevron (12px wide, `text3`);
- 10px colour square, radius 3;
- name (13/500, ellipsis, takes priority);
- count in mono 11 `text3`: "2 running" when open, "2 · 1" (running · ended) when collapsed.

On hover, a 22×20 **···** appears; it and **right-click** open the crew menu (see open question 4). Members sit in a column with gap 2, margin-left 13, padding-left 6, and a **1px solid `border` left edge**.

**CREWS label:** has a ▾. Collapsing it folds the section to "CREWS · n".

### Crew menu
220px, `surface2`, `rCard`, shadow. Items:
- Open crew page
- Rename
- Colour (the nine swatches)
- Collapse
- divider
- New session in crew
- Dismiss ended (count)
- End all running… (count, `danger`)

### Overview (max 760)
1. **CREWS:** one card per crew. Card: `surface`, `rCard`, padding 12/18.
   - Header: 12px square, name 15/600 (ellipsis), mono summary "3 running · 1 ended", and ···.
   - Open card rows: mark 28, name 14, role tag, the sub-line, a context-note chip (max 150, ellipsis) and "wrote n" in `accent`.
   - Collapsed card: stacked marks (24px, overlapping by 8) and "Collapsed · names…".
2. **NOT IN A CREW:** today's cards grouped by workspace and host ("storm on build-vm").
3. **START AN AGENT:** today's cards, unchanged, at the bottom.
4. The infrastructure line.

### Crew page
- **Header:**
  - square 18;
  - name 25/600, which **wraps** (no ellipsis);
  - "＋ New session in crew" (`accentSoft`);
  - ···.
- **Summary:** "3 running · 1 ended · build-vm, mac-mini". It names the crew's hosts, and rows keep their own host (open question 6).
- A members card, then **Started from** and **Wrote**.

### Session (crew member)
The shipped session view, plus:
- in the meta line, a crew chip (9px square and name, max 220px, ellipsis) and the role tag;
- in About, **Crew** (with "Move to crew ▾") and **Role** (launch role and label; "Change label ▾").

### Launcher
Two new rows after "Can write to": **Crew** and **Role**. Every dropdown has a chevron.
- **Crew:** None by default; "New crew: {context note}"; the existing crews.
- **Role:** None by default (a plain agent), then each role note. Each shows its **title, with its one-line summary under it**, grouped by folder when Kit has `projects/<name>/agents/`.
- **Optional quiet line,** when a write vault is granted: "The role note may tell the agent not to write." Drop it if it's noisy.

### Phone
- **List:** the same order as desktop, with 36px marks. Members sit behind a solid left edge (5px margin, 12px padding).
- **Crew page:** pushed onto the stack, with the back chevron in the corner bubble. ··· opens the actions as a sheet.
- **Long-press a session:** a sheet with two independent, separately headed sections:
  - **Crew:** Not in a crew, the crews, and New crew.
  - **Role:** with summaries, plus the note "launched as X and keeps that note; this changes only the label".
- **End all running:** a sheet listing Stops, Can't reach now and Already ended.
- No drag and drop on the phone.

## 4. Desktop interactions
- **Drag one loose session onto another:** makes a crew. The name field opens with "New crew" selected (open question 3).
- **Drag onto a crew header or member:** joins that crew.
- **Drag between members:** reorders them.
- **Drag onto Running or Ended:** leaves the crew.
- **Drag a crew header:** reorders crews.
- Dropping never sets a role.
- **Multi-select:** ⌘-click or Shift-click rows, then use the bar at the sidebar foot: Make crew / Move to… / Esc.

## 5. States
| State | Behaviour | Frame |
|---|---|---|
| Sync | A change shows instantly on the device that made it. Other devices show the old state until their next poll (about 15 s), with no placeholder. | 4i |
| Refused move, rename, recolour or reorder | The row snaps back, with a 1.5px `accent` outline, and a toast gives the reason, e.g. "Couldn't move relay-review into Gateway release: it was dismissed on Pixel 8." | 4i |
| Last member dismissed | The crew is removed. Toast: "{crew} was removed: its last session was dismissed. Undo" (open question 2). | 4i |
| Host offline | The member's badge is `amber`. End all running lists it under "Can't reach now" and **skips** it; nothing is queued. | 4a, 4d, 4l |
| 40-character crew name | Ellipsizes in the sidebar header and the session chip, fits on the overview card, wraps on the crew page. | 4a, 4e, 4h |
| 8 crews | Mostly collapsed; the CREWS section itself can collapse. | 4h |
| 900px | Sidebar 260; open count "2 running", and the "2 run" variant is gone. | 4h |
| No crews / loading | As in Round 3 (3j, 3k): one hint on the overview; skeletons without shimmer; crew headers render only after crews load. | 3j, 3k |

## 6. Phase 3: agent-to-agent (build after crews V1)
- **Spawning:**
  - Off for every session.
  - A person switches it on for **one running session** with "Allow this session to start agents" in its header, and can turn it off at any time.
  - No global setting, no launch option, and an agent can't turn it on.
  - Shown greyed out with a reason when the host can't support it, e.g. "This host doesn't isolate sessions yet".
- **Approvals:**
  - Every **start, message and end** an agent asks for waits for a person on a signed-in device, and expires after 10 minutes. There's no auto-approve.
  - The card, in the session or as a notification, shows: who is asking (session, agent, host); the kind; the exact text; the host's identity and isolation; the time left; Approve and Deny.
  - For a start only, it adds "Let the child write to {parent's vault}", off by default.
  - Outcomes: started, denied, expired, and refused with a reason.
- **Child sessions:**
  - A child runs on its parent's host with the same agent, and is given task text only.
  - It works in its own copy of the repository (committed work only), or with read-only access and a scratch folder.
  - Its sub-line reads "↳ from {parent} · own copy of {ws}" (or "· scratch"), drawn on a **dashed** indent. Crew membership keeps its solid edge.
- **Apply:** nothing reaches the parent's folder until a person clicks Apply. Apply creates `storm/<child>` or a new folder, and never merges or overwrites. The preview lists what it creates.
- **Resumed child:** independent. No indent; About says "Was started by {parent} (history)".
- **Limits:** 2 levels deep, 3 live children per parent, 6 live sessions per family, 2 pending requests per parent. A request over a limit is refused, never queued.
- **Ending:**
  - Ending a parent ends its children. The banner lists running and ended children separately, with an optional "Let children report first" (waits 30 s).
  - A crew's End all running lists every child it reaches, including any in another crew.
  - Families are dismissed only together, once every member has ended. "Dismiss ended" skips families with a live member, and its toast says how many it kept.
- **Needs attention:** shown beside the activity (sub-line and crew header) for a waiting request, a block or a permission prompt. It's never inferred from idle.

### Phase 3 server/runtime
- **Session fields:** `spawn_enabled` (default false), `parent_session_id`, `isolation` (`copy` or `scratch`), `family_id`.
- **Who can switch it on:** only the owner, from a signed-in device, by `PUT /v1/agent/sessions/{id}/spawn`. No launch parameter sets it; no agent tool can reach it.
- **Storm MCP tools, offered only when `spawn_enabled`:**
  - `request_start(task)`
  - `request_message(session, text)`
  - `request_end(child)`, which ends only the agent's own children.

  Each tool returns a request id and blocks until there's an outcome.
- **AgentRequest:** `{ id, session_id, kind: start|message|end, payload, target?, created_at, expires_at (+10 min), state: pending|approved|denied|expired|refused, reason?, decided_by_device? }`. Endpoints: list, approve (with `child_can_write` for a start), deny.
- **Refusals happen at request time and are never queued:**
  - Limits: depth > 2, > 3 live children, > 6 live per family, > 2 pending per parent.
  - Spawning is off.
  - The host isn't isolating.
  - The target isn't the agent's child (for end).
- **A child:**
  - runs on the parent's host with the parent's agent;
  - is given the task text only, never a workspace;
  - gets the parent's grants or fewer, and writes only if approved;
  - works in a git worktree of the parent's committed HEAD (`copy`), or with read-only access plus a scratch directory.
- **Apply:** `POST /v1/agent/sessions/{child}/apply`. It creates branch `storm/<child>` (or folder `<ws>-<child>`) and never merges or overwrites. It returns 409 if the target exists.
- **End and dismiss:**
  - Ending a parent ends the whole family. `report_first` waits 30 s.
  - Dismiss works per family, once every member has ended.
  - Resuming a child clears `parent_session_id` and keeps a `started_by` history field.
- **Notifications:** every pending request is pushed to signed-in devices, with Approve and Deny in the notification where the platform allows.
- **Needs attention:** set by the server for a pending request, a blocked session, or a provider permission prompt. It's never derived from idle.

### Phase 3 acceptance
- [ ] Spawning is off for every new and resumed session. Only the owner turns it on, per session, and the confirmation copy matches 4t.
- [ ] No setting, launch option or agent tool can turn it on.
- [ ] Every start, message and end waits for a person and expires at 10 minutes. There's no auto-approve path anywhere.
- [ ] Approval cards show who, the kind, the exact text, the host identity and isolation, and the time left. Start adds a write toggle, off by default.
- [ ] Each of approved, denied, expired and refused-with-reason appears in the Requests tab and as a Storm line in the agent's terminal.
- [ ] A request over any limit is refused immediately with the limit named.
- [ ] A child runs on the parent's host with the parent's agent, gets no more access than the parent, and sees no workspace choice.
- [ ] Apply creates a new branch or folder only. Nothing reaches the parent's folder before Apply.
- [ ] Lineage is drawn as a dashed indent with "↳ from"; crew membership keeps its solid edge.
- [ ] A resumed child is independent and its About shows the history line.
- [ ] Ending a parent lists its children and ends them. Crew End all reaches children in other crews.
- [ ] Dismiss works per family, and the "kept" toast appears.
- [ ] Hosts that can't isolate show the switch disabled with a reason.

## 7. Open questions (drawn side by side on the page; not decided)
1. **Collapse state:** per device, saved across restarts (suggested), or synced.
2. **Empty crew after the last dismissal:** auto-delete with Undo (suggested), or keep it.
3. **Name for a crew made by dragging** two sessions with different context notes: "New crew" with the field selected (suggested), or the target's note title.
4. **Sidebar crew actions:** ··· on hover plus right-click (suggested), or the crew page only.
5. **Where roles come from:**
   - every note in Kit `agents/` (no setup, but also lists subagent definitions);
   - only notes with a property such as `role: true`;
   - a separate `roles/` folder.

   No suggestion.
6. **Members sharing a host:** name the hosts in the crew header and keep them on rows (suggested), or drop the host from rows.

## 8. Acceptance checklist
- [ ] Crews persist on the server and reach a second device within one poll (about 15 s).
- [ ] A session is never in two crews.
- [ ] Sidebar order is CREWS (running before ended in each) → RUNNING → ENDED, with a 2px gap between rows. The CREWS section collapses.
- [ ] Sub-lines read "host · workspace · age". The workspace truncates first; host and age never truncate.
- [ ] Marks are 32 (sidebar), 36 (phone) and 28 (cards). The five badge states render as in §2, and only "working" pulses.
- [ ] Ended rows fade only the mark (.5) and turn the name `text2`.
- [ ] The sidebar is between 260 and 400 wide, and the overview at most 760.
- [ ] Role tags are neutral and solid, about 14ch at most, and shrink before the name. A deleted note is struck through; a renamed note updates the tag.
- [ ] A role can be set on a loose session. On the phone, the long-press sheet's Crew and Role are independent.
- [ ] Launching with a role passes the note as context; a later change only relabels. About shows the launch role and the label when they differ; resume keeps the launch role.
- [ ] For Shell, Cursor and Command Code, the launcher's Role row is disabled with its reason.
- [ ] The role picker shows each note's summary, grouped by folder when Kit has `projects/<name>/agents/`.
- [ ] End all running is an inline banner on desktop and a sheet on phone. It lists every session, and skips offline hosts with "Can't reach now".
- [ ] A refused change snaps back with a toast. A crew emptied by Dismiss is removed with Undo.
- [ ] The colour menu shows nine swatches in every theme. Crew colours are derived per theme.
- [ ] The hover ··· and right-click open the same crew menu.
- [ ] A 40-character name: ellipsis in the sidebar and chip, wraps on the crew page.
- [ ] No Phase 3 UI in the crews V1 release.

## Files
- `Storm Agent Crews.dc.html`: the design page. Round 4 is at the top.
- `Crews R4 Frame.dc.html`: Round 4's frame renderer. Scenes:
  - desktop: overview, overview-light, overview-earth, crew-endall, session-roles, launcher, launcher-shell, narrow, sync;
  - phone: p-list, p-sheet, p-crew-end;
  - Phase 3: p3-approval, p3-message, p3-end-request, p3-spawn-on, p3-result, p3-end-parent, p3-crew-endall, p3-resumed, p3-unavailable, p3-p-notify, p3-p-approve.
- `Agents Crews Frame.dc.html`: Round 3's frame renderer (history).
- `support.js`: the runtime needed to open the `.dc.html` files.
