//! QR-based device pairing — the bridge between an unpaired installation and a
//! paired one.
//!
//! Two flows, both ending at [`consume`]:
//!
//! 1. **Bootstrap** (no users): the server creates a pairing session at boot
//!    and logs the QR URI to the console. The new device scans it, completes
//!    the challenge/verify handshake, and calls `POST /v1/pair` — which
//!    reaches [`consume`].
//! 2. **Add device** (users exist): an authenticated client calls
//!    `POST /v1/pairings`, which reaches [`create`], gets back a QR payload,
//!    renders it, and the new device does the same handshake.
//!
//! The nonce is the trust anchor. The QR carries the server's public key and
//! the nonce; the challenge step proves the server holds the private key; and
//! [`consume`] proves the nonce was valid, fresh, and single-use.

use anyhow::{Context, Result, bail};
use data_encoding::BASE64URL_NOPAD;
use rand::Rng;

use super::db::AuthDb;
use super::identity::random_id;
use super::token;

/// 24 random bytes → 32-char base64url string. Enough entropy that guessing
/// is infeasible; short enough to fit in a QR without noise.
const NONCE_BYTES: usize = 24;

/// The `storm://pair` URI scheme version.
const QR_VERSION: &str = "1";

/// Pairing nonce TTL: 5 minutes.
///
/// Long enough to walk a QR across a room and type a password.
pub const PAIRING_TTL_SECS: i64 = 300;

/// Web bootstrap nonce TTL: 90 seconds.
///
/// Much shorter, because nothing human happens between issuing and consuming
/// one — the page loads and the client spends it immediately. It is also handed
/// to anything that can fetch the page, so its window is the main thing keeping
/// a scraped nonce worthless, and every page load mints one that nobody will
/// consume.
pub const WEB_BOOTSTRAP_TTL_SECS: i64 = 90;

/// Rate-limit: max verification attempts per nonce before it is locked out.
pub const PAIRING_MAX_ATTEMPTS: i32 = 10;

pub const EVENT_PAIRING_ISSUED: &str = "pairing_issued";
pub const EVENT_PAIRING_CONSUMED: &str = "pairing_consumed";

/// Purpose of the pairing session, matching the CHECK constraint in
/// `pairing_sessions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PairingPurpose {
    /// First user on a fresh server. Only possible when the user table is empty.
    FirstUser,
    /// Adding a new device to a server that already has users.
    AddDevice,
    /// A browser that was served Storm's own web client.
    ///
    /// Distinct from the two above rather than folded into either, and that is
    /// deliberate: overloading `FirstUser` would let a browser inherit
    /// first-run semantics it must not have, and would make `created_by IS
    /// NULL` mean two different things. Keeping it separate is what lets the
    /// audit trail say which door a device came through, and gives any future
    /// policy something to attach to. See *Storm Web Bootstrap*.
    WebBootstrap,
}

impl PairingPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            PairingPurpose::FirstUser => "first_user",
            PairingPurpose::AddDevice => "add_device",
            PairingPurpose::WebBootstrap => "web_bootstrap",
        }
    }

    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "first_user" => Ok(PairingPurpose::FirstUser),
            "add_device" => Ok(PairingPurpose::AddDevice),
            "web_bootstrap" => Ok(PairingPurpose::WebBootstrap),
            other => bail!("unknown pairing purpose: {other:?}"),
        }
    }
}

/// A stored pairing session. The nonce itself is never stored — only its
/// blake3 hash.
#[derive(Debug, Clone)]
pub struct PairingSession {
    pub id: String,
    #[allow(dead_code)] // Read by future audit/display; tested here.
    pub purpose: PairingPurpose,
    #[allow(dead_code)]
    pub created_by: Option<String>,
    #[allow(dead_code)]
    pub created: String,
    pub expires: String,
    #[allow(dead_code)]
    pub consumed: Option<String>,
    #[allow(dead_code)]
    pub consumed_by: Option<String>,
    #[allow(dead_code)]
    pub attempts: i32,
}

/// The raw row returned by the DB lookup, before being mapped to a
/// [`PairingSession`].
pub struct PairingRow {
    pub id: String,
    pub purpose: String,
    pub expires: String,
    pub consumed: Option<String>,
    pub attempts: i32,
    /// The peer this nonce was issued to, for `web_bootstrap`. `None` for the
    /// QR purposes, which travel by eye and by hand and are not bound.
    pub peer_ip: Option<String>,
}

/// The payload a client needs to render a QR code.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QrPayload {
    /// Server id, for challenge verification.
    pub sid: String,
    /// Public key, base64url no-pad.
    pub pk: String,
    /// The pairing nonce.
    pub n: String,
    /// Expiry as RFC 3339.
    pub exp: String,
    /// Address hint — the address the server believes it is reachable at.
    pub addr: String,
}

impl QrPayload {
    /// Encodes as a `storm://pair` URI.
    pub fn to_uri(&self) -> String {
        format!(
            "storm://pair?v={version}&sid={sid}&pk={pk}&n={nonce}&exp={exp}&addr={addr}",
            version = QR_VERSION,
            sid = self.sid,
            pk = self.pk,
            nonce = self.n,
            exp = self.exp,
            addr = self.addr,
        )
    }
}

/// What the server returns after a successful pairing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConsumeResult {
    pub device_id: String,
    pub device_secret: String,
    pub server_id: String,
    pub public_key: String,
    pub key_id: String,
}

/// Whether two peer addresses are the same client, for nonce binding.
///
/// Compared as parsed addresses rather than as strings, because a dual-stack
/// listener reports an IPv4 client as `::ffff:192.168.1.5` on one connection
/// and can report `192.168.1.5` on another — textually different, the same
/// machine. Ports are never compared: a browser opens a new connection for the
/// API call, so the port always differs.
pub fn same_peer(a: &str, b: &str) -> bool {
    use std::net::IpAddr;
    fn canonical(s: &str) -> Option<IpAddr> {
        let ip: IpAddr = s.parse().ok()?;
        Some(match ip {
            // Unwrap IPv4-mapped IPv6 so both spellings compare equal.
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => IpAddr::V6(v6),
            },
            v4 => v4,
        })
    }
    match (canonical(a), canonical(b)) {
        (Some(x), Some(y)) => x == y,
        // An address we cannot parse is not one we can claim matches.
        _ => false,
    }
}

/// Generates a pairing nonce: 24 random bytes as base64url without padding.
///
/// The result contains no `:` or `"` (base64url uses `A-Z a-z 0-9 - _` only),
/// so it is safe to embed in the challenge message's domain-separated format.
pub fn generate_nonce() -> String {
    let mut bytes = [0u8; NONCE_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    BASE64URL_NOPAD.encode(&bytes)
}

/// Hashes a nonce for storage. The plaintext is never kept.
pub fn hash_nonce(nonce: &str) -> Vec<u8> {
    token::hash(nonce)
}

/// Creates a new pairing session and returns the plaintext nonce.
///
/// The nonce is returned so the caller can:
/// - Encode it into a QR payload ([`encode_qr`])
/// - Log it to the console (bootstrap case)
/// - Store it in memory for later lookup (bootstrap case)
pub fn create(
    db: &mut AuthDb,
    purpose: PairingPurpose,
    created_by: Option<&str>,
    peer_ip: Option<&str>,
    now: &str,
) -> Result<(String, PairingSession)> {
    let nonce = generate_nonce();
    let nonce_hash = hash_nonce(&nonce);

    let session_id = random_id("pair_");
    let expires = {
        use time::Duration;
        use time::format_description::well_known::Rfc3339;
        let at = time::OffsetDateTime::parse(now, &Rfc3339)
            .context("parsing current time for pairing expiry")?;
        let ttl = match purpose {
            PairingPurpose::WebBootstrap => WEB_BOOTSTRAP_TTL_SECS,
            _ => PAIRING_TTL_SECS,
        };
        (at + Duration::seconds(ttl))
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
    };

    db.insert_pairing_session(
        &session_id,
        &nonce_hash,
        purpose.as_str(),
        created_by,
        peer_ip,
        now,
        &expires,
    )?;

    let session = PairingSession {
        id: session_id,
        purpose,
        created_by: created_by.map(str::to_string),
        created: now.to_string(),
        expires,
        consumed: None,
        consumed_by: None,
        attempts: 0,
    };

    db.record_event(
        EVENT_PAIRING_ISSUED,
        created_by,
        None,
        now,
        &format!(
            r#"{{"session":{:?},"purpose":{}}}"#,
            session.id,
            serde_json::to_string(purpose.as_str()).unwrap_or_default()
        ),
    )?;

    Ok((nonce, session))
}

/// Consumes a pairing nonce: verifies it, creates a device, returns
/// credentials.
///
/// The returned `device_secret` is the one time it exists in plaintext. The
/// caller must send it to the client exactly once and never store it.
pub fn consume(
    db: &mut AuthDb,
    nonce: &str,
    device_name: &str,
    platform: Option<&str>,
    client_version: Option<&str>,
    peer_ip: Option<&str>,
    now: &str,
) -> Result<ConsumeResult> {
    let nonce_hash = hash_nonce(nonce);

    let row = db
        .pairing_session_by_nonce_hash(&nonce_hash)?
        .ok_or_else(|| anyhow::anyhow!("invalid pairing nonce"))?;

    if row.consumed.is_some() {
        bail!("pairing nonce already used");
    }

    // **A bound nonce is only good from where it was issued.** Only
    // `web_bootstrap` sets this: that nonce is handed to anything that can
    // fetch the page, so binding it to the peer is what stops one scraped from
    // a log or a proxy being spent somewhere else. A QR nonce is deliberately
    // unbound — it is carried across the room to a different device, which is
    // its whole purpose.
    if let Some(issued_to) = row.peer_ip.as_deref() {
        match peer_ip {
            Some(now_from) if same_peer(issued_to, now_from) => {}
            _ => bail!("pairing nonce was issued to a different client"),
        }
    }

    // Check expiry.
    use time::format_description::well_known::Rfc3339;
    let expires_at =
        time::OffsetDateTime::parse(&row.expires, &Rfc3339).context("parsing pairing expiry")?;
    let now_at = time::OffsetDateTime::parse(now, &Rfc3339).context("parsing current time")?;
    if now_at > expires_at {
        bail!("pairing nonce expired");
    }

    // Rate-limit: cap the number of verification attempts.
    if row.attempts >= PAIRING_MAX_ATTEMPTS {
        bail!("too many pairing attempts — generate a new QR code");
    }
    db.increment_pairing_attempts(&row.id)?;

    // Create the device.
    let secret = token::mint(super::token::DEVICE_SECRET_PREFIX);
    let (device, device_id) = super::devices::create_paired(
        db,
        device_name,
        platform,
        client_version,
        &secret,
        &row.id,
        now,
    )?;

    // Mark the pairing session consumed.
    db.mark_pairing_consumed(&row.id, &device_id, now)?;

    db.record_event(
        EVENT_PAIRING_CONSUMED,
        None,
        Some(&device_id),
        now,
        &format!(
            r#"{{"session":"{}","purpose":{}}}"#,
            row.id,
            serde_json::to_string(&row.purpose).unwrap_or_default()
        ),
    )?;

    // Server info the client needs to pin.
    let server = db.server()?.context("server identity missing")?;
    let credential = db
        .active_credential()?
        .context("server credential missing")?;

    Ok(ConsumeResult {
        device_id: device.id,
        device_secret: secret,
        server_id: server.id,
        public_key: BASE64URL_NOPAD.encode(&credential.public_key),
        key_id: credential.key_id,
    })
}

/// Builds the QR payload from a pairing session and server identity.
pub fn encode_qr(
    server_id: &str,
    public_key_b64: &str,
    nonce: &str,
    expires: &str,
    addr: &str,
) -> QrPayload {
    QrPayload {
        sid: server_id.to_string(),
        pk: public_key_b64.to_string(),
        n: nonce.to_string(),
        exp: expires.to_string(),
        addr: addr.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::db::{CredentialRow, ServerRow};
    use crate::auth::users::{NewUser, Role};
    /// Seeds a server identity and active credential into an in-memory DB so
    /// that [`consume`] can return the server info the client needs to pin.
    /// This mirrors what [`super::super::identity::create`] does on first boot.
    fn seed_server_identity(db: &mut AuthDb) -> (ServerRow, CredentialRow) {
        let server = ServerRow {
            id: "srv_testserverid000000000".into(),
            name: "Test Server".into(),
            created: "2026-08-16T12:00:00Z".into(),
        };
        let credential = CredentialRow {
            key_id: "key_testkey00000000000000".into(),
            algorithm: "ed25519".into(),
            public_key: vec![0x01; 32], // dummy 32-byte public key
            created: "2026-08-16T12:00:00Z".into(),
        };
        db.insert_identity(&server, &credential).unwrap();
        (server, credential)
    }

    #[test]
    fn generate_nonce_is_printable_ascii_no_colon_no_quote() {
        for _ in 0..256 {
            let nonce = generate_nonce();
            assert_eq!(nonce.len(), 32, "24 bytes → 32 base64url chars");
            assert!(
                nonce
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "nonce contains unexpected character: {nonce:?}"
            );
        }
    }

    #[test]
    fn two_thousand_nonces_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            assert!(seen.insert(generate_nonce()), "nonce repeated");
        }
    }

    #[test]
    fn qr_payload_encodes_as_storm_uri() {
        let payload = QrPayload {
            sid: "srv_test".into(),
            pk: "dGVzdA".into(),
            n: "AAAA".into(),
            exp: "2026-08-16T12:00:00Z".into(),
            addr: "http://192.168.1.100:8080".into(),
        };
        let uri = payload.to_uri();
        assert!(uri.starts_with("storm://pair?v=1&sid=srv_test&pk=dGVzdA&n=AAAA"));
        assert!(uri.contains("exp=2026-08-16T12:00:00Z"));
        assert!(uri.contains("addr=http://192.168.1.100:8080"));
    }

    #[test]
    fn pairing_purpose_round_trips() {
        assert_eq!(PairingPurpose::FirstUser.as_str(), "first_user");
        assert_eq!(PairingPurpose::AddDevice.as_str(), "add_device");
        assert!(matches!(
            PairingPurpose::from_str("first_user").unwrap(),
            PairingPurpose::FirstUser
        ));
        assert!(matches!(
            PairingPurpose::from_str("add_device").unwrap(),
            PairingPurpose::AddDevice
        ));
        assert!(PairingPurpose::from_str("evil").is_err());
    }

    #[tokio::test]
    async fn create_and_consume_round_trip() {
        let mut db = AuthDb::open_in_memory().unwrap();
        let (srv, cred) = seed_server_identity(&mut db);
        let now = "2026-08-16T12:00:00Z";

        let (nonce, session) = create(&mut db, PairingPurpose::FirstUser, None, None, now).unwrap();
        assert!(session.id.starts_with("pair_"));
        assert_eq!(session.purpose, PairingPurpose::FirstUser);
        assert!(session.created_by.is_none());

        let result = consume(
            &mut db,
            &nonce,
            "Pixel 10",
            Some("android"),
            Some("0.3.0"),
            None,
            now,
        )
        .unwrap();
        assert!(result.device_id.starts_with("dev_"));
        assert!(result.device_secret.starts_with("dvs_"));
        assert_eq!(result.server_id, srv.id);
        assert_eq!(result.key_id, cred.key_id);
        assert!(!result.public_key.is_empty());
    }

    #[tokio::test]
    async fn consuming_twice_with_the_same_nonce_fails() {
        let mut db = AuthDb::open_in_memory().unwrap();
        seed_server_identity(&mut db);
        let now = "2026-08-16T12:00:00Z";

        let (nonce, _) = create(&mut db, PairingPurpose::FirstUser, None, None, now).unwrap();
        consume(&mut db, &nonce, "A", None, None, None, now).unwrap();

        let err = consume(&mut db, &nonce, "B", None, None, None, now);
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("already used"),
            "expected 'already used' error"
        );
    }

    #[tokio::test]
    async fn consuming_an_expired_nonce_fails() {
        let mut db = AuthDb::open_in_memory().unwrap();
        seed_server_identity(&mut db);
        let now = "2026-08-16T12:00:00Z";

        let (nonce, _) = create(&mut db, PairingPurpose::FirstUser, None, None, now).unwrap();

        // Step past the TTL.
        let past = "2026-08-16T12:06:00Z";
        let err = consume(&mut db, &nonce, "A", None, None, None, past);
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("expired"),
            "expected 'expired' error"
        );
    }

    #[tokio::test]
    async fn rate_limiting_after_max_attempts() {
        let mut db = AuthDb::open_in_memory().unwrap();
        seed_server_identity(&mut db);
        let now = "2026-08-16T12:00:00Z";

        let (nonce, _session) =
            create(&mut db, PairingPurpose::FirstUser, None, None, now).unwrap();

        // Exhaust the attempts on a *different* session. The rate limit is
        // per-session, so the real session's counter must remain untouched.
        for _ in 0..PAIRING_MAX_ATTEMPTS {
            db.increment_pairing_attempts("pair_nonexistent").ok();
        }

        // The real nonce still works (its session hasn't been bumped).
        let result = consume(&mut db, &nonce, "A", None, None, None, now);
        assert!(
            result.is_ok(),
            "correct nonce should still work, got: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn rate_limiting_kicks_in_when_session_is_exhausted() {
        let mut db = AuthDb::open_in_memory().unwrap();
        seed_server_identity(&mut db);
        let now = "2026-08-16T12:00:00Z";

        let (nonce, session) = create(&mut db, PairingPurpose::FirstUser, None, None, now).unwrap();

        // Exhaust attempts on this specific session.
        for _ in 0..PAIRING_MAX_ATTEMPTS {
            db.increment_pairing_attempts(&session.id).unwrap();
        }

        let err = consume(&mut db, &nonce, "A", None, None, None, now);
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("too many"),
            "expected rate-limit error"
        );
    }

    #[tokio::test]
    async fn purpose_add_device_has_created_by() {
        let mut db = AuthDb::open_in_memory().unwrap();
        let (srv, _) = seed_server_identity(&mut db);
        let now = "2026-08-16T12:00:00Z";

        // The `created_by` FK references a real user. Create one so the
        // FOREIGN KEY constraint is satisfied.
        let user = super::super::users::create_user(
            &mut db,
            NewUser {
                username: "creator",
                display_name: None,
                password_hash: "$argon2id$v=19$m=196608,t=1,p=1$dGVzdA$dGVzdA",
                role: Role::Owner,
            },
            now,
        )
        .unwrap();

        let (nonce, session) = create(
            &mut db,
            PairingPurpose::AddDevice,
            Some(&user.id),
            None,
            now,
        )
        .unwrap();
        assert_eq!(session.created_by.as_deref(), Some(user.id.as_str()));

        let result = consume(&mut db, &nonce, "iPad", None, None, None, now).unwrap();
        assert!(!result.device_id.is_empty());
        assert_eq!(result.server_id, srv.id);
    }
}
