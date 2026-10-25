//! Bridge identity (decision 135): a bridge or harness connection is taken
//! only from a live descendant of its session's leader.
//!
//! A handle is not proof: every process of the host account can read every
//! bridge's environment. So on a host that reports `verified`, the host asks
//! the kernel who connected (`SO_PEERPIDFD`) and walks that process's
//! ancestors to the session's leader (135b). Every failure refuses, as
//! `identity_mismatch`, never forwarded (135a). A host that cannot check
//! says so — `unavailable` with the failing precondition, or `unsupported` —
//! and keeps today's handle-only behaviour, which the server labels (135d).
//!
//! `verified` is about process ancestry only. Code planted in a session's
//! tree through the account's shared files passes as that session (B-2), and
//! it never makes a host eligible for spawning on its own (Q-A8).

use std::io;
use std::sync::Mutex;

use serde_json::json;

use crate::platform::peer::{self, Process, Support};

/// The walk gives up after this many parents.
pub const MAX_HOPS: usize = 64;

/// `identity.state` on the wire (135d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Verified,
    Unavailable,
    Unsupported,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
        }
    }
}

/// What the host reports as `identity` in `hello` and its inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub state: State,
    pub mechanism: Option<&'static str>,
    pub reason: Option<&'static str>,
}

impl Report {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "state": self.state.as_str(),
            "mechanism": self.mechanism,
            "reason": self.reason,
        })
    }
}

/// Why a connection was refused. The wire answer is always
/// `identity_mismatch`; this is what the audit line records.
#[derive(Debug)]
pub enum Mismatch {
    /// The process that connected has exited (it may have left the socket
    /// to something else).
    PeerExited,
    /// An ancestor exited, or the child changed parent, while being walked.
    Reparented,
    /// The walk reached pid 1 without meeting the leader.
    ReachedInit,
    /// The walk reached the host itself: the peer belongs to no session, or
    /// to another one.
    ReachedHost,
    TooDeep,
    /// A parent outside this pid namespace.
    OutsideNamespace,
    /// The session's leader has exited, or the host never held it.
    LeaderGone,
    /// A precondition stopped holding (`ptrace_scope` was lowered, say).
    Precondition(&'static str),
    Error(io::ErrorKind),
}

impl Mismatch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PeerExited => "peer_exited",
            Self::Reparented => "reparented",
            Self::ReachedInit => "reached_init",
            Self::ReachedHost => "reached_host",
            Self::TooDeep => "too_deep",
            Self::OutsideNamespace => "outside_namespace",
            Self::LeaderGone => "leader_gone",
            Self::Precondition(r) => r,
            Self::Error(_) => "error",
        }
    }
}

impl From<io::Error> for Mismatch {
    fn from(e: io::Error) -> Self {
        Self::Error(e.kind())
    }
}

/// The OS's view of processes, so the walk can be tested on synthetic trees.
pub trait Procs {
    type P;
    fn pid(&self, p: &Self::P) -> i32;
    fn alive(&self, p: &Self::P) -> io::Result<bool>;
    /// `pid`'s parent as the OS says now.
    fn parent(&self, pid: i32) -> io::Result<i32>;
    /// A handle that keeps naming `pid`'s process whatever reuses the number.
    fn open(&self, pid: i32) -> io::Result<Self::P>;
}

/// 135b: whether `peer` descends from `leader`, walking up from it. At each
/// hop the parent's handle is opened first, then the child is checked to be
/// alive and to still have that parent — otherwise the number read may
/// already name another process. Refused at pid 1, at `host`, or after
/// [`MAX_HOPS`]; the leader and the peer must both be alive at the end.
pub fn descends<S: Procs>(
    os: &S,
    peer: &S::P,
    leader: &S::P,
    host: Option<i32>,
) -> Result<(), Mismatch> {
    let leader_pid = os.pid(leader);
    let mut above: Option<S::P> = None;
    let mut hops = 0;
    loop {
        let node = above.as_ref().unwrap_or(peer);
        let pid = os.pid(node);
        if pid == 0 {
            return Err(Mismatch::OutsideNamespace);
        }
        if pid < 0 || !os.alive(node)? {
            return Err(if hops == 0 {
                Mismatch::PeerExited
            } else {
                Mismatch::Reparented
            });
        }
        if pid == leader_pid {
            break;
        }
        if pid == 1 {
            return Err(Mismatch::ReachedInit);
        }
        if Some(pid) == host {
            return Err(Mismatch::ReachedHost);
        }
        if hops == MAX_HOPS {
            return Err(Mismatch::TooDeep);
        }
        let ppid = os.parent(pid)?;
        if ppid <= 0 {
            return Err(Mismatch::OutsideNamespace);
        }
        let up = os.open(ppid)?;
        if !os.alive(node)? || os.parent(pid)? != ppid {
            return Err(Mismatch::Reparented);
        }
        above = Some(up);
        hops += 1;
    }
    // The leader is the host's unreaped child, so while it lives no other
    // process can have its number.
    if !os.alive(leader)? {
        return Err(Mismatch::LeaderGone);
    }
    if !os.alive(peer)? {
        return Err(Mismatch::PeerExited);
    }
    Ok(())
}

/// The real processes, through `platform::peer`.
pub struct Os;

impl Procs for Os {
    type P = Process;
    fn pid(&self, p: &Process) -> i32 {
        p.pid()
    }
    fn alive(&self, p: &Process) -> io::Result<bool> {
        p.alive()
    }
    fn parent(&self, pid: i32) -> io::Result<i32> {
        peer::parent(pid)
    }
    fn open(&self, pid: i32) -> io::Result<Process> {
        Process::open(pid)
    }
}

/// What the preconditions read, so each failure can be tested without root.
pub struct Probe {
    pub ptrace_scope: io::Result<u32>,
    pub proc_readable: bool,
    pub pidfd_open: bool,
    pub so_peerpidfd: bool,
    pub self_test: bool,
}

impl Probe {
    /// Reads every precondition and runs the self-test on this machine.
    pub fn run() -> Self {
        let me = std::process::id() as i32;
        let parent = peer::parent(me);
        let pidfd_open = Process::open(me).is_ok();
        let so_peerpidfd = peer::socketpair_peer().is_ok_and(|p| p.pid() == me);
        Self {
            ptrace_scope: peer::ptrace_scope(),
            proc_readable: parent.as_ref().is_ok_and(|p| *p > 0),
            pidfd_open,
            so_peerpidfd,
            self_test: pidfd_open && so_peerpidfd && self_test(&Os, me).is_ok(),
        }
    }

    /// 135b's list, in order; the first that fails is the reason.
    pub fn verdict(&self) -> Result<(), &'static str> {
        match self.ptrace_scope {
            Ok(n) if n >= 1 => {}
            _ => return Err("ptrace_scope"),
        }
        if !self.proc_readable {
            return Err("proc_unreadable");
        }
        if !self.pidfd_open {
            return Err("no_pidfd_open");
        }
        if !self.so_peerpidfd {
            return Err("no_so_peerpidfd");
        }
        if !self.self_test {
            return Err("self_test");
        }
        Ok(())
    }
}

/// The start-up self-test: the walk on real processes, both ways, and a
/// pidfd seeing a process exit.
///
/// - this process, connected to itself, descends from its own parent;
/// - its parent does not descend from it (the walk ends at pid 1 or above
///   the leader);
/// - a child it reaped is no longer alive through its pidfd — the check
///   that tells an exited connector from a live one.
fn self_test<S: Procs>(os: &S, me: i32) -> Result<(), &'static str> {
    let fail = |_| "self_test";
    let this = os.open(me).map_err(fail)?;
    let parent = os.open(os.parent(me).map_err(fail)?).map_err(fail)?;
    descends(os, &this, &parent, None).map_err(|_| "self_test")?;
    if descends(os, &parent, &this, None).is_ok() {
        return Err("self_test");
    }
    let mut child = std::process::Command::new(std::env::current_exe().map_err(fail)?)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(fail)?;
    let watched = os.open(child.id() as i32);
    let _ = child.wait();
    match watched {
        Ok(p) if !os.alive(&p).map_err(fail)? => Ok(()),
        _ => Err("self_test"),
    }
}

/// The host's identity state. Starts from the platform and the start-up
/// probe; can only go down while the host runs (a runtime downgrade takes
/// effect at once, 135d), never up.
pub struct Identity {
    report: Mutex<Report>,
}

impl Identity {
    pub fn detect() -> Self {
        Self::from_support(peer::support(), Probe::run)
    }

    fn from_support(support: Support, probe: impl FnOnce() -> Probe) -> Self {
        let report = match support {
            Support::Unsupported(reason) => Report {
                state: State::Unsupported,
                mechanism: None,
                reason: Some(reason),
            },
            Support::Unavailable(reason) => Report {
                state: State::Unavailable,
                mechanism: None,
                reason: Some(reason),
            },
            Support::Available(mechanism) => match probe().verdict() {
                Ok(()) => Report {
                    state: State::Verified,
                    mechanism: Some(mechanism),
                    reason: None,
                },
                Err(reason) => Report {
                    state: State::Unavailable,
                    mechanism: Some(mechanism),
                    reason: Some(reason),
                },
            },
        };
        Self {
            report: Mutex::new(report),
        }
    }

    pub fn report(&self) -> Report {
        self.report.lock().unwrap().clone()
    }

    pub fn verified(&self) -> bool {
        self.report.lock().unwrap().state == State::Verified
    }

    /// Drops a verified host to `unavailable`. Whether anything changed.
    pub fn downgrade(&self, reason: &'static str) -> bool {
        let mut r = self.report.lock().unwrap();
        if r.state != State::Verified {
            return false;
        }
        r.state = State::Unavailable;
        r.reason = Some(reason);
        true
    }

    #[cfg(test)]
    pub fn fixed(report: Report) -> Self {
        Self {
            report: Mutex::new(report),
        }
    }
}

/// The whole check for one connection on a verified host: the precondition
/// that can change at run time, then the walk from `peer` to `leader`.
pub fn check(peer: &Process, leader: &Process) -> Result<(), Mismatch> {
    check_with(peer::ptrace_scope(), peer, leader)
}

fn check_with(
    ptrace_scope: io::Result<u32>,
    peer: &Process,
    leader: &Process,
) -> Result<(), Mismatch> {
    match ptrace_scope {
        Ok(n) if n >= 1 => {}
        _ => return Err(Mismatch::Precondition("ptrace_scope")),
    }
    descends(&Os, peer, leader, Some(std::process::id() as i32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    type OnOpen = dyn Fn(&Tree, i32);
    type Case = (fn(&mut Probe), &'static str);

    /// A synthetic process table. A handle is `(pid, generation)`: a reused
    /// number gets a new generation, and a handle is alive only while its
    /// generation is the live one, as a pidfd is.
    #[derive(Default)]
    struct Tree {
        /// pid → (generation, parent, alive).
        procs: RefCell<HashMap<i32, (u32, i32, bool)>>,
        /// Called as every `open` starts, to move the tree under the walk.
        on_open: RefCell<Option<Box<OnOpen>>>,
        fail_parent: Cell<Option<i32>>,
        fail_open: Cell<Option<i32>>,
    }

    impl Tree {
        fn with(edges: &[(i32, i32)]) -> Self {
            let t = Tree::default();
            t.procs.borrow_mut().insert(1, (0, 0, true));
            for &(pid, parent) in edges {
                t.procs.borrow_mut().insert(pid, (0, parent, true));
            }
            t
        }
        fn kill(&self, pid: i32) {
            self.procs.borrow_mut().get_mut(&pid).unwrap().2 = false;
        }
        fn reparent(&self, pid: i32, to: i32) {
            self.procs.borrow_mut().get_mut(&pid).unwrap().1 = to;
        }
        /// `pid` exits and its number goes to a new process under `parent`.
        fn reuse(&self, pid: i32, parent: i32) {
            let mut p = self.procs.borrow_mut();
            let g = p[&pid].0 + 1;
            p.insert(pid, (g, parent, true));
        }
        fn h(&self, pid: i32) -> (i32, u32) {
            self.open(pid).unwrap()
        }
    }

    impl Procs for Tree {
        type P = (i32, u32);
        fn pid(&self, p: &(i32, u32)) -> i32 {
            p.0
        }
        fn alive(&self, p: &(i32, u32)) -> io::Result<bool> {
            Ok(self
                .procs
                .borrow()
                .get(&p.0)
                .is_some_and(|&(g, _, alive)| g == p.1 && alive))
        }
        fn parent(&self, pid: i32) -> io::Result<i32> {
            if self.fail_parent.get() == Some(pid) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            self.procs
                .borrow()
                .get(&pid)
                .map(|&(_, parent, _)| parent)
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }
        fn open(&self, pid: i32) -> io::Result<(i32, u32)> {
            if self.fail_open.get() == Some(pid) {
                return Err(io::ErrorKind::NotFound.into());
            }
            // The race happens between reading a parent's number and opening
            // it, so the handle names whatever has the number by then.
            if let Some(f) = self.on_open.borrow().as_ref() {
                f(self, pid);
            }
            let g = self
                .procs
                .borrow()
                .get(&pid)
                .map(|&(g, _, _)| g)
                .ok_or(io::ErrorKind::NotFound)?;
            Ok((pid, g))
        }
    }

    /// init(1) → host(10) → leader(20) → agent(30) → hook(40) → tool(50);
    /// host(10) → other leader(21) → other agent(31); init(1) → stray(60).
    fn tree() -> Tree {
        Tree::with(&[
            (10, 1),
            (20, 10),
            (30, 20),
            (40, 30),
            (50, 40),
            (21, 10),
            (31, 21),
            (60, 1),
        ])
    }

    fn walk(t: &Tree, peer: i32, leader: i32) -> Result<(), Mismatch> {
        let (p, l) = (t.h(peer), t.h(leader));
        descends(t, &p, &l, Some(10))
    }

    fn refused(r: Result<(), Mismatch>) -> &'static str {
        r.expect_err("should be refused").as_str()
    }

    #[test]
    fn the_leader_and_everything_under_it_is_accepted() {
        let t = tree();
        for peer in [20, 30, 40, 50] {
            assert!(walk(&t, peer, 20).is_ok(), "{peer}");
        }
    }

    #[test]
    fn another_sessions_process_reaches_the_host() {
        let t = tree();
        assert_eq!(refused(walk(&t, 31, 20)), "reached_host");
        assert_eq!(refused(walk(&t, 21, 20)), "reached_host");
        // The host itself is no session's process.
        assert_eq!(refused(walk(&t, 10, 20)), "reached_host");
    }

    #[test]
    fn a_process_outside_the_host_reaches_init() {
        let t = tree();
        assert_eq!(refused(walk(&t, 60, 20)), "reached_init");
        // A detached process: re-parented to init before it connected.
        t.reparent(50, 1);
        assert_eq!(refused(walk(&t, 50, 20)), "reached_init");
    }

    #[test]
    fn an_exited_connector_is_refused_even_if_its_parent_is_the_leader() {
        let t = tree();
        let peer = t.h(30);
        t.kill(30);
        assert_eq!(
            refused(descends(&t, &peer, &t.h(20), Some(10))),
            "peer_exited"
        );
        // Its number reused by a fresh process right under the leader.
        t.reuse(30, 20);
        assert_eq!(
            refused(descends(&t, &peer, &t.h(20), Some(10))),
            "peer_exited"
        );
    }

    #[test]
    fn a_peer_that_dies_after_the_walk_is_refused() {
        let t = tree();
        let peer = t.h(40);
        let leader = t.h(20);
        // The walk opens 20 last; the peer exits right then.
        *t.on_open.borrow_mut() = Some(Box::new(|t: &Tree, pid| {
            if pid == 20 {
                t.kill(40);
            }
        }));
        assert!(descends(&t, &peer, &leader, Some(10)).is_err());
    }

    #[test]
    fn a_reparent_while_the_parent_is_opened_is_caught() {
        // 40's parent 30 exits as the walk opens it; 40 goes to init and
        // 30's number goes to a process under the leader. Without the
        // re-check, the walk would follow the new 30 to the leader.
        let t = tree();
        *t.on_open.borrow_mut() = Some(Box::new(|t: &Tree, pid| {
            if pid == 30 && t.procs.borrow()[&30].0 == 0 {
                t.reparent(40, 1);
                t.reuse(30, 20);
            }
        }));
        let (peer, leader) = (t.h(40), t.h(20));
        assert_eq!(
            refused(descends(&t, &peer, &leader, Some(10))),
            "reparented"
        );
    }

    #[test]
    fn an_ancestor_that_exits_mid_walk_is_caught() {
        let t = tree();
        *t.on_open.borrow_mut() = Some(Box::new(|t: &Tree, pid| {
            if pid == 30 {
                t.kill(30);
            }
        }));
        let (peer, leader) = (t.h(50), t.h(20));
        assert_eq!(
            refused(descends(&t, &peer, &leader, Some(10))),
            "reparented"
        );
    }

    #[test]
    fn an_ancestor_that_exits_while_its_parent_is_opened_is_caught() {
        // 30 exits as the walk opens its parent, the leader: 50 and 40 are
        // no longer under the leader, though the table still says 20.
        let t = tree();
        *t.on_open.borrow_mut() = Some(Box::new(|t: &Tree, pid| {
            if pid == 20 {
                t.kill(30);
            }
        }));
        let (peer, leader) = (t.h(50), t.procs.borrow()[&20].0);
        let leader = (20, leader);
        assert_eq!(
            refused(descends(&t, &peer, &leader, Some(10))),
            "reparented"
        );
    }

    #[test]
    fn an_exited_leader_whose_number_was_reused_is_refused() {
        let t = tree();
        let leader = t.h(20);
        t.reuse(20, 10);
        // A new process under the new 20.
        t.procs.borrow_mut().insert(70, (0, 20, true));
        assert_eq!(
            refused(descends(&t, &t.h(70), &leader, Some(10))),
            "leader_gone"
        );
    }

    #[test]
    fn the_walk_stops_after_64_parents() {
        let mut edges = vec![(10, 1), (20, 10)];
        let mut parent = 20;
        for pid in 100..100 + MAX_HOPS as i32 + 1 {
            edges.push((pid, parent));
            parent = pid;
        }
        let t = Tree::with(&edges);
        // Exactly 64 parents from the deepest that still reaches the leader.
        let deepest_ok = 100 + MAX_HOPS as i32 - 1;
        assert!(walk(&t, deepest_ok, 20).is_ok());
        assert_eq!(refused(walk(&t, deepest_ok + 1, 20)), "too_deep");
    }

    #[test]
    fn a_parent_in_another_namespace_is_refused() {
        let t = tree();
        t.reparent(60, 0);
        assert_eq!(refused(walk(&t, 60, 20)), "outside_namespace");
    }

    #[test]
    fn a_peer_the_kernel_cannot_number_is_refused() {
        let t = tree();
        // `fdinfo` says 0 for a process in a pid namespace this one cannot
        // see, though it is alive; -1 once it is gone.
        t.procs.borrow_mut().insert(0, (0, 20, true));
        assert_eq!(
            refused(descends(&t, &(0, 0), &t.h(20), Some(10))),
            "outside_namespace"
        );
        assert_eq!(
            refused(descends(&t, &(-1, 0), &t.h(20), Some(10))),
            "peer_exited"
        );
    }

    #[test]
    fn every_os_error_refuses() {
        let t = tree();
        t.fail_parent.set(Some(40));
        assert_eq!(refused(walk(&t, 50, 20)), "error");
        let t = tree();
        let (peer, leader) = (t.h(50), t.h(20));
        t.fail_open.set(Some(30));
        assert_eq!(refused(descends(&t, &peer, &leader, Some(10))), "error");
    }

    fn passing() -> Probe {
        Probe {
            ptrace_scope: Ok(1),
            proc_readable: true,
            pidfd_open: true,
            so_peerpidfd: true,
            self_test: true,
        }
    }

    /// G-135-3: each precondition failure is `unavailable`, with its reason.
    #[test]
    fn each_failed_precondition_is_unavailable_with_its_reason() {
        assert_eq!(passing().verdict(), Ok(()));
        let cases: [Case; 7] = [
            (|p| p.ptrace_scope = Ok(0), "ptrace_scope"),
            (
                |p| p.ptrace_scope = Err(io::ErrorKind::NotFound.into()),
                "ptrace_scope",
            ),
            (|p| p.proc_readable = false, "proc_unreadable"),
            (|p| p.pidfd_open = false, "no_pidfd_open"),
            (|p| p.so_peerpidfd = false, "no_so_peerpidfd"),
            (|p| p.self_test = false, "self_test"),
            (|p| p.ptrace_scope = Ok(3), ""),
        ];
        for (break_it, reason) in cases {
            let mut p = passing();
            break_it(&mut p);
            let id = Identity::from_support(Support::Available("so_peerpidfd"), || p);
            let r = id.report();
            if reason.is_empty() {
                assert_eq!(r.state, State::Verified);
                continue;
            }
            assert_eq!(r.state, State::Unavailable, "{reason}");
            assert_eq!(r.reason, Some(reason));
            assert_eq!(r.mechanism, Some("so_peerpidfd"));
            assert!(!id.verified());
        }
    }

    #[test]
    fn the_platforms_without_the_mechanism_say_so_without_probing() {
        let probe = || -> Probe { panic!("must not probe") };
        let win = Identity::from_support(Support::Unsupported("windows"), probe);
        assert_eq!(
            win.report().to_json(),
            json!({"state": "unsupported", "mechanism": null, "reason": "windows"})
        );
        let mac = Identity::from_support(Support::Unavailable("macos_not_gated"), probe);
        assert_eq!(mac.report().state, State::Unavailable);
        assert_eq!(mac.report().reason, Some("macos_not_gated"));
    }

    #[test]
    fn a_downgrade_is_one_way() {
        let id = Identity::from_support(Support::Available("so_peerpidfd"), passing);
        assert!(id.verified());
        assert!(id.downgrade("ptrace_scope"));
        assert!(!id.verified());
        assert!(!id.downgrade("self_test"));
        assert_eq!(id.report().reason, Some("ptrace_scope"));
    }

    /// This platform's own report, as CI runs it on each: Linux hosts with
    /// the mechanism are `verified`; macOS `unavailable`; Windows
    /// `unsupported` (135b–135d, AM80).
    #[test]
    fn this_platform_reports_what_135_says() {
        let r = Identity::detect().report();
        #[cfg(target_os = "linux")]
        assert_eq!(
            (r.state, r.mechanism, r.reason),
            (State::Verified, Some("so_peerpidfd"), None),
            "this Linux host should verify; if it cannot, the reason says why"
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            (r.state, r.reason),
            (State::Unavailable, Some("macos_not_gated"))
        );
        #[cfg(windows)]
        assert_eq!((r.state, r.reason), (State::Unsupported, Some("windows")));
    }

    /// The real OS with one lie, to show the self-test notices it.
    #[cfg(target_os = "linux")]
    struct Lying {
        alive_always: bool,
        one_pid: Option<i32>,
        no_parents: bool,
        /// Every parent read after the first says init: a tree that moves.
        moving: Option<&'static std::sync::atomic::AtomicBool>,
    }

    #[cfg(target_os = "linux")]
    impl Procs for Lying {
        type P = Process;
        fn pid(&self, p: &Process) -> i32 {
            self.one_pid.unwrap_or(p.pid())
        }
        fn alive(&self, p: &Process) -> io::Result<bool> {
            Ok(self.alive_always || p.alive()?)
        }
        fn parent(&self, pid: i32) -> io::Result<i32> {
            if self.no_parents {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            if let Some(read) = self.moving
                && read.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(1);
            }
            peer::parent(pid)
        }
        fn open(&self, pid: i32) -> io::Result<Process> {
            Process::open(pid)
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_self_test_fails_on_a_kernel_that_lies() {
        let me = std::process::id() as i32;
        let honest = Lying {
            alive_always: false,
            one_pid: None,
            no_parents: false,
            moving: None,
        };
        assert_eq!(self_test(&honest, me), Ok(()));
        for (name, lie) in [
            (
                "an exited process looks alive",
                Lying {
                    alive_always: true,
                    ..honest
                },
            ),
            (
                "every process has one number",
                Lying {
                    one_pid: Some(me),
                    ..honest
                },
            ),
            (
                "no parent can be read",
                Lying {
                    no_parents: true,
                    ..honest
                },
            ),
            (
                "this process moves while it is walked",
                Lying {
                    moving: Some(Box::leak(Box::new(std::sync::atomic::AtomicBool::new(
                        false,
                    )))),
                    ..honest
                },
            ),
        ] {
            assert_eq!(self_test(&lie, me), Err("self_test"), "{name}");
        }
    }

    /// A lowered `ptrace_scope` refuses the connection and names the
    /// precondition, so the host downgrades itself (135d).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_lowered_ptrace_scope_refuses_at_run_time() {
        let me = || Process::open(std::process::id() as i32).unwrap();
        let parent = Process::open(peer::parent(std::process::id() as i32).unwrap()).unwrap();
        for scope in [Ok(0), Err(io::ErrorKind::NotFound.into())] {
            let r = check_with(scope, &me(), &parent);
            assert_eq!(refused(r), "ptrace_scope");
        }
        // With Yama on, the same pair gets as far as the walk (this process
        // is the host, so it stops there).
        assert_eq!(refused(check_with(Ok(1), &me(), &parent)), "reached_host");
    }

    /// The real kernel: a pidfd of a reaped child is not alive, and the walk
    /// accepts this process under its parent and refuses the reverse.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_self_test_passes_on_this_kernel() {
        assert_eq!(self_test(&Os, std::process::id() as i32), Ok(()));
    }
}
