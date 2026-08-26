//! The server's cryptographic identity: a stable id and an Ed25519 credential.
//!
//! Two halves, deliberately stored differently (A2):
//!
//! - **public metadata** — `key_id`, algorithm, public key — is a row in
//!   `auth.db`, so there is one source of truth for *which* key is active;
//! - **private bytes** are a file at `state/identity/<key_id>.key`, mode
//!   `0600`, so their protection is auditable with `ls -l` and a database dump
//!   never contains a usable secret.
//!
//! The `server_id` is **random, never derived from a key** (A3). Deriving it
//! from a fingerprint would be elegant and would mean rotating the credential
//! changed the server's identity, forcing every paired client to start over.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use data_encoding::{BASE64URL_NOPAD, Encoding, Specification};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::Rng;

use super::db::{AuthDb, CredentialRow, ServerRow};

/// Directory holding private key files, under the state directory.
pub const IDENTITY_DIR: &str = "identity";

pub const ALGORITHM: &str = "ed25519";

/// Crockford base32: no `I`, `L`, `O` or `U`, so an id read aloud or copied
/// off a screen cannot be mistyped into a different valid id.
fn crockford() -> &'static Encoding {
    static ENCODING: std::sync::OnceLock<Encoding> = std::sync::OnceLock::new();
    ENCODING.get_or_init(|| {
        let mut spec = Specification::new();
        spec.symbols.push_str("0123456789ABCDEFGHJKMNPQRSTVWXYZ");
        spec.encoding().expect("a valid base32 alphabet")
    })
}

/// A prefixed id: 26 Crockford base32 characters — 128 random bits.
///
/// Shared across the auth module (`srv_`, `usr_`, and later `dev_`/`ses_`) so
/// every Storm id is minted from one alphabet with one amount of entropy.
pub(super) fn random_id(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", crockford().encode(&bytes))
}

/// The loaded identity, held in memory for the life of the process.
///
/// Deliberately not `Debug`-derived and never serialized: the signing key must
/// not reach a log line, a JSON body or a panic message. The manual impl below
/// is what keeps `?identity` in a `tracing` call from being a key disclosure.
pub struct ServerIdentity {
    pub server_id: String,
    pub name: String,
    pub key_id: String,
    signing: SigningKey,
}

impl std::fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerIdentity")
            .field("server_id", &self.server_id)
            .field("name", &self.name)
            .field("key_id", &self.key_id)
            .field("signing", &"<redacted>")
            .finish()
    }
}

impl ServerIdentity {
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// The public key as the wire carries it: base64url, no padding.
    pub fn public_key_b64(&self) -> String {
        BASE64URL_NOPAD.encode(self.verifying_key().as_bytes())
    }

    /// Signs a client's nonce, proving this host holds the private half of the
    /// key the client pinned from a QR.
    ///
    /// Signs [`challenge_message`] rather than the raw nonce: this endpoint is
    /// unauthenticated, so it will sign whatever anyone sends it. Binding the
    /// domain and the server id into the signed bytes means the result cannot
    /// be replayed as a signature over anything else Storm ever signs — the
    /// ordinary defence against a signing oracle.
    pub fn sign_challenge(&self, nonce: &str) -> String {
        self.sign_domain(&challenge_message(&self.server_id, nonce))
    }

    /// Signs a relay's nonce, proving this host holds the private key behind
    /// the `server_id` it is registering at the relay — what stops one server
    /// from squatting another's id and having clients routed to it.
    ///
    /// Reuses the existing identity key; relay registration is one more thing
    /// it proves possession of, not a reason to mint a second keypair.
    ///
    /// Validates the nonce itself rather than trusting the caller to remember:
    /// this signs whatever a relay sends, exactly like [`sign_challenge`] signs
    /// whatever a client sends, so the same forgeable-field-boundary risk
    /// applies and the same guard belongs here rather than upstream.
    #[allow(dead_code)] // Called by relay registration, landing in a parallel change.
    pub fn sign_relay_auth(&self, nonce: &str) -> std::result::Result<String, &'static str> {
        validate_nonce(nonce)?;
        Ok(self.sign_domain(&relay_auth_message(&self.server_id, nonce)))
    }

    /// Signs `message` and encodes the result the way the wire carries every
    /// signature: base64url, no padding.
    ///
    /// The one signing path shared by every domain this identity signs for,
    /// so adding a domain never means duplicating sign-then-encode — only the
    /// message-building free function differs per domain.
    fn sign_domain(&self, message: &[u8]) -> String {
        BASE64URL_NOPAD.encode(&self.signing.sign(message).to_bytes())
    }
}

/// The exact bytes a challenge signature covers.
///
/// The client rebuilds this from the `server_id` it read out of the QR and the
/// nonce it generated, so both sides must agree byte for byte — a change here
/// is a wire-format change.
pub fn challenge_message(server_id: &str, nonce: &str) -> Vec<u8> {
    format!("storm-challenge:v1:{server_id}:{nonce}").into_bytes()
}

/// The exact bytes a relay-auth signature covers.
///
/// Deliberately a different prefix from [`challenge_message`], not a shared
/// builder with a domain parameter: `sign_challenge` proves this server's
/// identity *to a client*, this proves the right to register *at a relay*,
/// and a signature made for one must be structurally impossible to replay as
/// the other. Two free functions make that a type-level fact instead of
/// something a caller could get wrong by passing the wrong string.
#[allow(dead_code)] // Called by `sign_relay_auth` and relay registration; tested directly here.
pub fn relay_auth_message(server_id: &str, nonce: &str) -> Vec<u8> {
    format!("storm-relay-auth:v1:{server_id}:{nonce}").into_bytes()
}

/// Loads this server's identity, creating it on first boot.
///
/// Creating is the *only* silent path. If the row exists but its key file does
/// not, this fails loudly rather than generating a replacement: a new keypair
/// under an existing `server_id` would look fine from here and break every
/// client that pinned the old public key, with no error anywhere saying so.
pub fn load_or_create(db: &mut AuthDb, state_dir: &Path, now: &str) -> Result<ServerIdentity> {
    if let Some(server) = db.server()? {
        // Invariant: exactly one credential is active. SQLite cannot express
        // it, so it is checked here — and it is a refusal rather than "use the
        // newest", because two active keys means clients pinned to either one
        // are equally right and the server has no way to say which it is. That
        // is decision 40's failure shape: two records of the same fact, quietly
        // disagreeing. It can only arise from editing auth.db by hand.
        let active = db.active_credential_count()?;
        if active > 1 {
            bail!(
                "auth.db has {active} active server credentials; exactly one may be \
                 neither retired nor revoked. Retire the ones that are not in use."
            );
        }
        let credential = db.active_credential()?.with_context(|| {
            format!(
                "auth.db has a server row ({}) but no active credential. \
                 It cannot prove who it is; restore state/ from a backup.",
                server.id
            )
        })?;
        return load(&server, &credential, state_dir);
    }

    create(db, state_dir, now)
}

fn load(
    server: &ServerRow,
    credential: &CredentialRow,
    state_dir: &Path,
) -> Result<ServerIdentity> {
    let path = key_path(state_dir, &credential.key_id);
    let bytes = std::fs::read(&path).with_context(|| {
        format!(
            "reading the server's private key from {}. The identity in auth.db \
             says this file should exist; Storm will not mint a replacement, \
             because a new key under the same server id silently breaks every \
             paired client. Restore it from a backup.",
            path.display()
        )
    })?;
    let secret: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("{} is not a 32-byte key", path.display()))?;
    let signing = SigningKey::from_bytes(&secret);

    // The row and the file are two places; this is the one moment they can be
    // caught disagreeing. Trusting either silently would mean handing clients a
    // public key the server cannot actually sign for.
    if signing.verifying_key().as_bytes() != credential.public_key.as_slice() {
        bail!(
            "the private key at {} does not match the public key recorded for {} in auth.db",
            path.display(),
            credential.key_id
        );
    }

    warn_if_readable_by_others(&path);

    Ok(ServerIdentity {
        server_id: server.id.clone(),
        name: server.name.clone(),
        key_id: credential.key_id.clone(),
        signing,
    })
}

fn create(db: &mut AuthDb, state_dir: &Path, now: &str) -> Result<ServerIdentity> {
    let server_id = random_id("srv_");
    let key_id = format!("key_{}", uuid::Uuid::new_v4());

    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);
    let signing = SigningKey::from_bytes(&secret);

    // The file lands before the row: a key file with no row is an orphan the
    // next boot ignores, while a row with no file is a server that cannot
    // prove who it is and refuses to start.
    write_key_file(&key_path(state_dir, &key_id), &secret)?;

    let server = ServerRow {
        id: server_id.clone(),
        name: default_server_name(),
        created: now.to_string(),
    };
    let credential = CredentialRow {
        key_id: key_id.clone(),
        algorithm: ALGORITHM.to_string(),
        public_key: signing.verifying_key().as_bytes().to_vec(),
        created: now.to_string(),
    };
    db.insert_identity(&server, &credential)?;

    tracing::info!(
        server_id = %server.id,
        name = %server.name,
        key_id = %key_id,
        "generated this server's identity"
    );

    Ok(ServerIdentity {
        server_id,
        name: server.name,
        key_id,
        signing,
    })
}

pub fn key_path(state_dir: &Path, key_id: &str) -> PathBuf {
    state_dir.join(IDENTITY_DIR).join(format!("{key_id}.key"))
}

/// Writes private key bytes at mode 0600, with no window at a wider mode.
///
/// Created *with* the mode rather than chmod-ed afterwards: between the two
/// there is an instant where the file exists and anyone can read it, and a
/// homelab box with other users on it is exactly where that matters.
fn write_key_file(path: &Path, secret: &[u8; 32]) -> Result<()> {
    let dir = path.parent().expect("key path has a parent");
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("tightening {}", dir.display()))?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(secret)
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("flushing {}", path.display()))?;
    Ok(())
}

/// Says so when a key file is wider than 0600 — a warning, not a refusal.
///
/// Refusing to boot would turn a restore that did not preserve permissions
/// into a server that will not start, which is a worse failure than the one
/// being reported. The point is that this is visible in the journal.
fn warn_if_readable_by_others(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                tracing::warn!(
                    path = %path.display(),
                    mode = format!("{mode:o}"),
                    "the server's private key is readable beyond its owner; chmod 600 it"
                );
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// A label for the operator, not an identifier. Renaming is a settings change
/// in a later slice; this only has to be better than a constant on first boot.
fn default_server_name() -> String {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().all(|c| !c.is_control()));
    match hostname {
        Some(name) => name.chars().take(64).collect(),
        None => "Storm".to_string(),
    }
}

/// What a challenge nonce is allowed to be.
///
/// Bounded and printable because the value is echoed into the signed message,
/// and an unauthenticated endpoint should not accept a megabyte of anything.
/// 16 characters is the floor a 128-bit nonce needs in any sane encoding.
pub fn validate_nonce(nonce: &str) -> std::result::Result<(), &'static str> {
    if nonce.len() < 16 {
        return Err("nonce must be at least 16 characters");
    }
    if nonce.len() > 128 {
        return Err("nonce must be at most 128 characters");
    }
    if !nonce
        .bytes()
        .all(|b| b.is_ascii_graphic() && b != b':' && b != b'"')
    {
        return Err("nonce must be printable ASCII without ':' or '\"'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    const NOW: &str = "2026-08-13T00:00:00Z";

    fn fresh(dir: &Path) -> ServerIdentity {
        let mut db = AuthDb::open(dir).unwrap();
        load_or_create(&mut db, dir, NOW).unwrap()
    }

    #[test]
    fn a_first_boot_mints_an_srv_id_and_a_key_file() {
        let dir = tempdir::TempDir::new("storm-identity-new").unwrap();
        let id = fresh(dir.path());

        assert!(id.server_id.starts_with("srv_"), "{}", id.server_id);
        assert_eq!(
            id.server_id.len(),
            "srv_".len() + 26,
            "26 Crockford characters: {}",
            id.server_id
        );
        assert!(
            id.server_id[4..]
                .chars()
                .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c)),
            "not Crockford base32: {}",
            id.server_id
        );
        assert!(id.key_id.starts_with("key_"), "{}", id.key_id);

        let key = key_path(dir.path(), &id.key_id);
        assert!(key.exists(), "no key file at {}", key.display());
        assert_eq!(std::fs::read(&key).unwrap().len(), 32);
    }

    #[cfg(unix)]
    #[test]
    fn the_private_key_is_0600_and_its_directory_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir::TempDir::new("storm-identity-mode").unwrap();
        let id = fresh(dir.path());

        let key = key_path(dir.path(), &id.key_id);
        let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "key file mode is {mode:o}");

        let dir_mode = std::fs::metadata(key.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "identity dir mode is {dir_mode:o}");
    }

    #[test]
    fn the_identity_survives_a_restart() {
        // The whole point of storing it. A regenerated identity would look
        // healthy here and invalidate every client that pinned the old key.
        let dir = tempdir::TempDir::new("storm-identity-restart").unwrap();
        let first = fresh(dir.path());
        let second = fresh(dir.path());

        assert_eq!(first.server_id, second.server_id);
        assert_eq!(first.key_id, second.key_id);
        assert_eq!(first.public_key_b64(), second.public_key_b64());

        let db = AuthDb::open(dir.path()).unwrap();
        assert_eq!(
            db.active_credential_count().unwrap(),
            1,
            "a restart must not add a second credential"
        );
    }

    #[test]
    fn two_servers_get_different_ids() {
        let a = tempdir::TempDir::new("storm-identity-a").unwrap();
        let b = tempdir::TempDir::new("storm-identity-b").unwrap();
        assert_ne!(fresh(a.path()).server_id, fresh(b.path()).server_id);
    }

    #[test]
    fn the_server_id_does_not_move_when_the_key_does() {
        // A3: the id is random and stable, credentials rotate underneath it.
        // Derived-from-the-key would pass every other test in this file and
        // break re-pairing the first time a key changed.
        let dir = tempdir::TempDir::new("storm-identity-a3").unwrap();
        let before = fresh(dir.path());

        // Replace the credential with a different keypair, keeping the key_id.
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let replacement = SigningKey::from_bytes(&secret);
        std::fs::remove_file(key_path(dir.path(), &before.key_id)).unwrap();
        write_key_file(&key_path(dir.path(), &before.key_id), &secret).unwrap();
        {
            let conn = rusqlite::Connection::open(AuthDb::path_in(dir.path())).unwrap();
            conn.execute(
                "UPDATE server_credentials SET public_key = ?1",
                rusqlite::params![replacement.verifying_key().as_bytes().to_vec()],
            )
            .unwrap();
        }

        let after = fresh(dir.path());
        assert_eq!(before.server_id, after.server_id, "id followed the key");
        assert_ne!(
            before.public_key_b64(),
            after.public_key_b64(),
            "the key really did change"
        );
    }

    #[test]
    fn a_second_active_credential_stops_the_server() {
        // Invariant 2 in the data model. Picking the newest would look like it
        // worked and leave half the paired clients unable to verify anything.
        let dir = tempdir::TempDir::new("storm-identity-two").unwrap();
        fresh(dir.path());
        {
            let conn = rusqlite::Connection::open(AuthDb::path_in(dir.path())).unwrap();
            conn.execute(
                "INSERT INTO server_credentials
                     (key_id, algorithm, public_key, created, activated)
                 VALUES ('key_second', 'ed25519', ?1, '2026-08-14T00:00:00Z', '2026-08-14T00:00:00Z')",
                rusqlite::params![vec![3u8; 32]],
            )
            .unwrap();
        }

        let mut db = AuthDb::open(dir.path()).unwrap();
        let err = load_or_create(&mut db, dir.path(), NOW)
            .unwrap_err()
            .to_string();
        assert!(err.contains("2 active server credentials"), "{err}");
    }

    #[test]
    fn a_missing_key_file_is_an_error_not_a_new_keypair() {
        let dir = tempdir::TempDir::new("storm-identity-lost").unwrap();
        let id = fresh(dir.path());
        std::fs::remove_file(key_path(dir.path(), &id.key_id)).unwrap();

        let mut db = AuthDb::open(dir.path()).unwrap();
        let err = load_or_create(&mut db, dir.path(), NOW)
            .unwrap_err()
            .to_string();
        assert!(err.contains(&id.key_id), "the message names no file: {err}");
    }

    #[test]
    fn a_key_that_does_not_match_its_row_is_refused() {
        let dir = tempdir::TempDir::new("storm-identity-mismatch").unwrap();
        let id = fresh(dir.path());

        let mut other = [0u8; 32];
        rand::rng().fill_bytes(&mut other);
        let path = key_path(dir.path(), &id.key_id);
        std::fs::remove_file(&path).unwrap();
        write_key_file(&path, &other).unwrap();

        let mut db = AuthDb::open(dir.path()).unwrap();
        let err = load_or_create(&mut db, dir.path(), NOW)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn a_challenge_signature_verifies_against_the_published_key() {
        let dir = tempdir::TempDir::new("storm-identity-sign").unwrap();
        let id = fresh(dir.path());

        let nonce = "0123456789abcdef0123";
        let signature = id.sign_challenge(nonce);
        let raw = BASE64URL_NOPAD.decode(signature.as_bytes()).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&raw).unwrap();

        // Verified against the key as a client would read it, not against the
        // in-memory one: this is what proves what /v1/server publishes is what
        // /v1/server/challenge signs with.
        let published = BASE64URL_NOPAD
            .decode(id.public_key_b64().as_bytes())
            .unwrap();
        let key = VerifyingKey::from_bytes(&published.as_slice().try_into().unwrap()).unwrap();

        assert!(
            key.verify(&challenge_message(&id.server_id, nonce), &sig)
                .is_ok()
        );
        assert!(
            key.verify(
                &challenge_message(&id.server_id, "another-nonce-here"),
                &sig
            )
            .is_err(),
            "a signature must not verify over a different nonce"
        );
        assert!(
            key.verify(&challenge_message("srv_SOMEONEELSE", nonce), &sig)
                .is_err(),
            "the server id must be bound into the signed bytes"
        );
        assert!(
            key.verify(nonce.as_bytes(), &sig).is_err(),
            "the raw nonce must not be what gets signed"
        );
    }

    #[test]
    fn a_relay_auth_signature_verifies_against_the_published_key() {
        let dir = tempdir::TempDir::new("storm-identity-relay-sign").unwrap();
        let id = fresh(dir.path());

        let nonce = "0123456789abcdef0123";
        let signature = id.sign_relay_auth(nonce).unwrap();
        let raw = BASE64URL_NOPAD.decode(signature.as_bytes()).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&raw).unwrap();

        let published = BASE64URL_NOPAD
            .decode(id.public_key_b64().as_bytes())
            .unwrap();
        let key = VerifyingKey::from_bytes(&published.as_slice().try_into().unwrap()).unwrap();

        assert!(
            key.verify(&relay_auth_message(&id.server_id, nonce), &sig)
                .is_ok()
        );
    }

    #[test]
    fn challenge_and_relay_auth_messages_diverge_on_the_same_inputs() {
        // The domain-separation property itself: same server_id, same nonce,
        // different bytes — because the two prove different things.
        let server_id = "srv_SAMEID";
        let nonce = "0123456789abcdef0123";
        assert_ne!(
            challenge_message(server_id, nonce),
            relay_auth_message(server_id, nonce)
        );
    }

    #[test]
    fn a_challenge_signature_does_not_verify_as_a_relay_auth_signature() {
        let dir = tempdir::TempDir::new("storm-identity-no-cross-domain").unwrap();
        let id = fresh(dir.path());
        let nonce = "0123456789abcdef0123";

        let published = BASE64URL_NOPAD
            .decode(id.public_key_b64().as_bytes())
            .unwrap();
        let key = VerifyingKey::from_bytes(&published.as_slice().try_into().unwrap()).unwrap();

        let challenge_sig = {
            let raw = BASE64URL_NOPAD
                .decode(id.sign_challenge(nonce).as_bytes())
                .unwrap();
            ed25519_dalek::Signature::from_slice(&raw).unwrap()
        };
        let relay_sig = {
            let raw = BASE64URL_NOPAD
                .decode(id.sign_relay_auth(nonce).unwrap().as_bytes())
                .unwrap();
            ed25519_dalek::Signature::from_slice(&raw).unwrap()
        };

        assert!(
            key.verify(&relay_auth_message(&id.server_id, nonce), &challenge_sig)
                .is_err(),
            "a challenge signature must not verify as a relay-auth signature"
        );
        assert!(
            key.verify(&challenge_message(&id.server_id, nonce), &relay_sig)
                .is_err(),
            "a relay-auth signature must not verify as a challenge signature"
        );
    }

    #[test]
    fn the_message_formats_are_pinned_byte_for_byte() {
        // A refactor of the shared signing path must not be able to drift
        // either wire format silently — both clients and relays rebuild these
        // strings independently to verify.
        assert_eq!(
            challenge_message("srv_ABC123", "nonceXYZ"),
            b"storm-challenge:v1:srv_ABC123:nonceXYZ".to_vec()
        );
        assert_eq!(
            relay_auth_message("srv_ABC123", "nonceXYZ"),
            b"storm-relay-auth:v1:srv_ABC123:nonceXYZ".to_vec()
        );
    }

    #[test]
    fn relay_auth_refuses_a_nonce_that_would_forge_field_boundaries() {
        let dir = tempdir::TempDir::new("storm-identity-relay-nonce").unwrap();
        let id = fresh(dir.path());

        assert!(id.sign_relay_auth("0123456789abcdef0123").is_ok());
        assert!(
            id.sign_relay_auth("0123456789abcdef:x").is_err(),
            "a colon must be refused on the relay path exactly as on the challenge path"
        );
        assert!(
            id.sign_relay_auth("0123456789abcdef\"x").is_err(),
            "a quote must be refused on the relay path"
        );
        assert!(
            id.sign_relay_auth("short").is_err(),
            "a too-short nonce must be refused on the relay path"
        );
        assert!(
            id.sign_relay_auth(&"x".repeat(129)).is_err(),
            "a too-long nonce must be refused on the relay path"
        );
    }

    #[test]
    fn nonces_are_bounded_and_printable() {
        assert!(validate_nonce("0123456789abcdef").is_ok());
        assert!(validate_nonce("short").is_err());
        assert!(validate_nonce(&"x".repeat(129)).is_err());
        assert!(validate_nonce("0123456789abcdef\n").is_err());
        assert!(
            validate_nonce("0123456789abcdef:x").is_err(),
            "a colon would let a caller forge the message's field boundaries"
        );
    }
}
