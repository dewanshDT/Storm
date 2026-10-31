//! The host's side of per-session isolation (decision 136): with
//! `isolation = "per-session"`, a session is started by the root helper
//! `storm-runtime-isolation`, never by this process. The host leases a pool
//! user for it, keeps the terminal's master and hands the slave over as a
//! descriptor; the helper starts the session in its own transient unit and
//! hands back a pidfd for its leader. Ending it is the helper's too: it
//! stops the unit, which reaches everything the session started, and at
//! lease end hands back what the session's user wrote.
//!
//! The helper's wire format is its own crate's (`apps/runtime/isolation`,
//! `protocol.rs`): one JSON line each way, `SCM_RIGHTS` beside it. The two
//! crates share no code; `tests/isolation_wire.rs` parses every request this
//! client sends with the helper's own parser. Linux only (136k).

pub use imp::*;

/// A spawned child's workspace, as the helper made it (decision 137): its
/// own clone of the parent's `HEAD` (W2), or a scratch directory beside its
/// parent's workspace shown read-only (W3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildWorkspace {
    pub mode: ChildMode,
    /// Where the child works, at this same path in its own view.
    pub path: std::path::PathBuf,
    /// The commit a clone starts from.
    pub commit: Option<String>,
    /// The branch it is on; none when the parent's `HEAD` was detached.
    pub branch: Option<String>,
    /// Submodules and large-file pointers are not materialized (137b).
    pub submodules: bool,
    pub large_files: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildMode {
    Clone,
    Scratch,
}

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::peer_identity::{Report, State};

/// `isolation.mechanism` on the wire (136j): a leased pool user in a
/// transient systemd unit per session.
pub const MECHANISM: &str = "systemd_units";

/// A failed self-test is tried again no sooner than this, when a session
/// asks for one.
pub const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The host's isolation state (136j): `verified` only when isolation is on,
/// the helper answers, pasta works and the self-test passed. A helper that
/// can no longer be reached, or pasta gone, drops it to `unavailable` at
/// once; an operation that is only slow does not (139-D4). Only another
/// passing self-test brings it back.
pub struct Status {
    report: Mutex<Report>,
    /// When a failed self-test may run again.
    retry_at: Mutex<Option<Instant>>,
    /// Bumped on every change, so the host re-sends its inventory.
    changed: tokio::sync::watch::Sender<u64>,
}

impl Status {
    /// Before the self-test: what the platform and `runtime.toml` say.
    pub fn new(on: bool, helper: bool) -> Self {
        let unavailable = |mechanism, reason| Report {
            state: State::Unavailable,
            mechanism,
            reason: Some(reason),
        };
        let report = if !cfg!(target_os = "linux") {
            Report {
                state: State::Unsupported,
                mechanism: None,
                reason: Some(if cfg!(windows) { "windows" } else { "macos" }),
            }
        } else if !on {
            unavailable(None, "off")
        } else if !helper {
            unavailable(Some(MECHANISM), "no_helper")
        } else {
            unavailable(Some(MECHANISM), "pending")
        };
        Self {
            report: Mutex::new(report),
            retry_at: Mutex::new(None),
            changed: tokio::sync::watch::channel(0).0,
        }
    }

    pub fn report(&self) -> Report {
        self.report.lock().unwrap().clone()
    }

    pub fn verified(&self) -> bool {
        self.report.lock().unwrap().state == State::Verified
    }

    /// The self-test has not answered yet.
    pub fn pending(&self) -> bool {
        self.report.lock().unwrap().reason == Some("pending")
    }

    /// Whether a self-test may run now: one is due, or the last failed
    /// long enough ago. Claims the attempt.
    pub fn may_test(&self) -> bool {
        let report = self.report.lock().unwrap();
        if report.mechanism.is_none() || report.reason == Some("no_helper") {
            return false;
        }
        let mut at = self.retry_at.lock().unwrap();
        let due = at.is_none_or(|t| Instant::now() >= t);
        if due {
            *at = Some(Instant::now() + RETRY_AFTER);
        }
        due
    }

    /// A self-test's outcome.
    pub fn settle(&self, outcome: Result<(), &'static str>) {
        self.set(match outcome {
            Ok(()) => (State::Verified, None),
            Err(reason) => (State::Unavailable, Some(reason)),
        });
    }

    /// Something isolation needs stopped working. Whether it was verified.
    pub fn downgrade(&self, reason: &'static str) -> bool {
        if !self.verified() {
            return false;
        }
        self.set((State::Unavailable, Some(reason)));
        true
    }

    fn set(&self, (state, reason): (State, Option<&'static str>)) {
        let mut r = self.report.lock().unwrap();
        if (r.state, r.reason) == (state, reason) {
            return;
        }
        r.state = state;
        r.reason = reason;
        drop(r);
        self.changed.send_modify(|n| *n += 1);
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }

    #[cfg(test)]
    pub fn set_for_tests_pending(&self) {
        self.set((State::Unavailable, Some("pending")));
        *self.retry_at.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn before_the_self_test_the_platform_and_the_mode_say() {
        let r = Status::new(false, true).report();
        if cfg!(target_os = "linux") {
            assert_eq!(
                (r.state, r.mechanism, r.reason),
                (State::Unavailable, None, Some("off"))
            );
            let r = Status::new(true, false).report();
            assert_eq!(r.reason, Some("no_helper"));
            assert!(!Status::new(true, false).may_test());
            let s = Status::new(true, true);
            assert!(s.pending() && !s.verified());
            assert_eq!(s.report().mechanism, Some(MECHANISM));
        } else {
            assert_eq!(r.state, State::Unsupported);
        }
    }

    /// Linux only: elsewhere there is no isolation to test (136k).
    #[test]
    #[cfg(target_os = "linux")]
    fn only_a_passing_self_test_verifies_and_any_failure_drops_it() {
        let s = Status::new(true, true);
        let changes = s.subscribe();
        assert!(s.may_test());
        assert!(!s.may_test(), "once a minute");
        assert!(!s.downgrade("helper"), "nothing to drop while pending");
        s.settle(Ok(()));
        assert!(s.verified() && !s.pending());
        assert!(changes.has_changed().unwrap());
        assert!(s.downgrade("no_passt"));
        assert_eq!(
            s.report().to_json(),
            serde_json::json!({"state": "unavailable", "mechanism": MECHANISM, "reason": "no_passt"})
        );
        assert!(!s.downgrade("helper"), "the first reason stands");
        s.settle(Err("self_test"));
        assert_eq!(s.report().reason, Some("self_test"));
        s.settle(Ok(()));
        assert!(s.verified());
    }
}

/// A session's own bridge socket in its directory (136i), which the helper
/// shares with the session's group at `start`: `0660`, so that group can
/// connect and no one else.
#[cfg(unix)]
pub fn bind_session_socket(
    path: &std::path::Path,
) -> std::io::Result<std::os::unix::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(path);
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

#[cfg(not(target_os = "linux"))]
mod imp {
    /// No isolation off Linux (136k): a host there never gets a link, so
    /// with isolation on it starts nothing.
    #[derive(Clone, Debug)]
    pub struct Link {
        _never: std::convert::Infallible,
    }

    impl Link {
        pub fn installed(_: &std::path::Path, _: std::sync::Arc<super::Status>) -> Option<Self> {
            None
        }

        pub fn agent_job(
            &self,
            _: &str,
            _: &str,
        ) -> std::io::Result<Result<Option<String>, String>> {
            match self._never {}
        }

        pub fn status(&self) -> std::sync::Arc<super::Status> {
            match self._never {}
        }
    }

    #[derive(Clone)]
    pub struct Isolated {
        pub link: Link,
        pub provider: String,
    }

    impl Link {
        pub fn self_test(&self) -> Result<(), &'static str> {
            match self._never {}
        }

        pub fn child_create(
            &self,
            _: &str,
            _: &str,
            _: &std::path::Path,
        ) -> std::io::Result<super::ChildWorkspace> {
            match self._never {}
        }

        pub fn child_list(&self) -> std::io::Result<serde_json::Value> {
            match self._never {}
        }

        pub fn child_export(&self, _: &str) -> std::io::Result<serde_json::Value> {
            match self._never {}
        }

        pub fn child_import(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> std::io::Result<serde_json::Value> {
            match self._never {}
        }

        pub fn child_dispose(&self, _: &str) -> std::io::Result<serde_json::Value> {
            match self._never {}
        }
    }

    /// No helper off Linux, so never its refusal.
    pub fn refusal(_: &std::io::Error) -> Option<&str> {
        None
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io::{self, IoSlice, IoSliceMut};
    use std::mem::MaybeUninit;
    use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use rustix::net::{
        RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
        SendAncillaryMessage, SendFlags,
    };
    use serde_json::{Value, json};

    /// Where the helper's socket unit listens (136c).
    pub const SOCKET: &str = "/run/storm-runtime-isolation/helper.sock";
    /// Where the helper keeps sessions' homes: a self-test probe's
    /// workspace is its own home (136j).
    pub const HOMES: &str = "/var/lib/storm-runtime-isolation/homes";
    /// The helper's provider id for a self-test probe, which runs this
    /// host's own binary (`isolation-probe`).
    pub const PROBE: &str = "storm-probe";

    /// The helper refused an operation, with its code.
    #[derive(Debug)]
    pub struct Refused {
        pub op: String,
        pub code: String,
    }

    impl std::fmt::Display for Refused {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "the isolation helper refused {}: {}", self.op, self.code)
        }
    }

    impl std::error::Error for Refused {}

    /// The helper's code, when `e` is its refusal.
    pub fn refusal(e: &io::Error) -> Option<&str> {
        e.get_ref()
            .and_then(|e| e.downcast_ref::<Refused>())
            .map(|r| r.code.as_str())
    }

    /// Why a child's workspace could not be made (137f, 137h), for the
    /// child's `start_failed`: `workspace_cap`, `unreadable_repository`,
    /// `wrong_parent`, `child_exists`, … None when `e` is not the helper
    /// refusing `child_create`.
    pub fn child_failure(e: &io::Error) -> Option<&str> {
        e.get_ref()
            .and_then(|e| e.downcast_ref::<Refused>())
            .filter(|r| r.op == "child_create")
            .map(|r| r.code.as_str())
    }

    /// Why the self-test finds isolation cannot be had through the helper
    /// (136j): it does not answer, or no session could have a network.
    pub fn unavailable_because(e: &io::Error) -> Option<&'static str> {
        match refusal(e) {
            Some("no_passt") => Some("no_passt"),
            Some(_) => None,
            None => matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::PermissionDenied
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::WouldBlock
                    | io::ErrorKind::UnexpectedEof
            )
            .then_some("helper"),
        }
    }

    /// Whether an operation's failure says the helper cannot be reached
    /// at all (136j, 139-D4): it is not listening, or pasta is missing.
    /// One that was slow or cut short is only that operation's failure.
    fn unreachable_because(e: &io::Error) -> Option<&'static str> {
        match refusal(e) {
            Some("no_passt") => Some("no_passt"),
            Some(_) => None,
            None => matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::PermissionDenied
            )
            .then_some("helper"),
        }
    }

    /// A connect or a request that takes longer than this is a helper that
    /// is not answering; only `end` waits longer, for the stop's grace.
    const IO_TIMEOUT: Duration = Duration::from_secs(10);
    const END_TIMEOUT: Duration = Duration::from_secs(30);
    /// What the helper's units may run (`RuntimeMaxSec`): an install job
    /// 900 s (`agents.rs`), a child's clone, export or import 600 s
    /// (`children.rs`); then a minute for the helper's own work.
    const JOB_UNIT: u64 = 900;
    const CHILD_UNIT: u64 = 600;
    const MARGIN: u64 = 60;
    const MAX_RESPONSE: usize = 16 * 1024;
    const MAX_CHILD_RESPONSE: usize = 2 << 20;

    /// How long the host waits for `op`'s answer: a child's operation runs
    /// one unit with scans around it, and an Apply may run an export unit
    /// before its import unit.
    pub(crate) fn wait_for(op: &str) -> Duration {
        match op {
            "agent_job" => Duration::from_secs(JOB_UNIT + MARGIN),
            "child_import" => Duration::from_secs(2 * CHILD_UNIT + MARGIN),
            _ if op.starts_with("child_") => Duration::from_secs(CHILD_UNIT + 2 * MARGIN),
            "end" | "release" => END_TIMEOUT,
            _ => IO_TIMEOUT,
        }
    }

    /// One request to the helper, and its answer.
    pub trait Helper: Send + Sync {
        fn call(
            &self,
            request: &Value,
            fd: Option<BorrowedFd<'_>>,
        ) -> io::Result<(Value, Option<OwnedFd>)>;
    }

    /// The helper on its socket.
    pub struct Socket(pub PathBuf);

    impl Helper for Socket {
        fn call(
            &self,
            request: &Value,
            fd: Option<BorrowedFd<'_>>,
        ) -> io::Result<(Value, Option<OwnedFd>)> {
            let stream = UnixStream::connect(&self.0)?;
            let op = request["op"].as_str().unwrap_or_default();
            stream.set_read_timeout(Some(wait_for(op)))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            let mut line = request.to_string().into_bytes();
            line.push(b'\n');
            send_line(&stream, &line, fd)?;
            // An export carries the child's diff (decision 137e), bounded by
            // the helper; every other answer is small.
            let max = if op == "child_export" || op == "child_list" {
                MAX_CHILD_RESPONSE
            } else {
                MAX_RESPONSE
            };
            let (bytes, mut fds) = recv_line(&stream, max)?;
            let answer: Value = serde_json::from_slice(&bytes)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the helper's answer"))?;
            Ok((answer, fds.pop()))
        }
    }

    pub fn send_line(
        socket: impl AsFd,
        bytes: &[u8],
        fd: Option<BorrowedFd<'_>>,
    ) -> io::Result<()> {
        let socket = socket.as_fd();
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        let fds;
        if let Some(fd) = fd {
            fds = [fd];
            control.push(SendAncillaryMessage::ScmRights(&fds));
        }
        let mut sent = rustix::net::sendmsg(
            socket,
            &[IoSlice::new(bytes)],
            &mut control,
            SendFlags::NOSIGNAL,
        )?;
        while sent < bytes.len() {
            sent += rustix::net::send(socket, &bytes[sent..], SendFlags::NOSIGNAL)?;
        }
        Ok(())
    }

    /// A line and the descriptors that came with it, close-on-exec.
    pub fn recv_line(socket: impl AsFd, max: usize) -> io::Result<(Vec<u8>, Vec<OwnedFd>)> {
        let socket = socket.as_fd();
        let (mut line, mut fds) = (Vec::new(), Vec::new());
        let mut buf = [0u8; 4096];
        loop {
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
            let mut control = RecvAncillaryBuffer::new(&mut space);
            let got = rustix::net::recvmsg(
                socket,
                &mut [IoSliceMut::new(&mut buf)],
                &mut control,
                RecvFlags::CMSG_CLOEXEC,
            )?;
            for msg in control.drain() {
                if let RecvAncillaryMessage::ScmRights(received) = msg {
                    fds.extend(received);
                }
            }
            let chunk = &buf[..got.bytes];
            if let Some(end) = chunk.iter().position(|&b| b == b'\n') {
                line.extend_from_slice(&chunk[..end]);
                return Ok((line, fds));
            }
            line.extend_from_slice(chunk);
            if got.bytes == 0 || line.len() > max {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the helper's answer was cut short",
                ));
            }
        }
    }

    /// What a provider needs to start its sessions through the helper.
    #[derive(Clone)]
    pub struct Link {
        pub helper: Arc<dyn Helper>,
        /// The host's state directory: where it writes each session's
        /// launch for the helper to read.
        pub state_dir: PathBuf,
        /// [`HOMES`], or a test's.
        pub homes: PathBuf,
        /// What a helper that cannot be reached downgrades (136j).
        pub status: Arc<super::Status>,
    }

    impl std::fmt::Debug for Link {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Link")
                .field("state_dir", &self.state_dir)
                .finish_non_exhaustive()
        }
    }

    impl Link {
        /// The installed helper. On a host whose state directory is not the
        /// packaged one the helper would read the wrong launches, so there
        /// is none: with isolation on, that host starts nothing.
        pub fn installed(state_dir: &Path, status: Arc<super::Status>) -> Option<Self> {
            if !cfg!(target_os = "linux") || state_dir != Path::new(crate::platform::DEFAULT_STATE)
            {
                return None;
            }
            Some(Self {
                helper: Arc::new(Socket(SOCKET.into())),
                state_dir: state_dir.to_path_buf(),
                homes: HOMES.into(),
                status,
            })
        }

        pub fn launch_dir(&self) -> PathBuf {
            self.state_dir.join("launch")
        }

        pub fn status(&self) -> Arc<super::Status> {
            self.status.clone()
        }

        fn ask(
            &self,
            request: Value,
            fd: Option<BorrowedFd<'_>>,
        ) -> io::Result<(Value, Option<OwnedFd>)> {
            let answered = self.helper.call(&request, fd).and_then(|(answer, fd)| {
                if answer["ok"] != true {
                    return Err(io::Error::other(Refused {
                        op: request["op"].as_str().unwrap_or("?").to_string(),
                        code: answer["error"].as_str().unwrap_or("no answer").to_string(),
                    }));
                }
                Ok((answer, fd))
            });
            if let Err(e) = &answered
                && let Some(reason) = unreachable_because(e)
                && self.status.downgrade(reason)
            {
                tracing::error!(reason, error = %e, "session isolation is now unavailable");
            }
            answered
        }

        /// The leased user, and its uid: every bridge connection of the
        /// session must come from it (136i).
        pub fn lease(&self, session: &str) -> io::Result<(String, u32)> {
            let (answer, _) = self.ask(json!({"op": "lease", "session": session}), None)?;
            let user = answer["user"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| io::Error::other("the helper leased no user"))?;
            let uid = answer["uid"]
                .as_u64()
                .and_then(|u| u32::try_from(u).ok())
                .filter(|&u| u != 0)
                .ok_or_else(|| io::Error::other("the helper named no uid for the lease"))?;
            Ok((user, uid))
        }

        /// The helper's half of the self-test (136j): it answers, and pasta
        /// is there to give a session its network.
        pub fn ready(&self) -> io::Result<()> {
            self.ask(json!({"op": "self_test"}), None).map(drop)
        }

        /// The whole self-test: the helper's half, then two probe sessions
        /// (`crate::isolation_probe`). The reason when it fails.
        pub fn self_test(&self) -> Result<(), &'static str> {
            crate::isolation_probe::self_test(self)
        }

        pub fn prepare(&self, session: &str, user: &str) -> io::Result<()> {
            self.ask(
                json!({"op": "prepare", "session": session, "user": user}),
                None,
            )
            .map(drop)
        }

        /// Returns the leader's pid and its pidfd; none when it had already
        /// exited.
        pub fn start(
            &self,
            session: &str,
            user: &str,
            provider: &str,
            workspace: &Path,
            terminal: BorrowedFd<'_>,
        ) -> io::Result<(i32, Option<OwnedFd>)> {
            let workspace = workspace
                .to_str()
                .ok_or_else(|| io::Error::other("the workspace path is not UTF-8"))?;
            let (answer, pidfd) = self.ask(
                json!({"op": "start", "session": session, "user": user, "provider": provider, "workspace": workspace}),
                Some(terminal),
            )?;
            let pid = answer["pid"].as_i64().unwrap_or(0) as i32;
            Ok((pid, pidfd.filter(|_| pid > 0)))
        }

        pub fn signal(&self, session: &str, signal: &str) -> io::Result<()> {
            self.ask(
                json!({"op": "signal", "session": session, "signal": signal}),
                None,
            )
            .map(drop)
        }

        /// Stops the session's unit; how its main process ended.
        pub fn end(&self, session: &str) -> io::Result<(Option<i32>, Option<i32>)> {
            let (answer, _) = self.ask(json!({"op": "end", "session": session}), None)?;
            let n = |k: &str| answer[k].as_i64().map(|v| v as i32);
            Ok((n("exit_code"), n("signal")))
        }

        /// An install job under the install identity (136g): the version
        /// now installed (none after a remove), or the job's failure code.
        pub fn agent_job(
            &self,
            agent: &str,
            action: &str,
        ) -> io::Result<Result<Option<String>, String>> {
            let (answer, _) = self.ask(
                json!({"op": "agent_job", "agent": agent, "action": action}),
                None,
            )?;
            if let Some(code) = answer["failed"].as_str() {
                return Ok(Err(code.to_string()));
            }
            Ok(Ok(answer["version"].as_str().map(str::to_string)))
        }

        /// Makes `child`'s workspace from `parent`'s, which is `workspace`
        /// (decision 137): after `child`'s lease and `prepare`, before its
        /// `start`. Asked again for the same parent, it answers the same.
        pub fn child_create(
            &self,
            parent: &str,
            child: &str,
            workspace: &Path,
        ) -> io::Result<super::ChildWorkspace> {
            let workspace = workspace
                .to_str()
                .ok_or_else(|| io::Error::other("the workspace path is not UTF-8"))?;
            let (answer, _) = self.ask(
                json!({"op": "child_create", "parent": parent, "child": child, "workspace": workspace}),
                None,
            )?;
            let mode = match answer["mode"].as_str() {
                Some("clone") => super::ChildMode::Clone,
                Some("scratch") => super::ChildMode::Scratch,
                _ => return Err(io::Error::other("the helper named no workspace mode")),
            };
            let path = answer["workspace"]
                .as_str()
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .ok_or_else(|| io::Error::other("the helper named no child workspace"))?;
            let text = |k: &str| answer[k].as_str().map(str::to_string);
            Ok(super::ChildWorkspace {
                mode,
                path,
                commit: text("commit"),
                branch: text("branch"),
                submodules: answer["submodules"] == true,
                large_files: answer["large_files"] == true,
            })
        }

        /// The helper's answer, without its `ok` and `op`.
        fn answer(&self, request: Value) -> io::Result<Value> {
            let (mut answer, _) = self.ask(request, None)?;
            if let Some(map) = answer.as_object_mut() {
                map.remove("ok");
                map.remove("op");
            }
            Ok(answer)
        }

        /// Every child workspace the helper keeps, and the orphans
        /// (decision 137g).
        pub fn child_list(&self) -> io::Result<Value> {
            self.answer(json!({"op": "child_list"}))
        }

        /// An ended child's work as data (137e): its commits and diff, or
        /// its scratch files.
        pub fn child_export(&self, child: &str) -> io::Result<Value> {
            self.answer(json!({"op": "child_export", "child": child}))
        }

        /// Apply (137e): `storm/<child>`, or a new directory `target`.
        pub fn child_import(&self, child: &str, parent: &str, target: &str) -> io::Result<Value> {
            self.answer(
                json!({"op": "child_import", "child": child, "parent": parent, "target": target}),
            )
        }

        /// A person's delete of a child's workspace or an orphan (137g).
        pub fn child_dispose(&self, child: &str) -> io::Result<Value> {
            self.answer(json!({"op": "child_dispose", "child": child}))
        }

        pub fn release(&self, session: &str, user: &str) -> io::Result<()> {
            self.ask(
                json!({"op": "release", "session": session, "user": user}),
                None,
            )
            .map(drop)
        }
    }

    /// A provider's sessions run isolated through `link`.
    #[derive(Clone)]
    pub struct Isolated {
        pub link: Link,
        pub provider: String,
    }

    /// A test helper that runs sessions as this account, on the terminal it
    /// is handed, speaking the helper's wire format over a real socket.
    #[cfg(test)]
    pub mod fake {
        use super::*;
        use std::collections::HashMap;
        use std::os::unix::net::UnixListener;
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        use std::sync::Mutex;

        #[derive(Default)]
        pub struct Seen {
            pub ops: Vec<String>,
            pub launches: Vec<Value>,
            pub children: HashMap<String, std::process::Child>,
            pub released: Vec<String>,
            /// The uid each lease names; this account's by default, so a
            /// session's own connections pass 136i's user check.
            pub lease_uid: Option<u32>,
            /// As if pasta were missing (IT-7).
            pub no_passt: bool,
            /// What `child_create` is refused with, if it is.
            pub child_refusal: Option<String>,
            /// What `end` is refused with, if it is.
            pub end_refusal: Option<String>,
            /// Where `child_create` really makes a child's workspace, with
            /// real git as this account; unset, it only answers.
            pub children_root: Option<PathBuf>,
            /// Each child's workspace as made, which its `start` runs in, as
            /// the helper's unit's `WorkingDirectory=` is (137-1).
            pub made: HashMap<String, PathBuf>,
            /// The helper's record of each child made under
            /// `children_root`, for its listing, export and Apply (137-3).
            pub records: HashMap<String, Record>,
        }

        pub struct Record {
            pub parent: String,
            pub parent_workspace: PathBuf,
            pub made: Value,
            pub ended: bool,
            pub export: Option<serde_json::Map<String, Value>>,
            pub applied: Option<String>,
        }

        pub struct FakeHelper {
            pub socket: PathBuf,
            pub seen: Arc<Mutex<Seen>>,
            /// Every link's, so a host and its providers share it.
            pub status: Arc<crate::isolation::Status>,
            _dir: tempfile::TempDir,
        }

        impl FakeHelper {
            /// `program` is what every provider runs here.
            pub fn start(state_dir: &Path, program: &str) -> Self {
                let dir = tempfile::tempdir().unwrap();
                let socket = dir.path().join("helper.sock");
                let listener = UnixListener::bind(&socket).unwrap();
                let seen = Arc::new(Mutex::new(Seen::default()));
                let (s, state_dir, program) =
                    (seen.clone(), state_dir.to_path_buf(), program.to_string());
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(stream) = stream else { return };
                        let (s, state_dir, program) =
                            (s.clone(), state_dir.clone(), program.clone());
                        std::thread::spawn(move || serve(stream, &s, &state_dir, &program));
                    }
                });
                Self {
                    socket,
                    seen,
                    status: Arc::new(crate::isolation::Status::new(true, true)),
                    _dir: dir,
                }
            }

            pub fn link(&self, state_dir: &Path) -> Link {
                Link {
                    helper: Arc::new(Socket(self.socket.clone())),
                    state_dir: state_dir.to_path_buf(),
                    homes: self._dir.path().join("homes"),
                    status: self.status.clone(),
                }
            }
        }

        /// What the helper's `child_create` does, as this account: W2, a
        /// clone of a repository root's `HEAD` branch through git's own
        /// transport with nothing pointing back; W3 otherwise, an empty
        /// scratch directory.
        fn make_child(parent: &Path, dest: &Path) -> Value {
            std::fs::create_dir_all(dest).unwrap();
            if !parent.join(".git").is_dir() {
                return json!({"ok": true, "mode": "scratch", "workspace": dest, "commit": null, "branch": null, "submodules": false, "large_files": false});
            }
            let git = |dir: Option<&Path>, args: &[&str]| {
                let mut c = std::process::Command::new("git");
                if let Some(dir) = dir {
                    c.arg("-C").arg(dir);
                }
                let out = c
                    .args([
                        "-c",
                        "core.hooksPath=/dev/null",
                        "-c",
                        "protocol.file.allow=always",
                    ])
                    .args(args)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "git {args:?}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                String::from_utf8(out.stdout).unwrap().trim().to_string()
            };
            let source = format!("file://{}", parent.display());
            git(
                None,
                &[
                    "clone",
                    "--no-local",
                    "--single-branch",
                    "--no-tags",
                    "--origin",
                    "storm-parent",
                    "--",
                    &source,
                    dest.to_str().unwrap(),
                ],
            );
            git(Some(dest), &["remote", "remove", "storm-parent"]);
            let commit = git(Some(dest), &["rev-parse", "HEAD"]);
            let branch = git(Some(dest), &["symbolic-ref", "--short", "HEAD"]);
            json!({"ok": true, "mode": "clone", "workspace": dest, "commit": commit, "branch": branch, "submodules": false, "large_files": false})
        }

        /// 137-3's listing, export, Apply and delete for children made
        /// under `children_root`: the helper's own export and Apply code
        /// (`storm_runtime_isolation::review`) with real git, as this
        /// account rather than `storm-review` in a unit.
        fn review(op: &str, req: &Value, seen: &Mutex<Seen>) -> (Value, Option<OwnedFd>) {
            use storm_runtime_isolation::{clone, review};
            let refuse = |code: &str| (json!({"ok": false, "error": code}), None);
            let mut seen = seen.lock().unwrap();
            let root = seen.children_root.clone().unwrap();
            if op == "child_list" {
                let children: Vec<Value> = seen
                    .records
                    .iter()
                    .map(|(child, r)| {
                        json!({"child": child, "parent": r.parent,
                            "parent_workspace": r.parent_workspace.display().to_string(),
                            "mode": r.made["mode"], "stage": if r.ended { "ended" } else { "ready" },
                            "live": !r.ended, "commit": r.made["commit"], "branch": r.made["branch"],
                            "applied": r.applied})
                    })
                    .collect();
                return (
                    json!({"ok": true, "children": children, "orphans": [], "unknown_entries": 0}),
                    None,
                );
            }
            let child = req["child"].as_str().unwrap().to_string();
            if op == "child_dispose" {
                let removed = seen.records.remove(&child).is_some();
                let _ = std::fs::remove_dir_all(root.join(&child));
                return (
                    json!({"ok": true, "removed": removed, "orphan": false}),
                    None,
                );
            }
            let Some(r) = seen.records.get_mut(&child) else {
                return refuse("no_child");
            };
            if !r.ended {
                return refuse("child_live");
            }
            let (git, home) = (Path::new(clone::GIT), Path::new(clone::HOME));
            let tmp = std::env::temp_dir();
            let dir = root.join(&child).join("review");
            if r.export.is_none() {
                std::fs::create_dir_all(&dir).unwrap();
                let workspace = PathBuf::from(r.made["workspace"].as_str().unwrap());
                let base = r.made["commit"].as_str().unwrap_or("-");
                match review::export_clone(git, &workspace, &dir, base, 1 << 30, home, &tmp) {
                    Ok(Value::Object(v)) => r.export = Some(v),
                    _ => return refuse("failed"),
                }
            }
            let mut answer = r.export.clone().unwrap();
            if op == "child_export" {
                answer.insert("child".into(), child.clone().into());
                answer.insert("parent".into(), r.parent.clone().into());
                answer.insert(
                    "parent_workspace".into(),
                    r.parent_workspace.display().to_string().into(),
                );
                answer.insert("ok".into(), true.into());
                return (Value::Object(answer), None);
            }
            if req["parent"] != r.parent.as_str() {
                return refuse("wrong_parent");
            }
            let branch = format!("storm/{child}");
            if req["target"] != branch.as_str() {
                return refuse("bad_value");
            }
            if r.applied.is_some() {
                return refuse("branch_exists");
            }
            let tip = answer["tip"].as_str().unwrap().to_string();
            let bundle = dir.join(review::BUNDLE);
            match review::import_clone(git, &r.parent_workspace, &bundle, &branch, &tip, home, &tmp)
            {
                Ok(v) if v["already"] == false => {
                    r.applied = Some(branch.clone());
                    (
                        json!({"ok": true, "branch": branch, "commit": v["commit"], "child": child}),
                        None,
                    )
                }
                Ok(_) => refuse("branch_exists"),
                Err(f) => refuse(f.code()),
            }
        }

        fn serve(stream: UnixStream, seen: &Mutex<Seen>, state_dir: &Path, program: &str) {
            let Ok((line, mut fds)) = recv_line(&stream, 64 * 1024) else {
                return;
            };
            let req: Value = serde_json::from_slice(&line).unwrap();
            let op = req["op"].as_str().unwrap().to_string();
            let session = req["session"]
                .as_str()
                .or(req["child"].as_str())
                .unwrap_or("")
                .to_string();
            let what = match (req["signal"].as_str(), req["agent"].as_str()) {
                (Some(signal), _) => format!("{op} {session} {signal}"),
                (None, Some(agent)) => {
                    format!("{op} {agent} {}", req["action"].as_str().unwrap_or(""))
                }
                (None, None) => format!("{op} {session}"),
            };
            seen.lock().unwrap().ops.push(what);
            let refuse = |code: &str| (json!({"ok": false, "error": code}), None);
            let (answer, fd): (Value, Option<OwnedFd>) = match op.as_str() {
                "lease" => {
                    let uid = seen
                        .lock()
                        .unwrap()
                        .lease_uid
                        .unwrap_or_else(|| rustix::process::getuid().as_raw());
                    (json!({"ok": true, "user": "storm-s001", "uid": uid}), None)
                }
                "self_test" | "start" if seen.lock().unwrap().no_passt => refuse("no_passt"),
                "self_test" => (json!({"ok": true}), None),
                "prepare" | "release" => {
                    if op == "release" {
                        let mut seen = seen.lock().unwrap();
                        seen.released.push(session.clone());
                        // Lease end: the child's workspace is kept, for its
                        // review (137g).
                        if let Some(r) = seen.records.get_mut(&session) {
                            r.ended = true;
                        }
                    }
                    (json!({"ok": true}), None)
                }
                "start" => {
                    let slave = fds.pop().expect("a terminal");
                    let path = state_dir.join("launch").join(format!("{session}.json"));
                    let launch: Value =
                        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                    seen.lock().unwrap().launches.push(launch.clone());
                    let args = launch["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_string());
                    // A probe is this test binary's `probe_entry`, given the
                    // arguments the host would give `storm-runtime`.
                    let probe = req["provider"] == PROBE;
                    let mut cmd = if probe {
                        let mut c = std::process::Command::new(std::env::current_exe().unwrap());
                        c.args([
                            "--exact",
                            "isolation_probe::tests::probe_entry",
                            "--nocapture",
                            "-q",
                            "--test-threads=1",
                        ]);
                        c
                    } else {
                        let mut c = std::process::Command::new(program);
                        c.args(args.clone());
                        c
                    };
                    let made = seen.lock().unwrap().made.get(&session).cloned();
                    let workspace = if probe {
                        let home = PathBuf::from(req["workspace"].as_str().unwrap());
                        std::fs::create_dir_all(&home).unwrap();
                        home
                    } else if let Some(made) = made {
                        made
                    } else {
                        PathBuf::from(req["workspace"].as_str().unwrap())
                    };
                    cmd.env_clear().current_dir(&workspace);
                    if probe {
                        cmd.env(
                            "STORM_ISOLATION_PROBE",
                            serde_json::to_string(&args.collect::<Vec<_>>()).unwrap(),
                        )
                        .env("HOME", &workspace);
                    }
                    cmd.stdin(slave.try_clone().unwrap())
                        .stdout(slave.try_clone().unwrap())
                        .stderr(slave);
                    for pair in launch["env"].as_array().unwrap() {
                        cmd.env(pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
                    }
                    // SAFETY: two raw syscalls in the child before exec.
                    unsafe {
                        cmd.pre_exec(|| {
                            rustix::process::setsid()?;
                            rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
                            Ok(())
                        });
                    }
                    let Ok(child) = cmd.spawn() else {
                        let line = b"{\"ok\":false,\"error\":\"provider_unavailable\"}\n";
                        let _ = send_line(&stream, line, None);
                        return;
                    };
                    let pid = child.id();
                    let pidfd = rustix::process::pidfd_open(
                        rustix::process::Pid::from_child(&child),
                        rustix::process::PidfdFlags::empty(),
                    )
                    .unwrap();
                    seen.lock().unwrap().children.insert(session.clone(), child);
                    (json!({"ok": true, "pid": pid}), Some(pidfd))
                }
                "signal" => {
                    let sig = match req["signal"].as_str().unwrap() {
                        "hup" => rustix::process::Signal::HUP,
                        "kill" => rustix::process::Signal::KILL,
                        "int" => rustix::process::Signal::INT,
                        _ => rustix::process::Signal::TERM,
                    };
                    let pid = seen.lock().unwrap().children.get(&session).map(|c| c.id());
                    match pid {
                        Some(pid) => {
                            let _ = rustix::process::kill_process_group(
                                rustix::process::Pid::from_raw(pid as i32).unwrap(),
                                sig,
                            );
                            (json!({"ok": true}), None)
                        }
                        None => (json!({"ok": false, "error": "not_started"}), None),
                    }
                }
                "end" if seen.lock().unwrap().end_refusal.is_some() => {
                    refuse(&seen.lock().unwrap().end_refusal.clone().unwrap())
                }
                "end" => {
                    let child = seen.lock().unwrap().children.remove(&session);
                    match child {
                        Some(mut child) => {
                            let status = child.wait().unwrap();
                            let _ = rustix::process::kill_process_group(
                                rustix::process::Pid::from_raw(child.id() as i32).unwrap(),
                                rustix::process::Signal::KILL,
                            );
                            (
                                json!({"ok": true, "exit_code": status.code(), "signal": status.signal()}),
                                None,
                            )
                        }
                        None => (json!({"ok": false, "error": "not_started"}), None),
                    }
                }
                "agent_job" => (json!({"ok": true, "version": "9.9.9"}), None),
                "child_create" => {
                    let (refusal, root) = {
                        let seen = seen.lock().unwrap();
                        (seen.child_refusal.clone(), seen.children_root.clone())
                    };
                    match (refusal, root) {
                        (Some(code), _) => refuse(&code),
                        (None, Some(root)) => {
                            let parent = Path::new(req["workspace"].as_str().unwrap());
                            let made = make_child(parent, &root.join(&session).join("workspace"));
                            let mut seen = seen.lock().unwrap();
                            seen.made.insert(
                                session.clone(),
                                PathBuf::from(made["workspace"].as_str().unwrap()),
                            );
                            seen.records.insert(
                                session.clone(),
                                Record {
                                    parent: req["parent"].as_str().unwrap().to_string(),
                                    parent_workspace: parent.to_path_buf(),
                                    made: made.clone(),
                                    ended: false,
                                    export: None,
                                    applied: None,
                                },
                            );
                            (made, None)
                        }
                        (None, None) => (
                            json!({"ok": true, "mode": "clone", "workspace": format!("/var/lib/storm-runtime-isolation/children/{session}/workspace"), "commit": "a".repeat(40), "branch": "main", "submodules": false, "large_files": true}),
                            None,
                        ),
                    }
                }
                "child_list" | "child_export" | "child_import" | "child_dispose"
                    if seen.lock().unwrap().children_root.is_some() =>
                {
                    review(&op, &req, seen)
                }
                "child_list" => (
                    json!({"ok": true, "op": "child_list", "children": [{"child": "ags_C", "parent": "ags_P", "mode": "clone", "stage": "ended", "live": false}], "orphans": ["ags_LOST"], "unknown_entries": 0}),
                    None,
                ),
                // A whole diff's worth, so the answer is larger than any
                // other the helper gives.
                "child_export" if req["child"] == "ags_C" => (
                    json!({"ok": true, "op": "child_export", "child": "ags_C", "mode": "clone", "tip": "a".repeat(40), "patch": "+".repeat(200 << 10)}),
                    None,
                ),
                "child_import" if req["child"] == "ags_C" => (
                    json!({"ok": true, "op": "child_import", "child": "ags_C", "branch": req["target"], "commit": "a".repeat(40)}),
                    None,
                ),
                "child_dispose" => (
                    json!({"ok": true, "op": "child_dispose", "removed": true}),
                    None,
                ),
                "child_export" | "child_import" => refuse("no_child"),
                _ => (json!({"ok": false, "error": "not_built"}), None),
            };
            let mut line = answer.to_string().into_bytes();
            line.push(b'\n');
            let _ = send_line(&stream, &line, fd.as_ref().map(|f| f.as_fd()));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A helper that fails every request with `kind`, as one that is
        /// slow, cut short or gone does.
        struct Failing(io::ErrorKind);

        impl Helper for Failing {
            fn call(
                &self,
                request: &Value,
                _: Option<BorrowedFd<'_>>,
            ) -> io::Result<(Value, Option<OwnedFd>)> {
                Err(io::Error::new(self.0, format!("{}", request["op"])))
            }
        }

        fn verified_link(kind: io::ErrorKind) -> Link {
            let status = Arc::new(super::super::Status::new(true, true));
            status.settle(Ok(()));
            Link {
                helper: Arc::new(Failing(kind)),
                state_dir: "/nonexistent/state".into(),
                homes: "/nonexistent/homes".into(),
                status,
            }
        }

        /// 139-D4: an operation that outlasts its wait is its caller's
        /// error; only a failed verification check (the self-test, or a
        /// helper that cannot be reached at all) changes isolation's state.
        #[test]
        fn a_slow_child_operation_does_not_make_isolation_unavailable() {
            use io::ErrorKind::*;
            for kind in [TimedOut, WouldBlock, UnexpectedEof] {
                let link = verified_link(kind);
                let e = link
                    .child_import("ags_C", "ags_P", "storm/ags_C")
                    .unwrap_err();
                assert_eq!(e.kind(), kind);
                assert!(link.agent_job("opencode", "install").is_err());
                assert!(link.child_export("ags_C").is_err());
                assert!(
                    link.child_create("ags_P", "ags_C", Path::new("/w"))
                        .is_err()
                );
                assert!(link.lease("ags_A").is_err());
                assert!(
                    link.status.verified(),
                    "{kind:?} made isolation unavailable"
                );
                link.status.settle(link.self_test());
                assert_eq!(link.status.report().reason, Some("helper"), "{kind:?}");
            }
            for kind in [NotFound, ConnectionRefused, PermissionDenied] {
                let link = verified_link(kind);
                assert!(link.child_list().is_err());
                assert!(!link.status.verified(), "{kind:?}");
                assert_eq!(link.status.report().reason, Some("helper"));
            }
        }

        /// Each operation waits as long as the helper's units for it may
        /// run, and a minute more.
        #[test]
        fn each_operation_waits_as_long_as_its_units_may_run() {
            let secs = |op| wait_for(op).as_secs();
            assert_eq!(secs("child_import"), 2 * 600 + 60);
            for op in [
                "child_create",
                "child_export",
                "child_list",
                "child_dispose",
            ] {
                assert_eq!(secs(op), 660 + 60, "{op}");
            }
            assert_eq!(secs("agent_job"), 900 + 60);
            for op in ["end", "release"] {
                assert_eq!(secs(op), 30, "{op}");
            }
            for op in ["lease", "prepare", "start", "signal", "self_test"] {
                assert_eq!(secs(op), 10, "{op}");
            }
        }

        /// The helper fits a child's answer in its budget, which must fit
        /// this read (139-D2); the crates share no code, so both pin it.
        #[test]
        fn the_helpers_answer_budget_fits_the_hosts_read() {
            const HELPERS_BUDGET: usize = (2 << 20) - (64 << 10);
            assert_eq!(
                storm_runtime_isolation::review::CHILD_ANSWER_BUDGET,
                HELPERS_BUDGET
            );
            assert_eq!(MAX_CHILD_RESPONSE, 2 << 20);
            let (a, b) = UnixStream::pair().unwrap();
            let pad = HELPERS_BUDGET - r#"{"ok":true,"p":""}"#.len() - 1;
            let line = format!("{{\"ok\":true,\"p\":\"{}\"}}\n", "x".repeat(pad));
            assert_eq!(line.len(), HELPERS_BUDGET);
            let writer = std::thread::spawn(move || send_line(&a, line.as_bytes(), None));
            let (got, _) = recv_line(&b, MAX_CHILD_RESPONSE).unwrap();
            writer.join().unwrap().unwrap();
            assert_eq!(got.len(), HELPERS_BUDGET - 1);
        }
    }
}
