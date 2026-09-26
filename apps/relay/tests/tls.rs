//! The relay terminating TLS itself (decision 71), driven over real sockets
//! with a real certificate and a real rustls client.
//!
//! The reason TLS lives in the relay rather than in a proxy is that the socket
//! peer must stay the client's (§5.2), so
//! `relay_peer_ip_over_tls_is_the_socket_peer` is the property this file
//! exists for. The rest prove that TLS did not cost the relay anything a proxy
//! would have given it for free: a stalled handshake blocks nobody, and a
//! renewed certificate goes live without a restart.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use rustls_pki_types::pem::PemObject;
use serde_json::{Value, json};
use storm_relay::tls::CertStore;
use storm_relay::{Config, Relay};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const SERVER_ID: &str = "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV";

// ------------------------------------------------------------------ fixtures

/// A self-signed certificate for `localhost`, as the PEM files an operator
/// would point the relay at.
struct Cert {
    cert_pem: String,
    key_pem: String,
}

fn make_cert() -> Cert {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    Cert {
        cert_pem: cert.cert.pem(),
        key_pem: cert.key_pair.serialize_pem(),
    }
}

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "storm-relay-tls-{}",
            storm_relay::state::new_nonce()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Writes the pair where the relay reads it, as a renewal hook would.
    fn install(&self, cert: &Cert) -> (PathBuf, PathBuf) {
        let (c, k) = (self.0.join("fullchain.pem"), self.0.join("privkey.pem"));
        std::fs::write(&c, &cert.cert_pem).unwrap();
        std::fs::write(&k, &cert.key_pem).unwrap();
        (c, k)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    addr: std::net::SocketAddr,
    certs: Arc<CertStore>,
    _dir: Dir,
}

async fn start(cert: &Cert, handshake_timeout: Duration) -> Harness {
    let dir = Dir::new();
    let (c, k) = dir.install(cert);
    let certs = Arc::new(CertStore::load(&c, &k).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut config = Config::new("127.0.0.1:0".parse().unwrap(), "wss://localhost");
    config.hello_wait = Duration::from_millis(500);
    let relay = Arc::new(Relay::new(config));
    let serving = certs.clone();
    tokio::spawn(async move {
        let _ = storm_relay::serve_tls_with(listener, relay, serving, handshake_timeout).await;
    });
    Harness {
        addr,
        certs,
        _dir: dir,
    }
}

/// A client that trusts exactly `cert`, so a test can tell which certificate
/// the relay presented by which client succeeds.
fn trusting(cert: &Cert) -> TlsConnector {
    let mut roots = RootCertStore::empty();
    for der in rustls_pki_types::CertificateDer::pem_slice_iter(cert.cert_pem.as_bytes()) {
        roots.add(der.unwrap()).unwrap();
    }
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

type Ws = WebSocketStream<TlsStream<TcpStream>>;

async fn dial(
    h: &Harness,
    tls: &TlsConnector,
    path: &str,
) -> std::io::Result<(Ws, std::net::SocketAddr)> {
    let tcp = TcpStream::connect(h.addr).await?;
    let local = tcp.local_addr()?;
    let name = rustls_pki_types::ServerName::try_from("localhost").unwrap();
    let stream = tls.connect(name, tcp).await?;
    let (ws, _) = tokio_tungstenite::client_async(format!("wss://localhost{path}"), stream)
        .await
        .map_err(std::io::Error::other)?;
    Ok((ws, local))
}

async fn send(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn recv(ws: &mut Ws) -> Value {
    loop {
        match ws.next().await.expect("a frame").unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => continue,
            other => panic!("unexpected frame {other:?}"),
        }
    }
}

/// Registers `SERVER_ID` over the given connection. Spelled out rather than
/// imported, as the other suites do: the wire is what is under test.
async fn register(ws: &mut Ws) -> Value {
    let key = SigningKey::from_bytes(&[1; 32]);
    send(
        ws,
        json!({
            "v": 1, "type": "REGISTER_SERVER", "server_id": SERVER_ID,
            "pubkey": BASE64URL_NOPAD.encode(key.verifying_key().as_bytes()),
        }),
    )
    .await;
    let challenge = recv(ws).await;
    let nonce = challenge["nonce"].as_str().unwrap().to_string();
    let message = format!("storm-relay-auth:v1:{SERVER_ID}:{nonce}");
    send(
        ws,
        json!({
            "v": 1, "type": "CHALLENGE_RESPONSE",
            "sig": BASE64URL_NOPAD.encode(&key.sign(message.as_bytes()).to_bytes()),
        }),
    )
    .await;
    recv(ws).await
}

// --------------------------------------------------------------------- tests

#[tokio::test]
async fn a_server_registers_over_wss() {
    let cert = make_cert();
    let h = start(&cert, Duration::from_secs(5)).await;
    let (mut server, _) = dial(&h, &trusting(&cert), "/register").await.unwrap();
    let reply = register(&mut server).await;
    assert_eq!(reply["type"], "REGISTERED", "{reply}");
}

#[tokio::test]
async fn relay_peer_ip_over_tls_is_the_socket_peer() {
    // What TLS in the relay buys over a proxy. The origin's login limiter keys
    // on this field, so it must be the client's socket, not a middlebox's.
    let cert = make_cert();
    let h = start(&cert, Duration::from_secs(5)).await;
    let tls = trusting(&cert);
    let (mut server, _) = dial(&h, &tls, "/register").await.unwrap();
    assert_eq!(register(&mut server).await["type"], "REGISTERED");

    let (mut client, client_addr) = dial(&h, &tls, &format!("/connect/{SERVER_ID}"))
        .await
        .unwrap();
    send(
        &mut client,
        json!({ "v": 1, "type": "HELLO", "server_id": SERVER_ID }),
    )
    .await;
    assert_eq!(recv(&mut client).await["type"], "READY");
    send(
        &mut client,
        json!({ "v": 1, "type": "OPEN_STREAM", "attempt_id": "a1" }),
    )
    .await;
    let stream_id = recv(&mut client).await["stream_id"].as_u64().unwrap();

    let open = recv(&mut server).await;
    assert_eq!(open["type"], "STREAM_OPEN", "{open}");
    send(
        &mut server,
        json!({ "v": 1, "type": "STREAM_ACK", "stream_id": stream_id }),
    )
    .await;

    send(
        &mut client,
        json!({
            "v": 1, "type": "HTTP_REQUEST_HEAD", "stream_id": stream_id,
            "method": "GET", "path": "/v1/vaults", "headers": {},
        }),
    )
    .await;
    let head = recv(&mut server).await;
    assert_eq!(head["type"], "HTTP_REQUEST_HEAD", "{head}");
    assert_eq!(
        head["relay_peer_ip"],
        client_addr.ip().to_string(),
        "{head}"
    );
}

#[tokio::test]
async fn a_stalled_handshake_does_not_block_other_connections() {
    // A socket that connects and says nothing. Handshaking inside accept()
    // would leave every connection behind it waiting on this one.
    let cert = make_cert();
    let h = start(&cert, Duration::from_secs(30)).await;
    let _stalled = TcpStream::connect(h.addr).await.unwrap();

    let dialled = tokio::time::timeout(
        Duration::from_secs(2),
        dial(&h, &trusting(&cert), "/register"),
    )
    .await
    .expect("a stalled handshake blocked the next connection");
    let (mut server, _) = dialled.unwrap();
    assert_eq!(register(&mut server).await["type"], "REGISTERED");
}

#[tokio::test]
async fn a_stalled_handshake_is_cut_off_at_the_deadline() {
    let cert = make_cert();
    let h = start(&cert, Duration::from_millis(200)).await;
    let mut stalled = TcpStream::connect(h.addr).await.unwrap();
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), stalled.read(&mut buf))
        .await
        .expect("the relay held a silent socket past its handshake deadline");
    // EOF or a reset: either way the relay let go of it.
    assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
}

#[tokio::test]
async fn a_plaintext_client_cannot_speak_to_a_tls_port() {
    let cert = make_cert();
    let h = start(&cert, Duration::from_secs(1)).await;
    let url = format!("ws://{}/register", h.addr);
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async(url),
    )
    .await;
    assert!(
        matches!(result, Ok(Err(_))),
        "a plain ws:// client got through"
    );
}

#[tokio::test]
async fn a_reload_swaps_the_certificate_without_a_restart() {
    let (old, new) = (make_cert(), make_cert());
    let h = start(&old, Duration::from_secs(5)).await;
    assert!(dial(&h, &trusting(&old), "/register").await.is_ok());

    // A renewal hook writes the new pair over the old and sends SIGHUP.
    h._dir.install(&new);
    h.certs.reload().unwrap();

    assert!(dial(&h, &trusting(&new), "/register").await.is_ok());
    assert!(
        dial(&h, &trusting(&old), "/register").await.is_err(),
        "the relay is still presenting the old certificate"
    );
}

#[tokio::test]
async fn a_reload_that_fails_keeps_serving_the_old_certificate() {
    // Half-written renewal files must cost a log line, never the relay.
    let cert = make_cert();
    let h = start(&cert, Duration::from_secs(5)).await;
    std::fs::write(
        h._dir.0.join("fullchain.pem"),
        "-----BEGIN CERTIFICATE-----\ntrunc",
    )
    .unwrap();
    assert!(h.certs.reload().is_err());
    assert!(dial(&h, &trusting(&cert), "/register").await.is_ok());
}

#[test]
fn a_key_that_does_not_belong_to_the_certificate_is_refused_at_load() {
    // Otherwise the relay would start, and fail every handshake.
    let (a, b) = (make_cert(), make_cert());
    let dir = Dir::new();
    let (cert_path, _) = dir.install(&a);
    let wrong_key = dir.0.join("other.pem");
    std::fs::write(&wrong_key, &b.key_pem).unwrap();
    let err = CertStore::load(&cert_path, Path::new(&wrong_key)).unwrap_err();
    assert!(format!("{err:#}").contains("does not belong"), "{err:#}");
}
