//! Decision 161, slice 161-9: the ntfy sink's own tests at the
//! `AgentManager`/data-layer level — prefs round-trip and validation, the
//! token never surfacing from a GET, quiet-hours/pause suppression, the
//! `test` kind never entering this queue, and the grace/skip rules the
//! forwarder (`crate::ntfy_forwarder`) leans on. The HTTP publish call
//! itself (headers, retries, body) is covered in `agent::ntfy`'s own
//! `#[cfg(test)]` module against a local mock server.

use serde_json::json;

use super::*;
use crate::agent::AgentManager;

const HOST: &str = "hst_ntfy";

fn manager() -> (
    AgentManager,
    tempdir::TempDir,
    tokio::sync::mpsc::UnboundedReceiver<crate::agent::Envelope>,
) {
    let dir = tempdir::TempDir::new("storm-161-9-ntfy").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
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

fn set_topic(m: &AgentManager, url: &str) -> Prefs {
    let v = m.notification_prefs().unwrap().version;
    m.update_notification_prefs(
        v,
        PrefsPatch {
            ntfy_url: Some(url.into()),
            ..Default::default()
        },
    )
    .unwrap()
}

// ---------------------------------------------------------------------
// Prefs: round trip, the token never echoed, validation
// ---------------------------------------------------------------------

#[test]
fn ntfy_is_off_by_default() {
    let (m, _d, _link) = manager();
    let p = m.notification_prefs().unwrap();
    assert_eq!(p.ntfy_url, None);
    assert!(!p.ntfy_token_set);
    assert_eq!(p.ntfy_preview, "session");
}

#[test]
fn setting_a_token_never_returns_it_again() {
    let (m, _d, _link) = manager();
    let after_url = set_topic(&m, "https://ntfy.sh/storm-abc123");
    let after_token = m
        .update_notification_prefs(
            after_url.version,
            PrefsPatch {
                ntfy_token: Some("tk_super_secret_token".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(after_token.ntfy_token_set);
    // Neither the struct nor its JSON carries the raw token anywhere.
    let json = serde_json::to_string(&after_token).unwrap();
    assert!(!json.contains("tk_super_secret_token"));
    assert!(!json.contains("ntfy_token\""));
    // A fresh GET agrees.
    let fetched = m.notification_prefs().unwrap();
    assert!(fetched.ntfy_token_set);
}

#[test]
fn an_empty_token_clears_it() {
    let (m, _d, _link) = manager();
    let after_url = set_topic(&m, "https://ntfy.sh/storm-abc123");
    let after_set = m
        .update_notification_prefs(
            after_url.version,
            PrefsPatch {
                ntfy_token: Some("tk_x".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(after_set.ntfy_token_set);
    let after_clear = m
        .update_notification_prefs(
            after_set.version,
            PrefsPatch {
                ntfy_token: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!after_clear.ntfy_token_set);
}

#[test]
fn clearing_the_url_also_clears_any_stored_token() {
    let (m, _d, _link) = manager();
    let after_url = set_topic(&m, "https://ntfy.sh/storm-abc123");
    let after_token = m
        .update_notification_prefs(
            after_url.version,
            PrefsPatch {
                ntfy_token: Some("tk_x".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(after_token.ntfy_token_set);
    let cleared = m
        .update_notification_prefs(
            after_token.version,
            PrefsPatch {
                ntfy_url: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(cleared.ntfy_url, None);
    assert!(
        !cleared.ntfy_token_set,
        "an orphaned token must not survive"
    );
}

#[test]
fn a_bad_url_or_preview_is_rejected_and_nothing_is_saved() {
    let (m, _d, _link) = manager();
    let v = m.notification_prefs().unwrap().version;
    let bad = [
        PrefsPatch {
            ntfy_url: Some("http://ntfy.example.com/storm-x".into()), // not https, not LAN
            ..Default::default()
        },
        PrefsPatch {
            ntfy_url: Some("https://ntfy.sh/".into()), // no topic
            ..Default::default()
        },
        PrefsPatch {
            ntfy_url: Some("https://ntfy.sh/a/b".into()), // nested path
            ..Default::default()
        },
        PrefsPatch {
            ntfy_preview: Some("everything".into()),
            ..Default::default()
        },
    ];
    for patch in bad {
        assert!(
            m.update_notification_prefs(v, patch).is_err(),
            "should have been rejected"
        );
    }
    assert_eq!(
        m.notification_prefs().unwrap().version,
        v,
        "a rejected patch changes nothing"
    );
}

#[test]
fn https_and_lan_http_both_validate() {
    let (m, _d, _link) = manager();
    assert!(
        m.update_notification_prefs(
            m.notification_prefs().unwrap().version,
            PrefsPatch {
                ntfy_url: Some("https://ntfy.sh/storm-ok".into()),
                ..Default::default()
            },
        )
        .is_ok()
    );
    assert!(
        m.update_notification_prefs(
            m.notification_prefs().unwrap().version,
            PrefsPatch {
                ntfy_url: Some("http://192.168.1.13:8787/storm-ok".into()),
                ..Default::default()
            },
        )
        .is_ok()
    );
}

// ---------------------------------------------------------------------
// insert_notice's ntfy_state: pending vs skipped
// ---------------------------------------------------------------------

fn pending_ntfy_ids(m: &AgentManager) -> Vec<String> {
    let far_future = "2999-01-01T00:00:00Z";
    m.due_ntfy_notices(far_future, 100)
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect()
}

#[test]
fn a_silenced_notice_never_joins_the_ntfy_queue() {
    let (m, _d, _link) = manager();
    set_topic(&m, "https://ntfy.sh/storm-ok");
    // Pausing silences every automatic notice (precedence: pause first).
    m.pause_notifications(None).unwrap();
    let id = launch(&m);
    m.host_status(
        HOST,
        &id,
        &serde_json::from_value(json!({
            "status": "running",
            "program_status": {"state": "blocked", "kind": "permission", "app": "fake"},
        }))
        .unwrap(),
    )
    .unwrap();
    m.notifier_tick().unwrap();
    let notices = m.list_notifications(0, 100).unwrap();
    assert!(
        notices.iter().any(|n| n.kind == NEEDS_YOU && n.silent),
        "the notice is still written to the inbox, just silent"
    );
    assert!(
        pending_ntfy_ids(&m).is_empty(),
        "a silenced notice is skipped, not queued for ntfy"
    );
}

#[test]
fn the_test_kind_never_enters_the_general_ntfy_queue() {
    let (m, _d, _link) = manager();
    set_topic(&m, "https://ntfy.sh/storm-ok");
    m.send_test_notification(None).unwrap();
    assert!(
        pending_ntfy_ids(&m).is_empty(),
        "test has its own dedicated endpoint, not this queue"
    );
}

#[test]
fn a_non_silent_notice_is_pending_then_becomes_due_after_its_grace() {
    let (m, _d, _link) = manager();
    set_topic(&m, "https://ntfy.sh/storm-ok");
    let id = launch(&m);
    m.host_status(
        HOST,
        &id,
        &serde_json::from_value(json!({
            "status": "running",
            "program_status": {"state": "blocked", "kind": "permission", "app": "fake"},
        }))
        .unwrap(),
    )
    .unwrap();
    m.notifier_tick().unwrap();
    // Not due yet: "now" (far in the past relative to its due_at) finds
    // nothing.
    assert!(
        m.due_ntfy_notices("2000-01-01T00:00:00Z", 100)
            .unwrap()
            .is_empty(),
        "still inside its own grace"
    );
    // The test build's grace is milliseconds (`ntfy::GRACE`); give it a
    // moment, then it is due.
    std::thread::sleep(crate::agent::ntfy::GRACE * 2);
    let due = pending_ntfy_ids(&m);
    assert_eq!(due.len(), 1);
}

#[test]
fn marking_an_outcome_is_terminal_and_is_not_offered_again() {
    let (m, _d, _link) = manager();
    set_topic(&m, "https://ntfy.sh/storm-ok");
    let id = launch(&m);
    m.host_status(
        HOST,
        &id,
        &serde_json::from_value(json!({
            "status": "running",
            "program_status": {"state": "blocked", "kind": "permission", "app": "fake"},
        }))
        .unwrap(),
    )
    .unwrap();
    m.notifier_tick().unwrap();
    std::thread::sleep(crate::agent::ntfy::GRACE * 2);
    let due = pending_ntfy_ids(&m);
    assert_eq!(due.len(), 1);
    m.mark_ntfy_outcome(&due[0], "sent").unwrap();
    assert!(
        pending_ntfy_ids(&m).is_empty(),
        "a resolved row never comes up again"
    );
}

// ---------------------------------------------------------------------
// Grace cancellation: a device opening the session skips the send
// ---------------------------------------------------------------------

#[test]
fn an_active_device_already_showing_the_session_is_never_queued_for_it() {
    let (m, _d, _link) = manager();
    set_topic(&m, "https://ntfy.sh/storm-ok");
    let id = launch(&m);
    // A device has this very session open right now.
    m.set_presence("conn-a", "dev-a", true, Some(id.clone()));
    assert!(m.active_device_has_session_open(Some(id.as_str())));
    assert!(!m.active_device_has_session_open(None));
    assert!(!m.active_device_has_session_open(Some("ses_unrelated")));
}

#[test]
fn a_device_opening_the_session_during_the_grace_cancels_the_send() {
    let (m, _d, _link) = manager();
    set_topic(&m, "https://ntfy.sh/storm-ok");
    let id = launch(&m);
    m.host_status(
        HOST,
        &id,
        &serde_json::from_value(json!({
            "status": "running",
            "program_status": {"state": "blocked", "kind": "permission", "app": "fake"},
        }))
        .unwrap(),
    )
    .unwrap();
    m.notifier_tick().unwrap();
    // The person opens the session on another device before the grace
    // elapses — exactly what the forwarder re-checks at fire time.
    m.set_presence("conn-a", "dev-a", true, Some(id.clone()));
    std::thread::sleep(crate::agent::ntfy::GRACE * 2);
    let due = pending_ntfy_ids(&m);
    assert_eq!(
        due.len(),
        1,
        "it is still due — the forwarder decides, not the queue"
    );
    assert!(
        m.active_device_has_session_open(Some(id.as_str())),
        "the forwarder's own check would now skip it"
    );
}

// ---------------------------------------------------------------------
// Migration: an old database (before 161-9) gains the new columns, and
// every pre-existing row is swept to `skipped` rather than retroactively
// queued.
// ---------------------------------------------------------------------

#[test]
fn an_old_database_gains_the_ntfy_columns_and_old_rows_are_skipped() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    // The schema exactly as a database before this slice would have it
    // (`notifications` with `reopened` but no `ntfy_*`; `notification_prefs`
    // with no `ntfy_*` either).
    conn.execute_batch(
        "CREATE TABLE notifications (
             id TEXT PRIMARY KEY, seq INTEGER NOT NULL, updated_seq INTEGER NOT NULL,
             created TEXT NOT NULL, source TEXT NOT NULL, kind TEXT NOT NULL,
             session_id TEXT, host_id TEXT, provider TEXT, session_title TEXT,
             note_vault_id TEXT, note_id TEXT, title TEXT NOT NULL, body TEXT,
             urgency TEXT NOT NULL, key TEXT UNIQUE, silent INTEGER NOT NULL DEFAULT 0,
             silent_reason TEXT, read_at TEXT, read_by_device TEXT, due TEXT,
             cancelled INTEGER NOT NULL DEFAULT 0, reopened INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE notification_prefs (
             only_row INTEGER PRIMARY KEY CHECK (only_row = 1),
             version INTEGER NOT NULL DEFAULT 1, events TEXT NOT NULL,
             quiet_on INTEGER NOT NULL DEFAULT 0, quiet_start TEXT NOT NULL DEFAULT '22:00',
             quiet_end TEXT NOT NULL DEFAULT '08:00', quiet_tz TEXT NOT NULL DEFAULT 'UTC',
             quiet_days TEXT NOT NULL DEFAULT '*',
             quiet_allow_delete_requests INTEGER NOT NULL DEFAULT 0,
             summary_on INTEGER NOT NULL DEFAULT 1, paused_until TEXT,
             agent_default_on INTEGER NOT NULL DEFAULT 1,
             children_default_on INTEGER NOT NULL DEFAULT 0,
             per_session_limit INTEGER NOT NULL DEFAULT 20,
             retention_days INTEGER NOT NULL DEFAULT 30,
             retention_rows INTEGER NOT NULL DEFAULT 1000,
             mutes_version INTEGER NOT NULL DEFAULT 1,
             cleared_version INTEGER NOT NULL DEFAULT 1,
             updated_at TEXT NOT NULL
         );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO notifications (id, seq, updated_seq, created, source, kind, title, urgency)
         VALUES ('ntf_old', 1, 1, ?1, 'storm', 'needs_you', 'old notice', 'normal')",
        params![now()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO notification_prefs (only_row, version, events, updated_at)
         VALUES (1, 1, '{}', ?1)",
        params![now()],
    )
    .unwrap();

    super::init(&conn).unwrap();

    let state: String = conn
        .query_row(
            "SELECT ntfy_state FROM notifications WHERE id = 'ntf_old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "skipped",
        "a pre-161-9 row is never retroactively sent, whatever its default"
    );
    let url: Option<String> = conn
        .query_row(
            "SELECT ntfy_url FROM notification_prefs WHERE only_row = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(url, None);

    // Idempotent: running it again (as every later `Store::open` would)
    // changes nothing further.
    super::init(&conn).unwrap();
    let state_again: String = conn
        .query_row(
            "SELECT ntfy_state FROM notifications WHERE id = 'ntf_old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state_again, "skipped");
}

#[test]
fn a_read_or_cancelled_notice_shows_as_cleared_when_fetched_for_sending() {
    let (m, _d, _link) = manager();
    set_topic(&m, "https://ntfy.sh/storm-ok");
    let id = launch(&m);
    m.host_status(
        HOST,
        &id,
        &serde_json::from_value(json!({
            "status": "running",
            "program_status": {"state": "blocked", "kind": "permission", "app": "fake"},
        }))
        .unwrap(),
    )
    .unwrap();
    m.notifier_tick().unwrap();
    let notice_id = m.list_notifications(0, 10).unwrap()[0].id.clone();
    m.mark_notifications_read(std::slice::from_ref(&notice_id), None)
        .unwrap();
    std::thread::sleep(crate::agent::ntfy::GRACE * 2);
    let due = m.due_ntfy_notices("2999-01-01T00:00:00Z", 100).unwrap();
    let row = due.iter().find(|n| n.id == notice_id).unwrap();
    assert!(
        row.read_at.is_some(),
        "the forwarder's own read_at check would now skip it"
    );
}
