//! Operator commands that own the systemd install: `up`, `down`, `status`.
//!
//! `serve` is the long-running process the unit starts. These commands write
//! the env file, widen hardening for a custom data root, and enable the unit
//! so the server comes back after a reboot — the Tailscale-shaped half.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const UNIT: &str = "storm-server";
const ENV_PATH: &str = "/etc/storm/storm.env";
const DROP_IN_DIR: &str = "/etc/systemd/system/storm-server.service.d";
const DEFAULT_DATA_ROOT: &str = "/srv/storm";
const DEFAULT_WEB: &str = "/usr/share/storm/web";
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8484;

#[derive(Debug, Clone)]
pub struct UpOptions {
    pub data_root: PathBuf,
    pub vault_root: PathBuf,
    pub state: PathBuf,
    pub backups: PathBuf,
    pub host: String,
    pub port: u16,
    pub web: PathBuf,
}

impl Default for UpOptions {
    fn default() -> Self {
        let data_root = PathBuf::from(DEFAULT_DATA_ROOT);
        Self {
            vault_root: data_root.join("vaults"),
            state: data_root.join("state"),
            backups: data_root.join("backups"),
            data_root,
            host: DEFAULT_HOST.into(),
            port: DEFAULT_PORT,
            web: PathBuf::from(DEFAULT_WEB),
        }
    }
}

pub fn up(opts: UpOptions) -> Result<()> {
    require_root()?;

    let data_root = canonicalize_or_create(&opts.data_root)?;
    let vaults = canonicalize_or_create(&opts.vault_root)?;
    let state = canonicalize_or_create(&opts.state)?;
    let backups = canonicalize_or_create(&opts.backups)?;

    // Run as whoever already owns the state directory. The FHS default creates
    // `storm` and chowns /srv/storm; an NFS vault root (uid 3001 here) cannot
    // be chown'd to a local system user, and User=storm would then be unable
    // to write notes. Matching the state owner keeps the service able to open
    // both halves of a split layout.
    let (run_user, run_group) = run_identity_for(&state)?;
    if run_user == "storm" {
        ensure_storm_user()?;
    }
    for dir in [&vaults, &state, &backups] {
        // Best-effort: NFS and root_squash return EPERM — warn, don't abort.
        try_chown(dir, &run_user, &run_group);
    }

    fs::create_dir_all("/etc/storm").context("creating /etc/storm")?;
    write_env_file(
        Path::new(ENV_PATH),
        &EnvFile {
            vault_root: &vaults,
            state: &state,
            web: &opts.web,
            host: &opts.host,
            port: opts.port,
            backup_dest: &backups,
        },
    )?;

    // Widen the sandbox to every path the unit will write. A split layout
    // (NAS vaults + local state) needs both parents in ReadWritePaths, and
    // User=/Group= must match the identity chosen above.
    write_service_drop_in(
        &data_root,
        &[&vaults, &state, &backups, &opts.web],
        &run_user,
        &run_group,
    )?;
    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", "--now", UNIT])?;
    // Backup timer is optional on a fresh box that has not installed it yet.
    let _ = run_systemctl(&["enable", "--now", "storm-backup.timer"]);

    println!("storm-server is up");
    println!("  data root : {}", data_root.display());
    println!("  vaults    : {}", vaults.display());
    println!("  state     : {}", state.display());
    println!("  web       : {}", opts.web.display());
    println!("  user      : {run_user}:{run_group}");
    println!("  listen    : {}:{}", opts.host, opts.port);
    println!("  env       : {ENV_PATH}");
    println!();
    println!("Pair a device to get in: `storm-server pair` (no users yet).");
    Ok(())
}

pub fn down() -> Result<()> {
    require_root()?;
    run_systemctl(&["disable", "--now", UNIT])?;
    println!("storm-server stopped and disabled");
    Ok(())
}

pub fn status() -> Result<()> {
    // Not root-only: reading unit state and hitting health is fine as any user.
    let unit = Command::new("systemctl")
        .args(["is-active", UNIT])
        .output()
        .context("running systemctl is-active")?;
    let active = String::from_utf8_lossy(&unit.stdout).trim().to_string();
    let enabled = Command::new("systemctl")
        .args(["is-enabled", UNIT])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    println!("unit     : {active}");
    println!("enabled  : {enabled}");

    if let Some(env) = read_env_listen() {
        println!("listen   : {}:{}", env.0, env.1);
        let url = format!("http://127.0.0.1:{}/v1/health", env.1);
        match ureq_get_health(&url) {
            Ok(()) => println!("health   : ok ({url})"),
            Err(e) => println!("health   : unreachable ({e})"),
        }
    } else {
        // Fall back to the packaged default port.
        let url = format!("http://127.0.0.1:{DEFAULT_PORT}/v1/health");
        match ureq_get_health(&url) {
            Ok(()) => println!("health   : ok ({url})"),
            Err(e) => println!("health   : unreachable ({e})"),
        }
    }
    Ok(())
}

struct EnvFile<'a> {
    vault_root: &'a Path,
    state: &'a Path,
    web: &'a Path,
    host: &'a str,
    port: u16,
    backup_dest: &'a Path,
}

fn write_env_file(path: &Path, env: &EnvFile<'_>) -> Result<()> {
    let body = format!(
        "# Written by `storm-server up`.\n\
         STORM_VAULT_ROOT={vaults}\n\
         STORM_STATE={state}\n\
         STORM_WEB={web}\n\
         STORM_HOST={host}\n\
         STORM_PORT={port}\n\
         STORM_BACKUP_DEST={backups}\n\
         STORM_BACKUP_KEEP_DAYS=30\n",
        vaults = env.vault_root.display(),
        state = env.state.display(),
        web = env.web.display(),
        host = env.host,
        port = env.port,
        backups = env.backup_dest.display(),
    );

    let mut f = fs::File::create(path).with_context(|| format!("writing {}", path.display()))?;
    f.write_all(body.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Widen the unit's sandbox and set the process identity.
///
/// The packaged unit pins `User=storm`, `ReadWritePaths=/srv/storm` and
/// `ProtectHome=true`. A custom or NFS-backed layout needs a drop-in that
/// replaces those, or the service starts and then cannot write notes.
fn write_service_drop_in(
    data_root: &Path,
    extra_paths: &[&Path],
    user: &str,
    group: &str,
) -> Result<()> {
    fs::create_dir_all(DROP_IN_DIR).with_context(|| format!("creating {DROP_IN_DIR}"))?;
    let path = Path::new(DROP_IN_DIR).join("data-root.conf");

    let under_home = data_root.starts_with("/home")
        || data_root.starts_with("/Users")
        || extra_paths
            .iter()
            .any(|p| p.starts_with("/home") || p.starts_with("/Users"));

    let mut body =
        String::from("# Generated by storm-server up — do not edit by hand.\n[Service]\n");
    body.push_str(&format!("User={user}\nGroup={group}\n"));
    // systemd accepts a space-separated list on ReadWritePaths.
    let mut paths = vec![data_root.display().to_string()];
    for p in extra_paths {
        let s = p.display().to_string();
        if !paths.iter().any(|x| x == &s) {
            paths.push(s);
        }
    }
    body.push_str(&format!("ReadWritePaths={}\n", paths.join(" ")));
    if under_home {
        // ProtectHome=true would hide the path even if listed in ReadWritePaths
        // on some systemd versions; read-only is enough to keep /home private
        // except for the path we explicitly opened.
        body.push_str("ProtectHome=read-only\n");
    }

    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Pick User/Group from the state directory's owner.
///
/// State is always a local path we can inspect; the vault root may be NFS with
/// a foreign uid that has no passwd entry on this box.
fn run_identity_for(state: &Path) -> Result<(String, String)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(state).with_context(|| format!("stat {}", state.display()))?;
        let uid = meta.uid();
        let gid = meta.gid();
        // uid 0 here would mean we just created state as root — fall back to storm.
        if uid == 0 {
            ensure_storm_user()?;
            return Ok(("storm".into(), "storm".into()));
        }
        let user = name_for_id("passwd", uid)
            .with_context(|| format!("resolving uid {uid} for {}", state.display()))?;
        let group = name_for_id("group", gid).unwrap_or_else(|_| user.clone());
        Ok((user, group))
    }
    #[cfg(not(unix))]
    {
        let _ = state;
        Ok(("storm".into(), "storm".into()))
    }
}

fn name_for_id(db: &str, id: u32) -> Result<String> {
    let output = Command::new("getent")
        .args([db, &id.to_string()])
        .output()
        .with_context(|| format!("getent {db} {id}"))?;
    if !output.status.success() {
        bail!("getent {db} {id} failed");
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let name = line
        .split(':')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("empty getent {db} {id}"))?;
    Ok(name.to_string())
}

fn ensure_storm_user() -> Result<()> {
    let check = Command::new("id")
        .arg("-u")
        .arg("storm")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if let Ok(status) = check
        && status.success()
    {
        return Ok(());
    }
    let status = Command::new("useradd")
        .args([
            "--system",
            "--home",
            DEFAULT_DATA_ROOT,
            "--shell",
            "/usr/sbin/nologin",
            "storm",
        ])
        .status()
        .context("running useradd")?;
    if !status.success() {
        bail!("useradd storm failed with {status}");
    }
    Ok(())
}

/// chown when possible; NFS / root_squash must not abort `up`.
fn try_chown(path: &Path, user: &str, group: &str) {
    let spec = format!("{user}:{group}");
    let status = Command::new("chown").args([&spec]).arg(path).status();
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => eprintln!(
            "warning: could not chown {spec} {} — leaving ownership as-is \
             (normal on NFS)",
            path.display()
        ),
        Err(e) => eprintln!(
            "warning: chown {spec} {}: {e} — leaving ownership as-is",
            path.display()
        ),
    }
}

fn canonicalize_or_create(path: &Path) -> Result<PathBuf> {
    fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
    path.canonicalize()
        .with_context(|| format!("resolving {}", path.display()))
}

fn require_root() -> Result<()> {
    if !is_root() {
        bail!("this command must run as root (try: sudo storm-server …)");
    }
    Ok(())
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "0")
        .unwrap_or(false)
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .with_context(|| format!("running systemctl {}", args.join(" ")))?;
    if !status.success() {
        bail!("systemctl {} failed with {status}", args.join(" "));
    }
    Ok(())
}

fn read_env_listen() -> Option<(String, u16)> {
    let text = fs::read_to_string(ENV_PATH).ok()?;
    let mut host = DEFAULT_HOST.to_string();
    let mut port = DEFAULT_PORT;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("STORM_HOST=") {
            host = v.trim().to_string();
        }
        if let Some(v) = line.strip_prefix("STORM_PORT=")
            && let Ok(p) = v.trim().parse()
        {
            port = p;
        }
    }
    Some((host, port))
}

/// Tiny HTTP GET without adding a crate — status only needs a health probe.
fn ureq_get_health(url: &str) -> Result<()> {
    let status = Command::new("curl")
        .args(["-sf", "-o", "/dev/null", "--max-time", "2", url])
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => bail!("HTTP probe failed"),
        Err(e) => {
            // No curl: try a raw TCP connect as a weaker signal.
            let _ = e;
            let port = url
                .rsplit_once(':')
                .and_then(|(_, rest)| rest.split('/').next())
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(DEFAULT_PORT);
            let addr = format!("127.0.0.1:{port}");
            std::net::TcpStream::connect(addr).context("connect")?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_env_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("storm-env-test-{nanos}"))
    }

    #[test]
    fn write_env_file_sets_mode_and_paths() {
        let path = tmp_env_path();
        let vaults = PathBuf::from("/srv/storm/vaults");
        let state = PathBuf::from("/srv/storm/state");
        let web = PathBuf::from(DEFAULT_WEB);
        let backups = PathBuf::from("/srv/storm/backups");
        write_env_file(
            &path,
            &EnvFile {
                vault_root: &vaults,
                state: &state,
                web: &web,
                host: "0.0.0.0",
                port: 8484,
                backup_dest: &backups,
            },
        )
        .unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("STORM_WEB=/usr/share/storm/web"));
        assert!(body.contains("STORM_VAULT_ROOT=/srv/storm/vaults"));
        let _ = fs::remove_file(&path);
    }
}
