//! Child workspaces (decision 137): where a spawned session works, made by
//! the helper at approval (WS-4) and owned by the child's identity alone.
//!
//! They live under the children root, `<helper state>/children`, outside
//! every workspace root, so workspace discovery never finds one. Each child
//! has a root-owned directory holding its record and its `workspace/`,
//! which is the child's (`0700`) and which its unit shows at that same
//! path. A unit sees no other child's.
//!
//! - **W2 (137b)**, when the parent's workspace is a repository root: an
//!   independent clone of its `HEAD` commit, made by `clone.rs` running as
//!   the child's user in a unit of its own, with the parent's workspace
//!   shown read-only for that one operation and nothing the repository or
//!   an account's git configuration says allowed to run. The child's unit
//!   never sees the parent's workspace.
//! - **W3 (137c)**, otherwise (a linked worktree or a submodule checkout
//!   included): an empty scratch directory, and the parent's workspace
//!   read-only at its real path in the child's view.
//!
//! The caps (137f) are checked before anything is made: what the clone
//! would copy (the parent's `.git`, measured by the helper), and all the
//! children on the host. The clone itself stops before its checkout if the
//! checkout would cross the budget, and the helper measures what was made.
//! A creation that fails leaves nothing behind (137h). At the child's lease
//! end its workspace is kept, marked ended, and given to the review
//! identity (137g), which runs every later export and Apply in a unit of
//! its own (`review.rs`): Apply writes only `storm/<child>` (W2) or a new
//! directory (W3) in the parent's workspace, and never merges (137e). A
//! human deletes it (`child_dispose`), and directories no record claims are
//! listed as orphans, never removed unasked.

use std::ffi::{CStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{AtFlags, FileType, Mode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::files;
use crate::protocol::{Op, Refusal};
use crate::review::CHILD_ANSWER_BUDGET;
use crate::serve::Helper;
use crate::session::{Answer, failed};
use crate::units::{self, STOP_GRACE_SECS, property_safe};

/// The helper's own binary, which a clone unit runs as the child's user
/// (`child-clone`). Pinned against the package's install path.
pub const HELPER_BIN: &str = "/usr/sbin/storm-runtime-isolation";
/// A clone that takes longer than this is stopped and abandoned.
pub const CLONE_MAX_SECS: u32 = 600;
const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChildMode {
    /// W2: the child's own clone of the parent's repository.
    Clone,
    /// W3: the parent read-only, and a scratch directory.
    Scratch,
}

impl ChildMode {
    pub fn name(self) -> &'static str {
        match self {
            ChildMode::Clone => "clone",
            ChildMode::Scratch => "scratch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Being made: a helper that finds one at start abandons it.
    Creating,
    Ready,
    /// The child's lease ended; the workspace is kept (137g).
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildRecord {
    pub child: String,
    pub parent: String,
    pub parent_workspace: PathBuf,
    pub mode: ChildMode,
    pub stage: Stage,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub submodules: bool,
    #[serde(default)]
    pub large_files: bool,
    /// An Apply under way or made (137e): what it creates in the parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<Applied>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    pub target: String,
    /// False while it runs: a helper that finds it so after a restart lets
    /// the same Apply finish, and refuses any other.
    pub done: bool,
}

/// W2 when the workspace is a repository root: its `.git` is a directory
/// of its own (not a symlink, and not the file a linked worktree or a
/// submodule checkout has) with a `HEAD`. Anything else is W3 (137c).
pub fn parent_mode(workspace: &Path) -> ChildMode {
    let dot = workspace.join(".git");
    let is = |p: &Path, dir: bool| {
        p.symlink_metadata()
            .is_ok_and(|m| if dir { m.is_dir() } else { m.is_file() })
    };
    if is(&dot, true) && is(&dot.join("HEAD"), false) {
        ChildMode::Clone
    } else {
        ChildMode::Scratch
    }
}

/// What a tree takes on disk, as `du` counts it: every entry's blocks,
/// never following a symlink or leaving the filesystem. Nothing missing is
/// nothing. A tree past the depth cap is an error.
pub fn disk_usage(path: &Path) -> Result<u64> {
    match measure(path)? {
        (bytes, true) => Ok(bytes),
        (_, false) => bail!("nested deeper than {MAX_DEPTH} directories"),
    }
}

/// As [`disk_usage`], with `false` when part of the tree was past the depth
/// cap, so the count is short.
pub fn measure(path: &Path) -> Result<(u64, bool)> {
    let st = match rustix::fs::lstat(path) {
        Err(rustix::io::Errno::NOENT) => return Ok((0, true)),
        other => other.with_context(|| format!("stat {}", path.display()))?,
    };
    let own = blocks(&st);
    if FileType::from_raw_mode(st.st_mode as _) != FileType::Directory {
        return Ok((own, true));
    }
    let fd = rustix::fs::open(path, files::DIR_FLAGS, Mode::empty())
        .with_context(|| format!("opening {}", path.display()))?;
    let (under, complete) = usage_in(&fd, crate::convert::key_of(&st).dev, 0)?;
    Ok((own.saturating_add(under), complete))
}

#[allow(clippy::unnecessary_cast)]
fn blocks(st: &rustix::fs::Stat) -> u64 {
    st.st_blocks as u64 * 512
}

fn usage_in(dir: &OwnedFd, dev: u64, depth: usize) -> Result<(u64, bool)> {
    if depth >= MAX_DEPTH {
        return Ok((0, false));
    }
    let (mut total, mut complete) = (0u64, true);
    for name in files::dir_names(dir)? {
        let st = match rustix::fs::statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) => st,
            Err(rustix::io::Errno::NOENT) => continue,
            Err(e) => return Err(e.into()),
        };
        if crate::convert::key_of(&st).dev != dev {
            continue;
        }
        total = total.saturating_add(blocks(&st));
        if FileType::from_raw_mode(st.st_mode as _) == FileType::Directory
            && let Some(sub) = open_sub(dir, name.as_c_str(), &st)?
        {
            let (under, all) = usage_in(&sub, dev, depth + 1)?;
            total = total.saturating_add(under);
            complete &= all;
        }
    }
    Ok((total, complete))
}

/// The directory `statat` just saw, or `None` when it has gone or is no
/// longer that directory: a live child's tree changes under the scan.
fn open_sub(dir: &OwnedFd, name: &CStr, seen: &rustix::fs::Stat) -> Result<Option<OwnedFd>> {
    use rustix::io::Errno;
    let sub = match rustix::fs::openat(dir, name, files::DIR_FLAGS, Mode::empty()) {
        Ok(sub) => sub,
        Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let now = rustix::fs::fstat(&sub)?;
    Ok((crate::convert::key_of(&now) == crate::convert::key_of(seen)).then_some(sub))
}

/// The children root is outside every workspace root and contains none,
/// or a child could be found as a workspace, or hold one.
pub fn check_root(children: &Path, roots: &[PathBuf]) -> Result<()> {
    for root in roots {
        if children.starts_with(root) || root.starts_with(children) {
            bail!(
                "the workspace root {} overlaps the children root {}",
                root.display(),
                children.display()
            );
        }
    }
    Ok(())
}

pub fn clone_unit_name(child: &str) -> String {
    format!("storm-child-{child}-clone.service")
}

/// The unit a W2 child's clone runs in: the child's user, its parent's
/// workspace group (to read the parent's files) over a read-only view of
/// that one workspace, no network, no capability, and only the child's own
/// workspace writable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneSpec {
    pub child: String,
    pub user: String,
    pub parent_group: String,
    pub parent: PathBuf,
    pub dest: PathBuf,
    /// The host's state, the helper's and every workspace root.
    pub hidden: Vec<PathBuf>,
    pub budget: u64,
}

impl CloneSpec {
    pub fn properties(&self) -> Result<Vec<String>> {
        if let Some(bad) = [&self.parent, &self.dest]
            .into_iter()
            .chain(&self.hidden)
            .find(|p| !property_safe(p))
        {
            bail!("{} cannot be written into a unit property", bad.display());
        }
        let p = |s: &str| s.to_string();
        let mut out = vec![
            format!("Description=Storm child workspace {}", self.child),
            p("Type=exec"),
            format!("User={}", self.user),
            format!("Group={}", self.user),
            format!("SupplementaryGroups={}", self.parent_group),
            format!("WorkingDirectory={}", self.dest.display()),
            format!("Environment=HOME={}", crate::clone::HOME),
            p("UMask=0077"),
            format!("RuntimeMaxSec={CLONE_MAX_SECS}"),
            format!("TimeoutStopSec={STOP_GRACE_SECS}"),
            // No file it writes can be larger than the whole budget.
            format!("LimitFSIZE={}", self.budget),
        ];
        // A local clone needs no network: a repository that names a remote
        // reaches nothing.
        out.extend(sandbox());
        for hidden in &self.hidden {
            out.push(format!("TemporaryFileSystem={}:ro", hidden.display()));
        }
        out.push(format!("BindReadOnlyPaths={}", self.parent.display()));
        out.push(format!("BindPaths={}", self.dest.display()));
        Ok(out)
    }

    pub fn args(&self) -> Vec<OsString> {
        vec![
            HELPER_BIN.into(),
            "child-clone".into(),
            self.parent.clone().into_os_string(),
            self.dest.clone().into_os_string(),
            self.budget.to_string().into(),
        ]
    }

    pub fn systemd_run_args(&self) -> Result<Vec<OsString>> {
        let unit = clone_unit_name(&self.child);
        Ok(units::run_args(
            &unit,
            true,
            self.properties()?,
            self.args(),
        ))
    }

    pub fn unit_file(&self) -> Result<String> {
        let exec = units::command_line(&self.args());
        Ok(units::unit_file(self.properties()?, &exec))
    }
}

pub fn review_unit_name(child: &str) -> String {
    format!("storm-child-{child}-review.service")
}

/// The unit an export or an Apply runs in (137e, 137g): the review
/// identity, never a session's user, the host account or root; no network,
/// no capability; only the paths it reads shown, read-only, and the one it
/// writes. An Apply also has the parent workspace's group, to write there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSpec {
    pub child: String,
    pub parent_group: Option<String>,
    pub read_only: Vec<PathBuf>,
    pub writable: Vec<PathBuf>,
    pub hidden: Vec<PathBuf>,
    pub budget: u64,
    /// `child-review`'s own arguments (`review::main`).
    pub args: Vec<String>,
}

impl ReviewSpec {
    pub fn properties(&self) -> Result<Vec<String>> {
        if let Some(bad) = self
            .read_only
            .iter()
            .chain(&self.writable)
            .chain(&self.hidden)
            .find(|p| !property_safe(p))
        {
            bail!("{} cannot be written into a unit property", bad.display());
        }
        let user = crate::pool::REVIEW_ACCOUNT;
        let mut out = vec![
            format!("Description=Storm child workspace review {}", self.child),
            "Type=exec".to_string(),
            format!("User={user}"),
            format!("Group={user}"),
        ];
        if let Some(group) = &self.parent_group {
            out.push(format!("SupplementaryGroups={group}"));
        }
        out.extend([
            "WorkingDirectory=/".to_string(),
            format!("Environment=HOME={}", crate::clone::HOME),
            // What an Apply writes is the workspace group's to use, as the
            // conversion makes everything there; it is handed back after.
            if self.parent_group.is_some() {
                "UMask=0007".to_string()
            } else {
                "UMask=0077".to_string()
            },
            format!("RuntimeMaxSec={CLONE_MAX_SECS}"),
            format!("TimeoutStopSec={STOP_GRACE_SECS}"),
            format!("LimitFSIZE={}", self.budget),
        ]);
        out.extend(sandbox());
        for hidden in &self.hidden {
            out.push(format!("TemporaryFileSystem={}:ro", hidden.display()));
        }
        for p in &self.read_only {
            out.push(format!("BindReadOnlyPaths={}", p.display()));
        }
        for p in &self.writable {
            out.push(format!("BindPaths={}", p.display()));
        }
        Ok(out)
    }

    pub fn exec(&self) -> Vec<OsString> {
        let mut out: Vec<OsString> = vec![HELPER_BIN.into(), "child-review".into()];
        out.extend(self.args.iter().map(OsString::from));
        out
    }

    pub fn systemd_run_args(&self) -> Result<Vec<OsString>> {
        let unit = review_unit_name(&self.child);
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
}

/// What every unit the helper runs for a child's workspace has: no
/// capability, no network, nothing of the system writable.
fn sandbox() -> Vec<String> {
    [
        "NoNewPrivileges=yes",
        "CapabilityBoundingSet=",
        "AmbientCapabilities=",
        "RestrictSUIDSGID=yes",
        "ProtectSystem=strict",
        "ProtectHome=yes",
        "PrivateTmp=yes",
        "PrivateDevices=yes",
        "PrivateIPC=yes",
        "PrivateNetwork=yes",
        "RestrictAddressFamilies=AF_UNIX",
        "IPAddressDeny=any",
        "ProtectProc=invisible",
        "ProtectKernelTunables=yes",
        "ProtectKernelModules=yes",
        "ProtectKernelLogs=yes",
        "ProtectControlGroups=yes",
        "ProtectClock=yes",
        "ProtectHostname=yes",
        "LockPersonality=yes",
        "MemoryDenyWriteExecute=yes",
        "RestrictRealtime=yes",
        "RestrictNamespaces=yes",
        "KeyringMode=private",
        "SystemCallArchitectures=native",
    ]
    .map(String::from)
    .to_vec()
}

impl crate::session::Layout {
    pub fn children(&self) -> PathBuf {
        self.state.join("children")
    }
    pub fn child(&self, child: &str) -> PathBuf {
        self.children().join(child)
    }
    pub fn child_record(&self, child: &str) -> PathBuf {
        self.child(child).join("record.json")
    }
    /// The child's workspace, at this same path in its own view.
    pub fn child_workspace(&self, child: &str) -> PathBuf {
        self.child(child).join("workspace")
    }
    /// The review identity's own: where an export leaves its bundle.
    pub fn child_review(&self, child: &str) -> PathBuf {
        self.child(child).join("review")
    }
    /// What the last export found, the helper's.
    pub fn child_export(&self, child: &str) -> PathBuf {
        self.child(child).join("export.json")
    }
}

fn answer(r: &ChildRecord, workspace: &Path) -> Answer {
    (reply(r, workspace), None)
}

/// `child_create`'s answer: what the host learns of a child's workspace.
pub fn reply(r: &ChildRecord, workspace: &Path) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("mode".into(), r.mode.name().into());
    out.insert("workspace".into(), workspace.display().to_string().into());
    out.insert(
        "commit".into(),
        r.commit.clone().map_or(Value::Null, Value::from),
    );
    out.insert(
        "branch".into(),
        r.branch.clone().map_or(Value::Null, Value::from),
    );
    out.insert("submodules".into(), r.submodules.into());
    out.insert("large_files".into(), r.large_files.into());
    out
}

impl Helper {
    /// What the children root takes, one child at a time: a child that
    /// cannot be measured whole is charged at least its cap (139a), so a
    /// tree no one can finish counting fails closed for itself alone.
    fn children_usage(&self) -> Result<u64> {
        let root = self.layout.children();
        let mut total = match rustix::fs::lstat(&root) {
            Err(rustix::io::Errno::NOENT) => return Ok(0),
            other => blocks(&other?),
        };
        for entry in std::fs::read_dir(&root)? {
            let path = entry?.path();
            let charged = match measure(&path) {
                Ok((bytes, true)) => bytes,
                Ok((bytes, false)) => bytes.max(self.caps.child),
                Err(e) => {
                    eprintln!(
                        "storm-runtime-isolation: measuring {}: {e:#}",
                        path.display()
                    );
                    self.caps.child
                }
            };
            total = total.saturating_add(charged);
        }
        Ok(total)
    }

    pub(crate) fn child_record(&self, child: &str) -> Option<ChildRecord> {
        let bytes = std::fs::read(self.layout.child_record(child)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save_child(&self, r: &ChildRecord) -> Result<()> {
        files::write_root_private_atomic(
            &self.layout.child_record(&r.child),
            &serde_json::to_vec(r)?,
        )
    }

    /// `child_create {parent, child, workspace}`: the child's workspace,
    /// made for a child that holds a lease and has not started, from a
    /// parent that holds one in `workspace`. Asked again for the same
    /// parent and workspace, it answers what it made.
    pub(crate) fn child_create(
        &self,
        parent: &str,
        child: &str,
        workspace: &Path,
    ) -> Result<Answer, Refusal> {
        let user = {
            let st = self.state.lock().unwrap();
            let user = st.leases.user_of(child).ok_or(Refusal::NotLeased)?;
            if st.started.contains(child) {
                return Err(Refusal::AlreadyStarted);
            }
            let parent_in = st
                .leases
                .user_of(parent)
                .and_then(|u| st.records.get(u))
                .and_then(|r| r.workspace.as_deref());
            // A child's lease records no workspace, so a child is no one's
            // parent here: its workspace is its own, which only it may read.
            if parent == child || parent_in != Some(workspace) {
                return Err(Refusal::WrongParent);
            }
            user.to_string()
        };
        let _busy = self.session_busy(child)?;
        let dir = self.layout.child(child);
        match self.child_record(child) {
            Some(r)
                if r.stage == Stage::Ready
                    && r.parent == parent
                    && r.parent_workspace == workspace =>
            {
                return Ok(answer(&r, &self.layout.child_workspace(child)));
            }
            Some(r) if r.stage == Stage::Creating => self
                .abandon(child)
                .map_err(|e| failed("abandoning a child workspace", e))?,
            Some(_) => return Err(Refusal::ChildExists),
            // Not a record of this helper's: an orphan for a human.
            None if dir.symlink_metadata().is_ok() => return Err(Refusal::ChildExists),
            None => {}
        }
        let mode = parent_mode(workspace);
        let used = self
            .children_usage()
            .map_err(|e| failed("measuring the children", e))?;
        let estimate = match mode {
            ChildMode::Clone => match measure(&workspace.join(".git"))
                .map_err(|e| failed("measuring the parent's repository", e))?
            {
                (bytes, true) => bytes,
                (_, false) => return Err(Refusal::WorkspaceCap),
            },
            ChildMode::Scratch => 0,
        };
        if estimate > self.caps.child || used.saturating_add(estimate) > self.caps.children {
            return Err(Refusal::WorkspaceCap);
        }
        let budget = self.caps.child.min(self.caps.children - used);
        let mut record = ChildRecord {
            child: child.to_string(),
            parent: parent.to_string(),
            parent_workspace: workspace.to_path_buf(),
            mode,
            stage: Stage::Creating,
            commit: None,
            branch: None,
            submodules: false,
            large_files: false,
            applied: None,
        };
        let made = self.make_child(&mut record, &user, budget);
        if let Err(why) = made {
            if let Err(e) = self.abandon(child) {
                eprintln!("storm-runtime-isolation: abandoning {child}'s workspace: {e:#}");
            }
            return Err(why);
        }
        Ok(answer(&record, &self.layout.child_workspace(child)))
    }

    fn make_child(&self, record: &mut ChildRecord, user: &str, budget: u64) -> Result<(), Refusal> {
        let child = record.child.clone();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.layout.child(&child))
            .map_err(|e| failed("making the child's directory", e.into()))?;
        self.save_child(record)
            .map_err(|e| failed("recording the child", e))?;
        let (uid, gid) = (self.pool.users[user], self.pool.primary[user]);
        let dest = self.layout.child_workspace(&child);
        let own = self.deps.own.as_ref();
        files::make_home(&dest, uid, gid, own)
            .map_err(|e| failed("making the child's workspace", e))?;
        if record.mode == ChildMode::Clone {
            let parent_group = files::workspace_group(
                &record.parent_workspace,
                &self.pool,
                &self.layout.manifests(),
                own,
            )
            .map_err(|e| failed("the parent workspace's group", e))?;
            let spec = CloneSpec {
                child: child.clone(),
                user: user.to_string(),
                parent_group,
                parent: record.parent_workspace.clone(),
                dest: dest.clone(),
                hidden: self.hidden(),
                budget,
            };
            let out = self
                .deps
                .units
                .run_clone(&spec)
                .map_err(|e| failed("the child's clone", e))?;
            let report = match crate::clone::parse_line(&out) {
                Some(Ok(report)) => report,
                Some(Err("workspace_cap")) => return Err(Refusal::WorkspaceCap),
                Some(Err(_)) => return Err(Refusal::UnreadableRepository),
                None => {
                    return Err(failed(
                        "the child's clone",
                        anyhow::anyhow!("its answer did not parse"),
                    ));
                }
            };
            let (made, complete) = measure(&self.layout.child(&child))
                .map_err(|e| failed("measuring the child's workspace", e))?;
            if !complete || made > budget {
                return Err(Refusal::WorkspaceCap);
            }
            record.commit = Some(report.commit);
            record.branch = report.branch;
            record.submodules = report.submodules;
            record.large_files = report.large_files;
        }
        record.stage = Stage::Ready;
        self.save_child(record)
            .map_err(|e| failed("recording the child", e))
    }

    /// A creation that did not finish: its clone stopped, and nothing of
    /// it left.
    fn abandon(&self, child: &str) -> Result<()> {
        let unit = clone_unit_name(child);
        let units = &self.deps.units;
        if units.show(&unit).is_ok_and(|s| s.loaded) {
            units.stop(&unit)?;
            units.reset_failed(&unit);
        }
        files::remove_tree(&self.layout.child(child), self.deps.own.as_ref())
    }

    /// At the child's lease end: its workspace stays, but no longer its
    /// user's to reach, since that user goes back to the pool. It becomes
    /// the review identity's (137g), or root's on a host that has none yet.
    pub(crate) fn child_ended(&self, child: &str) -> Result<()> {
        let Some(mut r) = self.child_record(child) else {
            return Ok(());
        };
        match r.stage {
            Stage::Creating => self.abandon(child),
            Stage::Ready | Stage::Ended => {
                let workspace = self.layout.child_workspace(child);
                let own = self.deps.own.as_ref();
                match self.pool.review {
                    Some((uid, gid)) => {
                        files::give_tree(&workspace, uid, gid, own)?;
                    }
                    None => {
                        files::rootify(&workspace, own)?;
                    }
                }
                if r.stage == Stage::Ready {
                    r.stage = Stage::Ended;
                    self.save_child(&r)?;
                }
                Ok(())
            }
        }
    }

    fn stop_review(&self, child: &str) -> Result<()> {
        let unit = review_unit_name(child);
        let units = &self.deps.units;
        if units.show(&unit).is_ok_and(|s| s.loaded) {
            units.stop(&unit)?;
            units.reset_failed(&unit);
        }
        Ok(())
    }

    /// A child that has ended: its lease gone, its workspace kept.
    fn ended_child(&self, child: &str) -> Result<ChildRecord, Refusal> {
        let r = self.child_record(child).ok_or(Refusal::NoChild)?;
        if self.state.lock().unwrap().leases.user_of(child).is_some() {
            return Err(Refusal::ChildLive);
        }
        match r.stage {
            Stage::Ended => Ok(r),
            Stage::Ready | Stage::Creating => Err(Refusal::ChildLive),
        }
    }

    /// The child's workspace and the review's own directory, the review
    /// identity's (again, on a host whose identity came after the child
    /// ended).
    fn give_to_review(&self, child: &str) -> Result<(u32, u32), Refusal> {
        let (uid, gid) = self.pool.review()?;
        let own = self.deps.own.as_ref();
        let dir = self.layout.child_review(child);
        let made = (|| -> Result<()> {
            files::give_tree(&self.layout.child_workspace(child), uid, gid, own)?;
            if dir.symlink_metadata().is_err() {
                std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
            }
            files::give_tree(&dir, uid, gid, own)?;
            Ok(())
        })();
        made.map_err(|e| failed("giving the child's workspace to its review", e))?;
        Ok((uid, gid))
    }

    fn review_answer(r: &ChildRecord, mut review: Map<String, Value>) -> Answer {
        review.insert("child".into(), r.child.clone().into());
        review.insert("parent".into(), r.parent.clone().into());
        review.insert(
            "parent_workspace".into(),
            r.parent_workspace.display().to_string().into(),
        );
        if let Some(a) = &r.applied
            && a.done
        {
            review.insert("applied".into(), a.target.clone().into());
        }
        (review, None)
    }

    /// `child_export {child}` (137e): an ended child's work as data, for
    /// the person to read before Apply. Made once, by the review identity
    /// in its own unit; asked again, the same answer.
    pub(crate) fn child_export(&self, child: &str) -> Result<Answer, Refusal> {
        let _busy = self.session_busy(child)?;
        let r = self.ended_child(child)?;
        let review = self.export(&r)?;
        let (mut answer, fd) = Self::review_answer(&r, review);
        if !crate::review::fit(&mut answer, Op::ChildExport, CHILD_ANSWER_BUDGET) {
            return Err(failed(
                "the child's export",
                anyhow::anyhow!("too long for the host's read"),
            ));
        }
        Ok((answer, fd))
    }

    fn export(&self, r: &ChildRecord) -> Result<Map<String, Value>, Refusal> {
        let child = &r.child;
        if let Ok(bytes) = std::fs::read(self.layout.child_export(child))
            && let Ok(Value::Object(cached)) = serde_json::from_slice(&bytes)
        {
            return Ok(cached);
        }
        self.give_to_review(child)?;
        let used = self
            .children_usage()
            .map_err(|e| failed("measuring the children", e))?;
        let budget = self.caps.child.min(self.caps.children.saturating_sub(used));
        if budget == 0 {
            return Err(Refusal::WorkspaceCap);
        }
        let (workspace, dir) = (
            self.layout.child_workspace(child),
            self.layout.child_review(child),
        );
        let spec = ReviewSpec {
            child: child.clone(),
            parent_group: None,
            read_only: vec![workspace.clone()],
            writable: vec![dir.clone()],
            hidden: self.hidden(),
            budget,
            args: crate::review::export_args(
                r.mode,
                &workspace,
                &dir,
                r.commit.as_deref().unwrap_or("-"),
                budget,
            ),
        };
        let out = self
            .deps
            .units
            .run_review(&spec)
            .map_err(|e| failed("the child's export", e));
        let parsed = out.and_then(|out| match crate::review::parse_export(&out) {
            Some(Ok(Value::Object(v))) => Ok(v),
            Some(Ok(_)) | None => Err(failed(
                "the child's export",
                anyhow::anyhow!("its answer did not parse"),
            )),
            Some(Err(code)) => Err(refusal_of(code)),
        });
        let review = match parsed {
            Ok(v) => v,
            Err(why) => {
                let own = self.deps.own.as_ref();
                if let Err(e) = files::remove_tree(&dir, own) {
                    eprintln!("storm-runtime-isolation: clearing {child}'s review: {e:#}");
                }
                return Err(why);
            }
        };
        files::write_root_private(
            &self.layout.child_export(child),
            &serde_json::to_vec(&review).map_err(|e| failed("recording the export", e.into()))?,
        )
        .map_err(|e| failed("recording the export", e))?;
        Ok(review)
    }

    /// `child_import {child, parent, target}` (137e): Apply. A W2 child's
    /// `HEAD` becomes the new branch `storm/<child>` in the parent's
    /// repository, a W3 child's scratch files a new directory `target` in
    /// the parent's workspace. Written by the review identity with the
    /// workspace's group, then handed back to the workspace's owner;
    /// nothing existing is overwritten, nothing is merged, and a second
    /// Apply is refused.
    pub(crate) fn child_import(
        &self,
        child: &str,
        parent: &str,
        target: &str,
    ) -> Result<Answer, Refusal> {
        let _busy = self.session_busy(child)?;
        let mut r = self.ended_child(child)?;
        if r.parent != parent {
            return Err(Refusal::WrongParent);
        }
        let workspace = r.parent_workspace.clone();
        let ws = workspace.to_str().ok_or(Refusal::PathOutsideRoots)?;
        crate::policy::check_workspace(ws, &self.roots)?;
        let branch = crate::policy::apply_branch(child);
        match r.mode {
            ChildMode::Clone if target != branch => return Err(Refusal::BadValue("target")),
            ChildMode::Scratch if target == branch => return Err(Refusal::BadValue("target")),
            ChildMode::Clone if parent_mode(&workspace) != ChildMode::Clone => {
                return Err(Refusal::UnreadableRepository);
            }
            _ => {}
        }
        // A second Apply is refused; one a restart interrupted may finish.
        let resuming = match &r.applied {
            Some(a) if a.done => {
                return Err(match r.mode {
                    ChildMode::Clone => Refusal::BranchExists,
                    ChildMode::Scratch => Refusal::TargetExists,
                });
            }
            Some(a) => a.target == target,
            None => false,
        };
        let review = self.export(&r)?;
        let (uid, _) = self.give_to_review(child)?;
        let own = self.deps.own.as_ref();
        let group = files::workspace_group(&workspace, &self.pool, &self.layout.manifests(), own)
            .map_err(|e| failed("the parent workspace's group", e))?;
        let (read_only, args) = match r.mode {
            ChildMode::Clone => {
                let tip = review
                    .get("tip")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failed("the export", anyhow::anyhow!("it has no tip")))?;
                let dir = self.layout.child_review(child);
                let bundle = dir.join(crate::review::BUNDLE);
                (
                    vec![dir],
                    crate::review::import_clone_args(&workspace, &bundle, &branch, tip),
                )
            }
            ChildMode::Scratch => {
                let scratch = self.layout.child_workspace(child);
                (
                    vec![scratch.clone()],
                    crate::review::import_scratch_args(&scratch, &workspace, child, target),
                )
            }
        };
        r.applied = Some(Applied {
            target: target.to_string(),
            done: false,
        });
        self.save_child(&r)
            .map_err(|e| failed("recording the Apply", e))?;
        let spec = ReviewSpec {
            child: child.to_string(),
            parent_group: Some(group),
            read_only,
            writable: vec![workspace.clone()],
            hidden: self.hidden(),
            budget: self.caps.child,
            args,
        };
        let out = self
            .deps
            .units
            .run_review(&spec)
            .map_err(|e| failed("the child's Apply", e));
        // Whatever it wrote is the workspace owner's, as a session's is at
        // its lease end, before anyone is answered.
        if let Err(e) = files::hand_back(&workspace, uid, own) {
            eprintln!("storm-runtime-isolation: handing back {child}'s Apply: {e:#}");
        }
        let parsed = out.and_then(|out| match crate::review::parse_import(&out) {
            Some(Ok(Value::Object(v))) => Ok(v),
            Some(Ok(_)) | None => Err(failed(
                "the child's Apply",
                anyhow::anyhow!("its answer did not parse"),
            )),
            Some(Err(code)) => Err(refusal_of(code)),
        });
        let made = match parsed {
            // A branch that was already there at the child's tip is this
            // Apply's own only when a restart interrupted it.
            Ok(v) if v.get("already") == Some(&Value::Bool(true)) && !resuming => {
                Err(Refusal::BranchExists)
            }
            other => other,
        };
        match made {
            Ok(mut v) => {
                r.applied = Some(Applied {
                    target: target.to_string(),
                    done: true,
                });
                self.save_child(&r)
                    .map_err(|e| failed("recording the Apply", e))?;
                v.remove("already");
                v.insert("child".into(), child.into());
                Ok((v, None))
            }
            Err(why) => {
                r.applied = None;
                self.save_child(&r)
                    .map_err(|e| failed("recording the Apply", e))?;
                Err(why)
            }
        }
    }

    /// `child_dispose {child}` (137g, WS-6): a human's delete of an ended
    /// child's workspace, or of an orphan named by the listing. Nothing
    /// there is no error: a repeated cleanup does nothing.
    pub(crate) fn child_dispose(&self, child: &str) -> Result<Answer, Refusal> {
        let _busy = self.session_busy(child)?;
        if self.state.lock().unwrap().leases.user_of(child).is_some() {
            return Err(Refusal::ChildLive);
        }
        let dir = self.layout.child(child);
        let record = self.child_record(child);
        let mut out = Map::new();
        if dir.symlink_metadata().is_err() {
            out.insert("removed".into(), false.into());
            return Ok((out, None));
        }
        self.stop_review(child)
            .map_err(|e| failed("stopping the child's review", e))?;
        self.abandon(child)
            .map_err(|e| failed("removing the child's workspace", e))?;
        out.insert("removed".into(), true.into());
        out.insert("orphan".into(), record.is_none().into());
        Ok((out, None))
    }

    /// `child_list` (137g): every child workspace the helper keeps, and
    /// every directory under the children root that no record claims.
    pub(crate) fn child_list(&self) -> Result<Answer, Refusal> {
        let mut children = Vec::new();
        let (mut orphans, mut unknown) = (Vec::new(), 0u64);
        let entries = match std::fs::read_dir(self.layout.children()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            other => Some(other.map_err(|e| failed("listing the children", e.into()))?),
        };
        let leased: Vec<String> = {
            let st = self.state.lock().unwrap();
            st.records.values().map(|l| l.session.clone()).collect()
        };
        for entry in entries.into_iter().flatten() {
            let entry = entry.map_err(|e| failed("listing the children", e.into()))?;
            let Some(name) = entry
                .file_name()
                .to_str()
                .filter(|n| crate::policy::session_ok(n))
                .map(str::to_string)
            else {
                unknown += 1;
                continue;
            };
            match self.child_record(&name) {
                None => orphans.push(Value::from(name)),
                Some(r) => {
                    let stage = match r.stage {
                        Stage::Creating => "creating",
                        Stage::Ready => "ready",
                        Stage::Ended => "ended",
                    };
                    children.push(serde_json::json!({
                        "child": r.child,
                        "parent": r.parent,
                        "parent_workspace": r.parent_workspace.display().to_string(),
                        "mode": r.mode.name(),
                        "stage": stage,
                        "live": leased.contains(&r.child),
                        "commit": r.commit,
                        "branch": r.branch,
                        "applied": r.applied.as_ref().filter(|a| a.done).map(|a| a.target.clone()),
                    }));
                }
            }
        }
        children.sort_by(|a, b| a["child"].as_str().cmp(&b["child"].as_str()));
        orphans.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        let mut out = Map::new();
        out.insert("children".into(), children.into());
        out.insert("orphans".into(), orphans.into());
        out.insert("unknown_entries".into(), unknown.into());
        if !crate::review::fit_listing(&mut out, Op::ChildList, CHILD_ANSWER_BUDGET) {
            return Err(failed(
                "the children's listing",
                anyhow::anyhow!("too long for the host's read"),
            ));
        }
        Ok((out, None))
    }

    /// What a previous helper left half made is abandoned before this one
    /// answers.
    /// One child that cannot be recovered is logged and left as it is; the
    /// others go on (139-D1).
    pub(crate) fn recover_children(&self) -> Result<()> {
        let entries = match std::fs::read_dir(self.layout.children()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            other => other?,
        };
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let name = entry.file_name();
            let Some(child) = name.to_str().filter(|n| crate::policy::session_ok(n)) else {
                continue;
            };
            let Some(r) = self.child_record(child) else {
                continue;
            };
            let recovered = if r.stage == Stage::Creating {
                self.abandon(child)
            } else {
                // An export or an Apply the last helper did not see end: its
                // unit goes, and the same request can be made again (G-137-2).
                self.stop_review(child)
            };
            if let Err(e) = recovered {
                eprintln!(
                    "audit: skipped child={child} reason=recovery_failed detail={:?}",
                    format!("{e:#}")
                );
            }
        }
        Ok(())
    }
}

/// The refusal for a code a review unit printed.
fn refusal_of(code: &str) -> Refusal {
    match code {
        "workspace_cap" => Refusal::WorkspaceCap,
        "unreadable_repository" => Refusal::UnreadableRepository,
        "branch_exists" => Refusal::BranchExists,
        "target_exists" => Refusal::TargetExists,
        _ => Refusal::Failed,
    }
}

/// The descriptor-free check `start` makes for a child: its workspace is
/// made, and the host names the parent's workspace it was made from.
pub fn startable(r: &ChildRecord, workspace: &Path) -> Result<(), Refusal> {
    if r.stage != Stage::Ready || r.parent_workspace != workspace {
        return Err(Refusal::WrongParent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clone::tests::{git, hostile_at, repo};
    use crate::convert::{Key, Meta, Ownership};
    use crate::session::tests::{Fixture, fixture};
    use serde_json::json;
    use std::os::unix::fs::MetadataExt;

    fn meta(f: &Fixture, p: &Path) -> Meta {
        let md = std::fs::symlink_metadata(p).unwrap();
        f.own.meta(
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

    /// The parent, started in the fixture's workspace, and the child,
    /// leased and prepared as the host does before `child_create`.
    fn parent_and_child(f: &Fixture) -> (String, String) {
        let (parent_user, ..) = f.started("ags_P");
        let (v, _) = f.ok(json!({"op": "lease", "session": "ags_C"}), vec![]);
        let child_user = v["user"].as_str().unwrap().to_string();
        f.ok(
            json!({"op": "prepare", "session": "ags_C", "user": child_user}),
            vec![],
        );
        (parent_user, child_user)
    }

    fn create(f: &Fixture) -> Value {
        f.call(
            json!({"op": "child_create", "parent": "ags_P", "child": "ags_C", "workspace": f.workspace()}),
            vec![],
        )
        .0
    }

    fn start_child(f: &Fixture, user: &str) -> Value {
        f.launch("ags_C");
        let (_master, slave) = crate::tty::tests::pty();
        f.call(
            json!({"op": "start", "session": "ags_C", "user": user, "provider": "shell", "workspace": f.workspace()}),
            vec![slave],
        )
        .0
    }

    fn git_workspace(f: &Fixture) -> String {
        let ws = PathBuf::from(f.workspace());
        std::fs::remove_dir_all(&ws).unwrap();
        repo(&ws, &[("README", "x"), ("src/lib.rs", "fn main() {}\n")])
    }

    /// WT-2's half the helper decides, for W2: the child's workspace is
    /// its own clone, its user's alone, made in a unit that reads the
    /// parent once; the child's unit never sees the parent's workspace,
    /// and the parent's never sees the child's.
    #[test]
    fn wt_2_a_git_childs_workspace_is_its_own_clone() {
        let f = fixture();
        let commit = git_workspace(&f);
        let (_, child_user) = parent_and_child(&f);
        let v = create(&f);
        assert_eq!(v["ok"], true, "{v}");
        let dest = f.helper.layout.child_workspace("ags_C");
        assert_eq!(v["mode"], "clone");
        assert_eq!(v["workspace"], dest.display().to_string());
        assert_eq!(v["commit"], commit);
        assert_eq!(v["branch"], "main");
        assert_eq!(
            (v["submodules"].clone(), v["large_files"].clone()),
            (json!(false), json!(false))
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("src/lib.rs")).unwrap(),
            "fn main() {}\n"
        );
        let m = meta(&f, &dest);
        assert_eq!(
            (m.uid, m.gid, m.mode),
            (
                f.helper.pool.users[&child_user],
                f.helper.pool.primary[&child_user],
                0o700
            )
        );
        // The record is the helper's.
        let r = f.helper.child_record("ags_C").unwrap();
        assert_eq!((r.stage, r.mode), (Stage::Ready, ChildMode::Clone));
        let record = std::fs::metadata(f.helper.layout.child_record("ags_C")).unwrap();
        assert_eq!(record.mode() & 0o777, 0o600);
        // The clone ran as the child, reading the parent read-only.
        let clones = f.units.clones.lock().unwrap().clone();
        assert_eq!(clones.len(), 1);
        let c = &clones[0];
        assert_eq!(c.user, child_user);
        assert_eq!(c.parent, PathBuf::from(f.workspace()));
        let props = c.properties().unwrap();
        let writable: Vec<&String> = props
            .iter()
            .filter(|p| p.starts_with("BindPaths=") || p.starts_with("ReadWritePaths="))
            .collect();
        assert_eq!(writable, [&format!("BindPaths={}", dest.display())]);
        assert!(props.contains(&format!("BindReadOnlyPaths={}", f.workspace())));
        // The child's own unit: its clone, never the parent's workspace.
        let v = start_child(&f, &child_user);
        assert_eq!(v["ok"], true, "{v}");
        let specs = f.units.specs.lock().unwrap().clone();
        let child = specs.iter().find(|s| s.session == "ags_C").unwrap();
        let parent = specs.iter().find(|s| s.session == "ags_P").unwrap();
        assert_eq!(child.workspace, dest);
        assert_eq!(child.parent_view, None);
        assert_eq!(child.workspace_group, child_user, "not the parent's group");
        let props = child.properties().unwrap();
        assert!(props.contains(&format!("WorkingDirectory={}", dest.display())));
        let shown: Vec<&String> = props
            .iter()
            .filter(|p| p.starts_with("BindPaths=") || p.starts_with("BindReadOnlyPaths="))
            .collect();
        assert!(
            !shown.iter().any(|p| p.contains(&f.workspace())),
            "{shown:?}"
        );
        assert!(
            child.hidden.contains(&f.root),
            "every workspace root hidden"
        );
        // The parent's view hides the child's: the helper's state is
        // covered, and only the parent's own directories come back.
        assert!(parent.hidden.contains(&f.helper.layout.state));
        assert!(
            !parent
                .properties()
                .unwrap()
                .iter()
                .any(|p| p.contains("children"))
        );
        // Lease end never walks the parent's workspace for the child.
        let leases = files::load_leases(&f.helper.layout.leases()).unwrap();
        assert_eq!(leases[&child_user].workspace, None);
    }

    /// W3 (137c): a workspace that is not a repository root — no `.git`, or
    /// a linked worktree's `.git` file — gives the child a scratch
    /// directory and its parent read-only.
    #[test]
    fn w3_a_non_git_childs_view_is_its_parent_read_only_and_its_scratch() {
        for linked in [false, true] {
            let f = fixture();
            if linked {
                std::fs::write(
                    PathBuf::from(f.workspace()).join(".git"),
                    "gitdir: /elsewhere/.git/worktrees/app\n",
                )
                .unwrap();
            }
            let (_, child_user) = parent_and_child(&f);
            let v = create(&f);
            assert_eq!(v["ok"], true, "{v}");
            assert_eq!(v["mode"], "scratch");
            assert_eq!(v["commit"], Value::Null);
            assert!(f.units.clones.lock().unwrap().is_empty(), "nothing cloned");
            let dest = f.helper.layout.child_workspace("ags_C");
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
            let v = start_child(&f, &child_user);
            assert_eq!(v["ok"], true, "{v}");
            let specs = f.units.specs.lock().unwrap().clone();
            let child = specs.iter().find(|s| s.session == "ags_C").unwrap();
            let parent = specs.iter().find(|s| s.session == "ags_P").unwrap();
            assert_eq!(child.workspace, dest);
            assert_eq!(child.parent_view, Some(PathBuf::from(f.workspace())));
            assert_eq!(child.workspace_group, parent.workspace_group);
            let props = child.properties().unwrap();
            assert!(props.contains(&format!("BindPaths={}", dest.display())));
            assert!(props.contains(&format!("BindReadOnlyPaths={}", f.workspace())));
            assert!(!props.contains(&format!("BindPaths={}", f.workspace())));
        }
    }

    /// WT-1 at unit level: a child's workspace, its `.git` with it, is
    /// reachable by no other session. It is its user's `0700`, under the
    /// helper's own state, which every other unit's view covers and shows
    /// only that session's own directories again.
    #[test]
    fn wt_1_no_other_session_sees_a_childs_workspace() {
        let f = fixture();
        git_workspace(&f);
        let (_, child_user) = parent_and_child(&f);
        assert_eq!(create(&f)["ok"], true);
        let (other, ..) = f.started("ags_O");
        let dest = f.helper.layout.child_workspace("ags_C");
        assert!(dest.starts_with(&f.helper.layout.state));
        let specs = f.units.specs.lock().unwrap().clone();
        let o = specs.iter().find(|s| s.session == "ags_O").unwrap();
        assert_ne!(o.user, child_user);
        assert!(o.hidden.contains(&f.helper.layout.state));
        assert!(
            !o.properties()
                .unwrap()
                .iter()
                .any(|p| p.contains(&dest.display().to_string()))
        );
        let m = meta(&f, &dest);
        assert_eq!((m.uid, m.mode), (f.helper.pool.users[&child_user], 0o700));
        assert_ne!(m.uid, f.helper.pool.users[&other]);
        // The child's own hooks and config are its own: no template's.
        assert!(!dest.join(".git/hooks").exists());
        let dir = std::fs::metadata(f.helper.layout.child("ags_C")).unwrap();
        assert_eq!(dir.mode() & 0o777, 0o700);
    }

    /// WT-7 through the helper: a hostile parent and an account whose git
    /// configuration has its own hooks and filters; the child's workspace
    /// is made, and nothing of either ran. Untracked, ignored and
    /// uncommitted files are not in it.
    #[test]
    fn wt_7_a_hostile_parent_and_account_run_nothing_while_a_child_is_made() {
        let f = fixture();
        let ws = PathBuf::from(f.workspace());
        std::fs::remove_dir_all(&ws).unwrap();
        let h = hostile_at(&f.root.join(".plants"), &ws);
        std::fs::write(ws.join("a.txt"), "uncommitted\n").unwrap();
        std::fs::write(ws.join("untracked"), "x").unwrap();
        std::fs::create_dir_all(ws.join(".git/info")).unwrap();
        std::fs::write(ws.join(".git/info/exclude"), "ignored\n").unwrap();
        std::fs::write(ws.join("ignored"), "secret").unwrap();
        *f.units.clone_home.lock().unwrap() = Some(h.home.clone());
        parent_and_child(&f);
        let v = create(&f);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(h.plants.ran(), Vec::<String>::new());
        assert_eq!(v["commit"], h.commit);
        assert_eq!(
            (v["submodules"].clone(), v["large_files"].clone()),
            (json!(true), json!(true))
        );
        let dest = f.helper.layout.child_workspace("ags_C");
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "committed\n"
        );
        for absent in ["untracked", "ignored"] {
            assert!(!dest.join(absent).exists(), "{absent}");
        }
    }

    fn nothing_left(f: &Fixture) {
        assert!(!f.helper.layout.child("ags_C").exists());
        assert!(f.helper.child_record("ags_C").is_none());
    }

    /// WT-9: a start above either cap is refused as `workspace_cap`, and
    /// nothing is left behind, whether it is refused before the clone,
    /// by the clone before its checkout, or after it.
    #[test]
    fn wt_9_a_child_above_either_cap_is_refused_and_leaves_nothing() {
        // Above the per-child cap, measured before anything is made.
        let mut f = fixture();
        git_workspace(&f);
        parent_and_child(&f);
        let parents_git = disk_usage(&PathBuf::from(f.workspace()).join(".git")).unwrap();
        f.helper.caps.child = parents_git - 1;
        assert_eq!(create(&f)["error"], "workspace_cap");
        assert!(f.units.clones.lock().unwrap().is_empty());
        nothing_left(&f);
        // Above the host's cap, counting the children already there.
        f.helper.caps.child = 1 << 30;
        std::fs::create_dir_all(f.helper.layout.child("ags_OLD")).unwrap();
        std::fs::write(
            f.helper.layout.child("ags_OLD").join("f"),
            vec![1u8; 64 << 10],
        )
        .unwrap();
        let used = disk_usage(&f.helper.layout.children()).unwrap();
        f.helper.caps.children = used + parents_git - 1;
        assert_eq!(create(&f)["error"], "workspace_cap");
        nothing_left(&f);
        // A scratch child copies nothing, but a host already above its cap
        // takes no more children.
        let mut g = fixture();
        parent_and_child(&g);
        std::fs::create_dir_all(g.helper.layout.child("ags_OLD")).unwrap();
        std::fs::write(
            g.helper.layout.child("ags_OLD").join("f"),
            vec![1u8; 64 << 10],
        )
        .unwrap();
        g.helper.caps.children = 4096;
        assert_eq!(create(&g)["error"], "workspace_cap");
        nothing_left(&g);
    }

    /// WT-9 mid-way: a clone that crosses its budget is abandoned and
    /// cleaned up, when the clone finds it and when the helper does.
    #[test]
    fn wt_9_a_creation_that_crosses_a_cap_midway_is_abandoned() {
        let mut f = fixture();
        let ws = PathBuf::from(f.workspace());
        std::fs::remove_dir_all(&ws).unwrap();
        repo(&ws, &[("zeros", &"0".repeat(4 << 20))]);
        parent_and_child(&f);
        f.helper.caps.child = 2 << 20;
        assert_eq!(create(&f)["error"], "workspace_cap");
        assert_eq!(f.units.clones.lock().unwrap().len(), 1, "the clone ran");
        nothing_left(&f);
        // The helper measures what the clone made, whatever it said.
        *f.units.clone_fill.lock().unwrap() = Some(256 << 10);
        f.helper.caps.child = 128 << 10;
        *f.units.clone_output.lock().unwrap() =
            Some(crate::clone::line(&Ok(crate::clone::Report {
                commit: "a".repeat(40),
                branch: None,
                submodules: false,
                large_files: false,
            })));
        assert_eq!(create(&f)["error"], "workspace_cap");
        nothing_left(&f);
    }

    /// 137h: a creation that fails fails with its reason, leaves nothing,
    /// and its unit is stopped.
    #[test]
    fn a_failed_clone_is_its_reason_and_leaves_nothing() {
        let f = fixture();
        let ws = PathBuf::from(f.workspace());
        std::fs::create_dir(ws.join(".git")).unwrap();
        std::fs::write(ws.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        parent_and_child(&f);
        assert_eq!(create(&f)["error"], "unreadable_repository");
        nothing_left(&f);
        for (out, code) in [
            (r#"{"failed":"no_commit"}"#, "unreadable_repository"),
            ("garbage", "failed"),
            (r#"{"done":{"commit":"x"}}"#, "failed"),
        ] {
            *f.units.clone_output.lock().unwrap() = Some(out.into());
            assert_eq!(create(&f)["error"], code, "{out}");
            nothing_left(&f);
        }
        let calls = f.units.calls.lock().unwrap().clone();
        assert!(calls.contains(&"clone storm-child-ags_C-clone.service".to_string()));
    }

    /// `child_create` is for a child that holds a lease and has not
    /// started, from a live top-level parent in that very workspace.
    #[test]
    fn a_child_needs_a_live_parent_in_that_workspace() {
        let f = fixture();
        std::fs::create_dir(f.root.join("other")).unwrap();
        let req = |parent: &str, child: &str, ws: &str| {
            f.call(
                json!({"op": "child_create", "parent": parent, "child": child, "workspace": ws}),
                vec![],
            )
            .0["error"]
                .clone()
        };
        let ws = f.workspace();
        let other = f.root.join("other").display().to_string();
        assert_eq!(req("ags_P", "ags_C", &ws), "not_leased", "no child lease");
        let (v, _) = f.ok(json!({"op": "lease", "session": "ags_C"}), vec![]);
        let child_user = v["user"].as_str().unwrap().to_string();
        f.ok(
            json!({"op": "prepare", "session": "ags_C", "user": child_user}),
            vec![],
        );
        assert_eq!(req("ags_P", "ags_C", &ws), "wrong_parent", "no parent");
        f.started("ags_P");
        assert_eq!(req("ags_P", "ags_C", &other), "wrong_parent");
        assert_eq!(req("ags_C", "ags_C", &ws), "wrong_parent");
        assert_eq!(req("ags_P", "ags_C", "/etc"), "path_outside_roots");
        assert_eq!(req("ags_P", "ags_../C", &ws), "bad_session");
        // A child's own child is not built (137-1), even once it runs.
        assert_eq!(
            f.call(
                json!({"op": "child_create", "parent": "ags_P", "child": "ags_C", "workspace": ws}),
                vec![],
            )
            .0["ok"],
            true
        );
        assert_eq!(start_child(&f, &child_user)["ok"], true);
        f.ok(json!({"op": "lease", "session": "ags_G"}), vec![]);
        assert_eq!(req("ags_C", "ags_G", &ws), "wrong_parent");
        // A started session is no one's new child.
        f.started("ags_S");
        assert_eq!(req("ags_P", "ags_S", &ws), "already_started");
    }

    /// Creation is idempotent: asked again it answers what it made;
    /// for another parent, it is refused.
    #[test]
    fn asking_again_answers_what_was_made() {
        let f = fixture();
        git_workspace(&f);
        parent_and_child(&f);
        let first = create(&f);
        assert_eq!(first["ok"], true);
        assert_eq!(create(&f), first);
        assert_eq!(f.units.clones.lock().unwrap().len(), 1, "cloned once");
        f.started("ags_Q");
        let (v, _) = f.call(
            json!({"op": "child_create", "parent": "ags_Q", "child": "ags_C", "workspace": f.workspace()}),
            vec![],
        );
        assert_eq!(v["error"], "child_exists");
        // A directory no record claims is an orphan, never reused.
        f.ok(json!({"op": "lease", "session": "ags_D"}), vec![]);
        std::fs::create_dir_all(f.helper.layout.child("ags_D")).unwrap();
        let (v, _) = f.call(
            json!({"op": "child_create", "parent": "ags_P", "child": "ags_D", "workspace": f.workspace()}),
            vec![],
        );
        assert_eq!(v["error"], "child_exists");
        assert!(f.helper.layout.child("ags_D").exists());
    }

    /// 137g: at the child's lease end its workspace is kept and marked;
    /// its user, going back to the pool, can no longer reach it. A
    /// creation a restart interrupted is abandoned.
    #[test]
    fn a_childs_workspace_outlives_its_lease_and_a_half_made_one_does_not() {
        let f = fixture();
        git_workspace(&f);
        let (_, child_user) = parent_and_child(&f);
        assert_eq!(create(&f)["ok"], true);
        assert_eq!(start_child(&f, &child_user)["ok"], true);
        f.ok(
            json!({"op": "release", "session": "ags_C", "user": child_user}),
            vec![],
        );
        let dest = f.helper.layout.child_workspace("ags_C");
        assert!(dest.join("README").exists());
        assert_eq!(f.helper.child_record("ags_C").unwrap().stage, Stage::Ended);
        // The review identity's now (137g), and nobody else's.
        let review = f.helper.pool.review.unwrap();
        let m = meta(&f, &dest);
        assert_eq!((m.uid, m.gid, m.mode), (review.0, review.1, 0o700));
        let m = meta(&f, &dest.join(".git"));
        assert_eq!((m.uid, m.mode), (review.0, 0o700));
        let m = meta(&f, &dest.join("README"));
        assert_eq!((m.uid, m.mode & 0o077), (review.0, 0));
        // An ended child is never started again.
        let (v, _) = f.ok(json!({"op": "lease", "session": "ags_C"}), vec![]);
        assert_eq!(
            start_child(&f, v["user"].as_str().unwrap())["error"],
            "wrong_parent"
        );
        // A half-made child, as a crash leaves it.
        let mut r = f.helper.child_record("ags_C").unwrap();
        r.child = "ags_H".into();
        r.stage = Stage::Creating;
        std::fs::create_dir_all(f.helper.layout.child_workspace("ags_H")).unwrap();
        f.helper.save_child(&r).unwrap();
        f.helper.recover().unwrap();
        assert!(!f.helper.layout.child("ags_H").exists());
        assert!(
            f.helper.layout.child("ags_C").exists(),
            "an ended one stays"
        );
    }

    /// A W2 child that committed work and ended, as 134-5 will leave one.
    fn ended_clone_child(f: &Fixture) -> (String, String) {
        git_workspace(f);
        let (_, child_user) = parent_and_child(f);
        assert_eq!(create(f)["ok"], true);
        assert_eq!(start_child(f, &child_user)["ok"], true);
        let dest = f.helper.layout.child_workspace("ags_C");
        std::fs::write(dest.join("README"), "the child's\n").unwrap();
        git(&dest, &["commit", "-q", "-am", "child work"]);
        let tip = git(&dest, &["rev-parse", "HEAD"]);
        std::fs::write(dest.join("README"), "uncommitted\n").unwrap();
        f.ok(
            json!({"op": "release", "session": "ags_C", "user": child_user}),
            vec![],
        );
        (child_user, tip)
    }

    fn export_child(f: &Fixture) -> Value {
        f.call(json!({"op": "child_export", "child": "ags_C"}), vec![])
            .0
    }

    fn apply(f: &Fixture, target: &str) -> Value {
        f.call(
            json!({"op": "child_import", "child": "ags_C", "parent": "ags_P", "target": target}),
            vec![],
        )
        .0
    }

    /// WT-3 through the helper: an ended child's committed work reaches the
    /// parent's workspace only through Apply, as `storm/<child>`; its
    /// uncommitted work never does; the export and the Apply each run as
    /// the review identity in a unit that sees only what it reads, and
    /// writes only what it must.
    #[test]
    fn wt_3_a_childs_work_reaches_its_parent_only_through_apply() {
        let f = fixture();
        let (_, tip) = ended_clone_child(&f);
        let parent = PathBuf::from(f.workspace());
        let branches = || {
            git(
                &parent,
                &["for-each-ref", "--format=%(refname)", "refs/heads"],
            )
        };
        assert_eq!(branches(), "refs/heads/main", "nothing before Apply");
        let v = export_child(&f);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(
            (v["mode"].clone(), v["tip"].clone()),
            (json!("clone"), json!(tip))
        );
        assert_eq!(
            (v["child"].clone(), v["parent"].clone()),
            (json!("ags_C"), json!("ags_P"))
        );
        assert_eq!(v["commits"][0]["subject"], "child work");
        assert!(v["patch"].as_str().unwrap().contains("+the child's"));
        assert!(!v["patch"].as_str().unwrap().contains("uncommitted"));
        assert_eq!(branches(), "refs/heads/main", "an export changes nothing");
        // Asked again, the same answer, and no second unit.
        assert_eq!(export_child(&f), v);
        assert_eq!(f.units.reviews.lock().unwrap().len(), 1);
        let export = f.units.reviews.lock().unwrap()[0].clone();
        let review_dir = f.helper.layout.child_review("ags_C");
        let child_ws = f.helper.layout.child_workspace("ags_C");
        assert_eq!(export.read_only, std::slice::from_ref(&child_ws));
        assert_eq!(export.writable, std::slice::from_ref(&review_dir));
        assert_eq!(export.parent_group, None);
        let props = export.properties().unwrap();
        assert!(props.contains(&"User=storm-review".to_string()));
        assert!(
            !props.iter().any(|p| p.contains(&f.workspace())),
            "{props:#?}"
        );
        let before = std::fs::read(parent.join("README")).unwrap();
        let v = apply(&f, "storm/ags_C");
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(
            (v["branch"].clone(), v["commit"].clone()),
            (json!("storm/ags_C"), json!(tip))
        );
        assert_eq!(git(&parent, &["rev-parse", "refs/heads/storm/ags_C"]), tip);
        assert_eq!(git(&parent, &["symbolic-ref", "HEAD"]), "refs/heads/main");
        assert_eq!(std::fs::read(parent.join("README")).unwrap(), before);
        assert_eq!(git(&parent, &["status", "--porcelain"]), "");
        let import = f.units.reviews.lock().unwrap()[1].clone();
        assert_eq!(import.read_only, [review_dir]);
        assert_eq!(import.writable, std::slice::from_ref(&parent));
        let group = import.parent_group.clone().unwrap();
        assert!(group.starts_with("storm-w"), "{group}");
        let props = import.properties().unwrap();
        for want in [
            "User=storm-review".to_string(),
            format!("SupplementaryGroups={group}"),
            "UMask=0007".into(),
            "PrivateNetwork=yes".into(),
            "CapabilityBoundingSet=".into(),
        ] {
            assert!(props.contains(&want), "{want}");
        }
        assert!(!props.iter().any(|p| p.contains("children/ags_C/workspace")));
        // WT-6: a second Apply is refused, and changes nothing.
        assert_eq!(apply(&f, "storm/ags_C")["error"], "branch_exists");
        assert_eq!(git(&parent, &["rev-parse", "refs/heads/storm/ags_C"]), tip);
        let listed = f.ok(json!({"op": "child_list"}), vec![]).0;
        assert_eq!(listed["children"][0]["applied"], "storm/ags_C");
        assert_eq!(listed["children"][0]["stage"], "ended");
    }

    /// What an Apply wrote in the parent's workspace is the workspace
    /// owner's again, in its group, before anyone is answered, as a
    /// session's files are at its lease end.
    #[test]
    fn what_an_apply_wrote_is_handed_back() {
        let f = fixture();
        ended_clone_child(&f);
        let (uid, gid) = f.helper.pool.review.unwrap();
        *f.units.review_owner.lock().unwrap() = Some((f.own.clone(), uid, gid));
        assert_eq!(apply(&f, "storm/ags_C")["ok"], true);
        let parent = PathBuf::from(f.workspace());
        let made = parent.join(".git/refs/heads/storm/ags_C");
        let m = meta(&f, &made);
        let top = std::fs::symlink_metadata(&parent).unwrap();
        assert_eq!(
            (m.uid, m.gid),
            (top.uid(), top.gid()),
            "the workspace owner's"
        );
        let rec = f.own.0.lock().unwrap();
        assert!(
            !rec.overlay
                .borrow()
                .values()
                .any(|m| m.uid == uid && m.gid != uid),
            "nothing in the parent is left the review identity's"
        );
    }

    /// The child's work is its own while it holds its lease (137g), and
    /// only the parent it was made from is applied to.
    #[test]
    fn a_live_child_an_unknown_one_and_the_wrong_parent_are_refused() {
        let f = fixture();
        git_workspace(&f);
        let (_, child_user) = parent_and_child(&f);
        assert_eq!(export_child(&f)["error"], "no_child");
        assert_eq!(create(&f)["ok"], true);
        assert_eq!(start_child(&f, &child_user)["ok"], true);
        assert_eq!(export_child(&f)["error"], "child_live");
        assert_eq!(apply(&f, "storm/ags_C")["error"], "child_live");
        let (v, _) = f.call(json!({"op": "child_dispose", "child": "ags_C"}), vec![]);
        assert_eq!(v["error"], "child_live");
        assert!(f.helper.layout.child_workspace("ags_C").exists());
        f.ok(
            json!({"op": "release", "session": "ags_C", "user": child_user}),
            vec![],
        );
        let (v, _) = f.call(
            json!({"op": "child_import", "child": "ags_C", "parent": "ags_Q", "target": "storm/ags_C"}),
            vec![],
        );
        assert_eq!(v["error"], "wrong_parent");
        assert_eq!(
            apply(&f, "app-ags_C")["error"],
            "bad_value",
            "a clone is applied as its branch"
        );
        // Leased again under the same id, it is live again; a record that
        // never reached its end is not ended.
        f.ok(json!({"op": "lease", "session": "ags_C"}), vec![]);
        assert_eq!(export_child(&f)["error"], "child_live");
        let user = f
            .helper
            .state
            .lock()
            .unwrap()
            .leases
            .user_of("ags_C")
            .unwrap()
            .to_string();
        f.ok(
            json!({"op": "release", "session": "ags_C", "user": user}),
            vec![],
        );
        let mut r = f.helper.child_record("ags_C").unwrap();
        r.stage = Stage::Ready;
        f.helper.save_child(&r).unwrap();
        assert_eq!(export_child(&f)["error"], "child_live");
        assert!(f.units.reviews.lock().unwrap().is_empty(), "nothing ran");
    }

    #[test]
    fn without_the_review_identity_nothing_is_exported_and_a_child_stays_roots() {
        let mut f = fixture();
        f.helper.pool.review = None;
        ended_clone_child(&f);
        let m = meta(&f, &f.helper.layout.child_workspace("ags_C"));
        assert_eq!((m.uid, m.mode), (0, 0o700));
        assert_eq!(export_child(&f)["error"], "no_review_identity");
        assert_eq!(apply(&f, "storm/ags_C")["error"], "no_review_identity");
    }

    /// W3 (137e, 138p): Apply copies the scratch files into a new directory
    /// the person names in the parent's workspace; an existing name is
    /// refused, and so is a second Apply.
    #[test]
    fn a_scratch_childs_apply_is_a_new_directory_the_person_names() {
        let f = fixture();
        let (_, child_user) = parent_and_child(&f);
        assert_eq!(create(&f)["ok"], true);
        assert_eq!(start_child(&f, &child_user)["ok"], true);
        let scratch = f.helper.layout.child_workspace("ags_C");
        std::fs::write(scratch.join("out.md"), "result\n").unwrap();
        f.ok(
            json!({"op": "release", "session": "ags_C", "user": child_user}),
            vec![],
        );
        let v = export_child(&f);
        assert_eq!(
            (v["mode"].clone(), v["files"][0]["path"].clone()),
            (json!("scratch"), json!("out.md"))
        );
        let parent = PathBuf::from(f.workspace());
        assert_eq!(apply(&f, "README")["error"], "target_exists");
        assert_eq!(std::fs::read(parent.join("README")).unwrap(), b"x");
        assert_eq!(apply(&f, "storm/ags_C")["error"], "bad_value");
        let v = apply(&f, "app-ags_C");
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(
            v["directory"],
            parent.join("app-ags_C").display().to_string()
        );
        assert_eq!(
            std::fs::read_to_string(parent.join("app-ags_C/out.md")).unwrap(),
            "result\n"
        );
        assert_eq!(apply(&f, "app-ags_C2")["error"], "target_exists", "once");
        assert!(!parent.join("app-ags_C2").exists());
    }

    /// WT-4: the parent, the child or the host ends mid-work. The child's
    /// workspace is intact and marked; a delete removes it; the second does
    /// nothing.
    #[test]
    fn wt_4_an_end_mid_work_leaves_the_workspace_and_cleanup_twice_does_nothing() {
        let f = fixture();
        git_workspace(&f);
        let (parent_user, child_user) = parent_and_child(&f);
        assert_eq!(create(&f)["ok"], true);
        assert_eq!(start_child(&f, &child_user)["ok"], true);
        let dest = f.helper.layout.child_workspace("ags_C");
        std::fs::write(dest.join("half-done"), "wip").unwrap();
        // The parent ends first: the child's workspace is not the parent's.
        f.ok(
            json!({"op": "release", "session": "ags_P", "user": parent_user}),
            vec![],
        );
        assert_eq!(f.helper.child_record("ags_C").unwrap().stage, Stage::Ready);
        assert!(dest.join("half-done").exists());
        // The host restarts mid-work: the helper's recovery ends the child,
        // and the new helper holds no lease in memory.
        f.helper.recover().unwrap();
        {
            let mut st = f.helper.state.lock().unwrap();
            st.leases.release("ags_C", &child_user);
            st.records.remove(&child_user);
            st.started.remove("ags_C");
        }
        let r = f.helper.child_record("ags_C").unwrap();
        assert_eq!(r.stage, Stage::Ended, "marked");
        assert_eq!(std::fs::read(dest.join("half-done")).unwrap(), b"wip");
        let leases = files::load_leases(&f.helper.layout.leases()).unwrap();
        assert!(!leases.values().any(|l| l.session == "ags_C"), "revoked");
        // Cleanup run twice: still intact, still marked.
        f.helper.recover().unwrap();
        f.helper.child_ended("ags_C").unwrap();
        assert!(dest.join("half-done").exists());
        assert_eq!(f.helper.child_record("ags_C").unwrap().stage, Stage::Ended);
        // A human's delete, twice.
        let (v, _) = f.ok(json!({"op": "child_dispose", "child": "ags_C"}), vec![]);
        assert_eq!(
            (v["removed"].clone(), v["orphan"].clone()),
            (json!(true), json!(false))
        );
        nothing_left(&f);
        let (v, _) = f.ok(json!({"op": "child_dispose", "child": "ags_C"}), vec![]);
        assert_eq!(v["removed"], false);
        assert_eq!(export_child(&f)["error"], "no_child");
    }

    /// WS-6: directories under the children root that no record claims are
    /// listed, never removed by the helper on its own; a human's delete
    /// names one.
    #[test]
    fn orphans_are_listed_and_never_removed_unasked() {
        let f = fixture();
        ended_clone_child(&f);
        let orphan = f.helper.layout.child("ags_LOST");
        std::fs::create_dir_all(orphan.join("workspace")).unwrap();
        std::fs::write(orphan.join("workspace/x"), "y").unwrap();
        std::fs::create_dir_all(f.helper.layout.children().join("not-a-session")).unwrap();
        let (v, _) = f.ok(json!({"op": "child_list"}), vec![]);
        assert_eq!(v["orphans"], json!(["ags_LOST"]));
        assert_eq!(v["unknown_entries"], 1);
        assert_eq!(v["children"].as_array().unwrap().len(), 1);
        assert_eq!(v["children"][0]["child"], "ags_C");
        assert_eq!(v["children"][0]["live"], false);
        f.helper.recover().unwrap();
        f.ok(json!({"op": "child_list"}), vec![]);
        assert!(orphan.join("workspace/x").exists(), "recovery leaves it");
        let (v, _) = f.ok(json!({"op": "child_dispose", "child": "ags_LOST"}), vec![]);
        assert_eq!(
            (v["removed"].clone(), v["orphan"].clone()),
            (json!(true), json!(true))
        );
        assert!(!orphan.exists());
        assert!(
            f.helper.layout.child("ags_C").exists(),
            "only the one named"
        );
    }

    /// G-137-2's helper half: an Apply a restart interrupted is finished
    /// by the same request, once; an export a restart interrupted is redone;
    /// the units the last helper left are stopped first.
    #[test]
    fn g_137_2_an_interrupted_export_or_apply_is_finished_once() {
        let f = fixture();
        let (_, tip) = ended_clone_child(&f);
        // An export unit still loaded when the helper restarted.
        f.units.units.lock().unwrap().insert(
            review_unit_name("ags_C"),
            (
                crate::units::UnitState {
                    loaded: true,
                    running: true,
                    ..Default::default()
                },
                None,
            ),
        );
        f.helper.recover().unwrap();
        let calls = f.units.calls.lock().unwrap().clone();
        assert!(calls.contains(&"stop storm-child-ags_C-review.service".to_string()));
        assert_eq!(export_child(&f)["ok"], true);
        // The Apply made the branch, then the helper died before it was
        // recorded: the same Apply finishes; nothing else is let through.
        let parent = PathBuf::from(f.workspace());
        let mut r = f.helper.child_record("ags_C").unwrap();
        r.applied = Some(Applied {
            target: "storm/ags_C".into(),
            done: false,
        });
        f.helper.save_child(&r).unwrap();
        let bundle = f
            .helper
            .layout
            .child_review("ags_C")
            .join(crate::review::BUNDLE);
        git(
            &parent,
            &["fetch", "-q", bundle.to_str().unwrap(), "refs/heads/export"],
        );
        git(&parent, &["update-ref", "refs/heads/storm/ags_C", &tip]);
        f.helper.recover().unwrap();
        let v = apply(&f, "storm/ags_C");
        assert_eq!(v["ok"], true, "{v}");
        assert!(
            f.helper
                .child_record("ags_C")
                .unwrap()
                .applied
                .unwrap()
                .done
        );
        assert_eq!(apply(&f, "storm/ags_C")["error"], "branch_exists");
        // Without an interrupted Apply of its own, a branch already there is
        // never taken as this child's.
        let mut r = f.helper.child_record("ags_C").unwrap();
        r.applied = None;
        f.helper.save_child(&r).unwrap();
        assert_eq!(apply(&f, "storm/ags_C")["error"], "branch_exists");
        assert_eq!(f.helper.child_record("ags_C").unwrap().applied, None);
    }

    /// A failed or unparsable export leaves no review behind and says why.
    #[test]
    fn a_failed_export_is_its_reason_and_leaves_no_review() {
        let f = fixture();
        ended_clone_child(&f);
        for (out, code) in [
            (
                r#"{"failed":"unreadable_repository"}"#,
                "unreadable_repository",
            ),
            (r#"{"failed":"workspace_cap"}"#, "workspace_cap"),
            ("garbage", "failed"),
            (r#"{"done":{"mode":"clone"}}"#, "failed"),
        ] {
            *f.units.review_output.lock().unwrap() = Some(out.into());
            assert_eq!(export_child(&f)["error"], code, "{out}");
            assert!(!f.helper.layout.child_review("ags_C").exists());
            assert!(!f.helper.layout.child_export("ags_C").exists());
        }
    }

    fn sent(v: &Value) -> usize {
        serde_json::to_vec(v).unwrap().len() + 1
    }

    /// 139-D2 through the helper: an export at every limit reaches the host
    /// within its read, and says what it left out.
    #[test]
    fn an_export_at_every_limit_is_sent_within_the_hosts_read() {
        let f = fixture();
        ended_clone_child(&f);
        *f.units.review_output.lock().unwrap() =
            Some(crate::review::tests::export_at_every_limit(false));
        let v = export_child(&f);
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert!(
            sent(&v) <= crate::review::CHILD_ANSWER_BUDGET,
            "{}",
            sent(&v)
        );
        assert_eq!(
            (v["more_files"].clone(), v["patch_truncated"].clone()),
            (json!(true), json!(false))
        );
        assert_eq!(v["child"], "ags_C");
        assert_eq!(v["parent"], "ags_P");
        assert_eq!(v["tip"], "a".repeat(40));
        let cached = std::fs::read(f.helper.layout.child_export("ags_C")).unwrap();
        let cached: Value = serde_json::from_slice(&cached).unwrap();
        assert_eq!(
            cached["files"].as_array().unwrap().len(),
            crate::review::MAX_FILES
        );
    }

    /// A listing of more children than the host can read is cut, orphans
    /// first, and says so; its counts are kept.
    #[test]
    fn a_listing_too_long_for_the_hosts_read_is_cut_and_says_so() {
        let f = fixture();
        let long = PathBuf::from(format!("/{}", "\u{1}".repeat(4000)));
        for i in 0..120 {
            let child = format!("ags_C{i:03}");
            std::fs::create_dir_all(f.helper.layout.child(&child)).unwrap();
            f.helper
                .save_child(&ChildRecord {
                    child,
                    parent: "ags_P".into(),
                    parent_workspace: long.clone(),
                    mode: ChildMode::Scratch,
                    stage: Stage::Ended,
                    commit: None,
                    branch: None,
                    submodules: false,
                    large_files: false,
                    applied: None,
                })
                .unwrap();
        }
        for i in 0..50 {
            std::fs::create_dir_all(f.helper.layout.child(&format!("ags_L{i:03}"))).unwrap();
        }
        std::fs::create_dir_all(f.helper.layout.children().join("not-a-session")).unwrap();
        let (v, _) = f.ok(json!({"op": "child_list"}), vec![]);
        assert!(
            sent(&v) <= crate::review::CHILD_ANSWER_BUDGET,
            "{}",
            sent(&v)
        );
        assert_eq!(v["more"], true);
        assert_eq!(v["orphans"], json!([]));
        assert_eq!(v["omitted"]["orphans"], 50);
        let kept = v["children"].as_array().unwrap().len();
        assert!(kept > 0 && kept < 120, "{kept}");
        assert_eq!(v["omitted"]["children"], 120 - kept);
        assert_eq!(v["children"][0]["child"], "ags_C000");
        assert_eq!(v["unknown_entries"], 1);
    }

    #[test]
    fn an_ordinary_listing_is_whole() {
        let f = fixture();
        ended_clone_child(&f);
        let (v, _) = f.ok(json!({"op": "child_list"}), vec![]);
        assert_eq!(v["more"], false);
        assert!(v.get("omitted").is_none());
    }

    /// G-137-3 at unit level: a W3 child's view, every mount of it. The
    /// parent's workspace is bound read-only and nothing under it is
    /// writable; the only writable paths are the child's own.
    #[test]
    fn g_137_3_no_mount_in_a_scratch_childs_view_writes_its_parent() {
        let f = fixture();
        let (_, child_user) = parent_and_child(&f);
        assert_eq!(create(&f)["ok"], true);
        assert_eq!(start_child(&f, &child_user)["ok"], true);
        let specs = f.units.specs.lock().unwrap().clone();
        let child = specs.iter().find(|s| s.session == "ags_C").unwrap();
        let props = child.properties().unwrap();
        let parent = f.workspace();
        assert!(props.contains(&"ProtectSystem=strict".to_string()));
        assert!(props.contains(&format!("BindReadOnlyPaths={parent}")));
        for p in &props {
            let writable = p.starts_with("BindPaths=") || p.starts_with("ReadWritePaths=");
            if writable {
                let path = p.split_once('=').unwrap().1;
                assert!(
                    !Path::new(path).starts_with(&parent) && !Path::new(&parent).starts_with(path),
                    "{p} writes the parent's workspace"
                );
                assert!(
                    path.starts_with(&f.helper.layout.state.display().to_string())
                        || path.starts_with(&f.helper.layout.host_state.display().to_string()),
                    "{p} is not the child's own"
                );
            }
            // Only the session's own /dev/shm is a writable tmpfs (decision 140).
            assert!(
                !p.starts_with("TemporaryFileSystem=")
                    || p.ends_with(":ro")
                    || p.starts_with("TemporaryFileSystem=/dev/shm:"),
                "{p}"
            );
        }
    }

    /// The review unit as data, and as systemd reads it.
    #[test]
    fn the_review_unit_is_the_review_identity_with_nothing_but_its_paths() {
        let spec = ReviewSpec {
            child: "ags_C1".into(),
            parent_group: Some("storm-w002".into()),
            read_only: vec!["/var/lib/storm-runtime-isolation/children/ags_C1/review".into()],
            writable: vec!["/var/lib/storm-runtime/workspaces/app".into()],
            hidden: vec![
                "/var/lib/storm-runtime".into(),
                "/var/lib/storm-runtime-isolation".into(),
            ],
            budget: 1 << 30,
            args: vec!["import".into(), "clone".into()],
        };
        let props = spec.properties().unwrap();
        for want in [
            "User=storm-review",
            "Group=storm-review",
            "SupplementaryGroups=storm-w002",
            "NoNewPrivileges=yes",
            "CapabilityBoundingSet=",
            "PrivateNetwork=yes",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "TemporaryFileSystem=/var/lib/storm-runtime:ro",
            "TemporaryFileSystem=/var/lib/storm-runtime-isolation:ro",
            "BindReadOnlyPaths=/var/lib/storm-runtime-isolation/children/ags_C1/review",
            "BindPaths=/var/lib/storm-runtime/workspaces/app",
        ] {
            assert!(props.iter().any(|p| p == want), "{want} in {props:#?}");
        }
        let writable: Vec<&String> = props
            .iter()
            .filter(|p| p.starts_with("BindPaths=") || p.starts_with("ReadWritePaths="))
            .collect();
        assert_eq!(writable.len(), 1);
        let args: Vec<String> = spec
            .systemd_run_args()
            .unwrap()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--unit=storm-child-ags_C1-review.service".to_string()));
        let tail = &args[args.iter().position(|a| a == "--").unwrap()..];
        assert_eq!(tail, ["--", HELPER_BIN, "child-review", "import", "clone"]);
        let mut export = spec.clone();
        export.parent_group = None;
        let props = export.properties().unwrap();
        assert!(props.contains(&"UMask=0077".to_string()));
        assert!(!props.iter().any(|p| p.starts_with("SupplementaryGroups=")));
        let mut bad = spec.clone();
        bad.writable = vec!["/w/a b".into()];
        assert!(bad.properties().is_err());
        let Some(analyze) = crate::units::tests::which("systemd-analyze") else {
            return;
        };
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("storm-child-ags_C1-review.service");
        std::fs::write(&path, spec.unit_file().unwrap()).unwrap();
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

    /// The host names the parent's workspace at `start`, and it must be
    /// the one the child's was made from.
    #[test]
    fn a_child_starts_only_from_the_workspace_it_was_made_from() {
        let f = fixture();
        let (_, child_user) = parent_and_child(&f);
        assert_eq!(create(&f)["ok"], true);
        std::fs::create_dir(f.root.join("other")).unwrap();
        f.launch("ags_C");
        let (_master, slave) = crate::tty::tests::pty();
        let (v, _) = f.call(
            json!({"op": "start", "session": "ags_C", "user": child_user, "provider": "shell", "workspace": f.root.join("other").display().to_string()}),
            vec![slave],
        );
        assert_eq!(v["error"], "wrong_parent");
    }

    /// The clone unit as data: the child's user, nothing but its own
    /// workspace writable, the parent read-only, no network, no
    /// capability, and the helper's own binary.
    #[test]
    fn the_clone_unit_is_the_childs_user_with_only_its_workspace_writable() {
        let spec = CloneSpec {
            child: "ags_C1".into(),
            user: "storm-s009".into(),
            parent_group: "storm-w002".into(),
            parent: "/var/lib/storm-runtime/workspaces/app".into(),
            dest: "/var/lib/storm-runtime-isolation/children/ags_C1/workspace".into(),
            hidden: vec![
                "/var/lib/storm-runtime".into(),
                "/var/lib/storm-runtime-isolation".into(),
            ],
            budget: 1 << 30,
        };
        let props = spec.properties().unwrap();
        for want in [
            "User=storm-s009",
            "Group=storm-s009",
            "SupplementaryGroups=storm-w002",
            "NoNewPrivileges=yes",
            "CapabilityBoundingSet=",
            "AmbientCapabilities=",
            "PrivateNetwork=yes",
            "RestrictAddressFamilies=AF_UNIX",
            "ProtectSystem=strict",
            "ProtectHome=yes",
            "UMask=0077",
            "LimitFSIZE=1073741824",
            "RuntimeMaxSec=600",
            "TemporaryFileSystem=/var/lib/storm-runtime:ro",
            "TemporaryFileSystem=/var/lib/storm-runtime-isolation:ro",
            "BindReadOnlyPaths=/var/lib/storm-runtime/workspaces/app",
            "BindPaths=/var/lib/storm-runtime-isolation/children/ags_C1/workspace",
        ] {
            assert!(props.iter().any(|p| p == want), "{want} in {props:#?}");
        }
        assert!(
            !props
                .iter()
                .any(|p| p.starts_with("User=root") || p.starts_with("User=storm-runtime"))
        );
        let args: Vec<String> = spec
            .systemd_run_args()
            .unwrap()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--wait".into()) && args.contains(&"--pipe".into()));
        let tail = &args[args.iter().position(|a| a == "--").unwrap()..];
        assert_eq!(
            tail,
            [
                "--",
                HELPER_BIN,
                "child-clone",
                "/var/lib/storm-runtime/workspaces/app",
                "/var/lib/storm-runtime-isolation/children/ags_C1/workspace",
                "1073741824"
            ]
        );
        let mut bad = spec.clone();
        bad.parent = "/w/a b".into();
        assert!(bad.properties().is_err());
        // systemd itself accepts every directive (where it is installed).
        let Some(analyze) = crate::units::tests::which("systemd-analyze") else {
            return;
        };
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("storm-child-ags_C1-clone.service");
        std::fs::write(&path, spec.unit_file().unwrap()).unwrap();
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
    fn the_children_root_overlaps_no_workspace_root() {
        let children = Path::new("/var/lib/storm-runtime-isolation/children");
        assert!(check_root(children, &["/var/lib/storm-runtime/workspaces".into()]).is_ok());
        for bad in [
            "/var/lib/storm-runtime-isolation",
            "/var/lib/storm-runtime-isolation/children/x",
            "/var/lib",
        ] {
            assert!(check_root(children, &[bad.into()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn disk_usage_counts_blocks_and_never_follows_a_link() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("d");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/f"), vec![7u8; 100 << 10]).unwrap();
        let big = t.path().join("big");
        std::fs::write(&big, vec![7u8; 4 << 20]).unwrap();
        std::os::unix::fs::symlink(&big, d.join("link")).unwrap();
        let n = disk_usage(&d).unwrap();
        assert!((100 << 10..1 << 20).contains(&n), "{n}");
        assert_eq!(disk_usage(&t.path().join("missing")).unwrap(), 0);
    }

    /// 139a (F2): a child workspace the measure cannot finish is charged
    /// its whole cap; it never fails every other child's creation.
    #[test]
    fn a_child_nested_past_the_cap_is_charged_its_cap_not_an_error() {
        let f = fixture();
        parent_and_child(&f);
        crate::session::tests::nest(&f.helper.layout.child("ags_OLD"), 300);
        assert_eq!(create(&f)["ok"], true);
        let mut g = fixture();
        parent_and_child(&g);
        crate::session::tests::nest(&g.helper.layout.child("ags_OLD"), 300);
        let charged = g.helper.children_usage().unwrap();
        assert!(charged >= g.helper.caps.child, "{charged}");
        g.helper.caps.children = charged - 1;
        assert_eq!(create(&g)["error"], "workspace_cap", "fail closed");
    }

    /// 139a (F2): an entry a live child removes, or turns into a file or a
    /// link, between the scan's `stat` and its open is skipped.
    #[test]
    fn an_entry_that_changes_during_the_scan_is_skipped() {
        let t = tempfile::tempdir().unwrap();
        let x = t.path().join("x");
        std::fs::create_dir(&x).unwrap();
        let dir = rustix::fs::open(t.path(), files::DIR_FLAGS, Mode::empty()).unwrap();
        let seen = rustix::fs::statat(&dir, c"x", AtFlags::SYMLINK_NOFOLLOW).unwrap();
        assert!(open_sub(&dir, c"x", &seen).unwrap().is_some());
        std::fs::remove_dir(&x).unwrap();
        assert!(open_sub(&dir, c"x", &seen).unwrap().is_none(), "gone");
        std::fs::write(&x, b"").unwrap();
        assert!(open_sub(&dir, c"x", &seen).unwrap().is_none(), "a file");
        std::fs::remove_file(&x).unwrap();
        std::os::unix::fs::symlink(t.path(), &x).unwrap();
        assert!(open_sub(&dir, c"x", &seen).unwrap().is_none(), "a link");
        std::fs::remove_file(&x).unwrap();
        std::fs::create_dir(t.path().join("y")).unwrap();
        std::fs::create_dir(&x).unwrap();
        let other = rustix::fs::statat(&dir, c"y", AtFlags::SYMLINK_NOFOLLOW).unwrap();
        assert!(
            open_sub(&dir, c"x", &other).unwrap().is_none(),
            "another directory"
        );
    }
}
