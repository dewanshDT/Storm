//! §5.1–§5.4: streams — opening, the HTTP exchange, `relay_peer_ip`, bodies
//! and their frames, closing, aborting — and §3's rule that a `stream_id`
//! belongs to the relay and to exactly one client.

use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{Check, check};
use crate::harness::{
    Ctx, QUIET, STEP, accept_stream, close, open_stream, random_bytes, request_head, response_head,
};
use crate::wire::{self, BODY_REQUEST, BODY_RESPONSE, Conn, Frame};

pub fn checks() -> Vec<Check> {
    vec![
        check!(
            "§5.1",
            open_stream_echoes_any_attempt_id_and_reaches_the_origin
        ),
        check!(
            "§5.1",
            an_unacked_stream_is_server_timeout_and_never_retried
        ),
        check!("§5.2", an_acked_stream_that_stays_silent_is_never_timed_out),
        check!("§3", stream_ids_are_relay_assigned),
        check!(
            "§3",
            a_foreign_or_unknown_stream_id_is_refused_and_never_routed
        ),
        check!("§3", one_clients_response_never_reaches_another),
        check!("§5.2", a_request_and_response_cross_unchanged),
        check!("§5.2", the_empty_end_of_body_frame_is_forwarded),
        check!("§5.2", body_frames_are_forwarded_unmerged_and_unsplit),
        check!(
            "§5.2",
            a_body_over_16_mib_crosses_both_ways_in_256_kib_frames
        ),
        check!("§5.2", a_16_mib_frame_crosses_both_ways),
        check!("§5.2", a_response_streams_unbuffered),
        check!("§5.2", relay_peer_ip_is_the_socket_address_never_a_header),
        check!("§5.2", a_client_sent_relay_peer_ip_is_refused_even_null),
        check!("§5.4", close_ends_one_stream_and_the_trunk_carries_on),
        check!("§5.4", an_origin_close_follows_its_body_to_its_client_only),
        check!("§5.4", close_without_a_stream_id_ends_the_whole_trunk),
        check!(
            "§5.4",
            an_aborted_response_is_stream_aborted_for_that_stream_only
        ),
        check!(
            "§5.4",
            a_backlogged_client_is_told_and_its_other_streams_go_on
        ),
    ]
}

const FRAME: usize = 256 * 1024;

async fn open_stream_echoes_any_attempt_id_and_reaches_the_origin(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    // §5.1: echoed as the JSON value the client sent, whatever its type.
    for attempt_id in [
        json!("a-1"),
        json!(42),
        json!({ "n": [1, 2], "s": "x" }),
        json!(["x"]),
    ] {
        let stream_id = open_stream(&mut client, attempt_id.clone())
            .await
            .with_context(|| format!("attempt_id {attempt_id}"))?;
        let open = server.recv_type("STREAM_OPEN", STEP).await?;
        ensure!(
            open["stream_id"] == stream_id,
            "STREAM_OPEN {open} for STREAM_READY {stream_id}"
        );
        server
            .send_json(json!({ "v": 1, "type": "STREAM_ACK", "stream_id": stream_id }))
            .await?;
        client
            .send_json(request_head(stream_id, "GET", "/v1/server"))
            .await?;
        let head = server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
        ensure!(head["stream_id"] == stream_id, "{head}");
    }
    Ok(())
}

async fn an_unacked_stream_is_server_timeout_and_never_retried(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let stream_id = open_stream(&mut client, json!("never-acked")).await?;
    let opened = Instant::now();
    server.recv_type("STREAM_OPEN", STEP).await?;

    let error = client
        .expect_error("server_timeout", ctx.knobs.stream_ack_timeout + STEP)
        .await?;
    wire::check_on_stream(&error, stream_id)?;
    ensure!(
        opened.elapsed() >= ctx.knobs.stream_ack_timeout.mul_f64(0.8),
        "server_timeout after {:?}; the ack timeout is {:?}",
        opened.elapsed(),
        ctx.knobs.stream_ack_timeout
    );

    // No retry (§5.1): the origin hears of the stream once. A CLOSE freeing it
    // is allowed; a second STREAM_OPEN is not.
    while let Ok(frame) = server.next(QUIET).await {
        match frame {
            Frame::Text(v) if v["type"] == "CLOSE" && v["stream_id"] == stream_id => {}
            other => bail!("after server_timeout the origin got {}", other.describe()),
        }
    }
    Ok(())
}

async fn an_acked_stream_that_stays_silent_is_never_timed_out(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.client
        .send_json(request_head(pair.stream_id, "GET", "/v1/stream"))
        .await?;
    pair.client
        .send_body(BODY_REQUEST, pair.stream_id, b"")
        .await?;
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    pair.server.recv_body(STEP).await?;

    // Time to first byte bounds the ack, not the response: a quiet change
    // feed is normal (§5.2, §5.3).
    pair.client
        .expect_silence(ctx.knobs.stream_ack_timeout * 3)
        .await
        .context("a quiet stream after its ack")?;
    pair.server.send_json(response_head(pair.stream_id)).await?;
    pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    pair.server
        .send_body(BODY_RESPONSE, pair.stream_id, b"event: change\n\n")
        .await?;
    pair.client.recv_body(STEP).await?;
    Ok(())
}

async fn stream_ids_are_relay_assigned(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut alice = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut bob = ctx.client(&ctx.relay, &identity.server_id).await?;

    // The same attempt_id from two clients, concurrently: it is never routed
    // on, and the two streams must get different ids on the shared trunk.
    let open = json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "same" });
    alice.send_json(open.clone()).await?;
    bob.send_json(open).await?;
    let a = wire::stream_id_of(&alice.recv_type("STREAM_READY", STEP).await?)?;
    let b = wire::stream_id_of(&bob.recv_type("STREAM_READY", STEP).await?)?;
    ensure!(a != b, "two clients were both handed stream {a}");
    let mut origin = HashSet::new();
    for _ in 0..2 {
        origin.insert(accept_stream(&mut server).await?);
    }
    ensure!(
        origin == HashSet::from([a, b]),
        "the origin was told {origin:?} for streams {a} and {b}"
    );

    // A client cannot choose its id by asserting one.
    const ASSERTED: u32 = 0xDEAD_BEEF;
    alice
        .send_json(json!({
            "v": 1, "type": "OPEN_STREAM", "attempt_id": "mine", "stream_id": ASSERTED,
        }))
        .await?;
    let reply = alice.recv(STEP).await?;
    if reply["type"] == "ERROR" {
        wire::check_error(&reply, "protocol_error")?;
        return Ok(());
    }
    ensure!(reply["type"] == "STREAM_READY", "{reply}");
    let assigned = wire::stream_id_of(&reply)?;
    ensure!(
        assigned != ASSERTED,
        "the relay adopted the stream_id the client asserted"
    );
    ensure!(accept_stream(&mut server).await? == assigned);
    Ok(())
}

async fn a_foreign_or_unknown_stream_id_is_refused_and_never_routed(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut alice = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut bob = ctx.client(&ctx.relay, &identity.server_id).await?;
    let alices = open_stream(&mut alice, json!("a")).await?;
    accept_stream(&mut server).await?;

    const NEVER_ISSUED: u32 = 4_000_000_001;
    for (what, stream_id) in [
        ("Alice's live stream", alices),
        ("an id never issued", NEVER_ISSUED),
    ] {
        bob.send_json(request_head(stream_id, "GET", "/v1/notes"))
            .await?;
        let error = bob
            .expect_error("stream_closed", STEP)
            .await
            .with_context(|| format!("a request head naming {what}"))?;
        wire::check_on_stream(&error, stream_id)?;

        bob.send_body(BODY_REQUEST, stream_id, b"injected").await?;
        let error = bob
            .expect_error("stream_closed", STEP)
            .await
            .with_context(|| format!("a body frame naming {what}"))?;
        wire::check_on_stream(&error, stream_id)?;

        bob.send_json(close(stream_id)).await?;
        let error = bob
            .expect_error("stream_closed", STEP)
            .await
            .with_context(|| format!("a CLOSE naming {what}"))?;
        wire::check_on_stream(&error, stream_id)?;
    }

    server
        .expect_silence(QUIET)
        .await
        .context("a frame naming another client's stream reached the origin")?;
    alice
        .expect_silence(QUIET)
        .await
        .context("Alice's stream was disturbed")?;

    // Stream-scoped refusals: Bob's trunk is still good, and so is Alice's.
    let bobs = open_stream(&mut bob, json!("b")).await?;
    ensure!(accept_stream(&mut server).await? == bobs);
    alice
        .send_json(request_head(alices, "GET", "/v1/notes"))
        .await?;
    server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    Ok(())
}

async fn one_clients_response_never_reaches_another(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut alice = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut bob = ctx.client(&ctx.relay, &identity.server_id).await?;
    let alices = open_stream(&mut alice, json!("a1")).await?;
    accept_stream(&mut server).await?;
    let bobs = open_stream(&mut bob, json!("a1")).await?;
    accept_stream(&mut server).await?;

    server.send_json(response_head(alices)).await?;
    server
        .send_body(BODY_RESPONSE, alices, b"alice's private note")
        .await?;
    server.send_json(close(alices)).await?;

    let head = alice.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    ensure!(head["stream_id"] == alices, "{head}");
    let (_, id, body) = alice.recv_body(STEP).await?;
    ensure!(id == alices && body == b"alice's private note");
    alice.recv_type("CLOSE", STEP).await?;
    bob.expect_silence(QUIET)
        .await
        .context("a response crossed to another client")?;

    server.send_json(response_head(bobs)).await?;
    let head = bob.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    ensure!(head["stream_id"] == bobs, "{head}");
    alice.expect_silence(QUIET).await?;
    Ok(())
}

async fn a_request_and_response_cross_unchanged(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let id = pair.stream_id;
    let headers = json!({
        "authorization": "Bearer inside-the-tunnel",
        "content-type": "application/json",
        "x-storm-test": "unchanged",
    });
    pair.client
        .send_json(json!({
            "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": id,
            "method": "POST", "path": "/v1/notes?vault=x&q=a%20b", "headers": headers,
        }))
        .await?;
    pair.client
        .send_body(BODY_REQUEST, id, br#"{"title":"t"}"#)
        .await?;
    pair.client.send_body(BODY_REQUEST, id, b"").await?;

    let head = pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    ensure!(head["stream_id"] == id, "{head}");
    ensure!(head["method"] == "POST", "{head}");
    ensure!(head["path"] == "/v1/notes?vault=x&q=a%20b", "{head}");
    // R12: the credential rides inside, unparsed.
    ensure!(head["headers"] == headers, "headers were rewritten: {head}");
    let (kind, sid, body) = pair.server.recv_body(STEP).await?;
    ensure!(kind == BODY_REQUEST && sid == id && body == br#"{"title":"t"}"#);
    let (_, _, end) = pair.server.recv_body(STEP).await?;
    ensure!(end.is_empty());

    let response_headers = json!({ "content-type": "application/json", "etag": "\"v7\"" });
    pair.server
        .send_json(json!({
            "v": 1, "type": "HTTP_RESPONSE_HEAD", "stream_id": id,
            "status": 201, "headers": response_headers,
        }))
        .await?;
    pair.server
        .send_body(BODY_RESPONSE, id, br#"{"id":"n1"}"#)
        .await?;
    pair.server.send_json(close(id)).await?;

    let head = pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    ensure!(head["stream_id"] == id && head["status"] == 201, "{head}");
    ensure!(
        head["headers"] == response_headers,
        "headers were rewritten: {head}"
    );
    let (kind, sid, body) = pair.client.recv_body(STEP).await?;
    ensure!(kind == BODY_RESPONSE && sid == id && body == br#"{"id":"n1"}"#);
    let end = pair.client.recv_type("CLOSE", STEP).await?;
    ensure!(end["stream_id"] == id, "{end}");
    Ok(())
}

async fn the_empty_end_of_body_frame_is_forwarded(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let id = pair.stream_id;
    // A GET sends it straight after the head (rule 1).
    pair.client
        .send_json(request_head(id, "GET", "/v1/notes"))
        .await?;
    pair.client.send_body(BODY_REQUEST, id, b"").await?;
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    match pair.server.next(STEP).await? {
        Frame::Body {
            kind: BODY_REQUEST,
            stream_id,
            payload,
        } if stream_id == id && payload.is_empty() => {}
        other => bail!(
            "the end-of-body frame must arrive as the bare 5-byte header, got {}",
            other.describe()
        ),
    }

    let identity = pair.identity.clone();
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let post = open_stream(&mut client, json!("post")).await?;
    ensure!(accept_stream(&mut pair.server).await? == post);
    client
        .send_json(request_head(post, "PUT", "/v1/notes/x"))
        .await?;
    for chunk in [&b"a"[..], b"bc", b""] {
        client.send_body(BODY_REQUEST, post, chunk).await?;
    }
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    for expected in [&b"a"[..], b"bc", b""] {
        let (kind, sid, payload) = pair.server.recv_body(STEP).await?;
        ensure!(kind == BODY_REQUEST && sid == post && payload == expected);
    }
    Ok(())
}

async fn body_frames_are_forwarded_unmerged_and_unsplit(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let id = pair.stream_id;
    let request: Vec<Vec<u8>> = [1, 1000, FRAME, 7].into_iter().map(random_bytes).collect();
    pair.client
        .send_json(request_head(id, "PUT", "/v1/attachments/x"))
        .await?;
    for chunk in &request {
        pair.client.send_body(BODY_REQUEST, id, chunk).await?;
    }
    pair.client.send_body(BODY_REQUEST, id, b"").await?;
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    for (n, chunk) in request.iter().enumerate() {
        let (_, _, payload) = pair.server.recv_body(STEP).await?;
        ensure!(
            &payload == chunk,
            "request frame {n}: sent {} bytes, the origin got {}",
            chunk.len(),
            payload.len()
        );
    }
    pair.server.recv_body(STEP).await?;

    let response: Vec<Vec<u8>> = [FRAME, 3, 70_000].into_iter().map(random_bytes).collect();
    pair.server.send_json(response_head(id)).await?;
    for chunk in &response {
        pair.server.send_body(BODY_RESPONSE, id, chunk).await?;
    }
    pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    for (n, chunk) in response.iter().enumerate() {
        let (_, _, payload) = pair.client.recv_body(STEP).await?;
        ensure!(
            &payload == chunk,
            "response frame {n}: sent {} bytes, the client got {}",
            chunk.len(),
            payload.len()
        );
    }
    Ok(())
}

async fn a_body_over_16_mib_crosses_both_ways_in_256_kib_frames(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let id = pair.stream_id;
    let body = random_bytes(17 * 1024 * 1024 + 12_345);

    pair.client
        .send_json(request_head(id, "PUT", "/v1/attachments/big"))
        .await?;
    for chunk in body.chunks(FRAME) {
        pair.client.send_body(BODY_REQUEST, id, chunk).await?;
    }
    pair.client.send_body(BODY_REQUEST, id, b"").await?;
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    let mut received = Vec::with_capacity(body.len());
    loop {
        let (_, _, payload) = pair.server.recv_body(STEP).await?;
        if payload.is_empty() {
            break;
        }
        received.extend_from_slice(&payload);
    }
    ensure!(
        received == body,
        "the origin got {} bytes of a {}-byte upload, or different bytes",
        received.len(),
        body.len()
    );

    pair.server.send_json(response_head(id)).await?;
    for chunk in body.chunks(FRAME) {
        pair.server.send_body(BODY_RESPONSE, id, chunk).await?;
        // SRP has no flow control: an origin outrunning a reading client by
        // the relay's whole buffer is the backlogged case, not this one.
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    pair.server.send_json(close(id)).await?;
    pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    let mut received = Vec::with_capacity(body.len());
    loop {
        match pair.client.next(STEP).await? {
            Frame::Body { payload, .. } => received.extend_from_slice(&payload),
            Frame::Text(v) if v["type"] == "CLOSE" => break,
            other => bail!("downloading: {}", other.describe()),
        }
    }
    ensure!(
        received == body,
        "the client got {} bytes of a {}-byte download, or different bytes",
        received.len(),
        body.len()
    );
    Ok(())
}

async fn a_16_mib_frame_crosses_both_ways(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let id = pair.stream_id;
    // §5.2's ceiling is 16 MiB per frame, header included.
    let payload = random_bytes(16 * 1024 * 1024 - 5);

    pair.client
        .send_json(request_head(id, "PUT", "/v1/attachments/max"))
        .await?;
    pair.client.send_body(BODY_REQUEST, id, &payload).await?;
    pair.client.send_body(BODY_REQUEST, id, b"").await?;
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    let (_, _, got) = pair.server.recv_body(STEP).await?;
    ensure!(got == payload, "the origin got {} bytes", got.len());
    pair.server.recv_body(STEP).await?;

    pair.server.send_json(response_head(id)).await?;
    pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    pair.server.send_body(BODY_RESPONSE, id, &payload).await?;
    let (_, _, got) = pair.client.recv_body(STEP).await?;
    ensure!(got == payload, "the client got {} bytes", got.len());
    Ok(())
}

async fn a_response_streams_unbuffered(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let id = pair.stream_id;
    pair.client
        .send_json(request_head(id, "GET", "/v1/stream"))
        .await?;
    pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    // The head goes out before any body exists, and each chunk before the
    // next is written.
    pair.server
        .send_json(json!({
            "v": 1, "type": "HTTP_RESPONSE_HEAD", "stream_id": id, "status": 200,
            "headers": { "content-type": "text/event-stream" },
        }))
        .await?;
    pair.client.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    for chunk in [&b"event: change\n"[..], b"data: {}\n", b"\n"] {
        pair.server.send_body(BODY_RESPONSE, id, chunk).await?;
        let (_, _, got) = pair.client.recv_body(STEP).await?;
        ensure!(got == chunk);
    }
    Ok(())
}

async fn relay_peer_ip_is_the_socket_address_never_a_header(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let forged = "203.0.113.9";
    let headers = json!({
        "x-forwarded-for": forged,
        "forwarded": format!("for={forged}"),
        "x-real-ip": forged,
    });
    pair.client
        .send_json(json!({
            "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": pair.stream_id,
            "method": "GET", "path": "/v1/server", "headers": headers,
        }))
        .await?;
    let head = pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
    let peer: IpAddr = head["relay_peer_ip"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .with_context(|| format!("relay_peer_ip must be an IP address string: {head}"))?;
    ensure!(
        peer.to_string() != forged,
        "relay_peer_ip came from a forwarding header"
    );
    if let Some(expected) = ctx.options.peer_ip {
        ensure!(
            peer == expected,
            "relay_peer_ip is {peer}, the suite connected from {expected}"
        );
    }
    ensure!(
        head["headers"] == headers,
        "the relay must forward headers verbatim: {head}"
    );

    if ctx.options.multi_ip {
        let source: IpAddr = "127.0.0.2".parse()?;
        let mut other = ctx
            .client_from(&ctx.relay, &pair.identity.server_id, Some(source))
            .await?;
        let stream_id = open_stream(&mut other, json!("other-address")).await?;
        accept_stream(&mut pair.server).await?;
        other
            .send_json(request_head(stream_id, "GET", "/v1/server"))
            .await?;
        let head = pair.server.recv_type("HTTP_REQUEST_HEAD", STEP).await?;
        ensure!(
            head["relay_peer_ip"] == source.to_string(),
            "a client on {source} was attributed to {}",
            head["relay_peer_ip"]
        );
    }
    Ok(())
}

async fn a_client_sent_relay_peer_ip_is_refused_even_null(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    for forged in [json!("10.0.0.7"), Value::Null] {
        let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
        let stream_id = open_stream(&mut client, json!("forger")).await?;
        accept_stream(&mut server).await?;
        let mut head = request_head(stream_id, "GET", "/v1/server");
        head["relay_peer_ip"] = forged.clone();
        client.send_json(head).await?;
        crate::checks::framing::refused(&mut client)
            .await
            .with_context(|| format!("a client-sent relay_peer_ip of {forged}"))?;

        // The head itself never reaches the origin; freeing the stream may.
        while let Ok(frame) = server.next(QUIET).await {
            ensure!(
                !frame.is_type("HTTP_REQUEST_HEAD"),
                "a head carrying a client's relay_peer_ip ({forged}) was forwarded"
            );
        }
    }
    Ok(())
}

async fn close_ends_one_stream_and_the_trunk_carries_on(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    let id = pair.stream_id;
    pair.client.send_json(close(id)).await?;
    let closed = pair.server.recv_type("CLOSE", STEP).await?;
    ensure!(closed["stream_id"] == id, "{closed}");

    // The stream is dead; the trunk is not.
    pair.client
        .send_json(request_head(id, "GET", "/v1/notes"))
        .await?;
    let error = pair.client.expect_error("stream_closed", STEP).await?;
    wire::check_on_stream(&error, id)?;
    let next = open_stream(&mut pair.client, json!("next")).await?;
    ensure!(
        next != id,
        "a closed stream_id was handed out again at once"
    );
    ensure!(accept_stream(&mut pair.server).await? == next);
    Ok(())
}

async fn an_origin_close_follows_its_body_to_its_client_only(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut alice = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut bob = ctx.client(&ctx.relay, &identity.server_id).await?;
    let alices = open_stream(&mut alice, json!("a")).await?;
    accept_stream(&mut server).await?;
    open_stream(&mut bob, json!("b")).await?;
    accept_stream(&mut server).await?;

    server.send_json(response_head(alices)).await?;
    for chunk in [&b"one"[..], b"two", b"three"] {
        server.send_body(BODY_RESPONSE, alices, chunk).await?;
    }
    server.send_json(close(alices)).await?;

    alice.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    for chunk in [&b"one"[..], b"two", b"three"] {
        let (_, _, got) = alice
            .recv_body(STEP)
            .await
            .context("a CLOSE must never overtake the body it ends")?;
        ensure!(got == chunk);
    }
    let end = alice.recv_type("CLOSE", STEP).await?;
    ensure!(end["stream_id"] == alices, "{end}");
    bob.expect_silence(QUIET)
        .await
        .context("a CLOSE crossed to another client")?;
    Ok(())
}

async fn close_without_a_stream_id_ends_the_whole_trunk(ctx: &Ctx) -> Result<()> {
    let mut pair = ctx.stream().await?;
    pair.client
        .send_json(json!({ "v": 1, "type": "CLOSE" }))
        .await?;
    pair.client
        .expect_closed(STEP)
        .await
        .context("a client's CLOSE {}")?;

    let mut pair = ctx.stream().await?;
    pair.server
        .send_json(json!({ "v": 1, "type": "CLOSE" }))
        .await?;
    let error = pair
        .client
        .expect_error("trunk_lost", STEP)
        .await
        .context("an origin's CLOSE {} ends its trunk, and every stream on it")?;
    wire::check_on_stream(&error, pair.stream_id)?;
    Ok(())
}

async fn an_aborted_response_is_stream_aborted_for_that_stream_only(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut broken = ctx.client(&ctx.relay, &identity.server_id).await?;
    let mut healthy = ctx.client(&ctx.relay, &identity.server_id).await?;
    let aborted = open_stream(&mut broken, json!("aborted")).await?;
    accept_stream(&mut server).await?;
    let sibling = open_stream(&mut broken, json!("sibling")).await?;
    accept_stream(&mut server).await?;
    let whole = open_stream(&mut healthy, json!("whole")).await?;
    accept_stream(&mut server).await?;

    server.send_json(response_head(aborted)).await?;
    server
        .send_body(BODY_RESPONSE, aborted, b"half a bo")
        .await?;
    server
        .send_json(json!({ "v": 1, "type": "CLOSE", "stream_id": aborted, "aborted": true }))
        .await?;
    // The ERROR may overtake what is queued for its stream (§5.4), so the
    // head and partial body may or may not arrive first; nothing may follow.
    let error = broken
        .wait_for("stream_aborted", STEP, |f| match f {
            Frame::Text(v) => v["type"] == "ERROR" || v["stream_id"] != aborted,
            Frame::Body { stream_id, .. } => *stream_id != aborted,
            _ => true,
        })
        .await?;
    let Frame::Text(error) = error else {
        bail!("expected stream_aborted, got {}", error.describe());
    };
    wire::check_error(&error, "stream_aborted")?;
    wire::check_on_stream(&error, aborted)?;

    // `aborted: false` is a plain CLOSE.
    server.send_json(response_head(whole)).await?;
    server
        .send_json(json!({ "v": 1, "type": "CLOSE", "stream_id": whole, "aborted": false }))
        .await?;
    healthy.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    let end = healthy.recv_type("CLOSE", STEP).await?;
    ensure!(end["stream_id"] == whole, "{end}");

    // Not a trunk fault: the aborted stream's sibling carries on. Queued
    // frames of the ended stream may still trail its ERROR; the relay only
    // SHOULD discard them, and a client ignores them (§5.4 rule 5).
    server.send_json(response_head(sibling)).await?;
    let head = broken
        .wait_for("the sibling's head", STEP, |f| match f {
            Frame::Text(v) => v["stream_id"] != aborted,
            Frame::Body { stream_id, .. } => *stream_id != aborted,
            _ => true,
        })
        .await?;
    ensure!(
        matches!(&head, Frame::Text(v) if v["type"] == "HTTP_RESPONSE_HEAD" && v["stream_id"] == sibling),
        "expected the sibling's head, got {}",
        head.describe()
    );
    Ok(())
}

/// Sends until the relay gives up on the client and tells the origin, with
/// `CLOSE { stream_id }` (§5.4 rule 1).
async fn flood_until_dropped(server: &mut Conn, stream_id: u32, budget: usize) -> Result<()> {
    let chunk = vec![0x5a; FRAME];
    let limit = 256 * 1024 * 1024 + budget;
    let mut sent = 0;
    while sent < limit {
        server.send_body(BODY_RESPONSE, stream_id, &chunk).await?;
        sent += chunk.len();
        match server.try_next() {
            None => {}
            Some(Frame::Text(v)) if v["type"] == "CLOSE" && v["stream_id"] == stream_id => {
                return Ok(());
            }
            Some(other) => bail!("while flooding, the origin got {}", other.describe()),
        }
    }
    let closed = server
        .recv_type("CLOSE", STEP)
        .await
        .with_context(|| format!("{sent} bytes to a client that stopped reading"))?;
    ensure!(closed["stream_id"] == stream_id, "{closed}");
    Ok(())
}

async fn a_backlogged_client_is_told_and_its_other_streams_go_on(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;
    let mut client = ctx.client(&ctx.relay, &identity.server_id).await?;
    let flooded = open_stream(&mut client, json!("flooded")).await?;
    accept_stream(&mut server).await?;
    let other = open_stream(&mut client, json!("other")).await?;
    accept_stream(&mut server).await?;

    client.pause();
    server.send_json(response_head(other)).await?;
    server.send_body(BODY_RESPONSE, other, b"before").await?;
    server.send_json(response_head(flooded)).await?;
    flood_until_dropped(&mut server, flooded, ctx.knobs.max_client_buffer_bytes).await?;
    client.resume();

    // The flooded stream: maybe some body, then its ERROR, then nothing. The
    // other stream: everything, in order.
    let mut others = Vec::new();
    loop {
        match client.next(STEP).await.context("reading the backlog")? {
            Frame::Text(v) if v["type"] == "ERROR" => {
                wire::check_error(&v, "client_backlogged")?;
                wire::check_on_stream(&v, flooded)?;
                break;
            }
            Frame::Text(v) if v["stream_id"] == other => others.push(v["type"].to_string()),
            Frame::Text(v) if v["stream_id"] == flooded => {}
            Frame::Body { stream_id, .. } if stream_id == flooded => {}
            Frame::Body {
                stream_id, payload, ..
            } if stream_id == other => others.push(String::from_utf8_lossy(&payload).into()),
            Frame::Closed => bail!(
                "the relay closed the client trunk; §5.4 rule 3 allows that only when even the ERROR cannot be delivered"
            ),
            other => bail!("unexpected while reading the backlog: {}", other.describe()),
        }
    }

    server.send_body(BODY_RESPONSE, other, b"after").await?;
    server.send_json(close(other)).await?;
    loop {
        match client.next(STEP).await? {
            Frame::Text(v) if v["type"] == "CLOSE" && v["stream_id"] == other => break,
            Frame::Text(v) if v["stream_id"] == other => others.push(v["type"].to_string()),
            Frame::Body {
                stream_id, payload, ..
            } if stream_id == other => others.push(String::from_utf8_lossy(&payload).into()),
            // Queued frames of the ended stream may trail its ERROR: the relay
            // only SHOULD discard them (§5.4 rule 2), and a client ignores
            // them (rule 5).
            Frame::Text(v) if v["stream_id"] == flooded && v["type"] != "ERROR" => {}
            Frame::Body { stream_id, .. } if stream_id == flooded => {}
            frame => bail!("after client_backlogged, got {}", frame.describe()),
        }
    }
    ensure!(
        others == ["\"HTTP_RESPONSE_HEAD\"", "before", "after"],
        "the other stream's frames were {others:?}"
    );

    let next = open_stream(&mut client, json!("afterwards")).await?;
    ensure!(accept_stream(&mut server).await? == next);
    Ok(())
}
