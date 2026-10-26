//! The operator's command (136a): `on`, `off` and `status`, run as root.
//!
//! `on` checks the host can isolate (systemd ≥ 255, `passt`), creates the
//! pools from the package's declaration, converts every workspace, enables
//! the helper's socket and only then sets `isolation = "per-session"`.
//! `off` disables the helper, restores every workspace and sets
//! `isolation = "off"` last. Both refuse while the host is running, so no
//! session runs during a conversion. Whatever fails, the configuration is
//! the last thing written: a half-done `off` leaves the host refusing
//! sessions (fail closed), and either command can be run again.

use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::accounts::Accounts;
use crate::config::{self, HelperConfig, Isolation};
use crate::convert::{self, Manifest, Ownership};
use crate::pool::Pool;

/// 136k: per-unit network namespaces, `PrivateIPC` and the bind and
/// temporary-filesystem views the session units need.
pub const MIN_SYSTEMD: u32 = 255;

/// What the command needs from the machine, so the logic runs without
/// root in tests.
pub trait Machine {
    fn is_root(&self) -> bool;
    fn systemd_version(&self) -> Option<u32>;
    fn has_passt(&self) -> bool;
    fn unit_active(&self, unit: &str) -> bool;
    fn run(&self, argv: &[&str]) -> Result<()>;
    /// Live processes whose real, effective or saved uid is one of `uids`.
    fn processes_of(&self, uids: &[u32]) -> Vec<u32>;
}

pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
    pub sysusers: PathBuf,
    /// The host account's state, where the agents prefix is.
    pub host_state: PathBuf,
}

impl Paths {
    pub fn installed() -> Self {
        Self {
            config: crate::RUNTIME_CONFIG.into(),
            state: crate::STATE_DIR.into(),
            sysusers: crate::SYSUSERS_FILE.into(),
            host_state: crate::session::HOST_STATE.into(),
        }
    }

    pub fn manifests(&self) -> PathBuf {
        self.state.join("manifests")
    }

    /// The agents prefix's record (136g), apart from the workspaces'.
    pub fn agents_manifests(&self) -> PathBuf {
        self.state.join("agents-manifest")
    }
}

fn preflight(m: &dyn Machine) -> Result<()> {
    if !m.is_root() {
        bail!("run this as root: sudo storm-runtime-isolation …");
    }
    if m.unit_active(crate::HOST_UNIT) {
        bail!(
            "storm-runtime is running. Stop it first (this ends its sessions):\n  \
             sudo systemctl stop storm-runtime"
        );
    }
    Ok(())
}

pub fn on(
    m: &dyn Machine,
    accounts: &dyn Accounts,
    own: &dyn Ownership,
    paths: &Paths,
    out: &mut dyn Write,
) -> Result<()> {
    preflight(m)?;
    match m.systemd_version() {
        Some(v) if v >= MIN_SYSTEMD => {}
        Some(v) => bail!("isolation needs systemd {MIN_SYSTEMD} or newer; this host has {v}"),
        None => bail!("isolation needs systemd {MIN_SYSTEMD} or newer; systemctl did not answer"),
    }
    if !m.has_passt() {
        bail!(
            "isolation needs passt (pasta) and iproute2 for each session's network: \
             sudo apt install passt iproute2"
        );
    }
    let config = HelperConfig::load(&paths.config)?;
    if !paths.sysusers.is_file() {
        bail!(
            "{} is missing; reinstall the storm-runtime package",
            paths.sysusers.display()
        );
    }
    m.run(&["systemd-sysusers", &paths.sysusers.to_string_lossy()])?;
    let pool = Pool::resolve(accounts, crate::HOST_ACCOUNT)?;
    let workspaces = config::workspaces(&config.canonical_roots())?;
    let report = convert::on(&workspaces, &pool, &paths.manifests(), own)?;
    let agents = crate::agents::on(&paths.host_state, &pool, &paths.agents_manifests(), own)?;
    m.run(&["systemctl", "daemon-reload"])?;
    m.run(&["systemctl", "enable", "--now", crate::SOCKET_UNIT])?;
    write_isolation(&paths.config, Isolation::PerSession)?;
    writeln!(
        out,
        "Isolation is on: {} pool users, {} workspaces converted ({} entries changed).\n\
         The agents Storm installs now belong to {} ({agents} entries changed).\n\
         Every session now runs under its own user; none runs as the host's account\n\
         (decision 136a). Start the host again: sudo systemctl start storm-runtime",
        pool.users.len(),
        report.workspaces,
        report.changed,
        crate::pool::INSTALL_ACCOUNT,
    )?;
    Ok(())
}

pub fn off(
    m: &dyn Machine,
    accounts: &dyn Accounts,
    own: &dyn Ownership,
    paths: &Paths,
    out: &mut dyn Write,
) -> Result<()> {
    preflight(m)?;
    let pool = Pool::resolve(accounts, crate::HOST_ACCOUNT).context(
        "the isolation pools are not intact, so their files cannot be handed back safely",
    )?;
    let mut uids: Vec<u32> = pool.users.values().copied().collect();
    uids.extend(
        pool.install
            .into_iter()
            .chain(pool.review)
            .map(|(uid, _)| uid),
    );
    let left = m.processes_of(&uids);
    if !left.is_empty() {
        bail!(
            "{} process(es) still run as session users (pids {:?}); they must end before \
             their files are handed back",
            left.len(),
            left
        );
    }
    m.run(&[
        "systemctl",
        "disable",
        "--now",
        crate::SOCKET_UNIT,
        crate::SERVICE_UNIT,
    ])?;
    let report = convert::off(&pool, &paths.manifests(), own)?;
    let agents = crate::agents::off(&paths.host_state, &pool, &paths.agents_manifests(), own)?;
    write_isolation(&paths.config, Isolation::Off)?;
    writeln!(
        out,
        "Isolation is off: {} workspaces restored ({} entries changed); the agents \
         Storm installs are the host's again ({agents} entries changed).",
        report.workspaces, report.changed
    )?;
    for gone in &report.missing {
        writeln!(
            out,
            "  {} no longer exists; its record was dropped.",
            gone.display()
        )?;
    }
    for sealed in &report.sealed {
        writeln!(
            out,
            "  {} is nested too deep to restore; it was sealed to the host account (0700) and left as it is.",
            sealed.display()
        )?;
    }
    writeln!(
        out,
        "The pool accounts are kept, unused. Start the host again: sudo systemctl start storm-runtime"
    )?;
    Ok(())
}

pub fn status(m: &dyn Machine, paths: &Paths, out: &mut dyn Write) -> Result<()> {
    let config = HelperConfig::load(&paths.config)?;
    let converted = Manifest::load_all(&paths.manifests())
        .map(|v| v.len())
        .unwrap_or(0);
    writeln!(
        out,
        "isolation: {}\nhelper socket: {}\nconverted workspaces: {converted}",
        match config.isolation {
            Isolation::Off => "off",
            Isolation::PerSession => "per-session",
        },
        if m.unit_active(crate::SOCKET_UNIT) {
            "active"
        } else {
            "inactive"
        },
    )?;
    Ok(())
}

/// Rewrites the one line, keeping the file's mode and owner, atomically.
pub fn write_isolation(path: &Path, mode: Isolation) -> Result<()> {
    let (text, meta) = match std::fs::read_to_string(path) {
        Ok(text) => (text, Some(std::fs::metadata(path)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let new = crate::toml_edit::set_isolation(&text, mode)?;
    let tmp = path.with_extension("toml.isolation-tmp");
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        f.write_all(new.as_bytes())?;
        let perms = meta
            .as_ref()
            .map_or(0o644, |m| m.permissions().mode() & 0o7777);
        f.set_permissions(std::fs::Permissions::from_mode(perms))?;
        if let Some(m) = &meta {
            std::os::unix::fs::fchown(&f, Some(m.uid()), Some(m.gid()))?;
        }
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn parse_systemd_version(text: &str) -> Option<u32> {
    let first = text.lines().next()?;
    let rest = first.strip_prefix("systemd ")?;
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// The real machine.
pub struct Real;

impl Machine for Real {
    fn is_root(&self) -> bool {
        rustix::process::geteuid().is_root()
    }

    fn systemd_version(&self) -> Option<u32> {
        let out = Command::new("systemctl").arg("--version").output().ok()?;
        parse_systemd_version(&String::from_utf8_lossy(&out.stdout))
    }

    fn has_passt(&self) -> bool {
        Path::new(crate::net::IP).is_file()
            && Command::new(crate::net::PASTA)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    }

    fn unit_active(&self, unit: &str) -> bool {
        Command::new("systemctl")
            .args(["is-active", "--quiet", unit])
            .status()
            .is_ok_and(|s| s.success())
    }

    fn run(&self, argv: &[&str]) -> Result<()> {
        let status = Command::new(argv[0])
            .args(&argv[1..])
            .status()
            .with_context(|| format!("running {}", argv[0]))?;
        if !status.success() {
            bail!("{} failed ({status})", argv.join(" "));
        }
        Ok(())
    }

    fn processes_of(&self, uids: &[u32]) -> Vec<u32> {
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut pids = Vec::new();
        for entry in dir.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
                continue;
            };
            if status_uids(&status).iter().any(|u| uids.contains(u)) {
                pids.push(pid);
            }
        }
        pids
    }
}

/// The real, effective, saved and filesystem uids from `/proc/<pid>/status`.
pub fn status_uids(status: &str) -> Vec<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .map(|rest| {
            rest.split_whitespace()
                .filter_map(|u| u.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::Recorded;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        not_root: bool,
        systemd: Option<u32>,
        no_passt: bool,
        active: Vec<&'static str>,
        processes: Vec<u32>,
        ran: RefCell<Vec<String>>,
    }

    impl Machine for Fake {
        fn is_root(&self) -> bool {
            !self.not_root
        }
        fn systemd_version(&self) -> Option<u32> {
            self.systemd
        }
        fn has_passt(&self) -> bool {
            !self.no_passt
        }
        fn unit_active(&self, unit: &str) -> bool {
            self.active.contains(&unit)
        }
        fn run(&self, argv: &[&str]) -> Result<()> {
            self.ran.borrow_mut().push(argv.join(" "));
            Ok(())
        }
        fn processes_of(&self, _: &[u32]) -> Vec<u32> {
            self.processes.clone()
        }
    }

    fn ready() -> Fake {
        Fake {
            systemd: Some(255),
            ..Fake::default()
        }
    }

    struct Setup {
        _t: tempfile::TempDir,
        paths: Paths,
        root: PathBuf,
    }

    fn setup() -> Setup {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let root = base.join("workspaces");
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(root.join("app/src/lib.rs"), b"// code").unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        let config = base.join("runtime.toml");
        std::fs::write(
            &config,
            format!(
                "# the operator's\nworkspace_roots = [\"{}\"]\nmax_sessions = 4\n",
                root.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::fs::create_dir_all(base.join("host")).unwrap();
        let sysusers = base.join("sysusers.conf");
        std::fs::write(&sysusers, crate::pool::sysusers_conf()).unwrap();
        Setup {
            paths: Paths {
                config,
                state: base.join("state"),
                sysusers,
                host_state: base.join("host"),
            },
            root,
            _t: t,
        }
    }

    #[test]
    fn on_then_off_round_trips_the_host() {
        let s = setup();
        let m = ready();
        let accounts = crate::pool::tests::fixed();
        let own = Recorded::default();
        let config_before = std::fs::read_to_string(&s.paths.config).unwrap();
        let mut out = Vec::new();
        on(&m, &accounts, &own, &s.paths, &mut out).unwrap();
        assert_eq!(
            *m.ran.borrow(),
            [
                format!("systemd-sysusers {}", s.paths.sysusers.display()),
                "systemctl daemon-reload".into(),
                "systemctl enable --now storm-runtime-isolation.socket".into(),
            ]
        );
        let text = std::fs::read_to_string(&s.paths.config).unwrap();
        assert!(text.contains("isolation = \"per-session\""));
        assert!(
            text.starts_with("# the operator's\n"),
            "the operator's lines are kept"
        );
        let md = std::fs::metadata(&s.paths.config).unwrap();
        assert_eq!(md.permissions().mode() & 0o777, 0o640, "mode kept");
        assert_eq!(Manifest::load_all(&s.paths.manifests()).unwrap().len(), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("2 workspaces converted")
        );

        let m = ready();
        off(&m, &accounts, &own, &s.paths, &mut Vec::new()).unwrap();
        assert_eq!(
            *m.ran.borrow(),
            [
                "systemctl disable --now storm-runtime-isolation.socket storm-runtime-isolation.service"
            ]
        );
        let text = std::fs::read_to_string(&s.paths.config).unwrap();
        assert!(text.contains("isolation = \"off\""));
        assert_eq!(
            HelperConfig::parse(&text).unwrap().isolation,
            Isolation::Off
        );
        assert!(text.starts_with(&config_before[..config_before.len() - 1]));
        assert!(Manifest::load_all(&s.paths.manifests()).unwrap().is_empty());
        assert_eq!(
            std::fs::read(s.root.join("app/src/lib.rs")).unwrap(),
            b"// code"
        );
    }

    #[test]
    fn on_refuses_a_host_that_cannot_isolate_and_changes_nothing() {
        type Break = fn(&mut Fake);
        let cases: &[(&str, Break)] = &[
            ("not root", |f| f.not_root = true),
            ("host running", |f| f.active.push("storm-runtime.service")),
            ("old systemd", |f| f.systemd = Some(254)),
            ("no systemd", |f| f.systemd = None),
            ("no passt", |f| f.no_passt = true),
        ];
        for (what, break_it) in cases {
            let s = setup();
            let before = std::fs::read_to_string(&s.paths.config).unwrap();
            let mut m = ready();
            break_it(&mut m);
            let own = Recorded::default();
            assert!(
                on(
                    &m,
                    &crate::pool::tests::fixed(),
                    &own,
                    &s.paths,
                    &mut Vec::new()
                )
                .is_err(),
                "{what}"
            );
            assert!(m.ran.borrow().is_empty(), "{what}");
            assert!(own.overlay.borrow().is_empty(), "{what}");
            assert_eq!(
                std::fs::read_to_string(&s.paths.config).unwrap(),
                before,
                "{what}"
            );
        }
    }

    #[test]
    fn on_with_a_broken_pool_leaves_isolation_off() {
        let s = setup();
        let mut accounts = crate::pool::tests::fixed();
        accounts.add_user("storm-s009", 2009, 2009, "/bin/sh");
        let own = Recorded::default();
        assert!(on(&ready(), &accounts, &own, &s.paths, &mut Vec::new()).is_err());
        assert!(own.overlay.borrow().is_empty());
        let text = std::fs::read_to_string(&s.paths.config).unwrap();
        assert_eq!(
            HelperConfig::parse(&text).unwrap().isolation,
            Isolation::Off
        );
    }

    #[test]
    fn off_waits_for_every_session_user_to_be_gone() {
        let s = setup();
        let accounts = crate::pool::tests::fixed();
        let own = Recorded::default();
        on(&ready(), &accounts, &own, &s.paths, &mut Vec::new()).unwrap();
        let m = Fake {
            processes: vec![4242],
            ..ready()
        };
        let err = off(&m, &accounts, &own, &s.paths, &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("4242"), "{err}");
        assert!(m.ran.borrow().is_empty());
        let text = std::fs::read_to_string(&s.paths.config).unwrap();
        assert_eq!(
            HelperConfig::parse(&text).unwrap().isolation,
            Isolation::PerSession
        );
    }

    #[test]
    fn the_systemd_version_and_proc_uids_parse() {
        assert_eq!(
            parse_systemd_version("systemd 255 (255.4-1ubuntu8)\n+PAM"),
            Some(255)
        );
        assert_eq!(
            parse_systemd_version("systemd 261 (261.2-1-arch)"),
            Some(261)
        );
        assert_eq!(parse_systemd_version("nope"), None);
        assert_eq!(
            status_uids("Name:\tx\nUid:\t1000\t1000\t2001\t1000\nGid:\t5\n"),
            [1000, 1000, 2001, 1000]
        );
        assert!(
            Real.processes_of(&[rustix::process::getuid().as_raw()])
                .contains(&std::process::id())
        );
    }
}
