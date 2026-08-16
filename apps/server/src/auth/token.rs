//! Opaque credentials: 256 random bits, stored as a blake3 hash.
//!
//! **These are not passwords and must not be hashed like them** (A5). A token
//! is 256 bits of randomness with nothing to guess, so there is nothing for a
//! memory-hard KDF to slow down — and running Argon2id on every request would
//! cost the measured 173.6 ms *per API call* to defend against an attack that
//! does not exist. Passwords use [`super::password`]; everything here uses
//! blake3. That split is deliberate and is not an inconsistency to tidy up.
//!
//! **There is no JWT and never will be** (A5). A stateless token cannot be
//! revoked without a denylist, and a denylist is the `sessions` table wearing a
//! disguise. Immediate revocation is the entire reason per-device sessions
//! exist.
//!
//! Lookup is by *indexed equality on the hash*, not a byte-by-byte comparison
//! of a secret, so there is no constant-time question to answer here. Adding a
//! `subtle::ConstantTimeEq` would protect a comparison that never happens.

use data_encoding::BASE64URL_NOPAD;
use rand::Rng;

/// Session access token. Sent as `Authorization: Bearer <token>`.
pub const ACCESS_PREFIX: &str = "sta_";
/// Session refresh token. Only ever sent to the refresh endpoint.
pub const REFRESH_PREFIX: &str = "str_";
/// Device secret, half of `Authorization: StormDevice <id>:<secret>`.
pub const DEVICE_SECRET_PREFIX: &str = "dvs_";

/// 32 bytes — the same 256 bits the data model specifies for every credential.
const TOKEN_BYTES: usize = 32;

/// Mints a fresh credential.
///
/// The prefix is human-facing, not structure: the value stays opaque, carrying
/// no decodable state. It exists so a token found in a log or a config file
/// announces what it is and which endpoint it belongs to — pasting a refresh
/// token where an access token goes should fail as a token that does not
/// resolve, not as a puzzle.
pub fn mint(prefix: &str) -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", BASE64URL_NOPAD.encode(&bytes))
}

/// What gets stored. The plaintext token is shown once and never again.
///
/// The hash covers the **whole string, prefix included**, so a stored hash
/// cannot be reused under a different prefix.
pub fn hash(token: &str) -> Vec<u8> {
    blake3::hash(token.as_bytes()).as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_token_carries_its_prefix_and_256_bits() {
        let token = mint(ACCESS_PREFIX);
        assert!(token.starts_with(ACCESS_PREFIX), "{token}");
        let body = &token[ACCESS_PREFIX.len()..];
        assert_eq!(
            BASE64URL_NOPAD.decode(body.as_bytes()).unwrap().len(),
            TOKEN_BYTES,
            "a token must carry a full 256 bits"
        );
    }

    #[test]
    fn two_tokens_are_never_the_same() {
        // A weak or missing RNG here is not a bug that shows up in behaviour —
        // everything keeps working, and every session shares a token.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            assert!(seen.insert(mint(ACCESS_PREFIX)), "a token repeated");
        }
    }

    #[test]
    fn the_hash_is_stable_and_covers_the_prefix() {
        let token = mint(ACCESS_PREFIX);
        assert_eq!(hash(&token), hash(&token), "hashing must be deterministic");

        // Same random body, different prefix, must not collide — otherwise a
        // refresh token would resolve as an access token.
        let body = &token[ACCESS_PREFIX.len()..];
        let as_refresh = format!("{REFRESH_PREFIX}{body}");
        assert_ne!(hash(&token), hash(&as_refresh));
    }

    #[test]
    fn the_stored_form_is_not_the_token() {
        // The point of hashing: a stolen database dump contains nothing that
        // can be sent back as a credential.
        let token = mint(ACCESS_PREFIX);
        let stored = hash(&token);
        assert_ne!(stored.as_slice(), token.as_bytes());
        assert_eq!(stored.len(), 32, "blake3 is a 256-bit digest");
    }
}
