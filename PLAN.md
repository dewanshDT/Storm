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
| M16 | Marketing / home site (Astro) | **in progress** | SlowFlow redesign shipped in `apps/www` · CF hostname still TBD |
| M17 | Markdown Read Mode | **in progress** | `flutter_markdown_plus` · Read default · Edit keeps source editor |
| M18 | Desktop keyboard shortcuts | **done** | Intents/Actions · platform Meta/Ctrl · find + sidebar collapse |
| M19 | Auth phase 1 — server identity, users | **done** | slices 1–16 + A14 MCP keys + **the A10 cutover** · `STORM_TOKEN` removed entirely · pairing, sessions and MCP keys are the only credentials · authorization is its own release |

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

**Still open:**

- Optional: real running-app screenshot for the Client section (prototype PNGs
  stay design reference only — not shipped as product photos).

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
- **M16 marketing site** — SlowFlow redesign landed in `apps/www`. Remaining:
  confirm Cloudflare hostname / dashboard connection; optional real app
  screenshot for Client section. No GitHub deploy workflow.
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
  phase 1 complete. **Next:** Flutter client auth integration. A9 (vault-level
  grants) ships between client and relay. Relay work does not start until auth is
  done.
- Encryption at rest — deferred, per PRD §10.
- Read-only NAS export of `vault/` for grep and backup tooling. The watcher
  already makes this safe whenever it's wanted.
- iOS — same Dart codebase, needs a signing loop. Out of v1 scope.
- The client stores its token in plain `shared_preferences`.
  `flutter_secure_storage` is a prerequisite for anything beyond the LAN.
