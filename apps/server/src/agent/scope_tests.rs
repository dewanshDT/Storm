//! Decision 134-4: the scope rows of 134's checks table, the wait's bound
//! and the result's outcomes, at the manager. The tools over the gateway
//! are tested in `api.rs`.

use std::time::{Duration, Instant};

use super::scope::{MAX_WAIT_IDS, wait_bound, wait_cap};
use super::store::SessionRecord;
use super::store::spawn::{MAX_DETAILS_BYTES, MAX_SUMMARY_BYTES, RESULT_OUTCOMES, SpawnResult};
use super::{AgentError, AgentManager, Hello, Launch, LaunchMeta, McpGrant};

const VERIFIED: &str = r#"{"state": "verified", "mechanism": "so_peerpidfd", "reason": null}"#;

struct Fixture {
    m: AgentManager,
    _dir: tempdir::TempDir,
    _link: tokio::sync::mpsc::UnboundedReceiver<super::Envelope>,
}

fn fixture() -> Fixture {
    let dir = tempdir::TempDir::new("storm-scope").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (_, link) = m.connect_host("hst_A");
    let hello: Hello = serde_json::from_value(serde_json::json!({
        "providers": [{"id": "claude-code", "kind": "cli", "interactions": ["terminal"], "available": true}],
        "workspaces": ["storm", "other"],
        "max_sessions": 0,
        "mcp_bridge": true,
        "identity": serde_json::from_str::<serde_json::Value>(VERIFIED).unwrap(),
    }))
    .unwrap();
    m.hello("hst_A", hello).unwrap();
    m.verify_isolation_for_tests("hst_A");
    Fixture {
        m,
        _dir: dir,
        _link: link,
    }
}

impl Fixture {
    fn launch(&self, workspace: &str) -> SessionRecord {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": "claude-code",
            "terminal": {"cols": 80, "rows": 24},
        }))
        .unwrap();
        let grant = McpGrant {
            id: "storm".into(),
            slug: "storm".into(),
            plugin_origin: None,
        };
        let (r, ..) = self
            .m
            .launch("usr_A", launch, LaunchMeta::default(), vec![grant])
            .unwrap();
        self.report(&r.id, "running");
        r
    }

    fn report(&self, id: &str, status: &str) {
        let report =
            serde_json::from_value(serde_json::json!({"status": status, "exit_code": 0})).unwrap();
        self.m.host_status("hst_A", id, &report).unwrap();
    }

    fn parent(&self, workspace: &str) -> SessionRecord {
        let p = self.launch(workspace);
        self.m
            .set_spawn(&p.id, true, Some("dev_A"))
            .unwrap()
            .unwrap();
        p
    }

    fn child(&self, parent: &str) -> SessionRecord {
        self.m.spawn_child_for_tests(parent, "a task")
    }

    /// Two trees: `root` → `child` → `grandchild`, plus `sibling` under
    /// `root`; and `other` → `stranger`.
    fn trees(&self) -> Trees {
        let root = self.parent("storm");
        let child = self.child(&root.id);
        // Its workspace is its parent's until 137-1, so WS-9 would refuse
        // the switch: written as a person's enable is.
        self.m
            .store
            .lock()
            .unwrap()
            .set_spawn(&child.id, true, Some("dev_A"), "2026-10-25T10:00:00Z")
            .unwrap();
        let grandchild = self.child(&child.id);
        let sibling = self.child(&root.id);
        let other = self.parent("other");
        let stranger = self.child(&other.id);
        Trees {
            root: root.id,
            child: child.id,
            grandchild: grandchild.id,
            sibling: sibling.id,
            other: other.id,
            stranger: stranger.id,
        }
    }
}

struct Trees {
    root: String,
    child: String,
    grandchild: String,
    sibling: String,
    other: String,
    stranger: String,
}

fn not_found<T: std::fmt::Debug>(r: Result<T, AgentError>) -> bool {
    matches!(r, Err(AgentError::NotFound(_)))
}

#[test]
fn status_reaches_descendants_only() {
    let f = fixture();
    let t = f.trees();
    assert_eq!(f.m.descendant(&t.root, &t.child).unwrap().id, t.child);
    assert_eq!(
        f.m.descendant(&t.root, &t.grandchild).unwrap().id,
        t.grandchild
    );
    assert_eq!(
        f.m.descendant(&t.child, &t.grandchild).unwrap().id,
        t.grandchild
    );
    for (caller, target) in [
        (&t.root, &t.root),        // itself
        (&t.child, &t.root),       // an ancestor
        (&t.grandchild, &t.child), // its parent
        (&t.child, &t.sibling),    // a sibling
        (&t.root, &t.other),       // another tree's root
        (&t.root, &t.stranger),    // another tree's child
        (&t.other, &t.child),      // the other way
        (&t.root, &"ags_NOPE".to_string()),
    ] {
        assert!(
            not_found(f.m.descendant(caller, target)),
            "{caller} saw {target}"
        );
    }
}

#[test]
fn an_ended_descendant_is_still_seen_and_a_dismissed_one_is_gone() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report(&c.id, "completed");
    assert_eq!(f.m.descendant(&p.id, &c.id).unwrap().status, "completed");
    f.m.dismiss(&c.id).unwrap();
    assert!(not_found(f.m.descendant(&p.id, &c.id)));
}

#[test]
fn list_children_is_the_callers_own_children_only() {
    let f = fixture();
    let t = f.trees();
    let ids = |caller: &str| -> Vec<String> {
        f.m.children_of(caller)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    };
    assert_eq!(ids(&t.root), vec![t.child.clone(), t.sibling.clone()]);
    assert_eq!(ids(&t.child), vec![t.grandchild.clone()]);
    assert_eq!(ids(&t.other), vec![t.stranger.clone()]);
    assert!(ids(&t.grandchild).is_empty());
}

#[test]
fn a_resumed_parent_does_not_reach_its_predecessors_children() {
    // C3's read-only results for a resumed parent are 134-7's; status and
    // wait stay out of scope (LT-9).
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report(&p.id, "completed");
    let resumed = f.launch("other");
    let mut r = f.m.get(&resumed.id).unwrap();
    r.resumed_from = Some(p.id.clone());
    f.m.store.lock().unwrap().update(&r).unwrap();
    assert!(not_found(f.m.descendant(&resumed.id, &c.id)));
    assert!(f.m.children_of(&resumed.id).unwrap().is_empty());
}

#[tokio::test]
async fn wait_reaches_descendants_only_and_refuses_the_whole_call() {
    let f = fixture();
    let t = f.trees();
    let yes = || true;
    for outsider in [&t.root, &t.stranger, &t.other, &"ags_NOPE".to_string()] {
        let r =
            f.m.wait_agents(
                &t.child,
                &[t.grandchild.clone(), outsider.clone()],
                Duration::from_millis(50),
                &yes,
            )
            .await;
        assert!(not_found(r), "waited on {outsider}");
    }
    let r =
        f.m.wait_agents(&t.root, &[], Duration::from_millis(50), &yes)
            .await;
    assert!(matches!(r, Err(AgentError::BadRequest(_))));
    let many = vec![t.child.clone(); MAX_WAIT_IDS + 1];
    let r =
        f.m.wait_agents(&t.root, &many, Duration::from_millis(50), &yes)
            .await;
    assert!(matches!(r, Err(AgentError::BadRequest(_))));
}

#[test]
fn the_wait_is_bounded_below_every_adapters_tool_call_timeout() {
    let gateway = crate::gateway::session::CALL_TIMEOUT;
    // Codex's `tool_timeout_sec` and the MCP SDK's request default.
    let adapter = Duration::from_secs(60);
    for provider in ["claude-code", "opencode", "codex"] {
        let cap = wait_cap(provider);
        assert!(cap > Duration::ZERO, "{provider}");
        assert!(cap < gateway && cap < adapter, "{provider}: {cap:?}");
        assert_eq!(wait_bound(provider, None), cap);
        assert_eq!(
            wait_bound(provider, Some(Duration::from_secs(3600))),
            cap,
            "a long ask is capped"
        );
        assert_eq!(
            wait_bound(provider, Some(Duration::from_secs(2))),
            Duration::from_secs(2)
        );
    }
    for provider in ["shell", "cursor", "command-code"] {
        assert_eq!(
            wait_cap(provider),
            Duration::ZERO,
            "{provider} cannot spawn"
        );
    }
}

#[tokio::test]
async fn a_wait_returns_at_its_bound_when_nothing_settles() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let started = Instant::now();
    let (seen, timed_out) =
        f.m.wait_agents(
            &p.id,
            std::slice::from_ref(&c.id),
            Duration::from_millis(700),
            &|| true,
        )
        .await
        .unwrap();
    let took = started.elapsed();
    assert!(timed_out);
    assert_eq!(seen[0].status, "running");
    assert!(took >= Duration::from_millis(700), "{took:?}");
    assert!(took < Duration::from_millis(1500), "{took:?}");
}

#[tokio::test]
async fn a_wait_returns_once_every_listed_agent_has_reported_or_ended() {
    let f = fixture();
    let p = f.parent("storm");
    let a = f.child(&p.id);
    let b = f.child(&p.id);
    let m = std::sync::Arc::new(f);
    let waiter = {
        let m = m.clone();
        let (p, a, b) = (p.id.clone(), a.id.clone(), b.id.clone());
        tokio::spawn(async move {
            let started = Instant::now();
            let out =
                m.m.wait_agents(&p, &[a, b], Duration::from_secs(20), &|| true)
                    .await
                    .unwrap();
            (out, started.elapsed())
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    m.m.report_result(&a.id, "succeeded", "done", None).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!waiter.is_finished(), "one of two settled is not all");
    m.report(&b.id, "failed");
    let ((seen, timed_out), took) = waiter.await.unwrap();
    assert!(!timed_out);
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert_eq!(seen[1].status, "failed");
}

#[tokio::test]
async fn a_revocation_ends_a_wait() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let m = std::sync::Arc::new(f);
    let waiter = {
        let m = m.clone();
        let (p, c) = (p.id.clone(), c.id.clone());
        tokio::spawn(async move {
            let offered = {
                let m = m.clone();
                let p = p.clone();
                move || m.m.spawn_offered(&p)
            };
            m.m.wait_agents(&p, &[c], Duration::from_secs(20), &offered)
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    m.m.set_spawn(&p.id, false, Some("dev_A")).unwrap().unwrap();
    let out = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("the wait ended")
        .unwrap();
    assert!(matches!(out, Err(AgentError::Conflict(c)) if c == "not_permitted"));
}

#[test]
fn a_result_is_one_of_the_outcomes_and_replaces_the_last() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    assert_eq!(
        RESULT_OUTCOMES,
        ["succeeded", "partial", "failed", "blocked"]
    );
    for outcome in RESULT_OUTCOMES {
        let r = f.m.report_result(&c.id, outcome, "s", Some("d")).unwrap();
        assert_eq!(f.m.result_of(&c.id).unwrap().unwrap(), r);
    }
    for bad in ["", "done", "SUCCEEDED", "success", " succeeded"] {
        assert!(
            matches!(
                f.m.report_result(&c.id, bad, "s", None),
                Err(AgentError::BadRequest(_))
            ),
            "{bad:?}"
        );
    }
    assert_eq!(f.m.result_of(&c.id).unwrap().unwrap().outcome, "blocked");
    // The store refuses on its own too: it is the only writer.
    let raw = SpawnResult {
        outcome: "done".into(),
        summary: "s".into(),
        details: None,
        reported_at: "2026-10-25T10:00:00Z".into(),
    };
    assert!(f.m.store.lock().unwrap().set_result(&c.id, &raw).is_err());
}

#[test]
fn a_result_has_a_bounded_summary_and_details() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let bad = |s: &str, d: Option<&str>| {
        matches!(
            f.m.report_result(&c.id, "succeeded", s, d),
            Err(AgentError::BadRequest(_))
        )
    };
    assert!(bad("", None));
    assert!(bad("   ", None));
    assert!(
        bad(&"é".repeat(MAX_SUMMARY_BYTES / 2 + 1), None),
        "bytes, not chars"
    );
    assert!(bad("s", Some(&"x".repeat(MAX_DETAILS_BYTES + 1))));
    assert!(!bad(
        &"x".repeat(MAX_SUMMARY_BYTES),
        Some(&"x".repeat(MAX_DETAILS_BYTES))
    ));
    assert!(f.m.result_of(&c.id).unwrap().is_some());
}

#[test]
fn only_a_spawned_live_session_on_a_c0_host_is_offered_the_spawned_tools() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    assert!(f.m.spawned_offered(&c.id));
    assert!(
        !f.m.spawned_offered(&p.id),
        "a top-level session has no parent"
    );
    f.m.lose_isolation_for_tests("hst_A");
    assert!(!f.m.spawned_offered(&c.id), "C0");
    f.m.verify_isolation_for_tests("hst_A");
    assert!(f.m.spawned_offered(&c.id));
    f.report(&c.id, "completed");
    assert!(!f.m.spawned_offered(&c.id), "ended");
}

#[test]
fn list_agents_says_which_cap_a_start_would_hit() {
    let f = fixture();
    let p = f.parent("storm");
    assert_eq!(f.m.start_cap(&p.id).unwrap(), None);
    for _ in 0..3 {
        f.child(&p.id);
    }
    assert_eq!(f.m.start_cap(&p.id).unwrap(), Some("children"));
}
