//! The helper service (136c). It takes its listening socket from systemd and
//! never binds one itself, so the socket's directory, owner and mode are the
//! socket unit's alone. One request per connection, answered in turn.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

use crate::accounts::Accounts;
use crate::audit::{self, Audit};
use crate::config::{HelperConfig, Isolation};
use crate::leases::Leases;
use crate::policy::{self, Context};
use crate::pool::Pool;
use crate::protocol::{self, MAX_REQUEST, Op, Refusal};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Helper {
    pool: Pool,
    roots: Vec<PathBuf>,
    providers: Vec<String>,
    leases: Leases,
}

impl Helper {
    /// Refuses to start unless isolation is on and the pools are exactly
    /// what 136b declares: a helper with a doubtful pool must not answer.
    pub fn load(accounts: &dyn Accounts, config: &Path) -> Result<Self> {
        let config = HelperConfig::load(config)?;
        if config.isolation != Isolation::PerSession {
            bail!("isolation is off in runtime.toml; the helper has nothing to do");
        }
        let pool = Pool::resolve(accounts, crate::HOST_ACCOUNT)?;
        Ok(Self::new(
            pool,
            config.canonical_roots(),
            config.provider_ids(),
        ))
    }

    pub fn new(pool: Pool, roots: Vec<PathBuf>, providers: Vec<String>) -> Self {
        let leases = Leases::new(pool.users.keys().cloned());
        Self {
            pool,
            roots,
            providers,
            leases,
        }
    }

    /// One request, from a peer whose uid the kernel reported. Returns the
    /// response line.
    pub fn handle(
        &mut self,
        peer_uid: Option<u32>,
        request: &[u8],
        audit: &mut dyn Audit,
    ) -> String {
        match self.decide(peer_uid, request) {
            Ok(op) => {
                audit::accepted(audit, op, peer_uid);
                // Validated, but no operation does anything until it is
                // built (136-2 onward): an honest refusal, not a no-op.
                protocol::refused(&Refusal::NotBuilt)
            }
            Err((op, why)) => {
                audit::refused(audit, op, &why, peer_uid);
                protocol::refused(&why)
            }
        }
    }

    fn decide(
        &mut self,
        peer_uid: Option<u32>,
        request: &[u8],
    ) -> Result<Op, (Option<Op>, Refusal)> {
        policy::authorize_peer(peer_uid, self.pool.host_uid).map_err(|r| (None, r))?;
        let req = protocol::parse(request).map_err(|r| (None, r))?;
        let cx = Context {
            pool: &self.pool,
            roots: &self.roots,
            providers: &self.providers,
            leases: &self.leases,
        };
        policy::validate(&req, &cx).map_err(|r| (Some(req.op), r))?;
        Ok(req.op)
    }

    pub fn serve(&mut self, listener: &UnixListener, audit: &mut dyn Audit) -> ! {
        loop {
            match listener.accept() {
                Ok((stream, _)) => self.connection(stream, audit),
                Err(e) => {
                    audit.line(format!("accept failed: {e}"));
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    fn connection(&mut self, stream: UnixStream, audit: &mut dyn Audit) {
        let peer = rustix::net::sockopt::socket_peercred(&stream)
            .ok()
            .map(|c| c.uid.as_raw());
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        // The peer is checked before a byte is read, so nobody else can make
        // the helper parse anything.
        if let Err(why) = policy::authorize_peer(peer, self.pool.host_uid) {
            audit::refused(audit, None, &why, peer);
            let _ = (&stream).write_all(protocol::refused(&why).as_bytes());
            return;
        }
        let response = match read_request(&stream) {
            Ok(line) => self.handle(peer, &line, audit),
            Err(why) => {
                audit::refused(audit, None, &why, peer);
                protocol::refused(&why)
            }
        };
        let _ = (&stream).write_all(response.as_bytes());
    }
}

/// One line, at most [`MAX_REQUEST`] bytes before its newline.
pub fn read_request(stream: impl Read) -> Result<Vec<u8>, Refusal> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST as u64 + 1));
    let mut line = Vec::new();
    reader
        .read_until(b'\n', &mut line)
        .map_err(|_| Refusal::Malformed)?;
    if line.last() == Some(&b'\n') {
        line.pop();
    } else if line.len() > MAX_REQUEST {
        return Err(Refusal::TooLarge);
    }
    Ok(line)
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
    let mut helper = Helper::load(&crate::accounts::System, config)?;
    let mut journal = audit::Journal;
    journal.line(format!(
        "storm-runtime-isolation helper serving: {} pool users, {} roots",
        helper.pool.users.len(),
        helper.roots.len()
    ));
    helper.serve(&listener, &mut journal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper(root: &Path) -> Helper {
        let pool = Pool::resolve(&crate::pool::tests::fixed(), "storm-runtime").unwrap();
        Helper::new(pool, vec![root.to_path_buf()], vec!["shell".into()])
    }

    /// IT-1: each kind of bad request is refused, and audited.
    #[test]
    fn it_1_bad_requests_are_refused_and_audited() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("app")).unwrap();
        let mut h = helper(&root);
        // Another session holds storm-s001, so it is a leased user.
        assert_eq!(h.leases.lease("ags_OTHER").as_deref(), Some("storm-s001"));
        assert_eq!(h.leases.lease("ags_A").as_deref(), Some("storm-s002"));
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
            let response = h.handle(Some(900), request.as_bytes(), &mut lines);
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
        let mut h = helper(t.path());
        for peer in [None, Some(0), Some(2001), Some(1000)] {
            let mut lines = Vec::new();
            let response = h.handle(peer, br#"{"op":"self_test"}"#, &mut lines);
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
    fn a_valid_request_is_audited_and_not_yet_carried_out() {
        let t = tempfile::tempdir().unwrap();
        let mut h = helper(t.path());
        let mut lines = Vec::new();
        let response = h.handle(
            Some(900),
            br#"{"op":"lease","session":"ags_A"}"#,
            &mut lines,
        );
        assert_eq!(response, "{\"error\":\"not_built\",\"ok\":false}\n");
        assert_eq!(lines, ["audit: accepted op=lease peer_uid=900"]);
        assert_eq!(h.leases.user_of("ags_A"), None, "nothing was leased");
    }

    #[test]
    fn a_connection_is_one_bounded_line() {
        assert_eq!(
            read_request(&b"{\"op\":\"self_test\"}\nrest"[..]).unwrap(),
            b"{\"op\":\"self_test\"}"
        );
        assert_eq!(read_request(&b"{}"[..]).unwrap(), b"{}");
        let big = vec![b'x'; MAX_REQUEST + 10];
        assert_eq!(read_request(&big[..]), Err(Refusal::TooLarge));
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
            let mut h = Helper::new(pool, vec![], vec![]);
            let (client, server) = UnixStream::pair().unwrap();
            (&client).write_all(request).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            let mut lines = Vec::new();
            h.connection(server, &mut lines);
            let mut response = String::new();
            (&client).read_to_string(&mut response).unwrap();
            (response, lines)
        };
        let (response, lines) = talk(me, b"{\"op\":\"self_test\"}\n");
        assert!(response.contains("not_built"), "{response}");
        assert_eq!(
            lines,
            [format!("audit: accepted op=self_test peer_uid={me}")]
        );
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
        assert!(Helper::load(&fixed, &config).is_err());
        std::fs::write(&config, "isolation = \"per-session\"\n").unwrap();
        assert!(Helper::load(&fixed, &config).is_ok());
        let mut broken = crate::pool::tests::fixed();
        broken.users.remove("storm-s064");
        assert!(Helper::load(&broken, &config).is_err());
    }

    #[test]
    fn without_systemds_socket_it_does_not_serve() {
        // SAFETY: tests in this module do not read these variables.
        unsafe { std::env::remove_var("LISTEN_PID") };
        assert!(listener_from_systemd().is_err());
    }
}
