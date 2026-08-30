//! Relay configuration: a bind address, how to derive `public_address`, and an
//! optional pubkey allowlist.
//!
//! Deliberately small. §4.1 gives the relay exactly one operator decision —
//! allowlist or trust-on-first-use — and everything else here exists to make
//! the process runnable.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::auth::{PublicKey, validate_server_id};

/// A nonce is single-use with a 30-second TTL (§4).
pub const DEFAULT_CHALLENGE_TTL: Duration = Duration::from_secs(30);

/// How long a half-finished handshake may hold a socket.
///
/// Separate from the nonce TTL rather than derived from it: the nonce bounds
/// how long a *challenge* stays answerable, this bounds how long an unanswered
/// socket costs the relay anything. Tying them together would mean a test that
/// shortens one silently shortens the other.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// Scheme, host and port, without a trailing slash — `wss://relay.example`.
    /// `public_address` is `{public_base}/connect/{server_id}` (§4.3): derived,
    /// never allocated, so any client holding a `server_id` can construct it
    /// and the relay hands out no opaque identifier.
    pub public_base: String,
    /// `None` is trust-on-first-use. `Some` is the binding itself, and TOFU is
    /// off entirely — including on first sight (§4.1).
    pub allowlist: Option<Allowlist>,
    pub challenge_ttl: Duration,
    pub handshake_timeout: Duration,
}

impl Config {
    pub fn new(bind: SocketAddr, public_base: impl Into<String>) -> Self {
        Self {
            bind,
            public_base: public_base.into(),
            allowlist: None,
            challenge_ttl: DEFAULT_CHALLENGE_TTL,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    pub fn public_address(&self, server_id: &str) -> String {
        format!("{}/connect/{}", self.public_base, server_id)
    }
}

/// `server_id` → the one public key the operator will accept for it.
#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    entries: HashMap<String, PublicKey>,
}

impl Allowlist {
    pub fn from_entries(entries: impl IntoIterator<Item = (String, PublicKey)>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    pub fn get(&self, server_id: &str) -> Option<PublicKey> {
        self.entries.get(server_id).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Parses the allowlist file: one `server_id <whitespace> pubkey` per line,
    /// `#` comments, blank lines ignored.
    ///
    /// Plain text rather than JSON or TOML so the file is `cat`-auditable and
    /// hand-editable over SSH, the same reasoning that keeps the server's
    /// private key a file whose protection is visible in `ls -l`. It is also
    /// what an operator will diff after a rebind is refused.
    ///
    /// A duplicate `server_id` is an error, not last-wins: two lines claiming
    /// the same id is an operator mistake, and silently picking one of them
    /// decides a security question by file order.
    pub fn parse(text: &str) -> Result<Self> {
        let mut entries = HashMap::new();
        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(server_id), Some(pubkey)) = (parts.next(), parts.next()) else {
                bail!("line {line_no}: expected `<server_id> <pubkey>`");
            };
            if parts.next().is_some() {
                bail!("line {line_no}: unexpected third field");
            }
            validate_server_id(server_id)
                .map_err(|why| anyhow::anyhow!("line {line_no}: {why}"))?;
            let key = PublicKey::from_b64(pubkey)
                .map_err(|why| anyhow::anyhow!("line {line_no}: {why}"))?;
            if entries.insert(server_id.to_string(), key).is_some() {
                bail!("line {line_no}: {server_id} is listed twice");
            }
        }
        Ok(Self { entries })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading the allowlist at {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing the allowlist at {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_encoding::BASE64URL_NOPAD;

    fn key_b64(seed: u8) -> String {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        BASE64URL_NOPAD.encode(signing.verifying_key().as_bytes())
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let text = format!(
            "# storm relay allowlist\n\n  srv_A  {}   # the homelab box\n",
            key_b64(1)
        );
        let list = Allowlist::parse(&text).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list.get("srv_A").unwrap().to_b64(), key_b64(1));
        assert!(list.get("srv_B").is_none());
    }

    #[test]
    fn a_duplicate_server_id_is_refused_rather_than_resolved_by_file_order() {
        let text = format!("srv_A {}\nsrv_A {}\n", key_b64(1), key_b64(2));
        let err = Allowlist::parse(&text).unwrap_err().to_string();
        assert!(err.contains("listed twice"), "{err}");
    }

    #[test]
    fn a_malformed_line_names_its_line_number() {
        let err = Allowlist::parse("srv_A\n").unwrap_err().to_string();
        assert!(err.contains("line 1"), "{err}");
        let err = Allowlist::parse(&format!("srv_A {} extra\n", key_b64(1)))
            .unwrap_err()
            .to_string();
        assert!(err.contains("third field"), "{err}");
    }

    #[test]
    fn a_bad_key_or_id_is_refused_at_load_not_at_registration() {
        assert!(Allowlist::parse("srv_A notbase64!!\n").is_err());
        assert!(Allowlist::parse(&format!("srv:A {}\n", key_b64(1))).is_err());
    }

    #[test]
    fn the_public_address_is_derived_from_the_server_id() {
        let config = Config::new("127.0.0.1:8484".parse().unwrap(), "wss://relay.example");
        assert_eq!(
            config.public_address("srv_A"),
            "wss://relay.example/connect/srv_A"
        );
    }
}
