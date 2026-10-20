//! Token sync's launch decision (decision 124, token sync spec §0.4): whether a
//! session's `start` names a provider credential. The manager knows the
//! provider after fallback and the host's capabilities; `ops` knows whether
//! the credential is deliverable to the host at all.

use super::Capabilities;

/// Only Claude Code takes the synced login.
pub const PROVIDER: &str = "claude-code";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialOutcome {
    /// `start` names this kind; the host pulls it.
    Synced(&'static str),
    /// Nothing to deliver: another provider, no credential, or an unticked
    /// host. The host's own login.
    Native,
    /// Deliverable, but the host cannot receive it; the launch says so.
    OldHost,
}

impl CredentialOutcome {
    pub fn kind(self) -> Option<&'static str> {
        match self {
            Self::Synced(kind) => Some(kind),
            _ => None,
        }
    }
}

pub fn decide(
    provider: &str,
    deliverable: Option<&'static str>,
    caps: &Capabilities,
) -> CredentialOutcome {
    match deliverable {
        _ if provider != PROVIDER => CredentialOutcome::Native,
        None => CredentialOutcome::Native,
        Some(kind) if caps.provider_credentials.iter().any(|k| k == kind) => {
            CredentialOutcome::Synced(kind)
        }
        Some(_) => CredentialOutcome::OldHost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIND: &str = "claude_subscription_token";

    fn caps(kinds: &[&str]) -> Capabilities {
        Capabilities {
            provider_credentials: kinds.iter().map(|k| k.to_string()).collect(),
            ..Capabilities::default()
        }
    }

    #[test]
    fn only_claude_code_on_a_capable_host_is_synced() {
        assert_eq!(
            decide("claude-code", Some(KIND), &caps(&[KIND])),
            CredentialOutcome::Synced(KIND)
        );
        assert_eq!(
            decide("claude-code", Some(KIND), &caps(&[])),
            CredentialOutcome::OldHost
        );
        assert_eq!(
            decide("claude-code", Some(KIND), &caps(&["something_else"])),
            CredentialOutcome::OldHost
        );
        assert_eq!(
            decide("claude-code", None, &caps(&[KIND])),
            CredentialOutcome::Native
        );
    }

    #[test]
    fn other_providers_never_get_it() {
        for provider in ["opencode", "shell", "fake", "my-agent", "Claude-Code"] {
            assert_eq!(
                decide(provider, Some(KIND), &caps(&[KIND])),
                CredentialOutcome::Native,
                "{provider}"
            );
        }
    }
}
