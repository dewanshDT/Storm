//! Revision 115 rules that were "storm-relay only" or had no check (decision
//! 116l): each now holds both relays by URL.

use anyhow::{Result, ensure};
use serde_json::json;
use tokio::time::Instant;

use super::{Check, check};
use crate::harness::{Ctx, QUIET, STEP, accept_stream, close, hello, open_stream};
use crate::wire::{self, Frame};

pub fn checks() -> Vec<Check> {
    vec![
        check!("§5", a_hello_for_another_server_id_is_a_protocol_error),
        check!("§5", frames_before_ready_never_reach_the_origin),
        check!("§5.4", a_non_boolean_aborted_is_a_protocol_error),
        check!("§5.1", attempt_ids_up_to_64_characters_are_echoed),
        check!("§4.2", server_trunk_death_ends_every_client_trunk),
        check!("§4.2", a_non_pinging_origin_is_dropped),
        check!("§5.1", a_stream_closed_after_ready_is_released),
        check!("§5.4", every_stream_the_relay_ends_is_closed_at_the_origin),
    ]
}

/// §5: `HELLO.server_id` must equal the path, refused rather than resolved,
/// and a trunk-scoped `ERROR` is followed by a close (§6.1).
async fn a_hello_for_another_server_id_is_a_protocol_error(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let _server = ctx.register(&ctx.relay, &identity).await?;
    let other = ctx.identity();
    let mut client = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
    hello(&client, &other.server_id).await?;
    let error = client.expect_error("protocol_error", STEP).await?;
    wire::check_trunk_scoped(&error)?;
    client.expect_closed(STEP).await
}

/// §5: a stream frame sent while a `HELLO` is held for a server is refused and
/// never reaches the origin that registers afterwards.
async fn frames_before_ready_never_reach_the_origin(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let client = ctx.dial_connect(&ctx.relay, &identity.server_id).await?;
    hello(&client, &identity.server_id).await?;
    client
        .send_json(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "early" }))
        .await?;
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    server.expect_silence(ctx.knobs.hello_wait + QUIET).await?;
    let mut client = client;
    let error = client
        .expect_error("protocol_error", ctx.knobs.hello_wait + STEP)
        .await?;
    wire::check_trunk_scoped(&error)?;
    Ok(())
}

/// §5.4 rule 1: `aborted` is a boolean; anything else is `protocol_error` at
/// the relay, from the origin that sent it.
async fn a_non_boolean_aborted_is_a_protocol_error(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.server
        .send_json(
            json!({ "v": 1, "type": "CLOSE", "stream_id": pair.stream_id, "aborted": "yes" }),
        )
        .await?;
    let error = pair.server.expect_error("protocol_error", STEP).await?;
    wire::check_trunk_scoped(&error)?;
    Ok(())
}

/// §5.1: an `attempt_id` of 1–64 ASCII characters is echoed exactly.
async fn attempt_ids_up_to_64_characters_are_echoed(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    for len in [1, 64] {
        let id = "a".repeat(len);
        open_stream(&mut client, json!(id)).await?;
        accept_stream(&mut server).await?;
    }
    Ok(())
}

/// §4.2: a dead server trunk ends every client bound to it, a client holding
/// no stream included (trunk-scoped `trunk_lost`), then closes them.
async fn server_trunk_death_ends_every_client_trunk(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let mut idle = ctx.client(&ctx.relay, &pair.identity.server_id).await?;
    pair.server.kill();

    let error = pair.client.expect_error("trunk_lost", STEP).await?;
    wire::check_on_stream(&error, pair.stream_id)?;
    pair.client.expect_closed(STEP).await?;

    let error = idle.expect_error("trunk_lost", STEP).await?;
    wire::check_trunk_scoped(&error)?;
    idle.expect_closed(STEP).await
}

/// §4.2: answering the relay is not heartbeating. An origin that never sends a
/// frame of its own is dropped at the deadline.
async fn a_non_pinging_origin_is_dropped(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.server.stop_heartbeat();
    let since = Instant::now();
    let error = pair
        .client
        .expect_error("trunk_lost", ctx.knobs.heartbeat_deadline + STEP)
        .await?;
    wire::check_on_stream(&error, pair.stream_id)?;
    ensure!(
        since.elapsed() >= ctx.knobs.heartbeat_deadline / 2,
        "dropped after {:?}; the deadline is {:?}",
        since.elapsed(),
        ctx.knobs.heartbeat_deadline
    );
    Ok(())
}

/// §5.1, §5.4: a client `CLOSE` straight after `STREAM_READY` frees the
/// stream at the relay. Repeated past the stream cap, a leak would be refused.
async fn a_stream_closed_after_ready_is_released(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    for n in 0..ctx.knobs.max_total_streams + 2 {
        let stream_id = open_stream(&mut client, json!(format!("r{n}"))).await?;
        accept_stream(&mut server).await?;
        client.send_json(close(stream_id)).await?;
        server.recv_type("CLOSE", STEP).await?;
    }
    Ok(())
}

/// §5.4: whenever the relay ends a stream for any reason but the origin's own
/// `CLOSE`, the origin hears `CLOSE{stream_id}`: a client's `CLOSE`, and the
/// client trunk dying.
async fn every_stream_the_relay_ends_is_closed_at_the_origin(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.client.send_json(close(pair.stream_id)).await?;
    let end = pair.server.recv_type("CLOSE", STEP).await?;
    ensure!(end["stream_id"] == pair.stream_id, "client CLOSE: {end}");

    let second = open_stream(&mut pair.client, json!("second")).await?;
    ensure!(accept_stream(&mut pair.server).await? == second);
    pair.client.kill();
    let end = pair
        .server
        .wait_for(
            "CLOSE after the client trunk died",
            STEP,
            |f| matches!(f, Frame::Text(v) if v["type"] == "CLOSE"),
        )
        .await?;
    let Frame::Text(end) = end else {
        unreachable!()
    };
    ensure!(end["stream_id"] == second, "client trunk death: {end}");
    Ok(())
}
