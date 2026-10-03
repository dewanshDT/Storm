//! The provider contract (freeze §9.1, AM20; `PLAN.md` decision 77).
//!
//! **The provider API is not the PTY.** A provider is described by its
//! identity and availability, its *kind* (how a session of it is started) and
//! the *interactions* a session of it offers. The runtime owns how an
//! interaction is carried: for a `cli` provider's terminal, the runtime builds
//! the PTY around the launch description the provider gives it. A future
//! native, API or SDK integration is a new kind offering a new interaction, and
//! slots into the same session model without anything here changing shape.
//!
//! Output and endings flow through [`SessionEvents`], a sink the *runtime*
//! owns. The provider never holds the scrollback: the same sink serves a PTY
//! reader thread and the in-memory fake, and it is where the host link will
//! pick output up.
//!
//! The contract is synchronous and dependency-free. PTY I/O is blocking
//! threads anyway, and the async host link adapts at its own edge.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::status::{SessionEnd, SessionStatus};

/// A provider's id: `claude-code`, `opencode`, `shell`, `fake`.
///
/// Lowercase ASCII letters, digits and `-`, starting with a letter or digit,
/// at most 32 bytes. It is a wire value and a config key, and the server
/// compares it to the default it stores, so it has exactly one spelling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(id: &str) -> Option<Self> {
        let valid = (1..=32).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !id.starts_with('-');
        valid.then(|| Self(id.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a session of a provider is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// A command on the host, which the runtime runs inside a PTY.
    Cli,
    /// In process, with an in-memory terminal: the test provider. It is its
    /// own kind because calling it `cli` would be a claim the host acts on
    /// (decision 77). A host offers it only when its config names it.
    Fake,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Fake => "fake",
        }
    }
}

/// How a human and a session exchange input and output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionKind {
    /// A byte stream each way plus a size.
    Terminal,
}

impl InteractionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
        }
    }
}

/// Whether a provider can start sessions on this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// Its binary did not resolve. This is what the server's announced
    /// fallback (freeze §6) reports as `not_installed`.
    NotInstalled,
}

/// A terminal's size in character cells. Both dimensions are at least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    cols: u16,
    rows: u16,
}

impl TerminalSize {
    pub fn new(cols: u16, rows: u16) -> Option<Self> {
        (cols > 0 && rows > 0).then_some(Self { cols, rows })
    }

    pub fn cols(self) -> u16 {
        self.cols
    }

    pub fn rows(self) -> u16 {
        self.rows
    }
}

/// The interaction a session is started with, and its parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionSpec {
    Terminal(TerminalSize),
}

/// Everything a host is told about a session (freeze §7.1).
///
/// Never a user id or a credential: the host does not learn who owns a session.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub session_id: String,
    /// The workspace directory, already resolved inside a configured root.
    pub workspace: PathBuf,
    pub interaction: InteractionSpec,
}

/// Where a session's output and its ending go. The runtime implements it.
///
/// A provider session calls `output` for every chunk, in order, and `ended`
/// **exactly once**, after which it calls neither again. Both may be called
/// from any thread, a PTY reader's included.
pub trait SessionEvents: Send + Sync {
    fn output(&self, bytes: &[u8]);
    fn ended(&self, end: SessionEnd);
}

/// Why a session could not be started.
#[derive(Debug)]
pub enum StartError {
    /// The provider is not installed on this host.
    NotAvailable,
    /// Anything else, with the reason for the host's log and the session's
    /// `start_failure`.
    Failed(String),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAvailable => f.write_str("provider not installed on this host"),
            Self::Failed(reason) => write!(f, "could not start: {reason}"),
        }
    }
}

impl std::error::Error for StartError {}

/// An agent integration.
pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn kind(&self) -> ProviderKind;
    /// The interaction kinds a session of this provider offers. V1: terminal.
    fn interactions(&self) -> &[InteractionKind];
    fn available(&self) -> Availability;
    /// Starts a session. On success the session is running and owns `events`;
    /// on failure `events` has not been called.
    fn start(
        &self,
        spec: SessionSpec,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Box<dyn ProviderSession>, StartError>;
}

/// A running instance of a provider.
pub trait ProviderSession: Send {
    /// The interaction the session was started with.
    fn interaction(&mut self) -> Interaction<'_>;
    /// Asks the session to end, allowing `grace` before it is forced. The
    /// session then reports [`SessionEnd::Stopped`]. Calling it on an ended
    /// session does nothing.
    fn stop(&mut self, grace: Duration);
    fn status(&self) -> SessionStatus;
}

/// A session's interaction, borrowed for one exchange.
pub enum Interaction<'a> {
    Terminal(&'a mut dyn TerminalChannel),
}

/// The input half of a terminal interaction. Output arrives through
/// [`SessionEvents::output`].
pub trait TerminalChannel: Send {
    /// Raw input bytes: keystrokes, pastes, control sequences.
    ///
    /// Fails with [`io::ErrorKind::BrokenPipe`] once the session has ended.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn resize(&mut self, size: TerminalSize) -> io::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_id_has_exactly_one_spelling() {
        for good in ["claude-code", "opencode", "shell", "fake", "x", "a1-b2"] {
            assert!(ProviderId::new(good).is_some(), "{good}");
        }
        // Case, whitespace and a leading dash are refused rather than folded:
        // the server compares ids to the default it stores.
        for bad in ["", "Claude-Code", "claude code", "-shell", "claude_code"] {
            assert!(ProviderId::new(bad).is_none(), "{bad:?}");
        }
        assert!(ProviderId::new(&"a".repeat(33)).is_none());
    }

    #[test]
    fn a_terminal_has_at_least_one_cell() {
        assert!(TerminalSize::new(0, 24).is_none());
        assert!(TerminalSize::new(80, 0).is_none());
        let size = TerminalSize::new(80, 24).unwrap();
        assert_eq!((size.cols(), size.rows()), (80, 24));
    }

    #[test]
    fn the_wire_values_are_the_freezes() {
        assert_eq!(ProviderKind::Cli.as_str(), "cli");
        assert_eq!(ProviderKind::Fake.as_str(), "fake");
        assert_eq!(InteractionKind::Terminal.as_str(), "terminal");
    }
}
