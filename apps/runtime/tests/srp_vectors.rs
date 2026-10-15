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
use storm_runtime::identity::{
    RELAY_SET_PREFIX, challenge_message, decode_public_key, relay_set_message, verify,
};
use storm_runtime::srp::{SrpTrunk, codec, plaintext_allowed};

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
        let key = s(&doc["keys"], "test_public_key_b64");
        let err = SrpTrunk::connect("ws://127.0.0.1:9", s(case, "server_id"), key)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "invalid_relay_target", "{}", s(case, "name"));
    }
}

#[tokio::test]
async fn hello_pubkey_spellings_are_judged_as_recorded() {
    let doc = vectors();
    let section = &doc["hello_pubkey"];
    for case in cases(section, "cases") {
        let accepted = case["accepted"].as_bool().unwrap();
        assert_eq!(
            codec::validate_key(s(case, "pubkey")).is_ok(),
            accepted,
            "hello_pubkey vector `{}`",
            s(case, "name")
        );
        if !accepted {
            let err = SrpTrunk::connect("ws://127.0.0.1:9", "srv_A", s(case, "pubkey"))
                .await
                .unwrap_err();
            assert_eq!(err.code(), "invalid_relay_target", "{}", s(case, "name"));
        }
    }
    let hello: Value = serde_json::from_str(&codec::hello("srv_A", "k")).unwrap();
    assert_eq!(hello["pubkey"], "k");
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
fn every_relay_set_message_and_signature_is_judged_as_recorded() {
    let doc = vectors();
    let section = &doc["relay_set_message"];
    assert_eq!(s(section, "domain"), RELAY_SET_PREFIX);
    let list = cases(section, "cases");
    assert!(
        list.iter().any(|c| c["verifies"] == true),
        "no positive vector"
    );
    for case in list {
        let name = s(case, "name");
        let relays: Vec<&str> = case["relays"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap())
            .collect();
        let message = relay_set_message(s(case, "server_id"), &relays);
        if let Some(want) = case["message_utf8"].as_str() {
            assert_eq!(
                message,
                want.as_bytes(),
                "relay_set_message vector `{name}`"
            );
        }
        if let Some(sig) = case["signature_b64"].as_str() {
            let key = decode_public_key(s(case, "public_key_b64")).unwrap();
            assert_eq!(
                verify(&key, &message, sig),
                case["verifies"].as_bool().unwrap(),
                "relay_set_message vector `{name}`"
            );
        }
    }
}

#[test]
fn every_plaintext_relay_host_is_judged_as_recorded() {
    let doc = vectors();
    for case in cases(&doc["plaintext_relay_hosts"], "cases") {
        let host = s(case, "host");
        assert_eq!(
            plaintext_allowed(host),
            case["ws_allowed"].as_bool().unwrap(),
            "plaintext_relay_hosts vector `{host}`"
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

// ---- framing (SRP §3, §5, §6; decision 116l) -------------------------------

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn payload(case: &Value) -> Vec<u8> {
    match case["payload_utf8"].as_str() {
        Some(text) => text.as_bytes().to_vec(),
        None => hex(s(case, "payload_hex")),
    }
}

fn control(name: &str) -> Value {
    vectors()["framing"]["control_messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no vector {name}"))
        .clone()
}

#[test]
fn body_frames_match_the_framing_vectors() {
    let doc = vectors();
    let f = &doc["framing"];
    for case in f["body_frames"].as_array().unwrap() {
        let kind = case["type"].as_u64().unwrap() as u8;
        let id = case["stream_id"].as_u64().unwrap() as u32;
        let bytes = hex(s(case, "frame_hex"));
        if case["sender_may_send"] != false {
            assert_eq!(
                codec::encode_body_frame(kind, id, &payload(case)),
                bytes,
                "{case}"
            );
        }
        let (k, i, p) = codec::decode_body_frame(&bytes).expect("a vector frame decodes");
        assert_eq!((k, i, p.to_vec()), (kind, id, payload(case)), "{case}");
    }
    for case in f["too_short_frames"].as_array().unwrap() {
        assert!(
            codec::decode_body_frame(&hex(s(case, "frame_hex"))).is_none(),
            "{case}"
        );
    }
}

#[test]
fn the_control_frames_this_client_sends_match_the_vectors() {
    let json = |s: String| serde_json::from_str::<Value>(&s).unwrap();
    // The Worker answers exactly this PING without waking (SG-38).
    assert_eq!(codec::ping(), s(&control("canonical_ping"), "exact_text"));
    assert_eq!(codec::pong(), s(&control("canonical_pong"), "exact_text"));
    let hello = control("hello_with_pubkey")["json"].clone();
    assert_eq!(
        json(codec::hello(
            hello["server_id"].as_str().unwrap(),
            hello["pubkey"].as_str().unwrap()
        )),
        hello
    );
    assert_eq!(
        json(codec::close_stream(7)),
        control("close_stream")["json"]
    );
    assert_eq!(json(codec::close_trunk()), control("close_trunk")["json"]);
}

#[test]
fn the_attempt_ids_this_client_mints_are_valid_ones() {
    // §5.1: a client sends 1–64 ASCII characters. The runtime mints `a<n>`.
    for case in vectors()["framing"]["attempt_id"].as_array().unwrap() {
        let valid = case["attempt_id"]
            .as_str()
            .is_some_and(|a| (1..=64).contains(&a.len()) && a.is_ascii());
        assert_eq!(valid, case["valid"] == true, "{case}");
    }
    for n in [0u64, 1, u64::MAX] {
        let minted = format!("a{n}");
        assert!((1..=64).contains(&minted.len()) && minted.is_ascii());
    }
}
