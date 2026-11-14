//! Decision 155 (AM113, AM114): the program's own OSC 7501 root record, as
//! its host parsed and posted it. In memory like `activity`, evidence only,
//! and taken by capability: any session whose program reports one, whatever
//! its provider. `msg`, `title`, `progress` and `id` are never read.

use serde::{Deserialize, Serialize};

use super::attention::{PERMISSION_PROMPT, QUESTION, SIGN_IN};

pub const IDLE: &str = "idle";
pub const WORKING: &str = "working";
pub const DONE: &str = "done";
pub const BLOCKED: &str = "blocked";
pub const ERROR: &str = "error";

const STATES: [&str; 5] = [IDLE, WORKING, DONE, BLOCKED, ERROR];
const KINDS: [&str; 3] = ["permission", "question", "auth"];

/// `program_status` in a status POST (Freeze §11.5): `{state, kind, app}`.
/// Absent from an older host, `null` when no record is live (none yet,
/// after `clear`, or one that did not parse).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ProgramReport {
    pub state: String,
    #[serde(default)]
    pub kind: Option<String>,
    /// Accepted and never kept.
    #[serde(default)]
    pub app: Option<String>,
}

/// A malformed `program_status` reads as none, never as a refused report.
pub(super) fn lenient<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<ProgramReport>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).ok())
}

/// A root record Storm maps: a known state, and a known kind on `blocked`
/// only. `clear` and anything else is no record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Program {
    pub state: &'static str,
    pub kind: Option<&'static str>,
}

impl Program {
    pub fn of(report: &ProgramReport) -> Option<Self> {
        let state = STATES.into_iter().find(|s| *s == report.state)?;
        let kind = report
            .kind
            .as_deref()
            .filter(|_| state == BLOCKED)
            .and_then(|k| KINDS.into_iter().find(|known| *known == k));
        Some(Self { state, kind })
    }

    /// `working` while the program works, else `idle` (AM113).
    pub fn activity(self) -> &'static str {
        if self.state == WORKING { WORKING } else { IDLE }
    }

    /// Q7: `blocked kind=permission` is a permission prompt, `auth` a
    /// sign-in, a question or no kind a question. Nothing else is.
    pub fn attention(self) -> Option<&'static str> {
        (self.state == BLOCKED).then_some(match self.kind {
            Some("permission") => PERMISSION_PROMPT,
            Some("auth") => SIGN_IN,
            _ => QUESTION,
        })
    }
}

/// `program_status` on a session's view (Freeze §7.1): the record and when
/// it began, by its first report in this run of the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramStatus {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub since: String,
}
