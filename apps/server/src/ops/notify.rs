//! Decision 161, slice 161-2: the MCP `notify`/`cancel_notify` tools.
//! Agent-only, offered only to a session with the "Notifies you" capability
//! (Q3); `ops.rs`'s `Authorized::may_call` has already refused an unoffered
//! call before this is reached, so the check here is the defensive
//! re-check the draft calls for, never the real gate.
//!
//! Everything that writes the inbox lives in `agent::notifications`; this
//! file only validates the call (sanitises `title`/`body`, checks `key`'s
//! shape, resolves and checks `link`) and audits the outcome — metadata
//! only, never the text (G-D18).

use serde::{Deserialize, Serialize};

use super::agent_error;
use crate::agent::notifications::{
    self, AgentNotify, NOTIFY_BODY_MAX, NotifyRefusal, NotifyResult, valid_notify_key,
};
use crate::api::{ApiResult, Shared, vault_of};
use crate::auth::authz::{Access, Actor};

/// `notify`'s `delay_s` bound.
const MAX_DELAY_S: i64 = 3600;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NotifyUrgency {
    Low,
    Normal,
    High,
}

impl NotifyUrgency {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }
}

/// The session (default) or a note it can read. Storm-internal only,
/// resolved and checked here, never a URL.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NotifyLink {
    Session,
    Note {
        /// The vault's id, as returned by `list_vaults`.
        vault: String,
        /// The note's id. Must be one this session can read.
        note: String,
    },
}

fn default_link() -> NotifyLink {
    NotifyLink::Session
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct NotifyReq {
    /// 1–80 characters after Storm's sanitiser (control, bidi and
    /// zero-width characters removed, newlines collapsed, trimmed).
    pub title: String,
    /// 0–500 characters after the same sanitiser; truncated, never
    /// refused, if still over.
    #[serde(default)]
    pub body: Option<String>,
    /// Default "normal".
    #[serde(default)]
    pub urgency: Option<NotifyUrgency>,
    /// `{"kind":"session"}` (default) or
    /// `{"kind":"note","vault":<id>,"note":<id>}`.
    #[serde(default = "default_link")]
    pub link: NotifyLink,
    /// `[A-Za-z0-9._:-]`, 1–64 characters. Replaces this session's earlier
    /// notice under the same key.
    #[serde(default)]
    pub key: Option<String>,
    /// Seconds, 0–3600: deliver later instead of now. Cancellable with
    /// `cancel_notify`. Outside 0–3600 is the tool error `bad_delay`.
    #[serde(default)]
    pub delay_s: Option<i64>,
}

/// What `notify` answers with — the draft's wire shape, built directly
/// rather than through the generic `ApiError` path, since `rate_limited`
/// carries data beyond a message.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum NotifyOutcome {
    Sent(NotifyResult),
    Refused {
        error: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_after_s: Option<i64>,
    },
}

impl NotifyOutcome {
    fn refused(error: &'static str) -> Self {
        Self::Refused {
            error,
            retry_after_s: None,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Sent(_) => "sent",
            Self::Refused { error, .. } => error,
        }
    }
}

async fn audit(state: &Shared, owner: &str, detail: serde_json::Value) {
    let _ = state.auth_db.lock().await.record_event(
        "notify",
        Some(owner),
        None,
        &crate::index::now_rfc3339(),
        &detail.to_string(),
    );
}

/// `notify(title, body?, urgency?, link?, key?, delay_s?)`.
pub async fn notify(state: &Shared, actor: &Actor, req: NotifyReq) -> ApiResult<NotifyOutcome> {
    let Actor::Agent { session_id, .. } = actor else {
        return Ok(NotifyOutcome::refused("not_allowed"));
    };
    if !state.agent.notify_offered(session_id) {
        return Ok(NotifyOutcome::refused("not_allowed"));
    }
    let session = state.agent.get(session_id).map_err(agent_error)?;

    let Some(title) = notifications::checked_title(&req.title) else {
        return Ok(NotifyOutcome::refused("bad_title"));
    };
    let body = req
        .body
        .as_deref()
        .and_then(|b| notifications::sanitised_body(b, NOTIFY_BODY_MAX));
    if let Some(k) = &req.key
        && !valid_notify_key(k)
    {
        return Ok(NotifyOutcome::refused("bad_key"));
    }
    if let Some(s) = req.delay_s
        && !(0..=MAX_DELAY_S).contains(&s)
    {
        return Ok(NotifyOutcome::refused("bad_delay"));
    }
    let (note_vault_id, note_id) = match &req.link {
        NotifyLink::Session => (None, None),
        NotifyLink::Note { vault, note } => match vault_of(state, actor, Access::Read, vault).await
        {
            Ok(handle) => {
                let ix = handle.indexer.lock().await;
                match ix.db.get_note(note) {
                    Ok(Some(_)) => (Some(vault.clone()), Some(note.clone())),
                    _ => return Ok(NotifyOutcome::refused("bad_link")),
                }
            }
            Err(_) => return Ok(NotifyOutcome::refused("bad_link")),
        },
    };
    let urgency = req.urgency.as_ref().map_or("normal", NotifyUrgency::as_str);
    let due = req
        .delay_s
        .filter(|s| *s > 0)
        .map(|s| crate::index::rfc3339_minus_secs(&crate::index::now_rfc3339(), -s));
    let session_title = session
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(session.workspace.as_str())
        .to_string();
    let link_kind = match &req.link {
        NotifyLink::Session => "session",
        NotifyLink::Note { .. } => "note",
    };
    let an = AgentNotify {
        session_id: session_id.clone(),
        provider: session.provider.clone(),
        session_title,
        title,
        body: body.clone(),
        urgency,
        key: req.key.clone(),
        note_vault_id,
        note_id,
        due,
    };
    let title_chars = an.title.chars().count();
    let body_chars = body.as_ref().map_or(0, |b| b.chars().count());
    let outcome = match state.agent.agent_notify(an).map_err(agent_error)? {
        Ok(result) => NotifyOutcome::Sent(result),
        Err(NotifyRefusal::RateLimited { retry_after_s }) => NotifyOutcome::Refused {
            error: "rate_limited",
            retry_after_s: Some(retry_after_s),
        },
        Err(NotifyRefusal::TooManyPending) => NotifyOutcome::refused("too_many_pending"),
    };
    audit(
        state,
        &session.owner_user_id,
        serde_json::json!({
            "session_id": session_id,
            "urgency": urgency,
            "link_kind": link_kind,
            "title_len": title_chars,
            "body_len": body_chars,
            "delayed": req.delay_s.is_some(),
            "outcome": outcome.code(),
        }),
    )
    .await;
    Ok(outcome)
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CancelNotifyReq {
    /// The `key` an earlier `notify` call used.
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CancelNotifyResult {
    pub cancelled: bool,
}

pub async fn cancel_notify(
    state: &Shared,
    actor: &Actor,
    req: CancelNotifyReq,
) -> ApiResult<CancelNotifyResult> {
    let Actor::Agent { session_id, .. } = actor else {
        return Ok(CancelNotifyResult { cancelled: false });
    };
    if !state.agent.notify_offered(session_id) {
        return Ok(CancelNotifyResult { cancelled: false });
    }
    let cancelled = state
        .agent
        .cancel_agent_notify(session_id, &req.key)
        .map_err(agent_error)?;
    Ok(CancelNotifyResult { cancelled })
}
