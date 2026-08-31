//! `docs/srp-vectors.json`, checked against this crate's copy of the signed
//! bytes.
//!
//! `auth.rs` says it plainly: its functions are re-derived from
//! `apps/server/src/auth/identity.rs`, the two crates cannot depend on one
//! another, and nothing in either build makes them agree. Drift surfaces as
//! `auth_failed` at runtime — indistinguishable from an attack, an expired
//! nonce or a refused binding.
//!
//! This file is the enforcement. The same vectors are read by
//! `apps/server/src/auth/vectors_test.rs` and
//! `apps/client/test/srp_vectors_test.dart`, so a change to one
//! implementation's bytes fails a test in all three.
//!
//! The signatures come from an independent RFC 8032 implementation
//! (`tools/srp-vectors/`), so a valid-signature vector verifying under
//! `ed25519-dalek` here is a genuine cross-check rather than dalek agreeing
//! with itself.

use std::path::PathBuf;

use data_encoding::BASE64URL_NOPAD;
use serde_json::Value;
use storm_relay::auth::{PublicKey, relay_auth_message, validate_nonce, validate_server_id};

/// The vectors live at the repo root, not in the crate — three implementations
/// read the one file, and a copy per crate is exactly the drift being guarded
/// against.
fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/srp-vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("srp-vectors.json is not valid JSON")
}

fn cases<'a>(doc: &'a Value, section: &str) -> &'a Vec<Value> {
    doc[section]
        .as_array()
        .unwrap_or_else(|| panic!("section `{section}` missing from srp-vectors.json"))
}

fn s<'a>(case: &'a Value, field: &str) -> &'a str {
    case[field]
        .as_str()
        .unwrap_or_else(|| panic!("field `{field}` missing from vector {case}"))
}

#[test]
fn the_file_is_the_version_this_test_understands() {
    // A v2 file read by a v1 test would pass vacuously on the sections it
    // happens to still recognise.
    assert_eq!(vectors()["version"].as_u64(), Some(1));
}

#[test]
fn the_domain_prefixes_are_the_ones_this_crate_uses() {
    let doc = vectors();
    let relay = s(&doc["domains"], "relay_auth");
    let challenge = s(&doc["domains"], "client_challenge");
    assert_ne!(relay, challenge, "domain separation is the whole point");
    assert!(relay_auth_message("s", "n").starts_with(relay.as_bytes()));
    assert!(!relay_auth_message("s", "n").starts_with(challenge.as_bytes()));
}

#[test]
fn every_relay_auth_message_vector_matches_byte_for_byte() {
    let doc = vectors();
    let list = cases(&doc, "relay_auth_message");
    assert!(
        !list.is_empty(),
        "no vectors — the file would pass vacuously"
    );
    for case in list {
        let name = s(case, "name");
        let built = relay_auth_message(s(case, "server_id"), s(case, "nonce"));
        assert_eq!(
            built,
            s(case, "message_utf8").as_bytes(),
            "relay_auth_message vector `{name}` disagrees with this crate"
        );
        assert_eq!(
            BASE64URL_NOPAD.encode(&built),
            s(case, "message_b64"),
            "relay_auth_message vector `{name}`: base64url form disagrees"
        );
    }
}

#[test]
fn the_documented_delimiter_collision_still_exists() {
    // Not a curiosity: it is the reason both fields are validated. If a future
    // framing change removed the collision, this section of the file would be
    // describing a hazard that no longer exists and should be rewritten
    // deliberately rather than silently outlived.
    let doc = vectors();
    let pairs = doc["delimiter_confusion"]["colliding_pairs"]
        .as_array()
        .expect("delimiter_confusion.colliding_pairs missing");
    assert!(!pairs.is_empty());
    for pair in pairs {
        let (a, b) = (&pair["a"], &pair["b"]);
        let ma = relay_auth_message(s(a, "server_id"), s(a, "nonce"));
        let mb = relay_auth_message(s(b, "server_id"), s(b, "nonce"));
        assert_eq!(ma, mb, "the pair no longer collides");
        assert_eq!(ma, s(pair, "shared_message_utf8").as_bytes());

        // And the reason it is harmless: validation refuses both readings.
        let a_ok =
            validate_server_id(s(a, "server_id")).is_ok() && validate_nonce(s(a, "nonce")).is_ok();
        let b_ok =
            validate_server_id(s(b, "server_id")).is_ok() && validate_nonce(s(b, "nonce")).is_ok();
        assert!(
            !a_ok && !b_ok,
            "a colliding pair passed validation — one signature would cover \
             two different (server_id, nonce) readings"
        );
    }
}

#[test]
fn every_nonce_vector_gets_the_verdict_the_file_records() {
    let doc = vectors();
    for case in cases(&doc, "validate_nonce") {
        let name = s(case, "name");
        let expected = case["valid"].as_bool().expect("`valid` must be a bool");
        assert_eq!(
            validate_nonce(s(case, "nonce")).is_ok(),
            expected,
            "validate_nonce vector `{name}`: expected valid={expected}"
        );
    }
}

#[test]
fn every_server_id_vector_gets_the_verdict_the_file_records() {
    let doc = vectors();
    for case in cases(&doc, "validate_server_id") {
        let name = s(case, "name");
        let expected = case["valid"].as_bool().expect("`valid` must be a bool");
        assert_eq!(
            validate_server_id(s(case, "server_id")).is_ok(),
            expected,
            "validate_server_id vector `{name}`: expected valid={expected}"
        );
    }
}

#[test]
fn every_signature_vector_verifies_exactly_as_recorded() {
    let doc = vectors();
    let list = cases(&doc, "verify_relay_auth");
    assert!(
        list.iter().any(|c| c["verifies"].as_bool() == Some(true)),
        "no positive vector — a verifier stuck at `false` would pass"
    );
    for case in list {
        let name = s(case, "name");
        let expected = case["verifies"]
            .as_bool()
            .expect("`verifies` must be a bool");
        // A key that will not parse cannot verify anything, which is the same
        // observable outcome the vector records.
        let actual = PublicKey::from_b64(s(case, "public_key_b64")).is_ok_and(|key| {
            key.verify_relay_auth(
                s(case, "server_id"),
                s(case, "nonce"),
                s(case, "signature_b64"),
            )
        });
        assert_eq!(
            actual, expected,
            "verify_relay_auth vector `{name}`: expected verifies={expected}"
        );
    }
}

#[test]
fn every_public_key_parses_exactly_as_recorded() {
    let doc = vectors();
    let list = doc["public_key_parsing"]["cases"]
        .as_array()
        .expect("public_key_parsing.cases missing");
    for case in list {
        let name = s(case, "name");
        let expected = case["accepted"]
            .as_bool()
            .expect("`accepted` must be a bool");
        assert_eq!(
            PublicKey::from_b64(s(case, "public_key_b64")).is_ok(),
            expected,
            "public_key_parsing vector `{name}`: expected accepted={expected}"
        );
    }
}

#[test]
fn a_parsed_key_round_trips_to_the_unpadded_spelling() {
    // §4.1's bindings are compared on decoded bytes, and `to_b64` is what an
    // operator greps an allowlist for — so it must emit the one spelling the
    // wire uses, whatever spelling it was given.
    let doc = vectors();
    let canonical = s(&doc["keys"], "test_public_key_b64");
    let key = PublicKey::from_b64(canonical).expect("the file's own test key must parse");
    assert_eq!(key.to_b64(), canonical);
}
