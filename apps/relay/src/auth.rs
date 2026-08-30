//! The bytes a relay-registration signature covers, and the rules the nonce
//! inside them must satisfy.
//!
//! # This file is a duplicate, and that is the dangerous part
//!
//! [`relay_auth_message`] and [`validate_nonce`] are **re-derived** from
//! `apps/server/src/auth/identity.rs`. They must agree with it byte for byte,
//! and nothing in either build enforces that:
//!
//! - `apps/server` is a binary crate, so there is no library to depend on;
//! - and it must stay that way. The relay must never become mandatory (R6),
//!   and a shared crate is how "optional" quietly becomes "linked in".
//!
//! So this is a **wire commitment with no compiler enforcement across the two
//! crates**. Drift here does not fail a build or produce a type error — it
//! surfaces as `auth_failed`, which is also what a genuine attack, an expired
//! nonce and a refused binding all look like. That is the worst possible
//! failure signature for a mismatch, so:
//!
//! **If you change either function, change `apps/server/src/auth/identity.rs`
//! in the same commit.** The exact-bytes test below pins this side. A committed
//! test-vector file that both crates read would pin both, and is worth doing
//! the moment a third party (a Dart client, a second relay) needs the same
//! bytes — see the report accompanying this crate.

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// The exact bytes a relay-auth signature covers.
///
/// Mirrors `identity::relay_auth_message`. The domain prefix is deliberately
/// *not* `storm-challenge:v1:`, which is the client-facing identity challenge:
/// a signature proving the right to register at a relay must be structurally
/// impossible to replay as one proving identity to a client.
///
/// `server_id` and both colons are inside the signed bytes. A signature over
/// the prefix and the nonce alone is a different message and must not verify.
pub fn relay_auth_message(server_id: &str, nonce: &str) -> Vec<u8> {
    format!("storm-relay-auth:v1:{server_id}:{nonce}").into_bytes()
}

/// Mirrors `identity::validate_nonce`: 16–128 printable ASCII, no `:`, no `"`.
///
/// The exclusions are the point, not tidiness. [`relay_auth_message`] is
/// colon-delimited, so a nonce containing `:` could forge the message's own
/// field boundaries and make one signature cover a different
/// `(server_id, nonce)` split than either side intended. `"` is excluded
/// because the nonce travels inside JSON.
///
/// Validated **on receipt**, not only at generation (§4): an implementation
/// cannot assume the only nonces it ever sees are its own.
pub fn validate_nonce(nonce: &str) -> Result<(), &'static str> {
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

/// `server_id` is also a colon-delimited field of the signed message, and it
/// is also a URL path segment in the derived `public_address` (§4.3).
///
/// The spec constrains the *nonce* explicitly and says nothing about
/// `server_id`, which is a gap: the field-boundary argument applies to both
/// halves of the split. This is the stricter of the two readings — the charset
/// a `srv_` + Crockford-base32 id already lives in, with no escaping rules to
/// get wrong in either the signed bytes or the URL.
pub fn validate_server_id(server_id: &str) -> Result<(), &'static str> {
    if server_id.is_empty() || server_id.len() > 128 {
        return Err("server_id must be 1–128 characters");
    }
    if !server_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("server_id must be ASCII alphanumeric, '_' or '-'");
    }
    Ok(())
}

/// A registering server's Ed25519 public key.
///
/// Holds the decoded 32 bytes, because **bindings are compared on bytes, never
/// on the base64 string**: padded and unpadded encodings of the same key are
/// different strings, and a string comparison would read that as a key change
/// and refuse a legitimate re-registration for ever (§4.1 has no way back).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicKey {
    bytes: [u8; 32],
}

impl PublicKey {
    /// Decodes the wire form: base64url, no padding, matching
    /// `ServerIdentity::public_key_b64`.
    pub fn from_b64(encoded: &str) -> Result<Self, &'static str> {
        let decoded = BASE64URL_NOPAD
            .decode(encoded.as_bytes())
            .map_err(|_| "public key must be base64url, unpadded")?;
        let bytes: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| "public key must be 32 bytes")?;
        // Parsed once here so a key that is not a valid curve point is refused
        // at the door rather than at every verify.
        VerifyingKey::from_bytes(&bytes).map_err(|_| "public key is not a valid Ed25519 key")?;
        Ok(Self { bytes })
    }

    pub fn to_b64(self) -> String {
        BASE64URL_NOPAD.encode(&self.bytes)
    }

    /// Verifies `sig` over the relay-auth message for `(server_id, nonce)`.
    ///
    /// Every failure — bad encoding, wrong length, bad signature — collapses to
    /// `false`. The caller turns that into one `auth_failed` with one fixed
    /// message, so a registration attacker learns nothing about which check
    /// failed (§6).
    pub fn verify_relay_auth(self, server_id: &str, nonce: &str, sig_b64: &str) -> bool {
        let Ok(sig_bytes) = BASE64URL_NOPAD.decode(sig_b64.as_bytes()) else {
            return false;
        };
        let Ok(sig_bytes) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
            return false;
        };
        let Ok(key) = VerifyingKey::from_bytes(&self.bytes) else {
            return false;
        };
        key.verify(
            &relay_auth_message(server_id, nonce),
            &Signature::from_bytes(&sig_bytes),
        )
        .is_ok()
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A public key is not a secret, but logging 32 raw bytes is noise; the
        // encoded form is what an operator would grep an allowlist for.
        write!(f, "PublicKey({})", self.to_b64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signed_bytes_are_exactly_the_spec_string() {
        // The wire commitment, pinned literally. If this assertion is edited,
        // `apps/server/src/auth/identity.rs::relay_auth_message` must be
        // edited identically in the same commit — nothing else will catch it.
        assert_eq!(
            relay_auth_message("srv_ABC", "nonce-0123456789"),
            b"storm-relay-auth:v1:srv_ABC:nonce-0123456789".to_vec()
        );
    }

    #[test]
    fn the_relay_domain_differs_from_the_client_challenge_domain() {
        // Domain separation asserted, not assumed. The client-facing prefix is
        // `storm-challenge:v1:`; if these ever coincided, one signature would
        // prove both "I may register here" and "I am your server".
        let relay = relay_auth_message("srv_A", "nnnnnnnnnnnnnnnn");
        assert!(relay.starts_with(b"storm-relay-auth:v1:"));
        assert!(!relay.starts_with(b"storm-challenge:v1:"));
    }

    #[test]
    fn nonce_length_bounds_match_the_server() {
        assert!(validate_nonce(&"a".repeat(15)).is_err());
        assert!(validate_nonce(&"a".repeat(16)).is_ok());
        assert!(validate_nonce(&"a".repeat(128)).is_ok());
        assert!(validate_nonce(&"a".repeat(129)).is_err());
    }

    #[test]
    fn a_nonce_may_not_carry_the_message_delimiters() {
        assert!(validate_nonce("aaaaaaaa:aaaaaaaa").is_err());
        assert!(validate_nonce("aaaaaaaa\"aaaaaaaa").is_err());
        // Space is printable but not `is_ascii_graphic`, and the server
        // excludes it too.
        assert!(validate_nonce("aaaaaaaa aaaaaaaa").is_err());
        assert!(validate_nonce("aaaaaaaa\naaaaaaaa").is_err());
        assert!(validate_nonce("aaaaaaaaaaaaaaaa").is_ok());
    }

    #[test]
    fn server_ids_are_restricted_to_a_url_and_delimiter_safe_charset() {
        assert!(validate_server_id("srv_01ARZ3NDEKTSV4RRFFQ69G5FAV").is_ok());
        assert!(validate_server_id("").is_err());
        assert!(validate_server_id("srv_a:b").is_err());
        assert!(validate_server_id("srv_a/b").is_err());
        assert!(validate_server_id(&"a".repeat(129)).is_err());
    }

    #[test]
    fn a_public_key_round_trips_through_its_wire_form() {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let encoded = BASE64URL_NOPAD.encode(signing.verifying_key().as_bytes());
        let key = PublicKey::from_b64(&encoded).unwrap();
        assert_eq!(key.to_b64(), encoded);
    }

    #[test]
    fn a_padded_or_short_key_is_refused() {
        assert!(PublicKey::from_b64("not base64!").is_err());
        assert!(PublicKey::from_b64("AAAA").is_err());
    }
}
