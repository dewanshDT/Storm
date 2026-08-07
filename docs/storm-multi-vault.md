# Storm multi-vault — implementation doc (v0.1)

Scope: several vaults instead of one, a dashboard rebuilt around them, folder creation as a real operation, and a server-side vault storage root that can be changed from the app. This cuts through every layer — server routing and state, the wire format, the client cache schema, and the shell — which is what makes it different from the UI-layer pass in `docs/storm-ui-refactor.md`.

Status: **built.** M9/M10 in `PLAN.md`, which records the two bugs the new tests caught. Decisions 20–25 there hold the rationale and what would justify revisiting each; this doc is the shape.

---

## 1. What is changing, and why now

`docs/prd.md` §3 put multiple vaults out of scope for v1 ("one vault per app install initially"), and `docs/storm-ui-refactor.md` §2.2 pre-committed the top-left bubble slot to "become a vault switcher when multi-vault ships; same slot, same visual weight". That slot is now being cashed in.

Three requests, one design:

1. **Multiple vaults**, with the dashboard as a two-column grid of vault cards over a full-width "recently opened" list showing note name and vault.
2. **Folder creation.** Today a folder can only appear as a side effect of creating a note inside it — there is no folder record, no endpoint, and the server actively deletes directories that become empty.
3. **A configurable vault storage root**, settable from server settings in the app.

The topology is **one server hosting many vaults** under a storage root, rather than one server process per vault. That is the only shape in which a "vault storage root" setting refers to anything real.

## 2. Storage layout

```
/srv/storm/vaults/                      ← STORM_VAULT_ROOT
├── personal/
│   └── Daily/2026-08-07.md
├── work/
└── recipes/

/srv/storm/state/
├── vaults.json                         registry: root + [{id, name, dir, created}]
├── 7f3a…/index.db                      one index per vault
├── b21c…/index.db
└── 9e04…/index.db
```

A vault is identified by a **UUID**, not its directory name, for the same reason notes are tracked by UUID and not path: renaming must not orphan the index or every client's cached notes. `vaults.json` is plain JSON rather than another SQLite database because it is the one piece of state a human might need to repair by hand, and the vault-is-greppable ethos should reach it.

`state/` remains a sibling of the vault data, never inside it.

## 3. Server

### 3.1 Registry (`src/registry.rs`, new)

```rust
pub struct Registry { pub root: PathBuf, pub vaults: Vec<VaultEntry> }
pub struct VaultEntry { pub id: String, pub name: String, pub dir: String, pub created: String }
```

Saved with the same atomic temp-write-plus-rename `Vault::write` already uses.

- `scan_root()` adopts any unregistered directory under the root, and marks entries whose directory is gone as `missing` rather than dropping them. **The registry never deletes or moves files.**
- It runs at exactly three moments: startup, a successful root change, and vault create/delete through the API. It is **not** reactive — a directory dropped into the root over rsync does not appear until a restart. The root watcher covers note edits inside registered vaults; it does not register new vaults.
- It considers directories only, skipping `state_dir` and every dot-prefixed entry. Loose files directly under the root — a stray `README.md`, a `.DS_Store` — are ignored rather than half-registered.
- **Legacy migration, automatic and once:** if `state/index.db` exists and the registry has exactly one vault, move it to `state/<id>/index.db` and log it loudly. `note_versions` is the merge base and cannot be rebuilt from markdown, so it must be moved rather than regenerated.

### 3.2 State and locking

`AppState` goes from `{ indexer: Mutex<Indexer>, events, token }` to a `RwLock<VaultSet>` holding the root and a map of id → `Arc<VaultHandle>`, each handle owning its own `Mutex<Indexer>`. Today's locking granularity is preserved per vault, and two vaults can work concurrently. `VaultSet::reload(new_root)` rebuilds the map wholesale; that is what a root change calls.

### 3.3 One watcher over the root

Today `watcher::spawn` is called once with a single vault root and has no stop path. Rather than one watcher per vault plus shutdown plumbing, watch the **storage root** recursively and attribute each event to a vault by directory prefix. Adding or removing a vault then needs no watcher work at all, and a root change respawns the single watcher. An event that resolves to no registered vault is dropped, not an error.

### 3.4 Schema

Two additions per vault index, both idempotent `CREATE TABLE IF NOT EXISTS` and so needing no migration mechanism:

```sql
CREATE TABLE IF NOT EXISTS folders (
    path TEXT PRIMARY KEY,          -- vault-relative, no trailing slash
    created TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS note_access (
    note_id TEXT PRIMARY KEY,
    opened_at TEXT NOT NULL
);
```

`note_access` is a separate table rather than a column on `notes` because `record_note` rewrites the notes row on every index update and would clobber it — and an "opened" timestamp must not bump `version` or append to `change_log`.

A `PRAGMA user_version` guard lands alongside them, so the next change needing an *altered* column has somewhere to live. Additions never did.

### 3.5 Routes

| Method + path | Note |
|---|---|
| `GET /v1/health` | unchanged, unauthenticated, vault-less |
| `GET /v1/vaults` | `[{id, name, dir, note_count, missing, last_modified}]` |
| `POST /v1/vaults` | `{name}` → creates the directory under root, registers it |
| `PATCH /v1/vaults/{id}` | `{name}` → display name only |
| `DELETE /v1/vaults/{id}` | **unregisters; never deletes files** |
| `GET /v1/config` | `{vault_root, state_dir, vault_count, note_count}` |
| `PUT /v1/config` | `{vault_root, orphan_ok?}` — see §3.6 |
| `GET /v1/recents?limit=` | cross-vault, merged, `opened_at` desc |
| `GET /v1/vaults/{v}/…` | every existing note, sync, search, tag and attachment route, prefixed |
| `POST /v1/vaults/{v}/notes/{id}/opened` | fire-and-forget, writes `note_access` |
| `POST /v1/vaults/{v}/folders` | `{path}` |
| `DELETE /v1/vaults/{v}/folders/{*path}` | refused unless empty |
| `POST /v1/vaults/{v}/folders/rename` | `{from, to}` |
| `WS /v1/stream` | stays **global**; `Change` gains `vault_id` |

`/v1/sync` becomes `/v1/vaults/{v}/sync` because `change_log.seq` is per vault. The stream deliberately does not split: one socket carries every vault's changes and the client filters by `vault_id`, rather than holding a socket open per vault nobody is looking at.

A `vault(id)` extractor resolves the handle once — 404 for unknown, 409 for `missing` — so no handler repeats it.

**This breaks the wire format.** Client and server deploy together.

### 3.6 Changing the storage root

**The server never moves files.** Changing the root points Storm at directories an admin has already put there. The UI says so verbatim: *"This does not move your notes. Move the vault directories first, then change the root."*

The dangerous case is obvious once stated: point the root at an empty directory and every vault becomes `missing` — files safe on disk, invisible to every client. Nothing is lost, but a server that boots perfectly healthy with zero vaults reads as "my notes are gone", which is exactly the class of silent failure this project has been burned by twice. So the endpoint is not a plain setter:

1. **Validate** — absolute, exists, is a directory, writable, and not inside `state_dir`. (The other direction is legal; `scan_root()` skips `state_dir`. See §5.)
2. **Dry-run** the new root and report which registered vaults are found there, which would be orphaned, and which unregistered directories would be adopted.
3. **Refuse with `409`** if none of the currently-registered vaults are found under the new root, returning that report as the body — unless the caller passes `orphan_ok: true`. A first-run server with nothing registered is not refused; there is nothing to orphan.
4. Only then write `vaults.json`, reload, and respawn the watcher.

The client shows the step-2 report as a confirmation dialog and only sends `orphan_ok` once the user has read the list. Symmetrically, a `missing` vault is never quietly dropped: it stays in the registry, renders greyed on the dashboard, and returns `409` on writes.

systemd's `ReadWritePaths=/srv/storm` confines this in practice — a root outside it fails at step 1 with a permission error. Widening the root beyond `/srv/storm` means widening that line too.

### 3.7 Folders

`prune_empty_parents` walks upward removing empty directories after a delete or move. It gains a check against the `folders` table and stops at any recorded folder. **That exemption is the entire reason the table exists** — without it a newly created empty folder vanishes the moment its last note leaves.

`TreeResponse.folders`, today derived from note path prefixes, becomes the union of derived and recorded folders.

Rename renames the directory via the existing `Vault::rename` and rewrites every note whose path carries the old prefix, in one transaction, emitting one `moved` change per note.

### 3.8 `--vault-root` replaces `--vault`

`--vault` points **directly at a vault's markdown directory**. Reinterpreting that same path as a root would make the server scan *inside* the existing vault for sub-vaults — registering `Daily/` and `Projects/` as vaults and finding no notes at the top level. So the shim is explicit:

- `--vault-root` (default `./vaults`, env `STORM_VAULT_ROOT`) is the new flag.
- If `--vault-root` is unset and `--vault` is set, the root becomes the **parent** of the `--vault` path and that one child is registered as a vault named after it. `--vault /srv/storm/vault` ⇒ root `/srv/storm`, one vault `vault`. Logged at `warn` with both paths.
- Refuse to boot, loudly and non-zero with both paths in the message, if that shape does not hold. A skipped `mv` in the runbook must read as a clear failure, not as a vault that disappeared.
- Both flags set is an error, not a precedence rule.
- The shim lives for one release, then goes.

## 4. Client

### 4.1 Transport

`apiProvider` stays singular — `StormApi` is a transport for one *server*, and the vault is a parameter of each call. New models: `VaultInfo`, `RecentNote` (`vaultId`, `vaultName`, `noteId`, `path`, `title`, `openedAt`, `modified`), `ServerConfig`. `Change` gains `vaultId`.

### 4.2 Active vault

`Settings` gains `activeVault`, persisted. `apiProvider` already rebuilds on any settings change and `syncEngineProvider` already watches it, so engine teardown, socket close and session rebuild all come from machinery that exists and is tested — no provider families.

The hazard traded for is a stale frame: the route says one vault while the providers still hold another. A **`VaultGate`** at the `/v/:vault` route closes it by refusing to build children until route and settings agree, the same post-frame reconciliation `NoteScreen._load` already does. The route is the source of truth; the setting is a persisted mirror that also gives "reopen the last vault" for free.

### 4.3 Routes

```
/                          dashboard — vault grid + recents
/v/:vault/browse[/path]    directory browser
/v/:vault/note/:id
/v/:vault/search  /tags
/settings/server           storage root + vault management
/connect                   unchanged
```

`/v/:vault/…` nests under the dashboard route exactly as browse/note/search do today, so decision 17 holds: the dashboard sits beneath every location, going deeper `push`es, and the nav bubble still `go`s.

### 4.4 Cache

`cache_db.dart` is `schemaVersion => 1` with **no `MigrationStrategy` at all** — the default only ever runs `createAll`. Writing one is a prerequisite, and it is the first real migration this project has had.

- `CachedNotes` and `Outbox` gain `vaultId`; primary keys become `{vaultId, id}` and `{vaultId, noteId}`.
- `Meta`'s single `lastSeq` key becomes `lastSeq:<vaultId>`. One shared cursor would have two vaults overwriting each other's position, surfacing as randomly missed changes rather than as an error.
- A new `Recents` table mirrors the server's list so the dashboard renders offline, written optimistically on open.
- **v1 → v2:** existing rows are stamped `vaultId = 'legacy'`; on the first successful `GET /v1/vaults`, if there is exactly one vault, rewrite `'legacy'` to its id. On upgrade there will be exactly one, because the server migrates the old single vault into a single registered one. If there is not, **leave the rows alone and surface a notice** — `Outbox` rows are edits that exist nowhere else, and discarding them silently is data loss.

### 4.5 Dashboard

```
┌──────────────┐ ┌──────────────┐
│ P            │ │ W            │   two-column grid of vault cards
│ Personal     │ │ Work         │   initial, name, note count,
│ 214 notes ●  │ │ 88 notes  ●  │   status dot, relative last-synced
└──────────────┘ └──────────────┘

Recently opened
┌───────────────────────────────────────┐   full-width cards
│ Design notes                   2h ago │   note title
│ Storm · Projects/Storm                │   vault · folder
└───────────────────────────────────────┘
```

Vault cards come from `GET /v1/vaults`; a `missing` vault renders greyed with "directory not found" rather than disappearing. Recents come from `GET /v1/recents`, falling back to the cache table offline — the same server-then-cache shape `SyncEngine.tree()` already uses. Tapping a card pushes `/v/:id/browse`.

"Recently opened" replaces §2.1 of the UI-refactor doc, which defined Recent as recently *edited* specifically to avoid a second timestamp. Multi-vault broke that bargain: sorting by `modified` across vaults means fetching every vault's tree on every load of the home screen.

### 4.6 Vault switcher, folders, server settings

The vault bubble finally becomes what its own doc comment has promised since M7: its sheet grows a vault list above the status tiles, plus a link to server settings, and shows the *vault* initial rather than the server host's first character.

In the browser, the nav bubble's `+` offers New note / New folder (New vault on the dashboard); new folders are created relative to the folder being viewed and validated by the existing `validateVaultPath`; long-press offers Rename / Delete, with delete refused and explained when the folder holds notes. Creating a note from inside a folder should finally default to that folder rather than the vault root.

The server settings screen carries the storage root with a Change action, the vault list with create / rename / remove, and server counts. Remove must say plainly that it unregisters and leaves the files on disk.

## 5. Traps worth knowing before writing any of this

- **The `--vault` shim puts `state/` inside the root.** `--vault /srv/storm/vault` implies root `/srv/storm`, which contains `/srv/storm/state`. A naive scan registers `state` as a vault and indexes the SQLite files in it. `scan_root()` therefore skips `state_dir` unconditionally, and `PUT /v1/config` forbids only the root being *inside* `state_dir` — the direction with no legitimate use. There is a regression test for exactly the shim's layout: root with `vault/` and `state/` side by side registers one vault, not two.
- **`notes.path` is `UNIQUE` and `attachments.path` is a primary key.** Two vaults in one database collide on any shared path, and `Daily/2026-08-07.md` exists in most vaults. One index per vault sidesteps it; a `vault_id` column would need every key rewritten.
- **`FakeServer` returns `'folders': []` unconditionally**, and its `treeRequests` counter is declared but never incremented — which makes one existing "no refetch storms" assertion vacuous. Both need fixing before they can carry new tests.

## 6. Non-goals for this pass

- Cross-vault search, tags and backlinks. Each vault has its own FTS index; a merged search is a separate feature.
- Per-vault tokens or per-vault access control. One shared bearer token still covers the whole server, and it is still LAN-only.
- Two vaults live at once — split panes, or dragging a note between vaults. The routed active-vault design of §4.2 is what would need revisiting first.
- Moving a note between vaults.
- Deleting a vault's files from the app. Removing a vault unregisters it.
- Moving vault directories. Neither the config endpoint nor vault removal ever relocates files.
- Adopting a vault directory dropped into the root while the server is running. Restart to pick it up.
