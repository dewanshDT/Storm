//! Decision 144's acceptance tests at the manager: the queue (144a), its
//! evidence (144b), no keystrokes (144c) and its scope (144d). The tool
//! over the gateway, an MCP key and `until: "settled"` are tested in
//! `api.rs`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::McpGrant;
use super::attention::{GatedSignal, IDLE, PERMISSION_PROMPT, USAGE_LIMIT};
use super::cache::Read;
use super::events::{
    self, ChildEvent, EventsRead, IDLE_EVENT_AFTER, MAX_EVENTS_PER_CHILD, NOT_PERMITTED,
};
use super::{AgentError, AgentManager, AgentResult, Command, Envelope, Hello, Launch, LaunchMeta};

fn hello() -> Hello {
    let providers: Vec<_> = ["claude-code", "opencode"]
        .iter()
        .map(|id| serde_json::json!({"id": id, "kind": "cli", "interactions": ["terminal"], "available": true, "mcp": true}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "providers": providers,
        "workspaces": ["storm", "other"],
        "workspace_paths": {"storm": "/w/storm", "other": "/w/other"},
        "max_sessions": 16,
        "mcp_bridge": true,
        "identity": {"state": "verified", "mechanism": "so_peerpidfd", "reason": null},
        "isolation": {"state": "verified", "mechanism": "systemd_units", "reason": null},
    }))
    .unwrap()
}

struct Fixture {
    m: AgentManager,
    dir: tempdir::TempDir,
    generation: u64,
    link: std::sync::Mutex<UnboundedReceiver<Envelope>>,
}

fn fixture() -> Fixture {
    fixture_in(tempdir::TempDir::new("storm-events").unwrap())
}

fn fixture_in(dir: tempdir::TempDir) -> Fixture {
    let m = AgentManager::open(dir.path()).unwrap();
    let (generation, link) = m.connect_host("hst_A");
    m.hello("hst_A", hello()).unwrap();
    m.verify_isolation_for_tests("hst_A");
    Fixture {
        m,
        dir,
        generation,
        link: std::sync::Mutex::new(link),
    }
}

/// A gate passed for each: the idle and usage signals through the output,
/// the permission prompt through the hooks, all for Claude Code.
const CLAUDE_GATED: &[GatedSignal] = &[
    GatedSignal {
        provider: "claude-code",
        signal: PERMISSION_PROMPT,
        source: events::PROVIDER_HOOK,
    },
    GatedSignal {
        provider: "claude-code",
        signal: IDLE,
        source: events::OUTPUT_PATTERN,
    },
    GatedSignal {
        provider: "claude-code",
        signal: USAGE_LIMIT,
        source: events::OUTPUT_PATTERN,
    },
];

impl Fixture {
    fn launch_as(&self, provider: &str, workspace: &str, spawn: bool) -> String {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": provider,
            "terminal": {"cols": 80, "rows": 24}, "spawn": spawn,
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
        self.report(&r.id, serde_json::json!({"status": "running"}));
        r.id
    }

    fn parent(&self) -> String {
        self.launch_as("claude-code", "storm", true)
    }

    fn report(&self, id: &str, body: serde_json::Value) {
        self.m
            .host_status("hst_A", id, &serde_json::from_value(body).unwrap())
            .unwrap();
    }

    /// A child of `parent` on `provider`, left `starting` as an approval
    /// leaves one.
    fn child_of(&self, parent: &str, provider: &str) -> String {
        let mut r = self.m.get(parent).unwrap();
        let id = crate::auth::identity::random_id("ags_");
        r.id = id.clone();
        r.workspace = format!("children/{id}");
        r.provider = provider.into();
        r.status = "starting".into();
        r.title = None;
        r.harness_session = None;
        self.m
            .store
            .lock()
            .unwrap()
            .insert_spawned(&r, parent, "a task", "2026-10-27T10:00:00Z")
            .unwrap();
        self.m.report_children_for_tests("hst_A");
        id
    }

    fn running_child(&self, parent: &str, provider: &str) -> String {
        let c = self.child_of(parent, provider);
        self.report(&c, serde_json::json!({"status": "running"}));
        c
    }

    fn events(&self, parent: &str) -> Vec<ChildEvent> {
        self.m.events_of_for_tests(parent)
    }

    fn kinds(&self, parent: &str) -> Vec<String> {
        self.events(parent).into_iter().map(|e| e.kind).collect()
    }

    fn kinds_of(&self, parent: &str, child: &str) -> Vec<String> {
        self.events(parent)
            .into_iter()
            .filter(|e| e.child == child)
            .map(|e| e.kind)
            .collect()
    }

    async fn wait(
        &self,
        caller: &str,
        ids: &[String],
        after: Option<i64>,
        bound: Duration,
    ) -> AgentResult<EventsRead> {
        self.m
            .wait_events(caller, ids, after, bound, &|| true)
            .await
    }

    async fn read(&self, caller: &str, after: Option<i64>) -> EventsRead {
        self.wait(caller, &[], after, Duration::ZERO).await.unwrap()
    }

    fn sent(&self) -> Vec<Command> {
        let mut link = self.link.lock().unwrap();
        std::iter::from_fn(|| link.try_recv().ok())
            .map(|e| e.command)
            .collect()
    }

    fn restart(self) -> (AgentManager, tempdir::TempDir) {
        let Fixture { m, dir, .. } = self;
        drop(m);
        (AgentManager::open(dir.path()).unwrap(), dir)
    }
}

fn not_permitted<T: std::fmt::Debug>(r: AgentResult<T>) -> bool {
    matches!(r, Err(AgentError::Conflict(c)) if c == NOT_PERMITTED)
}

fn ids(read: &EventsRead) -> Vec<String> {
    read.events.iter().map(|e| e.id.clone()).collect()
}

// ---- A-144-1 -----------------------------------------------------------------

#[tokio::test]
async fn a_144_1_a_childs_changes_are_events_in_order_and_wake_its_waiting_parent() {
    let f = Arc::new(fixture());
    let p = f.parent();
    let c = f.child_of(&p, "claude-code");
    f.m.force_spawn_for_tests(&c);
    let g = f.m.spawn_child_for_tests(&c, "a sub-task").id;
    assert!(f.events(&p).is_empty(), "nothing has changed yet");

    let waiter = {
        let f = f.clone();
        let p = p.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let read = f.wait(&p, &[], None, Duration::from_secs(20)).await;
            (read, started.elapsed())
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!waiter.is_finished(), "no event yet");
    f.report(&c, serde_json::json!({"status": "running"}));
    let (read, took) = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("the first event ended the wait")
        .unwrap();
    let read = read.unwrap();
    assert!(took < Duration::from_secs(3), "{took:?}");
    assert!(!read.timed_out);
    assert_eq!(read.events.len(), 1);
    assert_eq!(read.events[0].kind, events::STARTED);
    assert_eq!(read.events[0].child, c);
    assert_eq!(read.cursor, read.events[0].seq);
    assert_eq!(read.events[0].id, format!("evt_{p}_{}", read.events[0].seq));
    assert!(f.m.result_of(&c).unwrap().is_none(), "not settled");

    // 145a: a message acts at once, so nothing waits for approval and no
    // `needs_approval` comes between.
    f.m.message_child(&c, &g, "look at the tests")
        .unwrap()
        .unwrap();
    f.m.report_result(&c, "succeeded", "done", None).unwrap();
    assert_eq!(f.kinds(&p), [events::STARTED, events::REPORTED]);
    let all = f.events(&p);
    assert_eq!(all[1].reason.as_deref(), Some("succeeded"));
    let seqs: Vec<i64> = all.iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
}

// ---- A-144-2 -----------------------------------------------------------------

#[tokio::test]
async fn a_144_2_one_wait_hears_either_child_and_only_the_callers() {
    let f = Arc::new(fixture());
    let p = f.parent();
    let a = f.running_child(&p, "claude-code");
    let b = f.running_child(&p, "claude-code");
    let q = f.launch_as("claude-code", "other", true);
    let z = f.running_child(&q, "claude-code");
    let first = f.read(&p, None).await;
    assert_eq!(first.events.len(), 2, "a and b started");
    assert!(first.events.iter().all(|e| e.child == a || e.child == b));

    let waiter = {
        let f = f.clone();
        let (p, a, b) = (p.clone(), a.clone(), b.clone());
        let after = first.cursor;
        tokio::spawn(async move {
            f.wait(&p, &[a, b], Some(after), Duration::from_secs(20))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    f.m.report_result(&z, "succeeded", "elsewhere", None)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!waiter.is_finished(), "another parent's child is not news");
    f.m.report_result(&b, "partial", "half", None).unwrap();
    let read = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("b's event ended it")
        .unwrap()
        .unwrap();
    assert_eq!(read.events.len(), 1);
    assert_eq!(read.events[0].child, b);

    // With ids empty: every child of the caller's, and no one else's.
    let everything = f.read(&p, Some(0)).await;
    assert_eq!(everything.events.len(), 3);
    assert!(everything.events.iter().all(|e| e.child != z));
    assert_eq!(f.kinds(&q), [events::STARTED, events::REPORTED]);
}

// ---- A-144-3 -----------------------------------------------------------------

#[tokio::test]
async fn a_144_3_with_no_events_it_times_out_at_the_cap_never_later() {
    let cap = super::scope::wait_cap("claude-code");
    assert_eq!(
        super::scope::wait_bound("claude-code", Some(Duration::from_secs(3600))),
        cap,
        "a longer ask is held to the cap"
    );
    let f = fixture();
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    let cursor = f.read(&p, None).await.cursor;
    let started = Instant::now();
    let read = f
        .wait(
            &p,
            std::slice::from_ref(&c),
            Some(cursor),
            Duration::from_millis(700),
        )
        .await
        .unwrap();
    let took = started.elapsed();
    assert!(read.timed_out);
    assert!(read.events.is_empty());
    assert_eq!(read.cursor, cursor, "nothing read, nothing moved");
    assert!(took >= Duration::from_millis(700), "{took:?}");
    assert!(took < Duration::from_millis(1500), "{took:?}");
}

// ---- A-144-4 -----------------------------------------------------------------

#[test]
fn a_144_4_identical_reports_write_nothing_and_each_idle_spell_one_event() {
    let f = fixture();
    f.m.gate_signals_for_tests(CLAUDE_GATED);
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    for _ in 0..5 {
        f.report(
            &c,
            serde_json::json!({"status": "running", "activity": "working"}),
        );
    }
    assert_eq!(f.kinds(&p), [events::STARTED], "identical reports");

    let idle = serde_json::json!({"status": "running", "activity": "idle"});
    let working = serde_json::json!({"status": "running", "activity": "working"});
    let idle_timeouts = || {
        f.kinds_of(&p, &c)
            .iter()
            .filter(|k| *k == events::IDLE_TIMEOUT)
            .count()
    };
    f.report(&c, idle.clone());
    f.m.advance_clock_for_tests(IDLE_EVENT_AFTER / 2);
    assert_eq!(f.m.fire_idle_timeouts().unwrap(), 0, "not yet");
    f.m.advance_clock_for_tests(IDLE_EVENT_AFTER);
    assert_eq!(f.m.fire_idle_timeouts().unwrap(), 1);
    f.report(&c, idle.clone());
    f.m.advance_clock_for_tests(IDLE_EVENT_AFTER * 2);
    assert_eq!(f.m.fire_idle_timeouts().unwrap(), 0, "once per spell");
    f.report(&c, working);
    f.report(&c, idle);
    f.m.advance_clock_for_tests(IDLE_EVENT_AFTER + Duration::from_secs(1));
    assert_eq!(f.m.fire_idle_timeouts().unwrap(), 1, "working re-armed it");
    assert_eq!(idle_timeouts(), 2);
    let last = f.events(&p).pop().unwrap();
    assert_eq!(
        (last.source.as_str(), last.confidence.as_str()),
        (events::OUTPUT_PATTERN, "heuristic")
    );
    assert_eq!(last.state.activity.as_deref(), Some("idle"));

    // A child that has reported gets none.
    let r = f.running_child(&p, "claude-code");
    f.m.report_result(&r, "succeeded", "done", None).unwrap();
    f.report(
        &r,
        serde_json::json!({"status": "running", "activity": "idle"}),
    );
    f.m.advance_clock_for_tests(IDLE_EVENT_AFTER * 2);
    f.m.fire_idle_timeouts().unwrap();
    assert_eq!(f.kinds_of(&p, &r), [events::STARTED, events::REPORTED]);
}

#[test]
fn a_144_4_the_per_child_cap_keeps_the_terminal_events() {
    let f = fixture();
    f.m.gate_signals_for_tests(CLAUDE_GATED);
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    for _ in 0..(MAX_EVENTS_PER_CHILD + 10) {
        f.report(
            &c,
            serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT}),
        );
        f.report(
            &c,
            serde_json::json!({"status": "running", "attention": null}),
        );
    }
    assert_eq!(f.events(&p).len() as i64, MAX_EVENTS_PER_CHILD);
    f.m.report_result(&c, "blocked", "stuck", None).unwrap();
    f.report(
        &c,
        serde_json::json!({"status": "completed", "exit_code": 0}),
    );
    let kinds = f.kinds(&p);
    assert_eq!(kinds.len() as i64, MAX_EVENTS_PER_CHILD + 2);
    assert_eq!(kinds[kinds.len() - 2..], [events::REPORTED, events::ENDED]);
}

// ---- A-144-5 -----------------------------------------------------------------

#[tokio::test]
async fn a_144_5_unacknowledged_events_come_back_and_survive_a_restart() {
    let f = fixture();
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    f.m.report_result(&c, "succeeded", "done", None).unwrap();
    let first = f.read(&p, None).await;
    assert_eq!(first.events.len(), 2);
    let again = f.read(&p, None).await;
    assert_eq!(ids(&again), ids(&first), "a reconnect gets them again");

    let (m, _dir) = f.restart();
    let read = m
        .wait_events(&p, &[], None, Duration::ZERO, &|| true)
        .await
        .unwrap();
    assert_eq!(
        ids(&read)[..2],
        ids(&first)[..],
        "unacknowledged, across a restart, with the same ids"
    );
    assert_eq!(read.events[2].kind, events::UNKNOWN, "the restart itself");
    assert_eq!(read.events[2].source, events::STORM);

    let acked = m
        .wait_events(&p, &[], Some(first.cursor), Duration::ZERO, &|| true)
        .await
        .unwrap();
    assert_eq!(ids(&acked), ids(&read)[2..]);
    let none = m
        .wait_events(&p, &[], None, Duration::ZERO, &|| true)
        .await
        .unwrap();
    assert_eq!(
        ids(&none),
        ids(&read)[2..],
        "acknowledged ones never come back"
    );
}

// ---- A-144-6 -----------------------------------------------------------------

#[tokio::test]
async fn a_144_6_the_same_after_returns_the_same_events_and_reading_writes_none() {
    let f = fixture();
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    f.m.report_result(&c, "succeeded", "done", None).unwrap();
    f.report(
        &c,
        serde_json::json!({"status": "completed", "exit_code": 0}),
    );
    let before = f.events(&p);
    for after in [0, 1] {
        let one = f.read(&p, Some(after)).await;
        let two = f.read(&p, Some(after)).await;
        assert_eq!(one.events, two.events, "after {after}");
        assert_eq!(one.cursor, two.cursor);
    }
    f.read(&p, None).await;
    assert_eq!(f.events(&p), before, "reading created nothing");
}

// ---- A-144-7 -----------------------------------------------------------------

#[tokio::test]
async fn a_144_7_events_are_read_only_by_their_parent() {
    let f = fixture();
    let root = f.parent();
    let child = f.running_child(&root, "claude-code");
    f.m.force_spawn_for_tests(&child);
    let grandchild = f.m.spawn_child_for_tests(&child, "deeper").id;
    let sibling = f.running_child(&root, "claude-code");
    f.m.force_spawn_for_tests(&sibling);
    let other = f.launch_as("claude-code", "other", true);
    let stranger = f.running_child(&other, "claude-code");
    f.m.report_result(&grandchild, "succeeded", "g", None)
        .unwrap();
    f.m.report_result(&child, "succeeded", "c", None).unwrap();
    f.m.report_result(&stranger, "succeeded", "s", None)
        .unwrap();
    let bound = Duration::from_millis(50);

    for (caller, target) in [
        (&sibling, &child),    // a sibling
        (&grandchild, &child), // a child's child, at its parent
        (&grandchild, &root),  // and its grandparent
        (&other, &child),      // an unrelated session
        (&other, &grandchild),
        (&child, &root), // a child, at its parent
        (&child, &sibling),
        (&root, &stranger),
        (&root, &root),
    ] {
        let r = f
            .wait(caller, std::slice::from_ref(target), None, bound)
            .await;
        assert!(not_permitted(r), "{caller} read {target}");
        let r = f
            .wait(caller, std::slice::from_ref(target), Some(0), bound)
            .await;
        assert!(not_permitted(r), "{caller} read {target} after 0");
    }
    // Each session's own queue holds only its own children's events.
    let own = |caller: &str| {
        let f = &f;
        let caller = caller.to_string();
        async move {
            f.wait(&caller, &[], Some(0), bound)
                .await
                .unwrap()
                .events
                .into_iter()
                .map(|e| e.child)
                .collect::<std::collections::BTreeSet<_>>()
        }
    };
    assert_eq!(own(&root).await, [child.clone(), sibling.clone()].into());
    assert_eq!(own(&child).await, [grandchild.clone()].into());
    assert!(own(&sibling).await.is_empty());
    assert!(own(&grandchild).await.is_empty());
    assert_eq!(own(&other).await, [stranger.clone()].into());
}

// ---- A-144-8 -----------------------------------------------------------------

#[tokio::test]
async fn a_144_8_no_child_text_is_ever_in_an_event() {
    const CANARY: &str = "CANARY-144-8";
    let f = fixture();
    f.m.gate_signals_for_tests(CLAUDE_GATED);
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    f.m.force_spawn_for_tests(&c);
    let g = f.m.spawn_child_for_tests(&c, "deeper").id;
    f.m.message_child(&c, &g, &format!("{CANARY} message"))
        .unwrap()
        .unwrap();
    f.m.message_child(&p, &c, &format!("{CANARY} from the parent"))
        .unwrap()
        .unwrap();
    f.m.report_result(
        &c,
        "blocked",
        &format!("{CANARY} summary"),
        Some(&format!("{CANARY} details")),
    )
    .unwrap();
    f.m.report_result(&g, "failed", &format!("{CANARY} g"), Some(CANARY))
        .unwrap();
    f.report(
        &c,
        serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT, "title": CANARY}),
    );
    f.report(
        &c,
        serde_json::json!({"status": "failed", "end_reason": "crashed"}),
    );

    let mut scanned = 0;
    for queue in [&p, &c, &g] {
        let read = f.read(queue, Some(0)).await;
        let text = serde_json::to_string(&read.events).unwrap();
        assert!(!text.contains(CANARY), "{queue}: {text}");
        scanned += read.events.len();
        for e in &read.events {
            let recent = serde_json::to_string(&f.m.recent_events(&e.child).unwrap()).unwrap();
            assert!(!recent.contains(CANARY), "{recent}");
        }
    }
    assert!(scanned >= 6, "the scan read the events: {scanned}");
    let raw: Vec<String> = {
        let store = f.m.store.lock().unwrap();
        let mut stmt = store
            .conn
            .prepare("SELECT kind || ' ' || coalesce(reason, '') FROM child_events")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(raw.iter().all(|r| !r.contains(CANARY)), "{raw:?}");
}

// ---- A-144-9 -----------------------------------------------------------------

#[test]
fn a_144_9_unknown_is_never_healthy() {
    let f = fixture();
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    f.report(
        &c,
        serde_json::json!({"status": "running", "activity": "working"}),
    );
    let r = f.m.get(&c).unwrap();
    assert_eq!(
        f.m.health_activity(&r).as_deref(),
        Some("unknown"),
        "no gated idle signal"
    );
    assert_eq!(
        r.activity.as_deref(),
        Some("working"),
        "the terminal title's display is as before (AM42)"
    );
    assert_eq!(f.events(&p)[0].state.activity.as_deref(), Some("unknown"));

    f.m.gate_signals_for_tests(CLAUDE_GATED);
    f.report(
        &c,
        serde_json::json!({"status": "running", "activity": "working"}),
    );
    let r = f.m.get(&c).unwrap();
    assert_eq!(f.m.health_activity(&r).as_deref(), Some("working"));

    f.m.disconnect_host("hst_A", f.generation);
    let r = f.m.get(&c).unwrap();
    assert_eq!(r.status, "unknown");
    assert_eq!(f.m.health_activity(&r).as_deref(), Some("unknown"));
    let lost = f.events(&p).pop().unwrap();
    assert_eq!(lost.kind, events::UNKNOWN);
    assert_eq!(lost.child, c);
    assert_eq!(
        (lost.source.as_str(), lost.confidence.as_str()),
        (events::HOST_PROCESS, "certain")
    );
    assert_eq!(lost.state.status, "unknown");
    assert_eq!(lost.state.activity.as_deref(), Some("unknown"));
}

// ---- A-144-10 ----------------------------------------------------------------

#[test]
fn a_144_10_each_event_says_its_source_and_confidence() {
    let f = fixture();
    let p = f.parent();
    let failed = f.child_of(&p, "claude-code");
    f.report(&failed, serde_json::json!({"status": "failed"}));
    let e = f.events(&p).pop().unwrap();
    assert_eq!(
        (
            e.kind.as_str(),
            e.reason.as_deref(),
            e.source.as_str(),
            e.confidence.as_str()
        ),
        (
            events::FAILED,
            Some("start_failure"),
            events::HOST_PROCESS,
            "certain"
        )
    );

    let ungated = f.running_child(&p, "opencode");
    let c = f.running_child(&p, "claude-code");
    let before = f.events(&p).len();
    for (id, body) in [
        (
            &ungated,
            serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT}),
        ),
        (
            &c,
            serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT}),
        ),
        (
            &c,
            serde_json::json!({"status": "running", "attention": USAGE_LIMIT}),
        ),
    ] {
        f.report(id, body);
    }
    assert_eq!(f.events(&p).len(), before, "no gate, no event");
    assert_eq!(f.m.needs_attention(&f.m.get(&c).unwrap()).unwrap(), None);

    f.m.gate_signals_for_tests(CLAUDE_GATED);
    f.report(
        &ungated,
        serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT}),
    );
    f.report(
        &c,
        serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT, "attention_source": "output_pattern"}),
    );
    assert_eq!(
        f.events(&p).len(),
        before,
        "another provider, another source"
    );

    f.report(
        &c,
        serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT}),
    );
    let e = f.events(&p).pop().unwrap();
    assert_eq!(
        (
            e.kind.as_str(),
            e.reason.as_deref(),
            e.source.as_str(),
            e.confidence.as_str()
        ),
        (
            events::NEEDS_ATTENTION,
            Some(PERMISSION_PROMPT),
            events::PROVIDER_HOOK,
            "reported"
        )
    );
    assert_eq!(e.state.needs_attention.as_deref(), Some(PERMISSION_PROMPT));

    f.report(
        &c,
        serde_json::json!({"status": "running", "attention": USAGE_LIMIT, "attention_source": "output_pattern"}),
    );
    let e = f.events(&p).pop().unwrap();
    assert_eq!(
        (e.kind.as_str(), e.source.as_str(), e.confidence.as_str()),
        (
            events::USAGE_LIMIT_EVENT,
            events::OUTPUT_PATTERN,
            "heuristic"
        )
    );
    assert_eq!(
        f.m.needs_attention(&f.m.get(&c).unwrap()).unwrap(),
        Some(USAGE_LIMIT)
    );

    f.m.report_result(&c, "succeeded", "done", None).unwrap();
    let e = f.events(&p).pop().unwrap();
    assert_eq!(
        (e.source.as_str(), e.confidence.as_str()),
        (events::STORM, "certain")
    );
}

// ---- A-144-11 ----------------------------------------------------------------

#[test]
fn a_144_11_a_hook_report_has_no_authority() {
    let f = fixture();
    f.m.gate_signals_for_tests(CLAUDE_GATED);
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    f.m.force_spawn_for_tests(&c);
    f.m.spawn_child_for_tests(&c, "deeper");
    f.m.report_result(&c, "blocked", "stuck", None).unwrap();
    f.sent();
    let events_before = f.events(&p);

    for attention in [
        serde_json::json!("approved"),
        serde_json::json!("approval"),
        serde_json::json!("denied"),
        serde_json::json!("blocked"),
        serde_json::json!("unblocked"),
        serde_json::json!("end"),
        serde_json::json!("start"),
        serde_json::json!(null),
    ] {
        for _ in 0..2 {
            f.report(
                &c,
                serde_json::json!({"status": "running", "attention": attention, "attention_source": "provider_hook"}),
            );
        }
    }
    // Another host's report for the session is refused outright.
    let forged: super::StatusReport = serde_json::from_value(
        serde_json::json!({"status": "completed", "attention": PERMISSION_PROMPT}),
    )
    .unwrap();
    assert!(matches!(
        f.m.host_status("hst_B", &c, &forged),
        Err(AgentError::NotYours)
    ));

    assert!(f.m.requests_of(&c).unwrap().is_empty(), "nor acted for it");
    assert_eq!(f.m.result_of(&c).unwrap().unwrap().outcome, "blocked");
    assert_eq!(f.m.get(&c).unwrap().status, "running", "nor ended it");
    assert_eq!(f.m.children_of(&c).unwrap().len(), 1, "nor started one");
    assert!(
        f.sent().is_empty(),
        "a report sent nothing to any session or host"
    );
    assert_eq!(f.events(&p), events_before, "an ungated word is no event");
}

// ---- A-144-12 ----------------------------------------------------------------

/// The lines of `text` that write to, or name, a terminal. Spelled so this
/// file's own text does not match its search.
fn terminal_writes(text: &str) -> Vec<String> {
    let forbidden = [
        ["Command", "::Input"].concat(),
        ["Command", "::Resize"].concat(),
        ["Command", "::Replay"].concat(),
        [".se", "nd("].concat(),
        ["send_", "input"].concat(),
        ["write_", "input"].concat(),
    ];
    let pty = ["pt", "y"].concat();
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or_default();
        let names_a_terminal = code
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .map(str::to_lowercase)
            .any(|w| {
                w == pty || w.starts_with(&format!("{pty}_")) || w.ends_with(&format!("_{pty}"))
            });
        if names_a_terminal || forbidden.iter().any(|f| code.contains(f.as_str())) {
            found.push(format!("{}: {}", n + 1, line.trim()));
        }
    }
    found
}

#[test]
fn a_144_12_no_event_or_wake_code_writes_to_a_terminal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent");
    let mut offences = Vec::new();
    for file in ["events.rs", "attention.rs"] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        for hit in terminal_writes(&text) {
            offences.push(format!("{file}:{hit}"));
        }
    }
    assert!(
        offences.is_empty(),
        "an event or wake path reaches a terminal:\n{}",
        offences.join("\n")
    );
    assert!(events::WAKE.is_empty(), "no wake has passed its gate");
    for provider in ["claude-code", "opencode", "codex", "shell"] {
        assert!(!events::wake_on(provider), "{provider}");
    }
}

/// The scan finds what it is for, so a silent pass means something.
#[test]
fn a_144_12_the_scan_finds_a_planted_write() {
    for planted in [
        "self.send(&r.host_id, cmd)?;",
        "let c = Command::Input { session, data };",
        "host.write_input(&session, bytes)",
        "pty_write(fd, b\"wake\")",
        "let writer = session.pty.clone();",
    ] {
        assert_eq!(terminal_writes(planted).len(), 1, "{planted}");
    }
    assert!(terminal_writes("if events.is_empty() { self.events.send_modify(f) }").is_empty());
}

#[tokio::test]
async fn a_144_12_events_move_no_terminal_and_wait_for_the_parents_next_call() {
    let f = fixture();
    f.m.gate_signals_for_tests(CLAUDE_GATED);
    let p = f.parent();
    f.report(
        &p,
        serde_json::json!({"status": "running", "activity": "idle"}),
    );
    f.m.host_output("hst_A", &p, 0, b"the parent's own output")
        .unwrap();
    let output = |m: &AgentManager| match m.read_output(&p, 0, 1 << 16) {
        Read::Data { from, bytes } => (from, bytes),
        other => panic!("{other:?}"),
    };
    let before = output(&f.m);
    f.sent();

    let c = f.running_child(&p, "claude-code");
    f.report(
        &c,
        serde_json::json!({"status": "running", "attention": PERMISSION_PROMPT}),
    );
    f.m.report_result(&c, "succeeded", "done", None).unwrap();
    f.report(
        &c,
        serde_json::json!({"status": "completed", "exit_code": 0}),
    );

    assert_eq!(output(&f.m), before, "the parent's output did not move");
    assert!(
        matches!(
            f.m.read_output(&p, before.1.len() as u64, 1 << 16),
            Read::UpToDate
        ),
        "nothing past its end"
    );
    let pushed: Vec<_> = f
        .sent()
        .into_iter()
        .filter(|c| c.session() == Some(p.as_str()))
        .map(|c| c.kind())
        .collect();
    assert!(
        pushed.is_empty(),
        "nothing pushed to the parent: {pushed:?}"
    );
    let read = f.read(&p, None).await;
    assert_eq!(
        read.events
            .iter()
            .map(|e| e.kind.as_str())
            .collect::<Vec<_>>(),
        [
            events::STARTED,
            events::NEEDS_ATTENTION,
            events::REPORTED,
            events::ENDED
        ]
    );
}

// ---- retention ---------------------------------------------------------------

#[test]
fn a_tree_dismissed_takes_its_queue_with_it() {
    let f = fixture();
    let p = f.parent();
    let c = f.running_child(&p, "claude-code");
    f.m.report_result(&c, "succeeded", "done", None).unwrap();
    assert_eq!(f.events(&p).len(), 2);
    f.m.store
        .lock()
        .unwrap()
        .delete_all(&[p.clone(), c.clone()])
        .unwrap();
    assert!(f.events(&p).is_empty());
    assert_eq!(f.m.pending_events(&c).unwrap(), 0);
    let left: i64 =
        f.m.store
            .lock()
            .unwrap()
            .conn
            .query_row("SELECT COUNT(*) FROM child_event_cursor", [], |r| r.get(0))
            .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn pending_events_count_what_the_parent_has_not_acknowledged() {
    let f = fixture();
    let p = f.parent();
    let a = f.running_child(&p, "claude-code");
    let b = f.running_child(&p, "claude-code");
    f.m.report_result(&a, "succeeded", "done", None).unwrap();
    assert_eq!(f.m.pending_events(&a).unwrap(), 2);
    assert_eq!(f.m.pending_events(&b).unwrap(), 1);
    let read = f.read(&p, None).await;
    assert_eq!(
        f.m.pending_events(&a).unwrap(),
        2,
        "read is not acknowledged"
    );
    f.read(&p, Some(read.cursor)).await;
    assert_eq!(f.m.pending_events(&a).unwrap(), 0);
    assert_eq!(f.m.pending_events(&b).unwrap(), 0);
    f.read(&p, Some(i64::MAX)).await;
    f.report(
        &b,
        serde_json::json!({"status": "completed", "exit_code": 0}),
    );
    assert_eq!(
        f.m.pending_events(&b).unwrap(),
        1,
        "acknowledging past the end acknowledges nothing to come"
    );
}
