//! Token sync's launch wire, server side (decision 124, token sync spec §0.4,
//! §5.8, AM69, AM72): what a launch says about the synced Claude login, and the
//! Host-tier pull's response body. The checks, the opening and the audit of a
//! pull are `ops::pull_provider_credential`'s.

use serde::Serialize;
use serde::ser::SerializeStruct;

use crate::agent::credential::{CredentialOutcome, PROVIDER};
use crate::agent::store::SessionRecord;
use crate::api::{ApiResult, Shared};
use crate::gateway::provider::{Availability, KIND, Secret};

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
        CredentialOutcome::Native
            if session.provider == PROVIDER
                && super::provider_credential_availability(state, &session.host_id)
                    == Availability::Expired =>
        {
            super::pc_note_expiry(state, time::OffsetDateTime::now_utc()).await;
            (
                "expired",
                Some("Your synced Claude login has expired — re-sync it in Settings.".into()),
            )
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

/// `GET /v1/runtime/sessions/{id}/provider-credential`.
pub async fn runtime_provider_credential(
    state: &Shared,
    host: &crate::auth::hosts::Host,
    session_id: &str,
) -> ApiResult<PulledCredential> {
    let value = super::pull_provider_credential(state, host, session_id).await?;
    let expires_at = super::provider_credential_status(state)
        .await
        .ok()
        .and_then(|v| v.present)
        .map(|p| p.expires_at);
    Ok(PulledCredential {
        kind: KIND,
        value,
        expires_at,
    })
}
