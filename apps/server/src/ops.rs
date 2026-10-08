//! The operations REST and MCP both call.
//!
//! Storm's domain logic used to live inside the axum handlers, which meant a
//! second caller could not reach it: a handler takes extractors and returns
//! HTTP types, so an MCP tool would have had to re-derive everything above the
//! `Db` call — vault resolution and its 404-vs-409 distinction, query
//! sanitising, the not-found cases. That is precisely the "second
//! implementation quietly diverging from the first" that `docs/storm-mcp.md`
//! forbids, and that the M9/M10 postmortem is full of.
//!
//! So each operation lives here as a plain async fn. The handler in `api.rs`
//! is extractors → `ops::` → `Json`; the tool in `mcp.rs` is params → `ops::` →
//! structured content. One implementation, two callers, no HTTP round trip
//! between them.
//!
//! **A new operation belongs here, not in a handler.** One added to `api.rs`
//! alone is invisible to MCP, and one added to `mcp.rs` alone is the drift this
//! module exists to prevent.
//!
//! Returns are the *inner* data — `Vec<VaultInfo>`, not `{"vaults": [...]}` —
//! so the REST envelopes stay exactly where they were and the wire format is
//! untouched. `tests/e2e.py` is what proves that.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::api::{ApiError, ApiResult, Shared, bad_request, conflict, not_found, vault_of};
use crate::auth::authz::{Access, Actor};
use crate::db::{NoteRow, RecentRow, SearchHit};

// ---- vaults ------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct VaultInfo {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub note_count: i64,
    /// The directory is gone. The entry is kept so the vault can be repaired
    /// rather than silently forgotten.
    pub missing: bool,
}

pub async fn list_vaults(state: &Shared, actor: &Actor) -> ApiResult<Vec<VaultInfo>> {
    let vaults = state.vaults.read().await;
    let mut out = Vec::with_capacity(vaults.registry.vaults.len());

    for entry in &vaults.registry.vaults {
        // A collection **filters**; it does not refuse. `403` is the right
        // answer for a named vault and the wrong one for a list — there is no
        // way to refuse half a list, and one unreachable vault must not blank
        // the whole thing. `StormPolicy` lets every caller read every vault.
        if !crate::api::may_see_vault(state, actor, &entry.id) {
            continue;
        }
        let (note_count, missing) = match vaults.get(&entry.id) {
            Some(handle) => {
                let ix = handle.indexer.lock().await;
                (ix.db.count_notes().unwrap_or(0), false)
            }
            None => (0, true),
        };
        out.push(VaultInfo {
            id: entry.id.clone(),
            name: entry.name.clone(),
            dir: entry.dir.clone(),
            note_count,
            missing,
        });
    }
    Ok(out)
}

/// Where a vault keeps its own configuration, as an ordinary note so it syncs
/// and stays greppable (decision 26).
const VAULT_CONFIG_PATH: &str = "_storm/vault.md";

#[derive(Debug, Clone, Serialize)]
pub struct VaultDetail {
    #[serde(flatten)]
    pub vault: VaultInfo,
    /// From `storm.description` in `_storm/vault.md`, absent if unset.
    pub description: Option<String>,
    pub folders: Vec<String>,
}

/// One vault, with the description a human wrote for it.
///
/// The description is read from the config note's frontmatter rather than from
/// a new column, for the reason decision 26 gives: it stays readable outside
/// Storm and needs no schema change. Reading it here is the first time the
/// *server* has looked inside that note — until now `_storm/` was only ever
/// something to exclude from counts.
pub async fn get_vault(state: &Shared, actor: &Actor, vault: &str) -> ApiResult<VaultDetail> {
    let info = list_vaults(state, actor)
        .await?
        .into_iter()
        .find(|v| v.id == vault)
        .ok_or_else(|| not_found("no such vault"))?;

    // A missing vault has no directory to read, so stop at the registry entry
    // rather than failing the whole call.
    if info.missing {
        return Ok(VaultDetail {
            vault: info,
            description: None,
            folders: Vec::new(),
        });
    }

    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    let description = match ix.db.get_note_by_path(VAULT_CONFIG_PATH)? {
        Some(note) => ix
            .vault
            .read(&note.path)
            .ok()
            .and_then(|raw| crate::frontmatter::get_scalar(&raw, "storm.description")),
        None => None,
    };
    let folders = ix.all_folders()?;

    Ok(VaultDetail {
        vault: info,
        description,
        folders,
    })
}

// ---- notes -------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NoteDetail {
    #[serde(flatten)]
    pub note: NoteRow,
    pub content: String,
}

pub async fn get_note(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    id: &str,
) -> ApiResult<NoteDetail> {
    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    let note = ix
        .db
        .get_note(id)?
        .ok_or_else(|| not_found("no such note"))?;
    let content = ix.vault.read(&note.path)?;
    Ok(NoteDetail { note, content })
}

/// The latest agent write to a note: the session that wrote it last,
/// even after a human edits it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentWrite {
    pub session_id: String,
    pub session_name: String,
    /// The session was dismissed; its name is still the one it had.
    pub session_dismissed: bool,
    pub kind: String,
    pub version: i64,
    pub at: String,
}

/// A note as REST serves it: [`NoteDetail`] plus its provenance.
#[derive(Debug, Clone, Serialize)]
pub struct NoteWithProvenance {
    #[serde(flatten)]
    pub detail: NoteDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_write: Option<AgentWrite>,
}

pub async fn get_note_with_provenance(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    id: &str,
) -> ApiResult<NoteWithProvenance> {
    let detail = get_note(state, actor, vault, id).await?;
    let agent_write = match state.agent.latest_write(vault, id).map_err(internal)? {
        Some(w) => {
            let launch = state.agent.launch_of(&w.session_id).map_err(internal)?;
            let session = state.agent.get(&w.session_id).ok();
            Some(AgentWrite {
                session_name: launch
                    .map(|l| l.name)
                    .or_else(|| session.as_ref().map(|s| s.workspace.clone()))
                    .unwrap_or_default(),
                session_dismissed: session.is_none(),
                session_id: w.session_id,
                kind: w.kind,
                version: w.version,
                at: w.at,
            })
        }
        None => None,
    };
    Ok(NoteWithProvenance {
        detail,
        agent_write,
    })
}

/// The latest agent write to a note, for the unseen dots.
#[derive(Debug, Clone, Serialize)]
pub struct LatestAgentWrite {
    pub version: i64,
    pub at: String,
    pub session_id: String,
}

/// Note id → its latest agent write, for every note in a vault an agent wrote.
pub async fn vault_agent_writes(
    state: &Shared,
    actor: &Actor,
    vault: &str,
) -> ApiResult<std::collections::BTreeMap<String, LatestAgentWrite>> {
    vault_of(state, actor, Access::Read, vault).await?;
    Ok(state
        .agent
        .latest_writes(vault)
        .map_err(internal)?
        .into_iter()
        .map(|w| {
            (
                w.note_id,
                LatestAgentWrite {
                    version: w.version,
                    at: w.at,
                    session_id: w.session_id,
                },
            )
        })
        .collect())
}

/// What `session_context` gives an agent: its session's context note, read
/// now, through the same path every client reads.
#[derive(Debug, Clone, Serialize)]
pub struct SessionContext {
    pub vault_id: String,
    pub note_id: String,
    pub title: String,
    pub path: String,
    pub version: i64,
    pub content: String,
}

/// The calling agent's own context note. Keyed on the actor's session, which
/// the gateway has proven is the calling host's and live, so no agent can
/// name another session's.
pub async fn session_context(state: &Shared, actor: &Actor) -> ApiResult<SessionContext> {
    let Actor::Agent { session_id, .. } = actor else {
        return Err(bad_request("only an agent session has a context note"));
    };
    let context = state
        .agent
        .launch_of(session_id)
        .map_err(internal)?
        .and_then(|l| l.context)
        .ok_or_else(|| not_found("this session was started without a note"))?;
    let note = match get_note(state, actor, &context.vault_id, &context.note_id).await {
        Ok(note) => note,
        Err(e) if e.0.is_client_error() => {
            return Err(not_found("the context note no longer exists"));
        }
        Err(e) => return Err(e),
    };
    Ok(SessionContext {
        vault_id: context.vault_id,
        note_id: note.note.id,
        title: note.note.title,
        path: note.note.path,
        version: note.note.version,
        content: note.content,
    })
}

pub struct Backlinks {
    pub title: String,
    pub notes: Vec<NoteRow>,
}

pub async fn backlinks(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    id: &str,
) -> ApiResult<Backlinks> {
    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    let note = ix
        .db
        .get_note(id)?
        .ok_or_else(|| not_found("no such note"))?;
    let notes = ix.db.backlinks(&note.title)?;
    Ok(Backlinks {
        title: note.title,
        notes,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Related {
    pub note_id: String,
    pub title: String,
    /// Notes that link here — the strongest signal Storm has, and an exact one.
    pub backlinks: Vec<NoteRow>,
    /// Notes sharing at least one tag, with which tags they share.
    pub shared_tags: Vec<RelatedByTag>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedByTag {
    #[serde(flatten)]
    pub note: NoteRow,
    pub tags: Vec<String>,
}

/// Notes related to this one, by link and by tag.
///
/// Deliberately no semantic similarity: both signals here are exact and
/// explainable, and the brief defers embeddings until lexical search actually
/// falls short. A related-notes list that cannot say *why* two notes are
/// related is worse than none.
pub async fn related(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    id: &str,
    limit: i64,
) -> ApiResult<Related> {
    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    let note = ix
        .db
        .get_note(id)?
        .ok_or_else(|| not_found("no such note"))?;

    let backlinks = ix.db.backlinks(&note.title)?;
    let content = ix.vault.read(&note.path)?;
    let own_tags = crate::frontmatter::get_tags(&content);

    // Accumulated per note rather than per tag, so a note sharing three tags
    // appears once carrying all three instead of three times.
    let mut by_note: Vec<RelatedByTag> = Vec::new();
    for tag in &own_tags {
        for row in ix.db.notes_with_tag(tag)? {
            if row.id == note.id {
                continue;
            }
            match by_note.iter_mut().find(|r| r.note.id == row.id) {
                Some(existing) => existing.tags.push(tag.clone()),
                None => by_note.push(RelatedByTag {
                    note: row,
                    tags: vec![tag.clone()],
                }),
            }
        }
    }
    // Most tags in common first — the closest thing to a relevance order that
    // is still fully explainable.
    by_note.sort_by_key(|r| std::cmp::Reverse(r.tags.len()));
    by_note.truncate(limit.max(0) as usize);

    Ok(Related {
        note_id: note.id,
        title: note.title,
        backlinks,
        shared_tags: by_note,
    })
}

// ---- history -----------------------------------------------------------

/// One stored revision, without its content.
///
/// Content is omitted on purpose: `note_versions` holds a full snapshot per
/// version, so a history of a long-lived note would be megabytes, and neither
/// a history list nor an agent deciding what to fetch needs the bodies.
/// `note_version` fetches one.
#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub version: i64,
    pub created_at: String,
    pub device_id: Option<String>,
    pub size: i64,
}

pub async fn note_history(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    id: &str,
) -> ApiResult<Vec<VersionInfo>> {
    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    if ix.db.get_note(id)?.is_none() {
        return Err(not_found("no such note"));
    }
    Ok(ix.db.list_versions(id)?)
}

pub async fn note_version(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    id: &str,
    version: i64,
) -> ApiResult<String> {
    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    ix.db
        .version_content(id, version)?
        .ok_or_else(|| not_found("no such version"))
}

/// Pushes a write to every connected client over the WebSocket.
///
/// In `ops` rather than in a handler because a write that never reaches the
/// other devices is exactly the divergence this module exists to prevent: an
/// agent's edit has to land on the phone the same way a phone's edit lands on
/// the laptop, without waiting for someone to pull to refresh.
pub fn broadcast_latest(state: &Shared, ix: &crate::index::Indexer, seq: i64) {
    if let Ok(Some(change)) = ix
        .db
        .changes_since(seq - 1, 1)
        .map(|c| c.into_iter().next())
    {
        let _ = state.events.send(change);
    }
}

// ---- writes ------------------------------------------------------------
//
// The same three operations the Flutter client performs, reached the same way.
// Nothing here is an MCP-specific write path: an agent's edit goes through the
// identical `base_version` + diff3 merge a phone's does, so two writers racing
// resolve exactly as two devices would.

pub async fn create_note(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    path: &str,
    content: &str,
) -> ApiResult<crate::index::WriteResult> {
    let handle = vault_of(state, actor, Access::Write, vault).await?;
    let mut ix = handle.indexer.lock().await;
    let result = ix
        .create_note(path, content)
        .map_err(|e| crate::api::bad_request(e.to_string()))?;
    broadcast_latest(state, &ix, result.seq);
    record_agent_write(state, actor, vault, &result.note, "created");
    Ok(result)
}

/// The write hook: an agent's successful write becomes a row of its
/// session's Wrote list and the note's provenance. Never fails the write,
/// which is already on disk.
fn record_agent_write(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    note: &crate::db::NoteRow,
    kind: &str,
) {
    let Actor::Agent { session_id, .. } = actor else {
        return;
    };
    let write = crate::agent::store::WriteRecord {
        session_id: session_id.clone(),
        vault_id: vault.to_string(),
        note_id: note.id.clone(),
        kind: kind.to_string(),
        version: note.version,
        at: write_stamp(),
    };
    if let Err(e) = state.agent.record_write(&write) {
        tracing::warn!(error = %e, session = %session_id, "could not record an agent's write");
    }
}

/// Fixed width, so the stamps order as text.
fn write_stamp() -> String {
    let format = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    );
    time::OffsetDateTime::now_utc()
        .format(&format)
        .unwrap_or_else(|_| crate::index::now_rfc3339())
}

/// Replaces a note's content, merging against the version the caller read.
///
/// `base_version` is not optional and there is no "just overwrite" path: it is
/// the whole reason a second writer is safe here. When the note has moved on,
/// the server merges and the result says so — `merged` or `conflict` — and the
/// caller is expected to adopt the returned text rather than resend its own,
/// which is the same contract the Flutter client obeys.
pub async fn update_note(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    id: &str,
    base_version: i64,
    content: &str,
    device_id: Option<&str>,
) -> ApiResult<crate::index::WriteResult> {
    let handle = vault_of(state, actor, Access::Write, vault).await?;
    let mut ix = handle.indexer.lock().await;
    let result = ix
        .put_note(id, base_version, content, device_id)
        .map_err(|e| not_found(e.to_string()))?;
    broadcast_latest(state, &ix, result.seq);
    record_agent_write(state, actor, vault, &result.note, "edited");
    Ok(result)
}

pub async fn delete_note(state: &Shared, actor: &Actor, vault: &str, id: &str) -> ApiResult<i64> {
    let handle = vault_of(state, actor, Access::Write, vault).await?;
    let mut ix = handle.indexer.lock().await;
    let seq = ix.delete_note(id).map_err(|e| not_found(e.to_string()))?;
    broadcast_latest(state, &ix, seq);
    Ok(seq)
}

// ---- search and tags ---------------------------------------------------

pub async fn search(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    query: &str,
    limit: i64,
) -> ApiResult<Vec<SearchHit>> {
    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    // FTS5 treats bare punctuation as syntax; a user typing `foo-bar` should
    // get a search, not a parse error.
    let sanitized = sanitize_fts_query(query);
    if sanitized.is_empty() {
        return Ok(Vec::new());
    }
    ix.db
        .search(&sanitized, limit.clamp(1, 500))
        .map_err(|e| crate::api::bad_request(e.to_string()))
}

/// Quotes each term so FTS5 special characters can't produce a syntax error.
pub fn sanitize_fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|term| term.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

pub async fn list_tags(state: &Shared, actor: &Actor, vault: &str) -> ApiResult<Vec<TagCount>> {
    let handle = vault_of(state, actor, Access::Read, vault).await?;
    let ix = handle.indexer.lock().await;
    Ok(ix
        .db
        .all_tags()?
        .into_iter()
        .map(|(tag, count)| TagCount { tag, count })
        .collect())
}

// ---- recents -----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RecentEntry {
    pub vault_id: String,
    pub vault_name: String,
    pub note_id: String,
    pub path: String,
    pub title: String,
    pub modified: String,
    pub opened_at: String,
}

/// Recently opened notes across every vault.
///
/// One call regardless of vault count. Sorting by the server's `modified`
/// instead would mean fetching every vault's whole tree on every load of the
/// home screen.
pub async fn recents(state: &Shared, actor: &Actor, limit: i64) -> ApiResult<Vec<RecentEntry>> {
    let limit = limit.clamp(1, 200);
    let vaults = state.vaults.read().await;
    let mut all: Vec<RecentEntry> = Vec::new();

    for entry in &vaults.registry.vaults {
        // Filtered, not refused — see `list_vaults`. `/v1/recents` spans every
        // vault, so refusing on one would take the dashboard with it.
        if !crate::api::may_see_vault(state, actor, &entry.id) {
            continue;
        }
        let Some(handle) = vaults.get(&entry.id) else {
            continue;
        };
        let ix = handle.indexer.lock().await;
        // Each vault only has to give up its own top `limit`; the merge below
        // takes the overall newest.
        for row in ix.db.recent_notes(limit)? {
            let RecentRow {
                note_id,
                path,
                title,
                modified,
                opened_at,
            } = row;
            all.push(RecentEntry {
                vault_id: entry.id.clone(),
                vault_name: entry.name.clone(),
                note_id,
                path,
                title,
                modified,
                opened_at,
            });
        }
    }

    all.sort_by(|a, b| b.opened_at.cmp(&a.opened_at));
    all.truncate(limit as usize);
    Ok(all)
}

// ---- scripts (kit vault) ------------------------------------------------
//
// Canonical, agent-run tooling gets exactly one home in Storm: the **kit
// vault**, found by its directory name, addressed by `name` under a `scripts/`
// root. Everything here is scoped to that vault and to a text-only extension
// allowlist — an agent gets no second way to write files anywhere else.
// Scripts are stored as attachments (they are non-markdown files), so the
// indexes, hashes and cross-device sync already work; what this section adds is
// the intent: only scripts, only in kit, only with an allowed extension.

/// The vault script tools are scoped to — the one whose directory is `kit`.
const KIT_VAULT_DIR: &str = "kit";

/// Scripts live under this folder inside the kit vault.
const SCRIPTS_ROOT: &str = "scripts";

/// Extensions agents may write. Text-only, and deliberately **not** `.md`:
/// markdown is the notes domain, and the allowlist is what keeps a tool from
/// dropping an executable no one meant to run.
const SCRIPT_EXTENSIONS: &[&str] = &[
    "ts", "js", "mjs", "cjs", "json", "sh", "py", "yaml", "yml", "toml", "csv",
];

/// One canonical script in the kit vault.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptInfo {
    /// Address for the other script tools, relative to the `scripts/` root.
    pub name: String,
    /// Vault-relative path (`scripts/<name>`), for callers that keep paths.
    pub path: String,
    pub size: i64,
    pub modified: String,
}

/// The result of storing a script.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptStored {
    pub name: String,
    pub path: String,
    pub size: i64,
}

/// A script and its text. Every allowed extension is a text format, so the
/// content never needs base64 — binary blobs cannot be written here.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptContent {
    pub name: String,
    pub path: String,
    pub size: i64,
    pub content: String,
}

/// The kit vault's id, or 404 when it is not registered.
///
/// Found by directory name rather than a hardcoded id, so a fresh server whose
/// registry is built by adopting the `kit/` folder resolves it the same way as
/// one that has it persisted.
async fn kit_vault_id(state: &Shared) -> ApiResult<String> {
    let vaults = state.vaults.read().await;
    vaults
        .registry
        .by_dir(KIT_VAULT_DIR)
        .map(|entry| entry.id.clone())
        .ok_or_else(|| not_found("no vault named “kit”"))
}

/// The kit vault's open handle — the one place the script tools ask whether the
/// caller may touch kit, so no tool can forget to.
async fn kit_handle(
    state: &Shared,
    actor: &Actor,
    access: Access,
) -> ApiResult<Arc<crate::api::VaultHandle>> {
    let id = kit_vault_id(state).await?;
    vault_of(state, actor, access, &id).await
}

/// Validates a script name and returns its vault-relative path under the
/// scripts root. One translation, shared by every script tool, so reads and
/// writes cannot disagree about what a name means.
fn script_path(name: &str) -> Result<String, ApiError> {
    if name.is_empty() || name.starts_with('/') || name.ends_with('/') {
        return Err(bad_request(
            "script name must be non-empty and cannot start or end with '/'",
        ));
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some(ext) if SCRIPT_EXTENSIONS.contains(&ext) => Ok(format!("{SCRIPTS_ROOT}/{name}")),
        _ => Err(bad_request(format!(
            "extension not allowed ('.{}'); scripts may only be: {}",
            extension.unwrap_or_default(),
            SCRIPT_EXTENSIONS.join(", ")
        ))),
    }
}

/// Scripts currently in the kit vault. Filtered by the scripts root **and** the
/// allowlist, so an attachment that happens to live in kit but is not a script
/// — an image, say — stays invisible here.
pub async fn list_scripts(
    state: &Shared,
    actor: &Actor,
    prefix: Option<&str>,
) -> ApiResult<Vec<ScriptInfo>> {
    let handle = kit_handle(state, actor, Access::Read).await?;
    let ix = handle.indexer.lock().await;
    let prefix = prefix.unwrap_or("");
    let mut out = Vec::new();
    for row in ix.db.list_attachments()? {
        let Some(name) = row.path.strip_prefix(&format!("{SCRIPTS_ROOT}/")) else {
            continue;
        };
        if !name.starts_with(prefix) {
            continue;
        }
        let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !SCRIPT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        out.push(ScriptInfo {
            name: name.to_string(),
            path: row.path.clone(),
            size: row.size,
            modified: row.modified,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// One script's full text.
pub async fn get_script(state: &Shared, actor: &Actor, name: &str) -> ApiResult<ScriptContent> {
    let rel = script_path(name)?;
    let handle = kit_handle(state, actor, Access::Read).await?;
    let ix = handle.indexer.lock().await;
    let bytes = ix
        .vault
        .read_bytes(&rel)
        .map_err(|_| not_found("no such script"))?;
    let content = String::from_utf8(bytes).map_err(|_| bad_request("script is not valid UTF-8"))?;
    Ok(ScriptContent {
        name: name.to_string(),
        path: rel,
        size: content.len() as i64,
        content,
    })
}

/// Stores a new script. Refuses a name that already exists, so a re-run cannot
/// silently clobber canonical tooling — change one with [`update_script`].
pub async fn create_script(
    state: &Shared,
    actor: &Actor,
    name: &str,
    content: &str,
) -> ApiResult<ScriptStored> {
    let rel = script_path(name)?;
    let handle = kit_handle(state, actor, Access::Write).await?;
    let mut ix = handle.indexer.lock().await;
    if ix.vault.exists(&rel) {
        return Err(conflict(format!("script “{name}” already exists")));
    }
    ix.put_attachment(&rel, content.as_bytes())
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(ScriptStored {
        name: name.to_string(),
        path: rel,
        size: content.len() as i64,
    })
}

/// Replaces an existing script's text. The mirror of [`create_script`]: fails
/// on a name that does not exist, which keeps the two from being interchangeable.
pub async fn update_script(
    state: &Shared,
    actor: &Actor,
    name: &str,
    content: &str,
) -> ApiResult<ScriptStored> {
    let rel = script_path(name)?;
    let handle = kit_handle(state, actor, Access::Write).await?;
    let mut ix = handle.indexer.lock().await;
    if !ix.vault.exists(&rel) {
        return Err(not_found("no such script"));
    }
    ix.put_attachment(&rel, content.as_bytes())
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(ScriptStored {
        name: name.to_string(),
        path: rel,
        size: content.len() as i64,
    })
}

// ---- server identity ---------------------------------------------------

/// What a client needs to pin this server: who it is, and which key to check.
///
/// Public by design — this is the `none` tier, answered before any credential
/// exists. It carries nothing an attacker on the LAN does not already learn by
/// connecting, and a client that cannot read it cannot pair.
#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    pub server_id: String,
    pub name: String,
    pub key_id: String,
    pub algorithm: String,
    /// base64url, no padding.
    pub public_key: String,
    /// The relays this server is **currently registered with** (SRP v1 §4.4).
    ///
    /// Appended after the existing fields, and only ever appended: an older
    /// client parses this response by name and must not break on a key it does
    /// not know, so the shape above stays exactly where it was.
    ///
    /// Empty on every server today — nothing registers with a relay yet. That
    /// is the honest answer, not a placeholder: an empty list means "no relay
    /// path to me", which is true.
    pub relays: Vec<RelayAdvert>,
}

/// One reachable relay path, as a client should read it.
#[derive(Debug, Clone, Serialize)]
pub struct RelayAdvert {
    /// The relay's own base URL. Identity, so a client can match this entry
    /// against one it already knows from its pairing payload instead of
    /// treating a refreshed list as a set of strangers.
    pub url: String,
    /// `wss://<relay-host>/connect/<server_id>` — **derived, not allocated**.
    /// Sent because it is what the client dials, not because it is stored;
    /// any client holding the `server_id` above can rebuild it byte for byte.
    pub public_address: String,
}

/// Who this server is, and how to reach it right now.
///
/// **Why the relay set is here.** A client learns its server's addresses from
/// the pairing payload, and that payload is frozen at the moment the QR was
/// issued. A server that later changes relays while a client is off the LAN
/// would strand that client for good. This endpoint is the live carrier: it is
/// the `none` tier, it is already on the identity-challenge path, and a client
/// refreshes from it whenever the server is reachable by *any* path at all —
/// including a relay it already knows.
///
/// **Registered, never merely configured.** The set comes from
/// `registered_relays`, not from `Registry::relays`. A relay the server failed
/// to register with is a dead path, and a client races its candidates on a
/// ~2 s budget — one dead entry costs part of that budget on every reconnect.
pub async fn server_info(state: &Shared) -> ApiResult<ServerInfo> {
    let identity = &state.identity;
    let registered = {
        let vaults = state.vaults.read().await;
        vaults.registry.registered_relays.snapshot()
    };
    let relays = registered
        .into_iter()
        .map(|url| RelayAdvert {
            public_address: crate::registry::public_address(&url, &identity.server_id),
            url,
        })
        .collect();

    Ok(ServerInfo {
        server_id: identity.server_id.clone(),
        name: identity.name.clone(),
        key_id: identity.key_id.clone(),
        algorithm: crate::auth::identity::ALGORITHM.to_string(),
        public_key: identity.public_key_b64(),
        relays,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ChallengeAnswer {
    pub server_id: String,
    pub key_id: String,
    /// base64url, no padding, over
    /// `storm-challenge:v1:<server_id>:<nonce>` — never over the bare nonce.
    pub signature: String,
}

/// Proves the host the client actually reached holds the private half of the
/// key it read out of a QR.
///
/// The QR itself cannot be signed — it carries the very key a signature would
/// be checked with — so this round trip is what turns "I was shown a public
/// key" into "this address holds it".
pub async fn sign_challenge(state: &Shared, nonce: &str) -> ApiResult<ChallengeAnswer> {
    crate::auth::identity::validate_nonce(nonce).map_err(bad_request)?;
    Ok(ChallengeAnswer {
        server_id: state.identity.server_id.clone(),
        key_id: state.identity.key_id.clone(),
        signature: state.identity.sign_challenge(nonce),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingQrPayload {
    pub sid: String,
    pub pk: String,
    pub n: String,
    pub exp: String,
    pub addr: String,
}

/// Creates a new pairing session and returns the QR payload.
///
/// Called by `POST /v1/pairings` (session tier). The client renders this as a
/// QR code for the new device to scan.
pub async fn issue_pairing_qr(
    state: &Shared,
    purpose: &str,
    user_id: Option<&str>,
    peer_ip: Option<&str>,
) -> ApiResult<PairingQrPayload> {
    let purpose = crate::auth::pairing::PairingPurpose::from_str(purpose)
        .map_err(|e| bad_request(e.to_string()))?;
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let (nonce, session) =
        crate::auth::pairing::create(&mut auth_db, purpose, user_id, peer_ip, &now)
            .map_err(|e| ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let qr = crate::auth::pairing::encode_qr(
        &state.identity.server_id,
        &state.identity.public_key_b64(),
        &nonce,
        &session.expires,
        &state.listen_addr,
    );

    Ok(PairingQrPayload {
        sid: qr.sid,
        pk: qr.pk,
        n: qr.n,
        exp: qr.exp,
        addr: qr.addr,
    })
}

// ---- MCP keys (A14) ----------------------------------------------------

/// A freshly minted key, **including the plaintext**.
///
/// The only type in Storm that carries one. It exists for exactly one response
/// and is never persisted, logged or returned again (A14.5).
#[derive(Debug, Clone, Serialize)]
pub struct CreatedApiKey {
    #[serde(flatten)]
    pub key: crate::auth::keys::ApiKey,
    /// Shown once. There is no endpoint that can produce this value again.
    pub secret: String,
}

/// Mints a key for the caller (A14). The plaintext is in the return value and
/// nowhere else.
pub async fn create_api_key(
    state: &Shared,
    actor: &Actor,
    name: &str,
    expires: Option<&str>,
    created_via: Option<&str>,
) -> ApiResult<CreatedApiKey> {
    let owner = actor.user_id();
    crate::auth::keys::validate_name(name).map_err(bad_request)?;

    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let (key, secret) =
        crate::auth::keys::create(&mut auth_db, owner, name, created_via, expires, &now)
            .map_err(|e| bad_request(e.to_string()))?;

    Ok(CreatedApiKey { key, secret })
}

/// Lists the account's keys.
pub async fn list_api_keys(
    state: &Shared,
    actor: &Actor,
) -> ApiResult<Vec<crate::auth::keys::ApiKey>> {
    let auth_db = state.auth_db.lock().await;
    auth_db
        .api_keys_for_user(actor.user_id())
        .map_err(|e| ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Revokes a key. Effective on the next request, not the next restart.
pub async fn revoke_api_key(state: &Shared, actor: &Actor, key_id: &str) -> ApiResult<()> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;

    let key = auth_db
        .api_key_by_id(key_id)
        .map_err(|e| ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| not_found("no such key"))?;

    if key.user_id != actor.user_id() {
        return Err(not_found("no such key"));
    }

    // The audit row is written inside `keys::revoke`, beside the act.
    crate::auth::keys::revoke(
        &mut auth_db,
        key_id,
        Some(actor.user_id()),
        "revoked from the app",
        &now,
    )
    .map_err(|e| ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(())
}

// ---- Agent Runtime: Runtime Hosts (decision 77b) ---------------------------

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// A host as the client sees it.
#[derive(Debug, Clone, Serialize)]
pub struct HostView {
    #[serde(flatten)]
    pub host: crate::auth::hosts::Host,
    /// `online`, `offline` or `revoked` (freeze §5.5).
    pub status: &'static str,
    /// What the host offers, while it is online.
    pub capabilities: Option<crate::agent::Capabilities>,
    /// Sessions inherit the host's network policy in V1 (freeze §12.2); the
    /// client shows it, so the record says it.
    pub egress: &'static str,
}

fn host_view(state: &Shared, host: crate::auth::hosts::Host) -> HostView {
    let live = state.agent.host_live(&host.id);
    let status = if host.is_revoked() {
        "revoked"
    } else if live.online {
        "online"
    } else {
        "offline"
    };
    HostView {
        host,
        status,
        capabilities: live.capabilities,
        egress: "host",
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IssuedEnrollment {
    /// The string the owner pastes into `storm-runtime enroll`. Shown once:
    /// it carries the single-use token.
    pub enrollment: String,
    pub expires: String,
}

pub async fn issue_host_enrollment(
    state: &Shared,
    actor: &Actor,
    server_url: &str,
) -> ApiResult<IssuedEnrollment> {
    crate::auth::hosts::validate_server_url(server_url).map_err(bad_request)?;
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let (enrollment, token) =
        crate::auth::hosts::issue_enrollment(&mut auth_db, actor.user_id(), &now)
            .map_err(internal)?;
    Ok(IssuedEnrollment {
        enrollment: crate::auth::hosts::enrollment_string(
            server_url.trim_end_matches('/'),
            &state.identity.server_id,
            &state.identity.public_key_b64(),
            &token,
        ),
        expires: enrollment.expires,
    })
}

pub async fn list_hosts(state: &Shared) -> ApiResult<Vec<HostView>> {
    let auth_db = state.auth_db.lock().await;
    Ok(auth_db
        .list_hosts()
        .map_err(internal)?
        .into_iter()
        .map(|h| host_view(state, h))
        .collect())
}

pub async fn rename_host(
    state: &Shared,
    actor: &Actor,
    host_id: &str,
    name: &str,
) -> ApiResult<HostView> {
    crate::auth::hosts::validate_name(name).map_err(bad_request)?;
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    crate::auth::hosts::rename(&mut auth_db, host_id, name, actor.user_id(), &now)
        .map_err(internal)?
        .map(|h| host_view(state, h))
        .ok_or_else(|| not_found("no such host"))
}

/// Revokes a host: its tokens die now, and it cannot authenticate again.
pub async fn revoke_host(state: &Shared, actor: &Actor, host_id: &str) -> ApiResult<()> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    if !crate::auth::hosts::revoke(&mut auth_db, host_id, actor.user_id(), &now)
        .map_err(internal)?
    {
        return Err(not_found("no such host"));
    }
    drop(auth_db);
    // Its link closes and its sessions fail now (freeze §5.6); the host ends
    // them itself when it is next refused.
    state.agent.revoke_host(host_id).map_err(internal)?;
    sweep_gateway_sessions(state);
    Ok(())
}

/// A host's refusal: **one generic answer to the host, a specific audit row**.
/// Telling an unauthenticated caller whether a token was unknown, used or
/// expired is free reconnaissance (the `keys` rule).
fn host_refusal(
    auth_db: &crate::auth::AuthDb,
    error: crate::auth::hosts::HostError,
    host_id: Option<&str>,
    remote: Option<&str>,
    now: &str,
) -> ApiError {
    use crate::auth::hosts::{HostError, HostFailure};
    match error {
        HostError::Refused(failure) => {
            let detail = serde_json::json!({ "reason": failure.code(), "host_id": host_id });
            let _ = auth_db.record_event_from(
                crate::auth::hosts::EVENT_HOST_AUTH_REJECTED,
                None,
                None,
                remote,
                now,
                &detail.to_string(),
            );
            if failure == HostFailure::Malformed {
                bad_request("malformed request")
            } else {
                ApiError(axum::http::StatusCode::UNAUTHORIZED, "refused".into())
            }
        }
        HostError::Internal(e) => internal(e),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrolledHost {
    pub host_id: String,
    pub server_id: String,
}

pub struct EnrollHost<'a> {
    pub token: &'a str,
    pub public_key: &'a str,
    pub key_id: &'a str,
    pub name: &'a str,
    pub signature: &'a str,
}

pub async fn enroll_host(
    state: &Shared,
    req: EnrollHost<'_>,
    remote: Option<&str>,
) -> ApiResult<EnrolledHost> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let request = crate::auth::hosts::EnrollRequest {
        token: req.token,
        public_key: req.public_key,
        key_id: req.key_id,
        name: req.name,
        signature: req.signature,
    };
    match crate::auth::hosts::enroll(&mut auth_db, &state.identity.server_id, &request, &now) {
        Ok(host) => Ok(EnrolledHost {
            host_id: host.id,
            server_id: state.identity.server_id.clone(),
        }),
        Err(e) => Err(host_refusal(&auth_db, e, None, remote, &now)),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HostChallenge {
    pub nonce: String,
    pub expires: String,
}

pub async fn host_challenge(
    state: &Shared,
    host_id: &str,
    remote: Option<&str>,
) -> ApiResult<HostChallenge> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    match crate::auth::hosts::issue_challenge(&mut auth_db, host_id, &now) {
        Ok((nonce, expires)) => Ok(HostChallenge { nonce, expires }),
        Err(e) => Err(host_refusal(&auth_db, e, Some(host_id), remote, &now)),
    }
}

pub async fn host_authenticate(
    state: &Shared,
    host_id: &str,
    nonce: &str,
    signature: &str,
    remote: Option<&str>,
) -> ApiResult<crate::auth::hosts::IssuedHostToken> {
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    match crate::auth::hosts::authenticate_key(
        &mut auth_db,
        &state.identity.server_id,
        host_id,
        nonce,
        signature,
        &now,
    ) {
        Ok(issued) => Ok(issued),
        Err(e) => Err(host_refusal(&auth_db, e, Some(host_id), remote, &now)),
    }
}

// ---- Agent Runtime: sessions and the host link (decision 77c) ----------

/// The Agent Manager's errors as HTTP. One place, so a route cannot answer
/// "host offline" with anything but 503.
fn agent_error(e: crate::agent::AgentError) -> ApiError {
    use crate::agent::AgentError as E;
    use axum::http::StatusCode as S;
    match e {
        E::NotFound(m) => not_found(m),
        E::BadRequest(m) => bad_request(m),
        E::Conflict(m) => conflict(m),
        E::HostOffline => ApiError(S::SERVICE_UNAVAILABLE, "the host is offline".into()),
        E::TooManySessions => ApiError(
            S::TOO_MANY_REQUESTS,
            "the host is running its maximum number of sessions".into(),
        ),
        E::NoProvider => ApiError(
            S::UNPROCESSABLE_ENTITY,
            "the host has no supported provider installed".into(),
        ),
        E::NotYours => ApiError(S::FORBIDDEN, "not a session on this host".into()),
        E::Internal(e) => internal(e),
    }
}

/// A host's workspaces, each with its live-session count (freeze §8).
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub name: String,
    pub live_sessions: usize,
}

pub async fn host_workspaces(state: &Shared, host_id: &str) -> ApiResult<Vec<WorkspaceView>> {
    state.agent.refresh(host_id);
    let caps = state.agent.host_live(host_id).capabilities.ok_or_else(|| {
        ApiError(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "the host is offline".into(),
        )
    })?;
    let counts = state.agent.live_counts(host_id).map_err(agent_error)?;
    Ok(caps
        .workspaces
        .into_iter()
        .map(|name| WorkspaceView {
            live_sessions: counts.get(&name).copied().unwrap_or(0),
            name,
        })
        .collect())
}

/// A session as clients see it: the record plus what it was launched with
/// and how many notes it has written.
#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    #[serde(flatten)]
    pub session: crate::agent::store::SessionRecord,
    pub name: String,
    pub context: Option<crate::agent::store::Context>,
    pub write_vault_id: Option<String>,
    pub wrote_count: i64,
}

fn session_view(
    state: &Shared,
    session: crate::agent::store::SessionRecord,
) -> ApiResult<SessionView> {
    let launch = state.agent.launch_of(&session.id).map_err(internal)?;
    let wrote_count = state.agent.write_count(&session.id).map_err(internal)?;
    // A session launched before names existed is called by its workspace.
    let (name, context, write_vault_id) = match launch {
        Some(l) => (l.name, l.context, l.write_vault_id),
        None => (session.workspace.clone(), None, None),
    };
    Ok(SessionView {
        session,
        name,
        context,
        write_vault_id,
        wrote_count,
    })
}

/// A launched session, plus what the MCP Gateway granted it (spec §6).
#[derive(Debug, Clone, Serialize)]
pub struct LaunchedSession {
    #[serde(flatten)]
    pub session: SessionView,
    pub mcp: LaunchMcp,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchMcp {
    /// The granted connections, the built-in `storm` among them.
    pub connections: Vec<crate::agent::store::McpGrant>,
    /// Whether the session has a write vault; kept for older clients.
    pub allow_vault_writes: bool,
    /// Said out loud when a session gets less than it asked for: the host is
    /// too old to bridge, or an older client asked for writes with no vault.
    pub notice: Option<String>,
}

pub async fn launch_session(
    state: &Shared,
    actor: &Actor,
    req: crate::agent::Launch,
) -> ApiResult<LaunchedSession> {
    // The host must exist and be live in auth.db, not merely connected.
    let host_name = {
        let auth_db = state.auth_db.lock().await;
        match auth_db.host_by_id(&req.host_id).map_err(internal)? {
            Some(h) if !h.is_revoked() => h.name,
            _ => return Err(not_found("no such host")),
        }
    };
    let context = match &req.context {
        Some(c) => {
            let note = get_note(state, actor, &c.vault_id, &c.note_id).await?;
            Some(crate::agent::store::Context {
                vault_id: c.vault_id.clone(),
                note_id: note.note.id,
                title: note.note.title,
            })
        }
        None => None,
    };
    if let Some(v) = &req.write_vault_id {
        vault_of(state, actor, Access::Write, v).await?;
    }
    let unhonoured_writes = req.allow_vault_writes && req.write_vault_id.is_none();
    let meta = crate::agent::LaunchMeta {
        context,
        write_vault_id: req.write_vault_id.clone(),
    };
    // Every non-disabled connection of the owner, plus `storm` (spec §6,
    // G-D9). Snapshotted here; the manager drops them for `shell` and for a
    // host that cannot bridge.
    let mut offered = vec![crate::agent::store::McpGrant {
        id: BUILTIN_ID.into(),
        slug: BUILTIN_SLUG.into(),
    }];
    offered.extend(
        gateway_store(state)
            .connections_of(actor.user_id())
            .map_err(internal)?
            .into_iter()
            .filter(|c| c.status != status::DISABLED)
            .map(|c| crate::agent::store::McpGrant {
                id: c.id,
                slug: c.slug,
            }),
    );
    let (session, launch, outcome) = state
        .agent
        .launch(actor.user_id(), req, meta, offered)
        .map_err(agent_error)?;
    let (connections, notice) = match outcome {
        crate::agent::McpOutcome::Granted(g) => (
            g,
            unhonoured_writes.then(|| {
                "This app is out of date, so the session is read only. Update Storm to \
                 choose a vault it may write to."
                    .to_string()
            }),
        ),
        crate::agent::McpOutcome::Shell => (Vec::new(), None),
        crate::agent::McpOutcome::OldHost => (
            Vec::new(),
            Some(format!(
                "{host_name} can't use integrations — update storm-runtime"
            )),
        ),
    };
    if !connections.is_empty() {
        let auth_db = state.auth_db.lock().await;
        let detail = serde_json::json!({
            "session_id": session.id,
            "host_id": session.host_id,
            "connections": connections.iter().map(|g| &g.id).collect::<Vec<_>>(),
            "write_vault_id": launch.write_vault_id,
        });
        let _ = auth_db.record_event(
            "integration_grant",
            Some(actor.user_id()),
            None,
            &crate::index::now_rfc3339(),
            &detail.to_string(),
        );
    }
    Ok(LaunchedSession {
        session: SessionView {
            session,
            name: launch.name,
            context: launch.context,
            write_vault_id: launch.write_vault_id.clone(),
            wrote_count: 0,
        },
        mcp: LaunchMcp {
            connections,
            allow_vault_writes: launch.write_vault_id.is_some(),
            notice,
        },
    })
}

pub async fn list_sessions(state: &Shared) -> ApiResult<Vec<SessionView>> {
    state
        .agent
        .list()
        .map_err(agent_error)?
        .into_iter()
        .map(|s| session_view(state, s))
        .collect()
}

pub async fn get_session(state: &Shared, id: &str) -> ApiResult<SessionView> {
    session_view(state, state.agent.get(id).map_err(agent_error)?)
}

/// One note a session wrote, as its Wrote list shows it. `title` and `path`
/// are the note's current ones; `None` once the note is gone.
#[derive(Debug, Clone, Serialize)]
pub struct SessionWrite {
    pub vault_id: String,
    pub note_id: String,
    pub title: Option<String>,
    pub path: Option<String>,
    pub kind: String,
    pub version: i64,
    pub at: String,
}

/// The notes a session created or edited, newest write first.
pub async fn session_writes(
    state: &Shared,
    actor: &Actor,
    id: &str,
) -> ApiResult<Vec<SessionWrite>> {
    state.agent.get(id).map_err(agent_error)?;
    let writes = state.agent.writes_of(id).map_err(internal)?;
    let mut out = Vec::with_capacity(writes.len());
    for w in writes {
        if !crate::api::may_see_vault(state, actor, &w.vault_id) {
            continue;
        }
        let row = match state.vaults.read().await.get(&w.vault_id) {
            Some(handle) => handle.indexer.lock().await.db.get_note(&w.note_id)?,
            None => None,
        };
        out.push(SessionWrite {
            title: row.as_ref().map(|r| r.title.clone()),
            path: row.map(|r| r.path),
            vault_id: w.vault_id,
            note_id: w.note_id,
            kind: w.kind,
            version: w.version,
            at: w.at,
        });
    }
    Ok(out)
}

pub async fn end_session(state: &Shared, id: &str) -> ApiResult<()> {
    state.agent.end(id).map_err(agent_error)
}

pub async fn dismiss_session(state: &Shared, id: &str) -> ApiResult<()> {
    state.agent.dismiss(id).map_err(agent_error)
}

pub async fn session_input(state: &Shared, id: &str, bytes: &[u8]) -> ApiResult<()> {
    state.agent.input(id, bytes).map_err(agent_error)
}

pub async fn session_resize(
    state: &Shared,
    id: &str,
    size: crate::agent::TerminalSize,
) -> ApiResult<()> {
    state.agent.resize(id, size).map_err(agent_error)
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentConfigView {
    pub default_provider: String,
}

pub async fn agent_config(state: &Shared) -> ApiResult<AgentConfigView> {
    Ok(AgentConfigView {
        default_provider: state.agent.default_provider(),
    })
}

/// Sets the global default provider (freeze §6). Audited.
pub async fn set_agent_config(
    state: &Shared,
    actor: &Actor,
    default_provider: &str,
) -> ApiResult<AgentConfigView> {
    let valid = (1..=32).contains(&default_provider.len())
        && !default_provider.starts_with('-')
        && default_provider
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        return Err(bad_request(
            "a provider id is 1–32 lowercase letters, digits and '-'",
        ));
    }
    state
        .agent
        .set_default_provider(default_provider)
        .map_err(internal)?;
    let now = crate::index::now_rfc3339();
    let auth_db = state.auth_db.lock().await;
    let _ = auth_db.record_event(
        "agent_default_provider_changed",
        Some(actor.user_id()),
        None,
        &now,
        &serde_json::json!({ "default_provider": default_provider }).to_string(),
    );
    Ok(AgentConfigView {
        default_provider: default_provider.to_string(),
    })
}

// What a host does on its link. No owner check: the `Host` tier is the
// boundary, and the manager checks that a host posts only for its own sessions.

pub async fn runtime_hello(
    state: &Shared,
    host_id: &str,
    hello: crate::agent::Hello,
) -> ApiResult<()> {
    {
        let auth_db = state.auth_db.lock().await;
        let _ = auth_db.touch_host(host_id, &crate::index::now_rfc3339());
    }
    state.agent.hello(host_id, hello).map_err(agent_error)?;
    sweep_gateway_sessions(state);
    Ok(())
}

pub fn runtime_inventory(
    state: &Shared,
    host_id: &str,
    caps: crate::agent::Capabilities,
) -> ApiResult<()> {
    state.agent.inventory(host_id, caps).map_err(agent_error)
}

pub fn runtime_output(
    state: &Shared,
    host_id: &str,
    session: &str,
    offset: u64,
    data_b64: &str,
) -> ApiResult<()> {
    let bytes = data_encoding::BASE64
        .decode(data_b64.as_bytes())
        .map_err(|_| bad_request("data must be base64"))?;
    state
        .agent
        .host_output(host_id, session, offset, &bytes)
        .map_err(agent_error)
}

pub fn runtime_status(
    state: &Shared,
    host_id: &str,
    session: &str,
    report: &crate::agent::StatusReport,
) -> ApiResult<()> {
    state
        .agent
        .host_status(host_id, session, report)
        .map_err(agent_error)?;
    sweep_gateway_sessions(state);
    Ok(())
}

// ---- MCP Gateway: integrations (decisions 81, 81c) -------------------------
//
// No MCP tool manages an integration (spec §14) and the routes are session
// tier, so an `stk_` key never can.

use crate::gateway::connections::{
    self as conn, BUILTIN_ID, BUILTIN_SLUG, Connection, StaticCredential, auth_kind,
    credential_kind, status,
};

/// A connection as the owner's client sees it. **Never a credential**: only
/// whether one is held.
#[derive(Debug, Clone, Serialize)]
pub struct IntegrationView {
    pub id: String,
    pub slug: String,
    pub display_name: String,
    /// `None` for the built-in connection, which has no network hop.
    pub url: Option<String>,
    pub auth_kind: String,
    pub status: String,
    pub builtin: bool,
    pub has_credential: bool,
    pub tool_allowlist: Vec<String>,
    pub known_tools: Option<Vec<String>>,
    /// Tools that appeared since the owner last reviewed this connection's
    /// tools: off, and shown as "N new tools — review" (spec §9). Upstream
    /// names, so a client cleans them before display.
    pub new_tools: Vec<String>,
    pub expose_resources: bool,
    pub expose_prompts: bool,
    pub upstream_account_label: Option<String>,
    pub last_ok: Option<String>,
    pub last_error_code: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Built-in only: whether agents may write at all (`agent_writes`); a
    /// session also needs a write vault chosen at launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_writes_available: Option<bool>,
}

fn builtin_view(state: &Shared) -> IntegrationView {
    IntegrationView {
        id: BUILTIN_ID.into(),
        slug: BUILTIN_SLUG.into(),
        display_name: "Storm vaults".into(),
        url: None,
        auth_kind: auth_kind::NONE.into(),
        status: status::CONNECTED.into(),
        builtin: true,
        has_credential: false,
        tool_allowlist: Vec::new(),
        known_tools: None,
        new_tools: Vec::new(),
        expose_resources: false,
        expose_prompts: false,
        upstream_account_label: None,
        last_ok: None,
        last_error_code: None,
        created_at: None,
        updated_at: None,
        vault_writes_available: Some(
            state
                .agent_writes
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
    }
}

fn integration_view(
    c: Connection,
    has_credential: bool,
    new_tools: Vec<String>,
) -> IntegrationView {
    IntegrationView {
        id: c.id,
        slug: c.slug,
        display_name: c.display_name,
        url: Some(c.url),
        auth_kind: c.auth_kind,
        status: c.status,
        builtin: false,
        has_credential,
        tool_allowlist: c.tool_allowlist,
        known_tools: c.known_tools,
        new_tools,
        expose_resources: c.expose_resources,
        expose_prompts: c.expose_prompts,
        upstream_account_label: c.upstream_account_label,
        last_ok: c.last_ok,
        last_error_code: c.last_error_code,
        created_at: Some(c.created_at),
        updated_at: Some(c.updated_at),
        vault_writes_available: None,
    }
}

fn view_of(state: &Shared, c: Connection) -> ApiResult<IntegrationView> {
    let (held, new_tools) = {
        let store = gateway_store(state);
        (
            store.has_credential(&c.id).map_err(internal)?,
            store.new_tools(&c.id).map_err(internal)?,
        )
    };
    Ok(integration_view(c, held, new_tools))
}

/// The gateway's store. A `std::sync::Mutex`: never hold the guard across an
/// `.await` (the `agent/` rule).
pub(crate) fn gateway_store(
    state: &Shared,
) -> std::sync::MutexGuard<'_, crate::gateway::store::GatewayDb> {
    state.gateway.store.lock().expect("gateway store lock")
}

/// The caller's own live connection, or `404` — for someone else's too, so an
/// owner probing ids learns nothing about another owner's integrations.
fn own_connection(state: &Shared, actor: &Actor, id: &str) -> ApiResult<Connection> {
    let store = gateway_store(state);
    match store.connection(id).map_err(internal)? {
        Some(c) if c.owner_user_id == actor.user_id() && c.status != status::REVOKED => Ok(c),
        _ => Err(not_found("no such integration")),
    }
}

/// The audit detail for an integration event: ids, slug, kind and the
/// upstream's **host only**. Never the URL — an owner can paste one with a key
/// in its query string — and never a credential.
fn integration_event(
    state_auth: &crate::auth::AuthDb,
    kind: &str,
    actor: &Actor,
    c: &Connection,
    now: &str,
) {
    let host = c
        .url
        .parse::<axum::http::Uri>()
        .ok()
        .and_then(|u| u.host().map(str::to_string));
    let detail = serde_json::json!({
        "connection_id": c.id,
        "slug": c.slug,
        "auth_kind": c.auth_kind,
        "upstream_host": host,
    });
    let _ = state_auth.record_event(kind, Some(actor.user_id()), None, now, &detail.to_string());
}

fn seal_static(
    state: &Shared,
    id: &str,
    credential: &StaticCredential,
) -> ApiResult<crate::gateway::crypto::Sealed> {
    state
        .gateway
        .keys
        .seal_json(id, credential_kind::STATIC, credential)
        .map_err(internal)
}

/// Every integration the owner has, the built-in `storm` connection first.
pub async fn list_integrations(state: &Shared, actor: &Actor) -> ApiResult<Vec<IntegrationView>> {
    let rows = gateway_store(state)
        .connections_of(actor.user_id())
        .map_err(internal)?;
    let mut out = vec![builtin_view(state)];
    for c in rows {
        out.push(view_of(state, c)?);
    }
    Ok(out)
}

pub async fn get_integration(
    state: &Shared,
    actor: &Actor,
    id: &str,
) -> ApiResult<IntegrationView> {
    if id == BUILTIN_ID {
        return Ok(builtin_view(state));
    }
    let c = own_connection(state, actor, id)?;
    view_of(state, c)
}

/// The request body of `POST /v1/integrations/connections`, as is.
#[derive(Deserialize)]
pub struct NewIntegration {
    pub display_name: String,
    #[serde(default)]
    pub slug: Option<String>,
    pub url: String,
    pub auth_kind: String,
    #[serde(default)]
    pub credential: Option<StaticCredential>,
}

/// Connects an integration. V1 here takes `static` (a header, G-D24's PAT)
/// and `none`; `oauth` arrives with its authorize flow (81g).
pub async fn create_integration(
    state: &Shared,
    actor: &Actor,
    req: NewIntegration,
) -> ApiResult<IntegrationView> {
    conn::validate_display_name(&req.display_name).map_err(bad_request)?;
    let slug = req
        .slug
        .clone()
        .unwrap_or_else(|| conn::slug_from(&req.display_name));
    conn::validate_slug(&slug).map_err(bad_request)?;
    conn::validate_url(&req.url, state.gateway.allow_http_upstreams()).map_err(bad_request)?;
    match (req.auth_kind.as_str(), &req.credential) {
        (auth_kind::STATIC, Some(c)) => {
            conn::validate_static(&c.header, &c.value).map_err(bad_request)?
        }
        (auth_kind::STATIC, None) => {
            return Err(bad_request("a static integration needs a credential"));
        }
        (auth_kind::NONE, None) => {}
        (auth_kind::NONE, Some(_)) => {
            return Err(bad_request(
                "an integration without auth takes no credential",
            ));
        }
        (auth_kind::OAUTH, None) => {}
        (auth_kind::OAUTH, Some(_)) => {
            return Err(bad_request(
                "an OAuth integration is authorized, not given a credential",
            ));
        }
        _ => return Err(bad_request("auth_kind is static or none")),
    }

    let now = crate::index::now_rfc3339();
    let c = Connection {
        id: crate::auth::identity::random_id("mcc_"),
        owner_user_id: actor.user_id().to_string(),
        slug,
        display_name: req.display_name.trim().to_string(),
        url: req.url,
        // An OAuth connection waits for its authorization (§8 lifecycle).
        status: if req.auth_kind == auth_kind::OAUTH {
            status::PENDING_AUTH.into()
        } else {
            status::CONNECTED.into()
        },
        auth_kind: req.auth_kind,
        tool_allowlist: Vec::new(),
        known_tools: None,
        expose_resources: true,
        expose_prompts: true,
        upstream_account_label: None,
        last_ok: None,
        last_error_code: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let sealed = match &req.credential {
        Some(credential) => Some(seal_static(state, &c.id, credential)?),
        None => None,
    };
    gateway_store(state)
        .insert_connection(&c, sealed.as_ref().map(|s| (credential_kind::STATIC, s)))
        .map_err(|e| match e {
            crate::gateway::store::InsertError::SlugTaken => conflict(e.to_string()),
            crate::gateway::store::InsertError::Other(e) => internal(e),
        })?;
    {
        let auth_db = state.auth_db.lock().await;
        integration_event(&auth_db, "integration_created", actor, &c, &now);
    }
    Ok(integration_view(c, sealed.is_some(), Vec::new()))
}

/// The request body of `PATCH /v1/integrations/connections/{id}`; every
/// field optional.
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct IntegrationPatch {
    pub display_name: Option<String>,
    /// `false` disables (every call refused at once, §6); `true` re-enables.
    pub enabled: Option<bool>,
    pub tool_allowlist: Option<Vec<String>>,
    pub expose_resources: Option<bool>,
    pub expose_prompts: Option<bool>,
    /// A replacement static credential (a rotated PAT).
    pub credential: Option<StaticCredential>,
}

/// Changes what may change. **Never the URL, the slug or the auth kind**: a
/// credential is presented only to its own upstream (AM24), so re-pointing a
/// connection would hand its token to a new host, and live sessions were told
/// the slug at launch.
pub async fn update_integration(
    state: &Shared,
    actor: &Actor,
    id: &str,
    patch: IntegrationPatch,
) -> ApiResult<IntegrationView> {
    if id == BUILTIN_ID {
        return Err(bad_request("the built-in connection cannot be changed"));
    }
    let mut c = own_connection(state, actor, id)?;
    if let Some(name) = &patch.display_name {
        conn::validate_display_name(name).map_err(bad_request)?;
        c.display_name = name.trim().to_string();
    }
    if let Some(tools) = &patch.tool_allowlist {
        conn::validate_allowlist(tools).map_err(bad_request)?;
        let mut tools = tools.clone();
        tools.sort();
        tools.dedup();
        c.tool_allowlist = tools;
    }
    if let Some(on) = patch.expose_resources {
        c.expose_resources = on;
    }
    if let Some(on) = patch.expose_prompts {
        c.expose_prompts = on;
    }
    let mut events: Vec<&str> = Vec::new();
    let sealed = match &patch.credential {
        Some(credential) => {
            if c.auth_kind != auth_kind::STATIC {
                return Err(bad_request("only a static integration takes a credential"));
            }
            conn::validate_static(&credential.header, &credential.value).map_err(bad_request)?;
            if c.status == status::NEEDS_REAUTH || c.status == status::ERROR {
                c.status = status::CONNECTED.into();
            }
            events.push("integration_reauthorized");
            Some(seal_static(state, &c.id, credential)?)
        }
        None => None,
    };
    match patch.enabled {
        Some(false) if c.status != status::DISABLED => {
            c.status = status::DISABLED.into();
            events.push("integration_disabled");
        }
        Some(true) if c.status == status::DISABLED => {
            c.status = status::CONNECTED.into();
            events.push("integration_enabled");
        }
        _ => {}
    }

    let now = crate::index::now_rfc3339();
    c.updated_at = now.clone();
    {
        let mut store = gateway_store(state);
        if let Some(sealed) = &sealed {
            store
                .put_credential(&c.id, credential_kind::STATIC, sealed, None, &now)
                .map_err(internal)?;
        }
        store.update_connection(&c).map_err(internal)?;
        // Saving the tool list is the owner's review of the new tools (§9):
        // they become known, on or off exactly as the list says.
        if patch.tool_allowlist.is_some() {
            store.review_tools(&c.id).map_err(internal)?;
        }
    }
    // A disabled connection, or one whose credential just changed, keeps no
    // live upstream session: the next call opens one with the new state.
    if c.status == status::DISABLED || sealed.is_some() {
        state.gateway.sessions.close_connection(&c.id);
    }
    if !events.is_empty() {
        let auth_db = state.auth_db.lock().await;
        for kind in events {
            integration_event(&auth_db, kind, actor, &c, &now);
        }
    }
    view_of(state, c)
}

/// Disconnects (§13): the connection becomes a `revoked` tombstone and its
/// ciphertexts are deleted at once, so every later call is refused. Upstream
/// revocation (RFC 7009) is best effort and arrives with OAuth (81g); a
/// static token such as a GitHub PAT has no revocation call from Storm, and
/// the owner revokes it upstream.
pub async fn delete_integration(state: &Shared, actor: &Actor, id: &str) -> ApiResult<()> {
    if id == BUILTIN_ID {
        return Err(bad_request("the built-in connection cannot be deleted"));
    }
    let c = own_connection(state, actor, id)?;
    // Read before the ciphertexts go: what an RFC 7009 revocation would send.
    let revocation = if c.auth_kind == auth_kind::OAUTH {
        state.gateway.revocation_for(&c).await
    } else {
        None
    };
    let now = crate::index::now_rfc3339();
    gateway_store(state)
        .revoke_connection(&c.id, &now)
        .map_err(internal)?;
    // Best effort, upstream (§13); the connection is already gone here.
    if let Some(r) = revocation {
        let gateway = state.gateway.clone();
        tokio::spawn(async move { gateway.revoke(r).await });
    }
    // Its grants die with it (§13; the rows stay for the audit), and its live
    // upstream sessions close, so the next call is refused at once.
    state.agent.revoke_grants_for(&c.id).map_err(internal)?;
    state.gateway.sessions.close_connection(&c.id);
    let auth_db = state.auth_db.lock().await;
    integration_event(&auth_db, "integration_deleted", actor, &c, &now);
    Ok(())
}

/// What the owner's `test` learns. **Never the upstream's error text**: only
/// the stable code (§12).
#[derive(Debug, Clone, Serialize)]
pub struct IntegrationTest {
    pub ok: bool,
    pub error_code: Option<&'static str>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub tool_count: usize,
    /// Tools awaiting the owner's review: off until the owner turns them on
    /// (G-D16), and listed here until the owner saves the tool list (§9).
    pub new_tools: Vec<String>,
    pub integration: IntegrationView,
}

/// One upstream tool as the owner's allowlist editor sees it.
#[derive(Debug, Clone, Serialize)]
pub struct IntegrationTool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub allowed: bool,
    pub new: bool,
}

/// Runs the owner's probe against a connection and records what it found:
/// `last_ok` or `last_error_code`, `needs_reauth` on a refused credential,
/// the allowlist rule for new tools, and one metadata-only audit row.
async fn probe_integration(
    state: &Shared,
    actor: &Actor,
    id: &str,
) -> ApiResult<(
    Connection,
    Result<crate::gateway::upstream::ProbeResult, crate::gateway::upstream::UpstreamError>,
)> {
    if id == BUILTIN_ID {
        return Err(bad_request("the built-in connection is served in process"));
    }
    let mut c = own_connection(state, actor, id)?;
    if c.status == status::DISABLED {
        return Err(conflict("the integration is disabled; enable it first"));
    }
    let started = std::time::Instant::now();
    // A listing executes nothing, so the refresh-once rule may apply.
    let result = state
        .gateway
        .with_target(&c, crate::gateway::upstream::probe)
        .await;
    let now = crate::index::now_rfc3339();
    match &result {
        Ok(_) => {
            c.last_ok = Some(now.clone());
            c.last_error_code = None;
            if c.status == status::NEEDS_REAUTH || c.status == status::ERROR {
                c.status = status::CONNECTED.into();
            }
        }
        Err(e) => {
            c.last_error_code = Some(e.code().to_string());
            // A connection still waiting for its first authorization stays
            // `pending_auth`: there was nothing to lose.
            if e.needs_reauth() && c.status != status::PENDING_AUTH {
                if c.auth_kind == auth_kind::OAUTH && c.status == status::CONNECTED {
                    record_refresh_failed(state, &c);
                }
                c.status = status::NEEDS_REAUTH.into();
            }
        }
    }
    c.updated_at = now.clone();
    {
        let mut store = gateway_store(state);
        store.update_connection(&c).map_err(internal)?;
        // The owner's own listing: the baseline on the first test, and after
        // it new tools recorded and left off (G-D16, §9).
        if let Ok(found) = &result {
            let names: Vec<String> = found.tools.iter().map(|t| t.name.to_string()).collect();
            store
                .observe_tools(&c.id, &names, true, &now)
                .map_err(internal)?;
        }
        if let Some(fresh) = store.connection(&c.id).map_err(internal)? {
            c = fresh;
        }
    }
    state
        .gateway
        .record_call(crate::gateway::store::CallRecord {
            at_ms: crate::gateway::now_ms(),
            owner_user_id: c.owner_user_id.clone(),
            connection_id: c.id.clone(),
            session_id: None,
            host_id: None,
            method: "tools/list".into(),
            tool: None,
            outcome: if result.is_ok() { "ok" } else { "error" }.into(),
            error_code: result.as_ref().err().map(|e| e.code().to_string()),
            duration_ms: Some(started.elapsed().as_millis() as i64),
            response_bytes: None,
        });
    Ok((c, result))
}

/// `POST /v1/integrations/connections/{id}/test` (§14).
pub async fn test_integration(
    state: &Shared,
    actor: &Actor,
    id: &str,
) -> ApiResult<IntegrationTest> {
    let (c, result) = probe_integration(state, actor, id).await?;
    let integration = view_of(state, c)?;
    let (ok, error_code, server_name, server_version, tool_count) = match &result {
        Ok(found) => (
            true,
            None,
            Some(found.server_name.clone()),
            Some(found.server_version.clone()),
            found.tools.len(),
        ),
        Err(e) => (false, Some(e.code()), None, None, 0),
    };
    Ok(IntegrationTest {
        ok,
        error_code,
        server_name,
        server_version,
        tool_count,
        new_tools: integration.new_tools.clone(),
        integration,
    })
}

/// `GET /v1/integrations/connections/{id}/tools` (§14): the upstream's tools
/// now, each with whether agents may call it. A failure is `502` with the
/// stable code, never the upstream's text.
pub async fn integration_tools(
    state: &Shared,
    actor: &Actor,
    id: &str,
) -> ApiResult<Vec<IntegrationTool>> {
    let (c, result) = probe_integration(state, actor, id).await?;
    let new_tools = gateway_store(state).new_tools(&c.id).map_err(internal)?;
    let found =
        result.map_err(|e| ApiError(axum::http::StatusCode::BAD_GATEWAY, e.code().to_string()))?;
    let mut tools: Vec<IntegrationTool> = found
        .tools
        .into_iter()
        .map(|t| {
            let name = t.name.to_string();
            IntegrationTool {
                allowed: c.tool_allowlist.contains(&name),
                new: new_tools.contains(&name),
                title: t.title.clone(),
                description: t.description.as_ref().map(|d| d.to_string()),
                name,
            }
        })
        .collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tools)
}

// ---- MCP Gateway: an agent's call (decision 81e, spec §7) ------------------

/// Methods an agent may send (spec §9). Anything else is refused.
fn method_permitted(method: &str, c: Option<&Connection>) -> bool {
    let resources = c.is_none_or(|c| c.expose_resources);
    let prompts = c.is_none_or(|c| c.expose_prompts);
    match method {
        "initialize" | "ping" | "tools/list" | "tools/call" | "completion/complete" => true,
        "resources/list"
        | "resources/templates/list"
        | "resources/read"
        | "resources/subscribe"
        | "resources/unsubscribe" => resources,
        "prompts/list" | "prompts/get" => prompts,
        _ => false,
    }
}

/// What authorization established about a call, for the forwarding half.
struct Authorized {
    owner: String,
    /// `None` for the built-in `storm` connection.
    connection: Option<Connection>,
    /// The session has a write vault **and** `agent_writes` is on.
    vault_writes: bool,
    write_vault: Option<String>,
    /// The session was started from a note.
    has_context: bool,
}

impl Authorized {
    /// Whether the agent may call tool `name` (G-D16, G-D5). **One rule for
    /// the `tools/call` gate and the `tools/list` filter**, so an agent is
    /// never shown a tool it cannot call, nor able to call one it was not shown.
    fn may_call(&self, name: &str) -> bool {
        match &self.connection {
            Some(c) => c.tool_allowlist.iter().any(|a| a == name),
            None => {
                !crate::mcp::NEVER_FOR_AGENTS.contains(&name)
                    && (self.vault_writes || !crate::mcp::WRITE_TOOLS.contains(&name))
            }
        }
    }
}

/// Which agent call an audit row describes. `owner` is known once the call
/// is authorized; a refused call is audited without one.
#[derive(Clone)]
struct CallScope {
    owner: Option<String>,
    session_id: String,
    host_id: String,
    connection_id: String,
}

impl CallScope {
    /// One metadata-only audit row (G-D18).
    fn audit(
        &self,
        gateway: &crate::gateway::Gateway,
        method: Option<&str>,
        tool: Option<&str>,
        outcome: Result<(), &str>,
        duration_ms: i64,
        response_bytes: Option<i64>,
    ) {
        gateway.record_call(crate::gateway::store::CallRecord {
            at_ms: crate::gateway::now_ms(),
            owner_user_id: self.owner.clone().unwrap_or_default(),
            connection_id: self.connection_id.clone(),
            session_id: Some(self.session_id.clone()),
            host_id: Some(self.host_id.clone()),
            method: method.unwrap_or("response").to_string(),
            tool: tool.map(str::to_string),
            outcome: if outcome.is_ok() { "ok" } else { "refused" }.into(),
            error_code: outcome.err().map(str::to_string),
            duration_ms: Some(duration_ms),
            response_bytes,
        });
    }
}

/// The code an agent gets for a connection the owner must reconnect. The
/// bridge and the client parse it, so it is spelled here only.
fn needs_reauth_code(slug: &str) -> String {
    format!("integration_needs_reauth:{slug}")
}

/// Every check in spec §7, in order. A refusal is a stable code.
async fn authorize_call(
    state: &Shared,
    host_id: &str,
    session_id: &str,
    connection_id: &str,
) -> Result<Authorized, String> {
    // The host owns the session, and it is live.
    let session = state
        .agent
        .get(session_id)
        .map_err(|_| "not_your_session".to_string())?;
    if session.host_id != host_id {
        return Err("not_your_session".into());
    }
    if !matches!(session.status.as_str(), "starting" | "running") {
        return Err("session_not_live".into());
    }
    // A live grant.
    state
        .agent
        .grant(session_id, connection_id)
        .map_err(|_| "not_granted".to_string())?
        .ok_or_else(|| "not_granted".to_string())?;
    // Keeps the stable `owner_inactive` code for a removed account's session.
    {
        let auth_db = state.auth_db.lock().await;
        if !matches!(auth_db.account_by_id(&session.owner_user_id), Ok(Some(_))) {
            return Err("owner_inactive".into());
        }
    }
    // The connection is the owner's, and connected.
    let connection = if connection_id == BUILTIN_ID {
        None
    } else {
        let c = gateway_store(state)
            .connection(connection_id)
            .map_err(|_| "not_granted".to_string())?
            .ok_or_else(|| "not_granted".to_string())?;
        if c.owner_user_id != session.owner_user_id {
            return Err("not_granted".into());
        }
        match c.status.as_str() {
            status::CONNECTED => {}
            status::NEEDS_REAUTH | status::ERROR => {
                return Err(needs_reauth_code(&c.slug));
            }
            _ => return Err("not_granted".into()),
        }
        Some(c)
    };
    let launch = state
        .agent
        .launch_of(session_id)
        .map_err(|_| "not_granted".to_string())?;
    let write_vault = launch.as_ref().and_then(|l| l.write_vault_id.clone());
    let vault_writes = write_vault.is_some()
        && state
            .agent_writes
            .load(std::sync::atomic::Ordering::Relaxed);
    Ok(Authorized {
        owner: session.owner_user_id,
        connection,
        vault_writes,
        write_vault: write_vault.filter(|_| vault_writes),
        has_context: launch.is_some_and(|l| l.context.is_some()),
    })
}

/// `POST /v1/runtime/sessions/{id}/mcp/{connection}` (spec §14): one JSON-RPC
/// message from a session's bridge. The answer is a stream of lines — the
/// request-scoped messages, then the final response — or nothing for a
/// notification or an agent's answer to an elicitation.
///
/// **At most once.** Nothing here retries, and a request the gateway did not
/// forward is answered `session_unknown`, which is the only case the bridge
/// may replay `initialize` and resend.
pub async fn integration_call(
    state: &Shared,
    host_id: &str,
    session_id: &str,
    connection_id: &str,
    message: serde_json::Value,
) -> tokio::sync::mpsc::Receiver<serde_json::Value> {
    use crate::gateway::session::{error_line, message_line};
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let id = crate::gateway::session::agent_id(&message);
    let method = message
        .get("method")
        .and_then(|m| m.as_str())
        .map(str::to_string);
    let is_request = method.is_some() && message.get("id").is_some();
    let mut scope = CallScope {
        owner: None,
        session_id: session_id.to_string(),
        host_id: host_id.to_string(),
        connection_id: connection_id.to_string(),
    };

    let authorized = match authorize_call(state, host_id, session_id, connection_id).await {
        Ok(a) => a,
        Err(code) => {
            if is_request {
                let _ = tx.send(error_line(&id, &code)).await;
            }
            scope.audit(&state.gateway, method.as_deref(), None, Err(&code), 0, None);
            return rx;
        }
    };
    scope.owner = Some(authorized.owner.clone());

    let Some(method) = method else {
        // The agent answering an upstream request (an elicitation). Dropped
        // when nothing waits for it — a late answer is never sent upstream.
        if let (Some(up), Some(agent_id)) = (
            state.gateway.sessions.get(session_id, connection_id),
            message.get("id").and_then(|v| v.as_str()),
        ) {
            up.relay.answer(agent_id, &message);
        }
        return rx;
    };
    if !is_request {
        // A notification. `initialized` is rmcp's to send, and it already
        // did; `cancelled` and the rest go upstream when a session exists.
        if method != "notifications/initialized"
            && let Some(up) = state.gateway.sessions.get(session_id, connection_id)
            && let Ok(n) = serde_json::from_value::<rmcp::model::ClientNotification>(
                serde_json::json!({"method": method, "params": message.get("params")}),
            )
        {
            up.notify(n).await;
        }
        return rx;
    }

    if !method_permitted(&method, authorized.connection.as_ref()) {
        let _ = tx.send(error_line(&id, "method_not_permitted")).await;
        return rx;
    }
    let tool = (method == "tools/call")
        .then(|| {
            message
                .pointer("/params/name")
                .and_then(|n| n.as_str())
                .map(str::to_string)
        })
        .flatten();
    if let Some(name) = &tool
        && !authorized.may_call(name)
    {
        let _ = tx.send(error_line(&id, "tool_not_allowed")).await;
        scope.audit(
            &state.gateway,
            Some(&method),
            tool.as_deref(),
            Err("tool_not_allowed"),
            0,
            None,
        );
        return rx;
    }

    let state = state.clone();
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let outcome = run_call(&state, &authorized, &scope, &method, message, tx.clone()).await;
        let (result, bytes) = match outcome {
            Ok(result) => {
                // Measured, not built: the line is serialized once, on the way out.
                let bytes = crate::gateway::session::serialized_len(&result);
                if bytes > crate::gateway::session::RESPONSE_CAP {
                    let _ = tx.send(error_line(&id, "response_too_large")).await;
                    (Err("response_too_large".to_string()), Some(bytes as i64))
                } else {
                    let _ = tx
                        .send(message_line(
                            serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
                        ))
                        .await;
                    (Ok(()), Some(bytes as i64))
                }
            }
            Err(CallFailure::Code(code)) => {
                if code == "session_unknown" {
                    // Not forwarded, so the bridge may replay `initialize`.
                    let _ = tx
                        .send(serde_json::json!({"storm_error": "session_unknown"}))
                        .await;
                } else {
                    let _ = tx.send(error_line(&id, &code)).await;
                }
                (Err(code), None)
            }
            Err(CallFailure::Rpc(e)) => {
                let _ = tx
                    .send(message_line(
                        serde_json::json!({"jsonrpc": "2.0", "id": id, "error": e}),
                    ))
                    .await;
                (Err("upstream_error".to_string()), None)
            }
        };
        scope.audit(
            &state.gateway,
            Some(&method),
            tool.as_deref(),
            result.as_ref().map(|_| ()).map_err(|e| e.as_str()),
            started.elapsed().as_millis() as i64,
            bytes,
        );
    });
    rx
}

enum CallFailure {
    Code(String),
    Rpc(rmcp::ErrorData),
}

async fn run_call(
    state: &Shared,
    authorized: &Authorized,
    scope: &CallScope,
    method: &str,
    mut message: serde_json::Value,
    tx: tokio::sync::mpsc::Sender<serde_json::Value>,
) -> Result<serde_json::Value, CallFailure> {
    if method == "initialize" {
        return open_upstream(state, authorized, scope, message).await;
    }
    let (session_id, connection_id) = (&scope.session_id, &scope.connection_id);
    let Some(upstream) = state.gateway.sessions.get(session_id, connection_id) else {
        return Err(CallFailure::Code("session_unknown".into()));
    };
    if method == "ping" {
        return Ok(serde_json::json!({}));
    }
    let Some(_permit) = state
        .gateway
        .sessions
        .permit(session_id, connection_id, method)
    else {
        return Err(CallFailure::Code("gateway_rate_limited".into()));
    };
    let agent_progress = message.pointer("/params/_meta/progressToken").cloned();
    // Moved, not copied: the params are the agent's tool arguments.
    let params = message
        .get_mut("params")
        .map(serde_json::Value::take)
        .unwrap_or_else(|| serde_json::json!({}));
    let request: rmcp::model::ClientRequest =
        serde_json::from_value(serde_json::json!({ "method": method, "params": params }))
            .map_err(|_| CallFailure::Code("invalid_request".into()))?;
    let mut result = upstream
        .forward(request, agent_progress, tx)
        .await
        .map_err(|e| match e {
            crate::gateway::session::ForwardError::Rpc(e) => CallFailure::Rpc(e),
            crate::gateway::session::ForwardError::Upstream(e) => {
                upstream_failure(state, authorized.connection.as_ref(), e)
            }
        })?;
    if method == "tools/list"
        && let Some(tools) = result.get_mut("tools").and_then(|t| t.as_array_mut())
    {
        // A tool the owner has not seen is recorded for review before it is
        // filtered out (§9): "N new tools — review" appears without the
        // owner having to run a test. Never a baseline, so never enabling.
        if let Some(c) = &authorized.connection {
            let names: Vec<String> = tools
                .iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect();
            if let Err(e) = gateway_store(state).observe_tools(
                &c.id,
                &names,
                false,
                &crate::index::now_rfc3339(),
            ) {
                tracing::warn!(error = %e, "could not record an upstream's new tools");
            }
        }
        // The agent sees only what it may call (G-D16, G-D5).
        tools.retain(|t| authorized.may_call(t.get("name").and_then(|n| n.as_str()).unwrap_or("")));
    }
    Ok(result)
}

/// An upstream failure as the agent sees it, recorded for the owner first.
/// One mapping for every path that talks to an upstream (§12).
fn upstream_failure(
    state: &Shared,
    connection: Option<&Connection>,
    e: crate::gateway::upstream::UpstreamError,
) -> CallFailure {
    note_upstream_failure(state, connection, e);
    CallFailure::Code(if e.needs_reauth() {
        needs_reauth_code(connection.map_or(BUILTIN_SLUG, |c| &c.slug))
    } else {
        e.code().to_string()
    })
}

/// An upstream that refused the credential is `needs_reauth` from now on
/// (§12); one that is down records its code for the owner.
fn note_upstream_failure(
    state: &Shared,
    connection: Option<&Connection>,
    e: crate::gateway::upstream::UpstreamError,
) {
    let Some(c) = connection else { return };
    let store = gateway_store(state);
    if let Ok(Some(mut row)) = store.connection(&c.id) {
        row.last_error_code = Some(e.code().to_string());
        if e.needs_reauth() && row.status == status::CONNECTED {
            row.status = status::NEEDS_REAUTH.into();
            if row.auth_kind == auth_kind::OAUTH {
                record_refresh_failed(state, &row);
            }
        }
        row.updated_at = crate::index::now_rfc3339();
        let _ = store.update_connection(&row);
    }
}

/// An OAuth connection whose refresh failed for good: audited (§14), never
/// with a token. Spawned because the callers hold the gateway's sync lock.
fn record_refresh_failed(state: &Shared, c: &Connection) {
    let state = state.clone();
    let detail = serde_json::json!({
        "connection_id": c.id, "slug": c.slug, "auth_kind": c.auth_kind,
    });
    let owner = c.owner_user_id.clone();
    tokio::spawn(async move {
        let auth_db = state.auth_db.lock().await;
        let _ = auth_db.record_event(
            "integration_refresh_failed",
            Some(&owner),
            None,
            &crate::index::now_rfc3339(),
            &detail.to_string(),
        );
    });
}

/// The agent's `initialize`: a fresh upstream session (replacing any old one)
/// presenting the agent's capabilities minus what is never forwarded.
async fn open_upstream(
    state: &Shared,
    authorized: &Authorized,
    scope: &CallScope,
    mut message: serde_json::Value,
) -> Result<serde_json::Value, CallFailure> {
    let agent_caps = message
        .pointer_mut("/params/capabilities")
        .map(serde_json::Value::take)
        .unwrap_or_else(|| serde_json::json!({}));
    // The relay lives inside the upstream session, which the gateway holds:
    // it keeps only what it uses, and the gateway weakly, so the session
    // never keeps the server's whole state (or itself) alive.
    let events = {
        let agent = state.agent.clone();
        let gateway = std::sync::Arc::downgrade(&state.gateway);
        let scope = scope.clone();
        move |event: crate::gateway::session::RelayEvent| match event {
            crate::gateway::session::RelayEvent::Unsolicited(m) => {
                agent.mcp_message(&scope.session_id, &scope.connection_id, m);
            }
            crate::gateway::session::RelayEvent::UrlElicitationDeclined => {
                if let Some(gateway) = gateway.upgrade() {
                    scope.audit(
                        &gateway,
                        Some("elicitation/create"),
                        None,
                        Err("url_elicitation_declined"),
                        0,
                        None,
                    );
                }
            }
        }
    };
    let relay = crate::gateway::session::Relay::new(&agent_caps, events);
    let service = match &authorized.connection {
        // Nothing the agent asked for has run at `initialize`, so the
        // refresh-once rule may apply.
        Some(c) => state
            .gateway
            .with_target(c, |target| {
                crate::gateway::upstream::connect(target, relay.clone())
            })
            .await
            .map_err(|e| upstream_failure(state, Some(c), e))?,
        None => connect_builtin(state, authorized, scope, relay)
            .await
            .map_err(|_| CallFailure::Code("upstream_unavailable".into()))?,
    };
    let info = service
        .peer_info()
        .map(|i| serde_json::to_value(&*i).unwrap_or_default())
        .unwrap_or_default();
    let mut result = info;
    // What the agent is told the server offers follows the owner's switches
    // (G-D17): a hidden capability is one the agent never tries.
    if let (Some(c), Some(caps)) = (
        &authorized.connection,
        result
            .get_mut("capabilities")
            .and_then(|c| c.as_object_mut()),
    ) {
        if !c.expose_resources {
            caps.remove("resources");
        }
        if !c.expose_prompts {
            caps.remove("prompts");
        }
    }
    if let Some(obj) = result.as_object_mut()
        && !obj.contains_key("serverInfo")
    {
        obj.insert(
            "serverInfo".into(),
            serde_json::json!({"name": authorized.connection.as_ref().map_or(BUILTIN_SLUG, |c| &c.slug), "version": ""}),
        );
    }
    state.gateway.sessions.put(
        &scope.session_id,
        &scope.connection_id,
        crate::gateway::session::Upstream::new(service),
    );
    Ok(result)
}

/// The built-in `storm` connection: `mcp.rs`'s own handler, served in process
/// over an in-memory pipe as `Actor::Agent` (spec §5). No network, no
/// credential, and the same client path as any upstream.
async fn connect_builtin(
    state: &Shared,
    authorized: &Authorized,
    scope: &CallScope,
    relay: crate::gateway::session::Relay,
) -> anyhow::Result<
    rmcp::service::RunningService<rmcp::service::RoleClient, crate::gateway::session::Relay>,
> {
    use rmcp::ServiceExt;
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let actor = Actor::Agent {
        session_id: scope.session_id.clone(),
        host_id: scope.host_id.clone(),
        user_id: authorized.owner.clone(),
        write_vault: authorized.write_vault.clone(),
    };
    let handler = crate::mcp::Storm::for_agent(
        state.clone(),
        authorized.vault_writes,
        authorized.has_context,
        actor,
    );
    tokio::spawn(async move {
        if let Ok(running) = handler.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    Ok(relay.serve(client_io).await?)
}

/// Retires what removed accounts left in `agent.db`/`gateway.db` (decision 82,
/// invariant I4). Runs on every boot; idempotent.
pub async fn reconcile_single_user(state: &Shared) -> anyhow::Result<()> {
    let account = {
        let auth_db = state.auth_db.lock().await;
        auth_db.account()?
    };
    let Some(account) = account else {
        return Ok(());
    };
    let now = crate::index::now_rfc3339();

    let orphaned = gateway_store(state).live_connections_not_of(&account.id)?;
    for c in &orphaned {
        gateway_store(state).revoke_connection(&c.id, &now)?;
        state.agent.revoke_grants_for(&c.id)?;
        state.gateway.sessions.close_connection(&c.id);
        let host = c
            .url
            .parse::<axum::http::Uri>()
            .ok()
            .and_then(|u| u.host().map(str::to_string));
        let detail = serde_json::json!({
            "connection_id": c.id,
            "former_owner": c.owner_user_id,
            "upstream_host": host,
        });
        let auth_db = state.auth_db.lock().await;
        auth_db.record_event(
            "integration_removed_single_user",
            Some(&account.id),
            None,
            &now,
            &detail.to_string(),
        )?;
    }

    let (failed, dismissed) = state.agent.retire_sessions_not_of(&account.id)?;
    sweep_gateway_sessions(state);
    if !orphaned.is_empty() || failed > 0 || dismissed > 0 {
        tracing::warn!(
            connections = orphaned.len(),
            sessions_failed = failed,
            sessions_dismissed = dismissed,
            "single-user: retired what removed accounts left in agent.db and gateway.db"
        );
    }
    Ok(())
}

/// Closes the upstream sessions of agent sessions that have ended (§13: a
/// session's grants die with it). Called whenever a host reports.
pub fn sweep_gateway_sessions(state: &Shared) {
    for session in state.gateway.sessions.sessions() {
        let ended = state.agent.get(&session).map_or(true, |r| r.is_ended());
        if ended {
            state.gateway.sessions.close_session(&session);
        }
    }
}

// ---- MCP Gateway: OAuth (decision 81g, spec §10) ----------------------------

#[derive(Deserialize)]
pub struct AuthorizeIntegration {
    pub redirect_uri: String,
    /// A client the owner registered by hand, when the server offers no
    /// dynamic registration (§10 step 2).
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorizationStarted {
    /// The URL the client opens in the system browser.
    pub authorization_url: String,
    pub expires: String,
}

fn oauth_refusal(f: crate::gateway::oauth::OAuthFailure) -> ApiError {
    use crate::gateway::oauth::OAuthFailure as F;
    let status = match f {
        F::NeedsClient | F::FlowSpent => axum::http::StatusCode::BAD_REQUEST,
        F::NoOAuth => axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        _ => axum::http::StatusCode::BAD_GATEWAY,
    };
    ApiError(status, f.code().into())
}

/// `POST /v1/integrations/connections/{id}/authorize`: discovery, a client
/// (reused, registered dynamically, or pasted), and the PKCE authorization
/// URL. The flow is recorded hashed and sealed, for ten minutes.
pub async fn authorize_integration(
    state: &Shared,
    actor: &Actor,
    id: &str,
    req: AuthorizeIntegration,
) -> ApiResult<AuthorizationStarted> {
    use crate::gateway::oauth::{self as oauth, OAuthFailure};
    let c = own_connection(state, actor, id)?;
    if c.auth_kind != auth_kind::OAUTH {
        return Err(bad_request("only an OAuth integration is authorized here"));
    }
    if c.status == status::DISABLED {
        return Err(conflict("the integration is disabled; enable it first"));
    }
    oauth::validate_redirect(&req.redirect_uri).map_err(bad_request)?;
    if let Some(client_id) = &req.client_id
        && (client_id.is_empty()
            || client_id.len() > 512
            || client_id.chars().any(char::is_control))
    {
        return Err(bad_request("a client id is 1–512 printable characters"));
    }
    let (mut manager, metadata) = oauth::discovered_manager(&state.gateway, &c.url)
        .await
        .map_err(oauth_refusal)?;
    let issuer = metadata
        .issuer
        .clone()
        .unwrap_or_else(|| metadata.authorization_endpoint.clone());
    let now = crate::index::now_rfc3339();
    let metadata_json = serde_json::to_string(&metadata).ok();

    let existing = gateway_store(state)
        .oauth_client_for(actor.user_id(), &issuer, &req.redirect_uri)
        .map_err(internal)?;
    let client = match (&req.client_id, existing) {
        // A pasted client always wins: the owner is telling us which one.
        (Some(client_id), _) => {
            let row_id = crate::auth::identity::random_id("oac_");
            let secret = match &req.client_secret {
                Some(s) => Some(
                    state
                        .gateway
                        .keys
                        .seal(&row_id, "client_secret", s.as_bytes())
                        .map_err(internal)?,
                ),
                None => None,
            };
            crate::gateway::store::OAuthClientRow {
                id: row_id,
                owner_user_id: actor.user_id().into(),
                issuer: issuer.clone(),
                client_id: client_id.clone(),
                redirect_uri: req.redirect_uri.clone(),
                registered: false,
                metadata: metadata_json.clone(),
                secret,
                created_at: now.clone(),
            }
        }
        (None, Some(row)) => crate::gateway::store::OAuthClientRow {
            metadata: metadata_json.clone(),
            ..row
        },
        (None, None) => {
            if metadata.registration_endpoint.is_none() {
                return Err(oauth_refusal(OAuthFailure::NeedsClient));
            }
            let scopes: Vec<&str> = req.scopes.iter().map(String::as_str).collect();
            let registered = manager
                .register_client("Storm", &req.redirect_uri, &scopes)
                .await
                .map_err(|_| oauth_refusal(OAuthFailure::Registration))?;
            crate::gateway::store::OAuthClientRow {
                id: crate::auth::identity::random_id("oac_"),
                owner_user_id: actor.user_id().into(),
                issuer: issuer.clone(),
                client_id: registered.client_id,
                redirect_uri: req.redirect_uri.clone(),
                registered: true,
                metadata: metadata_json.clone(),
                secret: None,
                created_at: now.clone(),
            }
        }
    };
    gateway_store(state)
        .insert_oauth_client(&client)
        .map_err(internal)?;
    state
        .gateway
        .configure_client(&mut manager, &client)
        .map_err(|_| oauth_refusal(OAuthFailure::Discovery))?;
    manager.set_state_store(oauth::FlowStore::new(&state.gateway, &c, &client));
    let scopes: Vec<&str> = req.scopes.iter().map(String::as_str).collect();
    let authorization_url = manager
        .get_authorization_url(&scopes)
        .await
        .map_err(|_| oauth_refusal(OAuthFailure::Discovery))?;
    Ok(AuthorizationStarted {
        authorization_url,
        expires: crate::gateway::rfc3339_in(oauth::FLOW_TTL_SECS),
    })
}

#[derive(Deserialize)]
pub struct OAuthCallback {
    pub state: String,
    pub code: String,
    /// RFC 9207's `iss`, when the redirect carried one.
    #[serde(default)]
    pub iss: Option<String>,
}

/// `POST /v1/integrations/oauth/callback` (§10 step 5): the client relays
/// what the browser brought back. The flow is single use and the caller's
/// own; the code is exchanged, the tokens sealed, and the integration tested.
pub async fn oauth_callback(
    state: &Shared,
    actor: &Actor,
    req: OAuthCallback,
) -> ApiResult<IntegrationTest> {
    use crate::gateway::oauth::{self as oauth, OAuthFailure};
    if req.state.is_empty() || req.state.len() > 512 || req.code.is_empty() || req.code.len() > 4096
    {
        return Err(bad_request("state and code are required"));
    }
    let hash = oauth::state_hash(&req.state);
    let flow = gateway_store(state)
        .live_flow(&hash, &crate::index::now_rfc3339())
        .map_err(internal)?
        .ok_or_else(|| oauth_refusal(OAuthFailure::FlowSpent))?;
    // Another owner's flow reads exactly like a spent one.
    if flow.owner_user_id != actor.user_id() {
        return Err(oauth_refusal(OAuthFailure::FlowSpent));
    }
    let c = own_connection(state, actor, &flow.connection_id)?;
    let client = gateway_store(state)
        .oauth_client(&flow.oauth_client)
        .map_err(internal)?
        .ok_or_else(|| oauth_refusal(OAuthFailure::FlowSpent))?;
    let (mut manager, _) = oauth::discovered_manager(&state.gateway, &c.url)
        .await
        .map_err(oauth_refusal)?;
    state
        .gateway
        .configure_client(&mut manager, &client)
        .map_err(|_| oauth_refusal(OAuthFailure::Exchange))?;
    manager.set_state_store(oauth::FlowStore::new(&state.gateway, &c, &client));
    manager.set_credential_store(oauth::TokenStore::new(&state.gateway, &c));
    // The flow store's `load` claims the flow, so a replayed callback — or a
    // second one racing this — finds nothing and exchanges nothing.
    let had_tokens = gateway_store(state)
        .credential(&c.id, credential_kind::OAUTH_TOKENS)
        .map_err(internal)?
        .is_some();
    manager
        .exchange_code_for_token_with_issuer(&req.code, &req.state, req.iss.as_deref())
        .await
        .map_err(|e| match e {
            rmcp::transport::auth::AuthError::InternalError(m) if m.contains("state not found") => {
                oauth_refusal(OAuthFailure::FlowSpent)
            }
            _ => oauth_refusal(OAuthFailure::Exchange),
        })?;

    let now = crate::index::now_rfc3339();
    let mut c = c;
    c.status = status::CONNECTED.into();
    c.last_error_code = None;
    c.updated_at = now.clone();
    {
        let store = gateway_store(state);
        store
            .set_connection_oauth(&c.id, &client.id, &now)
            .map_err(internal)?;
        store.update_connection(&c).map_err(internal)?;
    }
    // Live sessions pick the new tokens up on their next initialize.
    state.gateway.sessions.close_connection(&c.id);
    {
        let auth_db = state.auth_db.lock().await;
        let kind = if had_tokens {
            "integration_reauthorized"
        } else {
            "integration_authorized"
        };
        integration_event(&auth_db, kind, actor, &c, &now);
    }
    // §10 step 6: a test listing, which also turns every tool on.
    test_integration(state, actor, &c.id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_queries_are_sanitized() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
        // Characters that would otherwise be FTS5 syntax errors.
        assert_eq!(sanitize_fts_query("foo-bar"), "\"foo-bar\"");
        assert_eq!(sanitize_fts_query("NEAR(a b)"), "\"NEAR(a\" \"b)\"");
        assert_eq!(sanitize_fts_query("  "), "");
        assert_eq!(sanitize_fts_query("say \"hi\""), "\"say\" \"hi\"");
    }

    // ---- scripts: the extension allowlist ------------------------------

    #[test]
    fn a_script_name_maps_into_the_scripts_root() {
        assert_eq!(
            must(script_path("psi-item-import/run.spec.ts")),
            "scripts/psi-item-import/run.spec.ts"
        );
        assert_eq!(must(script_path("say-hello.sh")), "scripts/say-hello.sh");
    }

    #[test]
    fn the_extension_allowlist_is_case_insensitive() {
        assert_eq!(
            must(script_path("Demo/Seed.JSON")),
            "scripts/Demo/Seed.JSON"
        );
    }

    #[test]
    fn names_outside_the_allowlist_are_refused() {
        // Markdown belongs to the notes tools, executables belong to no one.
        for name in [
            "notes/README.md",
            "tool.bat",
            "virus.exe",
            "no-extension",
            "dir/",
            "/absolute.ts",
            "",
        ] {
            let err = script_path(name).unwrap_err();
            assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST, "{name}");
        }
    }

    // ---- GET /v1/server: the live relay set -----------------------------

    use crate::api::{AppState, VaultSet};
    use crate::registry::Registry;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// `ApiError` has no `Debug` impl — deliberately, it is an HTTP response —
    /// so `.unwrap()` is unavailable on an `ApiResult`.
    fn must<T>(result: ApiResult<T>) -> T {
        match result {
            Ok(value) => value,
            Err(ApiError(status, message)) => panic!("{status}: {message}"),
        }
    }

    /// Enough state to answer `/v1/server`, which is decided entirely from the
    /// identity and the registry — no vault is opened on this path.
    fn server_state(dir: &std::path::Path) -> Shared {
        let state_dir = dir.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let root = dir.join("vaults");
        std::fs::create_dir_all(&root).unwrap();

        let mut auth_db = crate::auth::AuthDb::open(&state_dir).unwrap();
        let identity = Arc::new(
            crate::auth::identity::load_or_create(&mut auth_db, &state_dir, "2026-08-26T00:00:00Z")
                .unwrap(),
        );
        let (events, _) = tokio::sync::broadcast::channel(8);
        let (root_changed, _) = tokio::sync::broadcast::channel(2);
        let registry = Registry::load(&state_dir, &root).unwrap();

        Arc::new(AppState {
            vaults: tokio::sync::RwLock::new(VaultSet {
                registry,
                open: HashMap::new(),
            }),
            events,
            state_dir,
            identity,
            root_changed,
            relays_changed: tokio::sync::watch::channel(Vec::new()).0,
            mcp_enabled: std::sync::atomic::AtomicBool::new(false),
            mcp_writable: std::sync::atomic::AtomicBool::new(false),
            agent_writes: std::sync::atomic::AtomicBool::new(false),
            auth_db: Arc::new(tokio::sync::Mutex::new(auth_db)),
            bootstrap_nonce: None,
            listen_addr: "127.0.0.1:8484".into(),
            vault_policy: Arc::new(crate::auth::authz::StormPolicy),
            hasher: crate::auth::Hasher::new(),
            login_limiter: crate::auth::ratelimit::LoginLimiter::new(),
            host_limiter: crate::auth::ratelimit::LoginLimiter::new(),
            agent: Arc::new(crate::agent::AgentManager::open(dir).unwrap()),
            gateway: Arc::new(crate::gateway::Gateway::open(dir, "2026-10-05T00:00:00Z").unwrap()),
        })
    }

    #[tokio::test]
    async fn server_info_carries_an_empty_relay_set_on_a_fresh_server() {
        let dir = tempdir::TempDir::new("storm-ops-relays").unwrap();
        let state = server_state(dir.path());

        let info = must(server_info(&state).await);
        let json = serde_json::to_value(&info).unwrap();

        // The field is live, not absent: a client that finds no `relays` key
        // cannot tell "this server has no relay path" from "this server is too
        // old to say", and would keep dialling a stale pairing payload.
        assert_eq!(
            json["relays"],
            serde_json::json!([]),
            "no tunnel client exists yet, so nothing is registered — and that is the honest answer"
        );

        // The rest of the response is unchanged, asserted key by key so a
        // future edit cannot quietly reshape what pairing depends on.
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "algorithm".to_string(),
                "key_id".to_string(),
                "name".to_string(),
                "public_key".to_string(),
                "relays".to_string(),
                "server_id".to_string(),
            ],
            "the addition is additive — nothing was added but `relays`, nothing removed"
        );
        assert_eq!(json["server_id"], state.identity.server_id.as_str());
        assert_eq!(json["name"], state.identity.name.as_str());
        assert_eq!(json["key_id"], state.identity.key_id.as_str());
        assert_eq!(json["algorithm"], crate::auth::identity::ALGORITHM);
        assert_eq!(json["public_key"], state.identity.public_key_b64().as_str());
    }

    #[tokio::test]
    async fn server_info_advertises_registered_relays_never_configured_ones() {
        // The distinction the field exists for. A relay the server failed to
        // register with is a dead path; the client races its candidates on a
        // ~2 s budget, so advertising one costs part of that budget on every
        // single reconnect.
        let dir = tempdir::TempDir::new("storm-ops-relays2").unwrap();
        let state = server_state(dir.path());

        {
            let mut vaults = state.vaults.write().await;
            vaults
                .registry
                .set_relays(&[
                    "wss://relay.example.com".to_string(),
                    "wss://relay.two.example".to_string(),
                ])
                .unwrap();
        }

        let info = must(server_info(&state).await);
        assert!(
            info.relays.is_empty(),
            "configured but unregistered relays must not reach the wire"
        );

        // Now one of them registers.
        {
            let vaults = state.vaults.read().await;
            vaults
                .registry
                .registered_relays
                .mark_registered("wss://relay.two.example")
                .unwrap();
        }

        let info = must(server_info(&state).await);
        assert_eq!(info.relays.len(), 1, "only the one that registered");
        assert_eq!(info.relays[0].url, "wss://relay.two.example");
        assert_eq!(
            info.relays[0].public_address,
            format!(
                "wss://relay.two.example/connect/{}",
                state.identity.server_id
            ),
            "derived from the server_id in the same response, never allocated"
        );

        // And when registration lapses it leaves again, without the
        // configuration changing at all.
        {
            let vaults = state.vaults.read().await;
            vaults
                .registry
                .registered_relays
                .mark_unregistered("wss://relay.two.example");
            assert_eq!(vaults.registry.relays.len(), 2, "still configured");
        }
        assert!(must(server_info(&state).await).relays.is_empty());
    }
}
