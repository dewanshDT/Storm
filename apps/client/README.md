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

## What works

- A dashboard of the server's vaults, over the notes you opened most recently
  across all of them
- A breadcrumb directory browser, with folders you can create, rename and
  delete
- Markdown editor with live styling and a keyboard formatting toolbar (see
  `lib/editor/`)
- Wikilink following and autocomplete
- Create, rename/move, delete — all of which work offline
- Full-text search with highlighted snippets
- Debounced autosave with merge/conflict handling
- Offline editing with an outbox that replays on reconnect
- Live updates pushed from other devices over a WebSocket
- Light/dark theme, font size, and the server's storage root

## Vaults

One server hosts many vaults, and the *route* says which one is open:
`/v/<vault-id>/browse/...`. `Settings.activeVault` mirrors it, persisted, and
`apiProvider` reads that — so switching vaults reuses the teardown that already
existed, disposing the engine and closing its socket.

`VaultGate` keeps the two in agreement. It refuses to build a vault's screens
until the route and the active vault match, because for one frame after
navigating the providers still hold the *previous* vault's notes.

The cache is keyed by `(vault, note)` throughout, and the sync cursor is per
vault — one shared cursor would have two vaults overwriting each other's
position, which surfaces as randomly missed changes rather than an error.

## Offline

`SyncEngine` (`lib/sync/sync_engine.dart`) owns the drift cache, the outbox and
the server connection. Nothing above it touches `StormApi` directly, so offline
behaviour lives in one place.

Online/offline is inferred from whether requests actually succeed rather than
from a connectivity plugin — a device can be on wifi with the homelab
unreachable, and only a real request distinguishes that. A socket failure is
queued and retried; an HTTP refusal never is, since retrying a request the
server already rejected would wedge the queue behind it.

The status bar shows `Offline` or `N unsent` whenever the server does not have
everything, because an edit that exists only in the outbox is one the user
needs to know about before closing the app.

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
├── cache/        drift cache, outbox, recents
├── state/        Riverpod providers + NoteSession
├── sync/         SyncEngine — one per active vault
└── ui/           connect, dashboard, browser, editor, search, server settings
```

`lib/editor/` began as the M0 spike. See `docs/editor-findings.md` for its
performance characteristics, the on-device measurements, and why the buffer has
to match what is rendered character for character.

## Tests

```sh
flutter test              # 326 tests, no server needed
flutter test test_live/   # 19 tests against a real storm-server
```

Or from the repo root, which starts and stops the server for you:

```sh
make test-live
```

`test_live/` sits outside `test/` deliberately, so a plain `flutter test` never
depends on a running server.

The unit tests mock HTTP, which proves the client behaves correctly against the
responses we *think* the server sends. The live tests run the real `StormApi`
against a real `storm-server`, which proves it against the ones it actually
sends — that gap is where a field-name or semantics mismatch would otherwise
hide until runtime. Start a server first:

```sh
cd ../server   # apps/server
cargo run -- --vault /tmp/v --state /tmp/s --token testtoken --port 8484
```

## Deploying the web client

The Rust server serves the built bundle, so the homelab runs one thing:

```sh
make serve-web            # from the repo root
```

## Known issues

- The token is stored in plain `shared_preferences`. Acceptable only while the
  server is LAN-only; `flutter_secure_storage` is a prerequisite for anything
  wider.
- Search, tags and backlinks are per vault. Each vault has its own index on the
  server, and a merged search is a separate feature.
- Only one vault is live at a time. Opening a second replaces the first rather
  than running both.
