//! `storm-runtime mcp-connect` as a client starts it: the real binary, over
//! real HTTP, against an in-process fake server (decision 105c).

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const SERVER: &str = "srv_pinned";
const BIN: &str = env!("CARGO_BIN_EXE_storm-runtime");

struct Request {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

type Log = Arc<Mutex<Vec<Request>>>;

fn pinned() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

/// A fake storm-server holding `signing`: the identity challenge and a
/// minimal `/mcp`.
async fn fake_server(signing: SigningKey) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let seen = log.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(connection(stream, signing.clone(), seen.clone()));
        }
    });
    (addr, log)
}

async fn connection(stream: TcpStream, signing: SigningKey, log: Log) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).await.unwrap();
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let (n, v) = h.split_once(':').unwrap();
            headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
        let length: usize = headers
            .iter()
            .find(|(n, _)| n == "content-length")
            .map(|(_, v)| v.parse().unwrap())
            .unwrap_or(0);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let (status, answer) = respond(&path, &body, &signing);
        log.lock().unwrap().push(Request {
            path,
            headers,
            body,
        });
        let answer = answer.map(|a| a.to_string()).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
            answer.len()
        );
        let stream = reader.get_mut();
        if stream.write_all(head.as_bytes()).await.is_err()
            || stream.write_all(answer.as_bytes()).await.is_err()
        {
            return;
        }
    }
}

fn respond(path: &str, body: &Value, signing: &SigningKey) -> (u16, Option<Value>) {
    if path == "/v1/server/challenge" {
        let nonce = body["nonce"].as_str().unwrap();
        let sig = signing.sign(format!("storm-challenge:v1:{SERVER}:{nonce}").as_bytes());
        return (
            200,
            Some(
                json!({"server_id": SERVER, "signature": BASE64URL_NOPAD.encode(&sig.to_bytes())}),
            ),
        );
    }
    let id = body["id"].clone();
    match body["method"].as_str() {
        Some("initialize") => (
            200,
            Some(json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": "2025-06-18"}})),
        ),
        Some(m) if m.starts_with("notifications/") => (202, None),
        _ => (
            200,
            Some(json!({"jsonrpc": "2.0", "id": id, "result": {"tools": []}})),
        ),
    }
}

/// Runs the binary with `args`, feeds it `lines`, closes stdin, and returns
/// its exit status, its stdout as messages, and its stderr.
async fn run(
    args: &[&str],
    env_key: Option<&str>,
    lines: &[Value],
) -> (std::process::ExitStatus, Vec<Value>, String) {
    // The relay cache goes here, never into the person's own state.
    let state = tempfile::tempdir().unwrap();
    let mut cmd = std::process::Command::new(BIN);
    cmd.arg("mcp-connect")
        .args(args)
        .env_remove("STORM_MCP_KEY")
        .env("XDG_STATE_HOME", state.path())
        .env("HOME", state.path())
        .env("LOCALAPPDATA", state.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(k) = env_key {
        cmd.env("STORM_MCP_KEY", k);
    }
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for l in lines {
        std::io::Write::write_all(&mut stdin, format!("{l}\n").as_bytes()).unwrap();
    }
    drop(stdin);
    // Waited for off the runtime, which is serving the fake server.
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::task::spawn_blocking(move || child.wait_with_output()),
    )
    .await
    .expect("mcp-connect did not exit when stdin closed")
    .unwrap()
    .unwrap();
    let out = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    (
        output.status,
        out,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn pk() -> String {
    BASE64URL_NOPAD.encode(pinned().verifying_key().as_bytes())
}

fn conversation() -> Vec<Value> {
    vec![
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "capabilities": {}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    ]
}

#[test]
fn there_is_no_key_argument() {
    let help = std::process::Command::new(BIN)
        .args(["mcp-connect", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--key-file"), "{help}");
    assert!(
        !help
            .split_whitespace()
            .any(|w| w.starts_with("--key") && !w.starts_with("--key-file")),
        "{help}"
    );
    let refused = std::process::Command::new(BIN)
        .args([
            "mcp-connect",
            "--server",
            SERVER,
            "--pk",
            &pk(),
            "--key",
            "stk_x",
        ])
        .output()
        .unwrap();
    assert!(!refused.status.success());
}

#[tokio::test]
async fn a_pinned_key_that_starts_with_a_hyphen_is_a_value_not_a_flag() {
    let signing = (0u8..=255)
        .map(|n| SigningKey::from_bytes(&[n; 32]))
        .find(|k| {
            BASE64URL_NOPAD
                .encode(k.verifying_key().as_bytes())
                .starts_with('-')
        })
        .expect("a seed whose key starts with a hyphen");
    let pk = BASE64URL_NOPAD.encode(signing.verifying_key().as_bytes());
    let (addr, _log) = fake_server(signing).await;
    let (status, out, stderr) = run(
        &["--server", SERVER, "--pk", &pk, "--addr", &addr],
        Some("stk_x"),
        &conversation(),
    )
    .await;
    assert!(status.success(), "{stderr}");
    assert_eq!(out.len(), 2, "{out:?} {stderr}");
    assert_eq!(out[1]["id"], 1);
}

#[tokio::test]
async fn the_key_file_works_and_is_sent_only_after_the_challenge() {
    let (addr, log) = fake_server(pinned()).await;
    let dir = tempfile::tempdir().unwrap();
    let key_file = dir.path().join("key");
    std::fs::write(&key_file, "stk_fromthefile\n").unwrap();
    let pk = pk();
    let (status, out, stderr) = run(
        &[
            "--server",
            SERVER,
            "--pk",
            &pk,
            "--addr",
            &addr,
            "--key-file",
            key_file.to_str().unwrap(),
        ],
        None,
        &conversation(),
    )
    .await;
    assert!(status.success(), "{stderr}");
    assert_eq!(out.len(), 2, "the notification was answered: {out:?}");
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(out[1]["id"], 1);
    let log = log.lock().unwrap();
    assert_eq!(log[0].path, "/v1/server/challenge");
    assert!(log[0].header("authorization").is_none());
    let mcp: Vec<&Request> = log.iter().filter(|r| r.path == "/mcp").collect();
    assert_eq!(mcp.len(), 3);
    assert_eq!(mcp[1].body["method"], "notifications/initialized");
    for r in &mcp {
        assert_eq!(r.header("authorization"), Some("Bearer stk_fromthefile"));
    }
    assert_eq!(mcp[2].header("mcp-protocol-version"), Some("2025-06-18"));
    assert!(!stderr.contains("stk_fromthefile"), "{stderr}");
}

#[tokio::test]
async fn an_impostor_is_identity_failed_and_never_sees_the_key() {
    let (addr, log) = fake_server(SigningKey::from_bytes(&[9; 32])).await;
    let pk = pk();
    let (status, out, stderr) = run(
        &["--server", SERVER, "--pk", &pk, "--addr", &addr],
        Some("stk_secret"),
        &conversation(),
    )
    .await;
    assert!(status.success(), "{stderr}");
    assert_eq!(out.len(), 2, "{out:?}");
    for m in &out {
        assert_eq!(m["error"]["data"]["storm_error"], "identity_failed", "{m}");
    }
    let log = log.lock().unwrap();
    assert!(log.iter().all(|r| r.path == "/v1/server/challenge"));
    assert!(log.iter().all(|r| r.header("authorization").is_none()));
    assert!(!stderr.contains("stk_secret"), "{stderr}");
}

#[tokio::test]
async fn a_server_that_is_down_is_server_offline() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let pk = pk();
    let (_, out, _) = run(
        &["--server", SERVER, "--pk", &pk, "--addr", &addr],
        Some("stk_secret"),
        &conversation()[..1],
    )
    .await;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["error"]["data"]["storm_error"], "server_offline");
}

#[tokio::test]
async fn with_neither_an_address_nor_a_relay_it_refuses_to_start() {
    let pk = pk();
    let (status, out, stderr) = run(
        &["--server", SERVER, "--pk", &pk],
        Some("stk_secret"),
        &conversation(),
    )
    .await;
    assert!(!status.success());
    assert!(out.is_empty());
    assert!(stderr.contains("--relay"), "{stderr}");
}

#[tokio::test]
async fn without_a_key_it_sends_nothing() {
    let (addr, log) = fake_server(pinned()).await;
    let pk = pk();
    let (status, out, stderr) = run(
        &["--server", SERVER, "--pk", &pk, "--addr", &addr],
        None,
        &conversation(),
    )
    .await;
    assert!(!status.success());
    assert!(out.is_empty());
    assert!(stderr.contains("STORM_MCP_KEY"), "{stderr}");
    assert!(log.lock().unwrap().is_empty());
}
