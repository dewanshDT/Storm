//! §5.1 and §6's abuse controls — the in-flight and total stream caps and the
//! per-IP `HELLO` limit — and decision 91's latency, which is optional.

use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{Check, Needs, check};
use crate::harness::{
    Ctx, Relay, STEP, accept_stream, close, hello, open_stream, request_head, response_head,
};
use crate::wire::{self, BODY_REQUEST, BODY_RESPONSE, Conn, Frame};

pub fn checks() -> Vec<Check> {
    vec![
        check!("§5.1", past_the_in_flight_cap_an_open_is_refused_by_attempt),
        check!("§6", past_the_total_cap_an_open_is_refused_by_attempt),
        check!("§6", hello_is_rate_limited_per_source_ip),
        check!("§6", the_hello_limit_is_per_source_address, Needs::MultiIp),
        check!(
            "§5.2",
            a_relayed_exchange_does_not_wait_on_delayed_acks,
            Needs::Latency
        ),
    ]
}

/// §5.1: an `ERROR{rate_limited}` naming the refused attempt, and no stream.
async fn refused_open(client: &mut Conn, attempt_id: Value) -> Result<()> {
    let started = Instant::now();
    client
        .send_json(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": attempt_id }))
        .await?;
    let error = client.expect_error("rate_limited", STEP).await?;
    ensure!(
        error["attempt_id"] == attempt_id,
        "the refusal must echo attempt_id {attempt_id}: {error}"
    );
    ensure!(
        error.get("stream_id").is_none(),
        "a refused open has no stream: {error}"
    );
    // Immediately, never queued: an overloaded relay and a slow server must
    // not look alike.
    ensure!(
        started.elapsed() < STEP / 5,
        "the refusal took {:?}; it was queued",
        started.elapsed()
    );
    Ok(())
}

async fn past_the_in_flight_cap_an_open_is_refused_by_attempt(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let cap = ctx.knobs.max_in_flight_streams;

    // Pipelined, so the whole cap is in flight well inside the ack timeout.
    for n in 0..cap {
        client
            .send_json(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": n }))
            .await?;
    }
    let mut streams = Vec::new();
    for _ in 0..cap {
        streams.push(wire::stream_id_of(
            &client.recv_type("STREAM_READY", STEP).await?,
        )?);
    }
    let mut opened = Vec::new();
    for _ in 0..cap {
        opened.push(wire::stream_id_of(
            &server.recv_type("STREAM_OPEN", STEP).await?,
        )?);
    }

    refused_open(&mut client, json!({ "over": "the cap" })).await?;
    server
        .expect_silence(Duration::from_millis(150))
        .await
        .context("a refused OPEN_STREAM reached the origin")?;

    // An ack frees a slot: the cap bounds what is in flight, not the trunk.
    for stream_id in &opened {
        server
            .send_json(json!({ "v": 1, "type": "STREAM_ACK", "stream_id": stream_id }))
            .await?;
    }
    let mut late = None;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        client
            .send_json(json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "after" }))
            .await?;
        let reply = client.recv(STEP).await?;
        if reply["type"] == "STREAM_READY" {
            late = Some(wire::stream_id_of(&reply)?);
            break;
        }
        wire::check_error(&reply, "rate_limited")?;
    }
    let late = late.context("acking every in-flight open never freed a slot")?;
    ensure!(accept_stream(&mut server).await? == late);

    // And the trunk carried on throughout: the first stream still works.
    client
        .send_json(request_head(streams[0], "GET", "/v1/server"))
        .await?;
    server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    Ok(())
}

async fn past_the_total_cap_an_open_is_refused_by_attempt(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut alice = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut bob = ctx.client(&ctx.relay, &identity.server_id).await?;
    let total = ctx.knobs.max_total_streams;

    // Acked one at a time, so the in-flight cap never binds.
    let mut alices = Vec::new();
    for n in 0..total.div_ceil(2) {
        alices.push(open_stream(&mut alice, json!(format!("a{n}"))).await?);
        accept_stream(&mut server).await?;
    }
    for n in alices.len()..total {
        open_stream(&mut bob, json!(format!("b{n}"))).await?;
        accept_stream(&mut server).await?;
    }

    refused_open(&mut bob, json!("bob-over")).await?;
    refused_open(&mut alice, json!("alice-over")).await?;
    server
        .expect_silence(Duration::from_millis(150))
        .await
        .context("a refused OPEN_STREAM reached the origin")?;

    alice.send_json(close(alices[0])).await?;
    server.recv_type("CLOSE", STEP).await?;
    let freed = open_stream(&mut bob, json!("bob-again")).await?;
    ensure!(accept_stream(&mut server).await? == freed);
    Ok(())
}

enum HelloOutcome {
    Ready,
    RateLimited,
}

async fn one_hello(
    ctx: &Ctx,
    relay: &Relay,
    server_id: &str,
    source: Option<IpAddr>,
) -> Result<HelloOutcome> {
    let mut conn = ctx.dial_connect_unpaced(relay, server_id, source).await?;
    // A relay may refuse at the upgrade, before reading HELLO; the send then
    // fails and the ERROR is already waiting.
    let _ = hello(&conn, server_id).await;
    match conn.next(ctx.knobs.hello_wait + STEP).await? {
        Frame::Text(v) if v["type"] == "READY" => Ok(HelloOutcome::Ready),
        Frame::Text(v) if v["type"] == "ERROR" => {
            wire::check_error(&v, "rate_limited")?;
            wire::check_trunk_scoped(&v)?;
            Ok(HelloOutcome::RateLimited)
        }
        other => bail!("a HELLO got {}", other.describe()),
    }
}

/// Opens `count` client trunks at once and counts the outcomes.
async fn burst(
    ctx: &Ctx,
    relay: &Relay,
    server_id: &str,
    source: Option<IpAddr>,
    count: usize,
) -> Result<(usize, usize)> {
    let attempts = (0..count).map(|_| one_hello(ctx, relay, server_id, source));
    let mut ready = 0;
    let mut limited = 0;
    for outcome in futures_util::future::join_all(attempts).await {
        match outcome? {
            HelloOutcome::Ready => ready += 1,
            HelloOutcome::RateLimited => limited += 1,
        }
    }
    Ok((ready, limited))
}

async fn recovered(ctx: &Ctx, relay: &Relay, server_id: &str) -> Result<()> {
    relay.rate_limit_spent().await;
    tokio::time::sleep(ctx.knobs.hello_rate_window * 2).await;
    ctx.client(relay, server_id)
        .await
        .context("a HELLO two windows after the limit was hit")?;
    Ok(())
}

async fn hello_is_rate_limited_per_source_ip(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let _server = ctx.register(&ctx.relay, &identity).await?;
    // Twice the limit and more, so a fixed window straddled by the burst still
    // has to refuse some.
    let count = ctx.knobs.hello_rate_limit * 2 + 10;
    let (ready, limited) = burst(ctx, &ctx.relay, &identity.server_id, None, count).await?;
    ensure!(
        limited > 0,
        "{count} HELLOs at once from one address, every one READY; the limit is {} per {:?}",
        ctx.knobs.hello_rate_limit,
        ctx.knobs.hello_rate_window
    );
    ensure!(ready > 0, "every HELLO in the burst was refused");
    recovered(ctx, &ctx.relay, &identity.server_id).await
}

async fn the_hello_limit_is_per_source_address(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let _server = ctx.register(&ctx.relay, &identity).await?;
    let count = ctx.knobs.hello_rate_limit * 2 + 10;
    let (_, limited) = burst(ctx, &ctx.relay, &identity.server_id, None, count).await?;
    ensure!(limited > 0, "the burst from 127.0.0.1 was never limited");

    let elsewhere: IpAddr = "127.0.0.2".parse()?;
    match one_hello(ctx, &ctx.relay, &identity.server_id, Some(elsewhere)).await? {
        HelloOutcome::Ready => {}
        HelloOutcome::RateLimited => {
            bail!("{elsewhere} was limited by 127.0.0.1's burst: the limit is not per address")
        }
    }
    recovered(ctx, &ctx.relay, &identity.server_id).await
}

async fn a_relayed_exchange_does_not_wait_on_delayed_acks(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut samples = Vec::new();
    for n in 0..21 {
        let started = Instant::now();
        let id = open_stream(&mut client, json!(n)).await?;
        accept_stream(&mut server).await?;
        client
            .send_json(request_head(id, "GET", "/v1/notes"))
            .await?;
        client.send_body(BODY_REQUEST, id, b"").await?;
        server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
        server.recv_body(STEP).await?;
        server.send_json(response_head(id)).await?;
        server.send_body(BODY_RESPONSE, id, b"{}").await?;
        server.send_json(close(id)).await?;
        client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
        client.recv_body(STEP).await?;
        client.recv_type("CLOSE", STEP).await?;
        samples.push(started.elapsed());
    }
    samples.sort();
    let median = samples[samples.len() / 2];
    ensure!(
        median < Duration::from_millis(15),
        "the median relayed exchange took {median:?} (decision 91: TCP_NODELAY missing?)"
    );
    Ok(())
}
