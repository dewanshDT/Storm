//! Decision 134-3 as amended by 145a: a parent's start, message and end act
//! at once, with every check 134 makes (C0, the switch, scope, the caps,
//! AP-1, AP-3, AP-7) and an act row for each. The tools are tested in
//! `api.rs`.

use std::sync::Barrier;
use std::time::Duration;

use rand::Rng;
use tokio::sync::mpsc::UnboundedReceiver;

use super::store::SessionRecord;
use super::store::spawn::{HostNow, MAX_CHILDREN, MAX_TREE, Refusal, Request, StartPayload};
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
    // Codex is not installed, Cursor gets no Storm servers (130a).
    let providers: Vec<_> = [
        ("claude-code", true, Some(true)),
        ("opencode", true, Some(true)),
        ("shell", true, None),
        ("codex", false, Some(true)),
        ("cursor", true, Some(false)),
    ]
    .iter()
    .map(|(id, available, mcp)| {
        serde_json::json!({"id": id, "kind": "cli",
        "interactions": ["terminal"], "available": available, "mcp": mcp})
    })
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

    fn start(&self, requester: &str) -> Result<SessionRecord, Refusal> {
        self.start_with(
            requester,
            StartPayload {
                task: "write the tests".into(),
                ..StartPayload::default()
            },
        )
    }

    fn start_with(&self, requester: &str, payload: StartPayload) -> Result<SessionRecord, Refusal> {
        self.m.start_child(requester, payload, None).unwrap()
    }

    fn start_as(&self, requester: &str, provider: Option<&str>) -> Result<SessionRecord, Refusal> {
        self.start_with(
            requester,
            StartPayload {
                task: "write the tests".into(),
                provider: provider.map(Into::into),
                name: Some("Test writer".into()),
                ..StartPayload::default()
            },
        )
    }

    fn acts(&self, requester: &str) -> Vec<Request> {
        self.m.requests_of(requester).unwrap()
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

    /// The `start` commands sent down the host's link since the last look.
    fn starts(&mut self) -> Vec<serde_json::Value> {
        std::iter::from_fn(|| self.link.try_recv().ok())
            .map(|e| serde_json::to_value(e).unwrap())
            .filter(|m| m["type"] == "start")
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

    fn parent_in(&self, workspace: &str) -> String {
        self.launch_as(workspace, None, true).id
    }

    fn children(&self, parent: &str) -> Vec<String> {
        self.m
            .store
            .lock()
            .unwrap()
            .descendants(parent, false)
            .unwrap()
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

// ---- A-145-1: no approval ------------------------------------------------------

/// A-145-1: with the switch on, a start, a message and an end each act at
/// once and leave a decided act row, never one awaiting a decision; with
/// it off each is refused `spawn_off`.
#[test]
fn a_145_1_start_message_and_end_act_at_once_with_no_request_waiting() {
    let f = fixture(16);
    let p = f.parent();
    let child = f.start(&p.id).unwrap();
    assert_eq!(child.status, "starting", "its host was told at once");
    let round = f.m.message_child(&p.id, &child.id, "and then").unwrap();
    assert_eq!(round, Ok(1));
    assert_eq!(f.m.end_child(&p.id, &child.id).unwrap(), Ok(()));
    let acts = f.acts(&p.id);
    assert_eq!(
        acts.iter()
            .map(|a| (a.kind.as_str(), outcome(a)))
            .collect::<Vec<_>>(),
        [
            ("start", ("decided", Some("started"), None)),
            ("message", ("decided", Some("delivered"), None)),
            ("end", ("decided", Some("ended"), None)),
        ]
    );
    for a in &acts {
        assert!(a.id.starts_with("agr_"), "{}", a.id);
        assert_eq!(a.decided_by_device, None, "no device decided it");
    }
    assert_eq!(acts[0].session.as_deref(), Some(child.id.as_str()));
    assert_eq!(acts[0].payload, r#"{"task":"write the tests"}"#);
    let pending: i64 =
        f.m.store
            .lock()
            .unwrap()
            .conn
            .query_row(
                "SELECT COUNT(*) FROM agent_requests WHERE state != 'decided'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(pending, 0);
    let inbox = f.m.inbox_of(&child.id).unwrap();
    assert_eq!(
        (inbox[0].text.as_str(), inbox[0].state.as_str()),
        ("and then", "queued")
    );

    // Off: every act is refused at once, and nothing is written.
    let q = f.parent_in("other");
    let c = f.start(&q).unwrap();
    f.m.set_spawn(&q, false, Some("dev_A")).unwrap().unwrap();
    assert_eq!(f.start(&q).unwrap_err(), because("spawn_off"));
    assert_eq!(
        f.m.message_child(&q, &c.id, "hi").unwrap(),
        Err(because("spawn_off"))
    );
    assert_eq!(f.m.end_child(&q, &c.id).unwrap(), Err(because("spawn_off")));
    assert_eq!(f.acts(&q).len(), 1);

    // Over the size limits is a bad request, never a row.
    assert!(matches!(
        f.m.start_child(
            &p.id,
            StartPayload {
                task: "x".repeat(16 * 1024 + 1),
                ..StartPayload::default()
            },
            None
        ),
        Err(super::AgentError::BadRequest(_))
    ));
    let c2 = f.start(&p.id).unwrap();
    assert!(matches!(
        f.m.message_child(&p.id, &c2.id, &"é".repeat(4097)),
        Err(super::AgentError::BadRequest(_))
    ));
}

/// An older server's request left waiting is expired once at open, its
/// message with it, and stays readable in the log.
#[test]
fn a_request_an_older_server_left_waiting_expires_at_open() {
    let f = fixture(16);
    let p = f.parent();
    let c = f.child(&p.id, "ags_C", false);
    for (id, kind, target, payload) in [
        ("agr_S", "start", None, r#"{"task":"t"}"#),
        ("agr_M", "message", Some(c.as_str()), r#"{"text":"hi"}"#),
    ] {
        f.m.insert_request_for_tests(&Request {
            id: id.into(),
            kind: kind.into(),
            requester: p.id.clone(),
            target: target.map(Into::into),
            payload: payload.into(),
            created_at: "2026-10-25T10:00:00Z".into(),
            expires_at: "2026-10-25T10:10:00Z".into(),
            state: "pending".into(),
            decided_by_device: None,
            decided_at: None,
            outcome: None,
            reason: None,
            session: None,
        });
    }
    f.raw(
        "INSERT INTO session_inbox (session_id, seq, sender, text, request_id, state, at)
         VALUES (?1, 1, ?2, 'hi', 'agr_M', 'pending_approval', '2026-10-25T10:00:00Z')",
        [&c, &p.id],
    );
    let Fixture { m, _dir: dir, link } = f;
    drop((m, link));
    let m = AgentManager::open(dir.path()).unwrap();
    let acts = m.requests_of(&p.id).unwrap();
    assert_eq!(acts.len(), 2);
    for a in &acts {
        assert_eq!(outcome(a), ("decided", Some("expired"), None));
    }
    assert_eq!(m.inbox_of(&c).unwrap()[0].state, "expired");
}

#[test]
fn acts_need_c0_spawn_on_a_live_requester_and_a_descendant_target() {
    let mut f = fixture(16);
    let p = f.parent();
    let off = f.launch("other", None);
    assert_eq!(f.start(&off.id), Err(because("spawn_off")), "never on");
    // Message and end name a live descendant, or are not found.
    let c = f.child(&p.id, "ags_C", false);
    let stranger = f.launch("other", None);
    for target in [stranger.id.as_str(), "ags_NOPE", p.id.as_str()] {
        assert_eq!(
            f.m.end_child(&p.id, target).unwrap(),
            Err(because("not_found")),
            "{target}"
        );
        assert_eq!(
            f.m.message_child(&p.id, target, "hi").unwrap(),
            Err(because("not_found")),
            "{target}"
        );
    }
    assert_eq!(f.m.message_child(&p.id, &c, "hi").unwrap(), Ok(1));
    // A child cannot address its parent.
    f.m.store
        .lock()
        .unwrap()
        .set_spawn(&c, true, None, "2026-10-25T10:00:00Z")
        .unwrap();
    assert_eq!(f.m.end_child(&c, &p.id).unwrap(), Err(because("not_found")));

    // C0, as the live link says now.
    f.told();
    f.m.lose_isolation_for_tests("hst_A");
    assert_eq!(f.start(&p.id), Err(because("isolation_unverified")));
    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert_eq!(f.start(&p.id), Err(because("identity_unverified")));
    assert_eq!(
        f.m.end_child(&p.id, &c).unwrap(),
        Err(because("identity_unverified"))
    );
    // An ended requester.
    f.m.inventory("hst_A", caps(VERIFIED, 16).capabilities)
        .unwrap();
    f.m.verify_isolation_for_tests("hst_A");
    f.report(&p.id, "completed");
    assert_eq!(f.start(&p.id), Err(because("requester_ended")));
    assert!(matches!(
        f.m.end_child("ags_NOPE", &c),
        Err(super::AgentError::NotFound(_))
    ));
}

// ---- C4: the caps, and AP-1 under concurrency --------------------------------

/// A-145-2: every cap of 134's C4 still holds with no approval: a fourth
/// live child, a grandchild and a sixth live session in a tree are refused,
/// each naming its cap, as is the host's max_sessions.
#[test]
fn a_145_2_each_cap_refuses_at_its_boundary_and_names_itself() {
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

    // Live children per parent: 3.
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
    f.child(&c1, "ags_G3", false);
    f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("tree")));
    let other_tree = f.parent_in("other");
    assert!(f.start(&other_tree).is_ok(), "another tree is not counted");

    // The host's max_sessions.
    let f = fixture(3);
    let p = f.parent();
    f.launch("other", None);
    assert!(f.start(&p.id).is_ok());
    assert_eq!(f.start(&p.id), Err(limit("max_sessions")));
}

/// AP-1: concurrent starts at each cap's boundary take exactly the free
/// slots; the rest are refused, naming the cap.
#[test]
fn ap1_concurrent_starts_take_exactly_the_free_slots() {
    const RACERS: usize = 8;
    for round in 0..10 {
        // (setup, requesters, free slots, the cap the rest hit)
        type Setup = fn(&Fixture) -> Vec<String>;
        let cases: [(Setup, u32, i64, &str); 4] = [
            (
                |f| {
                    let p = f.parent();
                    f.child(&p.id, "ags_C1", false);
                    vec![p.id; RACERS]
                },
                16,
                MAX_CHILDREN - 1,
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
                    // children cap.
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
            let results: Vec<Result<SessionRecord, Refusal>> = std::thread::scope(|s| {
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

// ---- the start ---------------------------------------------------------------

#[test]
fn a_start_records_the_child_narrowed_under_its_parent() {
    let f = fixture(16);
    let p = f.parent();
    let child = f.start(&p.id).unwrap();
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
    // 145a: a start inherits the write vault its parent launched with,
    // and nothing wider.
    assert_eq!(
        store
            .launch(&child.id)
            .unwrap()
            .unwrap()
            .write_vault_id
            .as_deref(),
        Some("vlt_W")
    );
    assert!(matches!(
        store.grant(&child.id, "storm"),
        Ok(Some((_, true)))
    ));
    // 134-5: it is started in its own workspace, which its host makes from
    // the parent's.
    assert_eq!(
        (c.status.as_str(), c.end_reason.as_deref()),
        ("starting", None)
    );
    assert_eq!(c.workspace, format!("children/{}", child.id));
}

/// AP-7: a grant the parent lost before the start never reaches the child.
#[test]
fn ap7_a_grant_the_parent_lost_never_reaches_the_child() {
    let f = fixture(16);
    let p = f.parent();
    f.m.revoke_grants_for("con_X").unwrap();
    let child = f.start(&p.id).unwrap();
    let store = f.m.store.lock().unwrap();
    let ids: Vec<String> = store
        .grants_of(&child.id)
        .unwrap()
        .into_iter()
        .map(|g| g.id)
        .collect();
    assert_eq!(ids, ["storm"]);
    assert_eq!(store.grant(&child.id, "con_X").unwrap(), None);
}

/// A-145-4, the store's half: a name is unique among the parent's live
/// children only; one made from the task is numbered among them.
#[test]
fn a_145_4_a_name_is_unique_among_live_siblings() {
    let f = fixture(16);
    let p = f.parent();
    let named = |name: &str| StartPayload {
        task: "write a poem about the sea".into(),
        name: Some(name.into()),
        ..StartPayload::default()
    };
    let poet = f.start_with(&p.id, named("poet")).unwrap();
    assert_eq!(f.m.launch_of(&poet.id).unwrap().unwrap().name, "poet");
    assert_eq!(
        f.start_with(&p.id, named("poet")),
        Err(because("name_taken"))
    );
    // Another parent's child may be called the same.
    let q = f.parent_in("other");
    assert!(f.start_with(&q, named("poet")).is_ok());
    // An ended child frees its name.
    f.report(&poet.id, "completed");
    assert!(f.start_with(&p.id, named("poet")).is_ok());
    // Without one, the task names it.
    let plain = StartPayload {
        task: "write a poem about the sea".into(),
        ..StartPayload::default()
    };
    let a = f.start_with(&p.id, plain.clone()).unwrap();
    let b = f.start_with(&p.id, plain).unwrap();
    assert_eq!(
        (
            f.m.launch_of(&a.id).unwrap().unwrap().name,
            f.m.launch_of(&b.id).unwrap().unwrap().name
        ),
        ("write-a-poem-about".into(), "write-a-poem-about-2".into())
    );
}

/// AP-3: a child whose parent the person ends between its start's commit
/// and its `start` going out is never sent `end` first, so the host cannot
/// drop the `end` and then start it.
#[test]
fn ap3_an_end_never_precedes_its_childs_start() {
    let mut f = fixture(16);
    let p = f.parent();
    let now = crate::index::now_rfc3339();
    let child =
        f.m.store
            .lock()
            .unwrap()
            .start_child(
                "agr_1",
                &p.id,
                &StartPayload {
                    task: "t".into(),
                    ..StartPayload::default()
                },
                &now,
                &|_| {
                    Ok(HostNow {
                        max_sessions: 16,
                        ..HostNow::default()
                    })
                },
            )
            .unwrap()
            .unwrap();
    assert_eq!(child.status, "creating");
    f.m.end_by_person(&p.id, Some(std::slice::from_ref(&child.id)), false)
        .unwrap()
        .unwrap();
    assert!(f.m.stop_cascade_targets(&now).unwrap().is_empty());

    f.m.dispatch_spawned(&child, None).unwrap();
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

/// An end acts at once: its target is sent `end`.
#[test]
fn an_end_sends_end_at_once() {
    let mut f = fixture(16);
    let p = f.parent();
    let c = f.start(&p.id).unwrap();
    f.starts();
    f.m.end_child(&p.id, &c.id).unwrap().unwrap();
    let ends: Vec<String> = std::iter::from_fn(|| f.link.try_recv().ok())
        .filter_map(|e| match e.command {
            Command::End { session } => Some(session),
            _ => None,
        })
        .collect();
    assert_eq!(ends, [c.id]);
}

// ---- C0 on the live link --------------------------------------------------------

/// A host whose identity or isolation drops refuses the next act at once,
/// and its spawning agents are told their tools changed, once per flip.
#[test]
fn a_host_downgrade_refuses_the_next_act_and_tells_spawning_agents() {
    let mut f = fixture(16);
    let p = f.parent();
    let quiet = f.launch("other", None);
    f.told();

    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert_eq!(f.start(&p.id), Err(because("identity_unverified")));
    assert_eq!(f.told(), std::slice::from_ref(&p.id), "spawn on only, once");
    assert!(!f.told().contains(&quiet.id));
    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert!(f.told().is_empty(), "no flip, no notification");

    f.m.inventory("hst_A", caps(VERIFIED, 16).capabilities)
        .unwrap();
    assert_eq!(f.told(), std::slice::from_ref(&p.id));
    assert!(f.start(&p.id).is_ok());
    f.told();

    f.m.lose_isolation_for_tests("hst_A");
    assert_eq!(f.start(&p.id), Err(because("isolation_unverified")));
    assert_eq!(f.told(), std::slice::from_ref(&p.id));
    f.m.verify_isolation_for_tests("hst_A");
    assert_eq!(f.told(), std::slice::from_ref(&p.id));

    // A new link that has not said it is isolated refuses at its hello.
    let (generation, _) = f.m.connect_host("hst_A");
    f.m.disconnect_host("hst_A", generation);
    let (_, mut link) = f.m.connect_host("hst_A");
    let mut hello = unisolated(VERIFIED, 16);
    hello.sessions =
        vec![serde_json::from_value(serde_json::json!({"id": p.id, "status": "running"})).unwrap()];
    f.m.hello("hst_A", hello).unwrap();
    assert_eq!(f.start(&p.id), Err(because("isolation_unverified")));
    let told: Vec<String> = std::iter::from_fn(|| link.try_recv().ok())
        .map(|e| serde_json::to_value(e).unwrap())
        .filter(|m| m["message"]["method"] == "notifications/tools/list_changed")
        .map(|m| m["session"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(told, std::slice::from_ref(&p.id), "told on the new link");
}

/// A session whose adapter cannot spawn is refused even with its switch on
/// (the switch route never allows it; this is the store's own check).
#[test]
fn an_adapter_that_cannot_spawn_is_refused() {
    let f = fixture(16);
    let p = f.parent();
    f.raw(
        "UPDATE sessions SET provider = 'shell' WHERE id = ?1",
        [&p.id],
    );
    assert_eq!(f.start(&p.id), Err(because("not_permitted")));
}

// ---- races ---------------------------------------------------------------------

/// A start racing the switch going off and the parent's own end: a child
/// exists only if its start committed first, and then its parent's end has
/// it in a cascade, so none outlives it (AP-2, AP-3).
#[test]
fn a_start_racing_the_switch_and_its_parents_end_leaves_no_orphan() {
    for round in 0..200 {
        let f = fixture(16);
        let p = f.parent();
        let mut jitter = [0u8; 3];
        rand::rng().fill_bytes(&mut jitter);
        let barrier = Barrier::new(3);
        let wait = |i: usize| {
            barrier.wait();
            std::thread::sleep(Duration::from_micros(u64::from(jitter[i]) * 4));
        };
        let started = std::thread::scope(|s| {
            let a = s.spawn(|| {
                wait(0);
                f.start(&p.id).is_ok()
            });
            s.spawn(|| {
                wait(1);
                f.m.set_spawn(&p.id, false, Some("dev_D")).unwrap().unwrap();
            });
            s.spawn(|| {
                wait(2);
                f.report(&p.id, "completed");
            });
            a.join().unwrap()
        });
        assert_eq!(
            f.children(&p.id).len(),
            usize::from(started),
            "round {round}"
        );
        let store = f.m.store.lock().unwrap();
        for child in store.descendants(&p.id, true).unwrap() {
            assert!(
                store.doomed(&child).unwrap(),
                "round {round}: {child} outlives its parent"
            );
        }
    }
}

// ---- 134-5: the child's start ---------------------------------------------------

/// 134e and the checks table's narrowing row, on the wire: the host is told
/// to start the child with exactly the parent's live grants, the parent's
/// agent on the model asked for (145b), on the parent's host, from the
/// parent's workspace into one of its own, as the parent's child, with the
/// spawned prompt's flag, no asks (145a), and nothing else.
#[test]
fn a_child_is_started_narrowed_in_its_own_workspace() {
    let mut f = fixture(16);
    let p = f.parent();
    f.starts();
    f.m.revoke_grants_for("con_X").unwrap();
    let child = f
        .start_with(
            &p.id,
            StartPayload {
                task: "write the tests".into(),
                name: Some("Test writer".into()),
                model: Some("haiku".into()),
                ..StartPayload::default()
            },
        )
        .unwrap();
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
            "context": true, "model": "haiku", "child_of": p.id,
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
    assert_eq!(
        launch.write_vault_id.as_deref(),
        Some("vlt_W"),
        "its parent's"
    );
    assert_eq!(launch.name, "Test writer");
    assert_eq!(launch.model.as_deref(), Some("haiku"));
}

/// 143a, the checks table's provider row: a child runs its parent's agent,
/// or another its host reports installed with Storm's servers; `shell`, an
/// agent not installed, one without Storm's servers (`mcp: false`) and a
/// name the host does not report are refused `agent_unavailable`, as is
/// one the host's live link dropped, and nothing reaches the host.
#[test]
fn a_child_runs_an_agent_its_host_can_start_or_nothing() {
    let mut f = fixture(16);
    let p = f.parent();
    f.starts();
    for other in ["shell", "codex", "cursor", "Claude-Code", "my-agent"] {
        assert_eq!(
            f.start_as(&p.id, Some(other)),
            Err(because("agent_unavailable")),
            "{other}"
        );
    }
    assert!(f.starts().is_empty(), "nothing reaches the host");
    assert_eq!(
        f.m.startable_agents(&p.id).unwrap(),
        ["claude-code", "opencode"]
    );
    assert!(f.start_as(&p.id, Some("claude-code")).is_ok());
    let mut hello = caps(VERIFIED, 16);
    hello
        .capabilities
        .providers
        .retain(|provider| provider.id != "opencode");
    f.m.inventory("hst_A", hello.capabilities).unwrap();
    assert_eq!(
        f.start_as(&p.id, Some("opencode")),
        Err(because("agent_unavailable"))
    );
}

/// 148a (A-148-4): Codex never runs as a child, even installed with Storm's
/// servers: it would stop on a prompt nobody answers. It is not offered,
/// and a start naming it is `agent_unavailable` with nothing sent.
#[test]
fn codex_is_never_a_child() {
    let mut f = fixture(16);
    let mut hello = caps(VERIFIED, 16);
    for provider in &mut hello.capabilities.providers {
        if provider.id == "codex" {
            provider.available = true;
        }
    }
    f.m.inventory("hst_A", hello.capabilities).unwrap();
    let p = f.parent();
    f.starts();
    assert_eq!(
        f.start_as(&p.id, Some("codex")),
        Err(because("agent_unavailable"))
    );
    assert!(f.starts().is_empty());
    assert_eq!(
        f.m.startable_agents(&p.id).unwrap(),
        ["claude-code", "opencode"]
    );
}

/// 143a, 143d: a Claude Code parent's OpenCode child starts as OpenCode,
/// with the parent's grants, and is named no synced login even when the
/// account has one for that host: the host's own setup for OpenCode
/// applies (136f).
#[test]
fn a_claude_code_parent_starts_an_opencode_child_on_the_hosts_own_setup() {
    const KIND: &str = "claude_subscription_token";
    let mut f = fixture(16);
    let mut hello = caps(VERIFIED, 16);
    hello.capabilities.provider_credentials = vec![KIND.into()];
    f.m.inventory("hst_A", hello.capabilities).unwrap();
    let p = f.parent();
    f.starts();
    let child =
        f.m.start_child(
            &p.id,
            StartPayload {
                task: "t".into(),
                provider: Some("opencode".into()),
                ..StartPayload::default()
            },
            Some(KIND),
        )
        .unwrap()
        .unwrap();
    let starts = f.starts();
    assert_eq!(starts.len(), 1, "{starts:?}");
    let mut start = starts[0].clone();
    start.as_object_mut().unwrap().remove("cmd_seq");
    assert_eq!(
        start,
        serde_json::json!({
            "type": "start", "session": child.id, "workspace": "storm",
            "provider": "opencode", "interaction": "terminal",
            "terminal": {"cols": 80, "rows": 24},
            "mcp": [{"id": "con_X", "slug": "con_X"}, {"id": "storm", "slug": "storm"}],
            "context": true, "child_of": p.id,
        })
    );
    let c = f.m.get(&child.id).unwrap();
    assert_eq!(
        (
            c.provider.as_str(),
            c.provider_kind.as_str(),
            c.status.as_str()
        ),
        ("opencode", "cli", "starting")
    );
    assert_eq!(f.m.requested_credential(&c.id), None);
}

/// A child whose host takes no command after commit is its own `failed:
/// start_failure`, and its act keeps `started`.
#[test]
fn a_child_its_host_cannot_be_told_of_fails_on_its_own_record() {
    let mut f = fixture(16);
    let p = f.parent();
    let (_, gone) = tokio::sync::mpsc::unbounded_channel();
    drop(std::mem::replace(&mut f.link, gone));
    let child = f.start_as(&p.id, None).unwrap();
    assert_eq!(f.acts(&p.id)[0].outcome.as_deref(), Some("started"));
    let c = f.m.get(&child.id).unwrap();
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
    let child = f.start_as(&p.id, None).unwrap();
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
        f.acts(&p.id)[0].outcome.as_deref(),
        Some("started"),
        "the act stands"
    );
}

/// What a child's host says it made is kept on the child's record once:
/// only for a session with a parent, and only in the shape a helper
/// answers (a hex commit, a branch of the clone's own characters).
#[test]
fn a_childs_workspace_is_kept_as_its_host_made_it() {
    let f = fixture(16);
    let p = f.parent();
    let child = f.start_as(&p.id, None).unwrap().id;
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

/// 143d: a child's credential is what a person's own launch of its agent
/// on its host would get, whatever its parent's. An OpenCode parent's Claude
/// Code child is named the synced login when the account delivers one to
/// that host, and none otherwise (the host's 136f path then); an OpenCode
/// child never is.
#[test]
fn a_child_gets_the_synced_login_a_launch_of_its_agent_would() {
    const KIND: &str = "claude_subscription_token";
    let dir = tempdir::TempDir::new("storm-start-credential").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (_, mut link) = m.connect_host("hst_A");
    let mut hello = caps(VERIFIED, 16);
    hello.capabilities.provider_credentials = vec![KIND.into()];
    m.hello("hst_A", hello).unwrap();
    let cases = [
        ("claude-code", true, Some(KIND)),
        ("claude-code", false, None),
        ("opencode", true, None),
    ];
    for (agent, synced, named) in cases {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": "storm",
            "provider": "opencode", "terminal": {"cols": 80, "rows": 24},
            "spawn": true,
        }))
        .unwrap();
        let (p, ..) = m
            .launch("usr_A", launch, LaunchMeta::default(), vec![grant("storm")])
            .unwrap();
        assert_eq!(m.requested_credential(&p.id), None, "the parent has none");
        let running = serde_json::from_value(serde_json::json!({"status": "running"})).unwrap();
        m.host_status("hst_A", &p.id, &running).unwrap();
        while link.try_recv().is_ok() {}
        let deliverable = synced.then_some(KIND);
        let child = m
            .start_child(
                &p.id,
                StartPayload {
                    task: "t".into(),
                    provider: Some(agent.into()),
                    ..StartPayload::default()
                },
                deliverable,
            )
            .unwrap()
            .unwrap()
            .id;
        let start = serde_json::to_value(link.try_recv().unwrap()).unwrap();
        assert_eq!(start["type"], "start");
        assert_eq!(start["provider"], agent);
        assert_eq!(
            start.get("provider_credential").and_then(|k| k.as_str()),
            named,
            "{start}"
        );
        assert_eq!(m.requested_credential(&child), named);
        m.report_children_for_tests("hst_A");
    }
}

/// C7 with 134-5: a child's own workspace is no listed one, so a resumed
/// child is a top-level session where its work began, its parent's.
#[test]
fn a_childs_work_began_in_its_parents_workspace() {
    let f = fixture(16);
    let p = f.parent();
    let child = f.start_as(&p.id, None).unwrap();
    assert_eq!(f.m.origin_workspace(&child.id).unwrap(), "storm");
    assert_eq!(f.m.origin_workspace(&p.id).unwrap(), "storm");
}
