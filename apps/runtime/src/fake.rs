//! The fake provider: an in-memory terminal, no PTY, no process (freeze §9.1).
//!
//! It exists for two reasons. It is the proof that the provider contract is
//! not the PTY, since it satisfies the whole contract without one. And it lets
//! the Agent Manager, the host link and the client be tested end to end
//! without an agent binary (AC-A1).
//!
//! Its terminal behaves like a line discipline in echo mode, closely enough to
//! assert on, and everything it does is observable through the contract alone:
//!
//! - On start it writes one line naming the session and its workspace, so a
//!   test can see that both arrived.
//! - Every input byte is echoed back as output.
//! - A resize writes `[resize COLSxROWS]\r\n`.
//! - Ctrl-D (`0x04`) ends the session by itself, exit code 0.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crate::provider::{
    Availability, Interaction, InteractionKind, InteractionSpec, Provider, ProviderId,
    ProviderKind, ProviderSession, SessionEvents, SessionSpec, StartError, TerminalChannel,
    TerminalSize,
};
use crate::status::{SessionEnd, SessionStatus};

/// End-of-transmission: what Ctrl-D sends.
const EOT: u8 = 0x04;

const INTERACTIONS: [InteractionKind; 1] = [InteractionKind::Terminal];

pub struct FakeProvider {
    id: ProviderId,
}

impl FakeProvider {
    pub fn new() -> Self {
        Self {
            id: ProviderId::new("fake").expect("a valid id"),
        }
    }
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for FakeProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Fake
    }

    fn interactions(&self) -> &[InteractionKind] {
        &INTERACTIONS
    }

    fn available(&self) -> Availability {
        Availability::Available
    }

    fn start(
        &self,
        spec: SessionSpec,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Box<dyn ProviderSession>, StartError> {
        let InteractionSpec::Terminal(_) = spec.interaction;
        events.output(
            format!(
                "fake session {} in {}\r\n",
                spec.session_id,
                spec.workspace.display()
            )
            .as_bytes(),
        );
        Ok(Box::new(FakeSession {
            terminal: FakeTerminal {
                events,
                status: SessionStatus::Running,
            },
        }))
    }
}

struct FakeSession {
    terminal: FakeTerminal,
}

struct FakeTerminal {
    events: Arc<dyn SessionEvents>,
    status: SessionStatus,
}

impl FakeTerminal {
    /// Ends the session, reporting it exactly once.
    fn end(&mut self, end: SessionEnd) {
        if self.status.is_ended() {
            return;
        }
        self.status = end.status();
        self.events.ended(end);
    }

    fn ensure_running(&self) -> io::Result<()> {
        if self.status.is_ended() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the session has ended",
            ))
        } else {
            Ok(())
        }
    }
}

impl TerminalChannel for FakeTerminal {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.ensure_running()?;
        // Echo up to and including an EOT, then end: a real line discipline
        // would not echo what follows a hang-up either.
        match bytes.iter().position(|&b| b == EOT) {
            Some(i) => {
                if i > 0 {
                    self.events.output(&bytes[..i]);
                }
                self.end(SessionEnd::Completed { exit_code: Some(0) });
            }
            None => self.events.output(bytes),
        }
        Ok(())
    }

    fn resize(&mut self, size: TerminalSize) -> io::Result<()> {
        self.ensure_running()?;
        self.events
            .output(format!("[resize {}x{}]\r\n", size.cols(), size.rows()).as_bytes());
        Ok(())
    }
}

impl ProviderSession for FakeSession {
    fn interaction(&mut self) -> Interaction<'_> {
        Interaction::Terminal(&mut self.terminal)
    }

    fn stop(&mut self, _grace: Duration) {
        // Nothing to wait for: there is no process to give a grace period to.
        self.terminal.end(SessionEnd::Stopped);
    }

    fn status(&self) -> SessionStatus {
        self.terminal.status
    }
}
