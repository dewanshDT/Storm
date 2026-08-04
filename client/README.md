# storm (client)

The Flutter client. One Dart codebase for macOS, Linux, Android and web.

Pure Dart — there is no Rust in the client. The PRD proposed a shared Rust core
so client and server could share merge logic, but in the server-of-record model
merging only ever happens server-side: the client sends `base_version` and the
server resolves. That left the client needing HTTP, a cache and an outbox, all
of which Dart does natively, so `flutter_rust_bridge` is off the critical path
entirely.

## Running it

```sh
flutter run -d macos        # or: -d linux, -d chrome
```

On first launch you're asked for the server address and token. The connection is
verified before it's saved, so a typo surfaces immediately rather than as an
empty vault later.

## What M2 covers

Online-only: every read and write goes straight to the server. There is no local
cache and no outbox — those are M3, and they layer *above* `StormApi` rather
than inside it.

- Vault tree with folders derived from note paths
- Markdown editor with live styling (see `lib/editor/`)
- Create, rename/move, delete
- Full-text search with highlighted snippets
- Debounced autosave with merge/conflict handling
- Light/dark theme, font size

## The save protocol

This is the part worth understanding, because it's where an edit can silently
disappear. `NoteSession` (`lib/state/note_session.dart`) owns it:

1. Every save carries the `base_version` the buffer was edited from.
2. If the response comes back `merged` or `conflict`, the server reconciled
   against a version this client never saw. The session **adopts the server's
   text** and bumps a revision counter; the editor reloads its controller.
   Keeping our own text here would make the next save race a version we don't
   have.
3. If the user typed while a save was in flight, the session stays `dirty`
   rather than claiming `saved`, so the newer keystrokes aren't lost.
4. A failed save also stays `dirty`. The edit exists only in this buffer, and
   marking it `failed` alone would invite losing it.

Conflicts arrive as text with git-style markers, with the client's own edit on
the `ours` side. A banner explains it; you resolve by deleting the markers.

## Layout

```
lib/
├── api/          StormApi (thin REST transport) + wire models
├── editor/       markdown tokenizer, theme, StormMarkdownController
├── state/        Riverpod providers + NoteSession
└── ui/           connect screen, vault tree, editor, search
```

`lib/editor/` graduated from `spike/editor_spike/` unchanged. See that
directory's `FINDINGS.md` for the performance characteristics and the reason
syntax markers are dimmed rather than hidden.

## Tests

```sh
flutter test          # 76 tests, no server needed
flutter test test_live/   # 12 tests against a real storm-server
```

`test_live/` sits outside `test/` deliberately, so a plain `flutter test` never
depends on a running server.

The unit tests mock HTTP, which proves the client behaves correctly against the
responses we *think* the server sends. The live tests run the real `StormApi`
against a real `storm-server`, which proves it against the ones it actually
sends — that gap is where a field-name or semantics mismatch would otherwise
hide until runtime. Start a server first:

```sh
cd ../server
cargo run -- --vault /tmp/v --state /tmp/s --token testtoken --port 8484
```

## Deploying the web client

The Rust server serves the built bundle, so the homelab runs one thing:

```sh
flutter build web --release
cd ../server && cargo run --release -- ... --web ../client/build/web
```

## Known issues

- **macOS builds are blocked on this machine.** Xcode 26.6 can't load
  `IDESimulatorFoundation` because
  `/Library/Developer/PrivateFrameworks/DVTDownloads.framework` is still v17.0
  from an older Xcode. Not a Flutter or Storm problem. Fix with
  `sudo xcodebuild -runFirstLaunch`. Web, Linux and `flutter test` are
  unaffected.
- **Android is untried** — no SDK installed yet.
- The token is stored in plain `shared_preferences`. Acceptable only while the
  server is LAN-only; `flutter_secure_storage` is a prerequisite for anything
  wider.
