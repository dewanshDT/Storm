//! Who this Runtime Host is, and the wire formats it signs (freeze §5.1–5.4;
//! `PLAN.md` decision 77b).
//!
//! `docs/runtime-vectors.json` pins every format here. `apps/server` checks
//! the same file and this crate cannot depend on it, so the vectors are the
//! only thing that makes the two agree. A mismatch surfaces at runtime as a
//! refused enrollment, indistinguishable from an attack.
//!
//! **The private key is a file, so its protection is `ls -l`-auditable**: a
//! raw 32-byte seed, mode `0600`, in an `identity/` directory of mode `0700`,
//! both *created* with those modes rather than chmod-ed afterwards (A2). It is
//! never logged and never serialized; [`HostKey`]'s `Debug` is written by hand
//! to redact it.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::Rng;
use serde::{Deserialize, Serialize};

pub const HOST_AUTH_PREFIX: &str = "storm-host-auth:v1:";
pub const CHALLENGE_PREFIX: &str = "storm-challenge:v1:";
pub const ENROLL_PREFIX: &str = "storm-enroll:v1:";

pub const IDENTITY_DIR: &str = "identity";
pub const HOST_FILE: &str = "host.json";
/// Where `host.json` goes when the server refuses the key (AM36): a revoked
/// identity is dead, and its absence is what stops launchd restarting the
/// host (AM35's `PathState`) and lets `enroll` run again without `--force`.
pub const REVOKED_FILE: &str = "host.json.revoked";

const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A parsed enrollment string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enrollment {
    pub server_url: String,
    pub server_id: String,
    pub server_pubkey: String,
    pub token: String,
}

impl Enrollment {
    /// Parses `storm-enroll:v1:<server_url>:<server_id>:<server_pubkey>:<token>`
    /// **from the right**: the URL contains colons and none of the last three
    /// fields can.
    pub fn parse(s: &str) -> std::result::Result<Self, &'static str> {
        let s = s.trim();
        let rest = s
            .strip_prefix(ENROLL_PREFIX)
            .ok_or("not a storm-enroll:v1 string")?;
        let mut fields = rest.rsplitn(4, ':');
        let token = fields.next().ok_or("missing token")?;
        let server_pubkey = fields.next().ok_or("missing server key")?;
        let server_id = fields.next().ok_or("missing server id")?;
        let server_url = fields.next().ok_or("missing server url")?;

        let after_scheme = server_url
            .strip_prefix("http://")
            .or_else(|| server_url.strip_prefix("https://"))
            .ok_or("server url must be http:// or https://")?;
        if after_scheme.is_empty() {
            return Err("server url has no host");
        }
        validate_server_id(server_id)?;
        decode_public_key(server_pubkey)?;
        token_id(token).ok_or("malformed enrollment token")?;
        Ok(Self {
            server_url: server_url.to_string(),
            server_id: server_id.to_string(),
            server_pubkey: server_pubkey.to_string(),
            token: token.to_string(),
        })
    }
}

/// `sen_<id>.<secret>` → `hen_<id>`: the public half names the enrollment, and
/// is what the enrollment signature covers.
pub fn token_id(token: &str) -> Option<String> {
    let (public, secret) = token.strip_prefix("sen_")?.split_once('.')?;
    let crockford = public.len() == 26 && public.bytes().all(|b| CROCKFORD.contains(&b));
    let secret_ok = BASE64URL_NOPAD
        .decode(secret.as_bytes())
        .is_ok_and(|raw| raw.len() == 32);
    (crockford && secret_ok).then(|| format!("hen_{public}"))
}

pub fn enroll_message(server_id: &str, token_id: &str) -> Vec<u8> {
    format!("{HOST_AUTH_PREFIX}{server_id}:{token_id}").into_bytes()
}

pub fn connect_message(server_id: &str, host_id: &str, nonce: &str) -> Vec<u8> {
    format!("{HOST_AUTH_PREFIX}{server_id}:{host_id}:{nonce}").into_bytes()
}

/// What the server signs to prove its identity to a client — the host checks
/// it against the key pinned in the enrollment string before trusting anything.
pub fn challenge_message(server_id: &str, nonce: &str) -> Vec<u8> {
    format!("{CHALLENGE_PREFIX}{server_id}:{nonce}").into_bytes()
}

/// The server's id rules, which make the colon in a signed message
/// unambiguous.
pub fn validate_server_id(id: &str) -> std::result::Result<(), &'static str> {
    if id.is_empty() || id.len() > 128 {
        return Err("server id must be 1–128 characters");
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("server id must be ASCII alphanumeric, '_' or '-'");
    }
    Ok(())
}

pub fn validate_host_id(id: &str) -> std::result::Result<(), &'static str> {
    let ok = id
        .strip_prefix("hst_")
        .is_some_and(|rest| rest.len() == 26 && rest.bytes().all(|b| CROCKFORD.contains(&b)));
    if ok { Ok(()) } else { Err("malformed host id") }
}

/// One key, one spelling: unpadded base64url, and the alternatives refused.
pub fn decode_public_key(b64: &str) -> std::result::Result<VerifyingKey, &'static str> {
    let raw = BASE64URL_NOPAD
        .decode(b64.as_bytes())
        .map_err(|_| "key must be unpadded base64url")?;
    let bytes: [u8; 32] = raw.try_into().map_err(|_| "key must be 32 bytes")?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "key is not an Ed25519 point")
}

pub fn verify(key: &VerifyingKey, message: &[u8], signature_b64: &str) -> bool {
    let Ok(raw) = BASE64URL_NOPAD.decode(signature_b64.as_bytes()) else {
        return false;
    };
    let Ok(bytes) = <[u8; 64]>::try_from(raw) else {
        return false;
    };
    key.verify(message, &Signature::from_bytes(&bytes)).is_ok()
}

/// The host's signing key. Never `Debug`-printed, never serialized.
pub struct HostKey {
    pub key_id: String,
    signing: SigningKey,
}

impl std::fmt::Debug for HostKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostKey")
            .field("key_id", &self.key_id)
            .field("signing", &"<redacted>")
            .finish()
    }
}

impl HostKey {
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        let mut id = [0u8; 16];
        rand::rng().fill_bytes(&mut id);
        Self {
            key_id: format!("key_{}", BASE64URL_NOPAD.encode(&id).replace('-', "_")),
            signing: SigningKey::from_bytes(&seed),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_seed(key_id: &str, seed: [u8; 32]) -> Self {
        Self {
            key_id: key_id.into(),
            signing: SigningKey::from_bytes(&seed),
        }
    }

    pub fn public_key_b64(&self) -> String {
        BASE64URL_NOPAD.encode(self.signing.verifying_key().as_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> String {
        BASE64URL_NOPAD.encode(&self.signing.sign(message).to_bytes())
    }

    pub fn path(state_dir: &Path, key_id: &str) -> PathBuf {
        state_dir.join(IDENTITY_DIR).join(format!("{key_id}.key"))
    }

    /// Writes the seed: `0600`, in a `0700` directory, both created with those
    /// modes. `create_new`, so an existing key is never overwritten.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let dir = state_dir.join(IDENTITY_DIR);
        if !dir.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }
        let path = Self::path(state_dir, &self.key_id);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        file.write_all(self.signing.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    pub fn load(state_dir: &Path, key_id: &str) -> Result<Self> {
        let path = Self::path(state_dir, key_id);
        let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let seed: [u8; 32] = raw
            .try_into()
            .map_err(|_| anyhow::anyhow!("{} is not a 32-byte key", path.display()))?;
        Ok(Self {
            key_id: key_id.into(),
            signing: SigningKey::from_bytes(&seed),
        })
    }
}

/// `host.json`: everything the host needs to find and trust its server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostConfig {
    pub server_url: String,
    pub server_id: String,
    /// Pinned at enrollment. Every connect re-verifies the server against it.
    pub server_pubkey: String,
    pub host_id: String,
    pub key_id: String,
}

impl HostConfig {
    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join(HOST_FILE)
    }

    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = Self::path(state_dir);
        if !path.exists() && state_dir.join(REVOKED_FILE).exists() {
            anyhow::bail!(
                "this host is not enrolled: the server revoked it ({} holds the old \
                 enrollment). Enroll it again with `storm-runtime enroll`.",
                state_dir.join(REVOKED_FILE).display()
            );
        }
        let text = fs::read_to_string(&path).with_context(|| {
            format!(
                "reading {} — has this host been enrolled? (storm-runtime enroll)",
                path.display()
            )
        })?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Written atomically (temp file, then rename), `0600`.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let path = Self::path(state_dir);
        let tmp = path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// The server refused this host's key (AM36): `host.json` becomes
    /// `host.json.revoked`, atomically, replacing any older one. Afterwards the
    /// host is visibly unenrolled, on every platform.
    pub fn mark_revoked(state_dir: &Path) -> Result<()> {
        let from = Self::path(state_dir);
        let to = state_dir.join(REVOKED_FILE);
        fs::rename(&from, &to)
            .with_context(|| format!("renaming {} to {}", from.display(), to.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../docs/runtime-vectors.json")).unwrap()
    }

    fn seed(b64: &str) -> [u8; 32] {
        BASE64URL_NOPAD
            .decode(b64.as_bytes())
            .unwrap()
            .try_into()
            .unwrap()
    }

    #[test]
    fn the_wire_formats_match_the_shared_vectors() {
        // The other half of apps/server's identical test (decision 77b).
        let v = vectors();
        let key = HostKey::from_seed(
            "key_v",
            seed(v["host_key"]["seed_b64url"].as_str().unwrap()),
        );
        assert_eq!(key.public_key_b64(), v["host_key"]["public_key_b64url"]);

        let e = &v["enroll"];
        let tid = token_id(e["token"].as_str().unwrap()).unwrap();
        assert_eq!(tid, e["token_id"]);
        let msg = enroll_message(e["server_id"].as_str().unwrap(), &tid);
        assert_eq!(msg, e["message"].as_str().unwrap().as_bytes());
        // Ed25519 is deterministic: the same seed signs the same bytes.
        assert_eq!(key.sign(&msg), e["signature_b64url"]);

        let c = &v["connect"];
        let msg = connect_message(
            c["server_id"].as_str().unwrap(),
            c["host_id"].as_str().unwrap(),
            c["nonce"].as_str().unwrap(),
        );
        assert_eq!(msg, c["message"].as_str().unwrap().as_bytes());
        assert_eq!(key.sign(&msg), c["signature_b64url"]);
    }

    #[test]
    fn enrollment_strings_parse_as_the_vectors_say() {
        let v = vectors();
        for case in v["enrollment_string"]["valid"].as_array().unwrap() {
            let parsed = Enrollment::parse(case["string"].as_str().unwrap()).unwrap();
            assert_eq!(parsed.server_url, case["server_url"]);
            assert_eq!(parsed.server_id, case["server_id"]);
            assert_eq!(parsed.server_pubkey, case["server_pubkey_b64url"]);
            assert_eq!(parsed.token, case["token"]);
        }
        for case in v["enrollment_string"]["invalid"].as_array().unwrap() {
            assert!(
                Enrollment::parse(case["string"].as_str().unwrap()).is_err(),
                "accepted: {}",
                case["why"]
            );
        }
    }

    #[test]
    fn the_key_is_created_private_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let key = HostKey::generate();
        key.save(dir.path()).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join(IDENTITY_DIR)), 0o700);
        assert_eq!(mode(&HostKey::path(dir.path(), &key.key_id)), 0o600);
        assert!(
            key.save(dir.path()).is_err(),
            "create_new: no silent replacement"
        );

        let loaded = HostKey::load(dir.path(), &key.key_id).unwrap();
        assert_eq!(loaded.public_key_b64(), key.public_key_b64());
        assert!(!format!("{loaded:?}").contains(&BASE64URL_NOPAD.encode(key.signing.as_bytes())));
    }

    #[test]
    fn host_json_round_trips_privately() {
        let dir = tempfile::tempdir().unwrap();
        let config = HostConfig {
            server_url: "http://127.0.0.1:8484".into(),
            server_id: "srv_X".into(),
            server_pubkey: "pk".into(),
            host_id: "hst_01HB6V3Z7Q2M4N8P0R5S9T1W3X".into(),
            key_id: "key_x".into(),
        };
        config.save(dir.path()).unwrap();
        assert_eq!(HostConfig::load(dir.path()).unwrap(), config);
        let mode = fs::metadata(HostConfig::path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_revoked_host_is_unenrolled_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let config = HostConfig {
            server_url: "http://127.0.0.1:1".into(),
            server_id: "srv_X".into(),
            server_pubkey: "k".into(),
            host_id: "hst_X".into(),
            key_id: "key_X".into(),
        };
        config.save(dir.path()).unwrap();
        HostConfig::mark_revoked(dir.path()).unwrap();
        assert!(
            !HostConfig::path(dir.path()).exists(),
            "host.json must be gone"
        );
        let kept: HostConfig =
            serde_json::from_slice(&std::fs::read(dir.path().join(REVOKED_FILE)).unwrap()).unwrap();
        assert_eq!(kept, config, "the old enrollment is kept beside it");
        let err = format!("{:#}", HostConfig::load(dir.path()).unwrap_err());
        assert!(err.contains("revoked"), "{err}");
    }

    #[test]
    fn a_key_id_is_safe_between_colons() {
        for _ in 0..64 {
            let k = HostKey::generate();
            assert!(k.key_id.starts_with("key_"));
            assert!(
                k.key_id[4..]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "{}",
                k.key_id
            );
        }
    }
}
