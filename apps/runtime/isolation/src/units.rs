//! A session's transient unit (136d). Every property is decided here, as
//! data, from what the helper validated: the unit runs as the leased user
//! and its workspace group, sees the system read-only, its workspace
//! read-write at its real path, its own home and Storm session directory,
//! and nothing of the host account's state or another session's
//! directories; it has its own `/tmp`, `/dev` (and with it `/dev/shm`) and
//! IPC namespace, its own network namespace (`net.rs`), and its own
//! control group, which stopping the unit ends whole. The terminal is the
//! one the host handed over.
//!
//! systemd creates the unit (`systemd-run`, from the helper's own unit);
//! nothing here needs a capability.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";
pub const SYSTEMCTL: &str = "/usr/bin/systemctl";

/// The unit of the host whose sessions these are: a session's unit stops
/// with it, as a session did when it was the host's child.
pub const HOST_UNIT: &str = crate::HOST_UNIT;

/// How long a session's processes get between the stop's SIGHUP and its
/// SIGKILL (freeze §7.3's grace).
pub const STOP_GRACE_SECS: u32 = 5;

pub fn unit_name(session: &str) -> String {
    format!("storm-session-{session}.service")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitSpec {
    pub session: String,
    pub user: String,
    /// The workspace's pool group (136h).
    pub workspace_group: String,
    pub workspace: PathBuf,
    /// A scratch child's parent workspace, shown read-only at its real
    /// path (137c).
    pub parent_view: Option<PathBuf>,
    /// The host's directory for this session (136i), shown read-only.
    pub session_dir: PathBuf,
    pub home: PathBuf,
    /// What the unit must not see: the host account's state, the helper's,
    /// and every workspace root (other workspaces).
    pub hidden: Vec<PathBuf>,
    /// Shown read-only when present: the agents Storm installed (136g).
    pub agents: Option<PathBuf>,
    pub tty: PathBuf,
    pub rows: u16,
    pub cols: u16,
    /// The environment the helper wrote, which systemd reads as root.
    pub env_file: PathBuf,
    /// The session's resolver (`net::resolv_conf`), shown as
    /// `/etc/resolv.conf`.
    pub resolv_conf: PathBuf,
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// A path that goes into a property as written: absolute, and made only of
/// characters no property list splits on or unescapes.
pub fn property_safe(path: &Path) -> bool {
    path.is_absolute()
        && path
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b"/._-+@".contains(&b))
}

/// `systemd-run`'s arguments for a unit: its properties as they are, and
/// the command never expanded. A `wait`ed unit is one the helper runs to
/// the end, its output piped back.
pub fn run_args(
    unit: &str,
    wait: bool,
    properties: Vec<String>,
    exec: Vec<OsString>,
) -> Vec<OsString> {
    let mut out: Vec<OsString> = vec![
        format!("--unit={unit}").into(),
        "--quiet".into(),
        "--no-ask-password".into(),
        "--expand-environment=no".into(),
    ];
    if wait {
        out.extend(["--wait".into(), "--pipe".into(), "--collect".into()]);
    }
    for property in properties {
        out.push("-p".into());
        out.push(property.into());
    }
    out.push("--".into());
    out.extend(exec);
    out
}

/// The same unit as a file, for `systemd-analyze verify` and `security` to
/// check the property set offline. `exec_start` is `ExecStart=`'s value.
pub fn unit_file(properties: Vec<String>, exec_start: &str) -> String {
    let mut unit = String::from("[Unit]\n");
    let mut service = String::from("[Service]\n");
    for property in properties {
        let key = property.split('=').next().unwrap_or("");
        let section = if matches!(key, "Description" | "BindsTo" | "After" | "Requires") {
            &mut unit
        } else {
            &mut service
        };
        section.push_str(&property);
        section.push('\n');
    }
    service.push_str(&format!("ExecStart={exec_start}\n"));
    format!("{unit}\n{service}")
}

/// A command as `ExecStart=` reads it.
pub fn command_line(exec: &[OsString]) -> String {
    let words: Vec<_> = exec.iter().map(|a| a.to_string_lossy()).collect();
    words.join(" ")
}

impl UnitSpec {
    fn paths(&self) -> impl Iterator<Item = &Path> {
        [
            &self.workspace,
            &self.session_dir,
            &self.home,
            &self.tty,
            &self.env_file,
            &self.resolv_conf,
        ]
        .into_iter()
        .map(PathBuf::as_path)
        .chain(self.hidden.iter().map(PathBuf::as_path))
        .chain(self.agents.as_deref())
        .chain(self.parent_view.as_deref())
    }

    /// The unit's properties, in order. Read by the tests as the spec.
    pub fn properties(&self) -> Result<Vec<String>> {
        if let Some(bad) = self.paths().find(|p| !property_safe(p)) {
            bail!("{} cannot be written into a unit property", bad.display());
        }
        let p = |s: &str| s.to_string();
        let net = crate::net::unit_name(&self.session);
        let mut out = vec![
            format!("Description=Storm session {}", self.session),
            p("Type=exec"),
            // The status stays readable, and what the agent left keeps
            // running, until the host ends the session (freeze §7.3).
            p("RemainAfterExit=yes"),
            format!("BindsTo={HOST_UNIT}"),
            format!("After={HOST_UNIT}"),
            // Its network is up before it starts, and a session whose
            // network is stopped is stopped; one whose pasta dies keeps
            // running with no way out.
            format!("Requires={net}"),
            format!("After={net}"),
            format!(
                "NetworkNamespacePath={}",
                crate::net::netns_path(&self.session).display()
            ),
            // Every family the namespace does not scope (AF_VSOCK, …) is
            // refused, so nothing local is reached around it.
            p("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK"),
            format!("User={}", self.user),
            format!("Group={}", self.user),
            format!("SupplementaryGroups={}", self.workspace_group),
            format!("WorkingDirectory={}", self.workspace.display()),
            // The terminal: systemd opens it by path and makes it the
            // session's controlling terminal.
            format!("TTYPath={}", self.tty.display()),
            p("StandardInput=tty"),
            p("StandardOutput=tty"),
            p("StandardError=tty"),
            format!("TTYRows={}", self.rows),
            format!("TTYColumns={}", self.cols),
            p("TTYReset=no"),
            p("TTYVHangup=no"),
            format!("Environment=HOME={}", self.home.display()),
            format!("EnvironmentFile={}", self.env_file.display()),
            // Ending: the whole control group, SIGHUP then SIGKILL.
            p("KillMode=control-group"),
            p("KillSignal=SIGHUP"),
            format!("TimeoutStopSec={STOP_GRACE_SECS}"),
            p("SendSIGKILL=yes"),
            // Private /tmp and /var/tmp, /dev (with /dev/shm), IPC.
            p("PrivateTmp=yes"),
            p("PrivateDevices=yes"),
            p("PrivateIPC=yes"),
            p("RemoveIPC=yes"),
            p("ProtectSystem=strict"),
            p("ProtectHome=yes"),
            // No other user's processes in /proc: the other sessions'.
            p("ProtectProc=invisible"),
            p("NoNewPrivileges=yes"),
            p("CapabilityBoundingSet="),
            p("AmbientCapabilities="),
            p("RestrictSUIDSGID=yes"),
            p("LockPersonality=yes"),
            p("RestrictRealtime=yes"),
            p("ProtectKernelTunables=yes"),
            p("ProtectKernelModules=yes"),
            p("ProtectKernelLogs=yes"),
            p("ProtectControlGroups=yes"),
            p("ProtectClock=yes"),
            p("ProtectHostname=yes"),
            p("KeyringMode=private"),
            // As the host's own unit: agents that sandbox themselves with
            // namespaces need a decision first (Process Isolation).
            p("RestrictNamespaces=yes"),
            p("UMask=0007"),
        ];
        for hidden in &self.hidden {
            out.push(format!("TemporaryFileSystem={}:ro", hidden.display()));
        }
        if self.workspace != self.home {
            out.push(format!("BindPaths={}", self.workspace.display()));
        }
        if let Some(parent) = &self.parent_view {
            out.push(format!("BindReadOnlyPaths={}", parent.display()));
        }
        out.push(format!("BindPaths={}", self.home.display()));
        out.push(format!("BindReadOnlyPaths={}", self.session_dir.display()));
        out.push(format!(
            "BindReadOnlyPaths={}:/etc/resolv.conf",
            self.resolv_conf.display()
        ));
        if let Some(agents) = &self.agents {
            out.push(format!("BindReadOnlyPaths=-{}", agents.display()));
        }
        Ok(out)
    }

    /// `systemd-run`'s arguments. The program comes from the operator's
    /// configuration; its arguments are passed as they are, never expanded.
    pub fn systemd_run_args(&self) -> Result<Vec<OsString>> {
        let unit = unit_name(&self.session);
        Ok(run_args(&unit, false, self.properties()?, self.exec()))
    }

    pub fn unit_file(&self) -> Result<String> {
        // `:` keeps systemd from expanding `$` and `%` in the arguments.
        let exec = format!(":{}", command_line(&self.exec()));
        Ok(unit_file(self.properties()?, &exec))
    }

    fn exec(&self) -> Vec<OsString> {
        let mut out = vec![self.program.clone().into_os_string()];
        out.extend(self.args.iter().cloned());
        out
    }
}

/// How a unit's main process ended, as systemd recorded it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitState {
    pub loaded: bool,
    /// `active`, `failed`, `inactive`, …
    pub running: bool,
    pub main_pid: u32,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

pub fn parse_show(text: &str) -> UnitState {
    let mut state = UnitState::default();
    let (mut code, mut status) = (0, None);
    let mut sub = "";
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "LoadState" => state.loaded = value == "loaded",
            "SubState" => sub = value,
            "MainPID" => state.main_pid = value.parse().unwrap_or(0),
            "ExecMainCode" => code = value.parse().unwrap_or(0),
            "ExecMainStatus" => status = value.parse().ok(),
            _ => {}
        }
    }
    state.running = sub == "running";
    // CLD_EXITED, CLD_KILLED, CLD_DUMPED.
    match code {
        1 => state.exit_code = status,
        2 | 3 => state.signal = status,
        _ => {}
    }
    state
}

pub const SHOW_PROPERTIES: &str = "LoadState,SubState,MainPID,ExecMainCode,ExecMainStatus";

/// systemd, as the helper uses it.
pub trait Units: Send + Sync {
    /// Whether pasta and `ip` are installed and pasta runs (136j).
    fn network_ready(&self) -> bool;
    /// Starts the session's network and returns once it is configured.
    fn start_network(&self, spec: &crate::net::NetSpec) -> Result<()>;
    fn start(&self, spec: &UnitSpec) -> Result<()>;
    fn show(&self, unit: &str) -> Result<UnitState>;
    /// Sends `signal` (a name, `SIGHUP`) to every process of the unit.
    fn kill(&self, unit: &str, signal: &str) -> Result<()>;
    /// Stops the unit: SIGHUP to all of it, SIGKILL after the grace. Returns
    /// once nothing of it is left.
    fn stop(&self, unit: &str) -> Result<()>;
    /// Forgets a failed unit's record, so the name can be used again.
    fn reset_failed(&self, unit: &str);
    /// Runs an install job to its end (136g) and returns what it printed.
    fn run_job(&self, spec: &crate::agents::JobSpec) -> Result<String>;
    /// Runs a child's clone to its end (137b) and returns what it printed.
    fn run_clone(&self, spec: &crate::children::CloneSpec) -> Result<String>;
    /// Runs an export or an Apply to its end (137e) and returns what it
    /// printed.
    fn run_review(&self, spec: &crate::children::ReviewSpec) -> Result<String>;
}

pub struct Systemd;

fn run(program: &str, args: &[OsString]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("SYSTEMD_PAGER", "")
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

impl Units for Systemd {
    fn network_ready(&self) -> bool {
        Path::new(crate::net::IP).is_file() && run(crate::net::PASTA, &["--version".into()]).is_ok()
    }

    fn start_network(&self, spec: &crate::net::NetSpec) -> Result<()> {
        for tool in [crate::net::PASTA, crate::net::IP] {
            if !Path::new(tool).is_file() {
                bail!("{tool} is not installed; a session needs it for its network");
            }
        }
        run(SYSTEMD_RUN, &spec.systemd_run_args()).map(drop)
    }

    fn start(&self, spec: &UnitSpec) -> Result<()> {
        run(SYSTEMD_RUN, &spec.systemd_run_args()?).map(drop)
    }

    fn show(&self, unit: &str) -> Result<UnitState> {
        let text = run(
            SYSTEMCTL,
            &[
                "show".into(),
                unit.into(),
                format!("--property={SHOW_PROPERTIES}").into(),
            ],
        )?;
        Ok(parse_show(&text))
    }

    fn kill(&self, unit: &str, signal: &str) -> Result<()> {
        run(
            SYSTEMCTL,
            &[
                "kill".into(),
                "--kill-whom=all".into(),
                format!("--signal={signal}").into(),
                unit.into(),
            ],
        )
        .map(drop)
    }

    fn stop(&self, unit: &str) -> Result<()> {
        run(SYSTEMCTL, &["stop".into(), unit.into()]).map(drop)
    }

    fn reset_failed(&self, unit: &str) {
        let _ = run(SYSTEMCTL, &["reset-failed".into(), unit.into()]);
    }

    fn run_job(&self, spec: &crate::agents::JobSpec) -> Result<String> {
        if !Path::new(crate::agents::STORM_RUNTIME).is_file() {
            bail!("{} is not installed", crate::agents::STORM_RUNTIME);
        }
        run(SYSTEMD_RUN, &spec.systemd_run_args()?)
    }

    fn run_clone(&self, spec: &crate::children::CloneSpec) -> Result<String> {
        for program in [crate::children::HELPER_BIN, crate::clone::GIT] {
            if !Path::new(program).is_file() {
                bail!("{program} is not installed");
            }
        }
        run(SYSTEMD_RUN, &spec.systemd_run_args()?)
    }

    fn run_review(&self, spec: &crate::children::ReviewSpec) -> Result<String> {
        for program in [crate::children::HELPER_BIN, crate::clone::GIT] {
            if !Path::new(program).is_file() {
                bail!("{program} is not installed");
            }
        }
        run(SYSTEMD_RUN, &spec.systemd_run_args()?)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn spec() -> UnitSpec {
        UnitSpec {
            session: "ags_A1".into(),
            user: "storm-s007".into(),
            workspace_group: "storm-w003".into(),
            workspace: "/var/lib/storm-runtime/workspaces/app".into(),
            parent_view: None,
            session_dir: "/var/lib/storm-runtime/sessions/ags_A1".into(),
            home: "/var/lib/storm-runtime-isolation/homes/ags_A1".into(),
            hidden: vec![
                "/var/lib/storm-runtime".into(),
                "/var/lib/storm-runtime-isolation".into(),
                "/var/lib/storm-runtime/workspaces".into(),
            ],
            agents: Some("/var/lib/storm-runtime/agents/managed".into()),
            tty: "/dev/pts/7".into(),
            rows: 40,
            cols: 120,
            env_file: "/var/lib/storm-runtime-isolation/sessions/ags_A1/env".into(),
            resolv_conf: "/var/lib/storm-runtime-isolation/sessions/ags_A1/resolv.conf".into(),
            program: "/usr/bin/bash".into(),
            args: vec!["-l".into(), "$HOME %h".into()],
        }
    }

    fn has(props: &[String], want: &str) -> bool {
        props.iter().any(|p| p == want)
    }

    /// 136d as data: who the session runs as, what it sees, and how it ends.
    #[test]
    fn the_unit_is_the_leased_user_in_its_own_private_namespaces() {
        let props = spec().properties().unwrap();
        for want in [
            "User=storm-s007",
            "Group=storm-s007",
            "SupplementaryGroups=storm-w003",
            "WorkingDirectory=/var/lib/storm-runtime/workspaces/app",
            "PrivateTmp=yes",
            "PrivateDevices=yes",
            "PrivateIPC=yes",
            "RemoveIPC=yes",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "ProtectProc=invisible",
            "NoNewPrivileges=yes",
            "CapabilityBoundingSet=",
            "AmbientCapabilities=",
            "RestrictSUIDSGID=yes",
            "RestrictNamespaces=yes",
            "KillMode=control-group",
            "KillSignal=SIGHUP",
            "TimeoutStopSec=5",
            "SendSIGKILL=yes",
            "RemainAfterExit=yes",
            "BindsTo=storm-runtime.service",
            "TTYPath=/dev/pts/7",
            "StandardInput=tty",
            "StandardOutput=tty",
            "StandardError=tty",
            "TTYRows=40",
            "TTYColumns=120",
            "Environment=HOME=/var/lib/storm-runtime-isolation/homes/ags_A1",
            "EnvironmentFile=/var/lib/storm-runtime-isolation/sessions/ags_A1/env",
            "NetworkNamespacePath=/run/netns/storm-ags_A1",
            "Requires=storm-session-ags_A1-net.service",
            "After=storm-session-ags_A1-net.service",
            "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK",
        ] {
            assert!(has(&props, want), "{want} missing from {props:#?}");
        }
        // The OpenCode rule (gate Q6) holds for sessions too.
        assert!(
            !props
                .iter()
                .any(|p| p.starts_with("MemoryDenyWriteExecute"))
        );
        // Nothing grants a capability or a way back to the host's account.
        assert!(!props.iter().any(|p| p.contains("storm-runtime\n")
            || p == "User=storm-runtime"
            || p == "User=root"));
    }

    /// The host's state, the helper's and other workspaces are hidden, then
    /// only this session's own directories are shown again.
    #[test]
    fn only_the_sessions_own_directories_are_visible() {
        let props = spec().properties().unwrap();
        let mounts: Vec<&str> = props
            .iter()
            .map(String::as_str)
            .filter(|p| {
                p.starts_with("TemporaryFileSystem=")
                    || p.starts_with("BindPaths=")
                    || p.starts_with("BindReadOnlyPaths=")
            })
            .collect();
        assert_eq!(
            mounts,
            [
                "TemporaryFileSystem=/var/lib/storm-runtime:ro",
                "TemporaryFileSystem=/var/lib/storm-runtime-isolation:ro",
                "TemporaryFileSystem=/var/lib/storm-runtime/workspaces:ro",
                "BindPaths=/var/lib/storm-runtime/workspaces/app",
                "BindPaths=/var/lib/storm-runtime-isolation/homes/ags_A1",
                "BindReadOnlyPaths=/var/lib/storm-runtime/sessions/ags_A1",
                "BindReadOnlyPaths=/var/lib/storm-runtime-isolation/sessions/ags_A1/resolv.conf:/etc/resolv.conf",
                "BindReadOnlyPaths=-/var/lib/storm-runtime/agents/managed",
            ]
        );
    }

    #[test]
    fn a_path_that_would_split_a_property_is_refused() {
        for bad in [
            "/w/my app",
            "/w/a:b",
            "/w/a\\x20",
            "/w/a\"b",
            "rel/a",
            "/w/a\nb",
        ] {
            let mut s = spec();
            s.workspace = bad.into();
            assert!(s.properties().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn systemd_run_gets_the_program_and_its_arguments_unexpanded() {
        let args = spec().systemd_run_args().unwrap();
        let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args[0], "--unit=storm-session-ags_A1.service");
        assert!(args.contains(&"--expand-environment=no"));
        let dash = args.iter().position(|a| *a == "--").unwrap();
        assert_eq!(&args[dash + 1..], ["/usr/bin/bash", "-l", "$HOME %h"]);
        // Every property is its own `-p`.
        let props = spec().properties().unwrap();
        assert_eq!(args.iter().filter(|a| **a == "-p").count(), props.len());
    }

    #[test]
    fn systemctl_show_parses_exits_and_signals() {
        let exited = parse_show(
            "LoadState=loaded\nSubState=exited\nMainPID=0\nExecMainCode=1\nExecMainStatus=3\n",
        );
        assert_eq!(
            exited,
            UnitState {
                loaded: true,
                running: false,
                main_pid: 0,
                exit_code: Some(3),
                signal: None,
            }
        );
        let killed =
            parse_show("LoadState=loaded\nSubState=failed\nExecMainCode=2\nExecMainStatus=9\n");
        assert_eq!((killed.exit_code, killed.signal), (None, Some(9)));
        let running = parse_show(
            "LoadState=loaded\nSubState=running\nMainPID=4242\nExecMainCode=0\nExecMainStatus=0\n",
        );
        assert!(running.running && running.main_pid == 4242);
        assert_eq!((running.exit_code, running.signal), (None, None));
        assert!(!parse_show("LoadState=not-found\n").loaded);
    }

    /// The same properties as a unit file pass `systemd-analyze verify`
    /// where systemd is installed. It checks every directive's name and
    /// value without a running manager.
    #[test]
    fn the_unit_verifies_offline() {
        let Some(analyze) = which("systemd-analyze") else {
            return;
        };
        let t = tempfile::tempdir().unwrap();
        let mut s = spec();
        s.program = "/bin/sh".into();
        let path = t.path().join("storm-session-ags_A1.service");
        std::fs::write(&path, s.unit_file().unwrap()).unwrap();
        let out = std::process::Command::new(analyze)
            .args(["verify", "--man=no", "--recursive-errors=no"])
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        // A missing path or user on this machine is not the property set's
        // fault; an unknown or malformed directive is.
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

    pub fn which(name: &str) -> Option<PathBuf> {
        std::env::var_os("PATH")
            .iter()
            .flat_map(std::env::split_paths)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    }
}
