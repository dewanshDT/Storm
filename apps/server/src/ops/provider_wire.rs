//! Token sync's launch wire, server side (decision 124, token sync spec §0.4,
//! §5.8, AM69, AM72): what a launch says about the synced Claude login, and the
//! Host-tier pull that hands the token to the session's own host while it is
//! `starting`. The store and the scope are 124n's; this only asks them.

use serde::Serialize;
use serde::ser::SerializeStruct;

use crate::agent::credential::{CredentialOutcome, PROVIDER};
use crate::agent::store::SessionRecord;
use crate::api::{ApiError, ApiResult, Shared};
use crate::gateway::provider::{Code, Secret};

pub const EVENT_DELIVERED: &str = "provider_credential_delivered";
pub const EVENT_REFUSED: &str = "provider_credential_refused";

/// What a launch did about the synced login, as `LaunchedSession` reports it.
pub(super) async fn launch_credential(
    state: &Shared,
    session: &SessionRecord,
    host_name: &str,
    outcome: CredentialOutcome,
) -> (&'static str, Option<String>) {
    match outcome {
        CredentialOutcome::Synced(_) => ("synced", None),
        CredentialOutcome::OldHost => (
            "old_host",
            Some(format!(
                "{host_name} can't receive your synced Claude login — update storm-runtime. \
                 This session uses the host's own login."
            )),
        ),
        CredentialOutcome::Native if session.provider == PROVIDER => {
            let view = super::provider_credential_status(state).await;
            let expired = view.present
                && view.warn.as_deref() == Some("expired")
                && view.hosts.iter().any(|h| h == &session.host_id);
            if expired {
                (
                    "expired",
                    Some("Your synced Claude login has expired — re-sync it in Settings.".into()),
                )
            } else {
                ("native", None)
            }
        }
        CredentialOutcome::Native => ("native", None),
    }
}

/// The pull's body. `Debug` is derived but redacts, because `Secret`'s does.
#[derive(Debug)]
pub struct PulledCredential {
    pub kind: &'static str,
    pub value: Secret,
    pub expires_at: Option<String>,
}

impl Serialize for PulledCredential {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut out = s.serialize_struct("PulledCredential", 3)?;
        out.serialize_field("kind", self.kind)?;
        out.serialize_field("value", self.value.expose())?;
        out.serialize_field("expires_at", &self.expires_at)?;
        out.end()
    }
}

/// `GET /v1/runtime/sessions/{id}/provider-credential`. Every outcome is
/// audited by its code and the token's fingerprint, never the token.
pub async fn runtime_provider_credential(
    state: &Shared,
    host_id: &str,
    session_id: &str,
) -> ApiResult<PulledCredential> {
    let session = match state.agent.get(session_id) {
        Ok(r) if r.host_id == host_id => Some(r),
        Ok(_) | Err(crate::agent::AgentError::NotFound(_)) => None,
        Err(e) => {
            tracing::error!(error = ?e, "provider credential pull: session lookup failed");
            return Err(refusal(Code::Internal));
        }
    };
    let owner = session.as_ref().map(|r| r.owner_user_id.clone());
    let result = match session {
        Some(r) => pull(state, host_id, &r).await,
        None => Err(Code::NotYourSession),
    };
    let now = crate::index::now_rfc3339();
    let auth_db = state.auth_db.lock().await;
    match result {
        Ok((pulled, fingerprint)) => {
            let detail = serde_json::json!({
                "kind": pulled.kind,
                "fingerprint": fingerprint,
                "session_id": session_id,
                "host_id": host_id,
            });
            let _ = auth_db.record_event(
                EVENT_DELIVERED,
                owner.as_deref(),
                None,
                &now,
                &detail.to_string(),
            );
            tracing::info!(
                host = host_id,
                session = session_id,
                "synced provider credential delivered"
            );
            Ok(pulled)
        }
        Err(code) => {
            let detail = serde_json::json!({
                "kind": crate::gateway::provider::KIND,
                "code": code.as_str(),
                "session_id": session_id,
                "host_id": host_id,
            });
            let _ = auth_db.record_event(
                EVENT_REFUSED,
                owner.as_deref(),
                None,
                &now,
                &detail.to_string(),
            );
            tracing::info!(
                host = host_id,
                session = session_id,
                code = code.as_str(),
                "synced provider credential refused"
            );
            Err(refusal(code))
        }
    }
}

/// The checks in the spec's order (§0.4), then the store's own: scope now,
/// and a row that is active, unexpired and opens.
async fn pull(
    state: &Shared,
    host_id: &str,
    session: &SessionRecord,
) -> Result<(PulledCredential, Option<String>), Code> {
    // Only while starting: a running session never re-pulls, which keeps the
    // window to the moment of launch.
    if session.status != "starting" {
        return Err(Code::SessionNotStarting);
    }
    if session.provider != PROVIDER {
        return Err(Code::WrongProvider);
    }
    // Only a session whose `start` named the credential (AM72), so widening
    // the scope never reaches a launch already in flight.
    let kind = state
        .agent
        .requested_credential(&session.id)
        .ok_or(Code::HostNotInScope)?;
    let value = super::pull_provider_credential(state, host_id, &session.id).await?;
    let view = super::provider_credential_status(state).await;
    Ok((
        PulledCredential {
            kind,
            value,
            expires_at: view.expires_at,
        },
        view.fingerprint,
    ))
}

fn refusal(code: Code) -> ApiError {
    let status = axum::http::StatusCode::from_u16(code.status())
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    ApiError(status, code.as_str().into())
}
