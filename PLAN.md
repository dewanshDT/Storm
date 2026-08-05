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
| M0 | Editor spike | **done** | 67 tests · `spike/editor_spike/FINDINGS.md` |
| M1 | Rust server core | **done** | 86 tests, 0 clippy · 43 e2e checks |
| M2 | Client vertical slice | **done** | superseded by M3 (online-only path) |
| M3 | Cache, outbox, offline, merge | **done** | sync matrix below |
| M4 | Search, tags, backlinks | **done** | 118 tests + 19 live · search p95 1.1ms |
| M5 | Android, Web (Linux deferred) | **done** | Android + web + macOS verified; perf gate open |
| M6 | Attachments, settings, deploy | in progress | |

Last updated: 2026-08-05, entering M6.

### Verify the current state

```sh
make check       # clippy + analyze + 86 Rust and 137 Dart unit tests
make test-live   # 43 server e2e checks + 19 client integration checks
```

`make test-live` starts a server, runs both integration suites and tears it
down, failing fast with the server log if it cannot bind.

Both need a server. `apps/apps/server/tests/e2e.py` creates its own fixtures under
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

```
storm/
├── README.md                 front door
├── PLAN.md                   ← this file: status, decisions, blockers
├── CLAUDE.md                 agent guidance + invariants
├── Makefile                  cross-toolchain tasks (`make help`)
├── apps/
│   ├── server/               Rust — see apps/server/README.md
│   └── client/               Flutter — see apps/client/README.md
├── docs/prd.md               original brief, not maintained
└── spike/editor_spike/       frozen M0 artifact, deleted after M5
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

**4. LAN-only, single shared bearer token.** No TLS, no Tailscale, no public
exposure. This is *only* defensible on the LAN.
*Revisit before:* the server is ever reachable from outside the LAN — TLS and
per-device token rotation must land **first**, not after.

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

**7. `spike/editor_spike/` is a frozen historical artifact, deleted after M5.**
Its four editor files are byte-identical copies of `apps/client/lib/editor/`. They
are deliberately *not* deduplicated into a shared package — the spike records
what was actually built and measured at M0, and restructuring working code to
remove a duplicate that is about to be deleted isn't worth it.

The catch: the spike's perf harness is what validates the editor on a real
Android device (M0's numbers are desktop-only). A frozen spike benchmarks
frozen code. **So if `apps/client/lib/editor/` changes before Android validation,
re-copy the changed files into the spike first, or the harness measures an
editor you no longer ship.** Delete the whole directory once M5 signs off; the
numbers and conclusions live on in its `FINDINGS.md`, which should be moved
somewhere durable at that point.

---

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

---

## Milestones

### M0 — Editor spike ✅

Answered the gating question: a Flutter `TextField` with a custom
`TextEditingController` works.

| Document | typing p95 | caret movement |
|---|---|---|
| 1,000 lines | 0.28–0.83 ms | 0.000 ms |
| 4,800 lines | 3.0–3.8 ms | 0.000 ms |

Findings that constrain later work (detail in `spike/editor_spike/FINDINGS.md`):

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

`apps/server/` — 3,079 lines, 86 tests, 0 clippy warnings, plus 43 end-to-end checks
against a live server (`apps/apps/server/tests/e2e.py`). Full API and behaviour documented
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
| 7 | `apps/apps/server/tests/e2e.py` (watcher picks up an external edit) |
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

### M5 — Android, Linux, Web 🟡

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

*Still open:* the M0 perf gate on Android. The spike's HUD has not been run on
a real device, so the editor's typing latency there is still unmeasured — M0's
numbers are desktop-only and assume a 3–5× penalty.

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

*Exit:* the same note edits round-trip across all four platforms, **and** the
editor is validated on a real Android device using the spike's perf HUD (see
decision 7 — re-copy `apps/client/lib/editor/` into the spike first if it has
changed). Delete `spike/editor_spike/` once that passes, preserving its
`FINDINGS.md`.

### M6 — Attachments, settings, deploy ⬜

Attachment upload/download (LWW by mtime), settings, theming. Deploy to an LXC on
`pve-II` with a systemd unit. Nightly rsync of `vault/` + `state/` to TrueNAS.

---

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
tested at all. The UI now has ~19 tests, and the habit worth keeping is to run
the thing on a device early rather than trusting a green suite to mean working
software.

Note also that the last of these was reported as "still the same error" after
two rounds of unrelated fixes. **Get the exception text before changing
anything** — "red screen" means an unhandled exception, which is a different
failure from any error state the app renders itself.

---

## Blockers

**macOS builds — needs one sudo command from the user.**
`/Library/Developer/PrivateFrameworks/DVTDownloads.framework` is v17.0 (Dec 2025,
from an older Xcode) while `/Applications/Xcode.app` is Xcode 26.6, so
`IDESimulatorFoundation` fails to load. Not an SPM issue; not a Flutter or Storm
problem. Fix:

```sh
sudo xcodebuild -runFirstLaunch
```

Until then macOS is verified only via `flutter test` and the web build.

**Android toolchain — not installed.** Needs the SDK + a JDK (~10–15 GB). The
user opted to free disk first; ~27 GB free as of 2026-08-05.

---

## Cutover discipline

Run against a **copy** of the real vault for all of M1–M6, and keep Obsidian +
Syncthing live throughout. Only after several weeks of parallel running does the
real vault move over and Syncthing get switched off.

Get the nightly backup working the day the server first touches real data — not
at M6. `state/` holds version history that the merge depends on, so back up both
`vault/` and `state/`.

---

## Open items (not v1-blocking)

- Encryption at rest — deferred, per PRD §10.
- Read-only NAS export of `vault/` for grep and backup tooling. The watcher
  already makes this safe whenever it's wanted.
- iOS — same Dart codebase, needs a signing loop. Out of v1 scope.
- The client stores its token in plain `shared_preferences`.
  `flutter_secure_storage` is a prerequisite for anything beyond the LAN.
