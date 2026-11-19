//! Decision 161, slice 161-1: the inbox, prefs, mutes, pause and test
//! notification APIs. Owner only (session tier) — no MCP tool reads or
//! changes any of this (Q18), so there is nothing here an `stk_` key or an
//! agent can reach.

use serde::{Deserialize, Serialize};

use super::{agent_error, crews_owner};
use crate::agent::notifications::{Mute, Notification, Prefs, PrefsPatch};
use crate::api::{ApiError, ApiResult, Shared, bad_request};
use crate::auth::authz::Actor;
use crate::auth::device_prefs::{self, DevicePrefs, DeviceRow};

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

/// `ids`' bound for `POST /v1/notifications/unread`: generous for "reopen a
/// batch from the inbox", never a vector for one call to touch the whole
/// inbox.
const MAX_UNREAD_IDS: usize = 500;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkUnread {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkUnreadResult {
    pub changed: i64,
}

pub async fn mark_notifications_unread(
    state: &Shared,
    actor: &Actor,
    body: MarkUnread,
) -> ApiResult<MarkUnreadResult> {
    crews_owner(actor)?;
    if body.ids.len() > MAX_UNREAD_IDS {
        return Err(bad_request(format!("at most {MAX_UNREAD_IDS} ids")));
    }
    let changed = state
        .agent
        .mark_notifications_unread(&body.ids)
        .map_err(agent_error)?;
    Ok(MarkUnreadResult {
        changed: changed.len() as i64,
    })
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

/// `device_id` is the calling device (161h item 4), so the test notice
/// targets the device that asked for it rather than whichever active
/// device's id happens to sort lowest.
pub async fn send_test_notification(
    state: &Shared,
    actor: &Actor,
    device_id: Option<&str>,
) -> ApiResult<Notification> {
    crews_owner(actor)?;
    state
        .agent
        .send_test_notification(device_id)
        .map_err(agent_error)
}

// ---- Device prefs and presence (decision 161, slice 161-4) ---------------
//
// Stored in `auth.db`, not `agent.db`: a device belongs to no user and
// outlives any one account's notification prefs, and it is the table
// `AM125` already put the revoke cascade on. Still owner-only, for the same
// reason as everything else in this file (Q18).

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePlatform {
    #[serde(default)]
    pub platform: Option<String>,
}

fn calling_device(device: Option<&str>) -> ApiResult<&str> {
    device.ok_or_else(|| bad_request("this session has no device"))
}

pub async fn get_device_prefs(
    state: &Shared,
    actor: &Actor,
    device: Option<&str>,
    platform: Option<&str>,
) -> ApiResult<DevicePrefs> {
    crews_owner(actor)?;
    let device = calling_device(device)?;
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    Ok(auth_db.ensure_device_prefs(device, platform, &now)?)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevicePrefsPatch {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub classes: Option<Vec<String>>,
    #[serde(default)]
    pub preview: Option<String>,
}

fn validate_device_patch(patch: &DevicePrefsPatch) -> ApiResult<()> {
    if let Some(classes) = &patch.classes {
        for class in classes {
            if !device_prefs::CLASSES.contains(&class.as_str()) {
                return Err(bad_request(format!("unknown notification class {class:?}")));
            }
        }
    }
    if let Some(preview) = &patch.preview
        && !device_prefs::PREVIEWS.contains(&preview.as_str())
    {
        return Err(bad_request(format!("unknown preview level {preview:?}")));
    }
    Ok(())
}

pub async fn put_device_prefs(
    state: &Shared,
    actor: &Actor,
    device: Option<&str>,
    platform: Option<&str>,
    patch: DevicePrefsPatch,
) -> ApiResult<DevicePrefs> {
    crews_owner(actor)?;
    validate_device_patch(&patch)?;
    let device = calling_device(device)?;
    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    auth_db.ensure_device_prefs(device, platform, &now)?;
    Ok(auth_db.update_device_prefs(
        device,
        patch.enabled,
        patch.classes.as_deref(),
        patch.preview.as_deref(),
        &now,
    )?)
}

/// `GET /v1/notifications/devices`: read-only, never creates a row — that
/// is [`get_device_prefs`]'s job, and only for the calling device.
pub async fn list_device_notification_rows(
    state: &Shared,
    actor: &Actor,
) -> ApiResult<Vec<DeviceRow>> {
    crews_owner(actor)?;
    let auth_db = state.auth_db.lock().await;
    Ok(auth_db.list_device_prefs()?)
}

/// `POST /v1/notifications/presence`: in-memory only (161-4's deliver
/// hint), never persisted — a restart simply forgets who was active, same
/// as the session list's own live state. Updates the *named connection*'s
/// presence — never creates one (that only ever happens on connect) — so
/// a device whose stream has already dropped gets a 404, not a ghost
/// entry that outlives it.
pub fn set_presence(
    state: &Shared,
    actor: &Actor,
    device: Option<&str>,
    connection_id: &str,
    raw_state: &str,
    open_session: Option<&str>,
) -> ApiResult<()> {
    crews_owner(actor)?;
    let device = calling_device(device)?;
    if connection_id.is_empty() {
        return Err(bad_request("connection_id must not be empty"));
    }
    let active = match raw_state {
        "active" => true,
        "background" => false,
        _ => return Err(bad_request("state must be \"active\" or \"background\"")),
    };
    let updated = state.agent.update_presence(
        connection_id,
        device,
        active,
        open_session.map(str::to_string),
    );
    if !updated {
        return Err(ApiError(
            axum::http::StatusCode::NOT_FOUND,
            "no such live connection".into(),
        ));
    }
    Ok(())
}
