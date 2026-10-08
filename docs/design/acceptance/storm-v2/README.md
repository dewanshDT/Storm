# Storm v2 — visual acceptance set

Screenshots of the **real** web client, against a **real** `storm-server`,
compared one for one with the approved references in
`design_handoff_storm_v2/screenshots/`. The plan is
`docs/design/STORM_V2_IMPLEMENTATION_PLAN.md` (§9.3).

## Reproduce

```sh
# once per change to the client or server
(cd apps/server && cargo build)
(cd apps/client && flutter build web --release)

python3 docs/design/acceptance/storm-v2/harness/shoot.py --set current   # baseline/
python3 docs/design/acceptance/storm-v2/harness/shoot.py --set v2        # the approved states
python3 docs/design/acceptance/storm-v2/harness/shoot.py --set v2 --only desktop-08
# slice 9: a preset, or other widths, into their own directory
python3 docs/design/acceptance/storm-v2/harness/shoot.py --set v2 \
    --only desktop-loop-0,desktop-01,phone-01 --preset storm-light --out …/presets/storm-light
python3 docs/design/acceptance/storm-v2/harness/shoot.py --set v2 \
    --only desktop-loop-0,desktop-01 --desktop-width 900 --phone-width 360 --out …/sweeps/d900-p360
```

`--only` takes comma-separated name prefixes; `steps` always run, and the
`loop` step needs `desktop-loop-0` (its session is launched in the UI), so
keep that shot in any subset that reaches the loop shots. `--preset` writes
the This device preset (`storm.theme`) into the page's storage and reloads,
as a launch would read it.

Needs `chromium` and Python 3 (standard library only). Ports: `HARNESS_PORT`
(default 7481), `HARNESS_DEBUG_PORT` (9333). `--keep` keeps the temp state
dir; on any failure the harness writes `failure.png` there and prints the
location and visible labels.

## What the harness does

1. Copies `harness/fixture/vaults` (the prototype's sample notes: `personal`
   with projects/storm, daily, reading; `work`) into a fresh root; the
   server seeds `kit` itself.
2. Starts the debug server with `--web` pointed at the release web bundle.
3. Claims the server over the API (pair + first account) to resolve vault
   and note ids for routes; `shots.py` routes use `{vault:…}` / `{note:…}`.
4. Starts headless Chromium; the web client bootstraps its own device from
   the served page and **signs in through the UI**.
5. Per shot: viewport (desktop 1280×800, phone 390×844, both DPR 2 — the
   references' 2560×1600 and 780×1688), in-app navigation, actions, settle,
   capture.

Two Flutter-web facts the harness depends on, found the hard way:

- **Typing needs semantics off.** With the semantics tree on, a tapped field's
  focus lands on the accessibility `<input>`, which the `TextField` does not
  read, and typed text vanishes. The harness maps the screen with semantics
  on, reloads (semantics off, same layout), then types.
- One text field is two `<input>`s when semantics is on; fill each kind once.

Chromium runs with `--no-sandbox` because its own sandbox cannot start under
a parent with `no_new_privs`; it only ever loads the local fixture server.

## Index

`baseline/` is the app before v2, kept for the record. The v2 set is filled
slice by slice; each row links the implementation shot to its reference and
the state recipe in `harness/shots.py`.

| Implementation | Reference | Slice | Status |
|---|---|---|---|
| `baseline/*` (10 shots) | — (pre-v2 record) | 0 | captured 2026-10-08 on `staging` `c9131b4` |
| `auth/phone-setup.png`, `auth/phone-signin.png` | — (not redesigned; existing style) | 1b | password only, captured 2026-10-08 |
| `design-system/gallery.png` | — (`/gallery`, three presets) | 2 | captured 2026-10-08 (`--set design`) |
| `desktop-01-notes-provenance.png` | same name | 4 + 8 | re-shot 2026-10-08 (slice 8) after the `board` session's real write: the provenance link is the server's `agent_write` |
| `desktop-02-note-start-session.png` | same name | 4 | captured 2026-10-08 |
| `desktop-03-vault-switcher.png` | same name | 4 | captured 2026-10-08 |
| `desktop-04-health-popover.png` | same name | 3 (+4 for sidebar/header) | rail + popover captured 2026-10-08; health rows are the fixture's real state (sync only) |
| `desktop-05-new-session-launcher.png` | same name | 6 | captured 2026-10-08 (Start session on Gateway spec); deltas below |
| `desktop-06-agents-overview.png` | same name | 6 | captured 2026-10-08; deltas below |
| `desktop-06b-agents-first-session.png` | same name | 6 | captured 2026-10-08 (`step` `hosts`) |
| `desktop-06c-agents-no-host.png` | same name | 6 | captured 2026-10-08 (before any host) |
| `desktop-07-session-running.png` … `desktop-11c-session-end-confirm.png` | same names | 6 | captured 2026-10-08 (`step` `sessions`); deltas below |
| `desktop-12-settings-this-device.png` … `desktop-21-settings-about-health.png` | same names | 7 | captured 2026-10-08 on `feat/v2-settings`; a key and a relay seeded through the API (`("api", …)` actions); deltas below |
| `desktop-loop-0-launched-on-context.png` | — (core loop: Start session → Launch in the UI) | 8 | captured 2026-10-08 |
| `desktop-loop-a-unseen-dots.png`, `desktop-loop-b-edited-by-session.png` | same names | 8 | captured 2026-10-08 (`step` `loop`); deltas below |
| `phone-02-place-picker.png` | same name | 3 (+4 for the root list, pill) | captured 2026-10-08; deltas below |
| `phone-01-notes-vault-root.png`, `phone-03-note-start-session.png` | same names | 4 | captured 2026-10-08 |
| `phone-10-settings-list.png`, `phone-11-settings-page-ai-access.png` | same names | 7 | captured 2026-10-08; deltas below |
| `phone-04` … `phone-09` | same names | 6 | captured 2026-10-08; deltas below |
| **every v2 shot above** | same names | 9 | **re-shot 2026-10-08 on `feat/v2-acceptance`** after the slice 9 polish (prose type, unseen baseline, phone-08's back link, Lucide pill, 20 phone inset, drawer default) |
| `presets/storm-light/*`, `presets/slowflow-earth/*` | — (handoff §7.1 presets) | 9 | desktop-01, 04, 06, 08, 15, loop-0, loop-b; phone-01, 02, 10 in each light preset |
| `sweeps/d900-p360/*`, `sweeps/d1100-p390/*`, `sweeps/d1600-p430/*` | — (resize sweep) | 9 | Notes (desktop-01), Agents (06), Settings (15), loop-0 at 900/1100/1600; phone-01, 05, 10 at 360/390/430 |

Agent states are real server state, never client fixtures (`harness/agents.py`,
slice 6). Shots in `shots.py` can carry a `step`, which moves the server on:
`hosts` enrolls two `storm-runtime` hosts against the harness's server
(`build-vm`, workspaces `storm` and `site`; `mac-mini`, workspace `storm`),
turns agent writes on and records three opens for START FROM A NOTE;
`sessions` launches four sessions through `POST /v1/agent/sessions`. The
hosts' `claude-code` and `opencode` providers run `harness/agent.py`, a
scripted agent started with the real MCP configuration (as
`apps/server/tests/gateway_e2e.py` does), whose `session_context`,
`update_note` and `create_note` calls go through the bridge and the gateway —
so names, contexts, write vaults, Wrote rows and versions are the server's.
`lint-fix`'s host is SIGKILLed and restarted under it (the runtime reports it
`failed`/`host_restart`) and then stopped, so `mac-mini` is offline; the BOARD
session's agent exits 0 (`completed`). Needs `cargo build` in `apps/runtime`
too. A shot with `fresh=True` starts from a new page load.

**The core loop (slice 8, handoff §11).** `desktop-loop-0` opens Gateway
spec, taps **Start session**, picks the `storm` workspace and **Launch**es
in the UI; the session opens on Context. `step` `loop` (`agents.py`
`run_loop`) then finds that session by its context note, checks its write
vault is `personal`, and has its agent read the note through
`session_context`, edit BOARD and create `projects/storm/log/2026-10-08`
through the gateway. The loop shots are the result: unseen dots on BOARD
and the new log note (the `log` folder opened by a tap), and BOARD's
provenance link. The sessions step no longer launches `gateway-spec` itself.
`--only desktop-loop` runs just the three, since steps always run. A tree
row's accessible label is "{name}\nChanged by an agent since you last opened
it" while it carries a dot (slice 9: the name first), so the taps match
`^name($|\n)`.

**Unseen baseline (slice 9).** The harness device's first `agent-writes` load
for `personal` happens at sign-in, before any agent wrote, so its baseline is
empty and the loop's writes still dot BOARD and the log note. A device whose
first look comes after the writes shows no dots for them, by design.

## Deliberate, documented deltas

Filled in as slices land (for example: the real xterm surface instead of the
prototype's illustrative coloured lines, §10 of the handoff).

- **phone-02:** vaults are listed in the server's order (kit, personal, work)
  rather than the prototype's; `kit` has no colour because the server seeds
  it (the fixture colours personal and work through `_storm/vault.md`);
  "1 running" needs a live session, which the harness gains with slices 5/6.
- **desktop-04:** the popover shows only the rows the fixture's real state
  produces — no hosts, no integrations, no version row until slice 5.
- **Slice 4 (desktop-01..03, phone-01/03):** recents are real opens the shot
  makes in order, so their ages read "1m" and the fixture's versions are v1
  (the references' v51/v14 and 2h/3h/4d are sample data); the rail dot is
  green and the badge absent (no hosts or integrations until slices 5/6); no
  provenance link or unseen dots (slice 8); the search hint reads "Ctrl K"
  because headless Chromium reports Linux (⌘K on macOS); the drawer shows the
  existing `NoteProperties` rows (created/modified chips, the colour
  swatches) rather than the prototype's simplified list; the note keeps its
  `color:` wash (Gateway spec is sage); the tree sorts notes by name.
  *Closed in slice 9:* the note body is now Newsreader ≈18 / 1.6 with a ≈22
  H2, in `text` (§7.2, the prototype); the pill draws the prototype's Lucide
  outlines; phone insets are 20.
- **Settings (desktop-12…21, phone-10/11), slice 7.** Every value is the
  fixture server's real state, so the sample data differs: one signed-in
  web device (plus the harness's `e2e` device) instead of three; the key reads
  "never used" and has no `stk_••••3f9a` hint (`GET /v1/keys` carries no
  prefix); the vaults are kit/personal/work under the temp root, with no
  missing `archive` row (the missing-row drawing is covered by widget tests);
  no integrations beyond Storm vaults, so no Integrations nav dot; no hosts
  ("No hosts enrolled yet." / About & health "No hosts enrolled · Enroll");
  address `127.0.0.1:7483`; Versions `1.0.0+1` (the local pubspec stamp).
- **Awaiting slice 5:** "Allow writes when chosen at launch" is drawn off and
  disabled with "Needs a newer server" until `GET /v1/config` returns
  `agent_writes`; Advanced › Versions shows the client only and About &
  health has no compatibility row until it returns `version`.
- **Deliberate:** phone settings pages and the settings list now sit on the
  shared 20 inset (slice 9), level with the bubbles. This device's Text size has − / + steppers and Note font
  opens a picker (the prototype's labels are static; the real controls stay);
  shortcuts read `Ctrl+` off macOS; a missing vault keeps **Remove**, the
  only way to clear one; Integrations keeps a ⋯ menu for Test, Replace token,
  Disable and Disconnect beside the row action; phone pages start below the
  corner bubbles at slice 3's inset, so "‹ Settings" sits ~14px lower.
- **Agents (desktop-05…11c, phone-04…09), slice 6.** The terminal is the real
  xterm surface with what the scripted agent printed about the calls it really
  made, not the prototype's illustrative lines (handoff §10): no "✕ host
  restarted" line on lint-fix (the real terminal just stops), no "ran 412
  tests" on test-sweep, and the cursor stays drawn on an ended session.
  Session names are the server's (Q12): `gateway-spec` from its note, `board`
  for the BOARD session, `site-1` and `storm-1` for the two without a note,
  where the prototype has hand-picked `test-sweep`, `docs-pass`, `lint-fix`.
  Ages are real ("now", "ran 1 s", today's clock) and versions the fixture's
  (v1, v3). No integrations are configured, so the infra line reads "0
  integrations", the risk box names none and About › Integrations is "None",
  and the rail dot is green. The launcher's Workspace defaults to the host's
  first, and the runtime lists workspaces by name, so it reads `site`; the
  overview's work cards are in newest-session order. The phone keeps the
  existing extra keys (⇧, ↑, ↓ besides the prototype's Esc Tab Ctrl ← →
  Paste). phone-08 is the note screen pushed over the session; its back link
  reads "‹ gateway-spec" in full (slice 9): the phone header also carries ⋯
  and the properties button, which the prototype does not draw, so when the
  name and the controls do not fit on one line the controls move to a second
  line rather than cutting the name. The corner bubbles stay (the prototype's
  pushed note has none). The
  Context tab's body uses the note view's own size and colour.
- **The loop (desktop-01, loop-a/b), slice 8.** Real state, so: BOARD is v2
  (desktop-01, edited by the `board` session) and v3 (loop-b, edited by
  `gateway-spec`), not v51/v52; ages read "just now"; BOARD's body is what
  the scripted agents wrote (a "Flaky sync test" line and a "Done" section),
  where the reference's agent rewrote the list. The log note is
  `2026-10-08` under `log`, as in the reference. **No unseen dot on RECENT**,
  matching the prototype and the references (the brief asked for RECENT too;
  the prototype wins). The drawer keeps the existing property rows, as in
  slice 4. The rail badge counts the harness's other live session too.
- **Slice 9, accepted as they are (no change).** The folder tree orders
  notes by name (the prototype hand-orders them); the launcher's Workspace
  defaults to the runtime's first (by name); vaults are listed in the
  server's order. Each is the real backend's order rather than sample data's.
- **Slice 9, properties drawer default.** Rail 56 + sidebar 260 + drawer 280
  leave the note pane `W − 596` between 900 and 1200 (sidebar and drawer are
  at their floor there), so the prose column is `W − 676`: 224 at 900, 424 at
  1100, 524 at 1200. Keeping it open only where the full 640 measure fits
  would need 1316 px and would shut it in the 1280 reference shots, which draw
  it open; so the drawer starts **open from the design frame (1200) up**,
  where the prose is the prototype's own 524, and **shut below it**. The
  toggle still opens it at any width and the choice sticks between notes.
  `sweeps/d900-p360/desktop-01` and `sweeps/d1100-p390/desktop-01` show it
  shut.
- **Slice 9, preset pass.** Storm light and SlowFlow earth keep every
  semantic: the provenance link and the unseen dot are `accent`, running dots
  `accent`, failed `danger`, ended `text3`, and the rail's `danger` dot (the
  offline `mac-mini` host). One finding, not changed: under SlowFlow earth the
  accent is a low-chroma brown (oklch 0.40 0.06 55, the handoff's own value),
  so a running dot and an ended (`text3`) dot are close
  (`presets/slowflow-earth/desktop-06…`); the RUNNING / ENDED sections and the
  status words carry the distinction.
- **Slice 9, resize sweep.** No overflow at 900, 1100 or 1600, or at 360,
  390 or 430: long names ellipsize (a work row's "gateway…" and a sidebar
  meta's "complet…" at 900), the drawer starts shut below 1200, and at 1600
  the side columns grow to their caps with the prose held at 640.
- **Not run on this host:** Android back on a device or emulator (no Android
  SDK, `adb` or emulator here; the back contract is widget-tested — see
  `CHECKLIST.md`), and the plan's manual Claude Code / OpenCode start (the
  CLIs are installed but would run on the operator's account; the scripted
  agent exercises the same opening prompt and `session_context` path).

The §11 checklist with evidence for every item is `CHECKLIST.md`.
