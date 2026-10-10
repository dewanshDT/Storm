//! `up`, `down`, `status` and `uninstall` on macOS (decision 84d).
//!
//! On a Mac the server is the person's own: it runs as a **LaunchAgent**, as
//! the logged-in user, with the notes in `~/Storm`. Nothing here needs root,
//! and `up` refuses to run as root, which would put the agent and the data in
//! root's home. The server runs while the person is logged in; that is enough
//! for a laptop, and it keeps the notes ordinary files in their home that any
//! editor can open. `~/Storm` is outside every folder macOS's privacy controls
//! (TCC) guard, so the server never triggers a prompt.
//!
//! Layout:
//! - `~/Library/Application Support/Storm/bin/storm-server` — the binary `up`
//!   copied itself to, so the launch agent never runs from a download folder;
//!   `bin/storm-backup.sh` beside it; `web/` — the web client;
//!   `server.json` — what `up` chose, for `status`, `down` and the installer;
//! - `~/Library/LaunchAgents/dev.storm.server.plist` and
//!   `dev.storm.server.backup.plist` (nightly, `deploy/storm-backup.sh`);
//! - `~/Library/Logs/Storm/storm-server.log`, `storm-backup.log`;
//! - data: `~/Storm/{vaults,state,backups}`, never removed by `uninstall`.
//!
//! The pure parts (layout, plists, `server.json`) compile everywhere and are
//! tested on Linux; only the `launchctl` calls are macOS-only in practice.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const LABEL: &str = "dev.storm.server";
pub const BACKUP_LABEL: &str = "dev.storm.server.backup";
const DEFAULT_PORT: u16 = 8484;
const DEFAULT_HOST: &str = "0.0.0.0";

/// Where everything goes for the user whose home is `home`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub support: PathBuf,
    pub bin: PathBuf,
    pub backup_script: PathBuf,
    pub web: PathBuf,
    pub record: PathBuf,
    pub agents: PathBuf,
    pub plist: PathBuf,
    pub backup_plist: PathBuf,
    pub logs: PathBuf,
    pub log_file: PathBuf,
    pub backup_log: PathBuf,
    pub default_data_root: PathBuf,
}

impl Layout {
    pub fn for_home(home: &Path) -> Self {
        let support = home.join("Library/Application Support/Storm");
        let agents = home.join("Library/LaunchAgents");
        let logs = home.join("Library/Logs/Storm");
        Self {
            bin: support.join("bin/storm-server"),
            backup_script: support.join("bin/storm-backup.sh"),
            web: support.join("web"),
            record: support.join("server.json"),
            plist: agents.join(format!("{LABEL}.plist")),
            backup_plist: agents.join(format!("{BACKUP_LABEL}.plist")),
            log_file: logs.join("storm-server.log"),
            backup_log: logs.join("storm-backup.log"),
            default_data_root: home.join("Storm"),
            support,
            agents,
            logs,
        }
    }

    pub fn current() -> Result<Self> {
        Ok(Self::for_home(&home()?))
    }
}

fn home() -> Result<PathBuf> {
    match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => Ok(PathBuf::from(h)),
        _ => bail!("HOME is not set"),
    }
}

/// What `up` chose, kept beside the binary: `status`, `down` and the
/// installer read it rather than parsing the plist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub data_root: PathBuf,
    pub vaults: PathBuf,
    pub state: PathBuf,
    pub backups: PathBuf,
    pub host: String,
    pub port: u16,
}

impl Record {
    fn load(layout: &Layout) -> Option<Self> {
        serde_json::from_slice(&fs::read(&layout.record).ok()?).ok()
    }
}

/// `up`'s choices; `None` takes the macOS default.
#[derive(Debug, Default)]
pub struct UpOptions {
    pub data_root: Option<PathBuf>,
    pub vault_root: Option<PathBuf>,
    pub state: Option<PathBuf>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub web: Option<PathBuf>,
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn plist(label: &str, args: &[String], env: &[(&str, String)], extra: &str) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    out.push_str(&format!(
        "  <key>Label</key>\n  <string>{}</string>\n",
        xml(label)
    ));
    out.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for a in args {
        out.push_str(&format!("    <string>{}</string>\n", xml(a)));
    }
    out.push_str("  </array>\n");
    if !env.is_empty() {
        out.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (k, v) in env {
            out.push_str(&format!(
                "    <key>{}</key>\n    <string>{}</string>\n",
                xml(k),
                xml(v)
            ));
        }
        out.push_str("  </dict>\n");
    }
    out.push_str(extra);
    out.push_str("</dict>\n</plist>\n");
    out
}

/// The server's launch agent: started at login and kept alive.
pub fn server_plist(layout: &Layout, r: &Record) -> String {
    let s = |p: &Path| p.display().to_string();
    plist(
        LABEL,
        &[
            s(&layout.bin),
            "serve".into(),
            "--vault-root".into(),
            s(&r.vaults),
            "--state".into(),
            s(&r.state),
            "--host".into(),
            r.host.clone(),
            "--port".into(),
            r.port.to_string(),
            "--web".into(),
            s(&layout.web),
        ],
        &[("RUST_LOG", "info".into())],
        &format!(
            "  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <true/>\n\
             \x20 <key>StandardOutPath</key>\n  <string>{log}</string>\n\
             \x20 <key>StandardErrorPath</key>\n  <string>{log}</string>\n",
            log = xml(&s(&layout.log_file))
        ),
    )
}

/// The nightly backup: `deploy/storm-backup.sh`, the same script as Linux's
/// timer, at 03:30 (or at the next wake, which launchd catches up on).
pub fn backup_plist(layout: &Layout, r: &Record) -> String {
    let s = |p: &Path| p.display().to_string();
    plist(
        BACKUP_LABEL,
        &["/bin/bash".into(), s(&layout.backup_script)],
        &[
            ("STORM_VAULT_ROOT", s(&r.vaults)),
            ("STORM_STATE", s(&r.state)),
            ("STORM_BACKUP_DEST", s(&r.backups)),
            ("STORM_SERVER_BIN", s(&layout.bin)),
            ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ],
        &format!(
            "  <key>StartCalendarInterval</key>\n  <dict>\n    <key>Hour</key>\n    \
             <integer>3</integer>\n    <key>Minute</key>\n    <integer>30</integer>\n  </dict>\n\
             \x20 <key>StandardOutPath</key>\n  <string>{log}</string>\n\
             \x20 <key>StandardErrorPath</key>\n  <string>{log}</string>\n",
            log = xml(&s(&layout.backup_log))
        ),
    )
}

/// The record `up` writes for `opts`, its paths resolved against `layout`.
pub fn record_for(layout: &Layout, opts: &UpOptions) -> Record {
    let data_root = opts
        .data_root
        .clone()
        .unwrap_or_else(|| layout.default_data_root.clone());
    Record {
        vaults: opts
            .vault_root
            .clone()
            .unwrap_or_else(|| data_root.join("vaults")),
        state: opts
            .state
            .clone()
            .unwrap_or_else(|| data_root.join("state")),
        backups: data_root.join("backups"),
        host: opts.host.clone().unwrap_or_else(|| DEFAULT_HOST.into()),
        port: opts.port.unwrap_or(DEFAULT_PORT),
        data_root,
    }
}

fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// `gui/<uid>` when the user has a login session (the normal case), else
/// `user/<uid>` (an SSH-only session, a CI runner without one).
fn domain() -> String {
    let uid = uid();
    let gui = format!("gui/{uid}");
    let ok = Command::new("launchctl")
        .args(["print", &gui])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok { gui } else { format!("user/{uid}") }
}

fn launchctl(args: &[&str]) -> Result<std::process::Output> {
    Command::new("launchctl")
        .args(args)
        .output()
        .with_context(|| format!("running launchctl {}", args.join(" ")))
}

fn load(domain: &str, label: &str, plist: &Path) -> Result<()> {
    // An older copy may be loaded (an upgrade): unload it first, quietly.
    let _ = launchctl(&["bootout", &format!("{domain}/{label}")]);
    let _ = launchctl(&["enable", &format!("{domain}/{label}")]);
    let out = launchctl(&["bootstrap", domain, &plist.display().to_string()])?;
    if !out.status.success() {
        bail!(
            "launchctl bootstrap {domain} {} failed: {}",
            plist.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn copy_file(from: &Path, to: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = to.with_extension("tmp");
    fs::copy(from, &tmp)
        .with_context(|| format!("copying {} to {}", from.display(), tmp.display()))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, to).with_context(|| format!("installing {}", to.display()))?;
    Ok(())
}

/// Replaces `to` with a copy of the directory `from`.
pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    let tmp = to.with_extension("new");
    let _ = fs::remove_dir_all(&tmp);
    copy_tree(from, &tmp)?;
    let _ = fs::remove_dir_all(to);
    fs::rename(&tmp, to).with_context(|| format!("installing {}", to.display()))?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn write(path: &Path, body: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn wait_healthy(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/v1/health");
    for _ in 0..40 {
        let ok = Command::new("curl")
            .args(["-sf", "-o", "/dev/null", "--max-time", "2", &url])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    false
}

pub fn up(opts: UpOptions) -> Result<()> {
    if uid() == "0" {
        bail!(
            "on macOS the server runs as you, not root: run `storm-server up` without sudo \
             (it installs a LaunchAgent in your account and keeps the notes in ~/Storm)"
        );
    }
    let layout = Layout::current()?;
    let r = record_for(&layout, &opts);
    for dir in [&r.data_root, &r.vaults, &r.state, &r.backups] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    for dir in [&r.data_root, &r.state, &r.backups] {
        let _ = crate::install::tighten_private_dir(dir);
    }
    fs::create_dir_all(layout.bin.parent().unwrap())?;
    fs::create_dir_all(&layout.logs)?;

    // The binary: whichever copy is running becomes the installed one, so `up`
    // from an extracted release tarball is also how a Mac upgrades.
    let exe = std::env::current_exe()
        .context("finding this binary")?
        .canonicalize()?;
    let exe_dir = exe.parent().unwrap_or(Path::new("/")).to_path_buf();
    if exe != layout.bin.canonicalize().unwrap_or_default() {
        copy_file(&exe, &layout.bin, 0o755)?;
    }
    let script = exe_dir.join("storm-backup.sh");
    if script.exists() && script != layout.backup_script {
        copy_file(&script, &layout.backup_script, 0o755)?;
    }
    let web_src = opts.web.clone().unwrap_or_else(|| exe_dir.join("web"));
    if web_src.join("index.html").exists() {
        if web_src.canonicalize().ok() != layout.web.canonicalize().ok() {
            copy_dir(&web_src, &layout.web)?;
        }
    } else if !layout.web.join("index.html").exists() {
        eprintln!(
            "warning: no web client beside this binary ({}) — the server runs, but \
             http://…:{} shows no app until one is installed",
            web_src.display(),
            r.port
        );
    }

    write(&layout.record, &serde_json::to_string_pretty(&r)?)?;
    write(&layout.plist, &server_plist(&layout, &r))?;
    let domain = domain();
    load(&domain, LABEL, &layout.plist)?;
    if layout.backup_script.exists() {
        write(&layout.backup_plist, &backup_plist(&layout, &r))?;
        if let Err(e) = load(&domain, BACKUP_LABEL, &layout.backup_plist) {
            eprintln!("warning: the nightly backup is not scheduled: {e:#}");
        }
    }

    let healthy = wait_healthy(r.port);
    println!(
        "storm-server is {}",
        if healthy {
            "up"
        } else {
            "starting (not answering yet)"
        }
    );
    println!("  vaults    : {}", r.vaults.display());
    println!("  state     : {}", r.state.display());
    println!("  backups   : {} (nightly, 03:30)", r.backups.display());
    println!("  listen    : {}:{}", r.host, r.port);
    println!("  binary    : {}", layout.bin.display());
    println!("  agent     : {} ({domain})", layout.plist.display());
    println!("  log       : {}", layout.log_file.display());
    println!();
    println!("It runs as you while you are logged in, and starts again at login.");
    println!(
        "macOS may ask once whether storm-server may accept incoming connections: allow it \
         so your phone can reach it."
    );
    if !healthy {
        bail!(
            "the server did not answer on port {} — see {}",
            r.port,
            layout.log_file.display()
        );
    }
    Ok(())
}

pub fn down() -> Result<()> {
    let layout = Layout::current()?;
    let domain = domain();
    for label in [LABEL, BACKUP_LABEL] {
        let _ = launchctl(&["bootout", &format!("{domain}/{label}")]);
        // Disabled, or the agent would start again at the next login.
        let _ = launchctl(&["disable", &format!("{domain}/{label}")]);
    }
    println!(
        "storm-server stopped; it stays off until `storm-server up` ({})",
        layout.plist.display()
    );
    Ok(())
}

pub fn status() -> Result<()> {
    let layout = Layout::current()?;
    let Some(r) = Record::load(&layout) else {
        println!(
            "storm-server is not installed for this user (no {})",
            layout.record.display()
        );
        std::process::exit(3);
    };
    let domain = domain();
    let out = launchctl(&["print", &format!("{domain}/{LABEL}")])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let state = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("state = "))
        .unwrap_or(if out.status.success() {
            "loaded"
        } else {
            "not loaded"
        })
        .to_string();
    let healthy = wait_healthy_once(r.port);
    println!("agent   : {LABEL} ({domain}): {state}");
    println!("health  : {}", if healthy { "ok" } else { "not answering" });
    println!("listen  : {}:{}", r.host, r.port);
    println!("vaults  : {}", r.vaults.display());
    println!("state   : {}", r.state.display());
    println!("log     : {}", layout.log_file.display());
    if !healthy {
        std::process::exit(1);
    }
    Ok(())
}

fn wait_healthy_once(port: u16) -> bool {
    Command::new("curl")
        .args([
            "-sf",
            "-o",
            "/dev/null",
            "--max-time",
            "2",
            &format!("http://127.0.0.1:{port}/v1/health"),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stops the server and removes what `up` installed. The data root is never
/// touched: it is the person's notes.
pub fn uninstall() -> Result<()> {
    let layout = Layout::current()?;
    let record = Record::load(&layout);
    let domain = domain();
    for label in [LABEL, BACKUP_LABEL] {
        let _ = launchctl(&["bootout", &format!("{domain}/{label}")]);
        let _ = launchctl(&["enable", &format!("{domain}/{label}")]);
    }
    for f in [
        &layout.plist,
        &layout.backup_plist,
        &layout.log_file,
        &layout.backup_log,
    ] {
        let _ = fs::remove_file(f);
    }
    if layout.support.exists() {
        fs::remove_dir_all(&layout.support)
            .with_context(|| format!("removing {}", layout.support.display()))?;
    }
    println!("storm-server is uninstalled for this user.");
    match record {
        Some(r) => println!(
            "Your notes are untouched: {} (vaults, state, backups). Delete it yourself if you \
             mean to.",
            r.data_root.display()
        ),
        None => println!("Your notes, if any, are untouched (default ~/Storm)."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_lives_in_the_users_library_and_the_data_in_storm() {
        let l = Layout::for_home(Path::new("/Users/ada"));
        assert_eq!(
            l.bin,
            Path::new("/Users/ada/Library/Application Support/Storm/bin/storm-server")
        );
        assert_eq!(
            l.plist,
            Path::new("/Users/ada/Library/LaunchAgents/dev.storm.server.plist")
        );
        assert_eq!(l.default_data_root, Path::new("/Users/ada/Storm"));
        assert_eq!(
            l.log_file,
            Path::new("/Users/ada/Library/Logs/Storm/storm-server.log")
        );
    }

    #[test]
    fn the_record_defaults_and_honours_overrides() {
        let l = Layout::for_home(Path::new("/Users/ada"));
        let r = record_for(&l, &UpOptions::default());
        assert_eq!(r.vaults, Path::new("/Users/ada/Storm/vaults"));
        assert_eq!(r.state, Path::new("/Users/ada/Storm/state"));
        assert_eq!(r.backups, Path::new("/Users/ada/Storm/backups"));
        assert_eq!((r.host.as_str(), r.port), ("0.0.0.0", 8484));
        let r = record_for(
            &l,
            &UpOptions {
                data_root: Some("/Volumes/x/S".into()),
                vault_root: Some("/elsewhere/v".into()),
                port: Some(9000),
                ..Default::default()
            },
        );
        assert_eq!(r.vaults, Path::new("/elsewhere/v"));
        assert_eq!(r.state, Path::new("/Volumes/x/S/state"));
        assert_eq!(r.port, 9000);
        let back: Record = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn the_server_plist_runs_serve_as_recorded_and_escapes_paths() {
        let l = Layout::for_home(Path::new("/Users/a&b"));
        let r = record_for(&l, &UpOptions::default());
        let p = server_plist(&l, &r);
        assert!(p.contains("<string>dev.storm.server</string>"));
        assert!(p.contains(
            "<string>/Users/a&amp;b/Library/Application Support/Storm/bin/storm-server</string>"
        ));
        for arg in [
            "serve",
            "--vault-root",
            "--state",
            "--host",
            "0.0.0.0",
            "--port",
            "8484",
            "--web",
        ] {
            assert!(p.contains(&format!("<string>{arg}</string>")), "{arg}");
        }
        assert!(p.contains("<key>KeepAlive</key>\n  <true/>"));
        assert!(p.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(!p.contains("a&b"), "an unescaped & breaks the plist");
        assert!(p.ends_with("</dict>\n</plist>\n"));
    }

    #[test]
    fn the_backup_plist_runs_the_backup_script_nightly_with_the_servers_paths() {
        let l = Layout::for_home(Path::new("/Users/ada"));
        let r = record_for(&l, &UpOptions::default());
        let p = backup_plist(&l, &r);
        assert!(p.contains("<string>dev.storm.server.backup</string>"));
        assert!(p.contains("<string>/bin/bash</string>"));
        assert!(p.contains("storm-backup.sh</string>"));
        assert!(p.contains("<key>STORM_STATE</key>\n    <string>/Users/ada/Storm/state</string>"));
        assert!(p.contains("<key>STORM_SERVER_BIN</key>"));
        assert!(p.contains("<key>Hour</key>\n    <integer>3</integer>"));
    }

    #[test]
    fn copy_dir_replaces_the_target() {
        let t = tempdir::TempDir::new("storm-copydir").unwrap();
        let from = t.path().join("from");
        fs::create_dir_all(from.join("assets")).unwrap();
        fs::write(from.join("index.html"), "new").unwrap();
        fs::write(from.join("assets/a.js"), "js").unwrap();
        let to = t.path().join("to");
        fs::create_dir_all(&to).unwrap();
        fs::write(to.join("stale.txt"), "old").unwrap();
        copy_dir(&from, &to).unwrap();
        assert_eq!(fs::read_to_string(to.join("index.html")).unwrap(), "new");
        assert_eq!(fs::read_to_string(to.join("assets/a.js")).unwrap(), "js");
        assert!(!to.join("stale.txt").exists());
    }
}
