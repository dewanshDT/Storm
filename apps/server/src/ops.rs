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

use serde::Serialize;

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
        // the whole thing. Today `AllowAuthenticated` keeps every entry.
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
    Ok(result)
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

/// The user a key operation acts on, and the refusal if the caller may not.
///
/// **The one authorization rule A14 adds, and it is deliberately not a policy.**
/// A user reaches their own keys; an owner reaches anyone's. That is it — no
/// grants, no vault scoping, no new abstraction for the authorization release
/// to unpick. When that release lands, this becomes one of its inputs rather
/// than a competing system.
fn target_user<'a>(actor: &'a Actor, requested: Option<&'a str>) -> ApiResult<&'a str> {
    // Every caller has a user since the cutover, so there is no ownerless
    // case to refuse any more — that branch existed only for the shared token.
    let caller = actor.user_id();

    match requested {
        None => Ok(caller),
        Some(other) if other == caller => Ok(caller),
        Some(other) => {
            if actor.role() == crate::auth::users::Role::Owner {
                Ok(other)
            } else {
                Err(ApiError(
                    axum::http::StatusCode::FORBIDDEN,
                    "you can only manage your own keys".into(),
                ))
            }
        }
    }
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
    let owner = target_user(actor, None)?;
    crate::auth::keys::validate_name(name).map_err(bad_request)?;

    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let (key, secret) =
        crate::auth::keys::create(&mut auth_db, owner, name, created_via, expires, &now)
            .map_err(|e| bad_request(e.to_string()))?;

    Ok(CreatedApiKey { key, secret })
}

/// Lists keys. Own by default; an owner may name another user.
pub async fn list_api_keys(
    state: &Shared,
    actor: &Actor,
    user: Option<&str>,
) -> ApiResult<Vec<crate::auth::keys::ApiKey>> {
    let owner = target_user(actor, user)?;
    let auth_db = state.auth_db.lock().await;
    auth_db
        .api_keys_for_user(owner)
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

    // **Checked against the key's real owner**, so naming someone else's key id
    // does not reveal that it exists — the refusal is the same shape whether
    // the id is wrong or merely not yours.
    let allowed = target_user(actor, Some(&key.user_id));
    if allowed.is_err() {
        return Err(not_found("no such key"));
    }

    // The audit row is written inside `keys::revoke`, beside the act.
    crate::auth::keys::revoke(
        &mut auth_db,
        key_id,
        Some(actor.user_id()),
        "revoked by user",
        &now,
    )
    .map_err(|e| ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(())
}

// ---- Agent Runtime: Runtime Hosts (decision 77b) ---------------------------

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// **The one gate on every `/v1/agent/*` operation** (AM5): agent execution and
/// host administration are the server owner's alone until the authorization
/// release. Anyone else gets `403` — never an empty list, which would read as
/// "no hosts" rather than "not yours to see".
pub fn require_owner(actor: &Actor) -> ApiResult<()> {
    if actor.role() == crate::auth::users::Role::Owner {
        Ok(())
    } else {
        Err(ApiError(
            axum::http::StatusCode::FORBIDDEN,
            "agents are available to the server owner only".into(),
        ))
    }
}

/// A host as the owner's client sees it.
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
    require_owner(actor)?;
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

pub async fn list_hosts(state: &Shared, actor: &Actor) -> ApiResult<Vec<HostView>> {
    require_owner(actor)?;
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
    require_owner(actor)?;
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
    require_owner(actor)?;
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

pub async fn host_workspaces(
    state: &Shared,
    actor: &Actor,
    host_id: &str,
) -> ApiResult<Vec<WorkspaceView>> {
    require_owner(actor)?;
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

pub async fn launch_session(
    state: &Shared,
    actor: &Actor,
    req: crate::agent::Launch,
) -> ApiResult<crate::agent::store::SessionRecord> {
    require_owner(actor)?;
    // The host must exist and be live in auth.db, not merely connected.
    {
        let auth_db = state.auth_db.lock().await;
        match auth_db.host_by_id(&req.host_id).map_err(internal)? {
            Some(h) if !h.is_revoked() => {}
            _ => return Err(not_found("no such host")),
        }
    }
    state
        .agent
        .launch(actor.user_id(), req)
        .map_err(agent_error)
}

pub async fn list_sessions(
    state: &Shared,
    actor: &Actor,
) -> ApiResult<Vec<crate::agent::store::SessionRecord>> {
    require_owner(actor)?;
    state.agent.list().map_err(agent_error)
}

pub async fn get_session(
    state: &Shared,
    actor: &Actor,
    id: &str,
) -> ApiResult<crate::agent::store::SessionRecord> {
    require_owner(actor)?;
    state.agent.get(id).map_err(agent_error)
}

pub async fn end_session(state: &Shared, actor: &Actor, id: &str) -> ApiResult<()> {
    require_owner(actor)?;
    state.agent.end(id).map_err(agent_error)
}

pub async fn dismiss_session(state: &Shared, actor: &Actor, id: &str) -> ApiResult<()> {
    require_owner(actor)?;
    state.agent.dismiss(id).map_err(agent_error)
}

pub async fn session_input(state: &Shared, actor: &Actor, id: &str, bytes: &[u8]) -> ApiResult<()> {
    require_owner(actor)?;
    state.agent.input(id, bytes).map_err(agent_error)
}

pub async fn session_resize(
    state: &Shared,
    actor: &Actor,
    id: &str,
    size: crate::agent::TerminalSize,
) -> ApiResult<()> {
    require_owner(actor)?;
    state.agent.resize(id, size).map_err(agent_error)
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentConfigView {
    pub default_provider: String,
}

pub async fn agent_config(state: &Shared, actor: &Actor) -> ApiResult<AgentConfigView> {
    require_owner(actor)?;
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
    require_owner(actor)?;
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
    state.agent.hello(host_id, hello).map_err(agent_error)
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
        .map_err(agent_error)
}

// ---- MCP Gateway: integrations (decisions 81, 81c) -------------------------
//
// Every operation here is the owner's alone (G-D20, AM29 `integration.manage`)
// and acts only on the caller's own connections. There is deliberately no MCP
// tool for any of them (spec §14), and the routes are session tier, so an
// `stk_` key can never manage an integration.

use crate::gateway::connections::{
    self as conn, BUILTIN_ID, BUILTIN_SLUG, Connection, StaticCredential, auth_kind,
    credential_kind, status,
};

/// The gate on every `/v1/integrations/*` operation: `403` for anyone but an
/// owner, never an empty list (the AM5 rule `require_owner` follows).
pub fn require_integration_owner(actor: &Actor) -> ApiResult<()> {
    if actor.role() == crate::auth::users::Role::Owner {
        Ok(())
    } else {
        Err(ApiError(
            axum::http::StatusCode::FORBIDDEN,
            "integrations are managed by the server owner only".into(),
        ))
    }
}

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
    pub expose_resources: bool,
    pub expose_prompts: bool,
    pub upstream_account_label: Option<String>,
    pub last_ok: Option<String>,
    pub last_error_code: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Built-in only: whether agents could write to the vault at all, which
    /// needs `mcp_writable` as well as the launch toggle (G-D5).
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
        expose_resources: false,
        expose_prompts: false,
        upstream_account_label: None,
        last_ok: None,
        last_error_code: None,
        created_at: None,
        updated_at: None,
        vault_writes_available: Some(
            state
                .mcp_writable
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
    }
}

fn integration_view(c: Connection, has_credential: bool) -> IntegrationView {
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
    let held = {
        let store = state.gateway.store.lock().expect("gateway store lock");
        store
            .credential(&c.id, credential_kind::STATIC)
            .map_err(internal)?
            .is_some()
            || store
                .credential(&c.id, credential_kind::OAUTH_TOKENS)
                .map_err(internal)?
                .is_some()
    };
    Ok(integration_view(c, held))
}

/// The caller's own live connection, or `404` — for someone else's too, so an
/// owner probing ids learns nothing about another owner's integrations.
fn own_connection(state: &Shared, actor: &Actor, id: &str) -> ApiResult<Connection> {
    let store = state.gateway.store.lock().expect("gateway store lock");
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
    let plaintext = serde_json::to_vec(credential).map_err(internal)?;
    state
        .gateway
        .keys
        .seal(id, credential_kind::STATIC, &plaintext)
        .map_err(internal)
}

/// Every integration the owner has, the built-in `storm` connection first.
pub async fn list_integrations(state: &Shared, actor: &Actor) -> ApiResult<Vec<IntegrationView>> {
    require_integration_owner(actor)?;
    let rows = state
        .gateway
        .store
        .lock()
        .expect("gateway store lock")
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
    require_integration_owner(actor)?;
    if id == BUILTIN_ID {
        return Ok(builtin_view(state));
    }
    let c = own_connection(state, actor, id)?;
    view_of(state, c)
}

pub struct NewIntegration {
    pub display_name: String,
    pub slug: Option<String>,
    pub url: String,
    pub auth_kind: String,
    pub credential: Option<StaticCredential>,
}

/// Connects an integration. V1 here takes `static` (a header, G-D24's PAT)
/// and `none`; `oauth` arrives with its authorize flow (81g).
pub async fn create_integration(
    state: &Shared,
    actor: &Actor,
    req: NewIntegration,
) -> ApiResult<IntegrationView> {
    require_integration_owner(actor)?;
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
        (auth_kind::OAUTH, _) => {
            return Err(bad_request(
                "OAuth integrations are not available yet; connect with a token",
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
        auth_kind: req.auth_kind,
        status: status::CONNECTED.into(),
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
    state
        .gateway
        .store
        .lock()
        .expect("gateway store lock")
        .insert_connection(&c, sealed.as_ref().map(|s| (credential_kind::STATIC, s)))
        .map_err(|e| match e {
            crate::gateway::store::InsertError::SlugTaken => conflict(e.to_string()),
            crate::gateway::store::InsertError::Other(e) => internal(e),
        })?;
    {
        let auth_db = state.auth_db.lock().await;
        integration_event(&auth_db, "integration_created", actor, &c, &now);
    }
    Ok(integration_view(c, sealed.is_some()))
}

#[derive(Default)]
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
    require_integration_owner(actor)?;
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
        let store = state.gateway.store.lock().expect("gateway store lock");
        if let Some(sealed) = &sealed {
            store
                .put_credential(&c.id, credential_kind::STATIC, sealed, None, &now)
                .map_err(internal)?;
        }
        store.update_connection(&c).map_err(internal)?;
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
    require_integration_owner(actor)?;
    if id == BUILTIN_ID {
        return Err(bad_request("the built-in connection cannot be deleted"));
    }
    let c = own_connection(state, actor, id)?;
    let now = crate::index::now_rfc3339();
    state
        .gateway
        .store
        .lock()
        .expect("gateway store lock")
        .revoke_connection(&c.id, &now)
        .map_err(internal)?;
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
    /// Tools seen for the first time, which stay off until the owner turns
    /// them on (G-D16). This is the owner's notice.
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
    Vec<String>,
)> {
    require_integration_owner(actor)?;
    if id == BUILTIN_ID {
        return Err(bad_request("the built-in connection is served in process"));
    }
    let mut c = own_connection(state, actor, id)?;
    if c.status == status::DISABLED {
        return Err(conflict("the integration is disabled; enable it first"));
    }
    let started = std::time::Instant::now();
    let result = match state.gateway.target(&c) {
        Ok(target) => crate::gateway::upstream::probe(target).await,
        Err(e) => Err(e),
    };
    let now = crate::index::now_rfc3339();
    let mut new_tools = Vec::new();
    match &result {
        Ok(found) => {
            let names: Vec<String> = found.tools.iter().map(|t| t.name.to_string()).collect();
            new_tools = crate::gateway::upstream::reconcile_tools(&mut c, &names);
            c.last_ok = Some(now.clone());
            c.last_error_code = None;
            if c.status == status::NEEDS_REAUTH || c.status == status::ERROR {
                c.status = status::CONNECTED.into();
            }
        }
        Err(e) => {
            c.last_error_code = Some(e.code().to_string());
            if e.needs_reauth() {
                c.status = status::NEEDS_REAUTH.into();
            }
        }
    }
    c.updated_at = now;
    {
        let store = state.gateway.store.lock().expect("gateway store lock");
        store.update_connection(&c).map_err(internal)?;
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
    Ok((c, result, new_tools))
}

/// `POST /v1/integrations/connections/{id}/test` (§14).
pub async fn test_integration(
    state: &Shared,
    actor: &Actor,
    id: &str,
) -> ApiResult<IntegrationTest> {
    let (c, result, new_tools) = probe_integration(state, actor, id).await?;
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
        new_tools,
        integration: view_of(state, c)?,
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
    let (c, result, new_tools) = probe_integration(state, actor, id).await?;
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
            auth_db: Arc::new(tokio::sync::Mutex::new(auth_db)),
            allow_registration: std::sync::atomic::AtomicBool::new(false),
            bootstrap_nonce: None,
            listen_addr: "127.0.0.1:8484".into(),
            vault_policy: Arc::new(crate::auth::authz::AllowAuthenticated),
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
