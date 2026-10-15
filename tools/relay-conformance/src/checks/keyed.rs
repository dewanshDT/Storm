//! §4.1 `open` mode and §5 `HELLO.pubkey`: registrations addressed by
//! `(server_id, pubkey)`, and a client reaching only the key it names.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{Check, Needs, check};
use crate::harness::{Ctx, QUIET, Relay, STEP, accept_stream, open_stream};
use crate::wire::{self, Conn};

const VECTORS: &str = include_str!("../../../../docs/srp-vectors.json");

pub fn checks() -> Vec<Check> {
    vec![
        check!("§5", a_hello_for_another_key_is_server_unreachable),
        check!("§5", a_bad_or_disagreeing_hello_pubkey_is_a_protocol_error),
        check!(
            "§4.1",
            open_registrations_are_keyed_by_server_id_and_pubkey,
            Needs::Open
        ),
        check!("§4.1", a_squatter_cannot_take_over, Needs::Open),
        check!("§4.2", supersession_is_per_key, Needs::Open),
        check!(
            "§5",
            a_keyless_hello_is_refused_on_an_open_relay,
            Needs::Open
        ),
        check!(
            "§5",
            a_bad_or_disagreeing_hello_pubkey_is_refused_on_an_open_relay,
            Needs::Open
        ),
    ]
}

/// Requires `conn` to be told `server_unreachable` and never `READY`.
async fn unreachable(ctx: &Ctx, conn: &mut Conn, what: &str) -> Result<()> {
    let reply = conn
        .recv(ctx.knobs.hello_wait + STEP)
        .await
        .with_context(|| what.to_string())?;
    wire::check_error(&reply, "server_unreachable").with_context(|| what.to_string())?;
    wire::check_trunk_scoped(&reply)
}

/// The origin's side of one stream, which must reach `server` and no other.
async fn reaches_only(
    client: &mut Conn,
    server: &mut Conn,
    others: &mut [&mut Conn],
) -> Result<()> {
    let stream_id = open_stream(client, json!("keyed")).await?;
    ensure!(
        accept_stream(server).await? == stream_id,
        "the stream reached its trunk under another id"
    );
    for other in others.iter_mut() {
        other
            .expect_silence(QUIET)
            .await
            .context("a stream reached a trunk under a key the client did not name")?;
    }
    Ok(())
}

/// Allowlist and TOFU: a `HELLO.pubkey` other than the registered key is as
/// if no trunk were live; the right key and no key are routed.
async fn a_hello_for_another_key_is_server_unreachable(ctx: &Ctx) -> Result<()> {
    let identity = ctx.identity();
    let mut server = ctx.register(&ctx.relay, &identity).await?;

    let stranger = ctx.intruder(&identity.server_id);
    let mut wrong = ctx.keyed_hello(&ctx.relay, &stranger).await?;
    unreachable(ctx, &mut wrong, "a HELLO naming another key").await?;

    let mut right = ctx.keyed_client(&ctx.relay, &identity).await?;
    reaches_only(&mut right, &mut server, &mut []).await?;

    let mut keyless = ctx.client(&ctx.relay, &identity.server_id).await?;
    reaches_only(&mut keyless, &mut server, &mut []).await
}

async fn bad_hello_pubkeys(ctx: &Ctx, relay: &Relay) -> Result<()> {
    let identity = ctx.identity();
    let _server = ctx.register(relay, &identity).await?;
    let vectors: Value = serde_json::from_str(VECTORS)?;
    let section = &vectors["hello_pubkey"];

    let mut refused = 0;
    for case in section["cases"].as_array().context("hello_pubkey.cases")? {
        if case["accepted"] == true {
            continue;
        }
        refused += 1;
        let mut conn = ctx.dial_connect(relay, &identity.server_id).await?;
        conn.send_json(json!({
            "v": 1, "type": "HELLO", "server_id": identity.server_id, "pubkey": case["pubkey"],
        }))
        .await?;
        conn.expect_error("protocol_error", STEP)
            .await
            .with_context(|| format!("HELLO.pubkey spelled {}", case["name"]))?;
    }
    ensure!(refused > 0, "no refused spellings in the vectors");

    let agree = &section["query_and_hello_must_agree"];
    let query = agree["query_pubkey"].as_str().context("query_pubkey")?;
    let mut conn = ctx
        .dial_connect_keyed(relay, &identity.server_id, query)
        .await?;
    conn.send_json(json!({
        "v": 1, "type": "HELLO", "server_id": identity.server_id,
        "pubkey": agree["hello_pubkey"],
    }))
    .await?;
    conn.expect_error(agree["result"].as_str().context("result")?, STEP)
        .await
        .context("HELLO.pubkey disagreeing with ?pubkey=")?;
    Ok(())
}

async fn a_bad_or_disagreeing_hello_pubkey_is_a_protocol_error(ctx: &Ctx) -> Result<()> {
    bad_hello_pubkeys(ctx, &ctx.relay).await
}

async fn a_bad_or_disagreeing_hello_pubkey_is_refused_on_an_open_relay(ctx: &Ctx) -> Result<()> {
    bad_hello_pubkeys(ctx, ctx.open()?).await
}

async fn open_registrations_are_keyed_by_server_id_and_pubkey(ctx: &Ctx) -> Result<()> {
    let relay = ctx.open()?;
    let owner = ctx.identity();
    let other = ctx.intruder(&owner.server_id);
    let mut owner_trunk = ctx.register(relay, &owner).await?;
    let mut other_trunk = ctx
        .register(relay, &other)
        .await
        .context("a second key for the same server_id must register")?;

    let mut to_owner = ctx.keyed_client(relay, &owner).await?;
    reaches_only(&mut to_owner, &mut owner_trunk, &mut [&mut other_trunk]).await?;
    let mut to_other = ctx.keyed_client(relay, &other).await?;
    reaches_only(&mut to_other, &mut other_trunk, &mut [&mut owner_trunk]).await?;

    // Both clients stay where they are.
    reaches_only(&mut to_owner, &mut owner_trunk, &mut [&mut other_trunk]).await
}

async fn a_squatter_cannot_take_over(ctx: &Ctx) -> Result<()> {
    let relay = ctx.open()?;
    let owner = ctx.identity();
    let squatter = ctx.intruder(&owner.server_id);

    // First: the squatter alone. The owner's clients wait, then are told the
    // server is not there; they never reach the squatter.
    let mut squatter_trunk = ctx.register(relay, &squatter).await?;
    let mut waiting = ctx.keyed_hello(relay, &owner).await?;
    unreachable(
        ctx,
        &mut waiting,
        "the owner's HELLO with only a squatter live",
    )
    .await?;
    squatter_trunk.expect_silence(QUIET).await?;

    // Then the owner, and the squatter registering again: nothing of the
    // owner's is displaced.
    let mut owner_trunk = ctx.register(relay, &owner).await?;
    let mut client = ctx.keyed_client(relay, &owner).await?;
    let stream_id = open_stream(&mut client, json!("before")).await?;
    ensure!(accept_stream(&mut owner_trunk).await? == stream_id);
    let mut again = ctx.register(relay, &squatter).await?;
    owner_trunk
        .expect_silence(QUIET)
        .await
        .context("the owner's trunk must not hear of a squatter's registration")?;
    client
        .expect_silence(QUIET)
        .await
        .context("the owner's client must not be ended by a squatter's registration")?;
    reaches_only(&mut client, &mut owner_trunk, &mut [&mut again]).await
}

async fn supersession_is_per_key(ctx: &Ctx) -> Result<()> {
    let relay = ctx.open()?;
    let owner = ctx.identity();
    let other = ctx.intruder(&owner.server_id);
    let mut first = ctx.register(relay, &owner).await?;
    let mut other_trunk = ctx.register(relay, &other).await?;

    let mut on_first = ctx.keyed_client(relay, &owner).await?;
    let first_stream = open_stream(&mut on_first, json!("first")).await?;
    ensure!(accept_stream(&mut first).await? == first_stream);
    let mut on_other = ctx.keyed_client(relay, &other).await?;
    let other_stream = open_stream(&mut on_other, json!("other")).await?;
    ensure!(accept_stream(&mut other_trunk).await? == other_stream);

    let mut second = ctx.register(relay, &owner).await?;
    let ended = on_first
        .expect_error("trunk_superseded", ctx.knobs.supersession_drain + STEP)
        .await
        .context("the same key re-registering supersedes")?;
    wire::check_on_stream(&ended, first_stream)?;

    // The other key's stream outlived the whole drain and still carries.
    other_trunk
        .send_json(json!({ "v": 1, "type": "CLOSE", "stream_id": other_stream }))
        .await?;
    let close = on_other
        .recv_type("CLOSE", STEP)
        .await
        .context("another key's trunk must not be superseded")?;
    ensure!(close["stream_id"] == other_stream, "{close}");

    let mut fresh = ctx.keyed_client(relay, &owner).await?;
    reaches_only(&mut fresh, &mut second, &mut [&mut other_trunk]).await
}

/// Storm's open relays refuse every keyless `HELLO` (§5, the 114f
/// addendum), even with one registration live.
async fn a_keyless_hello_is_refused_on_an_open_relay(ctx: &Ctx) -> Result<()> {
    let relay = ctx.open()?;
    let identity = ctx.identity();
    let mut server = ctx.register(relay, &identity).await?;

    let mut bare = ctx.dial_connect(relay, &identity.server_id).await?;
    crate::harness::hello(&bare, &identity.server_id).await?;
    unreachable(ctx, &mut bare, "a keyless HELLO").await?;

    let mut with_query = ctx
        .dial_connect_keyed(relay, &identity.server_id, &identity.pubkey_b64())
        .await?;
    crate::harness::hello(&with_query, &identity.server_id).await?;
    unreachable(ctx, &mut with_query, "a keyless HELLO on a keyed URL").await?;

    server.expect_silence(QUIET).await
}
