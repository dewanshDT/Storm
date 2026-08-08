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

`docs/prd.md` is the original brief and is **not** maintained.
Where it and `PLAN.md` disagree, `PLAN.md` is current.

## Layout

| Path | What |
|---|---|
| `apps/server/` | Rust sync server (axum + rusqlite). See `apps/server/README.md`. |
| `apps/client/` | Flutter app — macOS, Linux, Android, web. See `apps/client/README.md`. |
| `deploy/` | systemd units, `storm.env` template, nightly backup script. See `deploy/README.md`. |
| `docs/prd.md` | Original brief. Superseded by `PLAN.md`; not maintained. |
| `docs/editor-findings.md` | Why the editor is built the way it is, with measurements. |
| `docs/storm-ui-refactor.md` | M7/M8 design brief — dashboard, nav bubble, toolbar. |
| `docs/storm-multi-vault.md` | M9/M10 design brief — vaults, folders, storage root. |
| `docs/storm-properties.md` | M11 design brief — typed frontmatter properties. |
| `docs/storm-adaptive.md` | M12 design brief — the wide-screen layout. |

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

## Style

Match the surrounding code. Comments explain *why*, especially where something
looks odd — most non-obvious code here is guarding one of the invariants above,
and a future reader needs to know that before "simplifying" it.
