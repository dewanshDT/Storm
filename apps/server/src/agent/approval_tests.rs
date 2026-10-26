//! Decision 134-3: the approval engine, P-1…P-6 and AP-1…AP-5, AP-7. The
//! routes (AP-6 and approving through them) are tested in `api.rs`.

use std::sync::Barrier;
use std::time::Duration;

use rand::Rng;
use tokio::sync::mpsc::UnboundedReceiver;

use super::store::SessionRecord;
use super::store::spawn::{
    Decision, MAX_CHILDREN, MAX_PENDING, MAX_TREE, NewRequest, Refusal, Request,
};
use super::{AgentManager, Command, Envelope, Hello, Launch, LaunchMeta, McpGrant};

const VERIFIED: &str = r#"{"state": "verified", "mechanism": "so_peerpidfd", "reason": null}"#;
const UNAVAILABLE: &str = r#"{"state": "unavailable", "reason": "self_test"}"#;

/// A host isolating its sessions, with `identity` as given.
fn caps(identity: &str, max_sessions: u32) -> Hello {
    let mut hello = unisolated(identity, max_sessions);
    hello.capabilities.isolation = crate::agent::Isolation {
        state: "verified".into(),
        mechanism: Some("systemd_units".into()),
        reason: None,
    };
    hello
}

/// A host that does not say it isolates its sessions.
fn unisolated(identity: &str, max_sessions: u32) -> Hello {
    let providers: Vec<_> = ["claude-code", "opencode", "shell"]
        .iter()
        .map(|id| serde_json::json!({"id": id, "kind": "cli", "interactions": ["terminal"], "available": true}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "providers": providers,
        "workspaces": ["storm", "other"],
        // The test children's own directories too: WS-9 counts a workspace
        // with no reported directory as shared with every other.
        "workspace_paths": {
            "storm": "/w/storm", "other": "/w/other",
            "children/ags_C1": "/c/ags_C1", "children/ags_C2": "/c/ags_C2",
            "children/ags_G1": "/c/ags_G1", "children/ags_G2": "/c/ags_G2",
        },
        "max_sessions": max_sessions,
        "mcp_bridge": true,
        "identity": serde_json::from_str::<serde_json::Value>(identity).unwrap(),
    }))
    .unwrap()
}

struct Fixture {
    m: AgentManager,
    _dir: tempdir::TempDir,
    link: UnboundedReceiver<Envelope>,
}

/// A host whose hello says its identity is verified and it is isolated.
fn fixture(max_sessions: u32) -> Fixture {
    let dir = tempdir::TempDir::new("storm-approval").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (_, link) = m.connect_host("hst_A");
    m.hello("hst_A", caps(VERIFIED, max_sessions)).unwrap();
    Fixture { m, _dir: dir, link }
}

fn grant(id: &str) -> McpGrant {
    McpGrant {
        id: id.into(),
        slug: id.into(),
        plugin_origin: None,
    }
}

impl Fixture {
    fn launch(&self, workspace: &str, write_vault: Option<&str>) -> SessionRecord {
        self.launch_as(workspace, write_vault, false)
    }

    fn launch_as(&self, workspace: &str, write_vault: Option<&str>, spawn: bool) -> SessionRecord {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": "claude-code",
            "terminal": {"cols": 80, "rows": 24}, "spawn": spawn,
        }))
        .unwrap();
        let meta = LaunchMeta {
            write_vault_id: write_vault.map(Into::into),
            ..LaunchMeta::default()
        };
        let (r, ..) = self
            .m
            .launch("usr_A", launch, meta, vec![grant("storm"), grant("con_X")])
            .unwrap();
        self.report(&r.id, "running");
        r
    }

    fn report(&self, id: &str, status: &str) {
        let report =
            serde_json::from_value(serde_json::json!({"status": status, "exit_code": 0})).unwrap();
        self.m.host_status("hst_A", id, &report).unwrap();
    }

    /// A launched session with a write vault and spawn on.
    fn parent(&self) -> SessionRecord {
        self.launch_as("storm", Some("vlt_W"), true)
    }

    /// A running child of `parent`, recorded the way an approval records
    /// one, with spawn on when `spawning`.
    fn child(&self, parent: &str, id: &str, spawning: bool) -> String {
        let mut r = self.m.get(parent).unwrap();
        r.id = id.into();
        r.workspace = format!("children/{id}");
        r.status = "running".into();
        let mut store = self.m.store.lock().unwrap();
        store
            .insert_spawned(&r, parent, "a task", "2026-10-25T10:00:00Z")
            .unwrap();
        if spawning {
            store
                .set_spawn(id, true, Some("dev_A"), "2026-10-25T10:00:00Z")
                .unwrap();
        }
        drop(store);
        self.m.report_children_for_tests("hst_A");
        id.into()
    }

    fn start(&self, requester: &str) -> Result<Request, Refusal> {
        self.m
            .create_request(
                requester,
                NewRequest::Start {
                    task: "write the tests".into(),
                    provider: None,
                    name: None,
                    context_note: None,
                },
            )
            .unwrap()
    }

    fn approve(&self, id: &str, device: &str) -> Decision {
        let d = self.m.approve_request(id, device, false).unwrap();
        self.m.report_children_for_tests("hst_A");
        d
    }

    fn stored(&self, id: &str) -> Request {
        self.m.request(id).unwrap()
    }

    /// `list_changed` notifications sent since the last look, per session.
    fn told(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.link.try_recv().ok())
            .map(|e| serde_json::to_value(e).unwrap())
            .filter(|m| {
                m["type"] == "mcp.message"
                    && m["connection"] == "storm"
                    && m["message"]["method"] == "notifications/tools/list_changed"
            })
            .map(|m| m["session"].as_str().unwrap().to_string())
            .collect()
    }

    fn raw(&self, sql: &str, args: impl rusqlite::Params) {
        self.m
            .store
            .lock()
            .unwrap()
            .conn
            .execute(sql, args)
            .unwrap();
    }
}

fn limit(cap: &'static str) -> Refusal {
    Refusal {
        reason: "limit",
        cap: Some(cap),
    }
}

fn because(reason: &'static str) -> Refusal {
    Refusal { reason, cap: None }
}

fn outcome(r: &Request) -> (&str, Option<&str>, Option<&str>) {
    (r.state.as_str(), r.outcome.as_deref(), r.reason.as_deref())
}

fn decided(d: Decision) -> (Request, Option<Box<SessionRecord>>, Option<&'static str>) {
    match d {
        Decision::Decided {
            request,
            child,
            cap,
        } => (request, child, cap),
        other => panic!("not decided: {other:?}"),
    }
}

fn stamp(offset_ms: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    (time::OffsetDateTime::now_utc() + time::Duration::milliseconds(offset_ms))
        .format(&Rfc3339)
        .unwrap()
}

/// A request made directly in the store, past every check, expiring at
/// `expires_at`.
fn raw_request(
    f: &Fixture,
    id: &str,
    kind: &str,
    requester: &str,
    target: Option<&str>,
    expires_at: &str,
) {
    let payload = match kind {
        "start" => r#"{"task":"write the tests"}"#,
        "message" => r#"{"text":"hello"}"#,
        _ => "{}",
    };
    raw_request_with(f, id, kind, requester, target, expires_at, payload);
}

fn raw_request_with(
    f: &Fixture,
    id: &str,
    kind: &str,
    requester: &str,
    target: Option<&str>,
    expires_at: &str,
    payload: &str,
) {
    f.m.insert_request_for_tests(&Request {
        id: id.into(),
        kind: kind.into(),
        requester: requester.into(),
        target: target.map(Into::into),
        payload: payload.into(),
        created_at: stamp(-1000),
        expires_at: expires_at.into(),
        state: "pending".into(),
        decided_by_device: None,
        decided_at: None,
        outcome: None,
        reason: None,
        session: None,
    });
}

// ---- P-2: creation ----------------------------------------------------------

#[test]
fn a_request_is_pending_for_ten_minutes_with_the_exact_task() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    assert!(r.id.starts_with("agr_"), "{}", r.id);
    assert_eq!(
        (r.kind.as_str(), r.requester.as_str(), r.target.as_deref()),
        ("start", p.id.as_str(), None)
    );
    assert_eq!(r.payload, r#"{"task":"write the tests"}"#);
    assert_eq!(outcome(&r), ("pending", None, None));
    use time::format_description::well_known::Rfc3339;
    let at = |s: &str| time::OffsetDateTime::parse(s, &Rfc3339).unwrap();
    assert_eq!(
        at(&r.expires_at) - at(&r.created_at),
        time::Duration::minutes(10)
    );
    assert_eq!(f.stored(&r.id), r);
    // Over the size limits is a bad request, never a row.
    for new in [
        NewRequest::Start {
            task: "x".repeat(16 * 1024 + 1),
            provider: None,
            name: None,
            context_note: None,
        },
        NewRequest::Message {
            target: "ags_X".into(),
            text: "é".repeat(4097),
        },
    ] {
        assert!(matches!(
            f.m.create_request(&p.id, new),
            Err(super::AgentError::BadRequest(_))
        ));
    }
    assert_eq!(
        f.m.store.lock().unwrap().requests_of(&p.id).unwrap().len(),
        1
    );
}

#[test]
fn creation_needs_c0_spawn_on_a_live_requester_and_a_descendant_target() {
    let mut f = fixture(16);
    let p = f.parent();
    let off = f.launch("other", None);
    assert_eq!(f.start(&off.id), Err(because("not_permitted")), "spawn off");
    // Another provider than the parent's is refused in V1 (B4).
    let other = f.m.create_request(
        &p.id,
        NewRequest::Start {
            task: "t".into(),
            provider: Some("opencode".into()),
            name: None,
            context_note: None,
        },
    );
    assert_eq!(other.unwrap(), Err(because("not_permitted")));
    let same = f.m.create_request(
        &p.id,
        NewRequest::Start {
            task: "t".into(),
            provider: Some("claude-code".into()),
            name: None,
            context_note: None,
        },
    );
    assert!(same.unwrap().is_ok());
    // Message and end name a live descendant, or are not found.
    let c = f.child(&p.id, "ags_C", false);
    let stranger = f.launch("other", None);
    for target in [stranger.id.as_str(), "ags_NOPE", p.id.as_str()] {
        let r = f.m.create_request(
            &p.id,
            NewRequest::End {
                target: target.into(),
            },
        );
        assert_eq!(r.unwrap(), Err(because("not_found")), "{target}");
    }
    let to_child = f.m.create_request(
        &p.id,
        NewRequest::Message {
            target: c.clone(),
            text: "hi".into(),
        },
    );
    assert!(to_child.unwrap().is_ok());
    // A child cannot address its parent.
    f.m.store
        .lock()
        .unwrap()
        .set_spawn(&c, true, None, "2026-10-25T10:00:00Z")
        .unwrap();
    let up = f.m.create_request(
        &c,
        NewRequest::End {
            target: p.id.clone(),
        },
    );
    assert_eq!(up.unwrap(), Err(because("not_found")));

    // C0, as the live link says now.
    f.told();
    f.m.lose_isolation_for_tests("hst_A");
    assert_eq!(f.start(&p.id), Err(because("isolation_unverified")));
    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert_eq!(f.start(&p.id), Err(because("identity_unverified")));
    // An ended requester.
    f.m.inventory("hst_A", caps(VERIFIED, 16).capabilities)
        .unwrap();
    f.m.verify_isolation_for_tests("hst_A");
    f.report(&p.id, "completed");
    assert_eq!(f.start(&p.id), Err(because("requester_ended")));
    assert!(matches!(
        f.m.create_request("ags_NOPE", NewRequest::End { target: c }),
        Err(super::AgentError::NotFound(_))
    ));
}

// ---- C4: the caps, and AP-1 under concurrency --------------------------------

#[test]
fn each_cap_refuses_at_its_boundary_and_names_itself() {
    // Depth: C4 allows 2, but no child starts one until a decision says
    // how a grandchild reads its parent's own workspace (BUILT_DEPTH).
    let f = fixture(16);
    let p = f.parent();
    let c = f.child(&p.id, "ags_C", true);
    let g = f.child(&c, "ags_G", true);
    assert_eq!(
        f.start(&c),
        Err(limit("depth")),
        "a child starts none today"
    );
    assert_eq!(f.start(&g), Err(limit("depth")));
    assert!(f.start(&p.id).is_ok());

    // Pending per parent: 2.
    let f = fixture(16);
    let p = f.parent();
    for _ in 0..MAX_PENDING {
        f.start(&p.id).unwrap();
    }
    assert_eq!(f.start(&p.id), Err(limit("pending")));
    let message = f.m.create_request(
        &p.id,
        NewRequest::End {
            target: f.child(&p.id, "ags_C", false),
        },
    );
    assert_eq!(message.unwrap(), Err(limit("pending")), "every kind counts");

    // Live children per parent: 3, pending starts counted.
    let f = fixture(16);
    let p = f.parent();
    f.child(&p.id, "ags_C1", false);
    f.child(&p.id, "ags_C2", false);
    f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("children")));
    // An ended child frees its slot.
    f.report("ags_C1", "completed");
    assert!(f.start(&p.id).is_ok());

    // Live sessions per tree: 6, the root included. The grandchildren are
    // written past the caps: no start makes one today.
    let f = fixture(16);
    let p = f.parent();
    let c1 = f.child(&p.id, "ags_C1", false);
    f.child(&c1, "ags_G1", false);
    f.child(&c1, "ags_G2", false);
    // Four live; one pending start makes five, a second six.
    f.start(&p.id).unwrap();
    f.child(&c1, "ags_G3", false);
    assert_eq!(f.start(&p.id), Err(limit("tree")));
    let other_tree = f.parent_in("other");
    assert!(f.start(&other_tree).is_ok(), "another tree is not counted");

    // The host's max_sessions, pending starts counted.
    let f = fixture(3);
    let p = f.parent();
    f.launch("other", None);
    f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("max_sessions")));
}

impl Fixture {
    fn parent_in(&self, workspace: &str) -> String {
        let p = self.launch_as(workspace, None, true);
        p.id
    }
}

/// AP-1: concurrent requests at each cap's boundary create exactly the free
/// slots; the rest are refused, naming the cap.
#[test]
fn ap1_concurrent_requests_take_exactly_the_free_slots() {
    const RACERS: usize = 8;
    for round in 0..10 {
        // (setup, requesters, free slots, the cap the rest hit)
        type Setup = fn(&Fixture) -> Vec<String>;
        let cases: [(Setup, u32, i64, &str); 5] = [
            (
                |f| {
                    let p = f.parent();
                    vec![p.id; RACERS]
                },
                16,
                MAX_PENDING,
                "pending",
            ),
            (
                |f| {
                    let p = f.parent();
                    f.child(&p.id, "ags_C1", false);
                    f.child(&p.id, "ags_C2", false);
                    vec![p.id; RACERS]
                },
                16,
                MAX_CHILDREN - 2,
                "children",
            ),
            (
                |f| {
                    let p = f.parent();
                    let c = f.child(&p.id, "ags_C1", false);
                    for g in ["ags_G1", "ags_G2", "ags_G3"] {
                        f.child(&c, g, false);
                    }
                    // Five live: one free in the tree, two under the
                    // children and pending caps.
                    vec![p.id; RACERS]
                },
                16,
                MAX_TREE - 5,
                "tree",
            ),
            (
                |f| {
                    let p = f.parent();
                    f.launch("other", None);
                    vec![p.id; RACERS]
                },
                3,
                1,
                "max_sessions",
            ),
            (
                |f| {
                    let p = f.parent();
                    let c = f.child(&p.id, "ags_C", true);
                    vec![c; RACERS]
                },
                16,
                0,
                "depth",
            ),
        ];
        for (setup, max_sessions, free, cap) in cases {
            let f = fixture(max_sessions);
            let requesters = setup(&f);
            let barrier = Barrier::new(requesters.len());
            let results: Vec<Result<Request, Refusal>> = std::thread::scope(|s| {
                let handles: Vec<_> = requesters
                    .iter()
                    .map(|r| {
                        let (f, barrier) = (&f, &barrier);
                        s.spawn(move || {
                            barrier.wait();
                            f.start(r)
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let created = results.iter().filter(|r| r.is_ok()).count() as i64;
            assert_eq!(created, free, "round {round}: {cap}");
            for r in results.iter().filter_map(|r| r.as_ref().err()) {
                assert_eq!(*r, limit(cap), "round {round}");
            }
        }
    }
}

// ---- P-3: approval -----------------------------------------------------------

#[test]
fn an_approved_start_records_the_child_narrowed_under_its_parent() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    let (request, child, cap) = decided(f.approve(&r.id, "dev_A"));
    let child = child.expect("a child");
    assert_eq!(cap, None);
    assert_eq!(outcome(&request), ("decided", Some("started"), None));
    assert_eq!(request.session.as_deref(), Some(child.id.as_str()));
    assert_eq!(request.decided_by_device.as_deref(), Some("dev_A"));
    assert!(request.decided_at.is_some());
    assert_eq!(f.stored(&r.id), request);

    let store = f.m.store.lock().unwrap();
    let lineage = store.lineage(&child.id).unwrap();
    assert_eq!(
        (
            lineage.parent_session.as_deref(),
            lineage.root_session.as_str(),
            lineage.depth
        ),
        (Some(p.id.as_str()), p.id.as_str(), 1)
    );
    assert_eq!(
        store.task(&child.id).unwrap().as_deref(),
        Some("write the tests")
    );
    assert!(!store.spawn_enabled(&child.id).unwrap(), "spawn is off");
    let c = store.get(&child.id).unwrap().unwrap();
    assert_eq!(
        (
            c.host_id.as_str(),
            c.provider.as_str(),
            c.owner_user_id.as_str()
        ),
        (
            p.host_id.as_str(),
            p.provider.as_str(),
            p.owner_user_id.as_str()
        )
    );
    let names = |g: Vec<McpGrant>| g.into_iter().map(|g| g.id).collect::<Vec<_>>();
    assert_eq!(
        names(store.grants_of(&child.id).unwrap()),
        ["con_X", "storm"]
    );
    // D2: no write vault unless the person ticks it.
    assert_eq!(
        store.launch(&child.id).unwrap().unwrap().write_vault_id,
        None
    );
    assert!(matches!(
        store.grant(&child.id, "storm"),
        Ok(Some((_, false)))
    ));
    // 134-5: it is started in its own workspace, which its host makes from
    // the parent's.
    assert_eq!(
        (c.status.as_str(), c.end_reason.as_deref()),
        ("starting", None)
    );
    assert_eq!(c.workspace, format!("children/{}", child.id));
    assert!(
        store.spawn_asks(&child.id).unwrap(),
        "its start carries the asks"
    );
}

#[test]
fn ap7_a_grant_the_parent_lost_before_approval_never_reaches_the_child() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    f.m.revoke_grants_for("con_X").unwrap();
    let (_, child, _) = decided(f.m.approve_request(&r.id, "dev_A", true).unwrap());
    let child = child.unwrap();
    let store = f.m.store.lock().unwrap();
    let ids: Vec<String> = store
        .grants_of(&child.id)
        .unwrap()
        .into_iter()
        .map(|g| g.id)
        .collect();
    assert_eq!(ids, ["storm"]);
    assert_eq!(store.grant(&child.id, "con_X").unwrap(), None);
    // The person ticked the write vault: the parent's own, and only it.
    assert_eq!(
        store
            .launch(&child.id)
            .unwrap()
            .unwrap()
            .write_vault_id
            .as_deref(),
        Some("vlt_W")
    );
    drop(store);

    // A parent without a write vault gives none, ticked or not.
    f.m.report_children_for_tests("hst_A");
    let q = f.parent_in("other");
    let r = f.start(&q).unwrap();
    let (_, child, _) = decided(f.m.approve_request(&r.id, "dev_A", true).unwrap());
    let store = f.m.store.lock().unwrap();
    assert_eq!(
        store
            .launch(&child.unwrap().id)
            .unwrap()
            .unwrap()
            .write_vault_id,
        None
    );
}

/// Each condition flipped between request and approval refuses it with its
/// reason, and nothing acts. These reach the approval's own re-check: the
/// paths that refuse at the change itself are bypassed in SQL.
#[test]
fn approval_rechecks_current_state() {
    type Flip = fn(&Fixture, &str, &str);
    let cases: [(Flip, &str); 6] = [
        (
            |f, p, _| {
                f.raw(
                    "UPDATE sessions SET status = 'completed' WHERE id = ?1",
                    [p],
                )
            },
            "requester_ended",
        ),
        (
            |f, p, _| {
                f.raw(
                    "UPDATE session_spawn SET enabled = 0 WHERE session_id = ?1",
                    [p],
                )
            },
            "revoked",
        ),
        (
            |f, _, _| {
                let (generation, _) = f.m.connect_host("hst_A");
                f.m.disconnect_host("hst_A", generation);
            },
            "identity_unverified",
        ),
        (
            |f, _, _| {
                // Between the request and its approval, with no reassessment:
                // the approval's own re-check must see it.
                f.m.live
                    .lock()
                    .unwrap()
                    .hosts
                    .get_mut("hst_A")
                    .unwrap()
                    .capabilities
                    .as_mut()
                    .unwrap()
                    .isolation = crate::agent::Isolation::unavailable("helper");
            },
            "isolation_unverified",
        ),
        (|f, _, c| f.report(c, "completed"), "target_gone"),
        (
            |f, _, _| {
                for _ in 0..14 {
                    f.launch("other", None);
                }
            },
            "limit",
        ),
    ];
    for (flip, reason) in cases {
        let f = fixture(16);
        let p = f.parent();
        let c = f.child(&p.id, "ags_C", false);
        let kind = if reason == "target_gone" {
            "end"
        } else {
            "start"
        };
        let target = (kind == "end").then_some(c.as_str());
        raw_request(&f, "agr_1", kind, &p.id, target, &stamp(60_000));
        flip(&f, &p.id, &c);
        let (request, child, cap) = decided(f.approve("agr_1", "dev_A"));
        assert_eq!(
            outcome(&request),
            ("decided", Some("refused"), Some(reason))
        );
        assert_eq!(child, None, "{reason}");
        assert_eq!(cap, (reason == "limit").then_some("max_sessions"));
        assert_eq!(
            f.m.store.lock().unwrap().descendants(&p.id, false).unwrap(),
            [c]
        );
    }
}

/// AP-4: two devices approve the same request; one acts, the other finds it
/// taken. Once decided, nothing decides it again.
#[test]
fn ap4_a_request_is_decided_once() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    let (first, ..) = decided(f.approve(&r.id, "dev_A"));
    assert!(matches!(f.approve(&r.id, "dev_B"), Decision::Taken(t) if t == first));
    assert!(matches!(
        f.m.deny_request(&r.id, "dev_B").unwrap(),
        Decision::Taken(_)
    ));
    assert!(f.m.expire_requests().unwrap().is_empty());
    assert_eq!(f.stored(&r.id), first);
    assert_eq!(
        f.m.store
            .lock()
            .unwrap()
            .descendants(&p.id, false)
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        f.m.approve_request("agr_NOPE", "dev_A", false),
        Err(super::AgentError::NotFound(_))
    ));

    for _ in 0..20 {
        let f = fixture(16);
        let p = f.parent();
        let r = f.start(&p.id).unwrap();
        let barrier = Barrier::new(4);
        let decisions: Vec<Decision> = std::thread::scope(|s| {
            let handles: Vec<_> = ["dev_A", "dev_B", "dev_C", "dev_D"]
                .into_iter()
                .map(|d| {
                    let (f, barrier, id) = (&f, &barrier, &r.id);
                    s.spawn(move || {
                        barrier.wait();
                        f.approve(id, d)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let acted = decisions
            .iter()
            .filter(|d| matches!(d, Decision::Decided { .. }))
            .count();
        assert_eq!(acted, 1);
        assert_eq!(
            f.m.store
                .lock()
                .unwrap()
                .descendants(&p.id, false)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn a_denial_is_recorded_with_its_device_and_never_acts() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    let (request, child, _) = decided(f.m.deny_request(&r.id, "dev_B").unwrap());
    assert_eq!(child, None);
    assert_eq!(outcome(&request), ("decided", Some("denied"), None));
    assert_eq!(request.decided_by_device.as_deref(), Some("dev_B"));
    assert!(matches!(f.approve(&r.id, "dev_A"), Decision::Taken(_)));
    assert!(
        f.m.store
            .lock()
            .unwrap()
            .descendants(&p.id, false)
            .unwrap()
            .is_empty()
    );
    // A denied request frees its pending slot.
    f.start(&p.id).unwrap();
    f.start(&p.id).unwrap();
}

#[test]
fn an_approved_end_is_decided_ended() {
    let f = fixture(16);
    let p = f.parent();
    let c = f.child(&p.id, "ags_C", false);
    let r =
        f.m.create_request(&p.id, NewRequest::End { target: c })
            .unwrap()
            .unwrap();
    let (request, child, _) = decided(f.approve(&r.id, "dev_A"));
    assert_eq!(child, None);
    assert_eq!(outcome(&request), ("decided", Some("ended"), None));
    assert!(matches!(
        f.m.deny_request(&r.id, "dev_A").unwrap(),
        Decision::Taken(_)
    ));
}

// ---- P-5: expiry -------------------------------------------------------------

/// AP-5: approval after expiry is `expired`, and nothing acts; the sweeper
/// marks what is due and nothing else.
/// AP-3: a child the person ends the parent of while its approval is
/// between commit and `start` is never sent `end` first, so the host
/// cannot drop the `end` and then start it.
#[test]
fn ap3_an_end_never_precedes_its_childs_start() {
    let mut f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    let now = crate::index::now_rfc3339();
    let decision =
        f.m.store
            .lock()
            .unwrap()
            .approve_request(&r.id, "dev_A", false, &now, &|_| Ok(16))
            .unwrap()
            .unwrap();
    let child = *decided(decision).1.unwrap();
    assert_eq!(child.status, "creating");
    f.m.end_by_person(&p.id, Some(std::slice::from_ref(&child.id)), false)
        .unwrap()
        .unwrap();
    assert!(f.m.stop_cascade_targets(&now).unwrap().is_empty());

    f.m.dispatch_spawned(&child).unwrap();
    let stops = f.m.stop_cascade_targets(&now).unwrap();
    assert_eq!(stops.len(), 1);
    let sent: Vec<&str> = std::iter::from_fn(|| f.link.try_recv().ok())
        .filter_map(|e| match e.command {
            Command::Start { session, .. } if session == child.id => Some("start"),
            Command::End { session } if session == child.id => Some("end"),
            _ => None,
        })
        .collect();
    assert_eq!(sent, ["start", "end"]);
}

#[test]
fn ap5_an_expired_request_never_acts() {
    let f = fixture(16);
    let p = f.parent();
    raw_request(&f, "agr_OLD", "start", &p.id, None, &stamp(-1));
    let (request, child, _) = decided(f.approve("agr_OLD", "dev_A"));
    assert_eq!(outcome(&request), ("decided", Some("expired"), None));
    assert_eq!(child, None);
    assert!(
        f.m.store
            .lock()
            .unwrap()
            .descendants(&p.id, false)
            .unwrap()
            .is_empty()
    );

    raw_request(&f, "agr_DENY", "start", &p.id, None, &stamp(-1));
    let (request, ..) = decided(f.m.deny_request("agr_DENY", "dev_A").unwrap());
    assert_eq!(request.outcome.as_deref(), Some("expired"));

    // An unparseable expiry is expired, never open-ended.
    raw_request(&f, "agr_BAD", "start", &p.id, None, "soon");
    raw_request(&f, "agr_DUE", "start", &p.id, None, &stamp(-1));
    let other = f.parent_in("other");
    raw_request(&f, "agr_LATER", "start", &other, None, &stamp(60_000));
    let swept: Vec<String> =
        f.m.expire_requests()
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
    assert_eq!(swept.len(), 2);
    assert!(swept.contains(&"agr_BAD".to_string()) && swept.contains(&"agr_DUE".to_string()));
    for id in ["agr_BAD", "agr_DUE"] {
        let r = f.stored(id);
        assert_eq!(outcome(&r), ("decided", Some("expired"), None));
        assert_eq!(r.decided_by_device, None, "no device decided it");
    }
    assert_eq!(outcome(&f.stored("agr_LATER")), ("pending", None, None));
    assert!(
        f.m.expire_requests().unwrap().is_empty(),
        "a second pass is a no-op"
    );
    assert!(matches!(f.approve("agr_DUE", "dev_A"), Decision::Taken(_)));
}

#[test]
fn expiry_survives_a_restart() {
    let f = fixture(16);
    let p = f.parent();
    raw_request(&f, "agr_1", "start", &p.id, None, &stamp(-1));
    let Fixture { m, _dir: dir, link } = f;
    drop((m, link));
    let m = AgentManager::open(dir.path()).unwrap();
    assert_eq!(m.expire_requests().unwrap().len(), 1);
    assert_eq!(
        m.request("agr_1").unwrap().outcome.as_deref(),
        Some("expired")
    );
}

// ---- P-4: refusal by revocation, a requester's end and a host downgrade --------

#[test]
fn p4_a_requesters_end_refuses_its_requests_in_the_same_write() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    f.report(&p.id, "failed");
    assert_eq!(
        outcome(&f.stored(&r.id)),
        ("decided", Some("refused"), Some("requester_ended"))
    );
    assert!(matches!(f.approve(&r.id, "dev_A"), Decision::Taken(_)));
    // A host that drops its link leaves its sessions unknown, not ended:
    // their requests wait until they expire (L-6).
    let q = f.parent_in("other");
    let r = f.start(&q).unwrap();
    let (generation, _link) = f.m.connect_host("hst_A");
    f.m.disconnect_host("hst_A", generation);
    assert_eq!(f.m.get(&q).unwrap().status, "unknown");
    assert_eq!(outcome(&f.stored(&r.id)), ("pending", None, None));
}

#[test]
fn p4_a_host_downgrade_refuses_pending_requests_and_tells_spawning_agents() {
    let mut f = fixture(16);
    let p = f.parent();
    let quiet = f.launch("other", None);
    let r1 = f.start(&p.id).unwrap();
    f.told();

    // Identity drops on the live link.
    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert_eq!(
        outcome(&f.stored(&r1.id)),
        ("decided", Some("refused"), Some("identity_unverified"))
    );
    assert_eq!(f.told(), std::slice::from_ref(&p.id), "spawn on only, once");
    assert!(!f.told().contains(&quiet.id));
    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert!(f.told().is_empty(), "no flip, no notification");

    // Back: one notification, and nothing refused.
    f.m.inventory("hst_A", caps(VERIFIED, 16).capabilities)
        .unwrap();
    assert_eq!(f.told(), std::slice::from_ref(&p.id));
    let r2 = f.start(&p.id).unwrap();

    // Isolation drops.
    f.m.lose_isolation_for_tests("hst_A");
    assert_eq!(
        outcome(&f.stored(&r2.id)),
        ("decided", Some("refused"), Some("isolation_unverified"))
    );
    assert_eq!(f.told(), std::slice::from_ref(&p.id));
    f.m.verify_isolation_for_tests("hst_A");
    assert_eq!(f.told(), std::slice::from_ref(&p.id));

    // Away, its requests are kept; a new link that has not said it is
    // isolated refuses them at its hello.
    let r3 = f.start(&p.id).unwrap();
    let (generation, _) = f.m.connect_host("hst_A");
    f.m.disconnect_host("hst_A", generation);
    // Whatever is reassessed while it is away, its requests wait (L-6).
    f.m.lose_isolation_for_tests("hst_A");
    assert_eq!(outcome(&f.stored(&r3.id)), ("pending", None, None));
    let (_, mut link) = f.m.connect_host("hst_A");
    let mut hello = unisolated(VERIFIED, 16);
    hello.sessions =
        vec![serde_json::from_value(serde_json::json!({"id": p.id, "status": "running"})).unwrap()];
    f.m.hello("hst_A", hello).unwrap();
    assert_eq!(f.m.get(&p.id).unwrap().status, "running");
    assert_eq!(
        outcome(&f.stored(&r3.id)),
        ("decided", Some("refused"), Some("isolation_unverified"))
    );
    let told: Vec<String> = std::iter::from_fn(|| link.try_recv().ok())
        .map(|e| serde_json::to_value(e).unwrap())
        .filter(|m| m["message"]["method"] == "notifications/tools/list_changed")
        .map(|m| m["session"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(told, std::slice::from_ref(&p.id), "told on the new link");
}

/// A session whose adapter cannot spawn is refused even with its switch on
/// (the switch route never allows it; this is the engine's own check).
#[test]
fn an_adapter_that_cannot_spawn_is_refused_at_creation_and_approval() {
    let f = fixture(16);
    let p = f.parent();
    raw_request(&f, "agr_1", "start", &p.id, None, &stamp(60_000));
    f.raw(
        "UPDATE sessions SET provider = 'shell' WHERE id = ?1",
        [&p.id],
    );
    assert_eq!(f.start(&p.id), Err(because("not_permitted")));
    let (request, child, _) = decided(f.approve("agr_1", "dev_A"));
    assert_eq!(
        outcome(&request),
        ("decided", Some("refused"), Some("not_permitted"))
    );
    assert_eq!(child, None);
}

/// B4 again at approval: a payload naming another provider never starts.
#[test]
fn another_provider_is_refused_at_approval_too() {
    let f = fixture(16);
    let p = f.parent();
    let payload = r#"{"task":"t","provider":"opencode"}"#;
    raw_request_with(&f, "agr_1", "start", &p.id, None, &stamp(60_000), payload);
    let (request, child, _) = decided(f.approve("agr_1", "dev_A"));
    assert_eq!(
        outcome(&request),
        ("decided", Some("refused"), Some("not_permitted"))
    );
    assert_eq!(child, None);
}

/// At approval the request is counted once: one that took the last slot
/// still starts.
#[test]
fn a_request_holding_the_last_slot_is_approved() {
    let f = fixture(16);
    let p = f.parent();
    f.child(&p.id, "ags_C1", false);
    f.child(&p.id, "ags_C2", false);
    let last = f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("children")));
    let (request, child, _) = decided(f.approve(&last.id, "dev_A"));
    assert_eq!(outcome(&request), ("decided", Some("started"), None));
    assert!(child.is_some());
}

// ---- races: exactly one winner -----------------------------------------------

/// Approve, approve, deny, expire, revoke and the requester's end, all at
/// once on one request: exactly one decides it, and a child exists exactly
/// when that was a start (AP-2, AP-3).
#[test]
fn every_way_of_deciding_a_request_races_to_exactly_one_winner() {
    for round in 0..200 {
        let f = fixture(16);
        let p = f.parent();
        let mut jitter = [0u8; 8];
        rand::rng().fill_bytes(&mut jitter);
        let expires = stamp(i64::from(jitter[7] % 4));
        raw_request(&f, "agr_1", "start", &p.id, None, &expires);
        let barrier = Barrier::new(6);
        let wait = |i: usize| {
            barrier.wait();
            std::thread::sleep(Duration::from_micros(u64::from(jitter[i]) * 4));
        };
        let (approvals, denial, swept, revoked) = std::thread::scope(|s| {
            let a1 = s.spawn(|| {
                wait(0);
                f.approve("agr_1", "dev_A")
            });
            let a2 = s.spawn(|| {
                wait(1);
                f.approve("agr_1", "dev_B")
            });
            let d = s.spawn(|| {
                wait(2);
                f.m.deny_request("agr_1", "dev_C").unwrap()
            });
            let e = s.spawn(|| {
                wait(3);
                f.m.expire_requests().unwrap().len()
            });
            let r = s.spawn(|| {
                wait(4);
                f.m.set_spawn(&p.id, false, Some("dev_D"))
                    .unwrap()
                    .unwrap()
                    .refused
            });
            s.spawn(|| {
                wait(5);
                f.report(&p.id, "completed");
            });
            (
                [a1.join().unwrap(), a2.join().unwrap()],
                d.join().unwrap(),
                e.join().unwrap(),
                r.join().unwrap(),
            )
        });
        let won = |d: &Decision| matches!(d, Decision::Decided { .. });
        let claims =
            approvals.iter().filter(|d| won(d)).count() + won(&denial) as usize + swept + revoked;
        let r = f.stored("agr_1");
        assert_eq!(r.state, "decided", "round {round}");
        assert!(claims <= 1, "round {round}: {claims} winners");
        if claims == 0 {
            assert_eq!(
                r.reason.as_deref(),
                Some("requester_ended"),
                "round {round}"
            );
        }
        if revoked == 1 {
            assert_eq!(r.reason.as_deref(), Some("revoked"), "round {round}");
        }
        let children = f.m.store.lock().unwrap().descendants(&p.id, false).unwrap();
        assert_eq!(
            children.len(),
            usize::from(r.outcome.as_deref() == Some("started")),
            "round {round}: {r:?}"
        );
        // AP-2, AP-3: a child exists only if it was started before the
        // revocation or the end committed, and its parent's end has it in
        // a cascade: none outlives it.
        let store = f.m.store.lock().unwrap();
        for child in store.descendants(&p.id, true).unwrap() {
            assert!(
                store.doomed(&child).unwrap(),
                "round {round}: {child} outlives its parent"
            );
        }
    }
}

// ---- waiting -------------------------------------------------------------------

#[tokio::test]
async fn a_waiter_hears_the_decision_or_times_out_pending() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start(&p.id).unwrap();
    let t = std::time::Instant::now();
    let pending =
        f.m.wait_request(&r.id, Duration::from_millis(100))
            .await
            .unwrap();
    assert_eq!(pending.state, "pending");
    assert!(t.elapsed() >= Duration::from_millis(100));
    let (decided, _) = tokio::join!(f.m.wait_request(&r.id, Duration::from_secs(5)), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        f.m.deny_request(&r.id, "dev_A").unwrap();
    });
    let decided = decided.unwrap();
    assert_eq!(decided.outcome.as_deref(), Some("denied"));
    assert!(t.elapsed() < Duration::from_secs(2));
}

// ---- 134-5: the approved child's start ----------------------------------------

impl Fixture {
    /// The `start` commands sent down the host's link since the last look.
    fn starts(&mut self) -> Vec<serde_json::Value> {
        std::iter::from_fn(|| self.link.try_recv().ok())
            .map(|e| serde_json::to_value(e).unwrap())
            .filter(|m| m["type"] == "start")
            .collect()
    }

    fn start_as(&self, requester: &str, provider: Option<&str>) -> Result<Request, Refusal> {
        self.m
            .create_request(
                requester,
                NewRequest::Start {
                    task: "write the tests".into(),
                    provider: provider.map(Into::into),
                    name: Some("Test writer".into()),
                    context_note: None,
                },
            )
            .unwrap()
    }
}

/// 134e and the checks table's narrowing row, on the wire: the host is told
/// to start the child with exactly the parent's live grants (AP-7: one
/// revoked between request and approval is not among them), the parent's
/// agent, on the parent's host, from the parent's workspace into one of
/// its own, as the parent's child, with the asks and the spawned prompt's
/// flag, and nothing else. Spawn is off and the write vault is D2's.
#[test]
fn an_approved_child_is_started_narrowed_in_its_own_workspace() {
    let mut f = fixture(16);
    let p = f.parent();
    f.starts();
    let r = f.start_as(&p.id, None).unwrap();
    f.m.revoke_grants_for("con_X").unwrap();
    let (request, child, _) = decided(f.m.approve_request(&r.id, "dev_A", false).unwrap());
    let child = child.unwrap();
    assert_eq!(outcome(&request), ("decided", Some("started"), None));
    let starts = f.starts();
    assert_eq!(starts.len(), 1, "{starts:?}");
    let mut start = starts[0].clone();
    start.as_object_mut().unwrap().remove("cmd_seq");
    assert_eq!(
        start,
        serde_json::json!({
            "type": "start", "session": child.id, "workspace": "storm",
            "provider": "claude-code", "interaction": "terminal",
            "terminal": {"cols": 80, "rows": 24},
            "mcp": [{"id": "storm", "slug": "storm"}],
            "context": true, "spawn_asks": {"spawned": true}, "child_of": p.id,
        })
    );
    let c = f.m.get(&child.id).unwrap();
    assert_eq!(
        (c.host_id.as_str(), c.provider.as_str(), c.status.as_str()),
        ("hst_A", "claude-code", "starting")
    );
    assert_eq!(c.workspace, format!("children/{}", child.id));
    assert!(!f.m.spawn_enabled(&c.id).unwrap(), "spawn is off");
    let launch = f.m.launch_of(&c.id).unwrap().unwrap();
    assert_eq!(launch.write_vault_id, None, "D2: unticked, none");
    assert_eq!(launch.name, "test-writer");
    // A child the host has not placed may be anywhere, but it is the
    // parent's own tree: never counted as sharing (141b), and the switch
    // turns on again either way (141a).
    f.m.set_spawn(&p.id, false, Some("dev_A")).unwrap().unwrap();
    assert_eq!(f.m.shared_with(&p.id).unwrap(), 0);
    assert!(f.m.set_spawn(&p.id, true, Some("dev_A")).unwrap().is_ok());
    f.m.report_children_for_tests("hst_A");
    assert_eq!(f.m.shared_with(&c.id).unwrap(), 0, "its parent is its tree");

    // Ticked: the parent's own write vault, and only it.
    let r = f.start_as(&p.id, None).unwrap();
    let (_, child, _) = decided(f.m.approve_request(&r.id, "dev_A", true).unwrap());
    let launch = f.m.launch_of(&child.unwrap().id).unwrap().unwrap();
    assert_eq!(launch.write_vault_id.as_deref(), Some("vlt_W"));
}

/// B4, the checks table's provider row: another agent than the parent's is
/// refused when it is asked for and again at approval; naming the parent's
/// own is a same-provider start.
#[test]
fn b4_a_child_runs_its_parents_agent_or_nothing() {
    let mut f = fixture(16);
    let p = f.parent();
    for other in ["opencode", "codex", "shell", "Claude-Code"] {
        assert_eq!(
            f.start_as(&p.id, Some(other)),
            Err(because("not_permitted")),
            "{other}"
        );
    }
    let same = f.start_as(&p.id, Some("claude-code")).unwrap();
    let (request, ..) = decided(f.approve(&same.id, "dev_A"));
    assert_eq!(request.outcome.as_deref(), Some("started"));

    raw_request_with(
        &f,
        "agr_B4",
        "start",
        &p.id,
        None,
        &stamp(60_000),
        r#"{"task":"t","provider":"opencode"}"#,
    );
    f.starts();
    let (request, child, _) = decided(f.approve("agr_B4", "dev_A"));
    assert_eq!(
        outcome(&request),
        ("decided", Some("refused"), Some("not_permitted"))
    );
    assert!(child.is_none());
    assert!(f.starts().is_empty(), "nothing reaches the host");
}

/// A decided request cannot change: a child whose host takes no command
/// after commit is its own `failed: start_failure`, and the request keeps
/// `started` (134-3's reading).
#[test]
fn a_child_its_host_cannot_be_told_of_fails_on_its_own_record() {
    let mut f = fixture(16);
    let p = f.parent();
    let r = f.start_as(&p.id, None).unwrap();
    let (_, gone) = tokio::sync::mpsc::unbounded_channel();
    drop(std::mem::replace(&mut f.link, gone));
    let (request, child, _) = decided(f.approve(&r.id, "dev_A"));
    assert_eq!(request.outcome.as_deref(), Some("started"));
    let c = f.m.get(&child.unwrap().id).unwrap();
    assert_eq!(
        (c.status.as_str(), c.end_reason.as_deref()),
        ("failed", Some("start_failure"))
    );
    // The slot it held is free again.
    assert!(f.start_as(&p.id, None).is_ok());
}

/// 137f, 137h: a child whose workspace its host could not make ends with
/// why, on its own record.
#[test]
fn a_child_whose_workspace_was_refused_ends_with_why() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start_as(&p.id, None).unwrap();
    let (_, child, _) = decided(f.approve(&r.id, "dev_A"));
    let child = child.unwrap();
    let report = serde_json::from_value(
        serde_json::json!({"status": "failed", "end_reason": "workspace_cap"}),
    )
    .unwrap();
    f.m.host_status("hst_A", &child.id, &report).unwrap();
    let c = f.m.get(&child.id).unwrap();
    assert_eq!(
        (c.status.as_str(), c.end_reason.as_deref()),
        ("failed", Some("workspace_cap"))
    );
    assert_eq!(
        f.stored(&r.id).outcome.as_deref(),
        Some("started"),
        "the decision stands"
    );
}

/// What a child's host says it made is kept on the child's record once:
/// only for a session with a parent, and only in the shape a helper
/// answers (a hex commit, a branch of the clone's own characters).
#[test]
fn a_childs_workspace_is_kept_as_its_host_made_it() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start_as(&p.id, None).unwrap();
    let (_, child, _) = decided(f.approve(&r.id, "dev_A"));
    let child = child.unwrap().id;
    let report = |id: &str, made: serde_json::Value| {
        let report = serde_json::from_value(
            serde_json::json!({"status": "running", "child_workspace": made}),
        )
        .unwrap();
        f.m.host_status("hst_A", id, &report).unwrap();
    };
    for bad in [
        serde_json::json!({"mode": "clone", "commit": "not-hex"}),
        serde_json::json!({"mode": "clone"}),
        serde_json::json!({"mode": "clone", "commit": "a".repeat(40), "branch": "x;rm -rf"}),
        serde_json::json!({"mode": "scratch", "commit": "a".repeat(40)}),
        serde_json::json!({"mode": "worktree", "commit": "a".repeat(40)}),
    ] {
        report(&child, bad);
        assert_eq!(f.m.child_workspace_of(&child).unwrap(), None);
    }
    let made = serde_json::json!({"mode": "clone", "commit": "b".repeat(40),
        "branch": "feat/x-1", "submodules": true, "large_files": false});
    report(&child, made.clone());
    let kept = f.m.child_workspace_of(&child).unwrap().unwrap();
    assert_eq!(serde_json::to_value(&kept).unwrap(), made);
    report(&child, serde_json::json!({"mode": "scratch"}));
    assert_eq!(
        f.m.child_workspace_of(&child).unwrap(),
        Some(kept),
        "kept once"
    );
    // A top-level session has none, whatever its host says.
    report(&p.id, serde_json::json!({"mode": "scratch"}));
    assert_eq!(f.m.child_workspace_of(&p.id).unwrap(), None);
}

/// Token sync (decision 124) narrows too: a child pulls its parent's synced
/// login only when the parent had one, and never one the parent did not.
#[test]
fn a_child_gets_its_parents_synced_login_and_no_other() {
    const KIND: &str = "claude_subscription_token";
    let dir = tempdir::TempDir::new("storm-start-credential").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (_, mut link) = m.connect_host("hst_A");
    let mut hello = caps(VERIFIED, 16);
    hello.capabilities.provider_credentials = vec![KIND.into()];
    m.hello("hst_A", hello).unwrap();
    for synced in [true, false] {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": if synced { "storm" } else { "other" },
            "provider": "claude-code", "terminal": {"cols": 80, "rows": 24},
            "spawn": true,
        }))
        .unwrap();
        let meta = LaunchMeta {
            provider_credential: synced.then_some(KIND),
            ..LaunchMeta::default()
        };
        let (p, ..) = m
            .launch("usr_A", launch, meta, vec![grant("storm")])
            .unwrap();
        let running = serde_json::from_value(serde_json::json!({"status": "running"})).unwrap();
        m.host_status("hst_A", &p.id, &running).unwrap();
        m.set_spawn(&p.id, true, Some("dev_A")).unwrap().unwrap();
        while link.try_recv().is_ok() {}
        let r = m
            .create_request(
                &p.id,
                NewRequest::Start {
                    task: "t".into(),
                    provider: None,
                    name: None,
                    context_note: None,
                },
            )
            .unwrap()
            .unwrap();
        let (_, child, _) = decided(m.approve_request(&r.id, "dev_A", false).unwrap());
        let child = child.unwrap().id;
        let start = serde_json::to_value(link.try_recv().unwrap()).unwrap();
        assert_eq!(start["type"], "start");
        assert_eq!(
            start.get("provider_credential").and_then(|k| k.as_str()),
            synced.then_some(KIND),
            "{start}"
        );
        assert_eq!(m.requested_credential(&child), synced.then_some(KIND));
        m.report_children_for_tests("hst_A");
    }
}

/// C7 with 134-5: a child's own workspace is no listed one, so a resumed
/// child is a top-level session where its work began, its parent's.
#[test]
fn a_childs_work_began_in_its_parents_workspace() {
    let f = fixture(16);
    let p = f.parent();
    let r = f.start_as(&p.id, None).unwrap();
    let (_, child, _) = decided(f.approve(&r.id, "dev_A"));
    let child = child.unwrap();
    assert_eq!(f.m.origin_workspace(&child.id).unwrap(), "storm");
    assert_eq!(f.m.origin_workspace(&p.id).unwrap(), "storm");
}
