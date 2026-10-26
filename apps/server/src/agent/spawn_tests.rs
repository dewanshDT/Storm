//! Decision 134-1: the spawning schema, proved at the store. Kept out of
//! `store.rs` so the lineage source scan reads only product code.

use rusqlite::Connection;

use super::AgentManager;
use super::store::spawn::{Request, SpawnResult};
use super::store::{SessionRecord, Store};

const NOW: &str = "2026-10-25T10:00:00Z";

fn record(id: &str, status: &str) -> SessionRecord {
    SessionRecord {
        id: id.into(),
        owner_user_id: "usr_A".into(),
        host_id: "hst_A".into(),
        workspace: "storm".into(),
        provider: "claude-code".into(),
        provider_kind: "cli".into(),
        interaction: "terminal".into(),
        provider_fallback: None,
        status: status.into(),
        end_reason: None,
        signal: None,
        exit_code: None,
        created_at: NOW.into(),
        started_at: None,
        ended_at: None,
        last_activity: None,
        egress: "host".into(),
        cols: 80,
        rows: 24,
        title: None,
        activity: None,
        harness_session: None,
        resumed_from: None,
    }
}

fn store() -> (Store, tempdir::TempDir) {
    let d = tempdir::TempDir::new("storm-spawn").unwrap();
    (Store::open(&d.path().join("agent.db")).unwrap(), d)
}

/// A root `ags_P` with a child `ags_C` and a grandchild `ags_G`.
fn tree(s: &mut Store) {
    s.insert(&record("ags_P", "running")).unwrap();
    s.insert_spawned(&record("ags_C", "running"), "ags_P", "child task", NOW)
        .unwrap();
    s.insert_spawned(&record("ags_G", "running"), "ags_C", "grandchild task", NOW)
        .unwrap();
}

fn request(id: &str, kind: &str, requester: &str, target: Option<&str>) -> Request {
    Request {
        id: id.into(),
        kind: kind.into(),
        requester: requester.into(),
        target: target.map(Into::into),
        payload: r#"{"text":"hello"}"#.into(),
        created_at: NOW.into(),
        expires_at: "2026-10-25T10:10:00Z".into(),
        state: "pending".into(),
        decided_by_device: None,
        decided_at: None,
        outcome: None,
        reason: None,
        session: None,
    }
}

fn raw(s: &Store) -> &Connection {
    &s.conn
}

// ---- L-1: lineage is immutable --------------------------------------------

/// Every SQL write to lineage in `src`: an UPDATE, DELETE, REPLACE, ALTER or
/// DROP of `session_lineage`, or any UPDATE naming a lineage column.
fn lineage_writes(src: &str) -> Vec<String> {
    let flat = src
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut found = Vec::new();
    for verb in [
        "update session_lineage",
        "delete from session_lineage",
        "replace into session_lineage",
        "alter table session_lineage",
        "drop table session_lineage",
        "drop trigger",
    ] {
        if flat.contains(verb) {
            found.push(verb.to_string());
        }
    }
    let mut at = 0;
    while let Some(i) = flat[at..].find("update ") {
        let start = at + i;
        at = start + 7;
        let word = flat[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if !word || flat[..start].ends_with("before ") {
            continue;
        }
        let stmt = flat[start..].split('"').next().unwrap_or_default();
        if ["parent_session", "root_session", "depth"]
            .iter()
            .any(|c| stmt.contains(c))
        {
            found.push(stmt.to_string());
        }
    }
    found
}

#[test]
fn no_product_code_writes_lineage_after_it_is_set() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut writes = Vec::new();
    let mut inserts = 0;
    for entry in walkdir::WalkDir::new(&src) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("spawn_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(path).unwrap();
        for w in lineage_writes(&text) {
            writes.push(format!("{}: {w}", path.display()));
        }
        inserts += text.matches("INSERT INTO session_lineage").count();
    }
    assert_eq!(writes, Vec::<String>::new());
    assert_eq!(inserts, 1, "lineage is written in one place, at approval");
}

/// The scan itself catches what it is for, so a silent pass means something.
#[test]
fn the_lineage_scan_finds_a_planted_write() {
    for planted in [
        r#""UPDATE session_lineage SET depth = 0""#,
        r#""UPDATE sessions SET parent_session = ?1 WHERE id = ?2""#,
        r#""DELETE FROM session_lineage WHERE session_id = ?1""#,
        r#""INSERT OR REPLACE INTO session_lineage VALUES (?1)""#,
        r#""DROP TRIGGER lineage_no_update""#,
    ] {
        assert!(!lineage_writes(planted).is_empty(), "{planted}");
    }
    assert!(lineage_writes(r#""BEFORE UPDATE OF parent_session ON x""#).is_empty());
    assert!(lineage_writes(r#""UPDATE sessions SET status = ?2""#).is_empty());
    assert!(
        lineage_writes(r#""CREATE TRIGGER no_update BEFORE UPDATE ON t SET depth""#).is_empty()
    );
}

#[test]
fn lineage_is_set_at_approval_from_the_parent() {
    let (mut s, _d) = store();
    tree(&mut s);
    let top = s.lineage("ags_P").unwrap();
    assert_eq!(
        (top.parent_session, top.root_session.as_str(), top.depth),
        (None, "ags_P", 0)
    );
    let c = s.lineage("ags_C").unwrap();
    assert_eq!(
        (
            c.parent_session.as_deref(),
            c.root_session.as_str(),
            c.depth
        ),
        (Some("ags_P"), "ags_P", 1)
    );
    let g = s.lineage("ags_G").unwrap();
    assert_eq!(
        (
            g.parent_session.as_deref(),
            g.root_session.as_str(),
            g.depth
        ),
        (Some("ags_C"), "ags_P", 2)
    );
    assert_eq!(s.task("ags_G").unwrap().as_deref(), Some("grandchild task"));
}

#[test]
fn the_database_refuses_every_change_to_lineage() {
    let (mut s, _d) = store();
    tree(&mut s);
    let before = s.lineage("ags_G").unwrap();
    for sql in [
        "UPDATE session_lineage SET parent_session = 'ags_P' WHERE session_id = 'ags_G'",
        "UPDATE session_lineage SET root_session = 'ags_X' WHERE session_id = 'ags_G'",
        "UPDATE session_lineage SET depth = 1 WHERE session_id = 'ags_G'",
        "UPDATE session_lineage SET created_at = 'x'",
        "DELETE FROM session_lineage WHERE session_id = 'ags_G'",
        "INSERT OR REPLACE INTO session_lineage VALUES ('ags_G', 'ags_P', 'ags_P', 1, 'x')",
        "INSERT INTO session_lineage VALUES ('ags_G', 'ags_P', 'ags_P', 1, 'x')",
        "INSERT INTO session_lineage VALUES ('ags_Z', 'ags_Z', 'ags_Z', 1, 'x')",
        "INSERT INTO session_lineage VALUES ('ags_Z', 'ags_G', 'ags_P', 1, 'x')",
        "INSERT INTO session_lineage VALUES ('ags_Z', 'ags_P', 'ags_P', 0, 'x')",
        "INSERT INTO session_lineage VALUES ('ags_Z', 'ags_G', 'ags_P', 0, 'x')",
    ] {
        assert!(raw(&s).execute(sql, []).is_err(), "{sql}");
    }
    assert_eq!(s.lineage("ags_G").unwrap(), before);
}

/// Ending, updating, resuming and dismissing a child never touch its lineage
/// (L-1, C7).
#[test]
fn lifecycle_leaves_lineage_alone() {
    let (mut s, _d) = store();
    tree(&mut s);
    let before = s.lineage("ags_C").unwrap();
    let mut c = s.get("ags_C").unwrap().unwrap();
    c.status = "stopped".into();
    c.end_reason = Some("cascade".into());
    s.update(&c).unwrap();
    let mut resumed = record("ags_R", "running");
    resumed.resumed_from = Some("ags_C".into());
    s.insert(&resumed).unwrap();
    let r = s.lineage("ags_R").unwrap();
    assert_eq!(
        (r.parent_session, r.depth),
        (None, 0),
        "C7: a new top-level session"
    );
    assert!(
        !s.descendants("ags_P", false)
            .unwrap()
            .contains(&"ags_R".to_string())
    );
    s.delete("ags_C").unwrap();
    assert_eq!(s.lineage("ags_C").unwrap(), before, "kept as history");
    assert_eq!(s.lineage("ags_G").unwrap().root_session, "ags_P");
}

#[test]
fn a_child_needs_a_recorded_parent_and_a_bounded_task() {
    let (mut s, _d) = store();
    assert!(
        s.insert_spawned(&record("ags_C", "starting"), "ags_NONE", "t", NOW)
            .is_err()
    );
    assert!(s.get("ags_C").unwrap().is_none());
    s.insert(&record("ags_P", "running")).unwrap();
    let long = "x".repeat(16 * 1024 + 1);
    assert!(
        s.insert_spawned(&record("ags_C", "starting"), "ags_P", &long, NOW)
            .is_err()
    );
    assert!(
        raw(&s)
            .execute(
                "INSERT INTO session_task VALUES ('ags_C', ?1, 'x')",
                [&long]
            )
            .is_err()
    );
    let exact = "x".repeat(16 * 1024);
    s.insert_spawned(&record("ags_C", "starting"), "ags_P", &exact, NOW)
        .unwrap();
    assert!(
        raw(&s)
            .execute("UPDATE session_task SET text = 'other'", [])
            .is_err(),
        "the approved task is fixed"
    );
}

#[test]
fn descendants_are_nearest_first_and_unknown_is_live() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.insert_spawned(&record("ags_D", "completed"), "ags_P", "t", NOW)
        .unwrap();
    s.insert_spawned(&record("ags_U", "unknown"), "ags_P", "t", NOW)
        .unwrap();
    assert_eq!(
        s.descendants("ags_P", false).unwrap(),
        ["ags_C", "ags_D", "ags_U", "ags_G"]
    );
    assert_eq!(
        s.descendants("ags_P", true).unwrap(),
        ["ags_C", "ags_U", "ags_G"]
    );
    assert_eq!(s.descendants("ags_C", true).unwrap(), ["ags_G"]);
}

// ---- 134c: the spawn switch -------------------------------------------------

#[test]
fn spawn_is_off_by_default_and_not_inherited() {
    let (mut s, _d) = store();
    s.insert(&record("ags_P", "running")).unwrap();
    assert!(!s.spawn_enabled("ags_P").unwrap());
    s.set_spawn("ags_P", true, Some("dev_A"), NOW).unwrap();
    assert!(s.spawn_enabled("ags_P").unwrap());
    s.insert_spawned(&record("ags_C", "starting"), "ags_P", "t", NOW)
        .unwrap();
    assert!(!s.spawn_enabled("ags_C").unwrap());
    assert!(
        raw(&s)
            .execute(
                "INSERT INTO session_spawn VALUES ('ags_X', 2, NULL, 'x')",
                []
            )
            .is_err()
    );
}

#[test]
fn revoking_spawn_refuses_pending_requests_at_once() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.set_spawn("ags_P", true, Some("dev_A"), NOW).unwrap();
    s.insert_request(&request("agr_1", "start", "ags_P", None))
        .unwrap();
    s.insert_request(&request("agr_2", "message", "ags_P", Some("ags_C")))
        .unwrap();
    s.insert_request(&request("agr_3", "start", "ags_C", None))
        .unwrap();
    assert_eq!(s.set_spawn("ags_P", false, Some("dev_A"), NOW).unwrap(), 2);
    for id in ["agr_1", "agr_2"] {
        let r = s.request(id).unwrap().unwrap();
        assert_eq!(
            (r.state.as_str(), r.outcome.as_deref(), r.reason.as_deref()),
            ("decided", Some("refused"), Some("revoked"))
        );
    }
    assert_eq!(s.request("agr_3").unwrap().unwrap().state, "pending");
    assert_eq!(s.set_spawn("ags_P", false, None, NOW).unwrap(), 0);
}

// ---- 134o: requests -----------------------------------------------------------

#[test]
fn requests_keep_their_shape_and_are_decided_once() {
    let (mut s, _d) = store();
    tree(&mut s);
    assert!(
        s.insert_request(&request("agr_X", "start", "ags_P", Some("ags_C")))
            .is_err()
    );
    assert!(
        s.insert_request(&request("agr_X", "end", "ags_P", None))
            .is_err()
    );
    assert!(
        s.insert_request(&request("agr_X", "spawn", "ags_P", None))
            .is_err()
    );
    s.insert_request(&request("agr_1", "end", "ags_P", Some("ags_C")))
        .unwrap();
    let c = raw(&s);
    for sql in [
        "UPDATE agent_requests SET payload = 'other' WHERE id = 'agr_1'",
        "UPDATE agent_requests SET target = 'ags_G' WHERE id = 'agr_1'",
        "UPDATE agent_requests SET expires_at = '2099-01-01T00:00:00Z' WHERE id = 'agr_1'",
        "UPDATE agent_requests SET state = 'decided' WHERE id = 'agr_1'",
        "UPDATE agent_requests SET state = 'decided', outcome = 'refused' WHERE id = 'agr_1'",
        "UPDATE agent_requests SET state = 'decided', outcome = 'started' WHERE id = 'agr_1'",
        "UPDATE agent_requests SET state = 'decided', outcome = 'done' WHERE id = 'agr_1'",
    ] {
        assert!(c.execute(sql, []).is_err(), "{sql}");
    }
    c.execute(
        "UPDATE agent_requests SET state = 'approving' WHERE id = 'agr_1' AND state = 'pending'",
        [],
    )
    .unwrap();
    assert_eq!(
        c.execute(
            "UPDATE agent_requests SET state = 'approving' WHERE id = 'agr_1' AND state = 'pending'",
            [],
        )
        .unwrap(),
        0,
        "P-3: a second approval finds it taken"
    );
    c.execute(
        "UPDATE agent_requests SET state = 'decided', outcome = 'ended',
             decided_by_device = 'dev_A', decided_at = ?1 WHERE id = 'agr_1'",
        [NOW],
    )
    .unwrap();
    assert!(
        c.execute(
            "UPDATE agent_requests SET outcome = 'denied' WHERE id = 'agr_1'",
            []
        )
        .is_err()
    );
    assert_eq!(
        s.refuse_requests_of("ags_P", "requester_ended", NOW)
            .unwrap(),
        0
    );
    assert_eq!(
        s.request("agr_1").unwrap().unwrap().outcome.as_deref(),
        Some("ended")
    );
}

// ---- 134j: the inbox ------------------------------------------------------------

#[test]
fn the_inbox_numbers_messages_and_names_its_sender() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.insert_request(&request("agr_1", "message", "ags_P", Some("ags_C")))
        .unwrap();
    let a = s
        .queue_message(
            "ags_C",
            "ags_P",
            "hi",
            Some("agr_1"),
            "pending_approval",
            NOW,
        )
        .unwrap();
    let b = s
        .queue_message("ags_C", "storm", "report now", None, "queued", NOW)
        .unwrap();
    assert_eq!((a, b), (1, 2));
    let inbox = s.inbox("ags_C").unwrap();
    assert_eq!(
        inbox
            .iter()
            .map(|m| (m.sender.as_str(), m.state.as_str()))
            .collect::<Vec<_>>(),
        [("ags_P", "pending_approval"), ("storm", "queued")]
    );
    assert!(
        s.queue_message("ags_C", "storm", "x", Some("agr_1"), "queued", NOW)
            .is_err()
    );
    assert!(
        s.queue_message("ags_C", "ags_P", "x", None, "queued", NOW)
            .is_err()
    );
    assert!(
        s.queue_message("ags_C", "storm", "x", None, "read", NOW)
            .is_err()
    );
    let long = "x".repeat(8 * 1024 + 1);
    assert!(
        s.queue_message("ags_C", "storm", &long, None, "queued", NOW)
            .is_err()
    );
    assert!(
        raw(&s)
            .execute(
                "INSERT INTO session_inbox VALUES ('ags_C', 9, 'storm', ?1, NULL, 'queued', 'x')",
                [&long],
            )
            .is_err()
    );
}

// ---- L-3…L-7: cascades ------------------------------------------------------------

#[test]
fn a_cascade_snapshots_live_descendants_and_refuses_the_parents_requests() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.insert_spawned(&record("ags_D", "completed"), "ags_P", "t", NOW)
        .unwrap();
    s.insert_request(&request("agr_1", "start", "ags_P", None))
        .unwrap();
    s.insert_request(&request("agr_2", "start", "ags_C", None))
        .unwrap();
    let targets = s
        .open_cascade("cas_1", "human_end", "ags_P", Some("stopped"), false, NOW)
        .unwrap();
    assert_eq!(targets, ["ags_C", "ags_G"], "ended children are untouched");
    let r = s.request("agr_1").unwrap().unwrap();
    assert_eq!(r.reason.as_deref(), Some("requester_ended"));
    assert_eq!(s.request("agr_2").unwrap().unwrap().state, "pending");
    let c = s.cascade("cas_1").unwrap().unwrap();
    assert_eq!(c.completed_at, None);
    assert_eq!(
        c.targets,
        [
            ("ags_C".to_string(), "pending".to_string()),
            ("ags_G".to_string(), "pending".to_string())
        ]
    );
    assert!(
        raw(&s)
            .execute(
                "UPDATE spawn_cascades SET parent_end_reason = 'completed'",
                []
            )
            .is_err(),
        "L-5: the parent's reason is never overwritten"
    );
}

#[test]
fn end_all_children_keeps_the_parents_requests() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.insert_request(&request("agr_1", "start", "ags_P", None))
        .unwrap();
    assert!(
        s.open_cascade("cas_x", "end_all_children", "ags_P", Some("x"), false, NOW)
            .is_err(),
        "the parent is not ending, so it has no end reason"
    );
    s.open_cascade("cas_1", "end_all_children", "ags_P", None, false, NOW)
        .unwrap();
    assert_eq!(s.request("agr_1").unwrap().unwrap().state, "pending");
}

#[test]
fn cascade_targets_end_once_and_the_cascade_completes() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.open_cascade(
        "cas_1",
        "parent_end",
        "ags_P",
        Some("completed"),
        false,
        NOW,
    )
    .unwrap();
    assert!(
        s.set_cascade_target("cas_1", "ags_C", "unreachable", NOW)
            .unwrap()
    );
    assert!(
        s.set_cascade_target("cas_1", "ags_C", "ended", NOW)
            .unwrap()
    );
    assert!(
        !s.set_cascade_target("cas_1", "ags_C", "ended", NOW)
            .unwrap(),
        "an end sent to an ended session is a no-op"
    );
    assert!(
        !s.set_cascade_target("cas_1", "ags_C", "pending", NOW)
            .unwrap()
    );
    assert!(
        raw(&s)
            .execute(
                "UPDATE spawn_cascade_targets SET state = 'pending' WHERE session_id = 'ags_C'",
                []
            )
            .is_err()
    );
    assert_eq!(s.open_cascades().unwrap(), ["cas_1"]);
    assert!(
        s.set_cascade_target("cas_1", "ags_G", "already_ended", NOW)
            .unwrap()
    );
    assert!(s.cascade("cas_1").unwrap().unwrap().completed_at.is_some());
    assert!(s.open_cascades().unwrap().is_empty());
    s.open_cascade(
        "cas_2",
        "parent_end",
        "ags_G",
        Some("completed"),
        false,
        NOW,
    )
    .unwrap();
    assert!(
        s.cascade("cas_2").unwrap().unwrap().completed_at.is_some(),
        "nothing to stop"
    );
}

#[test]
fn open_cascades_survive_a_restart() {
    let d = tempdir::TempDir::new("storm-spawn").unwrap();
    let path = d.path().join("agent.db");
    {
        let mut s = Store::open(&path).unwrap();
        tree(&mut s);
        s.open_cascade("cas_1", "human_end", "ags_P", Some("stopped"), true, NOW)
            .unwrap();
        s.set_cascade_target("cas_1", "ags_C", "ended", NOW)
            .unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.open_cascades().unwrap(), ["cas_1"]);
    let c = s.cascade("cas_1").unwrap().unwrap();
    assert!(c.report_first);
    assert_eq!(c.targets[1], ("ags_G".to_string(), "pending".to_string()));
    s.set_cascade_target("cas_1", "ags_G", "ended", NOW)
        .unwrap();
    assert!(s.open_cascades().unwrap().is_empty());
}

/// L-4: a child keeps the reason of whatever ended it first; L-5: it carries
/// a copy of its parent's reason.
#[test]
fn the_first_cause_wins() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.open_cascade(
        "cas_1",
        "end_all_children",
        "ags_C",
        None,
        false,
        "2026-10-25T10:00:00Z",
    )
    .unwrap();
    s.open_cascade(
        "cas_2",
        "parent_end",
        "ags_P",
        Some("failed"),
        false,
        "2026-10-25T10:00:05Z",
    )
    .unwrap();
    let g = s.end_cause("ags_G").unwrap().unwrap();
    assert_eq!(
        (
            g.end_reason.as_str(),
            g.parent_end_reason,
            g.cascade_id.as_str()
        ),
        ("cascade", None, "cas_1")
    );
    let c = s.end_cause("ags_C").unwrap().unwrap();
    assert_eq!(
        (
            c.end_reason.as_str(),
            c.parent_end_reason.as_deref(),
            c.cascade_id.as_str()
        ),
        ("parent_ended", Some("failed"), "cas_2")
    );
    assert!(s.end_cause("ags_P").unwrap().is_none());
}

// ---- C8: what a dismissal takes ------------------------------------------------------

#[test]
fn dismissing_a_session_takes_its_spawn_state_and_keeps_history() {
    let (mut s, _d) = store();
    tree(&mut s);
    s.set_spawn("ags_C", true, Some("dev_A"), NOW).unwrap();
    s.set_result(
        "ags_C",
        &SpawnResult {
            outcome: "succeeded".into(),
            summary: "s".into(),
            details: None,
            reported_at: NOW.into(),
        },
    )
    .unwrap();
    s.queue_message("ags_C", "storm", "report now", None, "queued", NOW)
        .unwrap();
    s.insert_request(&request("agr_from", "start", "ags_C", None))
        .unwrap();
    s.insert_request(&request("agr_to", "end", "ags_P", Some("ags_C")))
        .unwrap();
    s.open_cascade("cas_1", "human_end", "ags_C", Some("stopped"), false, NOW)
        .unwrap();
    let lineage = s.lineage("ags_C").unwrap();
    s.delete("ags_C").unwrap();
    assert!(!s.spawn_enabled("ags_C").unwrap());
    assert_eq!(s.task("ags_C").unwrap(), None);
    assert_eq!(s.result("ags_C").unwrap(), None);
    assert!(s.inbox("ags_C").unwrap().is_empty());
    assert_eq!(
        s.request("agr_from").unwrap().unwrap().reason.as_deref(),
        Some("requester_ended")
    );
    assert_eq!(
        s.request("agr_to").unwrap().unwrap().reason.as_deref(),
        Some("target_gone")
    );
    assert_eq!(s.lineage("ags_C").unwrap(), lineage);
    assert!(s.cascade("cas_1").unwrap().is_some());
    assert_eq!(s.task("ags_G").unwrap().as_deref(), Some("grandchild task"));
}

#[test]
fn a_later_result_replaces_an_earlier_one() {
    let (mut s, _d) = store();
    tree(&mut s);
    for (outcome, at) in [
        ("blocked", "2026-10-25T10:00:00Z"),
        ("succeeded", "2026-10-25T10:05:00Z"),
    ] {
        s.set_result(
            "ags_C",
            &SpawnResult {
                outcome: outcome.into(),
                summary: format!("{outcome} summary"),
                details: Some("d".into()),
                reported_at: at.into(),
            },
        )
        .unwrap();
    }
    let r = s.result("ags_C").unwrap().unwrap();
    assert_eq!(
        (r.outcome.as_str(), r.reported_at.as_str()),
        ("succeeded", "2026-10-25T10:05:00Z")
    );
    assert!(
        s.set_result(
            "ags_C",
            &SpawnResult {
                outcome: String::new(),
                summary: "s".into(),
                details: None,
                reported_at: NOW.into(),
            },
        )
        .is_err()
    );
}

// ---- migration -------------------------------------------------------------------

const SPAWN_OBJECTS: &str = "SELECT COUNT(*) FROM sqlite_master WHERE name IN (
    'session_lineage', 'session_spawn', 'session_task', 'session_result', 'agent_requests',
    'session_inbox', 'spawn_cascades', 'spawn_cascade_targets', 'lineage_no_update',
    'lineage_no_delete', 'lineage_no_replace', 'task_no_update', 'request_text_fixed',
    'request_decided_once', 'cascade_fixed', 'cascade_target_final')";

/// A database from before decision 134 gains its tables and triggers on boot,
/// its sessions read as top-level with spawn off, and booting again changes
/// nothing.
#[test]
fn a_pre_134_database_is_migrated_on_boot() {
    let d = tempdir::TempDir::new("storm-spawn").unwrap();
    let path = d.path().join("agent/agent.db");
    {
        let fresh = {
            std::fs::create_dir_all(d.path().join("agent")).unwrap();
            Store::open(&path).unwrap()
        };
        fresh.insert(&record("ags_OLD", "running")).unwrap();
        // Drop back to exactly the schema a pre-134 build leaves.
        fresh
            .conn
            .execute_batch(
                "DROP TABLE session_lineage; DROP TABLE session_spawn; DROP TABLE session_task;
                 DROP TABLE session_result; DROP TABLE agent_requests; DROP TABLE session_inbox;
                 DROP TABLE spawn_cascades; DROP TABLE spawn_cascade_targets;",
            )
            .unwrap();
        let left: i64 = fresh
            .conn
            .query_row(SPAWN_OBJECTS, [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }
    for boot in 0..3 {
        let m = AgentManager::open(d.path()).unwrap();
        let s = m.store.lock().unwrap();
        let objects: i64 = s.conn.query_row(SPAWN_OBJECTS, [], |r| r.get(0)).unwrap();
        assert_eq!(objects, 16, "boot {boot}");
        assert_eq!(s.get("ags_OLD").unwrap().unwrap().status, "unknown");
        let l = s.lineage("ags_OLD").unwrap();
        assert_eq!((l.parent_session, l.depth), (None, 0));
        assert!(!s.spawn_enabled("ags_OLD").unwrap());
        if boot == 0 {
            drop(s);
            let mut s = m.store.lock().unwrap();
            s.insert_spawned(&record("ags_NEW", "running"), "ags_OLD", "t", NOW)
                .unwrap();
        } else {
            assert_eq!(s.lineage("ags_NEW").unwrap().depth, 1, "kept across boots");
            assert_eq!(s.task("ags_NEW").unwrap().as_deref(), Some("t"));
        }
    }
}

/// The oldest schema the tests know (before D15 AM42) migrates too.
#[test]
fn the_oldest_database_reaches_134() {
    let d = tempdir::TempDir::new("storm-spawn").unwrap();
    std::fs::create_dir_all(d.path().join("agent")).unwrap();
    let conn = Connection::open(d.path().join("agent/agent.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (
             id TEXT PRIMARY KEY, owner_user_id TEXT NOT NULL, host_id TEXT NOT NULL,
             workspace TEXT NOT NULL, provider TEXT NOT NULL, provider_kind TEXT NOT NULL,
             interaction TEXT NOT NULL, provider_fallback TEXT, status TEXT NOT NULL,
             end_reason TEXT, signal INTEGER, exit_code INTEGER, created_at TEXT NOT NULL,
             started_at TEXT, ended_at TEXT, last_activity TEXT,
             egress TEXT NOT NULL DEFAULT 'host', cols INTEGER NOT NULL, rows INTEGER NOT NULL);
         INSERT INTO sessions VALUES ('ags_OLD', 'usr_A', 'hst_A', 'w', 'shell', 'cli',
             'terminal', NULL, 'completed', NULL, NULL, 0, '2026-10-01T00:00:00Z',
             NULL, NULL, NULL, 'host', 80, 24);",
    )
    .unwrap();
    drop(conn);
    let m = AgentManager::open(d.path()).unwrap();
    let mut s = m.store.lock().unwrap();
    s.insert_spawned(&record("ags_C", "starting"), "ags_OLD", "t", NOW)
        .unwrap();
    assert_eq!(s.descendants("ags_OLD", false).unwrap(), ["ags_C"]);
}

// ---- 134-2: the spawn switch ------------------------------------------------

use super::store::spawn::{same_workspace, shares_workspace};
use super::{AgentError, Envelope, Hello, Launch, LaunchMeta, McpGrant};

const VERIFIED: &str = r#"{"state": "verified", "mechanism": "so_peerpidfd", "reason": null}"#;

fn isolated() -> serde_json::Value {
    serde_json::json!({"isolation": {"state": "verified", "mechanism": "systemd_units", "reason": null}})
}

fn hello(identity: Option<&str>) -> Hello {
    hello_with(identity, serde_json::json!({}))
}

fn hello_with(identity: Option<&str>, extra: serde_json::Value) -> Hello {
    let providers: Vec<_> = ["claude-code", "opencode", "shell"]
        .iter()
        .map(|id| serde_json::json!({"id": id, "kind": "cli", "interactions": ["terminal"], "available": true}))
        .collect();
    let mut caps = serde_json::json!({
        "providers": providers,
        "workspaces": ["storm", "storm/site", "other"],
        "workspace_paths": {"storm": "/w/storm", "storm/site": "/w/storm/site", "other": "/w/other"},
        "max_sessions": 16,
        "mcp_bridge": true,
        "resume": ["claude-code"],
    });
    if let Some(identity) = identity {
        caps["identity"] = serde_json::from_str(identity).unwrap();
    }
    for (k, v) in extra.as_object().unwrap() {
        caps[k] = v.clone();
    }
    serde_json::from_value(caps).unwrap()
}

fn host(identity: Option<&str>) -> (AgentManager, tempdir::TempDir, UnboundedReceiver<Envelope>) {
    let d = tempdir::TempDir::new("storm-spawn-switch").unwrap();
    let m = AgentManager::open(d.path()).unwrap();
    let (_, rx) = m.connect_host("hst_A");
    m.hello("hst_A", hello(identity)).unwrap();
    (m, d, rx)
}

/// A host whose identity is verified and, by the test-only hook, isolated.
fn verified_host() -> (AgentManager, tempdir::TempDir, UnboundedReceiver<Envelope>) {
    let (m, d, rx) = host(Some(VERIFIED));
    m.verify_isolation_for_tests("hst_A");
    (m, d, rx)
}

fn start(
    m: &AgentManager,
    workspace: &str,
    provider: &str,
    resume_of: Option<&str>,
) -> Result<SessionRecord, AgentError> {
    launch_as(m, workspace, provider, resume_of, false)
}

/// A launch with "Allow this session to start agents" on (141e).
fn start_on(
    m: &AgentManager,
    workspace: &str,
    provider: &str,
) -> Result<SessionRecord, AgentError> {
    launch_as(m, workspace, provider, None, true)
}

fn launch_as(
    m: &AgentManager,
    workspace: &str,
    provider: &str,
    resume_of: Option<&str>,
    spawn: bool,
) -> Result<SessionRecord, AgentError> {
    let launch: Launch = serde_json::from_value(serde_json::json!({
        "host_id": "hst_A", "workspace": workspace, "provider": provider,
        "terminal": {"cols": 80, "rows": 24}, "resume_of": resume_of, "spawn": spawn,
    }))
    .unwrap();
    let storm = McpGrant {
        id: "storm".into(),
        slug: "storm".into(),
        plugin_origin: None,
    };
    m.launch("usr_A", launch, LaunchMeta::default(), vec![storm])
        .map(|(r, ..)| r)
}

fn report(m: &AgentManager, id: &str, status: &str, harness: Option<&str>) {
    let report = serde_json::from_value(serde_json::json!({
        "status": status, "exit_code": 0, "harness_session": harness,
    }))
    .unwrap();
    m.host_status("hst_A", id, &report).unwrap();
}

fn refusal(m: &AgentManager, id: &str, enabled: bool) -> &'static str {
    m.set_spawn(id, enabled, Some("dev_A"))
        .unwrap()
        .unwrap_err()
}

use tokio::sync::mpsc::UnboundedReceiver;

#[test]
fn c0_refuses_an_old_host_and_unverified_identity_or_isolation() {
    // An old host says nothing about identity.
    let (m, _d, _rx) = host(None);
    let s = start(&m, "storm", "claude-code", None).unwrap();
    assert_eq!(refusal(&m, &s.id, true), "identity_unverified");

    // A host that says its identity is unavailable.
    let (m, _d, _rx) = host(Some(
        r#"{"state": "unavailable", "mechanism": "so_peerpidfd", "reason": "ptrace_scope"}"#,
    ));
    let s = start(&m, "storm", "claude-code", None).unwrap();
    assert_eq!(refusal(&m, &s.id, true), "identity_unverified");
    for state in ["unsupported", "VERIFIED", ""] {
        m.inventory(
            "hst_A",
            hello(Some(&format!(r#"{{"state": "{state}"}}"#))).capabilities,
        )
        .unwrap();
        assert_eq!(refusal(&m, &s.id, true), "identity_unverified", "{state}");
    }

    // Verified identity, isolation absent (an old host), or anything but
    // exactly `verified`.
    let (m, _d, _rx) = host(Some(VERIFIED));
    let s = start(&m, "storm", "claude-code", None).unwrap();
    assert_eq!(refusal(&m, &s.id, true), "isolation_unverified");
    for isolation in [
        serde_json::json!(null),
        serde_json::json!({"state": "unavailable", "mechanism": "systemd_units", "reason": "self_test"}),
        serde_json::json!({"state": "unavailable", "mechanism": null, "reason": "off"}),
        serde_json::json!({"state": "unsupported", "mechanism": null, "reason": "windows"}),
        serde_json::json!({"state": "VERIFIED", "mechanism": "systemd_units"}),
        serde_json::json!({"state": "isolated", "mechanism": "systemd_units"}),
        serde_json::json!({"mechanism": "systemd_units"}),
        serde_json::json!(true),
    ] {
        let claims = hello_with(
            Some(VERIFIED),
            serde_json::json!({ "isolation": isolation }),
        );
        m.inventory("hst_A", claims.capabilities).unwrap();
        assert_eq!(
            refusal(&m, &s.id, true),
            "isolation_unverified",
            "{isolation}"
        );
        assert_eq!(m.spawn_gate("hst_A"), Err("isolation_unverified"));
    }
    // Isolation verified without identity is no better.
    let (m, _d, _rx) = host(None);
    m.inventory("hst_A", hello_with(None, isolated()).capabilities)
        .unwrap();
    assert!(m.host_isolated("hst_A"));
    assert_eq!(m.spawn_gate("hst_A"), Err("identity_unverified"));
    let s = start(&m, "storm", "claude-code", None).unwrap();
    assert_eq!(refusal(&m, &s.id, true), "identity_unverified");
    assert!(!m.spawn_enabled(&s.id).unwrap(), "a refusal writes nothing");
    assert!(!m.spawn_offered(&s.id));
}

/// The integration of 134 with 136-5: a host whose `hello` says identity
/// and isolation are both verified, parsed as the route parses it, opens
/// C0; the session it is judged for is one launched once it held.
#[test]
fn a_verified_isolation_report_opens_c0() {
    let (m, _d, _rx) = host(Some(VERIFIED));
    let before = start(&m, "storm", "claude-code", None).unwrap();
    let hello: Hello = serde_json::from_str(
        r#"{"providers": [{"id": "claude-code", "kind": "cli", "interactions": ["terminal"], "available": true}],
            "workspaces": ["storm", "other"], "workspace_paths": {"storm": "/w/storm", "other": "/w/other"},
            "max_sessions": 16, "mcp_bridge": true,
            "identity": {"state": "verified", "mechanism": "so_peerpidfd", "reason": null},
            "isolation": {"state": "verified", "mechanism": "systemd_units", "reason": null},
            "sessions": [{"id": "BEFORE", "status": "running"}], "last_cmd_seq": 0}"#
            .replace("BEFORE", &before.id)
            .as_str(),
    )
    .unwrap();
    m.hello("hst_A", hello).unwrap();
    assert_eq!(m.spawn_gate("hst_A"), Ok(()));
    // Its CLI was configured before C0 held, so it never gets the tools.
    assert_eq!(refusal(&m, &before.id, true), "launched_without_asks");
    report(&m, &before.id, "completed", None);
    let s = start_on(&m, "storm", "claude-code").unwrap();
    assert!(m.spawn_offered(&s.id));

    // A downgrade in the inventory shuts it at once; a new report reopens it.
    let down = serde_json::json!({"isolation":
        {"state": "unavailable", "mechanism": "systemd_units", "reason": "no_passt"}});
    m.inventory("hst_A", hello_with(Some(VERIFIED), down).capabilities)
        .unwrap();
    assert!(!m.spawn_offered(&s.id));
    assert_eq!(m.spawn_gate("hst_A"), Err("isolation_unverified"));
    m.inventory("hst_A", hello_with(Some(VERIFIED), isolated()).capabilities)
        .unwrap();
    assert!(m.spawn_offered(&s.id));

    // A new link carries the last report over, but has said nothing yet:
    // neither half counts.
    let (_generation, _rx2) = m.connect_host("hst_A");
    assert_eq!(m.spawn_gate("hst_A"), Err("identity_unverified"));
    assert!(!m.spawn_offered(&s.id));
    // Its own inventory counts as its word.
    m.inventory("hst_A", hello_with(Some(VERIFIED), isolated()).capabilities)
        .unwrap();
    assert!(m.spawn_offered(&s.id));
}

#[test]
fn c0_holds_only_while_the_live_link_says_so() {
    let (m, _d, _rx) = verified_host();
    let s = start_on(&m, "storm", "claude-code").unwrap();
    assert!(m.spawn_offered(&s.id));

    // An identity downgrade hides the tools at once; the switch stays on.
    m.inventory(
        "hst_A",
        hello(Some(r#"{"state": "unavailable", "reason": "self_test"}"#)).capabilities,
    )
    .unwrap();
    assert!(!m.spawn_offered(&s.id));
    assert!(m.spawn_enabled(&s.id).unwrap());
    m.inventory("hst_A", hello_with(Some(VERIFIED), isolated()).capabilities)
        .unwrap();
    assert!(m.spawn_offered(&s.id));

    // A new link has not said it is isolated.
    let (generation, _rx2) = m.connect_host("hst_A");
    m.inventory("hst_A", hello(Some(VERIFIED)).capabilities)
        .unwrap();
    assert!(!m.spawn_offered(&s.id));
    m.disconnect_host("hst_A", generation);
    assert!(!m.spawn_offered(&s.id), "an offline host offers nothing");
    assert_eq!(refusal(&m, &s.id, true), "identity_unverified");
    // Revoking never waits for the host.
    assert!(!m.set_spawn(&s.id, false, None).unwrap().unwrap().enabled);
}

#[test]
fn spawn_starts_off_for_fresh_children_and_resumed_sessions() {
    let (m, _d, _rx) = verified_host();
    let plain = start(&m, "storm", "claude-code", None).unwrap();
    assert!(!m.spawn_enabled(&plain.id).unwrap());
    assert!(!m.spawn_offered(&plain.id));
    let parent = start_on(&m, "storm", "claude-code").unwrap();

    let mut child = record("ags_CHILD", "running");
    child.workspace = "other".into();
    m.store
        .lock()
        .unwrap()
        .insert_spawned(&child, &parent.id, "a task", NOW)
        .unwrap();
    assert!(!m.spawn_enabled("ags_CHILD").unwrap());
    assert!(!m.spawn_offered("ags_CHILD"));

    let uuid = "80d0cf8c-8176-43b4-8206-7908d74d1387";
    report(&m, &parent.id, "running", Some(uuid));
    report(&m, &parent.id, "completed", None);
    assert!(
        !m.spawn_offered(&parent.id),
        "an ended session offers nothing"
    );
    let resumed = start(&m, "storm", "claude-code", Some(&parent.id)).unwrap();
    assert_eq!(resumed.resumed_from.as_deref(), Some(parent.id.as_str()));
    assert!(!m.spawn_enabled(&resumed.id).unwrap());
    assert!(!m.spawn_offered(&resumed.id));
}

#[test]
fn only_a_live_session_of_a_spawning_adapter_can_turn_it_on() {
    let (m, _d, _rx) = verified_host();
    let shell = start(&m, "other", "shell", None).unwrap();
    assert_eq!(refusal(&m, &shell.id, true), "provider_cannot_spawn");
    report(&m, &shell.id, "completed", None);
    let s = start(&m, "storm", "opencode", None).unwrap();
    report(&m, &s.id, "completed", None);
    assert_eq!(refusal(&m, &s.id, true), "session_not_live");
    // Off is always allowed.
    assert_eq!(m.set_spawn(&s.id, false, None).unwrap().unwrap().refused, 0);
    assert!(matches!(
        m.set_spawn("ags_NOPE", true, None),
        Err(AgentError::NotFound(_))
    ));
}

/// LT-7: revoked with requests pending, they are refused at once; children
/// keep running; nothing new is offered.
#[test]
fn revoking_refuses_pending_requests_at_once_and_children_keep_running() {
    let (m, _d, _rx) = verified_host();
    let parent = start_on(&m, "storm", "claude-code").unwrap();
    {
        let mut s = m.store.lock().unwrap();
        let mut child = record("ags_CHILD", "running");
        child.workspace = "other".into();
        s.insert_spawned(&child, &parent.id, "a task", NOW).unwrap();
        s.insert_request(&request("agr_1", "start", &parent.id, None))
            .unwrap();
        s.insert_request(&request("agr_2", "end", &parent.id, Some("ags_CHILD")))
            .unwrap();
    }
    let change = m
        .set_spawn(&parent.id, false, Some("dev_A"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (change.was, change.enabled, change.refused),
        (true, false, 2)
    );
    for id in ["agr_1", "agr_2"] {
        let r = m.store.lock().unwrap().request(id).unwrap().unwrap();
        assert_eq!(
            (r.state.as_str(), r.outcome.as_deref(), r.reason.as_deref()),
            ("decided", Some("refused"), Some("revoked")),
            "{id}"
        );
    }
    assert_eq!(m.get("ags_CHILD").unwrap().status, "running");
    assert!(!m.spawn_offered(&parent.id), "new requests have no tool");
}

/// 141a: a spawning session does not hold its workspace. A launch beside
/// it and its switch beside another live session are both allowed.
#[test]
fn a_spawning_session_shares_its_workspace() {
    let (m, _d, _rx) = verified_host();
    let a = start_on(&m, "storm", "claude-code").unwrap();
    start_on(&m, "storm", "claude-code").unwrap();
    let nested = start(&m, "storm/site", "claude-code", None).unwrap();
    for workspace in ["storm", "storm/site"] {
        assert!(
            !start(&m, workspace, "claude-code", None)
                .unwrap()
                .is_ended()
        );
    }
    assert_eq!(m.shared_with(&a.id).unwrap(), 4, "b, nested and two more");
    report(&m, &nested.id, "completed", None);
    assert_eq!(m.shared_with(&a.id).unwrap(), 3);
    // `unknown` is live and still shares it.
    m.store.lock().unwrap().mark_unknown_on_boot().unwrap();
    assert_eq!(m.shared_with(&a.id).unwrap(), 3);
}

/// WT-8, as decision 141 amends it: WS-9 no longer refuses either way, and
/// the sharing it judges by the host's directories is what an approval card
/// reports. Two names for one directory, or one inside the other, are one
/// workspace; a name the host gives no directory for may be any of them.
#[test]
fn wt8_a_shared_workspace_allows_launch_and_spawn_both_ways() {
    let (m, _d, _rx) = host(Some(VERIFIED));
    let layout = |paths: serde_json::Value| {
        let mut extra = isolated();
        extra["workspaces"] = serde_json::json!(["storm", "alias", "site", "other", "bare"]);
        extra["workspace_paths"] = paths;
        hello_with(Some(VERIFIED), extra).capabilities
    };
    let paths = serde_json::json!({
        "storm": "/w/storm", "alias": "/w/storm/", "site": "/w/storm/site", "other": "/w/other",
    });
    m.inventory("hst_A", layout(paths.clone())).unwrap();

    for beside in ["storm", "alias", "site", "bare"] {
        // Spawn on, then a launch beside it.
        let a = start_on(&m, "storm", "claude-code").unwrap();
        let b = start(&m, beside, "claude-code", None).unwrap();
        let elsewhere = start(&m, "other", "shell", None).unwrap();
        assert_eq!(m.shared_with(&a.id).unwrap(), 1, "{beside}");
        report(&m, &a.id, "completed", None);
        // A live session beside it, then spawn on.
        let a = start_on(&m, "storm", "claude-code").unwrap();
        m.set_spawn(&a.id, false, Some("dev_A")).unwrap().unwrap();
        m.set_spawn(&a.id, true, Some("dev_A")).unwrap().unwrap();
        assert_eq!(m.shared_with(&a.id).unwrap(), 1, "{beside}");
        for r in [&a, &b, &elsewhere] {
            report(&m, &r.id, "completed", None);
        }
    }

    // A host that reports no directories: every pair may be one.
    m.inventory("hst_A", layout(serde_json::json!({}))).unwrap();
    let a = start_on(&m, "storm", "claude-code").unwrap();
    m.set_spawn(&a.id, false, Some("dev_A")).unwrap().unwrap();
    start(&m, "other", "claude-code", None).unwrap();
    m.set_spawn(&a.id, true, Some("dev_A")).unwrap().unwrap();
    assert_eq!(m.shared_with(&a.id).unwrap(), 1);
    // Once it does, the two are apart.
    m.inventory("hst_A", layout(paths)).unwrap();
    assert_eq!(m.shared_with(&a.id).unwrap(), 0);
}

#[test]
fn workspaces_are_shared_by_their_directories_or_when_unknown() {
    let paths: std::collections::HashMap<String, String> = [
        ("storm", "/w/storm"),
        ("alias", "/w/storm"),
        ("site", "/w/storm/site"),
        ("other", "/w/other"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    for (a, b) in [
        ("storm", "storm"),
        ("storm", "alias"),
        ("site", "storm"),
        ("storm", "site"),
        ("storm", "gone"),
        ("gone", "other"),
    ] {
        assert!(shares_workspace(a, b, &paths), "{a} {b}");
    }
    assert!(!shares_workspace("storm", "other", &paths));
    assert!(!shares_workspace("site", "other", &paths));
    assert!(shares_workspace("storm", "other", &Default::default()));
}

#[test]
fn workspaces_are_shared_when_one_is_inside_the_other() {
    for (a, b) in [
        ("/w/storm", "/w/storm"),
        ("/w/storm/", "/w/storm"),
        ("/w/storm", "/w/storm/site"),
        ("/w/storm/site", "/w/storm"),
        ("C:\\w\\storm", "C:\\w\\storm\\site"),
        ("/", "/w/storm"),
    ] {
        assert!(same_workspace(a, b), "{a} {b}");
    }
    for (a, b) in [("/w/storm", "/w/storm-site"), ("/w/storm", "/w/other")] {
        assert!(!same_workspace(a, b), "{a} {b}");
    }
}

fn start_command(rx: &mut UnboundedReceiver<Envelope>, id: &str) -> serde_json::Value {
    while let Ok(env) = rx.try_recv() {
        let v = serde_json::to_value(&env).unwrap();
        if v["type"] == "start" && v["session"] == id {
            return v;
        }
    }
    panic!("no start for {id}");
}

/// 134-8: a start says `spawn_asks` only where decision 134 could offer its
/// tools — C0 holds, a spawning adapter, the storm connection — and only
/// such a session's switch turns on, since a CLI reads its asks once.
#[test]
fn only_a_start_c0_held_for_carries_the_asks_and_only_it_can_spawn() {
    // Every host today: no asks, and the bytes of every start unchanged.
    let (m, _d, mut rx) = host(Some(VERIFIED));
    let before = start(&m, "storm", "claude-code", None).unwrap();
    let v = start_command(&mut rx, &before.id);
    assert!(v.get("spawn_asks").is_none(), "{v}");
    assert!(!m.store.lock().unwrap().spawn_asks(&before.id).unwrap());

    m.verify_isolation_for_tests("hst_A");
    assert_eq!(refusal(&m, &before.id, true), "launched_without_asks");
    assert!(!m.spawn_enabled(&before.id).unwrap());
    report(&m, &before.id, "completed", None);

    for provider in ["claude-code", "opencode", "codex"] {
        let s = start(&m, "storm", provider, None).unwrap();
        let v = start_command(&mut rx, &s.id);
        assert_eq!(v["spawn_asks"], serde_json::json!({}), "{provider}");
        assert!(m.store.lock().unwrap().spawn_asks(&s.id).unwrap());
        // 141e: asks alone are not enough; the launch turns it on.
        assert_eq!(refusal(&m, &s.id, true), "launched_without_spawn");
        report(&m, &s.id, "completed", None);
        let s = start_on(&m, "storm", provider).unwrap();
        assert_eq!(
            start_command(&mut rx, &s.id)["spawn_asks"],
            serde_json::json!({})
        );
        m.set_spawn(&s.id, false, Some("dev_A")).unwrap().unwrap();
        m.set_spawn(&s.id, true, Some("dev_A")).unwrap().unwrap();
        report(&m, &s.id, "completed", None);
    }
    let shell = start(&m, "storm", "shell", None).unwrap();
    assert!(
        start_command(&mut rx, &shell.id)
            .get("spawn_asks")
            .is_none()
    );
    report(&m, &shell.id, "completed", None);

    // Without the storm connection there is no Storm tool to ask about.
    let launch: Launch = serde_json::from_value(serde_json::json!({
        "host_id": "hst_A", "workspace": "storm", "provider": "claude-code",
        "terminal": {"cols": 80, "rows": 24},
    }))
    .unwrap();
    let (bare, ..) = m
        .launch("usr_A", launch, LaunchMeta::default(), Vec::new())
        .unwrap();
    assert!(start_command(&mut rx, &bare.id).get("spawn_asks").is_none());
    assert_eq!(refusal(&m, &bare.id, true), "launched_without_asks");
}

#[test]
fn dismissing_a_session_takes_its_asks_record() {
    let (m, _d, _rx) = verified_host();
    let s = start(&m, "storm", "claude-code", None).unwrap();
    assert!(m.store.lock().unwrap().spawn_asks(&s.id).unwrap());
    report(&m, &s.id, "completed", None);
    m.dismiss(&s.id).unwrap();
    assert!(!m.store.lock().unwrap().spawn_asks(&s.id).unwrap());
}

/// 141e: a launch with the switch on is refused, writing nothing, unless its
/// `start` carries `spawn_asks`; where it does, the switch is on before the
/// host can see `start`.
#[test]
fn a_launch_with_spawn_on_needs_the_asks_and_is_on_before_start() {
    let (m, _d, mut rx) = host(Some(VERIFIED));
    let refused = |r: Result<SessionRecord, AgentError>| match r {
        Err(AgentError::Conflict(code)) => code,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        refused(start_on(&m, "storm", "claude-code")),
        "spawn_unavailable"
    );
    m.verify_isolation_for_tests("hst_A");
    assert_eq!(refused(start_on(&m, "storm", "shell")), "spawn_unavailable");
    let bare: Launch = serde_json::from_value(serde_json::json!({
        "host_id": "hst_A", "workspace": "storm", "provider": "claude-code",
        "terminal": {"cols": 80, "rows": 24}, "spawn": true,
    }))
    .unwrap();
    assert!(matches!(
        m.launch("usr_A", bare, LaunchMeta::default(), Vec::new()),
        Err(AgentError::Conflict(code)) if code == "spawn_unavailable"
    ));
    assert!(m.list().unwrap().is_empty(), "no session was created");
    assert!(rx.try_recv().is_err(), "no start was sent");

    std::thread::scope(|scope| {
        let m = &m;
        let seen = scope.spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let env = loop {
                if let Ok(env) = rx.try_recv() {
                    break env;
                }
                assert!(std::time::Instant::now() < deadline, "no start");
                std::thread::yield_now();
            };
            let v = serde_json::to_value(&env).unwrap();
            assert_eq!(v["type"], "start");
            let id = v["session"].as_str().unwrap().to_string();
            (m.spawn_enabled(&id).unwrap(), v)
        });
        let s = launch_as(m, "storm", "claude-code", None, true).unwrap();
        let (on, start) = seen.join().unwrap();
        assert!(on, "the switch was on when start was sent");
        assert_eq!(start["session"], s.id.as_str());
        assert_eq!(start["spawn_asks"], serde_json::json!({}));
    });
    let s = &m.list().unwrap()[0];
    let view = m.spawn_view(s).unwrap().unwrap();
    assert_eq!(
        (view.enabled, view.launched_on, view.refused),
        (true, true, None)
    );
    // Off, then on again: it was launched with it.
    m.set_spawn(&s.id, false, None).unwrap().unwrap();
    assert!(m.spawn_turned_off(&s.id));
    m.set_spawn(&s.id, true, None).unwrap().unwrap();
    assert!(!m.spawn_turned_off(&s.id));
    report(&m, &s.id, "completed", None);
    m.dismiss(&s.id).unwrap();
    assert!(!m.store.lock().unwrap().spawn_launched(&s.id).unwrap());
}

/// 143a: a database from before `agent_unavailable` has its requests table
/// rebuilt at open, every row kept, and its indexes and triggers made again.
#[test]
fn a_requests_table_from_before_143_is_rebuilt_with_its_rows() {
    let d = tempdir::TempDir::new("storm-143-requests").unwrap();
    let path = d.path().join("agent.db");
    let old = Connection::open(&path).unwrap();
    old.execute_batch(
        "CREATE TABLE agent_requests (
             id TEXT PRIMARY KEY,
             kind TEXT NOT NULL CHECK (kind IN ('start', 'message', 'end')),
             requester TEXT NOT NULL, target TEXT, payload TEXT NOT NULL,
             created_at TEXT NOT NULL, expires_at TEXT NOT NULL,
             state TEXT NOT NULL CHECK (state IN ('pending', 'approving', 'decided')),
             decided_by_device TEXT, decided_at TEXT,
             outcome TEXT CHECK (outcome IN ('started', 'start_failed',
                 'delivered', 'ended', 'denied', 'expired', 'refused')),
             reason TEXT CHECK (reason IN ('limit', 'not_permitted', 'revoked',
                 'requester_ended', 'target_gone', 'identity_unverified',
                 'isolation_unverified')),
             session TEXT);
         CREATE INDEX requests_by_requester ON agent_requests(requester, state);
         CREATE TRIGGER request_decided_once
             BEFORE UPDATE ON agent_requests WHEN OLD.state = 'decided'
             BEGIN SELECT RAISE(ABORT, 'a request is decided once'); END;
         INSERT INTO agent_requests VALUES ('agr_OLD', 'start', 'ags_P', NULL,
             '{\"task\":\"t\"}', '2026-10-25T10:00:00Z', '2026-10-25T10:10:00Z',
             'decided', 'dev_A', '2026-10-25T10:01:00Z', 'refused', 'not_permitted', NULL);",
    )
    .unwrap();
    drop(old);
    let s = Store::open(&path).unwrap();
    let kept = s.request("agr_OLD").unwrap().unwrap();
    assert_eq!(
        (kept.outcome.as_deref(), kept.reason.as_deref()),
        (Some("refused"), Some("not_permitted"))
    );
    s.conn
        .execute(
            "INSERT INTO agent_requests (id, kind, requester, payload, created_at, expires_at,
                 state, outcome, reason)
             VALUES ('agr_NEW', 'start', 'ags_P', '{}', ?1, ?1, 'decided', 'refused',
                 'agent_unavailable')",
            [NOW],
        )
        .unwrap();
    assert!(
        s.conn
            .execute(
                "UPDATE agent_requests SET payload = 'x' WHERE id = 'agr_OLD'",
                []
            )
            .is_err(),
        "its triggers are back"
    );
    let objects: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE tbl_name = 'agent_requests'
                 AND type IN ('index', 'trigger') AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(objects, 5);
    drop(s);
    Store::open(&path).unwrap();
}
