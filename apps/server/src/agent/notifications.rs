//! Decision 161, slice 161-1: the server Notifier and inbox. The server
//! derives notices from state it already holds — the same transitions the
//! 154-4 toasts use today (`notice_of`/`notice_key`, ported from
//! `agent_notices.dart`) plus delete requests, usage limit and host offline
//! — stores every one in `notifications` (the inbox, read state synced
//! across devices), and applies account prefs (events, quiet hours, pause)
//! and mutes before deciding whether it interrupts. Nothing is raised for a
//! transition the server has not itself observed: at boot the Notifier's
//! per-session memory starts empty, so the first tick after a restart says
//! nothing, the same rule "nothing on a snapshot" gives the client — except
//! for a session created after the Notifier itself started, which has no
//! earlier state to have missed.
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
use super::{AgentError, AgentManager, AgentResult, SessionLive, TerminalNoticeItem, now};

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
/// 161-2: a notice the `notify` tool sent. Not in [`AUTOMATIC_KINDS`] — it
/// has no on/off toggle of its own (the "Notifies you" capability is the
/// gate); it still goes through [`silent_for`], so a mute, quiet hours or a
/// pause hold it exactly like an automatic notice.
pub const AGENT_MESSAGE: &str = "agent_message";
/// 161-2: "{session} sent N more notifications" when a session's own
/// notices are rate-limited (Q5). One per session per hour window, bumped
/// in place as more are limited.
pub const AGENT_ROLLUP: &str = "agent_rollup";
/// 161-5: a notice forwarded from a session's terminal (OSC 9/777/99,
/// AM126). Not in [`AUTOMATIC_KINDS`] either — like [`AGENT_MESSAGE`], its
/// gate is the "Notifies you" capability, not an account toggle; it goes
/// through [`silent_for`] under [`AGENT_MESSAGE`]'s own precedence.
pub const TERMINAL_NOTICE: &str = "terminal_notice";

// ---------------------------------------------------------------------
// 161-2: the `notify` tool's own limits (Q5), independent of the account
// prefs' `per_session_limit` setting applied on top of them.
// ---------------------------------------------------------------------

/// `title`'s bound after sanitising. Over this is `bad_title`, not a
/// silent truncation (the draft's parameter table).
pub const NOTIFY_TITLE_MAX: usize = 80;
/// `body`'s bound after sanitising; truncated, never refused.
pub const NOTIFY_BODY_MAX: usize = 500;
/// `key`'s own bound (`[A-Za-z0-9._:-]`, 1–64).
pub const NOTIFY_KEY_MAX: usize = 64;
/// A token bucket's burst allowance per session.
const AGENT_BURST: f64 = 3.0;
/// Refills one token every this many seconds.
const AGENT_REFILL_SECS: f64 = 120.0;
/// Account-wide agent notices per hour; Storm's own automatic notices are
/// never counted.
const AGENT_ACCOUNT_HOURLY: i64 = 60;
/// At most this many delayed notices pending per session.
const AGENT_MAX_PENDING: i64 = 10;
/// An identical (title, body) from the same session within this window is
/// the same notice.
const AGENT_DUP_WINDOW_SECS: i64 = 600;

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
             provider       TEXT,
             session_title  TEXT,
             note_vault_id  TEXT,
             note_id        TEXT,
             title          TEXT NOT NULL,
             body           TEXT,
             urgency        TEXT NOT NULL CHECK (urgency IN ('low', 'normal', 'high')),
             key            TEXT UNIQUE,
             silent         INTEGER NOT NULL DEFAULT 0,
             silent_reason  TEXT CHECK (silent_reason IN ('quiet', 'paused', 'off', 'muted')),
             read_at        TEXT,
             read_by_device TEXT,
             due            TEXT,
             cancelled      INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS notifications_by_seq ON notifications(seq);
         CREATE INDEX IF NOT EXISTS notifications_by_updated ON notifications(updated_seq);
         CREATE INDEX IF NOT EXISTS notifications_by_session ON notifications(session_id);

         -- One monotonic counter, persisted and never reused: `seq` on
         -- insert and `updated_seq` on every update (including a read) both
         -- take the next value, so the live stream's `updated_seq > cursor`
         -- poll can never skip a row — unlike deriving either from
         -- `MAX(seq)`/`MAX(updated_seq)` over `notifications`, which repeats
         -- a value after a read bump (no new max) or after `DELETE FROM
         -- notifications` (the max resets to nothing).
         CREATE TABLE IF NOT EXISTS notification_counter (
             only_row INTEGER PRIMARY KEY CHECK (only_row = 1),
             value    INTEGER NOT NULL DEFAULT 0
         );

         -- One row, account-wide (Q15), `version` for optimistic concurrency
         -- across two devices editing at once. `mutes_version` and
         -- `cleared_version` are separate counters for the stream's
         -- `mutes_changed`/`notifications_cleared` events; they are not part
         -- of the prefs a client edits.
         CREATE TABLE IF NOT EXISTS notification_prefs (
             only_row                    INTEGER PRIMARY KEY CHECK (only_row = 1),
             version                     INTEGER NOT NULL DEFAULT 1,
             events                      TEXT NOT NULL,
             quiet_on                    INTEGER NOT NULL DEFAULT 0,
             quiet_start                 TEXT NOT NULL DEFAULT '22:00',
             quiet_end                   TEXT NOT NULL DEFAULT '08:00',
             quiet_tz                    TEXT NOT NULL DEFAULT 'UTC',
             -- Every day for now (161-3 adds per-day filtering); stored so
             -- that slice is additive rather than another migration.
             quiet_days                  TEXT NOT NULL DEFAULT '*',
             quiet_allow_delete_requests INTEGER NOT NULL DEFAULT 0,
             summary_on                  INTEGER NOT NULL DEFAULT 1,
             paused_until                TEXT,
             agent_default_on            INTEGER NOT NULL DEFAULT 1,
             children_default_on         INTEGER NOT NULL DEFAULT 0,
             per_session_limit           INTEGER NOT NULL DEFAULT 20,
             retention_days              INTEGER NOT NULL DEFAULT 30,
             retention_rows              INTEGER NOT NULL DEFAULT 1000,
             mutes_version               INTEGER NOT NULL DEFAULT 1,
             cleared_version             INTEGER NOT NULL DEFAULT 1,
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
             ON notification_mutes(scope, target);

         -- 161-2: the `notify` tool's per-session token bucket (burst 3,
         -- refilling 1 every 2 minutes, Q5). `tokens` is fractional so a
         -- partial refill is never lost between calls.
         CREATE TABLE IF NOT EXISTS session_notify_bucket (
             session_id TEXT PRIMARY KEY,
             tokens     REAL NOT NULL,
             updated_at TEXT NOT NULL
         );

         -- 161-2: a `notify` call with `delay_s`, queued here (not yet in
         -- the inbox) until the Notifier fires it at `due`, under the
         -- prefs/mutes/quiet/pause in force then, not when it was asked
         -- for.
         CREATE TABLE IF NOT EXISTS notify_pending (
             id            TEXT PRIMARY KEY,
             session_id    TEXT NOT NULL,
             provider      TEXT,
             session_title TEXT,
             note_vault_id TEXT,
             note_id       TEXT,
             title         TEXT NOT NULL,
             body          TEXT,
             urgency       TEXT NOT NULL,
             key           TEXT,
             due           TEXT NOT NULL,
             created_at    TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS notify_pending_by_due ON notify_pending(due);
         CREATE INDEX IF NOT EXISTS notify_pending_by_session ON notify_pending(session_id);

         -- 161-2: one roll-up notice per session per hour window (Q5),
         -- bumped in place as more of the session's notices are limited.
         CREATE TABLE IF NOT EXISTS session_notify_rollup (
             session_id TEXT NOT NULL,
             window     TEXT NOT NULL,
             count      INTEGER NOT NULL,
             notice_id  TEXT NOT NULL,
             PRIMARY KEY (session_id, window)
         );",
    )?;
    Ok(())
}

/// The next value of the persisted counter, atomically. See the schema
/// comment on `notification_counter`.
fn next_counter(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "INSERT INTO notification_counter (only_row, value) VALUES (1, 1)
         ON CONFLICT(only_row) DO UPDATE SET value = value + 1
         RETURNING value",
        [],
        |r| r.get(0),
    )?)
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
/// sign-in. Delete requests are their own event (below), so — unlike the
/// client's `needsYou`, whose toast shares a slot with 153h's delete toast
/// — this does not need the client's `delete_approval` exclusion.
///
/// `"approval"` is checked for parity with the client's `kNeedsYouKinds`
/// (`sidebar_rules.dart:29-35`); nothing server-side produces it today, so
/// this is inert until something does. `attention::BLOCKED` ("134h": a
/// round whose result outcome is `blocked`) is deliberately **not** here —
/// `kNeedsYouKinds` does not contain it either, and `tierOf`
/// (`sidebar_rules.dart:54-61`) checks it only *after* `needsYou`, as its
/// own, lower `SidebarTier.blocked` tier. Promoting it to needs-you would
/// be a real behaviour change from the client this is a port of.
fn snap_needs_you(s: &Snap) -> bool {
    matches!(
        s.attention_kind,
        Some(PERMISSION_PROMPT | QUESTION | SIGN_IN)
    ) || s.attention_kind == Some("approval")
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
/// key (the Events table's column), prefixed `ses:<id>:`. `at` is the
/// fallback for `NeedsYou` when there is no `attention_since` (evidence
/// that may be missing, `attention.rs`) — without one, every later
/// needs-you for the session would otherwise share one constant key and,
/// because `key` is `UNIQUE`, never fire again.
pub(super) fn notice_key(n: Notice, id: &str, s: &Snap, at: &str) -> String {
    match n {
        Notice::NeedsYou => format!(
            "ses:{id}:need:{}",
            s.attention_since.as_deref().unwrap_or(at)
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
        Notice::StoppedWorking => {
            format!("ses:{id}:stopped:{}", s.watermark.as_deref().unwrap_or(at))
        }
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

/// An agent's own text (a delete request's `reason`, `notify`'s `title`
/// and `body`): plain, with control, bidi-override and zero-width
/// characters removed and newlines collapsed — the draft's security
/// section: agent text is always sanitised, never shown raw. `None` if
/// nothing is left. Not truncated: `notify`'s `title` must be rejected
/// (`bad_title`) rather than silently shortened, so the length check runs
/// on this, full length.
fn strip_agent_text(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut last_was_space = false;
    for c in s.chars() {
        let space = match c {
            '\n' | '\r' | '\t' => true,
            // Zero-width and bidi-control characters (ZWSP..ZWJ/LRM..RLM,
            // the bidi embedding/override/isolate controls, BOM/ZWNBSP).
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}' => continue,
            c if c.is_control() => continue,
            ' ' => true,
            _ => false,
        };
        if space {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(c);
            last_was_space = false;
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// [`strip_agent_text`], then bounded: the delete request's `reason` and
/// `notify`'s `body` are truncated rather than refused.
fn sanitise_agent_text(s: &str, max_chars: usize) -> Option<String> {
    strip_agent_text(s).map(|t| t.chars().take(max_chars).collect())
}

/// `notify`'s `title`: sanitised, then refused (`bad_title`) rather than
/// truncated if it is empty or still over [`NOTIFY_TITLE_MAX`] — the
/// draft's parameter table.
pub fn checked_title(s: &str) -> Option<String> {
    let t = strip_agent_text(s)?;
    (t.chars().count() <= NOTIFY_TITLE_MAX).then_some(t)
}

/// `notify`'s `body`: sanitised and bounded; never refused.
pub fn sanitised_body(s: &str, max_chars: usize) -> Option<String> {
    sanitise_agent_text(s, max_chars)
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

pub(super) fn load_prefs(conn: &Connection) -> Result<Prefs> {
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

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?))
}

fn valid_hhmm(s: &str) -> bool {
    parse_hhmm(s).is_some_and(|(h, m)| h < 24 && m < 60)
}

/// What the person may set is checked here, once, rather than at each of
/// `prune`/`in_quiet_hours`/`threshold_s`'s call sites: a stored row is
/// always within range, so those stay simple.
fn validate_prefs(p: &Prefs) -> std::result::Result<(), String> {
    if !(1..=365).contains(&p.retention_days) {
        return Err("retention_days must be 1-365".into());
    }
    if !(50..=10_000).contains(&p.retention_rows) {
        return Err("retention_rows must be 50-10000".into());
    }
    if !(1..=200).contains(&p.per_session_limit) {
        return Err("per_session_limit must be 1-200".into());
    }
    if !valid_hhmm(&p.quiet_start) || !valid_hhmm(&p.quiet_end) {
        return Err("quiet_start and quiet_end must be \"HH:MM\"".into());
    }
    if p.quiet_tz.parse::<chrono_tz::Tz>().is_err() {
        return Err(format!("\"{}\" is not a known time zone", p.quiet_tz));
    }
    for (k, v) in &p.events {
        if !AUTOMATIC_KINDS.contains(&k.as_str()) {
            return Err(format!("\"{k}\" is not a notification event"));
        }
        if let Some(t) = v.threshold_s
            && !(0..=86_400).contains(&t)
        {
            return Err(format!("{k}'s threshold_s must be 0-86400"));
        }
    }
    Ok(())
}

/// `save_prefs`'s two ways of not saving: `expected_version` was stale, or
/// the result of applying the patch would not validate.
pub(super) enum SaveOutcome {
    Saved(Prefs),
    Conflict,
    Invalid(String),
}

/// `UPDATE … WHERE version = expected`, so two devices saving at once don't
/// clobber each other (Q15).
fn save_prefs(conn: &Connection, expected_version: i64, patch: PrefsPatch) -> Result<SaveOutcome> {
    let current = load_prefs(conn)?;
    if current.version != expected_version {
        return Ok(SaveOutcome::Conflict);
    }
    let next = apply_patch(current, patch);
    if let Err(msg) = validate_prefs(&next) {
        return Ok(SaveOutcome::Invalid(msg));
    }
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
        return Ok(SaveOutcome::Conflict);
    }
    Ok(SaveOutcome::Saved(load_prefs(conn)?))
}

/// Bumps `version` too, so `prefs_changed` reaches every other device —
/// pausing from the phone has to be able to say "Paused" on the desktop.
fn set_paused_until(conn: &Connection, until: Option<&str>) -> Result<()> {
    load_prefs(conn)?; // ensures the row exists before the bare UPDATE below
    conn.execute(
        "UPDATE notification_prefs SET paused_until = ?1, version = version + 1, updated_at = ?2
         WHERE only_row = 1",
        params![until, now()],
    )?;
    Ok(())
}

fn bump_mutes_version(conn: &Connection) -> Result<()> {
    load_prefs(conn)?;
    conn.execute(
        "UPDATE notification_prefs SET mutes_version = mutes_version + 1 WHERE only_row = 1",
        [],
    )?;
    Ok(())
}

fn mutes_version(conn: &Connection) -> Result<i64> {
    load_prefs(conn)?;
    Ok(conn.query_row(
        "SELECT mutes_version FROM notification_prefs WHERE only_row = 1",
        [],
        |r| r.get(0),
    )?)
}

fn bump_cleared_version(conn: &Connection) -> Result<()> {
    load_prefs(conn)?;
    conn.execute(
        "UPDATE notification_prefs SET cleared_version = cleared_version + 1 WHERE only_row = 1",
        [],
    )?;
    Ok(())
}

fn cleared_version(conn: &Connection) -> Result<i64> {
    load_prefs(conn)?;
    Ok(conn.query_row(
        "SELECT cleared_version FROM notification_prefs WHERE only_row = 1",
        [],
        |r| r.get(0),
    )?)
}

/// `quiet_tz` parsed, falling back to UTC for a value that is not (or is no
/// longer) a known IANA zone rather than refusing the whole row. A stored
/// value is always valid (`validate_prefs`); this only guards an older row.
fn tz_of(prefs: &Prefs) -> chrono_tz::Tz {
    prefs.quiet_tz.parse().unwrap_or(chrono_tz::UTC)
}

/// Whether `at` (an RFC3339 instant) falls inside quiet hours (Q16). The
/// window may cross midnight; `quiet_days` is stored but not yet filtered
/// (every day), tracked for 161-3.
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

/// `s` as an RFC3339 instant strictly after `at`, reformatted to UTC `Z` —
/// what a mute's or a pause's `until` is stored as, so a later string
/// comparison against `now()` (also UTC `Z`) is correct even when the
/// caller sent an offset like `+01:00`.
fn parse_future_utc(s: &str, at: &str) -> std::result::Result<String, String> {
    use time::format_description::well_known::Rfc3339;
    let parsed = time::OffsetDateTime::parse(s, &Rfc3339)
        .map_err(|_| "must be an RFC3339 time".to_string())?;
    let now = time::OffsetDateTime::parse(at, &Rfc3339).map_err(|_| "bad clock".to_string())?;
    if parsed <= now {
        return Err("must be in the future".into());
    }
    parsed
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|_| "bad time".into())
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

/// Whether `id` is a session the server knows of (any status) or a
/// provider some session has used — the mute-target check (item 5): a
/// typo'd id would otherwise mute nothing, silently.
fn known_mute_target(conn: &Connection, scope: &str, target: &str) -> Result<bool> {
    let column = match scope {
        "session" => "id",
        _ => "provider",
    };
    Ok(conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM sessions WHERE {column} = ?1)"),
        params![target],
        |r| r.get(0),
    )?)
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

/// 161-2 F2: a session's queued delayed notices, dropped at the moment it
/// ends rather than left for the Notifier to find due and discard one by
/// one. Hooked where `update_with` first sees `is_ended()`.
pub(super) fn forget_due_at_end(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM notify_pending WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// 161-2 F2: the "Notifies you" flag and its rate-limit and roll-up state,
/// removed with a session (its inbox rows are kept, as a mute's are, for
/// provenance). Hooked where a session is forgotten.
pub(super) fn forget_session_notify(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM session_notify WHERE session_id = ?1",
        params![session_id],
    )?;
    conn.execute(
        "DELETE FROM session_notify_bucket WHERE session_id = ?1",
        params![session_id],
    )?;
    conn.execute(
        "DELETE FROM session_notify_rollup WHERE session_id = ?1",
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
    /// The session's agent, as it was when the notice was made (161-3/4:
    /// "{session} · {Agent}"); `null` for a notice with no session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The session's display title at the time, kept even if the session
    /// is later removed — the client would otherwise have to join against
    /// the session list, which a closed session may have left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
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
    /// Why it is silent (`quiet`/`paused`/`off`/`muted`), so the inbox can
    /// say why it was held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silent_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_at: Option<String>,
    pub cancelled: bool,
}

const NOTIF_COLUMNS: &str = "id, seq, updated_seq, created, source, kind, session_id, host_id, \
    provider, session_title, note_vault_id, note_id, title, body, urgency, key, silent, \
    silent_reason, read_at, cancelled";

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
        provider: row.get(8)?,
        session_title: row.get(9)?,
        note_vault_id: row.get(10)?,
        note_id: row.get(11)?,
        title: row.get(12)?,
        body: row.get(13)?,
        urgency: row.get(14)?,
        key: row.get(15)?,
        silent: row.get(16)?,
        silent_reason: row.get(17)?,
        read_at: row.get(18)?,
        cancelled: row.get(19)?,
    })
}

/// A notice to write, before precedence decides whether it is silent.
pub(super) struct NewNotice<'a> {
    pub source: &'static str,
    pub kind: &'static str,
    pub session_id: Option<&'a str>,
    pub host_id: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub session_title: Option<&'a str>,
    pub note_vault_id: Option<&'a str>,
    pub note_id: Option<&'a str>,
    pub title: String,
    pub body: Option<String>,
    pub urgency: &'static str,
    pub key: Option<String>,
}

impl<'a> NewNotice<'a> {
    /// A notice about `r`: `provider`/`session_title` filled in from it.
    fn for_session(r: &'a SessionRecord, kind: &'static str, title: String) -> Self {
        Self {
            source: "storm",
            kind,
            session_id: Some(&r.id),
            host_id: None,
            provider: Some(r.provider.as_str()),
            session_title: Some(r.title.as_deref().unwrap_or(r.workspace.as_str())),
            note_vault_id: None,
            note_id: None,
            title,
            body: None,
            urgency: default_urgency(kind),
            key: None,
        }
    }

    fn with_key(mut self, key: String) -> Self {
        self.key = Some(key);
        self
    }

    fn with_body(mut self, body: Option<String>) -> Self {
        self.body = body;
        self
    }
}

/// Inserts a notice, unless its key has already fired (the dedupe rule: a
/// transition seen twice never notifies twice). `Ok(None)` is the dedupe
/// case, not an error. `silence` is `None` when the notice interrupts, or
/// the reason it does not.
fn insert_notice(
    conn: &Connection,
    n: NewNotice<'_>,
    silence: Option<&'static str>,
) -> Result<Option<Notification>> {
    let id = crate::auth::identity::random_id("ntf_");
    let created = now();
    let tx = conn.unchecked_transaction()?;
    let seq = next_counter(&tx)?;
    let changed = tx.execute(
        "INSERT INTO notifications
             (id, seq, updated_seq, created, source, kind, session_id, host_id, provider,
              session_title, note_vault_id, note_id, title, body, urgency, key, silent,
              silent_reason)
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(key) DO NOTHING",
        params![
            id,
            seq,
            created,
            n.source,
            n.kind,
            n.session_id,
            n.host_id,
            n.provider,
            n.session_title,
            n.note_vault_id,
            n.note_id,
            n.title,
            n.body,
            n.urgency,
            n.key,
            silence.is_some(),
            silence,
        ],
    )?;
    if changed == 0 {
        tx.commit()?;
        return Ok(None);
    }
    let made = tx.query_row(
        &format!("SELECT {NOTIF_COLUMNS} FROM notifications WHERE id = ?1"),
        params![id],
        row_to_notification,
    )?;
    tx.commit()?;
    Ok(Some(made))
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
    let tx = conn.unchecked_transaction()?;
    let mut changed = Vec::new();
    for id in ids {
        let seq = next_counter(&tx)?;
        let n = tx.execute(
            "UPDATE notifications SET read_at = ?2, read_by_device = ?3, updated_seq = ?4
             WHERE id = ?1 AND read_at IS NULL",
            params![id, at, device, seq],
        )?;
        if n > 0 {
            changed.push(id.clone());
        }
    }
    tx.commit()?;
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

/// Clears the inbox. The counter is untouched: `seq`/`updated_seq` keep
/// climbing, so a stream subscriber's cursor from before the clear still
/// makes sense (nothing to catch up on), and a later `GET ?after=<old>`
/// never mistakes "cleared" for "nothing new".
fn clear_inbox(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM notifications", [])?;
    Ok(())
}

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

/// What a summary counts: only what quiet hours or a pause itself held
/// back, never a kind that is off by default or a muted one — otherwise
/// "while you were away" is noise (item 8).
fn silenced_counts(conn: &Connection, since: &str) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT kind, COUNT(*) FROM notifications
         WHERE created >= ?1 AND silent_reason IN ('quiet', 'paused')
         GROUP BY kind",
    )?;
    let rows = stmt.query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 30 days or 1,000 rows, whichever is fewer (Q21), on account prefs.
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
// nothing is lost, it just never interrupts. `Some(reason)` is silent;
// `None` is not.
// ---------------------------------------------------------------------

pub(super) fn silent_for(
    prefs: &Prefs,
    mutes: &[Mute],
    kind: &str,
    session_id: Option<&str>,
    provider: Option<&str>,
    at: &str,
) -> Option<&'static str> {
    if is_paused(prefs, at) {
        return Some("paused");
    }
    if in_quiet_hours(prefs, at) && !(kind == DELETE_REQUEST && prefs.quiet_allow_delete_requests) {
        return Some("quiet");
    }
    if !prefs.event_on(kind) {
        return Some("off");
    }
    if muted(mutes, session_id, provider, kind, at) {
        return Some("muted");
    }
    None
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
    let silence = silent_for(
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
    // The reason is the agent's own words (prompt-injectable): sanitised,
    // never shown raw (draft, Security and privacy).
    let body = sanitise_agent_text(&request.reason, 200);
    insert_notice(
        conn,
        NewNotice::for_session(
            r,
            DELETE_REQUEST,
            format!("{} wants to delete {what}", display_title(r)),
        )
        .with_body(body)
        .with_key(format!("ses:{}:del:{}", request.session_id, request.id)),
        silence,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------
// 161-2: the `notify` tool. `ops::notify` validates the call (sanitises
// `title`/`body`, checks `key`'s shape, resolves and checks `link`) and
// hands this a ready-made [`AgentNotify`]; everything here is the
// mechanics — limits, the duplicate/`key`-replace rule, `delay_s`, and
// running the result through the same [`silent_for`] precedence an
// automatic notice gets.
// ---------------------------------------------------------------------

/// A validated `notify` call, ready to write.
pub struct AgentNotify {
    pub session_id: String,
    pub provider: String,
    pub session_title: String,
    pub title: String,
    pub body: Option<String>,
    pub urgency: &'static str,
    /// `[A-Za-z0-9._:-]`, 1–64 chars: already checked by the caller.
    pub key: Option<String>,
    pub note_vault_id: Option<String>,
    pub note_id: Option<String>,
    /// `Some(0)` and `None` both mean "now"; `notify`'s `delay_s` is
    /// resolved to an absolute `due` instant by the caller, since only it
    /// knows "now".
    pub due: Option<String>,
}

/// `notify`'s result (the draft's wire shape): `sent` interrupted,
/// `queued` is a delayed notice not yet due, `replaced` took over an
/// earlier notice under the same `key`, `held` means a mute, quiet hours
/// or a pause kept it to the inbox only.
#[derive(Debug, Clone, Serialize)]
pub struct NotifyResult {
    pub id: String,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_reason: Option<&'static str>,
}

/// Why a call is refused outright rather than `held` (never reached for a
/// duplicate or a `key` replace, which skip the limits entirely).
#[derive(Debug, Clone, Copy)]
pub enum NotifyRefusal {
    RateLimited { retry_after_s: i64 },
    TooManyPending,
}

/// `key` scoped to its session: never shared with another session's key,
/// nor with a `storm`/`terminal` notice's own dedupe key.
fn agent_key(session_id: &str, key: &str) -> String {
    format!("ses:{session_id}:agent:{key}")
}

/// `[A-Za-z0-9._:-]`, 1–64 characters (the draft's parameter table).
pub fn valid_notify_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= NOTIFY_KEY_MAX
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// `silent_reason`'s stored word, as `notify`'s `held_reason` (the draft's
/// wire enum uses `quiet_hours`, not the column's `quiet`).
fn held_reason_wire(reason: &str) -> &'static str {
    match reason {
        "quiet" => "quiet_hours",
        "paused" => "paused",
        _ => "muted",
    }
}

/// An uncancelled notice from this session, this call's exact (title,
/// body), created within the duplicate window: the same notice (Q5), to
/// answer again rather than insert. Shared with 161-5's terminal notices
/// (`source IN ('agent', 'terminal')`): the same text forwarded twice,
/// from `notify` or a session's own terminal, is still the same notice.
fn recent_duplicate(
    conn: &Connection,
    session_id: &str,
    title: &str,
    body: Option<&str>,
    since: &str,
) -> Result<Option<Notification>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {NOTIF_COLUMNS} FROM notifications
         WHERE session_id = ?1 AND source IN ('agent', 'terminal') AND cancelled = 0
           AND title = ?2 AND body IS ?3 AND created >= ?4
         ORDER BY seq DESC LIMIT 1"
    ))?;
    Ok(stmt
        .query_row(params![session_id, title, body, since], row_to_notification)
        .optional()?)
}

/// Cancels this session's earlier live notice under `full_key`, freeing
/// the column's `UNIQUE` slot so the replacement can take it (`notify`'s
/// `key` replace). `Ok(true)` if one was live to cancel.
fn cancel_for_replace(conn: &Connection, session_id: &str, full_key: &str) -> Result<bool> {
    let seq = next_counter(conn)?;
    let n = conn.execute(
        "UPDATE notifications SET cancelled = 1, key = NULL, updated_seq = ?3
         WHERE session_id = ?1 AND key = ?2 AND cancelled = 0",
        params![session_id, full_key, seq],
    )?;
    Ok(n > 0)
}

/// Count and earliest `created` of this session's agent and terminal
/// notices in the rolling window (for the hourly cap and its
/// `retry_after_s`) — 161-5 shares `notify`'s limits, so both sources
/// count together.
fn session_window(
    conn: &Connection,
    session_id: &str,
    since: &str,
) -> Result<(i64, Option<String>)> {
    Ok(conn.query_row(
        "SELECT COUNT(*), MIN(created) FROM notifications
         WHERE session_id = ?1 AND source IN ('agent', 'terminal') AND created >= ?2",
        params![session_id, since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// The same, account-wide (every session's agent and terminal notices).
fn account_window(conn: &Connection, since: &str) -> Result<(i64, Option<String>)> {
    Ok(conn.query_row(
        "SELECT COUNT(*), MIN(created) FROM notifications
         WHERE source IN ('agent', 'terminal') AND created >= ?1",
        params![since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Seconds until the oldest notice in a window ages out of it (so the
/// count drops below the cap), at least 1.
fn retry_after(oldest: Option<&str>, at: &str) -> i64 {
    oldest
        .and_then(|o| elapsed_secs(o, at))
        .map(|elapsed| (3600 - elapsed).max(1))
        .unwrap_or(60)
}

/// The per-session token bucket: burst [`AGENT_BURST`], refilling one
/// token every [`AGENT_REFILL_SECS`]. `Ok(None)` consumed one; `Ok(Some(s))`
/// is the seconds until the next.
fn consume_token(conn: &Connection, session_id: &str, at: &str) -> Result<Option<i64>> {
    let now_s = now_epoch(at);
    let row: Option<(f64, String)> = conn
        .query_row(
            "SELECT tokens, updated_at FROM session_notify_bucket WHERE session_id = ?1",
            params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (tokens, last_s) = match row {
        Some((t, u)) => (t, now_epoch(&u)),
        None => (AGENT_BURST, now_s),
    };
    let elapsed = (now_s - last_s).max(0) as f64;
    let refilled = (tokens + elapsed / AGENT_REFILL_SECS).min(AGENT_BURST);
    let (left, retry) = if refilled >= 1.0 {
        (refilled - 1.0, None)
    } else {
        let wait = ((1.0 - refilled) * AGENT_REFILL_SECS).ceil() as i64;
        (refilled, Some(wait.max(1)))
    };
    conn.execute(
        "INSERT INTO session_notify_bucket (session_id, tokens, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET tokens = excluded.tokens, updated_at = excluded.updated_at",
        params![session_id, left, at],
    )?;
    Ok(retry)
}

fn now_epoch(at: &str) -> i64 {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::parse(at, &Rfc3339)
        .map(|t| t.unix_timestamp())
        .unwrap_or(0)
}

/// One roll-up notice per session per hour window (Q5), bumped in place
/// as more of the session's notices are rate-limited: "{session} sent N
/// more notifications". Runs through the same precedence an agent message
/// gets, so a muted or quiet session is not interrupted by its own
/// roll-up either.
fn bump_rollup(
    conn: &Connection,
    session_id: &str,
    provider: &str,
    session_title: &str,
    prefs: &Prefs,
    mutes: &[Mute],
    at: &str,
) -> Result<()> {
    let window = (now_epoch(at) / 3600).to_string();
    let count: i64 = conn.query_row(
        "INSERT INTO session_notify_rollup (session_id, window, count, notice_id)
         VALUES (?1, ?2, 1, '')
         ON CONFLICT(session_id, window) DO UPDATE SET count = count + 1
         RETURNING count",
        params![session_id, window],
        |r| r.get(0),
    )?;
    let title = format!(
        "{session_title} sent {count} more notification{}",
        if count == 1 { "" } else { "s" }
    );
    let key = format!("ses:{session_id}:rollup:{window}");
    if count == 1 {
        let silence = silent_for(
            prefs,
            mutes,
            AGENT_MESSAGE,
            Some(session_id),
            Some(provider),
            at,
        );
        if let Some(made) = insert_notice(
            conn,
            NewNotice {
                source: "storm",
                kind: AGENT_ROLLUP,
                session_id: Some(session_id),
                host_id: None,
                provider: Some(provider),
                session_title: Some(session_title),
                note_vault_id: None,
                note_id: None,
                title,
                body: None,
                urgency: "low",
                key: Some(key),
            },
            silence,
        )? {
            conn.execute(
                "UPDATE session_notify_rollup SET notice_id = ?3 WHERE session_id = ?1 AND window = ?2",
                params![session_id, window, made.id],
            )?;
        }
    } else {
        let seq = next_counter(conn)?;
        conn.execute(
            "UPDATE notifications SET title = ?3, updated_seq = ?4
             WHERE session_id = ?1 AND key = ?2 AND cancelled = 0",
            params![session_id, key, title, seq],
        )?;
    }
    Ok(())
}

impl AgentManager {
    /// The "Notifies you" capability, offered now: a live session whose
    /// flag is on. Checked again here (`ops`'s defensive re-check): the
    /// gateway's `Authorized::may_call` already refused an unoffered call
    /// before this is reached.
    pub fn notify_offered(&self, session_id: &str) -> bool {
        let store = self.store.lock().unwrap();
        store
            .get(session_id)
            .ok()
            .flatten()
            .is_some_and(|r| !r.is_ended())
            && store.notify_enabled(session_id).unwrap_or(false)
    }

    /// The "Notifies you" flag as last set (161-2 F1), for `SessionView`:
    /// unlike [`Self::notify_offered`] this does not require the session to
    /// be live, so an ended session's view still shows what it was.
    pub fn notify_enabled(&self, session_id: &str) -> bool {
        self.store
            .lock()
            .unwrap()
            .notify_enabled(session_id)
            .unwrap_or(false)
    }

    /// A person turns a session's "Notifies you" flag on or off from
    /// Session Details (161-2 F1, AM124): "the chip at launch, and Session
    /// Details afterwards, is the real grant". Bumped like `resize` and
    /// every other per-session write, so the flip reaches every client
    /// watching this session on `/v1/agent/events`, not only the one that
    /// made it.
    pub fn set_notify(
        &self,
        session_id: &str,
        enabled: bool,
        device: Option<&str>,
    ) -> AgentResult<()> {
        self.store
            .lock()
            .unwrap()
            .set_notify(session_id, enabled, device, &now())?;
        self.bump(session_id);
        Ok(())
    }

    /// `cancel_notify(key)`: a session cancels only its own key, pending or
    /// already live. `Ok(true)` if anything was cancelled.
    pub fn cancel_agent_notify(&self, session_id: &str, key: &str) -> AgentResult<bool> {
        let store = self.store.lock().unwrap();
        let removed_pending = delete_pending_by_key(&store.conn, session_id, key)?;
        let full_key = agent_key(session_id, key);
        let cancelled_live = cancel_for_replace(&store.conn, session_id, &full_key)?;
        drop(store);
        if cancelled_live {
            self.poke_notify();
        }
        Ok(removed_pending || cancelled_live)
    }

    /// `notify`: everything `ops::notify` has already validated, through
    /// the duplicate rule, the `key` replace, the limits, and — immediately
    /// or (`due` set) queued for the Notifier to fire — [`silent_for`].
    pub fn agent_notify(
        &self,
        req: AgentNotify,
    ) -> AgentResult<std::result::Result<NotifyResult, NotifyRefusal>> {
        let at = now();
        let store = self.store.lock().unwrap();
        let conn = &store.conn;

        // An identical (title, body) from this session within the window
        // is the same notice: answer again, consuming no limit.
        let dup_since = crate::index::rfc3339_minus_secs(&at, AGENT_DUP_WINDOW_SECS);
        if let Some(existing) = recent_duplicate(
            conn,
            &req.session_id,
            &req.title,
            req.body.as_deref(),
            &dup_since,
        )? {
            return Ok(Ok(NotifyResult {
                id: existing.id,
                state: if existing.silent { "held" } else { "sent" },
                held_reason: existing.silent_reason.as_deref().map(held_reason_wire),
            }));
        }

        // `delay_s`: queued, under its own pending cap, counted against the
        // same limits as an immediate call (no bypass by delaying).
        if let Some(due) = &req.due {
            let pending = pending_count(conn, &req.session_id)?;
            if pending >= AGENT_MAX_PENDING {
                return Ok(Err(NotifyRefusal::TooManyPending));
            }
            if let Some(refusal) = self.check_agent_limits(
                conn,
                &req.session_id,
                &req.provider,
                &req.session_title,
                &at,
            )? {
                return Ok(Err(refusal));
            }
            let id = crate::auth::identity::random_id("ntf_");
            insert_pending(conn, &id, &req, due, &at)?;
            return Ok(Ok(NotifyResult {
                id,
                state: "queued",
                held_reason: None,
            }));
        }

        if let Some(refusal) = self.check_agent_limits(
            conn,
            &req.session_id,
            &req.provider,
            &req.session_title,
            &at,
        )? {
            return Ok(Err(refusal));
        }
        Ok(Ok(self.write_agent_notice(conn, &req, &at)?))
    }

    /// The limits (Q5), without writing anything: `None` passes. Shared by
    /// `notify` and 161-5's terminal notices — the window queries below
    /// count `source IN ('agent', 'terminal')`, and so does the token
    /// bucket, which is keyed by `session_id` alone.
    fn check_agent_limits(
        &self,
        conn: &Connection,
        session_id: &str,
        provider: &str,
        session_title: &str,
        at: &str,
    ) -> AgentResult<Option<NotifyRefusal>> {
        let prefs = load_prefs(conn)?;
        let hour_ago = crate::index::rfc3339_minus_secs(at, 3600);
        let (session_count, session_oldest) = session_window(conn, session_id, &hour_ago)?;
        let session_cap = prefs.per_session_limit.max(0);
        if session_count >= session_cap {
            self.bump_and_poke_rollup(conn, session_id, provider, session_title, at)?;
            return Ok(Some(NotifyRefusal::RateLimited {
                retry_after_s: retry_after(session_oldest.as_deref(), at),
            }));
        }
        let (account_count, account_oldest) = account_window(conn, &hour_ago)?;
        if account_count >= AGENT_ACCOUNT_HOURLY {
            self.bump_and_poke_rollup(conn, session_id, provider, session_title, at)?;
            return Ok(Some(NotifyRefusal::RateLimited {
                retry_after_s: retry_after(account_oldest.as_deref(), at),
            }));
        }
        if let Some(wait) = consume_token(conn, session_id, at)? {
            self.bump_and_poke_rollup(conn, session_id, provider, session_title, at)?;
            return Ok(Some(NotifyRefusal::RateLimited {
                retry_after_s: wait,
            }));
        }
        Ok(None)
    }

    fn bump_and_poke_rollup(
        &self,
        conn: &Connection,
        session_id: &str,
        provider: &str,
        session_title: &str,
        at: &str,
    ) -> AgentResult<()> {
        let mutes = list_mutes(conn)?;
        let prefs = load_prefs(conn)?;
        bump_rollup(
            conn,
            session_id,
            provider,
            session_title,
            &prefs,
            &mutes,
            at,
        )?;
        self.poke_notify();
        Ok(())
    }

    /// Writes the notice now: a `key` replaces this session's earlier live
    /// one under it (`replaced`); otherwise a plain insert (`sent`/`held`).
    fn write_agent_notice(
        &self,
        conn: &Connection,
        req: &AgentNotify,
        at: &str,
    ) -> AgentResult<NotifyResult> {
        let prefs = load_prefs(conn)?;
        let mutes = list_mutes(conn)?;
        let silence = silent_for(
            &prefs,
            &mutes,
            AGENT_MESSAGE,
            Some(&req.session_id),
            Some(&req.provider),
            at,
        );
        let (full_key, replaced) = match &req.key {
            Some(k) => {
                let full = agent_key(&req.session_id, k);
                let replaced = cancel_for_replace(conn, &req.session_id, &full)?;
                (Some(full), replaced)
            }
            None => (None, false),
        };
        let made = insert_notice(
            conn,
            NewNotice {
                source: "agent",
                kind: AGENT_MESSAGE,
                session_id: Some(&req.session_id),
                host_id: None,
                provider: Some(&req.provider),
                session_title: Some(&req.session_title),
                note_vault_id: req.note_vault_id.as_deref(),
                note_id: req.note_id.as_deref(),
                title: req.title.clone(),
                body: req.body.clone(),
                urgency: req.urgency,
                key: full_key,
            },
            silence,
        )?
        .expect("a fresh or freed key is never a dedupe hit");
        self.poke_notify();
        Ok(NotifyResult {
            id: made.id,
            state: if replaced {
                "replaced"
            } else if made.silent {
                "held"
            } else {
                "sent"
            },
            held_reason: made.silent_reason.as_deref().map(held_reason_wire),
        })
    }

    /// The Notifier fires a delayed notice when it is due, under the
    /// prefs/mutes/quiet/pause in force *then*, not when `notify` queued
    /// it. Returns whether anything changed (for `notifier_tick`'s poke).
    ///
    /// A notice whose session has since ended or been removed, or no
    /// longer has the "Notifies you" capability, is dropped rather than
    /// fired (161-2 F2): ended and removed sessions are also cleaned up
    /// eagerly (`forget_due_at_end`, `forget_session_notify`), so this is
    /// the defensive case — a flag turned off after `notify` queued the
    /// delay, or a race between the two.
    pub(super) fn fire_due_notifies(&self, at: &str) -> AgentResult<bool> {
        let due = {
            let store = self.store.lock().unwrap();
            due_notifies(&store.conn, at)?
        };
        if due.is_empty() {
            return Ok(false);
        }
        let store = self.store.lock().unwrap();
        for (pending_id, req) in &due {
            let offered = store.get(&req.session_id)?.is_some_and(|r| !r.is_ended())
                && store.notify_enabled(&req.session_id)?;
            if offered {
                self.write_agent_notice(&store.conn, req, at)?;
            }
            delete_pending(&store.conn, pending_id)?;
        }
        Ok(true)
    }
}

// ---------------------------------------------------------------------
// 161-5: terminal notices (OSC 9/777/99; AM126), forwarded by 161-R1's host
// parser as `terminal_notice` on a session's status report. They run
// through the same sanitiser, limits and precedence `notify` gets —
// `check_agent_limits`, `recent_duplicate`, `silent_for` under
// [`AGENT_MESSAGE`] — and nothing here forks them.
// ---------------------------------------------------------------------

/// A terminal notice's outcome, metadata only (G-D18): for
/// `ops::runtime_status` to audit. Never the title or body.
pub struct TerminalNoticeAudit {
    pub owner_user_id: String,
    pub session_id: String,
    pub source_form: String,
    /// Sanitised char counts, like `notify`'s own `title_chars`/`body_chars`
    /// — never the raw byte length of the agent's untrusted text.
    pub title_chars: usize,
    pub body_chars: usize,
    pub outcome: &'static str,
}

/// `title`/`body` after decision 161's sanitiser (point 3 of 161-5's build
/// note): the OSC title if it survives sanitising, else the body cut to
/// [`NOTIFY_TITLE_MAX`] chars with no separate body (the title already
/// carries the only text there is). `None` once nothing is left of either.
fn sanitise_terminal_item(item: &TerminalNoticeItem) -> Option<(String, Option<String>)> {
    let body = sanitise_agent_text(&item.body, NOTIFY_BODY_MAX);
    match item
        .title
        .as_deref()
        .and_then(|t| sanitise_agent_text(t, NOTIFY_TITLE_MAX))
    {
        Some(title) => Some((title, body)),
        None => body.map(|b| (b.chars().take(NOTIFY_TITLE_MAX).collect(), None)),
    }
}

/// A short, deterministic signature of a terminal notice's sanitised text,
/// for its dedupe `key` (`ses:<id>:osc:<signature>`). The same text from
/// the same session collapses into the live (uncancelled) notice under
/// that key rather than filling the inbox with repeats, for as long as
/// that row exists — being read does not free it; only its being
/// cancelled, pruned or the inbox cleared does.
fn terminal_notice_signature(title: &str, body: Option<&str>) -> String {
    let mut h = blake3::Hasher::new();
    h.update(title.as_bytes());
    h.update(&[0]);
    h.update(body.unwrap_or("").as_bytes());
    h.finalize().to_hex()[..16].to_string()
}

/// Whether an uncancelled notice already holds `key`, regardless of how
/// long ago it was made. [`recent_duplicate`]'s 10-minute window only
/// catches a resend bunched close together; identical text spread further
/// apart still collides on `key`'s `UNIQUE` column, and checking this
/// before the limits is what makes that answer "duplicate" rather than
/// spend a token and then report "sent" on a row `insert_notice`'s
/// `ON CONFLICT(key) DO NOTHING` never actually writes.
fn live_notice_by_key(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM notifications WHERE key = ?1 AND cancelled = 0",
            params![key],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

impl AgentManager {
    /// Whether `seq` is new for `session_id` under its host's current link
    /// generation — `false` for a status post retried with notices the
    /// server already acted on. Never kept across a server restart (like
    /// every other field of `SessionLive`); a host restart or reconnect
    /// gets a new generation (`connect_host`), under which its own `seq`
    /// starts at 1 again, so a lower value is accepted.
    fn accept_terminal_seq(&self, host_id: &str, session_id: &str, seq: u64) -> bool {
        let mut live = self.live.lock().unwrap();
        let generation = live.hosts.get(host_id).map(|l| l.generation).unwrap_or(0);
        let s = live
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(SessionLive::new);
        match s.osc_seq {
            Some((g, last)) if g == generation && seq <= last => false,
            _ => {
                s.osc_seq = Some((generation, seq));
                true
            }
        }
    }

    /// 161-5: a host's `terminal_notice`/`terminal_notice_dropped` from its
    /// status report. Dropped silently — no error to the host — for a
    /// session without the "Notifies you" capability (`notify_enabled`);
    /// never gated by whether the session has since ended, since the very
    /// report that ends a session still carries a notice queued just before
    /// it did (161-R1 fix 3). Returns one audit row per item, for
    /// `ops::runtime_status` to record.
    pub(super) fn record_terminal_notices(
        &self,
        r: &SessionRecord,
        items: &[TerminalNoticeItem],
        host_dropped: u32,
    ) -> AgentResult<Vec<TerminalNoticeAudit>> {
        let mut audits = Vec::new();
        if items.is_empty() && host_dropped == 0 {
            return Ok(audits);
        }
        let enabled = self
            .store
            .lock()
            .unwrap()
            .notify_enabled(&r.id)
            .unwrap_or(false);
        if !enabled {
            tracing::debug!(session = %r.id, "terminal notices dropped: no capability");
            return Ok(audits);
        }
        let store = self.store.lock().unwrap();
        let conn = &store.conn;
        let prefs = load_prefs(conn)?;
        let mutes = list_mutes(conn)?;
        let at = now();
        let session_title = display_title(r);
        let dup_since = crate::index::rfc3339_minus_secs(&at, AGENT_DUP_WINDOW_SECS);
        for item in items {
            let Some((title, body)) = sanitise_terminal_item(item) else {
                continue;
            };
            if !self.accept_terminal_seq(&r.host_id, &r.id, item.seq) {
                continue;
            }
            let title_chars = title.chars().count();
            let body_chars = body.as_ref().map_or(0, |b| b.chars().count());
            let key = format!(
                "ses:{}:osc:{}",
                r.id,
                terminal_notice_signature(&title, body.as_deref())
            );
            // A key collision past the 10-minute window is still the same
            // notice (S1): checked before the limits, or a stale duplicate
            // would spend a token on a row `insert_notice` never writes.
            let outcome = if recent_duplicate(conn, &r.id, &title, body.as_deref(), &dup_since)?
                .is_some()
                || live_notice_by_key(conn, &key)?
            {
                "duplicate"
            } else if let Some(_refusal) =
                self.check_agent_limits(conn, &r.id, &r.provider, &session_title, &at)?
            {
                "rate_limited"
            } else {
                let silence = silent_for(
                    &prefs,
                    &mutes,
                    AGENT_MESSAGE,
                    Some(&r.id),
                    Some(&r.provider),
                    &at,
                );
                insert_notice(
                    conn,
                    NewNotice {
                        source: "terminal",
                        kind: TERMINAL_NOTICE,
                        session_id: Some(&r.id),
                        host_id: None,
                        provider: Some(&r.provider),
                        session_title: Some(&session_title),
                        note_vault_id: None,
                        note_id: None,
                        title,
                        body,
                        urgency: "normal",
                        key: Some(key),
                    },
                    silence,
                )?;
                if silence.is_some() { "held" } else { "sent" }
            };
            audits.push(TerminalNoticeAudit {
                owner_user_id: r.owner_user_id.clone(),
                session_id: r.id.clone(),
                source_form: item.source_form.clone(),
                title_chars,
                body_chars,
                outcome,
            });
        }
        for _ in 0..host_dropped {
            bump_rollup(
                conn,
                &r.id,
                &r.provider,
                &session_title,
                &prefs,
                &mutes,
                &at,
            )?;
        }
        self.poke_notify();
        Ok(audits)
    }
}

/// `notify_pending` rows whose `due` has passed, as ready-made
/// [`AgentNotify`]s (the Notifier fires them under the prefs in force now).
fn due_notifies(conn: &Connection, at: &str) -> Result<Vec<(String, AgentNotify)>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, provider, session_title, note_vault_id, note_id,
                title, body, urgency, key
         FROM notify_pending WHERE due <= ?1",
    )?;
    let rows = stmt.query_map(params![at], |r| {
        Ok((
            r.get::<_, String>(0)?,
            AgentNotify {
                session_id: r.get(1)?,
                provider: r.get(2)?,
                session_title: r.get(3)?,
                note_vault_id: r.get(4)?,
                note_id: r.get(5)?,
                title: r.get(6)?,
                body: r.get(7)?,
                urgency: match r.get::<_, String>(8)?.as_str() {
                    "high" => "high",
                    "low" => "low",
                    _ => "normal",
                },
                key: r.get(9)?,
                due: None,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn delete_pending(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM notify_pending WHERE id = ?1", params![id])?;
    Ok(())
}

/// `cancel_notify`: a still-queued delayed notice under this key, by this
/// session. `Ok(true)` if one was removed.
fn delete_pending_by_key(conn: &Connection, session_id: &str, key: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM notify_pending WHERE session_id = ?1 AND key = ?2",
        params![session_id, key],
    )? > 0)
}

fn pending_count(conn: &Connection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM notify_pending WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )?)
}

fn insert_pending(
    conn: &Connection,
    id: &str,
    req: &AgentNotify,
    due: &str,
    created_at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO notify_pending
             (id, session_id, provider, session_title, note_vault_id, note_id,
              title, body, urgency, key, due, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            req.session_id,
            req.provider,
            req.session_title,
            req.note_vault_id,
            req.note_id,
            req.title,
            req.body,
            req.urgency,
            req.key,
            due,
            created_at,
        ],
    )?;
    Ok(())
}

/// The Notifier's memory between ticks: nothing in here survives a
/// restart, which is why nothing is raised on one (the same rule the
/// client's toasts follow for a snapshot) — except for a session the
/// Notifier never had a chance to see an earlier state for.
pub(super) struct NotifierMemory {
    prev: HashMap<String, Snap>,
    /// The link generation an offline host was last notified for, so a
    /// reconnect and a fresh outage are told apart.
    host_offline_notified: HashMap<String, u64>,
    last_child_event_rowid: i64,
    /// When the current quiet-hours-or-paused spell began, if one is open.
    quiet_since: Option<String>,
    /// When this Notifier started: a session created at or after this
    /// instant has no earlier state to have missed, so its first observed
    /// state is still notified (item 4).
    started_at: String,
}

/// One `/v1/agent/events` connection's live state (161-4), keyed by
/// `(device_id, connection_id)` — connection id alone so a device's own
/// two tabs, or an old connection overlapping its replacement during a
/// reconnect, never let one clear the other's presence; the device id
/// too so another device can never collide a connection id into
/// overwriting this one's entry (review nit). A device counts as active
/// if *any* of its connections does. Never persisted: a restart forgets
/// it the same way it forgets which sessions are open on screen.
#[derive(Debug, Clone)]
pub(super) struct Presence {
    pub active: bool,
    pub open_session: Option<String>,
}

/// Caches [`AgentManager::deliver_hint_for`]'s answer per notice id,
/// computed once at the first subscriber's fan-out and reused by every
/// subsequent one (review item 4): computing it fresh per subscriber let
/// a presence change land between two subscribers' scans and give the
/// same notice two different verdicts — delivered twice, or never.
/// Bounded FIFO: an entry this old has long since been read or
/// cancelled, so losing it costs nothing.
#[derive(Default)]
pub(super) struct DeliverCache {
    hints: HashMap<String, Option<String>>,
    order: std::collections::VecDeque<String>,
}

const DELIVER_CACHE_CAP: usize = 4096;

impl NotifierMemory {
    pub(super) fn new(started_at: String) -> Self {
        Self {
            prev: HashMap::new(),
            host_offline_notified: HashMap::new(),
            last_child_event_rowid: 0,
            quiet_since: None,
            started_at,
        }
    }
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
    /// stored one, or this is a conflict (Q15) rather than a silent clobber,
    /// and the result must itself be a valid row (item 5).
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
            SaveOutcome::Saved(p) => {
                self.poke_notify();
                Ok(p)
            }
            SaveOutcome::Conflict => Err(AgentError::Conflict("prefs_version".into())),
            SaveOutcome::Invalid(msg) => Err(AgentError::BadRequest(msg)),
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
        let store = self.store.lock().unwrap();
        clear_inbox(&store.conn)?;
        bump_cleared_version(&store.conn)?;
        drop(store);
        self.poke_notify();
        Ok(())
    }

    pub fn notification_mutes(&self) -> AgentResult<Vec<Mute>> {
        Ok(list_mutes(&self.store.lock().unwrap().conn)?)
    }

    pub fn notifications_mutes_version(&self) -> AgentResult<i64> {
        Ok(mutes_version(&self.store.lock().unwrap().conn)?)
    }

    pub fn notifications_cleared_version(&self) -> AgentResult<i64> {
        Ok(cleared_version(&self.store.lock().unwrap().conn)?)
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
        if target.is_empty() {
            return Err(AgentError::BadRequest("target must not be empty".into()));
        }
        let at = now();
        let until = match until {
            Some(u) => Some(parse_future_utc(u, &at).map_err(AgentError::BadRequest)?),
            None => None,
        };
        let store = self.store.lock().unwrap();
        if !known_mute_target(&store.conn, scope, target)? {
            return Err(AgentError::BadRequest(format!(
                "no {scope} \"{target}\" to mute"
            )));
        }
        let m = create_mute(&store.conn, scope, target, until.as_deref(), device)?;
        bump_mutes_version(&store.conn)?;
        drop(store);
        self.poke_notify();
        Ok(m)
    }

    pub fn unmute_notifications(&self, id: &str) -> AgentResult<bool> {
        let store = self.store.lock().unwrap();
        let removed = delete_mute(&store.conn, id)?;
        if removed {
            bump_mutes_version(&store.conn)?;
        }
        drop(store);
        if removed {
            self.poke_notify();
        }
        Ok(removed)
    }

    /// 1 hour, until tomorrow 08:00, or indefinitely (`until: None`):
    /// a given `until` must be a real, future RFC3339 time.
    pub fn pause_notifications(&self, until: Option<&str>) -> AgentResult<Prefs> {
        let at = now();
        let resolved = match until {
            Some(u) => parse_future_utc(u, &at).map_err(AgentError::BadRequest)?,
            None => PAUSE_INDEFINITELY.to_string(),
        };
        set_paused_until(&self.store.lock().unwrap().conn, Some(&resolved))?;
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
                    provider: None,
                    session_title: None,
                    note_vault_id: None,
                    note_id: None,
                    title: "Test notification".into(),
                    body: Some("If this reached you, delivery to this device works.".into()),
                    urgency: "normal",
                    key: None,
                },
                None,
            )?
            .expect("no key: never deduped")
        };
        self.poke_notify();
        Ok(n)
    }

    // ---- Presence and the deliver hint (161-4) --------------------------

    /// Registers a brand new `/v1/agent/events` connection's presence,
    /// keyed by `(device_id, connection_id)` (the latter client-generated,
    /// unique per connection — never per device, so two tabs or a
    /// reconnect's overlap each keep their own entry). Always succeeds:
    /// connecting is how a connection id starts existing.
    pub fn set_presence(
        &self,
        connection_id: &str,
        device_id: &str,
        active: bool,
        open_session: Option<String>,
    ) {
        self.presence.lock().unwrap().insert(
            (device_id.to_string(), connection_id.to_string()),
            Presence {
                active,
                open_session,
            },
        );
    }

    /// `POST …/presence`: updates an *existing* connection of the calling
    /// device. `false` (404 to the caller) if that connection is not
    /// live — it may have just disconnected, and a stale POST must never
    /// resurrect a presence row for a connection that is really gone.
    pub fn update_presence(
        &self,
        connection_id: &str,
        device_id: &str,
        active: bool,
        open_session: Option<String>,
    ) -> bool {
        let mut presence = self.presence.lock().unwrap();
        match presence.get_mut(&(device_id.to_string(), connection_id.to_string())) {
            Some(p) => {
                p.active = active;
                p.open_session = open_session;
                true
            }
            None => false,
        }
    }

    /// Forgets one connection's presence — called once it ends, so a
    /// closed app is never mistaken for a quiet one. Never touches any
    /// other connection, including another of the same device's.
    pub fn clear_presence(&self, connection_id: &str, device_id: &str) {
        self.presence
            .lock()
            .unwrap()
            .remove(&(device_id.to_string(), connection_id.to_string()));
    }

    /// A device was just revoked (review item D): every connection it had
    /// open stops counting toward any deliver hint, immediately — not
    /// only once each one's own stream notices the socket is gone.
    pub fn clear_presence_for_device(&self, device_id: &str) {
        self.presence
            .lock()
            .unwrap()
            .retain(|(d, _), _| d != device_id);
    }

    /// The architecture's delivery order, for one notice about
    /// `session_id` (`None` for a notice with no session):
    ///
    /// 1. A device active and showing that very session: `None` — nothing,
    ///    as today's in-app toast already covers it.
    /// 2. Any other device active: `toast:<device_id>`, that device alone.
    /// 3. Otherwise: `os`, every background desktop.
    ///
    /// A device counts as active if *any* of its connections does. With
    /// more than one device active and none showing the session, the
    /// lowest device id by sort order is picked — the matrix does not say
    /// which wins, and ties are rare enough that determinism matters more
    /// than the choice.
    fn compute_deliver_hint(&self, session_id: Option<&str>) -> Option<String> {
        let presence = self.presence.lock().unwrap();
        let mut active_devices: Vec<&str> = Vec::new();
        for ((device_id, _), p) in presence.iter() {
            if !p.active {
                continue;
            }
            if session_id.is_some() && p.open_session.as_deref() == session_id {
                return None;
            }
            active_devices.push(device_id);
        }
        match active_devices.into_iter().min() {
            Some(device_id) => Some(format!("toast:{device_id}")),
            None => Some("os".to_string()),
        }
    }

    /// [`compute_deliver_hint`], computed once per `notice_id` and cached
    /// for every later caller (review item 4) — see [`DeliverCache`].
    pub fn deliver_hint_for(&self, notice_id: &str, session_id: Option<&str>) -> Option<String> {
        let mut cache = self.deliver_cache.lock().unwrap();
        if let Some(hint) = cache.hints.get(notice_id) {
            return hint.clone();
        }
        let hint = self.compute_deliver_hint(session_id);
        cache.hints.insert(notice_id.to_string(), hint.clone());
        cache.order.push_back(notice_id.to_string());
        if cache.order.len() > DELIVER_CACHE_CAP
            && let Some(oldest) = cache.order.pop_front()
        {
            cache.hints.remove(&oldest);
        }
        hint
    }

    /// Bypasses [`DeliverCache`] — a test polling for presence to clear
    /// must see it the moment it does, not whatever an earlier poll's
    /// now-stale cache entry said.
    #[cfg(test)]
    pub fn deliver_hint_uncached(&self, session_id: Option<&str>) -> Option<String> {
        self.compute_deliver_hint(session_id)
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
    /// its own memory of it (`notice_of`, plus usage limit and a lifecycle
    /// `failed`), checks hosts offline and new child events, sends the one
    /// quiet-hours/pause summary, and runs the retention sweep. Run every
    /// few seconds (`lib.rs`); nothing here is raised for a session this
    /// server has not itself watched change — unless the session did not
    /// exist yet when this Notifier started, which is what keeps a restart
    /// quiet without also silencing a session's very first turn.
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
            let started_at = mem.started_at.clone();
            for r in &sessions {
                let snap = self.snap_of(r)?;
                let before = match mem.prev.insert(r.id.clone(), snap.clone()) {
                    Some(b) => b,
                    None if r.created_at >= started_at => Snap {
                        status: "starting".into(),
                        ..Snap::default()
                    },
                    None => continue, // pre-existing: nothing on a snapshot or a restart
                };
                if let Some(n) = notice_of(&before, &snap) {
                    wrote |= self.emit_transition(&prefs, &mutes, r, n, (&before, &snap), &at)?;
                }
                if before.status != "failed" && snap.status == "failed" {
                    let silence =
                        silent_for(&prefs, &mutes, ERROR, Some(&r.id), Some(&r.provider), &at);
                    let store = self.store.lock().unwrap();
                    wrote |= insert_notice(
                        &store.conn,
                        NewNotice::for_session(r, ERROR, format!("{} failed", display_title(r)))
                            .with_key(format!("ses:{}:failed", r.id)),
                        silence,
                    )?
                    .is_some();
                }
                if before.attention_kind != Some(super::attention::USAGE_LIMIT)
                    && snap.attention_kind == Some(super::attention::USAGE_LIMIT)
                {
                    let silence = silent_for(
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
                        NewNotice::for_session(
                            r,
                            USAGE_LIMIT,
                            format!("{} hit its usage limit", display_title(r)),
                        )
                        .with_key(format!(
                            "ses:{}:limit:{}",
                            r.id,
                            snap.attention_since.clone().unwrap_or_else(|| at.clone())
                        )),
                        silence,
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
        wrote |= self.fire_due_notifies(&at)?;
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
        let (kind, title): (&'static str, String) = match n {
            Notice::NeedsYou => (NEEDS_YOU, format!("{} needs you", display_title(r))),
            Notice::FinishedTurn => {
                let started = before.program.as_ref().map(|p| p.since.as_str());
                let worked = started
                    .and_then(|s| elapsed_secs(s, at))
                    .unwrap_or(i64::MAX);
                if worked < prefs.threshold_s(FINISHED) {
                    return Ok(false);
                }
                (FINISHED, format!("{} finished its turn", display_title(r)))
            }
            Notice::HitError => (ERROR, format!("{} hit an error", display_title(r))),
            Notice::StoppedWorking => (
                STOPPED_WORKING,
                format!("{} stopped working", display_title(r)),
            ),
        };
        let key = notice_key(n, &r.id, after, at);
        let silence = silent_for(prefs, mutes, kind, Some(&r.id), Some(&r.provider), at);
        let store = self.store.lock().unwrap();
        Ok(insert_notice(
            &store.conn,
            NewNotice::for_session(r, kind, title).with_key(key),
            silence,
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
            let silence = silent_for(prefs, mutes, HOST_OFFLINE, None, None, at);
            let store = self.store.lock().unwrap();
            wrote |= insert_notice(
                &store.conn,
                NewNotice {
                    source: "storm",
                    kind: HOST_OFFLINE,
                    session_id: None,
                    host_id: Some(&host_id),
                    provider: None,
                    session_title: None,
                    note_vault_id: None,
                    note_id: None,
                    title: "A host went offline".into(),
                    body: None,
                    urgency: default_urgency(HOST_OFFLINE),
                    key: Some(format!("host:{host_id}:offline:{generation}")),
                },
                silence,
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
            let silence = silent_for(
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
                NewNotice::for_session(&r, CHILD_FINISHED, format!("{} {verb}", display_title(&r)))
                    .with_key(format!("ses:{child}:child:{rowid}")),
                silence,
            )?
            .is_some();
        }
        Ok(wrote)
    }

    /// One summary when quiet hours or a pause ends (Q16), counting what
    /// was held silent *by* quiet hours or the pause while it was active
    /// (never a kind that is off by default, nor a muted one). Keyed by
    /// when the spell began, not the calendar date, so a spell that crosses
    /// midnight (or two in one day) each get their own summary.
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
        Ok(insert_notice(
            &store.conn,
            NewNotice {
                source: "storm",
                kind: SUMMARY,
                session_id: None,
                host_id: None,
                provider: None,
                session_title: None,
                note_vault_id: None,
                note_id: None,
                title: "While you were away".into(),
                body: Some(body),
                urgency: "normal",
                key: Some(format!("summary:{since}")),
            },
            None,
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
