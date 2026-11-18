//! Decision 161, slice 161-1: the inbox, prefs, mutes, pause and test
//! notification APIs. Owner only (session tier) — no MCP tool reads or
//! changes any of this (Q18), so there is nothing here an `stk_` key or an
//! agent can reach.

use serde::{Deserialize, Serialize};

use super::{agent_error, crews_owner};
use crate::agent::notifications::{Mute, Notification, Prefs, PrefsPatch};
use crate::api::{ApiError, ApiResult, Shared, bad_request};
use crate::auth::authz::Actor;

#[derive(Debug, Clone, Serialize)]
pub struct NotificationsPage {
    pub items: Vec<Notification>,
    /// Pass back as `after` for the next page.
    pub cursor: i64,
}

pub async fn list_notifications(
    state: &Shared,
    actor: &Actor,
    after: i64,
    limit: Option<i64>,
) -> ApiResult<NotificationsPage> {
    crews_owner(actor)?;
    let fetched = state
        .agent
        .list_notifications(after, limit.unwrap_or(100))
        .map_err(agent_error)?;
    // The cursor advances past a cancelled row too, or the next page would
    // keep re-fetching it. A cancelled row (a `key` replace, or
    // `cancel_notify`) is never shown here: this is the client's backfill
    // and inbox list, and a row it replaced or withdrew is not something to
    // show or count as unread. It is still in `agent.db` for dedupe.
    let cursor = fetched.last().map_or(after, |n| n.seq);
    let items = fetched.into_iter().filter(|n| !n.cancelled).collect();
    Ok(NotificationsPage { items, cursor })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkRead {
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkReadResult {
    pub read: Vec<String>,
}

pub async fn mark_notifications_read(
    state: &Shared,
    actor: &Actor,
    device: Option<&str>,
    body: MarkRead,
) -> ApiResult<MarkReadResult> {
    crews_owner(actor)?;
    let read = match (body.ids, body.session_id) {
        (Some(ids), None) => state
            .agent
            .mark_notifications_read(&ids, device)
            .map_err(agent_error)?,
        (None, Some(session_id)) => state
            .agent
            .mark_session_notifications_read(&session_id, device)
            .map_err(agent_error)?,
        _ => return Err(bad_request("pass exactly one of ids or session_id")),
    };
    Ok(MarkReadResult { read })
}

pub async fn clear_notifications(state: &Shared, actor: &Actor) -> ApiResult<()> {
    crews_owner(actor)?;
    state.agent.clear_notifications().map_err(agent_error)?;
    Ok(())
}

pub async fn get_notification_prefs(state: &Shared, actor: &Actor) -> ApiResult<Prefs> {
    crews_owner(actor)?;
    state.agent.notification_prefs().map_err(agent_error)
}

/// `PUT /v1/notifications/prefs`'s body: the patch plus the version it was
/// read at (Q15's optimistic concurrency).
#[derive(Debug, Clone, Deserialize)]
pub struct PutPrefsBody {
    pub version: i64,
    #[serde(flatten)]
    pub patch: PrefsPatch,
}

pub async fn put_notification_prefs(
    state: &Shared,
    actor: &Actor,
    body: PutPrefsBody,
) -> ApiResult<Prefs> {
    crews_owner(actor)?;
    state
        .agent
        .update_notification_prefs(body.version, body.patch)
        .map_err(agent_error)
}

pub async fn list_notification_mutes(state: &Shared, actor: &Actor) -> ApiResult<Vec<Mute>> {
    crews_owner(actor)?;
    state.agent.notification_mutes().map_err(agent_error)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutePrefs {
    pub scope: String,
    pub target: String,
    #[serde(default)]
    pub until: Option<String>,
}

pub async fn create_notification_mute(
    state: &Shared,
    actor: &Actor,
    device: Option<&str>,
    body: MutePrefs,
) -> ApiResult<Mute> {
    crews_owner(actor)?;
    state
        .agent
        .mute_notifications(&body.scope, &body.target, body.until.as_deref(), device)
        .map_err(agent_error)
}

pub async fn delete_notification_mute(state: &Shared, actor: &Actor, id: &str) -> ApiResult<()> {
    crews_owner(actor)?;
    if !state.agent.unmute_notifications(id).map_err(agent_error)? {
        return Err(ApiError(
            axum::http::StatusCode::NOT_FOUND,
            "no such mute".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PausePrefs {
    /// An RFC3339 instant; omitted pauses indefinitely ("until I turn them
    /// back on").
    #[serde(default)]
    pub until: Option<String>,
}

pub async fn pause_notifications(
    state: &Shared,
    actor: &Actor,
    body: PausePrefs,
) -> ApiResult<Prefs> {
    crews_owner(actor)?;
    state
        .agent
        .pause_notifications(body.until.as_deref())
        .map_err(agent_error)
}

pub async fn unpause_notifications(state: &Shared, actor: &Actor) -> ApiResult<Prefs> {
    crews_owner(actor)?;
    state.agent.unpause_notifications().map_err(agent_error)
}

pub async fn send_test_notification(state: &Shared, actor: &Actor) -> ApiResult<Notification> {
    crews_owner(actor)?;
    state.agent.send_test_notification().map_err(agent_error)
}
