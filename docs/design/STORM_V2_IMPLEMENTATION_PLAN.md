# Storm v2 — Audit and Implementation Plan

**Status:** audit complete, plan proposed, **no production code changed**.
**Baseline:** `origin/staging` at `c9131b4` ("design handoff v2"), whose code
equals the handoff's baseline `d864f8d` plus docs.
**Sources read in full:** `design_handoff_storm_v2/` (README, both `.dc.html`
files, the screenshots), `docs/design/STORM_UI_UX_DISCOVERY.md`, `CLAUDE.md`,
`AGENTS.md`, the relevant `PLAN.md` decisions (17, 34, 40–44, 64, 77–81n).
**Code audited:** `apps/client/lib` (router, shell, agent, settings, auth,
design system), `apps/server/src` (api, ops, auth, agent, gateway, mcp,
registry), `apps/runtime/src/provider.rs`.

The approved prototype is the visual and behavioural source of truth. Where
this plan resolves a handoff question (§5) it says why; where it needs a
product decision it says so (§5.3).

---

## 0. Environment and baseline (Slice 0 — done 2026-10-08)

| Tool | State |
|---|---|
| Flutter | **3.44.8** stable (CI's pin), user-local in `~/flutter`; Dart 3.12.2. |
| Rust | 1.99 in `~/.cargo/bin` (prefix `PATH`). |
| Browser | `chromium` 152, headless, driven over CDP by the stdlib-only harness. |

**Baseline on untouched `staging` `c9131b4` — everything green:**

| Gate | Result |
|---|---|
| `cargo fmt --check` ×3 crates | clean |
| `cargo clippy --all-targets -D warnings` ×3 | clean |
| server `cargo test` | 546 passed, 2 ignored |
| relay `cargo test` | 112 passed |
| runtime `cargo test` | 53 passed |
| `dart format --set-exit-if-changed` | clean |
| `flutter analyze` | no issues |
| `flutter test` | **773 passed** (incl. `editor_save_loop_test`, which PLAN.md notes codebox could not finish) |
| `make test-live` | e2e 81 · mcp 80 · agent 61 · gateway 65 · auth 74 · client live 20 (+1 skipped: needs `STORM_AGENT_BASE`, not set by the Makefile) — 0 failed |

**Acceptance harness:** `docs/design/acceptance/storm-v2/` — real server +
real web build + headless Chromium, signs in through the UI, 10 baseline
shots of today's app in `baseline/`. Observations recorded for slice 4: the
desktop note view shows no title heading, and a note's `color:` tints the
page background, which the v2 references do not show.

## 1. Audit — what exists today

### 1.1 Client architecture

- **Routing:** one `GoRouter` (`lib/router.dart`). Everything is a child of
  `/` (decision 17), which is `DashboardScreen`. Two `ShellRoute`s under it:
  `AgentsShell` (`/agents`, `/agents/hosts`) and `VaultShell` + `VaultGate`
  (`/v/:vault/{browse[/path],note/:id,search,tags,settings/server,settings/client}`).
  `/settings/server`, `/settings/integrations` are dashboard children outside
  any shell; `/settings/mcp-keys`, `/add-device`, `/gallery`, auth screens are
  top level. The redirect owns auth state and an **owner guard** on `/agents*`.
- **Desktop home coupling (H7):** at ≥900 the dashboard renders blank and
  post-frame `go`es to `notesHome()`. The stale `GoRouter.of(context).state`
  read is load-bearing for system back (decision 78).
- **State:** Riverpod 3. `settingsProvider` (prefs + credentials),
  `vaultsProvider`, `activeVaultProvider`, `treeProvider`, `recentsProvider`
  (cross-vault, server-side), `syncEngineProvider` (active vault only),
  `agentOverviewProvider` (sessions + hosts, 15 s poll, never throws),
  `agentAccessProvider` (owner check via `GET /v1/config/agent`),
  `agentTabsProvider` (device-local open sessions), `agentTitlesProvider`
  (names read from terminal titles, device-local).
- **Design system:** `tokens.dart` (OKLCH derivation from ~15 inputs; type
  scale `fs=16`, `scale=1.25`: display 31.25, heading 20, body 16, code 12.8,
  label ≥11), `theme.dart`, `accents.dart` (vault tiles), `widgets.dart`
  (`StatusDot`, `TagChip`, `SectionLabel`, `StormInput`, `StormSwitch`,
  `PopoverItem`, `NoteRow`, `FolderRow`, `Breadcrumb`, `VaultCard`…),
  `states.dart` (`EmptyState`, `SkeletonRows`, `OfflineNotice`,
  `ConflictCard`), `surfaces.dart` (`StormSheet`, `StormPopover`),
  `shell/nav_bubble.dart` (`StormPill`, `PrimaryCircle`), `breakpoints.dart`
  (one breakpoint, ratio widths). `token_conformance_test.dart` forbids
  literals in `lib/ui/`.
- **Two visual grammars (P9):** notes screens are bespoke; Agents, Hosts,
  Integrations, MCP keys, Server settings use Material `AppBar`,
  `SwitchListTile`, `ChoiceChip`, FAB, `AlertDialog`.
- **Tests:** 59 files in `apps/client/test` (router, back navigation,
  adaptive layout, agents navigation, integrations, auth, tokens…). No
  `integration_test/`, no goldens.

### 1.2 Server architecture

- axum + rusqlite. Tiers as separate routers: `none`, `host` (`sht_`),
  `device` (`StormDevice`), `session` (`Bearer`), `mcp` (session or `stk_`).
- **All logic in `ops.rs`**; REST and MCP are thin callers.
- **Vault access seam:** `api::vault_of(state, actor, Access, vault)` →
  `VaultPolicy::decide`. Shipping policy `AllowAuthenticated`. Every note
  write in `ops.rs` asks for `Access::Write` through it.
- **State files:** `state/auth.db` (users, devices, sessions, pairing,
  `vault_grants` (unused), `security_events`, `api_keys`, hosts, enrollments,
  host tokens, ws tickets; `user_version` 5), `state/vaults.json` (registry:
  root, vaults, `mcp_enabled`, `mcp_writable`, `allow_registration`, relays),
  `state/agent/agent.db` (`sessions`, `session_mcp`, `session_mcp_grants`;
  default provider in `state/agent/config.json`), `state/gateway/gateway.db`
  (connections, calls, oauth; additive-only schema), per-vault `index.db`.
- **Agents → vault:** an agent reaches notes only through the gateway's
  built-in `storm` connection: `ops::connect_builtin` serves `mcp::Storm`
  in-process as `Actor::Agent{session_id, host_id, user_id, role}`;
  `delete_note` is never offered (`NEVER_FOR_AGENTS`); write tools only when
  `allow_vault_writes && mcp_writable`. `create_note` stamps `source: ai`.
- **Runtime:** `SessionSpec { workspace, interaction, launch: LaunchExtras
  {args, env} }` — a generic per-session argument/env carrier already exists
  (used for AM32 MCP config).

### 1.3 Backend capability verification (handoff §9 "unverified" items)

| Capability | Verdict | Evidence |
|---|---|---|
| Signed-in device list | **Exists (B).** | `GET /v1/auth/devices` (all devices: name, platform, last_seen, revoked), `DELETE /v1/auth/devices/{id}`, `GET/DELETE /v1/auth/sessions`. No client UI. |
| Relay list | **Exists (B).** | `GET /v1/config` → `relays` (configured), `PUT /v1/config/relays` (live reconfigure, decision 74), `GET /v1/server` → registered set. No client UI. |
| Server version | **Missing (S, small).** | `/v1/health` returns `{status, service}` only. |
| Note as launch context | **Missing (S).** | `agent::Launch` has no context field. |
| Per-session Wrote list | **Missing (S).** | Only `gateway.db.calls` (metadata, no note ids, 30-day retention). |
| "Edited by session" provenance | **Missing (S).** | Only the `source: ai` frontmatter stamp on create. |
| Split write permissions | **Missing (S).** | `mcp_writable` gates both `/mcp` and agent writes (`authorize_call`). |
| One write vault per session | **Missing (S).** | `session_mcp.allow_vault_writes` is a boolean across all vaults. |
| Session name | **Missing (S).** | No name column; client shows terminal titles, device-local. |
| Run-again prefill | **Partial.** | host/workspace/provider on the record; write flag in `session_mcp` but not exposed; no context/write vault. |

---

## 2. Design → implementation map

Legend: **R** reuse · **X** refactor/extend · **N** new · **D** delete ·
**S** server work · **M** migration · **T** new tests.

### 2.1 Shell, navigation, routing

| Design element | Today | Plan |
|---|---|---|
| Activity Rail (56, Notes/Agents/status/Settings, badge) | none | **N** `ActivityRail`, `RailItem`, `RailStatusDot`, `HealthPopover` in a new `AppShell` `ShellRoute` wrapping every signed-in route at ≥900. **T** |
| Shared sidebar frame (Notes/Agents/Settings, 260, `surface`, ⌘\) | two look-alike sidebars | **N** `SidebarFrame` (deferred in 78, now needed by three shells). `VaultSidebar`, `AgentsSidebar` **X** onto it; ⌘\ works on all three. |
| Phone corner bubbles (places / settings gear) | `VaultBubble`, `SettingsBubble` ("A") | **X** `StormBubble`: left = vault tile or agents icon + status dot; right = gear (accent in Settings). "A" popover **D**. |
| Place picker | none (vault popover has sync, server settings) | **N** `PlacePicker` popover (vaults, Agents + "N running", sync line). |
| Dashboard / Agents band / masthead / vault grid | `dashboard.dart`, `agents_band.dart` | **D**. Jobs re-homed (§1.2 of handoff). |
| Space switch Notes \| Agents | `space_switch.dart` | **D** (rail replaces it). |
| Routes | see §1.1 | **X** per §6.2; old routes redirect. **T** router + back tests rewritten to the new contract. |
| Launch restore | `notesHome()` + desk forward | **N** persisted last location per activity (device prefs); `/` redirects in the router, no build-time forward (removes H7 coupling). |

### 2.2 Notes

| Design element | Today | Plan |
|---|---|---|
| Vault header + switcher popover (no "Server settings", "Manage vaults ›") | `_VaultSwitcher` in `vault_sidebar.dart` | **X** `VaultHeader`, `VaultSwitcherPopover` on `StormPopover`. |
| Search field "Search {vault} ⌘K" | `_SearchField` | **R/X** restyle. |
| RECENT (4, cross-vault, vault tag) | phone dashboard only | **X** move `recentsProvider` rows into sidebar top; **N** `VaultTag`, `SidebarRow`. |
| FOLDERS tree + unseen dot | `FolderTree` | **X** add `UnseenDot` (needs §2.5 data). |
| Footer New note / folder / tags | sidebar footer with mentions + gear | **X** (gear and mentions leave the footer). |
| Note header: crumb, Read\|Edit, Start session, drawer toggle | `note_screen.dart` header + `NoteModeToggle` | **X** `NoteHeader`; **N** `StartSessionButton` (soft action). |
| Version line + provenance link | `SaveStateLabel`-based line | **X** `VersionLine` + **N** `ProvenanceLink` (**S**). |
| Body, mentions, properties drawer | existing editor, `mentions_section`, `properties_panel` | **R** unchanged behaviour; token-level spacing per handoff. |
| Phone vault root (title, RECENT, FOLDERS), folder drill-down ("‹ parent"), pill (4 slots) | `browse_screen.dart` (breadcrumbs), 6-slot pill | **X** browse screen layout; pill **X** to Directory/Search/＋/Tags on root and folder screens. |

### 2.3 Agents

| Design element | Today | Plan |
|---|---|---|
| Sidebar: title, ＋ New session (online host only), Overview, RUNNING/ENDED | `AgentsSidebar` + `AgentSessionList` + footer | **X** onto `SidebarFrame`; **N** `SessionRow` (dot incl. starting ring). |
| Overview: WORK cards by (workspace, host), context chip, "wrote n", START AN AGENT, infra line | empty "No session open" pane | **N** `WorkGroupCard`, `AgentStartCard`, `NoteContextChip`, `InfraLine`. Derived client-side from sessions + hosts + integrations; context/wrote need **S**. |
| First-session / no-host states | generic `EmptyState`s | **N** with `NumberedSteps` on `EmptyState`. |
| Launcher (modal 460 / bottom sheet; CONTEXT; Host/Workspace/Agent selects; Can write to + vault; risk box) | `_Launcher` bottom sheet with chips + `SwitchListTile` | **R** load logic (last host, workspaces, default provider, fallback announce, launch notice); **X** UI into `NewSessionLauncher` + `LauncherField`, `ContextBox`, `WriteVaultField`, `RiskNote`; **S** context + write vault + name. |
| Session detail desktop split (terminal \| Context/Wrote/About) | tab strip + terminal | **X** `SessionHeader`, `InlineConfirm` (replaces `AlertDialog`), `TerminalSurface` **R**; **N** `SessionPanelTabs`, `WroteList`, `KeyValueList`, context panel (read-only note via `StormMarkdownView` **R**). Tab strip + switcher sheet **D** (sessions become routes). |
| Phone session (chips, terminal, extra keys, details sheet, ended bar) | `_SessionPage` with AppBar | **X**; `_ExtraKeys` **R**; **N** `SessionDetailsSheet`. AppBar **D**. |
| Run again / Dismiss | Dismiss only | **N** Run again (prefill, **S**). |
| Status vocabulary drawing (accent running, ring starting, danger failed) | `StatusChip` tones (good/warn/bad/muted) | **X** `SessionStatusDot`/`StatusChip` per §3.2. Labels change to the handoff's set (Starting/Running/Unknown/Completed/Stopped/Failed + reason in meta). |

### 2.4 Settings

| Design page | Today | Plan |
|---|---|---|
| Shell (desktop sidebar nav + 680 page; phone list → pushed page, no AppBar) | `ServerSettingsScreen` (AppBar, one long page), `ClientSettingsScreen`, bubble popover | **N** `SettingsShell`, `SettingsNav`, `SettingsPage`, `SettingsRow`, `StormToggle` (from `StormSwitch`), `ChoiceChips`, `PresetPicker`, `InfoBox`, `HealthRow`. `server_settings_screen.dart` + `client_settings_screen.dart` **D** after content is re-homed. |
| This device | `ClientSettingsBody` | **X** content moves; text-size/note-font controls keep their real behaviour (prototype labels are static). |
| Devices & access | MCP keys screen; Add device; Sign out | **X** keys (shown-once dialog **R**); **N** device list on existing endpoints. |
| Vaults | Server settings ▸ Vaults | **X** rows + missing state + New vault; colour per §5 Q7. |
| AI access | Server settings ▸ AI access (MCP) | **X** two groups; agent toggle needs **S**. |
| Integrations | `integrations_screen.dart` (AppBar, FAB) | **X** restyle only; flows **R**; nav danger dot. `/settings/integrations` path kept (OAuth orphan). |
| Hosts & default agent | `hosts_screen.dart` | **X** restyle; flows **R**. |
| Storage | Server settings ▸ storage root | **X**. |
| Connection | sync line pieces, Disconnect | **X** + **N** relay list on existing endpoints. |
| Advanced | — | **N** (MCP endpoint, versions **S**, Re-pair → existing pairing flow). |
| About & health | — | **N** aggregation of sync, hosts, integrations, version compat. |

### 2.5 The loop (all **S** + client)

Context note on launch → Context tab → agent reads it → writes in one vault
→ Wrote list → provenance on the note → unseen dot → back to the session.
Detailed in §7.

---

## 3. Single-user audit and migration strategy

### 3.1 Where the multi-user model lives

**Server — schema** (`auth.db` unless noted):
`users` (username, username_fold, display_name, password_hash, **role**,
**status**, lockout), `sessions.user_id`, `pairing_sessions.purpose
'first_user'` + `created_by`, **`vault_grants`** (never read — A9
placeholder), `security_events.user_id`, `api_keys.user_id`,
`runtime_hosts.enrolled_by`, `host_enrollments.created_by`;
`agent.db sessions.owner_user_id`; `gateway.db` `owner_user_id` on
connections, calls, OAuth flows; `vaults.json allow_registration`.

**Server — code:** `auth::users::Role` and last-active-owner rules;
`Actor::{Session,Key,Agent}.role`; `ops::require_owner`,
`require_integration_owner`, `owner_only`, `target_user` (keys: "owner may
manage anyone's"); `api::require_owner_session` on every `PUT /v1/config*`;
`GET /v1/users` (device tier — **lists usernames to any paired device**),
`POST /v1/users` (registration), `GET /v1/auth/registration`,
`PUT /v1/config/registration`; `authorize_call`'s "owner active" check;
CLI `storm-server user {add,disable,enable,role}`; `cli_users.rs`,
`auth_e2e.py`, and the `only_an_owner_may_change_server_config` test.

**Client:** `signup_screen.dart` and `/signup`; the login **account
picker** (`listUsers`) and "Create an account"; pairing's first-account
creation (username + password); `AuthUser.role`; the registration switch
in Server settings; `agentAccessProvider` and every gate on it (rail/band/
switch visibility, router guard, Server settings sections); "Agents are
available to the server owner only" copy; tests `login_screen_test`,
`pairing_*`, `auth_settings_test`, `agents_navigation_test` member cases.

### 3.2 What must stay (security boundaries, unchanged)

Device credentials and pairing (QR, web bootstrap, identity challenge);
session tokens with refresh rotation; password + Argon2 bound (the
`Hasher` semaphore) and lockout; `stk_` keys on the MCP tier only and never
minting keys; host tokens on the host tier only; gateway per-call
authorization; credential sealing; `security_events` audit; the vault
access seam (repurposed in §7.4 for the write vault).

### 3.3 Migration design (`auth.db` v6) — approved with amendment

Decisions in force: §5.3 A (oldest active owner survives, every other
account is deleted) and §5.3 B (password only).

#### 3.3.1 Target schema

`users` is rebuilt as the one-row **account** table. It keeps its name so
every existing `REFERENCES users(id)` stays valid without rebuilding the
tables that hold them.

```sql
CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  -- At most one row, enforced by the schema, not by code: SQLite cannot
  -- express "exactly one", but UNIQUE + CHECK on a constant is "at most one".
  only_row      INTEGER NOT NULL UNIQUE DEFAULT 1 CHECK (only_row = 1),
  password_hash TEXT NOT NULL,
  created       TEXT NOT NULL,
  updated       TEXT NOT NULL,
  last_login    TEXT,
  failed_count  INTEGER NOT NULL DEFAULT 0,
  locked_until  TEXT
);
```

Gone: `username`, `username_fold`, `display_name`, `role`, `status`.
`vault_grants` is dropped. `pairing_sessions.purpose` keeps `'first_user'`
as a value (renaming it would be a second table rebuild for no behaviour).

A **fresh install** creates exactly this schema directly; there is one
schema, reached two ways, and a test asserts the two are identical.

#### 3.3.2 The procedure

Runs inside `AuthDb::migrate` when `user_version < 6`, before the server
binds a socket.

1. **Gate.** `user_version >= 6` → nothing to do. Also skip if `users` has no
   `role` column (a v6 schema under a stale version number — set the version
   and stop). This makes a second run a no-op by construction, not by luck.
2. **Choose the survivor**, read-only, before writing anything:
   `SELECT id FROM users WHERE role='owner' AND status='active' ORDER BY
   created, id LIMIT 1`. (`id` breaks a tie on identical timestamps, so the
   choice is deterministic.)
   - **No users at all** (installed, never set up): survivor = none; the
     schema is still rebuilt; setup creates the account later.
   - **Users exist but no active owner** (only members, or every owner
     disabled): **refuse to start**, change nothing, and say how to proceed:
     re-enable an owner with the current binary (`storm-server user enable`)
     before upgrading, or run the new one-shot
     `storm-server single-user --keep <username>` against the v5 database.
     Picking an account on the operator's behalf here would hand the server
     to whoever happens to be oldest.
3. **Backup.** Snapshot to `state/auth.db.pre-v6` with the existing
   `snapshot_to` (consistent under WAL), written to a temporary name, fsynced,
   then renamed. If the backup fails, **abort before any write** and refuse to
   start. An existing `auth.db.pre-v6` is replaced: the database being
   migrated is by construction still v5, so the new snapshot is the true
   pre-migration state. Mode `0600`, like `auth.db`.
4. **One transaction** (`BEGIN IMMEDIATE`), with `PRAGMA foreign_keys=OFF`
   set **outside** it (SQLite ignores the pragma inside one) and restored in
   every exit path — the v3→v4 lesson.
   1. Record which accounts go: every user id ≠ survivor.
   2. **Detach rows that reference a deleted account without a cascade.**
      `pairing_sessions.created_by` → `NULL` for consumed rows (history);
      unconsumed rows created by a deleted account are **deleted** (a pending
      invitation from someone who no longer exists must not stay redeemable).
      `runtime_hosts.enrolled_by` → `NULL` (hosts are the Storm's machines;
      they stay enrolled and listed in Settings › Hosts).
   3. **Revoke devices that belonged only to deleted accounts:** a device
      with at least one `sessions` row of a deleted account and none of the
      survivor gets `revoked = now, revoked_reason = 'single_user_migration'`.
      A device with no sessions at all is kept (it can only reach the device
      tier; it still needs the password to sign in).
   4. **Delete the deleted accounts' dependents explicitly** — not by relying
      on `ON DELETE CASCADE`, because FK enforcement is off during a rebuild:
      `ws_tickets` of their sessions, `sessions`, `api_keys`,
      `host_enrollments` they created.
   5. One `security_events` row per deleted account
      (`kind='account_removed_single_user'`, `detail` = `{user_id,
      role, sessions, keys, devices_revoked}` — counts and ids, never a
      username's secret, never a hash), and one
      `kind='single_user_migration'` row naming the survivor id.
   6. **Rebuild `users`**: create `users_new` (§3.3.1), copy the survivor's
      `id, password_hash, created, updated, last_login, failed_count,
      locked_until`, `DROP TABLE users`, `ALTER TABLE users_new RENAME TO
      users`. `DROP TABLE vault_grants`.
   7. **`PRAGMA foreign_key_check`** — any row returned aborts the
      transaction. This is the orphan proof, run by the database itself.
   8. `PRAGMA user_version = 6`, then `COMMIT`.
5. **Cross-database reconciliation** (`agent.db`, `gateway.db`). These files
   cannot share `auth.db`'s transaction, so this step is a separate,
   **idempotent sweep that runs on every boot**, keyed on "owner id is not
   the account id": gateway connections of a removed account are deleted
   with their sealed credentials and OAuth rows (audited, host name only);
   agent sessions of a removed account that have not ended become `failed`
   with `end_reason = 'owner_removed'` and their grants revoked; ended ones
   are dismissed. Because it is idempotent and keyed on current state, an
   interrupted boot simply finishes the sweep next time. (In practice only a
   *second owner* could own any of these — members never could.)

#### 3.3.3 Post-migration invariants

Asserted by tests (§3.3.4) **and** checked at runtime by
`AuthDb::check_single_user()` on every boot after migration; a violation
refuses to start with the failing invariant named.

| # | Invariant |
|---|---|
| I1 | `SELECT count(*) FROM users` ≤ 1, and = 1 once setup has happened. The schema makes 2 impossible (`only_row`). |
| I2 | The surviving row's `id` and `password_hash` are those of the oldest active owner before migration. |
| I3 | Every `sessions`, `api_keys`, `host_enrollments` row references the account id; every non-null `pairing_sessions.created_by`, `runtime_hosts.enrolled_by` does too. `PRAGMA foreign_key_check` returns nothing. |
| I4 | No row anywhere references a deleted account id (`auth.db` by FK check; `agent.db` sessions and `gateway.db` connections/flows by the sweep's query returning zero). |
| I5 | `users` has exactly the §3.3.1 columns; `vault_grants` does not exist; no column named `role`, `status`, `username*`, `display_name` exists in `auth.db`. |
| I6 | `user_version = 6`; re-running `migrate()` changes no byte of the database (schema dump and row hash identical). |
| I7 | Surviving resources are unchanged: survivor's sessions keep `access_hash`/`refresh_hash`/expiry; survivor's keys keep `secret_hash`; revoked things stay revoked; devices used by the survivor are not revoked. |
| I8 | `security_events` has one removal row per deleted account and one migration row; no event contains a password, hash or token. |

#### 3.3.4 Migration tests (Rust, `auth/db.rs` + `tests/`)

**Fixture `v5_multi_account()`** builds a real v5 database through the v5
code paths' SQL (frozen as constants in the test, as `V1_SESSIONS` already
is), containing:

- `owner_old` — active owner, created first (**survivor**); `owner_new` —
  active owner, created later; `owner_disabled` — disabled owner created
  *before* `owner_old` (must **not** win); `member_a`, `member_b` — active
  members; `member_disabled`.
- Survivor resources: 2 devices with live sessions, 1 revoked session, 2 API
  keys (1 revoked), a ws ticket, a consumed pairing it created, a host it
  enrolled, a pending host enrollment.
- Deleted-account resources: `member_a` on its own device (sessions + key +
  ws ticket + an **unconsumed** pairing it created); `member_b` sharing a
  device with the survivor (the shared device must survive); `owner_new`
  with a host it enrolled, a host enrollment, a gateway connection with a
  sealed credential, a live and an ended agent session; a consumed pairing
  created by `member_disabled`.
- A `vault_grants` row (to prove the drop), `security_events` history.

**Tests:**

1. `migrates_multi_account_to_the_oldest_active_owner` — I1, I2, I5.
2. `surviving_resources_stay_attached` — I3, I7 (row-by-row comparison
   against the fixture's survivor set).
3. `deleted_accounts_leave_no_orphans` — I4 incl. FK check; `member_a`'s
   device revoked, shared device kept, unconsumed pairing gone, consumed
   pairings kept with `created_by NULL`, `owner_new`'s host kept with
   `enrolled_by NULL`.
4. `cross_db_sweep_removes_deleted_owners_resources` — gateway connection +
   credential gone, live agent session `failed/owner_removed`, ended one
   dismissed; **second run is a no-op**.
5. `migration_is_not_applied_twice` — I6; also a v6 schema with
   `user_version` reset to 5 is detected and only re-stamped.
6. `failure_mid_migration_leaves_v5_untouched` — a test-only fault hook
   fails after each numbered step of 4.x in turn; after each, the database
   is byte-for-byte the fixture (schema dump + row hashes), `user_version`
   is 5, `foreign_keys` is back ON, and a subsequent clean `migrate()`
   succeeds.
7. `pre_v6_backup_is_written_first` — exists, is a valid v5 database equal
   to the fixture, mode `0600`; with the backup directory unwritable the
   migration refuses and the database is untouched.
8. `no_active_owner_refuses_and_changes_nothing` and
   `single_user_keep_flag_chooses_the_named_account`.
9. `fresh_install_schema_equals_migrated_schema` — `sqlite_master` of a
   fresh v6 database equals the migrated one (modulo row data).
10. `security_events_record_the_migration_without_secrets` — I8.
11. `check_single_user_refuses_a_violated_database` — e.g. an orphaned
    session inserted with FKs off.

**Boot and auth tests (in-process router, `api.rs` test module):**

12. `migrated_database_serves` — open the migrated fixture through the real
    `serve` setup path: health 200, survivor's existing access token
    authorizes `GET /v1/vaults`, refresh rotates, survivor's `stk_` key
    authorizes `/mcp`, deleted member's token and key are 401, revoked
    device's credential is refused at the device tier.
13. `fresh_install_serves_and_sets_up_once` — empty state → first setup
    (password only) → login → second setup attempt refused.
14. `password_login_after_migration` — survivor's original password signs
    in with `{password}` only; a deleted account's password fails; lockout
    counters carried over still apply; the Argon2 semaphore path is the one
    used (no KDF shortcut).
15. `older_client_login_body_still_works` — `{username, password}` from a
    v0.3.x client is accepted (username ignored), so an upgraded server does
    not lock out an un-upgraded phone.

**Live suite:** `tests/auth_e2e.py` rewritten for single user (setup,
login, refresh, revoke device, keys, ws ticket), plus a new
`tests/migration_e2e.py` that boots the real v0.3.1 schema fixture copied
into a state dir, starts the new binary, and runs 12–14 over HTTP.

#### 3.3.5 Code changes riding on the migration

Delete `Role`, role fields on `Actor`, `require_owner*`, `owner_only`,
`target_user`'s cross-user branch, registration routes and
`allow_registration` (ignored on load, dropped on next save),
`GET/POST /v1/users`, `user {add,disable,enable,role}` CLI (replaced by
`passwd` and the one-shot `single-user --keep`). `/v1/users/first` becomes
the one-time setup call (device tier, closed after first use).
`authorize_call` checks "the account exists" instead of "owner active".
`agent.db`/`gateway.db` keep their `owner_user_id` columns (gateway.db is
additive-only by invariant 81b; agent.db follows the same rule) and always
hold the account id; **no code reads them for an authorization decision**
any more — a test greps `ops.rs` for `role()` / `Role::` and fails on any.
Client: signup, picker, registration switch, `AuthUser.role`,
`agentAccessProvider` and every owner gate removed. Docs: decisions
77d/78/80/81c amended, new decision 82, `CLAUDE.md` M19 invariants
rewritten (owner/last-owner rules → single account rules).

## 4. Prototype-only behaviour to replace

Simulated session progression, terminal lines, BOARD edit and log-note
creation are replaced by: the real PTY stream (`TerminalSurface`), the
server's session status, and `session_writes` rows written by real agent
tool calls (§7.3). Sample data, static health rows, static text-size/font
labels, local-only toggles and no-op controls all bind to real providers
and endpoints. No timers or fixtures ship in `lib/`; fixtures live only in
the acceptance harness (§9.3), which drives a real server and the runtime's
real `fake` provider.

---

## 5. Handoff questions

### 5.1 Resolved here (engineering-level; implemented as stated)

- **Q1 Routes.** Adopt the proposed scheme: `/v/:vault/…` unchanged,
  `/agents`, `/agents/s/:id?tab=context|wrote|about`, `/settings`,
  `/settings/:page` (`device, access, vaults, ai, integrations, hosts,
  storage, connection, advanced, health`), plus `/notes` (Notes activity
  entry: last vault root, or the no-vaults empty state). Redirects:
  `/settings/server`, `/v/:v/settings/server` → `/settings/vaults`;
  `/settings/mcp-keys` → `/settings/access`; `/agents/hosts` →
  `/settings/hosts`; `/v/:v/settings/client` → `/settings/device`;
  `/add-device` stays as the QR page reached from Devices & access.
- **Q2 Root and back.** `/` is a redirect to the device's last location
  (per activity: last vault+note / last session or overview / last settings
  page). Back: pushed pages pop; a page reached by deep link with nothing to
  pop goes to its logical parent (note → its folder, folder → parent, session
  → `/agents`, settings page → `/settings` on phone); **Agents and Settings
  roots go back to the last Notes location; the Notes vault root exits.**
  This is Android's "back from a non-start destination returns to the start
  destination" rule with Notes as start. The redirect happens in the router,
  so the build-time forward and its stale read (H7) disappear; the back test
  is rewritten to this contract.
- **Q5 Phone note pill.** Prototype wins: no pill on the note screen. The
  Mentions badge's job is served by Linked mentions at the end of the note
  (existing). Note actions stay on the existing header "Note actions" button.
- **Q6 Phone properties.** Keep the existing properties sheet and its header
  entry point (the prototype shows neither; nothing replaces it).
- **Q7 Vault colour.** Tapping the vault tile in Settings › Vaults opens the
  existing `AccentPicker`; long-press on a vault row in the switcher / place
  picker does the same. Rows stay visually identical to the prototype.
  Stored as before (`storm.color` in `_storm/vault.md`).
- **Q8 Running colour.** `accent` (handoff semantic rule "accent = active and
  running"). `StatusChip`/`StatusDot` gain the session variants; the generic
  `ChipTone` stays for hosts/integrations.
- **Q9 Rail dot when healthy.** `green` as drawn (prototype wins).
- **Q10 Type sizes.** Use existing roles where they match (13→`codeSize`
  12.8, 16–20→`headingSize`, mono 11→`labelSize`, note title→`displaySize`
  31.25). Add two **derived** steps to `StormTokens`, no new inputs:
  `titleSize = fs·scale²` (= 25.0, page titles — exact) and `uiSize =
  fs/√scale` (≈14.3, UI body/rows). Both scale with the text-size setting;
  the conformance test still forbids literals.
- **Q12 Session naming.** **Server-assigned and stored** (one name on every
  device): slug of the context note title (`gateway-spec`), else
  `{workspace}-{n}`; a live duplicate gets `-2`. The device-local
  terminal-title map (`agentTitlesProvider`) is retired.
- **Q13 Provenance.** The note shows the latest agent writer only; every
  session's Wrote still lists everything it wrote. A later human edit does
  not clear the link (the statement "edited by session X, 2h ago" stays
  true); the unseen dot clears on open.
- **Q3 Run again.** Solved by exposing the launch record (§7.1).
- **Agent vs Session vs Kit.** UI: "Agent" = Claude Code/OpenCode/Shell
  (code keeps `provider`), "Session" = one run, Work = derived grouping,
  Workspace = host directory, Kit files are plain notes and never called
  agents nor offered in the launcher.
- **Extra keys.** Shown while the session is live (prototype), with the
  existing sticky Ctrl/Shift and Paste; Done only dismisses the keyboard.
- **Version compatibility.** Server reports its version (§7.6); "compatible"
  = same major.minor. Anything else reads "This client and the server may
  not be compatible" in `danger`.

### 5.2 Engineering detail decided

- **Tabs.** The per-device tab strip and switcher sheet go: sessions are
  routes, the rail/list/overview are the switchers, and "last open session"
  is persisted for launch restore.
- **Write-permission migration.** New registry flag `agent_writes`; absent
  on load → initialised from `mcp_writable`, so no server changes effective
  behaviour on upgrade. Fresh installs: off (matches today's defaults). The
  launcher's toggle defaults on when the flag is on (handoff §4.5).
- **Older clients** launching with `allow_vault_writes: true` and no vault
  get a read-only session plus a launch notice — never all-vault writes.

### 5.3 Needs a product decision (asked separately)

- **A. Existing extra accounts on upgrade** (the only destructive step).
- **B. Sign-in identity:** password only, or username + password.
- **C. Q11 context delivery:** how the agent receives the note.

**Answered 2026-10-08:**

- **A →** the oldest active owner becomes the Storm user; every other
  account is deleted (CASCADE ends its sessions and keys; its devices are
  revoked), one `security_events` row each, `auth.db.pre-v6` written first.
- **B →** password only. First run asks for a password; sign-in is a
  password field; recovery is `storm-server passwd`. The `users` rebuild
  drops `username`/`username_fold`/`display_name`/`role`/`status`.
- **C →** read, then wait. The agent reads the note through the built-in
  `storm` connection (a `session_context` tool, named in the connection's
  `instructions`); the launch adds a one-line opening prompt telling it to
  read its context note and then wait for the user. Needs a new optional
  field on the host `start` command carried into `LaunchExtras.args`
  (`claude "<prompt>"`, `opencode --prompt "<prompt>"`), shell excluded;
  `docs/runtime-vectors.json` is unaffected (no signed bytes change).
  The note body never travels in argv — only the instruction does.

---

## 6. Architecture

### 6.1 Client

```
MaterialApp.router
└─ GoRouter
   ├─ /starting /pairing /login           (auth, no shell)
   ├─ /gallery /add-device                (no shell)
   └─ ShellRoute AppShell                  rail (≥900) | bubbles+picker (<900)
      ├─ /            → redirect(last location)
      ├─ /notes       → redirect(last vault root) | NoVaults empty state
      ├─ ShellRoute VaultShell(VaultGate, SidebarFrame: Notes sidebar)
      │  └─ /v/:vault/{browse[/path], note/:id, search, tags}
      ├─ ShellRoute AgentsShell(SidebarFrame: Agents sidebar)
      │  └─ /agents, /agents/s/:id
      └─ ShellRoute SettingsShell(SidebarFrame: SettingsNav)
         └─ /settings, /settings/:page
```

- **Providers added:** `lastLocationProvider` (per activity, prefs),
  `healthProvider` (aggregates sync, hosts, integrations, version),
  `sessionWritesProvider(id)` (polled while the session is live and visible,
  same lifecycle rules as `agentOverviewProvider`), `agentWritesProvider
  (vault)` (note id → latest agent write), `unseenProvider` (device-local
  last-opened version per note, compared with agent writes),
  `integrationsSummaryProvider` (count / needs-sign-in for infra line and
  nav dot).
- **Removed:** `agentAccessProvider`, `agentTabsProvider`,
  `activeAgentTabProvider`, `agentTitlesProvider`, dashboard providers.
- **Session controllers** move from `_AgentsScreenState` to a
  `sessionControllerProvider.family(id)` (autoDispose) so the route-built
  detail view and the phone screen share one stream per session.

### 6.2 Server

New/changed operations, all in `ops.rs`, REST thin; MCP unaffected except
the agent write policy:

| Endpoint | Change |
|---|---|
| `POST /v1/agent/sessions` | + `context: {vault_id, note_id}?`, + `write_vault_id?` (replaces `allow_vault_writes`); validates note readable and write vault exists; assigns `name`. |
| `GET /v1/agent/sessions[/{id}]` | + `name`, `context {vault_id, note_id, title}?`, `write_vault_id?`, `wrote_count`. |
| `GET /v1/agent/sessions/{id}/writes` | **new**: `[{vault_id, note_id, title, path, kind: created\|edited, version, at}]`, newest write first. |
| `GET /v1/vaults/{v}/notes/{id}` | + `agent_write {session_id, session_name, kind, version, at}?` |
| `GET /v1/vaults/{v}/agent-writes` | **new**: map note id → `{version, at, session_id}` for the unseen dots. |
| `GET /v1/config` | + `agent_writes`, + `version`. |
| `PUT /v1/config/ai` | **new**: `{mcp_enabled, mcp_writable, agent_writes}` (or extend `/config/mcp`; decided in code review by smallest diff). |
| registration / users routes | **removed** (§3.3). |

### 6.3 Persistence (all additive except §3.3)

`agent.db`, following its existing "new table, sessions untouched" pattern:

```sql
CREATE TABLE IF NOT EXISTS session_launch (
  session_id       TEXT PRIMARY KEY,
  name             TEXT NOT NULL,
  context_vault_id TEXT, context_note_id TEXT, context_title TEXT,
  write_vault_id   TEXT,
  created_at       TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS session_writes (
  session_id TEXT NOT NULL, vault_id TEXT NOT NULL, note_id TEXT NOT NULL,
  kind       TEXT NOT NULL CHECK (kind IN ('created','edited')),
  version    INTEGER NOT NULL, at TEXT NOT NULL,
  PRIMARY KEY (session_id, vault_id, note_id)
);
CREATE INDEX IF NOT EXISTS writes_by_note ON session_writes(vault_id, note_id, at);
```

Nothing inside a vault (H1). `agent.db` is "rebuildable in spirit": losing it
loses provenance and Wrote history, never notes — stated in the decision.
Dismissing a session keeps its `session_writes` rows so a note's provenance
link still resolves (to a "dismissed session" state); decided in slice 5
with a test.

### 6.4 Authorization (single user)

- `Actor::Agent` gains `write_vault: Option<String>`.
- `AllowAuthenticated` → `StormPolicy`: allow, except `Actor::Agent` +
  `Access::Write` on any vault other than `write_vault` → `Deny`. This is the
  seam's intended use; every agent write already passes through `vault_of`.
- Gateway `vault_writes = write_vault.is_some() && agent_writes`;
  `/mcp` writes = `mcp_writable` only.
- `delete_note` stays never-for-agents; kit scripts follow the same rule (an
  agent writes a script only if `kit` is its write vault).

### 6.5 The write hook

`ops::create_note`, `update_note`, `move_note` (+ script writes) record a
`session_writes` upsert when the actor is `Actor::Agent`, after the index
write succeeds, with the resulting version. One helper, one test per op,
and an e2e through the real gateway path (`gateway_e2e.py`).

---

## 7. Backend capabilities in detail

1. **Launch record** (`session_launch`): name, context, write vault — read
   back on every session view; drives Run again exactly (Q3).
2. **Context note** — identification, retrieval and the argv rule.

   *Identification.* The launch request carries `context: {vault_id,
   note_id}`. The server resolves it with `Access::Read` through `vault_of`
   (so the policy applies), refuses a missing note (`404`), and stores
   `context_vault_id`, `context_note_id` and the title snapshot in
   `session_launch`. Notes are tracked by UUID, so a later rename or move
   still resolves; a later delete is reported, never guessed.

   *Retrieval through the built-in connection.* `mcp::Storm` gains one
   tool, offered only to `Actor::Agent`: **`session_context`**, no
   parameters. It looks the context up by **the actor's own
   `session_id`** — which `authorize_call` has already proven belongs to
   the calling host and is live — so an agent can neither name nor reach
   another session's context. It returns `{vault_id, note_id, title,
   path, version, content}` via the same `ops::get_note` read path every
   client uses (current version, not a launch-time copy, matching the
   Context tab). With no context it returns an explicit "this session was
   started without a note"; a deleted note returns "the context note no
   longer exists". The connection's `initialize` result carries
   `instructions` naming the tool ("This session was started from a note.
   Call session_context to read it.") so agents that surface MCP server
   instructions learn of it without any prompt.

   *The argv rule.* The note's content — and its title, path or id, which
   are note data too — **never** enters a process command line, shell
   argument or environment variable. The opening prompt (§5.3 C) is a
   **compile-time constant** with no interpolation:
   `Read your context note with the storm session_context tool, then wait
   for my instructions.` The host `start` command gains one optional
   boolean, `context: true`; the **runtime** maps it to that constant
   (`claude <const>`; `opencode --prompt <const>`; `shell` and `fake`
   ignore it). The server never sends prompt text at all, so no server bug
   can put note data into a host's argv.

   *Verification (slice 5):*
   - unit (server): launch with context stores ids + title; unreadable or
     missing note refused; `session_context` returns the right note for
     its own session, refuses a session id that is not the actor's, and
     reports no-context / deleted;
   - unit (runtime): `context: true` adds exactly the constant to args for
     `claude-code`/`opencode`, nothing for `shell`/`fake`; the env is
     unchanged; a test fails if the constant ever contains `{`/`$`;
   - e2e (`gateway_e2e.py`): launch with a context note whose body and
     title contain a unique marker; the scripted agent calls
     `session_context` through the real host bridge and receives the
     marked content; meanwhile the harness scans
     `/proc/*/cmdline` and `/proc/*/environ` of every process owned by
     the runtime user and asserts the marker appears in **none**;
   - e2e: a second session cannot read the first one's context;
   - manual (slice 9, where the CLIs are installed): Claude Code and
     OpenCode both start with the opening prompt and read the note.
3. **Wrote / provenance**: `session_writes` (§6.3, §6.5).
4. **Write vault**: policy (§6.4).
5. **Split permissions**: `agent_writes` (§5.2).
6. **Version**: `env!("CARGO_PKG_VERSION")` / release version in
   `GET /v1/config` (session tier, not the unauthenticated health route).
7. **Relays, devices**: existing endpoints; client only.

---

## 8. Implementation order (vertical slices)

Each slice: branch from `staging`, PR into `staging` (CLAUDE.md), `make
check` clean, relevant live suites, screenshots compared, `PLAN.md` updated
in the same change.

| # | Slice | Depends on | Done when |
|---|---|---|---|
| 0 | **Tooling + baseline**: Flutter 3.44.8 user-local; `make check` + `make test-live` on untouched staging; acceptance harness (§9.3) producing screenshots of today's app. | — | Baseline green (or pre-existing failures recorded); harness reproducible. |
| 1 | **Single-user server + client cleanup** (§3.3), after §5.3 A/B answers. | 0 | Migration tests pass incl. retry; no owner/member/registration code or UI; auth suites updated; older-client login checked. |
| 2 | **Design-system additions**: `titleSize`/`uiSize`, `SidebarFrame`, `StormToggle`, buttons (primary/outline/soft/danger), `SettingsRow`, `ChoiceChips`, `StatusChip` session variants, `NumberedSteps`, `Popover` positioning helper; gallery entries. | 0 | Gallery screenshots in 3 presets; conformance test green. |
| 3 | **Shell + routing**: `AppShell`, rail, health popover (sync/hosts/integrations rows), bubbles, place picker, new route tree + redirects, launch restore, back contract; dashboard/space switch removed. | 1, 2 | Router/back/adaptive tests rewritten and green; rail + picker screenshots match. |
| 4 | **Notes**: sidebar, switcher, recents, phone root/folder, note header with Start session (opens existing launcher until slice 6), version line, pill. | 3 | desktop-01..04, phone-01..03 match (minus provenance). |
| 5 | **Server agent capabilities** (§6.2–§7): launch record, naming, context, write vault + policy, `agent_writes`, writes table + hook + endpoints, version; Q11 delivery (+ runtime change if chosen). | 1 | Rust unit + `agent_e2e.py`/`gateway_e2e.py`/`e2e.py` green; write outside vault refused; Wrote rows from real tool calls. |
| 6 | **Agents**: sidebar, overview, first/no-host, launcher (modal/sheet), session route + split + panel tabs, inline end, run again/dismiss, phone session + details sheet. | 3, 5 | desktop-05..11c, phone-04..09 match with real sessions (fake provider). |
| 7 | **Settings**: shell + all ten pages on real endpoints (devices, keys, vaults+colour, AI access 2 groups, integrations restyle, hosts, storage, connection+relays, advanced, health). Old settings screens deleted. | 3, 5 | desktop-12..21, phone-10..11 match; OAuth orphan still lands on Integrations. |
| 8 | **Loop**: provenance link, "edited/created by this session", unseen dots (tree + phone rows + collapsed folders), Wrote live update, Open in Notes / ‹ Wrote. | 4, 6 | loop-a/b match; end-to-end core loop on a real server (handoff §11 "Core loop"). |
| 9 | **Regression + acceptance**: full suites, light/earth presets, resize sweeps, Android back on device/emulator if available, docs (`PLAN.md` 82, `CLAUDE.md`, `docs/storm-ui.md`), acceptance set committed. | all | Handoff §11 checklist fully ticked with evidence. |

Slice 1 is early on purpose: every later slice would otherwise be built on
owner gating it then has to remove. Slice 5 can run in parallel with 3–4.

---

## 9. Testing and visual verification

### 9.1 Automated

- **Server (Rust):** auth migration v6 (fresh, one user, multi-user per
  answer A, interrupted-and-retried); policy (agent write in/out of write
  vault, read anywhere, delete never); `session_launch` round trip; naming
  (slug, dedupe, no-note); writes hook per op; provenance latest-writer;
  `agent_writes` load-migration from `mcp_writable`; config view fields.
- **Live suites:** `e2e.py` (REST unchanged where intended),
  `auth_e2e.py` (rewritten for single user), `agent_e2e.py` (context, name,
  write vault, writes endpoint), `gateway_e2e.py` (write outside vault →
  refused code; writes recorded), `mcp_e2e.py` (`mcp_writable` no longer
  affects agents and vice versa).
- **Client:** router + redirects + back contract; adaptive (both sides of
  900 for every place, H5); rail badge/status; place picker; launcher
  defaults (host/workspace/agent/write vault/disabled-by-setting/shell);
  session detail tabs + empty copies; inline end; run-again prefill;
  settings pages bind to endpoints; unseen logic; token conformance in 3
  presets. Existing tests updated only where the approved design changes the
  contract (each such change named in the PR).

### 9.2 Functional (real backend)

Local `storm-server` with fixture vaults (personal/work/kit + a missing
`archive`), `storm-runtime` enrolled with the **`fake` provider** (exists for
exactly this) plus a real shell, and a scripted MCP client acting as the
session's agent to create/edit notes through the gateway — real writes,
real Wrote rows, real provenance, no client-side simulation.

### 9.3 Visual acceptance set

`docs/design/acceptance/storm-v2/`:
- `README.md` — index: implementation shot ↔ reference shot ↔ state recipe.
- `harness/` — `seed.py` (fixture server state), `shoot.py` (headless
  Chromium over CDP: login, navigate to route, set viewport 1280×800 or
  390×844 at DPR 2, wait for frame, capture). Uses only the Python stdlib +
  the existing web build.
- `desktop-*.png`, `phone-*.png` named exactly as the references.
- Per screen: capture → compare side by side with the reference (geometry,
  spacing, type, colour, borders, radii, icons, states) → fix → recapture,
  until no meaningful discrepancy remains. Remaining deliberate deltas (e.g.
  real xterm rendering vs illustrative lines) are listed in the README.

---

## 10. Risks

| Risk | Mitigation |
|---|---|
| `auth.db` v6 migration strands a server (it cannot be rebuilt). | Pre-copy, documented rebuild procedure, retry test, FK pragma outside txn. |
| Older clients against a new server. | Check v0.3.1 login on `GET /v1/users` 404; launch with old flag → read-only + notice. |
| Removing H7 forward breaks desktop back. | Back contract rewritten as tests first, then the router. |
| Flutter web screenshots differ from native (fonts, DPR). | Bundle fonts as the app already does; shoot at DPR 2 to match references; spot-check on Linux desktop build if a display is available. |
| Scope creep into IDE/admin dashboard. | Every surface traced to a handoff section; nothing else added. |

---

## 11. Checklist

- [x] Audit: design handoff, discovery doc, client, server, runtime
- [x] Backend capability verification (§1.3)
- [x] Single-user dependency map (§3.1)
- [x] Handoff questions resolved or escalated (§5)
- [x] §5.3 A/B/C answered
- [x] Slice 0 — tooling + baseline + harness
- [x] Migration plan strengthened: invariants I1–I8, fixture, 15 tests (§3.3)
- [x] Context-note retrieval and argv rule specified with verification (§7.2)
- [x] Slice 1 — single-user
- [x] Slice 2 — design-system additions
- [x] Slice 3 — shell + routing
- [x] Slice 4 — Notes
- [ ] Slice 5 — server agent capabilities
- [ ] Slice 6 — Agents
- [ ] Slice 7 — Settings
- [ ] Slice 8 — the loop
- [ ] Slice 9 — regression + acceptance
