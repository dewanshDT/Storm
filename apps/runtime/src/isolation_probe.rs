//! The isolation self-test (decision 136j): before an isolated host reports
//! `isolation: verified`, two probe sessions, A and B, run through the same
//! helper, lease, unit and network as every session, and B tries to reach
//! everything A opened. `verified` needs every attempt refused (AT-1, AT-2,
//! AT-5, AT-11 and a sample of AT-3) and every one of each probe's own
//! controls to work, so a probe that reaches nothing because it is broken
//! fails too.
//!
//! A probe is this host's own binary, `storm-runtime isolation-probe`: the
//! helper runs it for its `storm-probe` provider, in the probe's own home,
//! never in a workspace. A holds its endpoints open and prints where they
//! are; B is given that and prints what it reached. Each prints one line
//! after [`MARK`] on its terminal.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What a probe's one answer follows on its terminal.
pub const MARK: &str = "STORM-PROBE ";

/// What A opened, for B to try.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoints {
    pub port: u16,
    pub abstract_name: String,
    pub tmp: PathBuf,
    pub shm: PathBuf,
    pub sysv_key: i32,
    pub marker: PathBuf,
}

/// One probe's answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub uid: u32,
    pub pid: i32,
    pub cgroup: String,
    #[serde(default)]
    pub endpoints: Option<Endpoints>,
    /// Its own controls: each must work.
    pub own: BTreeMap<String, bool>,
    /// What it reached of the other probe's: each must not.
    #[serde(default)]
    pub reached: BTreeMap<String, bool>,
}

/// A's controls: it reaches what it opened.
pub const HOLD_OWN: &[&str] = &["abstract", "tcp", "tmp", "shm", "sysv", "home", "bridge"];
/// B's controls: its own bridge socket and home.
pub const REACH_OWN: &[&str] = &["bridge", "home"];
/// What B tries of A's, and which test each is.
pub const REACHED: &[(&str, &str)] = &[
    ("proc", "AT-2"),
    ("environ", "AT-2"),
    ("signal", "AT-2"),
    ("state", "AT-5"),
    ("bridge", "AT-5"),
    ("home", "AT-6"),
    ("abstract", "AT-11"),
    ("tcp", "AT-11"),
    ("tmp", "AT-11"),
    ("shm", "AT-11"),
    ("sysv", "AT-11"),
    ("agents_write", "AT-3"),
    ("system_write", "AT-3"),
];

/// Who connected to each probe's bridge socket, as the kernel said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Connections {
    pub a: Vec<u32>,
    pub b: Vec<u32>,
}

/// Whether the two answers show isolation; the first property that does
/// not hold otherwise.
pub fn evaluate(
    host_uid: u32,
    (a_id, a): (&str, &Report),
    (b_id, b): (&str, &Report),
    seen: &Connections,
) -> Result<(), String> {
    let all = |m: &BTreeMap<String, bool>, keys: &[&str], want: bool| {
        keys.iter().all(|k| m.get(*k) == Some(&want))
    };
    if [a.uid, b.uid].iter().any(|&u| u == 0 || u == host_uid) || a.uid == b.uid {
        return Err("AT-1: the probes ran as one identity, or as the host's".into());
    }
    for (id, r) in [(a_id, a), (b_id, b)] {
        if !r.cgroup.ends_with(&format!("/storm-session-{id}.service")) {
            return Err(format!("AT-1: {id} is not in its own unit"));
        }
    }
    if !all(&a.own, HOLD_OWN, true) || a.endpoints.is_none() {
        return Err("control: A could not reach what it opened".into());
    }
    if !all(&b.own, REACH_OWN, true) {
        return Err("control: B could not reach its own bridge or home".into());
    }
    for (what, test) in REACHED {
        if b.reached.get(*what) != Some(&false) {
            return Err(format!("{test}: B reached A's {what}"));
        }
    }
    // 136i, live: each socket took its own session's user, and only it.
    for (uids, own) in [(&seen.a, a.uid), (&seen.b, b.uid)] {
        if uids.is_empty() || uids.iter().any(|&u| u != own) {
            return Err("136i: a bridge socket took another user's connection".into());
        }
    }
    Ok(())
}

pub use imp::*;

#[cfg(not(target_os = "linux"))]
mod imp {
    /// No isolation off Linux (136k).
    pub fn self_test(_: &crate::isolation::Link) -> Result<(), &'static str> {
        Err("unsupported")
    }

    pub fn main(_: &[String]) -> i32 {
        2
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::BTreeMap;
    use std::io::{BufRead, Write};
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use super::{Connections, Endpoints, MARK, Report, evaluate};
    use crate::isolation::{Isolated, Link, PROBE, bind_session_socket, unavailable_because};

    /// How long a probe may take to answer.
    const ANSWER: Duration = Duration::from_secs(30);

    // ---- the probe itself, inside its unit ------------------------------

    /// `storm-runtime isolation-probe hold|reach …`: prints its answer and,
    /// for `hold`, keeps its endpoints open until it is ended.
    pub fn main(args: &[String]) -> i32 {
        let report = match args {
            [verb, tag, bridge] if verb == "hold" => hold(tag, Path::new(bridge)),
            [verb, bridge, a, a_bridge, a_state, agents] if verb == "reach" => {
                let Ok(a) = serde_json::from_str::<Report>(a) else {
                    return 2;
                };
                reach(
                    Path::new(bridge),
                    &a,
                    Path::new(a_bridge),
                    Path::new(a_state),
                    Path::new(agents),
                )
            }
            _ => return 2,
        };
        let Ok((report, keep)) = report else {
            return 1;
        };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(
            out,
            "{MARK}{}",
            serde_json::to_string(&report).unwrap_or_default()
        );
        let _ = out.flush();
        drop(out);
        if keep.is_some() {
            // Held until the hang-up that ends it; then what it made goes,
            // as a unit's end would take it.
            // SAFETY: the handler only stores to an atomic.
            unsafe { libc::signal(libc::SIGHUP, on_hangup as *const () as libc::sighandler_t) };
            while !HUNG_UP.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
            }
            if let Some(e) = &report.endpoints {
                let _ = std::fs::remove_file(&e.tmp);
                let _ = std::fs::remove_file(&e.shm);
                let _ = std::fs::remove_file(&e.marker);
                // SAFETY: plain syscalls; the only pointer is null.
                unsafe {
                    let id = libc::shmget(e.sysv_key, 0, 0);
                    if id >= 0 {
                        libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                    }
                }
            }
        }
        0
    }

    static HUNG_UP: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_hangup(_: libc::c_int) {
        HUNG_UP.store(true, Ordering::SeqCst);
    }

    fn me() -> (u32, i32, String) {
        let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
        (
            rustix::process::getuid().as_raw(),
            std::process::id() as i32,
            cgroup.lines().last().unwrap_or("").to_string(),
        )
    }

    fn home() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
    }

    /// A System V segment's key, from the probe's tag.
    fn sysv_key(tag: &str) -> i32 {
        let h = blake3::hash(tag.as_bytes());
        (i32::from_le_bytes(h.as_bytes()[..4].try_into().unwrap()) & 0x3fff_ffff) | 0x4000_0000
    }

    fn sysv_exists(key: i32) -> bool {
        // SAFETY: a plain syscall with no pointers.
        unsafe { libc::shmget(key, 0, 0) >= 0 }
    }

    fn connect_abstract(name: &str) -> bool {
        SocketAddr::from_abstract_name(name.as_bytes())
            .and_then(|a| UnixStream::connect_addr(&a))
            .is_ok()
    }

    fn connect_tcp(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_secs(1),
        )
        .is_ok()
    }

    /// The listeners A keeps open while it waits to be ended.
    type Kept = (UnixListener, std::net::TcpListener);

    fn hold(tag: &str, bridge: &Path) -> std::io::Result<(Report, Option<Kept>)> {
        let (uid, pid, cgroup) = me();
        let abstract_name = format!("storm-probe-{tag}");
        let unix =
            UnixListener::bind_addr(&SocketAddr::from_abstract_name(abstract_name.as_bytes())?)?;
        let tcp = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = tcp.local_addr()?.port();
        let tmp = PathBuf::from(format!("/tmp/storm-probe-{tag}"));
        std::fs::write(&tmp, b"a")?;
        let shm = PathBuf::from(format!("/dev/shm/storm-probe-{tag}"));
        let _ = std::fs::write(&shm, b"a");
        let key = sysv_key(tag);
        // SAFETY: a plain syscall with no pointers.
        unsafe { libc::shmget(key, 4096, libc::IPC_CREAT | 0o600) };
        let marker = home().join("storm-probe-marker");
        std::fs::write(&marker, b"a")?;
        let own = BTreeMap::from([
            ("abstract".to_string(), connect_abstract(&abstract_name)),
            ("tcp".to_string(), connect_tcp(port)),
            ("tmp".to_string(), tmp.exists()),
            ("shm".to_string(), shm.exists()),
            ("sysv".to_string(), sysv_exists(key)),
            ("home".to_string(), std::fs::read(&marker).is_ok()),
            ("bridge".to_string(), UnixStream::connect(bridge).is_ok()),
        ]);
        let report = Report {
            uid,
            pid,
            cgroup,
            endpoints: Some(Endpoints {
                port,
                abstract_name,
                tmp,
                shm,
                sysv_key: key,
                marker,
            }),
            own,
            reached: BTreeMap::new(),
        };
        Ok((report, Some((unix, tcp))))
    }

    /// Whether a file could be made in `dir`; one that was is removed.
    fn writable(dir: &Path) -> bool {
        let path = dir.join(format!(".storm-probe-{}", std::process::id()));
        let made = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .is_ok();
        if made {
            let _ = std::fs::remove_file(&path);
        }
        made
    }

    fn reach(
        bridge: &Path,
        a: &Report,
        a_bridge: &Path,
        a_state: &Path,
        agents: &Path,
    ) -> std::io::Result<(Report, Option<Kept>)> {
        let (uid, pid, cgroup) = me();
        let e = a.endpoints.clone().unwrap_or_default();
        let mine = home().join("storm-probe-b");
        let own = BTreeMap::from([
            ("bridge".to_string(), UnixStream::connect(bridge).is_ok()),
            ("home".to_string(), std::fs::write(&mine, b"b").is_ok()),
        ]);
        // SAFETY: a plain syscall with no pointers.
        let signal = a.pid > 0 && unsafe { libc::kill(a.pid, 0) } == 0;
        let reached = BTreeMap::from([
            (
                "proc".to_string(),
                a.pid > 0 && Path::new(&format!("/proc/{}", a.pid)).exists(),
            ),
            (
                "environ".to_string(),
                a.pid > 0 && std::fs::read(format!("/proc/{}/environ", a.pid)).is_ok(),
            ),
            ("signal".to_string(), signal),
            ("state".to_string(), std::fs::read(a_state).is_ok()),
            ("bridge".to_string(), UnixStream::connect(a_bridge).is_ok()),
            ("home".to_string(), std::fs::read(&e.marker).is_ok()),
            (
                "abstract".to_string(),
                !e.abstract_name.is_empty() && connect_abstract(&e.abstract_name),
            ),
            ("tcp".to_string(), e.port != 0 && connect_tcp(e.port)),
            ("tmp".to_string(), e.tmp.exists()),
            ("shm".to_string(), e.shm.exists()),
            (
                "sysv".to_string(),
                e.sysv_key != 0 && sysv_exists(e.sysv_key),
            ),
            ("agents_write".to_string(), writable(agents)),
            ("system_write".to_string(), writable(Path::new("/usr/bin"))),
        ]);
        let report = Report {
            uid,
            pid,
            cgroup,
            endpoints: None,
            own,
            reached,
        };
        Ok((report, None))
    }

    // ---- the host's side ------------------------------------------------

    /// A bridge socket the host holds for a probe, recording who connects.
    struct Socket {
        stop: Arc<AtomicBool>,
        uids: Arc<Mutex<Vec<u32>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Socket {
        fn bind(path: &Path) -> std::io::Result<Self> {
            let listener = bind_session_socket(path)?;
            listener.set_nonblocking(true)?;
            let stop = Arc::new(AtomicBool::new(false));
            let uids = Arc::new(Mutex::new(Vec::new()));
            let (s, u) = (stop.clone(), uids.clone());
            let thread = std::thread::spawn(move || {
                // What is already waiting is taken before a stop is obeyed.
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Ok(cred) = rustix::net::sockopt::socket_peercred(&stream) {
                                u.lock().unwrap().push(cred.uid.as_raw());
                            }
                        }
                        Err(_) if s.load(Ordering::SeqCst) => return,
                        Err(_) => std::thread::sleep(Duration::from_millis(20)),
                    }
                }
            });
            Ok(Self {
                stop,
                uids,
                thread: Some(thread),
            })
        }

        fn close(mut self) -> Vec<u32> {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            self.uids.lock().unwrap().clone()
        }
    }

    /// A probe session, started through the helper.
    struct Probe {
        id: String,
        dir: PathBuf,
        socket: Option<Socket>,
        running: Option<(crate::pty::Child, crate::pty::Group, crate::pty::Input)>,
        lines: Option<mpsc::Receiver<String>>,
    }

    impl Probe {
        fn new(id: String, link: &Link) -> std::io::Result<Self> {
            let dir = crate::mcp::session_dir(&link.state_dir, &id);
            crate::platform::create_private_dir(&dir)?;
            // Its Storm state, which the other probe must not read (AT-5).
            crate::mcp::write_private(&dir.join("mcp.json"), b"{}")
                .map_err(std::io::Error::other)?;
            let socket = Socket::bind(&dir.join(crate::mcp::SOCKET))?;
            Ok(Self {
                id,
                dir,
                socket: Some(socket),
                running: None,
                lines: None,
            })
        }

        fn socket_path(&self) -> String {
            self.dir.join(crate::mcp::SOCKET).display().to_string()
        }

        fn start(&mut self, link: &Link, args: Vec<String>) -> std::io::Result<()> {
            let launch = crate::pty::Launch {
                program: PathBuf::new(),
                args: std::iter::once("isolation-probe".to_string())
                    .chain(args)
                    .map(Into::into)
                    .collect(),
                cwd: link.homes.join(&self.id),
                env: Vec::new(),
                env_remove: Vec::new(),
                secret_env: Vec::new(),
                resumed_from: None,
                child_of: None,
                tag: self.id.clone(),
                isolated: Some(Isolated {
                    link: link.clone(),
                    provider: PROBE.into(),
                }),
            };
            let size = crate::provider::TerminalSize::new(200, 50).expect("a size");
            let crate::pty::Spawned {
                child,
                output,
                input,
                group,
                ..
            } = crate::pty::spawn(&launch, size)?;
            self.running = Some((child, group, input));
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(output);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader.read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => return,
                        Ok(_) => {
                            let text = String::from_utf8_lossy(&line);
                            if let Some(at) = text.find(MARK) {
                                let _ = tx.send(text[at + MARK.len()..].trim().to_string());
                            }
                        }
                    }
                }
            });
            self.lines = Some(rx);
            Ok(())
        }

        fn answer(&self, deadline: Instant) -> Option<Report> {
            let wait = deadline.saturating_duration_since(Instant::now());
            let line = self.lines.as_ref()?.recv_timeout(wait).ok()?;
            serde_json::from_str(&line).ok()
        }

        /// Ends it as every session ends (the unit stopped, the lease
        /// revoked) and says who connected to its socket.
        fn end(mut self) -> Vec<u32> {
            if let Some((child, group, _input)) = self.running.take() {
                group.hang_up();
                let (done, ended) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = child.wait();
                    let _ = done.send(());
                });
                if ended.recv_timeout(Duration::from_secs(3)).is_err() {
                    group.kill();
                    let _ = ended.recv_timeout(ANSWER);
                }
            }
            let uids = self.socket.take().map(Socket::close).unwrap_or_default();
            let _ = std::fs::remove_dir_all(&self.dir);
            uids
        }
    }

    fn probe_id(which: char) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("ags_isoprobe{}{nanos}{which}", std::process::id())
    }

    /// The two probes' answers, and who connected to their sockets.
    pub struct Probed {
        pub a: (String, Report),
        pub b: (String, Report),
        pub seen: Connections,
    }

    /// Runs the two probes. On failure, the reason to report and what went
    /// wrong.
    pub fn probe(link: &Link) -> Result<Probed, (&'static str, String)> {
        if let Err(e) = link.ready() {
            return Err((
                unavailable_because(&e).unwrap_or("self_test"),
                format!("the helper is not ready: {e}"),
            ));
        }
        let failed = |what: &str, e: &dyn std::fmt::Display| ("self_test", format!("{what}: {e}"));
        let mut a = Probe::new(probe_id('a'), link).map_err(|e| failed("a's directory", &e))?;
        let mut b = match Probe::new(probe_id('b'), link) {
            Ok(b) => b,
            Err(e) => {
                a.end();
                return Err(failed("b's directory", &e));
            }
        };
        let outcome = (|| -> Result<(Report, Report), (&'static str, String)> {
            let started = |e: std::io::Error| {
                (
                    unavailable_because(&e).unwrap_or("self_test"),
                    format!("starting a probe: {e}"),
                )
            };
            a.start(link, vec!["hold".into(), a.id.clone(), a.socket_path()])
                .map_err(started)?;
            let a_report = a
                .answer(Instant::now() + ANSWER)
                .ok_or_else(|| failed("a's answer", &"none in time"))?;
            let agents = link.state_dir.join("agents").join("managed");
            b.start(
                link,
                vec![
                    "reach".into(),
                    b.socket_path(),
                    serde_json::to_string(&a_report).unwrap_or_default(),
                    a.socket_path(),
                    a.dir.join("mcp.json").display().to_string(),
                    agents.display().to_string(),
                ],
            )
            .map_err(started)?;
            let b_report = b
                .answer(Instant::now() + ANSWER)
                .ok_or_else(|| failed("b's answer", &"none in time"))?;
            Ok((a_report, b_report))
        })();
        let (a_id, b_id) = (a.id.clone(), b.id.clone());
        let seen = Connections {
            a: a.end(),
            b: b.end(),
        };
        let (a_report, b_report) = outcome?;
        Ok(Probed {
            a: (a_id, a_report),
            b: (b_id, b_report),
            seen,
        })
    }

    /// 136j's self-test. The reason when isolation is not to be had:
    /// `helper` (it does not answer), `no_passt`, or `self_test`.
    pub fn self_test(link: &Link) -> Result<(), &'static str> {
        let probed = probe(link).map_err(|(reason, detail)| {
            tracing::error!(reason, %detail, "isolation self-test failed");
            reason
        })?;
        let host_uid = rustix::process::getuid().as_raw();
        evaluate(
            host_uid,
            (&probed.a.0, &probed.a.1),
            (&probed.b.0, &probed.b.1),
            &probed.seen,
        )
        .map_err(|property| {
            tracing::error!(%property, "isolation self-test failed");
            "self_test"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing() -> (Report, Report, Connections) {
        let own = |keys: &[&str]| keys.iter().map(|k| (k.to_string(), true)).collect();
        let a = Report {
            uid: 2001,
            pid: 4000,
            cgroup: "0::/system.slice/storm-session-ags_A.service".into(),
            endpoints: Some(Endpoints::default()),
            own: own(HOLD_OWN),
            reached: BTreeMap::new(),
        };
        let b = Report {
            uid: 2002,
            pid: 4001,
            cgroup: "0::/system.slice/storm-session-ags_B.service".into(),
            endpoints: None,
            own: own(REACH_OWN),
            reached: REACHED
                .iter()
                .map(|(k, _)| (k.to_string(), false))
                .collect(),
        };
        let seen = Connections {
            a: vec![2001],
            b: vec![2002],
        };
        (a, b, seen)
    }

    fn judge(a: &Report, b: &Report, seen: &Connections) -> Result<(), String> {
        evaluate(900, ("ags_A", a), ("ags_B", b), seen)
    }

    /// Two probes that were isolated pass.
    #[test]
    fn isolated_probes_pass() {
        let (a, b, seen) = passing();
        assert_eq!(judge(&a, &b, &seen), Ok(()));
    }

    /// Every property, broken alone, fails the self-test: its mechanism
    /// removed is caught.
    #[test]
    fn each_property_broken_alone_fails() {
        type Break = Box<dyn Fn(&mut Report, &mut Report, &mut Connections)>;
        let mut cases: Vec<(String, Break)> = vec![
            // Each probe's socket saw the uid it ran as, so only AT-1 is
            // left to catch these.
            (
                "one uid".into(),
                Box::new(|a, b, s| {
                    b.uid = a.uid;
                    s.b = vec![a.uid];
                }),
            ),
            (
                "the host's uid".into(),
                Box::new(|a, _, s| {
                    a.uid = 900;
                    s.a = vec![900];
                }),
            ),
            (
                "root".into(),
                Box::new(|_, b, s| {
                    b.uid = 0;
                    s.b = vec![0];
                }),
            ),
            (
                "not its own unit".into(),
                Box::new(|a, _, _| a.cgroup = "0::/system.slice/storm-runtime.service".into()),
            ),
            (
                "the other's unit".into(),
                Box::new(|_, b, _| {
                    b.cgroup = "0::/system.slice/storm-session-ags_A.service".into()
                }),
            ),
            (
                "no endpoints".into(),
                Box::new(|a, _, _| a.endpoints = None),
            ),
            (
                "another user on A's socket".into(),
                Box::new(|_, _, s| s.a.push(2002)),
            ),
            (
                "nobody on B's socket".into(),
                Box::new(|_, _, s| s.b.clear()),
            ),
        ];
        for k in HOLD_OWN {
            cases.push((
                format!("A's own {k}"),
                Box::new(move |a, _, _| {
                    a.own.insert(k.to_string(), false);
                }),
            ));
            cases.push((
                format!("A's {k} unreported"),
                Box::new(move |a, _, _| {
                    a.own.remove(*k);
                }),
            ));
        }
        for k in REACH_OWN {
            cases.push((
                format!("B's own {k}"),
                Box::new(move |_, b, _| {
                    b.own.insert(k.to_string(), false);
                }),
            ));
        }
        for (k, _) in REACHED {
            cases.push((
                format!("B reached {k}"),
                Box::new(move |_, b, _| {
                    b.reached.insert(k.to_string(), true);
                }),
            ));
            cases.push((
                format!("B's {k} unreported"),
                Box::new(move |_, b, _| {
                    b.reached.remove(*k);
                }),
            ));
        }
        for (name, break_it) in cases {
            let (mut a, mut b, mut seen) = passing();
            break_it(&mut a, &mut b, &mut seen);
            assert!(judge(&a, &b, &seen).is_err(), "{name} passed");
        }
    }

    /// The probes on real processes, through a fake helper that runs every
    /// session as this account with nothing between them: each control
    /// works, B reaches everything 136 must close, every connection on the
    /// sockets is named by the kernel, and the self-test fails at AT-1. So
    /// each detector detects; on a host that isolates, each must not.
    #[test]
    #[cfg(target_os = "linux")]
    fn without_isolation_the_probes_reach_everything_and_the_self_test_fails() {
        let state = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(state.path().join("agents/managed")).unwrap();
        let helper = crate::isolation::fake::FakeHelper::start(state.path(), "/bin/false");
        let link = helper.link(state.path());
        let probed = probe(&link).expect("both probes answered");
        let (a, b) = (&probed.a.1, &probed.b.1);
        let me = rustix::process::getuid().as_raw();
        assert_eq!((a.uid, b.uid), (me, me));
        for k in HOLD_OWN {
            assert_eq!(a.own.get(*k), Some(&true), "A's {k}");
        }
        for k in REACH_OWN {
            assert_eq!(b.own.get(*k), Some(&true), "B's {k}");
        }
        for (k, _) in REACHED {
            let want = match *k {
                "system_write" => false,
                "shm" => std::path::Path::new("/dev/shm").is_dir(),
                _ => true,
            };
            assert_eq!(b.reached.get(*k), Some(&want), "B reached A's {k}");
        }
        // A's own connection, and B's, which it should never have made.
        assert_eq!(probed.seen.a, [me, me]);
        assert_eq!(probed.seen.b, [me]);
        let verdict = evaluate(me, (&probed.a.0, a), (&probed.b.0, b), &probed.seen);
        assert!(verdict.unwrap_err().starts_with("AT-1"));
        // A took what it made with it when it was hung up.
        let e = a.endpoints.clone().unwrap();
        assert!(!e.tmp.exists() && !e.shm.exists() && !e.marker.exists());
        // SAFETY: a plain syscall with no pointers.
        assert!(unsafe { libc::shmget(e.sysv_key, 0, 0) } < 0);
        assert!(
            !state.path().join("sessions").join(&probed.a.0).exists(),
            "the probes' directories are gone"
        );
        assert_eq!(
            helper.seen.lock().unwrap().released.len(),
            2,
            "both leases released"
        );
    }

    /// Not a test: a probe run by a test's fake helper, given the arguments
    /// the host gives `storm-runtime isolation-probe`.
    #[test]
    fn probe_entry() {
        let Ok(args) = std::env::var("STORM_ISOLATION_PROBE") else {
            return;
        };
        let args: Vec<String> = serde_json::from_str(&args).unwrap();
        std::process::exit(main(&args[1..]));
    }
}
