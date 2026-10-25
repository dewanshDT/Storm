//! The session operations (136b, 136d): lease and release a pool user,
//! prepare and clean a session's directories, start it in its own unit with
//! the terminal the host handed over, signal it, and end it by stopping the
//! unit.
//!
//! At lease end (`release`) the unit is stopped, which ends every process
//! of the user: nothing else ever runs as a pool user, and nothing in a
//! unit's control group can leave it. Then everything the user wrote in
//! the workspace is handed back, its home is removed, and only then is the
//! user free again. Leases are recorded on disk first, so a helper that
//! restarts does all of that for whatever the last one left before it
//! answers anyone.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::{Map, Value, json};

use crate::files::{self, LeaseRecord};
use crate::leases::Leases;
use crate::policy::{self, Context};
use crate::protocol::{Op, Refusal, Request};
use crate::serve::Helper;
use crate::units::{self, UnitSpec};

/// Where everything lives.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The helper's own state, root's and `0700`.
    pub state: PathBuf,
    /// The host account's (`/var/lib/storm-runtime`).
    pub host_state: PathBuf,
}

pub const HOST_STATE: &str = "/var/lib/storm-runtime";

impl Layout {
    pub fn installed() -> Self {
        Self {
            state: crate::STATE_DIR.into(),
            host_state: HOST_STATE.into(),
        }
    }
    pub fn leases(&self) -> PathBuf {
        self.state.join("leases")
    }
    pub fn manifests(&self) -> PathBuf {
        self.state.join("manifests")
    }
    pub fn home(&self, session: &str) -> PathBuf {
        self.state.join("homes").join(session)
    }
    pub fn private(&self, session: &str) -> PathBuf {
        self.state.join("sessions").join(session)
    }
    pub fn env_file(&self, session: &str) -> PathBuf {
        self.private(session).join("env")
    }
    /// The host's directory for the session (its MCP config, 136i).
    pub fn session_dir(&self, session: &str) -> PathBuf {
        self.host_state.join("sessions").join(session)
    }
    /// Where the host writes a session's launch (`<session>.json`).
    pub fn launch_dir(&self) -> PathBuf {
        self.host_state.join("launch")
    }
    pub fn agents(&self) -> PathBuf {
        self.host_state.join("agents").join("managed")
    }
}

pub struct State {
    pub leases: Leases,
    pub records: BTreeMap<String, LeaseRecord>,
    /// Sessions whose unit exists.
    pub started: BTreeSet<String>,
    /// Sessions with an operation in flight.
    busy: BTreeSet<String>,
}

impl State {
    pub fn new(users: impl IntoIterator<Item = String>) -> Self {
        Self {
            leases: Leases::new(users),
            records: BTreeMap::new(),
            started: BTreeSet::new(),
            busy: BTreeSet::new(),
        }
    }

    #[cfg(test)]
    pub fn lease(&mut self, session: &str) -> Option<String> {
        self.leases.lease(session)
    }
}

/// Marks a session busy for one operation.
struct Busy<'a> {
    helper: &'a Helper,
    session: String,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.helper.state.lock().unwrap().busy.remove(&self.session);
    }
}

fn failed(what: &str, e: anyhow::Error) -> Refusal {
    eprintln!("storm-runtime-isolation: {what}: {e:#}");
    Refusal::Failed
}

type Answer = (Map<String, Value>, Option<OwnedFd>);

impl Helper {
    fn session_busy(&self, session: &str) -> Result<Busy<'_>, Refusal> {
        let mut st = self.state.lock().unwrap();
        if !st.busy.insert(session.to_string()) {
            return Err(Refusal::Busy);
        }
        Ok(Busy {
            helper: self,
            session: session.to_string(),
        })
    }

    pub(crate) fn carry_out(&self, req: &Request, fd: Option<OwnedFd>) -> Result<Answer, Refusal> {
        {
            let st = self.state.lock().unwrap();
            let cx = Context {
                pool: &self.pool,
                roots: &self.roots,
                providers: &self.providers,
                leases: &st.leases,
            };
            policy::validate(req, &cx)?;
        }
        let none = || Ok((Map::new(), None));
        let session = req.get("session");
        match req.op {
            Op::Lease => self.lease(session),
            Op::Prepare => {
                let _busy = self.session_busy(session)?;
                self.prepare(session, req.get("user")).map(|()| none())?
            }
            Op::Start => {
                let _busy = self.session_busy(session)?;
                let fd = fd.ok_or(Refusal::NoTerminal)?;
                self.start(
                    session,
                    req.get("user"),
                    req.get("provider"),
                    Path::new(req.get("workspace")),
                    fd,
                )
            }
            Op::Signal => {
                self.signal(session, req.get("signal"))?;
                none()
            }
            Op::End => {
                let _busy = self.session_busy(session)?;
                self.end(session)
            }
            Op::Clean => {
                let _busy = self.session_busy(session)?;
                self.clean(session, req.get("user"))?;
                none()
            }
            Op::Release => {
                let _busy = self.session_busy(session)?;
                self.release(session, req.get("user"))?;
                none()
            }
            Op::ChildCreate
            | Op::ChildExport
            | Op::ChildImport
            | Op::ChildDispose
            | Op::SelfTest => Err(Refusal::NotBuilt),
        }
    }

    fn uid_gid(&self, user: &str) -> (u32, u32) {
        (self.pool.users[user], self.pool.primary[user])
    }

    fn lease(&self, session: &str) -> Result<Answer, Refusal> {
        let mut st = self.state.lock().unwrap();
        if st.leases.user_of(session).is_some() {
            return Err(Refusal::AlreadyLeased);
        }
        let user = st.leases.lease(session).ok_or(Refusal::PoolExhausted)?;
        let record = LeaseRecord {
            session: session.to_string(),
            workspace: None,
        };
        // On disk before the host hears of it.
        if let Err(e) = files::save_lease(&self.layout.leases(), &user, &record) {
            st.leases.release(session, &user);
            return Err(failed("recording a lease", e));
        }
        st.records.insert(user.clone(), record);
        let mut out = Map::new();
        out.insert("user".into(), user.into());
        Ok((out, None))
    }

    fn prepare(&self, session: &str, user: &str) -> Result<(), Refusal> {
        let (uid, gid) = self.uid_gid(user);
        let own = self.deps.own.as_ref();
        files::make_home(&self.layout.home(session), uid, gid, own)
            .map_err(|e| failed("making the home", e))?;
        let private = self.layout.private(session);
        files::remove_tree(&private, own).map_err(|e| failed("clearing", e))?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&private)
            .map_err(|e| failed("making the session's own directory", e.into()))
    }

    fn program(&self, provider: &str) -> Result<PathBuf, Refusal> {
        let command = self
            .commands
            .iter()
            .find(|(id, _)| id == provider)
            .map(|(_, c)| c.as_str())
            .ok_or(Refusal::BadProvider)?;
        if command.is_empty() {
            return Ok(self.shell.clone());
        }
        files::resolve(command, &self.search).ok_or(Refusal::ProviderUnavailable)
    }

    fn hidden(&self) -> Vec<PathBuf> {
        let mut out = vec![self.layout.host_state.clone(), self.layout.state.clone()];
        for root in &self.roots {
            if !out.iter().any(|h| root.starts_with(h)) {
                out.push(root.clone());
            }
        }
        out
    }

    fn start(
        &self,
        session: &str,
        user: &str,
        provider: &str,
        workspace: &Path,
        tty: OwnedFd,
    ) -> Result<Answer, Refusal> {
        if self.state.lock().unwrap().started.contains(session) {
            return Err(Refusal::AlreadyStarted);
        }
        let terminal = crate::tty::verify(tty.as_fd())?;
        let program = self.program(provider)?;
        let launch = files::read_launch(
            &self.layout.launch_dir(),
            &format!("{session}.json"),
            self.pool.host_uid,
        )
        .map_err(|e| {
            eprintln!("storm-runtime-isolation: the launch: {e:#}");
            Refusal::BadLaunch
        })?;
        let own = self.deps.own.as_ref();
        let group = files::workspace_group(workspace, &self.pool, &self.layout.manifests(), own)
            .map_err(|e| failed("the workspace's group", e))?;
        // The workspace is recorded before the session can write in it, so
        // whatever happens next, lease end knows what to hand back.
        {
            let mut st = self.state.lock().unwrap();
            let record = LeaseRecord {
                session: session.to_string(),
                workspace: Some(workspace.to_path_buf()),
            };
            files::save_lease(&self.layout.leases(), user, &record)
                .map_err(|e| failed("recording the workspace", e))?;
            st.records.insert(user.to_string(), record);
        }
        let (uid, gid) = self.uid_gid(user);
        let session_dir = self.layout.session_dir(session);
        let host_depth = self.layout.host_state.components().count();
        files::open_owned_dir(&session_dir, host_depth, self.pool.host_uid)
            .map_err(|e| failed("the session's directory", e))?;
        files::retarget(&session_dir, self.pool.host_uid, own, &|k, m| {
            files::shared(k, m, gid)
        })
        .map_err(|e| failed("sharing the session's directory", e))?;
        self.deps
            .tty
            .give(tty.as_fd(), uid)
            .map_err(|e| failed("giving the terminal", e.into()))?;
        files::write_root_private(
            &self.layout.env_file(session),
            files::env_file(&launch.env).as_bytes(),
        )
        .map_err(|e| failed("writing the environment", e))?;
        let agents = self.layout.agents();
        let spec = UnitSpec {
            session: session.to_string(),
            user: user.to_string(),
            workspace_group: group,
            workspace: workspace.to_path_buf(),
            session_dir,
            home: self.layout.home(session),
            hidden: self.hidden(),
            agents: Some(agents),
            tty: terminal.path,
            rows: terminal.rows,
            cols: terminal.cols,
            env_file: self.layout.env_file(session),
            program,
            args: launch.args,
        };
        let unit = units::unit_name(session);
        self.state
            .lock()
            .unwrap()
            .started
            .insert(session.to_string());
        if let Err(e) = self.deps.units.start(&spec) {
            // A unit that failed to start may still be loaded.
            let _ = self.deps.units.stop(&unit);
            self.deps.units.reset_failed(&unit);
            self.state.lock().unwrap().started.remove(session);
            return Err(failed("starting the unit", e));
        }
        let mut out = Map::new();
        let (pid, pidfd) = self.leader(&unit);
        out.insert("pid".into(), pid.into());
        // The terminal stays open here until the unit has it, so its number
        // could not have named another terminal meanwhile.
        drop(tty);
        Ok((out, pidfd))
    }

    /// The unit's main process, by a pidfd checked to be that process: its
    /// number was the unit's main process both before and after the pidfd
    /// was opened, so it cannot be a later process reusing the number.
    fn leader(&self, unit: &str) -> (u32, Option<OwnedFd>) {
        let Ok(before) = self.deps.units.show(unit) else {
            return (0, None);
        };
        if !before.running || before.main_pid == 0 {
            return (0, None);
        }
        let Ok(fd) = files::pidfd(before.main_pid) else {
            return (0, None);
        };
        match self.deps.units.show(unit) {
            Ok(after) if after.running && after.main_pid == before.main_pid => {
                (before.main_pid, Some(fd))
            }
            _ => (0, None),
        }
    }

    fn started(&self, session: &str) -> Result<String, Refusal> {
        if !self.state.lock().unwrap().started.contains(session) {
            return Err(Refusal::NotStarted);
        }
        Ok(units::unit_name(session))
    }

    fn signal(&self, session: &str, signal: &str) -> Result<(), Refusal> {
        let unit = self.started(session)?;
        let name = match signal {
            "hup" => "SIGHUP",
            "int" => "SIGINT",
            "term" => "SIGTERM",
            "kill" => "SIGKILL",
            _ => return Err(Refusal::BadSignal),
        };
        self.deps
            .units
            .kill(&unit, name)
            .map_err(|e| failed("signalling", e))
    }

    /// Stops the unit and reports how its main process ended, as systemd
    /// recorded it before the stop.
    fn end(&self, session: &str) -> Result<Answer, Refusal> {
        let unit = self.started(session)?;
        let state = self.deps.units.show(&unit).unwrap_or_default();
        self.stop_unit(&unit)
            .map_err(|e| failed("stopping the unit", e))?;
        self.state.lock().unwrap().started.remove(session);
        let mut out = Map::new();
        out.insert("exit_code".into(), json!(state.exit_code));
        out.insert("signal".into(), json!(state.signal));
        Ok((out, None))
    }

    fn stop_unit(&self, unit: &str) -> Result<()> {
        let units = &self.deps.units;
        if units.show(unit).is_ok_and(|s| s.loaded) {
            units.stop(unit)?;
            units.reset_failed(unit);
        }
        if units.show(unit).is_ok_and(|s| s.running) {
            anyhow::bail!("{unit} is still running after its stop");
        }
        Ok(())
    }

    /// Everything a session's user can have written, made the host's
    /// again; the unit stopped first if it is still there.
    fn clean(&self, session: &str, user: &str) -> Result<(), Refusal> {
        let unit = units::unit_name(session);
        self.stop_unit(&unit)
            .map_err(|e| failed("stopping the unit", e))?;
        self.state.lock().unwrap().started.remove(session);
        let record = self.state.lock().unwrap().records.get(user).cloned();
        self.revoke(session, user, record.as_ref())
            .map_err(|e| failed("cleaning", e))
    }

    fn revoke(&self, session: &str, user: &str, record: Option<&LeaseRecord>) -> Result<()> {
        let own = self.deps.own.as_ref();
        let uid = self.pool.users[user];
        if let Some(workspace) = record.and_then(|r| r.workspace.as_ref())
            && workspace.is_dir()
        {
            files::hand_back(workspace, uid, own)
                .with_context(|| format!("handing back {}", workspace.display()))?;
        }
        let session_dir = self.layout.session_dir(session);
        let host_depth = self.layout.host_state.components().count();
        if session_dir.is_dir()
            && files::open_owned_dir(&session_dir, host_depth, self.pool.host_uid).is_ok()
        {
            files::retarget(&session_dir, self.pool.host_uid, own, &|k, m| {
                files::unshared(k, m, self.pool.host_gid)
            })?;
        }
        files::remove_tree(&self.layout.home(session), own)?;
        files::remove_tree(&self.layout.private(session), own)?;
        Ok(())
    }

    /// Lease end: cleaned first, whatever the host did before, then free.
    fn release(&self, session: &str, user: &str) -> Result<(), Refusal> {
        self.clean(session, user)?;
        files::remove_lease(&self.layout.leases(), user)
            .map_err(|e| failed("removing the lease", e))?;
        let mut st = self.state.lock().unwrap();
        st.records.remove(user);
        st.leases.release(session, user);
        Ok(())
    }

    /// What a previous helper left: every recorded lease is ended and
    /// revoked before this one answers.
    pub(crate) fn recover(&self) -> Result<()> {
        for (user, record) in files::load_leases(&self.layout.leases())? {
            if !self.pool.users.contains_key(&user) {
                continue;
            }
            self.stop_unit(&units::unit_name(&record.session))?;
            self.revoke(&record.session, &user, Some(&record))?;
            files::remove_lease(&self.layout.leases(), &user)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::audit::Audit;
    use crate::convert::{Entry, Key, Meta, Ownership, Recorded};
    use crate::serve::{Deps, Helper};
    use crate::units::{UnitState, Units};
    use std::sync::{Arc, Mutex};

    /// Recorded ownership, shareable.
    #[derive(Default, Clone)]
    pub struct Own(pub Arc<Mutex<Recorded>>);

    impl Ownership for Own {
        fn meta(&self, key: Key, seen: Meta) -> Meta {
            self.0.lock().unwrap().meta(key, seen)
        }
        fn chown(&self, e: &Entry, uid: u32, gid: u32) -> Result<()> {
            self.0.lock().unwrap().chown(e, uid, gid)
        }
        fn chmod(&self, e: &Entry, mode: u32) -> Result<()> {
            self.0.lock().unwrap().chmod(e, mode)
        }
        fn remove_all(&self, path: &Path) -> Result<()> {
            self.0.lock().unwrap().remove_all(path)?;
            std::fs::remove_dir_all(path)?;
            Ok(())
        }
    }

    /// systemd, as a table: units start running a real process (so the
    /// pidfd is real), and every call is recorded.
    #[derive(Default)]
    pub struct FakeUnits {
        pub calls: Mutex<Vec<String>>,
        pub specs: Mutex<Vec<UnitSpec>>,
        pub units: Mutex<BTreeMap<String, (UnitState, Option<std::process::Child>)>>,
        pub fail_start: std::sync::atomic::AtomicBool,
    }

    impl FakeUnits {
        /// The main process exits with `code`, as an agent would.
        pub fn exit(&self, unit: &str, code: Option<i32>, signal: Option<i32>) {
            let mut units = self.units.lock().unwrap();
            let (state, child) = units.get_mut(unit).unwrap();
            if let Some(mut c) = child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            state.running = false;
            state.main_pid = 0;
            state.exit_code = code;
            state.signal = signal;
        }
    }

    impl Units for Arc<FakeUnits> {
        fn start(&self, spec: &UnitSpec) -> Result<()> {
            let unit = units::unit_name(&spec.session);
            self.calls.lock().unwrap().push(format!("start {unit}"));
            self.specs.lock().unwrap().push(spec.clone());
            if self.fail_start.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("injected");
            }
            let child = std::process::Command::new("sleep").arg("30").spawn()?;
            let state = UnitState {
                loaded: true,
                running: true,
                main_pid: child.id(),
                ..UnitState::default()
            };
            self.units
                .lock()
                .unwrap()
                .insert(unit, (state, Some(child)));
            Ok(())
        }
        fn show(&self, unit: &str) -> Result<UnitState> {
            Ok(self
                .units
                .lock()
                .unwrap()
                .get(unit)
                .map(|(s, _)| *s)
                .unwrap_or_default())
        }
        fn kill(&self, unit: &str, signal: &str) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("kill {unit} {signal}"));
            if !self.units.lock().unwrap().contains_key(unit) {
                anyhow::bail!("no such unit");
            }
            Ok(())
        }
        fn stop(&self, unit: &str) -> Result<()> {
            self.calls.lock().unwrap().push(format!("stop {unit}"));
            if let Some((_, Some(mut c))) = self.units.lock().unwrap().remove(unit) {
                let _ = c.kill();
                let _ = c.wait();
            }
            Ok(())
        }
        fn reset_failed(&self, _unit: &str) {}
    }

    pub fn fake_deps() -> (Deps, Arc<FakeUnits>, Own, Arc<crate::tty::Recorded>) {
        let units = Arc::new(FakeUnits::default());
        let own = Own::default();
        let tty = Arc::new(crate::tty::Recorded::default());
        struct T(Arc<crate::tty::Recorded>);
        impl crate::tty::TtyOwner for T {
            fn give(&self, fd: std::os::fd::BorrowedFd<'_>, uid: u32) -> std::io::Result<()> {
                self.0.give(fd, uid)
            }
        }
        (
            Deps {
                units: Box::new(units.clone()),
                own: Box::new(own.clone()),
                tty: Box::new(T(tty.clone())),
            },
            units,
            own,
            tty,
        )
    }

    struct Fixture {
        _t: tempfile::TempDir,
        root: PathBuf,
        helper: Helper,
        units: Arc<FakeUnits>,
        own: Own,
        tty: Arc<crate::tty::Recorded>,
    }

    fn me() -> u32 {
        rustix::process::getuid().as_raw()
    }

    /// A host account with this test's uid, so the host's files are its.
    fn fixture() -> Fixture {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let root = base.join("workspaces");
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(root.join("app/README"), b"x").unwrap();
        let mut accounts = crate::pool::tests::fixed();
        accounts.add_user("storm-runtime", me(), me(), "/usr/sbin/nologin");
        let pool = crate::pool::Pool::resolve(&accounts, "storm-runtime").unwrap();
        let (deps, units, own, tty) = fake_deps();
        let mut settings = crate::serve::tests::settings(&root);
        settings.layout = Layout {
            state: base.join("isolation"),
            host_state: base.join("host"),
        };
        settings.commands = vec![("shell".into(), String::new())];
        let helper = Helper::new(pool, settings, deps);
        std::fs::create_dir_all(helper.layout.launch_dir()).unwrap();
        Fixture {
            _t: t,
            root,
            helper,
            units,
            own,
            tty,
        }
    }

    impl Fixture {
        fn call(&self, request: Value, fds: Vec<OwnedFd>) -> (Value, Option<OwnedFd>) {
            let mut lines: Vec<String> = Vec::new();
            let (line, fd) = self.helper.handle(
                Some(me()),
                request.to_string().as_bytes(),
                fds,
                &mut lines as &mut dyn Audit,
            );
            (serde_json::from_str(&line).unwrap(), fd)
        }

        fn ok(&self, request: Value, fds: Vec<OwnedFd>) -> (Value, Option<OwnedFd>) {
            let (v, fd) = self.call(request.clone(), fds);
            assert_eq!(v["ok"], true, "{request} → {v}");
            (v, fd)
        }

        fn workspace(&self) -> String {
            self.root.join("app").to_str().unwrap().to_string()
        }

        fn launch(&self, session: &str) {
            let dir = self.helper.layout.session_dir(session);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("mcp.json"), b"{}").unwrap();
            std::fs::write(
                self.helper.layout.launch_dir().join(format!("{session}.json")),
                json!({"args": ["-l"], "env": [["TERM", "xterm-256color"], ["TOKEN", "it's $secret"]]})
                    .to_string(),
            )
            .unwrap();
        }

        /// lease → prepare → start, as the host does it.
        fn started(&self, session: &str) -> (String, Value, Option<OwnedFd>, OwnedFd) {
            let (v, _) = self.ok(json!({"op": "lease", "session": session}), vec![]);
            let user = v["user"].as_str().unwrap().to_string();
            self.ok(
                json!({"op": "prepare", "session": session, "user": user}),
                vec![],
            );
            self.launch(session);
            let (master, slave) = crate::tty::tests::pty();
            let (v, pidfd) = self.ok(
                json!({"op": "start", "session": session, "user": user, "provider": "shell", "workspace": self.workspace()}),
                vec![slave],
            );
            (user, v, pidfd, master)
        }
    }

    /// AT-1's half the helper decides: two sessions, two users, two units.
    #[test]
    fn at_1_each_session_runs_as_its_own_leased_user() {
        let f = fixture();
        let (a, ..) = f.started("ags_A");
        let (b, ..) = f.started("ags_B");
        assert_ne!(a, b);
        let specs = f.units.specs.lock().unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].user, a);
        assert_eq!(specs[1].user, b);
        assert_ne!(specs[0].home, specs[1].home);
        assert_eq!(f.tty.0.lock().unwrap().len(), 2, "each terminal given away");
        assert_eq!(
            *f.tty.0.lock().unwrap(),
            [f.helper.pool.users[&a], f.helper.pool.users[&b]]
        );
    }

    #[test]
    fn a_start_runs_the_operators_program_with_the_hosts_arguments() {
        let f = fixture();
        let (user, v, pidfd, _master) = f.started("ags_A");
        let spec = f.units.specs.lock().unwrap()[0].clone();
        assert_eq!(spec.program, PathBuf::from("/bin/sh"));
        assert_eq!(spec.args, ["-l"]);
        assert_eq!(spec.workspace, PathBuf::from(f.workspace()));
        assert_eq!(spec.workspace_group, "storm-w001");
        assert_eq!(spec.user, user);
        assert!(spec.tty.starts_with("/dev/pts/"));
        assert_eq!((spec.rows, spec.cols), (33, 101));
        assert_eq!(
            spec.hidden,
            [
                f.helper.layout.host_state.clone(),
                f.helper.layout.state.clone(),
                f.root.clone()
            ]
        );
        // The leader's pidfd is the unit's main process.
        let pid = v["pid"].as_u64().unwrap() as u32;
        assert_eq!(
            pid,
            f.units.show(&units::unit_name("ags_A")).unwrap().main_pid
        );
        assert!(pidfd.is_some());
        // The environment is the helper's own file, never the unit's
        // properties; the host's HOME never gets in.
        let env = std::fs::read_to_string(f.helper.layout.env_file("ags_A")).unwrap();
        assert_eq!(env, "TERM=xterm-256color\nTOKEN=\"it's \\$secret\"\n");
        assert!(
            spec.properties()
                .unwrap()
                .iter()
                .all(|p| !p.contains("secret"))
        );
        // The workspace is recorded on disk before the unit starts.
        let leases = files::load_leases(&f.helper.layout.leases()).unwrap();
        assert_eq!(leases[&user].workspace, Some(PathBuf::from(f.workspace())));
    }

    #[test]
    fn a_start_needs_the_terminal_and_only_a_terminal() {
        let f = fixture();
        let (v, _) = f.ok(json!({"op": "lease", "session": "ags_A"}), vec![]);
        let user = v["user"].as_str().unwrap().to_string();
        f.launch("ags_A");
        let start = json!({"op": "start", "session": "ags_A", "user": user, "provider": "shell", "workspace": f.workspace()});
        let (v, _) = f.call(start.clone(), vec![]);
        assert_eq!(v["error"], "no_terminal");
        let (_r, w) = std::io::pipe().unwrap();
        let (v, _) = f.call(start.clone(), vec![OwnedFd::from(w)]);
        assert_eq!(v["error"], "not_a_terminal");
        let (master, slave) = crate::tty::tests::pty();
        let (v, _) = f.call(start.clone(), vec![master]);
        assert_eq!(
            v["error"], "not_a_terminal",
            "the master is not the session's end"
        );
        let (_m, slave2) = crate::tty::tests::pty();
        let (v, _) = f.call(start.clone(), vec![slave, slave2]);
        assert_eq!(v["error"], "unexpected_descriptor");
        assert!(f.units.specs.lock().unwrap().is_empty(), "nothing started");
    }

    #[test]
    fn a_start_without_a_valid_launch_is_refused() {
        let f = fixture();
        let (v, _) = f.ok(json!({"op": "lease", "session": "ags_A"}), vec![]);
        let user = v["user"].as_str().unwrap().to_string();
        let start = json!({"op": "start", "session": "ags_A", "user": user, "provider": "shell", "workspace": f.workspace()});
        let (_m, slave) = crate::tty::tests::pty();
        let (v, _) = f.call(start.clone(), vec![slave]);
        assert_eq!(v["error"], "bad_launch", "no launch file");
        std::fs::write(
            f.helper.layout.launch_dir().join("ags_A.json"),
            br#"{"args":[],"env":[],"program":"/bin/evil"}"#,
        )
        .unwrap();
        let (_m, slave) = crate::tty::tests::pty();
        let (v, _) = f.call(start, vec![slave]);
        assert_eq!(v["error"], "bad_launch", "a launch cannot name a program");
    }

    #[test]
    fn a_unit_that_fails_to_start_leaves_nothing_running() {
        let f = fixture();
        f.units
            .fail_start
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (v, _) = f.ok(json!({"op": "lease", "session": "ags_A"}), vec![]);
        let user = v["user"].as_str().unwrap().to_string();
        f.launch("ags_A");
        let (_m, slave) = crate::tty::tests::pty();
        let (v, _) = f.call(
            json!({"op": "start", "session": "ags_A", "user": user, "provider": "shell", "workspace": f.workspace()}),
            vec![slave],
        );
        assert_eq!(v["error"], "failed");
        let calls = f.units.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                "start storm-session-ags_A.service",
                "stop storm-session-ags_A.service"
            ]
        );
        let (v, _) = f.call(json!({"op": "end", "session": "ags_A"}), vec![]);
        assert_eq!(v["error"], "not_started");
    }

    /// The exit status or signal is systemd's record of the main process.
    #[test]
    fn end_stops_the_unit_and_reports_how_it_ended() {
        let f = fixture();
        f.started("ags_A");
        f.units.exit(&units::unit_name("ags_A"), Some(3), None);
        let (v, _) = f.ok(json!({"op": "end", "session": "ags_A"}), vec![]);
        assert_eq!(
            (v["exit_code"].clone(), v["signal"].clone()),
            (json!(3), Value::Null)
        );
        f.started("ags_B");
        f.units.exit(&units::unit_name("ags_B"), None, Some(9));
        let (v, _) = f.ok(json!({"op": "end", "session": "ags_B"}), vec![]);
        assert_eq!(
            (v["exit_code"].clone(), v["signal"].clone()),
            (Value::Null, json!(9))
        );
        let calls = f.units.calls.lock().unwrap().clone();
        assert!(calls.contains(&"stop storm-session-ags_A.service".to_string()));
        assert!(
            f.units
                .show("storm-session-ags_A.service")
                .map(|s| !s.loaded)
                .unwrap()
        );
        let (v, _) = f.call(json!({"op": "end", "session": "ags_A"}), vec![]);
        assert_eq!(v["error"], "not_started", "ended once");
    }

    /// IT-3's half the helper decides: a signal and the end reach the whole
    /// unit, not a process or a process group.
    #[test]
    fn it_3_signals_and_the_end_are_the_whole_units() {
        let f = fixture();
        f.started("ags_A");
        f.ok(
            json!({"op": "signal", "session": "ags_A", "signal": "hup"}),
            vec![],
        );
        f.ok(
            json!({"op": "signal", "session": "ags_A", "signal": "kill"}),
            vec![],
        );
        f.ok(json!({"op": "end", "session": "ags_A"}), vec![]);
        let calls = f.units.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                "start storm-session-ags_A.service",
                "kill storm-session-ags_A.service SIGHUP",
                "kill storm-session-ags_A.service SIGKILL",
                "stop storm-session-ags_A.service",
            ]
        );
        let spec = f.units.specs.lock().unwrap()[0].clone();
        let props = spec.properties().unwrap();
        assert!(props.contains(&"KillMode=control-group".to_string()));
        let (v, _) = f.call(
            json!({"op": "signal", "session": "ags_A", "signal": "hup"}),
            vec![],
        );
        assert_eq!(v["error"], "not_started");
    }

    /// IT-2's half the helper decides: at lease end the unit is stopped, the
    /// user's files are handed back, its home is gone, and only then is the
    /// user leased again.
    #[test]
    fn it_2_lease_end_revokes_before_the_user_is_leased_again() {
        let f = fixture();
        let (user, ..) = f.started("ags_A");
        let uid = f.helper.pool.users[&user];
        // The session wrote a file in the workspace, and left a process.
        let made = f.root.join("app/made-by-session");
        std::fs::write(&made, b"x").unwrap();
        let md = std::fs::metadata(&made).unwrap();
        use std::os::unix::fs::MetadataExt;
        f.own.0.lock().unwrap().overlay.borrow_mut().insert(
            Key {
                dev: md.dev(),
                ino: md.ino(),
            },
            Meta {
                uid,
                gid: uid,
                mode: 0o600,
            },
        );
        let home = f.helper.layout.home("ags_A");
        assert!(home.is_dir());
        // Released without an end: the release does it.
        f.ok(
            json!({"op": "release", "session": "ags_A", "user": user}),
            vec![],
        );
        let calls = f.units.calls.lock().unwrap().clone();
        assert_eq!(calls.last().unwrap(), "stop storm-session-ags_A.service");
        let meta = f.own.meta(
            Key {
                dev: md.dev(),
                ino: md.ino(),
            },
            Meta {
                uid: 0,
                gid: 0,
                mode: 0,
            },
        );
        let ws = std::fs::metadata(f.root.join("app")).unwrap();
        assert_eq!((meta.uid, meta.mode), (ws.uid(), 0o660));
        assert!(!home.exists());
        assert!(!f.helper.layout.private("ags_A").exists());
        assert!(
            files::load_leases(&f.helper.layout.leases())
                .unwrap()
                .is_empty()
        );
        // Only now is it free, and it comes back last.
        let st = f.helper.state.lock().unwrap();
        assert_eq!(st.leases.holder(&user), None);
    }

    #[test]
    fn a_lease_is_one_user_per_session_until_the_pool_runs_out() {
        let f = fixture();
        f.ok(json!({"op": "lease", "session": "ags_A"}), vec![]);
        let (v, _) = f.call(json!({"op": "lease", "session": "ags_A"}), vec![]);
        assert_eq!(v["error"], "already_leased");
        for n in 1..64 {
            f.ok(
                json!({"op": "lease", "session": format!("ags_N{n}")}),
                vec![],
            );
        }
        let (v, _) = f.call(json!({"op": "lease", "session": "ags_Z"}), vec![]);
        assert_eq!(v["error"], "pool_exhausted");
    }

    /// A helper that restarts ends and revokes what the last one left
    /// before it answers.
    #[test]
    fn a_restarted_helper_ends_what_the_last_one_left() {
        let f = fixture();
        let (user, ..) = f.started("ags_A");
        let records = files::load_leases(&f.helper.layout.leases()).unwrap();
        assert_eq!(records.len(), 1);
        f.helper.recover().unwrap();
        let calls = f.units.calls.lock().unwrap().clone();
        assert_eq!(calls.last().unwrap(), "stop storm-session-ags_A.service");
        assert!(
            files::load_leases(&f.helper.layout.leases())
                .unwrap()
                .is_empty()
        );
        assert!(!f.helper.layout.home("ags_A").exists());
        let _ = user;
    }

    #[test]
    fn the_session_directory_is_shared_read_only_and_given_back() {
        let f = fixture();
        let (user, ..) = f.started("ags_A");
        let gid = f.helper.pool.users[&user];
        let dir = f.helper.layout.session_dir("ags_A");
        let own = f.own.clone();
        let metas = |p: &Path| {
            use std::os::unix::fs::MetadataExt;
            let md = std::fs::metadata(p).unwrap();
            own.meta(
                Key {
                    dev: md.dev(),
                    ino: md.ino(),
                },
                Meta {
                    uid: md.uid(),
                    gid: md.gid(),
                    mode: md.mode() & 0o7777,
                },
            )
        };
        let shared = metas(&dir.join("mcp.json"));
        assert_eq!(shared.gid, gid);
        assert_eq!(shared.mode & 0o070, 0o040, "read, never write");
        f.ok(
            json!({"op": "release", "session": "ags_A", "user": user}),
            vec![],
        );
        let back = metas(&dir.join("mcp.json"));
        assert_eq!(back.gid, f.helper.pool.host_gid);
        assert_eq!(back.mode & 0o070, 0);
    }

    /// The terminal goes from one process to the helper, and the leader's
    /// pidfd comes back, over a real socket between two processes: the
    /// second is this test binary again, as the host.
    #[test]
    fn the_terminal_and_the_pidfd_cross_between_two_processes() {
        const SOCK: &str = "STORM_ISOLATION_TEST_SOCKET";
        if let Ok(path) = std::env::var(SOCK) {
            return host_side(&path, &std::env::var("STORM_ISOLATION_TEST_START").unwrap());
        }
        let f = fixture();
        let (v, _) = f.ok(json!({"op": "lease", "session": "ags_X"}), vec![]);
        let user = v["user"].as_str().unwrap().to_string();
        f.ok(
            json!({"op": "prepare", "session": "ags_X", "user": user}),
            vec![],
        );
        f.launch("ags_X");
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("helper.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let start = json!({"op": "start", "session": "ags_X", "user": user, "provider": "shell", "workspace": f.workspace()});
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::tests::the_terminal_and_the_pidfd_cross_between_two_processes",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(SOCK, &sock)
            .env("STORM_ISOLATION_TEST_START", start.to_string())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut lines: Vec<String> = Vec::new();
        f.helper.connection(stream, &mut lines);
        let out = child.wait_with_output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{said}");
        assert_eq!(
            lines,
            [format!("audit: accepted op=start peer_uid={}", me())]
        );
        let tty = said
            .lines()
            .find_map(|l| l.split_once("host-side pts=").map(|(_, t)| t))
            .unwrap_or_else(|| panic!("no terminal named in {said:?}"));
        let spec = f.units.specs.lock().unwrap()[0].clone();
        assert_eq!(
            spec.tty,
            PathBuf::from(tty),
            "the unit gets the terminal sent"
        );
        assert_eq!(*f.tty.0.lock().unwrap(), [f.helper.pool.users[&user]]);
        let pid = f.units.show(&units::unit_name("ags_X")).unwrap().main_pid;
        assert!(
            said.contains(&format!("host-side pidfd-pid={pid}")),
            "{said}"
        );
    }

    fn host_side(sock: &str, start: &str) {
        let (master, slave) = crate::tty::tests::pty();
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let stream = std::os::unix::net::UnixStream::connect(sock).unwrap();
        crate::fdpass::send_line(
            &stream,
            format!("{start}\n").as_bytes(),
            Some(slave.as_fd()),
        )
        .unwrap();
        drop(slave);
        let got = crate::fdpass::recv_line(&stream, 4096).unwrap();
        let answer: Value = serde_json::from_slice(&got.line).unwrap();
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(got.fds.len(), 1);
        let fd = std::os::fd::AsRawFd::as_raw_fd(&got.fds[0]);
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).unwrap();
        let pidfd_pid = info
            .lines()
            .find_map(|l| l.strip_prefix("Pid:"))
            .map(str::trim)
            .unwrap()
            .to_string();
        assert_eq!(answer["pid"].to_string(), pidfd_pid);
        println!("host-side pts={}", name.to_str().unwrap());
        println!("host-side pidfd-pid={pidfd_pid}");
    }

    #[test]
    fn one_operation_at_a_time_per_session() {
        let f = fixture();
        let (user, ..) = f.started("ags_A");
        let busy = f.helper.session_busy("ags_A").unwrap();
        let (v, _) = f.call(
            json!({"op": "release", "session": "ags_A", "user": user}),
            vec![],
        );
        assert_eq!(v["error"], "busy");
        drop(busy);
        f.ok(
            json!({"op": "release", "session": "ags_A", "user": user}),
            vec![],
        );
    }
}
