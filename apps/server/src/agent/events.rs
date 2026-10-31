//! Decision 144: child events. One durable queue per parent in `agent.db`
//! (144a), each event carrying its evidence (144b), and the `wait_agent`
//! that reads it. An event is metadata: a child's own text (its result,
//! its messages) is never part of one (144d).
//!
//! The queue is the inbox (144c). No provider has a gated `wake` yet, so a
//! parent learns of an event only by calling a tool, and nothing here, or
//! anywhere an event reaches, writes to a terminal (C9).

use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::attention::{BLOCKED, PERMISSION_PROMPT, USAGE_LIMIT};
use super::scope::MAX_WAIT_IDS;
use super::store::{SessionRecord, Store};
use super::{AgentError, AgentManager, AgentResult, now};

/// A child idle this long without having reported gets `idle_timeout`,
/// once per idle spell.
pub const IDLE_EVENT_AFTER: Duration = Duration::from_secs(600);

/// Past this many events for one child, only the terminal kinds are written.
pub const MAX_EVENTS_PER_CHILD: i64 = 64;

/// The most events one `wait_agent` returns; `cursor` says where it stopped.
pub const MAX_EVENTS_PER_WAIT: usize = 100;

/// How many of a child's latest events a person's view of it carries.
pub const RECENT_EVENTS: usize = 5;

pub const STARTED: &str = "started";
pub const NEEDS_ATTENTION: &str = "needs_attention";
pub const IDLE_TIMEOUT: &str = "idle_timeout";
pub const USAGE_LIMIT_EVENT: &str = "usage_limit";
pub const FAILED: &str = "failed";
pub const REPORTED: &str = "reported";
pub const ENDED: &str = "ended";
/// 145e: the child's round is done and its process was stopped; its
/// conversation and workspace are kept, and a message resumes it.
pub const PARKED: &str = "parked";
/// The child's host link was lost, or the server restarted: nothing is
/// known of it until its host reports again (77c).
pub const UNKNOWN: &str = "unknown";
/// 148b: a tool call the child's own permissions refused. The one kind that
/// carries child-originated text, `tool` and a bounded `detail`, as data.
pub const DENIED: &str = "denied";

/// The longest `detail` a denial keeps, in characters.
pub const MAX_DENIAL_DETAIL: usize = 500;
/// The longest tool name a denial keeps.
pub const MAX_DENIAL_TOOL: usize = 128;

/// The kinds still written past [`MAX_EVENTS_PER_CHILD`].
pub const KEPT_PAST_CAP: &[&str] = &[REPORTED, FAILED, ENDED, PARKED];

/// The kinds that close a round (145f): they carry the round, how long it
/// took by Storm's own clock, and the agent's own usage where it said.
pub const ROUND_KINDS: &[&str] = &[REPORTED, FAILED, ENDED, PARKED];

pub const HOST_PROCESS: &str = "host_process";
pub const STORM: &str = "storm";
pub const PROVIDER_HOOK: &str = "provider_hook";
pub const OUTPUT_PATTERN: &str = "output_pattern";

/// What a source's word is worth (144b).
pub fn confidence(source: &str) -> &'static str {
    match source {
        PROVIDER_HOOK => "reported",
        OUTPUT_PATTERN => "heuristic",
        _ => "certain",
    }
}

/// A provider-native way to tell an idle parent that events wait (144c):
/// the mechanism its vendor documents, never terminal input. A provider
/// joins only once its gate G-144-W-<provider> passes on the real CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatedWake {
    pub provider: &'static str,
    pub mechanism: &'static str,
}

/// No provider's wake has passed its gate. The host keeps the same list
/// (`harness::WAKE`), spelled the same: a test holds the two equal.
pub const WAKE: &[GatedWake] = &[];

/// The agents whose children's denials are taken (148b), once G-148 passes
/// for them; it passed locally on 2026-10-30 (Claude Code 2.1.312, OpenCode
/// 1.19.4). The host keeps the same list (`harness::DENIALS`).
pub const DENIALS: &[&str] = &["claude-code", "opencode"];

#[cfg(test)]
pub fn wake_on(provider: &str) -> bool {
    WAKE.iter().any(|w| w.provider == provider)
}

/// What an agent said a round cost (145f), as its own hook or plugin
/// reported it. Tokens only, each absent when the agent did not say.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

impl Usage {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// What a host reports when a child's tool call is denied (148b).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DeniedReport {
    pub tool: String,
    #[serde(default)]
    pub detail: String,
    /// `provider_hook`, the only source there is.
    #[serde(default)]
    pub source: Option<String>,
}

/// A child's state after the change an event records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventState {
    pub status: String,
    pub activity: Option<String>,
    pub needs_attention: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChildEvent {
    /// `evt_<parent>_<seq>`: stable across reads and restarts.
    pub id: String,
    pub seq: i64,
    pub child: String,
    pub kind: String,
    /// A reason code: an end or failure's `end_reason`, a report's outcome,
    /// a request's kind, an attention kind. Never text.
    pub reason: Option<String>,
    pub state: EventState,
    pub source: String,
    pub confidence: String,
    pub at: String,
    /// For a round's `reported`, `failed`, `ended` or `parked` (145f): the
    /// round, and how long it ran by Storm's own timestamps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// Only where the agent itself reported it; never guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// A `denied` event's tool, the command or path it was refused (the
    /// child's own text, bounded, as data), and how many identical denials
    /// of its round it stands for (148b).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
}

/// What the host last said of a session, already through its gates: the
/// half of a child's state that is not in `agent.db`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveSignals {
    /// `working` or `idle`, only from a gated idle signal.
    pub activity: Option<String>,
    /// `permission_prompt` or `usage_limit`, only from a gated signal.
    pub attention: Option<String>,
}

/// An event to write beside a status change, in its transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extra {
    pub kind: &'static str,
    pub reason: Option<String>,
    pub source: &'static str,
}

/// How a status change is recorded: by whom, with the live half of the
/// state, and any events that ride with it.
#[derive(Debug, Clone)]
pub struct EventCtx {
    pub source: &'static str,
    pub live: LiveSignals,
    pub extra: Vec<Extra>,
}

impl EventCtx {
    #[cfg(test)]
    pub fn storm() -> Self {
        Self::of(STORM, LiveSignals::default())
    }

    pub fn of(source: &'static str, live: LiveSignals) -> Self {
        Self {
            source,
            live,
            extra: Vec::new(),
        }
    }
}

/// 144b's activity: what a gated idle signal said while the session runs,
/// else `unknown`. Never `working` from silence.
pub fn health_activity(r: &SessionRecord, live: &LiveSignals) -> Option<String> {
    if r.is_ended() {
        return None;
    }
    match r.status.as_str() {
        "running" => Some(live.activity.clone().unwrap_or_else(|| UNKNOWN.into())),
        "unknown" => Some(UNKNOWN.into()),
        _ => None,
    }
}

/// The event a status change makes, if any: entering `running`, entering
/// `unknown`, or an end.
pub fn status_event(
    before: Option<&str>,
    r: &SessionRecord,
) -> Option<(&'static str, Option<String>)> {
    if before == Some(r.status.as_str()) {
        return None;
    }
    let was_ended = before.is_some_and(|s| matches!(s, "completed" | "failed" | "stopped"));
    match r.status.as_str() {
        "running" => Some((STARTED, None)),
        "parked" => Some((PARKED, None)),
        "unknown" => Some((UNKNOWN, None)),
        "failed" if !was_ended => Some((FAILED, r.end_reason.clone())),
        "completed" | "stopped" if !was_ended => Some((ENDED, r.end_reason.clone())),
        _ => None,
    }
}

// ---- agent.db ---------------------------------------------------------------

/// Two new tables, nothing else touched: the queue, and each parent's
/// sequence and acknowledged cursor. No CHECK on `kind`, so a later kind
/// needs no rebuild.
pub(super) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS child_events (
             parent          TEXT NOT NULL,
             seq             INTEGER NOT NULL,
             child           TEXT NOT NULL,
             kind            TEXT NOT NULL,
             reason          TEXT,
             status          TEXT NOT NULL,
             activity        TEXT,
             needs_attention TEXT,
             source          TEXT NOT NULL,
             confidence      TEXT NOT NULL,
             at              TEXT NOT NULL,
             PRIMARY KEY (parent, seq)
         );
         CREATE INDEX IF NOT EXISTS child_events_by_child ON child_events(child, seq);
         CREATE TABLE IF NOT EXISTS child_event_cursor (
             parent         TEXT PRIMARY KEY,
             last_seq       INTEGER NOT NULL,
             acked          INTEGER NOT NULL
         );
         -- 145f: what the agent itself said a round cost, as its host
         -- reported it with the round's final message.
         CREATE TABLE IF NOT EXISTS session_round_usage (
             session_id  TEXT NOT NULL,
             round       INTEGER NOT NULL,
             usage       TEXT NOT NULL,
             reported_at TEXT NOT NULL,
             PRIMARY KEY (session_id, round)
         );",
    )?;
    // 145f and 148b, additive: a queue from before them gains the round's
    // columns and a denial's.
    for (column, kind) in [
        ("round", "INTEGER"),
        ("duration_ms", "INTEGER"),
        ("usage", "TEXT"),
        ("tool", "TEXT"),
        ("detail", "TEXT"),
        ("count", "INTEGER"),
    ] {
        let has = conn
            .prepare("SELECT 1 FROM pragma_table_info('child_events') WHERE name = ?1")?
            .exists([column])?;
        if !has {
            conn.execute(
                &format!("ALTER TABLE child_events ADD COLUMN {column} {kind}"),
                [],
            )?;
        }
    }
    Ok(())
}

/// The round `child` is in, and when it began by Storm's own record: its
/// start for round 1, else the message that opened it.
fn round_start(conn: &Connection, child: &SessionRecord) -> Result<(i64, Option<String>)> {
    let opened: Option<(i64, String)> = conn
        .query_row(
            "SELECT round, changed_at FROM session_round WHERE session_id = ?1",
            params![child.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match opened {
        Some((round, at)) => (round, Some(at)),
        None => (
            1,
            child
                .started_at
                .clone()
                .or_else(|| Some(child.created_at.clone())),
        ),
    })
}

fn millis_between(from: &str, to: &str) -> Option<i64> {
    use time::format_description::well_known::Rfc3339;
    let from = time::OffsetDateTime::parse(from, &Rfc3339).ok()?;
    let to = time::OffsetDateTime::parse(to, &Rfc3339).ok()?;
    Some(((to - from).whole_milliseconds().max(0)) as i64)
}

/// The usage the agent reported for `round`, if it did.
pub(super) fn round_usage(
    conn: &Connection,
    session_id: &str,
    round: i64,
) -> Result<Option<Usage>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT usage FROM session_round_usage WHERE session_id = ?1 AND round = ?2",
            params![session_id, round],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
}

/// Keeps what the agent said `round` cost; a later word for the same round
/// replaces it.
pub(super) fn record_usage(
    conn: &Connection,
    session_id: &str,
    round: i64,
    usage: &Usage,
    now: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO session_round_usage (session_id, round, usage, reported_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (session_id, round) DO UPDATE SET usage = excluded.usage,
             reported_at = excluded.reported_at",
        params![session_id, round, serde_json::to_string(usage)?, now],
    )?;
    Ok(())
}

/// 134p's `needs_attention` with 144b's `usage_limit`: what a gated signal
/// says while the session runs, then a `blocked` result for the round it is
/// in (145). An ended session needs none; since 145a nothing waits for a
/// person's approval.
pub(super) fn needs_attention_in(
    conn: &Connection,
    r: &SessionRecord,
    live_attention: Option<&str>,
) -> Result<Option<&'static str>> {
    if r.is_ended() {
        return Ok(None);
    }
    if r.status == "running" {
        match live_attention {
            Some(PERMISSION_PROMPT) => return Ok(Some(PERMISSION_PROMPT)),
            Some(USAGE_LIMIT) => return Ok(Some(USAGE_LIMIT)),
            _ => {}
        }
    }
    let blocked = conn
        .query_row(
            "SELECT outcome FROM session_result WHERE session_id = ?1
               AND round >= COALESCE(
                   (SELECT round FROM session_round WHERE session_id = ?1), 1)",
            params![r.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|o| o == BLOCKED);
    Ok(blocked.then_some(BLOCKED))
}

fn state_in(conn: &Connection, r: &SessionRecord, live: &LiveSignals) -> Result<EventState> {
    Ok(EventState {
        status: r.status.clone(),
        activity: health_activity(r, live),
        needs_attention: needs_attention_in(conn, r, live.attention.as_deref())?.map(Into::into),
    })
}

/// Writes one event to `child`'s parent's queue, in the caller's
/// transaction. Nothing for a session without a parent, and nothing but a
/// terminal kind past the per-child cap. Returns whether it wrote.
pub(super) fn record_in(
    conn: &Connection,
    child: &SessionRecord,
    kind: &'static str,
    reason: Option<&str>,
    source: &'static str,
    live: &LiveSignals,
    now: &str,
) -> Result<bool> {
    let parent: Option<String> = conn
        .query_row(
            "SELECT parent_session FROM session_lineage WHERE session_id = ?1",
            params![child.id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(parent) = parent else {
        return Ok(false);
    };
    if !KEPT_PAST_CAP.contains(&kind) {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM child_events WHERE child = ?1",
            params![child.id],
            |r| r.get(0),
        )?;
        if n >= MAX_EVENTS_PER_CHILD {
            return Ok(false);
        }
    }
    let state = state_in(conn, child, live)?;
    let (round, duration_ms, usage) = if ROUND_KINDS.contains(&kind) {
        let (round, began) = round_start(conn, child)?;
        let usage = round_usage(conn, &child.id, round)?
            .map(|u| serde_json::to_string(&u))
            .transpose()?;
        (
            Some(round),
            began.and_then(|b| millis_between(&b, now)),
            usage,
        )
    } else {
        (None, None, None)
    };
    conn.execute(
        "INSERT INTO child_event_cursor (parent, last_seq, acked) VALUES (?1, 1, 0)
         ON CONFLICT (parent) DO UPDATE SET last_seq = last_seq + 1",
        params![parent],
    )?;
    let seq: i64 = conn.query_row(
        "SELECT last_seq FROM child_event_cursor WHERE parent = ?1",
        params![parent],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO child_events (parent, seq, child, kind, reason, status, activity,
             needs_attention, source, confidence, at, round, duration_ms, usage)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            parent,
            seq,
            child.id,
            kind,
            reason,
            state.status,
            state.activity,
            state.needs_attention,
            source,
            confidence(source),
            now,
            round,
            duration_ms,
            usage
        ],
    )?;
    Ok(true)
}

/// A session's events and its own queue go with its record (144a: as long
/// as its tree's records).
pub(super) fn forget_in(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM child_events WHERE parent = ?1 OR child = ?1",
        params![session_id],
    )?;
    conn.execute(
        "DELETE FROM child_event_cursor WHERE parent = ?1",
        params![session_id],
    )?;
    Ok(())
}

const EVENT_COLUMNS: &str = "parent, seq, child, kind, reason, status, activity, \
    needs_attention, source, confidence, at, round, duration_ms, usage, tool, detail, count";

fn event_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChildEvent> {
    let parent: String = r.get(0)?;
    let seq: i64 = r.get(1)?;
    Ok(ChildEvent {
        id: format!("evt_{parent}_{seq}"),
        seq,
        child: r.get(2)?,
        kind: r.get(3)?,
        reason: r.get(4)?,
        state: EventState {
            status: r.get(5)?,
            activity: r.get(6)?,
            needs_attention: r.get(7)?,
        },
        source: r.get(8)?,
        confidence: r.get(9)?,
        at: r.get(10)?,
        round: r.get(11)?,
        duration_ms: r.get(12)?,
        usage: r
            .get::<_, Option<String>>(13)?
            .and_then(|t| serde_json::from_str(&t).ok()),
        tool: r.get(14)?,
        detail: r.get(15)?,
        count: r.get(16)?,
    })
}

fn bounded(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// A child's denial (148b) in its parent's queue, in the caller's
/// transaction: a new `denied` event, or, for an identical one (same tool
/// and detail) already written in the same round, that event's count one
/// higher. Nothing past the per-child cap. Returns whether a new event was
/// written.
pub(super) fn record_denied_in(
    conn: &Connection,
    child: &SessionRecord,
    tool: &str,
    detail: &str,
    live: &LiveSignals,
    now: &str,
) -> Result<bool> {
    let tool = bounded(tool, MAX_DENIAL_TOOL);
    let detail = bounded(detail, MAX_DENIAL_DETAIL);
    let (round, _) = round_start(conn, child)?;
    let merged = conn.execute(
        "UPDATE child_events SET count = count + 1
         WHERE child = ?1 AND kind = ?2 AND round = ?3 AND tool = ?4 AND detail = ?5",
        params![child.id, DENIED, round, tool, detail],
    )?;
    if merged > 0 {
        return Ok(false);
    }
    if !record_in(conn, child, DENIED, None, PROVIDER_HOOK, live, now)? {
        return Ok(false);
    }
    conn.execute(
        "UPDATE child_events SET round = ?2, tool = ?3, detail = ?4, count = 1
         WHERE child = ?1 AND seq = (SELECT MAX(seq) FROM child_events WHERE child = ?1)",
        params![child.id, round, tool, detail],
    )?;
    Ok(true)
}

impl Store {
    pub(super) fn notify_events(&self) {
        self.events.send_modify(|v| *v += 1);
    }

    /// Writes one event in a transaction of its own: a change that is not
    /// in `agent.db` (an attention or idle signal).
    pub fn record_event(
        &self,
        child: &SessionRecord,
        kind: &'static str,
        reason: Option<&str>,
        source: &'static str,
        live: &LiveSignals,
    ) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let wrote = record_in(&tx, child, kind, reason, source, live, &now())?;
        tx.commit()?;
        if wrote {
            self.notify_events();
        }
        Ok(wrote)
    }

    /// [`record_denied_in`] in a transaction of its own.
    pub fn record_denied(
        &self,
        child: &SessionRecord,
        tool: &str,
        detail: &str,
        live: &LiveSignals,
    ) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let wrote = record_denied_in(&tx, child, tool, detail, live, &now())?;
        tx.commit()?;
        if wrote {
            self.notify_events();
        }
        Ok(wrote)
    }

    /// `parent`'s events after `after`, oldest first, for `children` (all
    /// of them when empty), at most `limit`.
    pub fn events_after(
        &self,
        parent: &str,
        after: i64,
        children: &[String],
        limit: usize,
    ) -> Result<Vec<ChildEvent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM child_events
             WHERE parent = ?1 AND seq > ?2 ORDER BY seq"
        ))?;
        let rows = stmt.query_map(params![parent, after], event_from_row)?;
        let mut out = Vec::new();
        for e in rows {
            let e = e?;
            if children.is_empty() || children.contains(&e.child) {
                out.push(e);
                if out.len() == limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// The newest event `parent` has not acknowledged, by its `seq`: the
    /// mark a parent's wake is for (144c). 0 with none.
    pub fn unread_events_mark(&self, parent: &str) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT CASE WHEN last_seq > acked THEN last_seq ELSE 0 END
                 FROM child_event_cursor WHERE parent = ?1",
                params![parent],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// The cursor `parent` last acknowledged; 0 before any.
    pub fn acked_events(&self, parent: &str) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT acked FROM child_event_cursor WHERE parent = ?1",
                params![parent],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// Acknowledges up to `upto`: never backwards, never past the last
    /// event written.
    pub fn ack_events(&self, parent: &str, upto: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE child_event_cursor SET acked = MAX(acked, MIN(?2, last_seq))
             WHERE parent = ?1",
            params![parent, upto],
        )?;
        Ok(())
    }

    /// `child`'s events in its parent's queue not yet acknowledged.
    pub fn pending_events(&self, child: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM child_events e
             WHERE e.child = ?1 AND e.seq > COALESCE(
                 (SELECT acked FROM child_event_cursor c
                  WHERE c.parent = e.parent), 0)",
            params![child],
            |r| r.get(0),
        )?)
    }

    /// `child`'s latest events, oldest first.
    pub fn recent_events(&self, child: &str, n: usize) -> Result<Vec<ChildEvent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM child_events WHERE child = ?1 ORDER BY seq DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![child, n as i64], event_from_row)?;
        let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        out.reverse();
        Ok(out)
    }
}

// ---- the manager ------------------------------------------------------------

/// What one `wait_agent` read: the events, the highest `seq` among them (or
/// where the read started, when none), and whether the bound ended it.
#[derive(Debug, Clone)]
pub struct EventsRead {
    pub events: Vec<ChildEvent>,
    pub cursor: i64,
    pub timed_out: bool,
}

/// The refusal for a session outside the caller's descendants (144d).
pub const NOT_PERMITTED: &str = "not_permitted: no such agent among your descendants";

impl AgentManager {
    /// The clock the idle spell is timed by; tests move it.
    pub(super) fn clock(&self) -> SystemTime {
        SystemTime::now() + *self.clock_skew.lock().unwrap()
    }

    #[cfg(test)]
    pub fn advance_clock_for_tests(&self, by: Duration) {
        *self.clock_skew.lock().unwrap() += by;
    }

    /// The gated half of a session's state, as its host last said.
    pub(super) fn live_signals(&self, session_id: &str) -> LiveSignals {
        self.live
            .lock()
            .unwrap()
            .sessions
            .get(session_id)
            .map(|s| LiveSignals {
                activity: s.health.clone(),
                attention: s.attention.clone(),
            })
            .unwrap_or_default()
    }

    /// 144b's activity for a session: `unknown` unless a gated idle signal
    /// says otherwise.
    pub fn health_activity(&self, r: &SessionRecord) -> Option<String> {
        health_activity(r, &self.live_signals(&r.id))
    }

    /// Writes `r` back with the events its change makes, in one
    /// transaction, and wakes every waiting reader when one was written.
    pub(super) fn update_record(&self, r: &SessionRecord, source: &'static str) -> Result<()> {
        let ctx = EventCtx::of(source, self.live_signals(&r.id));
        self.store.lock().unwrap().update_with(r, &ctx)
    }

    /// `wait_agent`'s read (144a): `caller`'s events after `after`, else
    /// after its acknowledged cursor, for `ids` (all its children when
    /// empty). Returns at once when there are any; otherwise waits up to
    /// `bound` for the first. Passing `after` acknowledges up to it.
    pub async fn wait_events(
        &self,
        caller: &str,
        ids: &[String],
        after: Option<i64>,
        bound: Duration,
        still_offered: &(dyn Fn() -> bool + Send + Sync),
    ) -> AgentResult<EventsRead> {
        if ids.len() > MAX_WAIT_IDS {
            return Err(AgentError::BadRequest(format!(
                "wait for at most {MAX_WAIT_IDS} agents at once"
            )));
        }
        if after.is_some_and(|a| a < 0) {
            return Err(AgentError::BadRequest(
                "after is a cursor wait_agent returned, from 0".into(),
            ));
        }
        for id in ids {
            if !self.store.lock().unwrap().is_descendant(caller, id)? {
                return Err(AgentError::Conflict(NOT_PERMITTED.into()));
            }
        }
        let from = self.read_from(caller, after)?;
        let deadline = Instant::now() + bound;
        let mut changed = self.store.lock().unwrap().events.subscribe();
        loop {
            changed.borrow_and_update();
            let events =
                self.store
                    .lock()
                    .unwrap()
                    .events_after(caller, from, ids, MAX_EVENTS_PER_WAIT)?;
            let left = deadline.saturating_duration_since(Instant::now());
            if !events.is_empty() || left.is_zero() {
                let timed_out = events.is_empty();
                let cursor = events.last().map_or(from, |e| e.seq);
                return Ok(EventsRead {
                    events,
                    cursor,
                    timed_out,
                });
            }
            if !still_offered() {
                return Err(AgentError::Conflict("not_permitted".into()));
            }
            // The tick bounds how late a revocation or a dismissal is seen.
            let _ =
                tokio::time::timeout(left.min(Duration::from_millis(500)), changed.changed()).await;
        }
    }

    /// A host's report of a child's denial (148b): taken only from the
    /// child's own host, for a running spawned session whose agent's gate
    /// lets denials through, from its agent's own hook; otherwise dropped,
    /// with nothing written. Evidence, never authority (144d): it tells the
    /// parent and decides nothing. Returns whether a new event was written.
    pub fn denied(
        &self,
        host_id: &str,
        session_id: &str,
        report: &DeniedReport,
    ) -> AgentResult<bool> {
        let r = self.owned_by(host_id, session_id)?;
        if r.status != "running" || !DENIALS.contains(&r.provider.as_str()) {
            return Ok(false);
        }
        if !matches!(report.source.as_deref(), None | Some(PROVIDER_HOOK)) {
            return Ok(false);
        }
        let tool = report.tool.trim();
        if tool.is_empty() || tool.chars().any(char::is_control) {
            return Ok(false);
        }
        let live = self.live_signals(session_id);
        let store = self.store.lock().unwrap();
        if store.lineage(session_id)?.parent_session.is_none() {
            return Ok(false);
        }
        let wrote = store.record_denied(&r, tool, &report.detail, &live)?;
        drop(store);
        self.bump(session_id);
        Ok(wrote)
    }

    /// The events a settled wait carries, read once without waiting.
    pub fn events_now(
        &self,
        caller: &str,
        ids: &[String],
        after: Option<i64>,
    ) -> AgentResult<EventsRead> {
        let from = self.read_from(caller, after)?;
        let events =
            self.store
                .lock()
                .unwrap()
                .events_after(caller, from, ids, MAX_EVENTS_PER_WAIT)?;
        let cursor = events.last().map_or(from, |e| e.seq);
        Ok(EventsRead {
            events,
            cursor,
            timed_out: false,
        })
    }

    fn read_from(&self, caller: &str, after: Option<i64>) -> AgentResult<i64> {
        let store = self.store.lock().unwrap();
        Ok(match after {
            Some(n) => {
                store.ack_events(caller, n)?;
                self.wake_poke.notify_one();
                n
            }
            None => store.acked_events(caller)?,
        })
    }

    /// `child`'s events its parent has not acknowledged.
    pub fn pending_events(&self, child: &str) -> AgentResult<i64> {
        Ok(self.store.lock().unwrap().pending_events(child)?)
    }

    pub fn recent_events(&self, child: &str) -> AgentResult<Vec<ChildEvent>> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .recent_events(child, RECENT_EVENTS)?)
    }

    /// Writes `idle_timeout` for every child idle past [`IDLE_EVENT_AFTER`]
    /// by a gated signal, once per idle spell, unless it has reported.
    /// Returns how many it wrote.
    pub fn fire_idle_timeouts(&self) -> AgentResult<usize> {
        let at = self.clock();
        let due: Vec<(String, &'static str, LiveSignals)> = self
            .live
            .lock()
            .unwrap()
            .sessions
            .iter()
            .filter(|(_, s)| {
                !s.idle_fired
                    && s.idle_since.is_some_and(|since| {
                        at.duration_since(since).unwrap_or_default() >= IDLE_EVENT_AFTER
                    })
            })
            .map(|(id, s)| {
                (
                    id.clone(),
                    s.health_source.unwrap_or(OUTPUT_PATTERN),
                    LiveSignals {
                        activity: s.health.clone(),
                        attention: s.attention.clone(),
                    },
                )
            })
            .collect();
        let mut wrote = 0;
        for (id, source, live) in due {
            // A session not running now keeps its spell armed: its host
            // says nothing true of it until it reports again.
            let spent = {
                let store = self.store.lock().unwrap();
                match store.get(&id)? {
                    Some(r) if r.status == "running" => {
                        let written = store.result(&id)?.is_none()
                            && store.record_event(&r, IDLE_TIMEOUT, None, source, &live)?;
                        Some(written)
                    }
                    _ => None,
                }
            };
            let Some(written) = spent else { continue };
            if let Some(s) = self.live.lock().unwrap().sessions.get_mut(&id) {
                s.idle_fired = true;
            }
            wrote += usize::from(written);
        }
        Ok(wrote)
    }

    #[cfg(test)]
    pub fn events_of_for_tests(&self, parent: &str) -> Vec<ChildEvent> {
        self.store
            .lock()
            .unwrap()
            .events_after(parent, 0, &[], usize::MAX)
            .unwrap()
    }
}
