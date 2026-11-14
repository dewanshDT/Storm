//! `needs_attention` (decision 134p, slice 134-9; 144b): beside `activity`,
//! not a status, and never inferred from idle (A3). One kind at a time:
//!
//! - `permission_prompt`: the agent's own CLI prompt waits, as its host
//!   reports it — only through a signal on the gated list below, and only
//!   for a child, whose parent's event says `permission_prompt` too (151
//!   N5): the person answers it in the child's terminal;
//! - `usage_limit` (144b): the agent's account is out of usage, the same way;
//! - `permission_prompt`, `question` and `sign_in` from the program's own
//!   OSC 7501 `blocked` (decision 155, AM114), for every session whose
//!   program reports one, by capability and not by provider. While a record
//!   is live it decides: any other state clears what it set, and a hook's
//!   permission prompt is not taken (Q10), so a prompt refused with Esc
//!   clears;
//! - `blocked`: the session reported a result whose outcome is `blocked`
//!   (134h) and is still live.
//!
//! Every kind is worked out when a view is read, from what is on record
//! (the last host report, for a signal), so it is set and cleared with the
//! record itself.
//!
//! **A signal is evidence, never authority** (144b): what a host reports
//! here can make Storm show something, and can never approve, refuse,
//! start, end or unblock anything.

use serde::Serialize;

use super::events::{self, Extra, LiveSignals, OUTPUT_PATTERN, PROGRAM_STATUS, PROVIDER_HOOK};
use super::program::{Program, ProgramStatus};
use super::store::SessionRecord;
use super::{AgentManager, AgentResult, StatusReport};

/// One provider's signal, through one source, switched on once its gate
/// (G-144-S-<provider>-<signal>; G-134-7 for the permission prompt) passes
/// on the real CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatedSignal {
    pub provider: &'static str,
    /// [`PERMISSION_PROMPT`], [`IDLE`] or [`USAGE_LIMIT`].
    pub signal: &'static str,
    /// `provider_hook` or `output_pattern`.
    pub source: &'static str,
}

/// The signals taken from hosts. The host keeps the same list
/// (`harness::GATED_SIGNALS`), spelled the same with string literals, and
/// reports only what is on it: a test in each crate holds the two equal.
/// On: a child's permission prompt for Claude Code and OpenCode (146c),
/// whose G-146-P passed locally on 2026-10-29 (Claude Code 2.1.312,
/// OpenCode 1.19.4); its host writes the hooks for a child only. Its VM 113
/// run is the operator's.
pub const GATED_SIGNALS: &[GatedSignal] = &[
    GatedSignal {
        provider: "claude-code",
        signal: "permission_prompt",
        source: "provider_hook",
    },
    GatedSignal {
        provider: "opencode",
        signal: "permission_prompt",
        source: "provider_hook",
    },
];

/// 134-9's adapters, whose permission prompt hooks every session of theirs
/// would get: none. A child's prompt is gated through [`GATED_SIGNALS`]
/// instead (146c). The host's own test reads this line to keep the two
/// crates' lists equal.
#[cfg(test)]
pub const PERMISSION_PROMPT_ADAPTERS: &[&str] = &[];

pub const PERMISSION_PROMPT: &str = "permission_prompt";
pub const USAGE_LIMIT: &str = "usage_limit";
pub const BLOCKED: &str = "blocked";
/// Decision 155 (AM114): the program asks the person something, or is
/// blocked on a dialog it did not name.
pub const QUESTION: &str = "question";
/// Decision 155 (AM114): the program needs the person to sign in again.
pub const SIGN_IN: &str = "sign_in";
/// 149c: a top-level session's deletion waits for the person's Approve.
/// Only on its view; never a child's, so never in an event.
pub const DELETE_APPROVAL: &str = "delete_approval";
/// The signal behind `activity` (144b).
pub const IDLE: &str = "idle";

/// Why a session waits for a person (153a), beside `needs_attention`,
/// which stays for older clients. Every field but `kind` is evidence that
/// may be missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attention {
    /// `needs_attention`'s value.
    pub kind: &'static str,
    /// When it began: a signal's first report in this run of the server, a
    /// blocked result's `reported_at`, a delete request's `created_at`.
    pub since: Option<String>,
    /// A blocked result's own summary: the agent's words, as data.
    pub detail: Option<String>,
    /// The session it waits on. No result names one as data yet, so always
    /// `None`; a blocked result says it in `detail`, if at all.
    pub blocked_on: Option<String>,
    /// When a usage limit lifts. No host reports it yet (144b's hook
    /// carries no time), so always `None`.
    pub resets_at: Option<String>,
}

impl Attention {
    fn new(kind: &'static str, since: Option<String>) -> Self {
        Self {
            kind,
            since,
            detail: None,
            blocked_on: None,
            resets_at: None,
        }
    }
}

/// What one host report says, through the gates.
pub(super) struct Taken {
    pub live: LiveSignals,
    /// The attention events its change makes.
    pub extra: Vec<Extra>,
}

fn source_of(reported: Option<&str>, default: &'static str) -> Option<&'static str> {
    match reported {
        None => Some(default),
        Some(PROVIDER_HOOK) => Some(PROVIDER_HOOK),
        Some(OUTPUT_PATTERN) => Some(OUTPUT_PATTERN),
        Some(_) => None,
    }
}

impl AgentManager {
    pub(super) fn gated(&self, provider: &str, signal: &str, source: Option<&'static str>) -> bool {
        source.is_some_and(|source| {
            self.signals
                .lock()
                .unwrap()
                .iter()
                .any(|g| g.provider == provider && g.signal == signal && g.source == source)
        })
    }

    /// Takes a host's report of a session's activity and attention: its
    /// display activity as before (AM42), and only what a gated signal
    /// says for 144b. An ungated report changes no state and makes no
    /// event. Times the idle spell.
    pub(super) fn take_signals(&self, r: &SessionRecord, report: &StatusReport) -> Taken {
        // A parked child (145e) has no process to say anything of.
        let live_session = !r.is_ended() && r.status != "parked";
        let program = report
            .program_status
            .as_ref()
            .filter(|_| live_session)
            .and_then(Program::of);
        // A null activity is the host's unknown (155-R1's safeguard), and a
        // record never makes it known.
        let display = report
            .activity
            .as_deref()
            .filter(|a| live_session && matches!(*a, "working" | "idle"))
            .map(|a| program.map_or_else(|| a.to_owned(), |p| p.activity().to_owned()));
        let attention_source = source_of(report.attention_source.as_deref(), PROVIDER_HOOK);
        let hooked = report
            .attention
            .as_deref()
            .filter(|a| {
                live_session
                    && matches!(*a, PERMISSION_PROMPT | USAGE_LIMIT)
                    && self.gated(&r.provider, a, attention_source)
            })
            .map(str::to_owned);
        let (attention, attention_source) = match program {
            Some(p) => match p.attention() {
                Some(a) => (Some(a.to_owned()), Some(PROGRAM_STATUS)),
                None => (hooked.filter(|a| a == USAGE_LIMIT), attention_source),
            },
            None => (hooked, attention_source),
        };
        let activity_source = source_of(report.activity_source.as_deref(), OUTPUT_PATTERN);
        let health = display
            .clone()
            .filter(|_| self.gated(&r.provider, IDLE, activity_source));
        let at = self.clock();
        let stamp = super::now();
        let mut live = self.live.lock().unwrap();
        let s = live
            .sessions
            .entry(r.id.clone())
            .or_insert_with(super::SessionLive::new);
        let mut extra = Vec::new();
        if let Some(a) = attention.as_deref()
            && s.attention.as_deref() != Some(a)
        {
            let kind = if a == USAGE_LIMIT {
                events::USAGE_LIMIT_EVENT
            } else {
                events::NEEDS_ATTENTION
            };
            extra.push(Extra {
                kind,
                reason: Some(a.to_string()),
                source: attention_source.unwrap_or(PROVIDER_HOOK),
            });
        }
        match health.as_deref() {
            Some("idle") => {
                s.idle_since.get_or_insert(at);
            }
            Some(_) => {
                s.idle_since = None;
                s.idle_fired = false;
            }
            None => s.idle_since = None,
        }
        s.activity = display;
        s.program = program.map(|p| match s.program.take() {
            Some(was) if (was.state, was.kind) == (p.state, p.kind) => was,
            _ => ProgramStatus {
                state: p.state,
                kind: p.kind,
                since: stamp.clone(),
            },
        });
        if s.attention != attention {
            s.attention_since = attention.as_ref().map(|_| stamp.clone());
        }
        s.attention = attention.clone();
        s.health = health.clone();
        s.health_source = health.as_ref().and(activity_source);
        Taken {
            live: LiveSignals {
                activity: health,
                attention,
            },
            extra,
        }
    }

    /// The program's own status (decision 155), shown only while the
    /// session runs, as its activity is.
    pub fn program_status(&self, r: &SessionRecord) -> Option<ProgramStatus> {
        if r.status != "running" {
            return None;
        }
        self.live
            .lock()
            .unwrap()
            .sessions
            .get(&r.id)
            .and_then(|s| s.program.clone())
    }

    /// The session's `needs_attention`, if any. An ended session needs none.
    /// A signal shows only while the session runs, as activity does.
    pub fn needs_attention(&self, r: &SessionRecord) -> AgentResult<Option<&'static str>> {
        let live = self.live_signals(&r.id);
        let store = self.store.lock().unwrap();
        Ok(events::needs_attention_in(
            &store.conn,
            r,
            live.attention.as_deref(),
        )?)
    }

    /// The session's attention with its reason (153a): `needs_attention`'s
    /// kind, when it began and what the record says of it. A deletion
    /// waiting for the person is [`DELETE_APPROVAL`] whatever else is.
    pub fn attention(&self, r: &SessionRecord) -> AgentResult<Option<Attention>> {
        if let Some(request) = self.pending_delete_of(&r.id)? {
            return Ok(Some(Attention::new(
                DELETE_APPROVAL,
                Some(request.created_at),
            )));
        }
        let since = self
            .live
            .lock()
            .unwrap()
            .sessions
            .get(&r.id)
            .and_then(|s| s.attention_since.clone());
        let Some(kind) = self.needs_attention(r)? else {
            return Ok(None);
        };
        if kind != BLOCKED {
            return Ok(Some(Attention::new(kind, since)));
        }
        let store = self.store.lock().unwrap();
        let result = events::blocked_result_in(&store.conn, r)?;
        Ok(Some(match result {
            Some((summary, reported_at)) => Attention {
                detail: Some(summary),
                ..Attention::new(BLOCKED, Some(reported_at))
            },
            None => Attention::new(BLOCKED, None),
        }))
    }

    /// Test-only: signals taken, as once their gates have passed.
    #[cfg(test)]
    pub fn gate_signals_for_tests(&self, signals: &'static [GatedSignal]) {
        *self.signals.lock().unwrap() = signals;
    }

    /// Test-only: an adapter's permission prompts taken from its hooks, as
    /// once G-134-7 has passed for it.
    #[cfg(test)]
    pub fn report_permission_prompts_for_tests(&self, adapters: &'static [&'static str]) {
        let gated: Vec<GatedSignal> = adapters
            .iter()
            .map(|provider| GatedSignal {
                provider,
                signal: PERMISSION_PROMPT,
                source: PROVIDER_HOOK,
            })
            .collect();
        self.gate_signals_for_tests(Box::leak(gated.into_boxed_slice()));
    }
}
