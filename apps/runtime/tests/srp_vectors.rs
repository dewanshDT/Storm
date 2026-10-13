//! `docs/srp-vectors.json`, checked against this crate's copy of SRP v1.
//!
//! The fourth reader of the file, beside `apps/server`, `apps/relay` and
//! `apps/client`: none may depend on another, so drift fails here rather than
//! surfacing as a refused `HELLO` or a failed challenge over the relay — which
//! is also what an impostor looks like. A client never signs relay-auth, so
//! what it must agree on is the `server_id` rule it dials with and the client
//! challenge it verifies whatever answers through a relay against.

use std::path::PathBuf;

use serde_json::Value;
use storm_runtime::identity::{challenge_message, decode_public_key, verify};
use storm_runtime::srp::{SrpTrunk, codec};

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/srp-vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("srp-vectors.json is not valid JSON")
}

fn cases<'a>(doc: &'a Value, section: &str) -> &'a Vec<Value> {
    let list = doc[section]
        .as_array()
        .unwrap_or_else(|| panic!("section `{section}` missing from srp-vectors.json"));
    assert!(
        !list.is_empty(),
        "`{section}` is empty — this would pass vacuously"
    );
    list
}

fn s<'a>(case: &'a Value, field: &str) -> &'a str {
    case[field]
        .as_str()
        .unwrap_or_else(|| panic!("field `{field}` missing from vector {case}"))
}

#[test]
fn the_file_is_the_version_this_test_understands() {
    assert_eq!(vectors()["version"].as_u64(), Some(1));
    assert_eq!(codec::PROTOCOL_VERSION, 1);
}

#[test]
fn the_client_challenge_domain_is_the_one_this_crate_builds() {
    let doc = vectors();
    let challenge = s(&doc["domains"], "client_challenge");
    let relay = s(&doc["domains"], "relay_auth");
    assert!(challenge_message("s", "n").starts_with(challenge.as_bytes()));
    assert!(!challenge_message("s", "n").starts_with(relay.as_bytes()));
}

#[test]
fn every_server_id_vector_is_judged_as_recorded() {
    let doc = vectors();
    for case in cases(&doc, "validate_server_id") {
        let id = s(case, "server_id");
        let valid = case["valid"].as_bool().unwrap();
        assert_eq!(
            codec::validate_server_id(id).is_ok(),
            valid,
            "validate_server_id vector `{}`",
            s(case, "name")
        );
    }
}

#[tokio::test]
async fn a_trunk_never_dials_a_server_id_the_vectors_refuse() {
    let doc = vectors();
    for case in cases(&doc, "validate_server_id") {
        if case["valid"].as_bool().unwrap() {
            continue;
        }
        // Port 9 on loopback: nothing should ever be dialled to find out.
        let err = SrpTrunk::connect("ws://127.0.0.1:9", s(case, "server_id"))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "invalid_relay_target", "{}", s(case, "name"));
    }
}

#[test]
fn every_client_challenge_message_matches_byte_for_byte() {
    let doc = vectors();
    for case in cases(&doc, "client_challenge_message") {
        assert_eq!(
            challenge_message(s(case, "server_id"), s(case, "nonce")),
            s(case, "message_utf8").as_bytes(),
            "client_challenge_message vector `{}`",
            s(case, "name")
        );
    }
}

#[test]
fn every_client_challenge_signature_verifies_as_recorded() {
    let doc = vectors();
    let list = cases(&doc, "verify_client_challenge");
    assert!(
        list.iter().any(|c| c["verifies"] == true),
        "no positive vector"
    );
    for case in list {
        let actual = decode_public_key(s(case, "public_key_b64")).is_ok_and(|key| {
            verify(
                &key,
                &challenge_message(s(case, "server_id"), s(case, "nonce")),
                s(case, "signature_b64"),
            )
        });
        assert_eq!(
            actual,
            case["verifies"].as_bool().unwrap(),
            "verify_client_challenge vector `{}`",
            s(case, "name")
        );
    }
}

#[test]
fn every_public_key_parses_as_recorded() {
    let doc = vectors();
    for case in cases(&doc["public_key_parsing"], "cases") {
        assert_eq!(
            decode_public_key(s(case, "public_key_b64")).is_ok(),
            case["accepted"].as_bool().unwrap(),
            "public_key_parsing vector `{}`",
            s(case, "name")
        );
    }
}
