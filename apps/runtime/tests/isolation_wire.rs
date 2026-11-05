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
            Some("lease") => json!({"ok": true, "op": "lease", "user": "storm-s001", "uid": 2001}),
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
        homes: storm_runtime::isolation::HOMES.into(),
        status: Arc::new(storm_runtime::isolation::Status::new(true, true)),
    };
    link.ready().unwrap();
    let (user, uid) = link.lease("ags_A").unwrap();
    assert_eq!(uid, 2001);
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
    // A self-test probe (136j): the helper's own provider, in its own home.
    let home = std::path::Path::new(storm_runtime::isolation::HOMES).join("ags_P");
    link.start(
        "ags_P",
        &user,
        storm_runtime::isolation::PROBE,
        &home,
        w.as_fd(),
    )
    .unwrap();
    let sent = rec.0.lock().unwrap().clone();
    let ops: Vec<&str> = sent
        .iter()
        .map(|(r, _)| r["op"].as_str().unwrap())
        .collect();
    assert_eq!(
        ops,
        [
            "self_test",
            "lease",
            "prepare",
            "start",
            "signal",
            "signal",
            "end",
            "release",
            "start"
        ]
    );
    for (request, with_fd) in &sent {
        let parsed = protocol::parse(request.to_string().as_bytes())
            .unwrap_or_else(|e| panic!("{request}: {}", e.code()));
        assert_eq!(parsed.op.name(), request["op"]);
        assert_eq!(*with_fd, parsed.op == protocol::Op::Start, "{request}");
    }
}

/// 136g's job, as the host asks for it.
#[test]
fn an_agent_job_is_one_the_helper_accepts() {
    let rec = Arc::new(Recording::default());
    let link = Link {
        helper: rec.clone(),
        state_dir: "/var/lib/storm-runtime".into(),
        homes: storm_runtime::isolation::HOMES.into(),
        status: Arc::new(storm_runtime::isolation::Status::new(true, true)),
    };
    for action in ["install", "update", "remove"] {
        link.agent_job("opencode", action).unwrap().unwrap();
    }
    for (request, with_fd) in rec.0.lock().unwrap().iter() {
        let parsed = protocol::parse(request.to_string().as_bytes())
            .unwrap_or_else(|e| panic!("{request}: {}", e.code()));
        assert_eq!(parsed.op, protocol::Op::AgentJob);
        assert!(!with_fd);
    }
    assert!(
        storm_runtime_isolation::agents::ACTIONS
            .contains(&storm_runtime::install::Op::Update.name())
    );
}

/// 137-1: the host's `child_create` is one the helper accepts, and the
/// helper's answer is one the host reads.
#[test]
fn a_child_workspace_is_asked_for_and_answered_in_one_shape() {
    use storm_runtime_isolation::children::{ChildMode, ChildRecord, Stage, reply};
    struct Child(Mutex<Vec<Value>>);
    impl Helper for Child {
        fn call(
            &self,
            request: &Value,
            _: Option<BorrowedFd<'_>>,
        ) -> io::Result<(Value, Option<OwnedFd>)> {
            self.0.lock().unwrap().push(request.clone());
            let record = ChildRecord {
                child: "ags_C".into(),
                parent: "ags_P".into(),
                parent_workspace: "/var/lib/storm-runtime/workspaces/app".into(),
                mode: ChildMode::Clone,
                stage: Stage::Ready,
                commit: Some("c".repeat(40)),
                branch: Some("main".into()),
                submodules: true,
                large_files: false,
                applied: None,
                resumed: false,
            };
            let workspace =
                std::path::Path::new("/var/lib/storm-runtime-isolation/children/ags_C/workspace");
            let line = protocol::ok_with(protocol::Op::ChildCreate, reply(&record, workspace));
            Ok((serde_json::from_str(&line).unwrap(), None))
        }
    }
    let rec = Arc::new(Child(Mutex::new(Vec::new())));
    let link = Link {
        helper: rec.clone(),
        state_dir: "/var/lib/storm-runtime".into(),
        homes: storm_runtime::isolation::HOMES.into(),
        status: Arc::new(storm_runtime::isolation::Status::new(true, true)),
    };
    let made = link
        .child_create(
            "ags_P",
            "ags_C",
            std::path::Path::new("/var/lib/storm-runtime/workspaces/app"),
        )
        .unwrap();
    assert_eq!(
        made,
        storm_runtime::isolation::ChildWorkspace {
            mode: storm_runtime::isolation::ChildMode::Clone,
            path: "/var/lib/storm-runtime-isolation/children/ags_C/workspace".into(),
            commit: Some("c".repeat(40)),
            branch: Some("main".into()),
            submodules: true,
            large_files: false,
        }
    );
    let sent = rec.0.lock().unwrap().clone();
    let parsed = protocol::parse(sent[0].to_string().as_bytes()).unwrap();
    assert_eq!(parsed.op, protocol::Op::ChildCreate);
    assert_eq!(parsed.get("parent"), "ags_P");
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
        homes: storm_runtime::isolation::HOMES.into(),
        status: Arc::new(storm_runtime::isolation::Status::new(true, true)),
    };
    let e = link.lease("ags_A").unwrap_err();
    assert!(e.to_string().contains("pool_exhausted"), "{e}");
    assert_eq!(
        storm_runtime::isolation::refusal(&e),
        Some("pool_exhausted")
    );
    // A refusal is the helper answering: isolation stays as it was.
    assert_eq!(storm_runtime::isolation::unavailable_because(&e), None);
}

/// 136i: a lease that names no uid, or root's, is no lease the host can
/// bind a session's connections to.
#[test]
fn a_lease_without_a_usable_uid_is_refused() {
    struct Leases(Value);
    impl Helper for Leases {
        fn call(
            &self,
            _: &Value,
            _: Option<BorrowedFd<'_>>,
        ) -> io::Result<(Value, Option<OwnedFd>)> {
            Ok((self.0.clone(), None))
        }
    }
    for answer in [
        json!({"ok": true, "user": "storm-s001"}),
        json!({"ok": true, "user": "storm-s001", "uid": 0}),
        json!({"ok": true, "user": "storm-s001", "uid": "2001"}),
        json!({"ok": true, "user": "storm-s001", "uid": -1}),
    ] {
        let link = Link {
            helper: Arc::new(Leases(answer.clone())),
            state_dir: "/var/lib/storm-runtime".into(),
            homes: storm_runtime::isolation::HOMES.into(),
            status: Arc::new(storm_runtime::isolation::Status::new(true, true)),
        };
        assert!(link.lease("ags_A").is_err(), "{answer}");
    }
}

/// 137-3, 145e: list, export, Apply, delete and park are each one the helper accepts,
/// with exactly their fields, and the host hands on the helper's answer
/// without its `ok` and `op`.
#[test]
fn the_child_operations_the_host_sends_are_ones_the_helper_accepts() {
    let rec = Arc::new(Recording::default());
    let link = Link {
        helper: rec.clone(),
        state_dir: "/var/lib/storm-runtime".into(),
        homes: storm_runtime::isolation::HOMES.into(),
        status: Arc::new(storm_runtime::isolation::Status::new(true, true)),
    };
    assert_eq!(link.child_list().unwrap(), json!({}));
    link.child_export("ags_C").unwrap();
    link.child_import("ags_C", "ags_P", "storm/ags_C").unwrap();
    link.child_import("ags_C", "ags_P", "app-ags_C").unwrap();
    link.child_dispose("ags_C").unwrap();
    link.child_park("ags_C").unwrap();
    let ops: Vec<protocol::Op> = rec
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(request, with_fd)| {
            assert!(!with_fd);
            protocol::parse(request.to_string().as_bytes())
                .unwrap_or_else(|e| panic!("{request}: {}", e.code()))
                .op
        })
        .collect();
    use protocol::Op::*;
    assert_eq!(
        ops,
        [
            ChildList,
            ChildExport,
            ChildImport,
            ChildImport,
            ChildDispose,
            ChildPark
        ]
    );
}
