//! `needs_attention` (decision 134p, slice 134-9; 144b): beside `activity`,
//! not a status, and never inferred from idle (A3). One kind at a time:
//!
//! - `approval`: a request the session made (start, message, end) waits for
//!   a person's decision (134o);
//! - `permission_prompt`: the agent's own CLI prompt waits, as its host
//!   reports it — only through a signal on the gated list below, which is
//!   empty;
//! - `usage_limit` (144b): the agent's account is out of usage, the same way;
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

use super::events::{self, Extra, LiveSignals, OUTPUT_PATTERN, PROVIDER_HOOK};
use super::store::SessionRecord;
use super::{AgentManager, AgentResult, StatusReport, now};

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

/// The signals taken from hosts. None has passed its gate, so every one is
/// off for every provider; the host keeps its own list
/// (`harness::PERMISSION_PROMPT_ADAPTERS`), where the gates' results are.
pub const GATED_SIGNALS: &[GatedSignal] = &[];

/// The adapters whose permission prompt is on [`GATED_SIGNALS`], spelled as
/// the host's list is: the host's own test reads this line to keep the two
/// crates' lists equal, and `attention_tests` holds it to the gated list.
#[cfg(test)]
pub const PERMISSION_PROMPT_ADAPTERS: &[&str] = &[];

pub const APPROVAL: &str = "approval";
pub const PERMISSION_PROMPT: &str = "permission_prompt";
pub const USAGE_LIMIT: &str = "usage_limit";
pub const BLOCKED: &str = "blocked";
/// The signal behind `activity` (144b).
pub const IDLE: &str = "idle";

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
    fn gated(&self, provider: &str, signal: &str, source: Option<&'static str>) -> bool {
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
        let live_session = !r.is_ended();
        let display = report
            .activity
            .as_deref()
            .filter(|a| live_session && matches!(*a, "working" | "idle"))
            .map(str::to_owned);
        let attention_source = source_of(report.attention_source.as_deref(), PROVIDER_HOOK);
        let attention = report
            .attention
            .as_deref()
            .filter(|a| {
                live_session
                    && matches!(*a, PERMISSION_PROMPT | USAGE_LIMIT)
                    && self.gated(&r.provider, a, attention_source)
            })
            .map(str::to_owned);
        let activity_source = source_of(report.activity_source.as_deref(), OUTPUT_PATTERN);
        let health = display
            .clone()
            .filter(|_| self.gated(&r.provider, IDLE, activity_source));
        let at = self.clock();
        let mut live = self.live.lock().unwrap();
        let s = live
            .sessions
            .entry(r.id.clone())
            .or_insert_with(super::SessionLive::new);
        let mut extra = Vec::new();
        if let Some(a) = attention.as_deref()
            && s.attention.as_deref() != Some(a)
        {
            extra.push(Extra {
                kind: if a == PERMISSION_PROMPT {
                    events::NEEDS_ATTENTION
                } else {
                    events::USAGE_LIMIT_EVENT
                },
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

    /// The session's `needs_attention`, if any. An ended session needs none.
    /// A waiting request comes first, since a person can answer it in Storm;
    /// a signal shows only while the session runs, as activity does.
    pub fn needs_attention(&self, r: &SessionRecord) -> AgentResult<Option<&'static str>> {
        let live = self.live_signals(&r.id);
        let store = self.store.lock().unwrap();
        Ok(events::needs_attention_in(
            &store.conn,
            r,
            live.attention.as_deref(),
            &now(),
        )?)
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
