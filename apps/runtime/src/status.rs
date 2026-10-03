//! The session status vocabulary (freeze §7.2).
//!
//! One vocabulary, replacing the four the design pack had grown. The strings
//! are wire values, shared by the host link, the server's records and the
//! client, so they are spelled out here rather than derived from the variant
//! names.

/// Where an agent session is in its life.
///
/// `creating → starting → running → completed | failed | stopped`. A session is
/// `unknown` while its host is unreachable or reconciliation is pending; that is
/// an honest state, not a placeholder. `Creating` and `Unknown` are the
/// server's to assign. A host reports only the others.
///
/// `crashed` and `restarting` are deliberately absent: nothing restarts a
/// session (D5), so a crash is `failed` with a reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionStatus {
    Creating,
    Starting,
    Running,
    /// The agent ended by itself.
    Completed,
    /// The session ended without ending by itself or being stopped by the
    /// owner. [`EndReason`] says which case.
    Failed,
    /// The owner ended it.
    Stopped,
    Unknown,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Unknown => "unknown",
        }
    }

    /// Whether the session is over. An ended session never changes status
    /// again, and only an ended session may be dismissed.
    pub fn is_ended(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Stopped)
    }
}

/// Why a session is `failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// It never got as far as running.
    StartFailure,
    /// A signal the owner did not send ended it.
    Signal(i32),
    /// Its host restarted; the session died with it.
    HostRestart,
    /// Its host was revoked, and ends everything it runs (freeze §5.6).
    HostRevoked,
    /// Its host did not report it on reconnect.
    Lost,
}

impl EndReason {
    /// The wire value. A signal's number travels separately.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartFailure => "start_failure",
            Self::Signal(_) => "signal",
            Self::HostRestart => "host_restart",
            Self::HostRevoked => "host_revoked",
            Self::Lost => "lost",
        }
    }
}

/// How a session ended, as its provider reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// It ended by itself. A `cli` provider has an exit code unless a signal
    /// ended it; another kind may have none.
    Completed {
        exit_code: Option<i32>,
    },
    /// The owner ended it. A provider reports this, and not the signal its own
    /// stop sent, once stop has been asked of it.
    Stopped,
    Failed(EndReason),
}

impl SessionEnd {
    pub fn status(self) -> SessionStatus {
        match self {
            Self::Completed { .. } => SessionStatus::Completed,
            Self::Stopped => SessionStatus::Stopped,
            Self::Failed(_) => SessionStatus::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_values_are_the_freezes() {
        // §7.2's spelling, which the server and the client also speak.
        let all = [
            (SessionStatus::Creating, "creating"),
            (SessionStatus::Starting, "starting"),
            (SessionStatus::Running, "running"),
            (SessionStatus::Completed, "completed"),
            (SessionStatus::Failed, "failed"),
            (SessionStatus::Stopped, "stopped"),
            (SessionStatus::Unknown, "unknown"),
        ];
        for (status, wire) in all {
            assert_eq!(status.as_str(), wire);
        }
        assert_eq!(EndReason::Signal(9).as_str(), "signal");
        assert_eq!(EndReason::HostRestart.as_str(), "host_restart");
    }

    #[test]
    fn only_the_three_endings_are_ended() {
        // `unknown` is not an ending: a session whose host is merely
        // unreachable comes back to its reported state on reconnect.
        assert!(SessionStatus::Completed.is_ended());
        assert!(SessionStatus::Failed.is_ended());
        assert!(SessionStatus::Stopped.is_ended());
        assert!(!SessionStatus::Unknown.is_ended());
        assert!(!SessionStatus::Running.is_ended());
    }

    #[test]
    fn an_ending_maps_to_its_status() {
        assert_eq!(
            SessionEnd::Completed { exit_code: Some(0) }.status(),
            SessionStatus::Completed
        );
        assert_eq!(SessionEnd::Stopped.status(), SessionStatus::Stopped);
        assert_eq!(
            SessionEnd::Failed(EndReason::Lost).status(),
            SessionStatus::Failed
        );
    }
}
