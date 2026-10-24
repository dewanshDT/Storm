# Agent Crews — Audit and Implementation Plan

**Status:** locked as **decision 138** (approved 2026-10-25), plan proposed,
**no production code changed**.
**Baseline:** `origin/staging` at `93a4391`.
**Sources read in full:** `design_handoff_storm_crews/` (README, Round 4 of
`Storm Agent Crews.dc.html`, `Crews R4 Frame.dc.html`),
`design_handoff_storm_v2/README.md`, decision 138 (vault:
`Storm/Agent Runtime/Agent Crews — decision 138`) and decisions 89, 130, 131,
134–137.
**Code audited:** `apps/server/src/agent/{mod,store}.rs`, `ops.rs`
(launch, `session_context`, resume, kit), `api.rs` (session router, tests),
`mcp.rs`; `apps/runtime/src/mcp.rs`; `apps/client/lib/agent/*`,
`lib/ui/{accents,breakpoints,surfaces}.dart`, `lib/state/terminal_prefs.dart`.

Round 4 of the handoff is the visual and behavioural source. **Decision 138
wins where the two differ**: §2 lists every difference. Crews V1 (handoff
§1–§5, §8) is this plan. The handoff's Phase 3 section (§6) is the UI for
decision 134 and is built only with 134's slices, after 135 → 136 → 137; it is
out of scope here, and V1 adds no Phase 3 schema.

---

## 1. Audit — what exists today

### 1.1 Server

| Area | Today | File |
|---|---|---|
| `agent.db` schema | One `execute_batch` of `CREATE TABLE IF NOT EXISTS` in `Store::init`; no version table. Columns added later are probed with `pragma_table_info` and `ALTER TABLE … ADD COLUMN` (`title`; `harness_session`, `resumed_from`). No foreign keys, no cascades. | `agent/store.rs` L146–257 |
| Its doc comment | "Durable but rebuildable in spirit: if this file is lost, the sessions a host still runs are re-adopted from its next `hello`." Crews are **not** rebuildable — the comment must say so. | `store.rs` L1–5 |
| Tables | `sessions`, `session_mcp`, `session_mcp_grants`, `session_launch` (name, context vault/note/title, write vault; **kept on dismiss** for provenance), `session_writes`, `session_integrations`, `session_mcp_notice`. | `store.rs` L149–232 |
| Dismiss | `Manager::dismiss` refuses unless ended, then `store.delete` runs only `DELETE FROM sessions`. Child tables are kept on purpose. `DELETE /v1/agent/sessions/{id}`. | `agent/mod.rs` L1650, `store.rs` L661 |
| Launch | `POST /v1/agent/sessions` → `ops::launch_session` → `Manager::launch`; request `agent::Launch {host_id, workspace, provider?, interaction, terminal, context?: ContextRef{vault_id, note_id}, write_vault_id?, allow_vault_writes, resume_of?}`; `insert_launch` writes `session_launch`. | `ops.rs` L2047, `agent/mod.rs` L343, L1105, L1254 |
| Host flag | `Command::Start {context: bool, …}` — "a flag, never text, so no note data reaches argv"; set `reads_storm && launch.context.is_some()`. | `agent/mod.rs` L48–63, L1288 |
| Opening prompt | `OPENING_PROMPT = "Read your context note with the storm session_context tool, then wait for my instructions."`, skipped on resume. | `runtime/src/mcp.rs` L386–391, `host.rs` L1355 |
| `session_context` | `Actor::Agent` only; one note from `launch_of(session).context`, read live; 404 "started without a note" / "the context note no longer exists". Returns `{vault_id, note_id, title, path, version, content}`. | `ops.rs` L319–360, `mcp.rs` L564 |
| Resume | Copies host, workspace, provider; reuses the old launch's context snapshot and write vault; new `session_launch` row. | `ops.rs` L2052–2066 |
| Session list | `GET /v1/agent/sessions` → `SessionView` (the record plus name, context, write_vault_id, wrote_count, integrations, …). Client polls every 15 s with hosts. | `ops.rs` L1980, L2206; client `agent_state.dart` L129, L168–185 |
| Auth | `/v1/agent/*` is the session tier (`require_auth`, `RequiredTier::Session`): session bearer only, `stk_` keys refused; `Actor::Agent` never reaches REST. Explicit owner checks: `pc_owner_only`. | `api.rs` L411, L2213; `ops.rs` L3963 |
| Kit | Found by directory name `kit` (`kit_vault_id`, 404 if absent). Seeded once with `agents/Storm {Architect,Lead,Coder,Researcher,Reviewer}.md` (`kind: agent`, `summary:`). **No server helper lists notes by folder or reads `summary`**; the index row has no frontmatter. | `kit.rs` L33, L73; `ops.rs` L937, L983 |

### 1.2 Client

| Area | Today | File |
|---|---|---|
| Sidebar | `AgentsSidebar` (a `section(label, list, top)` closure; Running and Ended in a `ListView`). | `agent/agents_shell.dart` L51–141 |
| Overview | `_Overview` → `_WorkCard` / `_WorkRow` (grouped by workspace and host), `_AgentCards`, `_PhoneList`, `_FirstSession`, `_NoHost`. Max width `sp×95` = 760. | `agent/agents_screen.dart` L294–944 |
| Row and mark | `SessionRow {session, host, onTap, selected, phone}` (sub-line `host · workspace · when`, mark 32/36); `AgentMark {session, size?, background?}`; badges already match Round 4 §2. | `agent/agent_widgets.dart` L64, L103, L320, L343 |
| Launcher | `_Field(label, child)`; `_Select<T>` dropdown (no chevron); "Can write to" hidden when the agent can't reach Storm. | `agent/launcher.dart` L339, L678, L708 |
| About | `aboutRows(...)` rendered by `_About` (`KeyValueList`). | `agent/session_screen.dart` L1217, L1290 |
| End banner | Inline banner, copy "The agent and everything it started are stopped on the host." | `session_screen.dart` L290 |
| Sheets | `showStormSheet<T>`; long-press precedent `showFolderActions`. | `ui/surfaces.dart` L69; `ui/browse_screen.dart` L376, L472 |
| Toasts | No helper; screens call `ScaffoldMessenger` directly; `rootMessengerKey` exists. No Undo action anywhere. | `state/app_state.dart` ~L1046 |
| Drag / multi-select | **None** in `lib/`. | — |
| Device prefs | `shared_preferences`; pattern `TerminalPrefs` and `agent.lastHost`. | `state/terminal_prefs.dart`; `agent_state.dart` L296 |
| Palette | `Accent` enum (nine words + none), `Accent.tile(tokens)` derives the swatch per theme. | `ui/accents.dart` |
| Token rule | `token_conformance_test.dart` scans `lib/ui/` only; `lib/agent/` already has literals. | `test/token_conformance_test.dart` |

---

## 2. Where decision 138 overrides the handoff

| Handoff (Round 4) | Locked (138) | Why |
|---|---|---|
| §7 open questions "not decided" | Q1 per device, saved; Q2 delete with Undo; Q3 "New crew" selected; Q4 hover ··· + right-click; Q5 **every note in Kit `agents/` and `projects/<name>/agents/`**; Q6 hosts in the header, kept on rows | 138c, e, m, n, o |
| `CrewMember {…, role_note_id?, launch_role_note_id?}` | Roles live on the **session**, not the membership (a loose session can carry one). Label in `session_role`; launch role on `session_launch`. | The handoff's own rule "role on any session" |
| Role row disabled for "Shell, Cursor and Command Code" | Disabled for any agent **without Storm's connection** (`reads_storm == false`); the copy names the agent. | Cursor and Command Code gain it when decision 131's gates pass (138g) |
| Crew swatch `oklch(L .10 h)` with its own per-theme L | `Accent.tile(tokens)` — the app's one derivation | 138l |
| Nullable `parent_session_id` in V1 | **No Phase 3 schema in V1.** 134d adds `parent_session`, `root_session`, `depth` with 134's slices. | 138p |
| §6 Phase 3 | Subordinate to 134–137; nine corrections in 138p (messages only to descendants via inbox; outcomes as tool results, nothing in a terminal; 134's tool names, non-blocking; clone not worktree; 134d lineage; revoke refuses pending; no approve in a push; identity is a state; specific unavailable reasons; the person names a scratch Apply folder) | Approved decisions 134–137 |
| README: Kit role tag "kit / agents / Reviewer" | Real titles: "storm reviewer", "sop reviewer" | Seeded titles carry a "Storm " prefix |

**Engineering details decided here** (no product effect; revisit freely):
- A **resumed** session joins its predecessor's crew, placed right after it,
  and keeps its label as well as its launch role.
- Crew names are 1–80 characters, trimmed; duplicates allowed.
- Crew ids are `crw_` + ULID, like `ags_`.

---

## 3. Server

### 3.1 Schema (`agent/store.rs`, additive)

```sql
CREATE TABLE IF NOT EXISTS crews (
  id TEXT PRIMARY KEY,              -- crw_<ulid>
  owner_user_id TEXT NOT NULL,      -- as sessions; single-user (decision 82)
  name TEXT NOT NULL,
  color TEXT NOT NULL,              -- Accent word; 'none' allowed
  ord INTEGER NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS crew_members (
  session_id TEXT PRIMARY KEY,      -- at most one crew per session (138b)
  crew_id TEXT NOT NULL,
  ord INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS crew_members_by_crew ON crew_members(crew_id, ord);
CREATE TABLE IF NOT EXISTS session_role (   -- the label (138d)
  session_id TEXT PRIMARY KEY,
  vault_id TEXT NOT NULL,
  note_id TEXT NOT NULL,
  set_at TEXT NOT NULL
);
-- session_launch, probed and added like `title`:
--   role_vault_id TEXT, role_note_id TEXT, role_title TEXT   (138f, fixed)
```

- Update the file's doc comment: crews, members and labels are the owner's
  data and are **not** rebuilt from `hello`; losing `agent.db` loses them.
- `Store::delete(session)` becomes one transaction: delete the session, its
  `crew_members` row and its `session_role` row; if that leaves its crew with
  no members, delete the crew and **return it** (138c). `session_launch` and
  `session_writes` stay, as today.
- Moves use the same "emptied crew" rule.

### 3.2 Endpoints (session tier; every handler refuses a non-`Actor::Session`, like `pc_owner_only`)

| Method and path | Body → result |
|---|---|
| `GET /v1/agent/crews` | → `[{id, name, color, order}]` |
| `POST /v1/agent/crews` | `{name, color?, session_ids[]}` → `{crew, removed_crews[]}` (sessions move in, in that order) |
| `PATCH /v1/agent/crews/{id}` | `{name?, color?}` → crew |
| `PUT /v1/agent/crews/order` | `{ids[]}` → 204 (must be a permutation of all crews) |
| `PUT /v1/agent/crews/{id}/members/order` | `{session_ids[]}` → 204 (a permutation of its members) |
| `DELETE /v1/agent/crews/{id}` | → 204; members become loose |
| `PUT /v1/agent/sessions/{id}/crew` | `{crew_id: string \| null, before?: session_id}` → `{removed_crews[]}` (join, move, reorder or leave) |
| `PUT /v1/agent/sessions/{id}/role` | `{vault_id, note_id}` → 204; `DELETE` clears |
| `GET /v1/agent/roles` | → `[{vault_id, note_id, title, path, folder, summary}]` |
| `DELETE /v1/agent/sessions/{id}` (existing) | now → `{removed_crew?}` instead of an empty 204 (older clients ignore the body) |

- **Validation:** `color` is an `Accent` word; a role note must be in the Kit
  vault under `agents/` or `projects/<name>/agents/` at the time it is set; an
  unknown session or crew is 404; a non-permutation order is 400.
- **`GET /v1/agent/roles`:** `kit_vault_id`, then the tree filtered by path,
  then each note's frontmatter `summary` (first line, ≤ 160 characters). It is
  read live; if this is slow on a large Kit, cache by `content_hash`.
- **Session list** (`SessionView`) gains:
  - `crew: {id, order} | null`;
  - `role: {vault_id, note_id, title | null}`, where `null` means the note was deleted;
  - `launch_role: {vault_id, note_id, title} | null`.

  Titles are resolved live from the index, so a rename follows.

### 3.3 Launch, `session_context`, resume

- `agent::Launch` gains `crew: {id} | {new_name} | null` and
  `role: ContextRef | null`.
- **With `reads_storm`:** the role is written to `session_launch.role_*`
  (fixed) **and** as the initial label. Without it, the role is a label only.
- The crew join happens in the same transaction as the launch row.
- **Host flag:** `context: reads_storm && (launch.context.is_some() ||
  launch.role.is_some())`. No new host capability is needed.
- **`session_context`** returns `{context: Note | null, role: Note | null}`,
  each read live by id. It is 404 only when both are absent; a deleted note is
  `null` with a `missing` reason.
  - This changes the shape. The only consumer is the agent, which reads it as
    text.
  - The server's MCP instructions line (`mcp.rs` L728) names both notes when
    both are set.
- **Opening prompt** (runtime): "Read your launch context with the storm
  session_context tool, then wait for my instructions." Older hosts keep the
  old sentence and still work, because the tool returns both notes.
- **Resume:** copy `role_*` from the old launch row, copy the label, and join
  the old session's crew after it (§2).

### 3.4 Persistence and authorization notes

- Crews never affect grants, hosts, or the end and dismiss semantics (138a).
  A test asserts that a launch's grants are identical with and without a crew
  and a role.
- `owner_user_id` is written like on `sessions`. No multi-user code is added
  or removed.

---

## 4. Client

### 4.1 Models, API, state
- `agent_models.dart`:
  - add `Crew`, `RoleRef`, and `SessionCrew`;
  - add `crew`, `role` and `launchRole` to `AgentSession.fromJson`.
- `agent_api.dart`: one method per endpoint in §3.2.
- `agent_state.dart`:
  - `agentOverviewProvider` fetches sessions, hosts and crews together;
  - add a `rolesProvider`, refreshed when the launcher or sheet opens and on
    the 15 s tick while they stay open;
  - add an **optimistic overlay** that applies a pending move and rolls it back
    on error with a toast (138j).
- `crew_prefs.dart`: per-device collapse state for each crew and for CREWS, in
  `shared_preferences` (`storm.crews.collapsed.*`), modelled on `TerminalPrefs`.

### 4.2 Shared widgets (`lib/agent/crew_widgets.dart`)
- `RoleTag`: Mono 10.5/16, `text2`, 1px `border`, radius 5, padding 0 5, 5ch minimum, `max-width 15ch`, ellipsis. Shrinks before the name (`Flexible` with a high flex factor against the name). Struck through at opacity .6 when deleted. **One widget everywhere.**
- `CrewSquare(accent, size)`: uses `Accent.tile`.
- `CrewChip`: a 9px square and the name, max 220, ellipsis.
- `CrewHeader`:
  - chevron, square, name and count ("2 running" open, "2 · 1" collapsed);
  - a hover ··· (22×20) and right-click (`onSecondaryTapUp`) opening `CrewMenu`.
- `CrewMenu`: 220 wide, `surface2`, `rCard`. Items:
  - Open crew page;
  - Rename;
  - Colour (nine swatches);
  - Collapse;
  - New session in crew;
  - Dismiss ended (n);
  - End all running… (n).
- `SessionRow` gains an optional `role` and keeps everything else. Its sub-line is already `host · workspace · when`; check that the workspace is what truncates first.

### 4.3 Screens
- **Sidebar** (`agents_shell.dart`):
  - a CREWS section (collapsible to "CREWS · n") above Running and Ended;
  - each crew is a `CrewHeader` plus its members behind a 1px solid left edge;
  - inside a crew, running members come before ended ones;
  - headers render only once crews have loaded.
- **Overview** (`agents_screen.dart`):
  - CREWS cards (open: rows with mark 28, the context chip and "wrote n"; collapsed: stacked marks);
  - NOT IN A CREW is today's `_WorkCard` grouping, limited to loose sessions;
  - START AN AGENT is unchanged;
  - the no-crews hint stays until the first crew exists.
- **Crew page:** a new route `/agents/c/:id`.
  - The header has an 18px square, a name 25/600 that wraps, "＋ New session in crew" and ···.
  - The summary line is counts plus distinct hosts.
  - Below it come the members card, **Started from** (the distinct `context` across members) and **Wrote** (each member's existing `/writes`, merged).
- **Session view:**
  - the meta line adds `CrewChip` and `RoleTag`;
  - `aboutRows` gains **Crew** (Move to crew ▾) and **Role** (launch role and label, "Change label ▾", the label-only and deleted-note copy).
- **Launcher:**
  - `_Select` gains a trailing chevron everywhere;
  - add a **Crew** row (None · New crew: {context title} · crews) and a **Role** row (None · roles grouped by folder, with their summaries);
  - Role is disabled with its reason when `!readsStorm`;
  - show the quiet line when a write vault is granted;
  - opened from a crew page, Crew is preset.
- **End all running:** an inline banner on the crew page (desktop) or a sheet (phone).
  - It lists Stops, Can't reach now (unknown host, skipped) and Already ended.
  - It loops over `endSession`. Dismiss ended loops over `dismiss`.
- **Phone:**
  - the crew screen is pushed, with the back chevron in the corner bubble and ··· as a sheet;
  - a long-press on any session row (`onLongPress`) opens `showStormSheet` with independent **Crew** and **Role** sections;
  - the same controls are in the details sheet;
  - there is no drag.

### 4.4 Desktop interactions
- **Drag:** `Draggable<SessionDrag>` on rows and `Draggable<CrewDrag>` on headers. `DragTarget` on:
  - a loose row: make a crew, then open the inline rename with "New crew" selected;
  - a crew header or member: join;
  - the gap between members: reorder;
  - the Running and Ended labels: leave;
  - the gap between headers: reorder crews.

  The ghost is the row on `surface2`, rotated -2°, with the shadow; the source dims to .3. Dropping never sets a role.
- **Multi-select:** ⌘- or Ctrl-click and Shift-click on rows set a selection in sidebar state. A foot bar offers Make crew / Move to… / Esc, and Esc clears.
- **Toasts:** a small `showUndoToast(message, onUndo)` on `rootMessengerKey`, used for refused changes and removed crews.

### 4.5 Tokens
New widgets live in `lib/agent/crew_*.dart` and use tokens only. Extend
`token_conformance_test.dart` to scan `lib/agent/crew_*.dart` (decision 43).

---

## 5. Implementation order (vertical slices)

Each slice is a branch from `staging` and a PR into `staging`, with
`make check` clean and the vault (decision 138, Active Work) updated in the
same change.

| # | Slice | Depends on | Done when |
|---|---|---|---|
| 138-1 | **Server: crews, membership, labels.** Schema, store, the §3.2 endpoints except launch, `SessionView` fields, the dismiss transaction, `GET /v1/agent/roles`. | — | Rust tests in §6 pass; `agent_e2e.py` adds a crew round trip. |
| 138-2 | **Server + runtime: launch role and crew.** `Launch.crew/role`, `session_launch.role_*`, the host flag, `session_context` shape, MCP instructions, opening prompt, resume carry. | 138-1 | Real Claude Code reads both notes in a live run (`agent_e2e.py`); Shell launch records a label only. |
| 138-3 | **Client: models, state, shared widgets, sidebar and overview** (read and display; actions through the crew menu only). | 138-1 | Widget tests; screenshots of 4a/4b/4c in three themes. |
| 138-4 | **Client: crew page, session chip and About, launcher Crew and Role, chevrons, End all and Dismiss ended.** | 138-2, 138-3 | 4d–4g reproduced; launcher tests for `!readsStorm`. |
| 138-5 | **Client: drag and drop, multi-select, hover ··· and right-click; phone long-press sheet, crew screen and End-all sheet.** | 138-4 | Gesture tests; 4h, 4j–4l reproduced. |
| 138-6 | **States and finish:** optimistic overlay and snap-back, Undo for removed crews, loading and no-crews states, the 900px check, docs (`docs/site/agents/crews.md`, linked from `sessions.md`), acceptance run. | 138-5 | §6.3 checklist all ticked on a real server. |

---

## 6. Testing

### 6.1 Server (Rust, inline `api.rs` and `agent/mod.rs` tests)
- A session is in at most one crew. A move removes it from the old crew, and an emptied crew is deleted and returned.
- Dismiss removes the membership and label, keeps `session_launch` and `session_writes`, and deletes the emptied crew.
- Order endpoints refuse non-permutations. A colour outside `Accent` is 400. A role outside Kit `agents/` folders is 400. A missing Kit vault is 404.
- Every new route refuses a `stk_` key (session tier) and is unreachable as `Actor::Agent`.
- Launch with a role and a context:
  - `session_context` returns both;
  - a later label change leaves `launch_role` and `session_context` unchanged;
  - resume keeps both and joins the crew.
- Launch with a role for Shell (`reads_storm=false`): a label only, no host flag, and `session_context` is not offered.
- A renamed role note updates `role.title`. A deleted one gives `title: null`.
- Grants are identical with and without a crew and a role (138a).

### 6.2 Live suites
- `agent_e2e.py`: a crew round trip, a launch with a role, Resume, and a real Claude Code `session_context` read (the gated real-binary step).

### 6.3 Client (flat `test/*_test.dart`; fakes in `agent_fakes.dart`)
`crews_test.dart`, `crew_page_test.dart`, `role_tag_test.dart` and
`crews_drag_test.dart`, matched to the handoff's §8 checklist:
- [ ] Sidebar order is CREWS (running before ended) → RUNNING → ENDED, with a 2px gap; CREWS collapses; collapse survives a restart.
- [ ] Sub-lines read `host · workspace · age`; the workspace truncates first.
- [ ] Marks are 32, 36 and 28; the five badge states are present (existing tests extended).
- [ ] `RoleTag` caps at about 14ch, shrinks before the name, strikes through when deleted, and follows a rename.
- [ ] A loose session can carry a role; the phone sheet's Crew and Role sections are independent.
- [ ] About shows the launch role and the label when they differ.
- [ ] The launcher Role row is disabled with a reason for `!readsStorm`, roles are grouped with summaries, every `_Select` has a chevron, and Crew is preset from a crew page.
- [ ] End all lists Stops, Can't reach now and Already ended, and skips unreachable hosts.
- [ ] A refused change snaps back with a toast; an emptied crew shows Undo, which restores it.
- [ ] Nine swatches appear in all three themes, derived through `Accent.tile`.
- [ ] Hover ··· and right-click open the same menu.
- [ ] A 40-character name ellipsizes in the sidebar and chip and wraps on the crew page.
- [ ] Drag: onto a loose row makes a crew; onto a crew joins; between members reorders; onto Running or Ended leaves; headers reorder; a drop never sets a role.
- [ ] Multi-select: Make crew, Move to…, Esc.
- [ ] No Phase 3 UI (a source scan of `lib/` for "start agents", "↳ from" and "Let children report").

### 6.4 Visual acceptance
The v2 harness (`docs/design/acceptance/`) gains `crews/`, with 4a–4l on a
seeded server, desktop and phone, in Storm dark, plus 4b and 4c.

---

## 7. Risks
- **Undo after a removed crew** recreates an empty crew with a new id. Two
  devices undoing at once make two crews. This is accepted because crews hold
  no data.
- **`GET /v1/agent/roles` reads each Kit agent note.** That is fine for tens of
  notes; cache by `content_hash` if Kit grows.
- **The `session_context` shape change** is visible to agents mid-flight
  during an upgrade. The tool is called once at start; a session that started
  on an old server and resumed on a new one simply reads the new shape.
- **Drag and drop is new to the client.** Keep it desktop-only (no
  `LongPressDraggable`) so phone scrolling is unaffected.

## 8. Checklist
- [ ] 138-1 server: crews, membership, labels
- [ ] 138-2 server + runtime: launch role and crew
- [ ] 138-3 client: models, widgets, sidebar, overview
- [ ] 138-4 client: crew page, session, launcher, End all
- [ ] 138-5 client: drag, multi-select, phone sheet
- [ ] 138-6 states, docs, acceptance
