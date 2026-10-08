# Storm v2 — handoff §11 acceptance checklist, with evidence

`design_handoff_storm_v2/README.md` §11, item for item, checked on
`feat/v2-acceptance` (slice 9, 2026-10-08). Evidence is a client test
(`apps/client/test/<file>`: "test name"), a live suite check
(`apps/server/tests/<file>`), or a shot in this directory taken by the harness
against a real `storm-server` and real `storm-runtime` hosts. An item left
unticked says why.

## Navigation

- [x] **Desktop shows the 56px rail on every screen: Notes, Agents, flexible
  spacer, status dot, Settings.** `shell_v2_test`: "only at desk width, with
  the current activity lit"; every `desktop-*.png`.
- [x] **Rail active state is `accent` on `accentSoft`. The Agents badge shows
  the live-session count only when it is above 0.** `agents_navigation_test`:
  "wide: the rail badge counts live sessions", "no badge when nothing is
  live"; `desktop-06-agents-overview.png`, `desktop-04-health-popover.png`
  (no badge before any session).
- [x] **Phone: the left bubble opens the place picker (vaults, Agents with
  "{n} running", sync line). The right bubble opens Settings. The "A" bubble
  is gone.** `shell_v2_test`: "the place picker moves between a vault and
  Agents", "the gear opens Settings and is lit there";
  `phone-02-place-picker.png`, `phone-10-settings-list.png`.
- [x] **No Home or dashboard exists on either device. Launch restores the last
  activity and location.** `back_navigation_test`: "launch restores the last
  activity and location"; `router_test`: "/ goes to the last location";
  `shell_v2_test`: "each activity reopens where it was left".
- [x] **No vault control contains "Server settings". The vault switcher ends
  with "Manage vaults ›".** `notes_v2_test`: "lists vaults with counts and a
  check, and Manage vaults", "vault header, search for this vault, and no gear
  or …"; `desktop-03-vault-switcher.png`.
- [x] **All old settings routes redirect (§1.4). The OAuth return still lands
  on Integrations.** `router_test`: the "$from → $to" redirect cases;
  `settings_pages_test`: "an OAuth orphan still lands on Integrations and is
  relayed …"; `integrations_test`: "a sign-in redirect no sign-in was waiting
  for is relayed …".
- [ ] **Android back unwinds pushed pages (note, folder, session, settings
  page, pushed note) without exiting early (Q2).** *Not run on a device:* this
  host has no Android SDK, `adb` or emulator, so nothing here could drive a
  real system back. The contract itself is covered by widget tests that send
  the system back through the same `BackButtonListener` the app uses —
  `back_navigation_test` (all ten tests, e.g. "pushed folders and notes pop
  back the way they came", "back at the vault root leaves the app", "a pushed
  phone settings page pops to the list") and `agents_v2_test` "phone: a deep
  link backs out to /agents", `loop_v2_test` "phone: a pushed context note
  backs out as the session". Tick after a run on an Android device.

## Notes

- [x] **The sidebar shows the vault header (tile, name, sync line), "Search
  {vault}", up to 4 cross-vault Recent rows with vault tags, and the folder
  tree.** `notes_v2_test`: "RECENT is the four newest notes from every vault,
  each …", "the folder tree carries no counts and indents by depth";
  `desktop-02-note-start-session.png`.
- [x] **The phone vault root shows Recent (title, "vault · folder" line, age)
  above Folders.** `notes_v2_test`: "vault root: the vault as title, RECENT
  with where each …"; `phone-01-notes-vault-root.png`.
- [x] **The note view keeps the existing editor, the 640 measure, Read | Edit,
  the properties drawer and Linked mentions.** `notes_v2_test`: "desktop:
  crumb, Read | Edit, Start session, drawer toggle, …"; `adaptive_layout_test`:
  "open as a drawer beside the note on a wide screen", "starts shut at 900 /
  1100 / 1199 …"; `markdown_read_mode_test`: "the editor writes prose at the
  same 18 / 1.6 (§7.2)"; `desktop-02-note-start-session.png`.
- [x] **The note header shows Start session. With no online host, it opens
  Agents instead of the launcher.** `notes_v2_test`: "Start session goes to
  Agents when no host is online at …"; `agents_v2_test`: "Start session on a
  note launches with it as context"; `desktop-05-new-session-launcher.png`.
- [x] **A note written by a session shows "Edited by session {name}, {age} ›"
  (or "Created by…") in `accent` on the version line. It opens that session's
  Wrote tab.** `loop_v2_test`: "desk/phone: an edit names its session and
  opens its Wrote", "a note the session made says Created";
  `desktop-01-notes-provenance.png`, `desktop-loop-b-edited-by-session.png`
  (the server's real `agent_write`).
- [x] **The unseen dot appears on agent-changed notes and their collapsed
  folders, and clears when the note is opened.** `loop_v2_test`: "desk: a
  collapsed folder rolls the dot up; opening the note clears it", "phone:
  folder and note rows carry it", "a fresh device baselines: no dots for what
  agents wrote before it looked, a dot for a later write";
  `desktop-loop-a-unseen-dots.png`.

## Agents

- [x] **The sidebar shows ＋ New session (only with an online host), Overview,
  and Running / Ended sections, each hidden when empty.** `agents_v2_test`:
  "desk: a row opens it beside the sidebar; Overview returns";
  `agents_navigation_test`: "no pill while no host is online";
  `desktop-06-agents-overview.png`, `desktop-06c-agents-no-host.png`.
- [x] **The overview shows WORK cards grouped by (workspace, host). Rows carry
  a context chip and "wrote {n}". START AN AGENT cards open the launcher with
  that agent. The infrastructure line links to Settings › Hosts.**
  `agents_v2_test`: "the overview groups work and says what each one is
  doing"; `desktop-06-agents-overview.png`.
- [x] **The no-host and first-session states match §2.7 / §2.8 copy. The
  phone shows a flat list with no work cards.** `agents_v2_test`: "first
  session: start from a note, or without one", "no host: three steps; the
  button only at desk width"; `agents_navigation_test`: "running then ended,
  with New session as a pill"; `desktop-06b`, `desktop-06c`,
  `phone-05-agents-list.png`.
- [x] **Status dots and chips follow §3.2, including the starting ring and
  `danger` for failed.** `v2_design_system_test` (the `session_status.dart`
  drawing); `agent_test` (status words); `desktop-11b-session-failed.png`.
- [x] **The UI never labels Kit files as agents. Claude Code, OpenCode and
  Shell are "Agents".** `agents_v2_test` (START AN AGENT cards are the
  providers); `desktop-06-agents-overview.png`; Kit's `agents/Storm Lead.md`
  appears only as a note (`desktop-02`, RECENT).

## Launcher

- [x] **It opens as a modal (desktop, 460 wide) or a bottom sheet (phone).**
  `agents_v2_test`: "a centred modal at desk width, a bottom sheet on a
  phone"; `desktop-05-new-session-launcher.png`,
  `phone-04-new-session-sheet.png`.
- [x] **It shows the CONTEXT box when started from a note, removable with ×.**
  `agents_v2_test`: "Start session on a note launches with it as context";
  `desktop-05`.
- [x] **Host (online only), Workspace (with the shared-workspace warning) and
  Agent (with the default marked) behave as in §3.5.** `integrations_test`
  launcher group; `agents_v2_test`: "an offline original host gives way to an
  online one"; `desktop-05`.
- [x] **"Can write to" defaults to the note's vault. It shows "Read only" when
  off, is disabled with "Off in Settings › AI access" when the setting is off,
  and is hidden for Shell.** `agents_v2_test`: "without a note it writes to
  the first vault, or not at all", "writes off in Settings: the toggle is
  disabled and says so", "a shell has no write field, and never sends a
  vault".
- [x] **The risk box text matches §3.5 for shell and non-shell.**
  `integrations_test`: "an agent is told it can use your integrations", "a
  shell gets no write field and no integrations (G-D9)"; `desktop-05`.

## Session

- [x] **Desktop shows the split: the terminal column (header, meta line, End
  or Run again / Dismiss) and the panel with Context | Wrote {n} | About.**
  `desktop-07-session-running.png` … `desktop-10-session-about.png`.
- [x] **End needs inline confirmation with the exact copy, and ends as
  stopped.** `agents_v2_test`: "desk: Cancel keeps it running; End session
  ends it"; `desktop-11c-session-end-confirm.png`.
- [x] **Ended sessions show the ended footer. Run again prefills the launcher;
  Dismiss removes the session and returns to the overview.**
  `agents_v2_test`: "Run again prefills everything and still waits for
  Launch", "Dismiss removes it and returns to the list";
  `desktop-11a-session-completed.png`.
- [x] **Context shows the source note read-only, with "Open in Notes ›".
  Without one, it shows the "Started without a note…" copy.**
  `agents_v2_test`: "Context shows the note read only, and the tab is the
  route", "the empty copies say why"; `desktop-08-session-context.png`.
- [x] **Wrote lists created and edited notes ("new" or "v{n}"), opens them in
  the panel with "‹ Wrote", and shows the two empty-state copies correctly.**
  `agents_v2_test`: "the empty copies say why", "Wrote is fetched again when
  wrote_count moves, and only then"; `loop_v2_test` (the panel lines);
  `desktop-09-session-wrote.png`.
- [x] **About shows Agent, Workspace, Vault access, Integrations and Network,
  with the shell variants.** `agents_v2_test`: "About says what the session
  may touch"; `loop_v2_test` About tests; `desktop-10-session-about.png`.
- [x] **Phone: a full-screen terminal with the chip row, the extra-keys row
  while live, the details sheet (Started from, Wrote, About, End with
  confirmation), and Run again / Dismiss bars when ended. A pushed note
  returns to the terminal.** `agents_v2_test`: "phone: the details sheet holds
  End and its confirmation"; `loop_v2_test`: "phone: a pushed context note
  backs out as the session", "phone 360/390/430: the session name is never
  cut"; `phone-06` … `phone-09`.

## Settings

- [x] **The hierarchy and order match §5.1. There is no Accounts section.**
  `settings_pages_test`: "every page renders its title at …";
  `phone-10-settings-list.png`, `desktop-12-settings-this-device.png`.
- [x] **Desktop keeps the rail, with the settings navigation in the sidebar.
  Phone has a list, then pushed pages with "‹ Settings" and no AppBar.**
  `settings_pages_test`: "phone: list → pushed page → ‹ Settings back to the
  list", "desk: the nav selects the page beside it";
  `phone-11-settings-page-ai-access.png`.
- [x] **Every page matches the §5.3 copy and controls.**
  `settings_pages_test` (a group per page); `desktop-12` … `desktop-21`;
  deliberate deltas in `README.md`.
- [x] **The AI access page shows two groups, and the agent-write toggle is
  independent of `mcp_writable`.** `settings_pages_test`: "a server with
  agent_writes: the toggle reaches it alone"; `agent_e2e.py` /
  `mcp_e2e.py` (`mcp_writable` no longer affects agents);
  `desktop-15-settings-ai-access.png`.
- [x] **The Integrations navigation item shows a `danger` dot when any
  connection needs attention.** `settings_pages_test`: "a connection needing
  sign-in puts a dot on the nav …", "a healthy connection shows no dot, and
  Choose tools".
- [x] **The theme preset change applies immediately and persists on this
  device.** `settings_pages_test`: "appearance, text size and note font change
  real settings"; `presets/storm-light/`, `presets/slowflow-earth/` (the
  preset set through This device's stored `storm.theme`, read on reload).

## Responsive

- [x] **One breakpoint at 900. Every place in §6 exists on both layouts; only
  presentation differs.** `adaptive_layout_test` (both sides of 900);
  `notes_v2_test`: "a window resized across 900 with a popover open stays
  sound"; `sweeps/d900-p360/`, `sweeps/d1100-p390/`, `sweeps/d1600-p430/`.
- [x] **Phone hit targets are at least 44 (bubbles, pill slots, rows).**
  Bubbles and pill slots are `sp * 5.5` (44) and the ＋ `sp * 6` (48)
  (`storm_scaffold.dart`, `nav_bubble.dart`); `phone-01-notes-vault-root.png`.

## Core loop (end to end, on a real backend)

- [x] **Note → Start session → launcher shows the note as context → Launch →
  session opens on Context.** Harness, through the UI on a real server and
  host: `desktop-loop-0-launched-on-context.png`; `loop_v2_test`: "the loop:
  note → session → …".
- [x] **The agent receives the note context (Q11).** `gateway_e2e.py`: the
  scripted agent reads the marked note through `session_context` over the
  real host bridge, and the marker is in no process's argv or environment;
  the harness's `run_loop` reads Gateway spec the same way before writing.
  *Not run:* the plan's manual check with the real Claude Code and OpenCode
  CLIs (they are installed here, but would run on the operator's account).
- [x] **With writes on to vault X, the notes the agent writes appear in Wrote,
  and only in vault X.** `gateway_e2e.py`: "with a write vault and agent
  writes on, the agent may write, and still never delete", "a write outside
  its write vault is refused with a stable code"; `desktop-09-session-wrote.png`.
- [x] **Each written note shows its provenance link and the unseen dot, and
  the link returns to that session's Wrote tab.** `desktop-loop-a-unseen-dots.png`,
  `desktop-loop-b-edited-by-session.png`; `loop_v2_test` (provenance opens
  `?tab=wrote`).
- [x] **With writes off, Wrote stays empty with the "can’t write" copy, and no
  vault changes.** `agents_v2_test`: "the empty copies say why";
  `agent_e2e.py`: "an older client's write toggle without a vault launches
  read only, and says so"; `gateway_e2e.py`: "the built-in storm connection is
  read-only without the flags, and never deletes".

## Tokens

- [x] **No literal colours, sizes or radii in `lib/ui/`; the token conformance
  test passes in all three presets.** `token_conformance_test` (source scan);
  `tokens_test` (contrast in every preset, "note prose is about 18 with a 22
  H2, and follows the text size"); `presets/`.

**Summary:** 43 of 44 ticked. Unticked: Android back on a device (no Android
tooling on this host; the contract is widget-tested).
