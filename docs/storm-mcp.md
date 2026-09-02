# Storm MCP — design brief (v0.1)

> **Status: Phase 1 + writes built.** M13 in `PLAN.md`, decisions 37–39. Eleven
> read tools and five write tools at `/mcp`, behind `--mcp` (off by default) and
> the existing bearer token. In addition to notes, the kit vault's canonical
> scripts are readable and writable through four `*_script` tools.
>
> Three corrections this brief earned during implementation, kept here so the
> next reader doesn't re-derive them:
>
> 1. **The Principle needed a refactor, not just discipline.** "A thin wrapper
>    over that route" was impossible — a route *is* an axum handler, taking
>    extractors and returning HTTP types, so no second caller can reach it.
>    `apps/server/src/ops.rs` now holds each operation as a plain async fn that
>    both the handler and the tool call.
> 2. **`get_note_history` was not backed by an existing route.**
>    `note_versions` is populated and `version_content` reads one revision, but
>    nothing listed them and neither was exposed. M13 added `Db::list_versions`
>    plus `/v1/vaults/{v}/notes/{id}/versions[/{n}]`.
> 3. **`get_vault`'s description was new server behaviour.** The server had
>    never parsed `_storm/vault.md` — it only excluded `_storm/` from note
>    counts. Small, since `frontmatter::get_scalar` exists, but not a wrapper.
>
> Two more the *deployment* earned, on 2026-08-08:
>
> 4. **`_storm/vault.md` was discoverable.** The brief says nothing about it
>    because the exclusion looked handled — but it lived in the Flutter client,
>    at five call sites, not in the query. One `NOT_CONFIG` predicate in
>    `db.rs` now covers search, recents and tags. Not the tree: the client reads
>    that note for a vault's colour and property types.
> 5. **On/off had to be a runtime setting, not a boot flag.** `--mcp` decided
>    whether the route was mounted, so switching AI access off meant an SSH
>    session. `/mcp` is now always mounted behind a gate, persisted in
>    `state/vaults.json`, with a switch in the app under **Server ▸ AI access**.
>
> And three SDK/spec defaults that would each have shipped as a bug: rmcp
> restricts `Host` to loopback (the LAN address is refused with nothing naming
> the cause), `structuredContent` must be a JSON object though rmcp will send a
> bare array, and a missing vault must be a *tool* error rather than a protocol
> error or clients need never show the model the one thing it could act on.

Companion to `docs/storm-multi-vault.md`, `docs/storm-properties.md`, and
`docs/storm-adaptive.md`. Written against the state as of the `adaptive-layout`
branch: M0–M12 shipped, real cutover from Obsidian+Syncthing completed
2026-08-07.

## Principle

**MCP is an interface to the existing domain layer. It is never a second way
to touch markdown.** Flutter, REST, and MCP all end up calling the same
functions — the sync protocol's `base_version` + diff3 merge, the FTS5 index,
the frontmatter line-splicer — none of it gets reimplemented for MCP. Where a
REST route already does the right thing, the MCP tool is a thin wrapper over
that route, not a new code path.

This matters more than it would in a fresh project: M9/M10's postmortem is
full of bugs from a second thing quietly diverging from the first (cache
writes reported as network failures, a provider reading the wrong vault). MCP
is a new caller of everything that already exists; it should not become a
second implementation of any of it.

## Why now, and why not further

Multi-vault, folders, properties, and adaptive layout are all shipped and
deployed. The domain layer MCP would sit on top of — vaults, notes, tags,
versions, search — is exactly the shape it needs to be. That removes the
objection that mattered before this branch existed: building MCP against a
single-vault domain shape and reworking it once multi-vault landed.

**What it doesn't remove: the real vault cutover finished one day before this
plan is being written.** Every milestone in `PLAN.md` that reached production
found real bugs there that no test suite caught first — that's not
incidental, it's the single most repeated lesson in the decision log. Adding
a new class of writer (an AI agent calling `update_note`) is exactly the kind
of new path that's found bugs every time one has been added. So:

- **Read-only tools (`list_vaults`, `search`, `get_note`, `get_note_history`,
  `recent_notes`, `list_tags`) can start now.** They touch nothing; worst case
  is a wrong answer, not a corrupted note.
- **Mutation tools (`create_note`, `update_note`, `append_to_note`, `tag_note`,
  `move_note`) wait until the freshly cut-over vault has had normal runtime —
  call it two weeks of ordinary use — before a second writer gets added to
  it.** This is a scheduling decision, not an architectural one; the code for
  both can be written and tested together, mutation tools just don't get
  pointed at the real vault's token until that runway has passed.

## Architecture

One binary, one process — consistent with decision 8 (bare static binary, no
Docker, because a second moving part buys nothing here). MCP is a mode of the
existing `storm-server`, not a separate bridge:

```
storm-server --mcp          # or: storm mcp, as a subcommand
```

serving MCP over Streamable HTTP on the same port (or an adjacent one) as the
REST API, in-process, calling the same `AppState`/`VaultHandle` the REST
handlers already use. `stdio` mode for local Claude Code use can be a thin
wrapper that dials the HTTP server over loopback — no reason to duplicate the
tool implementations for two transports.

The VM doesn't have a systemd unit yet (`run.sh`, manual, doesn't survive
reboot) — MCP doesn't add a second thing to that problem, since it's the same
process.

### SDK note

`rmcp` (the official Rust SDK) has Tier 2/beta support for the current
2026-07-28 spec revision — the largest revision the protocol has had, moving
to a stateless core. TypeScript/Python/Go/C# are Tier 1; Rust isn't yet.
Acceptable to build against, but expect some churn as `rmcp` catches up to
Tier 1 — pin a version and expect to bump it, don't chase `main`.

## Tool surface — mapped to what already exists

### Read (Phase 1, buildable now)

| Tool | Backed by |
|---|---|
| `list_vaults` | `GET /v1/vaults` — already returns id, name, note count, missing status |
| `get_vault` | `GET /v1/vaults` + the vault's `_storm/vault.md` description (see below) |
| `search` | Existing FTS5 index — 1.1ms p95 measured against 600 notes, no new indexing |
| `get_note` | `GET /v1/vaults/{v}/notes/{id}` — include tags, links, backlinks (M4 already computes these) |
| `get_related_notes` | Backlinks query from M4, plus shared tags — no semantic similarity in v1 |
| `list_tags` | Existing tag index, already groups hierarchical tags |
| `recent_notes` / `recent_changes` | `GET /v1/recents` — cross-vault, already server-side (decision 23) |
| `get_note_history` / `get_note_version` | `note_versions` table — already the merge base, already populated |
| `list_scripts` / `get_script` | The **kit vault** (directory `kit`), stored under its `scripts/` root as attachments — read via the existing attachment store |

Scripts are the one place an address is a name, not a `vault + note_id`: they
are scoped to exactly one vault — the kit vault — so an agent cannot aim them
anywhere else. `name` is relative to `scripts/`, may carry a folder prefix
(e.g. `psi-item-import/run.spec.ts`), and must end in an allowlisted text
extension: `ts`, `js`, `mjs`, `cjs`, `json`, `sh`, `py`, `yaml`, `yml`, `toml`,
`csv`. `.md` is deliberately *not* allowed — markdown belongs to the notes
tools.

### Write (Phase 2, gated on the runway above)

| Tool | Backed by |
|---|---|
| `create_note` | Same creation path Flutter uses; stamp `source: ai`, `created_by: <mcp client>` in frontmatter via the existing line-splicing writer — never a raw YAML dump |
| `update_note` / `append_to_note` | The **same** `PUT` route Flutter calls, carrying `expected_version` → the existing `base_version` + diff3 merge. A version mismatch returns the same `merged`/`conflict` shape the client already handles — MCP tools surface that as a structured error the calling agent can react to (re-read, retry), not a silent overwrite |
| `tag_note` | Existing tag-edit path (through the properties writer, decision 27/30 — never a direct frontmatter serialize) |
| `move_note` | Existing move/rename handling — UUID-tracked, so this is a metadata update, not a file operation from the model's perspective |
| `create_script` / `update_script` | Same kit-vault attachment store as the reads; `create_script` refuses an existing name (use `update_script` to change one), and both take the full text. One store, two surfaces — a script written over MCP is an ordinary attachment over REST |

### Explicitly not in this design

- **`delete_note`** — left out of the initial tool list entirely, not just
  gated. Storm has no soft-delete/trash today for *any* client; adding one
  only for MCP would mean Flutter-deleted notes and MCP-deleted notes behave
  differently, which is its own bug class. If trash is worth having, it's a
  Storm feature decided once, for every client — a separate design brief, not
  a side effect of this one.
- **Vault-scoped tokens.** Decision 4 stands: one shared bearer token,
  LAN-only, and that line holds for MCP too. Per-client/per-vault scopes are
  real value, gated behind the same TLS work already named as a prerequisite
  before this server is reachable beyond the LAN — not before.
- **Embeddings / semantic search.** FTS5 is fast and sufficient at the current
  vault size. Revisit only if lexical search actually falls short in
  practice, not preemptively.
- **Audit log, actor-identity system beyond frontmatter stamping, `query_knowledge`
  context bundling, MCP Apps, MCP Prompts, decision records/ADR convention,
  context packs.** All reasonable future ideas, all deferred — Phase 1 and
  Phase 2 above are the whole of this pass.
- **Filesystem paths never reach the model.** Tools address notes by
  `vault + note_id`, matching how the REST API already works — nothing new
  needed here, just a rule to hold to when writing the tool schemas.
- **No generic file/attachment tool, and scripts only in kit.** The kit-vault
  script tools are deliberately narrow: they cannot write markdown and cannot
  target any vault but kit. That thread is what keeps the MCP write surface to
  "notes, plus scripts in one vault" instead of a second way to drop files
  anywhere.

## Vault descriptions

Rather than a new field or table, use the pattern decision 26 already
established: `_storm/vault.md`, the hidden per-vault config note, gains a
`storm.description` key alongside its existing `storm.type.*` and
`storm.options.*` keys. `get_vault` reads it the same way the properties panel
reads vault colour. No schema change, no endpoint, stays greppable — the
same reasoning that put colour and property types there in the first place.

## Security

Single shared bearer token (decision 4) — MCP clients authenticate with it
exactly like the Flutter client and REST API do. No new auth model. If/when
the server is ever reachable beyond the LAN, TLS and per-device token
rotation land first, per the existing decision — MCP doesn't get a shortcut
around that just because it's convenient.

## Open questions

- Exact shape of the "version conflict" error surfaced to an MCP client —
  should it include the server's current content so the agent can decide how
  to merge, or just the fact of a conflict and let the agent call `get_note`
  again?
- Whether `create_note`'s `source: ai` / `created_by` metadata should be
  visible in the properties panel (per decision 30, every frontmatter key gets
  a row) or filtered out as Storm-internal the way `id` can be hidden — leans
  toward visible, since decision 30's whole point was refusing to let
  metadata hide.
- Whether the two-week runway before enabling write tools should be a real
  calendar wait or tied to some usage signal (e.g. N days of the vault syncing
  cleanly with no conflict markers) — the former is simpler, the latter is
  more honest about what's actually being waited for.
