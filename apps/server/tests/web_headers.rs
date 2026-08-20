//! What the web client is served with, driven as a real process.
//!
//! The serving lives in `main.rs`'s `run_serve` — the `ServeDir`, the SPA
//! fallback and the header layers are all assembled there, not in
//! `api::router` — so nothing inside the crate can reach it. This boots the
//! built binary with `--web` and asks it for the two things that matter.
//!
//! **Why this test exists.** `index.html` carries a single-use bootstrap nonce
//! and is correctly `no-store`. The JavaScript it points at had *no*
//! `cache-control` at all, which does not mean "do not cache" — it means the
//! browser applies **heuristic** caching and may reuse the old bundle without
//! revalidating. A deployed release therefore did not reach a returning
//! browser, twice in one afternoon, and both times looked like a missing
//! feature rather than a stale asset.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_storm-server");

/// A served response's status line and headers.
struct Head {
    status: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A minimal HTTP/1.1 GET, so the test needs no HTTP client dependency.
fn get(port: u16, path: &str) -> Head {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connecting");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .expect("writing request");

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("reading response");
    let text = String::from_utf8_lossy(&raw).to_string();
    let mut lines = text.lines();
    let status = lines.next().unwrap_or_default().to_string();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Head { status, headers }
}

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A port nothing is listening on right now.
///
/// `--port 0` would be tidier, but the server logs the address it was *asked*
/// for rather than the one it bound, so with 0 the log says `:0` and there is
/// nothing to connect to. Binding and dropping leaves a small race, which the
/// retrying connect below absorbs.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserving a port");
    listener.local_addr().unwrap().port()
}

/// Boots the server with a web directory, and waits until it answers.
fn serve_with_web(dir: &std::path::Path) -> Server {
    let web = dir.join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(
        web.join("index.html"),
        "<html><head><title>Storm</title></head><body>hi</body></html>",
    )
    .unwrap();
    std::fs::write(web.join("main.dart.js"), "console.log('storm');").unwrap();

    let port = free_port();
    let child = Command::new(BIN)
        .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
        .arg("--vault-root")
        .arg(dir.join("vaults"))
        .arg("--state")
        .arg(dir.join("state"))
        .arg("--web")
        .arg(&web)
        .arg("--token")
        .arg("testtoken")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("starting storm-server");

    let server = Server { child, port };
    // Poll rather than parse a log line: what this test needs to know is
    // "does it answer", which is the question `get` is about to ask anyway.
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("storm-server never accepted a connection on {port}");
}

#[test]
fn a_web_asset_is_revalidated_rather_than_heuristically_cached() {
    let dir = tempdir::TempDir::new("storm-web-headers").unwrap();
    let server = serve_with_web(dir.path());

    let asset = get(server.port, "/main.dart.js");
    assert!(asset.status.contains("200"), "{}", asset.status);

    // **`no-cache` means revalidate, not "never store".** Flutter's output
    // keeps stable filenames across builds, so there is no content hash that
    // would make a long `max-age` safe — and with no header at all the browser
    // is free to reuse the old bundle without asking, which is the bug.
    assert_eq!(
        asset.header("cache-control"),
        Some("no-cache"),
        "a web asset with no cache-control is heuristically cached, so a \
         deploy does not reach a returning browser"
    );

    // The ETag is what makes revalidating cheap — a conditional request and a
    // bodyless 304 rather than the whole bundle.
    assert!(
        asset.header("etag").is_some(),
        "no-cache without a validator would re-download the bundle every time"
    );
}

#[test]
fn the_index_is_never_stored_because_it_carries_a_nonce() {
    // The other half, and the one that was always right: `index.html` carries
    // a single-use, peer-bound bootstrap nonce. A cached copy is that
    // credential with an unbounded lifetime, sitting wherever the cache is.
    let dir = tempdir::TempDir::new("storm-web-index").unwrap();
    let server = serve_with_web(dir.path());

    for path in ["/", "/login", "/settings/server"] {
        let index = get(server.port, path);
        assert!(index.status.contains("200"), "{path}: {}", index.status);
        assert_eq!(
            index.header("cache-control"),
            Some("no-store"),
            "{path} serves the document, which carries a single-use credential"
        );
        assert_eq!(
            index.header("content-type"),
            Some("text/html; charset=utf-8"),
            "{path} is a client route and must get the document, not a 404"
        );
    }
}
