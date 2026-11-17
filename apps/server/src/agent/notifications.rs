//! Decision 161, slice 161-1: the server Notifier and inbox. The server
//! derives notices from state it already holds — the same transitions the
//! 154-4 toasts use today (`notice_of`/`notice_key`, ported from
//! `agent_notices.dart`) plus delete requests, usage limit and host offline
//! — stores every one in `notifications` (the inbox, read state synced
//! across devices), and applies account prefs (events, quiet hours, pause)
//! and mutes before deciding whether it interrupts. Nothing is raised for a
//! transition the server has not itself observed: at boot the Notifier's
//! per-session memory starts empty, so the first tick after a restart says
//! nothing, the same rule "nothing on a snapshot" gives the client.
//!
//! The MCP `notify` tool, OS/push delivery and device prefs are later
//! slices (161-2 onward); this one only builds the pipe and the inbox.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::attention::{PERMISSION_PROMPT, QUESTION, SIGN_IN};
use super::program::ProgramStatus;
use super::store::SessionRecord;
use super::{AgentError, AgentManager, AgentResult, now};

// ---------------------------------------------------------------------
// Event kinds
// ---------------------------------------------------------------------

pub const NEEDS_YOU: &str = "needs_you";
pub const DELETE_REQUEST: &str = "delete_request";
pub const FINISHED: &str = "finished";
pub const ERROR: &str = "error";
pub const USAGE_LIMIT: &str = "usage_limit";
pub const HOST_OFFLINE: &str = "host_offline";
pub const CHILD_FINISHED: &str = "child_finished";
/// Idle without `done`, or an ended session with no program status (the
/// client's existing fallback toast). Off for the server's own notice; the
/// in-app toast this amends is unchanged (161-3 keeps the client's own
/// `noticeOf` for that).
pub const STOPPED_WORKING: &str = "stopped_working";
pub const SUMMARY: &str = "summary";
pub const TEST: &str = "test";

/// Every automatic kind a pref row may toggle. `summary` and `test` are not
/// listed: a summary is controlled by `summary_on`, and a test always sends.
pub const AUTOMATIC_KINDS: &[&str] = &[
    NEEDS_YOU,
    DELETE_REQUEST,
    FINISHED,
    ERROR,
    USAGE_LIMIT,
    HOST_OFFLINE,
    CHILD_FINISHED,
    STOPPED_WORKING,
];

fn default_on(kind: &str) -> bool {
    !matches!(kind, CHILD_FINISHED | STOPPED_WORKING)
}

fn default_urgency(kind: &str) -> &'static str {
    match kind {
        NEEDS_YOU | DELETE_REQUEST => "high",
        CHILD_FINISHED | STOPPED_WORKING => "low",
        _ => "normal",
    }
}

/// Seconds a session must have worked before `finished` fires, or a host
/// must be offline before `host_offline` does (Q7). `None` for a kind with
/// no threshold.
fn default_threshold_s(kind: &str) -> Option<i64> {
    match kind {
        FINISHED => Some(30),
        HOST_OFFLINE => Some(120),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Schema (additive; agent.db)
// ---------------------------------------------------------------------

pub(super) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "-- 161-1: the inbox. `key` dedupes a transition seen twice (resync,
         -- restart, flap): the same key never notifies twice.
         CREATE TABLE IF NOT EXISTS notifications (
             id             TEXT PRIMARY KEY,
             seq            INTEGER NOT NULL,
             updated_seq    INTEGER NOT NULL,
             created        TEXT NOT NULL,
             source         TEXT NOT NULL CHECK (source IN ('storm', 'agent', 'terminal')),
             kind           TEXT NOT NULL,
             session_id     TEXT,
             host_id        TEXT,
             note_vault_id  TEXT,
             note_id        TEXT,
             title          TEXT NOT NULL,
             body           TEXT,
             urgency        TEXT NOT NULL CHECK (urgency IN ('low', 'normal', 'high')),
             key            TEXT UNIQUE,
             silent         INTEGER NOT NULL DEFAULT 0,
             read_at        TEXT,
             read_by_device TEXT,
             due            TEXT,
             cancelled      INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS notifications_by_seq ON notifications(seq);
         CREATE INDEX IF NOT EXISTS notifications_by_updated ON notifications(updated_seq);
         CREATE INDEX IF NOT EXISTS notifications_by_session ON notifications(session_id);

         -- One row, account-wide (Q15), `version` for optimistic concurrency
         -- across two devices editing at once.
         CREATE TABLE IF NOT EXISTS notification_prefs (
             only_row                    INTEGER PRIMARY KEY CHECK (only_row = 1),
             version                     INTEGER NOT NULL DEFAULT 1,
             events                      TEXT NOT NULL,
             quiet_on                    INTEGER NOT NULL DEFAULT 0,
             quiet_start                 TEXT NOT NULL DEFAULT '22:00',
             quiet_end                   TEXT NOT NULL DEFAULT '08:00',
             quiet_tz                    TEXT NOT NULL DEFAULT 'UTC',
             quiet_allow_delete_requests INTEGER NOT NULL DEFAULT 0,
             summary_on                  INTEGER NOT NULL DEFAULT 1,
             paused_until                TEXT,
             agent_default_on            INTEGER NOT NULL DEFAULT 1,
             children_default_on         INTEGER NOT NULL DEFAULT 0,
             per_session_limit           INTEGER NOT NULL DEFAULT 20,
             retention_days              INTEGER NOT NULL DEFAULT 30,
             retention_rows              INTEGER NOT NULL DEFAULT 1000,
             updated_at                  TEXT NOT NULL
         );

         -- Q17: per-session (until it ends, by default) or per-provider.
         CREATE TABLE IF NOT EXISTS notification_mutes (
             id                TEXT PRIMARY KEY,
             scope             TEXT NOT NULL CHECK (scope IN ('session', 'provider')),
             target            TEXT NOT NULL,
             until             TEXT,
             created_by_device TEXT,
             created_at        TEXT NOT NULL
         );
         CREATE UNIQUE INDEX IF NOT EXISTS notification_mutes_target
             ON notification_mutes(scope, target);",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------
// The transition engine: a Rust port of `noticeOf`/`noticeKey`
// (`agent_notices.dart`). Pure, so the 19-case matrix is tested without a
// database.
// ---------------------------------------------------------------------

/// What one tick knows of a session, enough to replay `noticeOf`. `None`
/// fields mean "no program status"/"no attention", exactly as an absent
/// `SessionView` field does on the client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Snap {
    pub status: String,
    pub activity: Option<String>,
    pub program: Option<ProgramStatus>,
    pub attention_kind: Option<&'static str>,
    pub attention_since: Option<String>,
    /// Where this session's output last moved: the client's `outputEnd`.
    /// Only used to key [`Notice::StoppedWorking`], which is silent by
    /// default, so an approximate watermark (here, `last_activity`) is
    /// enough — it only has to be stable, not byte-exact.
    pub watermark: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Notice {
    NeedsYou,
    FinishedTurn,
    HitError,
    StoppedWorking,
}

/// Decision 161's "needs you": a permission prompt, a question or a
/// sign-in. Delete requests and usage limit are their own events (below),
/// so — unlike the client's `needsYou`, whose toast shares a slot with
/// 153h's delete toast — this does not need the client's extra exclusion.
fn snap_needs_you(s: &Snap) -> bool {
    matches!(
        s.attention_kind,
        Some(PERMISSION_PROMPT | QUESTION | SIGN_IN)
    )
}

/// Port of `noticeOf(before, after)` (`agent_notices.dart:44-63`).
pub(super) fn notice_of(before: &Snap, after: &Snap) -> Option<Notice> {
    if !snap_needs_you(before) && snap_needs_you(after) {
        return Some(Notice::NeedsYou);
    }
    let idle = after.status == "running" && after.activity.as_deref() == Some("idle");
    let program = (after.status == "running")
        .then_some(after.program.as_ref())
        .flatten();
    let was = before.program.as_ref();
    let entered = |state: &str| {
        program.is_some_and(|p| p.state == state)
            && (was.map(|w| w.state) != Some(state)
                || was.map(|w| w.since.as_str()) != program.map(|p| p.since.as_str()))
    };
    if program.is_some() && idle {
        if entered("done") {
            return Some(Notice::FinishedTurn);
        }
        if entered("error") {
            return Some(Notice::HitError);
        }
    }
    if let Some(p) = program
        && p.state != "idle"
    {
        return None;
    }
    let was_working = before.status == "running" && before.activity.as_deref() == Some("working");
    let ended = matches!(after.status.as_str(), "completed" | "failed" | "stopped");
    (was_working && (ended || idle)).then_some(Notice::StoppedWorking)
}

/// Port of `noticeKey` (`agent_notices.dart:67-76`), as a collapse/dedupe
/// key (the Events table's column), prefixed `ses:<id>:`.
pub(super) fn notice_key(n: Notice, id: &str, s: &Snap) -> String {
    match n {
        Notice::NeedsYou => format!(
            "ses:{id}:need:{}",
            s.attention_since.as_deref().unwrap_or_default()
        ),
        Notice::FinishedTurn => format!(
            "ses:{id}:done:{}",
            s.program
                .as_ref()
                .map(|p| p.since.as_str())
                .unwrap_or_default()
        ),
        Notice::HitError => format!(
            "ses:{id}:err:{}",
            s.program
                .as_ref()
                .map(|p| p.since.as_str())
                .unwrap_or_default()
        ),
        Notice::StoppedWorking => format!(
            "ses:{id}:stopped:{}",
            s.watermark.as_deref().unwrap_or_default()
        ),
    }
}

fn display_title(r: &SessionRecord) -> String {
    r.title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| r.workspace.clone())
}

// ---------------------------------------------------------------------
// Prefs
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPref {
    pub on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_s: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub version: i64,
    pub events: HashMap<String, EventPref>,
    pub quiet_on: bool,
    pub quiet_start: String,
    pub quiet_end: String,
    pub quiet_tz: String,
    pub quiet_allow_delete_requests: bool,
    pub summary_on: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<String>,
    pub agent_default_on: bool,
    pub children_default_on: bool,
    pub per_session_limit: i64,
    pub retention_days: i64,
    pub retention_rows: i64,
}

impl Prefs {
    fn defaults() -> Self {
        let events = AUTOMATIC_KINDS
            .iter()
            .map(|k| {
                (
                    k.to_string(),
                    EventPref {
                        on: default_on(k),
                        threshold_s: default_threshold_s(k),
                    },
                )
            })
            .collect();
        Self {
            version: 1,
            events,
            quiet_on: false,
            quiet_start: "22:00".into(),
            quiet_end: "08:00".into(),
            quiet_tz: "UTC".into(),
            quiet_allow_delete_requests: false,
            summary_on: true,
            paused_until: None,
            agent_default_on: true,
            children_default_on: false,
            per_session_limit: 20,
            retention_days: 30,
            retention_rows: 1000,
        }
    }

    fn event_on(&self, kind: &str) -> bool {
        self.events
            .get(kind)
            .map(|e| e.on)
            .unwrap_or_else(|| default_on(kind))
    }

    pub(super) fn threshold_s(&self, kind: &str) -> i64 {
        self.events
            .get(kind)
            .and_then(|e| e.threshold_s)
            .or_else(|| default_threshold_s(kind))
            .unwrap_or(0)
    }
}

fn events_json(events: &HashMap<String, EventPref>) -> Result<String> {
    Ok(serde_json::to_string(events)?)
}

fn row_to_prefs(row: &rusqlite::Row) -> rusqlite::Result<Prefs> {
    let events_s: String = row.get(1)?;
    let events: HashMap<String, EventPref> = serde_json::from_str(&events_s).unwrap_or_default();
    Ok(Prefs {
        version: row.get(0)?,
        events,
        quiet_on: row.get(2)?,
        quiet_start: row.get(3)?,
        quiet_end: row.get(4)?,
        quiet_tz: row.get(5)?,
        quiet_allow_delete_requests: row.get(6)?,
        summary_on: row.get(7)?,
        paused_until: row.get(8)?,
        agent_default_on: row.get(9)?,
        children_default_on: row.get(10)?,
        per_session_limit: row.get(11)?,
        retention_days: row.get(12)?,
        retention_rows: row.get(13)?,
    })
}

const PREFS_COLUMNS: &str = "version, events, quiet_on, quiet_start, quiet_end, quiet_tz, \
    quiet_allow_delete_requests, summary_on, paused_until, agent_default_on, \
    children_default_on, per_session_limit, retention_days, retention_rows";

fn load_prefs(conn: &Connection) -> Result<Prefs> {
    if let Some(p) = conn
        .query_row(
            &format!("SELECT {PREFS_COLUMNS} FROM notification_prefs WHERE only_row = 1"),
            [],
            row_to_prefs,
        )
        .optional()?
    {
        return Ok(p);
    }
    let defaults = Prefs::defaults();
    conn.execute(
        "INSERT INTO notification_prefs
             (only_row, version, events, quiet_on, quiet_start, quiet_end, quiet_tz,
              quiet_allow_delete_requests, summary_on, paused_until, agent_default_on,
              children_default_on, per_session_limit, retention_days, retention_rows, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            defaults.version,
            events_json(&defaults.events)?,
            defaults.quiet_on,
            defaults.quiet_start,
            defaults.quiet_end,
            defaults.quiet_tz,
            defaults.quiet_allow_delete_requests,
            defaults.summary_on,
            defaults.paused_until,
            defaults.agent_default_on,
            defaults.children_default_on,
            defaults.per_session_limit,
            defaults.retention_days,
            defaults.retention_rows,
            now(),
        ],
    )?;
    Ok(defaults)
}

/// A pause with no explicit end (the dropdown's "until I turn them back
/// on"): a date past any real clock, rather than a second meaning for
/// `None` on `paused_until` (which means "not paused").
pub const PAUSE_INDEFINITELY: &str = "9999-12-31T23:59:59Z";

/// What a `PUT` may change; fields left `None` keep their stored value.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PrefsPatch {
    #[serde(default)]
    pub events: Option<HashMap<String, EventPref>>,
    #[serde(default)]
    pub quiet_on: Option<bool>,
    #[serde(default)]
    pub quiet_start: Option<String>,
    #[serde(default)]
    pub quiet_end: Option<String>,
    #[serde(default)]
    pub quiet_tz: Option<String>,
    #[serde(default)]
    pub quiet_allow_delete_requests: Option<bool>,
    #[serde(default)]
    pub summary_on: Option<bool>,
    #[serde(default)]
    pub agent_default_on: Option<bool>,
    #[serde(default)]
    pub children_default_on: Option<bool>,
    #[serde(default)]
    pub per_session_limit: Option<i64>,
    #[serde(default)]
    pub retention_days: Option<i64>,
    #[serde(default)]
    pub retention_rows: Option<i64>,
}

/// Applies `patch` over `base`, keeping everything the patch left out.
fn apply_patch(mut base: Prefs, patch: PrefsPatch) -> Prefs {
    if let Some(events) = patch.events {
        for (k, v) in events {
            base.events.insert(k, v);
        }
    }
    if let Some(v) = patch.quiet_on {
        base.quiet_on = v;
    }
    if let Some(v) = patch.quiet_start {
        base.quiet_start = v;
    }
    if let Some(v) = patch.quiet_end {
        base.quiet_end = v;
    }
    if let Some(v) = patch.quiet_tz {
        base.quiet_tz = v;
    }
    if let Some(v) = patch.quiet_allow_delete_requests {
        base.quiet_allow_delete_requests = v;
    }
    if let Some(v) = patch.summary_on {
        base.summary_on = v;
    }
    if let Some(v) = patch.agent_default_on {
        base.agent_default_on = v;
    }
    if let Some(v) = patch.children_default_on {
        base.children_default_on = v;
    }
    if let Some(v) = patch.per_session_limit {
        base.per_session_limit = v;
    }
    if let Some(v) = patch.retention_days {
        base.retention_days = v;
    }
    if let Some(v) = patch.retention_rows {
        base.retention_rows = v;
    }
    base
}

/// `UPDATE … WHERE version = expected`, so two devices saving at once don't
/// clobber each other (Q15). `Ok(None)` is the conflict.
fn save_prefs(
    conn: &Connection,
    expected_version: i64,
    patch: PrefsPatch,
) -> Result<Option<Prefs>> {
    let current = load_prefs(conn)?;
    if current.version != expected_version {
        return Ok(None);
    }
    let next = apply_patch(current, patch);
    let changed = conn.execute(
        "UPDATE notification_prefs SET
             version = version + 1, events = ?1, quiet_on = ?2, quiet_start = ?3,
             quiet_end = ?4, quiet_tz = ?5, quiet_allow_delete_requests = ?6,
             summary_on = ?7, agent_default_on = ?8, children_default_on = ?9,
             per_session_limit = ?10, retention_days = ?11, retention_rows = ?12,
             updated_at = ?13
         WHERE only_row = 1 AND version = ?14",
        params![
            events_json(&next.events)?,
            next.quiet_on,
            next.quiet_start,
            next.quiet_end,
            next.quiet_tz,
            next.quiet_allow_delete_requests,
            next.summary_on,
            next.agent_default_on,
            next.children_default_on,
            next.per_session_limit,
            next.retention_days,
            next.retention_rows,
            now(),
            expected_version,
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(load_prefs(conn)?))
}

fn set_paused_until(conn: &Connection, until: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE notification_prefs SET paused_until = ?1, updated_at = ?2 WHERE only_row = 1",
        params![until, now()],
    )?;
    Ok(())
}

/// `quiet_tz` parsed, falling back to UTC for a value that is not (or is no
/// longer) a known IANA zone rather than refusing the whole row.
fn tz_of(prefs: &Prefs) -> chrono_tz::Tz {
    prefs.quiet_tz.parse().unwrap_or(chrono_tz::UTC)
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?))
}

/// Whether `at` (an RFC3339 instant) falls inside quiet hours (Q16). The
/// window may cross midnight; `quiet_days` is not yet filtered (every day),
/// which is this slice's only simplification — tracked for 161-3.
pub(super) fn in_quiet_hours(prefs: &Prefs, at: &str) -> bool {
    if !prefs.quiet_on {
        return false;
    }
    let Ok(instant) = chrono::DateTime::parse_from_rfc3339(at) else {
        return false;
    };
    let local = instant.with_timezone(&tz_of(prefs)).time();
    let (Some((sh, sm)), Some((eh, em))) =
        (parse_hhmm(&prefs.quiet_start), parse_hhmm(&prefs.quiet_end))
    else {
        return false;
    };
    let start = chrono::NaiveTime::from_hms_opt(sh, sm, 0).unwrap_or_default();
    let end = chrono::NaiveTime::from_hms_opt(eh, em, 0).unwrap_or_default();
    if start <= end {
        local >= start && local < end
    } else {
        local >= start || local < end
    }
}

pub(super) fn is_paused(prefs: &Prefs, at: &str) -> bool {
    prefs
        .paused_until
        .as_deref()
        .is_some_and(|until| at < until)
}

// ---------------------------------------------------------------------
// Mutes
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Mute {
    pub id: String,
    pub scope: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    pub created_at: String,
}

fn row_to_mute(row: &rusqlite::Row) -> rusqlite::Result<Mute> {
    Ok(Mute {
        id: row.get(0)?,
        scope: row.get(1)?,
        target: row.get(2)?,
        until: row.get(3)?,
        created_at: row.get(5)?,
    })
}

const MUTE_COLUMNS: &str = "id, scope, target, until, created_by_device, created_at";

fn list_mutes(conn: &Connection) -> Result<Vec<Mute>> {
    let mut stmt = conn.prepare(&format!("SELECT {MUTE_COLUMNS} FROM notification_mutes"))?;
    let rows = stmt.query_map([], row_to_mute)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn create_mute(
    conn: &Connection,
    scope: &str,
    target: &str,
    until: Option<&str>,
    device: Option<&str>,
) -> Result<Mute> {
    let id = crate::auth::identity::random_id("ntm_");
    let created = now();
    conn.execute(
        "INSERT INTO notification_mutes (id, scope, target, until, created_by_device, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(scope, target) DO UPDATE SET until = ?4, created_by_device = ?5, created_at = ?6",
        params![id, scope, target, until, device, created],
    )?;
    Ok(conn.query_row(
        &format!("SELECT {MUTE_COLUMNS} FROM notification_mutes WHERE scope = ?1 AND target = ?2"),
        params![scope, target],
        row_to_mute,
    )?)
}

fn delete_mute(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM notification_mutes WHERE id = ?1", params![id])? > 0)
}

/// Removes every mute for a session (it covers a session that no longer
/// exists). Hooked where a session is forgotten.
pub(super) fn forget_session_mutes(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM notification_mutes WHERE scope = 'session' AND target = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// Q17: a session mute never hides a delete request (it expires in 10
/// minutes and needs the person regardless).
fn muted(
    mutes: &[Mute],
    session_id: Option<&str>,
    provider: Option<&str>,
    kind: &str,
    at: &str,
) -> bool {
    if kind == DELETE_REQUEST {
        return false;
    }
    mutes.iter().any(|m| {
        let hit = match m.scope.as_str() {
            "session" => session_id == Some(m.target.as_str()),
            "provider" => provider == Some(m.target.as_str()),
            _ => false,
        };
        hit && m.until.as_deref().is_none_or(|until| at < until)
    })
}

// ---------------------------------------------------------------------
// Notification rows
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub id: String,
    pub seq: i64,
    /// The stream's own cursor column (bumped on insert and on a read);
    /// not part of the wire shape.
    #[serde(skip)]
    pub updated_seq: i64,
    pub created: String,
    pub source: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_vault_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub urgency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub silent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_at: Option<String>,
    pub cancelled: bool,
}

const NOTIF_COLUMNS: &str = "id, seq, updated_seq, created, source, kind, session_id, host_id, \
    note_vault_id, note_id, title, body, urgency, key, silent, read_at, cancelled";

fn row_to_notification(row: &rusqlite::Row) -> rusqlite::Result<Notification> {
    Ok(Notification {
        id: row.get(0)?,
        seq: row.get(1)?,
        updated_seq: row.get(2)?,
        created: row.get(3)?,
        source: row.get(4)?,
        kind: row.get(5)?,
        session_id: row.get(6)?,
        host_id: row.get(7)?,
        note_vault_id: row.get(8)?,
        note_id: row.get(9)?,
        title: row.get(10)?,
        body: row.get(11)?,
        urgency: row.get(12)?,
        key: row.get(13)?,
        silent: row.get(14)?,
        read_at: row.get(15)?,
        cancelled: row.get(16)?,
    })
}

/// A notice to write, before precedence decides whether it is silent.
pub(super) struct NewNotice<'a> {
    pub source: &'static str,
    pub kind: &'static str,
    pub session_id: Option<&'a str>,
    pub host_id: Option<&'a str>,
    pub title: String,
    pub body: Option<String>,
    pub urgency: &'static str,
    pub key: Option<String>,
}

fn next_seq(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM notifications",
        [],
        |r| r.get(0),
    )?)
}

/// Inserts a notice, unless its key has already fired (the dedupe rule: a
/// transition seen twice never notifies twice). `Ok(None)` is the dedupe
/// case, not an error.
fn insert_notice(
    conn: &Connection,
    n: NewNotice<'_>,
    silent: bool,
) -> Result<Option<Notification>> {
    let id = crate::auth::identity::random_id("ntf_");
    let created = now();
    let seq = next_seq(conn)?;
    let changed = conn.execute(
        "INSERT INTO notifications
             (id, seq, updated_seq, created, source, kind, session_id, host_id, title, body,
              urgency, key, silent)
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(key) DO NOTHING",
        params![
            id,
            seq,
            created,
            n.source,
            n.kind,
            n.session_id,
            n.host_id,
            n.title,
            n.body,
            n.urgency,
            n.key,
            silent,
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(conn.query_row(
        &format!("SELECT {NOTIF_COLUMNS} FROM notifications WHERE id = ?1"),
        params![id],
        row_to_notification,
    )?))
}

fn bump_updated(conn: &Connection, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let seq = next_seq(conn)?;
    for (i, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE notifications SET updated_seq = ?1 WHERE id = ?2",
            params![seq + i as i64, id],
        )?;
    }
    Ok(())
}

fn list_notifications(conn: &Connection, after_seq: i64, limit: i64) -> Result<Vec<Notification>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {NOTIF_COLUMNS} FROM notifications WHERE seq > ?1 ORDER BY seq LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![after_seq, limit], row_to_notification)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// By `updated_seq` rather than `seq`: a mark-read moves a row's
/// `updated_seq` without changing its `seq`, so this is what the stream
/// polls to catch a read made from elsewhere (the "read sync" case) as
/// well as new rows. `GET /v1/notifications` uses [`list_notifications`]
/// (by `seq`) instead, since it is a stable page, not a live tail.
fn list_notifications_by_update(
    conn: &Connection,
    after_updated: i64,
    limit: i64,
) -> Result<Vec<Notification>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {NOTIF_COLUMNS} FROM notifications WHERE updated_seq > ?1 ORDER BY updated_seq LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![after_updated, limit], row_to_notification)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn mark_read(
    conn: &Connection,
    ids: &[String],
    device: Option<&str>,
    at: &str,
) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    for id in ids {
        let n = conn.execute(
            "UPDATE notifications SET read_at = ?2, read_by_device = ?3
             WHERE id = ?1 AND read_at IS NULL",
            params![id, at, device],
        )?;
        if n > 0 {
            changed.push(id.clone());
        }
    }
    bump_updated(conn, &changed)?;
    Ok(changed)
}

fn mark_read_for_session(
    conn: &Connection,
    session_id: &str,
    device: Option<&str>,
    at: &str,
) -> Result<Vec<String>> {
    let ids: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT id FROM notifications WHERE session_id = ?1 AND read_at IS NULL")?;
        let rows = stmt.query_map(params![session_id], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    mark_read(conn, &ids, device, at)
}

fn clear_inbox(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM notifications", [])?;
    Ok(())
}

/// 30 days or 1,000 rows, whichever is fewer (Q21), on account prefs.
fn child_events_since(conn: &Connection, after: i64) -> Result<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT rowid, child, kind FROM child_events
         WHERE rowid > ?1 AND kind IN ('reported', 'ended', 'failed')
         ORDER BY rowid",
    )?;
    let rows = stmt.query_map(params![after], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn silenced_counts(conn: &Connection, since: &str) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT kind, COUNT(*) FROM notifications
         WHERE created >= ?1 AND silent = 1 AND kind != ?2
         GROUP BY kind",
    )?;
    let rows = stmt.query_map(params![since, SUMMARY], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn prune(conn: &Connection, retention_days: i64, retention_rows: i64, at: &str) -> Result<usize> {
    let cutoff = crate::index::rfc3339_minus_secs(at, retention_days * 86_400);
    let by_age = conn.execute(
        "DELETE FROM notifications WHERE created < ?1",
        params![cutoff],
    )?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM notifications", [], |r| r.get(0))?;
    let by_count = if total > retention_rows {
        conn.execute(
            "DELETE FROM notifications WHERE id IN (
                 SELECT id FROM notifications ORDER BY seq LIMIT ?1
             )",
            params![total - retention_rows],
        )?
    } else {
        0
    };
    Ok(by_age + by_count)
}

// ---------------------------------------------------------------------
// Precedence (the Rules list under "Notification settings"): pause or
// quiet hours → the event's toggle → a session or provider mute. A notice
// stopped by any of these is still written to the inbox, marked silent —
// nothing is lost, it just never interrupts.
// ---------------------------------------------------------------------

pub(super) fn silent_for(
    prefs: &Prefs,
    mutes: &[Mute],
    kind: &str,
    session_id: Option<&str>,
    provider: Option<&str>,
    at: &str,
) -> bool {
    if is_paused(prefs, at) {
        return true;
    }
    if in_quiet_hours(prefs, at) && !(kind == DELETE_REQUEST && prefs.quiet_allow_delete_requests) {
        return true;
    }
    if !prefs.event_on(kind) {
        return true;
    }
    if muted(mutes, session_id, provider, kind, at) {
        return true;
    }
    false
}

fn elapsed_secs(from: &str, to: &str) -> Option<i64> {
    use time::format_description::well_known::Rfc3339;
    let a = time::OffsetDateTime::parse(from, &Rfc3339).ok()?;
    let b = time::OffsetDateTime::parse(to, &Rfc3339).ok()?;
    Some((b - a).whole_seconds())
}

fn summary_word(kind: &str, n: i64) -> &'static str {
    let plural = n != 1;
    match kind {
        FINISHED => "finished",
        NEEDS_YOU => "needing you",
        ERROR => {
            if plural {
                "errors"
            } else {
                "error"
            }
        }
        DELETE_REQUEST => "delete requests",
        USAGE_LIMIT => "usage limits",
        HOST_OFFLINE => {
            if plural {
                "hosts offline"
            } else {
                "host offline"
            }
        }
        CHILD_FINISHED => "child updates",
        _ => "events",
    }
}

/// Decision 149's `DeleteRequest`/`SessionRecord`, written into the inbox
/// on the same connection `request_delete` already holds, so the notice
/// and the request are made together or not at all. Called with the
/// store's lock already held (`deletes.rs`): never takes it itself.
pub(super) fn notify_delete_request(
    conn: &Connection,
    r: &SessionRecord,
    request: &super::deletes::DeleteRequest,
) -> Result<()> {
    let prefs = load_prefs(conn)?;
    let mutes = list_mutes(conn)?;
    let at = now();
    let silent = silent_for(
        &prefs,
        &mutes,
        DELETE_REQUEST,
        Some(&request.session_id),
        Some(&r.provider),
        &at,
    );
    let what = request
        .title
        .clone()
        .unwrap_or_else(|| request.path.clone());
    insert_notice(
        conn,
        NewNotice {
            source: "storm",
            kind: DELETE_REQUEST,
            session_id: Some(&request.session_id),
            host_id: None,
            title: format!("{} wants to delete {what}", display_title(r)),
            body: Some(request.reason.clone()),
            urgency: default_urgency(DELETE_REQUEST),
            key: Some(format!("ses:{}:del:{}", request.session_id, request.id)),
        },
        silent,
    )?;
    Ok(())
}

/// The Notifier's memory between ticks: nothing in here survives a
/// restart, which is why nothing is raised on one (the same rule the
/// client's toasts follow for a snapshot).
#[derive(Default)]
pub(super) struct NotifierMemory {
    prev: HashMap<String, Snap>,
    /// The link generation an offline host was last notified for, so a
    /// reconnect and a fresh outage are told apart.
    host_offline_notified: HashMap<String, u64>,
    last_child_event_rowid: i64,
    /// When the current quiet-hours-or-paused spell began, if one is open.
    quiet_since: Option<String>,
}

impl AgentManager {
    pub fn watch_notifications(&self) -> tokio::sync::watch::Receiver<u64> {
        self.notify_changed.subscribe()
    }

    pub(super) fn poke_notify(&self) {
        self.notify_changed.send_modify(|v| *v += 1);
    }

    pub fn notification_prefs(&self) -> AgentResult<Prefs> {
        Ok(load_prefs(&self.store.lock().unwrap().conn)?)
    }

    /// `PUT /v1/notifications/prefs`: `expected_version` must match the
    /// stored one, or this is a conflict (Q15) rather than a silent clobber.
    pub fn update_notification_prefs(
        &self,
        expected_version: i64,
        patch: PrefsPatch,
    ) -> AgentResult<Prefs> {
        let saved = {
            let store = self.store.lock().unwrap();
            save_prefs(&store.conn, expected_version, patch)?
        };
        match saved {
            Some(p) => {
                self.poke_notify();
                Ok(p)
            }
            None => Err(AgentError::Conflict("prefs_version".into())),
        }
    }

    pub fn list_notifications(&self, after: i64, limit: i64) -> AgentResult<Vec<Notification>> {
        Ok(list_notifications(
            &self.store.lock().unwrap().conn,
            after,
            limit.clamp(1, 500),
        )?)
    }

    /// The highest `updated_seq` right now (the stream's starting cursor;
    /// see [`list_notifications_by_update`]).
    pub fn notifications_update_cursor(&self) -> AgentResult<i64> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .conn
            .query_row(
                "SELECT COALESCE(MAX(updated_seq), 0) FROM notifications",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)?)
    }

    /// For `/v1/agent/events`: rows new or changed (including a read made
    /// elsewhere) since `after`, by `updated_seq`.
    pub fn list_notifications_since_update(
        &self,
        after: i64,
        limit: i64,
    ) -> AgentResult<Vec<Notification>> {
        Ok(list_notifications_by_update(
            &self.store.lock().unwrap().conn,
            after,
            limit.clamp(1, 500),
        )?)
    }

    pub fn mark_notifications_read(
        &self,
        ids: &[String],
        device: Option<&str>,
    ) -> AgentResult<Vec<String>> {
        let changed = {
            let store = self.store.lock().unwrap();
            mark_read(&store.conn, ids, device, &now())?
        };
        if !changed.is_empty() {
            self.poke_notify();
        }
        Ok(changed)
    }

    /// Opening a session marks its notices read everywhere (Q20).
    pub fn mark_session_notifications_read(
        &self,
        session_id: &str,
        device: Option<&str>,
    ) -> AgentResult<Vec<String>> {
        let changed = {
            let store = self.store.lock().unwrap();
            mark_read_for_session(&store.conn, session_id, device, &now())?
        };
        if !changed.is_empty() {
            self.poke_notify();
        }
        Ok(changed)
    }

    pub fn clear_notifications(&self) -> AgentResult<()> {
        clear_inbox(&self.store.lock().unwrap().conn)?;
        self.poke_notify();
        Ok(())
    }

    pub fn notification_mutes(&self) -> AgentResult<Vec<Mute>> {
        Ok(list_mutes(&self.store.lock().unwrap().conn)?)
    }

    pub fn mute_notifications(
        &self,
        scope: &str,
        target: &str,
        until: Option<&str>,
        device: Option<&str>,
    ) -> AgentResult<Mute> {
        if !matches!(scope, "session" | "provider") {
            return Err(AgentError::BadRequest(
                "scope must be \"session\" or \"provider\"".into(),
            ));
        }
        let m = create_mute(
            &self.store.lock().unwrap().conn,
            scope,
            target,
            until,
            device,
        )?;
        self.poke_notify();
        Ok(m)
    }

    pub fn unmute_notifications(&self, id: &str) -> AgentResult<bool> {
        let removed = delete_mute(&self.store.lock().unwrap().conn, id)?;
        if removed {
            self.poke_notify();
        }
        Ok(removed)
    }

    /// 1 hour, until tomorrow 08:00, or indefinitely: `until` is the
    /// resolved RFC3339 instant either way, chosen by the caller.
    pub fn pause_notifications(&self, until: &str) -> AgentResult<Prefs> {
        set_paused_until(&self.store.lock().unwrap().conn, Some(until))?;
        self.poke_notify();
        self.notification_prefs()
    }

    pub fn unpause_notifications(&self) -> AgentResult<Prefs> {
        set_paused_until(&self.store.lock().unwrap().conn, None)?;
        self.poke_notify();
        self.notification_prefs()
    }

    /// Sends a real notice through the inbox, so the page proves delivery
    /// rather than only permission. Never deduped: it carries no key.
    pub fn send_test_notification(&self) -> AgentResult<Notification> {
        let n = {
            let store = self.store.lock().unwrap();
            insert_notice(
                &store.conn,
                NewNotice {
                    source: "storm",
                    kind: TEST,
                    session_id: None,
                    host_id: None,
                    title: "Test notification".into(),
                    body: Some("If this reached you, delivery to this device works.".into()),
                    urgency: "normal",
                    key: None,
                },
                false,
            )?
            .expect("no key: never deduped")
        };
        self.poke_notify();
        Ok(n)
    }

    fn snap_of(&self, r: &SessionRecord) -> AgentResult<Snap> {
        let attention = self.attention(r)?;
        Ok(Snap {
            status: r.status.clone(),
            activity: r.activity.clone(),
            program: self.program_status(r),
            attention_kind: attention.as_ref().map(|a| a.kind),
            attention_since: attention.and_then(|a| a.since),
            watermark: r.last_activity.clone(),
        })
    }

    /// Everything one Notifier tick does: diffs every live session against
    /// its own memory of it (`notice_of`, plus usage limit), checks hosts
    /// offline and new child events, sends the one quiet-hours/pause
    /// summary, and runs the retention sweep. Run every few seconds
    /// (`lib.rs`); nothing here is raised for a session this server has
    /// not itself watched change, which is what keeps a restart quiet.
    pub fn notifier_tick(&self) -> AgentResult<()> {
        let at = now();
        let sessions = self.list()?;
        let (prefs, mutes) = {
            let store = self.store.lock().unwrap();
            (load_prefs(&store.conn)?, list_mutes(&store.conn)?)
        };
        let mut wrote = false;
        {
            let mut mem = self.notify_memory.lock().unwrap();
            for r in &sessions {
                let snap = self.snap_of(r)?;
                let Some(before) = mem.prev.insert(r.id.clone(), snap.clone()) else {
                    continue; // nothing on a snapshot or a restart
                };
                if let Some(n) = notice_of(&before, &snap) {
                    wrote |= self.emit_transition(&prefs, &mutes, r, n, (&before, &snap), &at)?;
                }
                if before.attention_kind != Some(super::attention::USAGE_LIMIT)
                    && snap.attention_kind == Some(super::attention::USAGE_LIMIT)
                {
                    let silent = silent_for(
                        &prefs,
                        &mutes,
                        USAGE_LIMIT,
                        Some(&r.id),
                        Some(&r.provider),
                        &at,
                    );
                    let store = self.store.lock().unwrap();
                    wrote |= insert_notice(
                        &store.conn,
                        NewNotice {
                            source: "storm",
                            kind: USAGE_LIMIT,
                            session_id: Some(&r.id),
                            host_id: None,
                            title: format!("{} hit its usage limit", display_title(r)),
                            body: None,
                            urgency: default_urgency(USAGE_LIMIT),
                            key: Some(format!(
                                "ses:{}:limit:{}",
                                r.id,
                                snap.attention_since.clone().unwrap_or_default()
                            )),
                        },
                        silent,
                    )?
                    .is_some();
                }
            }
            let live_ids: std::collections::HashSet<&str> =
                sessions.iter().map(|r| r.id.as_str()).collect();
            mem.prev.retain(|id, _| live_ids.contains(id.as_str()));
        }
        wrote |= self.notify_offline_hosts(&prefs, &mutes, &at)?;
        wrote |= self.notify_child_events(&prefs, &mutes, &at)?;
        wrote |= self.maybe_send_summary(&prefs, &at)?;
        {
            let store = self.store.lock().unwrap();
            prune(&store.conn, prefs.retention_days, prefs.retention_rows, &at)?;
        }
        if wrote {
            self.poke_notify();
        }
        Ok(())
    }

    fn emit_transition(
        &self,
        prefs: &Prefs,
        mutes: &[Mute],
        r: &SessionRecord,
        n: Notice,
        transition: (&Snap, &Snap),
        at: &str,
    ) -> AgentResult<bool> {
        let (before, after) = transition;
        let (kind, title, key): (&'static str, String, String) = match n {
            Notice::NeedsYou => (
                NEEDS_YOU,
                format!("{} needs you", display_title(r)),
                notice_key(n, &r.id, after),
            ),
            Notice::FinishedTurn => {
                let started = before.program.as_ref().map(|p| p.since.as_str());
                let worked = started
                    .and_then(|s| elapsed_secs(s, at))
                    .unwrap_or(i64::MAX);
                if worked < prefs.threshold_s(FINISHED) {
                    return Ok(false);
                }
                (
                    FINISHED,
                    format!("{} finished its turn", display_title(r)),
                    notice_key(n, &r.id, after),
                )
            }
            Notice::HitError => (
                ERROR,
                format!("{} hit an error", display_title(r)),
                notice_key(n, &r.id, after),
            ),
            // The lifecycle's own `failed` (no program status) is on by
            // default, unlike the generic stopped-working fallback it would
            // otherwise be ported as (Events table: "Hit an error / failed").
            Notice::StoppedWorking if after.status == "failed" => (
                ERROR,
                format!("{} failed", display_title(r)),
                format!("ses:{}:failed", r.id),
            ),
            Notice::StoppedWorking => (
                STOPPED_WORKING,
                format!("{} stopped working", display_title(r)),
                notice_key(n, &r.id, after),
            ),
        };
        let silent = silent_for(prefs, mutes, kind, Some(&r.id), Some(&r.provider), at);
        let store = self.store.lock().unwrap();
        Ok(insert_notice(
            &store.conn,
            NewNotice {
                source: "storm",
                kind,
                session_id: Some(&r.id),
                host_id: None,
                title,
                body: None,
                urgency: default_urgency(kind),
                key: Some(key),
            },
            silent,
        )?
        .is_some())
    }

    /// Q7: only if offline at least [`default_threshold_s`]`(HOST_OFFLINE)`
    /// and it had running sessions when it went down.
    fn notify_offline_hosts(&self, prefs: &Prefs, mutes: &[Mute], at: &str) -> AgentResult<bool> {
        let threshold = prefs.threshold_s(HOST_OFFLINE);
        let mut wrote = false;
        let mut mem = self.notify_memory.lock().unwrap();
        for (host_id, last_seen, generation) in self.offline_hosts() {
            if mem.host_offline_notified.get(&host_id) == Some(&generation) {
                continue;
            }
            let Some(elapsed) = elapsed_secs(&last_seen, at) else {
                continue;
            };
            if elapsed < threshold {
                continue;
            }
            mem.host_offline_notified
                .insert(host_id.clone(), generation);
            let had_running = {
                let store = self.store.lock().unwrap();
                store
                    .conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sessions WHERE host_id = ?1
                             AND status NOT IN ('completed', 'failed', 'stopped', 'parked'))",
                        params![host_id],
                        |r| r.get::<_, bool>(0),
                    )
                    .unwrap_or(false)
            };
            if !had_running {
                continue;
            }
            let silent = silent_for(prefs, mutes, HOST_OFFLINE, None, None, at);
            let store = self.store.lock().unwrap();
            wrote |= insert_notice(
                &store.conn,
                NewNotice {
                    source: "storm",
                    kind: HOST_OFFLINE,
                    session_id: None,
                    host_id: Some(&host_id),
                    title: "A host went offline".into(),
                    body: None,
                    urgency: default_urgency(HOST_OFFLINE),
                    key: Some(format!("host:{host_id}:offline:{generation}")),
                },
                silent,
            )?
            .is_some();
        }
        Ok(wrote)
    }

    /// Off by default (Q7): the parent already hears a child's events
    /// through decision 144's own queue. New rows only, by `rowid`.
    fn notify_child_events(&self, prefs: &Prefs, mutes: &[Mute], at: &str) -> AgentResult<bool> {
        let after = self.notify_memory.lock().unwrap().last_child_event_rowid;
        let rows = child_events_since(&self.store.lock().unwrap().conn, after)?;
        let mut wrote = false;
        for (rowid, child, kind) in &rows {
            self.notify_memory.lock().unwrap().last_child_event_rowid = *rowid;
            let Ok(r) = self.get(child) else { continue };
            let verb = match kind.as_str() {
                "failed" => "failed",
                "ended" => "ended",
                _ => "reported back",
            };
            let silent = silent_for(
                prefs,
                mutes,
                CHILD_FINISHED,
                Some(child),
                Some(&r.provider),
                at,
            );
            let store = self.store.lock().unwrap();
            wrote |= insert_notice(
                &store.conn,
                NewNotice {
                    source: "storm",
                    kind: CHILD_FINISHED,
                    session_id: Some(child),
                    host_id: None,
                    title: format!("{} {verb}", display_title(&r)),
                    body: None,
                    urgency: default_urgency(CHILD_FINISHED),
                    key: Some(format!("ses:{child}:child:{rowid}")),
                },
                silent,
            )?
            .is_some();
        }
        Ok(wrote)
    }

    /// One summary when quiet hours or a pause ends (Q16), counting what
    /// was held silent while it was active. On by default.
    fn maybe_send_summary(&self, prefs: &Prefs, at: &str) -> AgentResult<bool> {
        let active = in_quiet_hours(prefs, at) || is_paused(prefs, at);
        let since = {
            let mut mem = self.notify_memory.lock().unwrap();
            match (active, mem.quiet_since.clone()) {
                (true, None) => {
                    mem.quiet_since = Some(at.to_string());
                    None
                }
                (true, Some(_)) | (false, None) => None,
                (false, Some(since)) => {
                    mem.quiet_since = None;
                    Some(since)
                }
            }
        };
        let Some(since) = since else { return Ok(false) };
        if !prefs.summary_on {
            return Ok(false);
        }
        let store = self.store.lock().unwrap();
        let counts = silenced_counts(&store.conn, &since)?;
        if counts.is_empty() {
            return Ok(false);
        }
        let body = counts
            .iter()
            .map(|(kind, n)| format!("{n} {}", summary_word(kind, *n)))
            .collect::<Vec<_>>()
            .join(", ");
        let date = &at[..at.len().min(10)];
        Ok(insert_notice(
            &store.conn,
            NewNotice {
                source: "storm",
                kind: SUMMARY,
                session_id: None,
                host_id: None,
                title: "While you were away".into(),
                body: Some(body),
                urgency: "normal",
                key: Some(format!("summary:{date}")),
            },
            false,
        )?
        .is_some())
    }

    #[cfg(test)]
    pub fn store_for_tests(&self) -> std::sync::MutexGuard<'_, super::store::Store> {
        self.store.lock().unwrap()
    }

    /// A child event this test did not go through `record_in` to make, so
    /// a `notifier_tick` picks it up exactly as it would a real one.
    #[cfg(test)]
    pub fn insert_child_event_for_tests(&self, parent: &str, child: &str, kind: &str) {
        let store = self.store.lock().unwrap();
        store
            .conn
            .execute(
                "INSERT INTO child_events (parent, seq, child, kind, status, source, confidence, at)
                 VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM child_events WHERE parent = ?1),
                         ?2, ?3, 'running', 'storm', 'certain', ?4)",
                params![parent, child, kind, now()],
            )
            .unwrap();
    }

    /// Puts `host_id` offline (as `disconnect_host` would) as of
    /// `seconds_ago`, without needing a real link and a real clock.
    #[cfg(test)]
    pub fn set_host_offline_for_tests(&self, host_id: &str, seconds_ago: i64) {
        let mut live = self.live.lock().unwrap();
        live.hosts.remove(host_id);
        live.last_seen_at.insert(
            host_id.to_string(),
            crate::index::rfc3339_minus_secs(&now(), seconds_ago),
        );
        live.offline_generation.insert(host_id.to_string(), 1);
    }
}

#[cfg(test)]
mod tests;
