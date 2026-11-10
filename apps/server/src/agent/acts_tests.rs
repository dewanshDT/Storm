//! Decision 134-3 as amended by 145a: a parent's start, message and end act
//! at once, with every check 134 makes (C0, the switch, scope, the caps,
//! AP-1, AP-3, AP-7) and an act row for each. The tools are tested in
//! `api.rs`.

use std::sync::Barrier;
use std::time::Duration;

use rand::Rng;
use tokio::sync::mpsc::UnboundedReceiver;

use super::store::SessionRecord;
use super::store::spawn::{
    DEFAULT_CHILDREN, DEFAULT_TREE, HostLimits, HostNow, Refusal, Request, StartPayload,
};
use super::{AgentManager, Command, Envelope, Hello, Launch, LaunchMeta, McpGrant};

const VERIFIED: &str = r#"{"state": "verified", "mechanism": "so_peerpidfd", "reason": null}"#;
const UNAVAILABLE: &str = r#"{"state": "unavailable", "reason": "self_test"}"#;

/// A sandbox host whose isolation is verified, with `identity` as given.
fn caps(identity: &str, max_sessions: u32) -> Hello {
    let mut hello = unisolated(identity, max_sessions);
    hello.capabilities.isolation = crate::agent::Isolation {
        state: "verified".into(),
        mechanism: Some("systemd_units".into()),
        reason: None,
    };
    hello.capabilities.execution = Some(crate::agent::Execution {
        account: Some("storm-runtime".into()),
        isolation: "per-session".into(),
    });
    hello.capabilities.child_workspaces = true;
    hello
}

/// A host with isolation off (151 N6): it says who it runs as, starts a
/// child in a workspace of its own listing, and has Codex installed.
fn off_host(identity: &str, max_sessions: u32) -> Hello {
    let mut hello = unisolated(identity, max_sessions);
    hello.capabilities.execution = Some(crate::agent::Execution {
        account: Some("dewansh".into()),
        isolation: "off".into(),
    });
    for provider in &mut hello.capabilities.providers {
        if provider.id == "codex" {
            provider.available = true;
        }
    }
    hello.capabilities.resume = vec!["claude-code".into()];
    hello
}

/// [`fixture`] on a host with isolation off, whose identity is not
/// verified: identity is not a gate (R1.1).
fn off_fixture(max_sessions: u32) -> Fixture {
    let dir = tempdir::TempDir::new("storm-off-host").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (_, link) = m.connect_host("hst_A");
    m.hello("hst_A", off_host(UNAVAILABLE, max_sessions))
        .unwrap();
    Fixture { m, _dir: dir, link }
}

/// A host too old to say who it runs as or that it isolates its sessions.
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
        self.try_launch(workspace, write_vault, spawn, false)
            .unwrap()
    }

    fn try_launch(
        &self,
        workspace: &str,
        write_vault: Option<&str>,
        spawn: bool,
        children_write: bool,
    ) -> Result<SessionRecord, super::AgentError> {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": "claude-code",
            "terminal": {"cols": 80, "rows": 24}, "spawn": spawn,
            "children_write": children_write,
        }))
        .unwrap();
        let meta = LaunchMeta {
            write_vault_id: write_vault.map(Into::into),
            ..LaunchMeta::default()
        };
        let (r, ..) = self
            .m
            .launch("usr_A", launch, meta, vec![grant("storm"), grant("con_X")])?;
        self.report(&r.id, "running");
        Ok(r)
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

    /// [Self::parent] with its children's writes ticked (151m).
    fn writing_parent(&self) -> SessionRecord {
        self.try_launch("storm", Some("vlt_W"), true, true).unwrap()
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

    // C0, as the live link says now. Identity is not a gate (R1.1).
    f.told();
    f.m.lose_isolation_for_tests("hst_A");
    assert_eq!(f.start(&p.id), Err(because("isolation_unverified")));
    f.m.inventory("hst_A", unisolated(VERIFIED, 16).capabilities)
        .unwrap();
    assert_eq!(f.start(&p.id), Err(because("host_too_old")));
    assert_eq!(
        f.m.end_child(&p.id, &c).unwrap(),
        Err(because("host_too_old"))
    );
    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert_eq!(f.m.message_child(&p.id, &c, "again").unwrap(), Ok(1));
    // An ended requester.
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

fn set_limits(f: &Fixture, children: i64, tree: i64, sessions: Option<i64>) -> HostLimits {
    f.m.set_host_limits(
        "hst_A",
        &HostLimits {
            max_children: children,
            max_tree: tree,
            max_sessions: sessions,
        },
        "usr_A",
    )
    .unwrap()
}

/// 152b, 152c: with no row a host has today's caps.
#[test]
fn a_host_without_limits_has_the_defaults() {
    let f = fixture(16);
    assert_eq!(
        f.m.host_limits("hst_A").unwrap(),
        HostLimits {
            max_children: 3,
            max_tree: 6,
            max_sessions: None
        }
    );
    let p = f.parent();
    for _ in 0..DEFAULT_CHILDREN {
        f.start(&p.id).unwrap();
    }
    assert_eq!(f.start(&p.id), Err(limit("children")));
}

/// 152b, 152c: a host's own Limits refuse the start past each, and only on
/// that host.
#[test]
fn a_hosts_limits_refuse_the_start_past_them() {
    let f = fixture(16);
    let was = set_limits(&f, 1, 6, None);
    assert_eq!(was, HostLimits::default());
    let p = f.parent();
    f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("children")));

    let f = fixture(16);
    set_limits(&f, 5, 3, None);
    f.m.set_host_limits(
        "hst_B",
        &HostLimits {
            max_children: 1,
            max_tree: 1,
            max_sessions: Some(1),
        },
        "usr_A",
    )
    .unwrap();
    let p = f.parent();
    f.start(&p.id).unwrap();
    f.start(&p.id).unwrap();
    assert_eq!(
        f.start(&p.id),
        Err(limit("tree")),
        "three, the root included"
    );
    let other_tree = f.parent_in("other");
    assert!(f.start(&other_tree).is_ok(), "another tree is not counted");
}

/// 152f: lowering ends and parks nothing, and refuses only new starts;
/// raising is at once.
#[test]
fn lowering_a_limit_leaves_running_children_alone() {
    let mut f = fixture(16);
    let p = f.parent();
    let kids: Vec<String> = (0..3).map(|_| f.start(&p.id).unwrap().id).collect();
    for k in &kids {
        f.report(k, "running");
    }
    while f.link.try_recv().is_ok() {}
    set_limits(&f, 1, 2, None);
    assert!(
        f.link.try_recv().is_err(),
        "the host is told nothing (152h)"
    );
    for k in &kids {
        assert_eq!(f.m.get(k).unwrap().status, "running", "{k}");
    }
    assert_eq!(f.start(&p.id), Err(limit("children")));
    set_limits(&f, 5, 10, None);
    assert!(f.start(&p.id).is_ok());
}

/// 152d: the override only lowers what the host reports, for a person's
/// launch as for a start.
#[test]
fn the_sessions_override_only_lowers_the_hosts_own() {
    let lim = |over| HostLimits {
        max_sessions: over,
        ..HostLimits::default()
    };
    assert_eq!(lim(Some(10)).effective_sessions(4), 4);
    assert_eq!(lim(Some(2)).effective_sessions(4), 2);
    assert_eq!(lim(None).effective_sessions(4), 4);
    assert_eq!(lim(Some(2)).effective_sessions(0), 2, "lower than no limit");
    assert_eq!(lim(None).effective_sessions(0), 0);

    let f = fixture(4);
    set_limits(&f, 3, 6, Some(2));
    let p = f.parent();
    f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("max_sessions")));
    assert!(matches!(
        f.try_launch("other", None, false, false),
        Err(super::AgentError::TooManySessions)
    ));

    let f = fixture(3);
    set_limits(&f, 3, 6, Some(10));
    let p = f.parent();
    f.start(&p.id).unwrap();
    f.start(&p.id).unwrap();
    assert_eq!(
        f.start(&p.id),
        Err(limit("max_sessions")),
        "the host's 3, not 10"
    );
}

/// 152a: a revoked host keeps no Limits.
#[test]
fn revoking_a_host_clears_its_limits() {
    let f = fixture(16);
    set_limits(&f, 1, 1, Some(1));
    f.m.revoke_host("hst_A").unwrap();
    assert_eq!(f.m.host_limits("hst_A").unwrap(), HostLimits::default());
    let rows: i64 =
        f.m.store
            .lock()
            .unwrap()
            .conn
            .query_row("SELECT COUNT(*) FROM host_limits", [], |r| r.get(0))
            .unwrap();
    assert_eq!(rows, 0);
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
                DEFAULT_CHILDREN - 1,
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
                DEFAULT_TREE - 5,
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
    // 151m: its parent's launch did not tick its children's writes.
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
}

/// 151m, 151 X9: ticked at launch, a start inherits the write vault its
/// parent launched with, and nothing wider.
#[test]
fn a_start_ticked_at_launch_inherits_only_its_parents_write_vault() {
    let f = fixture(16);
    let p = f.writing_parent();
    let child = f.start(&p.id).unwrap();
    let store = f.m.store.lock().unwrap();
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
}

/// 151m: the tick needs spawn on and a write vault, else nothing launches.
#[test]
fn a_tick_without_spawn_or_a_write_vault_is_refused() {
    let f = fixture(16);
    for (write_vault, spawn) in [(Some("vlt_W"), false), (None, true), (None, false)] {
        let refused = f.try_launch("storm", write_vault, spawn, true).unwrap_err();
        assert!(
            matches!(&refused, super::AgentError::Conflict(c) if c == "children_write_unavailable"),
            "{write_vault:?} {spawn}: {refused:?}"
        );
    }
    assert!(f.m.list().unwrap().is_empty(), "nothing is recorded");
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

    // An identity downgrade is no flip (R1.1); a runtime too old to say
    // who it runs as is.
    f.m.inventory("hst_A", caps(UNAVAILABLE, 16).capabilities)
        .unwrap();
    assert!(f.told().is_empty(), "identity is not a gate");
    f.m.inventory("hst_A", unisolated(VERIFIED, 16).capabilities)
        .unwrap();
    assert_eq!(f.start(&p.id), Err(because("host_too_old")));
    assert_eq!(f.told(), std::slice::from_ref(&p.id), "spawn on only, once");
    assert!(!f.told().contains(&quiet.id));
    f.m.inventory("hst_A", unisolated(VERIFIED, 16).capabilities)
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
    let mut hello = caps(VERIFIED, 16);
    hello.capabilities.isolation = crate::agent::Isolation::unavailable("self_test");
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
    assert_eq!(launch.write_vault_id, None, "not ticked at launch");
    assert_eq!(launch.name, "Test writer");
    assert_eq!(launch.model.as_deref(), Some("haiku"));
}

/// 151m: ticked at launch, the child is started with its parent's write
/// vault.
#[test]
fn a_ticked_child_is_started_with_its_parents_write_vault() {
    let mut f = fixture(16);
    let p = f.writing_parent();
    f.starts();
    let child = f.start(&p.id).unwrap();
    assert_eq!(f.starts().len(), 1);
    let launch = f.m.launch_of(&child.id).unwrap().unwrap();
    assert_eq!(
        launch.write_vault_id.as_deref(),
        Some("vlt_W"),
        "its parent's"
    );
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

/// 151 N5: Codex, installed with Storm's servers, runs as a child like any
/// other agent: it is offered, and a start naming it reaches the host.
#[test]
fn codex_runs_as_a_child() {
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
        f.m.startable_agents(&p.id).unwrap(),
        ["claude-code", "opencode", "codex"]
    );
    let child = f.start_as(&p.id, Some("codex")).unwrap();
    assert_eq!(child.provider, "codex");
    assert_eq!(f.starts().len(), 1);
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

// ---- 151 N6: spawning on a host with isolation off --------------------------

impl Fixture {
    fn start_in(&self, requester: &str, cwd: &str) -> Result<SessionRecord, Refusal> {
        self.start_with(
            requester,
            StartPayload {
                task: "write the tests".into(),
                cwd: Some(cwd.into()),
                ..StartPayload::default()
            },
        )
    }
}

/// With the switch on, a session on a host with isolation off starts a
/// child of each spawning agent in its own workspace: no copy, no
/// `children/` record, no `cwd` sent, and spawn off for the child.
#[test]
fn off_the_sandbox_a_child_starts_in_its_parents_workspace() {
    let mut f = off_fixture(16);
    let p = f.parent();
    f.starts();
    assert_eq!(
        f.m.startable_agents(&p.id).unwrap(),
        ["claude-code", "opencode", "codex"]
    );
    for agent in ["claude-code", "opencode", "codex"] {
        let child = f.start_as(&p.id, Some(agent)).unwrap();
        let starts = f.starts();
        assert_eq!(starts.len(), 1, "{agent}: {starts:?}");
        let start = &starts[0];
        assert_eq!(
            (&start["workspace"], &start["provider"], &start["child_of"]),
            (
                &serde_json::json!("storm"),
                &serde_json::json!(agent),
                &serde_json::json!(p.id)
            )
        );
        assert!(start.get("cwd").is_none(), "{start}");
        let c = f.m.get(&child.id).unwrap();
        assert_eq!(c.workspace, "storm", "{agent}: its parent's, by name");
        assert!(!super::store::spawn::has_own_workspace(&c));
        assert_eq!(f.m.launch_of(&c.id).unwrap().unwrap().cwd, None);
        assert!(!f.m.spawn_enabled(&c.id).unwrap(), "spawn is off");
        f.m.end_child(&p.id, &c.id).unwrap().unwrap();
        f.report(&c.id, "completed");
        f.starts();
    }
}

/// With the switch off nothing starts, there or anywhere: the act is
/// refused and the tools are not offered.
#[test]
fn off_the_sandbox_the_switch_off_starts_nothing() {
    let mut f = off_fixture(16);
    let without = f.launch("storm", Some("vlt_W"));
    assert!(!f.m.spawn_offered(&without.id), "off by default");
    assert_eq!(f.start(&without.id), Err(because("spawn_off")));
    let p = f.parent();
    assert!(f.m.spawn_offered(&p.id));
    f.m.set_spawn(&p.id, false, Some("dev_A")).unwrap().unwrap();
    assert!(!f.m.spawn_offered(&p.id));
    f.starts();
    assert_eq!(f.start(&p.id), Err(because("spawn_off")));
    assert!(f.starts().is_empty(), "nothing reaches the host");
    assert!(f.children(&p.id).is_empty());
}

/// C4's caps and the built depth hold on a host with isolation off, as on
/// the sandbox (R3.4).
#[test]
fn off_the_sandbox_each_cap_and_the_depth_hold() {
    let f = off_fixture(16);
    let p = f.parent();
    let c = f.child(&p.id, "ags_C", true);
    assert_eq!(f.start(&c), Err(limit("depth")), "a child starts none");
    let started = f.start(&p.id).unwrap();
    assert_eq!(f.start(&started.id), Err(because("spawn_off")));

    let f = off_fixture(16);
    let p = f.parent();
    f.child(&p.id, "ags_C1", false);
    f.child(&p.id, "ags_C2", false);
    f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("children")));
    f.report("ags_C1", "completed");
    assert!(f.start(&p.id).is_ok());

    let f = off_fixture(16);
    let p = f.parent();
    let c1 = f.child(&p.id, "ags_C1", false);
    f.child(&c1, "ags_G1", false);
    f.child(&c1, "ags_G2", false);
    f.child(&c1, "ags_G3", false);
    f.start(&p.id).unwrap();
    assert_eq!(f.start(&p.id), Err(limit("tree")));

    let f = off_fixture(3);
    let p = f.parent();
    f.launch("other", None);
    assert!(f.start(&p.id).is_ok());
    assert_eq!(f.start(&p.id), Err(limit("max_sessions")));
}

/// R3.3: a `cwd` starts the child there only inside a listed workspace's
/// canonical directory, written as is; anything else is `bad_cwd`, and on
/// a sandbox host any `cwd` is `cwd_not_applicable`. A refusal records and
/// sends nothing.
#[test]
fn a_cwd_is_a_directory_inside_a_listed_workspace_or_refused() {
    let mut f = off_fixture(16);
    let p = f.parent();
    f.starts();
    for (cwd, workspace) in [
        ("/w/storm", "storm"),
        ("/w/storm/crates/core", "storm"),
        ("/w/other/docs", "other"),
    ] {
        let child = f.start_in(&p.id, cwd).unwrap();
        let start = f.starts().pop().unwrap();
        assert_eq!(start["cwd"], cwd);
        let c = f.m.get(&child.id).unwrap();
        assert_eq!(c.workspace, workspace, "{cwd}");
        assert_eq!(
            f.m.launch_of(&c.id).unwrap().unwrap().cwd.as_deref(),
            Some(cwd)
        );
        f.m.end_child(&p.id, &c.id).unwrap().unwrap();
        f.starts();
    }
    let before = f.children(&p.id).len();
    for cwd in [
        "/w",
        "/w/stormy",
        "/elsewhere",
        "w/storm/x",
        "/w/storm/",
        "/w/storm//x",
        "/w/storm/./x",
        "/w/storm/../other",
        "/w/storm/x/..",
        "/w/storm/x\n",
        "/w/storm/\u{0}",
        "/c/ags_C1",
    ] {
        assert_eq!(f.start_in(&p.id, cwd), Err(because("bad_cwd")), "{cwd:?}");
    }
    let long = format!(
        "/w/storm/{}",
        "x".repeat(super::store::spawn::MAX_CWD_BYTES)
    );
    assert_eq!(f.start_in(&p.id, &long), Err(because("bad_cwd")));
    assert_eq!(f.children(&p.id).len(), before, "nothing recorded");
    assert!(f.starts().is_empty(), "nothing reaches the host");

    let f = fixture(16);
    let p = f.parent();
    assert_eq!(
        f.start_in(&p.id, "/w/storm/x"),
        Err(because("cwd_not_applicable"))
    );
}

#[test]
fn a_cwd_matches_the_deepest_workspace_and_either_separator() {
    use super::store::spawn::cwd_workspace;
    let paths = |p: &[(&str, &str)]| -> Vec<(String, String)> {
        p.iter()
            .map(|(n, d)| (n.to_string(), d.to_string()))
            .collect()
    };
    let nested = paths(&[("storm", "/w/storm"), ("site", "/w/storm/site")]);
    assert_eq!(
        cwd_workspace("/w/storm/site/a", &nested).as_deref(),
        Some("site")
    );
    assert_eq!(
        cwd_workspace("/w/storm/a", &nested).as_deref(),
        Some("storm")
    );
    let windows = paths(&[("storm", "\\\\?\\C:\\w\\storm")]);
    assert_eq!(
        cwd_workspace("\\\\?\\C:\\w\\storm\\src", &windows).as_deref(),
        Some("storm")
    );
    assert_eq!(cwd_workspace("\\\\?\\C:\\w\\storm\\..\\x", &windows), None);
    assert_eq!(cwd_workspace("/w/storm", &paths(&[("x", "")])), None);
}

/// A resumed child starts where it did; once that directory is gone from
/// its host's workspaces, the resume is refused.
#[test]
fn a_resume_keeps_its_cwd_and_is_refused_once_it_is_gone() {
    let mut f = off_fixture(16);
    let p = f.parent();
    let child = f.start_in(&p.id, "/w/other/docs").unwrap();
    let report = serde_json::from_value(serde_json::json!({"status": "running",
        "harness_session": "0f8fad5b-d9cb-469f-a165-70867728950e"}))
    .unwrap();
    f.m.host_status("hst_A", &child.id, &report).unwrap();
    f.report(&child.id, "completed");
    f.starts();
    let resume = |f: &Fixture| {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": "storm", "provider": "claude-code",
            "terminal": {"cols": 80, "rows": 24}, "resume_of": child.id,
        }))
        .unwrap();
        f.m.launch("usr_A", launch, LaunchMeta::default(), vec![grant("storm")])
            .map(|(r, l, ..)| (r, l))
    };
    let (r, launch) = resume(&f).unwrap();
    assert_eq!(launch.cwd.as_deref(), Some("/w/other/docs"));
    let start = f.starts().pop().unwrap();
    assert_eq!(
        (&start["session"], &start["cwd"]),
        (
            &serde_json::json!(r.id),
            &serde_json::json!("/w/other/docs")
        )
    );
    f.report(&r.id, "completed");

    let mut gone = off_host(UNAVAILABLE, 16);
    gone.capabilities.workspace_paths.remove("other");
    f.m.inventory("hst_A", gone.capabilities).unwrap();
    assert!(matches!(
        resume(&f),
        Err(super::AgentError::Conflict(why)) if why == "working directory no longer exists"
    ));
}
