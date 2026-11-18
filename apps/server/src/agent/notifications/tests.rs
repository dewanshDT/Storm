//! Decision 161, slice 161-1.

use serde_json::json;

use super::*;
use crate::agent::AgentManager;

// ---------------------------------------------------------------------
// The transition matrix, ported from `agent_notices_test.dart`
// ("which changes say something"), plus `notice_key`'s dedupe rules.
// ---------------------------------------------------------------------

fn program(state: &'static str, kind: Option<&'static str>, since: &str) -> ProgramStatus {
    ProgramStatus {
        state,
        kind,
        since: since.into(),
    }
}

fn view(status: &str) -> Snap {
    Snap {
        status: status.into(),
        ..Snap::default()
    }
}

fn working() -> Snap {
    Snap {
        status: "running".into(),
        activity: Some("working".into()),
        ..Snap::default()
    }
}

fn blocked(kind: &'static str) -> Snap {
    Snap {
        status: "running".into(),
        attention_kind: Some(kind),
        attention_since: Some("t0".into()),
        ..Snap::default()
    }
}

/// A session whose program reports its own state (decision 155).
fn reporting(state: &'static str, kind: Option<&'static str>, since: &str) -> Snap {
    Snap {
        status: "running".into(),
        activity: Some(
            if state == "working" {
                "working"
            } else {
                "idle"
            }
            .into(),
        ),
        attention_kind: match state {
            "blocked" => Some(match kind {
                Some("permission") => PERMISSION_PROMPT,
                Some("auth") => SIGN_IN,
                _ => QUESTION,
            }),
            _ => None,
        },
        attention_since: (state == "blocked").then(|| since.to_string()),
        program: Some(program(state, kind, since)),
        ..Snap::default()
    }
}

const T1: &str = "2026-11-14T12:00:00Z";
const T2: &str = "2026-11-14T12:05:00Z";

#[test]
fn entering_needs_you_from_any_state() {
    let after = blocked(PERMISSION_PROMPT);
    for before in [
        view("running"),
        working(),
        blocked(crate::agent::attention::BLOCKED),
    ] {
        assert_eq!(notice_of(&before, &after), Some(Notice::NeedsYou));
    }
    assert_eq!(
        notice_of(&blocked(QUESTION), &blocked(PERMISSION_PROMPT)),
        None,
        "already in the tier"
    );
}

/// `attention::BLOCKED` (134h) is its own, lower sidebar tier on the
/// client (`tierOf`, checked after `needsYou`) — not a needs-you kind.
#[test]
fn a_blocked_round_outcome_is_not_needs_you() {
    assert_eq!(
        notice_of(&view("running"), &blocked(crate::agent::attention::BLOCKED)),
        None
    );
}

#[test]
fn working_to_idle_completed_failed_or_stopped_is_stopped_working() {
    for status in ["running", "completed", "failed", "stopped"] {
        let after = if status == "running" {
            Snap {
                status: "running".into(),
                activity: Some("idle".into()),
                ..Snap::default()
            }
        } else {
            view(status)
        };
        assert_eq!(notice_of(&working(), &after), Some(Notice::StoppedWorking));
    }
    assert_eq!(notice_of(&view("running"), &view("completed")), None);
    assert_eq!(notice_of(&working(), &working()), None);
    let starting = Snap {
        status: "starting".into(),
        ..Snap::default()
    };
    let idle = Snap {
        status: "running".into(),
        activity: Some("idle".into()),
        ..Snap::default()
    };
    assert_eq!(notice_of(&starting, &idle), None);
}

#[test]
fn working_to_unknown_says_nothing() {
    assert_eq!(notice_of(&working(), &view("running")), None);
    let done_unknown = Snap {
        status: "running".into(),
        activity: None,
        program: Some(program("done", None, T1)),
        ..Snap::default()
    };
    assert_eq!(
        notice_of(&reporting("working", None, T1), &done_unknown),
        None
    );
}

#[test]
fn what_the_program_said_in_its_words() {
    let busy = reporting("working", None, T1);
    assert_eq!(
        notice_of(&busy, &reporting("done", None, T1)),
        Some(Notice::FinishedTurn)
    );
    assert_eq!(
        notice_of(&busy, &reporting("error", None, T1)),
        Some(Notice::HitError)
    );
    for kind in [Some("permission"), Some("question"), Some("auth"), None] {
        assert_eq!(
            notice_of(&busy, &reporting("blocked", kind, T1)),
            Some(Notice::NeedsYou),
            "{kind:?}"
        );
    }
    assert_eq!(
        notice_of(&busy, &reporting("idle", None, T1)),
        Some(Notice::StoppedWorking),
        "Esc or Ctrl+C: idle without done"
    );
    assert_eq!(
        notice_of(
            &reporting("blocked", Some("permission"), T1),
            &reporting("done", None, T1)
        ),
        Some(Notice::FinishedTurn)
    );
    assert_eq!(notice_of(&busy, &busy), None);
    assert_eq!(
        notice_of(&working(), &reporting("done", None, T1)),
        Some(Notice::FinishedTurn),
        "the first record after the title said working"
    );
    assert_eq!(
        notice_of(&busy, &view("completed")),
        Some(Notice::StoppedWorking),
        "an ended session has no program status"
    );
}

#[test]
fn a_done_or_an_error_is_said_once_per_record() {
    let done = reporting("done", None, T1);
    assert_eq!(notice_of(&done, &reporting("done", None, T1)), None);
    assert_eq!(
        notice_of(&done, &reporting("done", None, T2)),
        Some(Notice::FinishedTurn)
    );
    assert_eq!(
        notice_of(&reporting("error", None, T1), &reporting("error", None, T1)),
        None
    );
    assert_eq!(
        notice_of(&done, &reporting("idle", None, T1)),
        None,
        "not working before"
    );
    let still_working = Snap {
        activity: Some("working".into()),
        ..reporting("done", None, T1)
    };
    assert_eq!(
        notice_of(&reporting("working", None, T1), &still_working),
        None,
        "still working, whatever the record says"
    );
}

#[test]
fn notice_key_dedupe() {
    let done = reporting("done", None, T1);
    assert_eq!(
        notice_key(Notice::FinishedTurn, "a", &done, T1),
        notice_key(Notice::FinishedTurn, "a", &reporting("done", None, T1), T1),
        "the same record never notifies twice"
    );
    assert_ne!(
        notice_key(Notice::FinishedTurn, "a", &done, T1),
        notice_key(Notice::FinishedTurn, "a", &reporting("done", None, T2), T1),
        "a newer record is a new key"
    );
    let at_50 = Snap {
        watermark: Some("50".into()),
        ..view("running")
    };
    let at_90 = Snap {
        watermark: Some("90".into()),
        ..view("running")
    };
    assert_eq!(
        notice_key(Notice::StoppedWorking, "a", &at_50, T1),
        notice_key(Notice::StoppedWorking, "a", &at_50, T1)
    );
    assert_ne!(
        notice_key(Notice::StoppedWorking, "a", &at_50, T1),
        notice_key(Notice::StoppedWorking, "a", &at_90, T1)
    );
    assert_ne!(
        notice_key(Notice::NeedsYou, "a", &blocked(PERMISSION_PROMPT), T1),
        notice_key(Notice::NeedsYou, "b", &blocked(PERMISSION_PROMPT), T1),
        "scoped by session"
    );
}

/// Item 3: a needs-you with no `attention.since` (evidence that may be
/// missing) must not fall back to a constant key, or the `UNIQUE` column
/// would silence every later needs-you for the session.
#[test]
fn a_missing_since_does_not_give_a_constant_needs_you_key() {
    let no_since = Snap {
        status: "running".into(),
        attention_kind: Some(PERMISSION_PROMPT),
        attention_since: None,
        ..Snap::default()
    };
    assert_ne!(
        notice_key(Notice::NeedsYou, "a", &no_since, T1),
        notice_key(Notice::NeedsYou, "a", &no_since, T2),
        "falls back to the observed instant, not a constant"
    );
}

// ---------------------------------------------------------------------
// Quiet hours and pause
// ---------------------------------------------------------------------

#[test]
fn quiet_hours_across_midnight() {
    let mut prefs = Prefs::defaults();
    prefs.quiet_on = true;
    prefs.quiet_start = "22:00".into();
    prefs.quiet_end = "08:00".into();
    prefs.quiet_tz = "UTC".into();
    assert!(
        in_quiet_hours(&prefs, "2026-11-14T23:30:00Z"),
        "before midnight"
    );
    assert!(
        in_quiet_hours(&prefs, "2026-11-15T03:00:00Z"),
        "after midnight"
    );
    assert!(!in_quiet_hours(&prefs, "2026-11-15T12:00:00Z"), "daytime");
    assert!(
        !in_quiet_hours(&prefs, "2026-11-15T08:00:00Z"),
        "the end is exclusive"
    );
}

#[test]
fn quiet_hours_across_time_zones() {
    let mut prefs = Prefs::defaults();
    prefs.quiet_on = true;
    prefs.quiet_start = "22:00".into();
    prefs.quiet_end = "08:00".into();
    prefs.quiet_tz = "America/New_York".into();
    // 03:00 UTC is 22:00 or 23:00 Eastern depending on DST; either way it's
    // inside the window Eastern clocks see, while the same instant read as
    // UTC time-of-day (03:00) would not be.
    assert!(in_quiet_hours(&prefs, "2026-11-15T03:00:00Z"));
    // 14:00 UTC is morning in New York on any date: outside the window.
    assert!(!in_quiet_hours(&prefs, "2026-11-15T14:00:00Z"));
}

#[test]
fn quiet_hours_off_by_default_and_when_off() {
    let prefs = Prefs::defaults();
    assert!(!prefs.quiet_on);
    assert!(!in_quiet_hours(&prefs, "2026-11-15T23:00:00Z"));
}

#[test]
fn pause() {
    let mut prefs = Prefs::defaults();
    assert!(!is_paused(&prefs, "2026-11-15T00:00:00Z"));
    prefs.paused_until = Some("2026-11-15T01:00:00Z".into());
    assert!(is_paused(&prefs, "2026-11-15T00:30:00Z"));
    assert!(
        !is_paused(&prefs, "2026-11-15T01:30:00Z"),
        "the pause ended"
    );
}

#[test]
fn silent_for_precedence() {
    let mut prefs = Prefs::defaults();
    let at = "2026-11-15T12:00:00Z";
    assert_eq!(
        silent_for(&prefs, &[], NEEDS_YOU, Some("a"), Some("fake"), at),
        None
    );

    prefs.paused_until = Some("2099-01-01T00:00:00Z".into());
    assert_eq!(
        silent_for(&prefs, &[], NEEDS_YOU, Some("a"), Some("fake"), at),
        Some("paused")
    );
    prefs.paused_until = None;

    assert_eq!(
        silent_for(&prefs, &[], CHILD_FINISHED, Some("a"), Some("fake"), at),
        Some("off"),
        "off by default"
    );

    let mute = Mute {
        id: "ntm_1".into(),
        scope: "session".into(),
        target: "a".into(),
        until: None,
        created_at: at.into(),
    };
    assert_eq!(
        silent_for(&prefs, &[mute], FINISHED, Some("a"), Some("fake"), at),
        Some("muted")
    );
    // Q17: a session mute never hides a delete request.
    assert_eq!(
        silent_for(
            &prefs,
            &[Mute {
                id: "ntm_2".into(),
                scope: "session".into(),
                target: "a".into(),
                until: None,
                created_at: at.into(),
            }],
            DELETE_REQUEST,
            Some("a"),
            Some("fake"),
            at,
        ),
        None
    );
}

// ---------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------

fn test_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    init(&conn).unwrap();
    conn
}

fn plain_notice(key: &str) -> NewNotice<'static> {
    NewNotice {
        source: "storm",
        kind: NEEDS_YOU,
        session_id: None,
        host_id: None,
        provider: None,
        session_title: None,
        note_vault_id: None,
        note_id: None,
        title: "t".into(),
        body: None,
        urgency: "normal",
        key: Some(key.to_string()),
    }
}

#[test]
fn retention_by_age_and_by_row_count() {
    let conn = test_conn();
    for i in 0..5 {
        insert_notice(&conn, plain_notice(&format!("k{i}")), None).unwrap();
    }
    conn.execute(
        "UPDATE notifications SET created = '2000-01-01T00:00:00Z' WHERE key IN ('k0', 'k1')",
        [],
    )
    .unwrap();
    let removed = prune(&conn, 30, 1000, &now()).unwrap();
    assert_eq!(removed, 2, "the two old rows age out");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM notifications", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );

    for i in 5..1005 {
        insert_notice(&conn, plain_notice(&format!("k{i}")), None).unwrap();
    }
    let removed = prune(&conn, 30, 1000, &now()).unwrap();
    assert!(removed > 0, "over the row cap");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM notifications", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1000
    );
}

/// A trimmed shape of the `sessions` table as it exists on `staging`
/// today (v0.18.0), before this slice. Opening it through the current
/// `Store::open` must add the new tables without disturbing the row —
/// everything here is `CREATE TABLE IF NOT EXISTS`, never an `ALTER`.
const OLD_SESSIONS_TABLE: &str = "CREATE TABLE sessions (
    id                TEXT PRIMARY KEY,
    owner_user_id     TEXT NOT NULL,
    host_id           TEXT NOT NULL,
    workspace         TEXT NOT NULL,
    provider          TEXT NOT NULL,
    provider_kind     TEXT NOT NULL,
    interaction       TEXT NOT NULL,
    provider_fallback TEXT,
    status            TEXT NOT NULL,
    end_reason        TEXT,
    signal            INTEGER,
    exit_code         INTEGER,
    created_at        TEXT NOT NULL,
    started_at        TEXT,
    ended_at          TEXT,
    last_activity     TEXT,
    egress            TEXT NOT NULL DEFAULT 'host',
    cols              INTEGER NOT NULL,
    rows              INTEGER NOT NULL,
    title             TEXT,
    harness_session   TEXT,
    resumed_from      TEXT
)";

#[test]
fn migrates_from_a_v0_18_0_db_without_losing_sessions() {
    let dir = tempdir::TempDir::new("storm-161-migrate").unwrap();
    let path = dir.path().join("agent.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(OLD_SESSIONS_TABLE).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, owner_user_id, host_id, workspace, provider, provider_kind,
                 interaction, status, created_at, cols, rows)
             VALUES ('ags_old', 'usr_OWNER', 'hst_A', 'storm', 'fake', 'cli', 'terminal',
                 'completed', '2026-10-01T00:00:00Z', 80, 24)",
            [],
        )
        .unwrap();
    }
    let store = crate::agent::store::Store::open(&path).unwrap();
    assert_eq!(
        store.get("ags_old").unwrap().map(|r| r.status),
        Some("completed".to_string()),
        "the pre-existing row survives"
    );
    let tables: Vec<String> = {
        let mut stmt = store
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    for expected in [
        "notifications",
        "notification_prefs",
        "notification_mutes",
        "notification_counter",
    ] {
        assert!(
            tables.iter().any(|t| t == expected),
            "missing {expected}: {tables:?}"
        );
    }
}

#[test]
fn a_key_never_notifies_twice() {
    let conn = test_conn();
    let first = insert_notice(&conn, plain_notice("dup"), None).unwrap();
    assert!(first.is_some());
    let second = insert_notice(&conn, plain_notice("dup"), None).unwrap();
    assert!(
        second.is_none(),
        "the same key is a dedupe, not a second row"
    );
}

/// Item 1: the counter is one persisted sequence, never derived from
/// `MAX(seq)`/`MAX(updated_seq)` over the rows themselves — so a read
/// bump, or a clear, can never make a later insert collide with (and so
/// never be streamed past) an earlier cursor value.
#[test]
fn the_counter_never_goes_backwards_across_a_read_or_a_clear() {
    let conn = test_conn();
    let a = insert_notice(&conn, plain_notice("a"), None)
        .unwrap()
        .unwrap();
    let b = insert_notice(&conn, plain_notice("b"), None)
        .unwrap()
        .unwrap();
    assert!(b.updated_seq > a.updated_seq);

    // A read bumps `a`'s updated_seq past `b`'s.
    mark_read(&conn, std::slice::from_ref(&a.id), None, &now()).unwrap();
    let a_after_read = conn
        .query_row(
            "SELECT updated_seq FROM notifications WHERE id = ?1",
            params![a.id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    assert!(a_after_read > b.updated_seq, "the read moved past b");

    // A third insert must still land strictly after that read bump.
    let c = insert_notice(&conn, plain_notice("c"), None)
        .unwrap()
        .unwrap();
    assert!(
        c.updated_seq > a_after_read,
        "an insert after a read is never skipped by the stream cursor"
    );

    // Clearing the inbox must not reset the counter backwards either.
    clear_inbox(&conn).unwrap();
    let d = insert_notice(&conn, plain_notice("d"), None)
        .unwrap()
        .unwrap();
    assert!(d.seq > c.seq, "the counter survives a clear");
}

// ---------------------------------------------------------------------
// New events, and the Notifier end to end
// ---------------------------------------------------------------------

const HOST: &str = "hst_A";

/// `usage_limit` has no production gate yet (no `GatedSignal` names it for
/// any provider), so a test that wants to see it through must install one
/// itself, the same seam 144b's own tests use.
const USAGE_LIMIT_GATE: &[crate::agent::attention::GatedSignal] =
    &[crate::agent::attention::GatedSignal {
        provider: "fake",
        signal: USAGE_LIMIT,
        source: crate::agent::events::PROVIDER_HOOK,
    }];

fn manager() -> (
    AgentManager,
    tempdir::TempDir,
    tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope>,
) {
    let dir = tempdir::TempDir::new("storm-161-notify").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    // Kept alive for the test's duration: dropping it closes the link, same
    // as a real disconnect, which would fail every launch on it.
    let (_gen, link) = m.connect_host(HOST);
    m.hello(
        HOST,
        serde_json::from_value(json!({
            "providers": [{"id": "fake", "kind": "fake", "interactions": ["terminal"], "available": true}],
            "workspaces": ["storm"],
            "max_sessions": 8,
        }))
        .unwrap(),
    )
    .unwrap();
    (m, dir, link)
}

fn launch(m: &AgentManager) -> String {
    m.launch(
        "usr_OWNER",
        serde_json::from_value(json!({
            "host_id": HOST, "workspace": "storm", "provider": "fake",
            "terminal": {"cols": 80, "rows": 24},
        }))
        .unwrap(),
        Default::default(),
        Vec::new(),
    )
    .unwrap()
    .0
    .id
}

fn report(m: &AgentManager, id: &str, status: serde_json::Value) {
    m.host_status(HOST, id, &serde_json::from_value(status).unwrap())
        .unwrap();
}

fn kinds(m: &AgentManager) -> Vec<String> {
    m.list_notifications(0, 100)
        .unwrap()
        .into_iter()
        .map(|n| n.kind)
        .collect()
}

#[test]
fn nothing_on_the_first_tick_after_a_restart() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    // Backdated so the session reads as one that existed before this
    // Notifier started (a real restart), not one launched fresh by this
    // test — which item 4 says should notify on its first tick instead.
    m.store_for_tests()
        .conn
        .execute(
            "UPDATE sessions SET created_at = '2000-01-01T00:00:00Z' WHERE id = ?1",
            params![id],
        )
        .unwrap();
    // Program-status attention (decision 155) is ungated, unlike the raw
    // hook path, so this is a reliable way to put a session in "needs you"
    // for any provider in a test.
    report(
        &m,
        &id,
        json!({"status": "running", "program_status": {"state": "blocked", "kind": "permission", "app": "fake"}}),
    );
    m.notifier_tick().unwrap();
    assert!(kinds(&m).is_empty(), "nothing on the first observation");

    // A later, real transition still fires.
    report(
        &m,
        &id,
        json!({"status": "running", "program_status": {"state": "idle", "app": "fake"}}),
    );
    m.notifier_tick().unwrap();
    report(
        &m,
        &id,
        json!({"status": "running", "program_status": {"state": "blocked", "kind": "permission", "app": "fake"}}),
    );
    m.notifier_tick().unwrap();
    assert!(kinds(&m).contains(&NEEDS_YOU.to_string()));
}

#[test]
fn a_usage_limit_transition_notifies() {
    let (m, _d, _link) = manager();
    m.gate_signals_for_tests(USAGE_LIMIT_GATE);
    let id = launch(&m);
    report(&m, &id, json!({"status": "running"}));
    m.notifier_tick().unwrap();
    report(
        &m,
        &id,
        json!({"status": "running", "attention": "usage_limit"}),
    );
    m.notifier_tick().unwrap();
    assert!(kinds(&m).contains(&USAGE_LIMIT.to_string()));
}

#[test]
fn a_delete_request_notifies_at_high_urgency() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    let r = m.get(&id).unwrap();
    let request = super::super::deletes::DeleteRequest {
        id: "adr_1".into(),
        session_id: id.clone(),
        vault_id: "vlt_1".into(),
        kind: "note".into(),
        note_id: Some("not_1".into()),
        path: "Ideas.md".into(),
        title: Some("Ideas".into()),
        reason: "stale".into(),
        fingerprint: "1:Ideas.md".into(),
        notes: 1,
        size: 10,
        modified: None,
        paths: vec!["Ideas.md".into()],
        created_at: now(),
        expires_at: now(),
        state: "pending".into(),
        decided_at: None,
        decided_by: None,
        trash_id: None,
    };
    {
        let store = m.store_for_tests();
        notify_delete_request(&store.conn, &r, &request).unwrap();
    }
    let got = m.list_notifications(0, 10).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, DELETE_REQUEST);
    assert_eq!(got[0].urgency, "high");
    assert_eq!(got[0].key, Some(format!("ses:{id}:del:adr_1")));
    assert_eq!(got[0].body, Some("stale".to_string()));
    assert_eq!(got[0].provider, Some("fake".to_string()));
    assert!(got[0].session_title.is_some());
}

/// Item 9: a delete request's `reason` is agent text — never shown raw.
#[test]
fn a_delete_requests_reason_is_sanitised() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    let r = m.get(&id).unwrap();
    let dirty = "line one\nline two\u{200B}\u{202E}evil";
    let request = super::super::deletes::DeleteRequest {
        id: "adr_2".into(),
        session_id: id.clone(),
        vault_id: "vlt_1".into(),
        kind: "note".into(),
        note_id: Some("not_1".into()),
        path: "Ideas.md".into(),
        title: None,
        reason: dirty.into(),
        fingerprint: "1:Ideas.md".into(),
        notes: 1,
        size: 10,
        modified: None,
        paths: vec!["Ideas.md".into()],
        created_at: now(),
        expires_at: now(),
        state: "pending".into(),
        decided_at: None,
        decided_by: None,
        trash_id: None,
    };
    {
        let store = m.store_for_tests();
        notify_delete_request(&store.conn, &r, &request).unwrap();
    }
    let got = m.list_notifications(0, 10).unwrap();
    let body = got[0].body.as_deref().unwrap();
    // The zero-width space and the bidi override are removed outright, not
    // turned into a visible gap — they carry no word boundary of their own.
    assert_eq!(body, "line one line twoevil");
}

/// Q7: off by default — it is still written to the inbox, marked silent.
#[test]
fn a_child_event_is_silent_by_default() {
    let (m, _d, _link) = manager();
    let parent = launch(&m);
    let child = launch(&m);
    m.insert_child_event_for_tests(&parent, &child, "reported");
    m.notifier_tick().unwrap();
    let got = m.list_notifications(0, 10).unwrap();
    let child_notice = got
        .iter()
        .find(|n| n.kind == CHILD_FINISHED)
        .expect("a child notice");
    assert!(child_notice.silent, "off by default, so silent, not absent");
}

/// Q7: only once offline at least the threshold, and only with running
/// sessions when it went down.
#[test]
fn a_host_offline_past_the_threshold_with_running_sessions_notifies() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    report(&m, &id, json!({"status": "running"}));
    m.notifier_tick().unwrap(); // seed memory

    m.set_host_offline_for_tests(HOST, 130);
    m.notifier_tick().unwrap();
    let got = m.list_notifications(0, 10).unwrap();
    assert!(got.iter().any(|n| n.kind == HOST_OFFLINE));
}

#[test]
fn a_host_offline_under_the_threshold_does_not_notify_yet() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    report(&m, &id, json!({"status": "running"}));
    m.notifier_tick().unwrap();

    m.set_host_offline_for_tests(HOST, 10);
    m.notifier_tick().unwrap();
    assert!(!kinds(&m).contains(&HOST_OFFLINE.to_string()));
}

/// Item 4: a session launched after the Notifier started has no earlier
/// state to have missed, so its first observed state is still notified —
/// "nothing on a snapshot/restart" applies only to a pre-existing session.
#[test]
fn a_session_launched_after_the_notifier_started_notifies_on_the_first_tick() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    report(
        &m,
        &id,
        json!({"status": "running", "program_status": {"state": "blocked", "kind": "permission", "app": "fake"}}),
    );
    m.notifier_tick().unwrap();
    assert!(
        kinds(&m).contains(&NEEDS_YOU.to_string()),
        "a brand-new session's first needs-you is not a restart"
    );
}

// ---------------------------------------------------------------------
// Validation (item 5) and the stream side-effects of pause/mute/clear
// (items 6, 7)
// ---------------------------------------------------------------------

#[test]
fn prefs_validation_rejects_out_of_range_values() {
    let (m, _d, _link) = manager();
    let v = m.notification_prefs().unwrap().version;
    let bad = [
        PrefsPatch {
            retention_days: Some(0),
            ..Default::default()
        },
        PrefsPatch {
            retention_rows: Some(10),
            ..Default::default()
        },
        PrefsPatch {
            per_session_limit: Some(0),
            ..Default::default()
        },
        PrefsPatch {
            quiet_start: Some("25:00".into()),
            ..Default::default()
        },
        PrefsPatch {
            quiet_tz: Some("Nowhere/Fake".into()),
            ..Default::default()
        },
        PrefsPatch {
            events: Some(HashMap::from([(
                "not_a_kind".into(),
                EventPref {
                    on: true,
                    threshold_s: None,
                },
            )])),
            ..Default::default()
        },
        PrefsPatch {
            events: Some(HashMap::from([(
                FINISHED.into(),
                EventPref {
                    on: true,
                    threshold_s: Some(999_999),
                },
            )])),
            ..Default::default()
        },
    ];
    for patch in bad {
        assert!(
            m.update_notification_prefs(v, patch).is_err(),
            "should have been rejected"
        );
    }
    // A valid change still goes through with the same version.
    assert!(
        m.update_notification_prefs(
            v,
            PrefsPatch {
                summary_on: Some(false),
                ..Default::default()
            },
        )
        .is_ok()
    );
}

#[test]
fn mute_validation_requires_a_known_target_and_a_future_until() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    assert!(
        m.mute_notifications("session", "ags_no_such_session", None, None)
            .is_err(),
        "no such session"
    );
    assert!(
        m.mute_notifications("provider", "no-such-provider", None, None)
            .is_err(),
        "no such provider"
    );
    assert!(
        m.mute_notifications("session", &id, Some("2000-01-01T00:00:00Z"), None)
            .is_err(),
        "until must be in the future"
    );
    let mute = m.mute_notifications("session", &id, None, None).unwrap();
    assert_eq!(mute.target, id);
    // A provider with a live session is a known target.
    m.mute_notifications("provider", "fake", None, None)
        .unwrap();
}

#[test]
fn pause_bumps_prefs_version_and_validates_until() {
    let (m, _d, _link) = manager();
    let before = m.notification_prefs().unwrap().version;
    assert!(
        m.pause_notifications(Some("2000-01-01T00:00:00Z")).is_err(),
        "must be in the future"
    );
    let after = m.pause_notifications(None).unwrap();
    assert!(after.version > before, "prefs_changed must be able to fire");
    assert_eq!(after.paused_until.as_deref(), Some(PAUSE_INDEFINITELY));
}

#[test]
fn clearing_and_muting_bump_their_own_versions() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    let cleared_before = m.notifications_cleared_version().unwrap();
    m.clear_notifications().unwrap();
    assert!(m.notifications_cleared_version().unwrap() > cleared_before);

    let mutes_before = m.notifications_mutes_version().unwrap();
    let mute = m.mute_notifications("session", &id, None, None).unwrap();
    assert!(m.notifications_mutes_version().unwrap() > mutes_before);
    m.unmute_notifications(&mute.id).unwrap();
    assert!(m.notifications_mutes_version().unwrap() > mutes_before + 1);
}

#[test]
fn a_silent_row_records_why() {
    let conn = test_conn();
    let n = insert_notice(&conn, plain_notice("sr"), Some("off"))
        .unwrap()
        .unwrap();
    assert!(n.silent);
    assert_eq!(n.silent_reason, Some("off".to_string()));
}

/// Item 8: the summary counts only what quiet hours or the pause itself
/// held back — never an off-by-default kind, and never a muted one.
#[test]
fn the_summary_counts_only_quiet_or_paused_rows() {
    let conn = test_conn();
    let at = now();
    insert_notice(&conn, plain_notice("q1"), Some("quiet")).unwrap();
    insert_notice(&conn, plain_notice("q2"), Some("paused")).unwrap();
    insert_notice(&conn, plain_notice("q3"), Some("off")).unwrap();
    insert_notice(&conn, plain_notice("q4"), Some("muted")).unwrap();
    insert_notice(&conn, plain_notice("q5"), None).unwrap();
    let counts = silenced_counts(&conn, &at).unwrap();
    let total: i64 = counts.iter().map(|(_, n)| n).sum();
    assert_eq!(total, 2, "only the quiet and paused rows: {counts:?}");
}

/// Item 10: `failed` fires from any running state, not only one the
/// literal `noticeOf` port would already call `StoppedWorking` for.
#[test]
fn failed_notifies_even_when_not_preceded_by_working() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    report(
        &m,
        &id,
        json!({"status": "running", "program_status": {"state": "blocked", "kind": "question", "app": "fake"}}),
    );
    m.notifier_tick().unwrap(); // seed memory on a non-working state
    report(&m, &id, json!({"status": "failed"}));
    m.notifier_tick().unwrap();
    assert!(
        kinds(&m).contains(&ERROR.to_string()),
        "failed from a blocked (not working) state still notifies: {:?}",
        kinds(&m)
    );
}

// ---------------------------------------------------------------------
// 161-2: the `notify` tool's own mechanics (limits, duplicate/`key`
// replace, `delay_s`/`cancel_notify`, the roll-up, sanitising).
// ---------------------------------------------------------------------

fn enable_notify(m: &AgentManager, id: &str) {
    m.store_for_tests()
        .set_notify(id, true, None, &now())
        .unwrap();
}

fn notify_req(id: &str, title: &str, key: Option<&str>) -> AgentNotify {
    AgentNotify {
        session_id: id.to_string(),
        provider: "fake".into(),
        session_title: "tidier".into(),
        title: title.into(),
        body: None,
        urgency: "normal",
        key: key.map(str::to_string),
        note_vault_id: None,
        note_id: None,
        due: None,
    }
}

#[test]
fn notify_offered_is_off_by_default_and_on_once_set() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    assert!(!m.notify_offered(&id), "off by default (Q3)");
    enable_notify(&m, &id);
    assert!(m.notify_offered(&id));
}

/// `silent_for`'s precedence governs an agent message exactly as it does
/// an automatic notice (pause and quiet hours both hold it).
#[test]
fn agent_messages_go_through_the_same_precedence() {
    let mut prefs = Prefs::defaults();
    prefs.quiet_on = true;
    prefs.quiet_start = "00:00".into();
    prefs.quiet_end = "23:59".into();
    assert_eq!(
        silent_for(
            &prefs,
            &[],
            AGENT_MESSAGE,
            Some("ags_X"),
            Some("fake"),
            "2026-11-15T12:00:00Z"
        ),
        Some("quiet")
    );
    prefs.quiet_on = false;
    prefs.paused_until = Some("2026-11-16T00:00:00Z".into());
    assert_eq!(
        silent_for(
            &prefs,
            &[],
            AGENT_MESSAGE,
            Some("ags_X"),
            Some("fake"),
            "2026-11-15T12:00:00Z"
        ),
        Some("paused")
    );
}

#[test]
fn held_under_a_session_mute_still_reaches_the_inbox() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    m.mute_notifications("session", &id, None, None).unwrap();
    let out = m
        .agent_notify(notify_req(&id, "hello", None))
        .unwrap()
        .unwrap();
    assert_eq!(out.state, "held");
    assert_eq!(out.held_reason, Some("muted"));
    assert!(
        kinds(&m).contains(&AGENT_MESSAGE.to_string()),
        "still in the inbox"
    );
}

#[test]
fn held_while_paused_still_reaches_the_inbox() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    m.pause_notifications(None).unwrap();
    let out = m
        .agent_notify(notify_req(&id, "hello", None))
        .unwrap()
        .unwrap();
    assert_eq!(out.state, "held");
    assert_eq!(out.held_reason, Some("paused"));
    assert!(kinds(&m).contains(&AGENT_MESSAGE.to_string()));
}

#[test]
fn an_identical_title_and_body_within_the_window_is_the_same_notice() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    let first = m
        .agent_notify(notify_req(&id, "progress", None))
        .unwrap()
        .unwrap();
    let second = m
        .agent_notify(notify_req(&id, "progress", None))
        .unwrap()
        .unwrap();
    assert_eq!(first.id, second.id, "the same notice, not a new one");
    assert_eq!(
        m.list_notifications(0, 100).unwrap().len(),
        1,
        "no second row, and no token consumed"
    );
}

#[test]
fn a_key_replaces_this_sessions_earlier_notice_under_it() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    let first = m
        .agent_notify(notify_req(&id, "step 1", Some("progress")))
        .unwrap()
        .unwrap();
    assert_eq!(first.state, "sent");
    let second = m
        .agent_notify(notify_req(&id, "step 2", Some("progress")))
        .unwrap()
        .unwrap();
    assert_eq!(second.state, "replaced");
    assert_ne!(first.id, second.id);
    let rows = m.list_notifications(0, 100).unwrap();
    let old = rows.iter().find(|n| n.id == first.id).unwrap();
    assert!(old.cancelled, "the earlier notice is cancelled");
    let new = rows.iter().find(|n| n.id == second.id).unwrap();
    assert!(!new.cancelled);
    assert_eq!(new.title, "step 2");
}

#[test]
fn the_burst_then_the_hourly_cap_rate_limit_a_session() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    // Burst 3: distinct keys, so none is treated as a duplicate.
    for i in 0..3 {
        let out = m
            .agent_notify(notify_req(&id, &format!("msg {i}"), Some(&format!("k{i}"))))
            .unwrap();
        assert!(out.is_ok(), "{i}: {out:?}");
    }
    let fourth = m
        .agent_notify(notify_req(&id, "msg 4", Some("k4")))
        .unwrap();
    match fourth {
        Err(NotifyRefusal::RateLimited { retry_after_s }) => assert!(retry_after_s > 0),
        other => panic!("the 4th past the burst should be rate_limited: {other:?}"),
    }
}

#[test]
fn a_rate_limited_session_gets_one_rollup_bumped_in_place() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    for i in 0..3 {
        m.agent_notify(notify_req(&id, &format!("msg {i}"), Some(&format!("k{i}"))))
            .unwrap()
            .unwrap();
    }
    for i in 4..6 {
        let out = m
            .agent_notify(notify_req(
                &id,
                &format!("over {i}"),
                Some(&format!("o{i}")),
            ))
            .unwrap();
        assert!(matches!(out, Err(NotifyRefusal::RateLimited { .. })));
    }
    let rollups: Vec<_> = m
        .list_notifications(0, 100)
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == AGENT_ROLLUP)
        .collect();
    assert_eq!(
        rollups.len(),
        1,
        "one roll-up notice, not one per limited call"
    );
    assert!(
        rollups[0].title.contains("2 more notification"),
        "{}",
        rollups[0].title
    );
}

#[test]
fn too_many_pending_is_refused_past_the_cap() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    // Seeded directly: 10 live calls would hit the rate limit long before
    // the pending cap, which is the thing this test isolates.
    {
        let store = m.store_for_tests();
        for i in 0..10 {
            store
                .conn
                .execute(
                    "INSERT INTO notify_pending
                         (id, session_id, provider, session_title, title, urgency, due,
                          created_at)
                     VALUES (?1, ?2, 'fake', 'tidier', 'seeded', 'normal', ?3, ?3)",
                    params![format!("ntf_seed{i}"), id, now()],
                )
                .unwrap();
        }
    }
    let mut req = notify_req(&id, "one too many", Some("p10"));
    req.due = Some(crate::index::rfc3339_minus_secs(&now(), -3600));
    assert!(matches!(
        m.agent_notify(req).unwrap(),
        Err(NotifyRefusal::TooManyPending)
    ));
}

#[test]
fn a_delayed_notify_is_queued_then_fired_under_the_prefs_in_force_then() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    let due = crate::index::rfc3339_minus_secs(&now(), -60);
    let mut req = notify_req(&id, "later", Some("later-key"));
    req.due = Some(due.clone());
    let out = m.agent_notify(req).unwrap().unwrap();
    assert_eq!(out.state, "queued");
    assert!(kinds(&m).is_empty(), "not in the inbox until fired");

    // Pause takes effect between queueing and firing: the fired notice is
    // held under it, not under whatever was in force when it was asked for.
    m.pause_notifications(None).unwrap();
    let past_due = crate::index::rfc3339_minus_secs(&due, -1);
    assert!(m.fire_due_notifies(&past_due).unwrap());
    let rows = m.list_notifications(0, 100).unwrap();
    let fired = rows.iter().find(|n| n.kind == AGENT_MESSAGE).unwrap();
    assert!(fired.silent, "held under the pause in force at firing time");
    assert_eq!(fired.silent_reason, Some("paused".to_string()));
}

#[test]
fn cancel_notify_removes_a_still_pending_delayed_notice() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    let mut req = notify_req(&id, "later", Some("cancel-me"));
    req.due = Some(crate::index::rfc3339_minus_secs(&now(), -3600));
    m.agent_notify(req).unwrap().unwrap();
    assert!(m.cancel_agent_notify(&id, "cancel-me").unwrap());
    let far_future = crate::index::rfc3339_minus_secs(&now(), -7200);
    assert!(
        !m.fire_due_notifies(&far_future).unwrap(),
        "nothing left to fire"
    );
    assert!(kinds(&m).is_empty());
}

#[test]
fn cancel_notify_cancels_an_already_live_notice_under_its_key() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    m.agent_notify(notify_req(&id, "hello", Some("k")))
        .unwrap()
        .unwrap();
    assert!(m.cancel_agent_notify(&id, "k").unwrap());
    let rows = m.list_notifications(0, 100).unwrap();
    assert!(rows.iter().all(|n| n.cancelled));
}

// ---------------------------------------------------------------------
// 161-2 F2: a delayed notice whose session is gone by the time it is due,
// and the cleanup that keeps that the rare case rather than the usual one.
// ---------------------------------------------------------------------

fn pending_count(m: &AgentManager, id: &str) -> i64 {
    m.store_for_tests()
        .conn
        .query_row(
            "SELECT COUNT(*) FROM notify_pending WHERE session_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

fn notify_rows(m: &AgentManager, id: &str) -> (i64, i64, i64) {
    let one = |table: &str| -> i64 {
        m.store_for_tests()
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?1"),
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    (
        one("session_notify"),
        one("session_notify_bucket"),
        one("session_notify_rollup"),
    )
}

/// Ending a session drops its queued delayed notices at once, rather than
/// leaving them for the Notifier to find due and discard one by one.
#[test]
fn ending_a_session_drops_its_queued_notices() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    let mut req = notify_req(&id, "later", Some("later-key"));
    req.due = Some(crate::index::rfc3339_minus_secs(&now(), -3600));
    m.agent_notify(req).unwrap().unwrap();
    assert_eq!(pending_count(&m, &id), 1);
    report(&m, &id, json!({"status": "completed"}));
    assert_eq!(pending_count(&m, &id), 0, "dropped the moment it ended");
}

/// `fire_due_notifies` is also a defensive check at firing time: a notice
/// due for a session that lost the capability between `notify` and now is
/// dropped, never fired — the end-time cleanup above does not cover a
/// flag turned off on a still-live session.
#[test]
fn a_due_notice_is_dropped_not_fired_once_the_capability_is_off() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    let mut req = notify_req(&id, "later", Some("later-key"));
    req.due = Some(crate::index::rfc3339_minus_secs(&now(), -3600));
    m.agent_notify(req).unwrap().unwrap();
    m.store_for_tests()
        .set_notify(&id, false, None, &now())
        .unwrap();
    let past_due = crate::index::rfc3339_minus_secs(&now(), -7200);
    assert!(
        m.fire_due_notifies(&past_due).unwrap(),
        "the row was consumed"
    );
    assert!(kinds(&m).is_empty(), "never fired");
    assert_eq!(pending_count(&m, &id), 0, "and not left behind either");
}

/// Removing a session (decisions 147-150) takes its "Notifies you" flag
/// and its rate-limit/roll-up state with it; its inbox rows are untouched
/// (kept for provenance, like a mute's).
#[test]
fn removing_a_session_drops_its_notify_state() {
    let (m, _d, _link) = manager();
    let id = launch(&m);
    enable_notify(&m, &id);
    m.agent_notify(notify_req(&id, "hello", Some("k")))
        .unwrap()
        .unwrap();
    assert_eq!(notify_rows(&m, &id), (1, 1, 0));
    report(&m, &id, json!({"status": "completed"}));
    assert_eq!(notify_rows(&m, &id), (1, 1, 0), "kept while only ended");
    assert!(m.dismiss(&id).unwrap().is_none());
    assert_eq!(notify_rows(&m, &id), (0, 0, 0));
    assert_eq!(
        kinds(&m),
        vec![AGENT_MESSAGE.to_string()],
        "the inbox row stays"
    );
}

#[test]
fn sanitiser_vectors() {
    assert_eq!(
        checked_title("  hello   world  "),
        Some("hello world".into())
    );
    assert_eq!(checked_title(""), None, "empty after sanitising");
    assert_eq!(checked_title("   "), None);
    assert_eq!(
        checked_title(&"a".repeat(81)),
        None,
        "over NOTIFY_TITLE_MAX"
    );
    assert_eq!(checked_title(&"a".repeat(80)).unwrap().chars().count(), 80);
    // Control characters (except whitespace) are dropped, not collapsed to
    // a space.
    assert_eq!(checked_title("a\u{0007}b"), Some("ab".into()));
    // Bidi overrides and zero-width characters are dropped outright.
    assert_eq!(
        checked_title("a\u{202E}b\u{200B}c\u{FEFF}d"),
        Some("abcd".into())
    );
    // Newlines collapse to a single space.
    assert_eq!(checked_title("a\nb\r\nc"), Some("a b c".into()));
    assert_eq!(
        sanitised_body(&"x".repeat(600), NOTIFY_BODY_MAX)
            .unwrap()
            .chars()
            .count(),
        NOTIFY_BODY_MAX,
        "truncated, never refused"
    );
}

#[test]
fn notify_key_shape() {
    assert!(valid_notify_key("step.1_2:3-4"));
    assert!(!valid_notify_key(""));
    assert!(!valid_notify_key(&"a".repeat(65)));
    assert!(!valid_notify_key("has space"));
    assert!(!valid_notify_key("has/slash"));
}
