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

`homelab-notes-app-PRD.md` is the original brief and is **not** maintained.
Where it and `PLAN.md` disagree, `PLAN.md` is current.

## Layout

| Path | What |
|---|---|
| `server/` | Rust sync server (axum + rusqlite). See `server/README.md`. |
| `client/` | Flutter app — macOS, Linux, Android, web. See `client/README.md`. |
| `spike/editor_spike/` | M0 throwaway. `FINDINGS.md` has the editor perf data. |

## Commands

```sh
# Server
cd server && cargo test && cargo clippy --all-targets
cargo run -- --vault /tmp/v --state /tmp/s --token testtoken --port 8484
cargo run -- --vault <real-vault> --state <state> --dry-run   # always first

# Client
cd client && flutter analyze && flutter test
flutter test test_live/      # needs a running server
flutter run -d chrome        # macOS is currently blocked, see PLAN.md
```

Both suites must be clean before a change is done. `cargo clippy` is expected to
emit zero warnings.

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
