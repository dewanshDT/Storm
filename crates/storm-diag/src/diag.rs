//! The diagnostics bundle shape (decision 162c), shared so 162-2's host
//! replies and 162-3's `/v1/feedback/preview` carry exactly what this
//! slice's CLI already prints. `schema` is bumped whenever a field's
//! meaning changes, never when one is only added.

use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diag {
    pub schema: u32,
    pub component: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeDiag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerDiag>,
    /// The ring's last N lines, already redacted on insertion and never
    /// re-checked here — this struct trusts its callers the same way the
    /// ring does, which is why both only ever get a `&[Sensitive]` list,
    /// never raw text to redact themselves.
    pub logs: Vec<String>,
    /// Set instead of populating `logs` when there is no running process
    /// to ask — `storm-server diag` / `storm-runtime diag` fall back to
    /// the configured log file, and say so when that isn't available
    /// either.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDiag {
    /// `Isolation::as_str()`: `"off"` or `"per-session"`.
    pub execution_profile: String,
    pub enrolled: bool,
    /// Whether *this* process is the connected daemon. `storm-runtime
    /// diag`, a separate process from any running host, always reports
    /// `false` here — it has no link to ask.
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerDiag {
    pub vault_count: usize,
    /// Configured relays (`Registry::relays`), never their URLs.
    pub relay_count: usize,
    /// Currently registered with (`Registry::registered_relays`), `None`
    /// when counted from a CLI invocation that has no live tunnel client
    /// to ask.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_registered: Option<usize>,
    pub host_count: usize,
    /// Hosts with a live link right now, `None` from the CLI for the same
    /// reason as `relay_registered`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosts_online: Option<usize>,
}

impl Diag {
    pub fn new(component: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            schema: SCHEMA,
            component: component.into(),
            version: version.into(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            runtime: None,
            server: None,
            logs: Vec::new(),
            logs_note: None,
        }
    }

    pub fn with_runtime(mut self, runtime: RuntimeDiag) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn with_server(mut self, server: ServerDiag) -> Self {
        self.server = Some(server);
        self
    }

    pub fn with_logs(mut self, logs: Vec<String>) -> Self {
        self.logs = logs;
        self
    }

    pub fn with_logs_note(mut self, note: impl Into<String>) -> Self {
        self.logs_note = Some(note.into());
        self
    }

    pub fn to_pretty_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::{Sensitive, redact};

    #[test]
    fn schema_is_present_and_versioned() {
        let diag = Diag::new("storm-server", "0.18.0");
        assert_eq!(diag.schema, 1);
        let json = diag.to_pretty_json().unwrap();
        assert!(json.contains("\"schema\": 1"));
    }

    #[test]
    fn bundle_json_carries_no_fixture_secret() {
        let secrets = [
            "Authorization: Bearer stk_SUPERSECRETVALUE0123456789",
            "StormDevice dev_abc:verysecretdevicesecretvalue",
            "storm-enroll:v1:http://10.0.0.5:8484:srv_x:pk_y:tok_SUPERSECRET",
            "reply_to: dewansh.dt@gmail.com",
            "workspace /home/dewansh/Storm/Secret Vault/note.md",
        ];
        let sensitive = vec![Sensitive::vault("Secret Vault"), Sensitive::note("note.md")];
        let logs: Vec<String> = secrets.iter().map(|s| redact(s, &sensitive)).collect();

        let diag = Diag::new("storm-server", "0.18.0")
            .with_server(ServerDiag {
                vault_count: 2,
                relay_count: 1,
                relay_registered: Some(1),
                host_count: 3,
                hosts_online: Some(2),
            })
            .with_logs(logs);

        let json = diag.to_pretty_json().unwrap();
        for needle in [
            "SUPERSECRETVALUE0123456789",
            "verysecretdevicesecretvalue",
            "SUPERSECRET",
            "dewansh.dt@gmail.com",
            "/home/dewansh",
            "Secret Vault",
        ] {
            assert!(!json.contains(needle), "leaked {needle:?} into {json}");
        }
    }

    #[test]
    fn omits_the_other_component_and_absent_fields() {
        let diag = Diag::new("storm-runtime", "0.18.0").with_runtime(RuntimeDiag {
            execution_profile: "off".to_string(),
            enrolled: true,
            online: false,
        });
        let json = diag.to_pretty_json().unwrap();
        assert!(!json.contains("\"server\""));
        assert!(!json.contains("\"logs_note\""));
    }
}
