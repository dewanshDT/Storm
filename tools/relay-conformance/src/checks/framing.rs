//! §3 framing and the version pin, and §6's `protocol_error`.

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use super::{Check, check};
use crate::harness::{Ctx, STEP, hello, open_stream};
use crate::wire::{self, Conn};

pub fn checks() -> Vec<Check> {
    vec![
        check!("§3", the_version_is_pinned_on_the_server_trunk),
        check!("§3", the_version_is_pinned_after_registration),
        check!("§3", the_version_is_pinned_on_the_client_trunk),
        check!("§3", there_is_no_third_body_type),
        check!("§6", a_malformed_frame_gets_one_undetailed_protocol_error),
        check!("§6", a_message_from_the_wrong_party_is_a_protocol_error),
        check!(
            "§6",
            an_open_stream_without_an_attempt_id_is_a_protocol_error
        ),
    ]
}

/// §6: `protocol_error` closes the connection, and says so trunk-wide.
pub async fn refused(conn: &mut Conn) -> Result<Value> {
    let error = conn.expect_error("protocol_error", STEP).await?;
    wire::check_trunk_scoped(&error)?;
    conn.expect_closed(STEP).await?;
    Ok(error)
}

async fn the_version_is_pinned_on_the_server_trunk(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    for v in [json!(2), Value::Null, json!("1"), json!(1.5)] {
        let mut conn = ctx.dial_register(&ctx.relay).await?;
        let mut frame = json!({
            "type": "REGISTER_SERVER",
            "server_id": identity.server_id, "pubkey": identity.pubkey_b64(),
        });
        if !v.is_null() {
            frame["v"] = v.clone();
        }
        conn.send_json(frame).await?;
        refused(&mut conn)
            .await
            .map_err(|e| e.context(format!("v = {v}")))?;
    }
    Ok(())
}

async fn the_version_is_pinned_after_registration(ctx: &Ctx) -> Result<()> {
    let mut server = ctx.register(&ctx.relay, &ctx.identity()).await?;
    server.send_json(json!({ "v": 2, "type": "PING" })).await?;
    refused(&mut server).await?;
    Ok(())
}

async fn the_version_is_pinned_on_the_client_trunk(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let _server = ctx.register(&ctx.relay, &identity).await?;
    for frame in [
        json!({ "v": 2, "type": "HELLO", "server_id": identity.server_id }),
        json!({ "type": "HELLO", "server_id": identity.server_id }),
    ] {
        let mut client = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
        client.send_json(frame.clone()).await?;
        refused(&mut client)
            .await
            .map_err(|e| e.context(frame.to_string()))?;
    }
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    client
        .send_json(json!({ "v": 2, "type": "OPEN_STREAM", "attempt_id": "a" }))
        .await?;
    refused(&mut client).await?;
    Ok(())
}

async fn there_is_no_third_body_type(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.client
        .send_binary(wire::body_frame(0x03, pair.stream_id, b"third"))
        .await?;
    refused(&mut pair.client).await?;

    let mut pair = ctx.stream().await?;
    pair.server
        .send_binary(wire::body_frame(0x03, pair.stream_id, b"third"))
        .await?;
    refused(&mut pair.server).await?;
    Ok(())
}

async fn a_malformed_frame_gets_one_undetailed_protocol_error(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut messages = Vec::new();
    for raw in [
        "not json at all".to_string(),
        "[]".into(),
        "42".into(),
        r#"{"v":1}"#.into(),
        r#"{"v":1,"type":"NOT_A_REAL_TYPE"}"#.into(),
        r#"{"v":1,"type":"CHALLENGE_RESPONSE","sig":"x"}"#.into(),
        json!({ "v": 1, "type": "REGISTER_SERVER", "server_id": identity.server_id }).to_string(),
    ] {
        let mut conn = ctx.dial_register(&ctx.relay).await?;
        conn.send_text(&raw).await?;
        let error = refused(&mut conn)
            .await
            .map_err(|e| e.context(format!("register: {raw}")))?;
        messages.push((raw, error["message"].clone()));
    }

    let _server = ctx.register(&ctx.relay, &identity).await?;
    for raw in [
        "not json at all".to_string(),
        r#"{"v":1,"type":"NOT_A_REAL_TYPE"}"#.into(),
        r#"{"v":1,"type":"HELLO"}"#.into(),
        json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "first-frame" }).to_string(),
    ] {
        let mut conn = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
        conn.send_text(&raw).await?;
        let error = refused(&mut conn)
            .await
            .map_err(|e| e.context(format!("connect: {raw}")))?;
        messages.push((raw, error["message"].clone()));
    }

    // "MUST carry minimal detail": the relay says the same thing whichever
    // part of the framing was wrong.
    let first = &messages[0].1;
    for (raw, message) in &messages {
        ensure!(
            message == first,
            "protocol_error's message varies with the cause: {first} vs {message} for {raw}"
        );
    }
    Ok(())
}

async fn a_message_from_the_wrong_party_is_a_protocol_error(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let _server = ctx.register(&ctx.relay, &identity).await?;
    for frame in [
        json!({ "v": 1, "type": "STREAM_ACK", "stream_id": 1 }),
        json!({ "v": 1, "type": "HTTP_RESPONSE_HEAD", "stream_id": 1, "status": 200, "headers": {} }),
        json!({ "v": 1, "type": "REGISTER_SERVER",
                "server_id": identity.server_id, "pubkey": identity.pubkey_b64() }),
        json!({ "v": 1, "type": "CHALLENGE_RESPONSE", "sig": "AAAA" }),
        json!({ "v": 1, "type": "STREAM_OPEN", "stream_id": 1 }),
        json!({ "v": 1, "type": "ERROR", "code": "trunk_lost", "message": "x" }),
    ] {
        let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
        client.send_json(frame.clone()).await?;
        refused(&mut client)
            .await
            .map_err(|e| e.context(format!("from a client: {frame}")))?;
    }

    for frame in [
        json!({ "v": 1, "type": "HELLO", "server_id": "srv_conf_00" }),
        json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "a" }),
    ] {
        let mut server = ctx.register(&ctx.relay, &ctx.identity()).await?;
        server.send_json(frame.clone()).await?;
        refused(&mut server)
            .await
            .map_err(|e| e.context(format!("from a server: {frame}")))?;
    }
    Ok(())
}

async fn an_open_stream_without_an_attempt_id_is_a_protocol_error(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
    hello(&client, &identity.server_id).await?;
    client.recv_type("READY", STEP).await?;
    let _ = open_stream(&mut client, json!("fine")).await?;
    client
        .send_json(json!({ "v": 1, "type": "OPEN_STREAM" }))
        .await?;
    refused(&mut client).await?;
    server
        .wait_for("STREAM_OPEN", STEP, |f| f.is_type("STREAM_OPEN"))
        .await?;
    Ok(())
}
