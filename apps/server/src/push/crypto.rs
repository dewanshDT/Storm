//! RFC 8291 (Web Push message encryption) and RFC 8292 (VAPID), against
//! `ring` directly.
//!
//! **Why hand-rolled:** the repo's crypto rule is `ring`-only (pure Rust, no
//! C toolchain for the musl/zig release build, `apps/server/Cargo.toml`).
//! No maintained Rust web-push/ECE crate builds on `ring` alone — the ones
//! that exist lean on OpenSSL or NSS for the EC math — so this is that
//! option exhausted, not skipped. `ring` already carries every primitive
//! RFC 8291/8292 need (P-256 ECDH in `agreement`, HKDF written out as plain
//! HMAC per the RFC's own pseudocode, AES-128-GCM in `aead`, ECDSA P-256 in
//! `signature`), so nothing here is original cryptography — it is wiring.
//!
//! The low-level functions ([`derive_cek_nonce`], [`build_header`],
//! [`pad`], [`seal`]) are pure and take their ECDH secret as a plain byte
//! slice, which is what makes [`tests::matches_rfc8291_appendix_a`]
//! possible at all: `ring::agreement::EphemeralPrivateKey` can only be
//! constructed by `generate()` (never from fixed bytes — its `rand`
//! dependency is a sealed trait no other crate can implement), so there is
//! no way to reproduce the vector's fixed `as_private` through `ring`'s own
//! ECDH call. The vector's *published* `ecdh_secret` is exactly what
//! sidesteps that: it is this module's own arithmetic (HKDF, header framing,
//! AES-128-GCM) under test, with the EC step taken as the RFC gives it.
//! [`encrypt`] is covered separately, by a live round trip through real
//! `ring` ECDH in `tests::round_trips_and_the_receiver_decrypts_it`.

#[cfg(test)]
use anyhow::Context;
use anyhow::{Result, bail};
use data_encoding::BASE64URL_NOPAD;
use ring::{aead, agreement, hmac, rand::SecureRandom, signature};

/// The uncompressed-point length of a P-256 public key (SEC1 §2.3.3): one
/// 0x04 byte plus two 32-byte coordinates.
pub const P256_PUBLIC_KEY_LEN: usize = 65;
pub const AUTH_SECRET_LEN: usize = 16;
const SALT_LEN: usize = 16;

/// "Payload text by channel" (decision 161): every notice is padded to the
/// same size before encryption, however short, so the push service sees
/// only size, timing and endpoint — never which kind of notice it was.
pub const PAD_TARGET: usize = 1024;

/// The `aes128gcm` content coding's `rs` (record size) header field: an
/// upper bound on one record, not this message's real length — the
/// Appendix A vector uses the same 4096 for a 41-byte message. Comfortably
/// above `PAD_TARGET` plus the padding delimiter and the AEAD tag.
const RECORD_SIZE: u32 = 4096;

/// HKDF-Extract immediately followed by one HKDF-Expand block, written out
/// as the two HMAC-SHA-256 calls RFC 8291 §3.4 and RFC 8188 §2.2 give in
/// pseudocode. Valid only because every `out_len` this module asks for is
/// at most 32 — the SHA-256 output size — so HKDF-Expand never needs a
/// second block (no counter byte beyond the fixed `0x01`).
fn hkdf_one_block(salt: &[u8], ikm: &[u8], info: &[u8], out_len: usize) -> Vec<u8> {
    debug_assert!(out_len <= 32);
    let prk = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, salt), ikm);
    let mut info_and_counter = Vec::with_capacity(info.len() + 1);
    info_and_counter.extend_from_slice(info);
    info_and_counter.push(1);
    let t = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, prk.as_ref()),
        &info_and_counter,
    );
    t.as_ref()[..out_len].to_vec()
}

/// RFC 8291 §3.3: `"WebPush: info" || 0x00 || ua_public || as_public`, both
/// keys in SEC1 uncompressed form. Order matters and is fixed regardless of
/// which side computes it.
fn key_info(ua_public: &[u8], as_public: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(14 + ua_public.len() + as_public.len());
    v.extend_from_slice(b"WebPush: info");
    v.push(0);
    v.extend_from_slice(ua_public);
    v.extend_from_slice(as_public);
    v
}

const CEK_INFO: &[u8] = b"Content-Encoding: aes128gcm\0";
const NONCE_INFO: &[u8] = b"Content-Encoding: nonce\0";

/// RFC 8291 §3.4, both halves: the shared secret combined with the
/// authentication secret (`PRK_key`, `IKM`), then RFC 8188's own derivation
/// of a content encryption key and nonce from `IKM` and the record `salt`.
pub(super) fn derive_cek_nonce(
    ecdh_secret: &[u8],
    auth_secret: &[u8; AUTH_SECRET_LEN],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8; SALT_LEN],
) -> ([u8; 16], [u8; 12]) {
    let ikm = hkdf_one_block(
        auth_secret,
        ecdh_secret,
        &key_info(ua_public, as_public),
        32,
    );
    let cek = hkdf_one_block(salt, &ikm, CEK_INFO, 16);
    let nonce = hkdf_one_block(salt, &ikm, NONCE_INFO, 12);
    (
        cek.try_into().expect("16 bytes requested"),
        nonce.try_into().expect("12 bytes requested"),
    )
}

/// RFC 8188 §2.1's `aes128gcm` header: `salt || rs (4 bytes, big-endian) ||
/// idlen (1 byte) || keyid`. `keyid` is the application server's ECDH
/// public key (RFC 8291 §3.1), so a single-record push message never needs
/// more than this to be decrypted.
pub(super) fn build_header(salt: &[u8; SALT_LEN], rs: u32, as_public: &[u8]) -> Vec<u8> {
    let mut h = Vec::with_capacity(16 + 4 + 1 + as_public.len());
    h.extend_from_slice(salt);
    h.extend_from_slice(&rs.to_be_bytes());
    h.push(as_public.len() as u8);
    h.extend_from_slice(as_public);
    h
}

/// RFC 8188 §2: appends the padding delimiter octet `0x02`, then zero
/// padding, so the result is exactly `target_len` — the server's one
/// record. Refuses rather than truncate when `plaintext` does not fit:
/// callers bound the notice text well below `target_len` already (the
/// preview levels cap it in the hundreds of characters), so this is a
/// programming error, not a thing to paper over.
pub(super) fn pad(plaintext: &[u8], target_len: usize) -> Result<Vec<u8>> {
    if plaintext.len() + 1 > target_len {
        bail!(
            "a {}-byte notice does not fit the {}-byte pad target",
            plaintext.len(),
            target_len
        );
    }
    let mut out = Vec::with_capacity(target_len);
    out.extend_from_slice(plaintext);
    out.push(0x02);
    out.resize(target_len, 0);
    Ok(out)
}

/// AES-128-GCM-seal `padded_plaintext` in place, no associated data (RFC
/// 8188's `aes128gcm` coding uses none), returning ciphertext with its
/// 16-byte tag appended.
pub(super) fn seal(cek: &[u8; 16], nonce: &[u8; 12], padded_plaintext: &[u8]) -> Result<Vec<u8>> {
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, cek)
            .map_err(|_| anyhow::anyhow!("building the AES-128-GCM key failed"))?,
    );
    let mut buf = padded_plaintext.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(*nonce),
        aead::Aad::empty(),
        &mut buf,
    )
    .map_err(|_| anyhow::anyhow!("AES-128-GCM seal failed"))?;
    Ok(buf)
}

/// RFC 8291 end to end: generates its own ephemeral sender ECDH key pair
/// and record salt, derives the content encryption key and nonce against
/// the subscription's `ua_public`/`auth_secret`, pads `plaintext` to
/// [`PAD_TARGET`], and returns the `aes128gcm` header plus ciphertext —
/// exactly the POST body a push service expects.
///
/// `ua_public` must be the subscriber's `p256dh`, 65 bytes, uncompressed
/// SEC1 form (what every browser and UnifiedPush distributor hands back).
pub fn encrypt(
    ua_public: &[u8],
    auth_secret: &[u8; AUTH_SECRET_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    if ua_public.len() != P256_PUBLIC_KEY_LEN {
        bail!("a push subscription's p256dh must be a 65-byte uncompressed P-256 point");
    }
    let rng = ring::rand::SystemRandom::new();
    let as_private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|_| anyhow::anyhow!("generating the ephemeral sender key failed"))?;
    let as_public = as_private
        .compute_public_key()
        .map_err(|_| anyhow::anyhow!("deriving the ephemeral sender's public key failed"))?;
    let mut salt = [0u8; SALT_LEN];
    rng.fill(&mut salt)
        .map_err(|_| anyhow::anyhow!("generating the record salt failed"))?;

    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public);
    let (cek, nonce) = agreement::agree_ephemeral(as_private, &peer, |ecdh_secret| {
        derive_cek_nonce(
            ecdh_secret,
            auth_secret,
            ua_public,
            as_public.as_ref(),
            &salt,
        )
    })
    .map_err(|_| anyhow::anyhow!("the subscription's p256dh is not a valid P-256 point"))?;

    let padded = pad(plaintext, PAD_TARGET)?;
    let ciphertext = seal(&cek, &nonce, &padded)?;
    let mut out = build_header(&salt, RECORD_SIZE, as_public.as_ref());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// RFC 8188 §2.1's header, parsed back apart — used only by this module's
/// own receiver-side test helper ([`tests::decrypt_for_test`]); a real
/// receiver is a browser or a UnifiedPush distributor, never this server.
#[cfg(test)]
struct ParsedHeader<'a> {
    salt: [u8; SALT_LEN],
    as_public: &'a [u8],
    ciphertext: &'a [u8],
}

#[cfg(test)]
fn parse_header(body: &[u8]) -> Result<ParsedHeader<'_>> {
    if body.len() < SALT_LEN + 4 + 1 {
        bail!("push body shorter than the aes128gcm header");
    }
    let salt: [u8; SALT_LEN] = body[..SALT_LEN].try_into().expect("checked above");
    let idlen = body[SALT_LEN + 4] as usize;
    let keyid_start = SALT_LEN + 4 + 1;
    let keyid_end = keyid_start
        .checked_add(idlen)
        .filter(|&e| e <= body.len())
        .context("push body shorter than its own idlen")?;
    Ok(ParsedHeader {
        salt,
        as_public: &body[keyid_start..keyid_end],
        ciphertext: &body[keyid_end..],
    })
}

/// The receiver's half of [`encrypt`] — a real receiver is never this
/// server (push messages are opened in a browser or on the phone), so this
/// exists only to prove the encrypt path round-trips, including that the
/// preview level actually constrains what is inside the ciphertext.
#[cfg(test)]
pub(super) fn decrypt_for_test(
    ua_private: agreement::EphemeralPrivateKey,
    auth_secret: &[u8; AUTH_SECRET_LEN],
    ua_public: &[u8],
    body: &[u8],
) -> Result<Vec<u8>> {
    let parsed = parse_header(body)?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, parsed.as_public);
    let (cek, nonce) = agreement::agree_ephemeral(ua_private, &peer, |ecdh_secret| {
        derive_cek_nonce(
            ecdh_secret,
            auth_secret,
            ua_public,
            parsed.as_public,
            &parsed.salt,
        )
    })
    .map_err(|_| anyhow::anyhow!("the sender's keyid is not a valid P-256 point"))?;

    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek)
            .map_err(|_| anyhow::anyhow!("building the AES-128-GCM key failed"))?,
    );
    let mut buf = parsed.ciphertext.to_vec();
    let opened = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::empty(),
            &mut buf,
        )
        .map_err(|_| anyhow::anyhow!("the push body did not authenticate"))?;

    // RFC 8188 §2: strip trailing zero padding, then the 0x02 delimiter
    // that must remain.
    let last_nonzero = opened
        .iter()
        .rposition(|&b| b != 0)
        .context("a padded record of all zeros")?;
    if opened[last_nonzero] != 0x02 {
        bail!(
            "padding delimiter octet was {:#x}, not 0x02",
            opened[last_nonzero]
        );
    }
    Ok(opened[..last_nonzero].to_vec())
}

/// Seals a `push_endpoints.auth_secret_enc` value at rest (decision 161,
/// AM125: "secrets encrypted at rest, never logged"). AES-256-GCM under a
/// key generated on first use and kept only in memory plus a 0600 file —
/// the same protection the server's own identity key gets. Losing this key
/// loses nothing irreplaceable: it only means every registered device
/// re-subscribes, which an expired or revoked push subscription already
/// requires from time to time.
pub struct SecretKey([u8; 32]);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SecretKey").field(&"<redacted>").finish()
    }
}

const SECRET_KEY_FILE: &str = "push-secrets.key";

impl SecretKey {
    /// Loads the key, generating it on first use. Unlike the VAPID keypair
    /// and the server's own identity key, a missing file here is *not* a
    /// hard failure — see [`crate::auth::push_endpoints`]: every endpoint
    /// sealed under a lost key simply stops decrypting and is dropped the
    /// next time a send to it fails, the same recoverable shape as an
    /// expired browser subscription.
    pub fn load_or_create(state_dir: &std::path::Path) -> Result<Self> {
        use crate::auth::identity::{
            IDENTITY_DIR, create_private_dir, warn_if_readable_by_others, write_secret_bytes,
        };
        let path = state_dir.join(IDENTITY_DIR).join(SECRET_KEY_FILE);
        if let Ok(bytes) = std::fs::read(&path) {
            warn_if_readable_by_others(&path, "the push secret-sealing key");
            let key: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("{} is not a 32-byte key", path.display()))?;
            return Ok(Self(key));
        }
        create_private_dir(path.parent().expect("key path has a parent"))?;
        let mut key = [0u8; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| anyhow::anyhow!("generating the push secret-sealing key failed"))?;
        write_secret_bytes(&path, &key)?;
        Ok(Self(key))
    }

    /// `nonce (12 bytes) || ciphertext || tag`, so opening needs nothing
    /// but the key and this one blob.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let rng = ring::rand::SystemRandom::new();
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| anyhow::anyhow!("generating a seal nonce failed"))?;
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_256_GCM, &self.0)
                .map_err(|_| anyhow::anyhow!("building the secret-sealing key failed"))?,
        );
        let mut buf = plaintext.to_vec();
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::empty(),
            &mut buf,
        )
        .map_err(|_| anyhow::anyhow!("sealing a secret failed"))?;
        let mut out = nonce_bytes.to_vec();
        out.extend_from_slice(&buf);
        Ok(out)
    }

    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        if sealed.len() < 12 {
            bail!("a sealed secret is shorter than its own nonce");
        }
        let (nonce_bytes, ciphertext) = sealed.split_at(12);
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_256_GCM, &self.0)
                .map_err(|_| anyhow::anyhow!("building the secret-sealing key failed"))?,
        );
        let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(|_| anyhow::anyhow!("a sealed secret's nonce is malformed"))?;
        let mut buf = ciphertext.to_vec();
        let opened = key
            .open_in_place(nonce, aead::Aad::empty(), &mut buf)
            .map_err(|_| anyhow::anyhow!("a sealed secret did not authenticate"))?;
        Ok(opened.to_vec())
    }
}

/// RFC 8292 §2: `{"typ":"JWT","alg":"ES256"}` is fixed for every VAPID
/// token, so its base64url form is computed once.
fn jwt_header_b64() -> String {
    BASE64URL_NOPAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#)
}

/// Builds and signs a VAPID JWT (RFC 8292 §2): ES256 over
/// `"{header}.{claims}"`, returned as the three-part compact form. `rng`
/// feeds the signing nonce only — ECDSA signing is not itself
/// deterministic, so a byte-identical token is never expected twice even
/// for the same claims.
pub fn vapid_jwt(
    key_pair: &signature::EcdsaKeyPair,
    rng: &dyn SecureRandom,
    claims_json: &[u8],
) -> Result<String> {
    let signing_input = format!(
        "{}.{}",
        jwt_header_b64(),
        BASE64URL_NOPAD.encode(claims_json)
    );
    let sig = key_pair
        .sign(rng, signing_input.as_bytes())
        .map_err(|_| anyhow::anyhow!("signing the VAPID JWT failed"))?;
    Ok(format!(
        "{signing_input}.{}",
        BASE64URL_NOPAD.encode(sig.as_ref())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};

    // RFC 8291 Appendix A, decoded from the published base64url with
    // internal whitespace removed (fetched verbatim from
    // https://www.rfc-editor.org/rfc/rfc8291.txt). Section 5's prose gives
    // a "Content-Length: 145" that does not match its own example body —
    // a documented erratum in the RFC text, not something this test
    // reproduces; the header (86) plus ciphertext (58) below is the
    // example body exactly as published, at its real length of 144.
    const AS_PUBLIC: [u8; 65] = [
        4, 254, 51, 244, 171, 13, 234, 113, 145, 77, 181, 88, 35, 247, 59, 84, 148, 143, 65, 48,
        109, 146, 7, 50, 219, 185, 165, 154, 83, 40, 100, 130, 32, 14, 89, 122, 123, 123, 194, 96,
        186, 28, 34, 121, 152, 88, 9, 146, 233, 57, 115, 0, 47, 48, 18, 162, 138, 232, 240, 107,
        187, 120, 229, 236, 15,
    ];
    const UA_PUBLIC: [u8; 65] = [
        4, 37, 113, 178, 190, 205, 253, 227, 96, 85, 26, 175, 30, 208, 244, 205, 54, 108, 17, 206,
        190, 85, 95, 137, 188, 183, 177, 134, 165, 51, 57, 23, 49, 104, 236, 226, 235, 224, 24, 89,
        123, 211, 4, 121, 184, 110, 60, 143, 142, 206, 213, 119, 202, 89, 24, 126, 146, 70, 153,
        13, 182, 130, 0, 139, 14,
    ];
    const SALT: [u8; 16] = [
        12, 107, 250, 173, 173, 103, 149, 136, 3, 9, 45, 69, 70, 118, 243, 151,
    ];
    const AUTH_SECRET: [u8; 16] = [
        5, 48, 89, 50, 161, 199, 234, 190, 19, 182, 206, 201, 253, 164, 136, 130,
    ];
    const ECDH_SECRET: [u8; 32] = [
        147, 42, 203, 214, 50, 8, 56, 113, 51, 131, 123, 12, 217, 149, 145, 28, 52, 65, 235, 102,
        0, 9, 152, 97, 74, 89, 39, 39, 174, 246, 145, 43,
    ];
    const CEK: [u8; 16] = [
        160, 136, 85, 91, 78, 12, 69, 220, 182, 92, 223, 66, 136, 162, 241, 78,
    ];
    const NONCE: [u8; 12] = [226, 31, 253, 230, 73, 87, 39, 145, 63, 170, 122, 13];
    const PLAINTEXT: &[u8] = b"When I grow up, I want to be a watermelon";
    const HEADER: [u8; 86] = [
        12, 107, 250, 173, 173, 103, 149, 136, 3, 9, 45, 69, 70, 118, 243, 151, 0, 0, 16, 0, 65, 4,
        254, 51, 244, 171, 13, 234, 113, 145, 77, 181, 88, 35, 247, 59, 84, 148, 143, 65, 48, 109,
        146, 7, 50, 219, 185, 165, 154, 83, 40, 100, 130, 32, 14, 89, 122, 123, 123, 194, 96, 186,
        28, 34, 121, 152, 88, 9, 146, 233, 57, 115, 0, 47, 48, 18, 162, 138, 232, 240, 107, 187,
        120, 229, 236, 15,
    ];
    const CIPHERTEXT: [u8; 58] = [
        242, 151, 222, 91, 66, 155, 186, 113, 83, 211, 164, 174, 12, 170, 9, 31, 212, 37, 243, 180,
        181, 65, 74, 221, 138, 179, 122, 25, 193, 187, 176, 92, 245, 203, 91, 42, 46, 5, 98, 213,
        88, 99, 86, 65, 236, 82, 129, 44, 108, 143, 244, 46, 149, 204, 184, 107, 231, 205,
    ];

    #[test]
    fn hkdf_derivation_matches_rfc8291_appendix_a() {
        let (cek, nonce) =
            derive_cek_nonce(&ECDH_SECRET, &AUTH_SECRET, &UA_PUBLIC, &AS_PUBLIC, &SALT);
        assert_eq!(cek, CEK, "content encryption key");
        assert_eq!(nonce, NONCE, "nonce");
    }

    #[test]
    fn header_matches_rfc8291_appendix_a() {
        assert_eq!(build_header(&SALT, 4096, &AS_PUBLIC), HEADER);
    }

    #[test]
    fn full_push_body_matches_rfc8291_appendix_a() {
        let (cek, nonce) =
            derive_cek_nonce(&ECDH_SECRET, &AUTH_SECRET, &UA_PUBLIC, &AS_PUBLIC, &SALT);
        // The vector's own plaintext has no zero padding beyond the single
        // delimiter octet — `pad()` always adds some, so it is bypassed
        // here to match the published example exactly.
        let mut padded = PLAINTEXT.to_vec();
        padded.push(0x02);
        let ciphertext = seal(&cek, &nonce, &padded).unwrap();
        assert_eq!(ciphertext, CIPHERTEXT);

        let mut body = build_header(&SALT, 4096, &AS_PUBLIC);
        body.extend_from_slice(&ciphertext);
        let mut expected = HEADER.to_vec();
        expected.extend_from_slice(&CIPHERTEXT);
        assert_eq!(body, expected);
    }

    #[test]
    fn pad_reaches_exactly_the_target_length_with_a_0x02_delimiter() {
        let padded = pad(b"hi", 16).unwrap();
        assert_eq!(padded.len(), 16);
        assert_eq!(&padded[..2], b"hi");
        assert_eq!(padded[2], 0x02);
        assert!(padded[3..].iter().all(|&b| b == 0));
    }

    #[test]
    fn pad_refuses_a_plaintext_that_does_not_fit() {
        assert!(pad(&[0u8; 16], 16).is_err());
    }

    #[test]
    fn round_trips_and_the_receiver_decrypts_it() {
        let rng = ring::rand::SystemRandom::new();
        let ua_private =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = ua_private.compute_public_key().unwrap();
        let mut auth_secret = [0u8; AUTH_SECRET_LEN];
        rng.fill(&mut auth_secret).unwrap();

        let plaintext = b"tidier \xc2\xb7 needs you";
        let body = encrypt(ua_public.as_ref(), &auth_secret, plaintext).unwrap();

        let opened = decrypt_for_test(ua_private, &auth_secret, ua_public.as_ref(), &body).unwrap();
        assert_eq!(opened, plaintext);
    }

    /// The preview level is applied to the plaintext *before* `encrypt`
    /// ever sees it (decision 161: "applied on the server before
    /// encryption"), so this proves the ciphertext carries only what the
    /// device's preview setting allows — not the fuller text a higher
    /// level would have let through. Decrypting is what makes this a real
    /// assertion about the wire bytes, not just about the caller's input.
    #[test]
    fn the_encrypted_payload_carries_only_what_the_preview_level_allowed() {
        let rng = ring::rand::SystemRandom::new();
        let ua_private =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = ua_private.compute_public_key().unwrap();
        let mut auth_secret = [0u8; AUTH_SECRET_LEN];
        rng.fill(&mut auth_secret).unwrap();

        let full = br#"{"title":"tidier needs you","body":"approve rm -rf build/"}"#;
        let storm_only = br#"{"title":"Storm"}"#;

        // "Storm only" is what the server actually sends when the
        // endpoint's own preview level is `storm` — the full agent text
        // never reaches `encrypt` at all for that device.
        let body = encrypt(ua_public.as_ref(), &auth_secret, storm_only).unwrap();
        let opened = decrypt_for_test(ua_private, &auth_secret, ua_public.as_ref(), &body).unwrap();
        assert_eq!(opened, storm_only);
        assert_ne!(
            opened, full,
            "the fuller text must never have been encrypted"
        );
    }

    #[test]
    fn vapid_jwt_verifies_with_its_own_public_key() {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap();

        let claims = br#"{"aud":"https://push.example.net","exp":1999999999,"sub":"https://github.com/dewanshDT/Storm"}"#;
        let jwt = vapid_jwt(&key_pair, &rng, claims).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "header.claims.signature");

        let header = BASE64URL_NOPAD.decode(parts[0].as_bytes()).unwrap();
        assert_eq!(header, br#"{"typ":"JWT","alg":"ES256"}"#);
        let decoded_claims = BASE64URL_NOPAD.decode(parts[1].as_bytes()).unwrap();
        assert_eq!(decoded_claims, claims);

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let sig = BASE64URL_NOPAD.decode(parts[2].as_bytes()).unwrap();
        let public_key_bytes = {
            use ring::signature::KeyPair;
            key_pair.public_key().as_ref().to_vec()
        };
        signature::UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public_key_bytes)
            .verify(signing_input.as_bytes(), &sig)
            .expect("the JWT's signature must verify under its own public key");
    }

    #[test]
    fn vapid_jwt_signature_does_not_verify_under_a_different_key() {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8_a = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key_a =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8_a.as_ref(), &rng)
                .unwrap();
        let pkcs8_b = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key_b =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8_b.as_ref(), &rng)
                .unwrap();

        let claims = br#"{"aud":"https://push.example.net","exp":1999999999}"#;
        let jwt = vapid_jwt(&key_a, &rng, claims).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let sig = BASE64URL_NOPAD.decode(parts[2].as_bytes()).unwrap();

        let public_key_b = {
            use ring::signature::KeyPair;
            key_b.public_key().as_ref().to_vec()
        };
        assert!(
            signature::UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public_key_b)
                .verify(signing_input.as_bytes(), &sig)
                .is_err()
        );
    }

    #[test]
    fn encrypt_refuses_a_malformed_subscriber_key() {
        let auth_secret = [0u8; AUTH_SECRET_LEN];
        assert!(encrypt(&[4u8; 64], &auth_secret, b"x").is_err());
    }
}
