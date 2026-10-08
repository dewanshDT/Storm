//! `storm-runtime install` and `uninstall` on macOS (D14, AM33–AM35, AM39).
//!
//! Root only, and idempotent: every step checks what is there before it acts,
//! so re-running `install` is how a host is upgraded. Every external command
//! is called by absolute path, because this runs as root with the invoking
//! user's `PATH`.

use std::os::unix::fs::{DirBuilderExt, PermissionsExt, chown};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::launchd::{LABEL, Layout, NO_LOGIN_SHELL, default_config, plist};
use super::{InstallOptions, SERVICE_ACCOUNT, UninstallOptions};

const DSCL: &str = "/usr/bin/dscl";
const LAUNCHCTL: &str = "/bin/launchctl";
const ID: &str = "/usr/bin/id";
const DSEDITGROUP: &str = "/usr/sbin/dseditgroup";
const LS: &str = "/bin/ls";
const CHMOD: &str = "/bin/chmod";

/// macOS's `admin` group: the operator reads the logs without `sudo`.
const ADMIN_GID: u32 = 80;

/// What a workspace grant allows, on the root and, inherited, on everything
/// created under it (AM39). Directory rights map onto the same bits for files
/// (list = read, add_file = write, search = execute). No `delete` on the root
/// itself: removing an entry needs `delete_child` on its parent, which this
/// grants everywhere below the root, but the root itself stays.
const WORKSPACE_RIGHTS: &str = "list,add_file,search,add_subdirectory,delete_child,\
readattr,writeattr,readextattr,writeextattr,readsecurity,file_inherit,directory_inherit";

fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn require_root(what: &str) -> Result<()> {
    if !rustix::process::geteuid().is_root() {
        bail!("{what} changes system services and accounts: run it with sudo");
    }
    Ok(())
}

/// The ids already taken in a directory-service list: `dscl . -list
/// /Users UniqueID` prints "name id" lines.
fn taken_ids(path: &str, key: &str) -> Result<Vec<u32>> {
    Ok(run(DSCL, &[".", "-list", path, key])?
        .lines()
        .filter_map(|l| l.split_whitespace().last()?.parse().ok())
        .collect())
}

fn numeric(program: &str, args: &[&str]) -> Result<u32> {
    run(program, args)?
        .trim()
        .parse()
        .with_context(|| format!("{program} {} did not print a number", args.join(" ")))
}

/// Creates the hidden `_stormruntime` account and group, or brings an
/// existing one back to AM34's shape. Returns its uid and gid.
fn ensure_account(layout: &Layout) -> Result<(u32, u32)> {
    let user = format!("/Users/{SERVICE_ACCOUNT}");
    let group = format!("/Groups/{SERVICE_ACCOUNT}");
    let user_exists = succeeds(DSCL, &[".", "-read", &user]);
    let group_exists = succeeds(DSCL, &[".", "-read", &group]);

    let gid = if group_exists {
        // `dscl -read` prints "PrimaryGroupID: 123".
        run(DSCL, &[".", "-read", &group, "PrimaryGroupID"])?
            .split_whitespace()
            .last()
            .and_then(|s| s.parse().ok())
            .context("reading the _stormruntime group's PrimaryGroupID")?
    } else {
        let (uids, gids) = (
            taken_ids("/Users", "UniqueID")?,
            taken_ids("/Groups", "PrimaryGroupID")?,
        );
        let id = (200..=400)
            .find(|n| !uids.contains(n) && !gids.contains(n))
            .context("no free id between 200 and 400 for the _stormruntime account")?;
        run(DSCL, &[".", "-create", &group])?;
        run(
            DSCL,
            &[".", "-create", &group, "PrimaryGroupID", &id.to_string()],
        )?;
        run(
            DSCL,
            &[".", "-create", &group, "RealName", "Storm Runtime Host"],
        )?;
        run(DSCL, &[".", "-create", &group, "Password", "*"])?;
        id
    };

    let uid = if user_exists {
        numeric(ID, &["-u", SERVICE_ACCOUNT])?
    } else {
        let uids = taken_ids("/Users", "UniqueID")?;
        let uid = if uids.contains(&gid) {
            (200..=400)
                .find(|n| !uids.contains(n))
                .context("no free uid between 200 and 400 for the _stormruntime account")?
        } else {
            gid
        };
        run(DSCL, &[".", "-create", &user])?;
        run(DSCL, &[".", "-create", &user, "UniqueID", &uid.to_string()])?;
        uid
    };

    // Re-asserted every time, so a hand-edited account is brought back.
    let home = layout.home.display().to_string();
    for (key, value) in [
        ("PrimaryGroupID", gid.to_string().as_str()),
        ("UserShell", NO_LOGIN_SHELL),
        ("NFSHomeDirectory", home.as_str()),
        ("RealName", "Storm Runtime Host"),
        ("IsHidden", "1"),
        ("Password", "*"),
    ] {
        run(DSCL, &[".", "-create", &user, key, value])?;
    }
    for privileged in ["admin", "wheel"] {
        if succeeds(
            DSEDITGROUP,
            &["-o", "checkmember", "-m", SERVICE_ACCOUNT, privileged],
        ) {
            bail!(
                "{SERVICE_ACCOUNT} is a member of {privileged}; remove it \
                 (sudo dseditgroup -o edit -d {SERVICE_ACCOUNT} -t user {privileged}) \
                 and run install again"
            );
        }
    }
    Ok((uid, gid))
}

/// A directory with exactly this owner and mode. An existing symlink is
/// refused rather than followed.
fn dir(path: &Path, uid: u32, gid: u32, mode: u32) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            bail!("{} exists and is not a directory", path.display())
        }
        Ok(_) => {}
        Err(_) => std::fs::DirBuilder::new()
            .mode(mode)
            .create(path)
            .with_context(|| format!("creating {}", path.display()))?,
    }
    chown(path, Some(uid), Some(gid)).with_context(|| format!("owning {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting the mode of {}", path.display()))?;
    Ok(())
}

/// Writes `bytes` to `path` atomically, owned and moded before it appears.
fn write_file(path: &Path, bytes: &[u8], uid: u32, gid: u32, mode: u32) -> Result<()> {
    let tmp = path.with_extension("storm-tmp");
    let _ = std::fs::remove_file(&tmp);
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    chown(&tmp, Some(uid), Some(gid))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, path).with_context(|| format!("installing {}", path.display()))?;
    Ok(())
}

/// Gives `who` the workspace rights on `root`, inherited by everything
/// created under it. Skipped when the entry is already there.
fn grant(root: &Path, who: &str) -> Result<()> {
    let root = root.display().to_string();
    let listing = run(LS, &["-led", &root])?;
    if listing
        .lines()
        .any(|l| l.contains(&format!("user:{who} allow")))
    {
        return Ok(());
    }
    run(
        CHMOD,
        &["+a", &format!("user:{who} allow {WORKSPACE_RIGHTS}"), &root],
    )?;
    Ok(())
}

/// Who the workspace root is shared with: `--operator`, else the user who
/// ran `sudo`. Never root and never the host's own account.
fn operator(options: &InstallOptions) -> Result<Option<String>> {
    let name = options
        .operator
        .clone()
        .or_else(|| std::env::var("SUDO_USER").ok())
        .filter(|n| !n.is_empty());
    let Some(name) = name else {
        return Ok(None);
    };
    if name == "root" || name == SERVICE_ACCOUNT {
        bail!("the operator must be a person's account, not {name}; pass --operator <user>");
    }
    if !succeeds(ID, &["-u", &name]) {
        bail!("no such user: {name}");
    }
    Ok(Some(name))
}

fn loaded() -> bool {
    succeeds(LAUNCHCTL, &["print", &format!("system/{LABEL}")])
}

/// Boots the job out and back in, so it picks up a new plist and binary.
/// launchd starts it at once if the host is enrolled (`PathState`).
fn reload(layout: &Layout) -> Result<()> {
    let target = format!("system/{LABEL}");
    if loaded() {
        // SIGTERM: the host ends its sessions in order (AM37).
        let _ = run(LAUNCHCTL, &["bootout", &target]);
    }
    let _ = run(LAUNCHCTL, &["enable", &target]);
    let plist = layout.plist.display().to_string();
    // A bootout returns before the job has fully gone; bootstrap answers
    // "Input/output error" until it has.
    let mut last = None;
    for _ in 0..20 {
        match run(LAUNCHCTL, &["bootstrap", "system", &plist]) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(last.unwrap()).context("loading the LaunchDaemon")
}

fn install_binary(layout: &Layout) -> Result<()> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("finding this binary")?;
    if layout.bin.canonicalize().ok().as_deref() == Some(exe.as_path()) {
        return Ok(());
    }
    let bytes = std::fs::read(&exe).with_context(|| format!("reading {}", exe.display()))?;
    write_file(&layout.bin, &bytes, 0, 0, 0o755)
}

pub fn install(options: &InstallOptions) -> Result<()> {
    require_root("install")?;
    let layout = Layout::standard();
    let operator = operator(options)?;
    let (uid, gid) = ensure_account(&layout)?;

    dir(&layout.root, 0, 0, 0o755)?;
    dir(layout.bin.parent().expect("bin has a parent"), 0, 0, 0o755)?;
    dir(&layout.state, uid, gid, 0o700)?;
    dir(&layout.home, uid, gid, 0o700)?;
    dir(&layout.workspaces, uid, gid, 0o750)?;
    grant(&layout.workspaces, SERVICE_ACCOUNT)?;
    if let Some(op) = &operator {
        grant(&layout.workspaces, op)?;
    }
    dir(&layout.logs, uid, ADMIN_GID, 0o750)?;

    install_binary(&layout)?;
    if !layout.config.exists() {
        write_file(
            &layout.config,
            default_config(&layout).as_bytes(),
            0,
            0,
            0o644,
        )?;
    }
    write_file(
        &layout.plist,
        plist(&layout, SERVICE_ACCOUNT).as_bytes(),
        0,
        0,
        0o644,
    )?;
    reload(&layout)?;

    let enrolled = layout.enrolled_marker().exists();
    println!("Installed the Storm Runtime Host.");
    println!("  service    : system/{LABEL} (launchd), as {SERVICE_ACCOUNT}");
    println!("  binary     : {}", layout.bin.display());
    println!("  config     : {}", layout.config.display());
    println!("  workspaces : {}", layout.workspaces.display());
    match &operator {
        Some(op) => {
            println!("               shared with {op}: you and the host can both read and write it")
        }
        None => {
            println!("               not shared with anyone: pass --operator <user> to share it")
        }
    }
    println!("  logs       : {}", layout.log_file.display());
    if enrolled {
        println!("\nThis host is enrolled; launchd restarted it with this binary.");
    } else {
        println!("\nNext, enroll it. In the Storm app: Agents > Hosts > Enroll a host, then run:");
        println!(
            "  sudo -u {SERVICE_ACCOUNT} {} enroll",
            layout.bin.display()
        );
        println!("launchd starts the host as soon as it is enrolled.");
    }
    println!("\nLog the agent CLIs in as the host's account, once:");
    println!("  cd / && sudo -u {SERVICE_ACCOUNT} -H claude");
    Ok(())
}

pub fn uninstall(options: &UninstallOptions) -> Result<()> {
    require_root("uninstall")?;
    let layout = Layout::standard();
    if loaded() {
        run(LAUNCHCTL, &["bootout", &format!("system/{LABEL}")])?;
    }
    let remove = |p: &Path| match std::fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", p.display()))
        }
        _ => Ok(()),
    };
    remove(&layout.plist)?;
    remove(&layout.bin)?;
    let _ = std::fs::remove_dir(layout.bin.parent().expect("bin has a parent"));
    println!("Removed the LaunchDaemon and the binary.");

    if options.purge {
        for d in [&layout.state, &layout.logs] {
            match std::fs::remove_dir_all(d) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(e).with_context(|| format!("removing {}", d.display()));
                }
                _ => {}
            }
        }
        remove(&layout.config)?;
        for path in [
            format!("/Users/{SERVICE_ACCOUNT}"),
            format!("/Groups/{SERVICE_ACCOUNT}"),
        ] {
            if succeeds(DSCL, &[".", "-read", &path]) {
                run(DSCL, &[".", "-delete", &path])?;
            }
        }
        let _ = std::fs::remove_dir(&layout.root);
        println!(
            "Purged the host's identity, state, logs, config and the {SERVICE_ACCOUNT} account."
        );
    } else {
        println!(
            "Kept the identity, state, logs, config and the {SERVICE_ACCOUNT} account; \
             `sudo storm-runtime uninstall --purge` removes them."
        );
    }
    if layout.workspaces.exists() {
        println!(
            "Kept the workspaces at {}; nothing in them was touched.",
            layout.workspaces.display()
        );
    }
    Ok(())
}
