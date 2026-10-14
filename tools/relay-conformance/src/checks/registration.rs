//! §4 server registration: the challenge, the signed bytes, the field rules,
//! the strict base64url of §4.0, and the §4.1 bindings.

use std::collections::HashSet;

use anyhow::{Context, Result, bail, ensure};
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

use super::{Check, Needs, check};
use crate::fixtures::Identity;
use crate::harness::{Ctx, QUIET, Relay, STEP, fresh_identity};
use crate::wire::{self, Conn, Frame};

const VECTORS: &str = include_str!("../../../../docs/srp-vectors.json");

pub fn checks() -> Vec<Check> {
    vec![
        check!("§4", a_correct_handshake_registers),
        check!("§4", the_signature_covers_the_domain_and_both_fields),
        check!(
            "§4",
            every_challenge_is_fresh_and_a_signature_answers_only_its_own
        ),
        check!("§4", an_expired_nonce_is_refused),
        check!("§4", an_invalid_server_id_is_refused_before_any_challenge),
        check!("§4.0", a_key_or_signature_in_another_spelling_is_refused),
        check!("§4.1", the_allowlist_refuses_before_spending_a_nonce),
        check!("§6", every_auth_failure_is_the_same_auth_failed),
        check!("§4.1", tofu_binds_the_first_key_for_good, Needs::Tofu),
        check!(
            "§4.1",
            tofu_records_only_after_a_good_signature,
            Needs::Tofu
        ),
        check!(
            "§4.1",
            tofu_compares_keys_as_decoded_bytes_of_one_spelling,
            Needs::Tofu
        ),
    ]
}

/// The §4 nonce rule: 16–128 printable ASCII, neither `:` nor `"`.
fn check_nonce(nonce: &str) -> Result<()> {
    ensure!(
        (16..=128).contains(&nonce.len())
            && nonce
                .bytes()
                .all(|b| (0x21..=0x7e).contains(&b) && b != b':' && b != b'"'),
        "the relay issued a nonce §4 forbids: {nonce:?}"
    );
    Ok(())
}

/// Sends `REGISTER_SERVER` and requires a refusal **instead of** a challenge:
/// the relay must not spend a nonce on a registration that cannot succeed.
async fn refused_before_challenge(
    ctx: &Ctx,
    relay: &Relay,
    server_id: &str,
    pubkey: &str,
    codes: &[&str],
) -> Result<Value> {
    let mut conn = ctx.dial_register(relay).await?;
    conn.send_json(json!({
        "v": 1, "type": "REGISTER_SERVER", "server_id": server_id, "pubkey": pubkey,
    }))
    .await?;
    let reply = conn
        .recv(STEP)
        .await
        .with_context(|| format!("REGISTER_SERVER {server_id:?} {pubkey:?}"))?;
    ensure!(
        reply["type"] != "CHALLENGE",
        "REGISTER_SERVER {server_id:?} {pubkey:?} was challenged; it must be refused first"
    );
    ensure!(
        reply["type"] == "ERROR" && codes.iter().any(|c| reply["code"] == *c),
        "REGISTER_SERVER {server_id:?} {pubkey:?}: expected ERROR {codes:?}, got {reply}"
    );
    Ok(reply)
}

/// After a refused `CHALLENGE_RESPONSE`: an `ERROR` with one of `codes`, and
/// never `REGISTERED`.
async fn refused_after_challenge(conn: &mut Conn, codes: &[&str]) -> Result<Value> {
    let reply = conn.recv(STEP).await?;
    ensure!(
        reply["type"] == "ERROR" && codes.iter().any(|c| reply["code"] == *c),
        "expected ERROR {codes:?}, got {reply}"
    );
    match conn.next(QUIET).await {
        Ok(Frame::Text(value)) if value["type"] == "REGISTERED" => {
            bail!("REGISTERED after {reply}")
        }
        _ => Ok(reply),
    }
}

async fn a_correct_handshake_registers(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut conn = ctx.dial_register(&ctx.relay).await?;
    let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
    check_nonce(&nonce)?;
    Ctx::respond(&mut conn, &identity.relay_auth(&nonce)).await?;
    let registered = conn.recv_type("REGISTERED", STEP).await?;

    ensure!(
        registered["trunk_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "REGISTERED needs a trunk_id: {registered}"
    );
    // §4.2 (116e): a third of the relay's deadline, whole seconds, at least 1;
    // 15 s at the default 45 s.
    let deadline = ctx.knobs.heartbeat_deadline.as_secs();
    ensure!(
        registered["heartbeat_interval_secs"] == (deadline / 3).max(1),
        "§4.2 derives the heartbeat from the {deadline} s deadline: {registered}"
    );
    // §4.3: derived, `<scheme>://<relay-host>/connect/<server_id>`.
    let address = registered["public_address"].as_str().unwrap_or_default();
    ensure!(
        (address.starts_with("ws://") || address.starts_with("wss://"))
            && address.ends_with(&format!("/connect/{}", identity.server_id)),
        "public_address must be ws(s)://<host>/connect/<server_id>: {registered}"
    );
    Ok(())
}

async fn the_signature_covers_the_domain_and_both_fields(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let other = ctx.identity();
    let intruder = ctx.intruder(&identity.server_id);
    // (what, who signs, the bytes before the nonce)
    let cases = [
        (
            "the client-facing domain storm-challenge:v1:",
            &identity,
            format!("storm-challenge:v1:{}:", identity.server_id),
        ),
        (
            "the prefix and nonce without the server_id",
            &identity,
            "storm-relay-auth:v1:".to_string(),
        ),
        (
            "another server_id",
            &identity,
            format!("storm-relay-auth:v1:{}:", other.server_id),
        ),
        (
            "the right message by the wrong key",
            &intruder,
            format!("storm-relay-auth:v1:{}:", identity.server_id),
        ),
    ];
    for (what, signer, prefix) in cases {
        let mut conn = ctx.dial_register(&ctx.relay).await?;
        let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
        Ctx::respond(&mut conn, &signer.sign(&format!("{prefix}{nonce}"))).await?;
        refused_after_challenge(&mut conn, &["auth_failed"])
            .await
            .with_context(|| format!("a signature over {what}"))?;
    }
    // The real one still works, so the refusals above were about the bytes.
    ctx.register(&ctx.relay, &identity).await?;
    Ok(())
}

async fn every_challenge_is_fresh_and_a_signature_answers_only_its_own(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut nonces = HashSet::new();
    let mut held = Vec::new();
    for _ in 0..5 {
        let mut conn = ctx.dial_register(&ctx.relay).await?;
        let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
        ensure!(
            nonces.insert(nonce.clone()),
            "the nonce {nonce} was issued twice"
        );
        held.push((conn, nonce));
    }

    // A captured CHALLENGE_RESPONSE, replayed onto another connection.
    let (mut first, first_nonce) = held.remove(0);
    let captured = identity.relay_auth(&first_nonce);
    Ctx::respond(&mut first, &captured).await?;
    first.recv_type("REGISTERED", STEP).await?;
    let (mut second, _) = held.remove(0);
    Ctx::respond(&mut second, &captured).await?;
    refused_after_challenge(&mut second, &["auth_failed"])
        .await
        .context("a signature replayed from another connection's challenge")?;
    Ok(())
}

async fn an_expired_nonce_is_refused(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut conn = ctx.dial_register(&ctx.relay).await?;
    let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
    tokio::time::sleep(ctx.knobs.challenge_ttl + std::time::Duration::from_millis(500)).await;
    // A relay may instead have closed the handshake at the TTL; either way it
    // must not register.
    if Ctx::respond(&mut conn, &identity.relay_auth(&nonce))
        .await
        .is_err()
    {
        return Ok(());
    }
    match conn.next(STEP).await? {
        Frame::Text(reply) => wire::check_error(&reply, "auth_failed"),
        Frame::Closed => Ok(()),
        other => bail!(
            "a stale nonce: expected auth_failed, got {}",
            other.describe()
        ),
    }
}

async fn an_invalid_server_id_is_refused_before_any_challenge(ctx: &Ctx) -> Result<()> {
    let vectors: Value = serde_json::from_str(VECTORS)?;
    let pubkey = ctx.identity().pubkey_b64();
    let mut tried = 0;
    for case in vectors["validate_server_id"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if case["valid"] == false {
            let server_id = case["server_id"].as_str().unwrap_or_default();
            refused_before_challenge(
                ctx,
                &ctx.relay,
                server_id,
                &pubkey,
                &["protocol_error", "auth_failed"],
            )
            .await?;
            tried += 1;
        }
    }
    ensure!(tried > 0, "docs/srp-vectors.json has no invalid server_ids");
    Ok(())
}

fn to_standard_alphabet(b64url: &str) -> String {
    b64url.replace('-', "+").replace('_', "/")
}

async fn a_key_or_signature_in_another_spelling_is_refused(ctx: &Ctx) -> Result<()> {
    // The listed key, padded: the same bytes, a spelling §4.0 forbids. A
    // lenient decoder would find it on the allowlist and challenge it.
    let identity = ctx.identity();
    refused_before_challenge(
        ctx,
        &ctx.relay,
        &identity.server_id,
        &format!("{}=", identity.pubkey_b64()),
        &["protocol_error", "auth_failed"],
    )
    .await
    .context("a padded pubkey")?;

    let with_url_chars = ctx
        .fixtures
        .servers
        .iter()
        .find(|s| s.pubkey_b64().contains(['-', '_']))
        .context("no fixture key has a - or _ to respell")?;
    refused_before_challenge(
        ctx,
        &ctx.relay,
        &with_url_chars.server_id,
        &to_standard_alphabet(&with_url_chars.pubkey_b64()),
        &["protocol_error", "auth_failed"],
    )
    .await
    .context("a pubkey in the standard base64 alphabet")?;

    let mut conn = ctx.dial_register(&ctx.relay).await?;
    let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
    Ctx::respond(&mut conn, &format!("{}==", identity.relay_auth(&nonce))).await?;
    refused_after_challenge(&mut conn, &["auth_failed", "protocol_error"])
        .await
        .context("a padded signature")?;

    // A signature has a - or _ most of the time; try until one does.
    for _ in 0..20 {
        let mut conn = ctx.dial_register(&ctx.relay).await?;
        let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
        let sig = identity.relay_auth(&nonce);
        if !sig.contains(['-', '_']) {
            continue;
        }
        Ctx::respond(&mut conn, &to_standard_alphabet(&sig)).await?;
        refused_after_challenge(&mut conn, &["auth_failed", "protocol_error"])
            .await
            .context("a signature in the standard base64 alphabet")?;
        return Ok(());
    }
    bail!("twenty signatures without a - or _; the odds of that are nil")
}

async fn the_allowlist_refuses_before_spending_a_nonce(ctx: &Ctx) -> Result<()> {
    let listed = ctx.identity();
    let intruder = ctx.intruder(&listed.server_id);
    refused_before_challenge(
        ctx,
        &ctx.relay,
        &listed.server_id,
        &intruder.pubkey_b64(),
        &["auth_failed"],
    )
    .await
    .context("a listed server_id with another key")?;

    let unlisted = crate::fixtures::UNLISTED_SERVER_ID;
    refused_before_challenge(
        ctx,
        &ctx.relay,
        unlisted,
        &intruder.pubkey_b64(),
        &["auth_failed"],
    )
    .await
    .context("an unlisted server_id, on first sight")?;
    refused_before_challenge(
        ctx,
        &ctx.relay,
        unlisted,
        &listed.pubkey_b64(),
        &["auth_failed"],
    )
    .await
    .context("an unlisted server_id with a listed key")?;

    // And the binding still holds for its owner.
    ctx.register(&ctx.relay, &listed).await?;
    Ok(())
}

async fn every_auth_failure_is_the_same_auth_failed(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let intruder = ctx.intruder(&identity.server_id);
    let mut seen: Vec<(&str, Value)> = Vec::new();

    let mut conn = ctx.dial_register(&ctx.relay).await?;
    let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
    Ctx::respond(&mut conn, &intruder.relay_auth(&nonce)).await?;
    seen.push((
        "a bad signature",
        conn.expect_error("auth_failed", STEP).await?,
    ));

    seen.push((
        "a refused binding",
        refused_before_challenge(
            ctx,
            &ctx.relay,
            &identity.server_id,
            &intruder.pubkey_b64(),
            &["auth_failed"],
        )
        .await?,
    ));
    seen.push((
        "an unlisted server_id",
        refused_before_challenge(
            ctx,
            &ctx.relay,
            crate::fixtures::UNLISTED_SERVER_ID,
            &intruder.pubkey_b64(),
            &["auth_failed"],
        )
        .await?,
    ));

    let mut conn = ctx.dial_register(&ctx.relay).await?;
    let nonce = Ctx::challenge(&mut conn, &identity.server_id, &identity.pubkey_b64()).await?;
    tokio::time::sleep(ctx.knobs.challenge_ttl + std::time::Duration::from_millis(500)).await;
    if Ctx::respond(&mut conn, &identity.relay_auth(&nonce))
        .await
        .is_ok()
        && let Ok(Frame::Text(reply)) = conn.next(STEP).await
    {
        seen.push(("an expired nonce", reply));
    }

    // §6: auth_failed MUST NOT say which check failed.
    let (first_cause, first) = &seen[0];
    for (cause, reply) in &seen {
        ensure!(
            reply == first,
            "auth_failed differs by cause: {first_cause} gave {first}, {cause} gave {reply}"
        );
    }
    Ok(())
}

async fn tofu_binds_the_first_key_for_good(ctx: &Ctx) -> Result<()> {
    let relay = ctx.tofu()?;
    let first = fresh_identity();
    let second = Identity {
        server_id: first.server_id.clone(),
        key: fresh_identity().key,
    };
    let (mut trunk, _) = ctx.register_full(relay, &first).await?;
    trunk.kill();

    refused_before_challenge(
        ctx,
        relay,
        &second.server_id,
        &second.pubkey_b64(),
        &["auth_failed"],
    )
    .await
    .context("a second key for a bound server_id")?;
    ctx.register(relay, &first)
        .await
        .context("the bound key again")?;

    // The binding is per server_id: the second key is free to bind another.
    ctx.register(relay, &second.with_id(&fresh_identity().server_id))
        .await?;
    Ok(())
}

async fn tofu_records_only_after_a_good_signature(ctx: &Ctx) -> Result<()> {
    let relay = ctx.tofu()?;
    let squatter = fresh_identity();
    let owner = Identity {
        server_id: squatter.server_id.clone(),
        key: fresh_identity().key,
    };

    // A first sight that fails its signature binds nothing.
    let mut conn = ctx.dial_register(relay).await?;
    let nonce = Ctx::challenge(&mut conn, &squatter.server_id, &squatter.pubkey_b64()).await?;
    Ctx::respond(&mut conn, &owner.relay_auth(&nonce)).await?;
    refused_after_challenge(&mut conn, &["auth_failed"]).await?;

    ctx.register(relay, &owner)
        .await
        .context("the first key to sign correctly must bind")?;
    refused_before_challenge(
        ctx,
        relay,
        &squatter.server_id,
        &squatter.pubkey_b64(),
        &["auth_failed"],
    )
    .await
    .context("the key whose signature failed, after the owner bound")?;
    Ok(())
}

async fn tofu_compares_keys_as_decoded_bytes_of_one_spelling(ctx: &Ctx) -> Result<()> {
    let relay = ctx.tofu()?;
    let vectors: Value = serde_json::from_str(VECTORS)?;
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    let identity = Identity {
        server_id: fresh_identity().server_id,
        key: SigningKey::from_bytes(&seed),
    };
    ensure!(
        vectors["keys"]["test_public_key_b64"] == identity.pubkey_b64(),
        "docs/srp-vectors.json's test key is no longer seed 00..1f"
    );
    ctx.register(relay, &identity).await?;

    for case in vectors["public_key_parsing"]["cases"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let spelling = case["public_key_b64"].as_str().unwrap_or_default();
        if case["accepted"] == true {
            ensure!(BASE64URL_NOPAD.decode(spelling.as_bytes()).is_ok());
            continue;
        }
        refused_before_challenge(
            ctx,
            relay,
            &identity.server_id,
            spelling,
            &["protocol_error", "auth_failed"],
        )
        .await
        .with_context(|| format!("the bound key spelled {}", case["name"]))?;
    }
    ctx.register(relay, &identity).await?;
    Ok(())
}
