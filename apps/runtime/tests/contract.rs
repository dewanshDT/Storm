//! The provider contract, driven the way a Runtime Host will drive it.
//!
//! The harness owns the scrollback and records endings, as the runtime does,
//! and reaches the provider only through `dyn Provider`. Nothing here knows the
//! provider is the fake. That is the point: if the launch → input → output →
//! end path works through the contract alone, the contract is not the PTY
//! (freeze §9.1, AC-A1's first half; the host link and the Agent Manager are
//! slice 4).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use storm_runtime::fake::FakeProvider;
use storm_runtime::provider::{
    Availability, Interaction, InteractionKind, InteractionSpec, Provider, ProviderKind,
    SessionEvents, SessionSpec, TerminalSize,
};
use storm_runtime::scrollback::{Read, Scrollback};
use storm_runtime::status::{SessionEnd, SessionStatus};

/// The runtime's side of a session: where its output lands and how it ended.
struct Recorder {
    scrollback: Mutex<Scrollback>,
    endings: Mutex<Vec<SessionEnd>>,
}

impl Recorder {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            scrollback: Mutex::new(Scrollback::new(1024)),
            endings: Mutex::new(Vec::new()),
        })
    }

    /// Everything from `offset` on, as text.
    fn output_from(&self, offset: u64) -> String {
        match self.scrollback.lock().unwrap().read(offset, usize::MAX) {
            Read::Data { bytes, .. } => String::from_utf8(bytes).unwrap(),
            Read::UpToDate => String::new(),
            other => panic!("unexpected read: {other:?}"),
        }
    }

    fn end_offset(&self) -> u64 {
        self.scrollback.lock().unwrap().end()
    }

    fn endings(&self) -> Vec<SessionEnd> {
        self.endings.lock().unwrap().clone()
    }
}

impl SessionEvents for Recorder {
    fn output(&self, bytes: &[u8]) {
        assert!(
            self.endings.lock().unwrap().is_empty(),
            "output after the session ended"
        );
        self.scrollback.lock().unwrap().append(bytes);
    }

    fn ended(&self, end: SessionEnd) {
        self.endings.lock().unwrap().push(end);
    }
}

fn spec() -> SessionSpec {
    SessionSpec {
        session_id: "ags_TEST".into(),
        workspace: PathBuf::from("/work/storm"),
        interaction: InteractionSpec::Terminal(TerminalSize::new(80, 24).unwrap()),
        launch: Default::default(),
    }
}

fn provider() -> Box<dyn Provider> {
    Box::new(FakeProvider::new())
}

fn write(session: &mut dyn storm_runtime::provider::ProviderSession, bytes: &[u8]) {
    let Interaction::Terminal(term) = session.interaction();
    term.write(bytes).unwrap();
}

#[test]
fn the_fake_describes_itself_through_the_contract() {
    let p = provider();
    assert_eq!(p.id().as_str(), "fake");
    assert_eq!(p.kind(), ProviderKind::Fake);
    assert_eq!(p.interactions(), &[InteractionKind::Terminal]);
    assert_eq!(p.available(), Availability::Available);
}

#[test]
fn launch_input_output_and_end_flow_through_the_contract_alone() {
    let rec = Recorder::new();
    let mut session = provider().start(spec(), rec.clone()).unwrap();
    assert_eq!(session.status(), SessionStatus::Running);

    // The start line proves the session id and workspace arrived.
    let banner = rec.output_from(0);
    assert_eq!(banner, "fake session ags_TEST in /work/storm\r\n");

    // Input comes back as output, at the next offset: a reader resuming from
    // where it stopped sees exactly the new bytes.
    let resume_at = rec.end_offset();
    write(session.as_mut(), b"echo hi\r");
    assert_eq!(rec.output_from(resume_at), "echo hi\r");

    // A resize is observable too.
    let resume_at = rec.end_offset();
    {
        let Interaction::Terminal(term) = session.interaction();
        term.resize(TerminalSize::new(120, 40).unwrap()).unwrap();
    }
    assert_eq!(rec.output_from(resume_at), "[resize 120x40]\r\n");

    // Ctrl-D ends it by itself: `completed`, exit 0, reported once.
    write(session.as_mut(), b"bye\x04ignored");
    assert_eq!(session.status(), SessionStatus::Completed);
    assert_eq!(
        rec.endings(),
        vec![SessionEnd::Completed { exit_code: Some(0) }]
    );
    assert!(rec.output_from(0).ends_with("bye"), "nothing after the EOT");
}

#[test]
fn stopping_reports_stopped_exactly_once() {
    let rec = Recorder::new();
    let mut session = provider().start(spec(), rec.clone()).unwrap();
    session.stop(Duration::from_secs(5));
    session.stop(Duration::from_secs(5));
    assert_eq!(session.status(), SessionStatus::Stopped);
    assert_eq!(rec.endings(), vec![SessionEnd::Stopped]);

    // An ending is final: a stop after a completion changes nothing.
    let rec = Recorder::new();
    let mut session = provider().start(spec(), rec.clone()).unwrap();
    write(session.as_mut(), b"\x04");
    session.stop(Duration::ZERO);
    assert_eq!(session.status(), SessionStatus::Completed);
    assert_eq!(rec.endings().len(), 1);
}

#[test]
fn an_ended_session_refuses_input_instead_of_dropping_it() {
    // The client never auto-retries input (freeze §11.3), so a write that went
    // nowhere must say so.
    let rec = Recorder::new();
    let mut session = provider().start(spec(), rec.clone()).unwrap();
    session.stop(Duration::ZERO);
    let Interaction::Terminal(term) = session.interaction();
    let err = term.write(b"late").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    let err = term.resize(TerminalSize::new(10, 10).unwrap()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
}
