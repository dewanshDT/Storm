# Storm

Self-hosted markdown notes: Flutter clients + a Rust sync server in the homelab,
replacing Obsidian + Syncthing.

## Read PLAN.md first

**`PLAN.md` is the living plan and status.** Read it before starting work in
this repo, and **update it as part of the change** — not afterwards:

- A milestone changing state, a new blocker, or a finding that would have
  changed an earlier decision all belong in `PLAN.md` in the same change that
  produced them.
- Its **Decision log** records settled choices and what would justify revisiting
  each. Don't relitigate them without a reason; if one does change, amend the
  entry with why.
- Leaving `PLAN.md` stale is a defect. A finding that exists only in a chat
  transcript is lost.

**Also keep `Storm/Active Work.md` current** (personal vault, via Storm MCP) —
the short in-flight checklist. Read it when starting multi-step work; update it
when an item moves. `.cursor/rules/storm-active-work.mdc` requires this.
`PLAN.md` still wins for decisions; Active Work is the queue.

**`Storm/Global Todo.md` is the one aggregate tracking surface** — every
milestone, the Agent Runtime phases, in-flight items, backlog and parking lot
in a single checklist. `PLAN.md` and Active Work stay authoritative for
decisions and the queue; Global Todo mirrors their status. When a milestone,
phase, or queue item changes state, update Global Todo in the same change.

**Every vault note an agent edits carries a `summary:` in its frontmatter.**
Add it when creating a note; refresh it whenever you update one — one
plain-text line capturing what the note is (for a change, what it now is).
It is what search indexes and what a later agent skims first; a note whose
summary still says "proposed" long after it shipped is a note that lied.

`docs/prd.md` is the original brief and is **not** maintained.
Where it and `PLAN.md` disagree, `PLAN.md` is current.

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
- **`PLAN.md`'s decision log is numbered, and `staging` may already carry
  entries `main` has never seen.** Read `PLAN.md` **on `staging`** before
  choosing a number. A gap is harmless; a duplicate is not.
- When a release does happen, `main` gets the merge and the tag together — see
  `PLAN.md`'s release entries for how versioning works.

## Layout

| Path | What |
|---|---|
| `apps/server/` | Rust sync server (axum + rusqlite). See `apps/server/README.md`. |
| `apps/relay/` | `storm-relay` — the SRP v1 relay. Standalone crate; **no workspace** (R6). |
| `apps/server/src/gateway/` | The MCP Gateway's store, data key and upstream client (M21, decision 81). Policy stays in `ops.rs`. |
| `apps/runtime/` | `storm-runtime` — the Agent Runtime's Runtime Host (M20, decision 77). Standalone crate; never depends on `apps/server`. |
| `apps/client/` | Flutter app — macOS, Linux, Android, web. See `apps/client/README.md`. |
| `deploy/` | systemd units, `storm.env` template, nightly backup script. See `deploy/README.md`. |
| `docs/srp-v1.md` | The relay wire spec, normative. `docs/srp-vectors.json` is its shared test vectors. |
| *Storm Relay Dart Client* (vault) | The client half's accepted architecture. Read before writing any Dart SRP code. |
| *Agent Runtime/V1 Specification Freeze* (vault) | What Agent Runtime V1 builds. Frozen; read it, then `PLAN.md` decision 77, before touching `apps/runtime` or `/v1/agent/*`. |
| `docs/prd.md` | Original brief. Superseded by `PLAN.md`; not maintained. |
| `docs/editor-findings.md` | Why the editor is built the way it is, with measurements. |
| `docs/storm-ui-refactor.md` | M7/M8 design brief — dashboard, nav bubble, toolbar. |
| `docs/storm-multi-vault.md` | M9/M10 design brief — vaults, folders, storage root. |
| `docs/storm-properties.md` | M11 design brief — typed frontmatter properties. |
| `docs/storm-adaptive.md` | M12 design brief — the wide-screen layout. |
| `docs/storm-ui.md` | What every screen does today, for designing against. |
| `docs/design_handoff_storm_design_system/` | M14 design system + prototype. `README.md` is the brief; the two `.dc.html` files open in a browser. |

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
  `lib/editor/frontmatter_edit.dart` on the client.
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
- **Notes are tracked by UUID, not path.** Renames and moves are metadata
  updates.

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
- **The first account is an owner, and the last *active* owner cannot be
  deleted, disabled or demoted.** Disabled owners do not count: an account that
  cannot log in cannot administer, so leaving only disabled ones is the same
  lockout as leaving none. SQLite cannot express either rule.
- **Usernames are ASCII and unique by casefold.** Uniqueness is decided on the
  fold, so the fold must be unambiguous — Unicode brings locale-dependent case
  rules and homoglyphs, and two visually identical usernames as separate rows
  is a security bug. `display_name` is unrestricted.
- **No `--password` flag, in any command, ever.** A password in an argument is
  in the shell history and in `ps` for every other user on the box. Prompt
  without echo, or read `--password-stdin`.
- **`security_events` never contains a secret** — not a password, not a hash,
  not a token. A test asserts it for every administrative act.

From the relay (SRP v1 — **`docs/srp-v1.md` is the normative wire spec and
lives in this repo**; the design rationale stays in the personal vault under
`Storm/Remote/`, and ADRs R1–R13 in *Storm Remote Decisions*):

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
  into a remote off-switch. The peer address comes from the accepted socket,
  never from a header.
- **A shutdown may cancel the phases before registration, never the serving
  phase.** Connect and registration have announced nothing, so dropping them
  costs nothing. `serve_trunk` owes the relay a `DEREGISTER` — cancelling it
  leaves the relay holding the `server_id` until a heartbeat timeout, which is
  the outage `DEREGISTER` exists to prevent.
- **The wire format is re-derived in three places and `docs/srp-vectors.json`
  is what makes them agree.** `relay_auth_message`, `validate_nonce`,
  `validate_server_id` and the base64url rules exist in `apps/server`,
  `apps/relay` and `apps/client`, which cannot depend on one another. Nothing
  in any build makes them agree, and drift surfaces at runtime as
  `auth_failed` — identical to a genuine attack, an expired nonce and a
  refused binding. All three test suites read the one vector file; regenerate
  it with `tools/srp-vectors/generate.py`, and treat a changed vector as a
  protocol change.
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
- **The two signing domains must never coincide.**
  `storm-relay-auth:v1:<server_id>:<nonce>` proves the right to register at a
  relay; `storm-challenge:v1:<server_id>:<nonce>` proves identity to a client.
  Both fields are validated, not just the nonce — they are two colon-delimited
  halves of one signed string, so a colon in *either* re-splits it.

From M20 (Agent Runtime V1 — the spec is the vault's *Agent Runtime/V1
Specification Freeze*; the slices and slice 1's decisions are `PLAN.md`
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
Specification*; the slices are `PLAN.md` decisions 81 and 81a onward):

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
- **Integrations are the owner's alone, on the session tier** (81c). Every
  `/v1/integrations/*` operation calls `ops::require_integration_owner`, and
  a connection is found only among the caller's own rows (`404` otherwise).
  No MCP tool manages one, so an `stk_` key never can.
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
- **`gateway.db`'s schema is additive only** — `CREATE … IF NOT EXISTS`, never
  `DROP` or `ALTER`; a test reads the schema to enforce it.

## Style

Match the surrounding code. Comments explain *why*, especially where something
looks odd — most non-obvious code here is guarding one of the invariants above,
and a future reader needs to know that before "simplifying" it.
