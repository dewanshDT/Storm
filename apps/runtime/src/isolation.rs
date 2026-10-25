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

#[cfg(not(target_os = "linux"))]
mod imp {
    /// No isolation off Linux (136k): a host there never gets a link, so
    /// with isolation on it starts nothing.
    #[derive(Clone, Debug)]
    pub struct Link {
        _never: std::convert::Infallible,
    }

    impl Link {
        pub fn installed(_: &std::path::Path) -> Option<Self> {
            None
        }
    }

    #[derive(Clone)]
    pub struct Isolated {
        pub link: Link,
        pub provider: String,
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

    /// A connect or a request that takes longer than this is a helper that
    /// is not answering; only `end` waits longer, for the stop's grace.
    const IO_TIMEOUT: Duration = Duration::from_secs(10);
    const END_TIMEOUT: Duration = Duration::from_secs(30);
    /// The job unit's own limit is 900 s (the helper's `agents.rs`).
    const JOB_TIMEOUT: Duration = Duration::from_secs(960);
    const MAX_RESPONSE: usize = 16 * 1024;

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
            let wait = if request["op"] == "agent_job" {
                JOB_TIMEOUT
            } else if request["op"] == "end" || request["op"] == "release" {
                END_TIMEOUT
            } else {
                IO_TIMEOUT
            };
            stream.set_read_timeout(Some(wait))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            let mut line = request.to_string().into_bytes();
            line.push(b'\n');
            send_line(&stream, &line, fd)?;
            let (bytes, mut fds) = recv_line(&stream, MAX_RESPONSE)?;
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
        pub fn installed(state_dir: &Path) -> Option<Self> {
            if !cfg!(target_os = "linux") || state_dir != Path::new(crate::platform::DEFAULT_STATE)
            {
                return None;
            }
            Some(Self {
                helper: Arc::new(Socket(SOCKET.into())),
                state_dir: state_dir.to_path_buf(),
            })
        }

        pub fn launch_dir(&self) -> PathBuf {
            self.state_dir.join("launch")
        }

        fn ask(
            &self,
            request: Value,
            fd: Option<BorrowedFd<'_>>,
        ) -> io::Result<(Value, Option<OwnedFd>)> {
            let (answer, fd) = self.helper.call(&request, fd)?;
            if answer["ok"] != true {
                let code = answer["error"].as_str().unwrap_or("no answer");
                return Err(io::Error::other(format!(
                    "the isolation helper refused {}: {code}",
                    request["op"].as_str().unwrap_or("?")
                )));
            }
            Ok((answer, fd))
        }

        pub fn lease(&self, session: &str) -> io::Result<String> {
            let (answer, _) = self.ask(json!({"op": "lease", "session": session}), None)?;
            answer["user"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| io::Error::other("the helper leased no user"))
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
        }

        pub struct FakeHelper {
            pub socket: PathBuf,
            pub seen: Arc<Mutex<Seen>>,
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
                    _dir: dir,
                }
            }

            pub fn link(&self, state_dir: &Path) -> Link {
                Link {
                    helper: Arc::new(Socket(self.socket.clone())),
                    state_dir: state_dir.to_path_buf(),
                }
            }
        }

        fn serve(stream: UnixStream, seen: &Mutex<Seen>, state_dir: &Path, program: &str) {
            let Ok((line, mut fds)) = recv_line(&stream, 64 * 1024) else {
                return;
            };
            let req: Value = serde_json::from_slice(&line).unwrap();
            let op = req["op"].as_str().unwrap().to_string();
            let session = req["session"].as_str().unwrap_or("").to_string();
            let what = match (req["signal"].as_str(), req["agent"].as_str()) {
                (Some(signal), _) => format!("{op} {session} {signal}"),
                (None, Some(agent)) => {
                    format!("{op} {agent} {}", req["action"].as_str().unwrap_or(""))
                }
                (None, None) => format!("{op} {session}"),
            };
            seen.lock().unwrap().ops.push(what);
            let (answer, fd): (Value, Option<OwnedFd>) = match op.as_str() {
                "lease" => (json!({"ok": true, "user": "storm-s001"}), None),
                "prepare" | "release" => {
                    if op == "release" {
                        seen.lock().unwrap().released.push(session.clone());
                    }
                    (json!({"ok": true}), None)
                }
                "start" => {
                    let slave = fds.pop().expect("a terminal");
                    let path = state_dir.join("launch").join(format!("{session}.json"));
                    let launch: Value =
                        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                    seen.lock().unwrap().launches.push(launch.clone());
                    let mut cmd = std::process::Command::new(program);
                    cmd.args(
                        launch["args"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|a| a.as_str().unwrap()),
                    )
                    .env_clear()
                    .current_dir(req["workspace"].as_str().unwrap())
                    .stdin(slave.try_clone().unwrap())
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
                _ => (json!({"ok": false, "error": "not_built"}), None),
            };
            let mut line = answer.to_string().into_bytes();
            line.push(b'\n');
            let _ = send_line(&stream, &line, fd.as_ref().map(|f| f.as_fd()));
        }
    }
}
