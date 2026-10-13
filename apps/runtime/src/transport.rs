//! One HTTP exchange with the Storm Server, over whichever path reached it
//! (mcp-connect, decision 105c).
//!
//! A [`Transport`] carries a request to the server and its answer back: the
//! LAN directly ([`DirectTransport`]), or a relay's trunk ([`SrpTransport`]).
//! A [`Connector`] finds a path and **proves the server's identity on it
//! before returning it**, so a caller never holds a transport to a server it
//! has not verified.

use std::future::Future;
use std::time::Duration;

use anyhow::{Context, anyhow};
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;

use crate::identity::{challenge_message, decode_public_key, validate_server_id, verify};
use crate::srp::{SrpError, SrpTrunk};

pub struct HttpRequest {
    pub method: String,
    pub path_and_query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Why a request got no answer. `code` is the `storm_error` a caller reports:
/// it names the hop that failed (`server_offline`, `unreachable`,
/// `identity_failed`, and for a relay `relay_down`, `trunk_lost`, …).
#[derive(Debug, Clone)]
pub struct TransportError {
    pub code: &'static str,
    pub detail: String,
}

impl TransportError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for TransportError {}

pub trait Transport: Send + Sync {
    /// One request, one answer. A request that fails is never resent here.
    fn send(
        &self,
        req: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send;

    /// Whether this path is known dead, so a caller reconnects before its
    /// next request instead of spending that request to find out.
    fn is_closed(&self) -> bool {
        false
    }
}

pub trait Connector: Send + Sync {
    type Transport: Transport + 'static;
    /// A path to the pinned server, verified with [`verify_identity`] before
    /// it is returned.
    fn connect(&self) -> impl Future<Output = Result<Self::Transport, TransportError>> + Send;
}

/// The server a caller means: its id and the key it must prove it holds.
#[derive(Clone)]
pub struct ServerPin {
    pub server_id: String,
    pub key: VerifyingKey,
}

impl ServerPin {
    pub fn new(server_id: &str, public_key: &str) -> anyhow::Result<Self> {
        validate_server_id(server_id).map_err(|e| anyhow!("--server: {e}"))?;
        let key = decode_public_key(public_key).map_err(|e| anyhow!("--pk: {e}"))?;
        Ok(Self {
            server_id: server_id.to_string(),
            key,
        })
    }
}

#[derive(Deserialize)]
struct ChallengeAnswer {
    server_id: String,
    signature: String,
}

/// The identity challenge (`storm-challenge:v1:<sid>:<nonce>`), sent over
/// `transport` itself. Anything that answers but cannot prove the pinned key —
/// a refusal, a wrong body, another server — is `identity_failed`; only a
/// path that carried no answer at all keeps its own code.
pub async fn verify_identity<T: Transport>(
    transport: &T,
    pin: &ServerPin,
) -> Result<(), TransportError> {
    let nonce = crate::client::challenge_nonce();
    let response = transport
        .send(HttpRequest {
            method: "POST".into(),
            path_and_query: "/v1/server/challenge".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&serde_json::json!({ "nonce": nonce }))
                .expect("a JSON object serializes"),
        })
        .await?;
    let failed = |why: String| TransportError::new("identity_failed", why);
    if !(200..300).contains(&response.status) {
        return Err(failed(format!(
            "the identity challenge was answered with HTTP {}",
            response.status
        )));
    }
    let answer: ChallengeAnswer = serde_json::from_slice(&response.body)
        .map_err(|_| failed("the identity answer is not a challenge answer".into()))?;
    if answer.server_id != pin.server_id {
        return Err(failed(format!(
            "this is {}, not the pinned {}",
            answer.server_id, pin.server_id
        )));
    }
    if !verify(
        &pin.key,
        &challenge_message(&pin.server_id, &nonce),
        &answer.signature,
    ) {
        return Err(failed(format!(
            "{} could not prove it holds the pinned key",
            pin.server_id
        )));
    }
    Ok(())
}

/// Plain HTTP(S) to the server's own address.
pub struct DirectTransport {
    http: reqwest::Client,
    base: String,
}

impl DirectTransport {
    pub fn new(base: &str) -> anyhow::Result<Self> {
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            anyhow::bail!("--addr must be an http:// or https:// URL");
        }
        // reqwest panics building a client with no process provider; `ring`
        // as everywhere else. Idempotent.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // No whole-request timeout, which would cut a slow tool off; a
            // server that stops sending for this long is gone, and the error
            // is what lets the next request reconnect.
            .read_timeout(Duration::from_secs(120))
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        })
    }
}

fn direct_error(e: reqwest::Error) -> TransportError {
    let code = if e.is_builder() {
        "bad_request"
    } else if e.is_connect() {
        let mut source = std::error::Error::source(&e);
        let mut refused = false;
        while let Some(s) = source {
            if let Some(io) = s.downcast_ref::<std::io::Error>() {
                refused |= io.kind() == std::io::ErrorKind::ConnectionRefused;
            }
            source = s.source();
        }
        // Refused: the machine answered and nothing listens, so the server is
        // down. Anything else (no route, a timeout, DNS) says nothing about
        // the server.
        if refused {
            "server_offline"
        } else {
            "unreachable"
        }
    } else if e.is_timeout() {
        "server_silent"
    } else {
        "stream_aborted"
    };
    TransportError::new(code, e.without_url().to_string())
}

impl Transport for DirectTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|_| TransportError::new("bad_request", "not an HTTP method"))?;
        let mut builder = self
            .http
            .request(method, format!("{}{}", self.base, req.path_and_query))
            .body(req.body);
        for (name, value) in req.headers {
            builder = builder.header(name, value);
        }
        let response = builder.send().await.map_err(direct_error)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let body = response.bytes().await.map_err(direct_error)?.to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// The `Host` a relayed request carries. storm-server rewrites it to its
/// bind address (`relay/dispatch.rs`); `localhost` is also on every
/// `mcp::allowed_hosts` list, so a server that did not rewrite it still
/// accepts it.
pub const RELAYED_HOST: &str = "localhost";

/// How long a relayed request may go without a byte before it is given up,
/// the direct path's `read_timeout`. A relay that drops a stream without
/// telling the client (`drop_backlogged_client`) would otherwise hang it.
pub const IDLE_LIMIT: Duration = Duration::from_secs(120);

/// One request per stream on an SRP trunk (decision 105a).
#[derive(Clone)]
pub struct SrpTransport {
    trunk: SrpTrunk,
    idle: Duration,
}

impl SrpTransport {
    pub fn new(trunk: SrpTrunk, idle: Duration) -> Self {
        Self { trunk, idle }
    }

    pub fn trunk(&self) -> &SrpTrunk {
        &self.trunk
    }
}

fn srp_error(e: SrpError) -> TransportError {
    TransportError::new(e.code(), e.to_string())
}

impl Transport for SrpTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let silent = || {
            TransportError::new(
                "server_silent",
                format!(
                    "nothing from the server through the relay for {}s",
                    self.idle.as_secs()
                ),
            )
        };
        let mut headers: Vec<(String, String)> = req
            .headers
            .into_iter()
            .filter(|(n, _)| !n.eq_ignore_ascii_case("host"))
            .collect();
        headers.push(("host".into(), RELAYED_HOST.into()));
        let response = tokio::time::timeout(
            self.idle,
            self.trunk.request(
                &req.method,
                &req.path_and_query,
                &headers,
                bytes::Bytes::from(req.body),
            ),
        )
        .await
        .map_err(|_| silent())?
        .map_err(srp_error)?;
        let mut body = Vec::new();
        let mut stream = response.body;
        loop {
            match tokio::time::timeout(self.idle, stream.chunk()).await {
                Err(_) => return Err(silent()),
                Ok(None) => break,
                Ok(Some(Err(e))) => return Err(srp_error(e)),
                Ok(Some(Ok(chunk))) => body.extend_from_slice(&chunk),
            }
        }
        Ok(HttpResponse {
            status: response.status,
            headers: response.headers,
            body,
        })
    }

    fn is_closed(&self) -> bool {
        self.trunk.is_closed()
    }
}

/// The LAN-only connector: one address, verified on every connect.
pub struct DirectConnector {
    base: String,
    pin: ServerPin,
}

impl DirectConnector {
    pub fn new(base: &str, pin: ServerPin) -> anyhow::Result<Self> {
        DirectTransport::new(base)?;
        Ok(Self {
            base: base.to_string(),
            pin,
        })
    }
}

impl Connector for DirectConnector {
    type Transport = DirectTransport;

    async fn connect(&self) -> Result<DirectTransport, TransportError> {
        // A fresh client, so a reconnect never reuses a pooled connection
        // from before the failure.
        let transport = DirectTransport::new(&self.base)
            .map_err(|e| TransportError::new("unreachable", format!("{e:#}")))?;
        verify_identity(&transport, &self.pin).await?;
        Ok(transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_encoding::BASE64URL_NOPAD;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::{Value, json};

    /// Answers the challenge as a server holding `signing` and calling
    /// itself `server_id` would.
    struct Answers {
        signing: SigningKey,
        server_id: String,
        status: u16,
    }

    impl Transport for Answers {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            assert_eq!(req.path_and_query, "/v1/server/challenge");
            let nonce: Value = serde_json::from_slice(&req.body).unwrap();
            let nonce = nonce["nonce"].as_str().unwrap();
            let sig = self
                .signing
                .sign(&challenge_message(&self.server_id, nonce));
            Ok(HttpResponse {
                status: self.status,
                headers: vec![],
                body: serde_json::to_vec(&json!({
                    "server_id": self.server_id,
                    "signature": BASE64URL_NOPAD.encode(&sig.to_bytes()),
                }))
                .unwrap(),
            })
        }
    }

    fn pin(signing: &SigningKey) -> ServerPin {
        ServerPin {
            server_id: "srv_pinned".into(),
            key: signing.verifying_key(),
        }
    }

    #[tokio::test]
    async fn the_pinned_server_passes() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let t = Answers {
            signing: key.clone(),
            server_id: "srv_pinned".into(),
            status: 200,
        };
        verify_identity(&t, &pin(&key)).await.unwrap();
    }

    #[tokio::test]
    async fn another_key_another_id_or_a_refusal_is_identity_failed() {
        let pinned = SigningKey::from_bytes(&[7; 32]);
        let impostor = SigningKey::from_bytes(&[9; 32]);
        for t in [
            Answers {
                signing: impostor,
                server_id: "srv_pinned".into(),
                status: 200,
            },
            Answers {
                signing: pinned.clone(),
                server_id: "srv_other".into(),
                status: 200,
            },
            Answers {
                signing: pinned.clone(),
                server_id: "srv_pinned".into(),
                status: 404,
            },
        ] {
            let e = verify_identity(&t, &pin(&pinned)).await.unwrap_err();
            assert_eq!(e.code, "identity_failed", "{e}");
        }
    }

    #[tokio::test]
    async fn a_path_that_carried_nothing_keeps_its_own_code() {
        struct Down;
        impl Transport for Down {
            async fn send(&self, _: HttpRequest) -> Result<HttpResponse, TransportError> {
                Err(TransportError::new("relay_down", "no relay answered"))
            }
        }
        let key = SigningKey::from_bytes(&[7; 32]);
        let e = verify_identity(&Down, &pin(&key)).await.unwrap_err();
        assert_eq!(e.code, "relay_down");
    }

    #[test]
    fn a_relay_failure_keeps_its_own_code() {
        for e in [
            SrpError::ClientBacklogged,
            SrpError::StreamAborted,
            SrpError::ServerOffline,
            SrpError::TrunkLost,
        ] {
            assert_eq!(srp_error(e.clone()).code, e.code());
        }
        assert_eq!(
            srp_error(SrpError::ClientBacklogged).code,
            "client_backlogged"
        );
    }

    #[tokio::test]
    async fn a_refused_connection_is_server_offline() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let t = DirectTransport::new(&format!("http://{addr}")).unwrap();
        let e = t
            .send(HttpRequest {
                method: "GET".into(),
                path_and_query: "/v1/health".into(),
                headers: vec![],
                body: vec![],
            })
            .await
            .err()
            .unwrap();
        assert_eq!(e.code, "server_offline", "{e}");
    }
}
