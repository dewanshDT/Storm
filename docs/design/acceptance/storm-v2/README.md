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
```

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
| `desktop-01-notes-provenance.png` | same name | 4 + 8 | — |
| `desktop-02-note-start-session.png` | same name | 4 | — |
| `desktop-03-vault-switcher.png` | same name | 4 | — |
| `desktop-04-health-popover.png` | same name | 3 (+4 for sidebar/header) | rail + popover captured 2026-10-08; health rows are the fixture's real state (sync only) |
| `desktop-05-new-session-launcher.png` | same name | 6 | — |
| `desktop-06-agents-overview.png` | same name | 6 | — |
| `desktop-06b-agents-first-session.png` | same name | 6 | — |
| `desktop-06c-agents-no-host.png` | same name | 6 | — |
| `desktop-07-session-running.png` … `desktop-11c-session-end-confirm.png` | same names | 6 | — |
| `desktop-12-settings-this-device.png` … `desktop-21-settings-about-health.png` | same names | 7 | captured 2026-10-08 on `feat/v2-settings`; a key and a relay seeded through the API (`("api", …)` actions); deltas below |
| `desktop-loop-a-unseen-dots.png`, `desktop-loop-b-edited-by-session.png` | same names | 8 | — |
| `phone-02-place-picker.png` | same name | 3 (+4 for the root list, pill) | captured 2026-10-08; deltas below |
| `phone-10-settings-list.png`, `phone-11-settings-page-ai-access.png` | same names | 7 | captured 2026-10-08; deltas below |
| `phone-01`, `phone-03` … `phone-09` | same names | 4–8 | — |

Agent states need an enrolled host: from slice 5 the harness also starts
`storm-runtime` with the `fake` provider (it exists for exactly this) and a
scripted MCP client acting as the session's agent, so sessions, Wrote rows and
provenance in these shots are real server state, never client fixtures.

## Deliberate, documented deltas

Filled in as slices land (for example: the real xterm surface instead of the
prototype's illustrative coloured lines, §10 of the handoff).

- **phone-02:** vaults are listed in the server's order (kit, personal, work)
  rather than the prototype's; `kit` has no colour because the server seeds
  it (the fixture colours personal and work through `_storm/vault.md`);
  "1 running" needs a live session, which the harness gains with slices 5/6.
- **desktop-04:** the popover shows only the rows the fixture's real state
  produces — no hosts, no integrations, no version row until slice 5.
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
- **Deliberate:** This device's Text size has − / + steppers and Note font
  opens a picker (the prototype's labels are static; the real controls stay);
  shortcuts read `Ctrl+` off macOS; a missing vault keeps **Remove**, the
  only way to clear one; Integrations keeps a ⋯ menu for Test, Replace token,
  Disable and Disconnect beside the row action; phone pages start below the
  corner bubbles at slice 3's inset, so "‹ Settings" sits ~14px lower.

