//! Linux: systemd, the `.deb` and its paths (decision 77e).

use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;

pub const NAME: &str = "linux";
pub const DEFAULT_STATE: &str = super::paths::LINUX_STATE;
pub const DEFAULT_CONFIG: &str = super::paths::LINUX_CONFIG;
pub const SERVICE_ACCOUNT: &str = super::paths::LINUX_ACCOUNT;
pub const SERVICE_COMMAND: &str = "sudo -u storm-runtime storm-runtime";
/// The unit's `ProtectHome` already hides `/home`, `/root` and
/// `/run/user` (77e).
pub const PROTECTED_ROOTS: &[&str] = &[];

/// systemd gives the unit a fixed `PATH`; the host keeps it (77a).
pub fn default_path() -> Option<std::ffi::OsString> {
    None
}

/// `$XDG_STATE_HOME/storm`, else `~/.local/state/storm`.
pub fn user_state_dir() -> Option<std::path::PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| var("HOME").map(|h| std::path::Path::new(&h).join(".local/state")))
        .map(|d| d.join("storm"))
}

const USE_THE_PACKAGE: &str = "on Linux the storm-runtime package installs and removes the \
service: `sudo apt install storm-runtime` / `sudo apt remove storm-runtime` \
(deploy/README.md, Runtime Hosts). To run a host as yourself instead, with no \
root: `storm-runtime install --user`";

pub fn install(options: &super::InstallOptions) -> anyhow::Result<()> {
    if !options.user {
        anyhow::bail!("{USE_THE_PACKAGE}")
    }
    user::install(!options.no_linger)
}

pub fn uninstall(options: &super::UninstallOptions) -> anyhow::Result<()> {
    if !options.user {
        anyhow::bail!("{USE_THE_PACKAGE}")
    }
    user::uninstall(options.purge)
}

/// `install --user` and `uninstall --user`, as `systemd_user` decides them.
mod user {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use anyhow::{Context, Result, bail};

    use super::super::systemd_user::{self as plan, Layout, Linger};

    fn refuse_the_service() -> Result<String> {
        if super::super::is_service_context() {
            bail!(
                "--user installs a host that runs as you: run it as yourself, not as root \
                 or {}",
                super::SERVICE_ACCOUNT
            );
        }
        super::super::current_account().context("this account has no name")
    }

    fn layout() -> Result<Layout> {
        plan::layout(|name| std::env::var_os(name)).map_err(anyhow::Error::msg)
    }

    /// `systemctl --user`, which finds the user manager through
    /// `XDG_RUNTIME_DIR`; `su` and `docker exec` leave it unset.
    fn systemctl(args: &[&str]) -> Command {
        let mut c = Command::new("systemctl");
        c.arg("--user").args(args);
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            let run = PathBuf::from(format!("/run/user/{}", rustix::process::geteuid().as_raw()));
            if run.is_dir() {
                c.env("XDG_RUNTIME_DIR", run);
            }
        }
        c
    }

    fn quietly(mut c: Command) -> bool {
        c.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn checked(mut c: Command, what: &str) -> Result<()> {
        let out = c.output().with_context(|| format!("running {what}"))?;
        if !out.status.success() {
            bail!(
                "{what} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    fn linger(user: &str) -> Linger {
        Command::new("loginctl")
            .args(["show-user", user, "--property=Linger", "--value"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map_or(Linger::Unknown, |o| {
                Linger::parse(&String::from_utf8_lossy(&o.stdout))
            })
    }

    fn set_linger(enable: bool) -> bool {
        let verb = if enable {
            "enable-linger"
        } else {
            "disable-linger"
        };
        let mut c = Command::new("loginctl");
        c.args(["--no-ask-password", verb]);
        quietly(c)
    }

    /// Beside `to`, then renamed over it: a running binary cannot be
    /// rewritten in place (ETXTBSY), but it can be replaced.
    fn copy_binary(from: &Path, to: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = to.parent().context("the binary's directory")?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = dir.join(".storm-runtime.new");
        std::fs::copy(from, &tmp).with_context(|| format!("copying to {}", tmp.display()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, to).with_context(|| format!("replacing {}", to.display()))
    }

    fn write(path: &Path, text: &str) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        crate::fsutil::write_atomic(path, text.as_bytes(), crate::fsutil::Access::Default)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn install(want_linger: bool) -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        let user = refuse_the_service()?;
        let layout = layout()?;
        let exe = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .context("finding this binary")?;
        let root_owned = std::fs::metadata(&exe)?.uid() == 0;
        let existing = std::fs::read_to_string(&layout.unit).ok();
        let plan = plan::plan_install(
            &layout,
            &exe,
            root_owned,
            existing.as_deref(),
            layout.config.exists(),
            linger(&user),
            want_linger,
        );
        if plan.copy {
            copy_binary(&exe, &plan.binary)?;
        }
        if plan.write_config {
            write(&layout.config, &plan::default_config(&layout))?;
            crate::platform::create_private_dir(&layout.state)
                .with_context(|| format!("creating {}", layout.state.display()))?;
            std::fs::create_dir_all(&layout.workspaces)
                .with_context(|| format!("creating {}", layout.workspaces.display()))?;
        }
        let mut linger_ours = plan.linger_ours;
        let mut linger_note = None;
        if plan.enable_linger {
            if set_linger(true) {
                linger_ours = true;
            } else {
                linger_note = Some(plan::linger_command(true, &user));
            }
        }
        let unit =
            plan::render_unit(&plan.binary, &layout, linger_ours).map_err(anyhow::Error::msg)?;
        write(&layout.unit, &unit)?;
        if let Err(e) = checked(
            systemctl(&["daemon-reload"]),
            "systemctl --user daemon-reload",
        ) {
            bail!(
                "{e:#}\nWrote {}, but your user manager is not running. Log in to this \
                 machine (a desktop or an ssh session starts it), or run `{}`, then \
                 run this again.",
                layout.unit.display(),
                plan::linger_command(true, &user)
            );
        }
        checked(
            systemctl(&["enable", plan::UNIT_NAME]),
            "systemctl --user enable",
        )?;
        let enrolled = layout.state.join("host.json").exists();
        let active = quietly(systemctl(&["is-active", "--quiet", plan::UNIT_NAME]));
        if active {
            checked(
                systemctl(&["restart", plan::UNIT_NAME]),
                "systemctl --user restart",
            )?;
        } else if enrolled {
            checked(
                systemctl(&["start", plan::UNIT_NAME]),
                "systemctl --user start",
            )?;
        }
        println!("Installed the Runtime Host as {user} (systemd --user).");
        println!("  unit   : {}", layout.unit.display());
        println!("  binary : {}", plan.binary.display());
        println!("  state  : {}", layout.state.display());
        println!("  config : {}", layout.config.display());
        match (linger_note, want_linger) {
            (Some(command), _) => println!(
                "It runs while you are logged in. To start it at boot without a login, \
                 an administrator runs once:\n  {command}"
            ),
            (None, true) => println!("Linger is on: it starts at boot, without a login."),
            (None, false) => {}
        }
        if active || enrolled {
            println!("Running.");
        } else {
            println!(
                "Next, in the Storm app: Settings › Hosts & default agent › Enroll a host, then:\n  \
                 {} enroll\n  systemctl --user start storm-runtime",
                plan.binary.display()
            );
        }
        Ok(())
    }

    pub fn uninstall(purge: bool) -> Result<()> {
        let user = refuse_the_service()?;
        let layout = layout()?;
        let existing = std::fs::read_to_string(&layout.unit).ok();
        let plan = plan::plan_uninstall(&layout, existing.as_deref());
        if existing.is_some() {
            let _ = quietly(systemctl(&["disable", "--now", plan::UNIT_NAME]));
        }
        for path in [&layout.wants, &layout.unit] {
            match std::fs::remove_file(path) {
                Ok(()) => println!("Removed {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
            }
        }
        if existing.is_some() {
            let _ = quietly(systemctl(&["daemon-reload"]));
        } else {
            println!(
                "No {}: no host was installed as you.",
                layout.unit.display()
            );
        }
        if plan.remove_binary && std::fs::remove_file(&layout.bin).is_ok() {
            println!("Removed {}", layout.bin.display());
        }
        if plan.disable_linger {
            if set_linger(false) {
                println!("Turned linger off, as install had turned it on.");
            } else {
                println!(
                    "install turned linger on. To turn it off, an administrator runs:\n  {}",
                    plan::linger_command(false, &user)
                );
            }
        }
        if purge {
            purge_state(&layout)?;
        } else {
            println!(
                "Kept the host's state and config: {}, {}",
                layout.state.display(),
                layout.config.display()
            );
        }
        Ok(())
    }

    fn purge_state(layout: &Layout) -> Result<()> {
        let roots = std::fs::read_to_string(&layout.config)
            .ok()
            .and_then(|t| toml::from_str::<crate::config::RuntimeConfig>(&t).ok())
            .map(|c| c.workspace_roots)
            .unwrap_or_default();
        let entries: Vec<PathBuf> = match std::fs::read_dir(&layout.state) {
            Ok(dir) => dir.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", layout.state.display())),
        };
        for path in plan::purge_targets(layout, &entries, &roots) {
            let gone = if path.is_dir() && !path.is_symlink() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            gone.with_context(|| format!("removing {}", path.display()))?;
        }
        let _ = std::fs::remove_dir(&layout.state);
        match std::fs::remove_file(&layout.config) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("removing {}", layout.config.display()));
            }
        }
        if let Some(dir) = layout.config.parent() {
            let _ = std::fs::remove_dir(dir);
        }
        println!(
            "Removed the host's state and config; workspaces stay ({}).",
            layout.workspaces.display()
        );
        Ok(())
    }
}

/// `poll(2)` supports a PTY master on Linux, and is what 77c verified.
pub fn wait_writable(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<bool> {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut fds = [PollFd::new(&fd, PollFlags::OUT)];
    Ok(poll(&mut fds, Some(&super::posix::timespec(timeout)))? > 0)
}

pub fn wait_readable(fd: BorrowedFd<'_>) -> io::Result<()> {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    poll(&mut fds, None)?;
    Ok(())
}

/// `/proc/<pid>/stat`: field 6 is the session. The command name (field 2)
/// may contain spaces and parentheses, so fields are counted after its last
/// `)`. Zombies (`Z`, `X`) are already gone for our purposes.
pub fn session_members(sid: i32) -> io::Result<Vec<i32>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        // A process may exit between the listing and the read.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        if stat_session(&stat) == Some(sid) {
            out.push(pid);
        }
    }
    Ok(out)
}

/// Every live process whose real or effective uid is `uid`: the ones a
/// process of that account may signal. `/proc/<pid>/status` has
/// `Uid: real effective saved fs`.
pub fn account_processes(uid: u32) -> io::Result<Vec<i32>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        if status_owned_by(&status, uid) {
            out.push(pid);
        }
    }
    Ok(out)
}

/// The environment `pid` was started with, NUL-separated: readable for the
/// processes of one's own account.
pub fn environment(pid: i32) -> Option<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/environ")).ok()
}

fn status_owned_by(status: &str, uid: u32) -> bool {
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .map(str::trim)
    };
    if field("State:").is_some_and(|s| s.starts_with('Z') || s.starts_with('X')) {
        return false;
    }
    field("Uid:").is_some_and(|ids| {
        ids.split_whitespace()
            .take(2)
            .any(|id| id.parse() == Ok(uid))
    })
}

/// The session of a live process, from its `stat` line; `None` for a zombie.
fn stat_session(stat: &str) -> Option<i32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?;
    if state == "Z" || state == "X" {
        return None;
    }
    // ppid, pgrp, then session.
    fields.nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn linux_keeps_the_units_path() {
        assert_eq!(super::default_path(), None);
    }

    #[test]
    fn install_and_uninstall_point_at_the_package() {
        let e = super::install(&Default::default()).unwrap_err().to_string();
        assert!(e.contains("apt install storm-runtime"), "{e}");
        assert!(e.contains("storm-runtime install --user"), "{e}");
        let e = super::uninstall(&Default::default())
            .unwrap_err()
            .to_string();
        assert!(e.contains("apt remove storm-runtime"), "{e}");
    }

    #[test]
    fn an_owner_is_the_real_or_effective_uid_of_a_live_process() {
        let s = |state: &str, ids: &str| format!("Name:\tx\nState:\t{state}\nUid:\t{ids}\n");
        assert!(super::status_owned_by(
            &s("S (sleeping)", "999\t999\t999\t999"),
            999
        ));
        assert!(super::status_owned_by(
            &s("R (running)", "0\t999\t0\t0"),
            999
        ));
        assert!(!super::status_owned_by(
            &s("S (sleeping)", "0\t0\t999\t0"),
            999
        ));
        assert!(!super::status_owned_by(
            &s("Z (zombie)", "999\t999\t999\t999"),
            999
        ));
    }

    #[test]
    fn the_session_is_read_after_the_last_paren() {
        let line = "4242 (a (b) c) S 1 4242 4200 34816 4242 4194560 0";
        assert_eq!(super::stat_session(line), Some(4200));
        assert_eq!(super::stat_session("7 (x) Z 1 7 7 0"), None);
        assert_eq!(super::stat_session("garbage"), None);
    }
}
