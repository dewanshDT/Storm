//! The real `storm-runtime serve` binary's lifecycle: how it stops (AM37) and
//! how a revocation ends it (AM36). The server is a stand-in that proves its
//! pinned key, so the host gets exactly as far as it would against a real one.
//!
//! **Never pass `--exclusive-account` here.** Its sweep signals every process
//! of the running account, and the account running this suite may be the
//! service account itself (a host's own agent session). The guard is
//! unit-tested in `platform`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use storm_runtime::identity::{HostConfig, HostKey, REVOKED_FILE, challenge_message};

const SERVER_ID: &str = "srv_LIFECYCLE";

const ENROLLED_HOST: &str = "hst_0123456789ABCDEFGHJKMNPQRS";

/// A stand-in server on 127.0.0.1. It answers the identity challenge with
/// `server_key`, accepts any enrollment (as `ENROLLED_HOST`), and refuses
/// every host key (`401`), as a server that revoked this host does. Returns
/// its URL.
fn revoking_server(server_key: HostKey) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let h = h.trim().to_lowercase();
                    if h.is_empty() {
                        break;
                    }
                    if let Some(v) = h.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                let (status, reply) = if path == "/v1/server/challenge" {
                    let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let nonce = req["nonce"].as_str().unwrap();
                    let signature = server_key.sign(&challenge_message(SERVER_ID, nonce));
                    (
                        "200 OK",
                        serde_json::json!({"server_id": SERVER_ID, "signature": signature})
                            .to_string(),
                    )
                } else if path == "/v1/runtime/enroll" {
                    (
                        "200 OK",
                        serde_json::json!({"host_id": ENROLLED_HOST, "server_id": SERVER_ID})
                            .to_string(),
                    )
                } else {
                    ("401 Unauthorized", "{}".to_string())
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{reply}",
                    reply.len()
                );
                if conn.write_all(response.as_bytes()).is_err() {
                    break;
                }
            }
        }
    });
    url
}

/// An enrolled host's state directory, pinned to `server_url` and the server
/// key `server_pubkey`.
fn enrolled_state(dir: &Path, server_url: &str, server_pubkey: &str) {
    let key = HostKey::generate();
    key.save(dir).unwrap();
    HostConfig {
        server_url: server_url.into(),
        server_id: SERVER_ID.into(),
        server_pubkey: server_pubkey.into(),
        host_id: "hst_LIFECYCLE".into(),
        key_id: key.key_id.clone(),
    }
    .save(dir)
    .unwrap();
}

/// `serve` with a config of its own, its output collected line by line.
fn serve(state: &Path) -> (Child, mpsc::Receiver<String>) {
    let roots = state.join("roots");
    std::fs::create_dir_all(&roots).unwrap();
    let config = state.join("runtime.toml");
    std::fs::write(
        &config,
        format!("workspace_roots = [\"{}\"]\n", roots.display()),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_storm-runtime"))
        .args(["serve", "--state"])
        .arg(state)
        .arg("--config")
        .arg(&config)
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    for out in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
    }
    (child, rx)
}

fn wait_for_line(rx: &mpsc::Receiver<String>, needle: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => {
                let hit = line.contains(needle);
                seen.push(line);
                if hit {
                    return seen;
                }
            }
            Err(_) => panic!("never saw {needle:?}; output was {seen:#?}"),
        }
    }
}

fn wait_exit(child: &mut Child, within: Duration) -> ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("serve did not exit within {within:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// SIGTERM (launchd's and systemd's stop) and SIGINT (a terminal's Ctrl-C) are
/// an orderly shutdown, exit 0 — never the default action, which would leave
/// every session's process group running behind it (AM37).
#[test]
fn sigterm_and_sigint_stop_the_host_cleanly() {
    for signal in [rustix::process::Signal::TERM, rustix::process::Signal::INT] {
        let state = tempfile::tempdir().unwrap();
        // A server nobody answers: the host stays up, retrying its link.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        enrolled_state(state.path(), &url, &HostKey::generate().public_key_b64());
        let (mut child, rx) = serve(state.path());
        wait_for_line(&rx, "link down");
        let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
        rustix::process::kill_process(pid, signal).unwrap();
        let status = wait_exit(&mut child, Duration::from_secs(10));
        assert_eq!(status.code(), Some(0), "{signal:?} gave {status:?}");
        assert!(
            HostConfig::path(state.path()).exists(),
            "a stop is not a revocation"
        );
    }
}

/// AM36: a refused key ends the host with exit 3 and moves `host.json` aside,
/// which is what keeps launchd (no `RestartPreventExitStatus`) from starting
/// it again. The next start says the host was revoked.
#[test]
fn a_revoked_host_exits_3_and_is_no_longer_enrolled() {
    let state = tempfile::tempdir().unwrap();
    let server_key = HostKey::generate();
    let pinned = server_key.public_key_b64();
    let url = revoking_server(server_key);
    enrolled_state(state.path(), &url, &pinned);
    let before = std::fs::read(HostConfig::path(state.path())).unwrap();

    let (mut child, _rx) = serve(state.path());
    let status = wait_exit(&mut child, Duration::from_secs(20));
    assert_eq!(status.code(), Some(3), "{status:?}");
    assert!(
        !HostConfig::path(state.path()).exists(),
        "host.json survived"
    );
    assert_eq!(
        std::fs::read(state.path().join(REVOKED_FILE)).unwrap(),
        before,
        "the old enrollment is kept as host.json.revoked"
    );

    // Started again (a service manager that ignored the exit status), it
    // refuses at once, saying why, and does not exit 3 again.
    let (mut again, rx) = serve(state.path());
    let status = wait_exit(&mut again, Duration::from_secs(10));
    assert_ne!(status.code(), Some(0));
    let said: Vec<String> = rx.try_iter().collect();
    assert!(
        said.iter().any(|l| l.contains("revoked")),
        "no mention of the revocation: {said:#?}"
    );
}

/// AM35 + AM37: under launchd, `host.json` appearing starts the host, whose
/// account sweep would end an `enroll` running as that account before it had
/// said anything. So `enroll` announces first, and `host.json` is its last
/// write. Also AM36: after a revocation it enrolls again without `--force`,
/// and the old `host.json.revoked` goes.
#[tokio::test]
async fn enrollment_is_announced_before_host_json_exists() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let state = tempfile::tempdir().unwrap();
    let server_key = HostKey::generate();
    let pinned = server_key.public_key_b64();
    let url = revoking_server(server_key);
    std::fs::write(state.path().join(REVOKED_FILE), b"{}").unwrap();
    let enrollment = format!(
        "storm-enroll:v1:{url}:{SERVER_ID}:{pinned}:sen_0123456789ABCDEFGHJKMNPQRS.{}",
        "A".repeat(43)
    );
    let host_json = HostConfig::path(state.path());
    let mut announced = false;
    let config = storm_runtime::client::enroll(state.path(), &enrollment, "mac", false, |c| {
        assert_eq!(c.host_id, ENROLLED_HOST);
        assert!(
            !host_json.exists(),
            "host.json was written before the announcement"
        );
        announced = true;
    })
    .await
    .unwrap();
    assert!(announced);
    assert_eq!(HostConfig::load(state.path()).unwrap(), config);
    assert!(
        !state.path().join(REVOKED_FILE).exists(),
        "the revoked marker stayed"
    );
}
