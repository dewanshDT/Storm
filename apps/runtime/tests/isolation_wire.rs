//! The host's requests to the isolation helper parse with the helper's own
//! parser (decision 136c): every one the host sends names exactly the
//! fields its operation takes.
#![cfg(target_os = "linux")]

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use storm_runtime::isolation::{Helper, Link};
use storm_runtime_isolation::protocol;

#[derive(Default)]
struct Recording(Mutex<Vec<(Value, bool)>>);

impl Helper for Recording {
    fn call(
        &self,
        request: &Value,
        fd: Option<BorrowedFd<'_>>,
    ) -> io::Result<(Value, Option<OwnedFd>)> {
        self.0.lock().unwrap().push((request.clone(), fd.is_some()));
        let answer = match request["op"].as_str() {
            Some("lease") => json!({"ok": true, "op": "lease", "user": "storm-s001"}),
            Some("start") => json!({"ok": true, "op": "start", "pid": 0}),
            Some("end") => json!({"ok": true, "op": "end", "exit_code": 0, "signal": null}),
            _ => json!({"ok": true}),
        };
        Ok((answer, None))
    }
}

#[test]
fn every_request_the_host_sends_is_one_the_helper_accepts() {
    let rec = Arc::new(Recording::default());
    let link = Link {
        helper: rec.clone(),
        state_dir: "/var/lib/storm-runtime".into(),
    };
    let user = link.lease("ags_A").unwrap();
    link.prepare("ags_A", &user).unwrap();
    let (_r, w) = std::io::pipe().unwrap();
    link.start(
        "ags_A",
        &user,
        "claude-code",
        std::path::Path::new("/var/lib/storm-runtime/workspaces/app"),
        w.as_fd(),
    )
    .unwrap();
    for signal in ["hup", "kill"] {
        link.signal("ags_A", signal).unwrap();
    }
    assert_eq!(link.end("ags_A").unwrap(), (Some(0), None));
    link.release("ags_A", &user).unwrap();
    let sent = rec.0.lock().unwrap().clone();
    let ops: Vec<&str> = sent
        .iter()
        .map(|(r, _)| r["op"].as_str().unwrap())
        .collect();
    assert_eq!(
        ops,
        [
            "lease", "prepare", "start", "signal", "signal", "end", "release"
        ]
    );
    for (request, with_fd) in &sent {
        let parsed = protocol::parse(request.to_string().as_bytes())
            .unwrap_or_else(|e| panic!("{request}: {}", e.code()));
        assert_eq!(parsed.op.name(), request["op"]);
        assert_eq!(*with_fd, parsed.op == protocol::Op::Start, "{request}");
    }
}

#[test]
fn a_refusal_is_an_error_naming_its_code() {
    struct No;
    impl Helper for No {
        fn call(
            &self,
            _: &Value,
            _: Option<BorrowedFd<'_>>,
        ) -> io::Result<(Value, Option<OwnedFd>)> {
            Ok((json!({"ok": false, "error": "pool_exhausted"}), None))
        }
    }
    let link = Link {
        helper: Arc::new(No),
        state_dir: "/var/lib/storm-runtime".into(),
    };
    let e = link.lease("ags_A").unwrap_err();
    assert!(e.to_string().contains("pool_exhausted"), "{e}");
}
