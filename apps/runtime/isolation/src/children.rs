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
//! end its workspace is kept, root's, marked ended, until 137-3's review
//! identity takes it.

use std::ffi::OsString;
use std::os::fd::OwnedFd;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::files;
use crate::protocol::Refusal;
use crate::serve::Helper;
use crate::session::{Answer, failed};
use crate::units::{STOP_GRACE_SECS, property_safe};

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
/// nothing.
pub fn disk_usage(path: &Path) -> Result<u64> {
    let st = match rustix::fs::lstat(path) {
        Err(rustix::io::Errno::NOENT) => return Ok(0),
        other => other.with_context(|| format!("stat {}", path.display()))?,
    };
    let own = blocks(&st);
    if FileType::from_raw_mode(st.st_mode as _) != FileType::Directory {
        return Ok(own);
    }
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .with_context(|| format!("opening {}", path.display()))?;
    Ok(own + usage_in(&fd, dev_of(&st), 0)?)
}

#[allow(clippy::unnecessary_cast)]
fn blocks(st: &rustix::fs::Stat) -> u64 {
    st.st_blocks as u64 * 512
}

#[allow(clippy::unnecessary_cast)]
fn dev_of(st: &rustix::fs::Stat) -> u64 {
    st.st_dev as u64
}

fn usage_in(dir: &OwnedFd, dev: u64, depth: usize) -> Result<u64> {
    if depth >= MAX_DEPTH {
        bail!("nested deeper than {MAX_DEPTH} directories");
    }
    let mut names = Vec::new();
    for entry in Dir::read_from(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            names.push(name.to_owned());
        }
    }
    let mut total = 0u64;
    for name in names {
        let st = match rustix::fs::statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) => st,
            Err(rustix::io::Errno::NOENT) => continue,
            Err(e) => return Err(e.into()),
        };
        if dev_of(&st) != dev {
            continue;
        }
        total = total.saturating_add(blocks(&st));
        if FileType::from_raw_mode(st.st_mode as _) == FileType::Directory {
            let sub = rustix::fs::openat(
                dir,
                name.as_c_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            total = total.saturating_add(usage_in(&sub, dev, depth + 1)?);
        }
    }
    Ok(total)
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
            p("NoNewPrivileges=yes"),
            p("CapabilityBoundingSet="),
            p("AmbientCapabilities="),
            p("RestrictSUIDSGID=yes"),
            p("ProtectSystem=strict"),
            p("ProtectHome=yes"),
            p("PrivateTmp=yes"),
            p("PrivateDevices=yes"),
            p("PrivateIPC=yes"),
            // A local clone needs no network: a repository that names a
            // remote reaches nothing.
            p("PrivateNetwork=yes"),
            p("RestrictAddressFamilies=AF_UNIX"),
            p("IPAddressDeny=any"),
            p("ProtectProc=invisible"),
            p("ProtectKernelTunables=yes"),
            p("ProtectKernelModules=yes"),
            p("ProtectKernelLogs=yes"),
            p("ProtectControlGroups=yes"),
            p("ProtectClock=yes"),
            p("ProtectHostname=yes"),
            p("LockPersonality=yes"),
            p("MemoryDenyWriteExecute=yes"),
            p("RestrictRealtime=yes"),
            p("RestrictNamespaces=yes"),
            p("KeyringMode=private"),
            p("SystemCallArchitectures=native"),
        ];
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
        let mut out: Vec<OsString> = vec![
            format!("--unit={}", clone_unit_name(&self.child)).into(),
            "--quiet".into(),
            "--no-ask-password".into(),
            "--expand-environment=no".into(),
            "--wait".into(),
            "--pipe".into(),
            "--collect".into(),
        ];
        for property in self.properties()? {
            out.push("-p".into());
            out.push(property.into());
        }
        out.push("--".into());
        out.extend(self.args());
        Ok(out)
    }

    pub fn unit_file(&self) -> Result<String> {
        let mut service = String::from("[Service]\n");
        for property in self.properties()? {
            if !property.starts_with("Description=") {
                service.push_str(&property);
                service.push('\n');
            }
        }
        let exec: Vec<String> = self
            .args()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        service.push_str(&format!("ExecStart={}\n", exec.join(" ")));
        Ok(format!(
            "[Unit]\nDescription=Storm child workspace {}\n\n{service}",
            self.child
        ))
    }
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
    pub(crate) fn child_record(&self, child: &str) -> Option<ChildRecord> {
        let bytes = std::fs::read(self.layout.child_record(child)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save_child(&self, r: &ChildRecord) -> Result<()> {
        let path = self.layout.child_record(&r.child);
        let tmp = path.with_extension("tmp");
        files::write_root_private(&tmp, &serde_json::to_vec(r)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
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
        let used =
            disk_usage(&self.layout.children()).map_err(|e| failed("measuring the children", e))?;
        let estimate = match mode {
            ChildMode::Clone => disk_usage(&workspace.join(".git"))
                .map_err(|e| failed("measuring the parent's repository", e))?,
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
            let made = disk_usage(&self.layout.child(&child))
                .map_err(|e| failed("measuring the child's workspace", e))?;
            if made > budget {
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
    /// user's to reach, since that user goes back to the pool.
    pub(crate) fn child_ended(&self, child: &str) -> Result<()> {
        let Some(mut r) = self.child_record(child) else {
            return Ok(());
        };
        match r.stage {
            Stage::Creating => self.abandon(child),
            Stage::Ready | Stage::Ended => {
                files::rootify(&self.layout.child_workspace(child), self.deps.own.as_ref())?;
                if r.stage == Stage::Ready {
                    r.stage = Stage::Ended;
                    self.save_child(&r)?;
                }
                Ok(())
            }
        }
    }

    /// What a previous helper left half made is abandoned before this one
    /// answers.
    pub(crate) fn recover_children(&self) -> Result<()> {
        let entries = match std::fs::read_dir(self.layout.children()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            other => other?,
        };
        for entry in entries {
            let name = entry?.file_name();
            let Some(child) = name.to_str().filter(|n| crate::policy::session_ok(n)) else {
                continue;
            };
            if self
                .child_record(child)
                .is_some_and(|r| r.stage == Stage::Creating)
            {
                self.abandon(child)?;
            }
        }
        Ok(())
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
    use crate::clone::tests::{hostile_at, repo};
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
        let m = meta(&f, &dest);
        assert_eq!((m.uid, m.mode), (0, 0o700));
        let m = meta(&f, &dest.join(".git"));
        assert_eq!((m.uid, m.mode), (0, 0o700));
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
}
