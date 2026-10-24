# Storm

Self-hosted markdown notes: Flutter clients + a Rust sync server in the homelab,
replacing Obsidian + Syncthing.

## The vault is the source of truth

**Storm's plan, status and decisions live in the personal vault (Storm MCP),
never in this repo.** There is no `PLAN.md`; it grew to 600 KB and was removed
on 2026-10-15 (its text is in git history). Read the vault before starting
work, and **update it as part of the change** — not afterwards:

- **`Storm/Active Work.md`** is the short in-flight queue. Read it when
  starting multi-step work; update it when an item moves.
  `.cursor/rules/storm-active-work.mdc` requires this.
- **`Storm/Global Todo.md`** is the one aggregate checklist — every milestone,
  the Agent Runtime phases, in-flight items, backlog and parking lot. When a
  milestone, phase or queue item changes state, update it in the same change.
- **Decisions are numbered** and recorded in the vault note for their area
  (the feature's design note, `Storm/Remote/…`, the area's *Decisions* note),
  with what would justify revisiting each. Don't relitigate a settled one
  without a reason; if one changes, amend it there with why. Code and docs
  cite them as "decision 77b". Active Work names the next free number.
- **Specs are normative in the vault**, never written into the repo.
- A finding that exists only in a chat transcript is lost. A slice that
  lands without its vault note updated is not done.

**Every vault note an agent edits carries a `summary:` in its frontmatter.**
Add it when creating a note; refresh it whenever you update one — one
plain-text line capturing what the note is (for a change, what it now is).
It is what search indexes and what a later agent skims first; a note whose
summary still says "proposed" long after it shipped is a note that lied.

`docs/prd.md` is the original brief and is **not** maintained. Where it and
the vault disagree, the vault is current.

## Branches: `staging` is where development lands

**`staging` is the development trunk. `main` is the release branch.**

- **Open every PR against `staging`.** Feature branches branch from `staging`
  and merge back into it. This is true for agents and humans alike, and it is
  true regardless of how small the change is.
- **Never open a PR against `main`, never push to `main`, never merge to
  `main`** — not as a shortcut for a one-line fix, not because `staging` looks
  behind. `main` moves only when a release is being cut.
- **A merge to `main` *is* the release decision**, and it is the operator's to
  make, never an agent's. `main` reflects what is deployed or deployable; if
  `main` moved, something shipped. Ask; do not infer that a green branch wants
  merging.
- **`staging` is not a staging *server*.** Nothing on it is deployed. Prod
  serves what came off `main`, so "it works on `staging`" says nothing about
  what users are running — and the gap can be many commits wide. State the
  branch whenever you report status.

Practical consequences worth knowing before you start:

- **Branch from `staging`, not `main`**, or your PR will carry every commit
  that is on `staging` and not yet released, and its diff will be unreviewable.
- **Decision numbers are global, and work on `staging` may already have used
  some `main` has never seen.** Take the next free number from
  `Storm/Active Work.md`. A gap is harmless; a duplicate is not.
- When a release does happen, `main` gets the merge and the tag together — see
  the vault's *Storm Releases* for how versioning works.

## Layout

| Path | What |
|---|---|
| `apps/server/` | Rust sync server (axum + rusqlite). See `apps/server/README.md`. |
| `apps/relay/` | `storm-relay` — the SRP v1 relay. Standalone crate; **no workspace** (R6). |
| `tools/relay-conformance/` | Black-box SRP v1 suite every relay must pass, by URL only (decision 108a). Its `fixtures/` are the contract a relay under test is started with. |
| `apps/relay-cf/` | The same relay on Cloudflare Workers + Durable Objects (decision 108b). TypeScript; nothing in it may live only in memory, since hibernation evicts it. |
| `apps/server/src/gateway/` | The MCP Gateway's store, data key and upstream client (M21, decision 81). Policy stays in `ops.rs`. |
| `apps/runtime/` | `storm-runtime` — the Agent Runtime's Runtime Host (M20, decision 77). Standalone crate; never depends on `apps/server`. |
| `apps/client/` | Flutter app — macOS, Windows, Linux, Android, web. See `apps/client/README.md`. |
| `deploy/` | systemd units, `storm.env` template, nightly backup script. See `deploy/README.md`. |
| `apps/www/` | The marketing site and the docs (Astro + Starlight). See `apps/www/README.md`. |
| `docs/site/` | **The docs' pages** (`/docs`), markdown next to the code. A user-facing change updates its page here in the same PR. |
| `docs/srp-v1.md` | A pointer: the normative relay spec is the vault note *Storm Relay Protocol*. `docs/srp-vectors.json` is its shared test vectors. |
| *Storm Relay Dart Client* (vault) | The client half's accepted architecture. Read before writing any Dart SRP code. |
| *Agent Runtime/V1 Specification Freeze* (vault) | What Agent Runtime V1 builds. Frozen; read it, then decision 77, before touching `apps/runtime` or `/v1/agent/*`. |
| `docs/prd.md` | Original brief. Superseded by the vault; not maintained. |
| `docs/editor-findings.md` | Why the editor is built the way it is, with measurements. |
| `docs/storm-ui-refactor.md` | M7/M8 design brief — dashboard, nav bubble, toolbar. |
| `docs/storm-multi-vault.md` | M9/M10 design brief — vaults, folders, storage root. |
| `docs/storm-properties.md` | M11 design brief — typed frontmatter properties. |
| `docs/storm-adaptive.md` | M12 design brief — the wide-screen layout. |
| `docs/storm-ui.md` | What every screen does today, for designing against. |
| `docs/design_handoff_storm_design_system/` | M14 design system + prototype. `README.md` is the brief; the two `.dc.html` files open in a browser. |
| `design_handoff_storm_v2/` | **M22 (Storm v2) design source of truth** — activity rail, single user, the agent loop. The prototype wins over its README. |
| `docs/design/` | v2 discovery (`STORM_UI_UX_DISCOVERY.md`), the implementation plan (`STORM_V2_IMPLEMENTATION_PLAN.md`) and the visual acceptance set + harness (`acceptance/storm-v2/`). |

Read `docs/editor-findings.md` before changing anything in
`apps/client/lib/editor/`. It records the constraint the whole editor rests on
— the rendered span tree must match the buffer character for character — and
the on-device numbers that say where the limits are.

Read `docs/storm-multi-vault.md` before touching vault resolution, the registry,
the watcher, or the client's cache schema.

## Commands

Use the Makefile — it encodes the cross-toolchain steps, and `test-live` starts
and tears down a server around the integration suites.

```sh
make help                    # every target
make check                   # clippy + analyze + both unit suites
make test-live               # integration suites against a real server
make fmt                     # cargo fmt + dart format (CI enforces both)
make server VAULT_ROOT=~/vaults   # run the sync server
make dry-run VAULT_ROOT=~/vaults  # ALWAYS do this before importing a real vault
make serve-web               # build the web client and serve it
```

`VAULT_ROOT` points at a directory *containing* vaults, not at a vault —
pointing it at one would make the server treat that vault's own folders as
vaults. Every target needs a `## name: description` line above it or
`make help` won't list it.

`make check` must be clean before a change is done — clippy runs with
`-D warnings`, so a warning is a failure. The Makefile targets GNU Make 3.81
(what macOS ships), so no `.ONESHELL` or `.SHELLFLAGS`.

**Never filter a build's output down to a success pattern.** Piping
`flutter build` through `grep "✓ Built"` prints nothing when the build *fails*,
and silence reads exactly like success — a stale APK then installs happily and
the missing feature looks like a UI bug. Check the exit status, or read the
tail of the real output.

**Android plugins must support Flutter's built-in Kotlin.** A plugin that
applies its own Kotlin Gradle Plugin won't have its classes compiled, while
the generated registrant still references them, so the build fails on a symbol
that looks like it should exist. Prefer first-party `flutter/packages` plugins
(`file_selector` over `file_picker`, for instance).

**A release APK has no network unless the *main* manifest grants it.** Flutter's
template declares `android.permission.INTERNET` in
`android/app/src/{debug,profile}/AndroidManifest.xml` only, so hot reload can
reach the host — every debug build works, and the first release build on a
phone cannot open a socket at all. The kernel refuses it with `EPERM`, which
the app reports as "Couldn't reach the server", indistinguishable from a wrong
address. `test/android_manifest_test.dart` guards it. **A hash-verified install
proves the bytes arrived, not that the app works** — open it.

**The macOS app is sandboxed, so its entitlements are part of the build.**
`macos/Runner/{DebugProfile,Release}.entitlements` must grant
`network.client` — without it the app builds, launches and then reports the
server unreachable whatever address it is given — and
`files.user-selected.read-only`, which is what lets it read a file chosen in
the attachment picker. `test/macos_entitlements_test.dart` guards both, because
neither failure says anything about entitlements. `make install-mac` builds the
release app and puts it in `/Applications`.

**A new server operation goes in `apps/server/src/ops.rs`.** REST handlers and
MCP tools are both thin callers of it — a handler holds extractors and `Json`,
a tool holds params and structured content, and neither holds logic. Adding an
operation to `api.rs` alone makes it invisible to MCP; adding one to `mcp.rs`
alone starts the divergence `ops.rs` exists to prevent. `tests/e2e.py` is what
proves an extraction left REST unchanged.

## Invariants worth knowing before editing

These are load-bearing and each has a regression test. Breaking one loses user
data quietly.

- **The vault is plain markdown, always.** Storm's own state lives in a sibling
  `state/` directory, never inside the vault.
- **Frontmatter is never serialized.** Storm rewrites individual *lines* and
  passes every other byte through. Running a user's YAML through a serializer
  reorders keys and drops comments, dirtying the whole vault. There are two
  writers and both obey this: `frontmatter.rs` `set_scalars` on the server, and
  `lib/editor/frontmatter_edit.dart` on the client. **A leading UTF-8 BOM is
  part of no block and always stays at byte 0** (decision 128e): both sides
  look past it for the opening fence, and a new block is written after it.
- **A value that spans lines is spliced as a range, never as one line.** The
  server's writer replaces a key's single line, which is right for stamping
  `id` and wrong for anything else: aimed at a `tags:` block list it orphans
  the `- item` children and leaves invalid YAML. The client's writer knows the
  span, and refuses to write a nested map or a block scalar at all.
- **The server owns `modified:`.** Clients must not write it, and it is
  normalised out of all three sides before a merge — otherwise every concurrent
  write conflicts on that line.
- **The editor's span tree must flatten back to the buffer exactly.** A gap or
  overlap silently corrupts rendered text and every caret offset after it.
- **A `merged` or `conflict` response means the client adopts the server's
  text.** Keeping local text makes the next save race a version it never had.
  **What was typed while that save was in flight is never dropped** (decision
  128b): it is replayed onto the server's text (`replayEdit`), or, where its
  place cannot be found exactly once, kept on the base it was edited from and
  saved again so the server's diff3 merges it.
- **An unsynced edit outlives its note's deletion** (decision 128c). When the
  outbox's write gets a 404 because the note was deleted elsewhere, the edit
  is kept as a new note at the note's path (`Name (recovered).md` if taken),
  and the user is told; a remote delete or a 404 on open never drops the
  cached copy of a queued edit. A recovery create whose answer never came is
  not resent blind (SRP §8.5): the next attempt looks for it first.
- **Notes are tracked by UUID, not path.** Renames and moves are metadata
  updates.
- **An id seen at a new path is a move or a copy, never a version reset**
  (decision 128d). Outside Storm, a file whose `id` is indexed at a path that
  is gone is a move (same row, history kept); one whose indexed file still
  exists is a copy and gets a fresh id. Every write takes
  `Db::next_version` — past the note's whole history — and `note_versions` is
  append-only (plain `INSERT`). Before 128d an external rename restarted the
  note at version 1 and overwrote its history.
- **Unreadable is not deleted** (128d). Reconcile keeps the row of a file it
  cannot read, removes nothing after an I/O error, and does not prune a vault
  that scans empty while its index holds several notes.

From M9/M10 (`docs/storm-multi-vault.md`):

- **A vault is a directory under the storage root, tracked by UUID.**
  `state/vaults.json` maps id → directory and display name, so renaming either
  is a registry edit. If the directory name were the identity, a rename would
  orphan the vault's index and every client's cached notes.
- **Each vault has its own index at `state/<vault-id>/index.db`.**
  `change_log.seq` is therefore per vault, and so is a client's sync cursor.
  One shared cursor would have two vaults overwriting each other's position,
  which surfaces as randomly missed changes rather than as an error.
- **Explicitly created folders are recorded and exempt from pruning.** The
  server deletes directories that become empty; without the `folders` table
  exemption a new empty folder disappears on the next delete or move.
- **The storage root holds vault directories and nothing else Storm reads.**
  `scan_root()` skips `state_dir` and every dot-prefixed directory — the
  `--vault` compatibility shim puts `state/` inside the root, and a naive scan
  would register it as a vault and index the SQLite files in it.
- **A colour is stored as a word, never a hex value.** `color: sage` in a
  note, `storm.color:` in a vault's config. The vault has to stay readable
  outside Storm, and a stored hex would pin it to one theme and mean nothing
  in Obsidian.
- **The phone layout is the default; wide-screen behaviour is additive.**
  There is one breakpoint (900px, `lib/ui/breakpoints.dart`). A change that
  alters what renders below it is a defect, not a design choice — every
  adaptive test asserts both sides for that reason.
- **Every pane beside the rail or a sidebar sits in `PaneSemantics`** (Storm
  v2). The pane is a nested navigator whose route carries a modal barrier
  with `BlockSemantics`, which otherwise removes every sibling painted before
  it — the rail, the sidebar — from screen readers.
- **`/` is not a screen** (Storm v2). It redirects to the device's last
  location (`NavMemory`), and system back follows `logicalParent` in
  `AppShell` once the router has nothing to pop; only a vault root exits.
- **Unseen is device-local and never stored on the server** (Storm v2,
  slice 8): the last version each device opened is in its prefs
  (`SeenVersions`), compared with the server's `agent-writes` map. A server
  copy would clear a phone's dot because the laptop opened the note. A
  device's first successful `agent-writes` load for a vault records a
  `"<vault>/"` baseline marker and takes the versions it found as seen
  (slice 9); markers are never evicted, or a long history would re-baseline
  and hide real dots.
- **Prose is never `SelectableText` on the open note** (Storm v2). Flutter
  web leaves a read-only text field's text out of the DOM, so the note body
  vanished from screen readers; Read mode is plain text in a `SelectionArea`.
  `test/semantics_test.dart` holds it, with the provenance link's own node.
- **The terminal's theme is one instance per colour set** (Storm v2). xterm2
  reports the colour scheme to the agent on every non-identical theme while
  DEC 2031 is on; a report after the agent restored echo prints as
  `^[[?997;1n` in the scrollback.
- **A terminal link opens only for `http`, `https` and `mailto`, and a tap
  asks first** (decision 122). Agent output is untrusted and an OSC 8 link's
  text can differ from its target, so the touch sheet shows the real target;
  a tap is also how the terminal takes focus, so it never opens directly.
  Only a Cmd/Ctrl-click opens at once.
- **Storm never moves vault directories.** Changing the storage root points the
  server at directories someone already moved. A change that would orphan every
  registered vault is refused rather than applied quietly, and a vault whose
  directory is gone stays in the registry marked `missing`.
- **The stored storage root wins over `--vault-root`.** `state/vaults.json` is
  the source of truth; the flag seeds a registry that does not exist yet, and a
  disagreement is logged rather than silently resolved. `Registry::load` used to
  overwrite the parsed root with its argument, so a root chosen in the app was
  recorded, ignored on the next boot, then erased by the next save — with every
  vault adopted under it left registered as `missing`. **A setting that does not
  survive a restart is not a setting.**
- **Storm's data is its service user's and group's, never other accounts'**
  (decision 76). Both units set `UMask=0027`, and `postinst`, `up` and `serve`
  each restrict the directories Storm alone owns. Up to 0.2.9 every local
  account could read every note and `state/auth.db`. Every layer only ever
  *removes* group-write and other bits, so a directory an operator made
  stricter keeps its mode. **A vault root outside the data root is never
  chmod-ed:** it is a NAS share other machines write, and its modes are the
  NAS's. A local account that needs the data joins the service user's group.
  Agent Runtime's `storm-runtime` user must never join it, because this mode is
  all that keeps a co-located runtime out of `auth.db` and the vaults.

From M19 (auth phase 1 — design in the personal vault, *Storm Auth Data Model*
and *Storm Auth Protocol*; ADRs A1–A11 in *Storm Remote Decisions*):

- **`state/auth.db` and `state/identity/` cannot be rebuilt.** Everything else
  in `state/` regenerates by rescanning markdown; these do not. Anything that
  touches `backup_all()` must keep carrying both — and the key files, not just
  the database, because `auth.db` alone restores a server that knows which key
  is active and cannot sign with it. Backing up before the "no vaults
  registered" early return is deliberate.
- **The private key is a file so its protection is `ls -l`-auditable.** Mode
  `0600`, in a `0700` directory, created *with* that mode rather than chmod-ed
  afterwards. It is never logged, never serialized, and never in a payload —
  `ServerIdentity`'s `Debug` impl is hand-written to redact it.
- **`server_id` is random, never derived from a key** (A3). Deriving it would
  make rotating a credential change the server's identity and force every
  paired client to re-pair.
- **A route that must be unauthenticated goes *below* the `require_token`
  layer.** axum applies a layer only to the routes registered above it.
  `/v1/health` is the older shape — above the layer, exempted by path inside
  the middleware — and `/v1/server` + `/v1/server/challenge` are below it.
  Both patterns have a test; do not change one to match the other by eye.
- **The challenge signs `storm-challenge:v1:<server_id>:<nonce>`, not the
  nonce.** An unauthenticated endpoint signs whatever it is sent, so the domain
  prefix and the server id are what stop it being a signing oracle. The client
  rebuilds the same string, so this is a wire-format commitment.
- **Passwords and tokens will hash differently on purpose.** Argon2id for
  low-entropy secrets, blake3 for 256-bit random ones. This is not an
  inconsistency to tidy up in either direction.
- **Auth work is additive until the middleware slice.** `require_token` and the
  shared token stay exactly as they are, so `apps/server/tests/e2e.py`'s 81
  checks pass unmodified — that unchanged pass is the evidence. New coverage
  goes in `tests/auth_e2e.py`.

From M19 slice 2 (users and passwords):

- **Argon2id's parameters are measured, not chosen** — `m = 192 MiB, t = 1,
  p = 1` on the VM (Q18/A1). `p` stays 1 because the `argon2` crate does not
  thread without its `parallel` feature, so more lanes cost memory and buy no
  speed. Different hardware is a re-measure with `tools/argon2-bench`, not an
  edit.
- **Every Argon2 call goes through `Hasher`, which holds a semaphore of 2.**
  A verify runs on `spawn_blocking`, whose pool defaults to 512 threads, and
  512 × 192 MiB is an OOM that takes the vault server down with it — the notes
  go offline because someone held the login button. **Never shrink the KDF to
  work around a missing bound; the bound is the fix.**
- **A password is refused above 1024 bytes, never truncated.** Accept 200
  characters, hash the first 72, and every password sharing that prefix opens
  the account. A test hashes 1000 bytes and checks that a variant differing at
  byte 900 fails.
- **Storm is single-user** (decision 82). `users` is the one-row account table
  (`only_row`), password only — no usernames, roles or status. `auth.db` v6
  (`auth/single_user.rs`) collapsed older databases onto the oldest active
  owner after writing `auth.db.pre-v6`, in one transaction; its invariants are
  checked on every open. `agent.db`/`gateway.db` are reconciled on every boot
  (`ops::reconcile_single_user`). `user_id` columns hold the account id.
- **No `--password` flag, in any command, ever.** A password in an argument is
  in the shell history and in `ps` for every other user on the box. Prompt
  without echo, or read `--password-stdin`.
- **`security_events` never contains a secret** — not a password, not a hash,
  not a token. A test asserts it for every administrative act.

From the relay (SRP v1 — **the normative wire spec is the vault note
*Storm Relay Protocol*** (Storm MCP, personal vault); specs live in the vault,
never in this repo. `docs/srp-v1.md` only points there, and section numbers
cited in code are that note's. ADRs R1–R14 are in *Storm Remote Decisions*):

- **The relay is never an authority (R5) and authenticates no clients (R12).**
  It has no user database, no vault data and no authorization. A client
  presents it no credential; the client's credential rides *inside* the
  tunnelled request and is checked by the origin server. A change that would
  have `apps/relay` know what a Storm *user* is, is wrong. Adding client
  authentication is the mistake that made the first draft of the spec wrong.
- **The relay must never become mandatory (R6).** Hence `apps/relay` does not
  depend on `apps/server`, and there is deliberately **no root Cargo
  workspace** joining them — a workspace is how "optional" quietly becomes
  "linked in".
- **A tunnelled request is the same request (R13).** It is dispatched
  **in process, through the same `Router`**, so it runs the same
  `require_auth`, the same tier routers and the same `ops.rs` calls as a LAN
  request. There is no loopback TCP hop and no bypass: if a handler cannot
  serve a relayed request, the *handler* is what changes.
- **`relay_peer_ip` travels out of band and forwarding headers are stripped.**
  `HTTP_REQUEST_HEAD.headers` is client-supplied end to end — the relay
  forwards it verbatim — so a client could otherwise set headers that other
  code treats as *proxy-set* facts. `web_bootstrap_nonce` refuses to mint a
  nonce when one is present, which a client that could set them would turn
  into a remote off-switch. The peer address comes from the accepted socket —
  or, on the Worker, whose provider edge *is* the front end, from the
  `CF-Connecting-IP` that edge attests and refuses to accept from a client
  (403, error 1000) — never from a header a client can set.
- **A shutdown may cancel the phases before registration, never the serving
  phase.** Connect and registration have announced nothing, so dropping them
  costs nothing. `serve_trunk` owes the relay a `DEREGISTER` — cancelling it
  leaves the relay holding the `server_id` until a heartbeat timeout, which is
  the outage `DEREGISTER` exists to prevent.
- **The wire format is re-derived in five places and `docs/srp-vectors.json`
  is what makes them agree.** The signed messages (`storm-relay-auth:v1:`,
  `storm-challenge:v1:`, `storm-relays:v1:`), `validate_nonce`,
  `validate_server_id`, the base64url rules, body-frame bytes and control
  shapes exist in `apps/server`, `apps/relay`, `apps/relay-cf`, `apps/client`
  and `apps/runtime`, which cannot depend on one another. Nothing in any build
  makes them agree, and drift surfaces at runtime as `auth_failed` — identical
  to a genuine attack, an expired nonce and a refused binding. All five test
  suites read the one vector file; regenerate it with
  `tools/srp-vectors/generate.py`, and treat a changed vector as a protocol
  change: the vault spec first, then a conformance or role-suite check
  (`tools/relay-conformance`, `--role=origin`).
- **base64url, unpadded, and decoders must *reject* the alternatives.** One key
  has one spelling. `dart:convert`'s `base64Url` decoder accepts the standard
  `+/` alphabet and accepts padding; `data_encoding`'s `BASE64URL_NOPAD` does
  not. §4.1 binds a `server_id` to a pubkey permanently with no way back, so
  two sides disagreeing about whether a spelling is valid disagree about
  whether a key has *changed*.
- **Keys are compared as decoded bytes, never as strings**, for the same
  reason.
- **A TOFU binding is on disk before `REGISTERED` goes out** (decision 69).
  With `--bindings`, `record` writes atomically under the lock and un-records
  the pair if the write fails. Announcing a binding a restart could forget
  re-opens the first-use window, and §4.1 makes a squatted `server_id`
  permanent. A corrupt bindings file stops the relay rather than starting empty.
- **A public relay terminates TLS itself, never behind a proxy** (decision 71).
  A proxy makes every socket its own, collapsing the per-IP `HELLO` limit and
  the origin's `relay_peer_ip` to one address. `PeerAddr` comes from the TCP
  peer on both listeners. **A TLS handshake never runs inside
  `Listener::accept`**, or one silent socket stalls every connection.
- **storm-server installs `ring` as the rustls provider at the top of `main`.**
  Tungstenite's `wss://` client uses the process default, and rustls panics
  on the first handshake if two providers make it ambiguous. Never enable
  `aws-lc-rs`: the musl/zig release build cannot compile it.
- **storm-server waits for its storage** (decision 73). The unit is
  `After=remote-fs.target`, and `up`'s drop-in has `[Unit] RequiresMountsFor=`
  matching its `ReadWritePaths`. systemd will not spawn a process whose
  `ReadWritePaths` entry is missing, so an NFS vault root that mounts late
  was a `226/NAMESPACE` loop, and with 0.2.9's live restart limit a dead
  server. `[Unit]` keys under `[Service]` are ignored with only a warning.
- **Every `PING` is answered, at any point on a trunk, and a `PONG` is never
  dropped** (SRP §4.2, decision 108c). The Worker relay `PING`s to measure
  what a peer has read, so a `PONG` lost on a full queue reads as a stalled
  reader. storm-server queues them on its `Control` lane, outside the
  bounded queue.
- **Every SRP socket is bounded at 16 MiB a message** (SRP §3, decision
  128f): storm-relay's upgrade sets `max_message_size`, the Worker refuses a
  larger message, and storm-server's and storm-runtime's clients pass a
  `WebSocketConfig` with the same ceiling. A frame limit alone is not enough:
  tungstenite's message default is 64 MiB, and a message split into
  continuation frames was buffered whole for any unauthenticated client.
- **Control before bulk** (SRP §5.2, decision 116b). Every sender — storm-server,
  storm-runtime's client, the Dart trunk, storm-relay — writes `PING`, `PONG`,
  `STREAM_ACK`, `CLOSE` and stream `ERROR` ahead of queued body frames and
  never drops one for want of room. Silence is not judged while a side's
  own `PING` is unwritten.
  Trunk sockets set `TCP_NOTSENT_LOWAT` (Linux, Android, Apple for Dart), or
  the kernel's send buffer undoes the lane on a slow uplink. A response's
  own `CLOSE` stays behind its body.
- **Two relays, one spec.** `apps/relay` and `apps/relay-cf` both pass
  `tools/relay-conformance` (`make relay-conformance-local`,
  `make relay-conformance-cf`). A behaviour the spec leaves open is amended
  into the vault spec in the same change, never settled in one relay's code.
- **Password pairing is LAN-only** (decision 118). `POST
  /v1/pairings/password` refuses a request the relay dispatched (the
  `relay::Relayed` extension, set in `dispatch.rs`; `relay_peer_ip` alone looks
  like any socket peer), a peer outside loopback, RFC 1918, link-local and
  IPv6 ULA, and everything while discovery is off. The client sends the
  password only on a direct route whose challenge verified against the
  advertised key, with no relay candidate at all.
- **Storm never opens a port silently, and never needs one** (decision 119).
  Only the installers add a firewall rule, after asking, LAN-only (ufw from
  local subnets, firewalld home/trusted zones, Windows Private), recorded in
  `/etc/storm/lan-firewall` so uninstall removes exactly that; the package and
  the server never run `ufw` or `firewall-cmd` against a rule. On a network the
  OS calls untrusted (`nettrust.rs`: Windows Public, firewalld or
  NetworkManager `public`/`external`/`drop`/`block`) the server neither
  advertises nor accepts password pairing. Installer tests use the fakes;
  never touch a real firewall from a test.
- **The two signing domains must never coincide.**
  `storm-relay-auth:v1:<server_id>:<nonce>` proves the right to register at a
  relay; `storm-challenge:v1:<server_id>:<nonce>` proves identity to a client.
  Both fields are validated, not just the nonce — they are two colon-delimited
  halves of one signed string, so a colon in *either* re-splits it.

From M20 (Agent Runtime V1 — the spec is the vault's *Agent Runtime/V1
Specification Freeze*; the slices and slice 1's decisions are
decision 77):

- **A Runtime Host is never an authority.** `storm-runtime` runs agents and
  nothing else: it never learns who owns a session, holds no user data and
  makes no authorization decision. The Agent Manager in storm-server is the
  only session authority. A change that has `apps/runtime` know what a Storm
  *user* is, is wrong.
- **`apps/runtime` never depends on `apps/server`**, and there is still no
  workspace. The two share a wire format, not code. Where they must agree on
  bytes, a vector file in `docs/` read by both test suites is what makes them
  agree, as `docs/srp-vectors.json` does for the relay.
- **The provider contract is not the PTY** (AM20). A provider has a kind and
  offers interactions; the runtime owns the carrier. A PTY type in
  `src/provider.rs` is the regression, and the fake provider exists to prove
  the contract works without one.
- **A host token reaches `/v1/runtime/*` and nothing else, and nothing else
  reaches `/v1/runtime/*`** (decision 77b). `RequiredTier::Host` is a tier,
  not a flag, for the reason `Mcp` is, and `require_auth` checks the `sht_`
  prefix in both directions before looking anything up.
- **`storm-host-auth:v1:` is a third signing domain, on a third key.** The
  host signs it; the server never does. It must never coincide with
  `storm-challenge:v1:` or `storm-relay-auth:v1:`. Enrollment signs four
  fields (`…:<server_id>:<token_id>`) and connect signs five
  (`…:<server_id>:<host_id>:<nonce>`), every field validated against the
  colon. `docs/runtime-vectors.json`, regenerated with
  `tools/runtime-vectors/generate.py`, is what keeps both crates in agreement.
- **An enrollment string is parsed from the right.** Its `server_url` has
  colons; nothing after it may.
- **The Agent Manager is the only session authority** (decision 77c). A host
  reports; the manager decides. A session that has ended never changes status
  again, a host may post only for its own sessions, and at boot every session
  that has not ended becomes `unknown` until its host's `hello` says
  otherwise. Guessing `running` after a restart is the bug this prevents.
- **The output cache accepts bytes in order only.** Overlap is skipped, a jump
  forward restarts the ring, and an empty cache takes the first offset it is
  given. That last rule is what lets `terminal.replay` refill it after a server
  restart.
- **A host link is identified by its generation.** An old stream closing must
  never mark a newer link offline. Every lock in `agent/` is a
  `std::sync::Mutex` that is never held across an `.await`.
- **Input is at-most-once, and never blocks the link.** It is refused with 503
  while the host is offline and never queued on the server. On the host, each
  session has its own ordered writer, and every PTY write is bounded, so a hung
  agent can never hold the session against the owner's `end`.
- **`storm-runtime` is its own account and never joins the server's group**
  (decision 77e). The group mode of P3 is all that keeps a co-located runtime
  out of the vaults. Its unit never sets `MemoryDenyWriteExecute`, because
  OpenCode cannot start under it, and `RestartPreventExitStatus=3` keeps a
  revoked host stopped. `apps/runtime/tests/packaging.rs` holds all three.
- **A host verifies its server before sending it anything** — a token, a
  signature, an enrollment. Every connect re-runs the challenge against the
  key pinned at enrollment.
- **Terminal output is addressed by absolute byte offset, and a reader that
  fell behind gets an explicit gap.** Never a short read and never
  `Last-Event-ID`: resume has to be exact, and it has to cross the relay
  (SRP §5.3).

From M21 (the MCP Gateway — the spec is the vault's *MCP Gateway/V1
Specification*; the slices are decisions 81 and 81a onward):

- **`state/gateway/gateway.db` and `state/gateway/keys/` cannot be rebuilt,
  and travel together** (81b). `backup_all()` carries both, before the "no
  vaults" early return. Ciphertexts without their key restore a gateway that
  can authenticate nothing.
- **An upstream credential is never in the clear at rest.** It is sealed with
  XChaCha20-Poly1305 (`chacha20poly1305`, pure Rust) under an AAD of
  `<id> 0x00 <kind>`, so a ciphertext moved to another row does not open. An
  opened secret is a `Plaintext`, whose `Debug` is redacted. Say plainly what
  this buys: protection against a leak of the database alone, not against
  root or a full backup.
- **The data key is a file, `0600` in a `0700` directory, created with those
  modes.** A missing key at boot is not a lockout: a new key becomes active
  and the affected connections become `needs_reauth`.
- **Integrations are managed on the session tier only** (81c). A connection
  is found only among the account's own rows (`404` otherwise). No MCP tool
  manages one, so an `stk_` key never can.
- **A connection's URL, slug and auth kind never change after creation.** A
  credential is presented only to its own upstream (AM24); re-pointing a
  connection would hand its token to a new host. The store's update does not
  write those columns.
- **An integration's audit row names the upstream's host, never its URL** —
  a pasted URL can carry a key in its query string — and never a credential.
- **Only rmcp's `*_once` request methods send anything upstream** (AM26,
  81d). `call_tool`, `get_prompt` and `read_resource` re-send their request
  to drive SEP-2322 rounds, so one agent call would execute several times.
  `only_the_once_methods_send_a_request_upstream` reads `ops.rs` and
  `src/gateway/` and fails on them.
- **The gateway's HTTP client never follows a redirect**, so a credential
  header is never replayed to a host it was not configured for, and its TLS
  is `ring` with the bundled roots, configured on the client itself.
- **An upstream's error text never leaves `gateway::upstream`.** Failures are
  the stable §12 codes; rmcp's `HTTP <status>: <body>` is read for the status
  only. `--gateway-allow-http-upstreams` is hidden and exists for the test
  suites.
- **Every agent call is authorized in `ops::integration_call`, per call**
  (spec §7, 81e): host owns session, session live, live grant, owner is the account,
  connection the account's and `connected`, method permitted, tool allowed,
  and vault writes only with a write vault chosen at launch **and**
  `agent_writes` (not `mcp_writable`, which is `/mcp`'s alone). A refusal is
  a JSON-RPC error with a stable code, never an HTTP error.
- **An agent writes only to its session's write vault, refused at the vault
  seam** (decision 82, slice 5): `StormPolicy` denies `Actor::Agent` +
  `Access::Write` on any other vault, and `mcp.rs` turns that 403 into the
  JSON-RPC code `vault_write_not_allowed`. An older client's
  `allow_vault_writes` without a vault launches read only, never all-vault.
- **No note data on a command line or in an environment** (slice 5). The
  host's `start` carries `context: true`, never text; the runtime maps it to
  the compile-time `mcp::OPENING_PROMPT`, and the agent reads the note
  through `session_context`, keyed on its own session. A launch role
  (decision 138f) rides the same flag and the same tool, which returns
  `{context, role}`; only a session that reads Storm records one, and every
  launch with a role gets it as its first label.
- **A request the gateway did not forward is `session_unknown`, and nothing
  else is.** That is the only answer the bridge may replay `initialize` on.
  Upstream sessions live in memory so a restart produces it; closing one
  cancels it, so an in-flight call fails once and is never re-sent.
- **Request-scoped messages ride their call's own response stream**;
  only unsolicited ones use the link's `mcp.message`. A call's progress and
  elicitations die with it.
- **The host is told connection ids and slugs, never a credential** (AM23);
  `shell` and a host without `mcp_bridge` get no grants. **Agents never see
  `delete_note`** (`mcp::NEVER_FOR_AGENTS`), and `Actor::Agent` is the
  account's identity, read through `user_id()`.
- **The bridge never retries, and replays `initialize` only on
  `session_unknown`** (81f, spec §11). `apps/runtime/src/bridge.rs`'s six
  rules each have a test, and `gateway_e2e.py` catches the gates' `leak_init`,
  `retry` and `no_cancel` mutations against the real build. A late answer to
  an elicitation is dropped, and elicitation ids are unique per upstream
  session so one can never answer another.
- **A session's MCP config is AM32 as amended by AM66**: every `opencode`
  session gets `OPENCODE_DISABLE_PROJECT_CONFIG=1`, grants or not, and its
  workspace's own `AGENTS.md` (else `CLAUDE.md`, else `CONTEXT.md`) back by
  absolute path through `OPENCODE_CONFIG` (`mcp::provider_baseline`; a path
  with a glob character is not restored). With grants, `claude-code` gets
  `--mcp-config …` beside its own servers (decision 132a; `--strict-mcp-config`
  too, grants or not, under `runtime.toml` `agent_mcp = "storm-only"`); an
  integration whose slug the account's `.claude.json` or the workspace's
  `.mcp.json` already uses is written as `storm_<slug>`, since Claude Code
  loads `--mcp-config`'s entry and drops theirs (gate G-132a), except the
  built-in `storm`, which the opening prompt names; `opencode` gets its
  servers and `"<slug>_*": "ask"` in `OPENCODE_CONFIG_CONTENT`, on top of its
  global config (132b; a slug the global config uses is `storm_<slug>`), or
  under `storm-only` its own `XDG_CONFIG_HOME`. **A project's OpenCode config
  stays off either way** (G-132b): OpenCode evaluates permissions last match
  first and a merged key keeps its first position, so a project listing
  `"<slug>_*"` before `"*": "allow"` would turn Storm's asks off; everything
  else gets nothing. A
  config that cannot be written fails the start rather than launching
  unconfined. It holds a handle and a socket path, never a credential.
- **A built-in agent is a profile** (decision 130, `apps/runtime/src/profile.rs`):
  its safe launch args, prompt form, MCP writer, resume id shape and title
  policy. **An agent whose writer has not passed its real-account gate gets
  no Storm servers** (130a): the host reports `mcp: false` for it in `hello`
  and the server grants nothing (`McpOutcome::Unsupported`) — Cursor and
  Command Code today. Their writers exist (131j `cursor.rs`: bridges as a
  `--plugin-dir` plugin; 131k `command_code.rs`: the workspace's local
  `mcp.json`, no handle in it, the account's entries kept and a clashing
  bridge `storm_<slug>`); only under `storm-only` do they disable every
  other server (Cursor's own `mcp-disabled.json`; a disabled stub under
  every other name) (132d). The profiles stay `mcp: None` until G-C1/G-CC1
  pass on a signed-in account. Both slugs are the CLIs' own — Command Code's
  is `@sindresorhus/slugify`, and a non-ASCII path fails the start rather
  than guess; `tests/cursor_command_code_gate.rs` checks them against the
  real binaries. No profile passes a flag that turns an agent's own
  permission prompts off.
- **Codex runs in Storm's own `CODEX_HOME`** (130b, `apps/runtime/src/codex.rs`),
  `<state>/agents/codex`, shared by its sessions so Resume finds a
  conversation; a session's servers are in its own `-p` profile (`0600`,
  removed at the end), each `default_tools_approval_mode = "prompt"`, named
  `storm_<slug>` where the account's config uses the slug (a profile merges
  key by key). **The account's config there is its own** (decision 132c):
  its servers, profiles, plugins, plugin servers, apps, hooks and folder
  trust are kept, and Codex asks its own questions. Storm adds only its
  `SessionStart` hook (matcher `startup|clear`: Codex starts a resume's hook
  without feeding it), appended, **with its `trusted_hash` under
  `hooks.state."<config path>:session_start:<i>:0"`** — SHA-256 of the
  compact key-sorted JSON of the event, matcher and handler with the
  platform's command (gate G-132c; `the_trust_hash_is_codexs` holds a value
  Codex 0.163 reported) — so **no `--dangerously-bypass-hook-trust`**: every
  other hook gets Codex's review. A home with `storm-plugins-off.json` still
  carries 130f/131's switches and the `untrusted` pins; the first launch
  undoes them, once. Under `agent_mcp = "storm-only"`: the account's servers
  withheld with `-c mcp_servers.<name>.enabled=false` (never deleted; `-c
  mcp_servers={}` merges and does nothing), every workspace `untrusted`, a
  plugin's remote servers off and its stdio ones per `plugin_mcp` (131i),
  `features.apps = false` (131l), and what is left out named in the
  session's notice. `--no-daemon` always: a shared daemon holds a
  conversation's writer lock. **No Codex sandbox** (130c): it needs user
  namespaces, which `storm-runtime.service`'s `RestrictNamespaces=true`
  forbids, and relaxing the unit would weaken every agent.
  `tests/codex_gate.rs` proves all of it against a real `codex` when
  `STORM_CODEX_BIN` is set.
- **A host installs only what a recipe compiled into it fetches** (decision
  133, `apps/runtime/src/install.rs`). The server's `agent.install` names an
  agent and an operation, never a URL, version or command; an id with no
  recipe is refused. Downloads are https to the vendors' hosts on every
  redirect hop, capped in size, and a published checksum is always
  verified — **a mismatch never installs** (Cursor publishes none, reported
  as `checksum_verified: false`). A version from the network is checked
  before it names a folder. **Storm updates and removes only what it
  installed**, under `<state>/agents/managed`, whose `bin/` is first on the
  host's search path; activation swaps `bin/<command>` atomically, so a
  running session keeps its binary. `remove` with a live session of the
  agent is `in_use`. **No agent, MCP key or MCP tool can trigger an
  install**: the app's routes are session tier, owner only, and a job is
  audited in `security_events` with codes only, never a URL or npm output.
  `tests/agent_install_gate.rs` (`STORM_AGENT_INSTALL_GATE=1`) proves every
  recipe against the real channels.
- **An agent's conversation id reaches argv only in its provider's shape**
  (decision 89): a lowercase UUID for `claude-code`, `ses_` + alphanumerics
  for `opencode`, checked by the host on every report and every `resume`,
  and by the server before storing. `docs/harness-session-vectors.json`
  keeps the two crates in agreement. The `SessionStart` hook prints nothing
  but a JSON `systemMessage` (decision 131a): plain stdout reaches the agent,
  a `systemMessage` only the person — checked on Claude Code 2.1.306 with a
  positive control.
- **Under `agent_mcp = "storm-only"`, a Claude Code session's plugins' local
  MCP servers are written into its `mcp.json` by Storm, and strict stays**
  (decision 131, `plugins.rs`); otherwise Claude Code loads its plugins
  itself (decision 132). The
  host lists the enabled plugins with the provider's own `claude plugin list
  --json`; a stdio server is keyed `plugin_<plugin>_<server>` (Claude Code's
  own tool prefix; no slug has `_`) with `${CLAUDE_PLUGIN_ROOT}`/`DATA` filled
  in and also set in its `env`, and one needing `${user_config.…}` is
  skipped. A remote server never loads on a host. Everything skipped —
  remote, unresolved, the account's and the workspace's own servers, by name
  only — goes in the session's `mcp-notice.txt`, which `harness-session`
  prints on a start or a resume, and in the first `running` report.
  `runtime.toml` `plugin_mcp = "off"` keeps every plugin server out.
- **A plugin's remote server reaches a session only as an integration, and a
  host reports it without a secret** (131g, 131h). `capabilities`
  `plugin_mcp_remote` (refreshed in the background, so `hello` never waits
  on `claude plugin list`) carries the URL as declared and at most a
  header's *name* (`auth_hint`), never a header or env value or a vendor's
  client id; the server keeps it with the host's in-memory capabilities and
  `GET /v1/integrations/suggestions` asks every live host for a fresh one
  first. A connection added from one records `connection_plugin_origin` at
  creation only, and its grant carries it so the `claude-code` writer names
  that bridge `plugin_<plugin>_<server>` (the slug on a clash; the bridge is
  still addressed by slug). The first `running` report's `mcp_skipped` is
  kept, bounded, in `agent.db` `session_mcp_notice` and never replaced.
  `gateway_e2e.py` plants a header canary in a plugin and scans for it.
- **OAuth discovery never reaches a private address** (81g, §10). rmcp's
  OAuth code gets one HTTP client, `gateway::oauth::SsrfHttp`: https only,
  every URL and redirect hop resolved and refused unless all its addresses
  are public, then connected to exactly those addresses.
- **A pasted or confidentially registered OAuth client's secret is sealed
  like a credential** (131e, 131f; `oauth_clients`, kind `client_secret`).
  With no scopes given the server asks for MCP's (rmcp `select_scopes`).
  An authorization server that does not offer `none` gets a confidential
  RFC 7591 registration of Storm's own (`oauth::register_confidential`,
  through `SsrfHttp`, no redirects), with the auth method rmcp's token client
  will use — `client_secret_basic` when offered, else `client_secret_post` —
  and a reply without a secret is a failure. A pasted client on Linux and
  Windows returns to the fixed loopback port `ownAppPort` (27181), since a
  hand-registered app lists its redirects.
- **An OAuth flow is single use by SQL, not by code order**: loading it
  claims it with one `UPDATE … WHERE used_at IS NULL`. The state is stored
  only as a hash; the redirect URI is loopback or `storm://oauth`, never LAN.
- **A refresh is single flight per connection, and a rotated pair is
  persisted before it is used**; a rejected refresh is `needs_reauth`, never
  retried in a loop.
- **The client's sign-in redirect is the spec's, per platform** (§10.4,
  81l): **`storm://oauth/callback` on Android and macOS, a loopback listener
  on Linux and Windows.** Never loopback everywhere: that is the design the
  spec rejected for Android, where the app is backgrounded during the login.
  Either way the client relays only `{state, code}` and never sees a token.
  - The loopback listener listens before it opens the browser, answers only
    its redirect path, and closes after one sign-in.
  - A `storm://oauth` link is buffered by the native side
    (`MainActivity.kt`, `AppDelegate.swift`) until Dart calls `takeLinks`.
  - A link no sign-in in this process is waiting for is an **orphan, relayed
    by the Integrations screen, never dropped**: Android may deliver it to a
    fresh instance, and the flow lives on the server.
  - The registrations are configuration that fails silently, and
    `test/oauth_scheme_registration_test.dart` guards them. Flutter's own
    deep linking is off on Android, so the link never becomes a page route.
  - The macOS Release app no longer grants `network.server`, and the
    entitlements test holds that.
  - PR CI never compiles the Android or macOS projects; push to
    `acceptance/*` and the acceptance workflow does. The Windows and Linux
    runners are built on every client change (decision 125).

  **"Can write to" is off by default and absent for `shell`.**
- **`gateway.db`'s schema is additive only** — `CREATE … IF NOT EXISTS`, never
  `DROP` or `ALTER`; a test reads the schema to enforce it.
- **An agent's listings never count against its session's budget**
  (AM-G11, 81m). `tools/list`, `prompts/list`, `resources/list` and
  `resources/templates/list` take only their connection's slot
  (`counts_against_session`). An agent CLI lists every server in parallel as
  it starts, and refusing that silently leaves an integration with no tools
  for the session. Execution keeps the full budget, and a limit hit is still
  refused, never queued (§12).
- **A new upstream tool is recorded, never enabled** (spec §9, 81k). Only
  the owner's *first* test turns tools on (G-D16). After that, any listing,
  the owner's or an agent's, records unknown names in `new_tools`, and the
  owner sees "N new tools — review" until they save the tool list. Nothing
  but that save changes the allowlist. `known_tools` is written only by
  `observe_tools` and `review_tools`, never by `update_connection`, so an
  older copy of the row cannot undo a review. Tool names are the upstream's:
  the client shows them through `displayToolName`.
- **An integration's host scope is enforced at launch and again on every
  call** (decision 121). No `connection_hosts` rows means every host; a scope
  whose hosts are all revoked reaches nothing and never widens back to all.
  The per-call check (`host_not_in_scope`) is what makes narrowing cut off a
  live session — grants alone are fixed at launch. `storm` is never scoped.
- **A synced Claude login reaches the CLI only as `CLAUDE_CODE_OAUTH_TOKEN`
  in its process environment** (decision 124, AM71; `apps/runtime/src/credential.rs`).
  Never argv, `--settings`, a file or a log: it is a `SecretOsString`, pulled
  per launch and dropped once the CLI is spawned. Its pins live in the one
  `claude-settings.json`, and an invalid `--settings` file is ignored by the
  CLI with only a warning, so every value is a string (`"apiKeyHelper": ""`,
  never `null`) and the file is re-read before launch. Any failure ends the
  start; a synced session never falls back to the host's own login.

From M26 (Windows — the vault's *Agent Runtime/Windows Runtime Host*, D16,
and *Installer/Windows Server*; decision 99):

- **The carrier is per platform, the ending policy is not.** `pty/` holds
  the PTY (Unix) and ConPTY (Windows) behind one interface; `cli.rs` decides
  how a session ends and never names a signal or a pid. A `cfg(windows)` in
  `cli.rs` is the regression.
- **A Windows session's process is created suspended, put in its Job
  Object, then resumed**, and the job is `KILL_ON_JOB_CLOSE` without
  `BREAKAWAY_OK`. Resuming before the assignment lets the agent start a
  child outside the job; nothing would then end it.
- **A `.cmd`/`.bat` CLI's arguments are refused, never escaped**, when they
  hold anything `cmd.exe` interprets (`platform::wincmd`). Escaping for a
  batch file is the BatBadBut vulnerability class.
- **Private files go through `platform::{create_private_dir,
  open_private, is_private}`**: modes on Unix, created with them (A2);
  protected DACLs on Windows. A raw `OpenOptionsExt::mode` outside
  `platform/` does not compile on Windows, and a plain `create` there
  inherits whatever its parent grants.
- **The bridge socket is `platform::ipc`**: a Unix socket, or on Windows a
  first-instance named pipe whose DACL is the host's account and SYSTEM.
- **`install.ps1` fails with `throw`, never `exit`**: run through `irm |
  iex`, `exit` closes the person's PowerShell window.
- **`storm-server` is a library with two binaries.** `storm-serverw` is the
  same program for the Windows GUI subsystem, which the logon task runs so no
  console opens; it logs only through `--log-file`.

From M28 (desktop clients — the vault's *Desktop Clients — decision 125*):

- **One Storm per user on a desktop** (125b). Two instances would open one
  drift cache and replay one outbox. Windows takes the named mutex
  `Local\dev.storm.storm` in `main.cpp` before the engine starts; Linux keeps
  GApplication's uniqueness on, so a second launch is an `activate` in the
  running process, which raises the window and never builds a second engine.
- **The data directory is named by the runner, so its names are frozen.**
  `path_provider` builds it from `Runner.rc`'s `CompanyName`/`ProductName` on
  Windows and from `APPLICATION_ID` on Linux; the drift cache, prefs and
  secure store live there. The runner tests guard all three.
- **A desktop package carries what the machine may lack.** The Windows zip
  has the MSVC runtime beside `storm.exe` (`tool/package_windows.sh` fails
  without it). The Linux packages are built on Ubuntu 22.04 and refused if
  the bundle needs a glibc newer than `STORM_MAX_GLIBC` (2.35): the runner
  and plugins link against the build machine's glibc, and a newer build
  does not start on Debian 12.
- **The installers never kill a running Storm**: `install.ps1` asks it to
  close (it saves as you type) and stops if it does not.

## Style

Match the surrounding code. Comments explain *why*, especially where something
looks odd — most non-obvious code here is guarding one of the invariants above,
and a future reader needs to know that before "simplifying" it.
