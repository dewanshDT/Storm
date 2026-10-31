//! Decision 134-6: 134j's inbox states, A2's scope, the cap, C9's notice,
//! LT-13 and 145's rounds, at the manager. The tools over the gateway are
//! tested in `api.rs`.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::store::SessionRecord;
use super::store::spawn::{MAX_INBOX, NOTICE_GRACE_SECS, PARENT_ENDED_NOTICE, Refusal};
use super::{AgentError, AgentManager, Command, Envelope, Hello, Launch, LaunchMeta, McpGrant};

const VERIFIED: &str = r#"{"state": "verified", "mechanism": "so_peerpidfd", "reason": null}"#;

struct Fixture {
    m: AgentManager,
    _dir: tempdir::TempDir,
    generation: u64,
    link: UnboundedReceiver<Envelope>,
}

fn fixture() -> Fixture {
    let dir = tempdir::TempDir::new("storm-inbox").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (generation, link) = m.connect_host("hst_A");
    let hello: Hello = serde_json::from_value(serde_json::json!({
        "providers": [{"id": "claude-code", "kind": "cli", "interactions": ["terminal"], "available": true}],
        "workspaces": ["storm", "other"],
        "workspace_paths": {"storm": "/w/storm", "other": "/w/other"},
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
        generation,
        link,
    }
}

impl Fixture {
    fn launch_as(&self, workspace: &str, spawn: bool) -> SessionRecord {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": "claude-code",
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
        self.report(&r.id, "running");
        r
    }

    fn report(&self, id: &str, status: &str) {
        let report =
            serde_json::from_value(serde_json::json!({"status": status, "exit_code": 0})).unwrap();
        self.m.host_status("hst_A", id, &report).unwrap();
    }

    fn parent(&self, workspace: &str) -> SessionRecord {
        self.launch_as(workspace, true)
    }

    fn child(&self, parent: &str) -> SessionRecord {
        self.m.spawn_child_for_tests(parent, "a task")
    }

    /// A message, at once (145a): the round it belongs to.
    fn message(&self, from: &str, to: &str, text: &str) -> Result<i64, Refusal> {
        self.m.message_child(from, to, text).unwrap()
    }

    fn states(&self, session: &str) -> Vec<(i64, String)> {
        self.m
            .inbox_of(session)
            .unwrap()
            .into_iter()
            .map(|m| (m.seq, m.state))
            .collect()
    }

    async fn read(&self, session: &str, after: Option<i64>) -> Vec<(i64, String)> {
        let (read, _) = self
            .m
            .read_messages(session, after, Duration::ZERO, &|| true)
            .await
            .unwrap();
        read.into_iter().map(|m| (m.seq, m.text)).collect()
    }

    /// Everything that went down the host's link since the last look.
    fn sent(&mut self) -> Vec<Command> {
        std::iter::from_fn(|| self.link.try_recv().ok())
            .map(|e| e.command)
            .collect()
    }

    /// `root` → `child` → `grandchild`, `sibling` under `root`; `other` →
    /// `stranger`. `child` has spawn on, written past the route's WS-9.
    fn trees(&self) -> Trees {
        let root = self.parent("storm");
        let child = self.child(&root.id);
        self.m.force_spawn_for_tests(&child.id);
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

fn after_secs(secs: i64) -> String {
    crate::index::rfc3339_minus_secs(&crate::index::now_rfc3339(), -secs)
}

/// The stop pass, as each child and whether its `end` was sent.
fn stops(m: &AgentManager, now: &str) -> Vec<(String, bool)> {
    m.stop_cascade_targets(now)
        .unwrap()
        .into_iter()
        .map(|s| (s.session_id, s.sent))
        .collect()
}

fn no_terminal_input(sent: &[Command]) {
    for c in sent {
        assert!(
            !matches!(c, Command::Input { .. } | Command::Resize { .. }),
            "{} went to a terminal",
            c.kind()
        );
    }
}

// ---- 134j: states ------------------------------------------------------------

#[tokio::test]
async fn a_message_is_queued_at_once_then_read_and_acknowledged() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    assert_eq!(f.message(&p.id, &c.id, "look at store.rs"), Ok(1));
    assert_eq!(f.states(&c.id), [(1, "queued".to_string())]);
    assert_eq!(
        f.read(&c.id, None).await,
        [(1, "look at store.rs".to_string())]
    );
    assert_eq!(f.states(&c.id), [(1, "delivered".to_string())]);
    // At least once: not acknowledged until the cursor passes it.
    assert_eq!(f.read(&c.id, None).await.len(), 1);
    assert!(f.read(&c.id, Some(1)).await.is_empty());
    assert_eq!(f.states(&c.id), [(1, "acknowledged".to_string())]);
    assert!(f.read(&c.id, None).await.is_empty());

    // The parent sees its message's state, under its act.
    let act = f.m.requests_of(&p.id).unwrap().pop().unwrap();
    let mine = f.m.messages_from(&p.id, &c.id).unwrap();
    assert_eq!(
        mine.iter()
            .map(|m| (m.request_id.as_deref(), m.state.as_str()))
            .collect::<Vec<_>>(),
        [(Some(act.id.as_str()), "acknowledged")]
    );
    no_terminal_input(&f.sent());
}

/// 145: a message after the child's result for its round starts the next
/// round; one before it joins the round it is in.
#[test]
fn a_message_after_a_result_starts_the_next_round() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    assert_eq!(f.message(&p.id, &c.id, "also this"), Ok(1));
    f.m.report_result(&c.id, "succeeded", "round one", None)
        .unwrap();
    assert_eq!(f.message(&p.id, &c.id, "next"), Ok(2));
    assert_eq!(f.message(&p.id, &c.id, "and"), Ok(2), "no result yet");
    f.m.report_result(&c.id, "partial", "round two", None)
        .unwrap();
    assert_eq!(f.message(&p.id, &c.id, "last"), Ok(3));
    let results = f.m.results_of(&c.id).unwrap();
    assert_eq!(
        results
            .iter()
            .map(|r| (r.round, r.summary.as_str()))
            .collect::<Vec<_>>(),
        [(1, "round one"), (2, "round two")]
    );
    assert_eq!(f.m.result_of(&c.id).unwrap().unwrap().round, 2);
    assert_eq!(f.m.round_of(&c.id).unwrap(), 3);
}

#[tokio::test]
async fn a_message_to_a_child_that_ends_first_is_undeliverable() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.message(&p.id, &c.id, "queued, never read").unwrap();
    f.report(&c.id, "completed");
    // L-7: unread when its child ended.
    assert_eq!(
        f.m.inbox_of(&c.id)
            .unwrap()
            .into_iter()
            .map(|m| m.state)
            .collect::<Vec<_>>(),
        ["undeliverable"]
    );
    assert_eq!(
        f.message(&p.id, &c.id, "too late").unwrap_err().reason,
        "not_found"
    );
}

#[tokio::test]
async fn a_negative_cursor_is_refused() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let r =
        f.m.read_messages(&c.id, Some(-1), Duration::ZERO, &|| true)
            .await;
    assert!(matches!(r, Err(AgentError::BadRequest(_))));
}

// ---- A2: the requester's own descendants only ----------------------------------

#[test]
fn a_message_goes_only_to_the_requesters_own_descendants() {
    let f = fixture();
    let t = f.trees();
    for (from, to) in [
        (&t.root, &t.child),
        (&t.root, &t.grandchild),
        (&t.child, &t.grandchild),
    ] {
        assert!(f.message(from, to, "ok").is_ok(), "{from} to {to}");
    }
    for (from, to) in [
        (&t.child, &t.root),     // its parent
        (&t.child, &t.sibling),  // a sibling
        (&t.child, &t.child),    // itself
        (&t.other, &t.child),    // another tree
        (&t.other, &t.root),     // another tree's root
        (&t.child, &t.stranger), // another tree's child
        (&t.child, &"ags_NOPE".to_string()),
    ] {
        assert_eq!(
            f.message(from, to, "no").unwrap_err().reason,
            "not_found",
            "{from} to {to}"
        );
        assert!(
            f.m.messages_from(from, to).unwrap().is_empty(),
            "{to} got it"
        );
    }
    // A session without spawn has no parent tools: a grandchild cannot
    // message anyone.
    assert_eq!(
        f.message(&t.grandchild, &t.child, "no").unwrap_err().reason,
        "spawn_off"
    );
    // An ended descendant is out of scope too.
    f.report(&t.sibling, "completed");
    assert_eq!(
        f.message(&t.root, &t.sibling, "no").unwrap_err().reason,
        "not_found"
    );
    assert_eq!(f.states(&t.grandchild).len(), 2);
    assert!(f.m.inbox_of(&t.stranger).unwrap().is_empty());
}

// ---- the cap ---------------------------------------------------------------------

#[tokio::test]
async fn the_inbox_holds_32_and_keeps_the_last_for_storms_notice() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    for i in 0..MAX_INBOX - 1 {
        f.message(&p.id, &c.id, &format!("m{i}")).unwrap();
    }
    let refused = f.message(&p.id, &c.id, "one too many").unwrap_err();
    assert_eq!((refused.reason, refused.cap), ("limit", Some("inbox")));
    assert_eq!(f.m.inbox_of(&c.id).unwrap().len() as i64, MAX_INBOX - 1);

    // Its parent then ends by itself, and the notice still fits, as the
    // 32nd.
    f.report(&p.id, "completed");
    let inbox = f.m.inbox_of(&c.id).unwrap();
    assert_eq!(inbox.len() as i64, MAX_INBOX);
    assert!(inbox.iter().all(|m| m.state == "queued"));
    assert_eq!(inbox.last().unwrap().sender, "storm");

    // Reading and acknowledging makes room again.
    let read = f.read(&c.id, None).await;
    assert_eq!(read.len() as i64, MAX_INBOX);
    f.read(&c.id, Some(read.last().unwrap().0)).await;
    assert!(
        f.m.inbox_of(&c.id)
            .unwrap()
            .iter()
            .all(|m| m.state != "queued" && m.state != "delivered")
    );
}

// ---- C9 and LT-13 ------------------------------------------------------------------

#[tokio::test]
async fn a_parent_that_ends_by_itself_notices_then_stops_its_children() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.force_spawn_for_tests(&c.id);
    let g = f.child(&c.id);
    f.sent();

    f.report(&p.id, "completed");
    for child in [&c.id, &g.id] {
        let inbox = f.m.inbox_of(child).unwrap();
        assert_eq!(inbox.len(), 1, "{child}");
        assert_eq!(
            (
                inbox[0].sender.as_str(),
                inbox[0].text.as_str(),
                inbox[0].request_id.as_deref(),
                inbox[0].state.as_str()
            ),
            ("storm", PARENT_ENDED_NOTICE, None, "queued")
        );
    }
    assert!(
        !PARENT_ENDED_NOTICE.contains(&p.id),
        "nothing of the parent's"
    );
    assert_eq!(f.read(&c.id, None).await[0].1, PARENT_ENDED_NOTICE);
    // The parent's own end is kept (L-5).
    assert_eq!(f.m.get(&p.id).unwrap().status, "completed");

    // No stop before the grace, and nothing went down the link.
    let early = after_secs(NOTICE_GRACE_SECS - 1);
    assert!(stops(&f.m, &early).is_empty());
    assert!(f.sent().is_empty(), "the notice reaches no terminal");

    let due = after_secs(NOTICE_GRACE_SECS + 1);
    let stopped = stops(&f.m, &due);
    assert_eq!(
        stopped,
        [(c.id.clone(), true), (g.id.clone(), true)],
        "nearest first"
    );
    // Once each: a second pass sends nothing.
    assert!(stops(&f.m, &due).is_empty());
    let sent = f.sent();
    no_terminal_input(&sent);
    assert_eq!(
        sent.iter()
            .map(|c| match c {
                Command::End { session } => session.clone(),
                other => panic!("{} sent", other.kind()),
            })
            .collect::<Vec<_>>(),
        [c.id.clone(), g.id.clone()]
    );

    // The host's report of each stop is recorded as Storm's stop.
    f.report(&c.id, "stopped");
    f.report(&g.id, "failed");
    for child in [&c.id, &g.id] {
        let r = f.m.get(child).unwrap();
        assert_eq!(
            (r.status.as_str(), r.end_reason.as_deref()),
            ("stopped", Some("parent_ended")),
            "{child}"
        );
    }
    let store = f.m.store.lock().unwrap();
    assert!(store.open_cascades().unwrap().is_empty());
    let cause = store.end_cause(&c.id).unwrap().unwrap();
    assert_eq!(cause.parent_end_reason, Some("completed".into()));
    assert_eq!(
        store.cascade(&cause.cascade_id).unwrap().unwrap().targets,
        [
            (c.id.clone(), "ended".to_string()),
            (g.id.clone(), "ended".to_string())
        ]
    );
}

#[test]
fn a_child_that_ends_in_its_grace_keeps_its_own_end() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.force_spawn_for_tests(&c.id);
    let g = f.child(&c.id);
    let d = f.child(&p.id);
    f.report(&p.id, "failed");
    // It ends by itself too: its own child is already told, once.
    f.report(&c.id, "completed");
    assert_eq!(f.m.inbox_of(&g.id).unwrap().len(), 1);
    let due = after_secs(NOTICE_GRACE_SECS + 1);
    assert_eq!(
        stops(&f.m, &due),
        [(d.id.clone(), true), (g.id.clone(), true)]
    );
    let r = f.m.get(&c.id).unwrap();
    assert_eq!((r.status.as_str(), r.end_reason), ("completed", None));
    let store = f.m.store.lock().unwrap();
    let cause = store.end_cause(&c.id).unwrap().unwrap();
    assert_eq!(
        store.cascade(&cause.cascade_id).unwrap().unwrap().targets[0],
        (c.id.clone(), "already_ended".to_string())
    );
    drop(store);
    no_terminal_input(&f.sent());
}

#[test]
fn a_persons_stop_gives_no_notice_and_a_host_restart_nothing() {
    let mut f = fixture();
    // A person's stop (134-7's C1): no notice, and the child is stopped at
    // once, not after a grace.
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report(&p.id, "stopped");
    assert!(f.m.inbox_of(&c.id).unwrap().is_empty());
    f.sent();
    assert_eq!(stops(&f.m, &after_secs(0)), [(c.id.clone(), true)]);
    f.report(&c.id, "stopped");
    let r = f.m.get(&c.id).unwrap();
    assert_eq!(
        (r.status.as_str(), r.end_reason.as_deref()),
        ("stopped", Some("cascade"))
    );

    let p = f.parent("other");
    let c = f.child(&p.id);
    let report = serde_json::from_value(
        serde_json::json!({"status": "failed", "end_reason": "host_restart"}),
    )
    .unwrap();
    f.m.host_status("hst_A", &p.id, &report).unwrap();
    assert!(f.m.inbox_of(&c.id).unwrap().is_empty());
    assert!(stops(&f.m, &after_secs(NOTICE_GRACE_SECS + 1)).is_empty());
    f.report(&c.id, "completed");
    assert!(
        f.m.store
            .lock()
            .unwrap()
            .open_cascades()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_child_whose_host_is_away_at_the_stop_is_unreachable() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report(&p.id, "completed");
    f.sent();
    f.m.disconnect_host("hst_A", f.generation);
    let stopped = stops(&f.m, &after_secs(NOTICE_GRACE_SECS + 1));
    assert_eq!(stopped, [(c.id.clone(), false)]);
    let store = f.m.store.lock().unwrap();
    let open = store.open_cascades().unwrap();
    assert_eq!(open.len(), 1, "not done before it is ended (LT-4)");
    assert_eq!(
        store.cascade(&open[0]).unwrap().unwrap().targets,
        [(c.id.clone(), "unreachable".to_string())]
    );
    drop(store);
    assert!(f.sent().is_empty());
}

// ---- waiting -----------------------------------------------------------------------

#[tokio::test]
async fn a_waiting_read_wakes_on_a_message_and_ends_at_its_bound() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let started = Instant::now();
    let (read, timed_out) =
        f.m.read_messages(&c.id, None, Duration::from_millis(600), &|| true)
            .await
            .unwrap();
    assert!(read.is_empty() && timed_out);
    assert!(started.elapsed() >= Duration::from_millis(600));

    let m = std::sync::Arc::new(f);
    let reader = {
        let m = m.clone();
        let c = c.id.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let out =
                m.m.read_messages(&c, None, Duration::from_secs(20), &|| true)
                    .await
                    .unwrap();
            (out, started.elapsed())
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!reader.is_finished(), "nothing to read yet");
    let mut woken = m.m.watch(&c.id);
    woken.borrow_and_update();
    m.message(&p.id, &c.id, "wake up").unwrap();
    assert!(woken.has_changed().unwrap(), "the message wakes its child");
    let ((read, timed_out), took) = reader.await.unwrap();
    assert!(!timed_out);
    assert_eq!(read[0].text, "wake up");
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[tokio::test]
async fn losing_the_offer_ends_a_waiting_read() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let m = std::sync::Arc::new(f);
    let reader = {
        let m = m.clone();
        let c = c.id.clone();
        tokio::spawn(async move {
            let offered = {
                let m = m.clone();
                let c = c.clone();
                move || m.m.spawned_offered(&c)
            };
            m.m.read_messages(&c, None, Duration::from_secs(20), &offered)
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    m.m.lose_isolation_for_tests("hst_A");
    let out = tokio::time::timeout(Duration::from_secs(2), reader)
        .await
        .expect("the read ended")
        .unwrap();
    assert!(matches!(out, Err(AgentError::Conflict(c)) if c == "not_permitted"));
}
