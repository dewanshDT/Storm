//! Decision 149: an agent's `delete_note`/`delete_folder` waits for the
//! person's Approve, and the deletion runs when they give it, checked
//! against the target as it was asked for. The store is
//! `agent::deletes`; this is the policy around it.

use serde::{Deserialize, Serialize};

use super::{agent_error, broadcast_latest, owners_session};
use crate::agent::deletes::{
    self, DECLINED, DELETED, DeleteRequest, DeleteTarget, FOLDER, GONE, NOTE, Refusal,
};
use crate::api::{ApiError, ApiResult, Shared, bad_request, conflict, not_found, vault_of};
use crate::auth::authz::{Access, Actor};

/// A deletion asked for, with its target and the reason's length, never
/// its text.
pub const EVENT_DELETE_REQUEST: &str = "agent_delete_request";
/// The person's answer, or its time-out.
pub const EVENT_DELETE_DECISION: &str = "agent_delete_decision";

/// The folder Storm keeps the vault's own settings in, never deleted.
const PROTECTED: &str = "_storm";

const DELETED_SENTENCE: &str = "The person approved. It is in the vault's Trash, which only \
    they can open, restore from or empty.";
const DECLINED_SENTENCE: &str = "The person declined: nothing was deleted. Do not ask again for \
    the same thing unless the person says so.";
const TIMED_OUT_SENTENCE: &str = "Nobody answered within 10 minutes, so nothing was deleted.";
const GONE_SENTENCE: &str = "It changed, moved or vanished after you asked, so nothing was \
    deleted. Read it again, and ask again if it should still go.";
const PENDING_SENTENCE: &str = "The person has not answered yet. Call the same tool again for \
    the same target to keep waiting; it never asks twice.";
const BUSY_SENTENCE: &str = "Another of your deletions is waiting for the person. Wait for it \
    (call that delete again) before asking for anything else.";

/// What the agent asks to delete.
#[derive(Debug, Clone)]
pub enum AgentDelete {
    Note(String),
    Folder(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteAnswer {
    /// `deleted`, `declined`, `timed_out`, `gone`, `pending` or `busy`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub message: &'static str,
}

impl DeleteAnswer {
    fn of(r: &DeleteRequest) -> Self {
        let (status, message) = match r.state.as_str() {
            DELETED => ("deleted", DELETED_SENTENCE),
            DECLINED => ("declined", DECLINED_SENTENCE),
            deletes::TIMED_OUT => ("timed_out", TIMED_OUT_SENTENCE),
            GONE => ("gone", GONE_SENTENCE),
            _ => ("pending", PENDING_SENTENCE),
        };
        Self {
            status,
            request: Some(r.id.clone()),
            trash_id: r.trash_id.clone(),
            expires_at: r.pending().then(|| r.expires_at.clone()),
            message,
        }
    }
}

async fn audit(
    state: &Shared,
    kind: &str,
    owner: &str,
    device: Option<&str>,
    detail: serde_json::Value,
) {
    let _ = state.auth_db.lock().await.record_event(
        kind,
        Some(owner),
        device,
        &crate::index::now_rfc3339(),
        &detail.to_string(),
    );
}

fn target_detail(r: &DeleteRequest) -> serde_json::Value {
    serde_json::json!({
        "session_id": r.session_id,
        "request_id": r.id,
        "vault_id": r.vault_id,
        "kind": r.kind,
        "note_id": r.note_id,
        "path": r.path,
        "notes": r.notes,
    })
}

/// A reason is the agent's own words, 1 to 500 characters.
fn checked_reason(reason: &str) -> ApiResult<&str> {
    let reason = reason.trim();
    let chars = reason.chars().count();
    if chars == 0 || chars > deletes::MAX_REASON_CHARS {
        return Err(bad_request(format!(
            "reason is required: say why, in 1 to {} characters, for the person who approves it",
            deletes::MAX_REASON_CHARS
        )));
    }
    Ok(reason)
}

fn protected(path: &str) -> bool {
    path == PROTECTED || path.starts_with(&format!("{PROTECTED}/"))
}

fn folder_fingerprint(notes: &[crate::db::NoteRow]) -> String {
    let mut rows: Vec<String> = notes
        .iter()
        .map(|n| format!("{}\0{}\0{}", n.id, n.version, n.path))
        .collect();
    rows.sort();
    blake3::hash(rows.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

/// The target as it is now, read under the vault's lock.
fn snapshot(
    ix: &crate::index::Indexer,
    vault_id: &str,
    what: &AgentDelete,
) -> ApiResult<DeleteTarget> {
    match what {
        AgentDelete::Note(id) => {
            let note = ix
                .db
                .get_note(id)?
                .ok_or_else(|| not_found("no such note"))?;
            if protected(&note.path) {
                return Err(bad_request(
                    "“_storm” holds the vault's own settings and cannot be deleted",
                ));
            }
            Ok(DeleteTarget {
                vault_id: vault_id.to_string(),
                kind: NOTE,
                note_id: Some(note.id.clone()),
                fingerprint: format!("{}:{}", note.version, note.path),
                title: Some(note.title),
                paths: vec![note.path.clone()],
                path: note.path,
                notes: 1,
                size: note.size,
                modified: Some(note.modified),
            })
        }
        AgentDelete::Folder(path) => {
            let clean = crate::index::normalise_folder(path)?;
            if protected(&clean) {
                return Err(bad_request(
                    "“_storm” holds the vault's own settings and cannot be deleted",
                ));
            }
            if !ix.all_folders()?.contains(&clean) {
                return Err(not_found("no such folder"));
            }
            let notes = ix.db.notes_under(&clean)?;
            let mut paths: Vec<String> = notes.iter().map(|n| n.path.clone()).collect();
            paths.sort();
            Ok(DeleteTarget {
                vault_id: vault_id.to_string(),
                kind: FOLDER,
                note_id: None,
                fingerprint: folder_fingerprint(&notes),
                title: None,
                path: clean,
                notes: notes.len() as i64,
                size: notes.iter().map(|n| n.size).sum(),
                modified: notes.iter().map(|n| n.modified.clone()).max(),
                paths,
            })
        }
    }
}

/// `delete_note`/`delete_folder` for an agent session (149a/b): only a
/// writable top-level session, with a reason; protected folders refused.
/// Makes one request (asking again for the same target waits on it), then
/// waits up to the agent's wait cap for the person.
pub async fn agent_delete(
    state: &Shared,
    actor: &Actor,
    vault: &str,
    what: AgentDelete,
    reason: &str,
) -> ApiResult<DeleteAnswer> {
    let Actor::Agent { session_id, .. } = actor else {
        return Err(ApiError(
            axum::http::StatusCode::FORBIDDEN,
            "only an agent session asks to delete".into(),
        ));
    };
    let reason = checked_reason(reason)?;
    let session = state.agent.get(session_id).map_err(agent_error)?;
    if state
        .agent
        .lineage(session_id)
        .map_err(agent_error)?
        .parent_session
        .is_some()
    {
        return Err(ApiError(
            axum::http::StatusCode::FORBIDDEN,
            "an agent you were started by deletes, never you: say what should go in \
             report_result"
                .into(),
        ));
    }
    let handle = vault_of(state, actor, Access::Write, vault).await?;
    let target = {
        let ix = handle.indexer.lock().await;
        snapshot(&ix, vault, &what)?
    };
    let (request, new) = match state
        .agent
        .request_delete(session_id, &target, reason)
        .map_err(agent_error)?
    {
        Ok(made) => made,
        Err(refusal) => {
            let (code, answer) = match refusal {
                Refusal::Busy(open) => (
                    "busy",
                    DeleteAnswer {
                        status: "busy",
                        request: Some(open.id),
                        trash_id: None,
                        expires_at: Some(open.expires_at),
                        message: BUSY_SENTENCE,
                    },
                ),
                Refusal::NotLive => return Err(conflict("session_not_live")),
            };
            audit(
                state,
                EVENT_DELETE_REQUEST,
                &session.owner_user_id,
                None,
                serde_json::json!({
                    "session_id": session.id,
                    "vault_id": target.vault_id,
                    "kind": target.kind,
                    "note_id": target.note_id,
                    "path": target.path,
                    "refused": code,
                }),
            )
            .await;
            return Ok(answer);
        }
    };
    if new {
        let mut detail = target_detail(&request);
        detail["provider"] = session.provider.clone().into();
        detail["reason_chars"] = request.reason.chars().count().into();
        audit(
            state,
            EVENT_DELETE_REQUEST,
            &session.owner_user_id,
            None,
            detail,
        )
        .await;
    }
    let bound = crate::agent::scope::wait_cap(&session.provider);
    let waited = state
        .agent
        .wait_delete(&request.id, bound)
        .await
        .map_err(agent_error)?;
    if waited.state == deletes::TIMED_OUT {
        audit_timed_out(state, &session.owner_user_id, &waited).await;
    }
    Ok(DeleteAnswer::of(&waited))
}

async fn audit_timed_out(state: &Shared, owner: &str, r: &DeleteRequest) {
    let mut detail = target_detail(r);
    detail["outcome"] = r.state.clone().into();
    audit(state, EVENT_DELETE_DECISION, owner, None, detail).await;
}

/// `POST /v1/agent/sessions/{id}/delete-requests/{request}`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideDelete {
    /// `approve` or `decline`.
    pub decision: String,
}

/// The person's answer (149b/c), on the session tier only: the first wins.
/// Approval deletes now, as the agent: under the vault's lock, the target
/// is checked against what was asked for, and anything changed is `gone`.
pub async fn decide_delete_request(
    state: &Shared,
    actor: &Actor,
    device: Option<(&str, &str)>,
    session_id: &str,
    request_id: &str,
    req: DecideDelete,
) -> ApiResult<DeleteRequest> {
    let (owner, session) = owners_session(state, actor, session_id)?;
    let approve = match req.decision.as_str() {
        "approve" => true,
        "decline" => false,
        _ => return Err(bad_request("decision is approve or decline")),
    };
    let request = state
        .agent
        .delete_request(request_id)
        .map_err(agent_error)?
        .filter(|r| r.session_id == session_id)
        .ok_or_else(|| not_found("no such request"))?;
    // The person's handle, to hold the lock both answers take: a decline
    // never lands between an approval's check and its delete.
    let handle = vault_of(state, actor, Access::Read, &request.vault_id).await;
    let decided = match handle {
        Err(_) => decide(state, request_id, GONE, device, None),
        Ok(handle) => {
            let mut ix = handle.indexer.lock().await;
            let current = state
                .agent
                .delete_request(request_id)
                .map_err(agent_error)?
                .ok_or_else(|| not_found("no such request"))?;
            if !current.pending() {
                if current.state == deletes::TIMED_OUT {
                    audit_timed_out(state, &owner, &current).await;
                }
                return Err(conflict(format!("request_{}", current.state)));
            }
            if !approve {
                decide(state, request_id, DECLINED, device, None)
            } else {
                match approved(state, &session, &owner, &mut ix, &current) {
                    Ok(trash_id) => decide(state, request_id, DELETED, device, trash_id),
                    Err(()) => decide(state, request_id, GONE, device, None),
                }
            }
        }
    }?;
    let mut detail = target_detail(&decided);
    detail["outcome"] = decided.state.clone().into();
    detail["trash_id"] = decided.trash_id.clone().into();
    audit(
        state,
        EVENT_DELETE_DECISION,
        &owner,
        device.map(|d| d.0),
        detail,
    )
    .await;
    Ok(decided)
}

fn decide(
    state: &Shared,
    id: &str,
    outcome: &str,
    device: Option<(&str, &str)>,
    trash_id: Option<String>,
) -> ApiResult<DeleteRequest> {
    state
        .agent
        .decide_delete(id, outcome, device, trash_id.as_deref())
        .map_err(agent_error)
}

/// Deletes `r`'s target as its session's agent, if it is still what was
/// asked for and the session may still write there. `Err` is `gone`.
fn approved(
    state: &Shared,
    session: &crate::agent::store::SessionRecord,
    owner: &str,
    ix: &mut crate::index::Indexer,
    r: &DeleteRequest,
) -> Result<Option<String>, ()> {
    let writes = state
        .agent_writes
        .load(std::sync::atomic::Ordering::Relaxed);
    let write_vault = state
        .agent
        .launch_of(&session.id)
        .ok()
        .flatten()
        .and_then(|l| l.write_vault_id)
        .filter(|_| writes);
    let agent = Actor::Agent {
        session_id: session.id.clone(),
        host_id: session.host_id.clone(),
        user_id: owner.to_string(),
        write_vault,
    };
    if let crate::auth::authz::Decision::Deny(_) =
        state
            .vault_policy
            .decide(&agent, &r.vault_id, Access::Write)
    {
        return Err(());
    }
    let what = match r.kind.as_str() {
        NOTE => AgentDelete::Note(r.note_id.clone().unwrap_or_default()),
        _ => AgentDelete::Folder(r.path.clone()),
    };
    let now = snapshot(ix, &r.vault_id, &what).map_err(|_| ())?;
    if now.fingerprint != r.fingerprint {
        return Err(());
    }
    let by = crate::db::DeletedBy::agent(&session.host_id, &session.id, Some(r.reason.clone()));
    let trashed = match what {
        AgentDelete::Note(id) => ix.delete_note(&id, &by),
        AgentDelete::Folder(path) => ix.trash_folder(&path, &by),
    };
    let trashed = match trashed {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(request = %r.id, error = %e, "an approved deletion failed");
            return Err(());
        }
    };
    for seq in &trashed.seqs {
        broadcast_latest(state, ix, *seq);
    }
    Ok(trashed.trash_id)
}

/// A request with what the dialog says of its session: the agent's name,
/// provider and model, and the note it was started from.
#[derive(Debug, Clone, Serialize)]
pub struct DeleteRequestView {
    #[serde(flatten)]
    pub request: DeleteRequest,
    pub session: DeleteSession,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteSession {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::agent::store::Context>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteRequestsPage {
    /// Pass it back as `after` to wait for the next change.
    pub version: u64,
    pub requests: Vec<DeleteRequestView>,
}

/// The longest a listing waits for a change.
pub const LIST_WAIT_CAP: std::time::Duration = std::time::Duration::from_secs(25);

/// Every request waiting for the person, and those answered in the last
/// two minutes (so another device can close its dialog), owner only. With
/// `after` equal to the current version, waits up to `wait` seconds for a
/// change first.
pub async fn delete_requests(
    state: &Shared,
    actor: &Actor,
    after: Option<u64>,
    wait: Option<u64>,
) -> ApiResult<DeleteRequestsPage> {
    let owner = super::crews_owner(actor)?.to_string();
    let mut changed = state.agent.watch_deletes();
    if let Some(after) = after
        && *changed.borrow_and_update() == after
    {
        let bound = std::time::Duration::from_secs(wait.unwrap_or(0)).min(LIST_WAIT_CAP);
        let _ = tokio::time::timeout(bound, changed.changed()).await;
    }
    let version = *changed.borrow();
    let mut requests = Vec::new();
    for r in state.agent.shown_delete_requests().map_err(agent_error)? {
        let Ok(s) = state.agent.get(&r.session_id) else {
            continue;
        };
        if s.owner_user_id != owner {
            continue;
        }
        let launch = state.agent.launch_of(&s.id).ok().flatten();
        requests.push(DeleteRequestView {
            session: DeleteSession {
                id: s.id.clone(),
                name: launch
                    .as_ref()
                    .map_or_else(|| s.workspace.clone(), |l| l.name.clone()),
                title: s.title.clone(),
                provider: s.provider.clone(),
                model: launch.as_ref().and_then(|l| l.model.clone()),
                context: launch.and_then(|l| l.context),
            },
            request: r,
        });
    }
    Ok(DeleteRequestsPage { version, requests })
}
