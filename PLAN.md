# Storm — implementation plan and status

> **This is the living plan. Read it at the start of every session, and update
> it as part of doing the work — not afterwards.**
>
> - Before starting: read the **Status** table and the **Decision log**. The
>   decisions there are settled; don't relitigate them without a reason, and if
>   one does change, record why.
> - While working: when a milestone's state changes, when you learn something
>   that would have changed an earlier choice, or when you hit a blocker, edit
>   this file in the same change. A finding that only exists in a chat
>   transcript is lost.
> - Before finishing: make sure **Status**, **Blockers**, and the milestone's
>   own section reflect reality. If a milestone is partly done, say which part.
>
> `docs/prd.md` is the original brief and is **not** maintained —
> where the two disagree, this file is current. See **Decision log** for what
> changed and why.

---

## Context

The setup being replaced: Obsidian + Syncthing in a VM on `pve-II`. Syncthing is
a generic file mover — no note awareness, no real conflict resolution beyond
`.sync-conflict` copies, no web access, and a whole VM whose only job is
shuttling `.md` files around.

Storm replaces both: Flutter clients talking to a small Rust server in the
homelab that owns the canonical vault. Plain markdown on disk stays
non-negotiable — it's what makes the vault greppable, backupable, and escapable.

---

## Status

| | Milestone | State | Evidence |
|---|---|---|---|
| M0 | Editor spike | **done** | retired · `docs/editor-findings.md` |
| M1 | Rust server core | **done** | 91 tests, 0 clippy · 43 e2e checks |
| M2 | Client vertical slice | **done** | superseded by M3 (online-only path) |
| M3 | Cache, outbox, offline, merge | **done** | sync matrix below |
| M4 | Search, tags, backlinks | **done** | 196 tests + 19 live · search p95 1.1ms |
| M5 | Android, Web (Linux deferred) | **done** | perf gate passed on device: 8.6 ms p95 |
| M6 | Attachments, settings, deploy | **done** | deployed to the VM; backup/restore verified |
| M7 | UI refactor stage 1 — shell | **done** | 221 tests · web deep links serve 200 |
| M8 | UI refactor stage 2 — editor | **done** | 309 tests · toolbar, links, formatting, autocomplete |
| M9 | Multi-vault server + configurable root | **done** | 138 Rust tests · 81 e2e checks |
| M10 | Folders, vault dashboard, recents | **done** | 326 Dart tests · folders, grid, recents |
| M11 | Typed note properties | **done** | properties, colours, fonts |
| M12 | Adaptive layout for wide screens | **done** | 453 Dart tests · sidebar, tree, flowing grid |
| M13 | MCP — read-only tools | **done** | 144 Rust tests · 33 MCP e2e checks · 9 tools, off by default |
| M14 | The design system, applied | **done** | 522 Dart tests · tokens, chrome, every screen |
| M15 | Releases, versioning, apt repo | **done** | v0.2.2 · apt Pages · VM on packaged install |
| M16 | Marketing / home site (Astro) | **in progress** | SlowFlow redesign shipped in `apps/www` · post-M19 truth pass + `www-check` (65) · CF hostname still TBD |
| M17 | Markdown Read Mode | **in progress** | `flutter_markdown_plus` · Read default · Edit keeps source editor |
| M18 | Desktop keyboard shortcuts | **done** | Intents/Actions · platform Meta/Ctrl · find + sidebar collapse |
| M19 | Auth phase 1 — server identity, users | **done** | slices 1–16 + A14 MCP keys + **the A10 cutover** · `STORM_TOKEN` removed entirely · pairing, sessions and MCP keys are the only credentials · authorization is its own release |
| M20 | Agent Runtime V1 — Runtime Hosts, sessions, terminal | **done** | decisions 77–78 released in **v0.3.0**, fixes in **v0.3.1** (#72, #73) · **accepted 2026-10-05 on the operator's daily use on Android, macOS and web** (AC items not logged one by one) · AM22 (host-owned terminal protocol) drafted, awaiting approval |
| M21 | MCP Gateway V1 — integrations through Storm | **in progress** | decision 81 · spec approved (vault, rev 3) and **frozen 2026-10-08** (C3 passed) · M20 accepted 2026-10-05, so the build may start (G-D1) · **built** in eight slices (81a): the store (81b), connections (81c), the upstream client (81d), the gateway route (81e), the runtime bridge (81f), OAuth (81g), the client (81h), acceptance (81i), plus a simplify pass (81j); then, to the frozen spec, the new-tools notice (81k), `storm://oauth` (81l), AM-G11 (81m) and the Add-integration UI (81n); **merged into `staging` 2026-10-08 (#76–#93)** · **Android native-OAuth acceptance passed** (Notion, Linear) · released in **v0.4.0** · left: macOS acceptance and the journal grep on the real build |
| M22 | Storm v2 — activity rail, single user, the knowledge ↔ agent loop | **done, released in v0.4.0** | decision 82 · design approved (`design_handoff_storm_v2/`) · plan, single-user migration and acceptance harness in `docs/design/` · slices 0–9 (#95–#105), the real-run fixes (#106) and the layout pass (#107) **merged into `staging` 2026-10-08** · real Claude Code run passed · handoff §11 43 of 44 (Android back on a device not run) · open: the login hang in Zen / Firefox (vault: *Storm v2/Issue — Zen login hang*) — cause found (an empty stored server address) and fixed, a real Firefox signs in on every PR (decision 90); confirmed in the operator's Zen 2026-10-12 |
| M23 | macOS Runtime Hosts — the same host, on launchd | **done, released in v0.5.0** | decision 83 · spec amendment D14 (AM33–AM41) in the vault, *Agent Runtime/macOS Runtime Host* · seven stacked PRs #110–#116 (83–83i), **merged into `staging` 2026-10-10** · real-Mac acceptance on `macos-latest` 99 passed, 0 failed, 5 manual · the operator's manual Mac round 28 PASS / 11 HUMAN / 5 FAIL / 1 SKIPPED, its 12-item backlog fixed in the stack (83i) · left: the operator's re-test of those fixes (vault: *Agent Runtime/macOS Re-test Prompt (fix loop)*), an Intel Mac |
| M24 | Interactive installer — one script for server, runtime, app and pairing | **done, released in v0.6.0** | decision 84 · spec + plan in the vault (*Installer/*) · #120 server commands, #121 macOS server (LaunchAgent), #122 release assets, #128 the script (111 bats tests, bash 3.2) + end-to-end CI (clean Ubuntu and macOS runners), #129 docs — **merged into `staging` 2026-10-10** · left: the operator's runs (a Mac runtime-only to prod; a laptop Everything) |
| M25 | Session experience — agent-named sessions, logo rows, native image paste, terminal settings | **done, released in v0.7.0** | decision 85 · spec amendment D15 (AM42–AM47) in the vault, *Agent Runtime/Session Experience — D15*, approved 2026-10-10 · seven stacked slices 85a–85g (#123–#127, #130, #131) + Android 16 back (86, #134), **merged into `staging` 2026-10-11** · live-test races (86a, #137) · xterm2 vendored for Gboard images (87, #138) · real Claude Code run passed (85) · Mac first-run fixes (88) · Resume reopens the agent's own conversation (89, on `staging`) |
| M26 | Windows — the Runtime Host as a Windows Service, and the server per user | **done, released in v0.8.0** | decision 99 · spec amendment D16 (AM48–AM57) in the vault, *Agent Runtime/Windows Runtime Host*, and *Installer/Windows Server* · stacked slices on `staging`: 99a carrier, platform and service (ConPTY, Job Objects, named pipe, Windows CI) · 99b the enroll sheet's Windows steps · 99c the server (Scheduled Task, `storm-serverw`, backup, filename rules) · 99d release zips, `install.ps1`, docs, Windows acceptance · **acceptance on `windows-latest` green** (AC-W1–W9 68/0/3, server 28/0, `install.ps1` end to end) · **merged into `staging` 2026-10-12** (#154 `fdff6fa`, #155 `54f0021`, #158 `1ef580e`, #160 `13ceb3e`) · left: a real Windows 11 round (a logged-in CLI, the phone handoff, a reboot, logoff/logon, a Notepad edit reaching the watcher) |

**Release state (2026-10-12).** **v0.8.0 is being cut** (decision 72's
steps; this sentence is the prep PR's), at the operator's request after #165
merged into `staging`. It carries **M26, Windows** (decision 99): the Runtime
Host as a Windows Service (`NT SERVICE\StormRuntime`, ConPTY, Job Objects)
and the server per user as a Scheduled Task, shipped as release zips with
`install.ps1`, and the enroll sheet's Windows steps. It also carries **Resume**
(89: an ended Claude Code or OpenCode session reopens its own conversation),
the Firefox / Zen sign-in fix and a real Firefox sign-in on every PR (90),
the terminal re-claiming a PTY sized on another device (103), the compact
phone session header (104), the Mac keychain relaunch fix (#144) and the
Agents sidebar row gap (#145). **Upgrading prod:** Resume needs both
`storm-server` and `storm-runtime` 0.8.0 — `agent.db` gains
`sessions.harness_session` and `sessions.resumed_from` by `ALTER TABLE` on
start (additive); with an older runtime Resume is hidden. Nothing else needs
a migration. **v0.7.1** (PR #143, `a570b5f`, release run 38130833301,
2026-10-11) carried **decision 88**, the Mac first-run fixes: the
installer offers a first workspace (and `--workspace NAME|URL`), the launcher
shows an empty host's workspace folder with **Check again**, an image paste
to a host without the new `images` capability fails at once (409) instead
of after 30 s, and a held Ctrl+V no longer reaches the agent as `^V`. No
migration. A 0.7.0 runtime does not report `images`, but stages images, so
the server also takes a reported version of 0.7.0 or later as able (found in
the prep, before the tag). The launcher shows an empty host's folder only
once its runtime is 0.7.1. **v0.7.0** (PR #140, `6ed9de4`,
release run 38094345519, apt `0.7.0-1`) and **prod runs it**: the operator
upgraded the VM's `storm-server` the same day, with a state tarball taken
first (vault: *Storm/Prod Upgrade — v0.7.0*). The codebox and omarchy Runtime
Hosts still run 0.4.0; a Mac host was enrolled on 0.7.0 with the installer,
and its first run found decision 88's three gaps. It carries **M25, the session experience** (decision
85): sessions named by the agent's own terminal title, logo rows with a
working / idle badge, **pasting and dropping images** into a session on
desktop, web and Android (staged on the session's host, then bracket-pasted
as a path), images inserted from Gboard (87, xterm2 vendored as
`5.2.0+storm.1`), and **Settings › Terminal** (text size, line spacing,
padding); Android 16's predictive back goes to the parent page (86); the
flaky identity-leak test and two live-suite races are fixed (#117, 86a).
**Upgrading prod:** titles, activity and image paste need both
`storm-server` and `storm-runtime` 0.7.0 — `agent.db` gains
`sessions.title` by `ALTER TABLE` on start (additive, no version bump);
older hosts and servers keep working without those features (an image paste
to an older host fails; 88 makes that fail at once). **v0.6.1**
(PR #136, `14049f1`, 2026-10-10) carried M24, the interactive installer and
the macOS server (v0.6.0, `2b0956c`, was tagged but never published);
**v0.5.0** (PR #119, `39cf955`) M23, macOS Runtime Hosts; **v0.4.0** (PR
#109, `018a063`) M21 + M22 (`auth.db` v6), on prod until 2026-10-11. v0.3.1 (PR #75), v0.3.0 (PR #71), v0.2.9 (PR #48), v0.2.8
(PR #37). **Known issue (still open):** the web client can stay on the
spinner after a correct password in Firefox-based browsers (vault: *Storm
v2/Issue — Zen login hang*).

Last updated: 2026-08-19. M0–M15 deployed. VM runs `storm-server` **0.2.2-1**
from apt (state `/srv/storm/state`, vaults on NAS `/mnt/media/Docs/storm`, web
`/usr/share/storm/web`). Android keystore still optional. **M16** Astro site
(`apps/www`) redesigned onto SlowFlow earth tokens with Storm-own product
positioning (no competitor framing), MCP-forward homepage, and real apt
install CTAs. CI builds on PR/main. Deploy remains Cloudflare static (no
GitHub deploy workflow). Decision 49 is the site hosting split. **M17** adds
a Read Mode on the note screen (Storm-styled `flutter_markdown_plus`); the
existing editor remains Edit Mode. Markdown stays canonical — no AST, no
server/MCP/sync changes. **M18** adds desktop-first keyboard shortcuts
(Shortcuts/Actions/Intent), platform-aware Meta vs Control, in-note find,
and wide-layout sidebar collapse — phone touch layout unchanged. Shipped in
**v0.2.4** (M18 is client-only; the server release is a version stamp).
**M19** starts authentication (decisions 52 / 52a). Six slices — five server,
one client — are complete and **merged to main** (PRs #10, #11, #13–#16,
2026-08-17). Nothing is deployed: the VM still runs 0.2.2-1 with the shared
token, and `legacy_token_enabled` defaults on, so merging changed nothing for
any live client. **Slice 1:** the server mints and keeps its own cryptographic identity,
`state/auth.db` exists with the full designed schema, backups carry it, and
`GET /v1/server` / `POST /v1/server/challenge` answer unauthenticated. **Slice
2:** local user accounts with Argon2id passwords at the parameters measured on the
VM, plus `storm-server user …` and `passwd` — no new routes, because creating a
user over the network needs device auth (A8) and that arrives with pairing.
**Slice 3:** session domain — `(user, device)` sessions, access/refresh tokens,
replay detection, WS tickets, device registration and revocation. **Slice 4:**
three-tier HTTP middleware in `api.rs` — device tier for pairing and login, session
tier for vault ops and auth management, the shared bearer token retired. **Slice
5:** QR-based pairing — `POST /v1/pair`, `POST /v1/pairings`, bootstrap QR at
boot, `storm://pair` URI scheme. **`storm-server pair`** regenerates the
bootstrap QR from the CLI. Not deployed.

**Slice 6:** the Flutter client pairs — a first-run `PairingScreen` that takes a
`storm://pair` URI, verifies the server's Ed25519 identity, claims the nonce,
creates the owner and logs in; `Settings` carries the device credentials and
session tokens; `apiProvider` sends the session token.

**Slice 7:** the legacy-token switch becomes real (above). **Slice 8:** a
login-only screen — `/login` with an account picker from `GET /v1/users` on the
device credential, "Sign out" in settings so `logout()` finally has a caller,
and every wire error code mapped to its own sentence. *"Couldn't reach the
server" is now reserved for the one case that really is unreachable*, and a
test asserts no auth refusal can claim it; describing an HTTP refusal as a
network failure is the M9/M10 bug. Slice 8 also fixed a protocol divergence
found while wiring those messages: a rate-limited login answered `401` with
`retry_after_secs` discarded, where the table says `429` + `Retry-After`.

**Slice 9:** the client's credentials move out of `shared_preferences` into the
platform keychain. **Slice 10:** the vault access **boundary** — `vault_of` is
the only route to a vault handle and will not hand one over without an `Actor`
and an `Access`, with the shipped policy (`AllowAuthenticated`) deliberately
permissive. **Slice 11:** the identity behind an MCP request, below.
**Slice 12:** the first tests that present a device credential — which found
that the device tier deadlocked on every request, that the first-user endpoint
never closed, and that the Argon2id bound was decorative. Details below; **the
authentication path could not have worked against a real server before it.**

**Next, in order:** ~~the client half of the device tier → a pass against the
VM~~ (slices 13–16) → ~~**A14, MCP keys**~~ (done) → ~~the cutover~~ (done,
2026-08-20 — brought forward ahead of the authorization release, see below) →
the **authorization release**.

Slice 12 reordered this. The RBAC policy was next while the assumption held
that the authentication path worked and only lacked a permission model; it did
not work — the device tier deadlocked on its first request, so **no
authentication code has ever completed against a real server.** The client's
`test_live/` suite still authenticates with the legacy shared token, which
leaves its half of that tier untested in precisely the way the server's half
was. Close that, pair against the VM, and only then build a permission model on
top of a path known to run.

The RBAC policy itself is unchanged and still deferred on purpose: A9's seam
exists, roles are enforced against each other but nothing consults them for
access, and `vault_grants` remains the only table in the data model never read
or written. Q19–Q25 in *Auth Authorization Review (A9)* are the questions that
slice has to answer, left open so the boundary did not have to guess them. Not
deployed.

**Deploying M19 is its own step, and the risky one.** Everything so far has been
merged behind `legacy_token_enabled`, which defaults on — the shared token still
works, which is exactly why the merge was safe. Turning it off is the cutover,
and it is the first change in this project that can lock a working client out of
a working server. It needs, in order: a release carrying `auth.db` and
`state/identity/` through `backup_all()` on a real upgrade, a device paired
against the VM, and only then the flag off. **A backup that has not been
restored is a hope, not a backup** — and `auth.db` plus the key files are the
only part of `state/` that cannot be rebuilt by rescanning markdown.

**The flag was a constant, and is now the switch A10 specified.** It had been
`AppState`'s field, hardcoded `true` — not read from `vaults.json`, not a CLI
flag, not reachable from the app — so the cutover as designed (flip it off,
check, flip it back if something broke) meant editing source and redeploying,
precisely the irreversible-migration shape A10 was written to avoid. As of
2026-08-17 it is persisted in the registry, atomic in `AppState` so it applies
to the next request rather than the next restart, togglable at
`PUT /v1/config/legacy-token`, and shown in Server settings.

**Its default direction is the opposite of the MCP switches, deliberately.**
`mcp_enabled` defaults off on an older registry because the risk there is
exposure; this defaults *on*, because a registry written before the field
existed belongs to a server whose clients all hold the shared token, and
loading it as `false` would refuse every one of them on the first boot after an
upgrade. `Registry`'s `Default` is hand-written for the same reason —
`derive(Default)` yields `false` for bools, safe for MCP and a lockout here.

**One guard, and one that was deleted.** It refuses to switch *off* over the
legacy token itself: the caller would lose access with the response and could
not turn it back on, so reversibility would be a fiction. An "at least one
active owner" guard was also written, and the mutation campaign showed nothing
could make it fire — the last-active-owner invariant means zero owners implies
zero users, which implies no session, which means only the legacy credential
can reach the route, already refused by the first guard. Unreachable code,
removed.

**M9/M10 deployment.** Client and server together — M9 breaks the wire format,
so they cannot go separately. The migration ran clean: the reconcile reported
`scanned=7 indexed=0 updated=0`, which is the proof that the index was *carried
over* rather than rebuilt, and 106 version snapshots across 11 notes survived
with it. Notes kept their real versions (v32, v25, v11), not a reset to 1. Four
bugs came back from the phone the same day, all traced to one broken cache
migration and the error handling that disguised it — see the M9/M10 section.

**M11 deployment.** Server, web and APK, each verified by comparing the local
build's sha256 against `/proc/<pid>/exe`, the bytes served over HTTP, and the
installed package. *The server did change* — the `_storm/` exclusion touches
`db.rs` and `index.rs` — which a first reading of the diff missed. **Check
which files a commit touched before claiming a component is unaffected;** a
summary written from memory is not evidence.

### Verify the current state

```sh
make check       # clippy + analyze + 266 Rust and 581 Dart unit tests
make test-live   # 81 server + 56 MCP + 66 auth e2e checks, 19 client checks
```

`make test-live` starts a server, runs every integration suite and tears it
down, failing fast with the server log if it cannot bind.

The **81** is load-bearing during the auth work: `e2e.py` is deliberately left
alone, so an unchanged pass there is the evidence that a slice which adds
authentication has not quietly changed the existing API.

Both need a server. `apps/server/tests/e2e.py` creates its own fixtures under
`E2E/` and deletes them afterwards, so it is safe to run repeatedly against any
vault. The client's live tests live outside `test/` so a plain `flutter test`
never needs a server.

---

## Architecture

```
┌──────── clients (Flutter, one Dart codebase) ────────┐
│  macOS · Linux · Android · Web                       │
│  editor · UI/tree · drift cache(M3) · sync engine(M3)│
└──────────────────────┬───────────────────────────────┘
              REST + WebSocket (LAN HTTP)
┌──────────────────────┴───────────────────────────────┐
│ storm-server (Rust, axum)                            │
│ merge (diffy) · frontmatter · link/tag parser        │
│ FTS5 · change_log · file watcher · serves web bundle │
└──────┬──────────────────────────────┬────────────────┘
  /srv/storm/vault/              /srv/storm/state/index.db
  canonical plain markdown       derived index + history
```

`state/` is a **sibling** of `vault/`, never inside it, so the vault directory
contains nothing but notes.

M9 changes the left-hand box to a storage *root* holding one directory per
vault, and the right-hand box to one index per vault plus a registry:

```
  /srv/storm/vaults/<dir>/       /srv/storm/state/vaults.json
  canonical plain markdown       registry: root + id/name/dir
                                 /srv/storm/state/<vault-id>/index.db
                                 derived index + history, per vault
```

The sibling rule survives unchanged, and gains a companion: the storage root
holds vault directories and nothing else that Storm reads.

```
storm/
├── README.md                 front door
├── PLAN.md                   ← this file: status, decisions, blockers
├── CLAUDE.md                 agent guidance + invariants
├── Makefile                  cross-toolchain tasks (`make help`)
├── deploy/                   systemd units, env template, backup script
├── apps/
│   ├── server/               Rust — see apps/server/README.md
│   ├── client/               Flutter — see apps/client/README.md
│   └── www/                  Astro marketing site (M16) — see apps/www/README.md
└── docs/
    ├── prd.md                original brief, not maintained
    ├── www/                  M16 marketing page copy (mirrors Storm Website notes)
    ├── editor-findings.md    M0 editor measurements, incl. on-device
    ├── storm-ui-refactor.md  M7/M8 design brief
    ├── storm-multi-vault.md  M9/M10 design brief
    ├── storm-properties.md   M11 design brief
    ├── storm-adaptive.md     M12 design brief
    └── design_handoff_storm_design_system/  M14 tokens + HTML prototypes
```

A monorepo with `apps/` rather than `src/apps/`: `src/` conventionally holds
the source of one package, and nesting whole deployable apps inside it buys
nothing (`src/apps/server/src/main.rs` says "src" twice). Shared code, if any
ever appears, goes in a sibling `packages/`.

---

## Decision log

Settled. Each entry says what was decided and what would justify revisiting it.

**1. No Rust in the client** *(departs from PRD §4.2)*
The PRD wanted a shared Rust core so client and server could share merge logic.
But in the server-of-record model, merging only ever happens server-side — the
client sends `base_version` and the server resolves. The client needs HTTP, a
cache and an outbox, all native to Dart. This drops `flutter_rust_bridge` and
per-target FFI build config entirely.
*Revisit if:* the client ever needs to merge locally (it shouldn't while the
server is the only source of record).

**2. 3-way merge, not `yrs` CRDT** *(departs from PRD §6)*
One authoritative copy plus one user means optimistic concurrency with a diff3
merge against stored version history covers the real cases. It keeps markdown as
the only content store — a CRDT needs a binary sidecar per note — and gives
version history as a byproduct, for one crate (`diffy`).
*Revisit if:* adjacent-edit conflicts (see M1 findings) turn out to be frequent
in practice. The wire format already carries `base_version`, which a CRDT would
subsume rather than contradict.

**3. V1 targets: macOS, Linux, Android, Web.** No iOS.

**4. LAN-only, single shared bearer token.** ~~No TLS, no Tailscale, no public
exposure.~~ **Superseded 2026-08-20 by the A10 cutover (decision 54).** The
shared token no longer exists: authentication is per-device pairing, sessions
and MCP keys. The *revisit trigger this decision set for itself* — "per-device
token rotation must land first" — is what M19 was, and it has landed. TLS and
public exposure remain out of scope, so the LAN-only half still holds.

**8. Ship the server as a bare static binary, not a Docker image.**
It is a 5.4 MB statically-linked musl binary with no runtime dependencies, so
Docker's isolation buys nothing. PRD §4.6 chose LXC precisely to avoid a guest
OS, and Docker on Proxmox means either a VM (the thing being removed) or nested
containers in an LXC. It would also put a bind mount and a UID/GID mapping
between the container and the host user who wants to grep, rsync and
ZFS-snapshot the vault — friction against the property the project exists to
protect. Cross-compiling with `cargo-zigbuild` removes the only real argument
for containerising (build reproducibility).
*Revisit if:* the homelab standardises on Docker/Compose for everything else,
where ops uniformity would outweigh minimalism.

**5. Editor dims syntax markers rather than hiding them.**
A `TextEditingController` can't change the buffer's character count, so true
hiding needs zero-width rendering, which breaks caret arithmetic and hit
testing. V1 ships Obsidian's *source mode with good highlighting*.
*Revisit if:* a block-based editor (a `ListView` of per-paragraph fields) is
built — that's what real hiding requires, and it's a much larger job.

**6. Conflicts are written into the note with markers**, never rejected and
never spawned as a sibling file. Nothing is lost (the pre-merge server text is
in `note_versions`), it needs no conflict-resolution UI, and the failure mode is
deleting four lines — strictly better than hunting `.sync-conflict-*` copies.

**7. `spike/editor_spike/` — retired.** *(discharged)*
The M0 spike existed to answer one question and then to hold the perf harness
until it could run on a real device. Both are done: the gate passed on a Pixel
10 at 8.6 ms p95 (budget 16.7 ms), and the directory was deleted along with its
duplicated copy of the editor. Its findings live on in `docs/editor-findings.md`.

Worth keeping from it: the duplicated editor had to be hand-synced on every
change, and drifting once would have meant benchmarking code that was no longer
shipped. If another throwaway ever needs to share code with the app, make it a
path dependency rather than a copy.

**9. `go_router` owns "where am I", replacing the single-screen shell.**
The nav bubble's Context slot changes meaning by location, and a second flag
tracking that alongside the screen state is two things to keep in agreement.
A router makes it one. It also gave the web client working deep links, which the
old shell simply did not have — `/browse/Projects/Storm` and `/note/:id` now
serve the app instead of 404ing.
*Revisit if:* nested navigators are ever needed for split panes (a non-goal).

**10. The router is refreshed, never rebuilt.**
`routerProvider` must not `ref.watch(settingsProvider)`. Watching recomputes the
provider into a *different* `GoRouter` while the `MaterialApp` keeps holding the
old one — navigation silently stops working and the back stack is discarded on
every settings change. It listens instead, and pokes `refreshListenable` so
`redirect` re-runs on the same instance. There is a test asserting the router
instance survives a settings change.

**11. "Is the keyboard up?" is asked once per screen, above the `Scaffold`.**
Both obvious ways to ask are wrong from inside a Scaffold body.
`MediaQuery.viewInsetsOf` reads zero there, because `resizeToAvoidBottomInset`
works by *removing* that inset — the removal is the resize. `View.of(context)
.viewInsets` reads the right number but never rebuilds, since view metrics are
not an inherited dependency; it would answer once and keep that answer forever.
So `keyboardIsOpen()` uses `MediaQuery` and every screen calls it above its own
Scaffold, passing the result down to the nav bubble and the formatting toolbar.
That single call per screen is what guarantees they are never both visible and
never both gone.

**12. A prefix button toggles; a prefix *picker* does not.**
Re-applying a block prefix a line already has removes it, which is right for the
bullet and quote buttons — a lone on/off control has nowhere else to say "off".
It is wrong for the heading picker, which lists Paragraph as its own entry:
choosing "Heading 1" on a line that is already H1 means "make this H1". Getting
this backwards is what made the heading button look like it did nothing, since
most notes open with `# Title` — the exact line you would try it on.
`setBlockPrefix` takes `toggle:` and the picker passes false.

**13. The formatting toolbar is inside the text field's tap region.**
`TextField`'s default `onTapOutside` unfocuses on desktop and web, and an
unfocused field closes the keyboard — which hides the toolbar out from under the
tap and abandons the caret. `TapRegion(groupId: EditableText)` around the
toolbar says "this is part of the editor". Android and iOS keep focus anyway, so
this only bites on desktop and web, which is why it survived the phone.

**14. An async toolbar action must not be gated on the toolbar's own context.**
Opening the heading menu closes the keyboard, which makes `keyboardIsOpen`
false, which unmounts the toolbar — so by the time the menu returns a choice,
`context.mounted` is *always* false. Guarding on it meant headings silently did
nothing while every other button worked, because the rest apply synchronously.
The controller belongs to the editor and outlives the toolbar, so it is safe to
call after the await; what must not be touched is `context`.

**15. An ordered list is a sequence, not a repeated string.**
`1. ` on every line is what a numbered list looks like when nobody counted.
`setBlockPrefix` numbers the block, continuing from the item directly above the
selection so extending a list resumes rather than restarting, and a blank line
ends the run — which is where markdown starts a new list anyway. Enter inside a
list continues it through `ListContinuationFormatter`, a `TextInputFormatter`
because that is the only hook that sees an edit *before* it lands and can move
text and caret together.

**16. The toolbar mutates text only through the controller.**
`StormMarkdownController` owns `toggleInline`, `setBlockPrefix` and
`insertWikilink`; each computes text *and* selection and assigns `value` once.
Assigning `text` and `selection` separately fires two notifications with an
intermediate state whose caret points into a buffer that no longer matches it.
`wikilinks_test.dart` puts a recording controller behind the toolbar and fails
if any button reaches past those three methods.

**17. `go` replaces the stack; going *deeper* must `push`.**
Every navigation used `context.go`, which leaves exactly one route on the
stack — so the Android back gesture popped it and left the app instead of
returning anywhere. Two halves to the fix: browse, note, search and tags are
declared as *children* of the dashboard route, so any location builds a stack
with the dashboard beneath it (a deep link included); and opening a note or
drilling into a folder uses `push`, while the nav bubble still uses `go`
because it swaps top-level destinations rather than descending.
In-app back buttons hid this completely — they called
`canPop() ? pop() : go(parent)` and kept working throughout. The regression
test drives `handlePopRoute`, which is the real system-back signal.

**18. Autocomplete state is derived, never tracked.**
Whether a `[[` is open comes from reading the buffer back from the caret, not
from a "currently completing" flag. A flag has to be kept in agreement with
every edit, undo and caret move, and the first disagreement leaves the
suggestion list offering to complete text that is no longer there. Suggestions
also display the same string they are matched against — a list that matches on
one name and shows another looks broken even when it is right.

**19. Following a wikilink reads the caret, not a gesture.**
A tap inside an editable `TextField` is consumed for caret placement, so a
`TapGestureRecognizer` on a `TextSpan` never fires. The tap does its ordinary
job and `TextField.onTap` then asks `wikilinkAt()` what the caret landed in.
Touch follows on a plain tap; a pointer needs Cmd/Ctrl, because clicking into
text to edit it is the common intent. `contextMenuBuilder` adds "Open …" as the
discoverable path that does not depend on knowing a modifier key.

**20. One server hosting many vaults, not one server per vault**
*(departs from PRD §3)*
The PRD deferred multi-vault entirely ("one vault per app install initially").
The alternative that needs no server change at all is N processes, one per
vault, with the client holding a list of `(url, token)` connections — but that
is N systemd units, N backup runs and N ports to remember, and there is then no
single thing a "vault storage root" setting could refer to. One server owning a
root directory keeps the homelab surface at one unit, one token and one backup,
and makes the root a real setting rather than a fiction.
*Revisit if:* vaults ever need different access control. Per-vault tokens are
the natural shape and would push back toward separate processes.

**21. Vaults are tracked by UUID; `vaults.json` maps id to directory.**
The same reasoning that makes notes path-independent (see **Data model**, and
the invariant in `CLAUDE.md`). If the directory name were the id, renaming a
vault would orphan its index and its clients' cached notes. A registry keeps
display name, directory and id independent, so renaming either is an edit
rather than a migration. It is plain JSON rather than a SQLite database because
it is the one piece of state a human might need to fix by hand when something
has gone wrong, and the vault-is-greppable ethos should extend to it.

**22. Per-vault index and per-vault `seq`; the change stream stays global.**
`notes.path` is `UNIQUE` and `attachments.path` is a primary key, so two vaults
in one database would collide on any shared path — `Daily/2026-08-07.md` exists
in most vaults. One `index.db` per vault sidesteps that entirely and needs no
schema migration, at the cost of splitting `change_log.seq`, which was the sync
protocol's single global cursor. So the cursor becomes per vault too. The
WebSocket does *not* split: one socket carries every vault's changes with a
`vault_id` on each frame, because the alternative is one socket per vault held
open for vaults nobody is looking at.

**23. Recents are recorded server-side and served across vaults.**
`docs/storm-ui-refactor.md` §2.1 defined "Recent" as recently *edited*
specifically to avoid tracking a second timestamp. Multi-vault breaks that
bargain: sorting by `modified` across vaults means fetching every vault's tree
on every dashboard load, and the dashboard is the home screen. A `note_access`
table and one `GET /v1/recents` is one call regardless of vault count, is
genuinely "opened" rather than "edited", and gives the same list on the phone
and the desktop. It is a separate table, not a column on `notes`, because
`record_note` rewrites the notes row on every index update and an "opened"
timestamp must not bump `version` or append to `change_log`.

**24. The active vault is routed, with a persisted mirror — not a provider
family.**
The structurally pure option is `syncEngineProvider.family(vaultId)` and a
vault argument threaded through every consumer. But `apiProvider` already
rebuilds on any settings change and `syncEngineProvider` already watches it, so
putting `activeVault` in `Settings` gets engine teardown, socket close and
session rebuild from machinery that exists and is already tested — at roughly a
tenth of the diff. The hazard it buys is a stale frame where the route says one
vault and the providers still hold another; a `VaultGate` at the `/v/:vault`
route closes it by refusing to build children until the two agree, which is the
same post-frame reconciliation `NoteScreen._load` already does.
*Revisit if:* two vaults ever need to be live at once — split panes, dragging a
note between vaults, or cross-vault search. The family is the right answer then
and this becomes the wrong shape rather than a slow one.

**25. Storm never moves vault directories, and refuses a root change that
orphans every vault.**
Changing the storage root points the server at directories an admin has already
moved; it does not relocate anything. Relocating on the user's behalf would
make a mistyped path a data move. But the honest version of that has a failure
mode this project has already been burned by twice: point the root somewhere
empty and the server boots perfectly healthy with zero vaults, files safe on
disk and invisible to every client — which reads as "my notes are gone". So
both directions are made loud instead. At runtime, `PUT /v1/config` refuses with
`409` and names every vault that would be orphaned, unless the caller passes
`orphan_ok`. At boot, a `--vault` path that does not exist exits non-zero with
both paths in the message rather than starting empty. A vault whose directory
has vanished stays in the registry marked `missing`; it is never quietly
dropped.

**26. Property values live in the note; property *types* live in a hidden
vault note.**
`_storm/vault.md` is an ordinary markdown note carrying `storm.type.<key>`,
`storm.options.<key>` and a vault description. A note rather than a server
table, because it then syncs, merges, versions and backs up with everything
else — no endpoint, no schema, no wire change — and it stays greppable and
hand-editable, which is the property the whole vault is built around. It is
hidden from browse, search, recents and wikilinks, and excluded from the
server's note count so a vault card does not read one too high.
*Revisit if:* types ever need to differ per device, or a vault grows enough
config that a note stops being a sensible container.

**27. The client gets its own frontmatter writer.**
`set_scalars` could not be reused. It replaces a key's *single line*, which is
correct for stamping `id` and destructive for anything else: aimed at a `tags:`
block list it orphans the `- item` children and produces invalid YAML. It also
normalises CRLF, drops trailing comments, and does no quoting at all — safe
only because its callers write UUIDs and timestamps. `frontmatter_edit.dart`
keeps the same principle (splice lines, never serialize) with a span model that
knows where a value starts and ends.
*Revisit if:* the two writers ever disagree about which bytes are frontmatter.
Each points at the other in a comment for that reason.

**28. A list keeps the form it was written in.**
Block stays block, inline stays inline, and block indentation is copied from
the existing items. Normalising to inline would have been one line of code, and
would mean the first tag edit silently reformats a file the user did not ask to
reformat — the same objection that made frontmatter line-surgery in the first
place.

**29. A local failure is never reported as a network failure.**
Carried forward from M9/M10's bug, and now structural: cache writes go through
`_cache`, which logs and continues, and `create` no longer wraps its cache
write in the same `try` as its request. The server is the copy of record.

**30. The properties list is the only way to edit frontmatter.**
Raw-YAML mode is gone. It existed as the escape hatch for anything the panel
could not represent, and that made the panel a place where *some* metadata
lived while the rest hid behind a mode switch — the split the panel was
supposed to remove. Now every key in the block gets a row in file order:
editable where the writer can represent it, read-only where it cannot
(`id`/`created`/`modified`, nested maps, block scalars). Shown rather than
hidden, because metadata you cannot see is worse than metadata you cannot
edit. No disclosure, no divider under the list.
*The cost, accepted:* a malformed frontmatter block can no longer be repaired
from inside the app. The vault is plain markdown; that is what it is for.
*Revisit if:* users hit unrepairable blocks in practice.

**31. One breakpoint, not a matrix.**
900px, and nothing else. The only structural question this app has is "is
there room for a sidebar and a note side by side"; every additional threshold
would multiply the states needing tests for a distinction nothing makes.
Tablet portrait stays on the phone layout, which is the honest answer at that
width. `MediaQuery.sizeOf`, not a `LayoutBuilder`, because it is a property of
the window — every widget must agree on it regardless of the box it happens to
sit in, and it has to rebuild while a browser edge is being dragged.

**32. The tree and the drill-down share data and rows, not a widget.**
They are different navigation models: one replaces the screen, the other opens
a branch while everything else stays visible. A single widget doing both would
carry two sets of rules and two sets of bugs. What they do share is real and
one level down — `childrenOfFolder` derives every level in both, and
`EntryTile` draws every row, so a folder looks and behaves identically either
way. *Reuse the thing that is actually the same, not the thing that merely
looks similar.*

**33. The vault routes live under a `ShellRoute` so the tree can hold its
state.**
Wrapping each route's child individually rebuilt the whole subtree on every
navigation. A drill-down list does not care; a tree collapses the moment you
open a note. `ShellRoute` builds the frame once and swaps only the pane, which
also let `VaultGate` move from five wrappers to one. The paths are unchanged,
so decision 17 still holds, and `back_navigation_test.dart` is what proves it.

**34. The phone layout is the default; the wide branch is additive.**
Every adaptive test comes in a pair — the wide assertion, and its compact
counterpart. "It works on desktop" is half a result; the other half is that
nothing moved at 411px, which is the width this project exists for.

**35. The macOS app stays sandboxed, so its entitlements are load-bearing.**
Turning the sandbox off would make every entitlement question disappear in one
line, and it is the wrong trade for an app that talks to the network and reads
files the user picks. Keeping it means `network.client` and
`files.user-selected.read-only` are as much a part of the build as any Dart
file — and the failure without them is silent, which is why
`test/macos_entitlements_test.dart` exists.
*Revisit if:* something the app needs turns out to be impossible inside the
sandbox. Nothing so far is: the LAN server is reachable with `network.client`,
and ATS does not apply because `package:http` uses `dart:io` sockets rather
than `NSURLSession`.

**36. One logo, generated icons — and three source images, not one.**
`apps/client/assets/logo.svg` is the only drawing; every icon in the repo comes
from `tool/make_icons.py` plus `icons_launcher`. Three PNGs rather than one
because the platforms want different *pictures*: macOS the margin Apple
reserves around an icon, Android full bleed because its launcher masks, and an
adaptive foreground on transparency so the system can shift it. Linux is off —
that handler emits only snap packaging, which Storm does not use.
*Revisit if:* an SVG rasteriser is ever installed. `make_icons.py` renders
through `qlmanage`, the only renderer macOS ships, and works around its
flattening of transparency; `_render` is the single function that would change,
and regenerating icons would stop needing a Mac.

**40. The stored storage root wins over the flag.**
`state/vaults.json` is the source of truth for where vaults live; `--vault-root`
seeds a registry that does not exist yet, and a disagreement between the two is
logged rather than quietly resolved. Found by asking the obvious question of a
feature that had shipped: `Registry::load` overwrote the parsed root with its
argument, so a root set in the app was written to the file, ignored on the next
boot, and then *erased* by the next save — with any vault adopted under the new
root left registered as `missing`, which is the "where did my notes go" shape
decision 25 exists to prevent. A setting that does not survive a restart is not
a setting.
*Revisit if:* the root ever needs to be changed on a server whose registry
cannot be reached — in which case the honest answer is editing `vaults.json`,
which the module is deliberately written to allow.

**37. MCP is a caller of the domain layer, so the domain layer had to exist.**
`ops.rs` holds one plain async fn per operation; `api.rs` handlers and `mcp.rs`
tools both call it and neither owns logic. The brief's "MCP is never a second
implementation" could not be honoured by discipline alone — the logic lived
inside axum handlers a second caller cannot reach. **A new operation goes in
`ops.rs`.** One added to a handler is invisible to MCP; one added to a tool is
the drift this prevents.
*Revisit if:* an operation is genuinely HTTP-shaped — streaming, or a WebSocket
— in which case it stays in `api.rs` and MCP simply doesn't offer it.

**38. Writes exist, and are off unless switched on.** *(supersedes the runway)*
Originally: read tools now, writes after the freshly cut-over vault had had
ordinary use. The user overrode that on 2026-08-08, which is their call — so
`create_note`, `update_note` and `delete_note` ship, and the caution moved from
the calendar into the design instead.

Read-only is the default and a first-class mode, not a disabled state:
`mcp_writable` is a second flag that defaults false, cannot be on while the
endpoint is off, and **filters the tool router** — a read-only server does not
advertise the write tools at all, so an agent cannot pick one it was never
shown. Writes go through the same `put_note` with `base_version` and diff3 the
Flutter client uses, so an agent racing a phone resolves exactly as two phones
would, and `ops::broadcast_latest` means an agent's edit reaches every connected
device rather than waiting for a pull.
*Revisit if:* an agent turns out to need `move_note`, `tag_note` or
`append_to_note` — all Phase 2 designs, all the same shape.

**38a. Still no trash, and MCP does not get one of its own.**
`delete_note` removes the file immediately, exactly as the Flutter client's
delete does. `note_versions` still holds the text, so it is recoverable from the
index, but nothing in the vault is. Adding a trash only for MCP would make
agent-deleted and user-deleted notes behave differently, which is its own bug
class — if trash is worth having it is a Storm feature decided once, for every
client. The settings screen says this before write access is granted.

**39. MCP speaks HTTP only; `rmcp` is pinned exactly.**
No stdio shim: Claude Code connects to HTTP MCP servers directly with a bearer
header, so a shim would be a second thing to maintain for no capability. The pin
is `=3.1.2` because rmcp shipped five releases in the eleven days around the
2026-07-28 spec revision, and it is still Tier 2 — `ProtocolVersion::LATEST` is
2025-11-25, which Storm follows rather than announcing a revision the SDK's own
transport treats as provisional.
*Revisit if:* rmcp reaches Tier 1, or a client Storm wants only speaks stdio.

**40. There is no app bar anywhere inside a vault.**
The design puts the vault and settings bubbles at the top corners of every
vault screen and gives each screen its own header underneath them. A title bar
would be a third band of chrome above content that already says where it is —
the note's folder path, the directory's breadcrumb. `StormChrome` places the
bubbles, the header, the content and the nav pill; `StormScaffold` wraps it for
every screen but the note, which tints its own background and so builds its
`Scaffold` itself.
*Revisit if:* a screen appears whose header cannot be expressed as one row.

**41. Search and Tags are sheets, and still routes.**
The design draws both as overlays over the screen they came from, but
`/v/<vault>/search` and `/v/<vault>/tags` have to keep resolving — the web
client's deep links are the reason the router exists at all, and a test covers
them. `SheetHost` renders the screen the sheet belongs over and raises the
sheet on the first frame; dismissing it backs out the way the route came in, so
one system back from Tags still reaches the dashboard.
*Revisit if:* a third overlay needs its own route, at which point the pattern
is worth a route-level abstraction rather than a widget.

**42. Note actions are a long-press, not a menu button.**
Pin, attach, rename and delete left the app bar with the app bar. The design's
note chrome is back, path and a gear, and long-press is already how this app
offers a row's secondary actions — it is what a folder row does. The trade is
discoverability, accepted because the grammar is consistent and because three
of the four actions are rare.
*Revisit if:* someone cannot find delete.

**43. A literal in `lib/ui/` is a test failure.**
`test/token_conformance_test.dart` scans the source for `Colors.*`, a bare
`fontSize:`, a numeric `circular()` other than the 999 pill, and any
`OutlineInputBorder` outside `theme.dart`. A source scan rather than a widget
test because this class of mistake is invisible at runtime in the default
theme: `circular(8)` looks right under Storm dark, where `rControl` is 10, and
only goes wrong under SlowFlow, where `rCard` is 2 and every hardcoded corner
stays round while the cards go square.
*Revisit if:* a genuine exception appears; add it to the exemption beside
`theme.dart` rather than deleting the test.

**44. The skeleton does not shimmer.**
A repeating animation never lets `pumpAndSettle` return, so every widget test
that renders a loading state hangs. These lists come from cache and are gone in
a frame or two; the animation would cost more than it buys.
*Revisit if:* a loading state appears that is genuinely slow, in which case it
wants its own widget with a `TickerMode` guard.

**45. One tag builds every artifact, and nothing is hand-uploaded.**
The server binary, the `.deb`, the APK, the macOS app and the web bundle all
come out of the same tagged run. If `release.yml` did not build it, it is not a
release. Hand-uploading one artifact "just this once" is how a release ends up
containing two different builds, and nothing in the release itself would show
it.
*Revisit if:* a target appears that genuinely cannot be built in CI — iOS
signing would be the candidate — in which case it gets its own documented step
rather than a quiet exception.

**46. The git tag is the only version source.**
`Cargo.toml` and `pubspec.yaml` are stamped from `GITHUB_REF_NAME` during the
release build, never edited by hand. Both are static today, and the failure
mode is silent on both sides: apt reads the `.deb`'s control version, so a pool
of identically-versioned packages never upgrades, and Android compares
`versionCode`, so the second APK is refused as a downgrade. Renaming the output
files is not versioning.
*Revisit if:* client and server ever need to version independently — which
would mean the wire format is stable enough to mix versions, and it is not.

**47. The `.deb` owns the systemd install.**
`deploy/storm-server.service` and `storm.env.example` become package payload
rather than files someone copies by hand, and the maintainer scripts do the
`useradd`, the directories and the token generation that `deploy/README.md`
currently asks a human to remember. The env file is a `conf-file` so `dpkg`
prompts rather than overwriting a real token on upgrade.
*Revisit if:* the target box is ever not Debian-family. Nothing else in the
deployment assumes it, so this would be a packaging change, not a redesign.

**48. `up` configures; `serve` runs — and the data root is chosen at `up`.**
The long-running process is `storm-server serve` (what systemd starts). Operator
commands `up` / `down` / `status` own the install: write `/etc/storm/storm.env`,
widen `ReadWritePaths` via a drop-in when paths are not under `/srv/storm`, and
`enable --now`. Defaults stay FHS (`/srv/storm`); `up --data-root` is the common
case, and `--vault-root` / `--state` cover a split layout (NAS vaults + local
state on the current VM). The web client is package-owned at
`/usr/share/storm/web`, not under the data root, so an apt upgrade refreshes
the UI without touching notes.
*Revisit if:* a non-systemd host becomes a first-class target — then `up`
would need a different supervisor backend.

**49. Marketing site is Astro in `apps/www`, hosted apart from apt Pages.**
Astro over Mintlify/Docusaurus: marketing-first, thin pages, not a docs portal.
`https://dewanshdt.github.io/Storm/` is the **apt repository root**
(`apt-repo.yml` publishes reprepro as the Pages site root) — a marketing
deploy must never overwrite it. Host on **Cloudflare Pages** (recommended
default) or another static host / second Pages project on a custom domain.
CI builds on PR/main; deploy is Cloudflare Pages connected to the
repo (dashboard owns build + publish on push). Never chained into
`apt-repo.yml` or `release.yml`. Package manager: **npm** (no JS elsewhere in
the monorepo; one lockfile under `apps/www`).
*Revisit if:* Cloudflare is unavailable or a custom domain is already pinned
elsewhere — then Netlify or a second GitHub Pages project is fine as long as
it does not share the apt site root.

**50. Read Mode is a styled Markdown renderer beside the existing editor —
not a replacement and not an AST.**
Opening a note defaults to Read Mode (`StormMarkdownView` over
`flutter_markdown_plus`, Storm tokens) when `settings.readMode` is on
(the default). Edit Mode keeps the source `TextEditingController` editor
from M0/M8. Turning Read Mode off in client settings hides the Read /
Edit switch and keeps the source editor only — the pre-M17 note screen.
Markdown remains the only document format; there is no Storm AST, no
block editor, and no second persistence path. Read Mode always renders
`NoteSession.body` (including unsaved edits). Task-list checkboxes are
read-only in this phase.
*Revisit if:* Read Mode is not enough document quality and a real
block/AST renderer becomes worth the migration cost — decision 5's
block-editor revisit is the larger cousin of that call.

**51. Desktop keyboard shortcuts are Shortcuts/Actions/Intent, platform-aware,
and context-scoped — not RawKeyboardListeners.**
Chords use Meta on macOS (and Mac web) and Control elsewhere via
`defaultTargetPlatform`. Three nesting levels: global (search, new note/
folder, sidebar), note (save, read↔edit, find, Esc), editor-focus only
(bold/italic). Undo/redo stay with Flutter's text-editing defaults.
Web does not steal browser refresh/close (`⌘/Ctrl R` / `W` unbound).
Phone touch layout is unchanged — Shortcuts only fire on key events;
sidebar toggle is a no-op below the wide breakpoint. Deferred: shortcut
overlay, command palette.

Settled while shipping:

- **Esc is a ladder, not a jump.** Close the find bar → step out of Edit back
  to Read → *then* leave the note. An Esc that pops the note while the caret
  is mid-body is how you lose your place; with Read Mode off there is no lower
  rung and Esc leaves directly.
- **The header back arrow and Esc share one leave-note answer** (pop the
  stack, else fall back to `browse`, matching search/tags) — never a second,
  dashboard-shaped exit that the two could drift apart on.
- **The shell's global Focus autofocuses only where a physical keyboard
  exists** (desktop/web). On a phone it arms nothing and would fight a field
  that autofocuses on entry (the search field does).
- **Sidebar collapse is transient chrome, deliberately not persisted** — like
  the properties drawer, not a Settings field. Re-expanding is one chord, and
  a per-launch layout preference does not earn the Settings machinery.
*Revisit if:* a command palette becomes the primary action surface and
needs the same Intent set exposed as searchable commands.

**52. Remote access is two separate subsystems, and the relay is not one of
the authorities.** *(architecture accepted 2026-08-13; nothing built)*
Decision 4 said TLS and per-device credential rotation land **before** the
server is ever reachable off the LAN. This is the design that discharges it,
and it deliberately splits into two things that never answer each other's
question:

- **Authentication** — Storm owns it. Users are **local to a server**, so the
  canonical identity is `(server_id, user_id)` and there is no global Storm
  account; a server has its own cryptographic `server_id`; passwords with a
  modern KDF; revocable per-device sessions; QR pairing that carries a
  short-lived nonce and **never** a permanent secret. No external identity
  provider is the source of truth — a self-hosted server that cannot log a
  user in with the internet unplugged is not self-hosted.
- **Connectivity** — an optional, self-hostable WSS tunnel (`storm-relay`)
  that the *server* dials outward, so a NATed homelab box needs no port
  forwarding. It carries Storm protocol traffic only, never generic TCP, and
  it is an availability mechanism rather than an authority: no identity
  store, no vault store, no authorization ownership. Storm may host one for
  convenience (Cloudflare Worker + Durable Object initially); the protocol
  stays provider-independent, and the self-hosted relay is what proves it.
  **A relay outage costs remote access, never vault access.**

MCP rides the same transport rather than getting a network architecture of its
own — the same reasoning as `ops.rs` (decision 37), one layer down. Agent
Runtime later reuses it too, and is deliberately *not* built alongside it.

Order matters: authentication ships completely before any relay work starts.
Building both at once means debugging an identity bug through a tunnel.

Full architecture, the ADRs (R1–R11), the open questions and two
implementation checklists live in the personal vault under `Storm/Remote/` —
start at **Storm Remote Connectivity**. Decision 4 is not superseded until
authentication actually ships.
*Revisit if:* any of R1–R11 changes — amend the ADR with why rather than
relitigating here.

**52a. The auth design is complete; the build order puts it entirely before
any networking.** *(2026-08-13, still no code)*
Q1–Q9 are resolved as ADRs **A1–A11**: Argon2id with parameters *measured on
the VM* (verify on `spawn_blocking`), an Ed25519 server credential whose
private bytes are a `0600` file while its public metadata is a row, a
server-scoped **`state/auth.db`**, opaque blake3-hashed tokens resolved
server-side (never a JWT — revocation has to be immediate), three credential
tiers (none → device → session) so pairing creates a *device* and never a
login, owner/admin/member plus `vault_grants` with refusal as an explicit
`403`, `storm-server passwd` as the honest recovery bypass, and a **reversible
`legacy_token_enabled` switch** that lets `testtoken` keep working for one
release but can never create the first user.

Two findings from writing it that change existing code:

- **`auth.db` is the first thing in `state/` that cannot be rebuilt.**
  `backup_all()` walks vault indexes only, so a restore would produce a server
  with every note and nobody able to log in. It learns about `auth.db` in the
  same change that creates it.
- **Short session lifetimes are wrong here.** This client is offline-first
  with an outbox; a session expiring while a phone is out of range means an
  auth wall in front of edits that exist nowhere else. Access 30d / refresh
  180d sliding is safe *because* revocation is server-side and instant.

Phases: **1** auth, complete and local · **2** a `StormConnection` seam so
nothing above it knows local from relayed · **3** a minimal relay with server,
client and `storm-relay` all on one machine · **4** self-hosted relay on a VPS
— the milestone that actually solves the problem · **5** the Cloudflare-hosted
relay as a convenience. Remote MCP and the Agent Runtime transport come after.
Details in **Storm Auth Data Model** and **Storm Auth Protocol**.

**52b. Every auth slice is additive until the middleware slice, and the
challenge signs a domain-separated message.** *(2026-08-13, slice 1 shipped)*
Two things settled by building the first slice:

- **Additive, one slice at a time.** `require_token` and the shared token are
  untouched until the three-tier middleware lands, so each slice ships without a
  client change and `e2e.py`'s 81 checks pass unmodified. That unchanged pass is
  the evidence, which is why new coverage went into a *new* `tests/auth_e2e.py`
  rather than being mixed into the file that serves as the control.
- **`/v1/server/challenge` signs `storm-challenge:v1:<server_id>:<nonce>`,
  never the bare nonce.** The endpoint is unauthenticated by design, so it will
  sign whatever it is sent — a signing oracle unless the signed bytes are
  domain-separated and bound to this server. The nonce is bounded (16–128
  printable ASCII, no `:` or `"`) so it cannot forge the message's own field
  boundaries. The client rebuilds this string, so it is a wire-format
  commitment.

And one amendment to **A4**, found while writing `backup_all()`: the ADR named
`auth.db`, but the private keys live *outside* it at
`state/identity/<key_id>.key` (A2). Backing up the database alone restores a
server that knows which key is active and cannot sign with it — an identity
loss wearing a healthy-looking database. `backup-db` therefore copies the key
files too, at `0600`, into the same layout so a restore stays a plain copy. The
ADR in the vault is amended with this rather than the code quietly doing more
than the design says.
*Revisit if:* the middleware slice finds that "additive" cannot hold — at which
point client and server ship together, as decision 46 already assumes.

**52c. Accounts are created on the host until pairing exists, and the KDF gets a
concurrency bound rather than smaller parameters.** *(2026-08-16, slice 2
shipped)*

- **`storm-server user add` can create the first user, and any user.** A8 says
  the first account needs device auth plus an empty user table plus a console
  nonce — all of which is the pairing slice. Rather than stall the user model
  behind it, account creation is a host-side command, which is the same trust
  level A11 already grants `passwd`: root on the box can read `auth.db` and every
  vault anyway, so requiring more than shell access would be theatre. **A8 is
  amended** to name the CLI as a second, host-only path rather than the code
  quietly exceeding the design. A8's actual subject — what an *unauthenticated
  network client* may do — is untouched, and stays untouched until pairing.
- **A semaphore of 2, not a cheaper hash.** Q18's 192 MiB is only safe because
  the number of simultaneous verifies is capped; `spawn_blocking`'s 512-thread
  default would otherwise turn a login burst into an OOM that takes the notes
  offline. The temptation when that is discovered is to shrink the KDF, which
  trades an availability bug for a security one. `Hasher` owns the permits and
  is the only way to reach Argon2id.
- **Passwords: 12 characters minimum, refused rather than truncated above 1024
  bytes.** A homelab owner account is a single high-value credential with no MFA
  behind it. Silent truncation is the failure worth naming explicitly — accept
  200 characters, hash the first 72, and every password sharing that prefix now
  opens the account.
- **Usernames are ASCII and unique by casefold.** Uniqueness is decided on the
  fold, so the fold has to be unambiguous; Unicode brings locale-dependent case
  rules and homoglyphs that would make two visually identical usernames distinct
  rows. `display_name` is unrestricted.

*Revisit if:* pairing lands and the console-nonce flow makes the CLI creation
path redundant — at which point it stays anyway as the recovery path, beside
`passwd`.

**54. The shared token is removed, not disabled, and the compatibility window
is skipped.** *(2026-08-20, the A10 cutover)*

`STORM_TOKEN` no longer exists — not the flag, not the env var, not the
`legacy_token_enabled` switch, not the `Bearer` branch that accepted it.
Authentication is per-device pairing, sessions, and MCP keys.

**This overrides A10's own sequencing, and the reason is worth stating.** A10
specified a compatibility window: ship auth with the token still working, pair
every device, flip the switch, confirm nothing broke, remove it a release
later. That order exists so there is never a moment where someone is locked out
of a server they cannot yet log into. Skipping it was chosen deliberately over
shipping a release that contains a working backdoor, and the cost was priced
rather than discovered: **prod has no `auth.db`, so the upgrade locks out every
client**, and recovery is shell access to read a pairing QR out of the journal.

That trade is defensible *here* — a homelab, one operator, SSH always
available — and would not be on a machine somebody else runs. A future
deployment story that removes shell access has to revisit this, because
"recovery is SSH" stops being a recovery path at that point.

Two rules from A10 survive unchanged and are now unconditional: the shared
token could never create the first user, and it can now never do anything at
all. A third was added and then made moot: it could not mint an MCP key,
because it had no user for one to belong to.

*Revisit if:* Storm is ever distributed to operators who do not have shell
access to the machine — the bootstrap path (A8) is the only way in, and it
assumes a console.

**56. The relay's §0 prerequisites ship together, and the rate-limit numbers
are a decision rather than a measurement.** *(2026-08-26, branch
`worktree-relay-phase0`)*

Two defences gate the first line of relay code, and neither depends on the
tunnel. Both are now built.

*Login-path rate limiting.* `login()` runs a full Argon2id verify for a
username that does not exist — deliberately, so response time cannot enumerate
accounts — which means a junk username can never trip the per-user lockout. The
KDF's semaphore is global (2 permits, ~174 ms), so the whole server sustains
~11 verifies/sec. Web bootstrap hands a device credential to anything that can
fetch the app, so the full chain is: fetch app → pair → flood with random
usernames → login is dead for everyone. Two hand-rolled token buckets, no new
crate: **per-caller 30/min burst 30**, **global 60/min burst 60**. Per-caller
is generous because a relay collapses a household behind one NAT address;
global is strict because N addresses each under their own limit still saturate
a globally bounded resource, and it is the half that actually protects the
permits. Charged before the permit, refunded on success.

**The numbers are chosen, not measured.** *Revisit if:* a real deployment sees
legitimate users throttled (raise per-caller), or the global ceiling is reached
by anything but an attack (it is ~9% of the Argon2 ceiling, so that would mean
the derivation is wrong).

*Challenge-on-connect.* The client re-proves the server's identity on every
connect, including every reconnect, against the key pinned at pairing. It is
the property that lets the relay stay untrusted infrastructure, and it was the
one part of the relay's trust model that was designed but never running.

Three findings worth keeping, each of which cost a cycle:

- **`Option<ConnectInfo<SocketAddr>>` does not compile on axum 0.8.** There is
  no blanket `Option<E>` impl; `E` must implement `OptionalFromRequestParts`
  and `ConnectInfo` does not, so the obvious signature fails the `Handler`
  bound with an error naming neither. This is the same shape as
  `Option<WebSocketUpgrade>` on `stream`, already recorded — the checklist item
  saying `pair_handler` should become `Option<ConnectInfo<..>>` is therefore
  **not implementable as written** and needs a hand-written extractor.
- **A sixth `Option<&str>` argument is a silent wrong column.** Adding `remote`
  to `record_event` meant threading a `None` through twenty call sites with
  three interchangeable optional slots; the first cut put it in the `device_id`
  position everywhere and wrote every device id into `remote`. It compiled and
  the suite passed. `record_event` keeps five arguments and
  `record_event_from` takes the sixth.
- **A rate limiter cannot be tested by a debug build sending requests one at a
  time.** Each attempt costs a whole Argon2id verify (seconds unoptimised), so
  the bucket refills faster than requests arrive and the limiter correctly
  allows all of them. The router tests inject their own limits; the e2e suite
  fires a concurrent burst, which is what a flood actually looks like.

*Revisit if:* nothing here — these are properties of the tools, not choices.

**57. Query-string authentication was dead, and the symptom had been filed as a
flaky test.** *(2026-08-26, branch `staging`)*

A WebSocket handshake carries no headers and neither does an `<img>` request,
so `require_auth` also accepts the credential as `?token=`. Two independent
faults meant that path could not authenticate anyone:

- the server **never percent-decoded** the query value, so a conforming client
  sending `token=Bearer%20abc` was matched raw against `"Bearer "` and missed;
- both client call sites — the change-feed socket and `attachmentUrl` — sent
  the **bare** token, which names no scheme and matches neither
  `"Bearer "` nor `"StormDevice "`.

Neither was noticed because the shared token contained no space and was
compared whole. It was the only credential the raw, scheme-less form ever fit,
so **removing it (decision 54) took query authentication with it, silently.**
There was no test on the path at all, which is the A10 lesson recurring in the
same release it was written about: *a credential every test is handed is a
credential no test checks.* The one client test that did touch it asserted the
bare token — it encoded the bug and passed while the URL it checked
authenticated nothing.

**The finding worth keeping is about the symptom, not the cause.** This was
visible for six days as `two_client_sync_test.dart` scenario 2 — WebSocket push
reaching the other client's cache — recorded in the vault as a timing flake
that "failed once under CI load, passed on re-run". It was never a flake. The
socket 401s, `_scheduleReconnect` fires, and the pull that follows does the work
the push should have done; the test passes or fails on whether a 1s-to-60s
backoff beats its own timeout. Every user has been on reconnect-driven polling
rather than push since v0.2.6.

> **A flaky test is a diagnosis, and it is the easiest one to reach for.**
> Nothing else in the system reports a degraded-but-working path, so the only
> place the breakage could surface was a test that sometimes lost a race —
> and "flaky" is exactly the label that stops anyone looking further.

The fix: the server decodes, and both clients send `api.credential` — one
accessor shared by the header path and both query paths, so they cannot drift
apart again. A server test pins both halves (bare refused, scheme-prefixed
accepted), because the contract had none.

*Revisit if:* the credential stops travelling in a URL.
`POST /v1/auth/ws-ticket` exists for exactly that reason and mints a 60-second
single-use ticket, but **nothing consumes it** — `consume_ws_ticket` has no
callers. Wiring it up is the real answer and is now the recorded follow-up; a
session token in a query string lands in proxy logs and browser history, and
this decision only restores the behaviour that was intended, not the one that
is right.

**58. The published wire spec described a protocol where the relay
authenticates clients.** *(2026-08-28)*

`docs/srp-v1.md` §4 said *"a device registers with the relay"* and *"the client
generates or loads its device keypair (Ed25519, the same pair pinned at
pairing)"*. There is no such keypair — pairing pins the **server's** public key
and hands the device a shared secret. The section described client
authentication at the relay, which is exactly what **R12** forbids and what
pass 0 retired Q13 over. Five more wire details were wrong in ways that would
have compiled: the signed message omitted `server_id` and its separators, §4.1's
allowlist/TOFU/account-owned binding table was absent, §6 had no registration
messages at all, `stream_id` was client-allocated rather than relay-assigned,
and §3 invented uniform binary framing where control is JSON in text frames.

§3–§6 are regenerated from the accepted design, with an errata note kept at the
top of the file.

**Three things worth keeping, and none of them is "check specs more
carefully".**

*A transcription is not a copy.* The spec was written from the design note by
an agent that had read it, and the result is **internally consistent** — it
reads like a protocol, just not this one. That is precisely why a read-through
does not catch it: there is no seam to notice, no contradiction on the page.
The error is only visible against the source.

*Review finds what it went looking for.* I reviewed that file and recorded it
as reviewed. What I actually did was check the two things I had in mind —
Cloudflare mentions (R7) and whether `Last-Event-ID` was left open — plus the
non-goals, where I found and fixed two real contradictions. Finding something
is what made it feel reviewed. **The sections that turned out correct are
exactly the ones I wrote or touched; every section I did not read against the
design was wrong.** A review with a checklist covers the checklist.

*The first implementer is the first reviewer.* This was caught by the agent
building the relay against the file, on its first read, before writing code —
which is the argument for **R7** stated in miniature. A protocol with no
implementation is a protocol nobody has checked, and the same is true of a
spec: `docs/srp-v1.md` sat committed and cited for two days, and every note
pointing at it was pointing at the wrong protocol.

*Revisit if:* nothing here. The corrective is procedural and already applied —
the spec now carries its own errata, and the vault's *Relay Review Log* records
this as pass 7.

**59. The ws-ticket had no expiry, and the comment above it said it did.**
*(2026-08-30)*

Decision 57 restored query-string authentication to the behaviour that was
*intended* — a thirty-day session token in a URL — and recorded that the right
answer was `POST /v1/auth/ws-ticket`, which mints a sixty-second single-use
credential and which **nothing consumed**. Wiring it up found that the
mechanism did not do the one thing it exists for.

`consume_ws_ticket` had no callers. Its lookup filtered on `used IS NULL`
alone, while the comment directly above it read *"Returns (ticket_id,
session_id) for an unused, **unexpired** ticket"*. The function then checked
the **session's** expiry — thirty days — and never the ticket's. So a ticket
would have authenticated for a month, in a URL, which is the single place it
must not. `purge_expired_ws_tickets` ran only *during* a successful consume, so
nothing swept it either.

There were **no tests for ws tickets at all** — fifteen mentions in the file,
every one of them implementation. That is how a missing expiry survived being
written, reviewed and shipped.

Three things worth keeping:

*A comment is not a check, and an unexecuted one drifts freely.* The comment
described the intended behaviour and was written at the same time as the query
that failed to implement it. Nothing ever ran the pair together, so nothing
could disagree with it. **This is the third instance of the same shape in nine
days**: the `remote` column whose writer ignored it (56), query auth that no
credential could satisfy (57), and now this. Each was code that existed,
compiled, read correctly, and had never been called.

*The obvious fix would have been a second bug.* Adding `AND expires > ?` to the
SQL is the natural move and it is wrong here — `now_rfc3339()` emits however
many fractional digits an instant needs, so `…:44.1Z` sorts *before*
`…:44.05Z`. This file's own rule, stated at the top, is that timestamps are
parsed and never compared as strings. The check is in Rust.
`purge_expired_ws_tickets` still does the string comparison; it is best-effort
cleanup rather than a gate, and it is noted rather than silently left.

*The duplication was the tell.* `consume_ws_ticket` reimplemented two of the
four rules in `check` — the helper whose doc comment says it exists "so
[`authenticate`] and [`refresh`] cannot drift apart on them." A third caller
had drifted, in the direction the comment predicted, because it copied the
rules instead of calling them.

**Still exposed, deliberately:** `attachmentUrl` puts the session credential in
a query string, because an `Image.network` cannot make a round trip before
rendering. Narrower than a WebSocket's exposure and closing it needs a
different mechanism, so it is recorded rather than half-fixed.

*Revisit if:* an attachment mechanism appears that can carry a header, or the
credential in an attachment URL is shown to leak somewhere that matters.

**60. `storm-relay` exists, and building it found the spec's own security rule
was half-written.** *(2026-08-30)*

Phase 3's first slice: a standalone crate that accepts server registrations and
authenticates them. Registration only — client trunks, streams, routing and
`relay_peer_ip` are absent rather than stubbed, and the error enum defines only
the two codes the crate can honestly send.

**The finding.** A relay-auth message is
`storm-relay-auth:v1:<server_id>:<nonce>` — two colon-delimited fields in one
signed string. The spec was emphatic that a nonce may contain no `:`, *"because
a nonce carrying `:` could make the signed bytes parse as a different
`(server_id, nonce)` split"*. It then said nothing whatever about `server_id`,
which is the other half of that split:

```
relay_auth_message("a:b", "c")  ==  relay_auth_message("a", "b:c")
```

Byte-identical. **A signature over one is a signature over the other**, and the
rule that named the hole did not close it.

On the server this was safe by accident — `server_id` is self-generated and
could never carry a colon. **On the relay it arrives from the wire**, chosen by
whoever is registering, which is exactly where it is reachable. The rule now
lives in `identity.rs` beside `validate_nonce` as the shared definition rather
than in the relay alone, because two implementations agreeing by coincidence is
what this class of bug is made of.

*The lesson is about the shape of the reasoning, not the omission.* The spec
did not fail to think about delimiter injection — it thought about it
carefully, wrote the argument down, and applied the conclusion to one of the
two fields the argument covers. **A correct rationale stopping one field short
is far harder to notice than a missing rationale**, because the paragraph reads
as complete: it states the threat, states a rule, and moves on.

> Where a security rule exists because of a *structural* property — a
> delimiter, an encoding, an ambiguity — check the rule covers every field that
> has that property, not the one that prompted it.

Three more gaps the same implementation exposed, each of which would have
produced conformant, non-interoperable code whose only symptom is
`auth_failed`: no server-facing path was named (now `/register`), no encoding
was stated for `pubkey`/`sig` (base64url unpadded), and "single-use" permitted
re-arming a spent nonce by re-issuing it.

**This is the second time implementing against `docs/srp-v1.md` has corrected
it** — decision 58 was the first. Both corrections came from writing code, not
from reading, which is R7's argument holding at the level of the document as
well as the protocol.

*Revisit if:* nothing here. The remaining relay work is listed in the vault; the
one deliberate gap carried forward is that **TOFU bindings are in memory**, so
a relay restart re-opens the first-use window for any server that has not yet
reconnected. An allowlist is unaffected.


**61. A request now travels client → relay → server and back, and making three
copies of one wire format prove they agree found a fourth spelling of a key.**
*(2026-08-31)*

Phase 3 complete on `staging`. `storm-relay` grew its client half — `HELLO`/
`READY`, `OPEN_STREAM`/`STREAM_READY`, `STREAM_OPEN`/`STREAM_ACK`, stream
routing and multiplexing, `relay_peer_ip` from the accepted socket — and
`apps/server` grew `src/relay/`, the tunnel client that registers over WSS and
dispatches tunnelled requests **in process, through the same `Router`**. No
loopback hop, no second logic path (R13). `relay_peer_ip` reaches the login
limiter as `CallerKey::Ip`, which is the seam decision 56 left open.

**`docs/srp-vectors.json`.** `relay_auth_message`, `validate_nonce`,
`validate_server_id` and the base64url rules exist in three places that cannot
depend on one another — `apps/server` is a binary crate with no library to link
against, and it must stay that way, because the relay must never become
mandatory (R6). Nothing in any build made them agree, and drift does not fail a
compile: it surfaces at runtime as `auth_failed`, which is also what a genuine
attack, an expired nonce and a refused binding all look like. That is the worst
failure signature a mismatch could have. All three test suites now read one
vector file, and the signatures in it come from an independent RFC 8032
implementation, so the positive cases cross-check rather than having one library
agree with itself.

**The finding.** `dart:convert`'s `base64Url` decoder also accepts the standard
`+/` alphabet, and accepts padding. `data_encoding`'s `BASE64URL_NOPAD`, which
both Rust crates use, refuses both. The same 32-byte key therefore had **three
valid spellings on the client and one everywhere else** — and §4.1 binds a
`server_id` to a pubkey permanently, with no way back from a refused binding.
Two implementations disagreeing about whether a spelling is valid disagree about
whether a key has *changed*. The spec now states the rule, `decodeBase64UrlNoPad`
enforces it, and reverting the check fails two vector tests by name.

*This is the third finding of its shape*: decisions 57, 60 and now 61 are all
one side of a wire being lenient, or strict, in a way the other side was not.
The first two were caught by writing a second implementation. This one was
caught by making the implementations testify against a common artifact, which is
cheaper and repeatable.

> A wire format re-derived in N places has N−1 opportunities to disagree, and a
> library default is as likely a cause as a misreading. Shared vectors turn that
> from a runtime symptom into a build failure.

**Two defects were fixed while integrating**, both in code that had never
compiled and so had never run:

- **A clean shutdown could not send `DEREGISTER`.** The supervisor raced its own
  reconnect loop against `serve_trunk`, so shutdown cancelled the future that
  owed the relay a goodbye — leaving the relay holding the `server_id` until a
  heartbeat timeout, which is the outage `DEREGISTER` exists to prevent. The
  rule is now explicit: **shutdown may cancel the phases before registration,
  never the serving phase.** The former have announced nothing; the latter owes
  a `DEREGISTER`.
- **`connect_async` had no deadline.** A relay that completes the TCP handshake
  and then says nothing left the supervisor pending for ever — never retrying,
  never backing off, never returning. Unreachable and silent must cost the same.

And a test-harness race: the fake relay emits its "registered" event as it
*sends* `REGISTERED`, necessarily before the client has received it, so three
tests read the advertised set at a moment it could not yet be populated. They
wait for the state now, with a deadline, so a registration that never lands
still fails — on the timeout rather than on a misleading empty set.

*Revisit if:* nothing here. Carried forward: **TOFU bindings are still in
memory** (decision 60), `trunk_superseded`'s 30 s drain and the 45 s heartbeat
deadline are unimplemented on the relay, and `attachmentUrl` still puts a
credential in a query string. Next is phase 4 — the same relay on a VPS.

**62. `staging` is the development trunk; `main` is the release branch.**
*(2026-08-31)*

Every PR targets `staging`. `main` moves only when a release is being cut, and
**the merge to `main` is the release decision** — the operator's to make, never
an agent's.

This was already how the work was flowing and was written down nowhere. That
gap had a cost: `staging` accumulated **25 commits and decisions 56–61** while
every tool default, and the harness banner in an agent's own session, still said
*"main — you will usually use this for PRs."* An agent following the default
would have opened a release PR for a mid-slice relay.

The rule also fixes what "done" means when reporting. `staging` is **not** a
staging *server*; nothing on it is deployed. Prod serves what came off `main`,
so a green `staging` says nothing about what users are running, and the gap can
be many commits wide. **State the branch when reporting status** — "the relay
works" and "the relay is released" have been many days apart.

Two consequences worth knowing before starting a change:

- **Branch from `staging`, not `main`.** Branching from `main` makes a PR carry
  every unreleased commit, and its diff unreviewable.
- **Read `PLAN.md` on `staging` before numbering a decision.** `staging` may
  already carry entries `main` has never seen — this branch runs 56–62 while
  `main` ends at 54, and decision 55 lives on a third branch entirely. A
  numbering gap is harmless; a duplicate is not.

*Revisit if:* releases become frequent enough that the two branches stay within
a commit or two of each other, at which point the split is bookkeeping without
a benefit. The opposite is the live risk: `staging` running so far ahead that
merging it is a release nobody can review in one sitting. **25 commits is
already at the edge of that.**

**63. The Dart SRP client's architecture, and the discovery that
`Image.network` can never work over a tunnel.** *(2026-08-31)*

Phase 3 left a gap the phase numbering hides: the relay works, the server's
tunnel client works, and **no user can benefit from either**, because there is
no Dart SRP implementation at all. `grep` for `HELLO`, `OPEN_STREAM` or
`HTTP_REQUEST_HEAD` across `apps/client/` returns nothing. Four decisions, with
the full design in the vault's *Storm Relay Dart Client*.

**D1 — the tunnel is an `http.Client`, not a new API layer.**
`SrpHttpClient extends http.BaseClient`. `StormApi` already takes an injectable
client, so wiring is one argument and no call site changes. This is R13 restated
in Dart: the relay introduces no application protocol, so the client must not
grow one either — a parallel `RelayApi` would be decision 37's `ops.rs` mistake
one layer up. `StreamedResponse` also carries SSE for free.

**D2 — one trunk, multiplexed.** REST and the change feed share it. One failure
domain, one `HELLO` against the relay's per-IP limit, one reconnect state
machine. A second trunk for the feed buys isolation that the per-stream cap
already provides more cheaply.

**D3 — the relay set is persisted and refreshed on any success.** In `Settings`
beside `serverId` and `serverPublicKey`, seeded from the pairing payload. §4.3
requires both carriers. **No TTL**: a stale entry costs part of one connection
race, while having none costs reachability entirely — and staleness
self-corrects on the first success where unreachability does not.

**D4 — attachments go through a custom `ImageProvider`.**

**The finding.** This began as a narrower question — `attachmentUrl` puts the
credential in a query string, and through a relay that URL is read in plaintext
by the relay operator, which is worse than the LAN case that put it on the
backlog. Checking how the four call sites actually fetch turned up something
larger:

> **`Image.network` fetches through the platform HTTP stack, bypassing the
> injected `http.Client` entirely.**

So a relayed attachment URL would be a plain HTTPS GET against the relay's
`/connect/<server_id>` — which is a **WebSocket upgrade endpoint expecting
`HELLO`**. There is no value of `attachmentUrl` that makes it work. The problem
was never only the credential; it was that the widget is not on the transport
at all.

A `StormImageProvider` fetching bytes through the injected client fixes three
things at once: attachments route over the tunnel, the credential leaves the URL
on *both* paths, and **the web client is fixed too** — the header change I first
proposed would have worked on mobile and desktop and silently not on web, where
`Image.network`'s headers are ignored.

*The lesson is about where a seam stops.* The `StormConnection` seam was
described as complete, with "wiring real relays later is configuration". That
was true of everything routed *through* `StormApi` and false of four widgets
that quietly route around it. **A transport seam only covers the callers that
actually pass through it**, and the ones that don't are invisible precisely
because they work fine today.

Two more of that shape were already in the same file and are recorded in the
vault note: `_candidates()` string-munges `wss://…/connect/<id>` into
`https://…` and hands it to an `http.Client` as a base URL, and
`connectionProvider` passes no relays at all — which is why neither has ever
failed. **A path that is never exercised cannot report that it is wrong.**

*Revisit if:* the tunnel's framing turns out to need its own isolate, which is
deliberately unresolved and should be settled by a measurement rather than in
advance; or if MCP over the tunnel needs more than a client swap (R10 says it
should not, and the checklist's §5 says verify rather than assume).

**Build state (2026-08-31):** decision 63 steps 1–7 DONE. `SrpTrunk`/`SrpStream`/`SrpHttpClient` in `apps/client/lib/api/relay/` + unit tests (`test/srp_tunnel_test.dart`, 9 passing); `StormConnection` relay wiring (relayed candidates dial a trunk, one per relay, losers released on connect); D3 persistence (`Settings.relays` ↔ `storm.relays` pref, passed to `connectionProvider` and mirrored back via `onRelaysChanged`); SSE feed branch (`SyncEngine._openSocket` uses WebSocket on LAN, SSE when relayed via `StormConnection.sseUri`/`transportClient`/`authHeaders`); `StormImageProvider` (replaces 4 `Image.network` call sites in `attachment_strip.dart` ×2 and `storm_markdown_view.dart` ×2, fetching bytes through the injected transport client so attachments route over the tunnel and credentials leave the URL); offline-vs-relay distinction + direct-vs-relayed rendering (`SyncEngine.connectionTier`, `SyncEngine.isRelayed`, `DotStatus.relayed`, `dotStatusFor` tier param). `flutter analyze` clean, `dart format` applied. All Dart client work for §4 complete. ~~Track B not started~~: Track B landed in the same commit (686a2d1), which this line contradicted. **Its real scope (review of 2026-09-02):** the HELLO per-IP rate limit, the per-trunk stream cap, the `trunk_superseded` 30 s drain and the 45 s heartbeat deadline are all wired. `max_client_buffer_bytes`, however, is declared and never read, `RateLimiter::prune_stale` has no production caller, and the drain and the deadline have no test. In the v0.2.8 tag's source in that state, though never in a binary: `release.yml` does not build `apps/relay` (see 69). Closed by decision 67, which also found the heartbeat deadline closing every healthy trunk.

**64. The `kit` vault is seeded by the server on first run**
Storm is meant to be driven by coding agents, and an agent needs to be told how
a project is laid out and what each role may read and write. That guidance is
itself notes, so it belongs in a vault — `kit`, created and filled on a server's
first boot from templates embedded out of `kit/vault/` at compile time. Making
it core rather than an optional setup step is the whole point: every Storm
server has one, so an adapter can assume it exists.
Seeding keys off **the registry file being absent**, not off `kit` being
missing. Someone who deletes the vault has said something, and restoring it on
every start would override that silently. It is also non-fatal — a server that
cannot write one vault still serves the others, loudly.
*Revisit if:* the role set grows enough that shipping it in the binary is the
wrong distribution (a fetch-on-demand catalogue would be the alternative), or if
users want the templates without the vault.

---

**65. The marketing site's claims are checked, not just its build.**
The A10 cutover deleted `STORM_TOKEN` on 2026-08-20 and swept the READMEs.
`apps/www` was not swept, so for the next thirteen days `/install` told every
visitor to point their client at "the token from `/etc/storm/storm.env`" — a
file that now says, in bold, that there is no token in it — and `/how-it-works`
advertised "no accounts product" for a release whose headline feature is
accounts. **CI was green the entire time**, because the `www` job builds the
site and a wrong sentence compiles.
So the site gets a check that can fail on content: `scripts/check-claims.sh`
(`make www-check`, and a step in the `www` job) refuses the phrasings that
describe the retired credential, and refuses a `release.ts` `tag` older than the
newest `v*` tag — the version drift being the same defect in a second form, four
releases wide, because nothing bumps that file automatically.
**The general shape is worth naming: a documentation surface outside the code
does not fail with the code.** Every other consumer of `STORM_TOKEN` broke
loudly at the cutover. The site kept working and kept being wrong, which is the
worse failure, and the only defence is a check that reads prose.
*Revisit if:* the banned-phrase list starts costing more in false positives than
it catches — the honest alternative is generating the install copy from
`deploy/` rather than mirroring it by hand.

---

**66. `result_large_err` is allowed crate-wide, and CI floats on `stable` for
Rust while it pins Flutter.**
`cargo clippy --all-targets -- -D warnings` went red on 2026-08-31 over
`result_large_err`: axum handlers return `Response` as their error type, which
is how a handler answers with a status and a body rather than a 500, and clippy
counts those bytes. **PR #34 was merged past it**, so `staging` carried a
failing gate and every PR after it inherited one.
Allowed rather than boxed. `Box<Response>` would put an already-heap-backed
body behind a second allocation in every handler, to shrink a value that never
outlives one request.
**The part worth keeping is where it came from.** It fires on x86_64-linux and
not on aarch64-darwin, *on the same 1.98.0* — a clean
`cargo clean -p storm-server && cargo clippy --all-targets` with CI's exact
toolchain passes on the dev machine. So it arrived through
`dtolnay/rust-toolchain@stable` moving under the job, and it could be neither
reproduced nor fixed-and-verified locally. **A gate that only one machine in
the world can run is a gate that gets merged past**, which is exactly what
happened. Flutter is already pinned at 3.44.8 for this reason; Rust is not.
*Revisit if:* pinning the Rust toolchain is taken up — that is the other real
answer here and it stays open. A pin needs an owner for the bump, or it becomes
an ageing compiler nobody dares move.

---

**67. The relay's liveness was untested, and the untested half was broken.**
*(2026-09-25, `fix/relay-debt`)*

The 2026-09-02 review found the heartbeat deadline and the supersession drain
"implemented but untested", `max_client_buffer_bytes` declared and never read,
and `RateLimiter::prune_stale` with no caller. All four were in the v0.2.8
tag's source. *(Corrected in 69: `release.yml` builds no relay binary, so none
of this ever ran outside a developer's machine.)* Writing the tests found that "untested" was hiding "broken":

- **The heartbeat deadline closed every healthy trunk.** It measured time since
  the last `PONG`. But `apps/server` heartbeats by *sending* `PING` every 15 s,
  which the relay answers, and the relay never pings. So a server never has a
  reason to send `PONG`: `last_pong` stayed at registration time, and the first
  frame after 45 s closed the trunk. **Every relayed session would have dropped
  about once a minute**, from the first minute of phase 4. It was also inverted
  for the one case it exists for: checked *between* reads, it could never fire
  for a silent server, because the read never returns. Now it is a timeout on
  the read itself, and **any frame is proof of life**.
- **The drain never closed the superseded socket.** Clients were told
  `trunk_superseded`, but the trunk itself lived on until the server or the
  (broken) heartbeat ended it.
- **`max_client_buffer_bytes` is enforced**, as a queued-and-unwritten byte
  budget on each client trunk's queue: charged on queue, refunded after the
  socket write. A frame that would exceed it drops its stream, the same
  response a full frame queue already gave. **An empty queue always admits one
  frame**, because an attachment reaches the relay as one frame the size of the
  file; otherwise a budget smaller than the largest attachment could never
  deliver it. **The default is 32 MiB, not the 1 MiB declared.** 1 MiB would
  drop the second of two images opened together on a slow link. 32 MiB replaces
  an unbounded worst case (256 frames of up to 16 MiB each) with a ceiling.
  Decided, not measured, so it belongs to Q14.
- **`prune_stale` runs from `try_take`, once per window.** Pruning is lossless:
  a bucket idle for a full window has refilled to `limit`, which is exactly the
  fresh bucket that would replace it.

Relay tests 84 → 99. The four behavioural tests each fail against the v0.2.8
`src/` and pass against the fix. The fifth (a reading client receives 25× the
budget) guards the refund.

**The lesson is the same one `docs/srp-vectors.json` taught about the wire
format, one layer up.** The server's heartbeat and the relay's deadline were
each written against the spec's "`PING`/`PONG` every 15 s", which does not say
who pings. Each side's reading was self-consistent, and nothing made the two
meet. **A protocol behaviour implemented on two sides needs a test that puts
both directions on one wire.** For the heartbeat, that test is
`a_server_that_heartbeats_outlives_the_deadline`, written the way `apps/server`
actually heartbeats.

Found and deliberately **not** fixed here:

- **SRP has no per-stream flow control.** The relay cannot slow an origin down
  for one client without stalling every client sharing that trunk. So a
  response larger than the backlog budget, going to a client slower than the
  origin, is dropped. The budget decides where that happens; only a protocol
  change (credit-based windows) removes it. Decide this before a hosted relay.
- **An attachment over 16 MiB cannot cross the relay.** The origin sends it as
  one frame, and the relay's WebSocket reader caps a frame at 16 MiB.
- **The Dart client treats every trunk-level `ERROR` as fatal**
  (`srp_trunk.dart`). The relay refuses an `OPEN_STREAM` over the in-flight cap
  with a trunk-level `rate_limited`, so a client that hits the cap also loses
  its other in-flight streams. The spec needs to say whether that refusal is
  stream- or trunk-scoped.
- **A draining trunk still accepts `OPEN_STREAM`** from the clients already
  bound to it. Bounded by the 30 s window, and refusing it runs into the point
  above.
- **The `HELLO` limit is per address**, so an IPv6 /64 gets a fresh bucket per
  address. Pruning bounds the memory, not the limit's effectiveness.

*Revisit if:* the budget drops real traffic in phase 4. Measure first, then
choose between a larger number and the flow-control change. Do not remove the
empty-queue admission; without it, large attachments stop working.

---

**68. `apps/relay` gets a CI job; it never had one.** *(2026-09-25)*

`ci.yml` ran fmt, clippy and tests for `apps/server`, `apps/client` and
`apps/www`, and nothing for `apps/relay`. From its first commit (decision 60)
the relay's only gate was a local `make check`. Several of its PRs merged
with "CI green" while no machine outside the author's had compiled the crate.
That includes the v0.2.8 relay work decision 67 found broken, and #39 itself.
"`apps/relay` was in the gate from its first commit" was true of the Makefile
and false of CI, and only CI is a gate.

A separate `relay (rust)` job: fmt, clippy `-D warnings`, test. It is not a
step in `server`, because the two crates are deliberately not a workspace
(R6), and one job building both would be the first step towards treating them
as one thing. `cargo test` includes `tests/vectors.rs`, so the relay's third of
the `docs/srp-vectors.json` agreement is now actually enforced.

**The shape to keep:** a new crate or app needs its CI job in the same change
that adds it. The Makefile is where the local convenience lives; CI is where
the gate is. The two drifted here for 25 days without anyone noticing.

*Revisit if:* never, as long as the crate exists. If it moves into a workspace,
that is an R6 question first.

---

**69. TOFU bindings persist, and a binding is on disk before it is announced.**
*(2026-09-25, `feat/relay-tofu-persist`)*

The gate on phase 4. TOFU bindings lived in memory, so every relay restart
re-opened the first-use window for every server that had not reconnected yet,
and §4.1 makes a squatted `server_id` permanent. `--bindings <path>` closes it.

- **The file uses the allowlist's format, and loading goes through
  `Allowlist::parse`.** One format, one validator, and a bindings file an
  operator has come to trust can be promoted to `--allowlist` unchanged.
- **Durable before `REGISTERED`.** `record` writes a temp file, fsyncs it,
  renames it over the old one, and fsyncs the directory, all while holding the
  bindings lock. A write that fails un-records the pair, and the registration
  is refused with `protocol_error` (the relay's fault, and §6 has no honest
  code for that). **A binding the relay announced but could lose on restart is
  exactly the window this exists to close**, so failing closed is the point.
- **A corrupt file stops the relay; a missing one starts empty.** Starting
  empty on a parse error would open every `server_id` at once.
- **Opt-in, and `--allowlist` conflicts with it** (an allowlist switches TOFU
  off). Without either flag the relay starts, and says loudly that a public
  relay must not run that way. Not the default, because the relay has no state
  directory to put a default path in, and inventing one would be a second
  thing to back up that nobody chose.

Relay tests 99 → 104. Each of the four new tests fails when `record` skips
the write, checked by mutation, since the tests cannot compile against the old
API.

**Found while writing this up: the relay has never been released.**
`release.yml` builds `storm-server`, the clients and the web bundle, and
nothing from `apps/relay`, and `deploy/` has no unit for it. So "shipped in
v0.2.8" (said of the relay in 67, and in the vault notes) meant *in the tag's
source*, not *in a binary anyone ran*. The heartbeat bug 67 fixed never ran
anywhere. **Phase 4 therefore starts with packaging**: a `storm-relay` release
artifact, a systemd unit and a `deploy/` section, with `--bindings` pointing
into a state directory that unit owns. That is also where a default bindings
path can finally live.

*Revisit if:* a hosted relay needs account-owned claims (§4.1's third row),
which is a different store, not this file grown; or if a relay ever holds
bindings for so many servers that rewriting the whole file per first
registration costs something. Each server registers for the first time once,
so that is not close.

---

**70. The relay is packaged, and a public relay is blocked on how it learns a
client's address.** *(2026-09-25, `feat/relay-packaging`)*

Phase 4, step 1. 69 found that no release had ever built `apps/relay`.

- **Its own package, `storm-relay`, in the same apt repository.** `apt-repo.yml`
  already takes every `.deb` in a release, so a `relay` job in `release.yml`
  (musl binary plus `.deb`, version stamped from the tag, like the server's) is
  the whole distribution change. Neither package depends on the other (R6).
- **`deploy/storm-relay.service`**: its own `storm-relay` user, never `storm`,
  so a relay sharing a box cannot read a vault. `StateDirectory=storm-relay`
  owns `/var/lib/storm-relay`, and the env file defaults `--bindings` into it,
  so **a packaged relay is never TOFU-in-memory by accident**. The unit carries
  no flags, since the relay reads `STORM_RELAY_*` itself. The same hardening as
  storm-server.
- **Installed, not started.** A relay with no `STORM_RELAY_PUBLIC_BASE` would
  hand every server a `public_address` that goes nowhere. An upgrade restarts a
  relay that was running.
- **Purge keeps `/var/lib/storm-relay`.** The bindings file cannot be rebuilt,
  and deleting it opens every `server_id` to the next registration.
- **Default port 8486, not 8484.** 8484 is storm-server's, and phase 4 is built
  with both on one machine first. The old default collided with it.

Verified: `cargo deb` builds a package with the binary, the unit, the env file
as a conffile and all three maintainer scripts; `systemd-analyze verify` is
clean; the env file drives the binary exactly as the unit loads it, and both
binding modes set through the environment are refused.

**The open problem: a public relay cannot see its clients.** The relay speaks
plain `ws://`, and takes a client's address from the accepted socket because a
header is forgeable (§5.2). Behind a TLS reverse proxy every client arrives
from the proxy, so the per-IP `HELLO` limit becomes one global bucket, and the
origin's login limiter gets a single `relay_peer_ip` for every relayed client:
one noisy client locks out everyone. Plain `ws://` on a public port avoids that
and puts session tokens on the wire in cleartext. **Phase 4 step 2 needs one of
these:** TLS in the relay itself (rustls, with the certificate as files), or
PROXY-protocol support from a trusted local proxy. Either keeps the address a
fact about a connection rather than a claim in a header. Until then the relay
is for a LAN or a VPN, and `deploy/README.md` says so. *(Settled by decision 71: TLS in the relay.)*

Found on the way, recorded rather than fixed here:

- **storm-server's `prerm` disables the unit on every upgrade.** It ignores
  `$1`, and dpkg runs `prerm upgrade`, so after `apt upgrade` storm-server is
  no longer enabled at boot. The documented `systemctl restart` hides that
  until the next reboot. **It disables `storm-backup.timer` the same way, so
  nightly backups stop after any upgrade.** The relay's `prerm` acts only on
  `remove`. *Fixed in `fix/server-prerm-upgrade`: `prerm` acts only on
  `remove`/`deconfigure` (with a test that runs the real script against a
  recording `systemctl` and fails on the old one). Because dpkg runs the
  **old** package's `prerm`, the upgrade to the fix still disables both once,
  so `postinst` re-enables and starts them, one time, on an upgrade from
  ≤ 0.2.8 where `up`'s drop-in exists. That re-enables a box someone had run
  `storm-server down` on; it says so. `postinst` itself has no test, since it
  creates users and writes under `/srv` and `/etc`.*
- **storm-server's `StartLimitIntervalSec` has never applied.** It sits under
  `[Service]`, where systemd ignores it (found by `systemd-analyze verify`), so
  the "don't spin" comment above it has not been true since M6. The relay's
  unit puts it under `[Unit]`. *Moved to `[Unit]` in the same fix.*
- **A server connects to relays only at boot.** `PUT /v1/config/relays` stores
  the list, and nothing reconnects until the next start, and no client screen
  sets it. The doc comment on `put_relays` still says the server has no tunnel
  client "yet". *(The restart half is closed by decision 74; there is still
  no client screen.)*

*Revisit if:* the relay grows state beyond the bindings file, which would
change what "purge keeps it" protects.

---

**71. A public relay terminates TLS itself, and storm-server can finally dial
`wss://`.** *(2026-09-25, `feat/relay-tls`; operator's choice between the two
options 70 named)*

**The choice.** Relay-side TLS over PROXY protocol, because the relay stays one
binary with one trust boundary: the address is a fact about the socket it
accepted, as §5.2 needs, with no second process to configure and no header to
trust. The cost is certificate handling, which is two files and a reload.

- **`--tls-cert` / `--tls-key`** (`STORM_RELAY_TLS_*`), each requiring the
  other. rustls with **`ring`**, not the default `aws-lc-rs`, because the
  release builds static musl with zig and aws-lc needs cmake and a C toolchain
  it does not have. ALPN offers `http/1.1` only, since a WebSocket upgrade
  cannot happen over `h2`.
- **A handshake never blocks an accept.** axum calls `Listener::accept` in one
  loop, so a handshake inside it would let a client that connects and says
  nothing stall every connection behind it. Each handshake gets its own task,
  a 10 s deadline and a slot out of 1024; past that, new sockets are dropped
  unhandshaken.
- **`SIGHUP` reloads the certificate in place** (`ExecReload`), so a renewal
  drops no trunk and makes no server re-register. A reload that fails keeps
  the old pair. Loading refuses a key that does not match its certificate,
  which would otherwise start a relay that fails every handshake.
- **`PeerAddr`**, a newtype, replaces `ConnectInfo<SocketAddr>`: axum only
  provides the latter for its own `TcpListener`, and the orphan rule forbids
  providing it for ours. Both listeners supply it from the TCP peer.
- **Scheme sanity at start:** `--tls-cert` with a `ws://` public base refuses
  to start; a `wss://` base without `--tls-cert` warns. The default base now
  follows TLS (`wss://` with it, `ws://` without) instead of always `wss://`.
- **The unit** gains `ExecReload` and `CAP_NET_BIND_SERVICE` as both the
  ambient and the bounding set, which allows port 443 and drops every other
  capability.

**The server half.** `storm-server`'s `tokio-tungstenite` had **no TLS
feature**, so every `wss://` relay was refused before a byte was sent.
`client.rs` even logged it. So no server could ever have used a public relay.
It now has `rustls-tls-webpki-roots` (bundled Mozilla roots, since the musl
binary must not depend on the host's store), and `main` installs `ring` as the
process-default provider **before anything can dial**, because tungstenite
builds its client config from that default, and rustls panics on the first
handshake if a second provider ever makes the default ambiguous.

Evidence: relay tests 104 → 112. `tests/tls.rs` covers registering over
`wss://`; `relay_peer_ip` over TLS being the client's socket; a stalled
handshake not blocking the next connection; a stalled handshake cut off at its
deadline; plaintext refused; a reload swapping the certificate; a failed reload
keeping the old one; and a mismatched key refused at load. Two mutations were
caught by exactly the test written for each: an inline handshake fails the
stall test, and a no-op reload fails the swap test. Server tests 420 → 421.
**The new server test first passed with the TLS feature removed**, because
`connect_async` opens TCP before it checks for TLS, so a closed port fails
identically either way. It now dials a listener that hangs up, and it was
checked to fail without the feature. A musl `cargo zigbuild` of both crates,
the release toolchain, was run locally.

Not tested end to end: a storm-server registering with a TLS relay. The server
trusts only the public web PKI, so that needs a real certificate on a real
host, which is phase 4 step 3.

*Revisit if:* certificate automation should live in the relay (ACME built in),
or a hosted relay needs SNI across many names, which `CertStore` does not
attempt.

---

**72. How a release is cut, now that `staging` is the trunk.** *(2026-09-26,
v0.2.9)*

Written down because the vault's *Storm Releases* note predated decision 62 and
described what a release builds, not how one is cut, and because v0.2.9 hit a
check that made every release PR red.

1. Everything for the release is merged into `staging`, green.
2. A prep PR into `staging` bumps `apps/www/src/data/release.ts` to the new
   tag and updates **Release state** above.
3. A `release/vX.Y.Z` branch **at `staging`'s head**, PR into `main`. It adds
   nothing of its own, so after the merge `main` is exactly `staging` plus one
   merge commit.
4. Merge it, then tag **the merge commit** `vX.Y.Z` and push the tag.
   `release.yml` runs `make check` and `make test-live`, builds every artifact
   (45) and publishes the GitHub Release, then `apt-repo.yml` publishes every
   `.deb`. The version comes from the tag (46).
5. Check that the release has every artifact and that the apt index lists each
   package at the new version. Then the operator upgrades the VM, and **the
   deploy is recorded here that day**.

`www-check` compared `release.ts` to the newest tag **for equality**, so a
release PR that bumped it failed (the tag did not exist yet), and one that
did not bump it shipped a tagged commit whose site linked the previous
release. Now it fails only when `release.ts` is **older** than the newest
tag. A newer, not-yet-tagged value passes with a note.

*Revisit if:* a release ever needs its own commits (version files, a
changelog). Put them in the prep PR on `staging`, never on the release branch,
or `main` and `staging` diverge.

---

**73. Prod did not survive a reboot, and the server now waits for its
storage.** *(2026-09-26, found on the VM's own journal)*

"Whenever the VM restarts Storm breaks until `systemctl restart`" had two
causes, both read off prod (0.2.8, booted 2026-09-21):

- **The unit was disabled** (70's `prerm` bug): `storm-server` and
  `storm-backup.timer` both `disabled`, five upgrades since August, and at the
  2026-09-21 boot nothing started the server — the first start was the
  operator's `systemctl restart`, 20 s after boot. **The backup timer had not
  run since 2026-08-20**: one backup in `/srv/storm/backups`, a minute before
  the 0.2.7 upgrade that disabled it. v0.2.9's `postinst` re-enables both.
- **Nothing waited for the NFS vault root.** The unit had only
  `After=network-online.target`, which does not wait for fstab's network
  mounts. That boot the mount failed outright (`nas.lan` did not resolve; the
  operator switched fstab to the IP at 14:58), and every start failed with
  `226/NAMESPACE`, because systemd refuses to spawn a process whose
  `ReadWritePaths` entry does not exist — 73 failures before the manual fix.

The fix, in two layers:

- The packaged unit gains **`After=remote-fs.target`**, ordering only, so every
  install waits for fstab network mounts on upgrade, without re-running `up`.
- `storm-server up`'s drop-in gains **`[Unit] RequiresMountsFor=`** listing
  exactly its `ReadWritePaths`: systemd pulls in those mounts and waits, and a
  mount that fails fails the start once, as a dependency, instead of a
  `226/NAMESPACE` loop. A test pins the section (under `[Service]`, systemd
  ignores the key with a warning, as it did `StartLimitIntervalSec`) and that
  the two path lists match. Existing installs get it by re-running `up`, or
  with a one-line drop-in.

**Why this matters more from 0.2.9 on:** 70 moved `StartLimitIntervalSec` into
`[Unit]`, so the restart limit (5 starts in 60 s) now applies. On 0.2.8 it was
ignored, and a mount that came up late was eventually caught by the endless
3 s retries; on 0.2.9 without this fix, the server gives up after about 15 s.

*Revisit if:* a layout mounts storage by an automount unit
(`x-systemd.automount`), where the path exists before the filesystem does and
ordering behaves differently.

---

**74. A saved relay list takes effect without a restart.** *(2026-09-26,
`feat/relays-live-reconfigure`)*

`PUT /v1/config/relays` stored the list and nothing acted on it until the next
boot (found in 70). Now:

- **`Tunnels` keeps one supervisor per relay URL, each with its own shutdown
  channel**, and `reconcile(list)` diffs the running set against the list:
  removed relays are stopped together, each sending its `DEREGISTER` first;
  added ones are started; **a relay in both lists is not touched**, so adding
  a second relay or re-saving the same list never drops a working trunk.
- **`put_relays` sends the saved, normalised list on a `watch` channel** in
  `AppState` (after the save, so what the tunnels act on is what a restart
  would read), and `relay::manage` owns the tunnels for the server's life,
  applying each change to completion. `reconcile` runs in the `select!` arm's
  body, never raced by it, so shutdown cannot cancel a `DEREGISTER` in flight,
  which is the supervisor's own rule.
- `PUT` still returns before any connection is attempted. `GET /v1/server`
  remains the only answer to "is it connected".

Evidence: three tests. Two drive real fake relays (add, add a second, remove
the first: the first sees `DEREGISTER`, the second stays up, and the first
was connected exactly once; and the same through `manage` and the channel),
and one is the handler test, that a save reaches the channel normalised.
Mutations: dropping the handler's `send` fails the handler test; making
`reconcile` restart every relay fails the "an unchanged relay was
reconnected" assertion. Server suite 424.

*Revisit if:* relays gain per-relay settings (a pinned key, a priority), at
which point "same URL" stops meaning "same relay" and the diff must compare
more than the URL.

---

**75. The packaged nightly backup had never run.** *(2026-09-27, prod's
journal, after the 0.2.9 upgrade re-enabled the timer)*

Once 0.2.9 re-enabled `storm-backup.timer` (73), it fired, and failed:
`203/EXEC`, `/usr/local/bin/storm-backup.sh: No such file or directory`. The
unit still named M6's hand-install path; the `.deb` has installed the script to
`/usr/bin` since M15. So **no packaged install has ever taken a backup**, and
prod's last one, 2026-08-20, was a manual run. Fixing the path alone would not
have been enough:

- **The script sourced `/etc/storm/storm.env` itself**, and that file is
  `0600 root`, so as the service user it stopped at "cannot read". The unit
  now has `EnvironmentFile=` (systemd reads it as root), and the script sources
  the file only when run by hand with nothing already set.
- **It ran as `storm`**, while `up` had given the layout to the state owner
  (`dewansh` on prod) through a drop-in for the server unit only. `up` now
  writes the same drop-in (user, `ReadWritePaths`, `RequiresMountsFor`) for
  `storm-backup.service` too, and the unit waits for `remote-fs.target`.

Evidence: `tests/packaging.rs` reads the real unit and `Cargo.toml` and
asserts the unit runs the path the `.deb` installs to, and that it takes its
environment from systemd; both fail on the old unit. The script was run as
systemd would run it (values in the environment, the env file unreadable) and
completed a verified backup, and by hand with nothing set it still refuses.

**The lesson**: a timer that is disabled cannot fail, so the packaging bug in
70 hid this one for a month. Re-enabling a unit is the moment to watch it
run once.

*Revisit if:* the env file ever holds a secret the backup must not see; then
the backup gets its own env file.

**76. Storm's data was readable by every local account, and is now its
user's and group's.** *(2026-10-03, found while designing Agent Runtime V1;
prod confirmed by the operator)*

**The defect.** `postinst` creates `/srv/storm` with `mkdir -p` as root, so the
tree is 0755. `storm-server.service` set no `UMask`, so systemd's default 0022
applied to everything the server wrote. The result: **any local account could
read every note, every index, `vaults.json`, and `state/auth.db`**, which holds
the password hashes. Only `state/identity/` was protected, because A2 chmods
it explicitly.

**Evidence.**
- Reproduced with the real server under systemd: as `nobody`, a note,
  `auth.db` and `vaults.json` were all readable.
- Prod, read by the operator:

  | Path | Mode | Owner |
  |---|---|---|
  | `/srv/storm` | 755 | `dewansh:dewansh` |
  | `/srv/storm/vaults` | 755 | `dewansh:dewansh` |
  | `/srv/storm/state` | 775 | `dewansh:dewansh` |
  | `/srv/storm/state/auth.db` | 644 | `dewansh:dewansh` |

It became blocking because Agent Runtime V1 puts a second service account,
`storm-runtime`, on the same machine (vault: *Agent Runtime/V1 Specification
Freeze*, prerequisite P3).

**The fix, in four layers:**

1. **Both units set `UMask=0027`.** The backup unit too: its archives carry
   `auth.db` and the identity keys (A4).
2. **`postinst`** creates a fresh tree 0750. On every install, upgrades
   included, it runs `chmod o-rwx /srv/storm`. That is owner-agnostic and
   non-recursive, so the service user keeps every bit it has, and it never runs
   through a symlink, which could point at a share other machines use.
3. **`storm-server up`** restricts the data root, state and backups, and the
   vault root only when it sits under the data root.
4. **`serve`** tightens its own state directory at boot. An install made any
   other way, or a tree loosened by hand, still keeps `auth.db` from other
   accounts.

Every layer only ever *removes* group-write and other bits, so a directory an
operator made stricter keeps its mode.

**What it deliberately does not touch: a vault root on a share.** Prod's vault
root is NFS from the NAS, and other machines write it. A probe showed **the NAS
assigns 664 whatever the client's umask**, so `UMask` neither harms the share
nor protects it. Tightening it from the VM would lock those machines out. Its
exposure to local accounts on the VM is a NAS-side decision, recorded in the
vault's Global Todo.

**Prod needs a one-time remediation**, because this release cannot reach an
already-installed tree beyond the root's other bits. The commands are in
`deploy/README.md`: chmod the root and `state/` 0750, a `find -xdev` that
removes other-access from the local tree only, and `UMask=0027` drop-ins for
both units until the fixed package is installed.

**Verified (2026-10-03, codebox):**
- **The checks.** `make check` under CI's Flutter 3.44.8 is clean and
  reformats nothing, with one exception: `editor_save_loop_test`'s huge-note
  case never completes on codebox. It is out of memory there (3.2 GB RSS on a
  4 GB VM, CPU-bound), passes in 0.3 s on CI, and P3 changes no Dart.
- **The live suites.** `make test-live`: `e2e.py` 81/81 unmodified, MCP 80/80,
  auth 74/74, client integration 20/20.
- **Prod-shaped, under systemd.** The installed binary ran the 0.2.9 unit and
  a `data-root.conf` like prod's, as a non-`storm` owner, with the vault root
  outside `/srv/storm` on a share that ignores the umask. The data was first
  loosened to prod's exact modes, and `nobody` could read `auth.db` and
  `vaults.json`. The remediation commands above were then applied verbatim:
  - `e2e.py` 81/81 (unmodified), MCP 80/80, auth 74/74, and a backup run all
    passed.
  - Everything created was 640/750, identity 700/600, and nothing in the tree
    was other-accessible. `nobody` was refused `auth.db`, `vaults.json` and
    `ls /srv/storm`, the journal was clean, and the share's modes were
    untouched.
  - The two `e2e.py` storage-root checks first failed on the harness, not on
    P3: the suite makes a sibling of `VAULT_ROOT`, which must be inside
    `ReadWritePaths` and writable by the service user, as the unit already
    documents.
- **A fresh install** (`postinst` → `up --vault-root <share>`) gave a 0750
  tree, the right drop-in, `UMask=0027`, `auth.db` 640, and `nobody` refused.

**Prod (2026-10-03, verified over SSH after the operator ran the remediation):**
- **The server:** running as `dewansh` with `UMask=0027` on both units,
  health OK, all four vaults reconciled, MCP enabled.
- **Modes:** `/srv/storm`, `state`, `vaults` and `backups` are 750;
  `auth.db`, its WAL/SHM and `vaults.json` are 640; `identity` is 700. No
  path in the local tree is other-accessible, and `nobody` is in no group
  that could reach it.
- **MCP:** a write and a read-back worked, and the index files it wrote came
  out 640. The probe note was then deleted.
- **The NFS vault root:** still 775, and the probe note on it came out 664.
  That is the NAS's to fix, as recorded above.
- **Logs:** no errors since the restart. The one WARN, stored root versus
  the `--vault-root` flag, predates this change (585 earlier occurrences) and
  is decision-shaped behaviour: the stored root wins.

**A lesson from the run.** The first attempt was a pasted block of separate
`sudo` lines. The first one stopped the server, then the password prompt
swallowed the rest, including the `start`, and prod was down for 77 s. The
README now gives one `sudo sh -c` that always ends by starting the server.
A multi-step prod procedure that stops a service is given that way from now on.

**Status:** verified on prod. Agent Runtime implementation waits only on the
operator's confirmation.

*Revisit if:* a deployment needs another local account to read the data tree.
Grant that account group membership; never restore other-access.

**77. Agent Runtime V1 is frozen, and it is built in seven slices, the
provider contract first.** *(2026-10-03; operator's go after P3 was verified
on prod)*

**The spec.** *Agent Runtime/V1 Specification Freeze* in the personal vault,
revision 4. It was approved 2026-10-03 and is frozen as of this entry. Its
amendments AM1–AM21 are D12 in *Agent Runtime/Decisions*. The vault holds the
spec; this entry records that it is frozen, how it is sliced and the decisions
the code makes that the spec left open. Where they disagree, the freeze wins
and this entry gets fixed.

**In one paragraph.**
- The Storm Server is the control plane: the Agent Manager, session records
  in `state/agent/agent.db`, and the host registry in `state/auth.db`.
- A **Runtime Host** (`storm-runtime`, its own crate, `.deb` and OS user) is
  the execution plane. It dials the server, authenticates with an Ed25519 key
  under a third signing domain `storm-host-auth:v1:`, and runs agents in
  workspaces under its configured roots.
- Clients talk only to the server, over REST + SSE, owner-only.
- V1 providers are `claude-code` (default), `opencode` and `shell`, all of
  kind `cli`, offering one `terminal` interaction that the runtime carries over
  a PTY.
- Direct network only. The relay is a design constraint, not V1 acceptance.

**The slices**, in the freeze's §16 order. Each is its own PR to `staging`:
1. **`apps/runtime` and the provider contract.** `Provider`,
   `ProviderSession`, `TerminalChannel`, the §7.2 status vocabulary, the
   offset-addressed scrollback, and the fake provider with an in-memory
   terminal and no PTY. Plus the Makefile targets and a CI job.
2. **The PTY carrier and the `cli` providers.** Availability resolution, and
   ending a session as SIGHUP to the process group, then SIGKILL after 5 s.
3. **Host identity and enrollment.** Server: `runtime_hosts`, enrollment and
   host tokens, the `Host` tier. Runtime: `enroll`, the identity key and
   `host.json`. Both: `docs/runtime-vectors.json`.
4. **The host link and the Agent Manager.** `/v1/runtime/*`,
   `state/agent/agent.db`, and reconciliation from `hello`. It ends with
   `tests/agent_e2e.py` driving the fake provider end to end, which is AC-A1.
5. **The client routes.** `/v1/agent/*`, owner-only on every route, the
   terminal SSE stream and its `?offset=` resume. *(Merged into slice 4 by
   77c.)*
6. **The client UI.** The `xterm2` surface, hosts, launcher, terminal view,
   tabs and the extra-keys row.
7. **Packaging.** The `storm-runtime` `.deb` and its systemd unit (§5.8:
   no `MemoryDenyWriteExecute`).

Then the on-device acceptance criteria. Claude Code is verified before OpenCode
is claimed (AC-P1).

**Slice 1's decisions, where the spec was illustrative:**
- **Output and endings go through a runtime-owned sink, `SessionEvents`.** The
  provider never owns the scrollback. The runtime, which already owns the
  carrier, owns where output lands. The same sink serves a PTY reader thread
  and the fake.
- **The contract is synchronous and has no dependencies.** PTY I/O is blocking
  threads anyway. The async host link adapts at its edge, so no async runtime
  leaks into every provider.
- **The fake provider has its own kind, `fake`.** Kind says how a session
  starts, and the fake starts in process. Labelling it `cli` would be a lie the
  host would act on. AC-A1 runs it inside a real `storm-runtime`, so `fake` is
  a wire value. A host offers it only when its config names it.
- **The scrollback is a fixed-capacity ring addressed by absolute byte
  offset.** A read below the floor returns an explicit `Gap`, the §11.2 `gap`
  event, never silently fewer bytes.
- **`apps/runtime` depends on nothing in `apps/server`, and there is still no
  workspace** (the R6 reasoning, applied by the freeze §5.8). The host and the
  server share only the wire format, and `docs/runtime-vectors.json` is what
  keeps the two in agreement.

*Revisit if:* the async host link (slice 4) cannot drive the synchronous
contract without a thread per session, or a second interaction kind needs the
provider rather than the runtime to own its carrier.

**77a. Slice 2: the PTY is ours, on `rustix`, and a session ends only when
its whole process group does.** *(2026-10-03)*

**The PTY carrier.** It is built directly:
- `openpt`, `grantpt`, `unlockpt` and `ptsname` from `rustix`.
- `std::process::Command` with the slave as stdio, plus one `pre_exec` that
  calls `setsid` and takes the slave as the controlling terminal.

We do not use `portable-pty`. Its `ExitStatus` reports a signal as a name,
and V1 needs the number for `failed (signal N)`. We also need exact control of
the process group: `setsid` makes the agent a group leader, so its pid is the
group id and one `kill(-pgid)` reaches everything it spawned. The cost is the
`pre_exec` block, the crate's only `unsafe`. It calls two raw syscalls, both
async-signal-safe.

**Ending a session** (freeze §7.3):
- `stop` sends SIGHUP to the group, then SIGKILL to the group after the
  grace, 5 s by default. It reports `stopped` whatever signal finished the
  job, because the owner asked.
- A session that exits by itself is `completed` with its exit code.
- A signal the owner did not send makes it `failed (signal N)`.
- When the agent exits, its group gets SIGHUP and, after the grace,
  SIGKILL. That covers a background job that keeps the terminal open and
  would otherwise hold the session open forever. Nothing the session spawned
  outlives it.
- `ended` is reported only after the output has drained, so no output
  arrives after a session has ended.

**The environment.** The host's own environment, with `TERM=xterm-256color`
and `COLORTERM=truecolor` on top, then the provider's variables, and the
workspace as the working directory (§9.2). The host needs `HOME` and `PATH`
for the agents to work at all.

**Providers:**
- `claude-code` runs `claude --permission-mode default`. The flag is always
  passed, so host CLI settings cannot start it in another mode (gate Q6).
- `opencode` runs `opencode`.
- `shell` runs `$SHELL -l`, falling back to `/bin/sh`.

Availability means the command resolves to an executable file: on `PATH`, or
as given when it contains a `/`.

Loading provider env files (`0600`, referenced by name) belongs to
`runtime.toml` and arrives with the daemon in slice 4.

**77b. Slice 3: hosts enroll with a two-part token and authenticate with a
key, in a tier of their own.** *(2026-10-03)*

**The wire formats.** `docs/runtime-vectors.json` pins every one of these,
and both crates read it. `tools/runtime-vectors/generate.py` regenerates it,
from a reference implementation independent of either crate.
- **The enrollment token is `sen_<26 Crockford>.<43 base64url>`.** The first
  part is public: it is the enrollment's record id, `hen_<26>`, and it is the
  freeze's `<token_id>`, which the host derives without asking. The second
  part is 256 secret bits. The server stores the blake3 of the whole token and
  looks it up by that hash, like every other token. A public id inside the
  token keeps the signed message free of secrets and of hashing. That matters
  because the generator has no blake3, and the message must be reproducible by
  an unrelated implementation.
- **The enrollment string is
  `storm-enroll:v1:<server_url>:<server_id>:<server_pubkey>:<token>`.** It is
  parsed **from the right**: the URL contains colons, and none of the last
  three fields can. `server_url` is supplied by the client that asks for the
  enrollment. The server does not know how it is reached, and the owner's
  client does.
- **Signed messages** (both under the host's key, never the server's):
  - Enrollment: `storm-host-auth:v1:<server_id>:<token_id>`.
  - Connect: `storm-host-auth:v1:<server_id>:<host_id>:<nonce>`.

  The two differ in field count, and every field is validated against the
  delimiter rules, so neither message can be read as the other. Neither can
  be read as `storm-challenge:v1:` or `storm-relay-auth:v1:`, which are signed
  by a different key.

**Credentials**, in `state/auth.db`, created additively in the
`CREATE TABLE IF NOT EXISTS` batch, so `SCHEMA_VERSION` stays 5:
- `runtime_hosts`: the host's id, name, public key, key id, who enrolled it
  and when, `last_seen` and `revoked`.
- `host_enrollments`: single use. They expire after 10 minutes.
- `host_tokens`: they expire after 24 hours and are minted only by proving the
  key.
- `host_challenges`: single use, server-generated, expiring after 60 s.

The token prefixes are `sen_` (enrollment) and `sht_` (host token). Both are
blake3-hashed, never stored.

**The `Host` tier** is a fifth `RequiredTier`:
- It accepts `Bearer sht_…` on `/v1/runtime/*`, and nothing else does.
- A session token, an `stk_` key or a device credential is refused there with
  `401`. A host token is refused everywhere else.
- `/v1/runtime/enroll`, `/auth/challenge` and `/auth` are none-tier, and are
  rate-limited by a limiter of their own (not the login one, whose budget they
  must not spend).

**The owner check.** Every `/v1/agent/*` route requires the owner role.
Anyone else gets `403`, never an empty list (AM5). A helper in `ops.rs`
enforces it, so a route cannot forget it.

**Revocation.** It sets `revoked` and revokes every live host token. The link
closes when slice 4 exists. A revoked host is refused at authentication.

**Security events.** `host_enrollment_issued`, `host_enrolled`,
`host_renamed`, `host_revoked` and `host_auth_rejected`. Never a token,
never a key.

**The host side** (`storm-runtime enroll`):
1. It reads the enrollment string from stdin, never from an argument.
2. It verifies the server through `POST /v1/server/challenge`, against the
   pinned key, before anything else.
3. It generates an Ed25519 key, written `0600` in a `0700` `identity/`
   directory, both created with those modes.
4. It enrolls, then writes `host.json`.

The HTTP client is `reqwest` with rustls on `ring`. Never `aws-lc-rs`: the
musl/zig release build cannot compile it.

**77c. Slices 4 and 5 merge: the host link, the Agent Manager and the agent
routes land together.** *(2026-10-03)*

AC-A1 drives launch → input → output → end through the Agent Manager *and*
the host link. That cannot be tested without the client routes, so they ship
as one slice. The later slices renumber: **5** is the client UI and **6** is
packaging.

**The server** (`apps/server/src/agent/`):
- **Session records** live in `state/agent/agent.db`, one row per `ags_`
  session with the §7.1 fields. The default provider lives in
  `state/agent/config.json`.
- **At boot**, every session that has not ended becomes `unknown` (§13).
- **Link state is in memory:** which hosts are online, each host's command
  sender, its capabilities, and a per-session output cache. The cache is the
  same offset-addressed ring as the host's. A link drop marks the host
  `offline` and its sessions `unknown`. A link is identified by a generation
  number, so an old stream closing never marks a newer one offline.
- **The output cache only accepts bytes in order.** Overlap is skipped, a
  jump forward starts the ring at the new offset, and an empty cache takes
  whatever offset arrives first. After a server restart the cache is empty,
  so `hello` answers each live session with `terminal.replay {from: 0}`. The
  host re-sends its ring from its floor, in order, before anything new.
- **`hello` reconciles** (§13):
  - Sessions the host reports take its reported state.
  - Its sessions that have not ended and that it does not report become
    `failed (lost)`.
  - Sessions a restarted host reports as dead become
    `failed (host_restart)`.
- **The client stream is pulled from the cache.** A per-session `watch`
  counter wakes subscribers. A subscriber reads from its cursor:
  - data is an `output` event whose `id:` is the end offset;
  - a missing range is a `gap` event;
  - a status change is a full `status` event.

  Keepalives go out every 15 s.
- **Launch:**
  - The host must be online and must offer the workspace.
  - The provider is the requested one, or the default. If the host does not
    offer it, the server falls back through `claude-code`, `opencode`,
    `shell`, records `provider_fallback`, and returns 422 when none is
    available.
  - `max_sessions` is enforced as a 429.
  - The record starts as `creating`, then becomes `starting` when the
    `start` command is sent.
- **Input** is refused with 503 while the host is offline. It is
  at-most-once and never queued.

**The link:**
- One SSE stream, server to host. Every event's `data:` is a JSON command
  with a `cmd_seq`. The commands are `start`, `end`, `terminal.input`
  (base64), `terminal.resize`, `terminal.replay` and `refresh`.
- The host POSTs `hello`, `inventory`, output batches and status changes.
- Commands lost while the link is down are not replayed.

**The host** (`storm-runtime serve`):
- `/etc/storm-runtime/runtime.toml` holds `workspace_roots`, `providers`
  (the built-ins, or `kind = "cli"` with a command, args and `env_file`, or
  `kind = "fake"`), `max_sessions` and `scrollback_bytes`.
- **Output.** Each session has one uploader task. It posts its ring from its
  last sent offset, coalescing up to about 20 ms or 64 KiB, and retries until
  the link is back. `terminal.replay` rewinds its offset.
- **Restarts.** `sessions.json` records the live sessions. A host that
  restarts reports every entry as `failed (host_restart)` in its first
  `hello`.
- **Reconnecting** backs off from 1 s to 60 s, and re-authenticates when its
  token is refused.
- **Revocation.** When the key is refused, the host ends every session and
  exits non-zero (§5.6).
- **Workspaces** are the non-hidden subdirectories of each root. A name is
  never a path, and symlinks that leave a root are refused. A root that is
  inside, or contains, a Storm data root it can see (`/srv/storm` by default)
  is refused at startup (D3).
- **Terminal bytes** are standard padded base64 (RFC 4648 §4) in both
  directions: in the host's output posts, in `terminal.input`, and in the
  client stream's `output` events.
- **Input is bounded on the host.** A PTY write polls for room in 256-byte
  chunks within a 5 s deadline, in a per-session ordered writer task. A hung
  agent cannot block the link or hold the session against `end`.

**Verified** (`apps/server/tests/agent_e2e.py`, 54 checks, about 19 s; run by
`make test-live`): a real server and a real host, with the fake and shell
providers. It covers:
- AC-A1, F1 and F7
- R1: a server restart, after which scrollback is replayed from the host and
  the agent is never interrupted
- R2: a host restart, after which the session is `host_restart`
- R3: revocation, after which the host exits 3 and cannot authenticate
- S1, S2, S4 and S5
- offset resume and the explicit gap

AC-T2 is a relay unit test: the terminal stream served over the tunnel by the
same handler, under the same auth and owner check.

**77d. Slice 5: the client — one terminal surface, one stream per session,
and entry points that exist only for the owner.** *(2026-10-03)*

Designed against the existing M14 system (tokens, Lucide icons, the 900 px
breakpoint) and the taste-skill's product rules: explicit loading, empty and
error states; contrast; one accent and one radius scale; no decorative
status dots, only a real status chip.

**Entry points.**
- **Owner check.** The client probes `GET /v1/config/agent`. A 200 shows the
  *Agents* entry in Server settings and the shell; a 403 hides it. The
  client never infers a role. The server's own check decides, so AC-S1's
  "hidden for non-owners" cannot drift from the 403.
- **Routes.** `/agents` holds the sessions and tabs; `/agents/hosts` holds
  hosts, enrollment and the default provider.

**The terminal surface.**
- `lib/agent/terminal_surface.dart` is the only file that imports `xterm2`
  (5.2.x, pinned), so it can be replaced or forked (gate Q5).
- Output bytes go through a chunked UTF-8 decoder, because a stream event
  can split a multibyte character.
- The terminal's colours come from the tokens.

**The stream** (`lib/agent/terminal_stream*.dart`).
- On native, it is an `http` streamed GET with the bearer header.
- On web, it is `EventSource` with a single-use `?ticket=`. A browser
  cannot set a header on it, and `package:http`'s browser client buffers
  whole responses.
- Either way it resumes by `?offset=`, never `Last-Event-ID`. It reconnects
  with backoff from the last offset it rendered.
- A `gap` at the start of a view clears the terminal and renders from the
  oldest retained byte (freeze §14).

**Input.**
- Keystrokes coalesce for about 15 ms and are sent as serial POSTs per
  session, never retried (freeze §11.3).
- A failure is shown inline: a `503` reads "the host is offline".

**Resize.**
- Resizes are debounced and carry `focus: true` when this client takes focus
  (§11.4).
- Every client renders at the size in the session's `status`.

**Tabs.**
- They are per device: `ags_` ids persisted in shared preferences and
  reconciled with `GET /v1/agent/sessions` at launch, so a dismissed session
  drops off.
- At 900 px and wider there is a tab strip. Below it, one session fills the
  screen with a switcher sheet. The phone layout stays the default.

**Phone.** An extra-keys row: Esc, Tab, sticky Ctrl, the four arrows and
Paste.

**Desktop.** The M18 shortcuts are suppressed while the terminal has focus. A
local `Shortcuts` maps every global activator to
`DoNothingAndStopPropagationIntent`, so a key the terminal leaves unhandled
does not reach the app.

**Launch.**
- The flow is host (online only), then workspace (warning when it already
  has a live session), then provider (default preselected, uninstalled ones
  disabled).
- The launch sheet shows the network line: "Network: inherits <host>'s
  policy".
- A fallback is announced in plain words.

**The visual pass** (2026-10-04, at the operator's request). Every screen was
rendered to PNG with the real fonts and icons at phone (400 px) and desktop
(1400 px) width, in both themes, and checked by eye. That found:
- the extra-keys row overflowing a phone;
- status chips inheriting the mono face in lists only;
- Revoke shown as an equal, accent-coloured button on every host card;
- no host "last seen";
- no thumb-reachable New session on a phone.

All of it is fixed.

**For the device pass (AC-Q5):** `xterm2` lays out a run of text as one
paragraph. A glyph missing from IBM Plex Mono (Claude Code's `✻`, say) falls
back to a font of another width and shifts the rest of that line. If it shows
on a phone, the fix is a bundled terminal font with wide symbol coverage
behind the surface boundary.

**77e. Slice 6: `storm-runtime` ships as its own `.deb`, as its own user,
with the §5.8 sandbox.** *(2026-10-03)*

**The package.** It mirrors `storm-relay` (decision 70): a musl binary built
with zig in its own release job, and `cargo deb --no-build`. Its maintainer
scripts are written the #46 way: `prerm` acts on `remove` only, because dpkg
runs the old package's `prerm` on upgrade.

**The account.**
- `postinst` creates a system user `storm-runtime`, whose home is
  `/var/lib/storm-runtime/home`. That is what `sudo -u storm-runtime -H`
  logs Claude Code into, and the unit's `HOME` matches it.
- **It is never added to the server's group.** That group mode is what keeps
  a co-located runtime out of the vaults and `auth.db` (P3, AC-S3).
  `tests/packaging.rs` asserts that no script grants it.

**Layout.**
- `/var/lib/storm-runtime` (`0700`) holds the identity, `host.json` and
  `sessions.json`.
- `/var/lib/storm-runtime/workspaces` is the default workspace root, inside
  the unit's writable paths.
- Other roots need a `ReadWritePaths=` drop-in, which `deploy/README.md`
  shows.
- `/etc/storm-runtime/runtime.toml` is a conffile.

**The unit** (freeze §5.8):
- **Kept:** `NoNewPrivileges`, `PrivateTmp`, `PrivateDevices` (the PTY
  subsystem survives it, per gate Q6), `ProtectSystem=strict`,
  `ProtectHome`, the kernel protections, `LockPersonality`,
  `RestrictNamespaces`, and `RestrictAddressFamilies` with `AF_NETLINK`.
- **`MemoryDenyWriteExecute` is never set:** OpenCode's TUI dies under it.
- `UMask=0027`.
- `RestartPreventExitStatus=3`: a revoked host stops rather than loops.
- **A fresh install is not started.** It runs once enrolled.

**Verified on codebox** with the real `postinst` and the real unit under
systemd, against a real server:
- `storm-runtime` is in its own group only, and its directories came out
  `700`/`700`/`750`. It comes online offering all four providers.
- **Shell:** a real PTY (`/dev/pts/N`) under `PrivateDevices`, as
  `storm-runtime`, with the right `HOME`.
- **Claude Code** started as `claude --permission-mode default` and drew its
  first-run screen.
- **OpenCode's TUI** drew. No `mprotect` failure, so the MDWE omission
  holds.
- Each session's process group was gone after End.
- **AC-S3 analogue:** the runtime account is refused a `0750` data tree it
  does not own.
- **Revocation:** exit 3, `NRestarts=0`, and the unit stays stopped.
- No token in the journal.

**AC-F6** is in `agent_e2e.py`, now 61 checks: three concurrent sessions
across two real hosts. The shared workspace reports two live sessions (the
warning), input stays with its own session, and each session ends
independently. `docs/srp-v1.md` §5.3 gains AM4's note on the terminal stream.

**AC-P1 itself still needs the operator.** Claude Code and OpenCode must be
logged in as `storm-runtime` before an agent can accept a prompt and edit a
file.

**78. Agents are a space beside Notes, not a page inside settings.**
*(2026-10-04, after the first hands-on test of V1 on the dev server)*

**What was wrong.** The only way to the agent screens was Server settings →
Agents, and only for an owner. An owner looking for "the agents" found
nothing on the dashboard, nothing in the sidebar, and a settings section
they had no reason to open. Sessions are something you *do*; settings is
where you configure. The routes sat outside the dashboard's subtree, so
Android back from them left the app (the bug decision 17 fixed for
everything else). A member could still open `/agents` by URL and get an
empty frame.

**The model: two spaces, Notes and Agents, owner-only.** An account that is
not an owner sees exactly the app it saw before (AC-S1).

- **Phone (the default).** The dashboard is home for both spaces; there is no
  mode switch. An **Agents band** sits directly above *Recently opened*,
  always in the same place:
  - live sessions as rows (provider in workspace, host, age), a tap opens the
    terminal;
  - idle, one quiet row that follows setup progress: *Set up a host* (no
    hosts) · *<host> is offline* (none online) · *New session* (otherwise).
  It never moves when a session ends: a home screen that reshuffles itself
  when an agent finishes breaks muscle memory. There is **no "N live" stat**
  in the masthead; it would be a second entry point for one intent.
- **The Agents space on a phone.** The list — *Running*, then *Ended* — with
  *New session* as a pill at the bottom: the nav bubble's grammar, replacing a
  Material FAB, because the space is a peer of the vault screens
  (settings-shaped screens such as Hosts and MCP keys keep theirs). The
  launcher preselects the host this device used last, and a host's only
  workspace, so most launches are one tap. An open session fills the screen
  with its extra-keys row where the pill was — the two are never on screen
  together — and Android back returns to the list rather than leaving.
- **Wide (≥ 900px).** A **Notes | Agents** switch tops the sidebar, for
  owners only. The Agents side is its **own shell** drawn like the vault
  sidebar — sessions where the tree was, *New session* and *Hosts* at the
  bottom — rather than a mode of `VaultSidebar`, because that sidebar lives
  inside `VaultShell` behind `VaultGate` and needs a vault, and an agent has
  none. The terminal and its tab strip fill the pane.
- **Settings keeps configuration only:** hosts and the default provider.
- **Freshness.** The band and the list reload on appear, on pull-to-refresh
  and every 15 s while visible; an open session already has its own stream.
  **Offline is a state, not an error** (the ground rule): agents cannot run
  without the server, and the copy says that plainly instead of showing a
  failure.
- **Routes.** `/agents` and `/agents/hosts` become children of the dashboard
  (same URLs). The router sends a non-owner on any `/agents` path back to the
  dashboard. The server's 403 was always the boundary; this is usability.

**Why "Notes" and not "Vaults".** The switch names what you do in each
space; a vault is a container, and "Agents" is an activity.

**Not in this pass:** a per-session deep link (`/agents/s/:id`) — tabs
already reopen sessions — and keyboard shortcuts for the switch, which belong
with the M18 set.

**Built** on `feat/agents-navigation`:
- One data source: `agentOverviewProvider` (sessions + hosts), `autoDispose`
  so the 15 s timer dies with the last watcher, and it **never throws** —
  unreachable is a value. Riverpod 3 retries a failing provider on its own
  schedule, which would have been a second, unbounded poll.
- `AgentsShell` is a `ShellRoute` under the dashboard; `AgentsSidebar`,
  `AgentsBand`, `SpaceSwitch` and one `AgentSessionRow` shared by all three
  surfaces. The switch gates itself, so a sidebar places it unconditionally.
- Pull-to-refresh on the dashboard re-asks the owner check, so a role changed
  on the server no longer needs an app restart to show.
- **Found on the way:** the dashboard decides its desk-width forward by
  reading the router's location *during build*, and on a `go('/')` from
  another route that read still returns the old one — so it drew the phone
  dashboard on a wide screen. Nothing went to `/` at desk width before this.
  The switch goes to `notesHome()` (the vault last in use) instead of
  relying on the forward. **Reading `GoRouterState.of` instead — the obvious
  root fix — was tried in review and broke back navigation:** the same stale
  read is why system back at desk width lands on the dashboard and leaves the
  vault instead of being forwarded straight back in. The read is
  load-bearing; changing it is its own decision, not a tidy-up.
- **Review pass (`/simplify`):** the agent API factory provider now has a
  real default derived from the session's address and token only, so a theme
  change or a font-size drag no longer re-runs the owner check (and the
  test-only branch is gone); sessions and hosts load in parallel; a poll that
  falls while the app is backgrounded is skipped and made up on resume;
  Hosts refreshes the overview after a change; the session row and the band's
  idle row share one `AgentRow`; the pill is the nav bubble's own
  `StormPill`/`PrimaryCircle`; ages use `relativeTime`, the wording *Recently
  opened* uses directly below. **Deferred:** one sidebar frame shared by the
  Notes and Agents sides (the Agents side ignores ⌘\ collapse today), a
  shared segmented control with the note's Read/Edit toggle (different scale;
  tokens aligned instead), and sessions as routes, which would also remove the
  sidebar's "if on Hosts, go back to the list" branch.

**Verified:** `test/agents_navigation_test.dart`, 14 tests on the real router
— member: no band, no switch, `/agents` and `/agents/hosts` bounce home,
nothing in Server settings (scrolled to the end, since the list is lazy and
"not found" would otherwise mean "not built"); owner: band above Recents in
all four idle states and with live sessions, a tap lands in the terminal,
system back goes list → dashboard; phone list *Running* before *Ended* with
the pill and no FAB; wide switch round trip, Hosts in the pane; the launcher's
remembered host. **Both guards were mutation-tested**: with the redirect off
the member test fails, with `PopScope` letting back through the band test
fails. The full client suite passes (720) bar `editor_save_loop_test`, which
codebox cannot finish (CI judges it). A screenshot pass at 400 and 1400 px,
dark and light, found one defect — the idle row's icon cramped in the dot's
16 px column, now 22 px for both.

**Revisit if** a non-owner role gains agent access (the band and switch are
gated on the one owner check), or a third space appears, at which point the
switch wants to become a real space picker rather than a two-way toggle.

**79. The backup copies the storage root the server uses, not the one in
`storm.env`.** *(2026-10-04, found planning the v0.3.0 prod upgrade)*

`storm-backup.sh` rsynced `$STORM_VAULT_ROOT`. But the stored root wins
(`Registry::load`; the invariant in `CLAUDE.md`): `STORM_VAULT_ROOT` only seeds
a registry that does not exist yet, and a root chosen in the app is recorded in
`state/vaults.json` and never written back to the env file. Prod is exactly
that case — `storm.env` says `/srv/storm/vaults` (empty), the registry says
`/mnt/media/Docs/storm` (the NAS) — so the moment 75's fixed backup unit ran
there, every nightly backup would have succeeded, said `verified`, and held
**no notes**: the index snapshot verifies, the empty rsync does not complain.

- **`storm-server storage-root --state DIR`** prints the root the registry
  records, exit 0. With no registry, or one that names no root, it prints
  nothing and exits 3, and the script falls back to `STORM_VAULT_ROOT` — a
  server that has never run has nothing else to go on.
- **The script asks for it and says which it used**, and names the env
  file's value too when the two disagree, so a backup log shows the drift
  instead of hiding it.
- Reading the registry through the server, not with `sed` on the JSON:
  one parser for the file, the one that also writes it.

**Not done here:** rewriting `STORM_VAULT_ROOT` in `storm.env` when the app
changes the root. The env file is root-owned and the server runs unprivileged,
and once the backup asks the server, nothing else reads that value after the
first run.

**Revisit if** the backup ever runs where `storm-server` is not installed
(a backup host pulling over the network); then it needs the root some other
way.

**80. Every server-wide setting is owner-only.** *(2026-10-04, found during
the MCP Gateway spec review)*

`PUT /v1/config/registration` refused a member from the start (A13), and its
neighbours did not. **Any signed-in member could re-point the storage root
(`PUT /v1/config`), switch MCP on and arm its writes (`/v1/config/mcp`), or
replace the relay list (`/v1/config/relays`).** `AllowAuthenticated` made
that look harmless, but these are server settings, not vault access, and A9's
matrix never gave them to a member. A member arming `mcp_writable` matters
more once agents use the vault (the gateway spec in the vault puts agent
writes behind that switch).

- **One helper, `require_owner_session`, called by every `PUT /v1/config*`
  handler**, with registration's own check folded into it and its message
  unchanged. A refusal is `403` and leaves the setting untouched.
- **`only_an_owner_may_change_server_config` loops over the four routes**, so
  a new config route added without the check is caught by adding it to the
  list. That is the shape of the bug: the check existed on one route and was
  copied to none of the others.
- **Not here:** vault create, rename and remove are still open to a member.
  They are vault administration, and A9's matrix gives them to admin too, so
  they belong to the authorization release rather than to an owner check.
  `GET /v1/config` (Q21) is also left as it is.

**Revisit if** the authorization release lands: these checks become the
policy's "server config" rule, and admin may gain them (A9's matrix says
yes).

**81. The MCP Gateway: Storm owns integration credentials, and a Runtime Host
only forwards.** *(2026-10-04; spec approved by the operator, nothing built)*

**The spec** is *MCP Gateway/V1 Specification* in the personal vault, rev 3:
decisions G-D1 to G-D25, gate evidence in its §19. This entry records that it
is approved and which existing decisions it amends. Where they disagree, the
spec wins and this entry gets fixed. **It is not built.** It is its own track
and starts after Agent Runtime V1's on-device acceptance (G-D1).

**In one paragraph.**
- The owner connects an MCP integration (Notion, Linear, GitHub…) **once, in
  Storm**. storm-server holds its credential, encrypted in a new
  `state/gateway/gateway.db`, and is the MCP client to the upstream.
- An agent session reaches it through a per-session **stdio bridge**
  (`storm-runtime mcp-bridge`). The bridge forwards to the host daemon,
  which POSTs `/v1/runtime/sessions/{ags}/mcp/{connection}` with its
  **existing host token**.
- **No new credential exists.** The agent holds nothing it could use off the
  host, and the host never sees an upstream credential.
- Every call is authorized in `ops.rs`: host owns session, live, grant,
  connection, owner active, method and tool allowed.
- Storm's own vault is a **built-in connection**. This is how Roadmap Phase 1
  (agents use the vault) arrives.

**The amendments** (Agent Runtime **AM23–AM32**, D13 in *Agent
Runtime/Decisions*; AM22 stays reserved for the terminal-protocol draft):

| AM | Was | Amends | Change |
|---|---|---|---|
| AM23 | AM-G1 | Freeze §7.1 | The host is also told the ids and slugs of granted connections; never a credential or a user. |
| AM24 | AM-G2 | Freeze §12.2, *Security* | "Never stores or forwards **model-provider** secrets." **Integration** credentials are stored encrypted, sent only to their own upstream, never to a host or agent. |
| AM25 | AM-G3 | *Auth Data Model* invariant 5 | Invariant 5 is scoped to Storm's own credentials in `auth.db`. Storm may be an OAuth **client** to upstreams; that is additive under R1. |
| AM26 | AM-G4 | **Decision 39** | storm-server's MCP surface stays HTTP only. The host's per-session bridge is stdio. rmcp gains its client features at the same `=3.1.2` pin, and **only its `*_once` methods** may send a request upstream. |
| AM27 | AM-G5 | AM6, *Networking* | The host-inherited egress exception now also covers vault and integration data. It is shown at launch, and closed by Phase 2. |
| AM28 | AM-G6 | *Roadmap*, freeze §2 | Phase 1 is delivered through the built-in `storm` connection. Its exit criteria are written before the track starts. |
| AM29 | AM-G7 | *Security* scopes | `integration.manage` (owner) and `integration.use:<connection>` (a grant). |
| AM30 | AM-G8 | *MCP Integration* | Agents in sessions reach Storm's vault through the gateway, as `Actor::Agent`, not with `stk_` keys. |
| AM31 | AM-G9 | `authz.rs` | `Actor::Agent { session_id, host_id, user_id, role }`: the owner's identity (A14.3), with the session id for audit. |
| AM32 | AM-G10 | Freeze §9.2; **decision 77a** "The environment" | With grants, `claude-code` gains `--mcp-config <session>/mcp.json --strict-mcp-config`. `opencode` gains `XDG_CONFIG_HOME=<session>/xdg`, `OPENCODE_DISABLE_PROJECT_CONFIG=1` and `permission: {"<slug>_*": "ask"}`. `shell` gets nothing. |

**What the gates found that a reader would otherwise re-derive** (run
2026-10-04 on a Runtime Host, throwaway harness, evidence in the spec):
- **OpenCode runs MCP tools without asking by default**, and its
  `OPENCODE_CONFIG*` variables *merge* with the global config rather than
  replacing it. Without AM32's three settings, "the provider prompts before a
  tool" is false for OpenCode.
- **rmcp's `call_tool` re-sends `tools/call`** to drive SEP-2322 rounds. Only
  `call_tool_once` keeps a call at-most-once. An in-flight call whose stream
  breaks is not re-sent, and an expired upstream session (`404`) re-initializes
  with the call executed exactly once.
- **Notion and Linear refuse a LAN `http` OAuth redirect** at registration,
  and accept loopback and `storm://`. So the browser returns to the *client*,
  which relays the code to the server. GitHub's remote MCP has no dynamic
  registration, so it is a PAT connection in V1 (G-D24).
- **The bridge rules were proved by breaking them.** Each mutation was
  caught: leaking the replayed `initialize` result, retrying a failed call
  (2 executions), and not cancelling an open elicitation. A "drop stale
  progress" filter turned out unreachable: progress rides its call's own
  response stream, so a failed call has no stream left to deliver it.
- **No upstream credential reached the host.** A canary was absent from
  62,561 host files and from 205 live processes' `environ` and `argv`, with
  positive controls. The journal check (C3) needs an account that can read
  the journal, and is the one item before the spec is called frozen.

**Accepted, and recorded as V1 risks rather than mitigated:**
- every non-shell session gets all of the owner's integrations and vaults,
  with host-inherited egress;
- sessions on one host share an OS user, so Phase 2 isolation must precede
  any multi-user gateway;
- a GitHub PAT is long-lived and scoped at GitHub, not by Storm.

**Revisit if** OpenCode gains a strict-config flag (AM32 simplifies);
either CLI adopts the 2026-07-28 revision (G-D25's pass-through gets an
actor that acts on it); or a supported upstream stops accepting loopback
redirects (G-D13).

**81a. The gateway is built in eight slices, storage first and the client
last.** *(2026-10-05; M20 accepted, so G-D1 lets the build start)*

When this was decided the spec was frozen on C3, the operator's journal
grep (it passed on 2026-10-08, printing 1). The build did not wait for it: C3 can only confirm or refute a property of the gates'
harness, not change the design. If C3 prints anything but 1, the slices stop
and the spec is re-opened. Each slice is one PR, stacked on the one before,
each with its own sub-decision (81b, 81c, …). Like 77, a slice that finds a
concrete contradiction in the spec stops and reports it; nothing is
redesigned in code.

**The slices**, in build order:
1. **The gateway store** (81b). `apps/server/src/gateway/`: `gateway.db`
   with the five §8 tables, created additively; XChaCha20-Poly1305 over
   `connection_id ‖ kind`; the key file `state/gateway/keys/<key_id>.key`
   created `0600` in a `0700` directory; `backup_all()` carries the database
   and its keys together. No route, no network.
2. **Connections** (81c). The connection operations in `ops.rs` and the
   owner-only `/v1/integrations/connections` routes (§14) for `static` and
   `none` connections. Credentials are written encrypted and never returned.
   `security_events` gains the `integration_*` events, none with a secret.
   The `storm` slug is reserved for the built-in connection.
3. **The upstream client** (81d). rmcp's client features at the same `=3.1.2`
   pin (AM26), `ring` only; **only `*_once` request methods**, enforced by a
   test that reads the source. The SSRF rule (https; no loopback, link-local
   or private address, checked on the resolved address). `POST …/test` and
   `GET …/tools`, the allowlist (tools that appear later default off), the
   `calls` audit with its retention.
4. **The gateway route** (81e). `Actor::Agent` (AM31); grants snapshotted at
   launch into `agent.db.session_mcp_grants` with `allow_vault_writes`;
   `start` gains `mcp: [{id, slug}]`; `hello` gains
   `capabilities.mcp_bridge`, and an old host gets no grants and a launch
   notice; the Host-tier `POST /v1/runtime/sessions/{id}/mcp/{connection}`
   with request-scoped messages on its own response stream; the per-call
   authorization in `ops::integration_call` (§7) with stable error codes;
   the built-in `storm` connection served in process from `mcp.rs`'s
   router, without `delete_note`, with writes only under both flags;
   `session_unknown` after a restart; the §9 limits; `mcp.message` for
   unsolicited messages; disconnect revokes grants.
5. **The runtime** (81f). `storm-runtime mcp-bridge` (bridge rules 1–6), the
   daemon's `run/mcp.sock` in a `0700` directory, per-session directories
   and handles, `capabilities.mcp_bridge`, and the AM32 config writers:
   `claude-code` gets `--mcp-config <session>/mcp.json --strict-mcp-config`;
   `opencode` gets `XDG_CONFIG_HOME=<session>/xdg`,
   `OPENCODE_DISABLE_PROJECT_CONFIG=1` and `permission: {"<slug>_*":
   "ask"}`; `shell` gets nothing. The bridge's R5, R7 and R8 rules each have
   a test and the `leak_init`, `retry` and `no_cancel` mutations as negative
   tests.
6. **OAuth** (81g). RFC 9728 → RFC 8414 discovery under the SSRF rule;
   dynamic registration as a public client, else a pasted client; PKCE S256
   flows (hashed state, encrypted verifier, single use, 10 minutes);
   `POST …/{id}/authorize` and `POST /v1/integrations/oauth/callback`;
   single-flight refresh that persists a rotated pair before using it;
   RFC 7009 revocation on disconnect. rmcp's `CredentialStore` and
   `StateStore` are implemented over `gateway.db`.
7. **The client** (81h). Settings ▸ Integrations (owner only, by the
   server's 403, as 77d's Agents entry), the launcher's "Allow vault writes"
   toggle and its egress line, the old-host notice, and the OAuth flow:
   `storm://oauth` on Android and macOS, a loopback listener on desktop,
   static tokens only on web. **This host has no Flutter: CI is its only
   verification.**
8. **Acceptance** (81i). `apps/server/tests/gateway_e2e.py` against a real
   server, a real `storm-runtime` and a mock upstream: R1–R8 with the three
   mutations as negative tests, the credential boundary C1, C2 and C4–C7 on
   the test host with positive controls, and the automatable §17 items
   (member 403 everywhere, `stk_` refused, an old host's notice, disconnect
   mid-session, URL-mode elicitation never reaching the agent). In
   `make test-live` and CI. The gates' harness is ported, not shipped.

Then on-device acceptance, the operator's: the real logins G1 deferred,
Claude Code before OpenCode (AC-P1), a GitHub PAT, a write merged against a
phone edit, and C3 against the real build.

**Why this order.**
- **Storage, then the things that write it, then the things that read it.**
  Every later slice stores or reads a credential, and the encryption and
  backup rules are the ones a regression loses data under.
- **The upstream client before the route.** The route is the client's
  caller. Testing the client against a mock upstream alone keeps the `*_once`
  and SSRF rules provable without a host.
- **The route before the runtime.** The bridge is tested against the real
  route, the way 77c tested the host against the real link.
- **OAuth after the whole static path works.** Static tokens are
  first-class (G-D13), GitHub is a PAT (G-D24), and the operator deferred the
  real logins. OAuth adds a way to obtain a credential, not a new way to use
  one.
- **The client last**, because it cannot be run here, and every server
  surface it calls is then fixed.

**Decisions this plan makes where the spec was open:**
- **The AEAD is the RustCrypto `chacha20poly1305` crate's
  `XChaCha20Poly1305`** (the spec said "to confirm"). Pure Rust, no
  `aws-lc-rs`, builds on musl with zig. The 24-byte nonce is random per
  write, which XChaCha's nonce size makes safe without a counter.
- **The gateway is a module of storm-server, `src/gateway/`**, beside
  `src/agent/`. Operations stay in `ops.rs` (decision 37). `gateway/` holds
  storage, crypto and the upstream client, never a policy decision: those
  are in `ops::integration_*`.
- **`gateway.db` is opened at boot and held for the process's life**, as
  `agent.db` is. A missing key file with ciphertexts present is a loud boot
  warning and those connections become `needs_reauth`, never a refusal to
  start: losing the key means reconnecting, not a lockout (§8).
- **`tests/e2e.py` stays 81/81 unmodified** across every slice. New live
  coverage goes in `tests/gateway_e2e.py`, as auth's went in
  `auth_e2e.py`.
- **Decision 80 (#76) is assumed merged before slice 4 ships.** Agent
  writes are gated on `mcp_writable`, and until 80 a member could switch it
  on. No slice depends on 80's code.

**Revisit if** a slice finds the rmcp client cannot tell which in-flight
call an upstream elicitation belongs to (slice 4 needs that to put it on the
right response stream), or `chacha20poly1305` turns out to pull a C
toolchain into the musl build.

**81b. Gateway slice 1: `gateway.db`, its data key, and both in every
backup.** *(2026-10-05)*

**What exists.** `apps/server/src/gateway/`, opened at boot beside the Agent
Manager and held in `AppState`. No route reads it yet; 81c's connection
operations are its first caller.
- **`gateway.db`** (`state/gateway/`): `connections`, `credentials`,
  `oauth_clients`, `oauth_flows` and `calls` (spec §8), plus `meta` for the
  active key id. Every statement is `CREATE … IF NOT EXISTS`, and a test
  fails on a `DROP`, an `ALTER` or a bare `CREATE`. The OAuth tables are
  created now, with the columns slice 6 needs (issuer metadata, the
  `resource`, the scope), so slice 6 adds behaviour, not columns.
- **The AEAD** is `chacha20poly1305` 0.10 (`XChaCha20Poly1305`, pure Rust; no
  new C, no `aws-lc-rs`). A random 24-byte nonce per seal. **The AAD is
  `id 0x00 kind`**: the spec's `connection_id ‖ kind`, with a separator so
  `("ab","c")` and `("a","bc")` differ. A ciphertext moved to another row,
  or to another kind, does not open.
- **The key** is 32 raw bytes in `state/gateway/keys/gwk_<26>.key`, created
  `0600` with `create_new` in a directory created `0700`. `meta.active_key_id`
  names the sealing key; every key file that loads can open, because each
  row records its key.
- **Opened secrets are a `Plaintext`** whose `Debug` prints `<redacted>`
  and whose buffer is overwritten on drop. `Sealed` prints its key id and
  length only.
- **`backup_all()` calls `gateway::backup`** right after `backup_auth`,
  before the "no vaults" early return: `gateway.db` by `VACUUM INTO`, and the
  key files copied and re-tightened to `0600` in `0700`. The snapshot mirrors
  the state layout, so a restore is a plain copy.
- **The `calls` audit** stores `at_ms` (epoch milliseconds) so the 30-day
  retention is an exact comparison. The row type has no field for
  arguments or results, so G-D18 is kept by the type. `prune_calls`
  applies age first, then the 100k-row cap.

**A lost key is not a lockout** (§8). If `meta.active_key_id` names a key
file that is missing or unreadable, boot makes a new key active, marks every
connection sealed under an unloadable key `needs_reauth` (not `revoked` or
`disabled` ones), and logs one warning that says to restore
`state/gateway/keys/`. The old key file, if it is merely unreadable, is left
where it is. The notes stay online either way.

**`revoked` is a connection status the spec's §5 list omits.** §13 says a
disconnect marks the connection revoked and keeps rows for the audit, so the
row stays as a tombstone with its credentials deleted, and the unique index
on `(owner_user_id, slug)` excludes it, so the slug is free again. This is
§13 read literally, not a design change.

**Verified.**
- 18 new unit tests, plus `a_backup_carries_the_gateway_store_and_its_data_key`
  through `backup_all` on a server with no vaults. The backup test restores
  by plain copy and opens the credential.
- **Each guard was proved by mutation**, all seven caught: `backup_all`
  without the gateway call; a backup without the keys; the AAD ignored;
  plaintext stored as the ciphertext (the database and its WAL are
  scanned for the canary); a key created `0644`; a lost key not marking
  its connections; `Plaintext`'s `Debug` printing the secret.
- `cargo tree -i aws-lc-rs` finds nothing. fmt, clippy `-D warnings` and
  all 476 unit tests pass. Against a live server: `e2e.py` 81/81
  unmodified, `mcp_e2e.py` 80/80, `agent_e2e.py` 61/61.

**Revisit if** data-key rotation is built (it is LATER in the spec): it needs
a re-seal pass and a rule for when an old key file may be deleted, and
`meta.active_key_id` is where that starts.

**81c. Gateway slice 2: connections, the owner's alone, and a credential
bound to its upstream for life.** *(2026-10-05)*

**The routes** (session tier, every one gated by `ops::require_integration_owner`):
- `GET` / `POST /v1/integrations/connections`
- `GET` / `PATCH` / `DELETE /v1/integrations/connections/{id}`

A member gets `403` on all five. Another owner's connection is `404`, the
same as an id that does not exist, so ids cannot be probed. An `stk_` key is
`401`, because the tier refuses it (§14: no MCP tool manages integrations).

**What this slice connects:**
- `static` connections: a header, `Authorization` by default, and a value.
  A GitHub PAT is `Bearer ghp_…` (G-D24).
- `none` connections.
- `oauth` is refused with `400` until its authorize flow exists (81g), so
  no connection can sit in `pending_auth` with no way out.

**Decisions:**
- **A connection's URL, slug and auth kind never change.** PATCH has no
  field for them, and the store's update never writes them:
  - Re-pointing a connection would send its credential to a new host.
    AM24 says a credential is presented only to its own upstream.
  - The slug is what live sessions were told at launch.

  Moving a connection means a new one.
- **The built-in `storm` connection is listed first, with id and slug
  `storm`, and is not a row.** It reports `vault_writes_available`, which
  mirrors `mcp_writable`. PATCH and DELETE on it are `400`. Its id equals
  its slug, so `start.mcp` and the gateway route name it like any `mcc_`
  connection.
- **Slugs are `[a-z0-9-]`, 1–32 characters, with no leading or trailing
  `-`, and never `storm`.** They are derived from the display name when
  omitted. A slug becomes OpenCode's `"<slug>_*"` permission glob (AM32), so
  `_` is refused: one slug's glob must not match another's tools.
  Uniqueness is per owner, decided by the unique index (`409`), not by a
  read-then-write. A disconnected slug is free again.
- **No private-range check on a static connection's URL.** It must be
  absolute `https`, with no userinfo and no fragment. §10's SSRF rule
  applies to OAuth discovery, whose URLs come from an upstream's metadata
  (81g). A static URL is typed by the owner, and a homelab's own MCP server
  on the LAN is a main use.
- **A static credential cannot inject a header.** The value is printable
  ASCII with no CR or LF. Names the transport owns (`Host`,
  `Mcp-Session-Id`, `Content-Type` …) are refused.
- **Disconnect** turns the row into the `revoked` tombstone and deletes its
  ciphertexts in one transaction (§13). Upstream revocation arrives with
  OAuth (81g). A PAT has no revocation call from Storm.
- **Insert is one transaction with its credential**, so there is never a row
  whose credential failed to land.

**Audit.** The `security_events` kinds are `integration_created`,
`integration_reauthorized` (a rotated static token), `integration_disabled`,
`integration_enabled` and `integration_deleted`:
- `integration_enabled` is the inverse §14 did not name. It is additive.
- The detail is the id, the slug, the auth kind and the upstream's **host
  only**: an owner can paste a URL with a key in its query string.

**Verified.**
- Six route tests: the full flow (create, list, disable, re-enable, rotate,
  allowlist, delete, slug reuse, ciphertexts deleted); member `403` on every
  route; a second owner's `404` and an independent slug space; `stk_`
  `401`; ten bad inputs; and canaries in a credential and in a URL's query,
  absent from every response and every event.
- Unit tests for slugs, URLs, static headers and a redacted `Debug`.
- **Mutation-proved, all 8 caught:**
  - the owner check removed
  - rows not scoped to their owner
  - the audit recording the full URL
  - disconnect keeping ciphertexts
  - CR/LF allowed in a value
  - `http` allowed
  - `storm` claimable
- fmt, clippy `-D warnings`, 486 unit tests.
- Live: `e2e.py` 81/81 unmodified, `mcp_e2e.py` 80/80, `agent_e2e.py` 61/61.

**Not yet:** `POST …/test` and `GET …/tools` need the upstream client
(81d), and so does `known_tools`. The allowlist is stored as given until
then.

**81d. Gateway slice 3: the upstream client, `*_once` only, and the
owner's test.** *(2026-10-05)*

**rmcp gains its client features at the same `=3.1.2` pin** (AM26):
`client`, `transport-streamable-http-client-reqwest` and
`reqwest-tls-no-provider`. `cargo tree -i aws-lc-rs` still finds nothing.
- **`only_the_once_methods_send_a_request_upstream`** reads `ops.rs` and every
  file in `src/gateway/`, and fails on `.call_tool(`, `.get_prompt(`,
  `.read_resource(` or their `_with_mrtr_max_rounds` forms.
- `list_all_tools` is allowed: it pages with cursors and re-sends nothing.

**The HTTP client is the gateway's own**, handed to rmcp's transport with
`with_client`:
- **No redirects followed.** A test points an upstream at a 307 to a second
  server and asserts the credential header never arrives there.
- **rustls on `ring` with the bundled webpki roots,** passed as a
  preconfigured config. It does not depend on `/etc/ssl` (static musl), nor
  on the process default provider having been installed first, which unit
  tests never do.
- **A connect timeout, and no whole-request timeout.** A whole-request
  timeout would also cut the standalone SSE stream. Each operation carries
  its own deadline: 30 s for the owner's probe here, and the spec's 60 s per
  agent call in slice 4.
- **`reinit_on_expired_session(true)` is set by name**, so it is not
  tidied off (G3, R3b).

**The credential reaches the transport as a header and nowhere else.**
`Gateway::target` opens the sealed value per call. It becomes a
`HeaderValue` marked sensitive. `Target`'s `Debug` names header names only.
A static credential uses rmcp's `custom_headers`, not `auth_header`, because
the owner stores the whole value (`Bearer ghp_…`, or an `X-Api-Key`).

**Errors are stable codes, never upstream text** (§12):
- `upstream_unauthorized`: 401, or 403 with a challenge. It sets the
  connection to `needs_reauth`.
- `upstream_rate_limited`
- `upstream_unavailable`
- `upstream_protocol_error`
- `integration_needs_reauth`: the credential cannot be opened.

rmcp formats a non-success answer as `HTTP <status>: <body>`, and only the
status is read. **Neither `ClientInitializeError::TransportError` nor
`ServiceError::TransportSend` marks its transport error as a `source`**, so
the classifier steps into them by hand. Found by a failing test, which first
read a 401 as unavailable.

**The owner's routes:**
- `POST /v1/integrations/connections/{id}/test` and `GET …/tools` open one
  short-lived session with no capabilities, list every tool, and close.
- They record `last_ok` or `last_error_code`, and one `calls` row with
  method `tools/list` and no session.
- A disabled connection is `409`, and the built-in one is `400`.
- `tools` answers a failure with `502` and the code only.

**The allowlist rule (G-D16), in `reconcile_tools`:**
- The first listing turns every tool on and records them in `known_tools`.
- After that, a tool not in `known_tools` stays off and is reported once as
  `new`. That report is the owner's notice: `new_tools` on `test`, and
  `new: true` on `tools`.
- `known_tools` only grows, so a tool that vanishes and returns keeps the
  owner's decision.
- Only the owner's listings update it. An agent's listing (slice 4) will
  filter by the allowlist and record nothing.

**The call audit** is written through `Gateway::record_call`, which never
fails the call it describes. It prunes to 30 days and 100k rows at boot and
every 1000 rows.

**`--gateway-allow-http-upstreams`, a hidden `serve` flag, for the test
suites only.** It lets a mock upstream on loopback be `http://`, and logs a
warning when set. Nothing in a deployment sets it.

**The elicitation question in 81a is answered.** rmcp 3.1.2 tags an inbound
upstream request with `InboundStreamOrigin::OutboundRequest(<request id>)`,
the id of the POST whose response stream carried it (SEP-2260). Slice 4 can
put an elicitation on the right agent call's stream with no workaround.

**Verified.**
- Seven upstream tests against a real rmcp server in process: a probe with
  its credential; a refused credential; an unreachable port; the redirect;
  the allowlist rule; the source guard; the redacted `Debug`.
- Three route tests: the owner's test turns everything on and a later tool
  stays off; a refusal sets `needs_reauth` until the token is rotated; `http`
  needs the flag, and disabled or built-in connections are not probed. The
  member test now covers `test` and `tools`.
- **Mutation-proved, all 7 caught:**
  - a `.call_tool(` in the gateway
  - redirects followed
  - new tools on by default
  - 401 read as unavailable
  - a refusal not setting `needs_reauth`
  - `http` without the flag
  - the probe not audited
- **Real TLS:** `a_real_https_upstream_answers_through_ring_and_the_bundled_roots`
  is ignored in CI because it needs the network. Run here, Notion's MCP
  endpoint answered `Unauthorized` over TLS through `ring` and the bundled
  roots.
- fmt, clippy `-D warnings`, 496 unit tests. Live: `e2e.py` 81/81
  unmodified, `mcp_e2e.py` 80/80, `agent_e2e.py` 61/61.

**Not verified here:** the musl/zig release build with the new dependencies
(no zig on this host). G3 built the same rmcp feature set as a static musl
binary with system gcc. The release job is the check.

**81e. Gateway slice 4: the gateway route — grants at launch, per-call
authorization, the built-in `storm` connection.** *(2026-10-05)*

**Grants (spec §6).**
- `POST /v1/agent/sessions` gains `allow_vault_writes`, off by default.
- At launch, `ops::launch_session` offers the built-in `storm` connection
  plus every non-disabled connection of the owner.
- The Agent Manager drops all of them for `shell` (G-D9), and for a host
  whose capabilities lack `mcp_bridge`. For that host, the launch answer
  carries `mcp.notice`: "<host> can't use integrations — update
  storm-runtime".
- The grants are written to `agent.db` before `start` goes out:
  - `session_mcp` holds the session's write flag;
  - `session_mcp_grants` holds one row per connection, with `revoked_at`
    for §13.
  - Both are new tables, created additively, and `sessions` is untouched.
- `start` gains `mcp: [{id, slug}]`. The field is omitted when empty, so a
  session without grants sends the same bytes as before.
- The launch answer is the session record flattened, plus `mcp:
  {connections, allow_vault_writes, notice}`.
- `integration_grant` is audited with ids only.
- An old `storm-runtime` ignores `mcp` in `start`, and logs and skips an
  unknown `mcp.message`, so nothing breaks. Its sessions just have no
  grants.

**`Actor::Agent { session_id, host_id, user_id, role }`** (AM31) carries the
owner's identity. `describe()` is `agent`. Every policy reads `user_id()`
and `role()`, so it needs no special case.

**The route, `POST /v1/runtime/sessions/{id}/mcp/{connection}`** (Host tier):
- The body is one JSON-RPC message from the bridge.
- The answer is `application/x-ndjson`: lines of `{"message": <JSON-RPC>}`,
  or the one bridge signal `{"storm_error": "session_unknown"}`.
- Every refusal is one JSON-RPC error with code `-32001`, the stable code as
  its `message`, and `data.storm_error`. Never an HTTP error once the host
  is authenticated.
- The codes are §12's (`not_granted`,
  `integration_needs_reauth:<slug>`, `upstream_unavailable`,
  `upstream_rate_limited`, `gateway_rate_limited`) and five additive ones:
  `not_your_session`, `session_not_live`, `owner_inactive`,
  `method_not_permitted`, `tool_not_allowed`, plus `response_too_large`
  for the 1 MiB cap.

**`ops::integration_call`** runs §7's checks in order: the host owns the
session; the session is `starting`/`running`; a live grant exists; the owner
is active; the connection is the owner's and `connected`; the method is in
§9's list (resources and prompts follow the owner's switches); the tool is
in the allowlist; and for the vault, writes need the launch flag **and**
`mcp_writable`, read per call.

**Upstream sessions (`gateway::session`).**
- One per (agent session, connection), opened by the agent's `initialize`
  and held in memory only. A restart loses them, so the next request is
  answered `session_unknown` without being forwarded (G-D19).
- The agent's capabilities go upstream minus `sampling`, `roots` and
  `elicitation.url`. URL-only means no elicitation at all.
- The agent's `initialize` answer is the upstream's, with `resources` and
  `prompts` hidden when the owner switched them off.
- **Request-scoped messages ride their call's response stream:**
  - Progress is rewritten to the agent's own token, and only sent if the
    agent asked for it.
  - A form elicitation is found by rmcp's
    `InboundStreamOrigin::OutboundRequest` and sent as `storm-elicit-<n>`.
    The agent answers it with a second POST to the same route.
  - When a call ends, its route and pending elicitations are dropped, so a
    late answer is never sent upstream and a stale message has nowhere to
    go.
  - A URL elicitation is declined and audited as `url_elicitation_declined`
    (G-D23).
- **Unsolicited messages** (`list_changed`, `resources/updated`) go down the
  link as `mcp.message` (at most once).
- **Closing is cancelling.** A disconnect, a disable, a credential rotation,
  a replaced `initialize` or a session that ended cancels the upstream
  session. A call in flight on it fails at once, once, and is never re-sent.
  A test holds a call in flight, closes the session, and counts one
  upstream execution.
- **Limits** (§9) refuse rather than queue: per session 4 concurrent calls
  and 10 per second; per connection 8 concurrent.
- Ended sessions' upstream sessions are swept whenever a host reports or is
  revoked.

**Requests are sent with `send_cancellable_request`, not the `*_once`
helpers.** This is an interpretation of AM26, not a change to it:
- The helpers are one-line wrappers over the same single send. The gateway
  must know the request's id and rmcp's progress token at send time, to
  route its stream, and the helpers hide both.
- AM26's purpose, at most one upstream execution per agent call, is
  unchanged.
- The source guard still forbids the re-sending helpers.

The operator should confirm this reading.

**The built-in `storm` connection** is `mcp.rs`'s own handler,
`Storm::for_agent`, served in process over a `tokio::io::duplex` pipe as
`Actor::Agent`, and reached through the same client path as any upstream:
- **`delete_note` is never offered** (`NEVER_FOR_AGENTS`).
- Write tools are listed and callable only under both flags.
- **It does not require `/mcp` to be enabled.** That switch governs the HTTP
  surface for `stk_` keys. The built-in is reached only by a session's own
  grant.

**Not here:**
- The owner is not notified when an agent's listing shows a new tool
  (§9). The tool stays off, which is the safety property, and the owner
  sees it as `new` on the next test. The client (81h) surfaces that.
- An upstream 404 re-initializing transparently is rmcp's
  (`reinit_on_expired_session`, proved by G3). It is re-checked against the
  real build in acceptance (81i).

**Verified.**
- Six route tests through the real router, with a real enrolled host and a
  real rmcp upstream:
  - the full flow: the host sees ids and slugs and no canary;
    `session_unknown` before `initialize` with zero upstream calls; the
    allowlist on listing and calling; the stripped capabilities upstream;
    the audit; a user credential gets `401`;
  - every §7 refusal, a second host's token included;
  - `shell` and an old host get nothing, and the notice is shown;
  - the built-in under each flag combination, never deleting;
  - progress, a form elicitation answered and a late answer dropped, a URL
    elicitation declined and audited;
  - an in-flight call failing once with no re-send.
- Manager tests: grants and `start`, `shell` and old hosts, `mcp.message`.
- Unit tests: capability stripping and the limits.
- **Mutation-proved, all 16 caught:**
  - each of the seven authorization checks removed;
  - `delete_note` offered;
  - `mcp_writable` ignored;
  - sampling forwarded;
  - URL elicitation forwarded or delivered;
  - the progress token not rewritten;
  - forwarding without a session;
  - close not cancelling;
  - disconnect leaving grants;
  - `shell` granted.
- fmt, clippy `-D warnings`, 508 unit tests.
- Live: `e2e.py` 81/81 unmodified, `mcp_e2e.py` 80/80, `agent_e2e.py` 61/61.

**81f. Gateway slice 5: the runtime — `storm-runtime mcp-bridge`, the daemon
socket, and the AM32 config writers.** *(2026-10-05)*

**The host forwards and never authorizes.** It learns ids and slugs from
`start.mcp`, and holds no credential, because none exists on the host.

**What `storm-runtime` does:**
- **`hello` and `inventory` report `mcp_bridge: true`.**
- **On a `start` with grants,** it writes the provider's config into
  `sessions/<ags>/` (`0700`, files `0600`, created with those modes). It
  mints a random 256-bit session handle and registers it against the
  session's slugs → connection ids. A config that cannot be written fails
  the start, rather than launching without `--strict-mcp-config` (G-D12).
- **The writers (AM32, exactly):**
  - **`claude-code`** gets `--mcp-config <session>/mcp.json
    --strict-mcp-config`, with one stdio server per slug:
    `storm-runtime mcp-bridge <slug>`, its env holding only
    `STORM_MCP_HANDLE` and `STORM_MCP_SOCK`.
  - **`opencode`** gets `XDG_CONFIG_HOME=<session>/xdg`, with the config in
    `xdg/opencode/opencode.json`, plus `OPENCODE_DISABLE_PROJECT_CONFIG=1`
    and `permission: {"<slug>_*": "ask"}` for every connection. That
    permission is set last, so no setting relaxes it.
  - **Anything else** (`shell`, `fake`, a custom id) gets nothing, and no
    directory is created.
- **`runtime.toml`'s provider entry gains `settings`** (G-D22): it is
  merged into OpenCode's session config, since the host's global OpenCode
  settings no longer load. Its `mcp` key is replaced, so a setting can never
  add an MCP server.
- **The provider contract stays MCP-free (AM20).** `SessionSpec` gains a
  generic `launch: LaunchExtras { args, env }`. A `cli` provider appends the
  args after its own and the env after everything else, so the session's
  values win over an env file.
- **The daemon socket** is `run/mcp.sock`: `0600`, in a `0700` directory,
  and the unit already allows `AF_UNIX`.
  - **A bridge connects per message:** one line `{handle, connection,
    message}` in, the server's NDJSON streamed back verbatim.
  - **While the link is down,** the answer is `{"storm_error":
    "storm_unreachable"}`. Never queued, never retried. A `401` asks the
    link to re-authenticate.
  - **An unknown handle** gets `not_granted`.
  - **A second kind of connection, `subscribe: true`,** stays open and
    carries the link's `mcp.message` (unsolicited messages) to that
    session's bridges.
- **Cleanup.** When a session ends or fails to start, its handle stops
  working, its subscribers go, and its directory is deleted. At boot every
  `sessions/` directory is removed, because they belonged to sessions that
  died with the previous process.

**The bridge (`src/bridge.rs`)** implements spec §11's six rules over a
`Daemon` trait (the socket, or a test's script):
- It keeps the agent's `initialize`.
- On `session_unknown`, it replays `initialize` under `storm-reinit-N`,
  swallows the answer, sends `notifications/initialized`, then resends the
  original request once.
- Any other failure, including a stream that ends without a response, is
  one JSON-RPC error and is never retried.
- When a call ends, every upstream request it opened toward the agent that
  is still unanswered is cancelled with `notifications/cancelled`.
- An answer to an unknown or closed elicitation is dropped.
- Stdout has one writer task, so lines never interleave. It logs nothing,
  because stdout is the MCP stream.

**Found on the way, and fixed on the server:** elicitation ids were
`storm-elicit-<n>` per upstream session, so after a restart the first new one
reused `storm-elicit-1`. A late answer to the old elicitation could then have
answered the new one. Ids are now `storm-elicit-<random>-<n>`, unique per
upstream session. `gateway_e2e.py`'s R8 check found it.

**`apps/server/tests/gateway_e2e.py`** (in `make test-live`, 50 checks, about
a minute) runs a real server, a real `storm-runtime`, a mock upstream and a
scripted agent:
- The mock upstream and the scripted agent (`tests/gateway_support/`) are
  ported from the gates. They are fixtures, not shipped code.
- The runtime launches the scripted agent as `claude-code` and as
  `opencode`, with exactly the config it would give the real CLIs. The agent
  starts the bridges from that config.
- It checks:
  - the owner-only routes, and `stk_` refused;
  - the AM32 launch for both providers;
  - calls, the allowlist and the built-in `storm` connection;
  - capabilities stripped upstream;
  - progress, form elicitation, and URL elicitation never reaching the
    agent;
  - **R2–R8 for real:** a server restart re-initializes with exactly one
    initialize result; an in-flight call during a `SIGKILL` fails once and
    is executed upstream once; an open elicitation is cancelled and a late
    answer never reaches upstream; an upstream restart (404) re-initializes
    transparently and runs once;
  - `shell` has no grants, and a disconnect refuses the next call;
  - **the credential boundary on this host:**
    - C1, C5, C7: every file under the suite's directory except the
      upstream's own and the server's state, which holds the credential
      only sealed;
    - C2, C4: every readable process's `environ` and `cmdline`;
    - the spellings raw, `Bearer`, base64 and base64url;
    - two positive controls: the canary file, and a planted process.

**Verified.**
- Runtime unit tests: both writers, shell and unknown providers, handles,
  and the bridge's R5, R7, R8 and timely-answer rules.
- **Mutation-proved in the unit tests, 7/7 caught:** `leak_init`, `retry`,
  `no_cancel`, a late answer forwarded, no `--strict-mcp-config`, no
  OpenCode `ask`, project config allowed.
- **The gates' three mutations against the real build**, each caught by
  `gateway_e2e.py` (R5, R7 and R8 failing respectively):
  - `leak_init`;
  - `retry`, which retries until the server is back and re-initializes;
  - `no_cancel`.

  Run by rebuilding `storm-runtime` with each mutation; the runner is not
  shipped.
- A naive retry without re-initializing is refused by the gateway itself
  (`session_unknown`), so it cannot double-execute. Only the gates' full
  mutation can.
- fmt and clippy `-D warnings` on both crates, and `cargo tree -i aws-lc-rs`
  finds nothing. Server 508 unit tests.
- Live: `e2e.py` 81/81 unmodified, `mcp_e2e.py` 80/80, `agent_e2e.py`
  61/61, `gateway_e2e.py` 50/50.

**Not here:** the real Claude Code and OpenCode against the real build is
on-device acceptance (G2 proved the configs with the gates' harness). C3, the
journal, stays the operator's.

**81g. Gateway slice 6: OAuth — Storm as the client, the browser back to the
app, and the tokens sealed.** *(2026-10-05)*

**rmcp's `auth` feature does the protocol.** That is
`AuthorizationManager`: RFC 9728 → 8414 discovery, RFC 7591 registration as
a public client (`token_endpoint_auth_method: none`), PKCE S256, RFC 8707
`resource`, the code exchange and the refresh. Storm owns every seam it
exposes. `cargo tree -i aws-lc-rs` still finds nothing.
- **`SsrfHttp`** (rmcp's `OAuthHttpClient`) is the only HTTP client it gets:
  - **https only**;
  - every URL and **every redirect hop** is resolved, and refused unless
    **all** its addresses are public (loopback, RFC 1918, link-local,
    CGNAT, ULA and v4-mapped are all refused);
  - then it connects to exactly those addresses (`resolve_to_addrs`), so a
    second DNS answer cannot change the target;
  - redirects are followed by hand, with `Authorization` dropped across
    origins.
- **`TokenStore`** (`CredentialStore`): the token set is sealed under kind
  `oauth_tokens`. rmcp saves a refreshed pair through it before returning
  the new access token, so a rotated refresh token is persisted before use.
- **`FlowStore`** (`StateStore`) over `oauth_flows`:
  - the `state` is stored as its blake3 hash, and the PKCE state sealed (AAD
    = hash, `pkce_verifier`);
  - ten minutes;
  - **loading claims**: one `UPDATE … WHERE used_at IS NULL`, so of two
    racing callbacks exactly one exchanges the code.
- **Discovery must be published.** rmcp's legacy fallback (synthesized
  `/authorize`, `/token`) is refused as `oauth_not_offered`.

**The routes (owner only, session tier):**
- **`POST …/connections/{id}/authorize`** `{redirect_uri, client_id?,
  client_secret?, scopes?}`:
  - **the redirect URI must be an `http` loopback or `storm://oauth`**
    (G-D13, G1), never a LAN or public URL;
  - the client is a pasted one (its secret sealed), else one already
    registered for this owner, issuer and redirect, else a new dynamic
    registration, else `oauth_client_required`;
  - the answer is the authorization URL.
- **`POST /v1/integrations/oauth/callback`** `{state, code, iss?}`:
  - another owner's state reads exactly like a spent one;
  - the code is exchanged, the connection becomes `connected`, live
    upstream sessions close so the next `initialize` uses the new tokens,
    and the integration is tested;
  - audited `integration_authorized`, or `_reauthorized` when it already
    had tokens.
- Every failure is a stable code (`oauth_discovery_failed`,
  `oauth_registration_failed`, `oauth_exchange_failed`,
  `oauth_flow_expired_or_used` …), never the upstream's text.

**Using the tokens:**
- `Gateway::target` is now async. For `oauth` it presents `Bearer <access>`,
  refreshing first when the token is within a minute of expiry.
- **Refresh is single flight**: one lock per connection is held across the
  refresh, and the token set is re-read under it.
- An upstream `401` at `initialize`, or on the owner's listing, gets one
  forced refresh and one retry. Neither executes anything an agent asked
  for, so this is not a retry of a call.
- A refresh the authorization server **rejects** makes the connection
  `needs_reauth` and records `integration_refresh_failed`. A transient
  failure is `upstream_unavailable`.

**A new `oauth` connection starts `pending_auth`** and stays so until it is
authorized; a failed test does not move it to `needs_reauth`.
**Disconnecting** reads what RFC 7009 revocation needs before the
ciphertexts go, then POSTs it to the advertised `revocation_endpoint`, best
effort, under the SSRF rule, and nothing waits on it.

**Additive schema:** `connection_oauth` records which client a connection
was authorized with, so a refresh uses it. The `oauth_clients` row (81b) now
also holds the discovered metadata, which is where the revocation endpoint
comes from.

**Not the spec's literal shape, by design (recorded, not a contradiction):**
- A loopback redirect with a new port per desktop launch registers a new
  dynamic client per port. That is harmless, and it is what Notion's
  post-login redirect check will need.
- The `resource` sent on a refresh comes from re-running discovery, because
  rmcp keeps the discovered resource private. A refresh therefore costs a
  discovery round trip, which is rare.

**Verified.**
- A mock authorization server and protected MCP resource (axum, in process):
  - the full flow: `pending_auth`; a LAN redirect refused; S256, `resource`
    and a registered public client in the URL; the client reused; the state
    never stored raw; the exchange carries the verifier and `resource`;
    `connected` with tools; replay and expiry refused; no token in
    `gateway.db`, its WAL or `security_events`;
  - refresh: two concurrent callers refresh once; the rotated refresh
    token is what is stored; a rejected refresh means `needs_reauth` and
    `integration_refresh_failed`; re-authorizing restores it as
    `_reauthorized`; disconnect revokes the latest refresh token;
  - racing callbacks exchange once, and another owner's state is
    refused;
  - discovery against an `https://127.0.0.1` integration never connects;
    a member gets `403`; a static integration is refused.
- Unit tests for the address rules, `check_target`, the client refusing
  before connecting, and redirect URIs.
- **Mutation-proved, 7/7 caught:**
  - the SSRF check off
  - the claim not single use
  - the refresh lock removed
  - LAN redirects allowed
  - the flow-owner check removed
  - no revocation
  - a rejected refresh treated as transient
- **Real discovery** (an ignored network test, run here): Notion, Linear
  and GitHub's remote MCP resolve through `SsrfHttp`. Notion and Linear
  offer registration; GitHub does not (G-D24).
- fmt, clippy `-D warnings`, 516 unit tests. Live: `e2e.py` 81/81
  unmodified, `mcp_e2e.py` 80/80, `agent_e2e.py` 61/61, `gateway_e2e.py`
  50/50.

**Not verified:** a real login, token exchange, refresh rotation and
revocation at Notion or Linear. G1's logins were deferred by the operator
into acceptance, and they stay there.

**81h. Gateway slice 7: the client — Settings ▸ Integrations, the launcher
toggle, and a loopback sign-in.** *(2026-10-07)*

**Settings ▸ Integrations** (`lib/agent/integrations_screen.dart`, route
`/settings/integrations`):
- **Entry and access.** The entry appears in Server settings beside Hosts,
  under the same owner check. The screen itself shows the server's `403`
  text, never an empty list (the 77d rule).
- **The list.** The built-in `Storm vaults` comes first, with whether
  writes are possible at all. Each integration shows its status in plain
  words (`Needs reconnecting`, …) and its last error from the §12 codes;
  never upstream text.
- **Actions:**
  - Test.
  - Choose tools: new tools are marked and start off (G-D16).
  - Sign in again, or Replace token.
  - Enable or Disable.
  - Disconnect. For a token, the confirmation says Storm cannot revoke it;
    for OAuth, that Storm asks the service to.
- **Add.** Sign in (OAuth), Token, or None, over https only.
  - A token for `Authorization` is sent as `Bearer <token>` unless it
    already carries a scheme. It is sent once and never shown again.
  - The GitHub PAT advice from risk 6 is shown.
  - On the web there is no Sign in (G-D13); the dialog says so.
- **The intro states the blast radius** (spec risk 1): every non-shell
  session can use these, with its host's network.

**The launcher (G-D5, G-D9, AM27):**
- For every provider but `shell`, an **"Allow vault writes" switch, off by
  default**, sends `allow_vault_writes`.
- A second line under the unchanged "Network: inherits <host>'s policy"
  says agents can use the integrations and read the vaults with that
  network access.
- A launch answer's `mcp.notice` (an old host) is shown as a snackbar.

**The sign-in uses a loopback listener on every native platform**
(`oauth_flow_io.dart`; the web build gets `oauth_flow_web.dart`, which
refuses):
- It binds `127.0.0.1` on an OS-chosen port, **listening before** the
  browser opens, asks the server to authorize with
  `http://127.0.0.1:<port>/oauth/callback`, and opens the system browser.
- It answers only that path with a `state` or `error`. A stray request (a
  favicon) gets `404` and does not end the sign-in.
- It relays `{state, code, iss}` to `/v1/integrations/oauth/callback` and
  closes. It never sees a token.
- macOS's sandbox needs `com.apple.security.network.server` in
  `Release.entitlements` (`DebugProfile` already had it). The entitlements
  test guards it.

**A deviation from §10 step 4, recorded here and in the report.** The spec
allows `storm://oauth` on Android and macOS, and that is **not built**:
- It needs an intent filter, a URL type and native glue, and only a release
  build on each platform can prove them. CLAUDE.md's Android-plugin rule is
  the warning.
- Loopback is what G1 found Notion and Linear accept, and Linear on any
  port, so one plugin-free path serves macOS, Linux and Android.
- **Risk on Android:** the app is backgrounded while the browser is open,
  and the redirect must reach it while it still runs. That is the device
  pass's first check. If it fails there, `storm://oauth` is the fix the
  spec already allows.

**The test found a bug.** The first version started listening only after
the browser call returned, so a redirect that arrived first waited forever.
Now the listener runs before the browser opens.

**Verified — locally, with Flutter 3.44.8 (CI's version), installed for this
slice:**
- `dart format`, `flutter analyze` (no issues) and the full `flutter test`:
  761 tests, all passing.
- `integrations_test.dart` (10 tests):
  - the list and its statuses, with no menu on the built-in;
  - a member's `403`;
  - a token sent once as `Bearer` and then tested, never on screen after;
  - `http` refused before any request;
  - the web is token-only;
  - new tools off, and the allowlist PATCH;
  - the disconnect wording and its DELETE;
  - the launcher toggle off by default and sent when on;
  - `shell` with no toggle and no integrations line;
  - the old-host notice parsed from the launch answer.
- `oauth_flow_test.dart` (2 tests): the redirect is caught on loopback (a
  stray request ignored) and relayed, and the port is closed afterwards; a
  cancelled sign-in relays nothing.
- **Mutation-proved, 6/6 caught:** writes on by default, the toggle shown
  for `shell`, listening after opening, a stray request ending the sign-in,
  `http` accepted, the `network.server` entitlement removed.

**Not verified:** a real browser sign-in on any device, and the Android
background case above. Both are on-device acceptance.

**81i. Gateway slice 8: acceptance — what can be automated is, and the rest
is the operator's.** *(2026-10-07)*

**`tests/gateway_e2e.py` (81f) grows to 58 checks.** It runs in `make
test-live` and CI against a real server, a real `storm-runtime`, a mock
upstream and the scripted agent. This slice adds **Phase 1's exit (AM28)**:
- the owner turns MCP writes on;
- a session launched with "Allow vault writes" sees `create_note` and
  `update_note`, and never `delete_note`;
- the agent creates a note, a "phone" (REST) edits it, and the agent's
  stale `update_note` comes back **merged**, with both edits in the note;
- a `delete_note` is `tool_not_allowed`;
- switching MCP writes off refuses the same live session's next write.

**The gates' three mutations, as negative tests:**
- `make test-gateway-mutations` (`tests/gateway_mutations.py`) rebuilds
  `storm-runtime` with the bridge broken three ways (`leak_init`, `retry`
  and `no_cancel`, the gates' own), runs the suite against each, and
  requires the matching R5, R7 or R8 check to fail. It always restores the
  source.
- Not in `make test-live`: three rebuilds take minutes.
- Run here: **3 of 3 caught**.

**The spec's §17 acceptance list, item by item:**

| §17 item | Where it is proved |
|---|---|
| Connect, authorize, launch, use | `gateway_e2e.py` (static); route tests (OAuth, 81g) |
| An agent write merges against a phone edit | `gateway_e2e.py`, this slice |
| Disconnect mid-session refuses the next call | `gateway_e2e.py` |
| R1–R8 against the real build, with the 3 mutations | `gateway_e2e.py` + `make test-gateway-mutations` |
| No token on the host: files, environment, argv, bridge config, scrollback | `gateway_e2e.py` (C1, C2, C4, C5, C7, positive controls) |
| OpenCode asks, and loads only the session's servers | config proved in `gateway_e2e.py`; the real prompt is the operator's |
| No `elicitation.url` upstream, and URL mode never reaches the agent | `gateway_e2e.py` |
| An old host's launch announces the degrade | route test (81e), client test (81h) |
| A member gets 403 everywhere | route tests; `gateway_e2e.py` |
| An `stk_` key is refused on the integration routes | route test; `gateway_e2e.py` |
| `e2e.py` 81/81 unmodified | every slice |

**Left for the operator (on-device acceptance):**
- **C3**: passed 2026-10-08 (the grep printed 1: the planted control line
  and no credential). The window held the real build's local test runs,
  but those servers log to files, not the journal, so the grep is repeated
  once the gateway runs under systemd after a deploy.
- **G1's real logins:** Notion and Linear sign-in, exchange, refresh
  rotation and revocation, through the app's loopback sign-in (81h), on
  macOS, Linux and **Android**. Android is the case to watch, because the
  app is backgrounded while the browser is open.
- **A GitHub PAT** connection making a real tool call.
- **Real Claude Code, then real OpenCode** (AC-P1):
  - a gateway tool prompts before it runs;
  - only the session's servers load;
  - an agent writes a note while a phone edits it.
- **Releases.** Neither the musl/zig release build nor the native app
  builds have been run with these changes.

**Status 2026-10-08, on the staging environment** (dew-omarchy, build
`dcb0a80`, signed acceptance APK `acfa4d9`):
- **Real Claude Code 2.1.292: passed.**
  - It launches with `--mcp-config <session>/mcp.json --strict-mcp-config`.
  - `/mcp` shows only the session's servers, none of the host's own.
  - It asks before every gateway tool.
  - An agent's write against a phone edit kept both edits. The edit
    overlapped, so it came back `conflict` with markers.
- **Real OpenCode: passed.**
  - Its config matches AM32 exactly.
  - It used Notion through the gateway and got real data.
  - Its live prompt was blocked by its model quota at first, then exercised
    on the phone.
- **Android native OAuth (G1's real logins): passed** for Notion and
  Linear.
  - Both came back to the app through `storm://oauth/callback` (81l).
  - Disconnecting Notion refused the next call with `not_granted`, and
    Linear kept working.
- **Also passed:**
  - a real TLS upstream (DeepWiki), and a disconnect mid-session;
  - the released v0.3.1 host's launch announcing it can't use integrations;
  - a member getting 403 and an `stk_` key 401 on every integration route;
  - no canary in any process, file or scrollback, with a live session.
- **Found and fixed on the way:**
  - the per-session limit against Claude Code's startup (AM-G11, 81m);
  - releases never upload-key signed (81m);
  - the web Add-integration UI (81n).
- **Left:**
  - macOS native OAuth;
  - a GitHub PAT tool call;
  - the journal grep against the real build (`journalctl -t
    storm-staging-server`, expect 1, the control line);
  - the release builds and the release.

**Verified.** `gateway_e2e.py` 58/58 here; the mutation runner 3/3 caught.

**81j. A simplify pass over the gateway stack: no behaviour changes, and
five findings left open on purpose.** *(2026-10-07)*

A reuse, simplification, efficiency and altitude review of 81b–81i. **What
changed, with behaviour held fixed:**

*One copy of each rule:*
- **"May this agent call this tool"** is `Authorized::may_call`. It is used
  by both the `tools/call` gate and the `tools/list` filter, so neither can
  drift from the other.
- **The `integration_needs_reauth:<slug>` code** is spelled in one place
  (`needs_reauth_code`). Every upstream failure maps to the agent's code
  through `upstream_failure`.
- **The OAuth refresh-once rule** is `Gateway::with_target`, used by the
  owner's probe and by an agent's `initialize`. `target()` never fails with
  `Unauthorized`, so this matches both old copies exactly.
- **OAuth manager setup** is `Gateway::configure_client`, with
  `FlowStore::new` and `TokenStore::new`. A connection's OAuth client is
  `oauth_client_of`.
- **The SSRF-pinned reqwest client** is `pinned_client`.
- **Key files** use one set of helpers in `auth::identity`
  (`write_key_file`, `create_private_dir`, `warn_if_readable_by_others`,
  `copy_key_dir`). `backup_auth` and the gateway's backup share the key copy.
- **Owner gates** share a single predicate (`owner_only`).
- **JSON through the keyring** is `Keyring::seal_json` / `open_json`.
- **The REST bodies** are the `ops` input structs, as `agent_launch` already
  did.
- **The runtime's authenticated POST** is one path, `post_for_response`, so
  the 401 → reauth rule exists once.
- **In the client:** one `sessionCredentialsProvider`; the tile status is a
  `StatusChip`; the token dialog returns only the credential; two unread
  fields are removed.

*Cheaper paths:*
- **A result's size** is counted, not built as a string.
- **An agent's params** are moved, not deep-copied.
- **The TLS config** is built once.
- **Call-audit pruning** walks the primary key instead of building a
  100,000-row `NOT IN` set, which it did under the gateway's lock.
- **Listing integrations** costs one `EXISTS` per row.
- **The bridge** takes each message out of its line instead of cloning it.
- **The host forwards a bridge's message as raw JSON**
  (`serde_json/raw_value`).

*Leaks and dead code:*
- **An upstream session's event hook** now holds the agent manager and a
  `Weak` gateway. It used to capture `AppState` whole, a reference cycle
  through `gateway.sessions`.
- **Removed:** the `Storm.for_agent` flag (now read off the actor), the
  never-written `oauth_flows.scope` column (the schema is unreleased), and
  `ops::integration_call`'s `Result`, which never returned an error.

**Left open: each changes behaviour or is a redesign.**
1. **Session cleanup is polled.** `sweep_gateway_sessions` runs after a host
   reports, not when a session ends, and `end_session` doesn't call it. The
   deeper fix is for the agent manager to publish a session's end.
2. **The connection status rules differ by path.** `note_upstream_failure`
   marks `needs_reauth` only from `connected`; the probe does so from
   anything but `pending_auth`; `status::ERROR` is read and never set. The
   fix is a `Status` enum with transitions, and it changes which
   transitions happen.
3. **No shared test vectors.** The gateway's NDJSON line format and error
   shape are re-derived in both crates, and the messages differ (`<code>` vs
   `storm: <code>`). The fix is a `docs/gateway-vectors.json`.
4. **AM26 is enforced by grepping source.** A wrapper type that hides rmcp's
   `Peer` would make the type system enforce it.
5. **A spent OAuth flow is recognised by rmcp's error text.**
   `FlowStore::load` could record the outcome instead.

**Also not done:**
- Refresh still rediscovers OAuth metadata. Reusing the stored copy would
  refresh against stale endpoints after an upstream moves them.
- The rate limiter and the elicitation route wait are as they were.
- `gateway_e2e.py` still copies `agent_e2e.py`'s helpers; a shared harness
  means rewriting that older suite too.

**Verified.**
- `make check`: clippy `-D warnings` and `flutter analyze` clean; the server's
  516 unit tests and the client's 761 pass.
- `make test-live`: `e2e.py` 81/81 **unmodified**, `mcp_e2e.py` 80/80,
  `agent_e2e.py` 61/61, `gateway_e2e.py` 58/58, `auth_e2e.py` 74/74, and the
  client integration suite.
- `make test-gateway-mutations`: 3 of 3 caught after the bridge changes.


**81k. New upstream tools: recorded, off, and announced to the owner (spec
§9, built to the frozen spec).** *(2026-10-08)*

Spec §9 says tools added after the allowlist was set default off, **"and
the owner is notified."** 81d–81i built the first half: the allowlist names
tools explicitly, so a new tool was already off. The notice existed only in
the owner's next manual test, and only once, because that test marked the
tool known. The operator chose to build the spec as frozen, without an
amendment (2026-10-08).

- **What records a new tool:**
  - *Any listing.* The owner's test, the owner's tool listing, and **an
    agent's `tools/list`** all record names not in `known_tools` into a
    new `new_tools` table. It's a table, not a column, so an observation
    is an `INSERT OR IGNORE` that no rewrite of the connection row can
    clobber (the `note_access` precedent).
  - *Only the owner's first test is a baseline* (G-D16, unchanged). An
    agent's listing never is, so nothing an agent does can enable a tool.
  - *Only valid names* are recorded (`valid_tool_name`: the allowlist's
    rule), and at most 200 stay pending per connection, because the
    upstream controls both.
- **What the owner sees:** "N new tools — review" on the integration's row,
  opening the tool chooser with those marked *(new)*.
  - **Saving the tool list is the review:** the pending tools become known,
    on or off exactly as the list says. Nothing else changes the allowlist.
  - The client shows upstream names through `displayToolName`: control,
    format (bidi, zero-width) and line-separator characters removed,
    whitespace collapsed, at most 64 characters. Display only; the allowlist
    keeps the raw name.
- **Not done, by the operator's decision:** push or email notification. That
  would be an amendment, proposed separately.
- **`known_tools` has its own writers** (`observe_tools`, `review_tools`), and
  `update_connection` no longer writes it, so a stale copy of the row cannot
  undo a review.

**Verified:**
- Store tests:
  - the first-test baseline;
  - an agent's listing that never baselines;
  - a review that enables nothing;
  - an older row copy that can't undo a review;
  - invalid names, and the 200 cap;
  - a disconnect forgetting pending tools.
- `gateway_e2e.py` 65/65, adding seven checks for a tool the mock upstream
  adds mid-session:
  - hidden from the agent;
  - recorded, valid name only;
  - refused if called;
  - cleared by a review while staying off;
  - enabled only by the owner.
- Client tests: the review button (plural and singular), a review that sends
  no new tool, and name cleaning.


**81l. `storm://oauth` on Android and macOS, loopback on Linux and Windows
(spec §10.4, built to the frozen spec).** *(2026-10-08)*

81h signed in through loopback on every native platform and left
`storm://oauth` for later. That contradicted the frozen spec §10.4: a
loopback listener on desktop, and a newly registered `storm://oauth` on
Android and macOS. The operator chose to build the spec, not amend it
(2026-10-08).

**Why the split matters:**
- On Android the listener lives inside an app that is backgrounded during
  the login, so an app frozen or killed meanwhile loses the sign-in.
- A loopback port changes on every sign-in, so each one registered a new
  OAuth client upstream.
- A scheme hands the redirect to the app and brings it to the front. One
  fixed redirect URI reuses one client per authorization server.

**What it is:**
- **The redirect:** `storm://oauth/callback` (`appRedirect`) on Android and
  macOS (`usesAppScheme`); the 81h loopback listener, unchanged, on Linux and
  Windows. The server's `validate_redirect` already accepted both.
- **Native side:** no third-party plugin, by the Kotlin-plugin rule.
  - Android: a `VIEW`/`BROWSABLE` intent filter for `storm://oauth` only, with
    `flutter_deeplinking_enabled=false` so the link never becomes a router
    page.
  - macOS: `CFBundleURLTypes` for `storm`, and `application(_:open:)`.
  - Both buffer links until Dart calls `takeLinks` on the `storm/links`
    channel, then forward them as `link` calls, so a link that launched the
    app is not lost.
- **Dart (`OAuthLinks`):**
  - A redirect goes to the sign-in waiting for its `state`, which the client
    reads from the authorization URL.
  - Anything but `storm://oauth` with a `state` is ignored, because the
    scheme is public.
  - **A link no sign-in in this process is waiting for is an orphan**: the
    app root opens Integrations, which relays it. The flow, PKCE and all,
    lives on the server, so any instance can finish it.
- **Android's launch mode is unchanged** (`singleTop`, `taskAffinity=""`).
  Whether a browser's link reaches the running instance or a fresh one is
  for on-device acceptance to show. Both paths are handled: the fresh
  instance relays the orphan, and a sign-in still waiting learns on resume
  that its integration became connected, instead of waiting out the deadline.
- **macOS Release drops `network.server`**: the release app listens on no
  port now. The debug build keeps the template's grant.
- **Native builds:** `.github/workflows/acceptance.yml` builds the APK
  (debug-signed, no secrets) and the macOS app on a push to `acceptance/*`.
  It never releases. It's the first place this Kotlin and Swift compile:
  PR CI does not build native projects.

**Verified:**
- Dart tests:
  - the scheme redirect is matched by `state` and relayed;
  - another sign-in's redirect and a non-oauth `storm://` link don't finish
    it;
  - a cancel relays nothing;
  - a sign-in finished by another instance completes on resume;
  - links buffered before Dart listens are taken at start;
  - the Integrations screen relays an orphan.
- Registration guards for the manifest, Info.plist, `MainActivity` and the
  macOS delegate.
- The Release entitlements no longer grant `network.server`.

**Not verified here:** the Kotlin and Swift compile only on the acceptance
workflow, and their behaviour on a device is on-device acceptance.


**81m. An agent's listings don't count against its session budget
(amendment AM-G11, approved by the operator).** *(2026-10-08)*

**Found in acceptance on staging.**
- Real Claude Code 2.1.292 lists every server's tools, prompts and
  resources in parallel as it starts.
- The per-session limit (§9's operational defaults: 4 concurrent, 10/s,
  shared by all of a session's connections) refuses rather than waits.
- With the built-in connection and two integrations, one integration's
  `tools/list` and `prompts/list` came back `gateway_rate_limited`, and
  Claude Code showed "tools fetch failed": that integration had no tools for
  the whole session. With the operator's real setup (Notion, Linear, GitHub
  and `storm`) it would happen at nearly every launch.

**Decided:**
- **Option A, recorded in the frozen spec as AM-G11**, not edited silently:
  listings (`tools/list`, `prompts/list`, `resources/list`,
  `resources/templates/list`) take only their connection's slot (8
  concurrent).
- The session budget stays on execution: `tools/call`, `resources/read`,
  subscriptions, `prompts/get` and `completion/complete`.
- §12 is unchanged: a limit that is hit is refused, never queued.

**Rejected:**
- *Raising the numbers:* that still breaks as integrations are added, and
  loosens the bound on real calls.
- *Queueing:* it contradicts §12 and adds latency to every call.

**Verified:**
- Unit tests:
  - Claude Code's startup pattern (3 connections × 3 listings) succeeds
    while the session budget is spent, and takes nothing from it;
  - listings are still capped per connection;
  - exactly the four listing methods are exempt.
- **On staging:** the same real Claude Code startup that failed made 10
  gateway calls with 0 refused. `/mcp` shows all three servers with tools
  (canary 3, deepwiki 3, storm 11).

**Also found while preparing the Android acceptance build: no release has
ever been signed with an upload key.** `release.yml` reads
`STORM_UPLOAD_STORE_BASE64` and `STORM_UPLOAD_*`; none of those secrets
exist, and the v0.3.1 run logged "APK will be debug-signed". So every
released APK is signed by a throwaway key from its own CI run: no release
installs as an update over another, and no new build can update an
installed release.
- `acceptance.yml` now takes the same secrets the same way, when they
  exist, and stamps the latest release's run number as the build number, so
  a correctly signed acceptance APK installs as an update.
- Creating the upload keystore is the operator's; it is Storm's Android
  identity from then on.
- **Created 2026-10-08.** The operator ran `storm-signing-setup.sh` on
  dew-omarchy:
  - The key: PKCS12, RSA 4096, alias `storm`, subject `CN=Storm Android
    upload key`, SHA-256 `C0:90:38:74:…:5C:12:AB:4E`.
  - It lives in the operator's home, which the `storm-runtime` account (every
    agent on the host) cannot read. It is backed up off the machine.
  - The four `STORM_UPLOAD_*` secrets are set. The acceptance APK `acfa4d9`
    is verified signed with it, its v2 block read directly.
  - **v0.3.1 and earlier were signed with throwaway keys**, so the first
    install of an upload-key-signed build needs one uninstall. Every build
    after it updates in place.


**81n. The Add-integration dialog says how Storm connects; the web shows
sign-in as unavailable (UI amendment, approved by the operator).**
*(2026-10-08)*

Found in acceptance: on the web, the dialog offered only "Token | None",
and the one sentence explaining why sat at the bottom, under the GitHub
text. That is the spec's behaviour, not a bug: G-D13 and §10.7 make the web
token-only, because a browser can't receive a redirect (a LAN `http`
redirect is refused upstream, G1). But it read like a missing feature. Two
real defects came with it:
- On the web, an OAuth integration's **Reconnect** and **Sign in again**
  could only fail.
- The failure was reported as "Could not reach the server".

**Client only; the architecture and the spec are unchanged:**
- **A labelled section, "How Storm connects",** directly under the address:
  - the selector stretched to the fields' width;
  - one help line for the chosen option;
  - on the web, the explanation directly under it.
- **The same three choices everywhere: Sign in · Token · No sign-in**
  ("None" renamed).
  - On the web, Sign in is disabled and visibly so: greyed by the disabled
    state, a lock icon, and the tooltip "Available in the Storm apps".
  - No check mark, and single-line compact labels, so nothing wraps at phone
    width.
- **On the web, an OAuth integration needing reconnection** says "Reconnect
  from a Storm app" on its own line, and its "Sign in again from a Storm
  app" menu item is disabled. An `UnsupportedError` reports its own message,
  never "Could not reach the server".
- **Not done, by the operator's decision:** collapsing the Header field.

**Verified:**
- Widget tests:
  - the web's disabled Sign in, the token default and the adjacent note;
  - the help line per option;
  - the web OAuth row: no reconnect action, the menu item disabled, no
    `/authorize` sent.
- 773 client tests pass. The UI was reviewed as rendered screenshots before
  merging.

**82. Storm v2: the approved design is built as specified, Storm becomes
single-user, and the knowledge ↔ agent loop gets real backend support.**
*(2026-10-08; design approved by the operator; slices 0–9 built, awaiting
merge into `staging`)*

**The source of truth** is `design_handoff_storm_v2/` (README, the two
`.dc.html` prototype files, the screenshots); where its README and the
prototype disagree, the prototype wins. `docs/design/STORM_UI_UX_DISCOVERY.md`
is the discovery behind it. **The plan** — audit, design-to-code map,
resolved questions, slice order, tests — is
`docs/design/STORM_V2_IMPLEMENTATION_PLAN.md`; this entry records what is
decided and points there for the rest.

**Product shape:**
- Activities, not vaults: a desktop **activity rail** (Notes, Agents, a
  health dot, Settings) and a phone **place picker** in the left corner
  bubble. **No Home or dashboard**; launch restores the last activity and
  location per device. Settings is one global destination with a fixed
  hierarchy (This device · Devices & access · Storm › Vaults, AI access,
  Integrations, Hosts & default agent, Storage, Connection, Advanced ·
  About & health). Infrastructure appears only as status lines that link
  into Settings.
- **Agent ≠ Session.** In the UI "Agent" is Claude Code / OpenCode / Shell
  (code keeps `provider`); a session is one run, now a route
  (`/agents/s/:id`); Work groups sessions by (workspace, host). Kit's
  `agents/*.md` are plain notes and are never called agents.
- **The loop:** start a session from a note (launch context), choose **one
  vault it may write to**, see what it wrote (**Wrote**), and see on each
  written note which session edited it (**provenance**, an unseen dot).

**Storm is single-user** — not hidden, removed. One account, password-only
sign-in; no members, roles, registration or account picker. On upgrade the
oldest active owner becomes the account and every other account is deleted
with an audit row; `auth.db` is snapshotted to `auth.db.pre-v6` first, the
v6 migration runs in one transaction, and invariants I1–I8 (plan §3.3) are
checked on every boot. Device, session, key, host and gateway boundaries are
unchanged.

**Engineering decisions recorded with their reasons in plan §5:** the route
scheme and redirects; `/` redirects to the last location and back from
Agents/Settings returns to Notes (the desk-width forward and its stale read,
decision 78, go away); no pill on the phone note; vault colour from the tile
in Settings › Vaults; running drawn in `accent`; two derived type steps
(`fs·scale²`, `fs/√scale`); server-assigned session names; latest agent
writer only on a note; tabs and terminal-title names retired; a separate
`agent_writes` setting seeded from `mcp_writable` so no upgrade changes
behaviour.

**Context delivery:** the agent reads its note through the built-in `storm`
connection (`session_context`, keyed on the caller's own session); a fixed
opening prompt tells it to read the note and wait. **No note content, title
or id ever enters a command line or environment** — the server sends a
boolean, the runtime adds a compile-time constant (plan §7.2).

**Amends, as the slices land** (until then those entries describe the
code): 78 (dashboard band, space switch, owner-only Agents), 77d / 80 / 81c
(owner-only → the one account), and the M19 invariants in `CLAUDE.md` that
speak of owners and members.

**Slice 0 (done):** Flutter 3.44.8 user-local; baseline on `c9131b4` all
green — Rust 546 + 112 + 53 tests, clippy and fmt clean, 773 client tests,
`flutter analyze` clean, `make test-live` 361 checks + 20 client live
tests; `docs/design/acceptance/storm-v2/` drives the real web build against
a real server in headless Chromium and signs in through the UI.

**Slice 1a (single user, server; `feat/single-user-server`):** built.
`auth/account.rs` replaces `auth/users.rs`; `auth/single_user.rs` is the v6
migration with its boot-time invariant check and `storm-server single-user
--keep`; `Role`, every owner check, registration, `GET|POST /v1/users` and the
`user` CLI are gone (`passwd` sets up or resets the account); login and setup
take `{password}`, a v0.3.x `username` is ignored; `ops::reconcile_single_user`
sweeps `agent.db`/`gateway.db` on every boot. A v0.3.x login screen that finds
no `/v1/users` falls back to a username field, so no compatibility route was
kept. Tests: the plan's 15 migration tests plus the sweep, mutation-checked
three ways; `cli_account.rs` replaces `cli_users.rs`; `make test-migration`
builds the pre-v6 binary to make a real v5 database. In the live suites,
member checks became access-key checks, and `auth_e2e`'s throttle flood now
runs before setup (no account, so a 429 can only be the limiter).

**Slice 1b (single user, client; `feat/single-user-client`):** the sign-in
and pairing screens take only a password and choose between setup and sign-in
from `GET /v1/account`; signup, the account picker, the registration switch,
`AuthUser`, `agentAccessProvider` and every owner gate are gone. 767 client
tests pass; the Dart live auth test runs password-only against the new server.

**Slice 2 (design-system additions; `feat/v2-design-system`):** two derived
type steps on `StormTokens` (`titleSize = fs·scale²`, `uiSize = fs/√scale`; no
new inputs); `controls.dart` — `StormButton` (primary, outline, soft, danger,
text), `StormToggle` (replaces `StormSwitch`), `ChoiceChips`, `SettingsRow`,
`NumberedSteps` (also on `EmptyState`); `session_status.dart` — the §3.2
status drawing (`SessionStatusDot`, `SessionStatusChip`, `StatusPill`); the
shared `SidebarFrame` (`surface`, right hairline), which the Notes and Agents
sidebars now use; `placePopover`, with a `right` side for the rail. All in
`lib/ui/`, so the conformance scan covers them. Gallery shot
`docs/design/acceptance/storm-v2/design-system/gallery.png` (three presets);
784 client tests.

**Slice 3 (shell + routing; `feat/v2-shell-routing`):** `AppShell` wraps every
signed-in route: the activity rail at ≥900 (Notes, Agents with a live-session
badge, the health dot and its popover, Settings), the phone's places bubble
(place picker) and settings gear below it. `/` is a redirect to the device's
last location (`NavMemory`, prefs `storm.nav.*`, read once by
`SettingsNotifier` and owned by `navMemoryProvider` after); `/notes` opens the
last vault or the no-vaults state; retired routes redirect (`/settings/server`,
`/v/:v/settings/*`, `/settings/mcp-keys`, `/agents/hosts`). System back follows
Q2 through a `BackButtonListener`: the router pops first, then
`logicalParent` (note → folder → parent; Agents/Settings → last Notes
location); a vault root exits. The dashboard, the space switch, the agents
band and the "A" appearance popover are gone; until slice 7 the settings
pages mount the old screens in the new shell. Found on the way: each nested
navigator's modal barrier carries `BlockSemantics`, which hid the rail and the
sidebars from screen readers; `PaneSemantics` puts each pane in its own
semantics container. 804 client tests; the back contract and health rows
mutation-checked; desktop-04 and phone-02 captured. Gap until slice 4:
cross-vault recents (they lived on the dashboard).

**Slice 4 (Notes; `feat/v2-notes`):** the desktop Notes sidebar is rebuilt on
`SidebarFrame`: `VaultHeader` (tile, name, sync line) opening
`VaultSwitcherPopover` (vaults with counts and ✓, "Synced … · Sync now",
Manage vaults ›; no Server settings), "Search {vault} ⌘K" (Ctrl K off macOS),
**RECENT** — the four newest cross-vault rows from `recentsProvider`, each with
a `VaultTag`, which closes the slice 3 gap — then the **FOLDERS** tree
(`SidebarRow`s, drawn twisties, no counts), and a footer of New note + folder +
tags (gear and mentions left it). A long-press on a vault in the switcher or
the place picker opens `AccentPicker` (Q7). The note header is crumb (phone:
"‹ parent"), the restyled Read | Edit, a soft **Start session** (phone
"Session") that opens the existing launcher while a host is online and Agents
otherwise, and the drawer toggle; then the file-name title at `displaySize`
(omitted when the body opens with its own `# heading`) and `VersionLine`
(`v51 · Saved`, the id when asked for, a `provenance` slot for slice 8). The
drawer is open by default with its own ×. The phone vault root is the vault
name, RECENT ("vault · folder" and an age) and FOLDERS; a folder is "‹ parent"
over its name; the pill is Directory / Search / ＋ / Tags (a long-press on ＋
makes a folder) and is absent on the note (Q5); properties keep the sheet
(Q6). `SectionLabel` is mono, as §7.2 has it. Unseen hooks only: `FolderTree`
and `VaultSidebar` take `unseen(noteId)`, `EntryTile` an `unseen` flag. 827
client tests; recents limit, recent selection and the note pill
mutation-checked; desktop-01..03 and phone-01/03 captured (harness: shots
reload on a viewport switch, which sometimes stalled Chromium).

**Slice 5 (server agent capabilities; `feat/v2-agent-capabilities`):**
`agent.db` gains `session_launch` (name, context note ids and title snapshot,
write vault) and `session_writes` (one row per note or kit script a session
wrote: `created` stays `created`, version and time follow its latest write),
both additive and
kept when a session is dismissed, so provenance still names it. `POST
/v1/agent/sessions` takes `context {vault_id, note_id}` (read through the vault
seam, 404 if gone) and `write_vault_id` (must exist); `allow_vault_writes`
without a vault launches read only with a notice. The server names a session:
the slug of its note's title, else `{workspace}-{n}`, a live duplicate `-2`.
Session views add `name`, `context`, `write_vault_id`, `wrote_count`; new
`GET /v1/agent/sessions/{id}/writes` and `GET /v1/vaults/{v}/agent-writes`;
`GET …/notes/{id}` adds `agent_write` (the latest agent writer, by version,
`session_dismissed` once gone); `GET /v1/config` adds `agent_writes` and
`version`. **`agent_writes` extends `PUT /v1/config/mcp`** rather than a new
`/config/ai`: one handler and one persisted file, and an older client's
`{enabled, writable}` body still means exactly what it did (`enabled` absent
now leaves MCP alone). It is seeded on load from `mcp_enabled &&
mcp_writable`, what agents needed before, and `mcp_writable` no longer touches
agents. `StormPolicy` replaces `AllowAuthenticated`: an `Actor::Agent` writes
only to its `write_vault`, refused at `vault_of` and answered as the JSON-RPC
code `vault_write_not_allowed`; kit scripts follow the same rule. The write
hook is in `ops::create_note`/`update_note`/`create_script`/`update_script`:
**Wrote is every successful write a session performs** (operator, 2026-10-08),
so a kit script is a row too — `kind: script_created|script_edited`, `note_id`
and `version` null, `path` its vault-relative path, `title` its script name. It
counts in `wrote_count`; it is not in a note's provenance or the vault's
`agent-writes` map, which are keyed by note id and a script has none. A
refused write records nothing; `move_note` is REST only, never an agent's.
**The launch context is history** (operator, 2026-10-08): its ids, title and
the session's name are fixed at launch and a later rename or move changes none
of them, while `session_context` resolves the note by id. Context delivery: the built-in
connection offers agents `session_context` and names it in its instructions;
`start` carries `context: true`, and the runtime adds `mcp::OPENING_PROMPT`
(`claude <prompt>`, `opencode --prompt <prompt>`, nothing for `shell`/`fake`).
`gateway_e2e.py` scans every process's argv and environment for the note's
title, body and ids with a positive control. Mutation-checked: the policy
allowing every agent write, and note data sent in `start` / the prompt growing
an argument, and a script write left unrecorded. Rust 562 + 54 tests; live 81
(`e2e.py` unmodified) · mcp 86 · agent 78 · gateway 84 · auth 72 · client 20.

**Slice 7 (Settings; `feat/v2-settings`):** the ten pages are built on real
endpoints in `lib/ui/settings/` (`SettingsPage`: 680 column beside
`SettingsNav` at ≥900, a pushed screen with "‹ Settings" and no AppBar
below). This device keeps the real preset, text-size, note-font and Read-mode
controls; Devices & access lists `GET /v1/auth/devices` (revoke =
`DELETE /v1/auth/devices/{id}`, which also ends its sessions; this device
signs out) and the access keys with the shown-once dialog; Vaults draws the
tile, count and path, a missing vault greyed and still removable, and the
tile opens `AccentPicker` (Q7, `setVaultAccent`); AI access puts the MCP
switches and "Storm agents" on `/v1/config` + `/v1/config/mcp`, with
"Allow writes when chosen at launch" wired **only when `GET /v1/config`
returns `agent_writes`** (sent alone as `{agent_writes}`, slice 5's shape) and
otherwise disabled, "Needs a newer server"; Integrations and Hosts are
restyled with every flow unchanged (the FABs became buttons; Integrations
keeps `/settings/integrations` and the orphan relay, and its nav item carries
a danger dot from `integrationsAttentionProvider`); Storage, Connection
(address, route, pinned-key fingerprint, relays on `PUT /v1/config/relays`,
Disconnect with a confirm) and Advanced (MCP endpoint, versions, Re-pair →
the pairing screen in a `rePair` mode that pops itself) are new; About &
health is the shared health rows with actions plus a compatibility row
(same major.minor) once the server reports `version`. Each page re-reads
`/v1/config` when it opens. `server_settings_screen.dart`,
`client_settings_screen.dart`, `mcp_keys_screen.dart`, `ClientSettingsBody`
and `settingsLeading` are gone.

**Slice 6 (Agents; `feat/v2-agents`):** sessions are routes,
`/agents/s/:id?tab=context|wrote|about`; `logicalParent` takes one back to
`/agents` and `NavMemory` remembers it. `sessionControllerProvider` (an
autoDispose family) owns each stream, so the desk split and the phone screen
share one; `sessionDetailProvider` re-reads a live session's record every 4 s
while it is shown (the stream's `status` event is the bare record, with no
name, context or count) and `sessionWritesProvider` re-fetches
`…/writes` only when `wrote_count` moves. `agentTabsProvider`,
`activeAgentTabProvider`, `agentTitlesProvider`, the tab strip, the switcher
sheet, the End `AlertDialog`, `AgentRow`/`AgentSessionRow`/
`AgentSessionList`, `agentChosenTitle` and `launcherForTest` are gone; the
name is the server's. The sidebar is on `SidebarFrame` (＋ New session only
with a host online, Overview, RUNNING/ENDED `SessionRow`s with the §3.2 dots);
the desk pane is the overview (work cards by (workspace, host) with the
context chip and "wrote n", START AN AGENT cards, the infra line), or the
first-session / no-host states (`NumberedSteps` gains an `inline` action);
the phone is the flat list with the labelled pill. The launcher
(`launcher.dart`) is a 460 modal at desk width and a bottom sheet below,
keeps the old load logic (last host, workspaces, default agent, fallback and
launch notices), and sends `context` and `write_vault_id`, never
`allow_vault_writes`: "Can write to" defaults on when `agent_writes` is on,
to the context note's vault (else the first), is disabled with "Off in
Settings › AI access" when it is off, and is absent for `shell`. A launch
opens the session on Context (with a note) or About. `launchAgentSession`
keeps its positional signature and adds `contextNote`, `provider` and
`runAgain`; slice 4's Start session passes the open note. Session detail:
`SessionHeader` with End → `InlineConfirm` (desk) or the details sheet
(phone), Run again (prefilled with context, host — falling back to an online
one — workspace, agent and write vault; Launch still pressed) and Dismiss;
the panel's Context shows the note read only (`StormMarkdownView`, the
launch-snapshot title), Wrote lists writes with a "‹ Wrote" in-panel view
(a kit-script row is a file under its path, never a link), About is a
`KeyValueList`. Status words are Starting/Running/Unknown/Completed/Stopped/
Failed with the reason in the meta and ended lines. `lib/ui/panels.dart`
holds `PanelTabs`, `KeyValueList`, `InlineConfirm`, `MonoTag`. Contract
changes in tests: `agents_navigation_test.dart` (sessions are routes; no
tabs; "No hosts yet" → the no-host steps; Hosts test kept from slice 7) and
`agent_test.dart` (status words, server names; the tab and terminal-title
tests retired), `integrations_test.dart`'s launcher group (write vault, not
the toggle). 883 client tests; mutation-checked: Wrote ignoring
`wrote_count`, the launcher ignoring `agent_writes`, Run again dropping the
write vault. The acceptance harness enrolls two real `storm-runtime` hosts
and drives a scripted agent through the gateway (`harness/agents.py`); 19
shots captured, deltas in its README. Open: the session view carries no
grants, so About › Integrations lists the account's current connections.

**Slice 8 (the loop; `feat/v2-loop`):** a note's version line carries the
provenance link — "Edited by session {name}, {age} ›" / "Created by session
…", from `GET …/notes/{id}`'s `agent_write`, opening
`/agents/s/{id}?tab=wrote`; it is fetched (`noteProvenanceProvider`) only for
a note in the vault's `agent-writes` map, and again only when that note's
latest agent version moves. A later human edit keeps it (Q13). **A dismissed
session's name stays, as plain `text3` text with no ›**: its route is a 404
("This session is gone"), so there is nothing to link to. **Unseen is
device-local and never on the server** (`SeenVersions`, prefs `storm.seen`,
`{"<vault>/<note>": version}`, the newest 2000): the note screen records
whatever version it has on screen, including one adopted from sync while
open, and a note is unseen when its latest agent write version (`GET
…/agent-writes`, `agentWritesProvider`) is above that. The map is re-read with
the vault's sync (`vaultRevisionProvider`) and whenever the sessions' summed
`wrote_count` moves in the overview the rail and bubbles already poll. Dots:
desktop tree note rows and a collapsed folder holding one (an open folder
does not roll up), phone folder and note rows. **Not on RECENT**: the
prototype draws none there (desktop or phone, loop-a's reference agrees), and
the prototype wins over the brief. The session panel's version line adds "·
edited/created by this session" when this session is the note's latest agent
writer (Context and a Wrote note opened in-panel); the panel re-reads its
note when the session's `wrote_count` moves, and the session screen
invalidates the overview then, so the sidebar's and overview's "wrote n"
follow within the 4 s detail poll rather than the 15 s list poll. A note
pushed from a phone session carries `?session=<id>` (`Routes.note(…,
session:)`) and its back link reads "‹ {session name}", back to the session;
the desk keeps the crumb. **Server (additive):** session views gain
`integrations: [{id, slug, display_name}]` — what the session was granted at
launch, named as then, Storm itself excluded — from a new `agent.db` table
`session_integrations` written in the launch's store transaction from the
connections offered (`LaunchMeta.integration_names`); a rename or disconnect
changes none of it (orchestrator: grants are launch history, like the
context). A session from before the table falls back to its
`session_mcp_grants` slugs; `shell` and an old host have none. About ›
Integrations uses it, and the account's current connections only from a
server that does not send it. Tests: `loop_v2_test.dart` (17: provenance
created/edited/dismissed/after a human edit at both widths, unseen rollup /
clear-on-open / version / per-device persistence, the panel lines, About,
phone-08 at both widths, and a widget-level loop from Start session to the
provenance link back to Wrote); Rust
`a_sessions_integrations_are_launch_history_named_as_at_launch` and the
gateway-writes API test (rename after launch); `gateway_e2e.py` +3. 900
client tests. Mutation-checked: unseen ignoring the version, the folder
rollup off, the provenance link opening Context, "by this session" ignoring
the session id, and the integrations never stored. The acceptance harness
launches the core loop's session **through the UI** (Start session → Launch
on Gateway spec) and its scripted agent reads the note with
`session_context`, edits BOARD and creates a log note through the gateway
(`agents.py` `run_loop`); desktop-01 is now shot after the BOARD session's
write, so its provenance link is real.

**Slice 9 (regression + acceptance; `feat/v2-acceptance`):** the polish
list, then every suite and the acceptance set. **Note prose** is §7.2's
Newsreader 18 / 1.6 with a 22 / 600 H2, in Read mode and as the editor's base
style, through two derived steps — `proseSize = fs·√scale` (17.9 at the
default; the step above `bodySize` that slice 2's `uiSize = fs/√scale` is
below it, so it follows the text-size setting with no new input) and
`proseHeadingSize = proseSize·scale` (22.4) — and `StormTokens.proseLeading`
1.6; the editor's span tree is untouched (a test flattens it). Read-mode
prose is now `text`, as the v2 prototype draws it (it was the M14
prototype's `text2`). **Unseen baseline:** a device's first successful
`agent-writes` load for a vault writes a `"<vault>/"` marker into
`SeenVersions` and takes the agent versions it found as seen, so a new device
does not dot every note an agent ever wrote; a failed load baselines nothing,
and markers are never evicted by the 2000 limit (eviction would re-baseline
and hide real dots). **phone-08:** a note pushed from a session lays its
header out as a `Wrap` — "‹ {session}" keeps its whole width and the controls
drop to a second line when both do not fit (they do not at 360–430 beside ⋯
and Properties, which the prototype does not draw). **Unseen dot semantics:**
the dot's label is a plain `Semantics` label after the row's name; Flutter web
announced the tooltip *before* the name. **Pill:** Lucide folder / search /
plus / hash, as the prototype draws them. **Phone inset 20** (§2):
`StormChrome.contentInset` is `sp·2.5`, which also puts the bubbles at 20 / 20;
the one-left-edge test asserts it and the settings list moved onto it.
**Properties drawer:** open by default from 1200 px (the design frame), shut
below it — between 900 and 1200 the prose column is `W − 676` (224 at 900),
and holding it open until 640 fits (1316 px) would shut it in the 1280
reference shots; the toggle still sticks between notes. **Accepted, not
changed:** the tree orders notes by name, the launcher's workspace default is
the runtime's first, vaults come in the server's order. Tests: 915 client
(was 900); mutation-checked — the baseline never taken, the baseline taken
every load, markers evictable, `proseSize = fs`, the editor ignoring
`proseSize`, the dot's tooltip back in the semantics. **Regression on the
tip:** `make check` clean (fmt, clippy `-D warnings`, `flutter analyze`);
Rust 563 server + 112 relay + 54 runtime; client 915; `make test-live` — e2e
81 (unmodified) · mcp 86 · agent 78 · gateway 87 · auth 72 · client live 20
(+1 skipped); `make test-migration` 16. **Acceptance:** every v2 shot
re-shot; the harness gained `--preset` (writes This device's `storm.theme`
and reloads), `--desktop-width` / `--phone-width` and comma-separated
`--only`; `presets/storm-light|slowflow-earth/` and `sweeps/d900-p360`,
`d1100-p390`, `d1600-p430` show no overflow and every colour semantic intact
— one finding left as is: under SlowFlow earth the low-chroma accent makes a
running dot close to an ended one. `docs/storm-ui.md` rewritten for v2.
**Not run here:** Android back on a device (no Android SDK, `adb` or emulator
on this host; the contract is widget-tested) and the plan's manual Claude
Code / OpenCode start (installed, but it would run on the operator's
account). `CHECKLIST.md` reproduces handoff §11 with evidence per item.

**Real-run fixes (2026-10-08; `fix/v2-acceptance-findings`, stacked on
slice 9):** the operator's real Claude Code run on `c63eacd` passed all eight
checks and surfaced three implementation bugs, fixed before merge, with no
design or contract change. **Compatibility:** release CI stamps the client and
every crate from the tag, but a source build reported pubspec's `1.0.0`
against the server crate's `0.1.0`, so every unreleased build read "may not be
compatible". The pubspec placeholder is now `0.1.0+1`, the same major.minor as
`apps/server/Cargo.toml`; `version_compat_test` fails if they drift, and the
major.minor check itself is unchanged. **Accessibility (web):** Read mode drew
every block as `SelectableText`, a read-only text field whose text Flutter web
never writes into the DOM (`SemanticTextField` sets `aria-label` only), so the
note body was absent from the accessibility tree. It is now plain text in a
`SelectionArea`, pixel-identical in the acceptance set, with selection and
links intact; links in prose became real links. The provenance link is its own
semantics node (it had absorbed `v51 · Saved` into its label), the note title
is a heading, and `SettingsRow` names a trailing `StormToggle` by the row's
label with the sub-line as hint (AI access and This device). Verified in
Chromium's own accessibility tree on the real build. **Terminal:** xterm2
reports the colour scheme (`CSI ? 997 ; Ps n`) whenever `TerminalView` gets a
theme that is not identical while the agent has DEC 2031 on, and
`TerminalTheme` has no value equality, so every session-screen rebuild sent
Claude Code a report. Claude Code restores echo about 0.25 s after `/exit` or
End and exits about 0.5 s later; a report landing in that window is echoed by
the PTY as `^[[?997;1n` (reproduced under a local PTY with 2.1.283). The theme
is now one instance per colour set; a real theme change still reports. A
theme switch in that half second can still echo once: that is the PTY's
behaviour with any late input, left as is. **Kept as is:** About still lists
a session's integrations as granted at launch after a disconnect (launch
history; calls are refused `not_granted`). **Deployment:** a Runtime Host must
run under its own OS account, never one that can reach the vaults
(`deploy/README.md`); the real-run staging, one Unix user for everything,
could edit vault files around Storm. Tests: client 926 (was 915); `make
check`, `make test-live` and `make test-migration` unchanged otherwise; every
fix mutation-checked; the v2 set re-shot from both builds, differing only in
the compatibility row and health dot; the real Claude Code smoke re-run on
`15bb470`, scrollback clean after End.

**UI polish (2026-10-08; `fix/v2-ui-polish`, stacked on the real-run
fixes):** the operator's layout pass after testing the whole stack on the LAN
staging, client only, no change to IA, server or agent architecture. **Session
workspace (desk):** the header (name, status, meta, End) spans the terminal
and the inspector; the terminal fills its column at a 10 px inset; Context /
Wrote / About became a **Details inspector** that works like Properties —
opens by default from 1200 px, toggled from the header and closed from inside,
320 px, its left edge drags it between 260 and 500 while the terminal keeps
360 — instead of a permanent column at 10/21 of the width. Its open state and
width are in-memory providers, not persisted, as Properties and the sidebar
are (the codebase's rule for chrome). The header's actions now sit at the far
edge (a `Flexible` name and a `Spacer` had split the free space). **Phone
session:** a 10 px terminal inset, and the extra keys are a keyboard accessory
shown only while `viewInsets.bottom > 0` (animated in and out; the terminal
takes the room back). **Notes:** the editor surface fills the space between
sidebar and Properties — the header row spans it — and `kEditorMeasure` is
900 (was 640), applied to the prose column only (operator's choice among
three). **Rail foot:** the health dot and Settings share one 40 px square on
the rail axis, 8 apart and 8 from the bottom. **End confirmation:** a compact
banner — bold "End {name}", the message, Cancel and End session on one row
from 560 px of banner, wrapped below; 16 / 12 padding. Tests: client 941 (was
926), `ui_polish_test` and `inline_confirm_test` new, three tests' contracts
changed (the extra keys are absent with the keyboard down; the end prompt is a
title and a message); each new assertion mutation-checked. **Open, not in this
slice:** the login hangs after a correct password in Zen (Firefox engine) on
the operator's profile — the server signs in, Chrome works, headless Zen
does not reproduce; logged in the vault (*Storm v2/Issue — Zen login hang*).

**Revisit if** a second human user becomes a real requirement (Teams, A9):
that is a new authorization design, not a restoration of the removed one.

**83. macOS Runtime Hosts: the same host, a LaunchDaemon, a dedicated
account.** *(2026-10-08, the operator's brief)*

**The spec.** D14 in *Agent Runtime/Decisions*, full text in *Agent
Runtime/macOS Runtime Host* (vault), amends the freeze with AM33–AM41 and adds
AC-M1–AC-M9. The operator's brief fixed the architecture: macOS is a second
platform of the **same** Runtime Host. It runs as a launchd LaunchDaemon under
a dedicated `_stormruntime` account, and never as the logged-in human for
convenience. The amendment is the smallest set of decisions that make that
true. Two of them change Linux behaviour too:
- **AM36:** a revoked host renames `host.json` to `host.json.revoked`.
- **AM37:** SIGTERM and SIGINT end every session's process group, and
  `--exclusive-account` sweeps the service account.

**Why not Docker or a LaunchAgent.**
- **Docker** on a Mac is a Linux VM. File access across the boundary is
  slow, memory is capped, and agents would lose the Mac's own toolchains.
- **A LaunchAgent** running as the human would hand every agent the human's
  whole home. The brief rules it out. The account is the boundary, as on
  Linux.

**The slices**, each its own PR to `staging`, stacked:
1. **The platform module and macOS CI.** `apps/runtime/src/platform/` holds
   paths, the service account, the host name and the PTY write's wait. A
   `runtime-macos` CI job runs clippy and tests on `macos-latest`.
2. **launchd and the install lifecycle.** `install` / `uninstall`, the
   `_stormruntime` account, the layout and the plist, gated on enrollment
   (AM33–AM35), plus AM36's final revocation.
3. **The provider environment** (AM38).
4. **Process lifecycle on shutdown** (AM37), on both platforms.
5. **Workspaces and the TCC boundary** (AM39).
6. **Per-platform enrollment instructions in the client** (AM40).
7. **Release and distribution** (AM41).

Then real-Mac acceptance, AC-M1–AC-M9.

**Slice 1's findings.**
- **macOS's `poll(2)` does not support devices** (its man page, BUGS). On a
  PTY master it answers `POLLNVAL` at once. 77c's bounded input write was
  built on `poll`, so on macOS it would never wait, and the write after it
  would block for as long as an agent ignores its input. The session's lock
  would be held, so not even `end` could get in.
  - **Fix:** the wait is per platform. Linux keeps `poll`, which 77c
    verified. macOS uses `select(2)` through `rustix`, which lifts
    `FD_SETSIZE` on Apple.
  - **Guard:** `input_to_an_agent_that_never_reads_times_out_instead_of_hanging`.
    It is mutation-checked: a wait that always says "writable" makes it fail.
  - **The first run on a real Mac (CI, `macos-latest`) still hung.** xnu's
    `ptcselect` calls a master writable while the slave's queue is below
    `TTYHOG - 2`, which may be room for one byte. `ptcwrite` then sleeps
    until a whole 256-byte chunk fits. So "writable" bounds only the first
    byte. **Fix:** `platform::PTY_WRITE_CHUNK` is 1 on macOS (256 on Linux,
    as verified), so no write after a wait can block. The same test is the
    guard.
- **The default host name** read `/proc/sys/kernel/hostname` and
  `/etc/hostname`, neither of which exists on macOS. Every Mac would have
  enrolled as `runtime-host`. It now comes from `uname`'s node name, with
  `.local` dropped (AM40).
- **The crate cross-checks for `aarch64-apple-darwin`** (`cargo clippy
  --all-targets -D warnings`, with zig as the C compiler for `ring`). That
  check is how the `select` code was first compiled. A Linux box cannot link
  a macOS binary without Apple's SDK, so the CI job on `macos-latest` is
  what builds, links and runs the tests.

*Revisit if:* a third platform arrives (the platform module becomes a trait
rather than `cfg`-selected modules); or `poll` is shown to support PTYs on
every supported macOS (the split can go).

**83b. Slice 2: `install` writes a LaunchDaemon that runs exactly while the
host is enrolled.** *(2026-10-08)*

**The layout and the plist are pure functions.** They live in
`platform/launchd.rs`, compiled on every platform, so the Linux suite tests
the plist too. A Python `plistlib` parse of the output gave exactly the
intended dictionary. Executing them is `platform/macos_service.rs`. It is
root only, idempotent, and calls every tool by absolute path, because it
runs as root with the invoking user's `PATH`.

**`sudo storm-runtime install [--operator <user>]`:**
- **The account.** It creates, or brings back into shape, the hidden
  `_stormruntime` user and group with `dscl`. The id is the first one free in
  200–400, the shell `/usr/bin/false`, the password `*`, `IsHidden`, and the
  home `state/home`. It refuses to continue if the account is in `admin` or
  `wheel`.
- **The directories.** Each is created with its mode and owner (AM33), and an
  existing symlink is refused rather than followed.
- **The workspace grants.** Two inheritable ACL entries go on the workspace
  root, for the account and for the operator (`--operator`, else
  `$SUDO_USER`). There is no `delete` on the root itself.
- **The binary** is copied to `/Library/StormRuntime/bin`, `root:wheel
  0755`, so only root can replace what launchd runs.
- **The config** is written only when there is none.
- **The plist** is written `root:wheel 0644`, then loaded with `bootout`
  (if loaded) → `enable` → `bootstrap system`. Bootstrap is retried while a
  bootout is still finishing.

**The plist:**
- `UserName` and `GroupName` are `_stormruntime`.
- Explicit `HOME`, `PATH`, `SHELL=/bin/zsh` and `LANG`.
- **`KeepAlive` is `PathState` on `state/host.json`, with no `RunAtLoad`.**
  An unenrolled host never starts. Enrolling starts it, and a crash restarts
  it. A revoked host, whose `host.json` AM36 renames, is not restarted.
- `ProcessType Standard`, `Umask` 23 (= 0o027), `ExitTimeOut` 20, and logs to
  `/Library/Logs/StormRuntime/storm-runtime.log`.

**`uninstall [--purge]`** boots the job out and removes the plist and the
binary. `--purge` also removes the state, logs, config and the account.
Workspaces are never removed.

**On Linux, both commands refuse** with a pointer to the package.

**Verified here:**
- plist tests: the account and environment, enrollment gating, never
  `Background`, and well-formedness. They are mutation-checked: adding
  `RunAtLoad` or `ProcessType Background` fails two tests.
- the default config parses;
- clippy `-D warnings` for `x86_64-linux` and for `aarch64-apple-darwin`.

**Not verified here:** the `dscl`, `chmod +a` and `launchctl` calls
themselves. They need a Mac, and they are AC-M1, AC-M2 and AC-M6.

**83c. Slice 3: one explicit `PATH` for finding an agent CLI and for
running it (AM38).** *(2026-10-08)*

**The problem.** Under launchd the host's `PATH` is bare
(`/usr/bin:/bin:/usr/sbin:/sbin`). `claude` in `/opt/homebrew/bin` was
reported `not_installed`, and a CLI that did resolve would have run with a
`PATH` that could not find its own tools.

**The fix:**
- **`runtime.toml` gains an optional `path`** (a list of directories). Unset,
  the platform decides:
  - **macOS:** the host's own `PATH`, rooted at the account's `HOME`:
    `$HOME/.local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin`.
    The plist sets the same string, from the same function.
  - **Linux:** the unit's own `PATH`, unchanged (`platform::default_path()`
    is `None`).
- **Every `CliProvider` is built with that one `PATH`.** `available()`
  resolves against it, and every session's environment carries it, set
  after the provider's env file, so an env file cannot make "installed" and
  "runs" disagree.
- **`HOME` and `SHELL`** come from the service definition: on macOS the plist
  sets `/Library/StormRuntime/state/home` and `/bin/zsh`. The shell provider
  is still `$SHELL -l` (§9.2).

**Tests**, both mutation-checked:
- `agent_clis_are_found_on_the_configured_path_not_only_the_hosts`: a
  `claude` and a custom CLI in a directory off the host's `PATH` are
  available only through `path`.
- `a_session_runs_with_the_path_its_cli_was_found_on`: the session sees the
  configured `PATH` even when its env file says `/nowhere`.

Removing either half fails its test. The real `/opt/homebrew/bin` case on a
Mac is AC-M3.

**83d. Slice 4: a host stops in order, an ending empties the whole
session, and revocation is final.** *(2026-10-08; AM36, AM37)*

**SIGTERM and SIGINT.** `serve` ends every session's process group: SIGHUP,
then SIGKILL after the grace. It waits until each session is `ended`, bounded
at 2×grace + 3 s (13 s, inside launchd's 20 s `ExitTimeOut`), and exits 0.
- **Nothing is posted** from the first signal on: one flag, checked in
  `post_for_response`.
- **`sessions.json` keeps those sessions**, so the next `hello` reports them
  `failed (host_restart)`. §13 is unchanged.

**The kill reaches the session, not just the group.** A job-control shell
(`zsh -l`, `bash -i`) puts `cmd &` in a process group of its own, which
`kill(-pgid)` never reached.
- **The gap.** A job that ignored SIGHUP survived End, on both platforms,
  against §7.3. Found by the macOS acceptance harness's review (83g).
- **The fix.** The SIGKILL step now also signals every process whose session
  id is the agent's pid. `platform::session_members` lists them:
  - **Linux:** `/proc/<pid>/stat` field 6, read after the last `)`;
  - **macOS:** `proc_listallpids` + `getsid`.

  Zombies are skipped.

**`ended` means the session is empty.** Before the reader reports it,
anything still in the session gets SIGHUP, then SIGKILL after the grace.
Shutdown waits for `ended`, so it leaves nothing behind. Only a process that
called `setsid` itself has left the session.

**`--exclusive-account`** is passed by the systemd unit and the launchd plist.
- **What it does:** it lists every live process of the account except
  itself, then sends SIGHUP, then SIGKILL after 0.5 s to what is left. It
  runs at startup and after a shutdown or revocation.
- **It is not `kill(-1)`.** The first version used `kill(-1)`, which on Linux
  excludes the caller. **On macOS a POSIX `kill(-1)` includes the caller:** on
  the first real-Mac run (CI, `macos-latest`) the host killed itself the
  moment launchd started it, wrote nothing, and sat in `spawn scheduled`
  forever. No test had run the real sweep, because it would kill the test
  runner. The process list is testable without signalling anything:
  `the_sweep_targets_the_accounts_other_processes_never_itself` lists a
  child and never the caller, and is mutation-checked.
  - **Listing:** `/proc/<pid>/status` `Uid:` on Linux; `proc_pidinfo`
    `pbi_uid`/`pbi_ruid` on macOS. That is the counterpart of systemd's
  control-group kill. It also covers a host that was SIGKILLed or crashed,
  which launchd's job-group kill does not reach.
- **The guard:** it is refused as root and as any account but
  `platform::SERVICE_ACCOUNT` (`getpwuid_r`), and the check runs before any
  sweep can.
- **Not tested:** no test runs the sweep, because a host's own agent session
  may run the suite as the service account.

**Enrollment announces first.** `host.json` is its last write: under launchd
that file starts the host (AM35), and the host's sweep would end an `enroll`
still running as the same account.

**AM36.** A refused key renames `host.json` to `host.json.revoked` before
exit 3, which is what stops launchd. After that, `serve` and `check` say the
host was revoked, and `enroll` needs no `--force` and removes the marker.

**Tests:**
- **`tests/lifecycle.rs`** drives the real binary against a stand-in server
  that proves its pinned key:
  - SIGTERM and SIGINT exit 0;
  - revocation exits 3 with the rename;
  - enrollment announces before it writes.
- **A host test** with a recording server: no ending is posted during
  shutdown.
- **`tests/cli.rs`:**
  - a `set -m` job in its own group is gone after End;
  - a detached job is gone before `ended`;
  - `session_members` lists the leader and not the runtime.
- **The plist** passes `--exclusive-account`.
- **`agent_e2e.py`**, 85 checks, adds:
  - **the SIGTERM case:** exit 0, a HUP-ignoring child gone, no ending
    posted, `host_restart` after the restart;
  - **the AM36 checks.**

Every new test was mutation-checked (eight mutations, each caught).
**83e. Slice 5: a macOS host works only in Storm's own root, never in a
person's home or on a mounted volume (AM39).** *(2026-10-08)*

**The boundary is three things together:**
- **The account.** `_stormruntime` is not in `admin` or `staff`. A person's
  home is `0750` (owner and `staff`), so the account cannot enter it.
- **The default root, `/Library/StormRuntime/workspaces`,** sits outside every
  TCC-protected location. The host neither needs nor asks for Full Disk
  Access, and a LaunchDaemon could never answer TCC's prompts anyway.
- **Configuration.**
  - **`platform::PROTECTED_ROOTS`** is `/Users`, `/Volumes`, `/Network` and
    `/System/Volumes/Data` on macOS. The last is the data volume's own
    spelling of the first two, behind their firmlinks. On Linux the list is
    empty, because the unit's `ProtectHome` already hides homes.
  - **`check_roots()` refuses a root that is inside, or contains, any of
    them**, as it refuses one that overlaps a Storm data root (D3). A root of
    `/` is therefore refused. The list is not configurable in V1.

**Sharing a workspace with the operator** is slice 2's two inheritable ACL
entries: everything either account creates under the root stays readable and
writable by both. An existing checkout is cloned or moved into the root.

**Tests:**
- Homes, volumes, autofs and the data-volume spelling are refused, and so
  are `/` and `/System` because they contain them.
- `/Library/StormRuntime/workspaces`, `/opt/work` and `/Users-shared` are
  accepted, since components are compared, not strings.
- `check_roots()` uses the platform's own list.
- The "contains" half is mutation-checked.

**What needs a Mac:** that the account really is refused a home,
`~/Documents`, a `/Volumes` mount and a server's state tree, and that the
operator really can write what the agent wrote. That is AC-M7, in the
acceptance harness (83g).

**83f. Slice 6: the enroll sheet is per platform (AM40).** *(2026-10-08)*

**The problem.** The sheet showed only Linux commands (`sudo -u storm-runtime
…`, `systemctl`). The machine being enrolled is not necessarily the client.

**The fix.** The sheet offers Linux and macOS through the existing
`ChoiceChips`. It preselects macOS when `defaultTargetPlatform` is macOS
(native or web), and Linux elsewhere. Each platform shows only its own
commands, each with a copy button:
- **Linux:** `apt install`, `sudo -u storm-runtime storm-runtime enroll`,
  `systemctl enable --now`.
- **macOS:**
  - `brew tap dewanshdt/storm …`;
  - `brew install storm-runtime`;
  - `sudo "$(brew --prefix)/bin/storm-runtime" install`;
  - `sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime enroll`.

  The install step uses the full path because `sudo` need not search
  Homebrew's prefix. The formula's caveats and `deploy/README.md` use the
  same form (83g).

A footnote names where the host runs and where its workspaces are. The
enrollment-string box is unchanged.

**Tests.** The step lists are pure functions (`enrollSteps`,
`enrollFootnote`). Tests assert that neither platform ever shows the other's
commands, that the string is never an argument, that the default follows
the client's platform, and that switching works.

**Not run locally.** There is no Flutter SDK in the build box, and its disk
has no room for one. CI's `client` job is the first run. The APIs the sheet
uses (`ChoiceChips`, `ChoiceOption`, `copyToClipboard`) were checked against
their definitions.

**83g. Slice 7: macOS distribution, and scripted acceptance on a real Mac
(AM41).** *(2026-10-08)*

**A tag also builds `runtime-macos`** on `macos-latest`:
- an arm64 and an x86_64 build, combined with `lipo` into one universal
  binary;
- ad-hoc signed, like the app;
- shipped as `storm-runtime-X-macos-universal.tar.gz` with a README of the
  install and enroll commands, listed in `checksums.txt` and the release
  notes.

The version is stamped from the tag (46), with `sed -i.bak` because BSD sed
needs a suffix.

**`Formula/storm-runtime.rb` makes the repository the tap**
(`brew tap dewanshdt/storm https://github.com/dewanshDT/Storm`).
- **It builds from the git tag** with `cargo install`, so there is no sha
  and no second version source.
- **Its caveats** use `sudo #{opt_bin}/storm-runtime install`, since `sudo`
  need not search Homebrew's prefix. The client and `deploy/README.md` use
  `"$(brew --prefix)/bin/storm-runtime"`, the same idea.
- **`make formula-check`** (also run by the `www` CI job) fails when the
  formula's tag differs from `release.ts`, so the release-prep PR bumps
  both.
- **Until the release that carries macOS hosts, it names v0.4.0,** which has
  no `install` subcommand. Nobody is pointed at it before then: the client's
  instructions ship in that same release.

**`deploy/README.md`'s Runtime Hosts section is now one host on two
platforms:**
- install, enrollment and launchd's enrollment gate;
- `launchctl print`, `kickstart` and `bootout`, plus logs, upgrade and
  uninstall;
- where CLIs must live;
- logging CLIs in as `_stormruntime`. Claude Code may keep its credentials
  in a Keychain that a daemon account has no session for; the fallback is
  `claude setup-token` into a `0600` `env_file`.
- workspace ACLs, TCC and refused roots, and troubleshooting.

**`apps/runtime/tests/macos_acceptance.py`** drives AC-M1–M8 against the
installed LaunchDaemon and a throwaway storm-server, through REST/SSE,
`launchctl`, `dscl`, `ps` and `stat`.
- **Output:** a PASS/FAIL/MANUAL table keyed by AC id, exiting non-zero on
  any FAIL.
- **Safeguards:** it refuses to run over an existing install unless
  `STORM_ACCEPT_REPLACE=1`. It stubs `claude` and `opencode` only with
  `STORM_ACCEPT_STUBS=1`, and removes the stubs afterwards.
- **MANUAL** (a human must check): a real CLI prompt, the Mac → phone
  handoff, a reboot, and the Full Disk Access inspection.

**`macos-acceptance.yml`** runs it on `macos-latest`, on `acceptance/**`
pushes and on dispatch.

**Its review found the §7.3 job-control gap that 83d fixed.**

**Verified here (Linux):**
- every workflow parses;
- the packaging step, rendered from YAML, built a correct tarball;
- `formula-check` passes, and fails on a mutated tag;
- the harness compiles, and its parsers pass against sample macOS output.

**Not yet run on a Mac.**

**83h. Real-Mac acceptance: four defects only a Mac could show, and 97 of
97 automated checks.** *(2026-10-08)*

`macos_acceptance.py` ran on GitHub's `macos-latest` (dispatch on
`acceptance/macos-runtime`), and the stack's `runtime (rust, macOS)` job ran
the suite there. Three rounds:
- **Round 1 (PRs as first pushed):**
  - **Install passed in full (AC-M1).** The account, modes, owners, ACLs, the
    plist, and launchd loaded but not running before enrollment.
  - **The host never came up after enrolling:** an empty log and
    `spawn scheduled`.
  - **The suite's input-deadline test hung.**
- **Fixes:**
  1. **The sweep killed the host** (83d). On macOS a POSIX `kill(-1)`
     includes the caller. The sweep now lists the account's processes.
  2. **The PTY write still blocked** (83, slice 1). xnu reports a master
     writable with room for one byte. The chunk is now 1 byte on macOS.
  3. **The harness misread `dscl`'s `dsAttrTypeNative:IsHidden`.** It also
     keeps `launchctl print` and launchd's log lines on failure now.
- **Round 2: 96 passed, 1 failed, 5 manual.**
  - `launchctl print` still found the job right after `uninstall --purge`,
    because `bootout` returns early. `uninstall` now waits, up to 20 s, until
    launchd has let the job go.
  - The suite on slice 2 showed a macOS-only race in
    `stopping_ends_everything_the_session_spawned`. **BSD revokes a
    session's terminal when its leader exits**, so the master saw
    end-of-file while a HUP-ignoring job lived. The session reported
    `ended`, and the grace's SIGKILL was skipped.
    - **Fixed by 83d:** a session ends only once it is empty.
    - **On slices 1–3** the test is marked ignored on macOS, with that
      reason. Slice 4 turns it back on.
- **Round 3: 97 passed, 0 failed, 5 manual.**
  - **MANUAL**, which a runner cannot do:
    - Claude Code and OpenCode logged in and taking a prompt;
    - the Mac → phone handoff;
    - a reboot;
    - the Full Disk Access inspection.
  - **AC-M9** (Linux unchanged) is the Linux CI and `agent_e2e.py` (85/85).
  - **CI is green on all seven PRs.**

**Found while writing the manual checklist: git and the shared root.** Git
refuses a repository another uid owns ("dubious ownership") and ignores
ACLs. In AM39's shared root, the agent's git would therefore have failed in
every checkout the operator made.
- **The fix:** `install` adds `safe.directory = <workspaces>/*` to the
  `_stormruntime` account's `~/.gitconfig`, once, leaving the rest of the
  file alone.
- **The operator's side:** `install` prints the same line for the
  operator's own git config, which it never edits.
- **Guards:** a pure, tested merge function, and an AC-M7 harness check.

**What still needs the operator's Mac:**
- **The four MANUAL rows**, ideally with the real Claude Code login as
  `_stormruntime`. Claude Code on macOS may keep its login in a Keychain
  the daemon account has no session for. The documented fallback is
  `claude setup-token` into a `0600` `env_file`.
- **An Intel Mac:** the universal binary includes x86_64, untested.

**83i. The operator's manual Mac round, and the fixes folded into the
stack.** *(2026-10-10)*

The operator ran the MANUAL rows on their Mac (macOS 27.0, arm64, FileVault
on), with a follow-up round for reboot, sleep/wake, revoke + re-enroll and the
AM40 sheet: **28 PASS · 11 HUMAN · 5 FAIL · 1 SKIPPED**. The record, with
evidence and code pointers, is the vault note *Agent Runtime/macOS Manual
Test Report 2026-10-09*.
- **Passed:** a token-authenticated Claude Code and OpenCode, each taking a
  prompt as `_stormruntime`; Mac → phone handoff; restart; reboot (boot →
  `link up` 45 s, login → `link up` 6 s); sleep/wake (reconnect in 5 s); no
  Full Disk Access and no privacy prompt; revoke and re-enroll; no secret in
  the log or in any argv.
- **The operator chose to fold the fixes into the stack** rather than open
  new PRs. Each fix is a commit on the slice it belongs to, merged up to
  #116.

Fixed:
- **F1 (security), slice 4.** OpenCode's TUI daemonizes `opencode serve
  --service` with `setsid`. It left the agent's process session, survived
  End, and served its unauthenticated HTTP API on 127.0.0.1 as the host's
  account until the host restarted. **Every process a CLI session starts now
  carries `STORM_RUNTIME_SESSION=<id>`**, and the ending ends the session's
  members plus every process of the account carrying the tag (read from
  `/proc/<pid>/environ`, or `KERN_PROCARGS2` on macOS). The exclusive-account
  sweep stays the backstop for a process that replaced its environment.
- **B-0, slice 4.** After a reboot two sessions sat at Starting for 17
  minutes and End did nothing. The link handled each command inline, so the
  first start, which never returned, held every later command. **A start
  now runs as its own task, with a 30 s deadline** (`start_failure` past it;
  whatever it returns later is stopped). Commands for a starting session
  queue behind it and apply in order. The hang's own cause is not known: the
  new logs will say whether the command never arrived or the provider
  blocked.
- **B-2 / F4, slice 4.** Neither side logged a command. The host now logs a
  start banner (version, paths, providers and their availability, roots, the
  sweep's counts), each command received and each start's duration; the
  server logs each command sent, each status report and each revocation.
  Kinds and ids only, never payloads.
- **B-1, slice 4.** With the Mac asleep the host looked online and input
  typed from the phone was lost: the keepalives kept landing in a dead
  connection's send buffer. **Hosts now heartbeat** (`POST
  /v1/runtime/heartbeat` every 15 s; `heartbeat: true` in `hello`), and the
  server drops the link of a heartbeating host silent for 45 s, which marks
  it offline and its sessions `unknown` and refuses input with 503 (which
  the client already words as "The host is offline"). Older hosts are never
  dropped for silence. The host also treats 45 s without link bytes as a
  dead link.
- **F3, slice 3.** The Shell provider's `zsh -l` ran `/etc/zprofile`'s
  `path_helper`, which put the system directories first. Sessions now carry
  `STORM_RUNTIME_PATH`, and `serve` writes a managed `~/.zprofile` in the
  service account's home that restores it. A `.zprofile` without the managed
  first line is the operator's and is never replaced.
- **B-5, slice 4.** Re-enrolling deleted `host.json.revoked` and left the
  revoked key in `identity/`. Both now move to `revoked/<old host id>/`.
- **S1, slice 6.** A workspace cloned after the host linked appeared only on
  a second look. The workspace listing now waits up to 2 s for the inventory
  its refresh asked for.
- **B-4, B-6, B-7, slice 6.** The macOS enroll command starts with
  `cd / &&`; the enrollment string is masked until Show; About & health says
  "App x · Server y" and the compatibility warning names both versions.
- **B-3, F5, slice 7.** Acceptance builds carry the latest release's version
  name, so a test app no longer reads as incompatible with the server. The
  token route (`claude setup-token` into a `0600` `env_file`) is the
  documented Claude Code login on macOS, in `install`'s output and
  `deploy/README.md`: the interactive login needs a login Keychain the
  hidden account does not have.

**A review of the fixes** found that a start in flight across a reconnect
was missing from `hello` (the server would mark it lost while it ran), and
that concurrent starts could pass `max_sessions`. In-flight starts are now
reported as `starting` and hold their slot; a duplicate Start is ignored. F1
also ends a per-account daemon a session started on demand (tmux,
ssh-agent) with that session: accepted, since nothing a session started
outlives it.

**Still open:** the Mac re-test of these fixes; an Intel Mac.


**84. An interactive installer: one script, any part of Storm, on a Linux or
macOS laptop (M24).** *(2026-10-10)*

Spec and plan: the vault notes *Installer/Interactive Installer* and
*Installer/Interactive Installer — Plan*. `deploy/install.sh` becomes one
dependency-free script with its own TUI, still served from
`https://dewanshdt.github.io/Storm/install.sh`. On one machine it installs any
of the server, the Runtime Host, the desktop app and phone pairing; sets up the
account; and enrolls the runtime. Re-running it is how a machine is upgraded,
checked, paired or cleaned up.
- **84a.** One script, bash ≥ 3.2, no `dialog`/`gum`. Keys come from
  `/dev/tty`, so `curl | sh` stays interactive; flags (`--yes`, `--dry-run`,
  `--server --runtime --app --mobile --agents --everything`) script it.
- **84b.** Components are a registry (detect, plan, install, configure,
  verify, uninstall); presets are Everything / Server and app / Runtime only.
  The relay is registered and disabled until Remote Connectivity ships.
- **84c.** Linux takes the apt repo; macOS takes release assets, each checked
  against `checksums.txt`. Homebrew is offered for the agent CLIs only.
- **84d.** **A macOS server**: a universal `storm-server`, and `up` on macOS
  installs a LaunchAgent that runs as the user, with data in `~/Storm`.
  Notes are the person's own files; on a laptop the server need only run
  while they are logged in; `~/Storm` is outside every TCC-protected folder.
  *Revisit if* a Mac must serve while logged out (a LaunchDaemon with its own
  account, like the runtime's).
- **84e.** `storm-server host-enrollment` mints an enrollment string on the
  server's own machine, as the account, and prints it only into a pipe.
  The installer pipes it straight into `storm-runtime enroll`, so a runtime
  beside its server enrolls with no app and no visible secret.
  `storm-server has-account` (exit 0/1) lets the installer wait for a first
  device's pairing; `storm-server qr TEXT` draws its links.
- **84f.** A runtime for a server elsewhere enrolls by pasting the app's
  string at a hidden prompt; it never reaches argv, the screen or the log.
- **84g.** A fresh server's account comes from pairing (the first device to
  scan creates it) or `passwd`, offered in that order.
- **84h.** Safety: everything inside `main`; a plan screen lists every step and
  every `sudo` before anything changes; re-runs are idempotent; secrets go on
  stdin only; uninstall never deletes vaults or workspaces; no telemetry.

---

**85. The session experience: agent-named sessions, logo rows, native image
paste, terminal metrics and settings.** *(2026-10-10, approved by the
operator)*

**The spec.** D15 in *Agent Runtime/Decisions*, full text in *Agent
Runtime/Session Experience — D15* (vault), amends the freeze with AM42–AM47
and adds AC-N1–N5, AC-I1–I5 and AC-T1–T3. 84 is the installer.

**What a same-day spike on a Runtime Host settled** (Claude Code 2.1.296,
OpenCode 1.18.31, a real PTY, no `DISPLAY`):
- **A bracketed paste of an image's absolute path is each agent's own image
  attach.** Claude Code shows `[Image #1]` with no path, and the model named
  the test image's colour; OpenCode shows `[Image 1]`. No permission prompt,
  although the file sat outside the workspace.
- **Claude Code's Ctrl+V cannot be served on a headless host.** With shims
  for `xclip`, `wl-paste` and `xsel` first on `PATH`, none was called, with or
  without `DISPLAY`. Faking the host clipboard is dropped.
- **Claude Code's title carries a name and an activity glyph**: `✳ <name>`
  idle, `◐`/`◑ <name>` working. OpenCode's is always `OpenCode`.

**The decisions.**
- **AM42–AM43:** the host parses each session's output once, live, with
  `vte`, for OSC 0/2 only, and reports a cleaned `title` and an `activity`
  through the status POST and `hello`. The server stores `title` (kept after
  the end); `activity` is not a status. **The client never parses titles.**
- **AM44:** session rows lead with the agent's real logo, status as a badge
  on it, and `host · workspace · age` underneath.
- **AM45–AM46:** a pasted or dropped image is pulled by the host into
  `sessions/<ags>/inbox/` and its path bracket-pasted. Never in a workspace;
  the server holds the bytes in memory only until staged.
- **AM47:** terminal line height 1.25 (was 1.7), even padding, the leftover
  under the last row split above and below it; and **Settings › Terminal**,
  client-only.

**The slices**, each its own PR to `staging`: 85a terminal metrics and
settings; 85b–85d the naming stack (host, server, client); 85e–85g the image
stack (wire, desktop and web capture, Android capture).

*Revisit if:* AM22 is approved (its screen model replaces AM43's observer;
the wire is unchanged); an agent stops treating a pasted image path as an
attachment; terminal values need to follow the person across devices.

**85a. Slice 1: the terminal's metrics, and Settings › Terminal (AM47).**
*(2026-10-10)*
- **Why the bottom gap grew in v0.4.0.** xterm2 fits
  `floor(height / cellHeight)` rows and leaves the remainder under the last
  one. The v2 handoff's line height of 1.7 made a row about 22 px, so up to
  21 px sat below the padding. `StormTerminalView` now measures a row exactly
  as xterm2's painter does (`terminalCellHeight`) and splits the remainder
  above and below (`evenTerminalPadding`), a hair under it so xterm2's own
  `~/` still fits every row.
- **Line height 1.25**, and the padding `sp × 1.25` on every side in both
  layouts (10 px, the inset the v2 polish pass pinned on phone and desk; the
  spec's tighter phone value was dropped for it).
- **Settings › Terminal** (`lib/ui/settings/terminal_page.dart`), right after
  This device: text size (10–20, "Default" keeps each layout's size), line
  spacing Compact 1.15 / Default 1.25 / Relaxed 1.5, padding Tight / Default /
  Roomy, a live preview through the real view, and Reset. `TerminalPrefs`
  lives in prefs under `storm.terminal.*`, is read with Settings at launch so
  a terminal opens at its final size, and is never sent to the server.
- **The view owns the layout values.** Callers pass `phone` and the prefs;
  the font size, padding and line height are decided in one place, so the
  preview and every session agree.
- Tests: `terminal_settings_test.dart` (row fit at every spacing, the
  remainder split, prefs round trip and fallback, the page end to end);
  `ui_polish_test.dart` reads the inset from the rendered `TerminalView`.

**85b. Slice 2: the host reads the agent's title (AM43).** *(2026-10-10)*
- `apps/runtime/src/terminal.rs`: a `vte` 0.15 parser fed from
  `Events::output`, acting only on OSC 0/2 (BEL or ST, `;` inside a title
  kept). It observes; the ring and the stream are untouched, and a replay
  re-sends stored bytes without parsing them.
- `clean()` takes one leading glyph (`✳` idle; `◐◑◒◓` and braille
  working), drops control characters, collapses whitespace, caps at 120
  characters, and gives no name for an empty title, a bare `Claude Code` /
  `OpenCode`, or a shell's `user@host:path`. Pinned by
  `docs/terminal-title-vectors.json`, captured from Claude Code 2.1.296 and
  OpenCode 1.18.31. A name, once given, survives a later title without one.
- **Reported only on a change** of `(title, activity)`, at most once a second
  per session, in the uploader after the output it came with: the status
  POST carries `{"status": "running", "title", "activity"}`, so a server
  without 85c ignores the extra fields. The ending's POST and `hello`'s
  live sessions carry them too, so a reconnect restores them.
- Test: `the_agents_title_is_reported_once_per_change` runs a real shell that
  sets four titles in one burst and a fifth later: two reports (the first
  burst coalesced, no spinner frame), and `hello` carries the last.

**85c. Slice 3: the server keeps the title and the activity (AM42).**
*(2026-10-10)*
- `agent.db` `sessions` gains `title TEXT`, added by `ALTER TABLE` when an
  older database lacks it (no version bump; the store has always been
  additive). The record carries `title` and `activity` in its JSON.
- A report's non-empty `title` replaces the stored one; null keeps it, so a
  name survives the session's end. The server also drops control characters
  and caps it at 120 characters, so a misbehaving host's text stays printable.
- `activity` lives in `SessionLive`, never on disk. Only `working` / `idle`
  are accepted, and it is shown only while the record is `running` — an
  `unknown` session's last word may be stale. `hello` restores it.
- A report that only changes the title or activity is logged at no level
  (the record still updates and the stream gets a `status` event); a status
  change is logged as before.
- Tests: three manager tests (title kept and never cleared, activity only
  while running, a restart keeps the title and `hello` restores activity, an
  old database gains the column) and `agent_e2e.py` (a title typed into the
  fake provider's echo reaches the stream as a status and outlives the end):
  88 checks.

**85d. Slice 4: every surface names a session by its agent, and rows lead
with the agent's mark (AM44).** *(2026-10-10)*
- `AgentSession` gains `title`, `activity` and `displayName` (`title`, else
  the launch name); the stream's bare record brings both, and a record
  without a title keeps the one known. Sidebar, phone list, overview rows,
  the session header, the end confirmation and a note's "from session" link
  all use `displayName`. The launch `name` stays the handle.
- `AgentMark` (`agent_widgets.dart`): the agent's real mark on a `surface2`
  tile — Claude's starburst in `#D97757`, OpenCode's mark in `text` (its
  brand black vanishes on dark), Lucide `square_terminal` for the shell,
  `bot` otherwise — from Simple Icons 16.34.0 (CC0), drawn with
  `flutter_svg`; source noted in `assets/agents/README.md`. The badge:
  accent and pulsing while working (still under reduced motion), text3 while
  starting, amber while unknown, danger when failed; an ended tile at 50%.
- Rows read `host · workspace · age`; the overview's grouped rows, already
  under "workspace on host", read just the age. Rows are announced as "Fix
  the login, Claude Code on build-vm, working".
- The harness's scripted agent takes `{"title": …}`, and its sessions now
  name themselves as Claude Code does, so the shots show names and badges.
- Tests: `agent_mark_test.dart` (marks, badge colours, pulse, reduced
  motion, faded ended tiles), model tests for `displayName` and the live
  merge; two list tests updated for the new sub-line. Client 965.

**85e. Slice 5: an image is staged on the session's host (AM46).**
*(2026-10-10)*
- `POST /v1/agent/sessions/{id}/terminal/images`, raw bytes, up to 10 MiB
  (the route's own body limit). The type is the bytes' (`agent/images.rs`
  sniffs PNG, JPEG, GIF, WebP); a declared `image/*` type must agree, any
  other declared type is ignored. 413 / 415 / 409 ended / 503 offline / 429
  over 4 per session or 64 MiB in total / 504 after 30 s / 502 with the
  host's reason.
- The bytes stay in the manager's memory only while in flight. The host is
  sent `terminal.image {session, image, size, blake3, ext}` — **BLAKE3, not
  the spec's SHA-256**: both crates already depend on it — and pulls them
  with `GET /v1/runtime/sessions/{id}/terminal/images/{image}`, which only
  that session's host may read. It answers `POST` with `{"staged": path}` or
  `{"failed": reason}`; the server accepts only an absolute, printable path.
- The host runs it as its own task, like `refresh` (found by
  `agent_e2e.py`: the generic path would have queued it), checks the id
  (`img_` + alphanumerics, so nothing can leave the inbox), the size and the
  hash, and writes `state/sessions/<ags>/inbox/<image>.<ext>` `0600` through
  a temporary file and a rename, the directories `0700`. The inbox goes with
  the session directory when the session ends.
- Tests: `images.rs` (sniffing, limits), two manager tests (the round trip,
  only the session's host can fetch, every refusal and the timeout), two
  host tests (a private whole file; a damaged image reported, not staged; an
  escaping id refused) and `agent_e2e.py`: a real stage with the file `0600`
  in the inbox, 415, 413, and the inbox gone after the end (93 checks).

**85f. Slice 6: pasting and dropping images on desktop and web (AM45).**
*(2026-10-10)*
- **Keys.** `StormTerminal` asks `onImagePaste` on the platform's paste
  (Cmd+V on a Mac, Ctrl+Shift+V elsewhere) and on Ctrl+V (Claude Code's
  image key). With an image on the clipboard it is staged and pasted;
  without one the paste chord pastes text and Ctrl+V sends `^V`, as before.
  Not on the web, where the browser owns the clipboard.
- **The clipboard.** **Not `super_clipboard`, as the spec said:** it builds a
  Rust library for every app. A `storm/clipboard` method channel instead, in
  `AppDelegate.swift` beside `storm/links` (81l): a copied image file's own
  bytes (PNG/JPEG/GIF/WebP kept, anything else AppKit reads → PNG), a PNG as
  it is, a screenshot's TIFF → PNG, and nothing for a copied file that is not
  an image (Finder puts its icon there too). Dart re-encodes anything else
  it can decode (BMP) as PNG.
- **The web** listens for `paste` events carrying image files while the
  terminal has focus; text pastes pass untouched. **Drops** on every
  platform come through `desktop_drop` 0.8.4; the terminal shows an accent
  outline while something is held over it.
- **The flow** (`SessionController.pasteImages`): each image staged in turn,
  every path typed through `StormTerminal.paste` (so bracketed paste follows
  the agent's mode) only once all arrived; a chip after 300 ms with a cancel
  (a cancelled paste's path is never typed); a failure says why for 6 s and
  types nothing.
- **Verified on the real web build**: the harness's new `desktop-23` shot
  dispatches a browser paste of a PNG into a running session on a real
  server and Runtime Host; the host staged it in the session's inbox and its
  path was typed. The Swift is compiled by `acceptance.yml`, not here.
- Tests: `image_paste_test.dart` (sniffing, which keys are a paste, the
  controller's stage-then-type, the 300 ms chip, failure, cancel, Ctrl+V with
  and without an image). Client 973.

**85g. Slice 7: an image from the Android clipboard (AM45).** *(2026-10-10)*
- `storm/clipboard` in `MainActivity.kt` beside `storm/links`: the first
  clip item whose content URI names an `image/*` type, read through the
  content resolver, bounded at 12 MiB.
- The extra-keys **Paste** asks for an image first, exactly as the desktop's
  paste keys do; without one it pastes text as before. Drops on Android go
  through `desktop_drop` (85f).
- **Not built: Gboard's image insertion.** xterm2 5.2's own input
  connection (`CustomTextEdit`) declares no `allowedMimeTypes` and has no
  `insertContent`, so the keyboard never offers an image to the terminal.
  It needs a fork of xterm2 (MIT, the freeze's sanctioned path, but a
  decision: the freeze reserves the fork for its revisit triggers). Left for
  the operator. Android's long-press paste is the Paste key here: the
  terminal has no system text-selection menu.
- Tests: `image_paste_phone_test.dart` (Paste with an image stages it and
  types the path; without one nothing is staged). A malformed answer to the
  stage now fails the paste instead of throwing. Client 975. The Kotlin is
  compiled by `acceptance.yml`.

**85, the real run** *(2026-10-10, on a Runtime Host, the stack's head)*: a
real `storm-server` and `storm-runtime`, a `claude-code` session (Claude
Code 2.1.296), driven only through Storm's REST and SSE surface. A 32×32 red
PNG posted to `…/terminal/images` was staged in the session's inbox; its
path, bracket-pasted, became **`[Image #1]`** with the path not on screen;
asked its colour, the model answered **"Red"**; the session record took
Claude's own title, **"Single fill colour in image"**, activity `working`.
A bare script must answer the terminal's device-attributes query, as the
app's emulator does — without it Claude Code's parser swallowed the first
arrow key.

**86. System back on Android 16: every page claims it while it has
somewhere to go.** *(2026-10-10, the operator's phone report)*

**The report.** On the phone, back from a note left the app instead of
going to its folder; back from a session left the app instead of going to
the Agents list; and the places bubble's Agents opened the last session
instead of the list.

**The cause.** The app targets SDK 36 (Flutter 3.44's default), so Android
16 runs **predictive back**: the platform asks the app *in advance* whether
it handles back (`SystemNavigator.setFrameworkHandlesBack`), and closes the
app when it says no. Flutter says yes only when a navigator can pop or its
top route blocks the pop. Storm reaches most pages with `go`, so nothing can
pop, and the Q2 logic in `AppShell`'s `BackButtonListener` — correct, and
covered by `back_navigation_test` — was never asked. The tests drove
`handlePopRoute` directly, which skips the platform's question. A second
layer: when a `go` swaps one shell for another (Notes → a session), the
outer navigator reports for its own top page *after* the leaf, so a leaf's
claim alone loses.
**The places bubble** went to the remembered Agents location, which after
opening a session is that session — full screen on a phone.

**The fix.**
- `LogicalBack` (`app_shell.dart`): a `PopScope` that blocks the pop while
  `logicalParent` has somewhere to go and the page was not pushed, and
  goes there when the pop is attempted. Every signed-in leaf page and every
  shell around them (`AppShell`, `VaultShell`, `AgentsShell`,
  `SettingsShell`) is wrapped, so whichever navigator reports last says yes.
  Pushed pages still pop; the vault root still says no, so back there
  leaves the app as Q2 says.
- The places bubble's Agents goes to `/agents`, the list where sessions are
  and new ones start. The desk rail keeps the remembered location: its list
  is always beside the session.
- Test: `predictive_back_test.dart` records what the app tells the platform
  (`setFrameworkHandlesBack`) under Android, then backs: a note opened with
  `go` claims back and returns to its folder; a session claims back and
  returns to the list, and the list to Notes; the bubble opens the list.
  All three fail without the fix. Client 978.

*Revisit if:* Flutter changes how predictive back is negotiated; Storm opts
out of it with `enableOnBackInvokedCallback="false"` (which Android 16
ignores at target 36 anyway).

**86a. Two live-test races, found verifying `staging` after the merge.**
*(2026-10-11)* Each failed once in three full `make test-live` runs on
`990a1b8` and passed 3/3 alone; both are in test code older than D15.
- **`gateway_e2e.py` R7** waited 30 s for the host to reconnect. The host's
  backoff doubles from 1 s to a 60 s ceiling, and R7 restarts the server
  right after R5/R6 did, so the host can be waiting longer than that. All
  three reconnect waits now allow `RECONNECT = 90` s, as AC-R1 in
  `agent_e2e.py` already did.
- **`agent_e2e.py` AC-F6** waited for the second host to be `online`, then
  launched on it; a launch needs the capabilities its `hello` brings a moment
  later, and answered 503 without an `id`. The wait now requires both.
- Neither change can pass a real failure: one wait is longer, the other's
  precondition stricter. Verified: agent 93 and gateway 87, three runs each.

**87. xterm2 is forked for the keyboard's images.** *(2026-10-11, the
operator's approval of 85g's open decision)*
- **Why now.** The freeze kept the fork for AC-Q5's revisit triggers; the
  operator chose it for Gboard's image insertion, which 5.2 cannot take: its
  input connection declares no `allowedMimeTypes` and drops `insertContent`.
- **The fork is vendored**, `apps/client/third_party/xterm2`, a path
  dependency at `5.2.0+storm.1`: one commit of 5.2.0 as published, then the
  patch on its own, so a rebase onto a later xterm2 is that one diff. No
  second repository to keep. The analyzer skips it, as it skipped the
  package in the pub cache.
- **The patch** gives `TerminalView` Flutter's own
  `contentInsertionConfiguration` and passes it to `CustomTextEdit`, which
  declares its types on the connection, reopens the connection when they
  change, and hands `insertContent` on. Nothing else in xterm2 changes.
- **Storm's side** stays behind the terminal-surface boundary:
  `StormTerminal.onKeyboardImage` (PNG, JPEG, GIF, WebP), set by
  `SessionController` to the same stage-then-type flow as a paste (85f). A
  read-only terminal offers no types. The keyboard's `content://` URI is
  never typed.
- Tests: `keyboard_image_test.dart` drives the engine's `commitContent`
  message: the types reach `TextInput.setClient`, an image is staged and its
  path typed, an empty or non-image insertion stages nothing. With the
  forwarding removed, the staging test fails.
- **Not verified on a device**: Gboard on Android is the operator's check.

**88. A Mac host's first run: a first workspace, and image paste that fails
fast.** *(2026-10-11, the operator's first run of a 0.7.0 Runtime Host on a
Mac; vault: *Storm/Agent Runtime/Issue — Mac first run- no workspace, image
paste to an old host*)*
- **What happened.** The installer enrolled the Mac, but the launcher said
  "No workspaces on this host": nothing made one, or said where one goes.
  Pasting an image showed "pasting" for about 30 s and then failed. The
  session was on codebox, which runs runtime 0.4.0 and never answers
  `terminal.image`, so the server waited out `IMAGE_DEADLINE` (504).
  Meanwhile Claude Code said "no image found": a held Ctrl+V repeats, a
  `KeyRepeatEvent` skipped the image check (it only took `KeyDownEvent`),
  and xterm2 sent the repeat as `^V`, which made Claude Code look in the
  host's own empty clipboard.
- **An `images` capability.** The runtime says `"images": true` in
  `hello`; a host reporting version 0.7.0 or later counts too, since 0.7.0
  stages images without saying so (`Capabilities::stages_images`). For a host
  that does neither, `stage_image` answers **409** at once
  ("this host's runtime (0.4.0) cannot take images: upgrade it to 0.7.0 or
  later"), and nothing is admitted. The client's paste chip already shows
  the server's message. An offline host still gets 503 from `send`.
- **A held paste chord is swallowed.** A `KeyRepeatEvent` of a paste chord
  (when the terminal pastes images) is handled and sends nothing. One press
  still pastes once, and Ctrl+V with no image is still `^V`, as before:
  vim's block select and a shell's quoted insert need it.
- **The launcher says where a workspace goes.** The runtime also reports
  `workspace_roots` (the server carries it in `Capabilities`). A host with
  none shows its folder (`/Library/StormRuntime/workspaces` on a Mac) and
  **Check again**, which re-reads the host's workspaces.
- **The installer offers a first workspace.** After enrollment, an
  interactive run asks for a folder name or a git URL to clone (Enter
  skips), only when the runtime has none. `--workspace NAME|URL` does it
  without asking; a fresh `--yes` install without it says how in the plan's
  notes. On macOS it is made as the operator (the install's inherited ACL,
  and their git credentials); on Linux as `storm-runtime` (the folder is
  0750, that account's), so that step is `[sudo]`. Git never prompts: a
  clone that needs a login fails and says so.
- **Not in this change:** `curl … | sh` showing nothing on macOS (vault:
  *Storm/Installer/Issue — macOS curl | sh shows nothing*) has no fix yet.
- Tests: server `an_old_host_is_refused_an_image_at_once`; client a held
  Ctrl+V pastes once and sends no `^V`, and the launcher's empty host and
  Check again; installer bats for `--workspace` (args, plan on both OSes,
  runs that make, clone and keep) and the interactive prompt (re-asked,
  skipped, kept), and the pty run types a workspace. The installer e2e jobs
  pass `--workspace scratch` and check the folder on Ubuntu and macOS.

**89. Resume reopens the agent's own conversation.** *(2026-10-11, the
operator: an ended Claude Code or OpenCode session's Run again started a new
conversation, though OpenCode itself printed `opencode -s ses_…` to continue
it)*
- **The host learns the id while the session runs, never from the
  terminal.** The exit text is printed only on a clean exit, and a `/clear`
  or `/new` changes the conversation mid-run. `apps/runtime/src/harness.rs`:
  - **Claude Code**: a fresh session starts with `--session-id <uuid>`, so
    the id is known before the agent runs. Every session also gets
    `--settings` with a `SessionStart` hook that runs `storm-runtime
    harness-session`; it fires on startup, resume, `/clear` and compaction
    with the current `session_id`. The hook prints nothing (its output would
    reach the agent) and always exits 0.
  - **OpenCode** cannot be given an id. `OPENCODE_CONFIG_DIR` points at the
    session's directory, which holds a plugin: on `message.updated` it runs
    `storm-runtime harness-session <id>` for the session the message is in,
    once per change, skipping subagent sessions (those with a `parentID`).
    The variable adds to the operator's config rather than replacing it, and
    it loads with AM32's `OPENCODE_DISABLE_PROJECT_CONFIG=1` and
    `XDG_CONFIG_HOME` in place (checked on opencode 1.18.31).
  - `harness-session` sends the id to the daemon socket with a per-session
    handle (`STORM_HARNESS_HANDLE`), like `mcp-bridge`. The host takes it only
    for a handle it issued and only as an id the provider could make: a
    lowercase UUID, or `ses_` plus 1–64 alphanumerics.
    `docs/harness-session-vectors.json` holds both crates to the same rule.
- **Reported like the title.** `harness_session` rides the status posts as
  it changes, the ending and `hello`, so a crash or host restart doesn't lose
  it. The host says `"resume": ["claude-code", "opencode"]` in its
  capabilities.
- **A resume is its own launch.** `POST /v1/agent/sessions` with
  `resume_of`: `ops::launch_session` takes the old session's host, workspace,
  agent, note (as it was then) and write vault, whatever else the request
  says. The write vault is still checked. The manager refuses (409) a
  session that is running, has no conversation, sits on a host that doesn't
  list its agent under `resume`, or whose agent is gone (never a fallback),
  and a conversation a live session already holds. The new session records
  `resumed_from` and starts with the id already stored, so a second resume
  is refused before the host reports anything. `start` carries
  `resume: <id>`. The runtime runs `claude --resume <id>` or
  `opencode -s <id>`, and **not** the opening prompt.
- **The client.** An ended session shows **Resume** (no launcher) before Run
  again, which still starts fresh. When its host is offline, its runtime is
  too old or the conversation is open elsewhere, Resume is hidden and a line
  under the header says why. A resumed session leaves the Ended list; its
  conversation is listed once, under its latest session.
- **Rejected:** parsing the terminal for "Continue opencode -s" (clean exits
  only, and wrong after `/new`); `opencode session list` after the end (a
  guess when two sessions share a workspace); reopening the old record
  instead of a new one (its run time, Wrote and ending are history).
- **Known limits.** The id is per host and per workspace, so a resume runs
  only there. Removing the agent's transcript on the host makes the CLI exit
  at once, and the session fails like any other start. Switching OpenCode
  to another session without sending a message is not reported. An agent
  that runs `opencode` itself inherits the variable, and its messages would
  report as the session's.
- **Upgrading.** Both `storm-server` and `storm-runtime` must be this
  version. `agent.db` gains `sessions.harness_session` and `resumed_from` by
  `ALTER TABLE` on start (additive). With an older runtime, Resume is hidden
  and says to update it.
- Tests: runtime `harness` (ids against the vectors, launch arguments, the
  plugin file, other providers untouched),
  `the_agents_session_id_is_reported_as_it_changes`,
  `a_resume_reopens_the_conversation_or_fails_to_start`; server
  `a_resume_reopens_the_reported_conversation`,
  `a_resume_is_refused_without_a_conversation_or_a_host_that_can`, the
  vectors, and `a_resume_reopens_the_conversation_where_it_ran` over REST;
  `gateway_e2e.py` runs the real hook against the real runtime and checks
  the relaunched agent's argv (7 checks); client Resume, the phone bar, the
  two reasons, and shell. **Against the real CLIs** (Claude Code, opencode
  1.18.31), using the arguments `harness::launch` produces: the hook and the
  plugin each reported the id once, and `--resume` / `-s` reopened the
  conversation (each recalled the earlier message).
- **Also:** a 2px gap between Agents sidebar rows (#145), so a selected and
  a hovered row no longer merge into one fill.

**90. The web client heals an empty stored server address, a sign-in can
never spin forever, and a real Firefox signs in on every PR.** *(2026-10-11;
vault: *Storm v2/Issue — Zen login hang*)*
- **The issue.** In the operator's Zen (Firefox engine, macOS), a correct
  password left the Sign in button spinning. The server logged every attempt
  as `login_ok`. Chrome, and fresh Zen or Firefox profiles, signed in
  normally.
- **The cause, found with a traced build on the LAN staging server.**
  `settings.save` ran with `configured=false` straight after a successful
  login. `isConfigured` is `baseUrl.isNotEmpty && hasSession`, and the
  profile's stored `storm.baseUrl` was `''`; the login screen's address line
  was blank. Builds before 2026-08-19 (`b031b3b`) stored an empty address on
  the web, and since then load used `prefs.getString(_kUrl) ?? origin`, which
  keeps a stored `''` forever. With an empty address the login request is
  relative to the page, so it succeeds; the router then never leaves `/login`,
  and `_busy` is only ever cleared on failure. Every device in that profile
  inherited it. A fresh profile, or another browser, never had it.
- **The fix.**
  - On load, an empty or blank stored address counts as none, so the web
    falls back to the page's origin. Native clients still need one.
  - The login screen stops spinning when a sign-in leaves the app
    unconfigured, and says so (`router_test`: a sign-in that leaves the app
    unconfigured says so, never spins). The test fails with the guard
    disabled.
- **Also fixed on the way: a native submit reloaded the app mid sign-in.**
  Flutter keeps text fields in a hidden `<form method="post" action="#">` and
  cancels its `submit` event. A native `form.submit()` fires no event, so the
  browser POSTed the page; `storm-server` answers that with `index.html`, and
  the sign-in in flight was cut off. That is **not** what the operator hit.
  But a password manager or the browser can do it, and the browser test
  proved it. `web/form_guard.js`, loaded before Flutter, routes such a submit
  on a form inside `<flutter-view>` through `requestSubmit()`.
  `test/web_form_guard_test.dart` keeps it wired.
- **The test.** `apps/client/test_browser/web_sign_in.py` (`make
  test-browser`, CI job `browser`). It runs a real `storm-server` on a
  non-loopback, insecure origin as on a LAN, and drives headless Firefox over
  WebDriver BiDi, a fresh profile per case. Each case must reach the app with
  its token stored after a single page load:
  - Enter alone
  - a native `submit()` just after Enter
  - a native `submit()` as the login request goes out
  - `requestSubmit()`
  - **a profile that stored an empty server address**
  - an inline menu that keeps itself last in `<body>`
  - a paired browser whose session is gone signing in again

  The empty-address case failed exactly as the operator saw: on `/login`,
  token stored, one page load. The native-submit cases failed too. After the
  fixes all 8 pass, in Zen 1.22.1b locally and in the runner's Firefox 157.
- **Ruled out on the way:** password managers (the operator tested Bitwarden
  off), WebGL (hangs with `webgl.disabled`); and, in fresh profiles one
  setting at a time, resist-fingerprinting, fingerprinting protection,
  always-private browsing, first-party isolation, partitioned or blocked
  cookies, IndexedDB, `localStorage` or service workers off, wasm off, and
  site data left by the Oct 8 build.
- **How it was found:** a throwaway build with a sign-in trace and a
  once-a-second heartbeat (local branch `diag/firefox-login-trace`, never
  merged). The heartbeat proved the page was alive, so the trace's
  `configured=false` was the answer. Worth repeating for the next "it just
  spins" report.

**99. Windows: the Runtime Host as a Windows Service, and the server per
user (M26).** *(2026-10-12, the operator's choices)*

**The spec.** D16 in *Agent Runtime/Decisions*, full text in *Agent
Runtime/Windows Runtime Host* (AM48–AM57, AC-W1–W10), and *Installer/Windows
Server* for the server (W-S1–S5, AC-WS1–WS6). The operator chose:
- **the host runs as the virtual service account `NT SERVICE\StormRuntime`**:
  no password to manage, never an administrator, its own SID, no access to
  any person's profile. The counterpart of `_stormruntime` (D14).
- **the server runs per user at logon** (a Scheduled Task, data in
  `%USERPROFILE%\Storm`), as 84d's macOS server does.
- **zip + `install.ps1`**; winget later.
- **WSL2 was ruled out** for the reason Docker was on macOS (83): agents
  lose the Windows toolchains, and files across the VM are slow.

**What the type check found before any code** (`cargo check --target
x86_64-pc-windows-gnu`, zig as the C compiler for `ring`): the server
compiles for Windows except `install_macos.rs`; the runtime had 45 errors,
every one in the PTY, process groups, signals, uids and the Unix socket.

**The slices**, stacked PRs to `staging`:
1. **The carrier, the platform module and the service, with Windows CI**
   (99a).
2. **The client's Windows enroll steps** (AM56).
3. **The server on Windows** (W-S1–S4).
4. **Release, `install.ps1`, docs, and the Windows acceptance harness**
   that installs, enrolls, revokes and uninstalls the real service on
   `windows-latest` (AM57, W-S5, AC-W1–W9).

*Revisit if:* a host must confine agents more tightly than an account
(AppContainer breaks the CLIs today); ConPTY's input stops draining into
conhost's buffer (99a); Windows gains a polite signal for a process off the
console (AM52's hang-up).

**99a. Slice 1: ConPTY and a Job Object per session, one carrier interface,
a named pipe, and the service.** *(2026-10-12)*

**The carrier is per platform; the policy is not.** `pty.rs` became
`pty/{mod,unix,windows}.rs` with one interface: `spawn`, a bounded
`Input::write`, `resize`, `Child::wait`, and a `Group` that hangs up, kills
and says whether anything is left. `cli.rs` keeps every rule of §7.3 and
AM37 (the end order, the grace, "`ended` means the session is empty") and
no longer names a signal or a pid. The Unix side is the old code moved
whole: the 17 `tests/cli.rs` and every lib test pass unchanged.

**Windows (AM51, AM52):**
- **ConPTY** carries the terminal. The process is created with
  `CreateProcessW` directly (the pseudoconsole is a `STARTUPINFOEX`
  attribute std cannot pass), **suspended, put in its job, then resumed**.
  The standard handles are explicitly invalid, so a host whose own are
  redirected (CI, a log) does not hand them to the agent.
- **A Job Object per session**, `KILL_ON_JOB_CLOSE`, no `BREAKAWAY_OK`.
  Hang-up is closing the pseudoconsole (`CTRL_CLOSE_EVENT`), on its own
  thread because closing can wait for the reader; kill is
  `TerminateJobObject`; "anything left" is the job's active-process count.
  The host dying, however, kills every job: stronger than the Unix sweep.
- **The bounded write** is an overlapped write to a one-instance named pipe
  (the pseudoconsole reads its other end), waited for with the deadline's
  remainder and cancelled with `CancelIoEx` past it. **Expected on a real
  machine:** conhost drains its input pipe into the console's input buffer
  whether or not the agent reads, so the deadline should rarely fire.
- **Exit status** is the exit code; there are no signals (a crash is its
  `NTSTATUS`).
- **Command lines** are built by `platform::wincmd`, pure and tested on
  Linux against a reimplementation of the C runtime's parser. A `.cmd` /
  `.bat` CLI (npm's shims) runs through `cmd.exe /d /e:ON /v:OFF /c`, and an
  argument holding `" % ! ^ & | < >` or a line break is refused, never
  escaped (the BatBadBut class). `PATHEXT` drives the lookup, dir by dir.

**The platform module** gained `posix.rs` (what Linux and macOS share: the
node name, the PTY wait, session members, the tagged-process scan, the
sweep), `windows.rs`, and two pure modules compiled everywhere:
- `winsvc.rs`: the layout under `C:\ProgramData\StormRuntime`, the
  `icacls` plan (protected DACLs; only the workspace root names the
  operator), the service's environment, the default `PATH`, and AM54's
  root refusal (case-insensitive components, `\\?\` stripped);
- `wincmd.rs`: command lines, the batch guard, the environment block,
  `PATHEXT`.

**Private files** go through `platform::{create_private_dir,
tighten_private_dir, open_private, is_private}`: `0700`/`0600` created with
the mode on Unix, exactly as before (A2); a protected DACL of SYSTEM,
Administrators, this account and the service account on Windows (AM49). A
provider env file is refused on Windows when `Everyone`, `Authenticated
Users`, `Users`, `Guests` or `INTERACTIVE` may read it. Tests assert either
through `platform::assert_private`.

**The local channel** is `platform::ipc` (AM55): the same `run/mcp.sock` on
Unix; on Windows a pipe named from a digest of that path, DACL of the
account and SYSTEM, remote clients rejected, first instance only. `host`,
`bridge` and `harness` no longer name a socket type.

**The service (AM49, AM50)** is `platform::windows_service`, executing
`winsvc`'s pure plan, elevated only and every tool by absolute path:
- **`install`** creates the layout, copies the binary to `Program Files`,
  writes the config once, creates or updates `StormRuntime` as `NT
  SERVICE\StormRuntime` (manual start until enrolled; restart 10 s after a
  crash, three times; a clean stop with a code is not a failure; a 20 s
  pre-shutdown), sets its registry `Environment`, then the `icacls` plan
  (after the service exists, so the account resolves) and `safe.directory`.
  Re-running stops, replaces and restarts.
- **`enroll`** with the default state, elevated, makes it automatic
  (delayed) and (re)starts it.
- **`storm-runtime service`** is the SCM's entry: a log file in `logs\`
  (one `.old` past 10 MiB), the exclusive-account guard, then
  `serve::run --exclusive-account` stopped by Stop or PreShutdown. Started
  unenrolled it stops with code 2; revoked, with code 3, which recovery does
  not restart (AM36).
- **`uninstall [--purge]`** stops and deletes the service; workspaces are
  never touched.

None of it has run yet: slice 4's harness drives it on `windows-latest`.

**`serve` is one routine** (`serve::run`) whose stop is a future: SIGTERM
or SIGINT from `main` on Unix, the console events by hand on Windows, the
SCM's Stop/PreShutdown. A revocation is now told apart from
another run failure, but both still exit 3 on Unix, as before.

**Tests.** `tests/windows.rs` (the carrier: output and exit code, input,
resize, environment and workspace, Stop ending a detached child through
the job, `ended` waiting for an emptied job, input after the end, a missing
CLI, a `.cmd` CLI through `PATHEXT`, the batch guard, PowerShell as the
shell, the host name); `platform::windows` (the sweep's guard and target
list, the DACL helpers, AM54 on real paths, the default `PATH`);
`platform::ipc` (a round trip on both platforms; on Windows, no second
listener). `tests/cli.rs` and the POSIX-shell host tests are `cfg(unix)`;
`lifecycle.rs`'s revocation and enrollment run on Windows too (its config
names no root there: the runner's temp is under `C:\Users`, which AM54
refuses).

**Verified here:** clippy `-D warnings` for `x86_64-linux`,
`x86_64-pc-windows-gnu` and `aarch64-apple-darwin` (zig as the C compiler);
the Linux suite. **The new `runtime (rust, Windows)` job on `windows-latest`
is the first run of any of it.**

**The first Windows run** (MSVC, Server 2022): clippy `-D warnings` passed;
the lib suite had 75 passed and 8 failed, none in the new carrier:
- four `launchd` tests built macOS paths with `Path::join`, which writes `\`
  on Windows: they are macOS layout tests, now `cfg(unix)`;
- three host tests wrote a temp path into a TOML basic string, where
  `C:\Users` is an invalid `\U` escape: now `{:?}`;
- one config test used a temp dir under `C:\Users` as a workspace root,
  which AM54 rightly refuses: Windows uses `%ProgramData%` there.

**99b. Slice 2: the enroll sheet's Windows steps (AM56).** *(2026-10-12)*

The sheet's `ChoiceChips` gain **Windows**, preselected when the client is
on Windows (native or web); Linux stays the default elsewhere. Its three
commands are for an **elevated PowerShell**:
1. download `storm-runtime-windows-x86_64.zip` from the latest release
   (`releases/latest/download/…`, so the asset name carries no version)
   and expand it into `%TEMP%`;
2. `& "$env:TEMP\storm-runtime\storm-runtime.exe" install`;
3. `& "$env:ProgramFiles\StormRuntime\storm-runtime.exe" enroll` — the
   installed copy, the one the service runs; enrolling starts it.

The footnote names the service, its virtual account and
`C:\ProgramData\StormRuntime\workspaces`. Tests: Windows never shows a
POSIX command (`sudo`, `brew`, `systemctl`, `/Library/`), each platform has
exactly one enroll step and never takes the string as an argument, a
Windows client opens on Windows, and the switch reaches all three.
`flutter test test/agent_test.dart` (36) and `flutter analyze` pass here.

**99c. Slice 3: the server on Windows (W-S1–S4).** *(2026-10-12)*

- **`storm-server` is a library with two binaries.** `main.rs` became
  `lib.rs` (its `main` is now `pub`), and `main.rs` and
  `bin/storm-serverw.rs` only call it. **`storm-serverw.exe`** is the same
  program for the Windows GUI subsystem, like `pythonw`: a console program
  started at logon by Task Scheduler opens a console window (a Windows
  Terminal tab on 11), and no task setting hides it. A module `#[path]`-ing
  `main.rs` into a second binary would have broken every `crate::` path.
- **A global `--log-file`**, since `storm-serverw` has no terminal. Tracing
  now starts after the command line is parsed.
- **`install_windows`** (W-S1, W-S2): `up` copies both binaries and the web
  client into `%LOCALAPPDATA%\Storm`, writes `server.json`, makes the data
  root's DACL protected (the person's SID from `whoami /user`, SYSTEM,
  Administrators), and registers two tasks with `schtasks /xml` (UTF-16):
  **Storm Server** at this user's logon (`InteractiveToken`,
  `LeastPrivilege`, no time limit, restart every minute) and **Storm
  Backup** daily at 03:30 with `StartWhenAvailable`. Tasks sit in the root
  folder: a standard user may not create a task folder. `down` ends and
  disables both; `status` reads the task and `/health`; `uninstall` deletes
  them and the support folder, never the data. Health is a raw HTTP/1.0
  `GET` on a `TcpStream`, not curl. The firewall rule needs an
  administrator, so `up` prints the one `New-NetFirewallRule` line (W-S5;
  `install.ps1` will run it).
- **`storm-server backup`** (W-S3): the bash script's steps in Rust — the
  stored root (it wins over the flag), a fresh copy of the vaults (symlinks
  skipped, not followed), `backup_all` into `index\`, a verify open, then
  pruning of folders *named* as dates past `--keep-days`. Linux and macOS
  keep the script.
- **Windows filename rules** (W-S4): `vault::windows_name_problem` refuses
  `< > : " | ? * \`, control characters, a trailing dot or space, and
  `CON`/`PRN`/`AUX`/`NUL`/`COM1–9`/`LPT1–9` with any extension. `resolve`
  applies it only on Windows; the function is tested everywhere.
- **Dispatch:** Unix keeps its `cfg!` guards, so Linux still compiles and
  tests `install_macos`; Windows gets `#[cfg(windows)]` arms.
  `install_windows` is compiled everywhere for its tests, and `install.rs`'s
  systemd commands are allowed to be unused on Windows.
- **CI:** `server (rust, Windows)` on `windows-latest`.

**Verified here:** clippy `-D warnings` on Linux and `x86_64-pc-windows-gnu`;
the Linux suite (579 lib tests and every integration test). The new tests:
the task XML (logon/daily, principal, quoting through a reimplemented C
runtime parser, XML escaping), the record and layout, the health probe
against a stub, backup pruning and copying, and the name rules.

**99d. Slice 4: release, `install.ps1`, docs, and acceptance on
`windows-latest` (AM57, W-S5, AC-W1–W9, AC-WS).** *(2026-10-12)*

- **Release.** `runtime-windows` and `server-windows` build on
  `windows-latest` and zip `storm-runtime-windows-x86_64.zip` and
  `storm-server-windows-x86_64.zip` (both binaries and the web client), each
  with a README. The names carry no version, so the enroll sheet's
  `releases/latest/download/…` link always resolves; both are in
  `checksums.txt`, and so is `install.ps1`, which Pages also serves beside
  `install.sh`. Not code-signed (SmartScreen warns); x86_64 only (Windows on
  Arm emulates it).
- **`deploy/install.ps1`**, deliberately not a port of `install.sh`'s TUI:
  `-Server`, `-Runtime`, `-Version`, `-FromDir`, `-Workspace`, `-Yes`,
  `-DryRun`, or a three-way menu. It checks each zip against
  `checksums.txt`, runs `storm-server up` as the person, adds the one
  private-profile firewall rule when elevated (W-S5; otherwise prints it),
  shows a pairing code while the server has no account, runs `storm-runtime
  install`, and enrolls through a pipe from the server beside it or a
  `-AsSecureString` prompt, never an argument (84e/84f). A first workspace is
  a folder or a non-interactive `git clone`. It fails with `throw`, never
  `exit`, since `irm | iex` would close the window.
- **Acceptance.** `.github/workflows/windows-acceptance.yml` (on
  `acceptance/**` and dispatch) runs three jobs:
  `apps/runtime/tests/windows_acceptance.py` (AC-W1–W9 against the real
  service: `sc.exe`, `icacls`, process command lines through CIM, Stop,
  `taskkill /f` and recovery, the pipe refusing even an administrator,
  revocation's code 3 and no restart, uninstall keeping workspaces),
  `tools/windows-server-acceptance.py` (the logon task, a windowless
  `storm-serverw`, the protected data root, `backup`, down/up/uninstall),
  and `install.ps1` end to end from zips built in the job.
- **Docs.** `deploy/README.md` gains a Windows column and a Windows section;
  `README.md` the PowerShell one-liner; `CLAUDE.md` the invariants.

**Real-Windows acceptance (`windows-latest`, Server 2022), three rounds.**
- **Round 1: 32 passed, then stopped.** Install, enrollment and provider
  discovery passed whole. The first Claude Code session never started:
  **decision 89's `--settings` hook was inline JSON**, and npm installs
  Claude Code on Windows as `claude.cmd`, whose arguments the batch guard
  rightly refuses when they hold `"`. Every Claude Code session on such a
  host would have failed. **Fix (99a):** the settings are a private file in
  the session's directory (`gateway_e2e.py` reads it there; 95/95 on Linux).
- **Round 2: 64 passed, 4 failed.** Two product defects:
  - **A session's working directory was the canonical `\\?\C:\…`**, which
    `cmd.exe` and many tools refuse. **Fix:** the carrier passes the plain
    spelling (`winsvc::normalize`); `a_canonical_workspace_is_a_plain_working_directory`.
  - **`Stop-Service` reported a failed stop:** the service stayed RUNNING
    through its 13 s shutdown. **Fix:** STOP_PENDING with a 20 s hint the
    moment Stop or PreShutdown arrives.
  
  The other two were the harness's (a batch file's arguments arrive quoted;
  a refusal is read before the marker, not at the prompt).
- **Round 3: AC-W1–W9 68 passed, 0 failed, 3 manual** (a logged-in CLI
  taking a prompt, the phone handoff, a reboot). That includes End,
  `Stop-Service` and `taskkill /f` each ending a detached child through the
  job, recovery restarting the service, the pipe refusing even an
  administrator, revocation's code 3 with no restart, and uninstall keeping
  the workspaces.
- **The server's per-user task** did not start the server: the runner has
  an active console session, the task ran, and `storm-serverw` exited **2**
  (clap's usage error) before it opened its log. The task's arguments began
  with the global `--log-file`, and **`normalize_argv`'s legacy shim, seeing
  a leading flag, inserted a second `serve`.** **Fix (99c):** each task names
  its subcommand first, and `every_task_runs_a_command_line_that_parses`
  runs both tasks' arguments through the real `Cli` and shim. Two more, the
  same race: `schtasks /end` returns before the server exits, so `uninstall`
  could not delete it and an upgrading `up` could not replace it; both now
  retry for up to 15 s.
- **`install.ps1`** enrolled through `host-enrollment` before the fresh
  server had an account (84g), and piped the refusal into `enroll`. It now
  takes **`-PasswordStdin`** (as `install.sh --password-stdin`), waits for
  the first device to pair when interactive, and with `-Yes` alone says how
  to enroll later.
- **Final round: all three jobs green.** AC-W1–W9 **68 passed, 0 failed, 3
  manual**; the server's Scheduled Task **28 passed, 0 failed** (a
  windowless `storm-serverw` at logon, its log, the protected data root,
  `backup`, down/up, uninstall keeping the notes); `install.ps1` end to end:
  server up, service running, the host enrolled through the pipe from the
  server beside it, the first workspace, the private-only firewall rule, and
  a re-run upgrading in place.
- **Still manual, for a real Windows 11 machine:** a logged-in Claude Code
  and OpenCode taking a prompt, the phone handoff, a reboot, logging off and
  on (AC-WS1), and Notepad edits reaching the watcher (AC-WS3).

**103. The terminal follows the device being used: opening a session or typing
in it re-claims a PTY sized elsewhere.** *(2026-10-12; the operator: "huge
padding at the bottom" of a session on the phone)*
- **What happened.** On the phone, Claude Code's status bar sat about four
  rows above the bottom of the terminal, with blank rows under it. In a
  correctly sized terminal, Claude Code puts its status bar on the last row
  (checked in tmux at 66×50). So the host's PTY was shorter than the phone's
  grid, and the agent drew for that height.
- **Why.** The server applies whichever client sent the last resize. A
  client sent one only when its own grid changed size, or when its terminal
  took keyboard focus (freeze §11.4's "the PTY follows the client being
  used"). A resize from another device (the session was also open on the Mac),
  or one of the phone's own that was lost (the client never retries), stayed
  in place until the phone's grid happened to change again. Reading a session
  on the phone claimed nothing. The xterm2 emulator was ruled out: a fill,
  shrink and grow keeps the content anchored to the bottom.
- **The fix.** `SessionController` tracks the host's size: it is updated from
  each status event, and from each resize it sends. It re-sends its own size
  (with `focus`) when the two differ:
  - **on opening a session**: the first status event;
  - **on typing**: input is the signal that this device is the one in use.
  
  Another device's resize alone never triggers a claim, so two open devices
  never fight over the PTY; the one being typed on wins.
- Tests (`agent_test`): opening a session another device sized claims it;
  typing claims it back, only typing does, and only once. Both fail on the
  previous controller.

**104. The phone session's header sits between the corner bubbles; the
details sheet takes everything the chip row held.** *(2026-10-12; the
operator: agent pages give too much of the screen to the top bar)*
- **What changed.** The terminal now starts at 84 px from the top instead of
  about 200.
  - The header sits in the bubble row, 76 px in from each edge. Line 1: status
    dot, name and `▾`. Line 2, in mono: status · agent · context note ·
    `wrote n`. Both lines end with an ellipsis, and the dot and caret never
    shrink.
  - A tap anywhere on the header opens the details sheet. The sheet now starts
    with the full name, wrapped, and the status chip.
  - While a session is open, the left bubble shows a back chevron that returns
    to the Agents list. Everywhere else it is still the place picker, including
    on a context note pushed over the session.
  - Gone: `‹ Agents`, the 20 px title, the header's status chip, and the chip
    row (context, Wrote n, Details).
- **What did not change.** This is Claude Design's change to the handoff,
  adjusted to the changes made since v2: Resume still comes first in the ended
  bar (89); the extra keys are still a keyboard accessory; the desk header and
  inspector are untouched; the dot uses `AgentMark`'s colours and pulse (85);
  the other screens keep the 28 px bubble gap.
- **How.** `StormChrome` gained `bar` (a header in the bubble row) and `onBack`
  (the left bubble becomes `BackBubble`).
- Tests (`ui_polish_test`, *the compact phone session header*):
  - At 390 × 844, the terminal's top is 84 and its first line is within 100.
  - A 60-character name ellipsizes between the bubbles, and the sheet shows it
    whole.
  - The meta line on an ended session, with its bar.
  - The back bubble in a session, and the place picker elsewhere.
- The acceptance shots `phone-06` to `phone-09` are retaken.

**92. SRP §5.2 ends a request body with an empty `0x01` frame.** *(2026-10-12,
P2 from the relay TODO, measured 2026-10-03)*
- **The defect.** §5.2 defined body chunks but never their end. storm-server
  dispatches `GET`/`HEAD`/`OPTIONS`/`TRACE` at the head and waits for a
  zero-length `0x01` frame for everything else; the Dart client sent no
  terminator. **Every relayed POST or PUT with a body hung until the
  caller's timeout** — every note save, create and move from the app over a
  relay. Confirmed against a real `storm-relay` + storm-server from the Dart
  client: `createNote` and `saveNote` timed out at 10 s, while a POST with
  no body happened to work, because `http`'s `finalize()` yields one empty
  chunk and the client sent it as a frame.
- **The rule, now normative in `docs/srp-v1.md` §5.2:** a client sends
  exactly one empty `0x01` frame after the last non-empty chunk of **every**
  request, a `GET` included; never an empty frame for anything else (an
  empty chunk from the body source is skipped); nothing after it. The relay
  forwards it verbatim. An origin treats it as end of input and **may**
  dispatch the four body-less methods at the head, which keeps clients from
  before this rule working for reads. `CLOSE` is not an end of body: it
  abandons the stream. No signed bytes changed; the vectors are untouched.
- **Why one rule for every method rather than "only bodied ones":** no party
  has to decide from the method whether a body is coming, and the existing
  server already accepts it. The server is unchanged.
- **Why every unit test was green:** the client's fake relay answered 20 ms
  after the head, not at end of body. It now serves on the empty frame, as
  the origin does.
- Tests: client `test_live/relay_live_test.dart` (a GET, a POST with a JSON
  body, a PUT with `base_version` and a body-less POST, through a real
  relay; the two bodied ones fail on the old client), run by `make
  test-live`, which now builds and starts `storm-relay` on `RELAY_PORT`
  (8487) and registers the first server with it; client unit tests
  `every request ends its body with one empty 0x01 frame (§5.2)` and `an
  empty chunk mid-body is skipped, not sent as end of body` (both fail on
  the old client, as does the existing body round-trip once the fake waits
  for the end); relay
  `an_empty_request_body_frame_reaches_the_server_as_end_of_body`.
- **Unblocks** relay TODO §5 (MCP over the tunnel: tool calls are bodied
  POSTs). P1 (`TCP_NODELAY`) is separate.

**95. MCP crosses the relay unchanged; a body that outruns its handler
closes the stream.** *(2026-10-12, relay TODO §5, stacked on 92)*
- **Verified, not assumed (R10, R13).** A tunnelled `/mcp` call is the same
  request on the same `Router`: `initialize`, `tools/list`, `get_vault` and
  `get_note` answer byte-for-byte as on the LAN; an `update_note` written
  against a stale `base_version` over the relay diff3-merges with a LAN edit;
  a 4xx inside a tool stays HTTP 200 with `isError: true`; `mcp_enabled` off
  is the same 404 and `mcp_writable` off lists no write tool and refuses one,
  identically on both paths; an `stk_` key reaches `/mcp` over the relay and
  is still 401 on REST; the `Host` rewrite keeps rmcp's rebinding check
  passing on a concrete bind, where the relay's hostname is refused directly.
  MCP answers plain JSON (`with_json_response`), so "streaming"
  is the dispatch's: a response body leaves chunk by chunk, before the
  handler has produced the next one; the relay never times out an acked
  stream (`an_acked_stream_that_stays_silent_is_never_timed_out`).
- **The defect.** When a stream's request-body queue (32 chunks) filled
  because its handler had not started reading, the tunnel dropped the sender
  — which the handler reads as **end of body**, so it served the prefix as
  the whole request. A large note or attachment uploaded over a relay could
  be saved truncated with a 200. Now a full queue **aborts the dispatch and
  sends `CLOSE{stream_id}`**: the caller sees a transport failure, never a
  shortened request and never a tool error. A *closed* queue (the handler
  answered without reading the rest, e.g. a 401) still lets that response
  stand.
- **A request cut off mid-body does not run**: `CLOSE` before the empty
  `0x01` frame aborts the dispatch, and nothing is written.
- **Open — how an external MCP client reaches a relayed server.** Claude
  Code and friends speak HTTP, not SRP, so they cannot dial a relay. Not
  built here; the vault's *MCP Integration* says only "MCP Client → Storm MCP
  → Storm Relay". Recommended: a Rust SRP client in `storm-runtime` (which
  relay TODO §11 already needs for Runtime Hosts behind a relay) exposing a
  loopback `/mcp` that forwards over a trunk, verifying the server's identity
  as the app does. Rejected: an HTTP ingress on the relay itself — it would
  terminate the client's request in plaintext with no challenge-on-connect.
- Tests (`apps/server/src/relay/tests.rs`):
  `mcp_over_the_tunnel_answers_exactly_as_on_the_lan`,
  `a_relayed_mcp_write_merges_against_a_concurrent_lan_edit`,
  `a_tool_error_over_the_tunnel_is_still_a_tool_error`,
  `the_mcp_switches_gate_a_relayed_call_as_they_gate_a_lan_one`,
  `an_mcp_key_works_over_the_tunnel_on_mcp_and_nowhere_else`,
  `a_relayed_mcp_call_passes_rebinding_protection_on_a_concrete_bind`,
  `a_relayed_write_whose_stream_closes_before_its_body_ends_writes_nothing`,
  `a_body_that_outruns_its_handler_is_closed_not_truncated` (fails on the
  old code), `a_response_leaves_the_server_chunk_by_chunk`.
- *Amended by 102:* the queue's bound was 32 **chunks**, which shed an
  ordinary 20 MiB upload sent at line rate. The shed rule stands; the bound
  is now bytes.

**102. The tunnel buffers request bodies by bytes, not chunks.**
*(2026-10-12, amends 95)*
- **Why.** Measured through a real `storm-relay` once the Dart client split
  uploads into 256 KiB frames (100): a 20 MiB attachment arrived as 80 frames
  back to back, the attachment handler fell more than 32 chunks behind, and
  95's shed rule closed the stream — `no_response_head` for an ordinary
  upload. 95's bound counted chunks, so it measured how a client happened to
  frame a body, not memory.
- **Per stream: `STREAM_BODY_BUDGET` = `MAX_ATTACHMENT_BYTES` + 16 MiB.** The
  sender is never slowed (awaiting in the trunk's read loop would stall every
  other stream), so the buffer must hold a whole attachment arriving before
  its handler reads; the extra 16 MiB is one maximal WebSocket frame crossing
  the line. A larger body is the route's 413, not the tunnel's business.
- **Per trunk: `TRUNK_BODY_BUDGET` = 2 × `MAX_ATTACHMENT_BYTES`** (128 MiB) —
  two maximal attachments in flight at once. Without it the per-stream bound
  times the relay's 1000-stream cap is no bound. The stream whose frame would
  cross it is the one shed; the others are untouched.
- **Shed is still abort with `CLOSE`, never end of body** (95's invariant).
- **Accounting:** an unbounded channel per stream plus two counters. Only the
  trunk's read loop adds; the body's reader subtracts what it reads, and its
  drop (handler done, stream aborted, trunk gone) gives back what it never
  read. A send to a dropped reader takes its bytes back with the same `swap`,
  so they leave the trunk's count exactly once.
- *Revisit if:* SRP gains per-stream flow control (67), which would let a
  sender be slowed instead of buffered; or attachments grow past 64 MiB.
- Tests: `a_large_upload_at_line_rate_reaches_a_slow_handler_whole` (fails on
  95's code), `a_body_over_its_buffer_budget_is_closed_not_truncated` (95's
  test, now past the byte budget), and in `client.rs`
  `a_stream_is_refused_past_its_own_budget_not_truncated`,
  `the_trunk_budget_sheds_only_the_stream_that_would_exceed_it`,
  `read_bytes_free_the_budget_for_more`,
  `an_abandoned_body_gives_its_bytes_back_to_the_trunk`.

**98. Body frames are bounded; the live suites take a port and drive a real
relay further.** *(2026-10-12, relay completion loop.)*
- **Defect: one large download cut every relayed client off the server.**
  `serve_one` sent each handler chunk as one `0x02` frame, and an attachment
  is one chunk. Measured through a real `storm-relay`: up to 15 MiB crossed
  fine; at 17 MiB the relay refused the frame (the WebSocket 16 MiB default)
  by closing the **server's trunk**, so every relayed client lost the server
  until it re-registered (~5–6 s later). The server now splits a response
  into frames of at most `proto::MAX_BODY_FRAME_PAYLOAD` (256 KiB).
- **SRP §5.2 gains "Body frame size":** every sender MUST keep a frame under
  16 MiB and SHOULD split at 256 KiB, so streams on one trunk interleave.
- **Open — the Dart client's uploads.** `SrpHttpClient` sends a request body
  as one `0x01` frame, so an upload over 16 MiB closes that client's trunk
  (measured at 17 and 24 MiB). The fix is splitting in
  `srp_http_client.dart` at 256 KiB; left to the Dart trunk work (decision
  97) that owns the file. A skipped live test names it.
- **Decision 95's 32-chunk request queue holds for a chunking sender.** A
  15 MiB attachment sent as 60 back-to-back 256 KiB frames to the real
  attachment route lands whole
  (`a_chunked_upload_sent_back_to_back_lands_whole`), so splitting the Dart
  client's uploads will not trip it on that route.
- **Live suites take a port.** `e2e.py` reads `STORM_BASE`, and
  `live_server_test.dart` / `two_client_sync_test.dart` read the
  `STORM_BASE` dart-define; `make test-live PORT=… AUTH_PORT=… RELAY_PORT=…`
  now runs every suite off the defaults, so two checkouts can run it at once.
- **`relay_live_test.dart` against the real relay** now also covers the SSE
  change feed (a LAN edit reaches a relayed client), a multi-MB note, a
  12 MiB attachment round-trip, a 20 MiB download that leaves the server
  registered, and MCP `initialize` + `create_note` with an `stk_` key.
- §10's pubkey items were already covered in `apps/relay/tests/registration.rs`:
  `a_signature_by_the_wrong_key_is_auth_failed`,
  `tofu_binds_the_first_key_and_refuses_a_different_one_for_ever`,
  `a_tofu_binding_survives_a_relay_restart`,
  `an_allowlist_refuses_a_mismatched_key_and_an_unlisted_id_on_first_sight`.
- Tests: `a_response_larger_than_a_websocket_frame_crosses_in_bounded_frames`
  (fails on the old code: the fake relay's 16 MiB limit drops the trunk),
  `a_chunked_upload_sent_back_to_back_lands_whole`.

**97. The Dart trunk tells server offline from relay down, and drops a
silent relay.** *(2026-10-12, two gaps found by decision 94's §9 work)*
- **Server offline vs relay unavailable.** `SrpTrunk` turned every
  trunk-level `ERROR` into `relay_error`, and `ConnectionStatus` had only
  `offline`, so a relay saying `server_unreachable` and a relay that did not
  answer at all looked the same — the relay TODO's §4 box claiming the split
  was overstated. Now the trunk keeps the relay's code (`_relayErrorCode`,
  `SrpHandshakeFailed.code`, `lossReason` — the shape PR #153 also adds, so
  the two merge) and reports the server-side codes (`server_unreachable`,
  `server_timeout`, `trunk_lost`, §6) on `onServerUnreachable`, for the
  trunk or for one stream. `StormConnection` gains
  `ConnectionStatus.serverOffline` and a `statusChanges` stream; the status
  dot gets `DotStatus.serverOffline` (the same grey — nothing syncs — but
  the line reads "Server offline, relay reachable"). `offline` now means no
  path answered, the relays included.
- **The verdict can land after the race.** The relay holds `HELLO` up to
  5 s for a server mid-restart, and the race gives up after
  `publicRelayDelay` (2 s). The trunk is left alive on a failed race, so the
  relay's answer still arrives and moves the status `offline` →
  `serverOffline`; `SyncEngine` follows `statusChanges` and repaints. The
  trunk's handshake timeout goes from 5 s to 8 s so it outlasts the hold
  instead of racing it, and a handshake that times out now fails the trunk
  rather than leaving it dialling beside a second socket. `StormConnection`
  replaces a dead trunk instead of handing it out again.
- **A silent relay.** The trunk sent `PING` every 15 s but never required an
  answer, so after a network change or a sleep a half-open socket hung every
  relayed request until that request's own timeout. It now drops the trunk
  after `silenceLimit` (default three heartbeats, 45 s — the allowance the
  relay gives a server, §4.2) with nothing heard; any frame counts, and the
  relay answers every client `PING` (`connect.rs`). SRP-level, not
  WebSocket ping: it works on the web, where the browser owns WebSocket
  pings. Every open stream fails with `trunk_lost` (a body error, never a
  short body read as success), and the change feed's error sends
  `SyncEngine` through its reconnect, which re-races from scratch.
- Tests (`test/srp_liveness_test.dart`): the code survives (`relay_error`
  on `onTrunkLost`, `server_unreachable` as the code and on
  `onServerUnreachable`), a `rate_limited` refusal is not server offline,
  `StormConnection` reports `serverOffline` both when the refusal is
  immediate and when it lands after the race gave up, `reset` forgets it,
  the dot and its line; a silent relay fails the request in flight with
  `relay_silent` (hangs on the old trunk), a relay answering heartbeats is
  kept, a timed-out handshake does not dial twice. **PR #151's skipped
  test** (`server offline behind a live relay is told apart…`) expects the
  code on `onTrunkLost`; on rebase it should read `lossReason` instead.
- **Not changed:** a trunk-level `ERROR{rate_limited}` (the relay's refusal
  of one `OPEN_STREAM`) still kills the whole client trunk (67's open item).

**100. The Dart client sends a request body in frames of at most 256 KiB.**
*(2026-10-12, relay TODO §4; found by decision 98, PR #157.)* `SrpHttpClient`
sent each chunk `finalize()` yielded as one `0x01` frame — in practice the
whole body — so an upload over 16 MiB exceeded the WebSocket frame limit and
the relay closed **that client's trunk** (measured at 17 and 24 MiB through a
real `storm-relay`). `SrpTrunk.sendBody` now splits every chunk at
`maxBodyFramePayload` (256 KiB, `srp_codec.dart`), the size the server uses
for response bodies (`proto::MAX_BODY_FRAME_PAYLOAD`, 98). #150's end-of-body
rule is unchanged: non-empty chunks only, then exactly one empty frame. The
client never sends `0x02`, and receiving a body as many small frames is what
98's live download tests already exercise.
- Test: `a body over 16 MiB goes out in bounded frames, whole and in order`
  (`test/srp_tunnel_test.dart`) — 20 MiB as 80 frames of 256 KiB, byte-exact
  at the fake relay, then one end of body; the old trunk sends 1 frame.
- **The §5.2 "Body frame size" wording is #157's** and is not duplicated here.
  #157's live upload test (`relay_live_test.dart`, skipped there for this
  defect) should be un-skipped once both are on `staging`.
*Revisit if:* SRP gains per-stream flow control (67), which would bound
frames by window rather than by a constant.

**93. The pairing QR carries the registered relay set.** *(2026-10-12, relay
TODO §2's last pairing box; SRP v1 §4.3 names two carriers and only
`GET /v1/server` existed)*
- **Wire.** `storm://pair?…&addr=…` gains one `relay=<url>` per relay,
  percent-encoded (`url::form_urlencoded`), in the order `/v1/server` lists
  them. **No relays, no key:** a server without relays issues the URI it always
  did, and a client that predates the key ignores it. `POST /v1/pairings`
  (`PairingQrPayload`) gains `relays: [url, …]` beside `addr`.
- **Registered, never configured** — the same `registered_relays` snapshot
  `server_info` reads, so the QR and `/v1/server` agree at the instant of
  issue. `public_address` is not sent: the client derives it as
  `registry::public_address` does (`RelayAdvert.derive`).
- **Boot and `storm-server pair` QRs carry none.** The boot QR is minted
  before any tunnel starts, and `pair` is a separate process that cannot know
  what the running server registered with. Both devices learn the set from
  `/v1/server` on their first connection.
- **The client.** `PairingUri.relays` reads every `relay` key and drops one
  that is not `ws(s)://host` (the server's `normalize_relay_url` rule) rather
  than refusing the QR — pairing dials the LAN address; a relay is only an
  extra candidate. At pairing, `Settings.relays` is seeded with
  `pairedRelays`: the list `GET /v1/server` returned during pairing (live,
  from the server that then passed the challenge), or the QR's when that is
  empty (D3: a stale entry costs part of one race, none costs reachability).
  Before this, pairing read `/v1/server` and **dropped its relays**, so a
  device paired on the LAN knew no relay until its first connection race.
- **No trust from a QR relay.** It is a race candidate like any other: the
  privacy ordering is unchanged and whatever answers through it must pass
  the challenge against the QR's public key.
- **Not done here:** pairing *through* a relay. The pairing screen still
  dials only `addr`, so a QR scanned off the LAN fails; the field now carries
  what that flow would need.
- Tests: server `qr_payload_encodes_as_storm_uri` (no relays → old URI),
  `qr_payload_carries_one_encoded_relay_key_per_relay`,
  `pairing_qr_carries_registered_relays_never_configured_ones`; client
  `pairing_uri_test` (old URI, the server's exact bytes, malformed relays
  dropped, live set wins), `pairing_invite_test` (JSON → URI → parse),
  `storm_connection_test` (a QR-seeded device reaches the server through a
  relay before any refresh; a QR relay still faces the challenge).

**96. Pairing reaches its server through the QR's relays.** *(2026-10-12,
relay TODO §4; stacked on 93, which put the relays in the QR)*
- **The flow.** `openPairingRoute` (`lib/api/pairing_route.dart`) runs the
  same `StormConnection` race the app uses afterwards — the QR's address and
  its self-hosted relays together, a public relay only on failure — with the
  QR's `sid` and `pk` pinned, so the race's challenge is the pairing
  challenge. Only a verified route exists; pairing, account set-up and login
  all go over its `authApi`, the transport the challenge was answered on.
  Before this, pairing dialled `addr` alone, so a QR scanned off the LAN
  could not pair.
- **Nothing sensitive before the challenge passes.** Through a relay the
  pairing flow sends `HELLO {server_id}`, `GET /v1/server` and a fresh
  challenge nonce, all public; the pairing nonce, the device secret and the
  password are sent only on a route whose server signed
  `storm-challenge:v1:<sid>:<nonce>` under the QR's key. A relay that answers
  for an impostor is `identityFailed`, named, and sent nothing more.
- **Only `wss://` relays (or loopback) carry pairing.** The challenge proves
  who answers, not that no one else reads it; a password crossing the
  internet over a plain `ws://` relay would be readable on the path.
  `pairingRelays` drops the rest. A relay operator still reads what crosses
  its relay in plaintext — the accepted relay trust model (no end-to-end
  encryption yet), now including the pairing nonce and password.
- **Failures say what to look at.** When nothing is proven, each relay is
  dialled once more on its own (`probeRelay`, `HELLO` only, 8 s — longer than
  the relay's own 5 s hold) and the screen distinguishes: the server at
  `addr` unreachable (no relays); the relay unreachable; the relay up but the
  server not connected to it (`ERROR{server_unreachable}` — `SrpHandshakeFailed`
  now carries the relay's code); the relay has the server but the exchange
  failed; and the identity check failing, naming the relay it came through.
- **Timing.** The race gets 10 s rather than a reconnect's 2 s (a phone on
  mobile data: TLS, `HELLO`, `OPEN_STREAM`, then the request), and the whole
  attempt 30 s, after which it is unreachable rather than a spinner.
- **Needs 92 to work against a real relay.** The challenge, `/v1/pair` and
  login are POSTs with bodies; without the end-of-body frame (#150) they hang
  through a real relay. The unit tests use the fake relay, which answered on
  a timer before 92 and so does not depend on it.
- Tests (`test/pairing_route_test.dart`): direct wins on the LAN; off the LAN
  the QR relay carries pairing and only identity questions precede the route;
  an impostor through a relay is refused and sent nothing else; a key-holder
  naming another id is a mismatch; plain `ws://` relays off this machine are
  dropped; the four unreachable diagnoses; **through a real `SrpTrunk`** over
  the fake relay, `/v1/pair` completes and comes after the challenge; and
  `probeRelay` tells `server_unreachable` from no answer.
- **Amended by 111:** that held only for a fresh server. The usual case, an
  account that already exists, handed over to `/login`, which dialled the
  saved address alone.

**91. `TCP_NODELAY` on every relay socket (P1).** *(2026-10-12, relay TODO
§12.)* A relayed exchange is several small writes each way — the head, body
chunks, a `CLOSE` — and nothing turned Nagle off, so each write after the
first waited out the peer's delayed ACK. Measured 2026-10-03 against a real
relay on localhost: **~42 ms p50 per request**, against 0.4 ms direct.
- **The relay** sets it on every accepted socket: the plain listener is
  wrapped (`PlainListener`; axum's `tap_io` cannot carry `PeerAddr`'s
  `Connected` impl without overlapping axum's own), and the TLS accept loop
  sets it before the handshake task, never inside `Listener::accept`.
- **storm-server's tunnel** dials with `connect_async_with_config(.., true)`,
  under the same connect deadline.
- **The Dart client needs nothing**: `WebSocketChannel.connect` goes through
  dart:io's `HttpClient`, which sets `tcpNoDelay` on every TCP socket it
  opens (`_http/http_impl.dart`, Dart 3.12.2). Browsers do the same.
- Tests: relay `a_relayed_exchange_does_not_wait_on_delayed_acks` (82 ms
  median without the fix, under 1 ms with it) and server
  `a_tunnelled_request_does_not_wait_on_delayed_acks` (42 ms without, under
  1 ms with). Both assert a median under 15 ms over 21 exchanges, with Nagle
  off on the test's own end so only the code under test can add the delay.
*Revisit if:* a relayed exchange is ever batched into one write, which would
make Nagle irrelevant — not a reason to turn it back on.

**109. The browser test counts page loads in its own tab only; the "second
load" was Firefox preloading a new tab.** *(2026-10-14; CI job `browser`
failed about 1 run in 4 on PRs that never touched the web client)*
- **The flake.** A different case failed each time. Every failure signed in
  fine (`/notes`, token stored) but counted `page loads=2`. It hit cases that
  never submit a form, such as the inline-menu stand-in, so it could not be
  the form guard (90) letting a native submit through.
- **The cause: the test's measurement, not the product.** `loads()` counted
  every `browsingContext.load` event in the session. With the event's context
  and URL logged, the second load was always `about:newtab`, from a context
  that is not the tab under test and is not in `getTree`. That is Firefox's
  preloaded new-tab page (`browser.newtab.preload`), loaded in a hidden
  browser about 50 ms after the first navigation starts. Whether its load
  event arrives after `session.subscribe` is a race, hence the
  intermittency. The app's own tab always had exactly one load.
- **The fix.** `loads()` counts load events of the tested context only. A
  native submit that reloads the app navigates that same context, so the
  assertion keeps its teeth.
- **Tests.** `make test-browser` looped locally (Firefox 158, 3 runs at a
  time). Before: 29 of 30 runs failed (71 of 240 cases, every one
  `page loads=2`). After: 0 of 30. With `form_guard.js` removed from the
  build, the fixed test still fails both native-submit cases (`/login`, no
  token, `page loads=2`), so it still catches a real reload.
- *Revisit if:* the test ever opens a second tab or window on purpose, or a
  case needs to see a reload that opens a new context (a `target=_blank`
  form, say).

**110. A disposed sync engine touches nothing, its socket is live when
`start` returns, and the browser test clears tokens before the app runs.**
*(2026-10-14; CI run 38474971682 on #162, and 38473240712 attempt 1 on #163)*
- **a. The engine after `dispose` (product bug).** `two_client_sync_test`
  failed after completion with `Bad state: Can't re-open a database after
  closing it` from `CacheDb.removeNote` in `_applyChanges`, reached from the
  WebSocket's debounce timer. `dispose` already cancelled the timers and the
  subscriptions; what it could not stop was a pull *already in flight*: the
  response to `GET /v1/sync` arrived after the engine was disposed and its
  cache closed, and `_applyChanges` wrote to it. In the app that is sign-out,
  a server switch or a vault switch with a change message in flight.
  `_drainOutbox`, `_pull` and `_applyChanges` now stop at their next await
  once the engine is disposed; `_setOnline`, `start`, `_refreshPending` and
  the `changes` stream do nothing after it; `sync` swallows an error only if
  it is disposed, for a cache query already in flight when the cache closed;
  and an SSE response that lands after `dispose` is cancelled, not
  subscribed.
- **b. The socket was not live when `start` returned (product gap).** Looping
  the live test showed a second failure: *B should have been pushed A's
  change* (21 of 30 runs on a loaded machine, 9 of 10 on a quieter one). The
  engine opened the WebSocket without waiting for the handshake, so A's save
  could land before B's server-side subscription and the frame was never
  sent. `_openWebSocket` now awaits `channel.ready`. The server subscribes
  before answering the upgrade, so after that no change is missed. SSE
  already waited for its response head. The app never awaits `start`, so
  nothing in the UI waits on the handshake.
- **c. "A paired browser signs in again" (test bug).** The app went straight
  to `/v/<id>/browse` because the tokens were still in `localStorage`. The
  test removed them from the *running* app as soon as it reached `/notes`,
  but the app was still settling: `VaultGate` then saved the vault it landed
  on, and `Settings.save` writes every field, tokens included, from memory.
  A probe showed both tokens back within 300 ms of the removal. A real
  expired session does not race the app's own writes, and the relay changes
  (#149, #153, #156) are not involved. The test now clears the two keys in a
  preload script, which runs in the new page before the app does. (Clearing
  them from the JSON page at `/v1/health` did not work: Firefox's JSON viewer
  has its own storage.)
- **Tests.** `sync_engine_test` *disposed mid-sync*: a `/v1/sync` answered
  after dispose and a note fetch answered after dispose both fail on the old
  engine with the CI's exact error and pass now. A change frame scheduled
  just before dispose sends no request. `FakeServer.syncGate`/`noteGate` hold
  those requests open. Live: `two_client_sync_test` against a local server,
  30 runs, 0 failures (it was 21 of 30 before). Browser: `make test-browser`
  on Firefox 158, 3 at a time. Before: 3 of 30 runs failed, each on this
  case at `/v/<id>/browse`. After: 0 of 45.
- *Revisit if:* the engine gains another async path that touches the cache
  (it needs the same `_disposed` checks), `Settings.save` stops writing the
  credentials on every save, or the browser test needs a sign-out that
  happens inside the app.
- **d. Addendum: the test typed into the page it was leaving (test bug).**
  *(2026-10-14; CI run 38482550173 attempt 1 on #162, run 38483866802 on
  #170)* After (c) landed, the same case failed a new way, staying on
  `/login` with no token. The suspect was (c)'s preload script clearing a
  token from a later page load. **Logging ruled that out:** the preload ran
  exactly once in every run, failing runs included, and the tested tab loaded
  exactly once. The cause was `browsingContext.navigate(wait="complete")`
  returning while the **old** document was still current. The failing runs
  report the old path (`/v/<id>/browse`) and the old `performance.timeOrigin`
  right after the navigate. Flutter keeps its hidden password `<input>` in
  that page, so `type_password` found it at once and typed into the page
  being left. The new page then got only the Enter, logged at 70–120 ms
  with an empty field, long before the app had drawn the login screen. This
  had not been seen before (c), because then the case already failed earlier
  at `/v/<id>/browse`. `Browser.open` now waits until a document with a
  different `timeOrigin` has finished loading. Locally on staging (Firefox
  158, 3 at a time) the case failed in 9 of 90 runs before the fix, every
  failure with the old document still current, and in 0 of 45 after.
  *Revisit if:* Firefox's navigate is fixed to resolve only on the new
  document, or a case means to navigate within the same document (a
  same-document navigation keeps its `timeOrigin`, and `open` would time
  out).

**111. Sign-in and session refresh go through the same proven route as
pairing.** *(2026-10-14; found testing the relay on a real phone over mobile
data)*
- **The bug.** Pairing through the relay worked (96), but single-user Storm
  nearly always takes the "account already exists" branch: it saves the
  address, the device credential and the relays, closes the route and hands
  over to `/login`. `LoginScreen` then built `AuthApi(baseUrl:
  settings.baseUrl)` for the account check, sign-in and first-user set-up, so
  it dialled the saved address only. Off the LAN that is a LAN IP nothing
  answers ("Connection refused … /v1/auth/login"), so **a device added away
  from home never got past sign-in**, and no signed-out device off the LAN
  could sign back in. `SettingsNotifier.refreshSession` had the same shape,
  so a session could not be renewed off the LAN either and lapsed into the
  same dead end.
- **The fix: one route, generalised, not a second path.**
  `lib/api/pairing_route.dart` now has `openSavedRoute` beside
  `openPairingRoute`; both are thin callers of one `_openProven`, which builds
  a `StormConnection` with the pinned `serverId`/key and the relays, races
  them (direct and self-hosted together, a public relay only on failure),
  runs the identity challenge, and maps failures to the same
  `PairingRouteException`s, with `probeRelay` diagnosing which hop failed.
  `ServerRoute` is the proven transport; `PairingRoute` extends it with the
  QR's `ServerInfo`. Only `wss://` or loopback relays are used
  (`confidentialRelays`, which `pairingRelays` now calls).
- **Why the route and not the app's `connectionProvider`.** That connection
  exists only once the device has a session (`isConfigured`), it carries a
  bearer token, and the sync engine drives its `connect()`/`reset()`. A
  second caller racing it would change transports under the engine. The
  route is the unauthenticated half of the same seam, already built for this
  (one deliberate attempt, 10 s race, 30 s deadline, named failures), and it
  closes when the caller is done.
- **Call sites.** `LoginScreen._loadAccountState`, `_signIn` and `_setUp`
  share one route per screen, opened lazily, dropped on a transport error so
  the next attempt re-races, and closed on success and in `dispose`. Route
  failures are worded per hop as the pairing screen's are, the identity ones
  ending "Your password was not sent". `refreshSession` opens its own route
  and closes it in `finally`.
- **Audit of the rest of the client** (every `AuthApi(`, `StormApi(` and raw
  `http` use outside `StormConnection`):
  - `bootstrapWebDevice` (`app_state.dart`): web only, and its base URL is
    the page's own origin, which served the page, so it is reachable by
    definition. Left alone.
  - `logout()` is local only (no server call), so it has nothing to route.
    The client has no password-change call.
  - Post-sign-in, not fixed here: `AddDeviceScreen` builds
    `StormApi(baseUrl: settings.baseUrl)`, and `agentApiFactoryProvider` /
    `integrationsApiFactoryProvider` (and `terminal_stream_io.dart`) build
    their own clients from the saved address. All carry a session and bypass
    the relay, so off the LAN "Add a device", Agents and Integrations fail.
    Same class of bug, different fix (they should come from the app's
    connection); left as a follow-up. **Fixed by 112.**
- **Tests.** `test/login_through_relay_test.dart`, over the fake relay with
  the saved address refused: sign-in succeeds through the relay, the
  password only in `/v1/auth/login`, after `/v1/server/challenge`, and
  nothing before the challenge carries a credential; an impostor on the
  relay gets "failed the cryptographic challenge through the relay
  127.0.0.1 … Your password was not sent" and the origin sees neither
  `/v1/account` nor `/v1/auth/login`; a session refresh goes through the
  relay after the challenge; a plain `ws://` relay off loopback is never
  raced; a relay without the server is named. The two sign-in widget tests
  fail on the old code (no token; no identity message). `FakeRelay` now
  serves a second trunk. Live (`test_live/relay_sign_in_live_test.dart`, its own file because a `testWidgets` would put `relay_live_test.dart` under the widget binding, whose `HttpClient` answers 400): an extra
  device is paired from an invite whose address is a closed port, then signs
  in through the real `LoginScreen` and the real `storm-relay`, and its
  token reads the vaults.
- *Revisit if:* the app's connection gains an unauthenticated mode the login
  screen could share, the post-sign-in screens above move onto the
  connection, or a relay becomes able to carry a confidential channel of its
  own (then `ws://` relays could be allowed).

**112. Every session call after sign-in goes over the app's connection,
direct or relayed.** *(2026-10-14; the follow-up from 111's audit)*
- **The bug.** 111 fixed sign-in and refresh. After them, off the LAN, the
  features that built their own clients from `settings.baseUrl` still dialled
  only the saved address: "Add a device" (`StormApi(baseUrl: …)`), Agents
  (`agentApiFactoryProvider`, and the live terminal in
  `terminal_stream_io.dart`, which made its own `http.Client`) and
  Integrations (`integrationsApiFactoryProvider`). Notes and sync worked,
  because they go through `StormConnection`. The audit turned up two more:
  - **`apiProvider` was `connection.api`**, which follows the *active*
    candidate, and before the first race that is the saved address. Only the
    sync engine calls `connect()`, and only with a vault open, so a device
    signed in off the LAN with no vault open (a new one, on the dashboard)
    could not list its vaults.
  - **Attachment images** captured `connection.transportClient` and a
    transport-specific URL when the widget was built, so a reconnect left them
    on a disposed trunk. And a markdown image with an absolute `http(s)` URL
    was sent through the session's client **with the bearer header**, to
    whatever host it named.
- **The fix: one client that borrows the connection.**
  `lib/api/connection_client.dart`, `ConnectionClient extends
  http.BaseClient`. Callers keep addressing the saved address. Per request it
  takes the connection as it is *now*: if it is connected the request goes on
  `transportClient`, rebased onto the active candidate's base URL; if it has
  not raced (idle, offline) it calls `connect()`; if it is `unproven` it
  refuses. A URL that is not on the saved address is refused, never sent
  with the credential. `close()` does nothing, since the connection owns the
  transports.
  - `apiProvider` is now a `StormApi` on the saved address over a
    `ConnectionClient` of that connection. The engine still uses
    `connection.api`.
  - `sessionClientProvider` resolves `connectionProvider` per request
    (`ref.container.read`), so the Agents and Integrations factories do not
    rebuild on every settings save (the reason `sessionCredentialsProvider`
    selects two fields). Both factories pass it as their client. `AgentApi`'s
    `baseUrl` stays the saved address, because an enrollment sends it to the
    runtime as `server_url`.
  - `AddDeviceScreen` uses `apiProvider`. The invite's `addr` comes from the
    server's `listen_addr`, not the request, so a relayed mint is the same
    invite.
  - Images: `StormImageProvider.attachment(api, vault, path)` takes
    `apiProvider`'s client. The URL is the saved address on every transport,
    so the image cache keeps one entry per attachment. External images are a
    plain `NetworkImage`, with no credential.
  - `StormApi.attachmentUrl` (credential in the query string, used by no app
    code since 63) is removed for `attachmentUri`, which has no credential.
    The server's `?token=` fallback is still there. That open item is now
    server-side only.
- **No second race.** `StormConnection.connect()` is single-flight: a caller
  arriving during a race shares it, so the sync engine and these clients
  never race side by side. `reset()` bumps an epoch, and a race it overtook
  returns `unreachable` without adopting, setting status or releasing trunks.
  Without that, the engine's reset-then-connect after a feed drop could
  overlap a race a feature request had started. `transportClient` is now
  `api.client`. It used to build a fresh `http.Client` per call when nothing
  was cached.
- **A reconnect does not pin anything.** Each request picks its transport
  when it is sent. After `reset()` the next request joins (or starts) the
  re-race and lands on the new trunk. A live terminal on the dead trunk errors,
  and `SessionController` reconnects by offset, which is a new request.
- **The terminal stream relayed (SRP §5.3).** `terminal_stream_http.dart` is
  the streamed GET with the bearer header, on `AgentApi.client`, now shared by
  native and by web when the connection is relayed. `EventSource` cannot ride
  the tunnel, but the tunnel does not buffer as the browser's `http` client
  does. Web stays on `EventSource` when direct. `SrpHttpClient` hands each
  `0x01` chunk on as it arrives, like the change feed. Cancelling ends this
  response and no longer closes the client, which is shared now. A response
  that arrives after a cancel is drained.
- **Defect fixed on the way: a cancelled relayed body never closed its
  stream.** `SrpHttpClient`'s body controller had no `onCancel`, so a reader
  that stopped listening (a closed terminal, a disposed SSE feed) left the
  stream open and the origin streaming to nobody until the trunk died. The
  body's `onCancel` now sends `CLOSE`.
- **Cannot be relayed, and why.**
  - Web `EventSource` when direct. It does not need to be relayed: web is
    served by its server, so direct is the page's own origin.
  - `bootstrapWebDevice`, same reason (111).
  - The OAuth sign-in's browser leg goes to the upstream provider, not to
    Storm. Only `{state, code}` comes back, through `IntegrationsApi`, so
    relayed.
  - What a runtime dials is its own business. The enrollment carries the
    saved address.
- **Tests.** `test/relayed_features_test.dart`, over the fake relay with the
  saved address refused (port 1). Agents lists sessions and hosts;
  Integrations lists connections; `apiProvider` lists vaults with no engine
  started; Add a device mints an invite (widget); an attachment image decodes
  (widget, `RawImage` has an image); and the terminal delivers a status event
  and an output event while the body is still open, then cancelling sends
  `CLOSE`. Each checks the request went through the relay, after
  `/v1/server/challenge`, with nothing credentialed before it. Also covered:
  a `reset()` followed by a request re-races and succeeds on a new trunk;
  four callers together cause one challenge; a request to another host is
  refused and nothing is sent; and an impostor on the relay gets no
  credentialed request. **All 9 that compile on the old code fail on it**
  (connection refused to `127.0.0.1:1`, or the widget never loads).
  `srp_tunnel_test` adds "a reader that cancels a streamed body closes its
  stream", red on the old `SrpHttpClient`. `FakeRelay` gains
  `FakeResponse.streamed` and `closedByClient`. One test changed:
  `attachments_test`'s URL now asserts no credential.
  **Live** (`test_live/relay_features_live_test.dart`): `make test-live` now
  enrolls a `storm-runtime` host (`live-host`, the `fake` provider) on the
  first server. Against the real `storm-relay`, with the saved address a
  closed port and the app's own providers, the test lists hosts and sessions,
  launches a session, streams its terminal (the start line, then the echo of
  input typed after the stream opened), ends it and sees `stopped`. It also
  fetches an uploaded attachment through `StormImageProvider`, byte for byte.
  The host's state lives in a `mktemp -d` under `/tmp`, removed on exit. Under
  `.dev/` in a deep checkout, its bridge socket path passed `SUN_LEN` and the
  runtime would not start.
- **Results.** `make check` green: client 1072 tests, 1 skipped (was 1060).
  `make test-live PORT=19200 AUTH_PORT=19201 RELAY_PORT=19202` exits 0:
  server 81, mcp 86, agent 93, gateway 95, auth 72, mcp-connect 89 + 89,
  mcp-connect live 39, client integration 33, 1 skipped (`agent_live_test`
  still needs its own env). One earlier run had a gateway e2e failure, "the
  open elicitation is cancelled toward the agent (R8)", 94/95. It has its own
  server and host, none of this change is involved, and it passed in the other
  two runs. Noted as a flake.
- *Revisit if:* the app gains another client that does not take an
  `http.Client` (it must get `sessionClientProvider`), the connection gains
  liveness of its own (then `ConnectionClient` could stop racing an `offline`
  connection on each request, which is what lets a feature recover with no
  vault open), or web gains relays it would use instead of its origin.

**113. The session connection survives mobile data: the race waits for a
slow relay, and nothing resets a working trunk.** *(2026-10-14; found on a
Pixel on mobile data through the hosted relay, with the APK from `6f5e6d5`)*
- **The bug.** Paired and signed in through `wss://storm-relay.dewansh.space`
  (111's route), Settings showed the relay and a verified identity, yet notes
  would not create, sync did nothing and the app said the server was offline.
  The Worker logged a new client trunk every few seconds, each closing the
  last, and no error. On Wi-Fi the same relay path worked.
- **Reproduced from here** with the app's own `StormConnection`, `SyncEngine`
  and `ConnectionClient` against the deployed Worker and the test server, the
  saved address a closed port, and each trunk frame delayed by a 500 ms ±
  300 ms link (three round trips to open). The connection log (below) showed:
  ```
   0.04s probe direct failed in 10 ms: _ClientSocketException
   2.04s race window closed after 2000 ms
   2.04s status connecting -> offline
   2.04s reconnect in 1s: server unreachable
   2.24s trunk ready after 2203 ms
   3.05s reset: server unreachable
   3.05s trunk lost: closed after 3012 ms, 1 streams open
   3.07s create: Cannot reach the server — new notes need a connection.
   ...
  11.21s probe selfHosted ok in 4150 ms      (after the race had given up)
  13.06s reset: server unreachable           (and the trunk is closed)
  ```
  The same run on a desk link (no injected delay) connects in about 1.1 s,
  which is why `make test-live` and `wrangler dev` passed.
- **Root cause: the race gave the relay 2 s.** `_race` timed out every round
  at `publicRelayDelay`, including the first round when there is no public
  relay. A relayed probe is a TLS dial, `HELLO`/`READY`, a stream open and
  `GET /v1/server`: about seven phone round trips, 4.2 s on the emulated link.
  So every race reported `offline` while the relay was about to answer. The
  engine then reset (closing that trunk, the Worker's `canceled`), backed off
  1, 2, 4, 8, 16 s and raced a cold trunk again, which matches the Worker's
  cadence. Sign-in worked because 111's route uses a 10 s race; the session
  connection kept 2 s.
- **What else the reproduction and the client audit confirmed**, each fixed
  here:
  - **The engine sent to the saved address mid-race** (audit G5).
    `SyncEngine.api` was `connection.api`, which after a reset or a failed
    race is the direct candidate: a create during the re-race was refused and
    marked the engine offline. With a bearer token, on a foreign network.
  - **A feed ending reset the connection** (G2), closing a live trunk and
    every save in flight on it.
  - **A dead trunk stayed `connected`** (G4): nothing listened to
    `onTrunkLost`, so every request went to the closed trunk until the engine
    happened to reset, and with no vault open, never.
  - **Offline was a latch** (G3): one failed request set it, saves then
    queued without trying, and only a feed event or a reconnect cleared it.
  - **One trunk could dial twice** (G6): `SrpTrunk.connect()` was not single
    flight, so a second caller mid-handshake sent a second `HELLO` and
    orphaned the first socket.
  - **Every settings save rebuilt the connection** (G7), and the engine with
    it, and raced a new trunk; the old trunks were never closed
    (`connection.api.dispose` closed nothing on a relayed client).
  - **"Server offline" was sticky** (G8): a stream-scoped `server_timeout` on
    a working connection set the flag, and every later failure, the 2 s cap
    included, read "Server offline, relay reachable".
  - Not confirmed: several components each resetting (only the engine and the
    pairing route call `reset()`), and the `relay_silent` liveness. A link that
    goes silent without closing is detected at 45–60 s and costs one re-race,
    then the connection is stable (the live run below).
- **The fix, as rules** (for the spec PR to lift into `srp-v1.md` §8):
  1. **A race has one deadline, `raceTimeout` (15 s), not a round timer.** The
     first round (direct and self-hosted relays) runs until one answers, all
     fail, or the deadline. `publicRelayDelay` (2 s) only says when a public
     relay may *join*; the first round keeps running after it, so a slow
     self-hosted relay can still win. A refused candidate fails at once, so
     the deadline only matters for a LAN address that never answers. The
     route keeps its 10 s window and passes its 30 s deadline as `raceTimeout`.
  2. **Only the connection decides a transport is lost.** The active trunk
     closing resets the connection (`active trunk lost: <reason>`), so status
     leaves `connected` and the next request re-races. A feed ending is the
     feed's business: if the trunk is still up, the engine re-proves on it
     (no reset, no new trunk) and reopens the feed.
  3. **Session requests wait for proof.** `StormConnection.sessionApi` is the
     `ConnectionClient`-backed API on the saved address (112), and the engine
     uses it, so a request during a race joins it and goes out on the winner.
     `apiProvider` is the same object. `move` now refuses an unproven server
     as `save` does.
  4. **Offline is re-checked, not latched.** A request failure that marks the
     engine offline schedules the engine's reconnect (re-prove, then sync),
     and the connection becoming `connected` from any caller syncs if
     anything is queued or the engine is offline. The backoff resets when a
     request succeeds.
  5. **One dial per trunk.** A caller arriving mid-handshake joins it.
  6. **The connection lives as long as what it is built from.**
     `connectionProvider` watches only the address, token and pinned
     identity; the relay set is read at build (the connection keeps it
     current itself). A replaced connection is disposed, which closes its
     trunks.
  7. **"Server offline" is per race.** Each race clears the verdict, and a
     relay's server-side code counts only while the connection is not
     `connected`.
- **Connection log** (`lib/api/connection_log.dart`): the release APK logs
  nothing about connections, so there was no on-device evidence. A bounded
  in-memory log (400 lines, lost on exit) records races and candidates, each
  probe's outcome and time, trunk dial/ready/loss with the reason, resets with
  their cause, identity checks and status changes, with timestamps. It holds
  scheme, host and port only: never a token, key, ticket, path or note
  content (a test asserts it). Settings › Connection › Diagnostics ›
  "Connection log" copies it.
- **Settings copy (audit G9).** The Relays note said "A relay can't read your
  notes", and the add-relay help "It carries traffic and cannot read it".
  Both false: SRP traffic is not end-to-end encrypted, so a relay's operator
  can read notes and credentials (Relay Security §7.4). Now: "Another way to
  reach your server, tried alongside its address. Until Storm has end-to-end
  encryption, whoever runs a relay can read what passes through it, notes and
  sign-in included. Run your own relay or use one you trust." The help says
  whoever runs it can read the traffic it carries.
- **Tests.** `test/relayed_reconnect_test.dart`, over the fake relay (which
  gains `latency`, `hellos`, `streamErrors` and `dropTrunk`) with the saved
  address refused, through the app's providers:
  a relay 2.4 s away connects on the first race and the vaults load (one
  `HELLO`); the engine connects through it and creates a note on one trunk; a
  create during a re-race waits for it; the feed ending re-opens on the same
  trunk without failing a save in flight; a dropped trunk ends the connection
  and the next request re-races; one `server_timeout` does not latch the
  engine offline; that timeout is not remembered as "server offline"; an
  unrelated settings save keeps the connection and trunk; a replaced
  connection closes its trunk; two callers mid-handshake send one `HELLO`; and
  the log names hops and reasons and no credential. `storm_connection_test`
  adds a self-hosted relay slower than the public delay winning alone, and
  after the public relay joined, and a race nobody answers ending at
  `raceTimeout`. **All 13 fail on the old code** (run against
  `origin/staging` with the new test files and a test-only shim for the new
  getters). `settings_pages_test` pins the relays wording and the copied log.
  `srp_liveness_test`'s "verdict landing after the race gave up" sets a
  200 ms `raceTimeout`, since the race no longer gives up at 2 s.
- **Live, against the deployed Worker and the test server** (a temporary
  test, not committed, with the emulated link and the first trunk going silent
  at 40 s): connected at 6.7 s (probe 4.2 s, challenge 2.4 s), vaults, create
  and sync succeeded; at 92 s `trunk lost: relay_silent`, one reset and one
  re-race, connected again at 99.8 s; a create at 167 s succeeded; two trunks
  in 167 s.
- **Results.** `make check` green: client 1088 tests, 1 skipped
  (was 1072). `make test-live PORT=19310 AUTH_PORT=19311 RELAY_PORT=19312`
  exits 0: server 81, mcp 86, agent 93, gateway 95, auth 72, mcp-connect
  89 + 89, mcp-connect live 39, client integration 33, 1 skipped.
- *Revisit if:* the spec PR's client state machine (§8) lands (these rules
  move there), relay liveness needs to be faster than 45 s on mobile, or a
  connection log needs to survive a restart (it would then need a retention
  rule and the same no-credential test).

**94. Relay failure handling: a silent relay is dropped, and each §9 case has
a test.** *(2026-10-12, relay TODO §9/§10; stacked on 91.)* Each case was
established with a test first, then fixed where wrong.
- **Defect fixed — storm-server never gave up on a relay that went quiet.**
  The tunnel PINGed every heartbeat but never required an answer, so a relay
  lost to a sleep, a network change or a dropped NAT mapping (socket open,
  nobody behind it) stayed registered and advertised on `GET /v1/server`
  until TCP noticed, which takes many minutes. `serve_trunk` now drops the
  trunk after **three heartbeats of inbound silence** (45 s at the default
  15 s), the same allowance the relay gives a server, and redials. The relay
  answers every PING, so a live relay is never dropped. A closing trunk waits
  at most 1 s for its writer, so a peer that stopped reading cannot hold the
  supervisor and its reconnect.
- **Covered, already correct:**
  - The relay vanishing mid-response aborts the in-flight handler, stops
    advertising the relay, leaves the LAN listener alone, and redials after
    the 1 s served-then-closed backoff.
  - A relay `CLOSE` or `ERROR` for one stream cancels only that handler.
  - A server dying mid-body is `trunk_lost` on that stream at the client,
    never a clean `CLOSE`. A `HELLO` after the server left is
    `server_unreachable`. A restarted server is reachable on a fresh client
    trunk.
  - In Dart, a trunk dying mid-response, or a stream-scoped `trunk_lost`, is
    a body error (`SrpTransportException`), never a short successful body.
    The request is not re-sent, and a dead trunk is never reused.
- **Simulated, not reproduced:** network transition and laptop sleep are
  tested as trunk death and inbound silence on localhost. No real Wi-Fi to
  cellular switch, suspend or NAT has been exercised; that needs the phase 4
  VPS.
- **Open, not fixed here:**
  - `SrpTrunk` reports every trunk-level `ERROR` as `relay_error`, so
    `server_unreachable` (relay up, server down) is indistinguishable from a
    dead relay. `ConnectionStatus` has only `offline`. The relay TODO's
    ticked "server offline vs relay unavailable" box is overstated. The test
    is written and skipped (`srp_failure_test.dart`).
  - The Dart trunk PINGs but has no liveness deadline either, so a half-open
    socket after a network change hangs relayed requests until their own
    timeouts.
  - A server-side response body that *errors* mid-stream ends with a clean
    `CLOSE`, so the client takes a truncated body as complete. SRP has no
    server→relay error (§6: `ERROR` is relay→either), so the fix is a
    protocol amendment, not an implementation change.
- Tests: server `a_relay_vanishing_mid_response_cancels_the_handler_and_the_trunk_comes_back`,
  `a_relay_closing_a_stream_mid_response_cancels_only_that_handler`,
  `a_relay_erroring_a_stream_mid_response_cancels_that_handler`,
  `a_relay_gone_silent_is_dropped_at_the_heartbeat_deadline_and_redialled`
  (fails without the fix), `a_relay_that_answers_heartbeats_is_kept_past_the_deadline`.
  Relay: `a_server_dying_mid_response_body_is_trunk_lost_on_that_stream_not_a_clean_end`,
  `a_hello_after_the_server_went_away_is_server_unreachable`,
  `a_restarted_server_is_reachable_again_on_a_fresh_client_trunk`. Client:
  `test/srp_failure_test.dart`.
*Revisit if:* the relay's heartbeat interval stops being echoed in
`REGISTERED`, or a relay stops answering PING. Either makes inbound silence a
false signal.

**101. A response that fails after its head is aborted, never closed (P6).**
*(2026-10-12, relay TODO §9/§12.)* storm-server's tunnel ended every relayed
response with `CLOSE { stream_id }`, including one whose body stream failed
partway (`serve_one` broke out of the loop and fell through to the normal
`CLOSE`). The relay forwarded it, and the client took the bytes it had as the
whole body: a read error mid-note became a truncated note with a 200.
- **SRP §5.4 gains "A response that fails partway is aborted, never closed":**
  the origin ends such a stream with `CLOSE { stream_id, aborted: true }`; the
  relay turns that into `ERROR { code: "stream_aborted", stream_id }` toward
  the client and leaves the trunk alone; a client treats any stream-scoped
  `ERROR` after the head as a body error. `stream_aborted` joins §6's codes,
  and `aborted` that is not a boolean is a `protocol_error`.
- **Why a flag on `CLOSE`, not an origin-sent `ERROR`:** `ERROR` is
  relay-originated in §6, and a relay that predates this rule answers an
  origin's `ERROR` with `protocol_error` and drops the whole trunk — every
  relayed client of that server. That relay reads the flagged `CLOSE` as a
  plain one, which is today's behaviour and breaks nothing else. A client that
  predates the rule already ends a stream on a stream-scoped `ERROR` of any
  code, so the Dart client needed no change, only a test.
- Server: `proto::abort_stream`; `serve_one` sends it on a body error.
  Relay: `ErrorCode::StreamAborted`; `server_close` maps the flag.
- Tests: `a_response_body_that_fails_midway_is_aborted_not_closed` (server),
  `an_aborted_response_reaches_only_its_client_as_stream_aborted` and
  `an_aborted_flag_that_is_not_a_boolean_is_a_protocol_error` (relay) — all
  three fail on the old code; Dart `an aborted response is a body error, and
  only that stream ends` (passes on the old client too, by design above).
- Not covered: a live test through the real relay — no real route fails
  mid-body on demand.
*Revisit if:* SRP v2 lets the origin send stream-scoped `ERROR`s directly.

**106. A backlogged client is told its stream failed; a refused open names its
attempt.** *(2026-10-13, found building the Rust SRP client, decision 105a.)*
Two relay→client signals were missing, and each left a client guessing.
- **Defect: a backlogged client's stream hung for ever.**
  `drop_backlogged_client` closed the stream and sent `CLOSE` to the origin,
  and sent the client nothing: a started body never ended, a pending head
  never came, and no client can turn silence into an error. The relay now
  ends the stream with a new stream-scoped `ERROR{client_backlogged}`.
- **Delivered past the full queue, not into it.** The client trunk's writer
  has a second lane (`Tx::end_stream`) it drains first: the `ERROR` overtakes
  what is queued, the writer discards the ended stream's queued body frames
  (refunding the byte budget), and every other stream's frames go on in order.
  Rejected: discarding the trunk's queue (wedges or truncates the other
  streams), queueing the `ERROR` behind the backlog (never arrives at a stalled
  client), and dropping the whole client trunk (kills healthy streams for one
  slow download). Dropping the trunk is the fallback only if the lane itself
  (256) overflows — the relay kills the writer and reader, and the client sees
  trunk loss. A plain `CLOSE` never takes the lane, since overtaking its body
  would present a truncated body as complete; with no room for it the stream
  is ended `client_backlogged` instead. `server_timeout`, `stream_aborted` and
  the per-stream `trunk_lost`/`trunk_superseded` use the lane too, so no
  stream-ending `ERROR` is lost to a full queue any more.
- **Defect: `rate_limited` on `OPEN_STREAM` could not be attributed.** It was
  trunk-scoped with no id, while the trunk stayed up. The Dart client took it
  as fatal and dropped the trunk; the Rust client failed its oldest pending
  open, relying on the relay answering in order. The relay now echoes the
  open's `attempt_id` (as the JSON sent). The Dart client fails only that
  attempt (`SrpOpenRefused` → `SrpTransportException('rate_limited')`) and
  keeps the trunk; an `ERROR` without one is still fatal.
- **SRP §5.1** gains "A refused open names its attempt and does not end the
  trunk"; **§5.4** gains the backlogged-client rules; **§6** adds
  `client_backlogged` and `ERROR`'s `attempt_id?`.
- **Compatibility.** Old Dart client, new relay: ignores `attempt_id`, drops
  the trunk as before; reads `client_backlogged` as any stream-scoped error,
  i.e. a body error (§5.4 rule 3). Old Rust client, new relay: serde ignores
  the extra field and FIFO attribution is still right; an unknown stream code
  becomes `SrpError::Relay(code)`, a body error. New clients, old relay: no
  `attempt_id`, so Dart drops the trunk and Rust falls back to FIFO — both
  unchanged — and backlogged streams still hang, as before. The origin sees
  nothing new.
- **Rust client follow-up** (`feat/runtime-srp-client`, not on this branch):
  `codec::RelayError` gains `attempt_id: Option<serde_json::Value>`, and the
  trunk-scoped `rate_limited` arm removes the pending open whose `attempt_id`
  matches, falling back to `pop_front` only when the field is absent.
- Tests: relay
  `a_backlogged_client_is_told_its_stream_failed_and_its_other_streams_go_on`
  (no body frame for the stream follows its error; the other stream's body
  and `CLOSE` arrive; a later open works),
  `a_refused_open_names_its_attempt_and_the_trunk_carries_on` (an object
  `attempt_id` echoed whole), and `attempt_id` asserted in
  `past_the_in_flight_cap_open_stream_is_rate_limited_immediately` — all three
  fail on the old code; unit `a_stream_end_that_finds_no_room_drops_the_trunk`,
  `only_an_ended_streams_body_frames_are_discarded`. Dart: `a refused open
  fails only its own request, and the next request on the trunk works` (fails
  on the old client), `a refusal that names no attempt still drops the trunk`,
  `a stream dropped for a backlogged client is a body error, and the trunk
  stays up`.
*Revisit if:* SRP gains per-stream flow control (67) — a window would stop a
slow reader before its queue fills, and `client_backlogged` would become rare
rather than the only bound.

**108b. The hosted relay: SRP v1 on Cloudflare Workers + Durable Objects.**
*(2026-10-13, phase 5 step 1, slice B of decision 108; vault: *Hosted Relay —
Cloudflare*.)* `apps/relay-cf`, a standalone TypeScript Worker, wire-identical
to `storm-relay`: `/register` and `/connect/<server_id>`, the same handshake,
frames, codes and `public_address` shape, and the same `STORM_RELAY_*` knobs
as the conformance fixture contract (108a). Allowlist-only by default; TOFU is
a var for parity and the suite. Not deployed: no account id, no routes, out of
`release.yml`.
- **One Durable Object per `server_id`** (`RelayRoom`, `idFromName`,
  SQLite-backed) holds the server trunk, a superseded one while it drains, and
  every client trunk of that server, all hibernatable (`acceptWebSocket` with
  `s:<trunk_id>` / `c:<client_trunk_id>` tags). A frame is routed without
  leaving the object. The `HELLO` limit is a token bucket in its own DO per
  `CF-Connecting-IP` (`HelloLimiter`), persisted and deleted after two idle
  windows.
- **`/register` is the one hop that is not spliced.** It names no `server_id`,
  so the Worker must read `REGISTER_SERVER` to pick the room; a WebSocket
  cannot be handed on after its first message, so the Worker carries that
  trunk's frames for its life. Client trunks go to the room whole. Rejected: a
  per-connection handshake DO with an RPC hop per frame (two objects billed
  per frame), and a `?server_id=` hint (a wire change; see below).
- **Hibernation evicts memory, so nothing lives only there.** Per socket, the
  attachment holds the phase, deadlines, the server's next `stream_id`, the
  last-seen time (written at most once a second), the drain deadline, and the
  read watermarks below. SQLite holds the stream table (owner, acked, ack
  deadline), nonces (expiry, spent) and TOFU bindings. The constructor rebuilds
  the routing maps from `getWebSockets()` and the table, dropping rows whose
  sockets did not survive. Each socket's frames are handled in order across
  the WebCrypto await in `CHALLENGE_RESPONSE`.
- **Timers are one alarm**, set to the earliest of: handshake deadline, `HELLO`
  wait, ack deadline, drain end, heartbeat deadline. **PING uses
  `setWebSocketAutoResponse`**: storm-server, the Dart and the Rust clients all
  send exactly `{"v":1,"type":"PING"}`, and the reply is `{"v":1,"type":"PONG"}`,
  so the runtime answers without waking the object, and the heartbeat reads
  `getWebSocketAutoResponseTimestamp` as well as the last frame. Any other
  spelling of PING still reaches the handler. A trunk past its deadline is
  also refused at `HELLO`/`OPEN_STREAM` time, in case the alarm runs late.
- **"A client that fell behind", without a send buffer.** `WebSocket.send`
  never blocks or fails, and workerd queues without bound: 300 MiB sent to a
  paused reader grew the process to 420 MB with no error, and a Durable Object
  has 128 MB, so one slow client could take its server's whole room down. The
  relay now measures what each peer has *read* with SRP's own messages: after
  every `budget/8` bytes to a socket it sends a `PING` and records the byte
  count; a peer answers in order, so each `PONG` proves everything before that
  mark was read. **Backlog = bytes sent − bytes proved read.** A frame that
  would take it past `STORM_RELAY_MAX_CLIENT_BUFFER_BYTES` ends its stream
  under decision 106: `ERROR{client_backlogged, stream_id}` to the client,
  `CLOSE` to the origin, later frames for it dropped, other streams untouched.
  With no mark outstanding any one frame is admitted (storm-relay's "empty
  queue"). The `ERROR` cannot overtake what the runtime already holds (106's
  rule 2 makes overtaking a MAY); after 256 such ends unanswered the client
  trunk is closed (rule 3). A room-wide bound (`MAX_ROOM_BUFFER_BYTES`, 64
  MiB) covers several slow readers at once. Attachments carry the marks, so a
  backlog survives hibernation.
- **The same blindness server-ward.** storm-relay slows an uploading client by
  not reading its socket; a Durable Object cannot stop reading. An upload that
  would take the origin's unread bytes past `MAX_SERVER_BUFFER_BYTES` (32
  MiB, room for one 16 MiB frame and its end of body) ends that stream with a
  stream-scoped `ERROR{rate_limited}` and `CLOSE` to the origin.
- **Deviations, written down, with proposed spec text** (not applied to
  `docs/srp-v1.md`; the operator decides):
  1. *§5.2 "the relay derives the address from the socket, never from a
     header."* Here it is `CF-Connecting-IP`, which Cloudflare's edge sets
     and overwrites, and which the Worker overwrites `relay_peer_ip` from
     unconditionally (a client-sent field, even `null`, is still
     `protocol_error`). Proposed: "…from the connection as the relay's own
     front end received it — the TCP peer, or an address the relay's
     provider edge attests and a client cannot set — never from a header a
     client can set." Under `wrangler dev` a client *can* set it; only a
     deployed relay may rely on it.
  2. *The watermark needs PING answered.* §4.2/§6 never say a peer MUST
     answer a PING. Every implementation does; a client that did not would
     see its streams ended `client_backlogged` after one budget. Proposed for
     §4.2: "Either party MUST answer every PING with a PONG, in the order
     received. A relay MAY send PING on any trunk and use the answers to
     learn how much its peer has read." A client can also send PONGs it owes
     nothing for and so raise its own budget; the room bound limits that to
     its own server's room. A `PING` that carries a token the `PONG` echoes
     would close it, as a later, compatible change.
  3. *`rate_limited` on a stream.* §6 defines it on an open (`attempt_id`)
     and trunk-wide. Proposed: "A relay that cannot hold more for the origin
     MAY end the stream being written with `ERROR{rate_limited, stream_id}`
     and `CLOSE{stream_id}` toward the origin." Clients already fail a stream
     on any stream-scoped `ERROR` (§5.4 rule 3).
  4. *A PING during a handshake* (before `REGISTERED` or `READY`) gets PONG
     from the auto-response, where storm-relay answers `protocol_error`. The
     spec says nothing either way; proposed: "a PING MAY be answered at any
     point on a trunk."
  5. *Frames before `READY`* (sent while a `HELLO` is held) are refused with
     `protocol_error`; storm-relay reads them after `READY`. Unspecified;
     every client waits for `READY`.
  6. *Not built, as in storm-relay:* §6's bandwidth cap per `server_id`, and
     §4.1's account-owned binding for a public relay. Step 1 is invite-only
     by allowlist (the self-hosted-allowlist row of §4.1); both are step 2,
     with Q14's numbers.
- **What is logged** (`src/log.ts`): lifecycle and error codes, never a
  payload, header, path, nonce, signature, key or client address.
- Tests: 85 vitest cases inside workerd (`npm test`, in `make check` and the
  `relay-cf` CI job, `npm ci` from the lockfile): `docs/srp-vectors.json` in
  full (the fourth implementation it holds), registration and bindings in both
  modes, every §5/§6 behaviour above, the backlog and room bounds, and
  hibernation — `evictDurableObject` mid-stream, then the stream continues,
  an unacked open still times out, a held `HELLO` is answered, a drain ends,
  auto-answered PINGs keep a trunk, a backlog is remembered. Mutating the
  attachment write or the stream-table reload fails four of them. **The 108a
  suite passes 54/54** against two `wrangler dev` instances (allowlist and
  TOFU, with `RELAY_MULTI_IP` and `RELAY_LATENCY`); its first run failed two
  §5.2 checks at a 16 MiB server-ward budget, which is why it is 32 MiB.
  **Smoke against a real storm-server** registered through
  `PUT /v1/config/relays` in allowlist mode (`scripts/smoke.mjs`): identity
  challenge verified end to end, 16 concurrent requests, a 1 MiB note written
  and read, a 20 MiB attachment both ways, the change feed delivering a LAN
  edit, `server_unreachable` for an unknown id, and the trunk still up after
  65 s idle (past the 45 s deadline, on auto-answered PINGs).
*Revisit if:* the `/register` hop costs real CPU (then propose an optional
`server_id` on the upgrade so the Worker can splice); SRP gains flow control
(67), which replaces the watermark; or a runtime exposes a send-buffer size,
which makes it unnecessary.
*108b addendum (2026-10-14):* a CI run of PR #174 failed with `Can't call
WebSocket send() after close()`. Under load the 256 KiB test's server trunk
was dropped at the 1.5 s test heartbeat deadline although it had sent frames
well inside it: the room only recorded a frame as "seen" when it also wrote
the attachment, at most once a second, so a frame less than a second after the
last write was ignored. The test then wrote to the closed trunk. The deadline
now counts from the newest frame held in memory (`Entry.seen`), and the
attachment is still written at most once a second. The test sending after the
close was a symptom, but the same exposure was in the relay. workerd's
`WebSocket.send` throws once a socket is closing, whichever side closed it, and
still delivers messages that arrived before the close. The Worker's `/register`
hop forwarded server frames with a bare `send`. The room swallowed a failed
send and left the peer's streams open until its close event. Every send now goes
through `trySend` (`src/send.ts`): a closed or closing socket returns `false`,
never throws. In the room, a refused send ends that peer at once, exactly as
its `webSocketClose` would: the origin gets `CLOSE` for a client's streams, a
server's clients get `ERROR{trunk_lost}`. Each drop forgets the entry before it
sends anything, so the cleanup cannot recurse or run twice. A new stream's
`STREAM_READY` goes out before the stream exists, and `REGISTERED` before the
old trunk is superseded, so a peer already gone leaves nothing to undo. The
`/register` hop closes both sides when one is gone. The test harness's PING
answer had the same bare `send`. `closed-peer.test.ts` closes a peer inside the
room and then makes the room send to it, both ways. A routing test sends
frames 0.9 s apart past the deadline. All three fail on the old code, and the
suite is now 92 cases. 30 serial runs passed before and after the change.
Under load (four suites in parallel per tree, 32 runs each), the old code
failed 6/32 and 4/32 on the CI error and the new code 0/32.

**105a. storm-runtime gains an SRP v1 client (mcp-connect S1).**
*(2026-10-13, decision 105's first slice; vault *MCP over the Relay —
mcp-connect*.)* `mcp-connect` must reach a server through a relay, and §11's
Runtime Hosts behind a relay will need the same thing, so the client half of
SRP now exists in Rust: `apps/runtime/src/srp/` (`codec.rs`, `trunk.rs`,
`error.rs`). A port of the Dart `SrpTrunk`, with nothing new on the wire.
- **The codec is a copy, not a dependency** (R6): it is the fourth
  re-derivation, beside `apps/server/src/relay/proto.rs`, `apps/relay` and
  `srp_codec.dart`. `tests/srp_vectors.rs` reads `docs/srp-vectors.json`.
  A client never signs relay-auth, so the test covers the `server_id` rule it
  dials with and the client challenge (message, signatures, key parsing) that
  S2 will check the race winner with. `validate_server_id` is reused from
  `identity.rs`, which `runtime-vectors.json` already pins.
- **API.** `SrpTrunk::connect(relay_url, server_id)` / `connect_with(..,
  TrunkConfig)` dials `<relay>/connect/<server_id>` with `TCP_NODELAY` (91)
  and completes `HELLO`/`READY` inside 8 s, which outlasts the relay's 5 s
  hold (97). `ws://` is refused unless the host is loopback.
  `trunk.request(method, path_and_query, headers, body: Bytes)` does
  `OPEN_STREAM` → `STREAM_READY`, the head (headers as an object, never a
  `relay_peer_ip`), the body in frames of at most 256 KiB (100), then always
  one empty end-of-body frame, a `GET` included (92). It returns an
  `SrpResponse { status, headers, body: SrpBody }` once the head arrives.
- **The body streams.** `SrpBody` is a `Stream` of chunks, with
  `chunk()`/`collect()`. It ends cleanly only on the origin's plain `CLOSE`.
  `ERROR{stream_aborted}`, the raw `CLOSE{aborted: true}`, any other
  stream-scoped `ERROR` or a lost trunk is an `Err`, never a short body (101).
  Streaming rather than a collected `Vec` because §11 needs SSE. Each
  stream's channel is unbounded, so the trunk's one reader never waits on a
  slow consumer and one stream cannot stall the others. Dropping a body early
  sends `CLOSE{stream_id}`. Uploads are bounded by a 64-frame outbound queue.
- **Liveness** follows 97: `PING` every 15 s; with nothing heard for 45 s
  the trunk dies as `relay_silent`. Every stream still open, or still
  waiting for `STREAM_READY`, ends with `trunk_lost`. There is no
  open timeout: the relay answers `OPEN_STREAM` at once, so a relay that
  does not answer is caught by the heartbeat.
- **A `rate_limited` open no longer kills the trunk** (97's "Not changed").
  The relay refuses an over-cap `OPEN_STREAM` with a *trunk-scoped* `ERROR`
  that names no attempt (`connect.rs`), and keeps the trunk. It answers
  opens in order, so the client keeps pending opens in wire order (a lock
  spans the queue push and the send) and fails the oldest one.
- **`SrpError::code()`** is stable, because it surfaces as
  `error.data.storm_error`. The codes are `server_offline`
  (`server_unreachable` at `HELLO`), `relay_down`, `relay_silent`,
  `handshake_timeout`, `trunk_lost`, `trunk_superseded`, `stream_aborted`,
  `server_timeout`, `rate_limited`, `stream_closed`, `protocol_error`,
  `trunk_closed` (closed by this side), `invalid_relay_target`, and
  `relay_error` for a code this build does not know.
- Tests: `tests/srp_trunk.rs` runs 18 cases against a Rust fake relay that
  answers a request only when its end-of-body frame arrives. The fake has
  no timers; heartbeat timings are injected. The cases: end of body on every
  request, `GET` included; 16 MiB + 12 345 bytes in ≤ 256 KiB frames,
  byte-exact both ways; an aborted response, as `ERROR` and as the raw flag,
  failing only its own stream; a silent relay, which is `relay_silent`, with
  both a pre-head and a mid-body stream `trunk_lost`; `server_unreachable`
  as `server_offline`; eight interleaved streams without cross-talk; a
  `rate_limited` open that the trunk survives; the server's trunk lost;
  an early drop sending `CLOSE`; `close()`. There are also 10 unit tests and
  7 vector tests. Mutations were checked: dropping the end-of-body frame
  hangs 11 tests, and treating an abort as a clean end fails 1–3.
  `tests/srp_live.rs` (ignored) passed against a real `storm-relay` and
  storm-server on 18860/18861: `/v1/server`, a `POST` identity challenge
  verified, 16 concurrent `GET`s, and an unregistered id as `server_offline`.
*Revisit if:* SRP gains per-stream flow control (67), which would bound the
per-stream body buffer by window; or the relay starts naming the refused
attempt in its `rate_limited` reply, which would replace the FIFO inference.

**105b. mcp-connect races the address and the relays, and proves the winner
(mcp-connect S2).** *(2026-10-13, decision 105's second slice; vault *MCP over
the Relay — mcp-connect*.)* S3's `Connector` now has a real implementation
behind it: `src/race.rs` (`RaceConnector`, returning a `RaceTransport` enum of
`Direct | Srp`) and `transport::SrpTransport` over S1's `SrpTrunk`.
`--relay` is used, and `--addr` is optional once a relay is given (neither is
refused at startup).
- **The race copies `storm_connection.dart`** (SRP §2):
  1. The address and every self-hosted relay race together. A public relay
     is tried only once **all** of those have failed, so a fast public relay
     never beats a slow self-hosted one. The app also starts the public round
     after a 2 s timer; this does not, because the adapter has no UI to keep
     responsive and the timer is exactly how a public relay would beat a
     slow self-hosted one. Each candidate gets 20 s to connect and prove
     itself (`server_silent` after), which bounds the wait.
  2. A candidate wins only once `verify_identity` passes **over its own
     path**: the challenge is the first thing on a new trunk, so the trunk
     that proved the key is the one that carries it (this closes 105c's open
     item for relayed paths). An impostor on one path is one failed
     candidate; another path can still win.
  3. The losers are dropped as soon as one wins, which closes their trunks.
  4. When nothing wins, the code is the most telling failure:
     `identity_failed` (something answered and could not prove the key) >
     `server_offline` (a relay is up without the server, or the address
     refused) > the rest (`relay_down`, `unreachable`, `rate_limited`, …),
     the earliest candidate breaking a tie. The detail lists every
     candidate's failure.
- **Public or self-hosted.** There is no marker on the wire: `/v1/server`'s
  relay entries are `{url, public_address}` only. The app decides by host —
  `publicRelayHosts`, the hosts of relays Storm runs, which is empty in
  production because no Storm relay exists yet — and treats everything else
  as the operator's, which is the safe direction to err. The adapter does
  the same with `race::PUBLIC_RELAY_HOSTS`, also empty. When a Storm relay
  ships, its host goes in both lists.
- **The relay set refreshes (Q11).** After a win, `GET /v1/server` over the
  winner (no key; it is the `none` tier) gives the **registered** relays,
  which replace that `server_id`'s entry in a small JSON file, a mirror and
  not a merge, since the server is its only writer. The next connect races
  the flags ∪ the cache, flags first; the flags are only the seed. The file
  is `mcp-connect-relays.json` in `platform::user_state_dir()`
  (`$XDG_STATE_HOME/storm`, `~/Library/Application Support/Storm`,
  `%LOCALAPPDATA%\Storm`), private, written beside and renamed. A failed
  refresh changes nothing.
- **`SrpTransport`** collects the body, maps `SrpError::code()` straight onto
  `TransportError.code`, and sends `Host: localhost`. storm-server's dispatch
  replaces a relayed `Host` with its bind address anyway (`relay/dispatch.rs`),
  so what the client sends is never what rmcp checks. `localhost` is chosen
  because it is on every `mcp::allowed_hosts` list, so a server that did not
  rewrite would still accept it. Verified against a real server bound to
  `127.0.0.1`, where `allowed_hosts` is enforced (105d).
- **An idle limit on relayed requests.** No bytes for 120 s, the direct
  path's `read_timeout`, is `server_silent`, never resent. A relay before
  decision 106 drops a backlogged client's stream (`drop_backlogged_client`,
  105a's finding) and tells the client nothing, so without this a call could
  hang for ever; against a 106 relay it is a backstop.
- **Decision 106's two client signals, in the Rust client.**
  `ERROR{rate_limited, attempt_id}` with no `stream_id` fails the pending
  open whose `attempt_id` matches (an id no longer pending is ignored) and
  keeps the trunk; without `attempt_id` (an older relay) the oldest pending
  open is failed, as 105a did. `ERROR{client_backlogged, stream_id}` is
  `SrpError::ClientBacklogged`, code `client_backlogged`: a body error like
  `stream_aborted`, never a clean end, and it reaches `storm_error` as
  itself. Tests in `tests/srp_trunk.rs`: a relay that holds the first open,
  refuses the second by id and then readies the first (the FIFO rule would
  fail the wrong one), and a `client_backlogged` after a partial body that
  fails only its own stream.
- **A dead trunk is replaced before the next request.** `Transport` gained
  `is_closed()` (default `false`; a trunk's `is_closed` for SRP). The adapter
  reconnects on a closed transport, so a trunk that died while idle costs no
  request. A trunk that dies mid-call is still that call's one error
  (`trunk_lost`), never resent, and the next request re-races.
- Tests: `src/race.rs` (11): the public round under a paused clock (a timer
  would fire during the test's virtual 30 s), public only after every other
  candidate failed, an impostor not stopping another path, the error ranking,
  losers dropped, the candidate deadline, the tier rule, and the cache.
  `tests/mcp_race.rs` (10, the adapter over real trunks to fake relays and
  real HTTP to fake servers, all answering on events): an impostor at the
  address and one behind a relay, each never seeing the key while the other
  path wins; an impostor everywhere is `identity_failed`; a fast public relay
  losing to a slow self-hosted one; the public relay used only after the
  address and the self-hosted relay fail; `server_offline` vs `relay_down`
  and the two together; a trunk lost mid-call; an idle relayed call; the
  cache written and then used with no flags; one `Host`, `localhost`, on
  every relayed head. `tests/mcp_connect.rs` gained the no-path refusal and
  points the binary's state at a temporary directory.
*Revisit if:* a Storm-run public relay ships (fill both host lists, or add a
marker to `/v1/server`'s relay entries); every relay in use has decision 106
(the FIFO fallback for an unnamed `rate_limited` can go); or SRP gains E2E,
which would change what "public" costs.

**105c. `storm-runtime mcp-connect`: external MCP clients over stdio
(mcp-connect S3).** *(2026-10-13, operator design of 2026-10-13; answers 95's
open item; S1 is 105a, S2 105b)*
- **What.** Claude Code, Cursor and Claude Desktop start `storm-runtime
  mcp-connect --server srv_… --pk <b64url> --addr http://… [--relay wss://…]…`
  as a stdio MCP server, with the `stk_` key in `STORM_MCP_KEY` or
  `--key-file`. Each line it reads is one `POST /mcp`, and the answer is
  written back as a line. It is meant for the LAN too, so one config works at
  home and away, and it adds the server-identity check the plain HTTP config
  never had. `src/transport.rs` holds a `Transport` (one `HttpRequest` in, one
  `HttpResponse` or a `TransportError { code, detail }` out), a `Connector`
  that returns a transport **only after `verify_identity` has passed over that
  same transport**, and the direct implementations. `src/mcp_connect.rs` is
  the adapter. S2 adds an `SrpTransport` and a connector that races the direct
  address and the relays, verifying the winner. Until then `--relay` is parsed,
  logged once as unused, and `--addr` is required.
- **Why stdio is this simple.** `/mcp` is stateless (`with_legacy_session_mode
  (false)`) and answers plain JSON (`with_json_response(true)`), so there is no
  session to keep and nothing to resume.
- **Rules.**
  1. Nothing is sent before the challenge passes, the key included. An
     impostor is `identity_failed`, never "offline": once a path carried an
     answer, any failure to prove the pinned key (a 404, a wrong body, another
     `server_id`, a bad signature) is identity.
  2. A failed request is one JSON-RPC error (`-32001`, `data.storm_error`
     naming the hop) and is never resent. The direct path reports
     `server_offline` (connection refused), `unreachable` (no route, DNS, a
     connect timeout), `server_silent` (120 s with nothing read) and
     `stream_aborted` (failed after sending). A non-JSON-RPC HTTP answer is
     `unauthorized` / `forbidden` / `mcp_unavailable` / `rate_limited` /
     `http_error`, with `data.status`.
  3. A failed transport is forgotten, so the **next** request reconnects and
     re-verifies, with a fresh HTTP client and no pooled connection. Nothing
     migrates mid-call.
  4. The key never goes on argv: clap has no key flag, and a test reads
     `--help` for one. The key is never logged, and neither is a body. Logs go
     to stderr only.
  5. It holds no authority: it sends the client's key, and the server decides.
- **What rmcp 3.1.2 actually does**, measured against a real server:
  - **No batches.** A JSON array is a 415 (`did not match any variant of
    untagged enum JsonRpcMessage`), so the adapter splits a batch into one
    `POST` per message and writes the answers back as one array.
  - **A notification** gets 202 with an empty body, and nothing is written.
  - **A stdio client sends no HTTP headers, so the adapter supplies them.**
    `MCP-Protocol-Version` is the version `initialize`'s result negotiated, or
    the request's own `_meta` `io.modelcontextprotocol/protocolVersion` when
    present. rmcp refuses a `_meta` version with no header. `initialize`
    itself is sent without the header, since rmcp refuses a header that
    disagrees with its body. From 2026-07-28 rmcp also requires SEP-2243's
    `Mcp-Method` and `Mcp-Name` (`-32020 missing required Mcp-Method
    header`). The adapter always sends both, base64-wrapped (`=?base64?…?=`)
    when the value cannot travel bare. `Mcp-Param-*` is not sent: Storm's
    tools declare no `x-mcp-header`.
  - **`Accept` must name both** `application/json` and `text/event-stream`,
    or the answer is 406. rmcp may still answer with SSE in JSON mode when a
    handler emits anything before its result, so an SSE body is unpacked
    message by message.
- **Concurrency.** Requests run concurrently and are matched by their own id.
  `initialize` and notifications finish before the next line is read, so they
  reach the server in the client's order. On EOF it waits for in-flight
  answers, then exits 0.
- **Not covered.** Per-connection identity on plain HTTP: the challenge and
  later requests may use different TCP connections. This is the same exposure
  the host's `verify_server` has, and it closes with TLS or with S2's trunk,
  where the verified path is the connection.
- Tests: `src/mcp_connect.rs` (14, against a fake transport verified by the
  real `verify_identity`): `the_challenge_runs_before_the_key_is_sent`,
  `an_impostor_is_identity_failed_and_never_sees_the_key`,
  `a_tool_error_passes_through_untouched`,
  `a_transport_failure_is_one_error_never_resent_and_the_next_call_reconnects`,
  `notifications_write_nothing`, `concurrent_requests_are_answered_by_id`,
  `eof_exits_after_the_answers_in_flight`, the header, batch, SSE, refusal
  and key-source tests. `src/transport.rs` (5): identity pass and fail cases,
  and a refused connection being `server_offline`. `tests/mcp_connect.rs` (5,
  the real binary over real HTTP to an in-process fake): no key argument, a
  `--key-file` key sent only after the challenge, an impostor never seeing
  the key, a server that is down, and a missing key sending nothing.
  `tests/mcp_connect_live.py` (12 checks, a real storm-server on 18870–18879):
  `initialize`, `tools/list`, `list_vaults`, a `get_vault` round trip, a tool
  error, a 2026-07-28 call through the header checks, a batch, a wrong pinned
  key, and a bad key. It is run by hand for now; S4 wires the live suite into
  `make test-live`.
*Revisit if:* `/mcp` becomes stateful or starts streaming (the line-per-POST
model breaks); rmcp accepts batches; or a client needs server-initiated
requests (elicitation), which a stateless `/mcp` cannot carry back.
*105c addendum (2026-10-14):* a pinned key is base64url, which starts with
`-` one key in 64, and clap read `--pk -9…` as an unknown flag — CI's random
server key hit it. `--pk` now takes hyphen-led values
(`a_pinned_key_that_starts_with_a_hyphen_is_a_value_not_a_flag`, which fails
without it).

**105d. mcp-connect's live acceptance runs in `make test-live`
(mcp-connect S4).** *(2026-10-13, decision 105's fourth slice.)* The adapter
is now proven against the real server and the real relay on every live run,
by the same checks that prove `/mcp` itself.
- **The MCP suite runs three times.** `apps/server/tests/mcp_e2e.py` gained an
  adapter mode: with `MCP_CONNECT_BIN` set, every `/mcp` call goes over stdio
  to one long-lived `storm-runtime mcp-connect` (`MCP_CONNECT_ADDR`,
  `MCP_CONNECT_RELAY` pick the route) instead of HTTP, and an HTTP refusal
  comes back as `data.status`, which the harness turns back into the status
  each check already expects. A call with another key gets a one-shot
  adapter; the one call with no key at all stays on HTTP, since the adapter
  cannot send none. REST fixtures stay on `STORM_BASE`, which makes them the
  concurrent LAN side. `make test-live` runs it over HTTP (86), through the
  adapter direct (89: the 86 plus a clean exit, every call on the expected
  route, and no key in the log), and through the real `storm-relay` with
  `--addr` a closed port (89). Driving the existing suite rather than a copy
  is the point: a check added to it later is an adapter check too.
- **Re-runnable against one server.** A run through the adapter names its
  fixtures for the run (`MCP/Tornado-direct.md`, `demo/run-relay.spec.ts`)
  and starts by setting `/mcp` read-only, since an earlier run ends
  read-write. The HTTP run is unchanged.
- **`tests/mcp_connect_live.py` is in `make test-live`** (39 checks, its own
  servers and relays on 18870–18879). Its server binds `127.0.0.1`, not a
  wildcard, so rmcp's `allowed_hosts` is enforced and a relayed call proves
  the `Host` is accepted. S3's suite runs direct and relayed. Then: a relay
  alone; the registered relay set cached under `XDG_STATE_HOME`; a dead seed
  relay still connecting through the cached ones; a `base_version` diff3
  merge over the relay against a concurrent LAN edit (`merged: true`, both
  edits kept, and the LAN reads the merge); an impostor storm-server at
  `--addr` losing to the relay, which proves the real one; an impostor alone,
  and an impostor beside a dead relay, both `identity_failed`; a relay
  without the server `server_offline`; no relay `relay_down`.
- **The relay's per-IP `HELLO` limit shapes the harness.** A relay allows one
  address 10 `HELLO`s a minute (token bucket), and every mcp-connect process
  that tries a relay spends one, losers included. A harness spawning a
  process per check ran out (`rate_limited`) in its first draft. So
  `mcp_connect_live.py` runs two relays and gives each connect a fresh state
  directory unless the check is about the cache, since a cached set of two
  relays dials both. A real MCP client runs one process for its session and
  is far from the limit.
- **No server or relay defect found.** `make test-live PORT=18880
  AUTH_PORT=18881 RELAY_PORT=18887` and `make check` green.
*Revisit if:* the relay's `HELLO` limit becomes configurable (one relay would
do), or `/mcp` becomes stateful (one long-lived adapter per suite would then
also be testing session reuse).

**108a. A black-box conformance suite is what makes a relay an SRP v1 relay.**
*(2026-10-13, hosted relay phase 5 step 1; vault *Hosted Relay — Cloudflare*.)*
storm-relay's tests start it in process and read its tables, so they prove
storm-relay and nothing else. A second relay (the Worker, 108b) needs a
contract it can be held to without sharing a line of code.
`tools/relay-conformance` is a standalone crate that knows a relay only by its
URL: it plays the origin and the clients over WebSockets, and reports each of
its 54 checks under the `docs/srp-v1.md` section it enforces, so a failure
reads as a spec violation. It reads `docs/srp-vectors.json` for the invalid
`server_id`s and key spellings.
- **The fixture contract** (committed first, for 108b): 48 fixed identities
  (`fixtures/servers.json`, seeds derived and checked against
  `gen-fixtures`), the allowlist a relay loads, and `fixtures/relay.env` — the
  shortened timers and limits, named as storm-relay's `STORM_RELAY_*`
  environment. A relay under test runs in allowlist mode at `RELAY_URL`;
  `RELAY_TOFU_URL`, optional, runs without one.
- **storm-relay gains runtime knobs** for all ten (challenge TTL, `HELLO`
  wait, ack timeout, heartbeat deadline, drain, `HELLO` limit and window,
  in-flight and total caps, client buffer), defaulted to the `config`
  constants. It had none: every one was a constant only an in-process test
  could change.
- **Black-box vs storm-relay-only.** The suite covers every MUST observable on
  the wire. What needs internals or a restart stays in `apps/relay/tests`:
  pinned nonce sources (a spent nonce never resurrected), TOFU persistence
  across a restart, `registrations`/`tracked_count` assertions, the budget's
  refund, TLS, the vectors as functions, and storm-relay's own choices the
  spec does not make (path/`HELLO` mismatch, head shape, non-boolean
  `aborted`). The README lists them, and the readings the suite takes where
  the spec is implicit (per-client in-flight cap, `stream_closed` for a
  foreign id with the trunk kept, a held `HELLO` and drain of at least 80 %).
- **Two of its own first readings were wrong, and the spec was right:** a
  stream-ending `ERROR` may overtake the frames queued for its stream (§5.4
  rule 2), so `stream_aborted` can precede the partial body, and storm-relay
  then still delivers a queued head after it (discarding is a SHOULD; it
  drops only body frames). The checks accept both.
- `make relay-conformance RELAY_URL=…` runs it against any relay;
  `make relay-conformance-local` starts storm-relay twice (allowlist on 18900,
  TOFU on 18901), runs it with the latency check (decision 91) and, on Linux,
  the `127.0.0.2` per-address checks, and stops them. CI runs it in the
  `relay` job. `make check` lints and unit-tests the crate.
- Tests: 54/54 against storm-relay (release and debug builds, repeated). It
  fails on the pre-106 relay (`client_backlogged`, both `attempt_id`
  refusals) and on a relay mutated to forward a client's `relay_peer_ip`, to
  route a foreign `stream_id`, or to supersede without a drain. No storm-relay
  defect found.
*Revisit if:* the spec's implicit readings above are written into
`srp-v1.md` (then they become plain MUSTs here), or a relay needs a
knob the contract does not name — add it to `relay.env` and storm-relay
together.

**105e. mcp-connect in Settings and the installer (S5).** *(2026-10-13,
"MCP over the Relay — mcp-connect", the product surface; stacked on 101)*
- **Why.** External MCP clients reach Storm through `storm-runtime
  mcp-connect` (the stdio adapter 95 recommended, built in parallel), on the
  LAN too, so one config works at home and away. Nobody should assemble that
  config by hand: it carries a server id, a public key and relay URLs.
- **The key reveal writes it.** Settings ▸ Devices & access ▸ New key now
  offers a `claude mcp add … -- storm-runtime mcp-connect …` line and the
  stdio JSON entry first (`lib/ui/settings/mcp_connect_config.dart`); the old
  `type: http` entry moves under *Other clients (HTTP, LAN only)*. Still shown
  once. Filled from what the device pinned at pairing, never from an
  unverified answer: `--server` = `Settings.serverId`, `--pk` =
  `serverPublicKey`, `--addr` = `baseUrl` (the pairing address; a relayed
  route lives only in `StormConnection`, so it can never leak into `addr`),
  `--relay` = each registered relay's `url` from `Settings.relays`, the copy
  of `GET /v1/server` the connection keeps current. A device with no pinned
  identity gets the HTTP entry alone.
- **The key is in env, never argv** (`STORM_MCP_KEY`): `-e` for Claude Code,
  `env` in JSON. Shell words are quoted only when they need it, and a value
  starting with `-` is written `--flag=value`, since a base64url key does one
  time in 64 and clap would read it as a flag.
- **Installer: MCP only** (`--mcp`, and on the first screen). Downloads the
  runtime's release asset (the `x86_64-linux` binary, or the macOS universal
  tarball), verifies it against `checksums.txt` and copies it to
  `/usr/local/bin/storm-runtime`: no package, account, service or
  enrollment. On Linux a machine with the `storm-runtime` package needs
  nothing (a note, no steps), and the Runtime Host's detection now ignores
  the MCP copy (`rt_on_path`), which would otherwise read as an installed
  host. A Homebrew link at that path is left alone. The last step runs
  `mcp-connect --help`, so a binary that predates it fails visibly.
  **Unavailable until a release has `mcp-connect`:** `STORM_MCP_CONNECT_SINCE`
  is empty, which the checklist shows as the reason; set it in the release
  that ships it. `--from-dir` skips the gate (someone's own build).
  `install.ps1 -Mcp` mirrors it on Windows: `%LOCALAPPDATA%\Programs\Storm`,
  on the user PATH, no administrator.
- **Docs** (README, `deploy/README.md` *MCP clients*, `docs/storm-mcp.md`)
  describe it as unreleased. `apps/www` is untouched.
- Tests: `test/mcp_connect_snippet_test.dart` (relays present and absent, the
  key only in env, the documented shape, a real `sh` splitting the line back
  into the intended words, the reveal with and without an identity);
  `tools/installer-tests/mcp.bats` (15: the gate, plans and runs on both
  OSes, nothing but the copy, the check, the Runtime Host and Homebrew cases,
  status/upgrade/uninstall, the first screen).
- Open: whether Claude Desktop on macOS finds `/usr/local/bin` on its PATH
  (the hint says to use the full path); the `claude mcp add` line is POSIX
  shell, so Windows users take the JSON entry.
*Revisit if:* mcp-connect's flags change, or it learns the relay set itself
from the server (then `--relay` becomes optional here).

**108c. Acceptance: both relays pass one suite and the whole live suite, and
the spec says what they agree on.** *(2026-10-14, phase 5 step 1, slice C of
decision 108; vault: *Hosted Relay — Cloudflare*.)*
- **From make, both relays.** `make relay-conformance-cf` starts two
  `wrangler dev` instances with the fixture contract
  (`apps/relay-cf/scripts/dev-relay.sh`, allowlist on 18910, TOFU on 18911)
  and runs the 108a suite with `RELAY_MULTI_IP=1` and without
  `RELAY_LATENCY`; a `relay-conformance-cf` CI job runs it beside `relay-cf`.
  `make test-live RELAY_IMPL=cf` (default `rust`) starts the Worker in
  allowlist mode with the dev server's own `GET /v1/server` key instead of
  storm-relay and registers the server exactly as before; every relayed suite
  then crosses the Worker. `mcp_connect_live.py` takes the same switch: its
  two relays become two Workers allowlisting its server, started after the
  server so its key is known. Two still, for the reason they were two: the
  Worker's `HELLO` limit is storm-relay's 10 a minute per address, and the
  live suites run at production limits.
- **Results.** Conformance: storm-relay 55/55, the Worker 54/54 (+1 latency
  skipped). `make test-live`, identical on both (`rust` / `cf`): server e2e
  81, mcp e2e 86, agent 93, gateway 95, auth 72, mcp-connect direct 89 and
  over the relay 89, `mcp_connect_live.py` 39, client integration 30 (+1
  skip, the agent host test needing its own env) including the relay live
  tests — 20 MiB upload, 17 MiB download, the change feed, MCP through the
  relay. The first `cf` run was green before any fix: nothing in the live
  suite fails only on the Worker.
- **Spec amendments** (`docs/srp-v1.md`, each marked *(108c)*), so both
  relays are held to text rather than to each other:
  - The suite's readings (108a) become rules: a foreign, unissued or closed
    `stream_id` from a client is `stream_closed`, never forwarded, trunk
    kept (§3); a nonce is answerable for its TTL only, and a late response is
    `auth_failed` or the socket closes without `REGISTERED` (§4); the drain
    is the whole window (§4.2); the `HELLO` wait is the whole wait; a refused
    `HELLO` is trunk-scoped `rate_limited`, before or after the `HELLO` is
    read (§5); the in-flight cap is per client trunk (§5.1); a stream-ending
    `ERROR` may overtake its stream's queued frames, which may trail it, the
    others keeping order (§5.4); the direction column is normative, a
    message from the wrong party is `protocol_error` (§6); `protocol_error`
    is trunk-scoped, closes the connection and has one fixed message (§6);
    the concurrent-stream bound is per server trunk, refused by `attempt_id`
    (§6).
  - The Worker's (108b): `relay_peer_ip` comes from the TCP peer or an
    address the provider edge attests and a client cannot set (§5.2); every
    `PING` is answered with a `PONG` in order, never dropped on a full queue,
    and a relay may `PING` any trunk to learn what its peer has read (§4.2);
    a relay that cannot hold more for the origin may end a stream
    `ERROR{rate_limited, stream_id}` + `CLOSE` to the origin (§5.4, §6); a
    `PING` may arrive at any point on a trunk, handshake included (§4.2); a
    client sends nothing but `PING`/`PONG` before `READY`, and a relay may
    refuse it with `protocol_error` (§5).
  - The Worker's (4) asked only that answering be allowed; with (2) a MUST,
    refusing a handshake `PING` (storm-relay's behaviour) would break it, so
    the text makes answering the rule and storm-relay changed.
- **Implementations changed for them, each with a test:**
  - **storm-server dropped `PONG`s** (a defect): it answered a relay's `PING`
    with `try_send` onto the bounded outbound queue, so behind a full queue
    the `PONG` was lost — exactly when the Worker's watermark needs it, and a
    lost one reads as a server that stopped reading. `PONG`s are now owed
    through a counter the writer drains ahead of the queue
    (`a_relay_ping_is_answered_even_behind_a_full_outbound_queue`, red
    before).
  - **storm-relay** refused a `PING` before `REGISTER_SERVER`, mid-handshake
    or before `HELLO` with `protocol_error`; it now answers it
    (`recv_handshake_control`; two tests).
  - **The Worker** refused the same three: the `/register` hop's first frame
    had to be `REGISTER_SERVER`, and the room answered only the
    auto-response's exact bytes before `READY`/`REGISTERED` (any other
    spelling, or a client `PING` during the `HELLO` wait, was
    `protocol_error`). Both answer now (two workerd tests; 87 in all).
  - Both clients already answered a relay's `PING` before and after `READY`,
    treated a stream-scoped `rate_limited` as that stream's failure, and wait
    for `READY`; tests now pin each (Rust `srp_trunk`, Dart
    `srp_tunnel_test`/`srp_failure_test`).
  - The suite gains `a_ping_is_answered_at_any_point_on_a_trunk` (§4.2), red
    on both relays before the fixes above.
- **Deploy runbook**: `apps/relay-cf/DEPLOY.md` and a section in
  `deploy/README.md` — the operator's own account on the Free plan (SQLite
  Durable Objects), `wrangler login`/`deploy --env=""`, a custom domain, the
  allowlist as a secret from `GET /v1/server` (`storm-server status` does not
  print the identity), `STORM_RELAY_PUBLIC_BASE`, Settings › Connection ›
  Relays, verifying with the suite against a separate `conformance` Worker
  (and `conformance-tofu` for the TOFU checks, or skip them), the host into
  `PUBLIC_RELAY_HOSTS` (the app now has one constant, `kPublicRelayHosts`,
  for it), what is logged (Workers Logs off; Q15 open), and the Free plan's
  limits (relay errors; local Storm unaffected). `wrangler.jsonc` gains the
  two `conformance` environments. Not in `release.yml` or `apt-repo.yml`.
- **Not verifiable here, only on a real deploy:** that the edge overwrites a
  client-sent `CF-Connecting-IP` (under `wrangler dev` a client can set it),
  that it carries a 16 MiB WebSocket message, and whether the `/register`
  hop stays inside the Free plan's CPU limit for a busy trunk. The runbook
  says how to check each.
*Revisit if:* a deploy shows the `/register` hop's CPU (then an optional
`server_id` on the upgrade); or step 2 (relay accounts, Q14, Q15) changes
who may register.

## Data model

A note is a `.md` file. Frontmatter carries identity:

```yaml
---
id: 8f3a2c10-...      # stable UUID, assigned once, never reused
created: 2026-08-05T10:00:00Z
modified: 2026-08-05T10:04:12Z   # server-owned; clients must not write it
tags: [homelab, project]
---
```

**Frontmatter round-trips losslessly.** Real vaults carry arbitrary keys,
hand-chosen order, comments and inconsistent quoting. Storm never serializes
YAML — it replaces or inserts individual *lines* and passes every other byte
through. Serializing would reorder keys and reformat values, dirtying every file
in the vault on first scan.

SQLite schema (`state/index.db`): `notes`, `note_versions` (full snapshots),
`links`, `tags`, `notes_fts` (FTS5), `attachments`, `devices`, `change_log`.

`change_log` is the delta-sync primitive: every mutation from any source (API,
watcher, startup scan) appends a row, and a client asks `GET /sync?since=<seq>`.
No manifest diffing.

**M9 makes this per vault**: the index moves to `state/<vault-id>/index.db`, so
`seq` and every table above are scoped to one vault, and a client's sync cursor
is per vault too. Two tables are added, both pure `CREATE TABLE IF NOT EXISTS`
additions that need no migration mechanism:

- `folders (path, created)` — folders created explicitly rather than derived
  from note paths. It exists for exactly one reason: `prune_empty_parents`
  deletes directories that become empty, and without an exemption a new empty
  folder disappears the moment its last note leaves.
- `note_access (note_id, opened_at)` — when a note was last opened, feeding the
  cross-vault recents list. Separate from `notes` on purpose; see decision 23.

A `PRAGMA user_version` guard lands with them so the next change that needs an
*altered* column has somewhere to live. Additions never did.

The registry lives outside any index, at `state/vaults.json`.

---

## Milestones

### M0 — Editor spike ✅

Answered the gating question: a Flutter `TextField` with a custom
`TextEditingController` works.

| Document | typing p95 | caret movement |
|---|---|---|
| 1,000 lines | 0.28–0.83 ms | 0.000 ms |
| 4,800 lines | 3.0–3.8 ms | 0.000 ms |

Findings that constrain later work (detail in `docs/editor-findings.md`):

- The span tree must flatten back to the buffer **exactly**; any gap or overlap
  silently corrupts rendered text and every caret offset after it. Asserted
  across 42 hostile cases.
- Above ~1,000 lines the cost is span-tree *assembly*, not tokenization — O(lines)
  per keystroke regardless of caching. `maxStyledLines = 5000` degrades to
  unstyled as a backstop. On Android assume 3–5× the desktop numbers.
- `buildTextSpan` re-fires on cursor movement ([flutter#114158]), which the
  whole-span memo neutralises.

[flutter#114158]: https://github.com/flutter/flutter/issues/114158

### M1 — Rust server core ✅

`apps/server/` — 3,527 lines, 91 tests, 0 clippy warnings, plus 43 end-to-end checks
against a live server (`apps/server/tests/e2e.py`). Full API and behaviour documented
in `apps/server/README.md`.

Verified against a realistic fake Obsidian vault: `--dry-run` writes nothing;
import assigns UUIDs while preserving hand-written frontmatter byte-for-byte;
restart reconcile is a true no-op; `#not-a-tag` inside a code block is excluded;
a note survived fast-forward → merge → conflict → move with identity intact.

**Three bugs found here — do not reintroduce:**

1. `set_scalars` computed line indices against the original text while mutating
   the line vector. Inserting `id` shifted `modified` down, so the next write
   landed on the `id` line and destroyed it.
2. The `modified:` timestamp leaked into merges. The server rewrites it on every
   save, so it differs between base and server in *every* reconciliation —
   without normalising it out, every concurrent write would conflict.
3. Conflict marker labels were inverted; the other device's text was labelled
   `ours`.

**Known limitation:** diff3 is line-based with context, so *adjacent* edits
conflict even though they don't overlap — deleting a paragraph while another
device edits the next one is reported as a conflict. Rare for a single-user
vault. Frequency here is the trigger for revisiting decision 2.

### M2 — Client vertical slice ✅

`apps/client/` — the online-only path, since superseded by M3. See
`apps/client/README.md`, especially the save-protocol section.

Online-only by design: no cache, no outbox. Those are M3 and layer *above*
`StormApi`, not inside it.

The save protocol in `lib/state/note_session.dart` is where an edit can silently
vanish, so it is deliberately paranoid and directly tested:
- `merged`/`conflict` responses mean the server reconciled against a version this
  client never saw — it **adopts the server's text**.
- Typing during an in-flight save keeps the session `dirty`, not `saved`.
- A *failed* save also stays `dirty`, because the edit exists only in that buffer.

Two bugs fixed: Riverpod 3 API drift (`valueOrNull` → `value`;
`StateProvider`/`ChangeNotifierProvider` moved to `legacy.dart`), and SPA deep
links returning HTTP 404 with `index.html`'s body — tower-http's
`not_found_service` keeps ServeDir's status, breaking caching and the Flutter
service worker. Use `.fallback()`.

### M3 — Cache, outbox, offline, merge ✅

`SyncEngine` (`lib/sync/sync_engine.dart`) owns the drift cache, the outbox and
the connection. Everything above it — `NoteSession`, the UI — reads and writes
through it and never touches `StormApi`, so offline behaviour is implemented
once rather than at every call site.

Design points worth keeping:

- **Online/offline is inferred from whether requests actually succeed**, not
  read from a connectivity plugin. A device can be on wifi with the homelab
  unreachable (VPN down, server restarting, wrong subnet) and only a real
  request tells those apart.
- **A socket failure and an HTTP refusal are treated differently.** The first
  is queued and retried; the second never is, because retrying a request the
  server already rejected would wedge the queue behind it.
- **Coalesced edits keep the *original* `base_version`.** Advancing it would
  tell the server there is nothing to merge, silently clobbering whatever
  landed while the device was away.
- **A pull never overwrites a note with an unsent edit** — the outbox copy is
  the only one that exists.
- **The WebSocket only signals *that* something changed**; the authoritative
  list still comes from `GET /sync?since=`. A dropped frame therefore costs
  nothing. Reconnect uses capped exponential backoff (1s → 60s).
- A queued **move** outranks a later edit on the same note: one outbox row per
  note means a plain `update` would drop the rename, so the drain replays the
  move first, then the content.
- Eviction never touches pinned notes or notes with queued edits.

**Exit criterion — the sync matrix. All nine now covered by tests:**

| # | Scenario | Expected |
|---|---|---|
| 1 | Edit on A while B is closed | B shows it on next open |
| 2 | Both online, edit same note A then B | B sees A's change over WS before its own save |
| 3 | A offline, edit, reconnect | Outbox replays, clean 200 |
| 4 | A offline edits para 1, B online edits para 3 | Merges cleanly, no markers |
| 5 | A offline and B edit the same line | Markers, `conflict: true`, pre-merge version recoverable |
| 6 | A offline renames; B edits content | Both survive — the UUID payoff |
| 7 | Edit a file with `nvim` on the server | Watcher picks it up, clients update |
| 8 | Kill the server mid-write | No truncated files; index reconciles on restart |
| 9 | Delete on A while B has it open | B is told; no zombie resurrection |

Where each is proven:

| # | Covered by |
|---|---|
| 1, 2, 9 | `apps/client/test_live/two_client_sync_test.dart` — two real clients, real WebSocket |
| 3, 4 | `two_client_sync_test.dart` (offline edit → reconnect → merge) and `test/sync_engine_test.dart` |
| 5 | `test_live/live_server_test.dart` + `test/sync_engine_test.dart` |
| 6 | `test/sync_engine_test.dart` (offline rename + offline edit both survive) |
| 7 | `apps/server/tests/e2e.py` (watcher picks up an external edit) |
| 8 | M1: atomic writes + restart reconcile, verified during the M1 e2e run |

Scenario 2 in particular could not be reached by unit tests — `MockClient` has
no WebSocket — which is why `test_live/` exists.

**Bug found at scale, worth not reintroducing:** `_pull()` fetched a single
page of changes (limit 500) and then advanced `lastSeq` to the server's
*latest*, silently skipping everything past the first page. Invisible on a
small vault; the 600-note vault surfaced it immediately. It now pages until
caught up, only adopting the server's position when a short page proves there
is no more. Regression test: `sync_engine_test.dart`, "pages through more
changes than fit in one response".

**Still deferred to M5:** `drift` on web needs `sqlite3.wasm` and
`drift_worker.dart.js` in `web/`, served with `Content-Type: application/wasm`,
plus COOP/COEP headers so Chrome uses OPFS rather than falling back to
IndexedDB. Untested on web so far.

### M4 — Search, tags, backlinks ✅

Server: FTS5 maintained on write, link/tag extraction that excludes code, a
backlinks query, and query sanitizing so punctuation isn't an FTS5 syntax
error.

Client: the sidebar is now Files / Search / Tags. The tag browser groups
hierarchical tags on their first segment (`proj/storm` under `proj`) — a flat
list of forty siblings is unusable — while keeping an exact parent tag distinct
from its children. Linked mentions sit collapsed under the editor rather than
in a third column: it's a reference, not something to keep in view while
writing, and a third column doesn't fit on a phone.

Tags and backlinks are **server-only** by design: resolving them needs the
whole vault's index, which the client deliberately doesn't hold. Offline the
panels say so rather than showing an empty list, which would be a different and
wrong claim.

**Measured against a generated 600-note vault:**

| | |
|---|---|
| search | p50 0.9 ms, **p95 1.1 ms** (criterion was ~100 ms) |
| tags | 33 distinct, 0.3 ms; listing a tag matches its count |
| backlinks | 0.2 ms, resolved correctly across folders |

Reindexing is subtractive as well as additive — deleting a note, or editing a
tag out of one, removes it from the index. Covered by live tests.

### M5 — Android, Linux, Web ✅

**Web — wired, not yet run in a browser.**

`drift` on web needs two assets that are *not* pulled in by pub. They are now
committed to `apps/client/web/` and shipped in the bundle:

| | |
|---|---|
| `sqlite3.wasm` | 749 KB, from `sqlite3.dart` release `sqlite3-3.5.1` |
| `drift_worker.js` | 355 KB, from `drift` release `drift-2.34.3` |

Both are version-matched to `pubspec.lock`; bumping `drift` or `sqlite3` means
re-downloading them.

storm-server sets `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` when `--web` is given. Without
cross-origin isolation the browser withholds `SharedArrayBuffer` and drift
silently drops from OPFS to IndexedDB — slower, and on Chrome for Android it
loses cross-tab safety. Everything Storm serves is same-origin, so isolating
costs nothing. `application/wasm` is inferred correctly by tower-http.

**Bug this uncovered:** `driftDatabase()` *throws* on web unless a `web:`
argument is supplied, and `CacheDb` wasn't passing one. It compiled and passed
every test — native builds ignore the parameter — and would have crashed the
moment a browser first touched the cache. Now supplied, with an `onResult`
hook that logs which backend the browser actually allowed.

**Still to verify:** nothing has opened the app in a browser yet. There is no
Chrome and no webdriver on this machine, so it cannot be automated here.
Manual check:

```sh
cd apps/client && flutter build web --release
cd ../server && cargo run --release -- --vault <v> --state <s> \
    --token testtoken --port 8484 --web ../client/build/web
# then open http://127.0.0.1:8484
```

Watch the console for `storm: cache using …` — it only prints when the browser
withheld a feature, so silence means OPFS was available.

**Android — runs, verified against the homelab VM.**

Toolchain: OpenJDK 21 + cmdline-tools + platform-tools + build-tools 36 +
platform 36 + NDK 28 + CMake. The NDK is not optional —
`sqlite3_flutter_libs` compiles SQLite from source. Budget ~12 GB of disk for
all of it, not the ~600 MB the SDK alone suggests.

Verified end to end on a Pixel 10 (Android 17, API 37) against `storm-server`
on an Ubuntu VM at `192.168.91.51:8484`, over wifi: vault tree loads, folders
nest, notes open, edits save, new notes are created — and **no note's version
moved during 20 seconds idle**.

That deploy doubles as an M6 rehearsal: a cross-compiled static binary copied
to a machine with no Rust, no libc setup, nothing.

*Closed:* the M0 perf gate ran on the Pixel — **8.6 ms p95** at 5,000 lines
against a 16.7 ms budget. See `docs/editor-findings.md`.

**macOS — no longer blocked.** `flutter build macos` succeeds and produces
`storm.app`, even though `DVTDownloads.framework` is still v17.0 against Xcode
26.6. Whatever was wrong resolved itself; `make client` works.

**Linux desktop — deferred, deliberately.**
It cannot be built from a macOS host at all (`"build linux" only supported on
Linux hosts`), and no Linux workstation is in play. Nothing else depends on
it: the *server* already runs on Linux (that is where it deploys), and a
Linux user can reach the vault through the web client, which is served from
the same binary. Revisit if a Linux desktop actually needs the native app.

**Deploying to a Linux host** does *not* need Rust installed there. Cross-
compile a static musl binary and copy it:

```sh
rustup target add x86_64-unknown-linux-musl
cargo install cargo-zigbuild            # needs zig; handles the bundled SQLite C
cd apps/server && cargo zigbuild --release --target x86_64-unknown-linux-musl
```

68 s, 5.4 MB, statically linked, zero runtime deps. This is the M6 deploy
mechanism.



### M6 — Attachments, settings, deploy ✅

**Attachments — done, both halves.**

They live in the vault beside the notes as ordinary files, so it stays one
greppable, rsync-able tree with no separate blob store. Opaque blobs: no
parsing, no merge, last write wins. The startup scan indexes any already on
disk, so an existing Obsidian vault's images come across without a separate
import.

Server: `GET/PUT/DELETE /v1/attachments/{path}` plus a listing, capped at
64 MB (axum's 2 MB default would reject most photographs), with real content
types so browsers display rather than download. Verified with a real PNG:
byte-identical round trip, traversal refused including percent-encoded, 70 MB
rejected with 413.

Client: an attach button uploads and links the file. Never queued offline —
the outbox is for small text diffs, not megabytes of binary.

**Images can't render inline**, and won't until the editor changes. A
`WidgetSpan` contributes exactly one character to the span tree while
`![alt](path)` is many, and the buffer has to match what it renders character
for character. They appear as a thumbnail strip below the editor instead,
tappable for a zoomable view. True inline images need the block-based editor
from the M0 findings.

**Settings** — server URL, token, theme, font size and disconnect all exist.

**Deploy — built and verified, awaiting install.**

`deploy/` holds a systemd unit, a nightly backup timer, and an env file for the
token (kept out of the command line, since `/proc` exposes arguments to every
local user). `make deploy` cross-compiles the static binary, pushes it with the
web bundle, restarts, and fails loudly if the service doesn't come back — the
hand-typed `nohup` line had already cost one mistake, starting the server
without `--web` so the browser got a bare 401.

The backup covers both halves for different reasons. The vault is plain files,
so `rsync -a --delete` into a dated directory. The index is **not** rsynced: it
runs in WAL mode with the server holding it open, so a file copy can catch
committed pages still in the `-wal` and yield a database that opens but
silently lacks history. `storm-server --backup-db` uses SQLite's `VACUUM INTO`,
which is correct against a live database. The script then reopens the snapshot
to check it works.

Verified end to end locally, against a **running** server: back up, delete the
vault and state outright, restore by the documented steps, and the server comes
back with both versions of an edited note still in `note_versions` — the merge
base, which is the one thing a rescan cannot rebuild.

Two hazards found while building it, both now guarded: the verify step wrote
its probe to `/dev/null`, and `snapshot_to` deletes its destination first, so
it would have removed the device node. `snapshot_to` now refuses anything that
isn't a regular file, with a test.

*Remaining:* running the one-time install on the VM. It needs `sudo`, which is
the user's to give — see `deploy/README.md`.

---

### M7/M8 — UI refactor ✅

`docs/storm-ui-refactor.md` traded the desktop-first drawer shell for a
dashboard, a floating nav bubble, a breadcrumb browser and a keyboard
formatting toolbar. The old shell was 670 lines holding the shell, the note
actions and the dialogs at once, and it compressed badly at phone width — which
is the width this project exists for.

**Routing came first**, and paid for itself immediately. The Context slot has to
answer "where am I", and a router makes that one source of truth instead of a
flag kept in agreement with the screen. It also gave the web client deep links
it never had: `/browse/Projects/Storm` and `/note/:id` now serve the app rather
than 404.

**A fifth bug got through anyway, and it is the instructive one.** The heading
button did nothing on the phone — worse, it silently *stripped* headings. Every
existing test passed, because they all asserted that a prefix could be added to
a line that did not have one. Nobody tried the picker on a line that was already
a heading, which is the first thing a user does, since notes open with
`# Title`. See decision 12. The lesson is not "write more tests"; it is that a
test asserting the happy transition is not a test of the control's *semantics*.

**Three of the first four bugs here were found by tests, not by running it** —
which is new for this layer, and the whole point of writing the suites before
wiring the screens to real state. See decisions 10–13 for what each one taught.
The exception is worth noting: the *fourth* was that the app opened in light
mode despite a comment claiming dark-first, and only looking at a screenshot
from the phone caught it. Both halves of the lesson below still hold.

**Wikilink autocomplete** landed in its own pass afterwards, as planned.
`docs/storm-ui-refactor.md` §2.6 had assumed it already existed; nothing in
`lib/` matched, so it was held back rather than bolted onto a refactor. Typing
`[[` now offers matching notes above the formatting toolbar, and picking one
completes the link and steps the caret past the brackets.

**Still not possible:** true inline images and true syntax hiding. Both need the
block-based editor rewrite of decision 5, for the same reason as always — a
`WidgetSpan` contributes one character where `![alt](path)` is many, so a chip
"in the text flow" breaks the buffer invariant exactly as inline rendering does.
`AttachmentStrip` below the editor remains the honest interim.

---

### M9/M10 — Multi-vault, folders, storage root ✅

One server owns a storage root; each directory under it is a vault, tracked by
UUID via `state/vaults.json`. Every note-shaped route gained a
`/v1/vaults/{id}` segment, `AppState` went from one `Mutex<Indexer>` to a
`RwLock<VaultSet>` of them, and the dashboard became a grid of vaults over a
cross-vault "recently opened" list. Folders became a real thing you can create,
rename and delete. Full design in `docs/storm-multi-vault.md`.

**The watcher got simpler, not harder.** The obvious shape was one watcher per
vault plus a shutdown path for each. Watching the *root* and attributing each
event by directory prefix means adding or removing a vault needs no watcher
work at all, and a root change just respawns the one watcher.

**Three things that looked small and were not:**

1. *The client cache had no migration strategy at all.* `cache_db.dart` was
   `schemaVersion => 1` with no `MigrationStrategy`, so the default only ever
   ran `createAll`. Adding `vaultId` meant writing the first real migration
   this project has had. `Outbox` rows are edits that exist nowhere else, so
   when the vault cannot be inferred the migration stamps them `legacy` and
   leaves them, rather than guessing which vault they belong to.
2. *`Meta.lastSeq` was a single global key.* Two vaults would have overwritten
   each other's sync cursor — silent, and indistinguishable from randomly
   missed changes. It is `lastSeq:<vaultId>` now.
3. *The `--vault` compatibility shim puts `state/` inside the root.*
   `--vault /srv/storm/vault` implies root `/srv/storm`, which contains
   `/srv/storm/state`, and a naive scan would have registered `state` as a
   vault and indexed the SQLite files in it. `scan_root()` skips `state_dir`
   unconditionally; `registry.rs` has a test for exactly that layout.

**The migration shipped broken, and the tests said it was fine.** `addColumn`
cannot change a primary key, and v2 moved both cache tables from
`PRIMARY KEY(id)` to `PRIMARY KEY(vaultId, id)`. The column arrived; the key
did not. Drift then generated `ON CONFLICT(vault_id, id)`, SQLite rejected it
outright, and **every cache write threw** on the phone.

The migration test passed throughout because it opened
`NativeDatabase.memory()`, which runs `onCreate` and produces a *correct*
schema — it never executed the upgrade path at all. *A migration test that
never migrates is not a migration test.* `test/cache_migration_test.dart` now
builds the v1 schema by hand, stamps `user_version`, and opens `CacheDb` over
it. Schema v3 rebuilds both tables with `alterTable`, and carries a separate
repair branch for devices that already ran the broken v2 — their `user_version`
is 2, so without it no later upgrade would ever fire and they would be stuck on
a schema no write can succeed against.

**What made it unreadable was a second mistake: cache failures were reported as
network failures.** `create` wrapped its cache write in the same `try` as the
request, so a local schema error surfaced as "Cannot reach the server — new
notes need a connection", marked the client offline, and made everything after
it fail as offline too — while the note sat on the server perfectly fine.
`_applyChanges` did the same and worse: it returned `false`, pinning `lastSeq`,
so every sync re-pulled the same page and failed the same way. That is what
"sync got slow" actually was. Cache writes now go through `_cache`, which logs
and continues. *The server is the copy of record; a local cache failure is
never a network one.*

**Two more bugs the new tests caught, both the same shape.** Cached writes were not
stamped with the vault, so every read missed and the editor opened empty. And
`treeProvider` read the engine without watching the active vault, so switching
vaults kept showing the previous one's notes. Both were invisible to the type
checker and to every existing test; both are the "scoped thing still treated as
global" failure this milestone is made of.

**The gate test that was not.** The first version of "switching vaults never
shows stale notes" passed with `VaultGate`'s guard deleted — `pumpAndSettle`
waits out the exact frame the gate exists for. The version that ships pumps a
single frame, and deleting the guard fails it. *A test that passes when you
remove the thing it names is not a test of that thing.*

**Known limitation, stated rather than discovered:** the registry rescans at
startup, on a root change, and on vault create/delete — not continuously. A
directory dropped into the root over rsync does not appear until a restart. The
root watcher covers note edits inside registered vaults; it does not register
new vaults.

**Deliberately not in this pass:** cross-vault search, tags and backlinks (each
vault has its own FTS index); per-vault tokens; two vaults live at once; moving
a note between vaults; and deleting a vault's files from the app — removing a
vault unregisters it and leaves every byte on disk.

---

### M11 — Typed note properties ✅

`docs/storm-properties.md`. The frontmatter block became an editable key/value
list — key chip on the left, an input suited to the type on the right, `+` to
add one — instead of a read-only strip of raw YAML above the note.

**The panel was read-only for a good reason, and the reason had a hole in it.**
Its doc comment said writing values back means re-serialising the user's YAML.
True, and the server doesn't re-serialise either: it splices lines. So the
client got a second writer on the same principle, richer than the server's
because a user's metadata is not a UUID — see decision 27.

**Three bugs the tests caught before the device did**, all in the writer, all
about context:

1. A comma inside an *inline list item* has to be quoted or the item splits in
   two. A comma inside a scalar does not. Quoting had to learn where it was.
2. A checkbox writing `true` was being quoted into the *string* `"true"`,
   because `true` is on the "YAML would misread this" list. Correct for text a
   user typed, wrong for a boolean the UI generated — hence `raw:`.
3. A negative number was quoted for the same reason, since `-` starts a flow
   sequence.

**One layout bug found the same way.** A fixed-width chip cannot hold a text
label at every font size; the test font made "Add property" 69px too wide. The
sketch had it right — the `+` button carries no label.

**Then raw mode went, on the user's call.** The first build kept an "Edit raw"
escape hatch and folded `id`/`created`/`modified` behind a "Details"
disclosure, with a rule under the whole panel. That reintroduced the split the
panel existed to remove: some metadata in the list, some behind a mode switch.
Now every key in the block is a row in file order, editable or read-only, and
the list runs straight into the prose. See decision 30.

**Two more from a phone screenshot**, which is where this project's UI bugs
keep being found:

- *Chips were slabs.* `IconButton` enforces a 48px minimum tap target, so a
  remove affordance built from one pushed every chip to 48px tall and the tag
  rows read as stacked grey blocks. A plain `InkWell` gets it to 26. The
  regression test asserts the height, and fails at 48 if the `IconButton` ever
  comes back.
- *The add affordance was a bare text field beside the chips*, which left the
  row looking unfinished. It is a badge now — outlined, secondary, `+` — that
  becomes an empty editable badge in place, so the control looks like what it
  creates.

**The nav bubble no longer collapses.** It hid behind a `…` until tapped,
costing a tap before every navigation and concealing where you could go. The
bar is five small icons; hiding it bought nothing.

**An unobserved Future, found by a full-suite run.** `WebSocketChannel.connect`
reports a failed handshake through `ready`, not by throwing, and nothing was
listening to it — so an unreachable host became an *unhandled* async error in
the zone. It surfaced only when two new tests shifted the timing, and it passes
in isolation either way; the evidence is the full suite, which fails without
the fix. Reconnection was always driven by the stream's `onError`; the missing
piece was simply observing the rejection. *On a device with no DNS this was
noise in the log at best.*

**Colours, fonts, and naming a note.** Keep-style accents for notes and
vaults, a note-font choice, and a new-note dialog that asks for a name.

- *A colour is a word, not a hex.* `color: sage` in the note's frontmatter
  stays readable, greppable and meaningful in Obsidian or a text editor;
  `#B7CDB0` would be none of those and would pin the vault to one theme. Each
  accent carries a light *and* a dark value, because a tint that works on
  white is a glare on black. A colour the app does not recognise is left as
  plain text rather than offered a swatch — overwriting somebody else's
  convention would be worse than not styling it.
- *Colour is edited in the properties list*, as `PropertyType.color`, not
  through a separate menu. Decision 30 said the list is the only way
  frontmatter changes, and a colour is frontmatter like any other value. A
  vault's colour goes in its own `_storm/vault.md` and is set by long-pressing
  its card, since a vault has no properties list.
- *Three fonts, not a font list.* Serif (the bundled Newsreader), the
  platform sans, and monospace. Every extra family is another megabyte in the
  APK, and a runtime download is wrong for something that must work offline —
  the same reasoning that bundled the serif in M7.
- *The `id` is hidden by default*, behind a setting. It is a UUID nobody
  reads, and it cost the top row of every note. Off rather than removed,
  because the list is the only place frontmatter is visible at all — this is
  the one exception to decision 30, and it is the user's switch, not a hiding
  place.
- *Read-only timestamps are formatted.* The server writes RFC3339 with
  nanoseconds, which is precise and unreadable; `created` and `modified` show
  as `5 Aug 2026, 12:52` in local time. Anything that does not parse as a date
  passes through untouched, so a value that merely resembles one is never
  quietly rewritten on screen.
- *A new note asks for a name.* Not `Folder/Note.md`: the folder is wherever
  you already are and every note is markdown, so neither was a decision worth
  asking about. Separators become spaces rather than folders, and leading dots
  are stripped, so a typed name can never escape the vault, hide itself, or
  invent a directory. A test asserts every generated name passes
  `validateVaultPath`.

**Still not writable:** nested maps and block scalars. A key/value row cannot
represent them without guessing at a structure, and writing the guess back
would destroy it. They are listed read-only, alongside Storm's own fields —
see decision 30, which removed raw mode and with it the last place metadata
could hide.

---

### M12 — Adaptive layout ✅

`docs/storm-adaptive.md`. The client had **no responsive handling at all**:
`GridView.count(crossAxisCount: 2)` gave 980×725 vault cards on a 2000px
window, a floating pill sat in the middle of the screen, and one pane showed at
a time when there was room for two. Now a wide window gets a folder-tree
sidebar beside the note, a flowing card grid, and recents in a rail.

**Two tests that passed for the wrong reason**, both caught by breaking the
code on purpose rather than by reading them:

1. *"the tree keeps its expansion when a note is opened"* — the guard that
   justifies the `ShellRoute` — passed with the tree's state deliberately
   discarded, because `find.text('Ideas')` also matches the note's own AppBar
   title once it opens.
2. Scoping it to the sidebar **still** was not enough: `_revealOpenNote`
   re-expands the ancestors of whatever note is in the URL, so opening a note
   from the folder under test reopens that folder even from a freshly built
   tree. It only became a real guard once it expands `Daily` and opens a note
   at the *root*.

*A test that exercises the feature is not the same as a test that would notice
its absence.* Both versions ran green against broken code.

**An assertion that was hiding a defect.** Three existing wide-screen tests
started failing with "Multiple exceptions detected", which turned out to be
Flutter's *"ListTile background color or ink splashes may be invisible"* — the
sidebar used a coloured `Container`, and `ListTile` paints its ripple onto the
nearest `Material` ancestor. Fixing it with a `Material` was not silencing an
assert; it made every tap in the tree actually ripple.

---

### M13 — MCP, read-only ✅

**Deployed 2026-08-08.** Server only — `git diff` over `apps/client` between the
APK on the phone and this commit is empty, so the mobile app needed no rebuild;
the phone is already running this code. Binary verified by comparing the local
build's sha256 against `/proc/<pid>/exe` on the VM
(`54c158cf…3801be1`), and the client-facing routes (`/v1/vaults`, `/v1/recents`,
`/tree`, the web bundle) all answer 200 over the LAN afterwards.

Two things the deploy itself taught:

- ***`make build-server` failed and left the previous binary in place.***
  `~/.cargo/bin` is not on a non-interactive shell's PATH, so the target's
  `command -v cargo-zigbuild` guard fired — and the stale 6 MB binary from
  2026-08-07 was still sitting at the output path, which a deploy that only
  checked "does the file exist" would have shipped. Worse, `rustc` on PATH is
  **Homebrew's**, which has no `x86_64-unknown-linux-musl` target even though
  `rustup` has it installed; the build has to run with
  `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin` first. Always compare
  the artefact's timestamp and hash, never its existence.
- *The VM binds `--host 0.0.0.0`*, so `mcp::allowed_hosts` correctly returns an
  empty list and logs `allowed_hosts=[]` at startup. The LAN check that matters
  was then done for real: `initialize` from this Mac against
  `http://192.168.91.51:8484/mcp`, which carries `Host: 192.168.91.51:8484` and
  would have been refused by rmcp's default.

Rollback is one command: `storm-server.prev` and `run.sh.prev` sit beside the
live ones, and the pre-swap index snapshot is at the path in
`/home/dewansh/.storm-last-backup`.

**Two things the first live use turned up**, both fixed the same day:

- ***Storm's own config note was discoverable as one of the user's notes.***
  Searching the real vault through MCP returned `_storm/vault.md`. Pre-existing
  REST behaviour, not new — but the Flutter client had been filtering `_storm/`
  at **five separate call sites**, so the rule lived in the callers rather than
  in the query, and MCP was a caller that did not know it. Now one `NOT_CONFIG`
  predicate in `db.rs` covers search, recents, tags and tag listings.
  Deliberately *not* the tree or `get_note_by_path`: the client reads that note
  to load a vault's colour and property types.
- ***The endpoint could only be switched at boot.*** `--mcp` decided whether the
  route was mounted, so turning MCP off meant an SSH session — for the one
  setting most likely to be wanted in a hurry. `/mcp` is now always mounted
  behind a gate, the setting is persisted in `state/vaults.json`, and the app
  has a switch under **Server ▸ AI access**. `--mcp` is an override at boot, not
  the source of truth.

**A release APK could never reach the server, and three deploys hid it.**
Reported from the phone as *"Couldn't reach the server … OS Error: Operation
not permitted, errno = 1"*. `EPERM` on `connect` is Android refusing the socket
outright: Flutter's template declares `INTERNET` in the **debug** and
**profile** manifests only, so hot reload can talk to the host machine, and
`android/app/src/main/AndroidManifest.xml` had never carried it. Every debug
APT this project has installed worked; the switch to
`flutter build apk --release` for the icon deploy is what exposed it.

The deeper mistake is the verification, not the manifest. Each of those deploys
was "verified" by comparing the local APK's sha256 against the installed
package — which proves the bytes arrived and nothing else. **The app was never
opened.** A hash match is not a working app, and it took the user reporting a
red error screen to find out. `test/android_manifest_test.dart` guards the
permission now, and the fix was confirmed by launching the app on the device
and seeing the vault list, not by another hash.

Same class as the macOS entitlements finding two milestones earlier: a platform
permission that fails silently, at the socket, long after everything that could
have caught it has passed.

The switch also repeated a mistake this project has already made once: a
`SwitchListTile` inside a colour-carrying `Container` trips Flutter's
"ink splashes may be invisible" assertion, exactly as the M12 sidebar did.
Decision 34's `Material` fix was in `PLAN.md` and still got re-made — worth
knowing that a recorded lesson is not the same as a remembered one.


`docs/storm-mcp.md`, Phase 1. Nine read tools at `/mcp`, behind `--mcp` and the
same bearer token. Write tools are designed and deliberately not built yet.

**The brief's Principle needed a refactor to be true, not just discipline.**
"MCP is never a second way to touch markdown" is unachievable while every
operation lives inside an axum handler: a handler takes extractors and returns
HTTP types, so a second caller physically cannot reach it and would have to
re-derive vault resolution, the 404-vs-409 rule and FTS sanitising. `ops.rs` is
that refactor — one plain async fn per operation, called by the handler and by
the tool. The 81-check `e2e.py` suite, unchanged, is what proves the extraction
did not alter REST.

**Two claims in the brief did not survive contact.** `get_note_history` was
described as backed by existing plumbing: `note_versions` is populated and
`version_content` reads one revision, but nothing *listed* them and neither was
on a route, so this milestone added `Db::list_versions` and two REST routes —
the Flutter client could not see history either. And `get_vault`'s description
was said to come from `_storm/vault.md`; the server had never parsed that note,
only excluded it from counts.

**Three defaults that would have shipped as bugs**, each found by reading the
SDK and the spec rather than by testing:

1. *rmcp restricts `Host` to loopback by default*, as DNS-rebinding protection.
   Storm is reached at a LAN address, so every request from the phone or
   another laptop would have been refused with nothing naming the `Host` header
   as the cause.
2. *`structuredContent` must be a JSON object.* rmcp types it as any `Value`
   and will happily send a bare array, which is what the first nine tools did.
   List results are now wrapped under the REST envelopes' own keys.
3. *A missing vault was a protocol error.* The spec reserves those for
   malformed requests and says clients need not show them to the model, while
   tool execution errors are the ones carrying "actionable feedback that
   language models can use to self-correct". An agent holding a stale note id
   is exactly that case, so 4xx now returns `isError: true` with the message
   REST would give; only a server fault is a protocol error.

Each of the three has a test, and each test was verified by breaking the code:
mounting `/mcp` below the auth layer (unauthenticated calls returned 200),
reverting the error mapping, and unwrapping the lists. The last one initially
*crashed* the suite rather than reporting — a list where a dict was expected —
which is a worse signal than a failure, so the payload accessor now normalises
and the run reports all eight failures.

---

### M14 — The design system, applied ✅

`docs/design_handoff_storm_design_system/` is a high-fidelity system plus a
clickable prototype of every phone and wide screen. Three commits landed the
foundation — the OKLCH token layer, the derived `ThemeData`, the shared widgets
and the four state widgets — and then stopped at the dashboard. Directory,
Note, Search, Tags, Properties, the toolbar, attachments, mentions, Connect and
Server settings still carried the pre-design structure and about forty
hardcoded sizes, radii and paddings the token layer could not reach. The app
read as two products: the dashboard was the design, and everything a tap away
from it was not.

**What was structural rather than cosmetic.** Three things, and they are
decisions 40–42: there is no app bar; Search and Tags are overlays; the
properties chip hugs its key rather than reserving a fixed 132px column.

**What the sweep found that the diff would not have.** Several of these were
wrong in ways that never show up in the default theme or in a test:

- The note body rendered at `settings.fontSize + 1`, so every note was a point
  larger than the size the slider claimed.
- `OfflineNotice(queued: 1)` was a literal, so the notice said "1 edit queued"
  whatever the real depth of the outbox.
- Tags rendered in accent purple, which is the colour of *interactive*. The
  token doc is explicit that amber means tags and highlight and nothing else.
- The degraded-editor notice sat on amber-soft, borrowing the tag colour for a
  message about performance.
- The nav badge was 9px, below the 11px floor the handoff is explicit about.
- `note_properties.dart` used `fontFamily: 'monospace'` — a platform alias that
  resolves to whatever the OS ships, not the bundled IBM Plex Mono the rest of
  the app is set in.
- Four widgets drew "nothing here", three drew "loading", and raw `'$e'`
  reached the UI in nine places while `describeFailure()` already existed.

**The toolbar now says what is already on.** `inlineActive` and
`blockPrefixHere` are read-only queries on `StormMarkdownController`, and the
bar rebuilds on selection changes rather than on taps — which is the part that
makes the state honest, because moving the caret changes what is bold without
any button being pressed. A collapsed caret needs the marker count on the line:
`_wrappedAt` only sees the two characters either side of it, which is true only
of a `****` you have just typed.

**The phone directory chrome now rides on the shared shell, not per-screen
offsets.** The mockup gap was in the wrong layer: the crowding came from
`StormChrome` pinning the header directly under the corner bubbles, the browse
screen using narrower gutters than the rest of the shell, and the nav pill's
sixth slot being absent outside a note. The fix stayed in the shared chrome and
row widgets — wider shell insets, a real bubble-to-breadcrumb gap, roomier list
rows, note timestamps where they exist, and the always-present mentions slot —
so the phone screen moves toward the handoff without creating one-off layout
rules the next pass would have to undo.

**"Updated from another device" fired on your own typing.** The server owns
`modified:` and rewrites it on every save, so the copy that lands in the cache
differs from the open buffer by exactly that one line. `onRemoteChange`
compared raw text, called the difference a remote edit, and put a banner over
the note every time a save landed. The server already blanks the same field
before it merges — `VOLATILE` in `index.rs`, guarded by
`the_modified_timestamp_never_causes_a_conflict` — and the client now does the
same before it compares. It still adopts the incoming version silently, because
saving against a stale base would turn the *next* write into a conflict the
user never caused.

**`/gallery`** renders every shared widget in all three presets side by side.
The header of `widgets.dart` had claimed it existed since the widgets landed.

The handoff's own vocabulary — atoms, molecules, organisms — is a documentation
device and does not appear in the code. Shared widgets are `lib/ui/widgets.dart`
and overlay containers `lib/ui/surfaces.dart`.

### The VM did not run the layout deploy/README.md describes — resolved by M15

> **Resolved.** The M15 apt install moved the VM onto the packaged layout, and
> that was re-verified on 2026-08-13: unit `storm-server` **active**, process
> `/usr/bin/storm-server serve`, package **0.2.5-1**, state `/srv/storm/state`,
> web `/usr/share/storm/web`, storage root `/mnt/media/Docs/storm` (NFS), all
> three vaults present and none `missing`, `/etc/storm/storm.env` at `0600`.
> `/home/dewansh/storm` and `~/storm-m15-cutover` are gone. `systemctl
> is-active` is now the right question. **`make deploy` / `deploy-check` were
> never corrected and are still written against a `storm` service account —
> apt is the update channel, not them.**
>
> One thing the re-check did turn up: **the shared token is still `testtoken`**
> — it returns 200 from `/v1/vaults` — even though `postinst` and
> `storm-server up` generate a real one. LAN-only, so a known risk rather than
> an exposure; decision 52 is the design that removes the class.

The original finding, kept because the lesson is not layout-specific:

Found on 2026-08-09 while adding `deploy-web`, and worth recording because
every deploy target in the Makefile is written against the wrong paths.

`deploy/README.md` and `make deploy` assume `/srv/storm/{vaults,state,web}`, a
`storm` service account and a `storm-server.service` unit. None of that exists
on the VM. What is actually there:

```
/home/dewansh/storm/{run.sh,storm-server,state,vaults,web}
```

started by `run.sh` — no systemd, no `storm` user, and therefore no sudo
needed to replace the web bundle. `systemctl is-active storm-server` reports
`inactive`, which reads as "the server is down" and is simply the wrong
question. `pgrep -af storm-server` is the one that answers.

`deploy-web` takes `WEB_DIR` and defaults it to the real path. `deploy` and
`deploy-check` are still written against `/srv/storm` and systemd, and would
fail the same way — they are left alone rather than half-corrected, because
which layout is *intended* is a decision, not a bug fix.

---

### M15 — Releases, versioning and an apt repo ✅

Designed 2026-08-11; built the same day. A tag push produces one GitHub Release
carrying every platform's artifact, and the `.deb` is also served as an apt
repository on Pages. Operator setup is Tailscale-shaped: `storm-server up`
writes config and enables systemd; `serve` is what the unit runs.

**Shipped.**

- CLI subcommands: `serve`, `up`, `down`, `status`, `dry-run`, `backup-db`
  (legacy flat flags still work for one release via argv rewrite).
- `[package.metadata.deb]` + `apps/server/debian/` maintainer scripts;
  `LICENSE`; unit at `/usr/bin/storm-server serve`; web at
  `/usr/share/storm/web` (package-owned).
- `up --data-root` defaults to `/srv/storm` and writes a systemd drop-in when
  the root is elsewhere (decision 48) — the VM can keep
  `/home/dewansh/storm` without a second empty server.
- `.github/workflows/release.yml` (stamp versions, musl+deb, APK, macOS, web)
  and `apt-repo.yml` (reprepro → Pages). `ci.yml` also runs on `v*.*.*` tags.
- Android release signing reads `STORM_UPLOAD_*` env when set; otherwise debug.

**Still open / optional:**

- Android keystore secrets (`STORM_UPLOAD_*`) — without them the APK is
  debug-signed.
- Apt GPG + Pages — **done 2026-08-11**.
- Clean VM install via apt — **done 2026-08-11**: `storm-server` 0.2.2-1,
  state under `/srv/storm`, vaults on NAS, hand-rolled `~/storm` and
  `~/storm-m15-cutover` removed.

#### Versioning — the tag is the only source

`Cargo.toml` / `pubspec.yaml` stay at static placeholders on `main`. The
release job stamps them from `GITHUB_REF_NAME` and passes Flutter
`--build-name` / `--build-number` before anything builds. Renaming output
files alone is not versioning — apt reads the `.deb` control version, Android
compares `versionCode`. First public tag: **`v0.2.0`**.

Client settings shows that stamp (`package_info_plus` → `Version x.y.z+N`) under
an About section so a stale web service-worker cache is obvious without a
hard-refresh guessing game. Local `flutter run` still shows the pubspec
placeholder. The same page groups Appearance / Notes / Connection / About —
matching Server settings' section-label rhythm rather than a flat switch pile.

#### Review traps — status after the build

| Trap | Status |
|---|---|
| No `LICENSE` / `description` | Fixed |
| APK hardcoded debug signing | Fixed (env-gated release config) |
| `/usr/bin` vs `/usr/local/bin` | Fixed — unit uses `/usr/bin` |
| `postinst` missing useradd/dirs | Fixed; start is owned by `up`, not postinst |
| Missing `contents: read/write` | Fixed in both workflows |
| Artifact path LCA surprises | Flat `dist/` per job |
| Pages apt root | Documented; publish tree is the site root |
| Bootstrap `install.sh` | `deploy/install.sh` → Pages `/install.sh` (Tailscale-style) |
| `.deb` ships no web | Fixed — release copies Flutter web into packaging |
| cargo-deb web glob empty | Fixed — use `packaging/web/**/*` (not `/` or `/**`) |
| Apt not rebuilt after tag | Fixed — `release.yml` `workflow_call`s apt-repo (token can't chain) |
| `change-me` token | `postinst` + `up` generate a real one |
| Tags bypass CI | Fixed — `ci.yml` on tags; `release.yml` runs check first |
| macOS ad-hoc / arm64-only | Still true — called out in release notes |

`apt install` still needs the same sudo password as before — packaging shortens
the manual step rather than discharging it. Preferred first install is the
Tailscale-style bootstrap:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh
```

(`deploy/install.sh`, published at the apt Pages root by `apt-repo.yml`.) The
script only registers the apt source and installs the package; `storm-server up`
stays a separate step because vault/state paths are an operator choice. Later
updates are apt (`apt install --only-upgrade storm-server` +
`systemctl restart`) — documented on `/install#update`, root README, and
`deploy/README.md`. There is no `storm-server upgrade` subcommand.

---

### M16 — Marketing / home site (Astro) · in progress

Started 2026-08-11. Thin public front door — product story + install path —
in `apps/www`. **Not** a documentation portal; depth stays on GitHub /
`PLAN.md` / `deploy/`.

**Done:**

- Astro static app (TypeScript, npm lockfile); BrandMark; IBM Plex Sans /
  Newsreader / IBM Plex Mono.
- **SlowFlow earth** tokens for www (warm paper, brown accent, tight radii) —
  derived from `StormPreset.slowflowEarth` / the design handoff. Storm dark
  remains the app’s default identity; SlowFlow is the marketing surface.
- Routes: `/` (hero “Your knowledge. On your infrastructure.”, foundation,
  MCP with 12 real tools + static demo, architecture, storage, clients,
  sync, install summary, repo docs links), `/clients` (Get Storm download
  manifest), `/install` (apt + `up` from `deploy/release-secrets.md`; no
  Docker), `/how-it-works` (architecture sketch + MCP).
- Apt install command stays on `dewanshdt.github.io/Storm/install.sh` (apt
  Pages root; marketing host is `storm.dewansh.space`).
- `make www` / `make www-dev`; CI job `www` in `ci.yml` (build check only).
- Page copy in vault [[Storm Website]] notes and `docs/www/`.

**The auth truth pass (2026-09-02, decision 65):** the site described the
pre-cutover world for thirteen days after `STORM_TOKEN` was deleted. Fixed on
`fix/www-auth-truth`:

- `/install` no longer promises a generated token, and gained a **First login**
  section — browser bootstrap, `storm-server pair --qr` on the host, and
  `user add` / `passwd` as the recovery path, with the run-as-`storm` warning
  that `deploy/README.md` carries.
- `/` step 03 is **Sign in** (pair the device, create the owner account), not
  "point the client at the token".
- `/how-it-works` says accounts are local and yours — owner account, per-device
  pairing, sessions, revocable MCP keys, all in `state/auth.db` — instead of
  "no accounts product". `auth.db` is in the architecture sketch, since it is
  the one part of `state/` that cannot be rebuilt by rescanning.
- MCP copy names the `stk_` key (minted in the app, `/mcp` only) rather than
  implying the shared bearer token.
- `release.ts` bumped v0.2.3 → **v0.2.7**; every download link was four releases
  stale.
- `docs/www/` mirrored in the same change.
- Site infrastructure the redesign never got: absolute `og:image` (a relative
  one is silently dropped by every scraper, so the cards were blank), `og:url`,
  `rel=canonical`, `@astrojs/sitemap`, `robots.txt`, and a `404` page.
- **The astro patch bump was reverted, and the reason is a repo hazard.**
  `npm install` on macOS prunes two hoisted entries (`@emnapi/core`,
  `@emnapi/runtime`) that a Linux tree needs, and CI's `npm ci` then refuses
  the lockfile as out of sync. It cannot be regenerated correctly on a Mac —
  `--os=linux`, `--cpu=wasm32` and `--package-lock-only` all still prune them.
  So the lockfile change is now the sitemap subtree and nothing else: **6
  packages added, none removed, no version changed**, with the two entries
  restored verbatim. `apps/www/README.md` carries the warning.
- **`make www-check`** — the content gate, in the `www` CI job.

**Still open:**

- Optional: real running-app screenshot for the Client section (prototype PNGs
  stay design reference only — not shipped as product photos).
- **Confirm the Cloudflare hostname / dashboard connection.** Still the one
  thing between this site and being live; `astro.config.mjs` has committed to
  `https://storm.dewansh.space`, and the sitemap and canonical URLs now derive
  from it.
- The relay stays off the site until it is released — it works on `staging`,
  is undeployed, and phase 4 is blocked on TOFU persistence.

**Hosting (decision 49):** Cloudflare static deploy at `https://storm.dewansh.space`
(apart from apt Pages). **Never** deploy to `https://dewanshdt.github.io/Storm/`
— that URL is the apt repository root owned by `apt-repo.yml`. A marketing
overwrite would break every `sources.list` line. Do **not** chain marketing
deploys into `release.yml`.

**Content sources of truth:**

- Marketing page copy → vault notes [[Storm Website]] (+ Home / Install /
  How it works) and the mirror under `docs/www/`
- Install commands → `deploy/release-secrets.md` (apt) + `deploy/README.md`
- Product claims → live app + server/client READMEs; do not invent features
- Architecture depth → link to repo / `PLAN.md`, do not duplicate

---

### M17 — Markdown Read Mode · in progress

Started 2026-08-11. Polished read-only document view for notes, without
replacing the source editor or introducing an AST (decision 50).

**Done (client):**

- `flutter_markdown_plus` + `markdown` + `url_launcher` dependencies.
- `StormMarkdownView` + `stormMarkdownStyleSheet` under `lib/ui/markdown/`.
- Read / Edit toggle (`NoteModeToggle`); notes open in Read Mode when
  `settings.readMode` is on (default). Off hides the switch and stays in
  Edit Mode only.
- Edit Mode unchanged (existing controller, toolbar, attachment strip).
- Client settings: **Read mode** switch beside Show note id.
- Wikilinks via custom `WikilinkSyntax` (`storm-wikilink:` scheme).
- Images resolve through `StormApi.attachmentUrl`; tables/code/HR/lists/
  task lists (read-only `StormCheckbox`) styled from tokens.
- Task-list boxes stay square: custom `checkboxBuilder` must apply the
  same right pad as `listBulletPadding`, or the package's tight bullet
  column stretches the 18px box into a pill over the label.
- Read Mode body size (and the task box) follow `settings.fontSize`.
- Tests in `test/markdown_read_mode_test.dart`; existing editor suites
  enter Edit Mode via `enterEditMode`.

**Still open:**

- Visual pass on a real vault note (phone + desk + web).
- Optional later: interactive task-list toggles (still Markdown splice,
  not a second mutation API).

---

### M18 — Desktop keyboard shortcuts ✅

Started 2026-08-12, shipped in **v0.2.4** (2026-08-12). Desktop-first chords
via Flutter Shortcuts/Actions/Intent (decision 51). Phone touch layout
unchanged.

**Done (client):**

- `lib/keyboard/` — intents, platform `stormActivator` (Meta vs Control),
  shortcut maps, `StormGlobalShortcuts` / `StormNoteShortcuts` /
  `StormEditorShortcuts`.
- Global: search, new note, new folder, sidebar toggle (`sidebarCollapsedProvider`
  + `VaultShell`).
- Note: save now, Read↔Edit, find-in-note bar, Esc dismiss/leave.
- Editor-focus: bold / italic via existing `toggleInline`.
- Tests in `test/keyboard_shortcuts_test.dart` (activator, sidebar, search,
  create, Read↔Edit, save, find, bold/italic, Esc-leaves-note, Esc ladder, and
  the two buttons that used to be dead — the desk-width sidebar "New note" and
  the phone note-screen nav pill).

**Finding that moved the wiring:** global shortcuts could not hang off
`StormScaffold` alone. At desk width the browse route renders the empty pane
beside the sidebar (`NoNoteSelected`) and no `StormScaffold` at all, so the
chords and their `NewNoteRequest` / `NewFolderRequest` callbacks now live in
`VaultShell`, which wraps every vault route — and which fixed the pre-existing
dead "New note" button in the desk-width sidebar toolbar. The note screen had
an *empty* `NewNoteRequest` of its own, which made the phone nav pill's create
button dead inside a note; both now resolve the shell's single callback.
`StormScaffold` and the note screen dropped their own copies.

**Polish during the review pass (decision 51):** `sidebarCollapsedProvider`
moved out of `lib/keyboard/` into `state/app_state.dart` beside the other
chrome toggles; the global Focus autofocuses only where a physical keyboard
exists; Esc is a ladder (find → Read → leave) and the header back and Esc
share one `_leaveNote`; the find bar's field autofocuses itself instead of a
post-frame `requestFocus`.

**Deferred:** shortcut overlay, command palette, ⌘⇧C code, sync-on-R,
close-note-on-W.

### M19 — Authentication (decision 52 phase 1) · in progress

The design is complete and lives in the personal vault: **Storm Auth Data
Model**, **Storm Auth Protocol**, ADRs **A1–A12** in *Storm Remote Decisions*,
and the checklist *TODO — Storm Authentication*. Read those before touching
this; the checklist is the work list, not the design.

**Slice 1 — server identity and its storage ✅** (2026-08-13, not deployed)

- `apps/server/src/auth/` — `db.rs` creates `state/auth.db` with
  `PRAGMA user_version = 1` and **every** table the data model names (only
  `server` and `server_credentials` are written yet; the rest cost one
  `CREATE TABLE` now and a migration later). `identity.rs` mints a random
  `srv_`-prefixed id in Crockford base32 and an Ed25519 keypair, private bytes
  at `state/identity/<key_id>.key` mode `0600` in a `0700` directory, created
  *with* that mode rather than chmod-ed afterwards.
- `GET /v1/server` and `POST /v1/server/challenge`, both unauthenticated, both
  registered **below** the `require_token` layer — axum applies a layer only to
  routes above it, and `the_server_endpoints_answer_without_a_token` fails if
  they move.
- `backup_all()` snapshots `auth.db` with `VACUUM INTO` and copies the key
  files, *before* the "no vaults registered" early return — a server with no
  vaults still has an identity worth keeping. `deploy/storm-backup.sh` and
  `deploy/README.md` updated in the same change.
- `Db::snapshot_to` and `AuthDb::snapshot_to` share one
  `db::snapshot_connection`, so the "never overwrite a non-file" guard cannot
  drift between them.

**Evidence.** 180 Rust tests, clippy clean. `e2e.py` still 81/81 **unchanged**,
which is the proof the slice is additive; 21 new checks in `tests/auth_e2e.py`,
including a vendored Ed25519 verifier so a *running* server's signature is
checked against the key that same server published. Every new test was verified
by deliberately breaking what it names — 13 mutations (routes moved above the
layer, raw-nonce signing, identity regenerated per boot, backup skipping
`auth.db` or the keys, `backup_auth` moved after the early return, `0644` key
files, the credential-count check disabled, a table dropped, `server_id` derived
from the key, nonce validation removed, a missing key silently regenerated,
`user_version` never stamped) and all 13 failed the right test.

A real backup → wipe → restore cycle with the binary and a live server returned
the identical `server_id`, `key_id`, public key and challenge signature, with
the restored key still `0600`.

**Decided while building** (see decision 52b): the challenge signs
`storm-challenge:v1:<server_id>:<nonce>`; A4 is amended so backups carry
`state/identity/` as well as `auth.db`; a second active credential is a refusal
at boot rather than "use the newest" (data model invariant 2); a `server` row
whose key file is missing is a hard failure, never a regenerated keypair.

The server name defaults to `/etc/hostname` (falling back to `Storm`) and is
set once, at creation. There is no rename path yet — that belongs with the
settings surface in a later slice.

**Q18 — Argon2id parameters, measured ✅** (2026-08-16, on the VM)

`tools/argon2-bench` is the instrument, because A1's revisit trigger is a
*re-measure* and a number pasted into a chat is not one. Measured on the
homelab VM (3 vCPU, 3815 MB) while prod was serving, `nice`d, prod health
checked before and after:

```
m = 196608 KiB (192 MiB), t = 1, p = 1   ->  173.6 ms per verify
```

Stable across four runs (173.6 / 173.0 / 173.6 / 174.8 ms). Raw sweep in
`tools/argon2-bench/q18-vm-results.txt`.

Three findings that constrain the user-model slice:

- **`p` stays 1.** p=1 and p=2 measure identically — the `argon2` crate does
  not thread without its `parallel` feature, so more lanes would cost memory
  and buy no speed.
- **Memory beats passes.** 192 MiB / t=1 is preferred over the leanest
  in-window option (96 MiB / t=2, 150.5 ms): at a fixed time budget the
  attacker's cost is bounded by memory, which is also RFC 9106's ordering.
- **Login must bound its concurrency.** Verify runs on `spawn_blocking`, whose
  pool defaults to 512 threads; 512 × 192 MiB is an OOM that takes the vault
  server down. A semaphore of 2 holds the peak at ~384 MiB (measured 207 ms
  wall for two at once). **Do not shrink the KDF to work around a missing
  bound — add the bound.** The laptop, for contrast, wanted 192 MiB *at t=2*
  for the same latency, which is exactly the gap A1 was written about.

**Slice 1 verified on the VM** (2026-08-16). A staging server on :8585 with its
own vault root and state under `/tmp` — never `storm-server up`, never
systemctl, prod on :8484 untouched throughout and health-checked either side.
It minted `srv_Q2T0B4C90VQDQW9HGW7JNHJYB4`, named itself `ubuntu` from
`/etc/hostname`, wrote its key at `-rw-------`, answered `/v1/server` and the
challenge with no credential, still refused `/v1/vaults` without the token, and
came back from a backup → wipe → restore with an identical identity. Torn down;
prod's `state/` has no `auth.db`, which is the proof staging stayed in its own
directory.

**Slice 2 — user accounts and Argon2id ✅** (2026-08-16, not deployed)

The data model's `users` table, finally written to. **No new routes:** creating
a user over the network needs device auth (A8), which arrives with pairing, so
this slice is reachable only from the operator CLI and `e2e.py` stays untouched.

- `auth/password.rs` — Argon2id at Q18's measured parameters, behind a semaphore
  of **2 permits**. The bound is the point: `Hasher::hash` and `Hasher::verify`
  are the only way in, and both hold a permit across the whole `spawn_blocking`
  job. Passwords are 12 characters minimum and **refused, never truncated**,
  above 1024 bytes. `needs_rehash` compares a stored PHC string's parameters
  against this build's, which is what will let login upgrade old hashes.
- `auth/users.rs` — accounts, roles, and two enforced invariants: **the first
  account is an owner** (a server whose only user is a member has nobody who can
  promote anyone), and **the last *active* owner cannot be deleted, disabled or
  demoted**. Disabled owners deliberately do not count — an account that cannot
  log in cannot administer, so leaving only disabled owners is the same lockout
  as leaving none.
- Usernames are ASCII, 3–32 characters, unique by casefold. That is a decision:
  uniqueness is decided on the fold, and Unicode brings locale-dependent case
  rules and homoglyphs, so `dewansh` and a Cyrillic-`е` lookalike would be two
  accounts that render identically in the user list. `display_name` is
  unrestricted, so nobody is stuck with an ASCII name on screen.
- `storm-server user add|list|disable|enable|role|delete` and
  `storm-server passwd` (A11). **No `--password` flag anywhere** — an argument
  is in the shell history and in `ps` for every other user on the box while the
  process runs. The password is prompted for without echo, or piped with
  `--password-stdin`.
- Both writing commands **read the stored hash back and verify it** before
  reporting success, so a row that was written but cannot be logged into is
  caught immediately rather than at a login prompt weeks later.
- Every administrative act writes a `security_events` row — the table's first
  writer. A test asserts no password or hash ever reaches it.

**Evidence.** 213 Rust tests (204 unit + 9 driving the real binary as a
process), clippy clean, 581 Dart. `e2e.py` still **81/81 unchanged**. **19
mutations**, each a plausible mistake rather than a random edit — the semaphore
removed, `hash` bypassing it, passwords truncated at 72 bytes, the minimum and
maximum length checks disabled, the KDF weakened below what was measured,
`needs_rehash` always false, the last-owner guard removed, disabled owners
counted as owners, the first account allowed to be a member, the username fold
dropped, non-ASCII usernames allowed, the audit trail silenced, foreign keys
off, a reset leaving the lockout in place, the read-back verify removed, a piped
password keeping its newline, the username checked only after the password, and
the backup skipping `auth.db` — all 19 failed the right test.

A backup → wipe → restore now also proves the *accounts* came back: the restored
user's stored hash still verifies against their password, which is the thing
"the users survived" actually means.

**Open, deliberately:** the plaintext password is an ordinary `String` and is
not zeroed after use. Doing that properly needs a zeroizing type threaded
through `clap` and `rpassword`, and it buys little against an attacker who can
already read this process's memory. Worth revisiting if a `SecretString` lands
for other reasons.

**Next slices, in order:** sessions (login, refresh, revocation — where the
semaphore and `needs_rehash` finally sit on a request path) → the three-tier
middleware → pairing → client. The middleware slice is the one that stops being
additive. *(All three shipped as of 2026-08-17; now building the Flutter client.)*
*(Superseded: slice 6 shipped too, and all six merged to main the same day.)*

**Slice 6 — the Flutter client pairs ✅** (2026-08-17, not deployed)

`PairingScreen` walks the whole first run in one screen: paste a `storm://pair`
URI, verify the server's identity, claim the nonce for device credentials,
create the owner account, log in. `Settings` gained the device credentials
(`deviceId`, `deviceSecret`), the server's identity (`serverId`, `serverKeyId`,
`serverPublicKey`) and the session (`accessToken`, `refreshToken`,
`accessTokenExpiresAt`, `userId`), all persisted; `apiProvider` now sends
`bearerToken`, which prefers the session token.

Two things worth keeping from building it:

- **The client verifies the server before it trusts the pairing URI.** The URI
  carries the server's public key, so the client rebuilds
  `storm-challenge:v1:<server_id>:<nonce>` and checks the signature against it
  before sending the nonce anywhere. `ed25519_verify.dart` is that check, and
  the string it rebuilds is a wire-format commitment shared with the server.
- **`isConfigured` had to stay true for a legacy install, and briefly did not.**
  The first redirect sent anything unpaired to `/pairing`, which is every
  install that predates auth — a URL and a shared token, no device — and would
  have locked each of them out of a vault it could already read. `isConfigured`
  is now "a session *or* a legacy token", the redirect keys off it alone, and
  `router_test.dart` asserts a legacy install never reaches the pairing screen.
  This is decision 52b (additive until the middleware slice) applying to the
  client, and it is the kind of break a green server suite cannot see.

The session lifecycle is `refreshSession()`, `logout()` (keeps the device
paired) and `unpair()` (forgets the server). Only `refreshSession` has a
plausible caller today — see the login-screen gap noted above — so
`auth_settings_test.dart` covers all three directly rather than letting them
ship untested.

**All six slices merged to main on 2026-08-17** (PRs #10, #11, #13–#16), in
stack order. Nothing is deployed.

**Two formatter failures that `make check` cannot see.** Both had been red in CI
for days while `make check` reported clean, and getting the stack green meant
fixing them first:

- **`cargo fmt --check`** had been failing PR #11 since it opened — ordinary
  line-wrap drift in the slice 2 files. Fixed at the base of the stack and
  propagated by rebase.
- **`dart format --set-exit-if-changed`** was failing the client job on *every*
  PR, including #10, which contains no client changes at all. CI's
  `channel: stable` had resolved to Flutter **3.47.0** while development runs
  **3.44.8**, and the newer formatter rewrites two files nothing had touched.
  `main` was last green 2026-08-11 — before the upstream bump. Fixed by pinning
  `flutter-version: 3.44.8` in `ci.yml` *and* `release.yml`.

The finding worth keeping: **`make check` runs clippy and the tests but never
checks formatting**, so it is not the gate it looks like. `make fmt` rewrites
files, which means the only thing that ever *checks* formatting is CI. And
**a floating toolchain is a build input nobody wrote down** — `channel: stable`
meant the SDK could change under a branch that had not been touched, turning
the whole repo red with no commit to blame. Pinning `release.yml` too is
deliberate: a release should be built by the SDK that CI validated, not by
whatever `stable` pointed at that morning.

**Slices 9 and 10 ✅** (2026-08-16, not deployed, PRs #23 and #24). Slice 9 moved
the device secret, both session tokens and the legacy token into the platform
keychain — migration is write-then-erase rather than move, a leftover prefs copy
is erased even when the keychain already holds the value, and a platform with no
reachable keychain degrades to prefs rather than locking anyone out. Slice 10
built the vault access boundary described under A9 above. Both are recorded
per-item in *TODO — Storm Authentication*; the finding worth keeping from slice
10 is in the mutation note below.

**Slice 11 — the identity behind an MCP request** (code complete 2026-08-17;
**PR #25 open, not merged**, not deployed)

Slice 10 left one caller unfinished, and said so: an MCP tool resolved as
`Actor::Mcp` — "some MCP session", with no user behind it — while the identical
call over REST resolved as a real user. Harmless while the policy allows
everyone, and an authorization bypass the moment it does not, since MCP would be
the single caller whose grants could not be checked. That is why it had to land
**before** roles, not with them.

- **The mechanism follows rmcp's actual lifecycle, which was read rather than
  assumed** (`streamable_http_server/tower.rs`, 3.1.2): `handle_post` calls
  `get_service()` on the *request* task, then `tokio::spawn`s the task the
  handler runs on. **A `task_local` does not cross a spawn**, so a tool reading
  one would find nothing — the shape the earlier design review proposed could
  not have worked. Instead a layer inside `require_auth` scopes the `Actor` on
  the request task, the factory reads it *there*, and the value is moved into
  the `Storm` handler. The identity crosses the spawn **by ownership, not as
  ambient state** — which is also the isolation argument: one factory call per
  HTTP request, one owned `Actor` per handler, nothing shared.
- **`Actor::Mcp` is removed rather than populated.** Keeping it would have left
  a second identity concept beside the real one — the second authorization
  system this slice exists to prevent.
- **Fail closed.** A handler built with no identity refuses the vault tools
  rather than defaulting to a permitted one. The factory still tolerates `None`,
  because rmcp also builds a throwaway service for tool-schema validation that
  has no request scope and touches no vault.
- **No policy change.** `AllowAuthenticated` still allows everything; this slice
  moves *who is asking*, not *what they may do*.

**Evidence.** 272 Rust tests (263 unit + 9 driving the real binary as a
process), clippy clean. `make test-live` — the 81 server, 56 MCP and 21 auth
e2e checks against a real server — is green on this branch in CI, which is what
says the MCP transport still behaves as it did while its identity changed
underneath.

**Two mutations survived the first campaign, and both were test defects rather
than code defects:**

- **The concurrency test was not concurrent.** `#[tokio::test]` defaults to a
  **current-thread runtime**, so "parallel" tasks interleave only at await
  points on one thread — and a mutation that stashed the identity in a shared
  `Mutex` instead of a per-task scope passed underneath it. Rewritten as
  `multi_thread` with 4 workers and 64 interleaved requests.
- **Nothing reached the fail-closed branch.** Every HTTP test goes through
  `scope_actor`, so a mutation returning a permitted actor when the identity is
  missing passed the entire suite. The decision is now `resolve_actor()`, free
  of `AppState` so it can be called directly, with both branches covered.

Even then the HTTP test could only show that two users keep their own identity,
not that they *must*: there is no guaranteed yield between the scope being
entered and the factory reading it, so the shared-`Mutex` mutation survived on
timing alone — twice, including under 4 workers and 64 requests. The test that
finally pins it holds **32 tasks inside their own scope simultaneously** and
then reads back, so a shared slot is certain to have been overwritten. It tests
the isolation primitive rather than the transport; with the mutation proving the
factory reads the scope, and capture-by-value carrying it across rmcp's spawn,
the chain is covered end to end.

**The finding worth keeping: a concurrency test written under `#[tokio::test]`
is not a concurrency test.** One thread, interleaving only at await points — it
will happily pass over an implementation that shares one slot between every
caller. And **a test whose subject is reachable only through the happy path
never runs the other branch**: extracting the decision away from `AppState` is
what made the refusal testable at all. Both are the same lesson as slice 10's
vacuous collection-filter test — *a test that passes when you remove the thing
it names is not a test of that thing* — arriving by two new routes.

**Slice 12 — the device tier, actually exercised ✅** (2026-08-18, not deployed)

Written as "add the missing e2e coverage for login and account creation". The
coverage was the deliverable; what it found is the entry.

**No test in this repo had ever sent a `StormDevice` credential.** `e2e.py`
covers session-tier vault operations, `auth_e2e.py` stopped at slice 1's two
unauthenticated routes, and the client's `test_live/` suite authenticates with
the legacy shared token. Every route that *is* authentication — login, refresh,
`users`, `users/first` — sits behind the one credential nothing presented. All
three defects below were found by writing the first test that did.

- **The device tier was deadlocked, and one request took the server with it.**
  `require_auth`'s device branch held the `auth_db` guard across
  `next.run(request)`. `tokio::sync::Mutex` is not reentrant and every
  device-tier handler takes that lock, so the request hung forever — and since
  the wedged task never released the mutex, every later request needing
  `auth_db` queued behind it. **One `POST /v1/auth/login` disabled all
  authentication on the server until restart.** The session branch immediately
  below has always called `drop(auth_db)` first; this branch never did. The
  asymmetry is the whole story: the tested path was written correctly.
- **`POST /v1/users/first` had no bootstrap window.** Its only refusal was
  `create_user`'s duplicate-username check, so a paired device could pick an
  unused name and get another account — and the handler hardcodes
  `Role::Owner`. Any authenticated user can issue a pairing, pair a device and
  mint an owner: privilege escalation on any server with more than one user.
  The doc comment already said "fails if any user already exists"; only the
  code disagreed. A8 called this a one-shot window. It is now checked before
  the hash, so a refusal cannot burn a KDF permit.
- **The Argon2id semaphore bounded nothing.** Every handler called
  `Hasher::new()`, minting a fresh pair of permits per request, against
  `Hasher`'s own documentation: *the bound is only a bound if every caller goes
  through the same one.* Latent rather than live — these handlers hold `auth_db`
  across the KDF, so hashes were serialized at concurrency 1 by a global lock
  instead of by the mechanism built for it. The exposure is the next person to
  narrow that lock scope, which is an obvious thing to want (one mutex over a
  ~170 ms KDF serializes all authentication) and would remove the only real
  bound while leaving the decorative one in place. One `Hasher` now lives in
  `AppState`.

**Evidence.** 266 Rust unit tests + 9 process tests, clippy clean. `e2e.py`
**still 81/81 unchanged**, mcp 56/56, and `auth_e2e.py` 21 → **66**. Three
mutations, one per fix — the guard held across `next.run` again, the bootstrap
count check deleted, the handlers minting their own `Hasher` — and all three
failed the right test, the first by hanging, which is the bug reproducing.

Two of the new e2e checks exist because the suite failed honestly while being
written. `/v1/auth/refresh` is device tier, and sending it without the device
header made the replay check pass **for the wrong reason** — a refresh that
never happened cannot be replayed. Fixing that surfaced the next one: a replayed
refresh token revokes the *whole session*, so the logout checks had been running
against a token the replay had already killed. Both are now explicit checks
rather than accidents.

**The finding worth keeping: a tier is not covered until a test presents its
credential.** Three defects, one of them an outage on the first login, lived in
four routes for five slices while the suite around them stayed green — and the
green was honest, because every test used a credential that routed around them.
Coverage counted in checks says nothing about which *doors* were opened. It is
the M7/M8 lesson again (the layer the user touches is the untested one), and
slice 6's client redirect bug was the same shape: *a fully green server suite
could not have seen it.*

---

**Slices 13–16 — the client half, and what a real device found** (2026-08-19,
staging only)

Four slices in one branch, because each was found by the previous one. The
through-line: **every defect below was invisible to a green suite and obvious
within minutes of a real client.**

**Slice 13 — the client's device tier.** `test_live/` authenticated with the
legacy shared token, so no client test had ever presented a device credential.
Writing one found that `AuthApi.refresh` sent **no `Authorization` header** at
all (device tier, so every real server refused it — invisibly, because the only
caller swallows the failure into `false`), that `createFirstUser` did the same,
and that the login response did not parse: the client read
`access_expires_in` as seconds where the server sends `expires` as an absolute
RFC3339 instant, so `SessionTokens.fromJson` threw on every real login. The
wire contract had only ever existed as `{access, refresh, expires…}` in a
diagram — an ellipsis where a field list belongs — so both sides had guessed.
It is now written out exactly in *Storm Auth Protocol*.

**Slice 14 — the pairing QR.** "QR" had meant *a URI printed as text*
everywhere it appeared: nothing drew one, nothing scanned one, and the
first-run screen told people to scan. `storm-server pair --qr` now draws a
scannable block, an authenticated client shows one for a new device, and the
phone scans either. The scanner hands its string to the same `PairingUri.parse`
the paste field uses, because a scanner with its own parsing is a second
implementation of the security-critical step.

**Slice 15 — web bootstrap** (design: *Storm Web Bootstrap*; A7/A8 amended).
Storm serves its own web client and then asked the person who opened it to scan
a QR to reach a server the browser already knew. The served document now
carries a short-lived, single-use, **peer-bound** nonce, spent through the
ordinary `POST /v1/pair`. The browser becomes an ordinary device — listed,
revocable, session-bound. The accepted cost is recorded rather than stumbled
into: anything that can fetch the page can obtain a nonce, so username
enumeration via `/v1/users` is open to anyone who can reach the server.

**Slice 16 — registration is a switch** (A13). Persisted beside
`legacy_token_enabled`, **default off**, owner-only, member-only, taking effect
on the next request. `/v1/users/first` is untouched and still one-shot.

**Five defects a real device found, that no suite did:**

- **A bare `host:port` is not a URL.** The QR's `addr` was passed straight in as
  `baseUrl`; `Uri.parse` reads `192.168.91.51` as a scheme and a scheme may not
  begin with a digit. One of those callers persisted it into `Settings`, so this
  broke the whole app after pairing. The live test missed it because the harness
  hands it a full `http://host:port` and it never read the field.
- **The QR advertised the bind address.** `--host 0.0.0.0` is every real
  deployment, and no phone can dial a wildcard. The CLI's `--addr` default was
  `127.0.0.1`, which on a phone is the phone.
- **A mangled paste was reported as a network failure.** A keyboard broke the
  URI with a space; the parser half-succeeded into an empty address and the
  screen said *"Couldn't reach the server"* — the M9/M10 bug again, in the one
  place the rule had not been applied.
- **Pairing assumed first-run.** Every successful pair went to "create the owner
  account", so adding a second device asked for a second account on a server you
  already had one on. Invisible until "Add a device" made joining an existing
  server normal.
- **A device the server disowns bricked the client.** After a wipe or a
  *revocation*, the browser held a credential every device-tier call refused, and
  `bootstrapWebDevice` short-circuits on `isPaired` — so it could never mint
  another. Revoking a device is a normal thing to do, and it would have shipped
  as "revoke a browser and it bricks until someone clears storage".

**Two boundaries that were being enforced by accident.** `require_auth`'s device
branch ran whatever tier the route asked for, so a `StormDevice` header
satisfied *session* routes and was stopped only by handlers failing to extract a
`SessionAuth` nobody had inserted — a `500` where `401` was the truth. And
`accessTokenExpiresAt` was written by three screens and read by none:
`refreshSession()` existed, was tested, and had no caller, so a lapsed session
stayed "configured" forever while every request answered 401. Both now checked
where they are claimed.

**Evidence.** 286 Rust unit + 9 process tests, clippy clean; 649 client tests,
analyze clean. `make test-live`: **e2e 81/81 unchanged**, mcp 56/56, auth 66/66,
client 20/20. Against the VM over the network: auth_e2e 66/66, the Flutter
device flow, web bootstrap 12/12 and registration 13/13. A phone paired by
scanning a real QR, created an account, signed in, signed out and signed back
in. **The hand-test checklist is partly open** — *Auth Device Flow* in the
personal vault — and nothing here is deployed to prod.

**The finding worth keeping: a harness that supplies what a person would carry
tests everything except the carrying.** Every one of the five defects above sat
in a field the suites never read — the address in the QR, the message on the
screen, the credential in the browser — because the tests were handed a good
value instead of the one a human produces.

---

---

**A14 — MCP keys: the last of the authentication foundation ✅**
(2026-08-19, staging only)

A credential a signed-in user mints for a *machine* — an MCP client, a script,
an agent. It travels as an ordinary `Authorization: Bearer stk_…`, reaches
`/mcp` **and nothing else**, and resolves to
`Actor::Key { key_id, user_id, role }`: the owner's identity, with the key
alongside for audit. Design in *Storm MCP Keys*, decision **A14** in *Storm
Remote Decisions*, wire flow in *Storm Auth Protocol* (Flow 9).

**Why it had to exist before the cutover, and this is the finding.** MCP
authenticates with the legacy shared token today. The moment
`legacy_token_enabled` goes false, **every MCP client stops at once** — and MCP
is the one client that is not the Flutter app, so nothing prompts anyone to
sign in. The cutover checklist had four steps and none of them was "migrate the
clients that will not notice". It has five now.

**The near-miss is the same shape.** Moving `/mcp` from the session tier to its
own `RequiredTier::Mcp` silently retires the shared-token MCP path unless the
legacy condition widens to `Session | Mcp` in the same change. That is one line,
it breaks every existing MCP client, and nothing else would have failed.
`the_legacy_token_still_reaches_mcp` holds it, and reverting that condition
fails it.

**Three design choices, each with a live alternative:**

- **Bearer, not a `StormKey` scheme.** The symmetry with `StormDevice` reads
  better and loses to a fact: most MCP clients can send nothing else. *A
  credential the clients it exists for cannot present is not a credential.*
- **`/mcp` only.** A session token lives in a keychain; a key lives in a
  plaintext config file, and the two do not deserve the same reach.
- **`Actor::Key`, not a reused `Actor::Session` and not a principal of its
  own.** `authz.rs` already records why `Actor::Mcp` was deleted in slice 11;
  a key with an identity of its own would restore that mistake under a new
  name. `Actor` gained `user_id()` / `role()` accessors, and the authorization
  release must read those rather than match on the variant — a policy written
  that way covers keys the day it is written.

**Deliberately absent: any way to narrow a key.** No per-vault scoping, no
read-only key. An owner's key is an owner-powered bearer credential in a config
file, documented rather than mitigated — scoping is the same question the
authorization release must answer for users, and answering it twice is how two
definitions diverge.

**The migration is additive**, and that is a direct consequence of the v3→v4
defect found days earlier: `api_keys` comes from the `CREATE TABLE IF NOT
EXISTS` batch, so a v4 database gains it by being opened. No rebuild, nothing
to strand. `auth.db` is the one file in `state/` a rescan cannot reconstruct.

**Evidence.** 311 Rust unit + 9 process tests, clippy clean; 655 client tests,
analyze clean. `make test-live`: **e2e 81/81 unmodified**, mcp 56/56, auth
66/66, client 20/20. **Against staging with a real MCP client** — the official
MCP SDK over StreamableHTTP, full `initialize` → `tools/list` → `tools/call` —
ten checks: mint, connect, a real tool call, identity resolving to the owner,
the legacy MCP path still working, revoke, the revoked key refused, another
user's key unaffected, no cross-user identity, and REST/session untouched with
a key refused there at `401`.

**The bug that came back while writing it.** `create_key` required
`Extension<SessionAuth>`, which the legacy token never inserts, so it answered
`500` where `403` was the truth — the identical shape fixed for the device tier
two days earlier. Caught by the test that asserts the legacy token *cannot*
mint a key. **A refusal that arrives as an extractor panic is not a refusal**,
and this codebase has now produced that bug three times in three tiers.

---

**The A10 cutover — the shared token is gone, not switched off ✅**
(2026-08-20, decision **54**)

`STORM_TOKEN` is removed. The only ways into a Storm server are a paired
device, a session, and an MCP key — each minted per caller, each individually
revocable, each attributable to a person.

**This was brought forward, deliberately, against A10's own plan.** A10
specified a compatibility window: ship auth with the token still working, pair
everything calmly, flip the switch, verify, and remove it a release later. That
sequence exists so nobody is ever locked out of a server they cannot yet log
into. It was skipped on purpose, because the alternative was shipping a release
that contained a working backdoor — and the trade was made with the cost
stated: **prod has no `auth.db`, so this release locks out every existing
client on upgrade**, and recovery is SSH to read a pairing QR out of the
journal. On a homelab with shell access that is an outage, not a disaster. It
would be a different answer on a machine somebody else runs.

**What the removal took with it, and this is the interesting part.** Three
things became honest that had been shaped around the token's existence:

- **`Actor::user_id()` and `role()` no longer return `Option`.** There was
  exactly one caller with no user behind it. Removing that credential removed
  the only `None`, so every future policy reads a user and a role
  unconditionally rather than unwrapping at each site.
- **`create_key` takes a required `Extension<SessionAuth>` again.** It was an
  `Option` because the shared token satisfied the session tier while inserting
  no session — which is precisely what made it answer `500` where `403` was
  true, the third occurrence of that bug in three tiers.
- **`constant_time_eq` had no callers left**, and clippy said so. It existed
  only to compare that one secret. Code that becomes unreachable when a feature
  is removed is the cleanest evidence the removal is real rather than merely
  disconnected.

**`e2e.py` is still 81/81, unmodified in what it checks.** That number has been
the standing evidence through sixteen slices that the vault surface did not
move, and it survives the removal of the credential it used to present. Every
integration suite now *earns* a credential where all of them were previously
handed one — which means **none of them had exercised authentication at all**
before this change; they exercised a constant compare.

The bootstrap nonce is single-use, so the suites cannot each claim the server:
`bootstrap.py` does it once and the rest inherit, which is also how a real
deployment works. `auth_e2e.py` keeps its own virgin server, because it is the
suite that *tests* bootstrapping. `mcp_e2e.py` now carries two credentials on
purpose — a session for REST fixtures and an `stk_` key for `/mcp` — because a
key is refused on REST by design, so one credential could not do both.

**Evidence.** `make check` clean: clippy, analyze, 303 Rust and 660 Dart tests.
`make test-live`: e2e **81/81**, mcp 57/57, auth 66/66, client 20/20.
`no_shared_token_opens_anything` loops plausible bare strings over the REST
surface and `/mcp` and asserts `401` — a loop rather than one value, because
what it guards against is someone reintroducing a constant compare, not this
exact string. `a_server_with_no_users_still_refuses_everything` pins the state
prod is in the moment it upgrades: **no network route to authentication at
all**, which is A8 working rather than failing.

**The finding worth keeping: a credential every test is handed is a credential
no test checks.** Sixteen slices of authentication work sat on top of suites
that authenticated with a constant, so the whole surface was green against a
path no real client would ever take. The same shape as slice 12 (a tier no test
presented a credential for) and slices 13–16 (a harness that supplies what a
person would have to carry) — but larger, because it was every suite at once.

## A lesson worth keeping

Four bugs reached the user in a row, all in the same place: **widget and
provider wiring, which had no tests at all**, sitting on top of a sync layer
that had been tested exhaustively. Two of them destroyed data (every note
truncated to its frontmatter) and none were caught by a fully green suite.

- Watching a `ChangeNotifierProvider` rebuilds on every notification, so the
  editor session was destroyed mid-open.
- A theme object without value equality made every frame look like an edit,
  which saved empty notes in a loop.
- Note creation bypassed the sync engine, so a new note existed on the server
  and nowhere locally.
- A dialog disposed its `TextEditingController` when `showDialog` returned,
  while the route was still animating out.

Each was found only by running the app on a real device. The pattern: protocol
correctness was over-tested and *the layer the user actually touches* was not
tested at all.

**The follow-up is evidence the lesson took.** The UI refactor wrote its suites
before wiring the screens to real state, and four more bugs of exactly this
family surfaced — a router silently replaced out from under its `MaterialApp`,
a keyboard-inset check that could never be true, a menu conflating "Paragraph"
with "dismissed", a toolbar button that could have written text directly.
*Three of the four were caught by tests, before the app was ever launched.*

The fourth is why the second half of this lesson stands: the app opened in
light mode despite a comment two lines above the default claiming dark-first,
and nothing but a screenshot from the phone was ever going to catch that. Tests
find the wiring; running it finds what the wiring was wrong *about*. Do both.

Note also that the last of these was reported as "still the same error" after
two rounds of unrelated fixes. **Get the exception text before changing
anything** — "red screen" means an unhandled exception, which is a different
failure from any error state the app renders itself.

---

### The storage root the VM was actually serving

Found on 2026-08-08 while fixing the root persistence, and worth keeping because
the symptom was invisible: `state/vaults.json` said the root was
`/mnt/media/Docs/storm` — an NFS mount from `nas.lan`, set from the app — while
the server was serving `/home/dewansh/storm/vaults`, whose newest note was from
2026-08-07 10:56. The registry was being overwritten by `--vault-root` on every
restart, so **each restart quietly moved the user back onto a stale copy**, and
the `shit 💩` vault reported `missing` only because its directory exists solely
under the real root.

Nothing was lost — the stale copy is still at `/home/dewansh/storm/vaults` —
but it is the closest this project has come to the failure it is written to
avoid. Two things followed from it:

- The fix landed with every vault reconciling `indexed=0 updated=0` against the
  NAS root, which is the proof the index had been built from *those* files and
  the home copy was the impostor.
- `run.sh` no longer passes `--vault-root`. The storage root lives in
  `state/vaults.json` and is set from the app; a flag beside it would only be a
  second, misleading answer.

**A backup was taken before the switch** (`/home/dewansh/.storm-last-backup`),
and `storm-server.prev` / `run.sh.prev` sit beside the live ones.

---

## Blockers

**None for development.** Deployment notes that still matter:

- *Sudo on the VM.* `apt install` / `storm-server up` need a password; packaging
  shortens the manual step rather than discharging it. Clean apt install is
  **done** (M15): packaged unit, state under `/srv/storm`, vaults on NAS.
- *The shared token is still `testtoken`* — confirmed 2026-08-13, a request
  carrying it returns 200 from `/v1/vaults` on the VM, despite `postinst` and
  `up` generating a real one. Rotating means updating every device at the same
  moment, which is the whole problem; decision 52's per-device sessions are the
  real fix. LAN-only today, so a known risk rather than an exposure.

Both former build blockers are discharged as of 2026-08-05:

- *macOS builds* — `flutter doctor` reports Xcode 26.6 healthy, and
  `flutter build macos --debug` produces a running app. The
  `DVTDownloads.framework` version skew that needed
  `sudo xcodebuild -runFirstLaunch` is gone. **A running app was not a working
  one**, though: the target carried the stock Flutter entitlements, which
  sandbox the app and grant it no outgoing network access at all, so it could
  never have reached the server — in debug either. See decision 35;
  `make install-mac` is now the installed copy.
- *Android toolchain* — SDK 36.0.0 is installed; debug APKs build and install
  on the Pixel over `adb`.

One thing worth knowing rather than blocking: `flutter doctor` flags no Chrome,
so `flutter run -d chrome` won't launch. It does not affect `flutter build web`
— the bundle builds and is served by the server binary (`make serve-web`),
which is how the web client is actually used.

---

## Cutover discipline

Run against a **copy** of the real vault for all of M1–M6, and keep Obsidian +
Syncthing live throughout. Only after several weeks of parallel running does the
real vault move over and Syncthing get switched off.

Get the nightly backup working the day the server first touches real data — not
at M6. `state/` holds version history that the merge depends on, so back up both
`vault/` and `state/`.

**M9 adds the first step that moves the real vault's location.**
`/srv/storm/vault` becomes `/srv/storm/vaults/<name>`, and the server then moves
`state/index.db` into the new vault's own state directory on first boot. Take a
backup of all of `/srv/storm` immediately before the move, and verify it opens —
`deploy/storm-backup.sh` already reopens its snapshot for exactly this reason.
`note_versions` is the merge base and cannot be rebuilt from the markdown, so a
lost index is a lost merge history even though no note is lost.

Do it with the server stopped, and check `GET /v1/vaults` returns one vault
before pointing any client at it.

**Done on 2026-08-07.** Backup at `/home/dewansh/storm-backup-20260807-053117`
on the VM; `/home/dewansh/.storm-last-backup` holds the path. The one thing to
know if it ever has to be repeated: `pkill -f "storm-server --vault"` run over
SSH matches *its own command line* and kills the session before the next step.
Use a pattern that cannot match the invoking shell.

---

## Open items (not v1-blocking)

- **M18 Desktop keyboard shortcuts** — foundation + first-release set on
  `feat/m18-keyboard-shortcuts`; overlay / palette deferred.
- **M17 Markdown Read Mode** — client implementation on
  `feat/markdown-read-mode`; visual pass + ship still open.
- **M16 marketing site** — SlowFlow redesign landed in `apps/www`, and the
  post-M19 truth pass with it (decision 65). Remaining: confirm Cloudflare
  hostname / dashboard connection; optional real app screenshot for Client
  section. No GitHub deploy workflow.
- **Remote connectivity + authentication** — architecture accepted
  2026-08-13; **auth phase 1 has started** (decision 52, and M19 above).
  Server-local users, a cryptographic server identity, QR pairing, and an
  optional self-hostable relay. Docs in the personal vault under
  `Storm/Remote/`: *Storm Remote Connectivity* (index), *Storm Authentication*,
  *Storm Relay*, *Storm Remote Decisions* (R1–R11), *Storm Remote Open
  Questions*, and the checklists *TODO — Storm Authentication* /
  *TODO — Storm Relay*. **Auth is fully designed** (decision 52a): *Storm Auth
  Data Model* + *Storm Auth Protocol*, Q1–Q9 resolved as A1–A12. **Built so
  far:** the server identity slice — `auth.db`, the Ed25519 credential, backup
  coverage, and the two unauthenticated identity routes — verified on the VM
  against a staging server, with **Q18 measured** (192 MiB, t=1, p=1, 173.6 ms);
  and the user model — accounts, roles, Argon2id behind a 2-permit semaphore,
  and the `storm-server user` / `passwd` commands, with no network surface until
  pairing (decision 52c). **Built after the above was written:** sessions,
  three-tier middleware, pairing, `storm-server pair` — all server-side auth
  phase 1 complete. *(Superseded, kept for history.)* Auth phase 1 shipped in
  v0.2.6/0.2.7. Relay phases 2–3 (decisions 56–63) shipped in v0.2.8. **Next:**
  the relay hardening debt, then TOFU persistence, which gates phase 4 (the
  relay on a VPS). The authorization release (A9's second half) runs
  alongside.
- Encryption at rest — deferred, per PRD §10.
- Read-only NAS export of `vault/` for grep and backup tooling. The watcher
  already makes this safe whenever it's wanted.
- iOS — same Dart codebase, needs a signing loop. Out of v1 scope.
- The client stores its token in plain `shared_preferences`.
  `flutter_secure_storage` is a prerequisite for anything beyond the LAN.
