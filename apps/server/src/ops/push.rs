//! Decision 161, slice 161-6: VAPID public key and push endpoint
//! registration. Owner only (session tier) — Q18 again: no MCP tool
//! touches any of this, so neither does an `stk_` key or an agent.

use serde::{Deserialize, Serialize};

use super::crews_owner;
use crate::api::{ApiResult, Shared, bad_request, not_found};
use crate::auth::authz::Actor;
use crate::auth::push_endpoints::KINDS;

fn calling_device(device: Option<&str>) -> ApiResult<&str> {
    device.ok_or_else(|| bad_request("this session has no device"))
}

/// `GET /v1/push/vapid`: the server's VAPID public key, base64url — what a
/// browser's `PushManager.subscribe({applicationServerKey: ...})` wants.
pub async fn vapid_public_key(state: &Shared, actor: &Actor) -> ApiResult<VapidPublicKey> {
    crews_owner(actor)?;
    Ok(VapidPublicKey {
        public_key: state.vapid.public_key_b64(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct VapidPublicKey {
    pub public_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushEndpointKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterPushEndpoint {
    pub kind: String,
    pub endpoint: String,
    pub keys: PushEndpointKeys,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushEndpointView {
    pub id: String,
    pub kind: String,
    pub created: String,
}

fn decode_b64u(s: &str) -> ApiResult<Vec<u8>> {
    data_encoding::BASE64URL_NOPAD
        .decode(s.trim_end_matches('=').as_bytes())
        .map_err(|_| bad_request("not valid base64url"))
}

/// `POST /v1/push/endpoints`: registers (or, on a re-subscribe of the same
/// URL, replaces) this device's own push subscription. `endpoint` must be
/// `https://` — nothing here ever dials plain `http://`.
pub async fn register_push_endpoint(
    state: &Shared,
    actor: &Actor,
    device: Option<&str>,
    body: RegisterPushEndpoint,
) -> ApiResult<PushEndpointView> {
    crews_owner(actor)?;
    let device = calling_device(device)?;

    if !KINDS.contains(&body.kind.as_str()) {
        return Err(bad_request(format!("unknown push kind {:?}", body.kind)));
    }
    let url = url::Url::parse(&body.endpoint).map_err(|_| bad_request("endpoint is not a URL"))?;
    if url.scheme() != "https" {
        return Err(bad_request("endpoint must be https"));
    }

    let p256dh = decode_b64u(&body.keys.p256dh)?;
    if p256dh.len() != crate::push::crypto::P256_PUBLIC_KEY_LEN {
        return Err(bad_request(
            "keys.p256dh must be a 65-byte uncompressed P-256 key",
        ));
    }
    let auth_secret = decode_b64u(&body.keys.auth)?;
    if auth_secret.len() != crate::push::crypto::AUTH_SECRET_LEN {
        return Err(bad_request("keys.auth must be 16 bytes"));
    }
    let sealed = state.push_secrets.seal(&auth_secret)?;

    let now = crate::index::now_rfc3339();
    let mut auth_db = state.auth_db.lock().await;
    let endpoint = auth_db.upsert_push_endpoint(
        device,
        &body.kind,
        body.endpoint.trim(),
        &body.keys.p256dh,
        &sealed,
        &now,
    )?;
    Ok(PushEndpointView {
        id: endpoint.id,
        kind: endpoint.kind,
        created: endpoint.created,
    })
}

/// `DELETE /v1/push/endpoints/{id}`: this device's own endpoint only — a
/// request naming another device's row is a 404, never a 403 that would
/// confirm the id exists.
pub async fn delete_push_endpoint(
    state: &Shared,
    actor: &Actor,
    device: Option<&str>,
    id: &str,
) -> ApiResult<()> {
    crews_owner(actor)?;
    let device = calling_device(device)?;
    let mut auth_db = state.auth_db.lock().await;
    if !auth_db.delete_push_endpoint(id, device)? {
        return Err(not_found("no push endpoint with that id on this device"));
    }
    Ok(())
}
