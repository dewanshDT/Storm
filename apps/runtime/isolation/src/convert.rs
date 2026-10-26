//! The workspace conversion (136a, 136h). Turning isolation on gives each
//! workspace a pool group: every entry in it is group-owned by that group,
//! group-writable, and its directories inherit the group. Turning it off
//! puts back exactly what was recorded before the first change.
//!
//! This runs as root over trees that agents wrote, so it never follows a
//! symlink and never trusts a name twice: every directory is opened
//! relative to its parent with `O_NOFOLLOW`, every opened entry is checked
//! to be the inode just looked at, and changes are made through the open
//! descriptor (or, for a symlink, with `AT_SYMLINK_NOFOLLOW`). It never
//! crosses into another filesystem. Content is never read or written.
//!
//! Every change goes through [`Ownership`], so the logic runs unprivileged
//! in tests with the changes recorded instead of made.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, OsStr};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};
use serde::{Deserialize, Serialize};

use crate::pool::{self, Pool};

const MAX_DEPTH: usize = 256;
const S_ISUID: u32 = 0o4000;
const S_ISGID: u32 = 0o2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "d")]
    Dir,
    #[serde(rename = "f")]
    File,
    #[serde(rename = "l")]
    Link,
    /// A FIFO, socket or device: its owner changes, never its mode.
    #[serde(rename = "o")]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(rename = "u")]
    pub uid: u32,
    #[serde(rename = "g")]
    pub gid: u32,
    /// Permission bits with setuid, setgid and sticky (`0o7777`).
    #[serde(rename = "m")]
    pub mode: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub dev: u64,
    pub ino: u64,
}

pub enum Node<'a> {
    /// An entry opened and verified: a directory or a regular file.
    Fd(BorrowedFd<'a>),
    /// A symlink or special file, by name in its directory.
    At(BorrowedFd<'a>, &'a CStr),
}

pub trait Ownership {
    /// What the entry is now; `seen` is what `stat` reported.
    fn meta(&self, key: Key, seen: Meta) -> Meta;
    fn chown(&self, e: &Entry, uid: u32, gid: u32) -> Result<()>;
    fn chmod(&self, e: &Entry, mode: u32) -> Result<()>;
    /// What an opened entry is now, asked of its own descriptor.
    fn stat(&self, e: &Entry) -> Result<Meta>;
    /// Removes a tree whose directories are already root's.
    fn remove_all(&self, path: &Path) -> Result<()>;
}

/// The real thing, for root.
pub struct Real;

impl Ownership for Real {
    fn meta(&self, _: Key, seen: Meta) -> Meta {
        seen
    }

    fn chown(&self, e: &Entry, uid: u32, gid: u32) -> Result<()> {
        let (u, g) = (
            Some(rustix::fs::Uid::from_raw(uid)),
            Some(rustix::fs::Gid::from_raw(gid)),
        );
        match &e.node {
            Node::Fd(fd) => rustix::fs::fchown(fd, u, g)?,
            Node::At(dir, name) => {
                rustix::fs::chownat(dir, *name, u, g, AtFlags::SYMLINK_NOFOLLOW)?
            }
        }
        Ok(())
    }

    fn chmod(&self, e: &Entry, mode: u32) -> Result<()> {
        match &e.node {
            Node::Fd(fd) => rustix::fs::fchmod(fd, Mode::from_raw_mode(mode))?,
            Node::At(..) => bail!("never chmod by name"),
        }
        Ok(())
    }

    fn stat(&self, e: &Entry) -> Result<Meta> {
        match &e.node {
            Node::Fd(fd) => Ok(meta_of(&rustix::fs::fstat(fd)?)),
            Node::At(..) => bail!("never stat by name"),
        }
    }

    fn remove_all(&self, path: &Path) -> Result<()> {
        std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
    }
}

/// Records changes instead of making them; `fail_after` makes the n-th
/// change fail, as an interrupted run would.
#[derive(Default)]
pub struct Recorded {
    pub overlay: RefCell<HashMap<Key, Meta>>,
    pub changes: RefCell<usize>,
    pub fail_after: Option<usize>,
    pub removed: RefCell<Vec<PathBuf>>,
    /// Entries whose changes are taken and never made, as a filesystem
    /// that ignores them would.
    pub frozen: RefCell<Vec<Key>>,
}

impl Recorded {
    fn change(&self, key: Key, seen: Meta, f: impl FnOnce(&mut Meta)) -> Result<()> {
        let mut n = self.changes.borrow_mut();
        if self.fail_after.is_some_and(|max| *n >= max) {
            bail!("injected failure");
        }
        *n += 1;
        if self.frozen.borrow().contains(&key) {
            return Ok(());
        }
        let mut overlay = self.overlay.borrow_mut();
        let meta = overlay.entry(key).or_insert(seen);
        f(meta);
        Ok(())
    }
}

impl Ownership for Recorded {
    fn meta(&self, key: Key, seen: Meta) -> Meta {
        self.overlay.borrow().get(&key).copied().unwrap_or(seen)
    }
    fn chown(&self, e: &Entry, uid: u32, gid: u32) -> Result<()> {
        self.change(e.key, e.seen, |m| {
            m.uid = uid;
            m.gid = gid;
        })
    }
    fn chmod(&self, e: &Entry, mode: u32) -> Result<()> {
        self.change(e.key, e.seen, |m| m.mode = mode)
    }
    fn stat(&self, e: &Entry) -> Result<Meta> {
        Ok(self.meta(e.key, e.seen))
    }
    fn remove_all(&self, path: &Path) -> Result<()> {
        self.removed.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

pub(crate) fn kind_of(st: &rustix::fs::Stat) -> Kind {
    match FileType::from_raw_mode(st.st_mode as _) {
        FileType::Directory => Kind::Dir,
        FileType::RegularFile => Kind::File,
        FileType::Symlink => Kind::Link,
        _ => Kind::Other,
    }
}

// `Stat`'s field types differ between architectures.
#[allow(clippy::unnecessary_cast)]
pub(crate) fn key_of(st: &rustix::fs::Stat) -> Key {
    Key {
        dev: st.st_dev as u64,
        ino: st.st_ino as u64,
    }
}

#[allow(clippy::unnecessary_cast)]
pub(crate) fn meta_of(st: &rustix::fs::Stat) -> Meta {
    Meta {
        uid: st.st_uid,
        gid: st.st_gid,
        mode: st.st_mode as u32 & 0o7777,
    }
}

pub struct Entry<'a> {
    /// Relative to the workspace, `/`-joined bytes; empty for the workspace.
    pub rel: &'a [u8],
    pub kind: Kind,
    pub key: Key,
    pub seen: Meta,
    pub node: Node<'a>,
}

type Visit<'f> = &'f mut dyn FnMut(Entry) -> Result<()>;

/// Visits the workspace and everything under it, parents first. A tree
/// deeper than the cap is an error.
pub fn walk(workspace: &Path, visit: Visit) -> Result<()> {
    walk_from(workspace, visit, None).map(drop)
}

/// As [`walk`], but a directory at the cap is handed to `at_cap` (through
/// its own descriptor, after `visit` saw it) instead of being an error, and
/// nothing under it is visited. Returns those directories (139-D1).
pub fn walk_capped(workspace: &Path, visit: Visit, at_cap: Visit) -> Result<Vec<PathBuf>> {
    walk_from(workspace, visit, Some(at_cap))
}

fn walk_from(workspace: &Path, visit: Visit, at_cap: Option<Visit>) -> Result<Vec<PathBuf>> {
    let fd = rustix::fs::open(workspace, crate::files::DIR_FLAGS, Mode::empty())
        .with_context(|| format!("opening {}", workspace.display()))?;
    let st = rustix::fs::fstat(&fd)?;
    let dev = st.st_dev as u64;
    visit(Entry {
        rel: b"",
        kind: Kind::Dir,
        key: key_of(&st),
        seen: meta_of(&st),
        node: Node::Fd(fd.as_fd()),
    })?;
    let mut cap = Cap {
        at_cap,
        found: Vec::new(),
    };
    walk_dir(workspace, &fd, &mut Vec::new(), dev, 0, visit, &mut cap)?;
    Ok(cap.found)
}

struct Cap<'f> {
    at_cap: Option<Visit<'f>>,
    found: Vec<PathBuf>,
}

fn walk_dir(
    workspace: &Path,
    dir: &OwnedFd,
    rel: &mut Vec<u8>,
    dev: u64,
    depth: usize,
    visit: &mut dyn FnMut(Entry) -> Result<()>,
    cap: &mut Cap,
) -> Result<()> {
    if depth >= MAX_DEPTH {
        let Some(at_cap) = cap.at_cap.as_mut() else {
            bail!(
                "{} is nested deeper than {MAX_DEPTH} directories",
                workspace.display()
            );
        };
        let st = rustix::fs::fstat(dir)?;
        at_cap(Entry {
            rel,
            kind: Kind::Dir,
            key: key_of(&st),
            seen: meta_of(&st),
            node: Node::Fd(dir.as_fd()),
        })?;
        cap.found.push(workspace.join(OsStr::from_bytes(rel)));
        return Ok(());
    }
    for name in crate::files::dir_names(dir)? {
        let base = rel.len();
        if !rel.is_empty() {
            rel.push(b'/');
        }
        rel.extend_from_slice(name.to_bytes());
        let shown = || workspace.join(OsStr::from_bytes(rel));
        let st = match rustix::fs::statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) => st,
            // Gone since it was listed: nothing to convert.
            Err(rustix::io::Errno::NOENT) => {
                rel.truncate(base);
                continue;
            }
            Err(e) => return Err(e).with_context(|| format!("stat {}", shown().display())),
        };
        let kind = kind_of(&st);
        if st.st_dev as u64 != dev {
            bail!(
                "{} is on another filesystem; unmount it, or move it out of the workspace",
                shown().display()
            );
        }
        match kind {
            Kind::Dir | Kind::File => {
                let mut flags =
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NOCTTY;
                flags |= if kind == Kind::Dir {
                    OFlags::DIRECTORY
                } else {
                    OFlags::NONBLOCK
                };
                let fd = rustix::fs::openat(dir, name.as_c_str(), flags, Mode::empty())
                    .with_context(|| format!("opening {}", shown().display()))?;
                let now = rustix::fs::fstat(&fd)?;
                if key_of(&now) != key_of(&st) {
                    bail!("{} changed while it was being converted", shown().display());
                }
                visit(Entry {
                    rel,
                    kind,
                    key: key_of(&now),
                    seen: meta_of(&now),
                    node: Node::Fd(fd.as_fd()),
                })?;
                if kind == Kind::Dir {
                    walk_dir(workspace, &fd, rel, dev, depth + 1, visit, cap)?;
                }
            }
            Kind::Link | Kind::Other => visit(Entry {
                rel,
                kind,
                key: key_of(&st),
                seen: meta_of(&st),
                node: Node::At(dir.as_fd(), name.as_c_str()),
            })?,
        }
        rel.truncate(base);
    }
    Ok(())
}

/// What an entry becomes in a workspace with group `gid`.
pub fn converted(kind: Kind, m: Meta, gid: u32) -> Meta {
    let mode = match kind {
        Kind::Dir => (m.mode & !S_ISUID) | S_ISGID | 0o070,
        // Setuid and setgid are dropped: a group-executable setuid file
        // would let a session run as its owner.
        Kind::File => {
            let mode = (m.mode & !(S_ISUID | S_ISGID)) | 0o060;
            if mode & 0o100 != 0 {
                mode | 0o010
            } else {
                mode
            }
        }
        Kind::Link | Kind::Other => m.mode,
    };
    Meta {
        uid: m.uid,
        gid,
        mode,
    }
}

/// What an entry created while isolation was on becomes when it is turned
/// off: a pool user's file is handed back to the host account, a pool
/// group goes back to the host's group, and the group loses write, as the
/// host's `UMask=0027` would have made it.
pub fn restored_new(kind: Kind, m: Meta, pool: &Pool) -> Meta {
    let uid = if pool.is_pool_uid(m.uid) {
        pool.host_uid
    } else {
        m.uid
    };
    let gid = if pool.is_pool_gid(m.gid) {
        pool.host_gid
    } else {
        m.gid
    };
    let mode = match kind {
        Kind::Dir => m.mode & !(S_ISGID | 0o020),
        Kind::File => m.mode & !(S_ISUID | S_ISGID | 0o020),
        Kind::Link | Kind::Other => m.mode,
    };
    Meta { uid, gid, mode }
}

/// Seals a directory the walk could not go below (139-D1): `uid`:`gid`,
/// `0700`, through its own descriptor, then checked on that descriptor.
/// Anything else than exactly that is an error.
pub fn seal(own: &dyn Ownership, e: &Entry, shown: &Path, uid: u32, gid: u32) -> Result<()> {
    own.chown(e, uid, gid)?;
    own.chmod(e, 0o700)?;
    let now = own.stat(e)?;
    if (now.uid, now.gid, now.mode) != (uid, gid, 0o700) {
        bail!(
            "{} did not seal: {}:{} {:o}",
            shown.display(),
            now.uid,
            now.gid,
            now.mode
        );
    }
    // Debug-quoted: a name a session chose cannot forge another line.
    eprintln!("audit: sealed path={shown:?} uid={uid} gid={gid} mode=0700");
    Ok(())
}

/// [`walk_capped`], sealing each directory at the cap to `uid`:`gid`.
pub fn walk_sealing(
    path: &Path,
    uid: u32,
    gid: u32,
    own: &dyn Ownership,
    visit: Visit,
) -> Result<Vec<PathBuf>> {
    walk_capped(path, visit, &mut |e| {
        seal(own, &e, &path.join(OsStr::from_bytes(e.rel)), uid, gid)
    })
}

pub(crate) fn apply(own: &dyn Ownership, e: &Entry, target: Meta) -> Result<bool> {
    let now = own.meta(e.key, e.seen);
    let mut changed = false;
    if (now.uid, now.gid) != (target.uid, target.gid) {
        own.chown(e, target.uid, target.gid)?;
        changed = true;
    }
    // After the chown, which may clear setuid and setgid.
    if matches!(e.kind, Kind::Dir | Kind::File) && (changed || now.mode != target.mode) {
        own.chmod(e, target.mode)?;
        changed = true;
    }
    Ok(changed)
}

/// One workspace's record: its group, and every entry as it was before
/// isolation first touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub workspace: PathBuf,
    pub group: String,
    pub entries: BTreeMap<Vec<u8>, (Kind, Meta)>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    workspace: String,
    group: String,
}

#[derive(Serialize, Deserialize)]
struct Line {
    p: String,
    k: Kind,
    #[serde(flatten)]
    m: Meta,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        bail!("bad hex");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("bad hex"))
        .collect()
}

impl Manifest {
    fn path(dir: &Path, group: &str) -> PathBuf {
        dir.join(format!("{group}.jsonl"))
    }

    pub fn load_all(dir: &Path) -> Result<Vec<Manifest>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e).with_context(|| format!("listing {}", dir.display())),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension() != Some(OsStr::new("jsonl")) {
                continue;
            }
            out.push(Self::load(&path)?);
        }
        out.sort_by(|a, b| a.group.cmp(&b.group));
        Ok(out)
    }

    fn load(path: &Path) -> Result<Manifest> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut lines = text.lines();
        let header: Header = serde_json::from_str(lines.next().unwrap_or(""))
            .with_context(|| format!("{}: no header", path.display()))?;
        if pool::group_index(&header.group).is_none() && header.group != pool::INSTALL_ACCOUNT {
            bail!("{}: {} is not a pool group", path.display(), header.group);
        }
        let mut entries = BTreeMap::new();
        for line in lines {
            let l: Line = serde_json::from_str(line)
                .with_context(|| format!("{}: bad line", path.display()))?;
            entries.insert(unhex(&l.p)?, (l.k, l.m));
        }
        Ok(Manifest {
            workspace: PathBuf::from(OsStr::from_bytes(&unhex(&header.workspace)?)),
            group: header.group,
            entries,
        })
    }

    /// Written whole, synced, then renamed into place: a crash leaves the
    /// old record or the new one, never half of one.
    pub(crate) fn save(&self, dir: &Path) -> Result<()> {
        let path = Self::path(dir, &self.group);
        let mut out = String::new();
        out.push_str(&serde_json::to_string(&Header {
            workspace: hex(self.workspace.as_os_str().as_bytes()),
            group: self.group.clone(),
        })?);
        out.push('\n');
        for (rel, (k, m)) in &self.entries {
            out.push_str(&serde_json::to_string(&Line {
                p: hex(rel),
                k: *k,
                m: *m,
            })?);
            out.push('\n');
        }
        crate::files::write_root_private_atomic(&path, out.as_bytes())
    }

    pub(crate) fn remove(&self, dir: &Path) -> Result<()> {
        std::fs::remove_file(Self::path(dir, &self.group))?;
        Ok(())
    }
}

/// The workspaces' records; the prefix's (`agents.rs`) is never one.
fn workspace_manifests(dir: &Path) -> Result<Vec<Manifest>> {
    let mut all = Manifest::load_all(dir)?;
    all.retain(|m| pool::group_index(&m.group).is_some());
    Ok(all)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub workspaces: usize,
    pub changed: usize,
    pub missing: Vec<PathBuf>,
    /// Directories past the walk's cap, sealed to the host account.
    pub sealed: Vec<PathBuf>,
}

/// Held across load, choice, save and apply in `on` and `off`: a session's
/// start, a child's and the operator's CLI each convert, and two unlocked
/// conversions could give two workspaces one group and one manifest.
fn lock(manifests: &Path) -> Result<OwnedFd> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(manifests)?;
    let path = manifests.join(".lock");
    let fd = rustix::fs::open(
        &path,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .with_context(|| format!("opening {}", path.display()))?;
    rustix::fs::flock(&fd, FlockOperation::LockExclusive)
        .with_context(|| format!("locking {}", path.display()))?;
    Ok(fd)
}

#[cfg(test)]
thread_local! {
    /// Widens the window between choosing groups and saving them, so a
    /// test can race two conversions.
    pub(crate) static PAUSE_AFTER_CHOICE: std::cell::Cell<std::time::Duration> =
        const { std::cell::Cell::new(std::time::Duration::ZERO) };
}

/// Turns isolation on for `workspaces`. Groups are assigned before
/// anything is written, every manifest is complete before anything is
/// changed, and a rerun after an interruption finishes the job without
/// losing what was first recorded.
pub fn on(
    workspaces: &[PathBuf],
    pool: &Pool,
    manifests: &Path,
    own: &dyn Ownership,
) -> Result<Report> {
    let _lock = lock(manifests)?;
    let mut existing = workspace_manifests(manifests)?;
    let mut used: Vec<String> = existing.iter().map(|m| m.group.clone()).collect();
    let mut plan: Vec<Manifest> = Vec::new();
    for ws in workspaces {
        if let Some(i) = existing.iter().position(|m| &m.workspace == ws) {
            plan.push(existing.swap_remove(i));
            continue;
        }
        let Some(group) = pool.groups.keys().find(|g| !used.contains(g)).cloned() else {
            bail!(
                "this host has more workspaces than the pool has groups ({}); nothing was changed",
                pool.groups.len()
            );
        };
        used.push(group.clone());
        plan.push(Manifest {
            workspace: ws.clone(),
            group,
            entries: BTreeMap::new(),
        });
    }
    #[cfg(test)]
    std::thread::sleep(PAUSE_AFTER_CHOICE.get());
    // A recorded workspace that has gone keeps its record (and its group)
    // until isolation is turned off.
    for m in &mut plan {
        walk(&m.workspace, &mut |e| {
            m.entries
                .entry(e.rel.to_vec())
                .or_insert((e.kind, own.meta(e.key, e.seen)));
            Ok(())
        })?;
        m.save(manifests)?;
    }
    let mut report = Report {
        workspaces: plan.len(),
        ..Report::default()
    };
    for m in &plan {
        let gid = pool.groups[&m.group];
        walk(&m.workspace, &mut |e| {
            let target = converted(e.kind, own.meta(e.key, e.seen), gid);
            if apply(own, &e, target)? {
                report.changed += 1;
            }
            Ok(())
        })?;
    }
    Ok(report)
}

/// Turns isolation off: every recorded entry gets back exactly its
/// recorded owner, group and mode; anything created since is handed back
/// to the host account. A manifest is removed only once its workspace is
/// restored, so an interrupted run can be repeated.
pub fn off(pool: &Pool, manifests: &Path, own: &dyn Ownership) -> Result<Report> {
    let _lock = lock(manifests)?;
    let mut report = Report::default();
    for m in workspace_manifests(manifests)? {
        match std::fs::symlink_metadata(&m.workspace) {
            Ok(md) if md.is_dir() => {}
            _ => {
                report.missing.push(m.workspace.clone());
                m.remove(manifests)?;
                continue;
            }
        }
        report.workspaces += 1;
        let mut changed = 0;
        let sealed = walk_sealing(&m.workspace, pool.host_uid, pool.host_gid, own, &mut |e| {
            let now = own.meta(e.key, e.seen);
            let target = match m.entries.get(e.rel) {
                Some((kind, recorded)) if *kind == e.kind => *recorded,
                _ => restored_new(e.kind, now, pool),
            };
            if apply(own, &e, target)? {
                changed += 1;
            }
            Ok(())
        })?;
        report.changed += changed;
        report.sealed.extend(sealed);
        m.remove(manifests)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::tests::pool;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    struct Fixture {
        _t: tempfile::TempDir,
        root: PathBuf,
        outside: PathBuf,
        manifests: PathBuf,
    }

    fn fixture() -> Fixture {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let root = base.join("workspaces");
        let outside = base.join("outside");
        std::fs::create_dir_all(outside.join("dir")).unwrap();
        std::fs::write(outside.join("secret"), b"not yours").unwrap();
        let a = root.join("app");
        std::fs::create_dir_all(a.join("src/deep/er")).unwrap();
        std::fs::write(a.join("README"), b"hello").unwrap();
        std::fs::write(a.join("src/main.rs"), b"fn main() {}").unwrap();
        std::fs::write(a.join("src/deep/er/x"), b"x").unwrap();
        std::fs::write(a.join("run.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(a.join("run.sh"), PermissionsExt::from_mode(0o750)).unwrap();
        std::fs::write(a.join("setuid"), b"").unwrap();
        std::fs::set_permissions(a.join("setuid"), PermissionsExt::from_mode(0o4750)).unwrap();
        std::fs::write(a.join("private"), b"p").unwrap();
        std::fs::set_permissions(a.join("private"), PermissionsExt::from_mode(0o600)).unwrap();
        symlink(outside.join("secret"), a.join("to-secret")).unwrap();
        symlink(outside.join("dir"), a.join("to-dir")).unwrap();
        std::fs::hard_link(a.join("README"), a.join("README.hard")).unwrap();
        std::fs::write(a.join(OsStr::from_bytes(b"caf\xe9")), b"latin-1").unwrap();
        let fifo = std::ffi::CString::new(a.join("pipe").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o640) }, 0);
        let b = root.join("lib");
        std::fs::create_dir_all(b.join("x")).unwrap();
        std::fs::write(b.join("x/y"), b"y").unwrap();
        std::fs::create_dir_all(root.join(".trash")).unwrap();
        Fixture {
            manifests: base.join("state/manifests"),
            root,
            outside,
            _t: t,
        }
    }

    impl Fixture {
        fn workspaces(&self) -> Vec<PathBuf> {
            crate::config::workspaces(std::slice::from_ref(&self.root)).unwrap()
        }
    }

    /// Every entry's kind and content (a file's bytes, a link's target).
    fn contents(dir: &Path) -> BTreeMap<PathBuf, (Kind, Vec<u8>)> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                let md = std::fs::symlink_metadata(&p).unwrap();
                let entry = if md.file_type().is_symlink() {
                    (
                        Kind::Link,
                        std::fs::read_link(&p).unwrap().into_os_string().into_vec(),
                    )
                } else if md.is_dir() {
                    stack.push(p.clone());
                    (Kind::Dir, Vec::new())
                } else if md.is_file() {
                    (Kind::File, std::fs::read(&p).unwrap())
                } else {
                    (Kind::Other, Vec::new())
                };
                out.insert(p, entry);
            }
        }
        out
    }

    /// What `own` says each entry of `dir` is, by path.
    fn metas(dir: &Path, own: &dyn Ownership) -> BTreeMap<Vec<u8>, Meta> {
        let mut out = BTreeMap::new();
        walk(dir, &mut |e| {
            out.insert(e.rel.to_vec(), own.meta(e.key, e.seen));
            Ok(())
        })
        .unwrap();
        out
    }

    fn key(p: &Path) -> Key {
        let md = std::fs::symlink_metadata(p).unwrap();
        Key {
            dev: md.dev(),
            ino: md.ino(),
        }
    }

    use std::os::unix::ffi::OsStringExt;

    /// G-136-5 at unit level: on then off leaves every workspace's content
    /// and every entry's owner, group and mode as they were.
    #[test]
    fn on_then_off_leaves_every_workspace_intact() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        let before = contents(&f.root);
        let ws = f.workspaces();
        let metas_before: Vec<_> = ws.iter().map(|w| metas(w, &own)).collect();

        let report = on(&ws, &pool, &f.manifests, &own).unwrap();
        assert_eq!(report.workspaces, 2);
        assert!(report.changed > 0);
        assert_eq!(contents(&f.root), before, "content untouched by on");

        let report = off(&pool, &f.manifests, &own).unwrap();
        assert_eq!(report.workspaces, 2);
        assert_eq!(contents(&f.root), before, "content untouched by off");
        let metas_after: Vec<_> = ws.iter().map(|w| metas(w, &own)).collect();
        assert_eq!(metas_after, metas_before);
        assert!(Manifest::load_all(&f.manifests).unwrap().is_empty());
    }

    #[test]
    fn on_gives_each_workspace_its_own_inheriting_group() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        let app = metas(&f.root.join("app"), &own);
        let lib = metas(&f.root.join("lib"), &own);
        assert!(app.values().all(|m| m.gid == 3001), "{app:?}");
        assert!(lib.values().all(|m| m.gid == 3002), "{lib:?}");
        let uid = std::fs::metadata(&f.root).unwrap().uid();
        assert!(app.values().all(|m| m.uid == uid), "owners are kept");
        assert_eq!(
            app[&b""[..].to_vec()].mode & 0o2070,
            0o2070,
            "setgid, group rwx"
        );
        assert_eq!(app[&b"src/deep"[..].to_vec()].mode & 0o2070, 0o2070);
        assert_eq!(app[&b"run.sh"[..].to_vec()].mode, 0o770);
        assert_eq!(app[&b"private"[..].to_vec()].mode, 0o660);
        assert_eq!(app[&b"setuid"[..].to_vec()].mode, 0o770, "setuid dropped");
        assert_eq!(
            app[&b"pipe"[..].to_vec()].mode,
            0o640,
            "a FIFO's mode is never changed"
        );
        let manifests = Manifest::load_all(&f.manifests).unwrap();
        assert_eq!(manifests.len(), 2);
        assert_eq!(manifests[0].entries[&b"caf\xe9"[..].to_vec()].0, Kind::File);
    }

    #[test]
    fn symlinks_and_what_they_point_at_are_never_followed() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        let overlay = own.overlay.borrow();
        for p in [
            f.outside.join("secret"),
            f.outside.join("dir"),
            f.outside.clone(),
        ] {
            assert!(
                !overlay.contains_key(&key(&p)),
                "{} was touched",
                p.display()
            );
        }
        let link = overlay[&key(&f.root.join("app/to-dir"))];
        assert_eq!(link.gid, 3001, "the link itself gets the group");
        assert!(
            !overlay.contains_key(&key(&f.root.join(".trash"))),
            "not a workspace"
        );
        assert!(
            !overlay.contains_key(&key(&f.root)),
            "the root itself is left alone"
        );
    }

    #[test]
    fn on_is_idempotent_and_keeps_the_first_record() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        let first = Manifest::load_all(&f.manifests).unwrap();
        let again = on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        assert_eq!(again.changed, 0);
        assert_eq!(Manifest::load_all(&f.manifests).unwrap(), first);
    }

    #[test]
    fn an_interrupted_on_finishes_and_still_restores_the_original() {
        let f = fixture();
        let pool = pool();
        let ws = f.workspaces();
        let mut own = Recorded {
            fail_after: Some(5),
            ..Recorded::default()
        };
        let original: Vec<_> = ws.iter().map(|w| metas(w, &own)).collect();
        assert!(on(&ws, &pool, &f.manifests, &own).is_err());
        own.fail_after = None;
        on(&ws, &pool, &f.manifests, &own).unwrap();
        off(&pool, &f.manifests, &own).unwrap();
        let after: Vec<_> = ws.iter().map(|w| metas(w, &own)).collect();
        assert_eq!(after, original);
    }

    #[test]
    fn off_hands_back_what_sessions_created() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        // A session's user left a directory and a file behind, as the
        // workspace's group made them.
        let made = f.root.join("app/made");
        std::fs::create_dir(&made).unwrap();
        std::fs::write(made.join("out"), b"o").unwrap();
        own.overlay.borrow_mut().insert(
            key(&made),
            Meta {
                uid: 2005,
                gid: 3001,
                mode: 0o2775,
            },
        );
        own.overlay.borrow_mut().insert(
            key(&made.join("out")),
            Meta {
                uid: 2005,
                gid: 3001,
                mode: 0o664,
            },
        );
        // A file the host account itself wrote stays its own.
        let mine = f.root.join("app/mine");
        std::fs::write(&mine, b"m").unwrap();
        let me = std::fs::metadata(&mine).unwrap();
        own.overlay.borrow_mut().insert(
            key(&mine),
            Meta {
                uid: me.uid(),
                gid: 3001,
                mode: 0o660,
            },
        );
        off(&pool, &f.manifests, &own).unwrap();
        let o = own.overlay.borrow();
        assert_eq!(
            o[&key(&made)],
            Meta {
                uid: 900,
                gid: 900,
                mode: 0o755
            }
        );
        assert_eq!(
            o[&key(&made.join("out"))],
            Meta {
                uid: 900,
                gid: 900,
                mode: 0o644
            }
        );
        assert_eq!(
            o[&key(&mine)],
            Meta {
                uid: me.uid(),
                gid: 900,
                mode: 0o640
            }
        );
    }

    #[test]
    fn more_workspaces_than_groups_changes_nothing() {
        let f = fixture();
        for n in 0..63 {
            std::fs::create_dir(f.root.join(format!("ws{n:02}"))).unwrap();
        }
        let own = Recorded::default();
        let err = on(&f.workspaces(), &pool(), &f.manifests, &own).unwrap_err();
        assert!(err.to_string().contains("more workspaces"), "{err}");
        assert!(own.overlay.borrow().is_empty());
        assert!(Manifest::load_all(&f.manifests).unwrap().is_empty());
    }

    #[test]
    fn a_workspace_removed_while_on_is_forgotten_by_off() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        std::fs::remove_dir_all(f.root.join("lib")).unwrap();
        let report = off(&pool, &f.manifests, &own).unwrap();
        assert_eq!(report.missing, [f.root.join("lib")]);
        assert!(Manifest::load_all(&f.manifests).unwrap().is_empty());
    }

    #[test]
    fn a_new_workspace_gets_the_next_free_group_and_old_ones_keep_theirs() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        std::fs::create_dir(f.root.join("aaa")).unwrap();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        let groups: BTreeMap<_, _> = Manifest::load_all(&f.manifests)
            .unwrap()
            .into_iter()
            .map(|m| (m.workspace, m.group))
            .collect();
        assert_eq!(groups[&f.root.join("app")], "storm-w001");
        assert_eq!(groups[&f.root.join("lib")], "storm-w002");
        assert_eq!(groups[&f.root.join("aaa")], "storm-w003");
    }

    #[test]
    fn a_manifest_is_private_and_round_trips() {
        let f = fixture();
        on(&f.workspaces(), &pool(), &f.manifests, &Recorded::default()).unwrap();
        for e in std::fs::read_dir(&f.manifests).unwrap() {
            let md = e.unwrap().metadata().unwrap();
            assert_eq!(md.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn the_real_ownership_changes_modes_through_the_descriptor() {
        // Unprivileged: chown to one's own ids and chmod are both allowed.
        let f = fixture();
        let file = f.root.join("app/README");
        let md = std::fs::metadata(&file).unwrap();
        let secret_mode = std::fs::metadata(f.outside.join("secret"))
            .unwrap()
            .permissions()
            .mode();
        walk(&f.root.join("app"), &mut |e| {
            if e.rel == b"README" {
                Real.chown(&e, md.uid(), md.gid())?;
                Real.chmod(&e, 0o640)?;
            }
            if e.rel == b"to-secret" {
                Real.chown(&e, md.uid(), md.gid())?;
                assert!(Real.chmod(&e, 0o777).is_err(), "never by name");
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        assert_eq!(
            secret_mode,
            std::fs::metadata(f.outside.join("secret"))
                .unwrap()
                .permissions()
                .mode()
        );
    }

    /// 139-D1: what a session nested past the cap is sealed to the host
    /// account, `0700`, and the rest is turned off as usual.
    #[test]
    fn off_seals_what_a_session_nested_past_the_cap() {
        let f = fixture();
        let pool = pool();
        let own = Recorded::default();
        on(&f.workspaces(), &pool, &f.manifests, &own).unwrap();
        let app = f.root.join("app");
        let deep = app.join(std::iter::repeat_n("d", 300).collect::<PathBuf>());
        std::fs::create_dir_all(&deep).unwrap();
        let report = off(&pool, &f.manifests, &own).unwrap();
        let capped = app.join(std::iter::repeat_n("d", 256).collect::<PathBuf>());
        assert_eq!(
            own.overlay.borrow()[&key(&capped)],
            Meta {
                uid: 900,
                gid: 900,
                mode: 0o700
            }
        );
        assert_eq!(report.sealed, [capped]);
        assert_eq!(report.workspaces, 2);
        assert!(Manifest::load_all(&f.manifests).unwrap().is_empty());
    }

    /// 139-D1: past the cap, the strict walk (and so `on`) refuses; the
    /// capped one hands over the directory at the cap through its own
    /// descriptor and visits nothing under it.
    #[test]
    fn walk_reports_a_tree_past_the_cap() {
        let f = fixture();
        let app = f.root.join("app");
        let nest = |n| app.join(std::iter::repeat_n("d", n).collect::<PathBuf>());
        std::fs::create_dir_all(nest(300)).unwrap();
        assert!(walk(&app, &mut |_| Ok(())).is_err());
        let err = on(&f.workspaces(), &pool(), &f.manifests, &Recorded::default()).unwrap_err();
        assert!(err.to_string().contains("deeper than"), "{err}");
        let mut deepest = 0;
        let mut handed = Vec::new();
        let found = walk_capped(
            &app,
            &mut |e| {
                deepest = deepest.max(e.rel.split(|b| *b == b'/').count());
                Ok(())
            },
            &mut |e| {
                assert!(matches!(e.node, Node::Fd(_)));
                handed.push((e.rel.to_vec(), e.key));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(found, [nest(256)]);
        assert_eq!(handed.len(), 1);
        assert_eq!(handed[0].1, key(&nest(256)));
        assert_eq!(deepest, 256, "nothing under the cap is visited");
    }

    /// A seal is checked on the descriptor: one that does not hold is an
    /// error, never taken on trust.
    #[test]
    fn a_seal_that_does_not_hold_is_an_error() {
        let f = fixture();
        let app = f.root.join("app");
        let own = Recorded::default();
        own.frozen.borrow_mut().push(key(&app));
        let mut sealed = Ok(());
        walk(&app, &mut |e| {
            if e.rel.is_empty() {
                sealed = seal(&own, &e, &app, 900, 900);
            }
            Ok(())
        })
        .unwrap();
        assert!(sealed.is_err());
        own.frozen.borrow_mut().clear();
        walk(&app, &mut |e| {
            if e.rel.is_empty() {
                seal(&own, &e, &app, 900, 900)?;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            own.overlay.borrow()[&key(&app)],
            Meta {
                uid: 900,
                gid: 900,
                mode: 0o700
            }
        );
    }
}
