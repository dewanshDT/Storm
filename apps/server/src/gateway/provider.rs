//! STUB (124o) for slice 124n's provider-credential module (decision 124,
//! token sync spec §0.1). Only the names 124o codes against: on merge, 124n's
//! real module replaces this file wholesale.
#![allow(dead_code)]

use serde::Serialize;

/// The one provider credential kind (spec §0.1).
pub const KIND: &str = "claude_subscription_token";

/// A provider credential in the clear. Never `Debug`-printed, never logged.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.iter_mut().for_each(|b| *b = 0);
    }
}

/// The owner's view of the credential (spec §0.3). No field holds the token.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderCredentialView {
    pub kind: &'static str,
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_left: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warn: Option<String>,
    pub hosts: Vec<String>,
}

/// A refusal, by its stable code (spec §0.3, §0.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    BadTokenShape,
    UnknownHost,
    NotYourSession,
    SessionNotStarting,
    WrongProvider,
    HostNotInScope,
    NoCredential,
    Internal,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadTokenShape => "bad_token_shape",
            Self::UnknownHost => "unknown_host",
            Self::NotYourSession => "not_your_session",
            Self::SessionNotStarting => "session_not_starting",
            Self::WrongProvider => "wrong_provider",
            Self::HostNotInScope => "host_not_in_scope",
            Self::NoCredential => "no_credential",
            Self::Internal => "internal",
        }
    }

    pub fn status(self) -> u16 {
        match self {
            Self::BadTokenShape | Self::UnknownHost => 400,
            Self::NotYourSession | Self::NoCredential => 404,
            Self::SessionNotStarting | Self::WrongProvider => 409,
            Self::HostNotInScope => 403,
            Self::Internal => 500,
        }
    }
}

/// STUB (124o): an in-memory stand-in for 124n's sealed row and scope table,
/// keyed by state dir so parallel tests do not share it. Gone on merge.
pub mod stub {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::{Code, KIND, ProviderCredentialView, Secret};

    struct Row {
        token: String,
        minted_at: time::OffsetDateTime,
    }

    type Entry = (Option<Row>, Vec<String>);

    static ROWS: Mutex<Option<HashMap<PathBuf, Entry>>> = Mutex::new(None);

    fn with<T>(dir: &Path, f: impl FnOnce(&mut Entry) -> T) -> T {
        let mut rows = ROWS.lock().unwrap();
        f(rows
            .get_or_insert_with(HashMap::new)
            .entry(dir.to_path_buf())
            .or_insert((None, Vec::new())))
    }

    fn rfc3339(t: time::OffsetDateTime) -> String {
        t.format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default()
    }

    fn expires(row: &Row) -> time::OffsetDateTime {
        row.minted_at + time::Duration::days(365)
    }

    fn live(row: &Row) -> bool {
        expires(row) > time::OffsetDateTime::now_utc()
    }

    pub fn status(dir: &Path) -> ProviderCredentialView {
        with(dir, |(row, hosts)| {
            let mut view = ProviderCredentialView {
                kind: KIND,
                present: false,
                status: None,
                fingerprint: None,
                minted_at: None,
                expires_at: None,
                days_left: None,
                warn: None,
                hosts: hosts.clone(),
            };
            if let Some(row) = row {
                let left = (expires(row) - time::OffsetDateTime::now_utc()).whole_days();
                view.present = true;
                view.status = Some("active".into());
                view.fingerprint =
                    Some(blake3::hash(row.token.as_bytes()).to_hex()[..8].to_string());
                view.minted_at = Some(rfc3339(row.minted_at));
                view.expires_at = Some(rfc3339(expires(row)));
                view.days_left = Some(left.max(0));
                view.warn = (!live(row)).then(|| "expired".into());
            }
            view
        })
    }

    pub fn put(dir: &Path, token: Secret, minted_at: Option<String>) -> Result<(), Code> {
        let minted_at = match minted_at {
            Some(s) => {
                time::OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339)
                    .map_err(|_| Code::BadTokenShape)?
            }
            None => time::OffsetDateTime::now_utc(),
        };
        with(dir, |(row, _)| {
            *row = Some(Row {
                token: token.expose().to_string(),
                minted_at,
            })
        });
        Ok(())
    }

    pub fn delete(dir: &Path) {
        with(dir, |(row, hosts)| {
            *row = None;
            hosts.clear();
        });
    }

    pub fn set_hosts(dir: &Path, new: Vec<String>) {
        with(dir, |(_, hosts)| {
            let mut new = new;
            new.sort();
            new.dedup();
            *hosts = new;
        });
    }

    pub fn deliverable(dir: &Path, host_id: &str) -> Option<&'static str> {
        with(dir, |(row, hosts)| {
            (row.as_ref().is_some_and(live) && hosts.iter().any(|h| h == host_id)).then_some(KIND)
        })
    }

    pub fn pull(dir: &Path, host_id: &str) -> Result<Secret, Code> {
        with(dir, |(row, hosts)| {
            if !hosts.iter().any(|h| h == host_id) {
                return Err(Code::HostNotInScope);
            }
            match row {
                Some(row) if live(row) => Ok(Secret::new(row.token.clone())),
                _ => Err(Code::NoCredential),
            }
        })
    }
}
