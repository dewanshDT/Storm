//! The server's VAPID identity (decision 161, AM125, RFC 8292).
//!
//! Stored the same way as the server's own identity key
//! ([`crate::auth::identity`]), and deliberately not reusing that table:
//! the public half is one row in its own `vapid_keypair` table (not
//! `server_credentials` — that table's "exactly one active" invariant,
//! checked at boot, is about the server's signing identity and must never
//! be made to count an unrelated key), and the private half is a file at
//! `state/identity/<key_id>.vapid.key`, mode 0600, created with that mode.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use data_encoding::BASE64URL_NOPAD;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};

use crate::auth::db::AuthDb;
use crate::auth::identity::{
    IDENTITY_DIR, create_private_dir, random_id, warn_if_readable_by_others, write_secret_bytes,
};

/// Loaded once at boot and held for the process's life — signing a VAPID
/// JWT on every push send must not mean re-reading a key file per send.
pub struct VapidKey {
    key_pair: EcdsaKeyPair,
    rng: SystemRandom,
}

impl std::fmt::Debug for VapidKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VapidKey")
            .field("key_pair", &"<redacted>")
            .finish()
    }
}

impl VapidKey {
    /// The public key, uncompressed SEC1 form — what `GET /v1/push/vapid`
    /// serves and what a subscription's `applicationServerKey` is.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.key_pair.public_key().as_ref().to_vec()
    }

    pub fn public_key_b64(&self) -> String {
        BASE64URL_NOPAD.encode(&self.public_key_bytes())
    }

    /// Builds and signs a VAPID JWT (RFC 8292 §2) for a push to `aud`
    /// (the push resource's origin, scheme://host[:port]), valid from
    /// `now` for `ttl` (the caller holds it to the spec's 12-hour ceiling —
    /// this is just arithmetic, not the policy).
    pub fn jwt(&self, aud: &str, sub: &str, now_unix: i64, ttl_secs: i64) -> Result<String> {
        let claims = serde_json::json!({
            "aud": aud,
            "exp": now_unix + ttl_secs,
            "sub": sub,
        });
        super::crypto::vapid_jwt(&self.key_pair, &self.rng, &serde_json::to_vec(&claims)?)
    }
}

fn key_path(state_dir: &Path, key_id: &str) -> PathBuf {
    state_dir
        .join(IDENTITY_DIR)
        .join(format!("{key_id}.vapid.key"))
}

/// Loads the server's VAPID keypair, generating it on first use (decision
/// 161: "generated on first use"). The same "never regenerate under an
/// existing id" rule as [`crate::auth::identity::load_or_create`]: a row
/// with no key file is a hard failure, never a silent replacement — every
/// subscription still out there was created against that specific public
/// key, and a new one would make every registered endpoint undecryptable.
pub fn load_or_create(db: &mut AuthDb, state_dir: &Path) -> Result<VapidKey> {
    if let Some(row) = db.vapid_keypair()? {
        let path = key_path(state_dir, &row.key_id);
        let pkcs8 = std::fs::read(&path).with_context(|| {
            format!(
                "reading the VAPID private key from {}. auth.db says this key exists; Storm \
                 will not mint a replacement, because every push subscription registered \
                 against the old public key would silently stop decrypting. Restore it from \
                 a backup.",
                path.display()
            )
        })?;
        warn_if_readable_by_others(&path, "the VAPID private key");
        let rng = SystemRandom::new();
        let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng)
            .map_err(|_| anyhow::anyhow!("{} is not a valid PKCS#8 P-256 key", path.display()))?;
        if key_pair.public_key().as_ref() != row.public_key.as_slice() {
            anyhow::bail!(
                "the VAPID private key at {} does not match the public key recorded in auth.db",
                path.display()
            );
        }
        tracing::debug!(key_id = %row.key_id, created = %row.created, "loaded this server's VAPID keypair");
        return Ok(VapidKey { key_pair, rng });
    }

    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
        .map_err(|_| anyhow::anyhow!("generating the VAPID keypair failed"))?;
    let key_id = random_id("vapid_");
    let path = key_path(state_dir, &key_id);
    create_private_dir(path.parent().expect("key path has a parent"))?;
    write_secret_bytes(&path, pkcs8.as_ref())?;

    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
        .map_err(|_| anyhow::anyhow!("re-reading the just-generated VAPID keypair failed"))?;
    db.insert_vapid_keypair(
        &key_id,
        key_pair.public_key().as_ref(),
        &crate::index::now_rfc3339(),
    )?;

    tracing::info!(key_id = %key_id, "generated this server's VAPID keypair");
    Ok(VapidKey { key_pair, rng })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    #[test]
    fn first_load_generates_and_persists_a_keypair() {
        let dir = TempDir::new("vapid").unwrap();
        let mut db = AuthDb::open(dir.path()).unwrap();
        let key = load_or_create(&mut db, dir.path()).unwrap();
        assert_eq!(key.public_key_bytes().len(), 65);
    }

    #[test]
    fn a_second_load_reuses_the_same_key() {
        let dir = TempDir::new("vapid").unwrap();
        let mut db = AuthDb::open(dir.path()).unwrap();
        let first = load_or_create(&mut db, dir.path())
            .unwrap()
            .public_key_b64();
        let second = load_or_create(&mut db, dir.path())
            .unwrap()
            .public_key_b64();
        assert_eq!(first, second);
    }

    #[test]
    fn a_missing_key_file_under_an_existing_row_is_a_hard_failure() {
        let dir = TempDir::new("vapid").unwrap();
        let mut db = AuthDb::open(dir.path()).unwrap();
        load_or_create(&mut db, dir.path()).unwrap();
        std::fs::remove_dir_all(dir.path().join(IDENTITY_DIR)).unwrap();
        assert!(load_or_create(&mut db, dir.path()).is_err());
    }

    #[test]
    fn jwt_claims_round_trip() {
        let dir = TempDir::new("vapid").unwrap();
        let mut db = AuthDb::open(dir.path()).unwrap();
        let key = load_or_create(&mut db, dir.path()).unwrap();
        let jwt = key
            .jwt(
                "https://push.example.net",
                "https://github.com/dewanshDT/Storm",
                1_700_000_000,
                3600,
            )
            .unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims = BASE64URL_NOPAD.decode(parts[1].as_bytes()).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&claims).unwrap();
        assert_eq!(v["aud"], "https://push.example.net");
        assert_eq!(v["exp"], 1_700_003_600);
    }
}
