//! The helper service (136c). It takes its listening socket from systemd and
//! never binds one itself, so the socket's directory, owner and mode are the
//! socket unit's alone. One request per connection; connections are served
//! side by side, since ending one session waits out its grace.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

use crate::accounts::Accounts;
use crate::audit::{self, Audit};
use crate::config::{HelperConfig, Isolation};
use crate::convert::Ownership;
use crate::fdpass;
use crate::pool::Pool;
use crate::protocol::{self, MAX_REQUEST, Op, Refusal};
use crate::session::{Layout, State};
use crate::tty::TtyOwner;
use crate::units::Units;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// More connections than sessions the pool can hold is a misbehaving peer.
const MAX_CONNECTIONS: usize = 128;

/// What the helper changes on the machine, so every operation runs on
/// fakes in tests.
pub struct Deps {
    pub units: Box<dyn Units>,
    pub own: Box<dyn Ownership + Send + Sync>,
    pub tty: Box<dyn TtyOwner>,
}

impl Deps {
    pub fn real() -> Self {
        Self {
            units: Box::new(crate::units::Systemd),
            own: Box::new(crate::convert::Real),
            tty: Box::new(crate::tty::Real),
        }
    }
}

pub struct Helper {
    pub(crate) pool: Pool,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) providers: Vec<String>,
    /// Provider id → the command it runs.
    pub(crate) commands: Vec<(String, String)>,
    /// Where commands are looked for: the agents Storm installed first.
    pub(crate) search: Vec<PathBuf>,
    /// The host account's login shell, for `shell`.
    pub(crate) shell: PathBuf,
    /// The host's loopback ports sessions may reach (`net.rs`).
    pub(crate) host_ports: Vec<u16>,
    pub(crate) layout: Layout,
    pub(crate) deps: Deps,
    pub(crate) state: Mutex<State>,
}

pub struct Settings {
    pub roots: Vec<PathBuf>,
    pub providers: Vec<String>,
    pub commands: Vec<(String, String)>,
    pub search: Vec<PathBuf>,
    pub shell: PathBuf,
    pub host_ports: Vec<u16>,
    pub layout: Layout,
}

impl Settings {
    pub fn from_config(config: &HelperConfig, layout: Layout, shell: PathBuf) -> Result<Self> {
        let mut search = vec![layout.agents().join("bin")];
        match &config.path {
            Some(dirs) => search.extend(dirs.iter().cloned()),
            None => search.extend(
                std::env::var_os("PATH")
                    .iter()
                    .flat_map(std::env::split_paths),
            ),
        }
        Ok(Self {
            roots: config.canonical_roots(),
            providers: config.provider_ids(),
            commands: config.provider_commands(),
            search,
            shell,
            host_ports: crate::net::host_ports(&config.isolation_host_ports)?,
            layout,
        })
    }
}

impl Helper {
    /// Refuses to start unless isolation is on and the pools are exactly
    /// what 136b declares: a helper with a doubtful pool must not answer.
    pub fn load(accounts: &dyn Accounts, config: &Path, deps: Deps) -> Result<Self> {
        let config = HelperConfig::load(config)?;
        if config.isolation != Isolation::PerSession {
            bail!("isolation is off in runtime.toml; the helper has nothing to do");
        }
        let pool = Pool::resolve(accounts, crate::HOST_ACCOUNT)?;
        let shell = accounts
            .user(crate::HOST_ACCOUNT)
            .map(|u| u.shell)
            .filter(|s| !s.is_empty() && !crate::pool::NOLOGIN.contains(&s.as_str()))
            .unwrap_or_else(|| "/bin/sh".into());
        let settings = Settings::from_config(&config, Layout::installed(), shell.into())?;
        let helper = Self::new(pool, settings, deps);
        helper.recover()?;
        Ok(helper)
    }

    pub fn new(pool: Pool, settings: Settings, deps: Deps) -> Self {
        let state = State::new(pool.users.keys().cloned());
        Self {
            pool,
            roots: settings.roots,
            providers: settings.providers,
            commands: settings.commands,
            search: settings.search,
            shell: settings.shell,
            host_ports: settings.host_ports,
            layout: settings.layout,
            deps,
            state: Mutex::new(state),
        }
    }

    /// One request, from a peer whose uid the kernel reported, with the
    /// descriptors that came with it. Returns the response line and the
    /// descriptor to send back with it.
    pub fn handle(
        &self,
        peer_uid: Option<u32>,
        request: &[u8],
        fds: Vec<OwnedFd>,
        audit: &mut dyn Audit,
    ) -> (String, Option<OwnedFd>) {
        let result = crate::policy::authorize_peer(peer_uid, self.pool.host_uid)
            .map_err(|r| (None, r))
            .and_then(|()| protocol::parse(request).map_err(|r| (None, r)))
            .and_then(|req| {
                if fds.len() > 1 || (!fds.is_empty() && req.op != Op::Start) {
                    return Err((Some(req.op), Refusal::UnexpectedDescriptor));
                }
                let op = req.op;
                self.carry_out(&req, fds.into_iter().next())
                    .map(|answer| (op, answer))
                    .map_err(|r| (Some(op), r))
            });
        match result {
            Ok((op, (fields, fd))) => {
                audit::accepted(audit, op, peer_uid);
                (protocol::ok_with(op, fields), fd)
            }
            Err((op, why)) => {
                audit::refused(audit, op, &why, peer_uid);
                (protocol::refused(&why), None)
            }
        }
    }

    pub fn serve(
        self: Arc<Self>,
        listener: &UnixListener,
        audit: Arc<Mutex<dyn Audit + Send>>,
    ) -> ! {
        let open = Arc::new(AtomicUsize::new(0));
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        open.fetch_sub(1, Ordering::SeqCst);
                        let _ = fdpass::send_line(
                            &stream,
                            protocol::refused(&Refusal::Busy).as_bytes(),
                            None,
                        );
                        continue;
                    }
                    let (helper, audit, open) = (self.clone(), audit.clone(), open.clone());
                    std::thread::spawn(move || {
                        let mut lines: Vec<String> = Vec::new();
                        helper.connection(stream, &mut lines);
                        let mut audit = audit.lock().unwrap();
                        for line in lines {
                            audit.line(line);
                        }
                        open.fetch_sub(1, Ordering::SeqCst);
                    });
                }
                Err(e) => {
                    audit.lock().unwrap().line(format!("accept failed: {e}"));
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    pub(crate) fn connection(&self, stream: UnixStream, audit: &mut dyn Audit) {
        let peer = rustix::net::sockopt::socket_peercred(&stream)
            .ok()
            .map(|c| c.uid.as_raw());
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        // The peer is checked before a byte is read, so nobody else can make
        // the helper parse anything.
        if let Err(why) = crate::policy::authorize_peer(peer, self.pool.host_uid) {
            audit::refused(audit, None, &why, peer);
            let _ = fdpass::send_line(&stream, protocol::refused(&why).as_bytes(), None);
            return;
        }
        let (response, fd) = match fdpass::recv_line(&stream, MAX_REQUEST) {
            Ok(got) if got.too_large => {
                audit::refused(audit, None, &Refusal::TooLarge, peer);
                (protocol::refused(&Refusal::TooLarge), None)
            }
            Ok(got) => {
                // The answer to `end` may take the stop's grace.
                let _ = stream.set_read_timeout(None);
                self.handle(peer, &got.line, got.fds, audit)
            }
            Err(_) => {
                audit::refused(audit, None, &Refusal::Malformed, peer);
                (protocol::refused(&Refusal::Malformed), None)
            }
        };
        let _ = fdpass::send_line(&stream, response.as_bytes(), fd.as_ref().map(|f| f.as_fd()));
    }
}

/// The socket systemd passed (`sd_listen_fds`): exactly one, meant for this
/// process.
pub fn listener_from_systemd() -> Result<UnixListener> {
    let pid =
        std::env::var("LISTEN_PID").context("not started by its socket unit (no LISTEN_PID)")?;
    if pid.parse::<u32>().ok() != Some(std::process::id()) {
        bail!("LISTEN_PID is not this process");
    }
    if std::env::var("LISTEN_FDS").as_deref() != Ok("1") {
        bail!("expected exactly one socket from systemd");
    }
    // SAFETY: systemd's protocol hands the first socket over as fd 3, and
    // nothing in this process has opened anything that could be fd 3 yet.
    let fd = unsafe { OwnedFd::from_raw_fd(3) };
    let listener = UnixListener::from(fd);
    if !rustix::net::sockopt::socket_acceptconn(&listener).unwrap_or(false) {
        bail!("fd 3 is not a listening socket");
    }
    // SAFETY: single-threaded at this point.
    unsafe {
        std::env::remove_var("LISTEN_PID");
        std::env::remove_var("LISTEN_FDS");
        std::env::remove_var("LISTEN_FDNAMES");
    }
    Ok(listener)
}

pub fn run(config: &Path) -> Result<()> {
    if !rustix::process::geteuid().is_root() {
        bail!("the isolation helper runs as root, from its unit");
    }
    let listener = listener_from_systemd()?;
    let helper = Helper::load(&crate::accounts::System, config, Deps::real())?;
    let journal: Arc<Mutex<dyn Audit + Send>> = Arc::new(Mutex::new(audit::Journal));
    journal.lock().unwrap().line(format!(
        "storm-runtime-isolation helper serving: {} pool users, {} roots",
        helper.pool.users.len(),
        helper.roots.len()
    ));
    Arc::new(helper).serve(&listener, journal)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::{Read, Write};

    pub fn settings(root: &Path) -> Settings {
        Settings {
            roots: vec![root.to_path_buf()],
            providers: vec!["shell".into()],
            commands: vec![("shell".into(), String::new())],
            search: vec![],
            shell: "/bin/sh".into(),
            host_ports: vec![],
            layout: Layout {
                state: root.join(".isolation"),
                host_state: root.join(".host"),
            },
        }
    }

    pub fn helper(root: &Path) -> Helper {
        let pool = Pool::resolve(&crate::pool::tests::fixed(), "storm-runtime").unwrap();
        Helper::new(pool, settings(root), crate::session::tests::fake_deps().0)
    }

    fn lease(h: &Helper, session: &str) -> String {
        h.state.lock().unwrap().lease(session).unwrap()
    }

    /// IT-1: each kind of bad request is refused, and audited.
    #[test]
    fn it_1_bad_requests_are_refused_and_audited() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("app")).unwrap();
        let h = helper(&root);
        // Another session holds storm-s001, so it is a leased user.
        assert_eq!(lease(&h, "ags_OTHER"), "storm-s001");
        assert_eq!(lease(&h, "ags_A"), "storm-s002");
        let ws = root.join("app").to_str().unwrap().to_string();
        let cases: Vec<(String, &str)> = vec![
            (
                format!(r#"{{"op":"start","session":"ags_A","user":"storm-s002","provider":"shell","workspace":"{ws}","program":"/bin/sh"}}"#),
                "unexpected_field",
            ),
            (
                r#"{"op":"start","session":"ags_A","user":"storm-s002","provider":"shell","workspace":"/etc"}"#.into(),
                "path_outside_roots",
            ),
            (r#"{"op":"prepare","session":"ags_A","user":"root"}"#.into(), "root"),
            (r#"{"op":"prepare","session":"ags_A","user":"daemon"}"#.into(), "not_pool_user"),
            (r#"{"op":"prepare","session":"ags_A","user":"storm-runtime"}"#.into(), "host_account"),
            (r#"{"op":"prepare","session":"ags_A","user":"storm-s001"}"#.into(), "leased_elsewhere"),
            (r#"{"op":"chown","session":"ags_A"}"#.into(), "unknown_op"),
        ];
        for (request, code) in cases {
            let mut lines = Vec::new();
            let (response, fd) = h.handle(Some(900), request.as_bytes(), vec![], &mut lines);
            assert!(fd.is_none());
            assert_eq!(
                response,
                format!("{{\"error\":\"{code}\",\"ok\":false}}\n"),
                "{request}"
            );
            assert_eq!(lines.len(), 1, "{request}");
            assert!(lines[0].starts_with("audit: refused "), "{}", lines[0]);
            assert!(lines[0].contains(&format!("reason={code}")), "{}", lines[0]);
            assert!(
                !lines[0].contains("/bin/sh") && !lines[0].contains("/etc"),
                "{}",
                lines[0]
            );
        }
    }

    #[test]
    fn another_peer_is_refused_before_its_request_is_parsed() {
        let t = tempfile::tempdir().unwrap();
        let h = helper(t.path());
        for peer in [None, Some(0), Some(2001), Some(1000)] {
            let mut lines = Vec::new();
            let (response, _) = h.handle(peer, br#"{"op":"self_test"}"#, vec![], &mut lines);
            assert!(response.contains("peer_not_host"));
            assert_eq!(
                lines,
                [format!(
                    "audit: refused op=- reason=peer_not_host peer_uid={}",
                    peer.map_or("-".into(), |p: u32| p.to_string())
                )]
            );
        }
    }

    #[test]
    fn a_descriptor_is_taken_only_with_start() {
        let t = tempfile::tempdir().unwrap();
        let h = helper(t.path());
        let (_r, w) = std::io::pipe().unwrap();
        let mut lines = Vec::new();
        let (response, _) = h.handle(
            Some(900),
            br#"{"op":"lease","session":"ags_A"}"#,
            vec![OwnedFd::from(w)],
            &mut lines,
        );
        assert!(response.contains("unexpected_descriptor"), "{response}");
        assert_eq!(
            lines,
            ["audit: refused op=lease reason=unexpected_descriptor peer_uid=900"]
        );
        assert_eq!(h.state.lock().unwrap().leases.user_of("ags_A"), None);
    }

    #[test]
    fn the_operations_137_and_136_5_build_are_still_refused() {
        let t = tempfile::tempdir().unwrap();
        let h = helper(t.path());
        for request in [
            &br#"{"op":"self_test"}"#[..],
            br#"{"op":"child_export","child":"ags_C"}"#,
        ] {
            let mut lines = Vec::new();
            let (response, _) = h.handle(Some(900), request, vec![], &mut lines);
            assert!(response.contains("not_built"), "{response}");
        }
    }

    #[test]
    fn a_real_socket_carries_the_kernels_peer_uid() {
        let (a, b) = UnixStream::pair().unwrap();
        let cred = rustix::net::sockopt::socket_peercred(&a).unwrap();
        assert_eq!(cred.uid, rustix::process::getuid());
        drop(b);
    }

    #[test]
    fn over_a_real_socket_the_kernels_uid_decides() {
        let me = rustix::process::getuid().as_raw();
        if me == 0 {
            return;
        }
        // A refused peer's request is never read, so this one sends none:
        // closing over unread bytes would reset the connection.
        let talk = |host_uid: u32, request: &[u8]| {
            let mut accounts = crate::pool::tests::fixed();
            accounts.add_user("storm-runtime", host_uid, host_uid, "/usr/sbin/nologin");
            let pool = Pool::resolve(&accounts, "storm-runtime").unwrap();
            let t = tempfile::tempdir().unwrap();
            let h = Helper::new(
                pool,
                settings(t.path()),
                crate::session::tests::fake_deps().0,
            );
            let (client, server) = UnixStream::pair().unwrap();
            (&client).write_all(request).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            let mut lines = Vec::new();
            h.connection(server, &mut lines);
            let mut response = String::new();
            (&client).read_to_string(&mut response).unwrap();
            (response, lines)
        };
        let (response, lines) = talk(me, b"{\"op\":\"lease\",\"session\":\"ags_A\"}\n");
        assert!(response.contains("\"user\":\"storm-s001\""), "{response}");
        assert_eq!(lines, [format!("audit: accepted op=lease peer_uid={me}")]);
        let (response, lines) = talk(me + 1, b"");
        assert!(response.contains("peer_not_host"), "{response}");
        assert_eq!(
            lines,
            [format!(
                "audit: refused op=- reason=peer_not_host peer_uid={me}"
            )]
        );
    }

    #[test]
    fn the_helper_does_not_start_without_isolation_on() {
        let t = tempfile::tempdir().unwrap();
        let config = t.path().join("runtime.toml");
        std::fs::write(&config, "workspace_roots = []\n").unwrap();
        let fixed = crate::pool::tests::fixed();
        let deps = || crate::session::tests::fake_deps().0;
        assert!(Helper::load(&fixed, &config, deps()).is_err());
        let mut broken = crate::pool::tests::fixed();
        broken.users.remove("storm-s064");
        std::fs::write(&config, "isolation = \"per-session\"\n").unwrap();
        assert!(Helper::load(&broken, &config, deps()).is_err());
    }

    #[test]
    fn without_systemds_socket_it_does_not_serve() {
        // SAFETY: tests in this module do not read these variables.
        unsafe { std::env::remove_var("LISTEN_PID") };
        assert!(listener_from_systemd().is_err());
    }
}
