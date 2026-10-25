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
use super::{AgentManager, Envelope, Hello, Launch, LaunchMeta, McpGrant};

const VERIFIED: &str = r#"{"state": "verified", "mechanism": "so_peerpidfd", "reason": null}"#;
const UNAVAILABLE: &str = r#"{"state": "unavailable", "reason": "self_test"}"#;

fn caps(identity: &str, max_sessions: u32) -> Hello {
    let providers: Vec<_> = ["claude-code", "opencode", "shell"]
        .iter()
        .map(|id| serde_json::json!({"id": id, "kind": "cli", "interactions": ["terminal"], "available": true}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "providers": providers,
        "workspaces": ["storm", "other"],
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

/// A host whose identity is verified and, by the test-only hook, isolated.
fn fixture(max_sessions: u32) -> Fixture {
    let dir = tempdir::TempDir::new("storm-approval").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (_, link) = m.connect_host("hst_A");
    m.hello("hst_A", caps(VERIFIED, max_sessions)).unwrap();
    m.verify_isolation_for_tests("hst_A");
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
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": "claude-code",
            "terminal": {"cols": 80, "rows": 24},
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
        let p = self.launch("storm", Some("vlt_W"));
        self.m
            .set_spawn(&p.id, true, Some("dev_A"))
            .unwrap()
            .unwrap();
        p
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
        id.into()
    }

    fn start(&self, requester: &str) -> Result<Request, Refusal> {
        self.m
            .create_request(
                requester,
                NewRequest::Start {
                    task: "write the tests".into(),
                    provider: None,
                },
            )
            .unwrap()
    }

    fn approve(&self, id: &str, device: &str) -> Decision {
        self.m.approve_request(id, device, false).unwrap()
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
        },
    );
    assert_eq!(other.unwrap(), Err(because("not_permitted")));
    let same = f.m.create_request(
        &p.id,
        NewRequest::Start {
            task: "t".into(),
            provider: Some("claude-code".into()),
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
    // Depth 2: a grandchild cannot start one.
    let f = fixture(16);
    let p = f.parent();
    let c = f.child(&p.id, "ags_C", true);
    let g = f.child(&c, "ags_G", true);
    assert!(
        f.start(&c).is_ok(),
        "a child may start a depth-2 grandchild"
    );
    assert_eq!(f.start(&g), Err(limit("depth")));

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

    // Live sessions per tree: 6, the root included.
    let f = fixture(16);
    let p = f.parent();
    let c1 = f.child(&p.id, "ags_C1", true);
    let c2 = f.child(&p.id, "ags_C2", true);
    f.child(&c1, "ags_G1", false);
    f.child(&c1, "ags_G2", false);
    // Five live; one pending start makes six.
    f.start(&c1).unwrap();
    assert_eq!(f.start(&c2), Err(limit("tree")));
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
        let p = self.launch(workspace, None);
        self.m
            .set_spawn(&p.id, true, Some("dev_A"))
            .unwrap()
            .unwrap();
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
                    let c: Vec<String> = (0..3)
                        .map(|i| f.child(&p.id, &format!("ags_C{i}"), true))
                        .collect();
                    // Four live: two free in the tree. Two requests each, so
                    // the pending cap never binds first.
                    (0..6).map(|i| c[i % 3].clone()).collect()
                },
                16,
                MAX_TREE - 4,
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
                    let g = f.child(&c, "ags_G", true);
                    vec![g; RACERS]
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
    // Launching a child is 134-5's: until then the start fails, which
    // frees its slot, and nothing reaches the host.
    assert_eq!(
        (c.status.as_str(), c.end_reason.as_deref()),
        ("failed", Some("start_failure"))
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
                f.m.live
                    .lock()
                    .unwrap()
                    .hosts
                    .get_mut("hst_A")
                    .unwrap()
                    .isolation_verified = false;
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
fn acting_on_an_end_waits_for_its_slice() {
    let f = fixture(16);
    let p = f.parent();
    let c = f.child(&p.id, "ags_C", false);
    let r =
        f.m.create_request(&p.id, NewRequest::End { target: c })
            .unwrap()
            .unwrap();
    assert!(matches!(f.approve(&r.id, "dev_A"), Decision::NotBuilt(_)));
    assert_eq!(
        outcome(&f.stored(&r.id)),
        ("pending", None, None),
        "nothing was written"
    );
    let (denied, ..) = decided(f.m.deny_request(&r.id, "dev_A").unwrap());
    assert_eq!(denied.outcome.as_deref(), Some("denied"));
}

// ---- P-5: expiry -------------------------------------------------------------

/// AP-5: approval after expiry is `expired`, and nothing acts; the sweeper
/// marks what is due and nothing else.
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
    let mut hello = caps(VERIFIED, 16);
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
        // revocation or the end committed, and none is left live.
        assert!(
            f.m.store
                .lock()
                .unwrap()
                .descendants(&p.id, true)
                .unwrap()
                .is_empty()
        );
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
