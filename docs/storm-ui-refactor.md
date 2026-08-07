# Storm UI refactor — implementation doc (v0.1)

Scope: replace the sidebar-drawer navigation with a dashboard home screen, a floating bottom nav bubble, a breadcrumb-driven directory browser, a keyboard formatting toolbar, and refined wikilink/image interactions. This is a UI-layer pass only — no changes to sync, cache, outbox, or the server.

---

## 1. Design direction (recap)

- Dark, low-chrome theme; indigo accent (from the existing raindrop mark), amber reserved for tags/highlights.
- Serif (`font-voice`-equivalent) for note body text, sans for all chrome.
- Navigation moves off a side drawer entirely — everything lives in a bottom-anchored bubble plus a dashboard home screen.

## 2. Components

### 2.1 Dashboard (home screen)
- Header row: vault-status bubble (left), app name (center), profile/settings bubble (right).
- Stat row: note count, last-synced time.
- Recent notes list: title, one metadata line (snippet or tags), relative time.
- "Recent" defined as recently *edited* for v1 — not recently opened — to avoid tracking a second timestamp for a low-effort screen.

### 2.2 Corner bubbles
- **Vault bubble (top-left):** rounded-square, shows current vault initial + a small status dot (green = synced, amber = syncing, gray = offline). Tap opens a sheet with vault name, server address, last sync. Not a placeholder — it's the connection-status affordance today. Becomes a vault switcher when multi-vault ships; same slot, same visual weight.
- **Profile bubble (top-right):** rounded-square, user initial. Tap opens settings: theme, sync status/storage used, about. Becomes account/user management when multi-user ships; same slot.

### 2.3 Floating navigation bubble
- Bottom-anchored, tap-to-expand (not long-press) for consistency with the app's one interaction rule: **tap = primary action, long-press = secondary options**, applied everywhere (see 2.5).
- Exactly 4 slots, collapsed to a single icon by default: **Directory, Search, New note, Context** (tags/mentions).
- The **Context** slot is dynamic: shows a global tag browser at the vault root, and the open note's tags + linked mentions when a note is open.
- Icons can carry a small badge (e.g. sync-pending dot) — reuse the same dot/badge visual language as the vault-status dot for consistency.
- **Hidden entirely while the keyboard is open** (detected via `MediaQuery.viewInsets.bottom` — no app-state plumbing needed); replaced by the formatting toolbar during that time.

### 2.4 Directory browser
- Breadcrumb drill-down, not an indented tree: tapping a folder transitions the whole screen to that folder's contents, with a breadcrumb trail at top to go back up.
- Chosen over a tree specifically because trees compress badly at phone width; breadcrumbs stay readable at any depth.

### 2.5 Wikilink and inline-image interaction
Interaction rule applied consistently: **tap/click = primary action, long-press = secondary options bubble.**

- **Touch:** tap a wikilink → navigate to the note. Long-press → bubble with Edit link / Copy / Open in new pane (future).
- **Pointer:** click → position cursor for editing (standard text-editor behavior). Ctrl/Cmd+click → navigate. Matches VS Code / Obsidian conventions.
- **Inline images, v1 (interim, not full inline rendering):** image references render as a tappable chip/thumbnail in the text flow — not truly WYSIWYG-positioned. Tap (touch) or click (pointer) opens a full-size preview; same touch-vs-pointer split as wikilinks otherwise. **True inline rendering (image sitting exactly at its position in the flow) is out of scope for this pass** — it requires the block-based editor rewrite the original build deferred, because the current editor's core invariant is "buffer equals what's rendered, character for character," which an inline image breaks by definition. That rewrite gets its own future milestone.

### 2.6 Formatting toolbar (above keyboard)
- One scrollable row, shown only while the keyboard is open (replacing the nav bubble in that state).
- Buttons: heading (opens H1/H2/H3/paragraph picker, not a blind cycle), bold, italic, code, bullet/numbered list toggle, checklist, wikilink (inserts `[[` and lets existing autocomplete take over — reuses the same trigger path autocomplete already uses, no new logic), quote.
- **Every button must call the existing `TextEditingController`'s own insertion/formatting methods** — never manipulate the text buffer through a separate code path. That controller's caret-offset mapping is the single most load-bearing invariant in the editor; a toolbar that bypasses it is the most likely way this refactor reintroduces a data-corruption-class bug.

## 3. Migration path

- Additive only: the new nav shell (dashboard, bubble, directory, toolbar) is a new presentation layer reading from the **same** providers/notifiers the old drawer already reads from (file tree state, search, sync status). No changes to `state/`, `sync/`, or `api/`.
- Build on a branch with the old drawer still live; reach feature parity; flip the default entry point; delete the drawer code last. No half-migrated state where both nav systems can drift against each other.
- Add integration tests for the new nav flows (bubble expand/collapse, drill-down + back, note open/close, toolbar-to-editor calls) *before* wiring them to real state — the postmortem's own lesson: the last build's worst bugs were UI wiring with zero test coverage sitting on top of an exhaustively tested sync layer.

## 4. State and animation architecture

- Bubble expand/collapse and screen-transition state are **ephemeral UI state**, kept local to the nav widget (a small `ValueNotifier`/controller) — never pushed into the global providers that drive sync/cache. This directly avoids repeating the earlier bug where a `ChangeNotifierProvider` rebuilt on every notification.
- The Context slot's content (tags vs. linked-mentions) derives from the current route/screen — one source of truth for "where am I," not a separately maintained flag.
- Keyboard visibility drives bubble-hide / toolbar-show via `MediaQuery`, no app-state involvement.
- Animations: plain Flutter implicit animations (`AnimatedContainer`, `AnimatedAlign`, `AnimatedOpacity`) — no animation package dependency, consistent with the project's minimal-dependency approach elsewhere (e.g. bare binary instead of Docker).

## 5. Non-goals for this pass

- True inline image rendering (needs the block-editor rewrite — separate future milestone).
- True syntax hiding beyond what the current text-buffer editor already does.
- Multi-vault switching and multi-user accounts — the corner bubbles are built to *become* these later, not to implement them now.
- Split-pane / multi-note views ("open in new pane" referenced above is a placeholder menu item only).

## 6. Open questions

- Badge design: which states get a badge (sync-pending, offline, index-rebuilding) without turning the bubble into a notification center.
- Exact contents of the long-press action bubble for wikilinks/images beyond Edit link / Copy.
- Whether "Recent" on the dashboard should later become configurable (edited vs. opened) once real usage patterns are visible.
