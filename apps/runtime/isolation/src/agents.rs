//! The install identity (136g, amending decision 133 on isolated hosts).
//!
//! The agents Storm installs, `<host state>/agents/managed`, belong to
//! `storm-install`: neither the host account nor any session. Sessions see
//! the prefix read-only through their unit's bind and can run what is
//! there, the host can read it, and only an install job writes it. A job
//! runs `storm-runtime agent-job <agent> <action>`, a fixed command whose
//! recipes are compiled into the host's binary (133-1), in a transient unit
//! of its own under `storm-install` that sees nothing of the host's state
//! but the prefix.
//!
//! `on` hands the prefix to the install identity, recording every entry
//! first as it does for a workspace; `off` puts back exactly what it
//! recorded and gives the host account anything installed since.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::convert::{self, Kind, Manifest, Meta, Ownership};
use crate::pool::{INSTALL_ACCOUNT, Pool};
use crate::units::{self, property_safe};

/// The agents a host can install (storm-runtime's `agents::MANAGEABLE`;
/// pinned by its packaging tests).
pub const MANAGEABLE: &[&str] = &["claude-code", "opencode", "codex", "cursor", "command-code"];

pub const ACTIONS: &[&str] = &["install", "update", "remove"];

/// The host's own binary, as its package installs it; the job is its
/// `agent-job` command.
pub const STORM_RUNTIME: &str = "/usr/bin/storm-runtime";

/// A job may take this long; the host waits a little longer.
pub const JOB_MAX_SECS: u32 = 900;

pub fn job_unit_name(agent: &str) -> String {
    format!("storm-agent-job-{agent}.service")
}

/// One install job's unit (tested as data, like a session's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpec {
    pub agent: String,
    pub action: String,
    /// The prefix: the only thing the job may write.
    pub managed: PathBuf,
    /// What it must not see: the host's state, the helper's, the
    /// workspaces.
    pub hidden: Vec<PathBuf>,
    /// Where it finds `npm` and `node` (the host's search path).
    pub path: Vec<PathBuf>,
}

impl JobSpec {
    pub fn properties(&self) -> Result<Vec<String>> {
        if !MANAGEABLE.contains(&self.agent.as_str()) || !ACTIONS.contains(&self.action.as_str()) {
            bail!("not a job Storm runs");
        }
        if let Some(bad) = std::iter::once(&self.managed)
            .chain(&self.hidden)
            .find(|p| !property_safe(p))
        {
            bail!("{} cannot be written into a unit property", bad.display());
        }
        let path: Vec<String> = self
            .path
            .iter()
            .filter(|p| property_safe(p))
            .map(|p| p.display().to_string())
            .collect();
        let p = |s: &str| s.to_string();
        let mut out = vec![
            format!(
                "Description=Storm agent job ({} {})",
                self.action, self.agent
            ),
            p("Type=exec"),
            format!("User={INSTALL_ACCOUNT}"),
            format!("Group={INSTALL_ACCOUNT}"),
            format!("WorkingDirectory={}", self.managed.display()),
            format!("Environment=HOME={}", self.managed.join(".home").display()),
            format!("Environment=PATH={}", path.join(":")),
            // What it installs is for every session to read and run.
            p("UMask=0022"),
            format!("RuntimeMaxSec={JOB_MAX_SECS}"),
            p("NoNewPrivileges=yes"),
            p("CapabilityBoundingSet="),
            p("AmbientCapabilities="),
            p("RestrictSUIDSGID=yes"),
            p("ProtectSystem=strict"),
            p("ProtectHome=yes"),
            p("PrivateTmp=yes"),
            p("PrivateDevices=yes"),
            p("PrivateIPC=yes"),
            p("ProtectProc=invisible"),
            p("ProtectKernelTunables=yes"),
            p("ProtectKernelModules=yes"),
            p("ProtectKernelLogs=yes"),
            p("ProtectControlGroups=yes"),
            p("ProtectClock=yes"),
            p("ProtectHostname=yes"),
            p("LockPersonality=yes"),
            p("RestrictRealtime=yes"),
            p("RestrictNamespaces=yes"),
            p("KeyringMode=private"),
            // Downloads need the network; nothing local beyond it.
            p("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK"),
        ];
        for hidden in &self.hidden {
            out.push(format!("TemporaryFileSystem={}:ro", hidden.display()));
        }
        out.push(format!("BindPaths={}", self.managed.display()));
        Ok(out)
    }

    pub fn systemd_run_args(&self) -> Result<Vec<OsString>> {
        let unit = job_unit_name(&self.agent);
        Ok(units::run_args(
            &unit,
            true,
            self.properties()?,
            self.exec(),
        ))
    }

    pub fn unit_file(&self) -> Result<String> {
        let exec = units::command_line(&self.exec());
        Ok(units::unit_file(self.properties()?, &exec))
    }

    fn exec(&self) -> Vec<OsString> {
        vec![
            STORM_RUNTIME.into(),
            "agent-job".into(),
            self.agent.clone().into(),
            self.action.clone().into(),
        ]
    }
}

fn version_ok(v: &str) -> bool {
    (1..=64).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-_".contains(&b))
}

fn code_ok(c: &str) -> bool {
    (1..=40).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// The job's last line, `{"done": {"version": …}}` or `{"failed": code}`,
/// checked before anything of it reaches the host: a version and a code,
/// never text.
pub fn parse_result(stdout: &str) -> Result<Map<String, Value>> {
    let line = stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .context("the job printed nothing")?;
    let v: Value = serde_json::from_str(line).context("the job's answer")?;
    let mut out = Map::new();
    if let Some(done) = v.get("done") {
        match &done["version"] {
            Value::Null => {
                out.insert("version".into(), Value::Null);
            }
            Value::String(s) if version_ok(s) => {
                out.insert("version".into(), s.clone().into());
            }
            _ => bail!("the job reported a version that is not one"),
        }
    } else if let Some(Value::String(code)) = v.get("failed")
        && code_ok(code)
    {
        out.insert("failed".into(), code.clone().into());
    } else {
        bail!("the job's answer is neither done nor failed");
    }
    Ok(out)
}

/// What an entry of the prefix becomes under the install identity: its
/// owner's, readable by everyone, writable by it alone, setuid and setgid
/// dropped.
pub fn installed(kind: Kind, m: Meta, uid: u32, gid: u32) -> Meta {
    let mode = match kind {
        Kind::Dir => (m.mode & !0o7022) | 0o755,
        Kind::File => {
            let mode = (m.mode & !0o7022) | 0o444;
            if mode & 0o100 != 0 {
                mode | 0o111
            } else {
                mode
            }
        }
        Kind::Link | Kind::Other => m.mode,
    };
    Meta { uid, gid, mode }
}

/// And back, for what was installed while isolation was on: the host's,
/// as its `UMask=0027` makes things.
pub fn handed_back(kind: Kind, m: Meta, pool: &Pool) -> Meta {
    let (uid, gid) = match pool.install {
        Some((iu, ig)) => (
            if m.uid == iu { pool.host_uid } else { m.uid },
            if m.gid == ig { pool.host_gid } else { m.gid },
        ),
        None => (m.uid, m.gid),
    };
    let mode = match kind {
        Kind::Dir | Kind::File => m.mode & !0o7027,
        Kind::Link | Kind::Other => m.mode,
    };
    Meta { uid, gid, mode }
}

/// npm's cache in the prefix (storm-runtime's `install.rs`).
pub const NPM_CACHE: &str = "npm-cache";

pub fn managed(host_state: &Path) -> PathBuf {
    host_state.join("agents").join("managed")
}

/// `on`'s half for the prefix. The record is complete before anything
/// changes, and a rerun keeps the first one.
pub fn on(host_state: &Path, pool: &Pool, manifests: &Path, own: &dyn Ownership) -> Result<usize> {
    let Some((uid, gid)) = pool.install else {
        bail!("{INSTALL_ACCOUNT} does not exist; the package's sysusers file declares it");
    };
    let prefix = managed(host_state);
    let agents = host_state.join("agents");
    for dir in [&agents, &prefix] {
        if dir.symlink_metadata().is_err() {
            std::fs::DirBuilder::new().mode(0o700).create(dir)?;
            convert::walk(dir, &mut |e| {
                if e.rel.is_empty() {
                    own.chown(&e, pool.host_uid, pool.host_gid)?;
                }
                Ok(())
            })?;
        }
    }
    let md = prefix.symlink_metadata()?;
    if !md.is_dir() {
        bail!("{} is not a directory", prefix.display());
    }
    std::fs::create_dir_all(manifests)?;
    let mut record = Manifest::load_all(manifests)?
        .into_iter()
        .find(|m| m.group == INSTALL_ACCOUNT)
        .unwrap_or(Manifest {
            workspace: prefix.clone(),
            group: INSTALL_ACCOUNT.into(),
            entries: BTreeMap::new(),
        });
    convert::walk(&prefix, &mut |e| {
        record
            .entries
            .entry(e.rel.to_vec())
            .or_insert((e.kind, own.meta(e.key, e.seen)));
        Ok(())
    })?;
    record.save(manifests)?;
    let mut changed = 0;
    convert::walk(&prefix, &mut |e| {
        let now = own.meta(e.key, e.seen);
        let target = if e.rel == NPM_CACHE.as_bytes() {
            Meta {
                uid,
                gid,
                mode: 0o700,
            }
        } else if e.rel.starts_with(format!("{NPM_CACHE}/").as_bytes()) {
            // npm's logs can name a registry URL with its token; the cache
            // stays the install identity's alone.
            Meta {
                uid,
                gid,
                mode: now.mode & !0o7022,
            }
        } else {
            installed(e.kind, now, uid, gid)
        };
        if (now.uid == pool.host_uid || now.uid == uid) && convert::apply(own, &e, target)? {
            changed += 1;
        }
        Ok(())
    })?;
    Ok(changed)
}

/// `off`'s half: what was recorded gets exactly its owner and mode back;
/// anything installed since becomes the host's.
pub fn off(host_state: &Path, pool: &Pool, manifests: &Path, own: &dyn Ownership) -> Result<usize> {
    let Some(record) = Manifest::load_all(manifests)?
        .into_iter()
        .find(|m| m.group == INSTALL_ACCOUNT)
    else {
        return Ok(0);
    };
    let prefix = managed(host_state);
    let mut changed = 0;
    if prefix.symlink_metadata().is_ok_and(|m| m.is_dir()) {
        convert::walk(&prefix, &mut |e| {
            let now = own.meta(e.key, e.seen);
            let target = match record.entries.get(e.rel) {
                Some((kind, recorded)) if *kind == e.kind => *recorded,
                _ => handed_back(e.kind, now, pool),
            };
            if convert::apply(own, &e, target)? {
                changed += 1;
            }
            Ok(())
        })?;
    }
    record.remove(manifests)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{Key, Recorded};
    use crate::session::tests::me;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    fn spec() -> JobSpec {
        JobSpec {
            agent: "claude-code".into(),
            action: "install".into(),
            managed: "/var/lib/storm-runtime/agents/managed".into(),
            hidden: vec![
                "/var/lib/storm-runtime".into(),
                "/var/lib/storm-runtime-isolation".into(),
                "/srv/workspaces".into(),
            ],
            path: vec![
                "/var/lib/storm-runtime/agents/managed/bin".into(),
                "/usr/bin".into(),
                "/weird path".into(),
            ],
        }
    }

    /// 136g as data: the job is the install identity's, holds no
    /// capability, sees none of the host's state but the prefix, and runs
    /// the host's own fixed command.
    #[test]
    fn a_job_runs_under_the_install_identity_and_writes_only_the_prefix() {
        let props = spec().properties().unwrap();
        for want in [
            "User=storm-install",
            "Group=storm-install",
            "CapabilityBoundingSet=",
            "AmbientCapabilities=",
            "NoNewPrivileges=yes",
            "RestrictSUIDSGID=yes",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "PrivateTmp=yes",
            "UMask=0022",
            "TemporaryFileSystem=/var/lib/storm-runtime:ro",
            "TemporaryFileSystem=/var/lib/storm-runtime-isolation:ro",
            "TemporaryFileSystem=/srv/workspaces:ro",
            "BindPaths=/var/lib/storm-runtime/agents/managed",
            "Environment=PATH=/var/lib/storm-runtime/agents/managed/bin:/usr/bin",
        ] {
            assert!(props.iter().any(|p| p == want), "{want} in {props:#?}");
        }
        let writable: Vec<&String> = props
            .iter()
            .filter(|p| p.starts_with("BindPaths=") || p.starts_with("ReadWritePaths="))
            .collect();
        assert_eq!(
            writable,
            ["BindPaths=/var/lib/storm-runtime/agents/managed"]
        );
        assert!(
            !props
                .iter()
                .any(|p| p.starts_with("User=storm-s") || p.starts_with("User=storm-runtime"))
        );
        let args: Vec<String> = spec()
            .systemd_run_args()
            .unwrap()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let tail = &args[args.iter().position(|a| a == "--").unwrap()..];
        assert_eq!(
            tail,
            ["--", STORM_RUNTIME, "agent-job", "claude-code", "install"]
        );
        for flag in [
            "--wait",
            "--pipe",
            "--collect",
            "--unit=storm-agent-job-claude-code.service",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        for (agent, action) in [
            ("evil", "install"),
            ("claude-code", "exec"),
            ("shell", "install"),
        ] {
            let mut s = spec();
            s.agent = agent.into();
            s.action = action.into();
            assert!(s.properties().is_err(), "{agent} {action}");
        }
    }

    #[test]
    fn the_job_unit_verifies_offline() {
        let Some(analyze) = crate::units::tests::which("systemd-analyze") else {
            return;
        };
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("storm-agent-job-claude-code.service");
        std::fs::write(&path, spec().unit_file().unwrap()).unwrap();
        let out = std::process::Command::new(analyze)
            .args(["verify", "--man=no", "--recursive-errors=no"])
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        for line in stderr.lines() {
            assert!(
                !line.contains("Unknown key")
                    && !line.contains("Failed to parse")
                    && !line.contains("Invalid")
                    && !line.contains("Unknown section"),
                "{stderr}"
            );
        }
    }

    #[test]
    fn only_a_version_or_a_code_comes_back_from_a_job() {
        let ok = parse_result("npm noise\n{\"done\":{\"version\":\"2.1.306\"}}\n").unwrap();
        assert_eq!(ok["version"], "2.1.306");
        assert_eq!(
            parse_result("{\"done\":{\"version\":null}}").unwrap()["version"],
            Value::Null
        );
        assert_eq!(
            parse_result("{\"failed\":\"checksum_mismatch\"}").unwrap()["failed"],
            "checksum_mismatch"
        );
        for bad in [
            "",
            "garbage",
            "{\"done\":{\"version\":\"1.0; rm -rf /\"}}",
            "{\"done\":{\"version\":1}}",
            "{\"failed\":\"https://registry/?token=x\"}",
            "{\"other\":1}",
        ] {
            assert!(parse_result(bad).is_err(), "{bad}");
        }
    }

    fn pool() -> Pool {
        let mut a = crate::pool::tests::fixed();
        a.add_user("storm-runtime", me(), me(), "/usr/sbin/nologin");
        Pool::resolve(&a, "storm-runtime").unwrap()
    }

    fn meta(own: &Recorded, p: &Path) -> Meta {
        let md = std::fs::symlink_metadata(p).unwrap();
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
    }

    /// 136g: after `on`, the prefix is the install identity's and nobody
    /// else may write it, while every session can read and run it; `off`
    /// puts back exactly what was there and gives the host what came since.
    #[test]
    fn on_hands_the_prefix_to_the_install_identity_and_off_restores_it() {
        let t = tempfile::tempdir().unwrap();
        let host = t.path().canonicalize().unwrap().join("host");
        let prefix = managed(&host);
        let release = prefix.join("claude-code/2.1.306");
        std::fs::create_dir_all(&release).unwrap();
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::set_permissions(&prefix, PermissionsExt::from_mode(0o700)).unwrap();
        std::fs::write(release.join("claude"), b"\x7fELF").unwrap();
        std::fs::set_permissions(release.join("claude"), PermissionsExt::from_mode(0o4750))
            .unwrap();
        std::fs::write(prefix.join("claude-code/current"), b"2.1.306").unwrap();
        std::fs::set_permissions(
            prefix.join("claude-code/current"),
            PermissionsExt::from_mode(0o600),
        )
        .unwrap();
        symlink(release.join("claude"), prefix.join("bin/claude")).unwrap();
        std::fs::create_dir_all(prefix.join("npm-cache/_logs")).unwrap();
        std::fs::write(
            prefix.join("npm-cache/_logs/debug.log"),
            b"https://r/?_authToken=x",
        )
        .unwrap();
        std::fs::set_permissions(
            prefix.join("npm-cache/_logs/debug.log"),
            PermissionsExt::from_mode(0o640),
        )
        .unwrap();
        let manifests = t.path().join("agents-manifest");
        let pool = pool();
        let (iu, ig) = pool.install.unwrap();
        let own = Recorded::default();
        let before: Vec<(PathBuf, Meta)> = [
            &prefix,
            &release,
            &release.join("claude"),
            &prefix.join("claude-code/current"),
            &prefix.join("bin/claude"),
        ]
        .iter()
        .map(|p| ((*p).clone(), meta(&own, p)))
        .collect();
        on(&host, &pool, &manifests, &own).unwrap();
        assert_eq!(
            meta(&own, &prefix),
            Meta {
                uid: iu,
                gid: ig,
                mode: 0o755
            }
        );
        assert_eq!(
            meta(&own, &release.join("claude")),
            Meta {
                uid: iu,
                gid: ig,
                mode: 0o755
            },
            "runnable by all, setuid gone"
        );
        assert_eq!(meta(&own, &prefix.join("claude-code/current")).mode, 0o644);
        assert_eq!(meta(&own, &prefix.join("bin/claude")).uid, iu);
        // Nobody but the install identity may write anything there: not the
        // host, not a session. npm's cache is its alone.
        assert_eq!(meta(&own, &prefix.join("npm-cache")).mode, 0o700);
        assert_eq!(
            meta(&own, &prefix.join("npm-cache/_logs/debug.log")).mode,
            0o640
        );
        crate::convert::walk(&prefix, &mut |e| {
            let m = own.meta(e.key, e.seen);
            assert_eq!(m.uid, iu);
            assert!(!pool.is_pool_uid(m.uid) && m.uid != pool.host_uid);
            if matches!(e.kind, Kind::Dir | Kind::File) {
                assert_eq!(m.mode & 0o022, 0);
                if !e.rel.starts_with(b"npm-cache") {
                    assert_eq!(m.mode & 0o004, 0o004);
                }
            }
            Ok(())
        })
        .unwrap();
        // A rerun keeps the first record.
        on(&host, &pool, &manifests, &own).unwrap();
        // Installed while isolation was on.
        std::fs::write(prefix.join("claude-code/new"), b"x").unwrap();
        let md = std::fs::metadata(prefix.join("claude-code/new")).unwrap();
        own.overlay.borrow_mut().insert(
            Key {
                dev: md.dev(),
                ino: md.ino(),
            },
            Meta {
                uid: iu,
                gid: ig,
                mode: 0o644,
            },
        );
        off(&host, &pool, &manifests, &own).unwrap();
        for (p, was) in before {
            assert_eq!(meta(&own, &p), was, "{}", p.display());
        }
        assert_eq!(
            meta(&own, &prefix.join("claude-code/new")),
            Meta {
                uid: pool.host_uid,
                gid: pool.host_gid,
                mode: 0o640
            }
        );
        assert!(Manifest::load_all(&manifests).unwrap().is_empty());
    }

    #[test]
    fn on_makes_the_prefix_when_there_is_none() {
        let t = tempfile::tempdir().unwrap();
        let host = t.path().canonicalize().unwrap().join("host");
        std::fs::create_dir_all(&host).unwrap();
        let pool = pool();
        let own = Recorded::default();
        on(&host, &pool, &t.path().join("m"), &own).unwrap();
        assert_eq!(meta(&own, &host.join("agents")).uid, pool.host_uid);
        assert_eq!(meta(&own, &managed(&host)).uid, pool.install.unwrap().0);
        let mut before = crate::pool::tests::fixed();
        before.users.remove(INSTALL_ACCOUNT);
        before.add_user("storm-runtime", me(), me(), "/usr/sbin/nologin");
        let old = Pool::resolve(&before, "storm-runtime").unwrap();
        assert!(on(&host, &old, &t.path().join("m2"), &own).is_err());
    }
}
