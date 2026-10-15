//! `SrpTrunk` through a real `storm-relay` to a real storm-server. Ignored by
//! default: it needs both running, with the server registered at the relay.
//!
//! ```sh
//! STORM_SRP_RELAY=ws://127.0.0.1:18860 STORM_SRP_SERVER_ID=srv_… \
//! STORM_SRP_SERVER_KEY=<public_key from /v1/server> \
//!   cargo test --test srp_live -- --ignored
//! ```

use bytes::Bytes;
use serde_json::{Value, json};
use storm_runtime::identity::{challenge_message, decode_public_key, verify};
use storm_runtime::srp::SrpTrunk;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

#[tokio::test]
#[ignore = "needs a live storm-relay and storm-server"]
async fn a_real_relay_carries_the_identity_challenge() {
    let server_id = env("STORM_SRP_SERVER_ID");
    let key_b64 = env("STORM_SRP_SERVER_KEY");
    let key = decode_public_key(&key_b64).unwrap();
    let trunk = SrpTrunk::connect(&env("STORM_SRP_RELAY"), &server_id, &key_b64)
        .await
        .unwrap();

    let info = trunk
        .request("GET", "/v1/server", &[], Bytes::new())
        .await
        .unwrap();
    assert_eq!(info.status, 200);
    let info: Value = serde_json::from_slice(&info.body.collect().await.unwrap()).unwrap();
    assert_eq!(info["server_id"], server_id.as_str());

    // A POST through the relay: it is answered only if its end of body arrives.
    let nonce = "srp-live-0123456789abcdef";
    let answer = trunk
        .request(
            "POST",
            "/v1/server/challenge",
            &[("content-type".into(), "application/json".into())],
            Bytes::from(json!({ "nonce": nonce }).to_string()),
        )
        .await
        .unwrap();
    assert_eq!(answer.status, 200);
    let answer: Value = serde_json::from_slice(&answer.body.collect().await.unwrap()).unwrap();
    assert!(verify(
        &key,
        &challenge_message(&server_id, nonce),
        answer["signature"].as_str().unwrap()
    ));

    let concurrent = futures_util::future::join_all((0..16).map(|_| {
        let trunk = trunk.clone();
        async move {
            let resp = trunk
                .request("GET", "/v1/health", &[], Bytes::new())
                .await
                .unwrap();
            (resp.status, resp.body.collect().await.unwrap())
        }
    }))
    .await;
    assert!(concurrent.iter().all(|(status, _)| *status == 200));

    let unknown = SrpTrunk::connect(&env("STORM_SRP_RELAY"), "srv_not_registered_here", &key_b64)
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), "server_offline");
}
