//! Decision 134-6: 134j's inbox states, A2's scope, the cap, C9's notice and
//! LT-13, at the manager. The tools and the approval routes over the
//! gateway are tested in `api.rs`.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::store::SessionRecord;
use super::store::spawn::{
    Decision, MAX_INBOX, NOTICE_GRACE_SECS, NewRequest, PARENT_ENDED_NOTICE, Refusal, Request,
};
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

    fn message(&self, from: &str, to: &str, text: &str) -> Result<Request, Refusal> {
        self.m
            .create_request(
                from,
                NewRequest::Message {
                    target: to.into(),
                    text: text.into(),
                },
            )
            .unwrap()
    }

    fn approve(&self, id: &str) -> Request {
        match self.m.approve_request(id, "dev_A", false).unwrap() {
            Decision::Decided { request, .. } => request,
            other => panic!("not decided: {other:?}"),
        }
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
async fn a_message_waits_for_a_person_then_is_read_and_acknowledged() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let r = f.message(&p.id, &c.id, "look at store.rs").unwrap();
    assert_eq!(r.state, "pending");
    assert_eq!(f.states(&c.id), [(1, "pending_approval".to_string())]);
    assert!(
        f.read(&c.id, None).await.is_empty(),
        "nothing before approval"
    );

    let decided = f.approve(&r.id);
    assert_eq!(decided.outcome.as_deref(), Some("delivered"));
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

    // The parent sees its message's state.
    let mine = f.m.messages_from(&p.id, &c.id).unwrap();
    assert_eq!(
        mine.iter()
            .map(|m| (m.request_id.as_deref(), m.state.as_str()))
            .collect::<Vec<_>>(),
        [(Some(r.id.as_str()), "acknowledged")]
    );
    no_terminal_input(&f.sent());
}

#[tokio::test]
async fn denied_expired_and_refused_messages_never_reach_the_child() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);

    let denied = f.message(&p.id, &c.id, "denied").unwrap();
    f.m.deny_request(&denied.id, "dev_A").unwrap();
    let expired = f.message(&p.id, &c.id, "expired").unwrap();
    let swept =
        f.m.store
            .lock()
            .unwrap()
            .expire_requests(&after_secs(601))
            .unwrap();
    assert_eq!(swept.len(), 1);
    assert_eq!(swept[0].id, expired.id);
    let revoked = f.message(&p.id, &c.id, "revoked").unwrap();
    f.m.set_spawn(&p.id, false, Some("dev_A")).unwrap().unwrap();
    assert_eq!(
        f.m.request(&revoked.id).unwrap().reason.as_deref(),
        Some("revoked")
    );
    assert_eq!(
        f.m.inbox_of(&c.id)
            .unwrap()
            .into_iter()
            .map(|m| (m.text, m.state))
            .collect::<Vec<_>>(),
        [
            ("denied".to_string(), "denied".to_string()),
            ("expired".to_string(), "expired".to_string()),
            ("revoked".to_string(), "undeliverable".to_string()),
        ]
    );
    assert!(f.read(&c.id, None).await.is_empty());
}

#[tokio::test]
async fn a_message_to_a_child_that_ends_first_is_undeliverable() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let queued = f.message(&p.id, &c.id, "queued, never read").unwrap();
    f.approve(&queued.id);
    let pending = f.message(&p.id, &c.id, "pending").unwrap();
    f.report(&c.id, "completed");
    // L-7: unread when its child ended; still pending, then refused at
    // approval's re-check.
    let refused = f.approve(&pending.id);
    assert_eq!(refused.reason.as_deref(), Some("target_gone"));
    assert_eq!(
        f.m.inbox_of(&c.id)
            .unwrap()
            .into_iter()
            .map(|m| m.state)
            .collect::<Vec<_>>(),
        ["undeliverable", "undeliverable"]
    );
}

#[tokio::test]
async fn the_cursor_only_moves_forward_whatever_order_approvals_come_in() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let first = f.message(&p.id, &c.id, "first asked").unwrap();
    let second = f.message(&p.id, &c.id, "second asked").unwrap();
    f.approve(&second.id);
    let read = f.read(&c.id, None).await;
    assert_eq!(read.len(), 1);
    let (seen, _) = &read[0];
    f.approve(&first.id);
    // Numbered when it was queued: after everything the child has seen.
    let next = f.read(&c.id, Some(*seen)).await;
    assert_eq!(next.len(), 1);
    assert!(next[0].0 > *seen, "{next:?} after {seen}");
    assert_eq!(next[0].1, "first asked");
    // A cursor past anything given out acknowledges only what was given.
    let third = f.message(&p.id, &c.id, "third").unwrap();
    f.approve(&third.id);
    assert_eq!(f.read(&c.id, Some(1000)).await.len(), 1, "queued is unseen");
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
    for (from, to) in [(&t.root, &t.child), (&t.root, &t.grandchild)] {
        assert!(f.message(from, to, "ok").is_ok(), "{from} to {to}");
    }
    let to_grandchild = f.message(&t.child, &t.grandchild, "ok").unwrap();
    f.approve(&to_grandchild.id);
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
        "not_permitted"
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
    for i in 0..MAX_INBOX - 2 {
        let r = f.message(&p.id, &c.id, &format!("m{i}")).unwrap();
        f.approve(&r.id);
    }
    // One waiting for approval counts: it takes the last of the parent's.
    let held = f.message(&p.id, &c.id, "pending").unwrap();
    let refused = f.message(&p.id, &c.id, "one too many").unwrap_err();
    assert_eq!((refused.reason, refused.cap), ("limit", Some("inbox")));
    assert_eq!(f.m.inbox_of(&c.id).unwrap().len() as i64, MAX_INBOX - 1);

    // Approved, it fills the parent's 31; its parent then ends by itself,
    // and the notice still fits, as the 32nd.
    f.approve(&held.id);
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

#[test]
fn approval_recounts_the_cap() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    for i in 0..MAX_INBOX - 3 {
        let r = f.message(&p.id, &c.id, &format!("m{i}")).unwrap();
        f.approve(&r.id);
    }
    let late = f.message(&p.id, &c.id, "late").unwrap();
    // Filled behind its back, as no product path does.
    let mut store = f.m.store.lock().unwrap();
    for _ in 0..2 {
        store
            .queue_message(&c.id, "storm", "x", None, "queued", "2026-10-25T10:00:00Z")
            .unwrap();
    }
    drop(store);
    let refused = match f.m.approve_request(&late.id, "dev_A", false).unwrap() {
        Decision::Decided { request, cap, .. } => (request.reason, cap),
        other => panic!("{other:?}"),
    };
    assert_eq!(refused, (Some("limit".into()), Some("inbox")));
    assert_eq!(
        f.m.inbox_of(&c.id)
            .unwrap()
            .iter()
            .filter(|m| m.state == "undeliverable")
            .count(),
        1
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
    assert!(f.m.stop_notified_children(&early).unwrap().is_empty());
    assert!(f.sent().is_empty(), "the notice reaches no terminal");

    let due = after_secs(NOTICE_GRACE_SECS + 1);
    let stopped = f.m.stop_notified_children(&due).unwrap();
    assert_eq!(
        stopped,
        [(c.id.clone(), true), (g.id.clone(), true)],
        "nearest first"
    );
    // Once each: a second pass sends nothing.
    assert!(f.m.stop_notified_children(&due).unwrap().is_empty());
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
        f.m.stop_notified_children(&due).unwrap(),
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
fn a_persons_stop_and_a_host_restart_give_no_notice() {
    let f = fixture();
    for ending in ["stopped", "host_restart"] {
        let p = f.parent(if ending == "stopped" {
            "storm"
        } else {
            "other"
        });
        let c = f.child(&p.id);
        if ending == "stopped" {
            f.report(&p.id, "stopped");
        } else {
            let report = serde_json::from_value(
                serde_json::json!({"status": "failed", "end_reason": "host_restart"}),
            )
            .unwrap();
            f.m.host_status("hst_A", &p.id, &report).unwrap();
        }
        assert!(f.m.inbox_of(&c.id).unwrap().is_empty(), "{ending}");
        assert!(
            f.m.stop_notified_children(&after_secs(NOTICE_GRACE_SECS + 1))
                .unwrap()
                .is_empty(),
            "{ending}"
        );
        // Ending the child here frees the next round's workspace.
        f.report(&c.id, "completed");
    }
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
    let stopped =
        f.m.stop_notified_children(&after_secs(NOTICE_GRACE_SECS + 1))
            .unwrap();
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
async fn a_waiting_read_wakes_on_delivery_and_ends_at_its_bound() {
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
    let r = m.message(&p.id, &c.id, "wake up").unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!reader.is_finished(), "pending is not readable");
    let mut woken = m.m.watch(&c.id);
    woken.borrow_and_update();
    m.approve(&r.id);
    assert!(woken.has_changed().unwrap(), "the approval wakes its child");
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
