//! Decision 134-7: ending and cascades at the manager — LT-1…LT-12 of
//! "Decision 134 — dependencies", one test each, then the races between a
//! person's end and a parent's own end, the unreachable retry, an approved
//! `end_agent`, C3 and tree dismissal. The routes and the tool are tested
//! in `api.rs`.

use tokio::sync::mpsc::UnboundedReceiver;

use super::store::SessionRecord;
use super::store::spawn::{Decision, NOTICE_GRACE_SECS, NewRequest, PARENT_ENDED_NOTICE};
use super::{
    AgentError, AgentManager, Command, Envelope, Hello, Launch, LaunchMeta, McpGrant,
    ReportedSession, StatusReport,
};

const VERIFIED: &str = r#"{"state": "verified", "mechanism": "so_peerpidfd", "reason": null}"#;

fn hello(sessions: Vec<ReportedSession>) -> Hello {
    let mut hello: Hello = serde_json::from_value(serde_json::json!({
        "providers": [{"id": "claude-code", "kind": "cli", "interactions": ["terminal"], "available": true}],
        "workspaces": ["storm", "other", "third"],
        "workspace_paths": {"storm": "/w/storm", "other": "/w/other", "third": "/w/third"},
        "max_sessions": 0,
        "mcp_bridge": true,
        "resume": ["claude-code"],
        "identity": serde_json::from_str::<serde_json::Value>(VERIFIED).unwrap(),
    }))
    .unwrap();
    hello.sessions = sessions;
    hello
}

fn status(status: &str) -> StatusReport {
    serde_json::from_value(serde_json::json!({"status": status, "exit_code": 0})).unwrap()
}

struct Fixture {
    m: AgentManager,
    dir: tempdir::TempDir,
    generation: u64,
    link: UnboundedReceiver<Envelope>,
}

fn fixture() -> Fixture {
    let dir = tempdir::TempDir::new("storm-cascade").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (generation, link) = m.connect_host("hst_A");
    m.hello("hst_A", hello(Vec::new())).unwrap();
    m.verify_isolation_for_tests("hst_A");
    Fixture {
        m,
        dir,
        generation,
        link,
    }
}

fn after_secs(secs: i64) -> String {
    crate::index::rfc3339_minus_secs(&crate::index::now_rfc3339(), -secs)
}

impl Fixture {
    fn launch(&self, workspace: &str) -> SessionRecord {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": "claude-code",
            "terminal": {"cols": 80, "rows": 24},
        }))
        .unwrap();
        self.launch_with(launch)
    }

    fn launch_with(&self, launch: Launch) -> SessionRecord {
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

    fn report(&self, id: &str, s: &str) {
        self.m.host_status("hst_A", id, &status(s)).unwrap();
    }

    fn report_json(&self, id: &str, report: serde_json::Value) {
        let report = serde_json::from_value(report).unwrap();
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

    fn get(&self, id: &str) -> (String, Option<String>) {
        let r = self.m.get(id).unwrap();
        (r.status, r.end_reason)
    }

    /// The `end`s that went down the link since the last look; nothing
    /// else may have gone.
    fn ends(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.link.try_recv().ok())
            .filter_map(|e| match e.command {
                Command::End { session } => Some(session),
                Command::Input { .. } | Command::Resize { .. } => {
                    panic!("{} went to a terminal", e.command.kind())
                }
                _ => None,
            })
            .collect()
    }

    fn stops(&self, now: &str) -> Vec<(String, bool)> {
        self.m
            .stop_cascade_targets(now)
            .unwrap()
            .into_iter()
            .map(|s| (s.session_id, s.sent))
            .collect()
    }

    fn cascades_by(&self, parent: &str) -> Vec<(String, String)> {
        let conn = rusqlite::Connection::open(self.dir.path().join("agent/agent.db")).unwrap();
        let mut stmt = conn
            .prepare("SELECT id, cause FROM spawn_cascades WHERE parent_session = ?1")
            .unwrap();
        stmt.query_map([parent], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn reconnect(&mut self, sessions: &[(&str, &str)]) {
        let (generation, link) = self.m.connect_host("hst_A");
        self.generation = generation;
        self.link = link;
        let sessions = sessions
            .iter()
            .map(|(id, s)| ReportedSession {
                id: id.to_string(),
                status: status(s),
                output_end_offset: 0,
            })
            .collect();
        self.m.hello("hst_A", hello(sessions)).unwrap();
        self.m.verify_isolation_for_tests("hst_A");
    }
}

fn ids(rs: &[SessionRecord]) -> Vec<String> {
    rs.iter().map(|r| r.id.clone()).collect()
}

fn s(v: &str) -> String {
    v.to_string()
}

fn ended(reason: &str) -> (String, Option<String>) {
    (s("stopped"), Some(s(reason)))
}

// ---- LT-1…LT-12 ------------------------------------------------------------

/// LT-1: a person ends a parent with running and ended children. The
/// confirmation lists both; every live descendant is `stopped: cascade` at
/// once; the ended one is untouched.
#[test]
fn lt1_a_person_ends_a_parent_and_every_live_descendant_is_stopped() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.force_spawn_for_tests(&c.id);
    let g = f.child(&c.id);
    let done = f.child(&p.id);
    f.report(&done.id, "completed");
    f.ends();

    let (running, ended_ones) = f.m.descendants_of(&p.id).unwrap();
    assert_eq!(ids(&running), [c.id.clone(), g.id.clone()]);
    assert_eq!(ids(&ended_ones), std::slice::from_ref(&done.id));

    // Unconfirmed, or confirmed without one that is live: nothing happens.
    for confirmed in [None, Some(vec![c.id.clone()])] {
        let refused = f.m.end_by_person(&p.id, confirmed.as_deref(), false);
        assert!(
            matches!(refused, Err(AgentError::Conflict(_))),
            "{refused:?}"
        );
    }
    assert!(f.ends().is_empty());
    assert!(f.cascades_by(&p.id).is_empty());

    let all = [c.id.clone(), g.id.clone(), done.id.clone()];
    let cascade =
        f.m.end_by_person(&p.id, Some(&all), false)
            .unwrap()
            .unwrap();
    assert_eq!(
        (cascade.cause.as_str(), cascade.parent_end_reason.as_deref()),
        ("human_end", Some("stopped"))
    );
    assert_eq!(f.ends(), std::slice::from_ref(&p.id), "the parent first");
    assert!(f.m.inbox_of(&c.id).unwrap().is_empty(), "no notice");
    assert_eq!(
        f.stops(&after_secs(0)),
        [(c.id.clone(), true), (g.id.clone(), true)],
        "at once"
    );
    assert_eq!(f.ends(), [c.id.clone(), g.id.clone()]);

    f.report(&p.id, "stopped");
    f.report(&c.id, "stopped");
    f.report(&g.id, "completed");
    assert_eq!(f.get(&p.id), (s("stopped"), None));
    assert_eq!(f.get(&c.id), ended("cascade"));
    assert_eq!(f.get(&g.id), ended("cascade"));
    assert_eq!(f.get(&done.id), (s("completed"), None));
    let after = f.m.cascade(&cascade.id).unwrap();
    assert!(after.completed_at.is_some());
    assert_eq!(
        after.targets,
        [(c.id.clone(), s("ended")), (g.id.clone(), s("ended"))]
    );
    assert_eq!(f.cascades_by(&p.id).len(), 1, "one cause");
}

/// LT-2: a parent that exits normally: children `parent_ended`, the
/// parent `completed` with its exit code.
#[test]
fn lt2_a_parent_that_exits_normally_stops_its_children_as_parent_ended() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report_json(
        &p.id,
        serde_json::json!({"status": "completed", "exit_code": 3}),
    );
    let r = f.m.get(&p.id).unwrap();
    assert_eq!((r.status.as_str(), r.exit_code), ("completed", Some(3)));
    assert_eq!(f.m.inbox_of(&c.id).unwrap()[0].text, PARENT_ENDED_NOTICE);
    f.ends();
    assert_eq!(
        f.stops(&after_secs(NOTICE_GRACE_SECS + 1)),
        [(c.id.clone(), true)]
    );
    f.report(&c.id, "stopped");
    assert_eq!(f.get(&c.id), ended("parent_ended"));
    let r = f.m.get(&p.id).unwrap();
    assert_eq!((r.status.as_str(), r.exit_code), ("completed", Some(3)));
}

/// LT-3: a parent ended by a signal is `failed` with it, never shown as
/// completed; its children are `parent_ended`, with that end copied.
#[test]
fn lt3_a_signalled_parent_is_failed_and_its_children_parent_ended() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report_json(
        &p.id,
        serde_json::json!({"status": "failed", "end_reason": "signal", "signal": 9}),
    );
    let r = f.m.get(&p.id).unwrap();
    assert_eq!(
        (r.status.as_str(), r.end_reason.as_deref(), r.signal),
        ("failed", Some("signal"), Some(9))
    );
    f.stops(&after_secs(NOTICE_GRACE_SECS + 1));
    f.report(&c.id, "failed");
    assert_eq!(f.get(&c.id), ended("parent_ended"));
    let cause = f.m.store.lock().unwrap().end_cause(&c.id).unwrap().unwrap();
    assert_eq!(cause.parent_end_reason.as_deref(), Some("signal"));
    assert_eq!(f.get(&p.id).0, "failed");
}

/// LT-4: the host is away during a cascade: its targets are
/// `unreachable`, never done, and ended exactly once when it is back.
#[test]
fn lt4_an_unreachable_target_is_ended_once_when_its_host_returns() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let cascade =
        f.m.end_by_person(&p.id, Some(std::slice::from_ref(&c.id)), false)
            .unwrap()
            .unwrap();
    f.m.disconnect_host("hst_A", f.generation);
    assert_eq!(f.stops(&after_secs(0)), [(c.id.clone(), false)]);
    // While it is away, nothing is sent and nothing is recorded again.
    assert!(f.stops(&after_secs(1)).is_empty());
    let away = f.m.cascade(&cascade.id).unwrap();
    assert_eq!(away.targets, [(c.id.clone(), s("unreachable"))]);
    assert!(away.completed_at.is_none(), "no done before it is ended");

    // A link that is listed but already gone takes no `end` and changes
    // nothing.
    let (_, gone) = f.m.connect_host("hst_A");
    drop(gone);
    assert!(f.stops(&after_secs(2)).is_empty());
    assert_eq!(
        f.m.cascade(&cascade.id).unwrap().targets,
        [(c.id.clone(), s("unreachable"))]
    );
    f.ends();
    f.reconnect(&[(&p.id, "stopped"), (&c.id, "running")]);
    assert_eq!(f.stops(&after_secs(2)), [(c.id.clone(), true)]);
    assert!(f.stops(&after_secs(3)).is_empty(), "once");
    assert_eq!(f.ends(), std::slice::from_ref(&c.id));
    f.report(&c.id, "stopped");
    assert_eq!(f.get(&c.id), ended("cascade"));
    let done = f.m.cascade(&cascade.id).unwrap();
    assert_eq!(done.targets, [(c.id.clone(), s("ended"))]);
    assert!(done.completed_at.is_some());
}

/// LT-5: a host restart ends every session `host_restart`; no child is
/// relabelled, though Storm had sent its `end`, and running the cascade
/// again does nothing.
#[test]
fn lt5_a_host_restart_relabels_no_child() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let cascade =
        f.m.end_by_person(&p.id, Some(std::slice::from_ref(&c.id)), false)
            .unwrap()
            .unwrap();
    f.stops(&after_secs(0));
    let restart = serde_json::json!({"status": "failed", "end_reason": "host_restart"});
    f.report_json(&p.id, restart.clone());
    f.report_json(&c.id, restart);
    for id in [&p.id, &c.id] {
        assert_eq!(f.get(id), (s("failed"), Some(s("host_restart"))), "{id}");
    }
    f.ends();
    assert!(f.stops(&after_secs(NOTICE_GRACE_SECS + 1)).is_empty());
    assert!(f.ends().is_empty());
    let after = f.m.cascade(&cascade.id).unwrap();
    assert_eq!(after.targets, [(c.id.clone(), s("already_ended"))]);
    assert_eq!(f.cascades_by(&p.id).len(), 1);
}

/// LT-6: the server restarts mid-cascade. The cascade resumes from
/// `agent.db`: a target ended while the server was down is recorded as
/// Storm's stop, one still running is sent `end` once, and the cascade
/// completes once.
#[test]
fn lt6_a_cascade_resumes_after_a_server_restart_and_completes_once() {
    let f = fixture();
    let p = f.parent("storm");
    let a = f.child(&p.id);
    let b = f.child(&p.id);
    let cascade =
        f.m.end_by_person(&p.id, Some(&[a.id.clone(), b.id.clone()]), false)
            .unwrap()
            .unwrap();
    // The server goes before its stop pass, and a's end with it.
    let Fixture { m, dir, .. } = f;
    drop(m);
    let m = AgentManager::open(dir.path()).unwrap();
    let (generation, link) = m.connect_host("hst_A");
    let mut f = Fixture {
        m,
        dir,
        generation,
        link,
    };
    assert_eq!(f.get(&a.id).0, "unknown");
    f.m.hello(
        "hst_A",
        hello(vec![
            ReportedSession {
                id: p.id.clone(),
                status: status("stopped"),
                output_end_offset: 0,
            },
            ReportedSession {
                id: a.id.clone(),
                status: status("stopped"),
                output_end_offset: 0,
            },
            ReportedSession {
                id: b.id.clone(),
                status: status("running"),
                output_end_offset: 0,
            },
        ]),
    )
    .unwrap();
    f.m.verify_isolation_for_tests("hst_A");
    assert_eq!(f.get(&a.id), ended("cascade"), "its end was Storm's");
    assert_eq!(f.stops(&after_secs(0)), [(b.id.clone(), true)]);
    assert!(f.stops(&after_secs(1)).is_empty());
    assert_eq!(f.ends(), std::slice::from_ref(&b.id));
    f.report(&b.id, "stopped");
    assert_eq!(f.get(&b.id), ended("cascade"));
    let done = f.m.cascade(&cascade.id).unwrap();
    let completed = done.completed_at.clone().unwrap();
    assert!(
        f.m.store
            .lock()
            .unwrap()
            .open_cascades()
            .unwrap()
            .is_empty()
    );
    f.stops(&after_secs(5));
    assert_eq!(
        f.m.cascade(&cascade.id).unwrap().completed_at.as_deref(),
        Some(completed.as_str()),
        "completed once"
    );
}

/// LT-7: revoking spawn refuses pending requests at once and every new
/// one; children keep running.
#[test]
fn lt7_revocation_refuses_requests_and_leaves_children_running() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    let pending =
        f.m.create_request(
            &p.id,
            NewRequest::End {
                target: c.id.clone(),
            },
        )
        .unwrap()
        .unwrap();
    f.m.set_spawn(&p.id, false, Some("dev_A")).unwrap().unwrap();
    let r = f.m.request(&pending.id).unwrap();
    assert_eq!(
        (r.outcome.as_deref(), r.reason.as_deref()),
        (Some("refused"), Some("revoked"))
    );
    let refused =
        f.m.create_request(
            &p.id,
            NewRequest::End {
                target: c.id.clone(),
            },
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.reason, "not_permitted");
    assert_eq!(f.get(&c.id).0, "running");
    assert!(f.stops(&after_secs(NOTICE_GRACE_SECS + 1)).is_empty());
    assert!(f.ends().is_empty());
}

/// LT-8: "End all children" stops every descendant and leaves the parent
/// running, its spawn switch and its pending requests as they were.
#[test]
fn lt8_end_all_children_leaves_the_parent_as_it_was() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.force_spawn_for_tests(&c.id);
    let g = f.child(&c.id);
    let pending =
        f.m.create_request(
            &p.id,
            NewRequest::Message {
                target: c.id.clone(),
                text: "hi".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert!(matches!(
        f.m.end_children(&p.id, std::slice::from_ref(&c.id), false),
        Err(AgentError::Conflict(_))
    ));
    let cascade =
        f.m.end_children(&p.id, &[c.id.clone(), g.id.clone()], false)
            .unwrap();
    assert_eq!(
        (cascade.cause.as_str(), cascade.parent_end_reason.as_deref()),
        ("end_all_children", None)
    );
    f.ends();
    assert_eq!(
        f.stops(&after_secs(0)),
        [(c.id.clone(), true), (g.id.clone(), true)]
    );
    assert_eq!(f.ends(), [c.id.clone(), g.id.clone()], "never the parent");
    f.report(&c.id, "stopped");
    f.report(&g.id, "stopped");
    assert_eq!(f.get(&c.id), ended("cascade"));
    assert_eq!(f.get(&g.id), ended("cascade"));
    assert_eq!(f.get(&p.id), (s("running"), None));
    assert!(f.m.store.lock().unwrap().spawn_enabled(&p.id).unwrap());
    assert_eq!(f.m.request(&pending.id).unwrap().state, "pending");
    // Nothing is left to end.
    assert!(matches!(
        f.m.end_children(&p.id, &[], false),
        Err(AgentError::Conflict(c)) if c == "no_live_descendants"
    ));
}

/// LT-9 (C3): a resumed parent reads its predecessor's children's results
/// and nothing else of them: status, wait, message and end are not found.
#[test]
fn lt9_a_resumed_parent_reads_its_predecessors_results_and_nothing_more() {
    let f = fixture();
    let p = f.parent("storm");
    f.report_json(
        &p.id,
        serde_json::json!({"status": "running",
            "harness_session": "0f8fad5b-d9cb-469f-a165-70867728950e"}),
    );
    let c = f.child(&p.id);
    let quiet = f.child(&p.id);
    f.m.report_result(&c.id, "succeeded", "found it", None)
        .unwrap();
    f.report(&c.id, "completed");
    f.report(&quiet.id, "completed");
    f.report(&p.id, "completed");

    let launch: Launch = serde_json::from_value(serde_json::json!({
        "host_id": "hst_A", "workspace": "storm", "provider": "claude-code",
        "terminal": {"cols": 80, "rows": 24}, "resume_of": p.id,
    }))
    .unwrap();
    let resumed = f.launch_with(launch);
    f.m.force_spawn_for_tests(&resumed.id);

    let read = f.m.predecessor_results(&resumed.id).unwrap();
    assert_eq!(read.len(), 1, "only a child that reported");
    let (predecessor, child, result) = &read[0];
    assert_eq!((predecessor, &child.id), (&p.id, &c.id));
    assert_eq!(result.summary, "found it");

    assert!(matches!(
        f.m.descendant(&resumed.id, &c.id),
        Err(AgentError::NotFound(_))
    ));
    assert!(f.m.children_of(&resumed.id).unwrap().is_empty());
    for new in [
        NewRequest::End {
            target: c.id.clone(),
        },
        NewRequest::Message {
            target: c.id.clone(),
            text: "hi".into(),
        },
    ] {
        assert_eq!(
            f.m.create_request(&resumed.id, new)
                .unwrap()
                .unwrap_err()
                .reason,
            "not_found"
        );
    }
    // A session that resumes nothing reads nothing.
    assert!(f.m.predecessor_results(&p.id).unwrap().is_empty());
}

/// LT-10: no route changes lineage — a launch naming a parent is still
/// top-level — and after every kind of cascade the lineage, the outputs
/// and the results are as they were.
#[test]
fn lt10_lineage_outputs_and_results_outlast_every_cascade() {
    let f = fixture();
    let p = f.parent("storm");
    let launch: Launch = serde_json::from_value(serde_json::json!({
        "host_id": "hst_A", "workspace": "other", "provider": "claude-code",
        "terminal": {"cols": 80, "rows": 24},
        "parent_session": p.id, "root_session": p.id, "depth": 1,
    }))
    .unwrap();
    let posing = f.launch_with(launch);
    assert_eq!(f.m.lineage(&posing.id).unwrap().depth, 0);
    assert!(f.m.children_of(&p.id).unwrap().is_empty());

    let kids: Vec<SessionRecord> = (0..3).map(|_| f.child(&p.id)).collect();
    for (i, k) in kids.iter().enumerate() {
        f.m.host_output("hst_A", &k.id, 0, format!("out {i}").as_bytes())
            .unwrap();
        f.m.report_result(&k.id, "partial", &format!("r{i}"), None)
            .unwrap();
    }
    let lineages: Vec<_> = kids.iter().map(|k| f.m.lineage(&k.id).unwrap()).collect();
    // One of each: End all children, a person's end, a parent's own end.
    f.m.end_children(
        &p.id,
        &[kids[0].id.clone(), kids[1].id.clone(), kids[2].id.clone()],
        false,
    )
    .unwrap();
    f.stops(&after_secs(0));
    for k in &kids {
        f.report(&k.id, "stopped");
    }
    let q = f.parent("third");
    let qk = f.child(&q.id);
    f.m.end_by_person(&q.id, Some(std::slice::from_ref(&qk.id)), false)
        .unwrap();
    f.report(&q.id, "completed");
    for (i, k) in kids.iter().enumerate() {
        assert_eq!(f.m.lineage(&k.id).unwrap(), lineages[i]);
        assert_eq!(
            f.m.result_of(&k.id).unwrap().unwrap().summary,
            format!("r{i}")
        );
        match f.m.read_output(&k.id, 0, 64) {
            super::cache::Read::Data { bytes, .. } => {
                assert_eq!(bytes, format!("out {i}").as_bytes())
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        f.m.lineage(&qk.id).unwrap().parent_session.as_deref(),
        Some(q.id.as_str())
    );
}

/// LT-11 (C7, 138p point 5): a resumed child is a new top-level session.
/// Its old parent can neither see nor control it; the old lineage stays as
/// history.
#[test]
fn lt11_a_resumed_child_is_a_new_top_level_session() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report_json(
        &c.id,
        serde_json::json!({"status": "running",
            "harness_session": "7c9e6679-7425-40de-944b-e07fc1f90ae7"}),
    );
    f.report(&c.id, "completed");
    // The parent turns its switch off so the workspace is free (WS-9).
    f.m.set_spawn(&p.id, false, Some("dev_A")).unwrap().unwrap();
    let launch: Launch = serde_json::from_value(serde_json::json!({
        "host_id": "hst_A", "workspace": c.workspace, "provider": "claude-code",
        "terminal": {"cols": 80, "rows": 24}, "resume_of": c.id,
    }))
    .unwrap();
    let resumed = f.launch_with(launch);
    let l = f.m.lineage(&resumed.id).unwrap();
    assert_eq!(
        (l.parent_session, l.root_session.as_str(), l.depth),
        (None, resumed.id.as_str(), 0)
    );
    assert_eq!(resumed.resumed_from.as_deref(), Some(c.id.as_str()));
    assert!(f.m.task_of(&resumed.id).unwrap().is_none());
    assert_eq!(
        f.m.lineage(&c.id).unwrap().parent_session.as_deref(),
        Some(p.id.as_str()),
        "history kept"
    );
    assert!(matches!(
        f.m.descendant(&p.id, &resumed.id),
        Err(AgentError::NotFound(_))
    ));
    assert_eq!(
        ids(&f.m.children_of(&p.id).unwrap()),
        std::slice::from_ref(&c.id)
    );
    f.m.force_spawn_for_tests(&p.id);
    assert_eq!(
        f.m.create_request(
            &p.id,
            NewRequest::End {
                target: resumed.id.clone()
            }
        )
        .unwrap()
        .unwrap_err()
        .reason,
        "not_found"
    );
    // Its old parent's end does not reach it.
    f.report(&p.id, "completed");
    assert!(f.m.inbox_of(&resumed.id).unwrap().is_empty());
    assert!(f.stops(&after_secs(NOTICE_GRACE_SECS + 1)).is_empty());
    assert_eq!(f.get(&resumed.id).0, "running");
}

/// LT-12 (C8): one member of a live tree cannot be dismissed; an ended
/// tree goes only as a unit, with confirmation, every member together.
#[test]
fn lt12_a_tree_is_dismissed_only_as_a_unit() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.force_spawn_for_tests(&c.id);
    let g = f.child(&c.id);
    let alone = f.launch("other");
    f.report(&g.id, "completed");
    let refused = f.m.dismiss_session(&g.id, true).unwrap_err();
    assert!(matches!(refused, AgentError::Conflict(ref c) if c == "tree_live"));
    f.report(&c.id, "completed");
    f.report(&p.id, "completed");
    assert!(matches!(
        f.m.dismiss_session(&g.id, false),
        Err(AgentError::Conflict(c)) if c == "confirm_tree"
    ));
    for id in [&p.id, &c.id, &g.id] {
        assert!(f.m.get(id).is_ok(), "nothing dismissed yet");
    }
    let d = f.m.dismiss_session(&g.id, true).unwrap();
    assert_eq!(d.sessions, [g.id.clone(), p.id.clone(), c.id.clone()]);
    for id in [&p.id, &c.id, &g.id] {
        assert!(matches!(f.m.get(id), Err(AgentError::NotFound(_))), "{id}");
        // Lineage stays as history (L-1).
        assert!(f.m.lineage(id).unwrap().depth >= 0);
    }
    assert_eq!(
        f.m.lineage(&g.id).unwrap().root_session,
        p.id,
        "history kept"
    );
    // A session in no tree is dismissed alone, as before.
    f.report(&alone.id, "completed");
    assert_eq!(
        f.m.dismiss_session(&alone.id, false).unwrap().sessions,
        std::slice::from_ref(&alone.id)
    );
}

// ---- races: a person's end against the parent's own end --------------------

/// The person's end is recorded first: the parent's own end then opens
/// nothing, and its children are `cascade`, its own end kept (L-5).
#[test]
fn a_persons_end_before_the_parents_own_end_is_the_one_cause() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.end_by_person(&p.id, Some(std::slice::from_ref(&c.id)), false)
        .unwrap();
    f.report(&p.id, "completed");
    assert_eq!(f.get(&p.id), (s("completed"), None));
    assert!(
        f.m.inbox_of(&c.id).unwrap().is_empty(),
        "no parent_end notice"
    );
    f.ends();
    assert_eq!(f.stops(&after_secs(0)), [(c.id.clone(), true)]);
    f.report(&c.id, "stopped");
    assert_eq!(f.get(&c.id), ended("cascade"));
    assert_eq!(
        f.cascades_by(&p.id)
            .into_iter()
            .map(|(_, cause)| cause)
            .collect::<Vec<_>>(),
        ["human_end"]
    );
}

/// The parent's own end is recorded first: the person's end then finds it
/// ended, and the children keep `parent_ended`.
#[test]
fn the_parents_own_end_before_a_persons_end_is_the_one_cause() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.report(&p.id, "failed");
    assert!(matches!(
        f.m.end_by_person(&p.id, Some(std::slice::from_ref(&c.id)), false),
        Err(AgentError::Conflict(_))
    ));
    f.stops(&after_secs(NOTICE_GRACE_SECS + 1));
    f.report(&c.id, "stopped");
    assert_eq!(f.get(&c.id), ended("parent_ended"));
    assert_eq!(
        f.cascades_by(&p.id)
            .into_iter()
            .map(|(_, cause)| cause)
            .collect::<Vec<_>>(),
        ["parent_end"]
    );
}

/// Both at once, many times over: exactly one cascade for the parent, and
/// its children's end follows that one cause.
#[test]
fn a_persons_end_racing_the_parents_own_end_records_one_cause() {
    let f = std::sync::Arc::new(fixture());
    for round in 0..200 {
        let p = f.parent("storm");
        let c = f.child(&p.id);
        let person = {
            let f = f.clone();
            let (p, c) = (p.id.clone(), c.id.clone());
            std::thread::spawn(move || {
                let _ = f.m.end_by_person(&p, Some(&[c]), false);
            })
        };
        let own = {
            let f = f.clone();
            let p = p.id.clone();
            std::thread::spawn(move || {
                f.m.host_status("hst_A", &p, &status("completed")).unwrap();
            })
        };
        person.join().unwrap();
        own.join().unwrap();
        let causes = f.cascades_by(&p.id);
        assert_eq!(causes.len(), 1, "round {round}: {causes:?}");
        f.stops(&after_secs(NOTICE_GRACE_SECS + 1));
        f.report(&c.id, "stopped");
        let want = if causes[0].1 == "human_end" {
            "cascade"
        } else {
            "parent_ended"
        };
        assert_eq!(f.get(&c.id), ended(want), "round {round}");
        assert_eq!(f.get(&p.id).0, "completed", "round {round}: L-5");
    }
}

// ---- the rest -----------------------------------------------------------------

/// "Let children report first" (C9 for a person's stop): the notice at
/// once, the parent stopped at once, the children only after the grace.
#[test]
fn letting_children_report_first_gives_them_the_notice_and_the_grace() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.end_by_person(&p.id, Some(std::slice::from_ref(&c.id)), true)
        .unwrap();
    assert_eq!(f.ends(), std::slice::from_ref(&p.id));
    assert_eq!(f.m.inbox_of(&c.id).unwrap()[0].text, PARENT_ENDED_NOTICE);
    assert!(f.stops(&after_secs(NOTICE_GRACE_SECS - 1)).is_empty());
    f.report(&c.id, "completed");
    assert_eq!(
        f.get(&c.id),
        (s("completed"), None),
        "its own end, in the grace"
    );
    assert!(f.stops(&after_secs(NOTICE_GRACE_SECS + 1)).is_empty());
}

/// "End all children" with the host away: every target is unreachable,
/// then each is ended once when it is back.
#[test]
fn end_all_children_with_the_host_away_retries_on_its_return() {
    let mut f = fixture();
    let p = f.parent("storm");
    let a = f.child(&p.id);
    let b = f.child(&p.id);
    f.m.disconnect_host("hst_A", f.generation);
    let cascade =
        f.m.end_children(&p.id, &[a.id.clone(), b.id.clone()], false)
            .unwrap();
    assert_eq!(
        f.stops(&after_secs(0)),
        [(a.id.clone(), false), (b.id.clone(), false)]
    );
    f.reconnect(&[(&p.id, "running"), (&a.id, "running"), (&b.id, "running")]);
    f.ends();
    assert_eq!(
        f.stops(&after_secs(1)),
        [(a.id.clone(), true), (b.id.clone(), true)]
    );
    f.report(&a.id, "stopped");
    f.report(&b.id, "stopped");
    assert!(f.m.cascade(&cascade.id).unwrap().completed_at.is_some());
}

/// An approved `end_agent` sends its target `end`; the target's own
/// descendants follow it at once as `cascade` (C1).
#[test]
fn an_approved_end_stops_its_target_and_the_targets_descendants() {
    let mut f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.force_spawn_for_tests(&c.id);
    let g = f.child(&c.id);
    let r =
        f.m.create_request(
            &p.id,
            NewRequest::End {
                target: c.id.clone(),
            },
        )
        .unwrap()
        .unwrap();
    f.ends();
    match f.m.approve_request(&r.id, "dev_A", false).unwrap() {
        Decision::Decided { request, .. } => {
            assert_eq!(request.outcome.as_deref(), Some("ended"))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(f.ends(), std::slice::from_ref(&c.id));
    f.report(&c.id, "stopped");
    assert_eq!(f.get(&c.id), (s("stopped"), None), "a person's stop");
    assert_eq!(f.stops(&after_secs(0)), [(g.id.clone(), true)]);
    f.report(&g.id, "stopped");
    assert_eq!(f.get(&g.id), ended("cascade"));
    assert_eq!(f.get(&p.id).0, "running");
}

/// A session being ended asks for nothing more: a request it made in its
/// grace could add a child its cascade never snapshotted (AP-3).
#[test]
fn a_session_being_ended_cannot_ask_for_anything() {
    let f = fixture();
    let p = f.parent("storm");
    let c = f.child(&p.id);
    f.m.force_spawn_for_tests(&c.id);
    let g = f.child(&c.id);
    f.report(&p.id, "completed");
    let refused =
        f.m.create_request(
            &c.id,
            NewRequest::Message {
                target: g.id.clone(),
                text: "hi".into(),
            },
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.reason, "requester_ended");
    // A parent a person is ending, before its end is recorded, too.
    let q = f.parent("other");
    let qc = f.child(&q.id);
    let pending =
        f.m.create_request(
            &q.id,
            NewRequest::End {
                target: qc.id.clone(),
            },
        )
        .unwrap()
        .unwrap();
    f.m.end_by_person(&q.id, Some(std::slice::from_ref(&qc.id)), false)
        .unwrap();
    assert_eq!(
        f.m.request(&pending.id).unwrap().reason.as_deref(),
        Some("requester_ended")
    );
    let refused =
        f.m.create_request(
            &q.id,
            NewRequest::End {
                target: qc.id.clone(),
            },
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.reason, "requester_ended");
}

/// A person's end of a session with no live descendants is the end it
/// always was: one `end`, no cascade.
#[test]
fn ending_a_session_without_live_descendants_opens_no_cascade() {
    let mut f = fixture();
    let p = f.parent("storm");
    f.ends();
    assert!(f.m.end_by_person(&p.id, None, false).unwrap().is_none());
    assert_eq!(f.ends(), std::slice::from_ref(&p.id));
    f.report(&p.id, "stopped");
    assert!(f.cascades_by(&p.id).is_empty());
    assert!(matches!(
        f.m.end_by_person(&p.id, None, false),
        Err(AgentError::Conflict(_))
    ));
}
