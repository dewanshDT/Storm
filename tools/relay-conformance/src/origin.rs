//! The origin role suite (decision 116l): this tool plays the relay, a real
//! `storm-server` registers with it, and each check judges the server's half
//! of SRP v1 by what crosses the socket. It needs the server's address and an
//! owner session (`STORM_BASE`, `STORM_SESSION`), which `make test-live`
//! exports, and nothing from the server's code.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Instant;

use crate::harness::STEP;
use crate::wire::{Conn, Frame};

/// What the relay tells the server: §4.2's interval, short so a check of the
/// server's own heartbeat takes seconds.
const INTERVAL_SECS: u64 = 1;

pub struct Origin {
    base: String,
    session: String,
    listener: TcpListener,
    relay_url: String,
    server_id: String,
    pubkey: VerifyingKey,
}

/// One minimal HTTP/1.1 exchange with the server, direct (not relayed).
async fn http(
    base: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<(u16, Value)> {
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    let mut socket = tokio::net::TcpStream::connect(host)
        .await
        .with_context(|| format!("connecting to {host}"))?;
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(request.as_bytes()).await?;
    let mut raw = Vec::new();
    socket.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw);
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("no status line from {method} {path}"))?;
    let json = text
        .split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    Ok((status, json))
}

impl Origin {
    pub async fn new(base: String, session: String) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let relay_url = format!("ws://{}", listener.local_addr()?);
        let (status, info) = http(&base, "GET", "/v1/server", &session, None).await?;
        ensure!(status == 200, "GET /v1/server: {status}");
        let server_id = info["server_id"].as_str().context("server_id")?.to_string();
        let key = BASE64URL_NOPAD.decode(
            info["public_key"]
                .as_str()
                .context("public_key")?
                .as_bytes(),
        )?;
        let pubkey = VerifyingKey::from_bytes(&key.try_into().map_err(|_| anyhow!("key length"))?)?;
        Ok(Self {
            base,
            session,
            listener,
            relay_url,
            server_id,
            pubkey,
        })
    }

    async fn set_relays(&self, relays: &[&str]) -> Result<u16> {
        Ok(http(
            &self.base,
            "PUT",
            "/v1/config/relays",
            &self.session,
            Some(json!({ "relays": relays })),
        )
        .await?
        .0)
    }

    /// Waits for the server to dial, plays §4.1's handshake and checks the
    /// signature, then hands back the live trunk.
    async fn accept_trunk(&self, within: Duration) -> Result<Conn> {
        let (socket, _) = tokio::time::timeout(within, self.listener.accept())
            .await
            .context("the server never dialled the relay")??;
        let ws = tokio_tungstenite::accept_async(socket).await?;
        let mut trunk = Conn::spawn(ws, "origin@relay");
        let register = trunk.recv_type("REGISTER_SERVER", STEP).await?;
        ensure!(register["server_id"] == self.server_id, "{register}");
        let pubkey = register["pubkey"].as_str().unwrap_or_default();
        ensure!(
            pubkey == BASE64URL_NOPAD.encode(self.pubkey.as_bytes()),
            "REGISTER_SERVER names a key other than /v1/server's"
        );
        let nonce = format!("origin-suite-{:016x}", rand::random::<u64>());
        trunk
            .send_json(json!({ "v": 1, "type": "CHALLENGE", "nonce": nonce }))
            .await?;
        let answer = trunk.recv_type("CHALLENGE_RESPONSE", STEP).await?;
        let sig = BASE64URL_NOPAD.decode(answer["sig"].as_str().unwrap_or_default().as_bytes())?;
        let message = format!("storm-relay-auth:v1:{}:{nonce}", self.server_id);
        self.pubkey
            .verify(message.as_bytes(), &Signature::from_slice(&sig)?)
            .map_err(|_| {
                anyhow!("§4.1: the signature does not verify over the relay-auth domain")
            })?;
        trunk
            .send_json(json!({
                "v": 1, "type": "REGISTERED", "trunk_id": "trk_origin_suite",
                "public_address": format!("{}/connect/{}", self.relay_url, self.server_id),
                "heartbeat_interval_secs": INTERVAL_SECS,
            }))
            .await?;
        Ok(trunk)
    }
}

/// One relayed exchange: open, ack, head, end of body; the response head,
/// body and end.
async fn exchange(
    trunk: &mut Conn,
    stream_id: u32,
    method: &str,
    path: &str,
    headers: Value,
) -> Result<(Value, Vec<u8>)> {
    trunk
        .send_json(json!({ "v": 1, "type": "STREAM_OPEN", "stream_id": stream_id }))
        .await?;
    let ack = trunk.recv_type("STREAM_ACK", STEP).await?;
    ensure!(ack["stream_id"] == stream_id, "§5.1: {ack}");
    let mut head = json!({
        "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": stream_id,
        "method": method, "path": path, "headers": headers,
    });
    head["relay_peer_ip"] = json!("127.0.0.1");
    trunk.send_json(head).await?;
    trunk.send_body(0x01, stream_id, b"").await?;
    let response = trunk.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
    let mut body = Vec::new();
    loop {
        match trunk.next(STEP).await? {
            Frame::Body { payload, .. } => body.extend_from_slice(&payload),
            Frame::Text(v) if v["type"] == "CLOSE" && v["stream_id"] == stream_id => {
                ensure!(
                    v.get("aborted").is_none_or(|a| a == false),
                    "the body was aborted: {v}"
                );
                return Ok((response, body));
            }
            Frame::Text(v) if v["type"] == "ERROR" => bail!("§5.1: the origin sent ERROR: {v}"),
            Frame::Text(_) => {}
            Frame::Closed => bail!("the trunk closed mid-response"),
            Frame::Invalid(why) => bail!("an invalid frame: {why}"),
        }
    }
}

fn bearer(o: &Origin) -> Value {
    // `STORM_SESSION` is the whole header value, `Bearer sta_…`.
    json!({ "authorization": o.session })
}

/// Runs every origin check in order on one server, printing a line each.
pub async fn run(base: String, session: String) -> Result<bool> {
    let origin = Origin::new(base, session).await?;
    let status = origin.set_relays(&[&origin.relay_url]).await?;
    ensure!(status == 200, "PUT /v1/config/relays: {status}");
    let mut trunk = origin.accept_trunk(Duration::from_secs(20)).await?;
    let mut failed = 0;
    let mut report = |name: &str, result: Result<()>| match result {
        Ok(()) => println!("pass  origin  {name}"),
        Err(e) => {
            failed += 1;
            println!("FAIL  origin  {name}\n        {e:#}");
        }
    };

    trunk.show_heartbeats(true);
    report(
        "§4.2 the server pings at the advertised interval",
        async {
            trunk
                .wait_for(
                    "PING",
                    Duration::from_secs(INTERVAL_SECS * 3),
                    |f| matches!(f, Frame::Text(v) if v["type"] == "PING"),
                )
                .await
                .map(|_| ())
        }
        .await,
    );
    report(
        "§4.2 a relay PING is answered",
        async {
            trunk.send_json(json!({ "v": 1, "type": "PING" })).await?;
            trunk
                .wait_for(
                    "PONG",
                    STEP,
                    |f| matches!(f, Frame::Text(v) if v["type"] == "PONG"),
                )
                .await
                .map(|_| ())
        }
        .await,
    );
    trunk.show_heartbeats(false);

    report(
        "§5.2 a relayed request is served whole",
        async {
            let (head, body) = exchange(&mut trunk, 1, "GET", "/v1/health", json!({})).await?;
            ensure!(head["status"] == 200, "{head}");
            ensure!(!body.is_empty(), "an empty body");
            Ok(())
        }
        .await,
    );
    report(
        "§5.4 a served stream is forgotten, so its id opens again",
        async {
            let (head, _) = exchange(&mut trunk, 1, "GET", "/v1/health", json!({})).await?;
            ensure!(head["status"] == 200, "{head}");
            Ok(())
        }
        .await,
    );

    report("§5.2 a malformed head ends that stream only, never with ERROR", async {
        let vectors: Value = serde_json::from_str(include_str!("../../../docs/srp-vectors.json"))?;
        let mut id = 10;
        for case in vectors["framing"]["malformed_request_heads"]["cases"].as_array().context("vectors")? {
            id += 1;
            trunk.send_json(json!({ "v": 1, "type": "STREAM_OPEN", "stream_id": id })).await?;
            trunk.recv_type("STREAM_ACK", STEP).await?;
            let mut head = case["json"].clone();
            head["stream_id"] = json!(id);
            trunk.send_json(head).await?;
            let end = trunk
                .wait_for("the stream's end", STEP, |f| {
                    matches!(f, Frame::Text(v) if v["stream_id"] == id && (v["type"] == "CLOSE" || v["type"] == "ERROR"))
                })
                .await?;
            let Frame::Text(end) = end else { unreachable!() };
            ensure!(end["type"] == "CLOSE", "{}: §5.1 the origin sent {end}", case["name"]);
        }
        let (head, _) = exchange(&mut trunk, 30, "GET", "/v1/health", json!({})).await?;
        ensure!(head["status"] == 200, "the trunk did not carry on: {head}");
        Ok(())
    }.await);

    report("§5.2 a second head aborts its stream", async {
        trunk.send_json(json!({ "v": 1, "type": "STREAM_OPEN", "stream_id": 40 })).await?;
        trunk.recv_type("STREAM_ACK", STEP).await?;
        let head = json!({ "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": 40, "method": "GET",
            "path": "/v1/vaults", "headers": bearer(&origin), "relay_peer_ip": "127.0.0.1" });
        trunk.send_json(head.clone()).await?;
        trunk.send_json(head).await?;
        let end = trunk
            .wait_for("an aborted CLOSE", STEP, |f| {
                matches!(f, Frame::Text(v) if v["type"] == "CLOSE" && v["stream_id"] == 40 && v["aborted"] == true)
            })
            .await;
        end.map(|_| ())
    }.await);

    report(
        "§4.3 the relay set is advertised and signed",
        async {
            let (head, body) = exchange(&mut trunk, 50, "GET", "/v1/server", json!({})).await?;
            ensure!(head["status"] == 200, "{head}");
            let info: Value = serde_json::from_slice(&body)?;
            let urls: Vec<String> = info["relays"]
                .as_array()
                .context("relays")?
                .iter()
                .filter_map(|r| r["url"].as_str().map(str::to_string))
                .collect();
            ensure!(
                urls.contains(&origin.relay_url),
                "this relay is not advertised: {urls:?}"
            );
            let mut sorted = urls.clone();
            sorted.sort();
            let message = format!("storm-relays:v1:{}:{}", origin.server_id, sorted.join("\n"));
            let sig = BASE64URL_NOPAD.decode(
                info["relays_sig"]
                    .as_str()
                    .context("relays_sig")?
                    .as_bytes(),
            )?;
            origin
                .pubkey
                .verify(message.as_bytes(), &Signature::from_slice(&sig)?)
                .map_err(|_| anyhow!("relays_sig does not verify"))
        }
        .await,
    );

    report(
        "§5.5 an attachment resumes from a Range",
        async {
            let (status, vaults) =
                http(&origin.base, "GET", "/v1/vaults", &origin.session, None).await?;
            ensure!(status == 200, "GET /v1/vaults: {status}");
            let vault = vaults
                .as_array()
                .or_else(|| vaults["vaults"].as_array())
                .and_then(|v| v.first())
                .and_then(|v| v["id"].as_str())
                .context("no vault")?
                .to_string();
            let path = format!("/v1/vaults/{vault}/attachments/origin-suite.bin");
            trunk
                .send_json(json!({ "v": 1, "type": "STREAM_OPEN", "stream_id": 60 }))
                .await?;
            trunk.recv_type("STREAM_ACK", STEP).await?;
            trunk
                .send_json(
                    json!({ "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": 60, "method": "PUT",
                "path": path, "headers": bearer(&origin), "relay_peer_ip": "127.0.0.1" }),
                )
                .await?;
            trunk.send_body(0x01, 60, b"0123456789").await?;
            trunk.send_body(0x01, 60, b"").await?;
            let put = trunk.recv_type("HTTP_RESPONSE_HEAD", STEP).await?;
            ensure!(put["status"] == 200, "upload: {put}");
            trunk
                .wait_for(
                    "the upload's end",
                    STEP,
                    |f| matches!(f, Frame::Text(v) if v["type"] == "CLOSE" && v["stream_id"] == 60),
                )
                .await?;
            let mut headers = bearer(&origin);
            headers["range"] = json!("bytes=4-");
            let (head, body) = exchange(&mut trunk, 61, "GET", &path, headers).await?;
            ensure!(head["status"] == 206, "{head}");
            ensure!(body == b"456789", "{:?}", String::from_utf8_lossy(&body));
            Ok(())
        }
        .await,
    );

    report(
        "§4.2 a trunk that dies young does not reset the backoff",
        async {
            trunk.kill();
            let first = Instant::now();
            let mut again = origin.accept_trunk(Duration::from_secs(10)).await?;
            let d1 = first.elapsed();
            again.kill();
            let second = Instant::now();
            let mut third = origin.accept_trunk(Duration::from_secs(10)).await?;
            let d2 = second.elapsed();
            ensure!(
                d2 >= Duration::from_millis(1500) && d2 > d1,
                "redialled after {d1:?} then {d2:?}: a short-lived trunk reset the backoff"
            );
            third.kill();
            Ok(())
        }
        .await,
    );

    report(
        "§4.3 a public-host ws:// relay is refused",
        async {
            let status = origin.set_relays(&["ws://8.8.8.8:9"]).await?;
            ensure!(
                status == 400 || status == 422,
                "PUT /v1/config/relays with a public ws:// gave {status}"
            );
            Ok(())
        }
        .await,
    );

    let _ = origin.set_relays(&[]).await;
    println!("origin: {failed} failed");
    Ok(failed == 0)
}
