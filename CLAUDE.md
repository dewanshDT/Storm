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
| `docs/prd.md` | Original brief. Superseded by `PLAN.md`; not maintained. |
| `docs/editor-findings.md` | Why the editor is built the way it is, with measurements. |

Read `docs/editor-findings.md` before changing anything in
`apps/client/lib/editor/`. It records the constraint the whole editor rests on
— the rendered span tree must match the buffer character for character — and
the on-device numbers that say where the limits are.

## Commands

Use the Makefile — it encodes the cross-toolchain steps, and `test-live` starts
and tears down a server around the integration suites.

```sh
make help                    # every target
make check                   # clippy + analyze + both unit suites
make test-live               # integration suites against a real server
make fmt                     # cargo fmt + dart format (CI enforces both)
make server VAULT=~/vault    # run the sync server
make dry-run VAULT=~/vault   # ALWAYS do this before importing a real vault
make serve-web               # build the web client and serve it
```

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

## Invariants worth knowing before editing

These are load-bearing and each has a regression test. Breaking one loses user
data quietly.

- **The vault is plain markdown, always.** Storm's own state lives in a sibling
  `state/` directory, never inside the vault.
- **Frontmatter is never serialized.** Storm rewrites individual *lines* and
  passes every other byte through. Running a user's YAML through a serializer
  reorders keys and drops comments, dirtying the whole vault.
- **The server owns `modified:`.** Clients must not write it, and it is
  normalised out of all three sides before a merge — otherwise every concurrent
  write conflicts on that line.
- **The editor's span tree must flatten back to the buffer exactly.** A gap or
  overlap silently corrupts rendered text and every caret offset after it.
- **A `merged` or `conflict` response means the client adopts the server's
  text.** Keeping local text makes the next save race a version it never had.
- **Notes are tracked by UUID, not path.** Renames and moves are metadata
  updates.

## Style

Match the surrounding code. Comments explain *why*, especially where something
looks odd — most non-obvious code here is guarding one of the invariants above,
and a future reader needs to know that before "simplifying" it.
