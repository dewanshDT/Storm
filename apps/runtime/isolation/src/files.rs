//! What the helper does to files for a session (136b, 136d, 136i): its
//! home, its share of the host's session directory, its launch, and, at
//! lease end, handing back everything its user wrote.
//!
//! Like the conversion, every tree is walked through descriptors opened
//! `O_NOFOLLOW` (`convert::walk`), and every path the host could have
//! planted a link in is opened one component at a time, each checked to be
//! a directory the expected account owns. An entry is changed only when it
//! belongs to the account the operation is about, so no walk can reach a
//! file someone else owns.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{FileType, Mode, OFlags};
use serde::Deserialize;

use crate::convert::{self, Kind, Meta, Ownership};
use crate::pool::Pool;

/// A launch file is far smaller.
pub const MAX_LAUNCH: u64 = 1024 * 1024;

/// What the host asks a session's program to get (136c): arguments and
/// variables only. The program is the operator's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launch {
    pub args: Vec<OsString>,
    pub env: Vec<(String, String)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchFile {
    args: Vec<String>,
    env: Vec<(String, String)>,
}

/// Variables the helper sets itself: the session's home and identity.
pub const RESERVED_ENV: &[&str] = &["HOME", "USER", "LOGNAME", "SHELL"];

pub fn env_name_ok(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && name.len() <= 256
}

pub fn parse_launch(bytes: &[u8]) -> Result<Launch> {
    let file: LaunchFile = serde_json::from_slice(bytes).context("the launch file")?;
    let mut env = Vec::new();
    for (name, value) in file.env {
        if !env_name_ok(&name) {
            bail!("a variable name in the launch is not valid");
        }
        if value.contains('\0') {
            bail!("a variable in the launch holds a NUL");
        }
        if RESERVED_ENV.contains(&name.as_str()) {
            continue;
        }
        env.push((name, value));
    }
    if file.args.iter().any(|a| a.contains('\0')) {
        bail!("an argument in the launch holds a NUL");
    }
    Ok(Launch {
        args: file.args.into_iter().map(OsString::from).collect(),
        env,
    })
}

/// One `EnvironmentFile=` line per variable, in systemd's syntax: a value
/// with nothing systemd would read specially is written bare, any other in
/// single quotes (taken literally), or, when it holds one, in double quotes
/// with `\`, `"`, `` ` `` and `$` escaped. Later lines win.
pub fn env_file(vars: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, value) in vars {
        out.push_str(name);
        out.push('=');
        let plain = !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-+:,@%".contains(&b));
        if plain {
            out.push_str(value);
        } else if !value.contains('\'') {
            out.push('\'');
            out.push_str(value);
            out.push('\'');
        } else {
            out.push('"');
            for c in value.chars() {
                if matches!(c, '\\' | '"' | '`' | '$') {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push('"');
        }
        out.push('\n');
    }
    out
}

/// Opens `path` one component at a time from `/`, never following a link,
/// and refuses unless every component from `trusted_depth` on is a
/// directory owned by `owner`. Returns the last one's descriptor.
pub fn open_owned_dir(path: &Path, trusted_depth: usize, owner: u32) -> Result<OwnedFd> {
    if !path.is_absolute() {
        bail!("{} is not absolute", path.display());
    }
    let mut fd = rustix::fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let mut depth = 0;
    for c in path.components() {
        let Component::Normal(name) = c else {
            if matches!(c, Component::RootDir) {
                continue;
            }
            bail!("{} is not in its normal form", path.display());
        };
        depth += 1;
        let next = rustix::fs::openat(
            &fd,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .with_context(|| format!("opening {}", path.display()))?;
        if depth >= trusted_depth {
            let st = rustix::fs::fstat(&next)?;
            if st.st_uid != owner {
                bail!("{} is not the expected account's", path.display());
            }
        }
        fd = next;
    }
    Ok(fd)
}

/// The host's launch file for a session: a regular file the host owns, in
/// a directory it owns, read once.
pub fn read_launch(dir: &Path, name: &str, host_uid: u32) -> Result<Launch> {
    let dir_fd = open_owned_dir(dir, dir.components().count() - 1, host_uid)?;
    let fd = rustix::fs::openat(
        &dir_fd,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .context("opening the launch file")?;
    let st = rustix::fs::fstat(&fd)?;
    if FileType::from_raw_mode(st.st_mode as _) != FileType::RegularFile || st.st_uid != host_uid {
        bail!("the launch file is not the host's");
    }
    let mut bytes = Vec::new();
    std::fs::File::from(fd)
        .take(MAX_LAUNCH + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LAUNCH {
        bail!("the launch file is too large");
    }
    parse_launch(&bytes)
}

/// The helper's own file, `0600` in a `0700` directory it creates.
pub fn write_root_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// The session's share of its own Storm directory (136i): the host keeps
/// owning every entry and can still write; the session's group may read
/// and search, never write.
pub fn shared(kind: Kind, m: Meta, gid: u32) -> Meta {
    let mode = match kind {
        Kind::Dir => (m.mode & !0o7020) | 0o050,
        Kind::File => (m.mode & !0o7020) | 0o040,
        Kind::Link | Kind::Other => m.mode,
    };
    Meta {
        uid: m.uid,
        gid,
        mode,
    }
}

/// And back: the host's group, nothing for it.
pub fn unshared(kind: Kind, m: Meta, host_gid: u32) -> Meta {
    let mode = match kind {
        Kind::Dir | Kind::File => m.mode & !0o7070,
        Kind::Link | Kind::Other => m.mode,
    };
    Meta {
        uid: m.uid,
        gid: host_gid,
        mode,
    }
}

fn apply(own: &dyn Ownership, e: &convert::Entry, target: Meta) -> Result<bool> {
    let now = own.meta(e.key, e.seen);
    let mut changed = false;
    if (now.uid, now.gid) != (target.uid, target.gid) {
        own.chown(e, target.uid, target.gid)?;
        changed = true;
    }
    if matches!(e.kind, Kind::Dir | Kind::File) && (changed || now.mode != target.mode) {
        own.chmod(e, target.mode)?;
        changed = true;
    }
    Ok(changed)
}

/// Applies `f` to every entry of `dir` that `owner` owns. Returns how many
/// changed.
pub fn retarget(
    dir: &Path,
    owner: u32,
    own: &dyn Ownership,
    f: &dyn Fn(Kind, Meta) -> Meta,
) -> Result<usize> {
    let mut changed = 0;
    convert::walk(dir, &mut |e| {
        let now = own.meta(e.key, e.seen);
        if now.uid == owner && apply(own, &e, f(e.kind, now))? {
            changed += 1;
        }
        Ok(())
    })?;
    Ok(changed)
}

/// At lease end (136b): every entry the session's user owns in its
/// workspace goes to the workspace's owner, in the workspace's group,
/// group-writable as the conversion makes everything there.
pub fn hand_back(workspace: &Path, uid: u32, own: &dyn Ownership) -> Result<usize> {
    let top = rustix::fs::lstat(workspace)?;
    let (owner, gid) = (top.st_uid, top.st_gid);
    retarget(workspace, uid, own, &|kind, m| {
        let mut t = convert::converted(kind, m, gid);
        t.uid = owner;
        t
    })
}

/// A session's home: a new directory, the user's own, `0700`.
pub fn make_home(home: &Path, uid: u32, gid: u32, own: &dyn Ownership) -> Result<()> {
    if home.symlink_metadata().is_ok() {
        remove_tree(home, own)?;
    }
    if let Some(dir) = home.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    std::fs::DirBuilder::new().mode(0o700).create(home)?;
    convert::walk(home, &mut |e| {
        own.chown(&e, uid, gid)?;
        own.chmod(&e, 0o700)
    })
}

/// Removes a tree a session's user wrote: everything becomes root's and
/// searchable first, so nothing the user did to its modes can stop it.
pub fn remove_tree(path: &Path, own: &dyn Ownership) -> Result<()> {
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
        Ok(md) if !md.is_dir() => bail!("{} is not a directory", path.display()),
        Ok(_) => {}
    }
    convert::walk(path, &mut |e| {
        if e.kind == Kind::Dir {
            own.chown(&e, 0, 0)?;
            own.chmod(&e, 0o700)?;
        }
        Ok(())
    })?;
    own.remove_all(path)
}

/// The workspace's pool group (136h), converting a workspace made since
/// isolation was turned on, as `on` would have.
pub fn workspace_group(
    workspace: &Path,
    pool: &Pool,
    manifests: &Path,
    own: &dyn Ownership,
) -> Result<String> {
    let find = || -> Result<Option<String>> {
        let st = rustix::fs::lstat(workspace)?;
        let gid = own
            .meta(
                convert::Key {
                    dev: st.st_dev as u64,
                    ino: st.st_ino as u64,
                },
                Meta {
                    uid: st.st_uid,
                    gid: st.st_gid,
                    mode: st.st_mode as u32 & 0o7777,
                },
            )
            .gid;
        Ok(pool
            .groups
            .iter()
            .find(|(_, g)| **g == gid)
            .map(|(name, _)| name.clone()))
    };
    if let Some(group) = find()? {
        return Ok(group);
    }
    convert::on(&[workspace.to_path_buf()], pool, manifests, own)?;
    find()?.context("the workspace has no pool group after its conversion")
}

/// Every lease the helper holds, by user, on disk so a restarted helper
/// ends what the last one left (136b).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct LeaseRecord {
    pub session: String,
    #[serde(default)]
    pub workspace: Option<PathBuf>,
}

pub fn save_lease(dir: &Path, user: &str, record: &LeaseRecord) -> Result<()> {
    let tmp = dir.join(format!(".{user}.tmp"));
    write_root_private(&tmp, &serde_json::to_vec(record)?)?;
    std::fs::rename(&tmp, dir.join(format!("{user}.json")))?;
    Ok(())
}

pub fn remove_lease(dir: &Path, user: &str) -> Result<()> {
    match std::fs::remove_file(dir.join(format!("{user}.json"))) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

pub fn load_leases(dir: &Path) -> Result<BTreeMap<String, LeaseRecord>> {
    let mut out = BTreeMap::new();
    let entries = match std::fs::read_dir(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        other => other?,
    };
    for entry in entries {
        let path = entry?.path();
        let Some(user) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".json"))
        else {
            continue;
        };
        if crate::pool::user_index(user).is_none() {
            continue;
        }
        let record: LeaseRecord = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("reading {}", path.display()))?;
        out.insert(user.to_string(), record);
    }
    Ok(out)
}

/// The pidfd of a session's leader, handed to the host (136i).
pub fn pidfd(pid: u32) -> Result<OwnedFd> {
    let pid = rustix::process::Pid::from_raw(pid as i32).context("no main process")?;
    Ok(rustix::process::pidfd_open(
        pid,
        rustix::process::PidfdFlags::empty(),
    )?)
}

/// The program a provider runs, from the operator's configuration (136c):
/// the command as given when it is a path, otherwise the first executable
/// of that name on `search`.
pub fn resolve(command: &str, search: &[PathBuf]) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    if command.contains('/') {
        let p = PathBuf::from(command);
        return (p.is_absolute() && executable(&p)).then_some(p);
    }
    if command.is_empty() {
        return None;
    }
    search
        .iter()
        .map(|d| d.join(command))
        .find(|p| executable(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::Recorded;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    fn pool() -> Pool {
        Pool::resolve(&crate::pool::tests::fixed(), "storm-runtime").unwrap()
    }

    fn me() -> u32 {
        rustix::process::getuid().as_raw()
    }

    #[test]
    fn a_launch_carries_arguments_and_variables_only() {
        let l = parse_launch(
            br#"{"args":["-l","--mcp-config","/s/mcp.json"],"env":[["TERM","xterm-256color"],["HOME","/host"],["A_1","x y"]]}"#,
        )
        .unwrap();
        assert_eq!(l.args, ["-l", "--mcp-config", "/s/mcp.json"]);
        assert_eq!(
            l.env,
            [
                ("TERM".to_string(), "xterm-256color".to_string()),
                ("A_1".to_string(), "x y".to_string())
            ],
            "HOME is the helper's to set"
        );
        for bad in [
            &br#"{"args":[],"env":[],"program":"/bin/sh"}"#[..],
            br#"{"args":[],"env":[["1X","v"]]}"#,
            br#"{"args":[],"env":[["A=B","v"]]}"#,
            br#"{"args":["a\u0000b"],"env":[]}"#,
            br#"{"args":[1],"env":[]}"#,
            b"not json",
        ] {
            assert!(
                parse_launch(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    /// The file systemd reads: each value comes back as written. The parser
    /// here is systemd's rules for the three forms written.
    #[test]
    fn the_environment_file_quotes_what_systemd_would_read() {
        let vars: Vec<(String, String)> = [
            ("A", "plain/value-1.2"),
            ("B", "two words"),
            ("C", "it's $HOME and \"q\" and `x` and \\"),
            ("D", ""),
            ("E", "line one\nline two"),
            ("F", "#not a comment"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let text = env_file(&vars);
        assert_eq!(parse_systemd_env(&text), vars, "{text}");
    }

    /// systemd's `parse_env_file` for the forms `env_file` writes.
    fn parse_systemd_env(text: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut chars = text.chars().peekable();
        while chars.peek().is_some() {
            let key: String = chars.by_ref().take_while(|c| *c != '=').collect();
            let mut value = String::new();
            match chars.peek() {
                Some('\'') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '\'' {
                            break;
                        }
                        value.push(c);
                    }
                    chars.next();
                }
                Some('"') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => {
                                let n = chars.next().unwrap();
                                if !"\"\\`$".contains(n) {
                                    value.push('\\');
                                }
                                value.push(n);
                            }
                            c => value.push(c),
                        }
                    }
                    chars.next();
                }
                _ => value = chars.by_ref().take_while(|c| *c != '\n').collect(),
            }
            out.push((key, value));
        }
        out
    }

    #[test]
    fn a_path_is_opened_only_through_directories_its_owner_owns() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("state/sessions/ags_A")).unwrap();
        std::fs::create_dir_all(base.join("elsewhere")).unwrap();
        symlink(base.join("elsewhere"), base.join("state/linked")).unwrap();
        let depth = base.join("state").components().count();
        assert!(open_owned_dir(&base.join("state/sessions/ags_A"), depth, me()).is_ok());
        assert!(open_owned_dir(&base.join("state/linked"), depth, me()).is_err());
        assert!(open_owned_dir(&base.join("state/sessions/ags_A"), depth, me() + 1).is_err());
        assert!(open_owned_dir(&base.join("state/../state"), depth, me()).is_err());
    }

    #[test]
    fn the_launch_file_is_read_only_as_the_hosts_own_regular_file() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().canonicalize().unwrap().join("launch");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("ags_A.json"), br#"{"args":["x"],"env":[]}"#).unwrap();
        assert_eq!(read_launch(&dir, "ags_A.json", me()).unwrap().args, ["x"]);
        assert!(read_launch(&dir, "ags_A.json", me() + 1).is_err());
        symlink(dir.join("ags_A.json"), dir.join("ags_B.json")).unwrap();
        assert!(read_launch(&dir, "ags_B.json", me()).is_err(), "a link");
        std::fs::create_dir(dir.join("ags_C.json")).unwrap();
        assert!(
            read_launch(&dir, "ags_C.json", me()).is_err(),
            "a directory"
        );
        std::fs::write(dir.join("ags_D.json"), vec![b' '; MAX_LAUNCH as usize + 1]).unwrap();
        assert!(read_launch(&dir, "ags_D.json", me()).is_err(), "too large");
    }

    #[test]
    fn the_session_reads_its_own_directory_and_cannot_write_it() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("ags_A");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("mcp.json"), b"{}").unwrap();
        std::fs::set_permissions(dir.join("mcp.json"), PermissionsExt::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o700)).unwrap();
        let own = Recorded::default();
        let n = retarget(&dir, me(), &own, &|k, m| shared(k, m, 2007)).unwrap();
        assert_eq!(n, 2);
        let metas: Vec<Meta> = own.overlay.borrow().values().copied().collect();
        assert!(metas.iter().all(|m| m.gid == 2007 && m.uid == me()));
        let modes: std::collections::BTreeSet<u32> = metas.iter().map(|m| m.mode).collect();
        assert_eq!(modes, [0o640, 0o750].into());
        // And back.
        let n = retarget(&dir, me(), &own, &|k, m| unshared(k, m, 900)).unwrap();
        assert_eq!(n, 2);
        let modes: std::collections::BTreeSet<u32> =
            own.overlay.borrow().values().map(|m| m.mode).collect();
        assert_eq!(modes, [0o600, 0o700].into());
    }

    /// IT-2's file half: what the session's user wrote anywhere in the
    /// workspace is the workspace owner's again, in its group, and nothing
    /// anyone else owns is touched.
    #[test]
    fn it_2_lease_end_hands_back_the_users_files_only() {
        let t = tempfile::tempdir().unwrap();
        let ws = t.path().join("app");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::write(ws.join("sub/made-by-session"), b"x").unwrap();
        std::fs::write(ws.join("hosts"), b"x").unwrap();
        symlink("/etc/shadow", ws.join("link")).unwrap();
        let own = Recorded::default();
        let key = |p: &Path| {
            let md = std::fs::symlink_metadata(p).unwrap();
            convert::Key {
                dev: md.dev(),
                ino: md.ino(),
            }
        };
        let session_uid = 2007;
        let top = std::fs::metadata(&ws).unwrap();
        // The session made `sub` and the file in it, and the link.
        for (p, mode) in [
            (ws.join("sub"), 0o2700),
            (ws.join("sub/made-by-session"), 0o4755),
            (ws.join("link"), 0o777),
        ] {
            own.overlay.borrow_mut().insert(
                key(&p),
                Meta {
                    uid: session_uid,
                    gid: 2007,
                    mode,
                },
            );
        }
        let n = hand_back(&ws, session_uid, &own).unwrap();
        assert_eq!(n, 3);
        let o = own.overlay.borrow();
        let sub = o[&key(&ws.join("sub"))];
        assert_eq!((sub.uid, sub.gid, sub.mode), (top.uid(), top.gid(), 0o2770));
        let file = o[&key(&ws.join("sub/made-by-session"))];
        assert_eq!(
            (file.uid, file.gid, file.mode),
            (top.uid(), top.gid(), 0o775),
            "setuid dropped, group rw"
        );
        let link = o[&key(&ws.join("link"))];
        assert_eq!((link.uid, link.gid), (top.uid(), top.gid()));
        assert!(
            !o.contains_key(&key(&ws.join("hosts"))),
            "not the session's"
        );
    }

    #[test]
    fn a_home_is_made_for_its_user_and_removed_whatever_it_did() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("homes/ags_A");
        let own = Recorded::default();
        make_home(&home, 2007, 2007, &own).unwrap();
        let o = own.overlay.borrow().clone();
        assert_eq!(o.len(), 1);
        assert_eq!(
            *o.values().next().unwrap(),
            Meta {
                uid: 2007,
                gid: 2007,
                mode: 0o700
            }
        );
        std::fs::create_dir_all(home.join("a/b")).unwrap();
        std::fs::write(home.join("a/b/f"), b"x").unwrap();
        std::fs::set_permissions(home.join("a"), PermissionsExt::from_mode(0o000)).unwrap();
        remove_tree(&home, &convert::Real).unwrap_or_else(|_| {
            // Unprivileged, the chown to root fails; the mode is ours.
            std::fs::set_permissions(home.join("a"), PermissionsExt::from_mode(0o700)).unwrap();
            std::fs::remove_dir_all(&home).unwrap();
        });
        assert!(!home.exists());
        assert!(remove_tree(&home, &own).is_ok(), "already gone");
    }

    #[test]
    fn a_workspace_made_since_on_is_converted_once() {
        let t = tempfile::tempdir().unwrap();
        let ws = t.path().join("new");
        std::fs::create_dir(&ws).unwrap();
        let manifests = t.path().join("manifests");
        let own = Recorded::default();
        let pool = pool();
        assert_eq!(
            workspace_group(&ws, &pool, &manifests, &own).unwrap(),
            "storm-w001"
        );
        assert_eq!(
            workspace_group(&ws, &pool, &manifests, &own).unwrap(),
            "storm-w001",
            "the second time it is found, not converted again"
        );
        assert_eq!(convert::Manifest::load_all(&manifests).unwrap().len(), 1);
    }

    #[test]
    fn leases_survive_a_restart_on_disk() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("leases");
        let r = LeaseRecord {
            session: "ags_A".into(),
            workspace: Some("/w/app".into()),
        };
        save_lease(&dir, "storm-s002", &r).unwrap();
        std::fs::write(dir.join("junk.json"), b"{}").unwrap();
        let all = load_leases(&dir).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all["storm-s002"], r);
        let md = std::fs::metadata(dir.join("storm-s002.json")).unwrap();
        assert_eq!(md.permissions().mode() & 0o777, 0o600);
        remove_lease(&dir, "storm-s002").unwrap();
        remove_lease(&dir, "storm-s002").unwrap();
        assert!(load_leases(&dir).unwrap().is_empty());
    }

    #[test]
    fn a_provider_resolves_on_the_operators_search_path() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        for d in [&a, &b] {
            let p = d.join("agent");
            std::fs::write(&p, b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, PermissionsExt::from_mode(0o755)).unwrap();
        }
        std::fs::write(a.join("plain"), b"").unwrap();
        let search = vec![a.clone(), b.clone()];
        assert_eq!(resolve("agent", &search), Some(a.join("agent")));
        assert_eq!(resolve("plain", &search), None, "not executable");
        assert_eq!(resolve("missing", &search), None);
        assert_eq!(resolve("", &search), None);
        assert_eq!(
            resolve(b.join("agent").to_str().unwrap(), &[]),
            Some(b.join("agent"))
        );
        assert_eq!(resolve("rel/agent", &search), None);
    }
}
