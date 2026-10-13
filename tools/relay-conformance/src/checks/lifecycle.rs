//! §4.2 trunk lifecycle — heartbeat, `DEREGISTER`, supersession, trunk loss —
//! and §5's `HELLO`: what a client sees when the server is offline, and when
//! it comes back.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::json;
use tokio::time::Instant;

use super::{Check, check};
use crate::harness::{
    Ctx, QUIET, STEP, accept_stream, close, hello, open_stream, request_head, response_head,
};
use crate::wire::{self, BODY_RESPONSE, Frame};

pub fn checks() -> Vec<Check> {
    vec![
        check!("§5", hello_gets_ready_and_no_challenge),
        check!("§5", hello_with_no_live_trunk_is_server_unreachable),
        check!("§5", hello_waits_for_a_server_mid_restart),
        check!("§4.2", a_server_ping_is_answered),
        check!("§4.2", a_heartbeating_server_outlives_the_deadline),
        check!("§4.2", a_silent_server_is_dead_at_the_deadline),
        check!("§4.2", deregister_frees_the_server_id_at_once),
        check!("§4.2", a_superseded_trunk_drains_then_ends_trunk_superseded),
        check!("§4.2", an_unfinished_registration_supersedes_nothing),
        check!(
            "§4.2",
            a_server_lost_mid_body_is_trunk_lost_never_a_clean_end
        ),
        check!("§6", every_open_stream_is_told_trunk_lost),
        check!("§6", a_server_gone_is_unreachable_until_it_registers_again),
    ]
}

async fn hello_gets_ready_and_no_challenge(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let _server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
    hello(&client, &identity.server_id).await?;
    // R12: the first thing back is READY. There is nothing to answer.
    let ready = client.recv_type("READY", STEP).await?;
    ensure!(
        ready["client_trunk_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "READY needs a client_trunk_id: {ready}"
    );
    Ok(())
}

async fn hello_with_no_live_trunk_is_server_unreachable(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut client = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
    let started = Instant::now();
    hello(&client, &identity.server_id).await?;
    let error = client
        .expect_error("server_unreachable", ctx.knobs.hello_wait + STEP)
        .await?;
    wire::check_trunk_scoped(&error)?;
    ensure!(
        started.elapsed() >= ctx.knobs.hello_wait.mul_f64(0.8),
        "server_unreachable after {:?}; §5 holds a HELLO for the wait ({:?}) first",
        started.elapsed(),
        ctx.knobs.hello_wait
    );
    Ok(())
}

async fn hello_waits_for_a_server_mid_restart(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut client = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
    hello(&client, &identity.server_id).await?;
    tokio::time::sleep(ctx.knobs.hello_wait.mul_f64(0.3)).await;
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    client
        .recv_type("READY", STEP)
        .await
        .context("a server that registered during the HELLO wait")?;
    let stream_id = open_stream(&mut client, json!("after-restart")).await?;
    ensure!(accept_stream(&mut server).await? == stream_id);
    Ok(())
}

async fn a_server_ping_is_answered(ctx: &Ctx) -> Result<()> {
    let mut server = ctx.register(&ctx.relay, &ctx.identity()).await?;
    server.go_silent();
    server.show_heartbeats(true);
    for _ in 0..3 {
        server.send_json(json!({ "v": 1, "type": "PING" })).await?;
        server.wait_for("PONG", STEP, |f| f.is_type("PONG")).await?;
    }
    Ok(())
}

async fn a_heartbeating_server_outlives_the_deadline(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    tokio::time::sleep(ctx.knobs.heartbeat_deadline.mul_f64(2.5)).await;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let stream_id = open_stream(&mut client, json!("still-alive")).await?;
    ensure!(accept_stream(&mut server).await? == stream_id);
    Ok(())
}

async fn a_silent_server_is_dead_at_the_deadline(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.server.go_silent();
    let silent_since = Instant::now();

    let error = pair
        .client
        .wait_for(
            "trunk_lost",
            ctx.knobs.heartbeat_deadline + STEP,
            |f| matches!(f, Frame::Text(v) if v["type"] == "ERROR"),
        )
        .await?;
    let Frame::Text(error) = error else {
        unreachable!()
    };
    wire::check_error(&error, "trunk_lost")?;
    wire::check_on_stream(&error, pair.stream_id)?;
    ensure!(
        silent_since.elapsed() >= ctx.knobs.heartbeat_deadline / 2,
        "trunk_lost after {:?} of silence; the deadline is {:?}",
        silent_since.elapsed(),
        ctx.knobs.heartbeat_deadline
    );

    // A dead trunk gets no new HELLOs.
    let mut late = ctx
        .dial_connect(&ctx.relay, &pair.identity.server_id)
        .await?;
    hello(&late, &pair.identity.server_id).await?;
    late.expect_error("server_unreachable", ctx.knobs.hello_wait + STEP)
        .await?;
    Ok(())
}

async fn deregister_frees_the_server_id_at_once(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let started = Instant::now();
    pair.server
        .send_json(json!({ "v": 1, "type": "DEREGISTER" }))
        .await?;

    let error = pair
        .client
        .expect_error("trunk_lost", ctx.knobs.heartbeat_deadline)
        .await
        .context("an open stream when its server deregisters")?;
    wire::check_on_stream(&error, pair.stream_id)?;
    ensure!(
        started.elapsed() < ctx.knobs.heartbeat_deadline / 2,
        "trunk_lost took {:?}: DEREGISTER frees the id at once, not at the heartbeat deadline",
        started.elapsed()
    );

    let mut late = ctx
        .dial_connect(&ctx.relay, &pair.identity.server_id)
        .await?;
    hello(&late, &pair.identity.server_id).await?;
    late.expect_error("server_unreachable", ctx.knobs.hello_wait + STEP)
        .await?;

    // Freed, so the restart a DEREGISTER announces registers cleanly.
    let mut server = ctx.register(&ctx.relay, &pair.identity).await?;
    let mut client = ctx.client(&ctx.relay, &pair.identity.server_id).await?;
    let stream_id = open_stream(&mut client, json!("restarted")).await?;
    ensure!(accept_stream(&mut server).await? == stream_id);
    Ok(())
}

async fn a_superseded_trunk_drains_then_ends_trunk_superseded(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut old = ctx.register(&ctx.relay, &identity).await?;
    let mut alice = ctx.client(&ctx.relay, &identity.server_id).await?;
    let answered = open_stream(&mut alice, json!("answered")).await?;
    ensure!(accept_stream(&mut old).await? == answered);
    let abandoned = open_stream(&mut alice, json!("abandoned")).await?;
    ensure!(accept_stream(&mut old).await? == abandoned);

    // The same server reconnects: same id, same key, challenge completed.
    let mut new = ctx.register(&ctx.relay, &identity).await?;
    let superseded_at = Instant::now();

    // New clients go to the new trunk.
    let mut bob = ctx.client(&ctx.relay, &identity.server_id).await?;
    let bobs = open_stream(&mut bob, json!("bob")).await?;
    ensure!(
        accept_stream(&mut new).await? == bobs,
        "a client arriving after supersession must reach the new trunk"
    );

    // Inside the window, the old trunk still delivers what it owed.
    old.send_json(response_head(answered)).await?;
    old.send_body(BODY_RESPONSE, answered, b"drained").await?;
    old.send_json(close(answered)).await?;
    let head = alice.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    ensure!(head["stream_id"] == answered, "{head}");
    let (_, id, body) = alice.recv_body(STEP).await?;
    ensure!(id == answered && body == b"drained");
    let end = alice.recv_type("CLOSE", STEP).await?;
    ensure!(end["stream_id"] == answered, "{end}");

    // Then what is still open is ended trunk_superseded, by name.
    let error = alice
        .expect_error("trunk_superseded", ctx.knobs.supersession_drain + STEP)
        .await?;
    wire::check_on_stream(&error, abandoned)?;
    ensure!(
        superseded_at.elapsed() >= ctx.knobs.supersession_drain.mul_f64(0.8),
        "trunk_superseded after {:?}; the drain is {:?}",
        superseded_at.elapsed(),
        ctx.knobs.supersession_drain
    );

    // The new trunk is untouched by any of it.
    new.send_json(response_head(bobs)).await?;
    let head = bob.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    ensure!(head["stream_id"] == bobs, "{head}");
    Ok(())
}

async fn an_unfinished_registration_supersedes_nothing(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let identity = pair.identity.clone();

    // The right pubkey, a signature that fails: nobody holding the key.
    let mut impostor = ctx.dial_register(&ctx.relay).await?;
    let nonce = Ctx::challenge(&mut impostor, &identity.server_id, &identity.pubkey_b64()).await?;
    Ctx::respond(
        &mut impostor,
        &ctx.intruder(&identity.server_id).relay_auth(&nonce),
    )
    .await?;
    impostor.expect_error("auth_failed", STEP).await?;

    tokio::time::sleep(ctx.knobs.supersession_drain + Duration::from_millis(500)).await;
    pair.client
        .expect_silence(QUIET)
        .await
        .context("the live trunk's stream after a failed registration")?;
    pair.server.send_json(response_head(pair.stream_id)).await?;
    pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;

    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let stream_id = open_stream(&mut client, json!("still-the-old-trunk")).await?;
    ensure!(accept_stream(&mut pair.server).await? == stream_id);
    Ok(())
}

async fn a_server_lost_mid_body_is_trunk_lost_never_a_clean_end(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.client
        .send_json(request_head(pair.stream_id, "GET", "/v1/notes/x"))
        .await?;
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    pair.server.send_json(response_head(pair.stream_id)).await?;
    pair.server
        .send_body(BODY_RESPONSE, pair.stream_id, b"half a bo")
        .await?;
    pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    pair.client.recv_body(STEP).await?;

    pair.server.kill();
    let error = pair
        .client
        .expect_error("trunk_lost", ctx.knobs.heartbeat_deadline + STEP)
        .await
        .context("a body cut off by the server's death must not end cleanly")?;
    wire::check_on_stream(&error, pair.stream_id)?;
    Ok(())
}

async fn every_open_stream_is_told_trunk_lost(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut alice = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut bob = ctx.client(&ctx.relay, &identity.server_id).await?;
    let a1 = open_stream(&mut alice, json!("a1")).await?;
    accept_stream(&mut server).await?;
    let a2 = open_stream(&mut alice, json!("a2")).await?;
    accept_stream(&mut server).await?;
    let b1 = open_stream(&mut bob, json!("b1")).await?;
    accept_stream(&mut server).await?;

    server.kill();
    for (client, streams) in [(&mut alice, vec![a1, a2]), (&mut bob, vec![b1])] {
        let mut told = Vec::new();
        while told.len() < streams.len() {
            let error = client
                .expect_error("trunk_lost", ctx.knobs.heartbeat_deadline + STEP)
                .await?;
            told.push(wire::stream_id_of(&error)?);
        }
        told.sort_unstable();
        ensure!(
            told == streams,
            "{}: trunk_lost named {told:?}, its open streams were {streams:?}",
            client.label()
        );
    }
    Ok(())
}

async fn a_server_gone_is_unreachable_until_it_registers_again(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    server.kill();

    // Server offline, relay up: the client is told about the server.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut late = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
    hello(&late, &identity.server_id).await?;
    late.expect_error("server_unreachable", ctx.knobs.hello_wait + STEP)
        .await?;

    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let stream_id = open_stream(&mut client, json!("back")).await?;
    ensure!(accept_stream(&mut server).await? == stream_id);
    Ok(())
}
