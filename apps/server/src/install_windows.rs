//! `up` / `down` / `status` / `uninstall` on Windows (PLAN 99, W-S1–S3): the
//! server runs as the person, at logon, from a Scheduled Task, with the notes
//! in `%USERPROFILE%\Storm`. The counterpart of 84d's macOS LaunchAgent; no
//! step needs an administrator.
//!
//! The layout, the record and the task definitions are pure functions over
//! strings, compiled everywhere so the Linux suite tests them. Running them
//! (`schtasks`, `icacls`) is Windows only.

// Compiled everywhere for its tests; only Windows runs it.
#![cfg_attr(not(windows), allow(dead_code))]

use serde::{Deserialize, Serialize};

pub const TASK: &str = "Storm Server";
pub const BACKUP_TASK: &str = "Storm Backup";
const DEFAULT_PORT: u16 = 8484;
const DEFAULT_HOST: &str = "0.0.0.0";

/// Where `up` puts what it installs. Paths are strings: a `Path` on Linux
/// would not split a Windows one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub support: String,
    pub bin: String,
    /// The windowless twin the task runs (W-S1).
    pub binw: String,
    pub web: String,
    pub record: String,
    pub logs: String,
    pub log_file: String,
    pub backup_log: String,
    pub default_data_root: String,
}

impl Layout {
    /// For a person whose `%LOCALAPPDATA%` and `%USERPROFILE%` are these.
    pub fn for_profile(local_app_data: &str, user_profile: &str) -> Self {
        let support = format!(r"{}\Storm", local_app_data.trim_end_matches('\\'));
        let logs = format!(r"{support}\logs");
        Self {
            bin: format!(r"{support}\bin\storm-server.exe"),
            binw: format!(r"{support}\bin\storm-serverw.exe"),
            web: format!(r"{support}\web"),
            record: format!(r"{support}\server.json"),
            log_file: format!(r"{logs}\storm-server.log"),
            backup_log: format!(r"{logs}\storm-backup.log"),
            default_data_root: format!(r"{}\Storm", user_profile.trim_end_matches('\\')),
            support,
            logs,
        }
    }
}

/// What `up` installed, read back by `status` (84d's `server.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub data_root: String,
    pub vaults: String,
    pub state: String,
    pub backups: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Default)]
pub struct UpOptions {
    pub data_root: Option<String>,
    pub vault_root: Option<String>,
    pub state: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub web: Option<String>,
}

pub fn record_for(layout: &Layout, opts: &UpOptions) -> Record {
    let data_root = opts
        .data_root
        .clone()
        .unwrap_or_else(|| layout.default_data_root.clone());
    Record {
        vaults: opts
            .vault_root
            .clone()
            .unwrap_or_else(|| format!(r"{data_root}\vaults")),
        state: opts
            .state
            .clone()
            .unwrap_or_else(|| format!(r"{data_root}\state")),
        backups: format!(r"{data_root}\backups"),
        host: opts.host.clone().unwrap_or_else(|| DEFAULT_HOST.into()),
        port: opts.port.unwrap_or(DEFAULT_PORT),
        data_root,
    }
}

/// `arg` as the C runtime will read it back from a command line: quoted when
/// it has whitespace or a quote, backslashes before a quote doubled.
pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The task's trigger.
pub enum Trigger<'a> {
    /// When `user` logs on.
    Logon { user: &'a str },
    /// Every day at `hh:mm`, or as soon as possible after a missed run.
    Daily { at: &'a str },
}

/// A Task Scheduler definition that runs `command args` as `user` with no
/// elevation, only while they are logged on (`InteractiveToken`), never
/// stopped for its age, and restarted every minute if it fails.
pub fn task_xml(
    description: &str,
    user: &str,
    trigger: Trigger<'_>,
    command: &str,
    args: &[String],
    working_dir: &str,
) -> String {
    let trigger = match trigger {
        Trigger::Logon { user } => format!(
            "    <LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>{}</UserId>\n    </LogonTrigger>\n",
            xml(user)
        ),
        Trigger::Daily { at } => format!(
            "    <CalendarTrigger>\n      <StartBoundary>2026-01-01T{at}:00</StartBoundary>\n      \
             <Enabled>true</Enabled>\n      <ScheduleByDay>\n        <DaysInterval>1</DaysInterval>\n      \
             </ScheduleByDay>\n    </CalendarTrigger>\n"
        ),
    };
    let arguments = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{description}</Description>
  </RegistrationInfo>
  <Triggers>
{trigger}  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{working_dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        description = xml(description),
        user = xml(user),
        command = xml(command),
        arguments = xml(&arguments),
        working_dir = xml(working_dir),
    )
}

/// The server's task: `storm-serverw serve`, at logon, logging to a file.
pub fn server_task(layout: &Layout, r: &Record, user: &str) -> String {
    // The subcommand first: before it, a flag makes `normalize_argv`'s
    // legacy shim insert a second `serve` (found on a real Windows run).
    let args: Vec<String> = [
        "serve",
        "--log-file",
        &layout.log_file,
        "--vault-root",
        &r.vaults,
        "--state",
        &r.state,
        "--host",
        &r.host,
        "--port",
        &r.port.to_string(),
        "--web",
        &layout.web,
    ]
    .map(String::from)
    .to_vec();
    task_xml(
        "Storm's sync server, as you, while you are logged on (storm-server up).",
        user,
        Trigger::Logon { user },
        &layout.binw,
        &args,
        &r.data_root,
    )
}

/// The nightly backup (W-S3): `storm-serverw backup` at 03:30, or at the
/// next chance after a missed one.
pub fn backup_task(layout: &Layout, r: &Record, user: &str) -> String {
    let args: Vec<String> = [
        "backup",
        "--log-file",
        &layout.backup_log,
        "--vault-root",
        &r.vaults,
        "--state",
        &r.state,
        "--dest",
        &r.backups,
    ]
    .map(String::from)
    .to_vec();
    task_xml(
        "Storm's nightly backup of your notes (storm-server up).",
        user,
        Trigger::Daily { at: "03:30" },
        &layout.binw,
        &args,
        &r.data_root,
    )
}

/// Whether the server at `port` answers `/v1/health`, without curl.
pub fn healthy_once(port: u16) -> bool {
    use std::io::{Read, Write};
    use std::time::Duration;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(b"GET /v1/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut head = [0u8; 16];
    let n = stream.read(&mut head).unwrap_or(0);
    let line = String::from_utf8_lossy(&head[..n]);
    line.starts_with("HTTP/1.") && line.get(9..12) == Some("200")
}

#[cfg(windows)]
pub use imp::{down, status, uninstall, up};

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use anyhow::{Context, Result, bail};

    use super::*;

    fn env(name: &str) -> Result<String> {
        std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .with_context(|| format!("{name} is not set"))
    }

    fn layout() -> Result<Layout> {
        Ok(Layout::for_profile(
            &env("LOCALAPPDATA")?,
            &env("USERPROFILE")?,
        ))
    }

    fn system32(tool: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Path::new(&root).join("System32").join(tool)
    }

    /// `DOMAIN\name`, as Task Scheduler names a principal.
    fn user() -> Result<String> {
        let name = env("USERNAME")?;
        Ok(
            match std::env::var("USERDOMAIN").ok().filter(|d| !d.is_empty()) {
                Some(domain) => format!(r"{domain}\{name}"),
                None => name,
            },
        )
    }

    /// This user's SID, for a DACL that survives a renamed account.
    fn user_sid() -> Result<String> {
        let out = Command::new(system32("whoami.exe"))
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .context("running whoami")?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.trim()
            .rsplit(',')
            .next()
            .map(|s| s.trim_matches('"').to_string())
            .filter(|s| s.starts_with("S-1-"))
            .with_context(|| format!("no SID in whoami's {text:?}"))
    }

    fn run(tool: &str, args: &[&str]) -> Result<std::process::Output> {
        let out = Command::new(system32(tool))
            .args(args)
            .output()
            .with_context(|| format!("running {tool}"))?;
        Ok(out)
    }

    fn checked(tool: &str, args: &[&str]) -> Result<()> {
        let out = run(tool, args)?;
        if !out.status.success() {
            bail!(
                "{tool} {}: {}{}",
                args.join(" "),
                String::from_utf8_lossy(&out.stdout).trim(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// W-S2: the data root's DACL is the person's, SYSTEM's and
    /// Administrators', nothing inherited. The Runtime Host's account is
    /// never in it (P3's rule).
    fn owner_only(dir: &str, sid: &str) -> Result<()> {
        checked(
            "icacls.exe",
            &[
                dir,
                "/inheritance:r",
                "/grant:r",
                &format!("*{sid}:(OI)(CI)F"),
                "*S-1-5-18:(OI)(CI)F",
                "*S-1-5-32-544:(OI)(CI)F",
            ],
        )
    }

    /// Registers `name` from `definition`, written as UTF-16 (what
    /// `schtasks /xml` reads reliably).
    fn register(name: &str, definition: &str, scratch: &Path) -> Result<()> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in definition.encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        std::fs::write(scratch, bytes).with_context(|| format!("writing {}", scratch.display()))?;
        let result = checked(
            "schtasks.exe",
            &[
                "/create",
                "/tn",
                name,
                "/xml",
                &scratch.to_string_lossy(),
                "/f",
            ],
        );
        let _ = std::fs::remove_file(scratch);
        result
    }

    fn copy_tree(from: &Path, to: &Path) -> Result<()> {
        if to.exists() {
            std::fs::remove_dir_all(to).with_context(|| format!("removing {}", to.display()))?;
        }
        crate::backup::copy_tree(from, to).map(|_| ())
    }

    fn wait_healthy(port: u16) -> bool {
        for _ in 0..40 {
            if healthy_once(port) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        false
    }

    pub fn up(opts: UpOptions) -> Result<()> {
        let layout = layout()?;
        let r = record_for(&layout, &opts);
        for dir in [&r.data_root, &r.vaults, &r.state, &r.backups] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {dir}"))?;
        }
        let sid = user_sid()?;
        if let Err(e) = owner_only(&r.data_root, &sid) {
            eprintln!("warning: could not restrict {}: {e:#}", r.data_root);
        }
        std::fs::create_dir_all(Path::new(&layout.bin).parent().unwrap())?;
        std::fs::create_dir_all(&layout.logs)?;

        // A running server holds its binary; stop it before replacing it.
        let _ = run("schtasks.exe", &["/end", "/tn", TASK]);
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Whichever copy is running becomes the installed one, so `up` from an
        // unzipped release is also how Windows upgrades. Its windowless twin
        // ships beside it.
        let exe = std::env::current_exe().context("finding this binary")?;
        let exe_dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
        let same = |a: &Path, b: &str| a.canonicalize().ok() == Path::new(b).canonicalize().ok();
        for (name, dest) in [
            ("storm-server.exe", &layout.bin),
            ("storm-serverw.exe", &layout.binw),
        ] {
            let src = exe_dir.join(name);
            if !src.exists() {
                bail!("{name} is not beside this binary ({})", exe_dir.display());
            }
            if !same(&src, dest) {
                copy_when_released(&src, Path::new(dest))
                    .with_context(|| format!("copying {name} to {dest}"))?;
            }
        }
        let web_src = opts
            .web
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| exe_dir.join("web"));
        if web_src.join("index.html").exists() {
            if !same(&web_src, &layout.web) {
                copy_tree(&web_src, Path::new(&layout.web))?;
            }
        } else if !Path::new(&layout.web).join("index.html").exists() {
            eprintln!(
                "warning: no web client beside this binary ({}) — the server runs, but \
                 http://…:{} shows no app until one is installed",
                web_src.display(),
                r.port
            );
        }

        let old_port = std::fs::read(&layout.record)
            .ok()
            .and_then(|b| serde_json::from_slice::<Record>(&b).ok())
            .map(|old| old.port);
        std::fs::write(&layout.record, serde_json::to_string_pretty(&r)?)
            .with_context(|| format!("writing {}", layout.record))?;
        let user = user()?;
        let scratch = Path::new(&layout.support).join("task.xml");
        register(TASK, &server_task(&layout, &r, &user), &scratch)?;
        if let Err(e) = register(BACKUP_TASK, &backup_task(&layout, &r, &user), &scratch) {
            eprintln!("warning: the nightly backup is not scheduled: {e:#}");
        }
        checked("schtasks.exe", &["/run", "/tn", TASK])?;

        let healthy = wait_healthy(r.port);
        println!(
            "storm-server is {}",
            if healthy {
                "up"
            } else {
                "starting (not answering yet)"
            }
        );
        println!("  vaults    : {}", r.vaults);
        println!("  state     : {}", r.state);
        println!("  backups   : {} (nightly, 03:30)", r.backups);
        println!("  listen    : {}:{}", r.host, r.port);
        println!("  binary    : {}", layout.binw);
        println!("  task      : {TASK} (at logon, as {user})");
        println!("  log       : {}", layout.log_file);
        println!();
        println!("It runs as you while you are logged on, and starts again at logon.");
        let reminder = old_port
            .map(|old| {
                crate::firewall::port_change_reminder(
                    crate::nettrust::Platform::Windows,
                    &crate::nettrust::RealOs,
                    old,
                    r.port,
                    &layout.binw,
                )
            })
            .unwrap_or_default();
        if reminder.is_empty() {
            println!(
                "So your phone can reach it, allow it through Windows Firewall once, on \
                 private networks only, in an elevated PowerShell:\n  {}",
                crate::firewall::windows_rule_command(&layout.binw, r.port)
            );
        } else {
            for line in reminder {
                println!("{line}");
            }
        }
        if !healthy {
            bail!(
                "the server did not answer on port {} — see {}",
                r.port,
                layout.log_file
            );
        }
        Ok(())
    }

    pub fn down() -> Result<()> {
        for task in [TASK, BACKUP_TASK] {
            let _ = run("schtasks.exe", &["/end", "/tn", task]);
            // Disabled, or it would start again at the next logon.
            let _ = run("schtasks.exe", &["/change", "/tn", task, "/disable"]);
        }
        println!("storm-server stopped; it stays off until `storm-server up`.");
        Ok(())
    }

    pub fn status() -> Result<()> {
        let layout = layout()?;
        let Some(r) = std::fs::read(&layout.record)
            .ok()
            .and_then(|b| serde_json::from_slice::<Record>(&b).ok())
        else {
            println!(
                "storm-server is not installed for this user (no {})",
                layout.record
            );
            std::process::exit(3);
        };
        let out = run("schtasks.exe", &["/query", "/tn", TASK, "/fo", "list"])?;
        let text = String::from_utf8_lossy(&out.stdout);
        let state = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("Status:"))
            .map(str::trim)
            .unwrap_or(if out.status.success() {
                "registered"
            } else {
                "not registered"
            })
            .to_string();
        let healthy = healthy_once(r.port);
        println!("task    : {TASK}: {state}");
        println!("health  : {}", if healthy { "ok" } else { "not answering" });
        println!("listen  : {}:{}", r.host, r.port);
        println!("vaults  : {}", r.vaults);
        println!("state   : {}", r.state);
        println!("log     : {}", layout.log_file);
        if healthy {
            for line in crate::discovery::status_identity(r.port, 8) {
                println!("{line}");
            }
        }
        for line in crate::firewall::status_here(r.port, &layout.binw, 8) {
            println!("{line}");
        }
        if !healthy {
            std::process::exit(1);
        }
        Ok(())
    }

    /// `schtasks /end` returns before the server has exited, and Windows
    /// cannot replace a running binary: an upgrade retries for up to 15 s
    /// (found on a real Windows run).
    fn copy_when_released(src: &Path, dest: &Path) -> std::io::Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            match std::fs::copy(src, dest) {
                Ok(_) => return Ok(()),
                Err(e) if std::time::Instant::now() >= deadline => return Err(e),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
            }
        }
    }

    /// The same for `uninstall`: nor can it delete a running binary or an
    /// open log.
    fn remove_when_released(dir: &Path) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            match std::fs::remove_dir_all(dir) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) if std::time::Instant::now() >= deadline => {
                    return Err(e).with_context(|| format!("removing {}", dir.display()));
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
            }
        }
    }

    /// Stops the server and removes what `up` installed. The data root is
    /// never touched: it is the person's notes.
    pub fn uninstall() -> Result<()> {
        let layout = layout()?;
        let record = std::fs::read(&layout.record)
            .ok()
            .and_then(|b| serde_json::from_slice::<Record>(&b).ok());
        for task in [TASK, BACKUP_TASK] {
            let _ = run("schtasks.exe", &["/end", "/tn", task]);
            let _ = run("schtasks.exe", &["/delete", "/tn", task, "/f"]);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let me = std::env::current_exe().ok();
        let running_from_support = me
            .as_deref()
            .and_then(|m| m.canonicalize().ok())
            .zip(Path::new(&layout.support).canonicalize().ok())
            .is_some_and(|(m, s)| m.starts_with(s));
        if running_from_support {
            // Windows cannot delete a running binary.
            for path in [&layout.web, &layout.record] {
                let _ = std::fs::remove_dir_all(path).or_else(|_| std::fs::remove_file(path));
            }
            println!("Remove {} once this command has finished.", layout.support);
        } else if Path::new(&layout.support).exists() {
            remove_when_released(Path::new(&layout.support))?;
        }
        println!("storm-server is uninstalled for this user.");
        match record {
            Some(r) => println!(
                "Your notes are untouched: {} (vaults, state, backups). Delete it yourself if \
                 you mean to.",
                r.data_root
            ),
            None => println!(r"Your notes, if any, are untouched (default %USERPROFILE%\Storm)."),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::for_profile(
            r"C:\Users\Ada Lovelace\AppData\Local",
            r"C:\Users\Ada Lovelace",
        )
    }

    #[test]
    fn the_layout_is_the_persons_own() {
        let l = layout();
        assert_eq!(
            l.binw,
            r"C:\Users\Ada Lovelace\AppData\Local\Storm\bin\storm-serverw.exe"
        );
        assert_eq!(l.default_data_root, r"C:\Users\Ada Lovelace\Storm");
        assert_eq!(
            l.log_file,
            r"C:\Users\Ada Lovelace\AppData\Local\Storm\logs\storm-server.log"
        );
    }

    #[test]
    fn the_record_defaults_and_honours_overrides() {
        let l = layout();
        let r = record_for(&l, &UpOptions::default());
        assert_eq!(r.vaults, r"C:\Users\Ada Lovelace\Storm\vaults");
        assert_eq!(r.state, r"C:\Users\Ada Lovelace\Storm\state");
        assert_eq!(r.backups, r"C:\Users\Ada Lovelace\Storm\backups");
        assert_eq!((r.host.as_str(), r.port), ("0.0.0.0", 8484));
        let r = record_for(
            &l,
            &UpOptions {
                data_root: Some(r"D:\Notes".into()),
                vault_root: Some(r"\\nas\vaults".into()),
                port: Some(9000),
                ..UpOptions::default()
            },
        );
        assert_eq!(r.vaults, r"\\nas\vaults");
        assert_eq!(r.state, r"D:\Notes\state");
        assert_eq!(r.port, 9000);
    }

    /// What `CommandLineToArgvW` would split `line` into, for the cases
    /// `quote` produces (quoted runs, escaped quotes, backslashes).
    fn split(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut cur = String::new();
        let (mut quoted, mut any) = (false, false);
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '\\' => {
                    let n = chars[i..].iter().take_while(|&&c| c == '\\').count();
                    i += n;
                    if chars.get(i) == Some(&'"') {
                        cur.extend(std::iter::repeat_n('\\', n / 2));
                        if n % 2 == 1 {
                            cur.push('"');
                            i += 1;
                        }
                    } else {
                        cur.extend(std::iter::repeat_n('\\', n));
                    }
                    any = true;
                    continue;
                }
                '"' => {
                    quoted = !quoted;
                    any = true;
                }
                ' ' if !quoted => {
                    if any {
                        args.push(std::mem::take(&mut cur));
                        any = false;
                    }
                }
                c => {
                    cur.push(c);
                    any = true;
                }
            }
            i += 1;
        }
        if any {
            args.push(cur);
        }
        args
    }

    #[test]
    fn task_arguments_survive_the_c_runtimes_parser() {
        let args = [
            r"C:\Users\Ada Lovelace\Storm\vaults",
            r"C:\trailing\",
            r#"a"quote"#,
            "plain",
        ];
        let line = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
        assert_eq!(split(&line), args);
    }

    #[test]
    fn the_server_task_runs_serve_windowless_at_logon_as_the_person() {
        let l = layout();
        let r = record_for(&l, &UpOptions::default());
        let x = server_task(&l, &r, r"PC\Ada");
        assert!(x.contains("<LogonTrigger>"));
        assert!(x.contains(r"<UserId>PC\Ada</UserId>"));
        assert!(x.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(x.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(x.contains("<RestartOnFailure>"));
        assert!(x.contains(&format!("<Command>{}</Command>", l.binw)));
        // Paths with spaces are quoted, and XML-escaped inside the element.
        assert!(
            x.contains(r"--vault-root &quot;C:\Users\Ada Lovelace\Storm\vaults&quot;"),
            "{x}"
        );
        assert!(x.contains("<Arguments>serve "), "{x}");
        assert!(x.contains("--log-file"));
        assert!(x.starts_with(r#"<?xml version="1.0" encoding="UTF-16"?>"#));
    }

    /// What each task's action would run, parsed by the real command line,
    /// shim included. A task whose arguments do not parse exits 2 before it
    /// can open its log, and nothing says why.
    #[test]
    fn every_task_runs_a_command_line_that_parses() {
        use clap::Parser;
        let l = layout();
        let r = record_for(&l, &UpOptions::default());
        for x in [
            server_task(&l, &r, "PC\\Ada"),
            backup_task(&l, &r, "PC\\Ada"),
        ] {
            let args = x
                .split("<Arguments>")
                .nth(1)
                .and_then(|a| a.split("</Arguments>").next())
                .unwrap()
                .replace("&quot;", "\"");
            let mut argv: Vec<std::ffi::OsString> = vec!["storm-serverw".into()];
            argv.extend(split(&args).into_iter().map(Into::into));
            let parsed = crate::Cli::try_parse_from(crate::normalize_argv(argv.clone()));
            assert!(parsed.is_ok(), "{argv:?}: {}", parsed.unwrap_err());
            assert!(parsed.unwrap().log_file.is_some());
        }
    }

    #[test]
    fn the_backup_task_runs_nightly_and_catches_up() {
        let l = layout();
        let r = record_for(&l, &UpOptions::default());
        let x = backup_task(&l, &r, r"PC\Ada");
        assert!(x.contains("<StartBoundary>2026-01-01T03:30:00</StartBoundary>"));
        assert!(x.contains("<DaysInterval>1</DaysInterval>"));
        assert!(x.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(x.contains("<Arguments>backup "), "{x}");
        assert!(x.contains(&xml(&quote(&r.backups))), "{x}");
    }

    #[test]
    fn a_principal_or_path_cannot_break_the_xml() {
        let x = task_xml(
            "d",
            "a<b>&\"",
            Trigger::Daily { at: "01:00" },
            "c",
            &[],
            "w",
        );
        assert!(x.contains("<UserId>a&lt;b&gt;&amp;&quot;</UserId>"));
    }

    #[test]
    fn health_is_a_200_from_the_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let serve = |status: &'static str| {
            let listener = listener.try_clone().unwrap();
            std::thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 256];
                let _ = s.read(&mut buf);
                let _ = s.write_all(format!("HTTP/1.1 {status}\r\n\r\n").as_bytes());
            })
        };
        let t = serve("200 OK");
        assert!(healthy_once(port));
        t.join().unwrap();
        let t = serve("503 Service Unavailable");
        assert!(!healthy_once(port));
        t.join().unwrap();
        drop(listener);
        assert!(!healthy_once(port));
    }
}
