//! Apply's unprivileged half (137e, 137g): `storm-runtime-isolation
//! child-review`, which the helper runs in a unit of its own as the review
//! identity, never as root, the host account or a session. It prints one
//! line, which the helper checks before any of it reaches the host.
//!
//! - **Export** reads an ended child's workspace as data. A clone's work is
//!   fetched through git's transport into a fresh repository of the
//!   review's own, so nothing of the child's `.git` but its objects comes
//!   across, and every object is checked as it arrives; from there come its
//!   commits since the base, its diff, and a bundle of it. A scratch
//!   directory is listed, never followed.
//! - **Import** writes into the parent's workspace and nowhere else: for a
//!   clone the bundle's objects and one new branch, `storm/<child>`, made
//!   only if no such ref exists (never moved, never the branch the parent
//!   has checked out); for scratch a new directory, made only if no such
//!   name exists. Nothing is merged, and the parent's checked-out branch,
//!   index and working tree are never touched.
//!
//! Nothing the child's or the parent's repository, or an account's git
//! configuration, says may run: the same environment and protections as
//! the child's clone (`clone.rs`), with every hook event switched off.

use std::ffi::{CStr, CString};
use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::Path;

use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use serde_json::{Map, Value, json};

use crate::clone::{GIT, Git, HOME, commit_ok, environment, protections};

/// What an export leaves in the review directory: the bundle an Apply
/// imports from.
pub const BUNDLE: &str = "export.bundle";
const EXPORT_REPO: &str = "export.git";
const EXPORT_REF: &str = "refs/heads/export";
pub const MAX_COMMITS: usize = 200;
pub const MAX_FILES: usize = 1000;
/// The diff the person reads before Apply; a longer one is cut here and
/// marked so.
pub const MAX_PATCH: usize = 256 << 10;
const MAX_SUBJECT: usize = 200;
const MAX_PATH: usize = 4096;
const MAX_DEPTH: usize = 256;
const MAX_WORKTREES: usize = 256;
/// A W3 Apply copies into this first, then renames it into place, so a
/// half-made copy never has the name the person chose.
pub const STAGING_PREFIX: &str = ".storm-apply-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    WorkspaceCap,
    Unreadable,
    BranchExists,
    TargetExists,
    Failed,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Failure::WorkspaceCap => "workspace_cap",
            Failure::Unreadable => "unreadable_repository",
            Failure::BranchExists => "branch_exists",
            Failure::TargetExists => "target_exists",
            Failure::Failed => "failed",
        }
    }

    pub const CODES: [&'static str; 5] = [
        "workspace_cap",
        "unreadable_repository",
        "branch_exists",
        "target_exists",
        "failed",
    ];
}

fn git_for<'a>(repo: &Path, home: &Path, git: &'a Path) -> Git<'a> {
    Git {
        git,
        env: environment(home),
        config: protections(repo),
    }
}

/// Up to `max` bytes of printable text, cut at a character.
fn clip(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_control() { '?' } else { c };
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

fn commits(listing: &[u8]) -> Vec<Value> {
    listing
        .split(|&b| b == 0x1e)
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let record = record.trim_start_matches('\n');
            let (id, subject) = record.split_once('\x1f')?;
            commit_ok(id).then(|| json!({"id": id, "subject": clip(subject, MAX_SUBJECT)}))
        })
        .collect()
}

/// `git diff --numstat -z`: `added\tremoved\tpath\0`, or for a rename
/// `added\tremoved\t\0old\0new\0`. A binary file has `-` for both counts.
fn numstat(listing: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut fields = listing.split(|&b| b == 0).peekable();
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(record);
        let mut parts = text.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            let _old = fields.next();
            fields
                .next()
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .unwrap_or_default()
        } else {
            path.to_string()
        };
        let count = |s: &str| s.parse::<u64>().map_or(Value::Null, Value::from);
        out.push(json!({"path": clip(&path, MAX_PATH), "added": count(added), "removed": count(removed)}));
    }
    out
}

/// A W2 child's work, read from its ended workspace `child` into `dir`,
/// the review's own: what it committed since `base`, the diff, and a
/// bundle of its `HEAD` for Apply.
pub fn export_clone(
    git: &Path,
    child: &Path,
    dir: &Path,
    base: &str,
    budget: u64,
    home: &Path,
) -> Result<Value, Failure> {
    let repo = dir.join(EXPORT_REPO);
    let bundle = dir.join(BUNDLE);
    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_file(&bundle);
    std::fs::create_dir(&repo).map_err(|_| Failure::Failed)?;
    let g = git_for(child, home, git);
    g.run(&repo, &["init", "--quiet", "--bare", "--template="])
        .map_err(|()| Failure::Failed)?;
    let url = format!("file://{}", child.display());
    g.run(
        &repo,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--",
            &url,
            &format!("+HEAD:{EXPORT_REF}"),
        ],
    )
    .map_err(|()| Failure::Unreadable)?;
    let tip = g
        .run(
            &repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{EXPORT_REF}^{{commit}}"),
            ],
        )
        .map_err(|()| Failure::Unreadable)?;
    let tip = String::from_utf8_lossy(&tip).trim().to_string();
    if !commit_ok(&tip) {
        return Err(Failure::Unreadable);
    }
    if crate::children::disk_usage(dir).map_err(|_| Failure::Failed)? > budget {
        return Err(Failure::WorkspaceCap);
    }
    // A child that rewrote its history may no longer have the commit it
    // started from; then its work is shown from its tip alone.
    let base_found = commit_ok(base)
        && g.run(&repo, &["merge-base", "--is-ancestor", base, &tip])
            .is_ok();
    let range = if base_found {
        format!("{base}..{tip}")
    } else {
        tip.clone()
    };
    let (log, _) = g
        .run_limited(
            &repo,
            &[
                "log",
                "--no-color",
                "--no-show-signature",
                "--format=%H%x1f%s%x1e",
                &format!("--max-count={}", MAX_COMMITS + 1),
                &range,
            ],
            (MAX_COMMITS + 1) * (MAX_SUBJECT * 4 + 128),
        )
        .map_err(|()| Failure::Unreadable)?;
    let mut list = commits(&log);
    let more_commits = list.len() > MAX_COMMITS;
    list.truncate(MAX_COMMITS);
    let (mut files, mut more_files, mut patch, mut truncated) = (Vec::new(), false, None, false);
    if base_found {
        let diff = [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--find-renames",
        ];
        let (stat, cut) = g
            .run_limited(
                &repo,
                &[&diff[..], &["--numstat", "-z", base, &tip]].concat(),
                (MAX_FILES + 1) * (MAX_PATH + 64),
            )
            .map_err(|()| Failure::Unreadable)?;
        files = numstat(&stat);
        more_files = cut || files.len() > MAX_FILES;
        files.truncate(MAX_FILES);
        let (text, cut) = g
            .run_limited(
                &repo,
                &[&diff[..], &["--patch", base, &tip]].concat(),
                MAX_PATCH,
            )
            .map_err(|()| Failure::Unreadable)?;
        truncated = cut;
        let text = String::from_utf8_lossy(&text);
        patch = Some(clip_raw(&text, MAX_PATCH));
    }
    let bundle_arg = bundle.to_str().ok_or(Failure::Failed)?;
    g.run(
        &repo,
        &["bundle", "create", "--quiet", bundle_arg, EXPORT_REF],
    )
    .map_err(|()| Failure::Failed)?;
    let _ = std::fs::remove_dir_all(&repo);
    if crate::children::disk_usage(dir).map_err(|_| Failure::Failed)? > budget {
        return Err(Failure::WorkspaceCap);
    }
    Ok(json!({
        "mode": "clone",
        "tip": tip,
        "base": base,
        "base_found": base_found,
        "commits": list,
        "more_commits": more_commits,
        "files": files,
        "more_files": more_files,
        "patch": patch,
        "patch_truncated": truncated,
    }))
}

/// A diff's text as it is, but for its cut: at a character.
fn clip_raw(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn open_dir(path: &Path) -> Result<OwnedFd, Failure> {
    rustix::fs::open(path, crate::files::DIR_FLAGS, Mode::empty()).map_err(|_| Failure::Failed)
}

fn open_dir_at(dir: &OwnedFd, name: &CStr) -> Result<OwnedFd, Failure> {
    rustix::fs::openat(dir, name, crate::files::DIR_FLAGS, Mode::empty())
        .map_err(|_| Failure::Failed)
}

fn names(dir: &OwnedFd) -> Result<Vec<CString>, Failure> {
    crate::files::dir_names(dir).map_err(|_| Failure::Failed)
}

/// What a scratch child left and an Apply would copy: regular files and
/// directories. Links, devices, sockets and any `.git` are never copied
/// (a repository's hooks and configuration would be the child's in the
/// parent's workspace), only counted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tally {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub skipped: u64,
}

fn skipped_name(name: &CStr) -> bool {
    name.to_bytes() == b".git"
}

fn walk_scratch(
    dir: &OwnedFd,
    rel: &str,
    depth: usize,
    tally: &mut Tally,
    listed: &mut Vec<Value>,
) -> Result<(), Failure> {
    if depth >= MAX_DEPTH {
        return Err(Failure::Failed);
    }
    for name in names(dir)? {
        let st = rustix::fs::statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| Failure::Failed)?;
        let path = format!("{rel}{}", name.to_string_lossy());
        match FileType::from_raw_mode(st.st_mode as _) {
            _ if skipped_name(&name) => tally.skipped += 1,
            FileType::Directory => {
                tally.dirs += 1;
                let sub = open_dir_at(dir, &name)?;
                walk_scratch(&sub, &format!("{path}/"), depth + 1, tally, listed)?;
            }
            FileType::RegularFile => {
                tally.files += 1;
                tally.bytes = tally.bytes.saturating_add(st.st_size as u64);
                if listed.len() <= MAX_FILES {
                    listed.push(json!({"path": clip(&path, MAX_PATH), "size": st.st_size as u64}));
                }
            }
            _ => tally.skipped += 1,
        }
    }
    Ok(())
}

/// A W3 child's scratch directory, listed.
pub fn export_scratch(child: &Path) -> Result<Value, Failure> {
    let dir = open_dir(child)?;
    let (mut tally, mut listed) = (Tally::default(), Vec::new());
    walk_scratch(&dir, "", 0, &mut tally, &mut listed)?;
    let more_files = listed.len() > MAX_FILES;
    listed.truncate(MAX_FILES);
    Ok(json!({
        "mode": "scratch",
        "files": listed,
        "more_files": more_files,
        "file_count": tally.files,
        "bytes": tally.bytes,
        "skipped": tally.skipped,
    }))
}

/// Whether `branch` is what the parent's `HEAD`, or a linked worktree's,
/// has checked out (an unborn one included): creating it would change the
/// parent's checked-out branch.
fn checked_out(dot: &Path, branch: &str) -> bool {
    let want = format!("ref: refs/heads/{branch}");
    let names = |path: &Path| {
        let Ok(f) = std::fs::File::open(path) else {
            return false;
        };
        let mut text = String::new();
        let _ = f.take(4096).read_to_string(&mut text);
        text.trim() == want
    };
    if names(&dot.join("HEAD")) {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dot.join("worktrees")) else {
        return false;
    };
    entries
        .take(MAX_WORKTREES)
        .flatten()
        .any(|e| names(&e.path().join("HEAD")))
}

/// A W2 Apply: the bundle's objects into the parent's repository, then
/// `refs/heads/<branch>` created at `tip`, only if it does not exist.
/// Asked again after it was made, it answers that it was.
pub fn import_clone(
    git: &Path,
    parent: &Path,
    bundle: &Path,
    branch: &str,
    tip: &str,
    home: &Path,
) -> Result<Value, Failure> {
    let dot = parent.join(".git");
    if !dot.symlink_metadata().is_ok_and(|m| m.is_dir()) || !commit_ok(tip) {
        return Err(Failure::Unreadable);
    }
    if checked_out(&dot, branch) {
        return Err(Failure::BranchExists);
    }
    let g = git_for(parent, home, git);
    let git_dir = format!("--git-dir={}", dot.display());
    let reference = format!("refs/heads/{branch}");
    let existing = |g: &Git| {
        g.run(
            &dot,
            &[
                &git_dir,
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{reference}^{{commit}}"),
            ],
        )
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
    };
    match existing(&g) {
        Some(at) if at == tip => {
            return Ok(json!({"branch": branch, "commit": tip, "already": true}));
        }
        Some(_) => return Err(Failure::BranchExists),
        None => {}
    }
    let bundle = bundle.to_str().ok_or(Failure::Failed)?;
    g.run(
        &dot,
        &[
            &git_dir,
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--",
            bundle,
            EXPORT_REF,
        ],
    )
    .map_err(|()| Failure::Unreadable)?;
    g.run(
        &dot,
        &[&git_dir, "cat-file", "-e", &format!("{tip}^{{commit}}")],
    )
    .map_err(|()| Failure::Unreadable)?;
    // Created only where nothing is: an all-zero old value is "must not
    // exist", checked under git's own ref lock.
    let zero = "0".repeat(tip.len());
    let made = g.run(
        &dot,
        &[
            &git_dir,
            "update-ref",
            "-m",
            "storm: apply a child's work (decision 137e)",
            &reference,
            tip,
            &zero,
        ],
    );
    if made.is_err() {
        return Err(if existing(&g).is_some() {
            Failure::BranchExists
        } else {
            Failure::Failed
        });
    }
    Ok(json!({"branch": branch, "commit": tip, "already": false}))
}

fn copy_tree(src: &OwnedFd, dst: &OwnedFd, depth: usize, tally: &mut Tally) -> Result<(), Failure> {
    if depth >= MAX_DEPTH {
        return Err(Failure::Failed);
    }
    for name in names(src)? {
        let st = rustix::fs::statat(src, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| Failure::Failed)?;
        match FileType::from_raw_mode(st.st_mode as _) {
            _ if skipped_name(&name) => tally.skipped += 1,
            FileType::Directory => {
                rustix::fs::mkdirat(dst, name.as_c_str(), Mode::from_raw_mode(0o770))
                    .map_err(|_| Failure::Failed)?;
                let (s, d) = (open_dir_at(src, &name)?, open_dir_at(dst, &name)?);
                tally.dirs += 1;
                copy_tree(&s, &d, depth + 1, tally)?;
            }
            FileType::RegularFile => {
                let from = rustix::fs::openat(
                    src,
                    name.as_c_str(),
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|_| Failure::Failed)?;
                let seen = rustix::fs::fstat(&from).map_err(|_| Failure::Failed)?;
                if FileType::from_raw_mode(seen.st_mode as _) != FileType::RegularFile {
                    tally.skipped += 1;
                    continue;
                }
                let mode = if seen.st_mode & 0o100 != 0 {
                    0o770
                } else {
                    0o660
                };
                let to = rustix::fs::openat(
                    dst,
                    name.as_c_str(),
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::from_raw_mode(mode),
                )
                .map_err(|_| Failure::Failed)?;
                let (mut from, mut to) = (std::fs::File::from(from), std::fs::File::from(to));
                let n = std::io::copy(&mut from, &mut to).map_err(|_| Failure::Failed)?;
                tally.files += 1;
                tally.bytes = tally.bytes.saturating_add(n);
            }
            _ => tally.skipped += 1,
        }
    }
    Ok(())
}

/// A W3 Apply: `child`'s scratch files copied into a new directory `name`
/// in the parent's workspace. Refused if the name exists; nothing is
/// overwritten, and a half-made copy never has that name.
pub fn import_scratch(
    scratch: &Path,
    parent: &Path,
    child: &str,
    name: &str,
) -> Result<Value, Failure> {
    if !crate::policy::directory_name_ok(name) || !crate::policy::session_ok(child) {
        return Err(Failure::Failed);
    }
    let into = open_dir(parent)?;
    let target = CString::new(name).map_err(|_| Failure::Failed)?;
    let staging = CString::new(format!("{STAGING_PREFIX}{child}")).map_err(|_| Failure::Failed)?;
    if rustix::fs::statat(&into, target.as_c_str(), AtFlags::SYMLINK_NOFOLLOW).is_ok() {
        return Err(Failure::TargetExists);
    }
    if let Ok(st) = rustix::fs::statat(&into, staging.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        // What an interrupted Apply of this child left is its own; anything
        // else under that name is the parent's, and stays.
        if st.st_uid != rustix::process::geteuid().as_raw()
            || FileType::from_raw_mode(st.st_mode as _) != FileType::Directory
        {
            return Err(Failure::TargetExists);
        }
        std::fs::remove_dir_all(parent.join(staging.to_string_lossy().as_ref()))
            .map_err(|_| Failure::Failed)?;
    }
    rustix::fs::mkdirat(&into, staging.as_c_str(), Mode::from_raw_mode(0o770))
        .map_err(|_| Failure::Failed)?;
    let from = open_dir(scratch)?;
    let to = open_dir_at(&into, &staging)?;
    let mut tally = Tally::default();
    let copied = copy_tree(&from, &to, 0, &mut tally);
    let placed = copied.and_then(|()| {
        rustix::fs::renameat_with(
            &into,
            staging.as_c_str(),
            &into,
            target.as_c_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| {
            if e == rustix::io::Errno::EXIST {
                Failure::TargetExists
            } else {
                Failure::Failed
            }
        })
    });
    if let Err(f) = placed {
        let _ = std::fs::remove_dir_all(parent.join(staging.to_string_lossy().as_ref()));
        return Err(f);
    }
    Ok(json!({
        "directory": parent.join(name).display().to_string(),
        "files": tally.files,
        "dirs": tally.dirs,
        "bytes": tally.bytes,
        "skipped": tally.skipped,
    }))
}

/// The most the helper sends the host for one child answer: the host reads
/// at most 2 MiB (`MAX_CHILD_RESPONSE` in storm-runtime's `isolation.rs`).
pub const CHILD_ANSWER_BUDGET: usize = (2 << 20) - (64 << 10);

/// Room left for the flags and counts a trim adds.
const TRIM_ROOM: usize = 256;

fn serialized<T: serde::Serialize + ?Sized>(v: &T) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// Fits `answer`, as `protocol::ok_with(op, …)` puts it on the wire, in
/// `budget` bytes (139-D2): files go first, then the patch's tail, then
/// commits, each marked by its flag and counted under `omitted`. Nothing
/// else is cut; false when even that is too long.
pub fn fit(answer: &mut Map<String, Value>, op: crate::protocol::Op, budget: usize) -> bool {
    let envelope = crate::protocol::ok_with(op, Map::new()).len();
    let budget = budget.saturating_sub(envelope);
    let mut total = serialized(answer);
    if total <= budget {
        return true;
    }
    let target = budget.saturating_sub(TRIM_ROOM);
    let mut omitted = Map::new();
    let files = drop_tail(answer, "files", &mut total, target);
    if files > 0 {
        answer.insert("more_files".into(), true.into());
        omitted.insert("files".into(), files.into());
    }
    if total > target
        && let Some(Value::String(patch)) = answer.get_mut("patch")
        && !patch.is_empty()
    {
        let before = serialized(patch.as_str());
        let room = before.saturating_sub(total - target);
        let cut = longest_prefix(patch, room);
        omitted.insert("patch_bytes".into(), (patch.len() - cut).into());
        patch.truncate(cut);
        total -= before - serialized(patch.as_str());
        answer.insert("patch_truncated".into(), true.into());
    }
    let commits = drop_tail(answer, "commits", &mut total, target);
    if commits > 0 {
        answer.insert("more_commits".into(), true.into());
        omitted.insert("commits".into(), commits.into());
    }
    trimmed(answer, omitted, budget)
}

/// `child_list`'s answer in `budget` (139-D2): orphans go first, then
/// children; `more` says whether any did.
pub fn fit_listing(
    answer: &mut Map<String, Value>,
    op: crate::protocol::Op,
    budget: usize,
) -> bool {
    let envelope = crate::protocol::ok_with(op, Map::new()).len();
    let budget = budget.saturating_sub(envelope);
    answer.insert("more".into(), false.into());
    let mut total = serialized(answer);
    if total <= budget {
        return true;
    }
    let target = budget.saturating_sub(TRIM_ROOM);
    let mut omitted = Map::new();
    for key in ["orphans", "children"] {
        let n = drop_tail(answer, key, &mut total, target);
        if n > 0 {
            omitted.insert(key.into(), n.into());
        }
    }
    answer.insert("more".into(), true.into());
    trimmed(answer, omitted, budget)
}

fn trimmed(answer: &mut Map<String, Value>, omitted: Map<String, Value>, budget: usize) -> bool {
    if !omitted.is_empty() {
        answer.insert("omitted".into(), omitted.into());
    }
    serialized(answer) <= budget
}

/// Drops `key`'s items from its end until `total` is within `target`; how
/// many it dropped.
fn drop_tail(
    answer: &mut Map<String, Value>,
    key: &str,
    total: &mut usize,
    target: usize,
) -> usize {
    let Some(Value::Array(items)) = answer.get_mut(key) else {
        return 0;
    };
    let mut dropped = 0;
    while *total > target
        && let Some(last) = items.pop()
    {
        let comma = usize::from(!items.is_empty());
        *total -= serialized(&last) + comma;
        dropped += 1;
    }
    dropped
}

/// The longest prefix of `text`, cut at a character, that serializes in
/// `room` bytes.
fn longest_prefix(text: &str, room: usize) -> usize {
    let floor = |mut i: usize| {
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (mut lo, mut hi) = (0, text.len());
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if serialized(&text[..floor(mid)]) <= room {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    floor(lo)
}

pub fn line(result: &Result<Value, Failure>) -> String {
    match result {
        Ok(v) => json!({"done": v}).to_string(),
        Err(f) => json!({"failed": f.code()}).to_string(),
    }
}

fn str_field<'a>(v: &'a Map<String, Value>, k: &str, max: usize) -> Option<&'a str> {
    v.get(k)?.as_str().filter(|s| s.len() <= max)
}

fn count(v: &Map<String, Value>, k: &str) -> Option<u64> {
    v.get(k)?.as_u64()
}

fn flag(v: &Map<String, Value>, k: &str) -> Option<bool> {
    v.get(k)?.as_bool()
}

fn list<'a>(v: &'a Map<String, Value>, k: &str, max: usize) -> Option<&'a Vec<Value>> {
    v.get(k)?.as_array().filter(|a| a.len() <= max)
}

fn parse_files(v: &Map<String, Value>, sized: bool) -> Option<Vec<Value>> {
    list(v, "files", MAX_FILES)?
        .iter()
        .map(|f| {
            let f = f.as_object()?;
            let path = str_field(f, "path", MAX_PATH)?;
            if sized {
                Some(json!({"path": path, "size": count(f, "size")?}))
            } else {
                let n = |k: &str| match f.get(k) {
                    Some(Value::Null) => Some(Value::Null),
                    Some(Value::Number(n)) => n.as_u64().map(Value::from),
                    _ => None,
                };
                Some(json!({"path": path, "added": n("added")?, "removed": n("removed")?}))
            }
        })
        .collect()
}

/// The last line an export printed, rebuilt from only the fields an export
/// has, each of its shape; anything else is no answer.
pub fn parse_export(stdout: &str) -> Option<Result<Value, &'static str>> {
    let v = last_line(stdout)?;
    let v = match done(&v)? {
        Ok(v) => v,
        Err(code) => return Some(Err(code)),
    };
    match str_field(v, "mode", 16)? {
        "clone" => {
            let tip = str_field(v, "tip", 64).filter(|t| commit_ok(t))?;
            let base = str_field(v, "base", 64)?;
            let commits: Option<Vec<Value>> = list(v, "commits", MAX_COMMITS)?
                .iter()
                .map(|c| {
                    let c = c.as_object()?;
                    let id = str_field(c, "id", 64).filter(|i| commit_ok(i))?;
                    let subject = str_field(c, "subject", MAX_SUBJECT)?;
                    Some(json!({"id": id, "subject": subject}))
                })
                .collect();
            let patch = match v.get("patch")? {
                Value::Null => Value::Null,
                Value::String(p) if p.len() <= MAX_PATCH => Value::String(p.clone()),
                _ => return None,
            };
            Some(Ok(json!({
                "mode": "clone",
                "tip": tip,
                "base": base,
                "base_found": flag(v, "base_found")?,
                "commits": commits?,
                "more_commits": flag(v, "more_commits")?,
                "files": parse_files(v, false)?,
                "more_files": flag(v, "more_files")?,
                "patch": patch,
                "patch_truncated": flag(v, "patch_truncated")?,
            })))
        }
        "scratch" => Some(Ok(json!({
            "mode": "scratch",
            "files": parse_files(v, true)?,
            "more_files": flag(v, "more_files")?,
            "file_count": count(v, "file_count")?,
            "bytes": count(v, "bytes")?,
            "skipped": count(v, "skipped")?,
        }))),
        _ => None,
    }
}

/// The last line an import printed, likewise.
pub fn parse_import(stdout: &str) -> Option<Result<Value, &'static str>> {
    let v = last_line(stdout)?;
    let v = match done(&v)? {
        Ok(v) => v,
        Err(code) => return Some(Err(code)),
    };
    if let Some(branch) = v.get("branch") {
        let branch = branch.as_str().filter(|b| crate::clone::branch_ok(b))?;
        let commit = str_field(v, "commit", 64).filter(|c| commit_ok(c))?;
        return Some(Ok(
            json!({"branch": branch, "commit": commit, "already": flag(v, "already")?}),
        ));
    }
    let directory = str_field(v, "directory", MAX_PATH).filter(|d| d.starts_with('/'))?;
    Some(Ok(json!({
        "directory": directory,
        "files": count(v, "files")?,
        "dirs": count(v, "dirs")?,
        "bytes": count(v, "bytes")?,
        "skipped": count(v, "skipped")?,
    })))
}

fn last_line(stdout: &str) -> Option<Value> {
    let line = stdout.lines().rev().find(|l| !l.trim().is_empty())?;
    serde_json::from_str(line).ok()
}

fn done(v: &Value) -> Option<Result<&Map<String, Value>, &'static str>> {
    let v = v.as_object()?;
    if let Some(code) = v.get("failed") {
        let code = code.as_str()?;
        return Failure::CODES.into_iter().find(|c| *c == code).map(Err);
    }
    v.get("done")?.as_object().map(Ok)
}

/// `storm-runtime-isolation child-review …`, which only the helper's review
/// unit runs.
pub fn main(args: &[String]) -> std::process::ExitCode {
    if rustix::process::geteuid().is_root() {
        eprintln!("storm-runtime-isolation: a child's review never runs as root");
        return std::process::ExitCode::FAILURE;
    }
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let (git, home) = (Path::new(GIT), Path::new(HOME));
    let result = match a[..] {
        ["export", "clone", child, dir, base, budget] => {
            let Ok(budget) = budget.parse::<u64>() else {
                return std::process::ExitCode::from(2);
            };
            export_clone(git, Path::new(child), Path::new(dir), base, budget, home)
        }
        ["export", "scratch", child] => export_scratch(Path::new(child)),
        ["import", "clone", parent, bundle, branch, tip] => {
            import_clone(git, Path::new(parent), Path::new(bundle), branch, tip, home)
        }
        ["import", "scratch", scratch, parent, child, name] => {
            import_scratch(Path::new(scratch), Path::new(parent), child, name)
        }
        _ => {
            eprintln!("usage: storm-runtime-isolation child-review export|import …");
            return std::process::ExitCode::from(2);
        }
    };
    println!("{}", line(&result));
    std::process::ExitCode::SUCCESS
}

/// The review unit's own arguments, as `main` reads them.
pub fn export_args(
    mode: crate::children::ChildMode,
    child: &Path,
    dir: &Path,
    base: &str,
    budget: u64,
) -> Vec<String> {
    match mode {
        crate::children::ChildMode::Clone => vec![
            "export".into(),
            "clone".into(),
            path(child),
            path(dir),
            base.into(),
            budget.to_string(),
        ],
        crate::children::ChildMode::Scratch => vec!["export".into(), "scratch".into(), path(child)],
    }
}

pub fn import_clone_args(parent: &Path, bundle: &Path, branch: &str, tip: &str) -> Vec<String> {
    vec![
        "import".into(),
        "clone".into(),
        path(parent),
        path(bundle),
        branch.into(),
        tip.into(),
    ]
}

pub fn import_scratch_args(scratch: &Path, parent: &Path, child: &str, name: &str) -> Vec<String> {
    vec![
        "import".into(),
        "scratch".into(),
        path(scratch),
        path(parent),
        child.into(),
        name.into(),
    ]
}

fn path(p: &Path) -> String {
    p.display().to_string()
}

/// Real git as this test's user: everything an export or an Apply does but
/// the identity switch and the unit's view, which need root (the codebox
/// steps in decision 137).
#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::clone::tests::{HOOKS, Plants, config_hooks, git, git_with, repo};
    use std::collections::BTreeMap;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::PathBuf;

    /// Something a repository's own configuration can make git run while
    /// it is read or written: every hook, in `.git/hooks`, a hooks path
    /// and its configuration; fsmonitor; filters, diff and merge drivers
    /// and textconv named by `.gitattributes`; an external diff, a pager,
    /// an editor; an included file; alternates with an alternate-refs
    /// command; a pack-objects hook; an SSH command, askpass, credential
    /// helper and gpg.
    pub fn plant(plants: &Plants, repo: &Path, prefix: &str) {
        let dot = repo.join(".git");
        plants.hooks(&dot.join("hooks"), &format!("{prefix}-hook"));
        let hooks_path = plants.base.join(format!("{prefix}-hooks-path"));
        plants.hooks(&hooks_path, &format!("{prefix}-hooks-path"));
        let e = |n: &str| plants.evil(&format!("{prefix}-{n}")).display().to_string();
        let alternate = plants.base.join(format!("{prefix}-alternate"));
        if !alternate.exists() {
            repo_at(&alternate, &[("alt.txt", "alternate\n")]);
        }
        std::fs::create_dir_all(dot.join("objects/info")).unwrap();
        std::fs::write(
            dot.join("objects/info/alternates"),
            format!("{}\n", alternate.join(".git/objects").display()),
        )
        .unwrap();
        let included = plants.base.join(format!("{prefix}-included.config"));
        std::fs::write(
            &included,
            format!("[diff]\n\texternal = {}\n", e("included-diff")),
        )
        .unwrap();
        let mut config = std::fs::read_to_string(dot.join("config")).unwrap();
        config.push_str(&format!(
            "[core]\n\thooksPath = {hooks}\n\tfsmonitor = {fsmonitor}\n\talternateRefsCommand = {alt}\n\
             \tsshCommand = {ssh}\n\taskPass = {askpass}\n\tpager = {pager}\n\teditor = {editor}\n\
             [include]\n\tpath = {included}\n\
             [diff]\n\texternal = {external}\n\
             [diff \"evil\"]\n\ttextconv = {textconv}\n\tcommand = {diffcmd}\n\
             [filter \"evil\"]\n\tsmudge = {smudge} smudge\n\tclean = {clean} clean\n\
             [merge \"evil\"]\n\tdriver = {merge}\n\
             [uploadpack]\n\tpackObjectsHook = {packhook}\n\
             [credential]\n\thelper = !{cred}\n\
             [gpg]\n\tprogram = {gpg}\n\
             [log]\n\tshowSignature = true\n",
            hooks = hooks_path.display(),
            fsmonitor = e("fsmonitor"),
            alt = e("alternate-refs"),
            ssh = e("ssh"),
            askpass = e("askpass"),
            pager = e("pager"),
            editor = e("editor"),
            included = included.display(),
            external = e("diff-external"),
            textconv = e("textconv"),
            diffcmd = e("diff-command"),
            smudge = e("smudge"),
            clean = e("clean"),
            merge = e("merge"),
            packhook = e("pack-objects-hook"),
            cred = e("credential"),
            gpg = e("gpg"),
        ));
        config.push_str(&config_hooks(plants, &format!("{prefix}-config-hook")));
        std::fs::write(dot.join("config"), config).unwrap();
    }

    fn repo_at(dir: &Path, files: &[(&str, &str)]) -> String {
        repo(dir, files)
    }

    pub struct Child {
        pub plants: Plants,
        pub child: PathBuf,
        pub base: String,
        pub tip: String,
        pub home: PathBuf,
    }

    /// A child's clone that committed work on top of where it started, with
    /// everything `plant` names in its repository and an account's.
    pub fn hostile_child(t: &Path) -> Child {
        let plants = Plants::new(&t.join("plants"));
        let child = t.join("child");
        let base = repo(
            &child,
            &[
                ("a.txt", "one\n"),
                ("gone.txt", "old\n"),
                (".gitattributes", "* diff=evil filter=evil merge=evil\n"),
            ],
        );
        std::fs::write(child.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(child.join("new.txt"), "new\n").unwrap();
        std::fs::write(child.join("bin.dat"), [0u8, 1, 2, 3, 0, 255]).unwrap();
        std::fs::rename(child.join("gone.txt"), child.join("moved.txt")).unwrap();
        git(&child, &["add", "-A"]);
        git(&child, &["commit", "-q", "-m", "the child's work"]);
        git(&child, &["commit", "-q", "--allow-empty", "-m", "and more"]);
        let tip = git(&child, &["rev-parse", "HEAD"]);
        plant(&plants, &child, "child");
        // Uncommitted work is not the child's to hand over (137e).
        std::fs::write(child.join("a.txt"), "uncommitted\n").unwrap();
        std::fs::write(child.join("untracked.txt"), "x").unwrap();
        let home = t.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[core]\n\thooksPath = {}\n\tfsmonitor = {}\n[diff]\n\texternal = {}\n",
                plants.base.join("child-hooks-path").display(),
                plants.evil("account-fsmonitor").display(),
                plants.evil("account-diff").display()
            ),
        )
        .unwrap();
        Child {
            plants,
            child,
            base,
            tip,
            home,
        }
    }

    /// The positive control: each plant is live under plain git, so a test
    /// that finds none of them ran says something.
    #[test]
    fn the_plants_in_a_childs_and_a_parents_repository_are_live() {
        let t = tempfile::tempdir().unwrap();
        let c = hostile_child(t.path());
        let home = [("HOME", c.home.as_path())];
        git_with(&c.child, &["status", "--porcelain"], &home);
        git_with(&c.child, &["log", "-p", "-1"], &home);
        git_with(&c.child, &["diff", &c.base, "HEAD"], &home);
        git_with(
            &c.child,
            &["commit", "-q", "--allow-empty", "-m", "control"],
            &home,
        );
        let ran = c.plants.ran();
        for want in [
            "child-fsmonitor",
            "child-diff-command",
            "child-hooks-path-pre-commit",
            "child-hooks-path-reference-transaction",
            "child-config-hook-pre-commit",
            "child-config-hook-reference-transaction",
            "child-clean",
        ] {
            assert!(ran.contains(&want.to_string()), "{want} in {ran:?}");
        }
        c.plants.clear();
        // The parent's side: a plain fetch of a bundle and a ref made there.
        let parent = t.path().join("parent");
        repo(&parent, &[("p.txt", "p\n")]);
        plant(&c.plants, &parent, "parent");
        let bundle = t.path().join("plain.bundle");
        git(
            &c.child,
            &["bundle", "create", "-q", bundle.to_str().unwrap(), "HEAD"],
        );
        git_with(
            &parent,
            &["fetch", "-q", bundle.to_str().unwrap(), "HEAD"],
            &home,
        );
        git_with(&parent, &["update-ref", "refs/heads/plain", &c.tip], &home);
        let ran = c.plants.ran();
        for want in [
            "parent-alternate-refs",
            "parent-hooks-path-reference-transaction",
            "parent-config-hook-reference-transaction",
        ] {
            assert!(ran.contains(&want.to_string()), "{want} in {ran:?}");
        }
    }

    const BIG: u64 = 1 << 30;

    fn export(c: &Child, dir: &Path) -> Value {
        std::fs::create_dir_all(dir).unwrap();
        export_clone(Path::new(GIT), &c.child, dir, &c.base, BIG, &c.home).unwrap()
    }

    /// WT-3's export half: a hostile child's repository is read as data.
    /// Nothing it or the account configures runs; its commits since the
    /// base, its diff and a bundle of its tip come out, and its
    /// uncommitted work does not.
    #[test]
    fn wt_3_an_export_runs_nothing_from_a_hostile_child() {
        let t = tempfile::tempdir().unwrap();
        let c = hostile_child(t.path());
        let dir = t.path().join("review");
        let v = export(&c, &dir);
        assert_eq!(c.plants.ran(), Vec::<String>::new());
        assert_eq!(v["tip"], c.tip);
        assert_eq!(v["base_found"], true);
        let subjects: Vec<&str> = v["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["subject"].as_str().unwrap())
            .collect();
        assert_eq!(subjects, ["and more", "the child's work"]);
        let mut files: BTreeMap<String, Value> = BTreeMap::new();
        for f in v["files"].as_array().unwrap() {
            files.insert(f["path"].as_str().unwrap().into(), f.clone());
        }
        assert_eq!(
            files.keys().cloned().collect::<Vec<_>>(),
            ["a.txt", "bin.dat", "moved.txt", "new.txt"]
        );
        assert_eq!(files["a.txt"]["added"], 1);
        assert_eq!(files["bin.dat"]["added"], Value::Null, "binary");
        let patch = v["patch"].as_str().unwrap();
        assert!(patch.contains("+two"), "{patch}");
        assert!(!patch.contains("uncommitted"));
        assert!(!patch.contains("evil"), "no driver or textconv ran");
        assert_eq!(v["patch_truncated"], false);
        // The bundle holds the tip, and only the bundle is left.
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [BUNDLE]);
        let heads = git(
            t.path(),
            &["bundle", "list-heads", dir.join(BUNDLE).to_str().unwrap()],
        );
        assert!(heads.starts_with(&c.tip), "{heads}");
        // It parses back as the helper reads it.
        assert_eq!(parse_export(&line(&Ok(v.clone()))), Some(Ok(v)));
    }

    /// A child that rewrote its history away from where it started is
    /// shown from its tip alone, with no diff to a base it no longer has.
    #[test]
    fn a_child_that_left_its_base_is_shown_from_its_tip() {
        let t = tempfile::tempdir().unwrap();
        let child = t.path().join("child");
        repo(&child, &[("a", "1")]);
        git(&child, &["checkout", "-q", "--orphan", "fresh"]);
        git(&child, &["commit", "-q", "-m", "unrelated"]);
        let dir = t.path().join("review");
        std::fs::create_dir(&dir).unwrap();
        let v = export_clone(
            Path::new(GIT),
            &child,
            &dir,
            &"b".repeat(40),
            BIG,
            Path::new(HOME),
        )
        .unwrap();
        assert_eq!(v["base_found"], false);
        assert_eq!(v["commits"].as_array().unwrap().len(), 1);
        assert_eq!(v["patch"], Value::Null);
    }

    /// The export is bounded: a long diff is cut and said to be; an export
    /// above its budget is `workspace_cap`.
    #[test]
    fn an_export_is_bounded() {
        let t = tempfile::tempdir().unwrap();
        let child = t.path().join("child");
        let base = repo(&child, &[("a", "1")]);
        let line_of = |i: usize| format!("line {i} {}\n", "x".repeat(60));
        let long: String = (0..8000).map(line_of).collect();
        std::fs::write(child.join("long.txt"), &long).unwrap();
        git(&child, &["add", "-A"]);
        git(&child, &["commit", "-q", "-m", "long"]);
        let dir = t.path().join("review");
        std::fs::create_dir(&dir).unwrap();
        let v = export_clone(Path::new(GIT), &child, &dir, &base, BIG, Path::new(HOME)).unwrap();
        assert_eq!(v["patch_truncated"], true);
        assert_eq!(v["more_commits"], false);
        for i in 0..MAX_COMMITS {
            git(
                &child,
                &["commit", "-q", "--allow-empty", "-m", &format!("c{i}")],
            );
        }
        let v = export_clone(Path::new(GIT), &child, &dir, &base, BIG, Path::new(HOME)).unwrap();
        assert_eq!(v["commits"].as_array().unwrap().len(), MAX_COMMITS);
        assert_eq!(v["more_commits"], true);
        assert!(v["patch"].as_str().unwrap().len() <= MAX_PATCH);
        assert_eq!(
            export_clone(Path::new(GIT), &child, &dir, &base, 4096, Path::new(HOME)),
            Err(Failure::WorkspaceCap)
        );
    }

    /// Everything a test needs to show the parent is untouched but for the
    /// one new ref: its refs, `HEAD`, index and working tree, byte for byte.
    pub fn snapshot(parent: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![parent.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap() {
                let p = e.unwrap().path();
                let rel = p.strip_prefix(parent).unwrap().display().to_string();
                if rel == ".git/objects" {
                    continue;
                }
                let md = p.symlink_metadata().unwrap();
                if md.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(rel, std::fs::read(&p).unwrap_or_default());
                }
            }
        }
        out
    }

    fn parent_with(t: &Path, c: &Child) -> PathBuf {
        let parent = t.join("parent");
        repo(&parent, &[("p.txt", "p\n")]);
        // The parent has the child's base: it is where the child came from.
        git(
            &parent,
            &[
                "fetch",
                "-q",
                c.child.to_str().unwrap(),
                &format!("{}:refs/heads/from", c.base),
            ],
        );
        std::fs::write(parent.join("p.txt"), "uncommitted\n").unwrap();
        std::fs::write(parent.join("untracked"), "u").unwrap();
        plant(&c.plants, &parent, "parent");
        parent
    }

    /// WT-6 with real git: Apply into a hostile parent creates only
    /// `storm/<child>`, at the child's tip; the parent's checked-out
    /// branch, index and working tree are byte-identical; nothing that
    /// either repository or the account configures runs.
    #[test]
    fn wt_6_apply_creates_only_the_childs_branch_and_runs_nothing() {
        let t = tempfile::tempdir().unwrap();
        let c = hostile_child(t.path());
        let dir = t.path().join("review");
        export(&c, &dir);
        let parent = parent_with(t.path(), &c);
        let refs = |p: &Path| git(p, &["for-each-ref", "--format=%(refname) %(objectname)"]);
        let (before, refs_before) = (snapshot(&parent), refs(&parent));
        let head = git(&parent, &["symbolic-ref", "HEAD"]);
        c.plants.clear();
        let v = import_clone(
            Path::new(GIT),
            &parent,
            &dir.join(BUNDLE),
            "storm/ags_C1",
            &c.tip,
            &c.home,
        )
        .unwrap();
        assert_eq!(c.plants.ran(), Vec::<String>::new());
        assert_eq!(
            v,
            json!({"branch": "storm/ags_C1", "commit": c.tip, "already": false})
        );
        assert_eq!(
            git(&parent, &["rev-parse", "refs/heads/storm/ags_C1"]),
            c.tip
        );
        let mut after = snapshot(&parent);
        let made = after.remove(".git/refs/heads/storm/ags_C1");
        assert!(made.is_some(), "the one new ref");
        after.remove(".git/logs/refs/heads/storm/ags_C1");
        assert_eq!(after, before, "nothing else of the parent changed");
        assert_eq!(git(&parent, &["symbolic-ref", "HEAD"]), head);
        let new_refs: Vec<String> = refs(&parent)
            .lines()
            .filter(|l| *l != format!("refs/heads/storm/ags_C1 {}", c.tip))
            .map(String::from)
            .collect();
        assert_eq!(new_refs.join("\n"), refs_before);
        git(&parent, &["fsck", "--no-progress", "--connectivity-only"]);
        // Asked again, it says the branch is there; the helper decides
        // whether that is this Apply's own.
        let again = import_clone(
            Path::new(GIT),
            &parent,
            &dir.join(BUNDLE),
            "storm/ags_C1",
            &c.tip,
            &c.home,
        )
        .unwrap();
        assert_eq!(again["already"], true);
    }

    /// 137e: never a ref that exists, and never the branch the parent (or
    /// one of its worktrees) has checked out, even unborn.
    #[test]
    fn apply_never_moves_a_branch_or_takes_the_checked_out_one() {
        let t = tempfile::tempdir().unwrap();
        let c = hostile_child(t.path());
        let dir = t.path().join("review");
        export(&c, &dir);
        let parent = parent_with(t.path(), &c);
        let bundle = dir.join(BUNDLE);
        let apply =
            |branch: &str| import_clone(Path::new(GIT), &parent, &bundle, branch, &c.tip, &c.home);
        let main = git(&parent, &["rev-parse", "main"]);
        git(&parent, &["branch", "storm/ags_X1", "main"]);
        assert_eq!(apply("storm/ags_X1"), Err(Failure::BranchExists));
        assert_eq!(git(&parent, &["rev-parse", "storm/ags_X1"]), main);
        // The parent's HEAD names the branch, not yet born.
        let head = parent.join(".git/HEAD");
        let was = std::fs::read(&head).unwrap();
        std::fs::write(&head, "ref: refs/heads/storm/ags_X2\n").unwrap();
        assert_eq!(apply("storm/ags_X2"), Err(Failure::BranchExists));
        std::fs::write(&head, &was).unwrap();
        assert!(
            git(&parent, &["for-each-ref", "refs/heads/storm/ags_X2"]).is_empty(),
            "not created"
        );
        // A linked worktree's.
        git(
            &parent,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                t.path().join("wt").to_str().unwrap(),
            ],
        );
        let wt_head = parent.join(".git/worktrees/wt/HEAD");
        std::fs::write(&wt_head, "ref: refs/heads/storm/ags_X3\n").unwrap();
        c.plants.clear();
        assert_eq!(apply("storm/ags_X3"), Err(Failure::BranchExists));
        assert_eq!(c.plants.ran(), Vec::<String>::new());
        // A parent that is not a repository any more.
        let plain = t.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(
            import_clone(
                Path::new(GIT),
                &plain,
                &bundle,
                "storm/ags_X4",
                &c.tip,
                &c.home
            ),
            Err(Failure::Unreadable)
        );
        // A tip the bundle does not hold makes no branch.
        assert_eq!(
            import_clone(
                Path::new(GIT),
                &parent,
                &bundle,
                "storm/ags_X5",
                &"d".repeat(40),
                &c.home
            ),
            Err(Failure::Unreadable)
        );
        assert_eq!(c.plants.ran(), Vec::<String>::new());
        assert!(git(&parent, &["for-each-ref", "refs/heads/storm/ags_X5"]).is_empty());
    }

    /// A scratch directory with what an Apply must not carry: a link out,
    /// a FIFO, and a repository of its own.
    fn hostile_scratch(t: &Path) -> PathBuf {
        let s = t.join("scratch");
        std::fs::create_dir_all(s.join("sub/deep")).unwrap();
        std::fs::write(s.join("notes.md"), "# notes\n").unwrap();
        std::fs::write(s.join("sub/deep/run.sh"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(
            s.join("sub/deep/run.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(t.join("secret"), "the parent's").unwrap();
        symlink(t.join("secret"), s.join("link")).unwrap();
        symlink("/etc", s.join("sub/etc")).unwrap();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            s.join("fifo"),
            FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        std::fs::create_dir_all(s.join(".git/hooks")).unwrap();
        std::fs::write(s.join(".git/config"), "[core]\n\tfsmonitor = /bin/evil\n").unwrap();
        s
    }

    #[test]
    fn a_scratch_export_lists_files_and_follows_nothing() {
        let t = tempfile::tempdir().unwrap();
        let s = hostile_scratch(t.path());
        let v = export_scratch(&s).unwrap();
        let paths: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, ["notes.md", "sub/deep/run.sh"]);
        assert_eq!(v["skipped"], 4, "two links, a FIFO and .git");
        assert_eq!(v["file_count"], 2);
        assert_eq!(parse_export(&line(&Ok(v.clone()))), Some(Ok(v)));
    }

    /// A W3 Apply: a new directory of the scratch's files, made whole under
    /// its own name or not at all; never a link followed, never an existing
    /// name overwritten.
    #[test]
    fn a_scratch_apply_makes_a_new_directory_and_nothing_else() {
        let t = tempfile::tempdir().unwrap();
        let s = hostile_scratch(t.path());
        let parent = t.path().join("ws");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("keep.txt"), "mine").unwrap();
        let v = import_scratch(&s, &parent, "ags_C1", "ws-ags_C1").unwrap();
        assert_eq!(v["files"], 2);
        assert_eq!(v["skipped"], 4);
        let made = parent.join("ws-ags_C1");
        assert_eq!(
            std::fs::read_to_string(made.join("notes.md")).unwrap(),
            "# notes\n"
        );
        let run = std::fs::metadata(made.join("sub/deep/run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert!(run & 0o100 != 0 && run & 0o7002 == 0, "{run:o}");
        for absent in ["link", "fifo", ".git", "sub/etc"] {
            assert!(made.join(absent).symlink_metadata().is_err(), "{absent}");
        }
        let mut names: Vec<String> = std::fs::read_dir(&parent)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["keep.txt", "ws-ags_C1"], "no staging left");
        // Never over an existing name, file or directory.
        assert_eq!(
            import_scratch(&s, &parent, "ags_C1", "ws-ags_C1"),
            Err(Failure::TargetExists)
        );
        assert_eq!(
            import_scratch(&s, &parent, "ags_C1", "keep.txt"),
            Err(Failure::TargetExists)
        );
        assert_eq!(
            std::fs::read_to_string(parent.join("keep.txt")).unwrap(),
            "mine"
        );
        symlink("/etc", parent.join("dangling")).unwrap();
        assert_eq!(
            import_scratch(&s, &parent, "ags_C1", "dangling"),
            Err(Failure::TargetExists)
        );
        // An interrupted Apply's staging is its own and is redone; the
        // parent's own entry under that name is not touched.
        std::fs::create_dir(parent.join(".storm-apply-ags_C2")).unwrap();
        std::fs::write(parent.join(".storm-apply-ags_C2/half"), "x").unwrap();
        import_scratch(&s, &parent, "ags_C2", "second").unwrap();
        assert!(!parent.join("second/half").exists());
        assert!(!parent.join(".storm-apply-ags_C2").exists());
        std::fs::write(parent.join(".storm-apply-ags_C3"), "the parent's file").unwrap();
        assert_eq!(
            import_scratch(&s, &parent, "ags_C3", "third"),
            Err(Failure::TargetExists)
        );
        assert!(parent.join(".storm-apply-ags_C3").is_file());
        // The parent's workspace itself is never reached through a link.
        let linked = t.path().join("linked");
        symlink(&parent, &linked).unwrap();
        assert_eq!(
            import_scratch(&s, &linked, "ags_C4", "x"),
            Err(Failure::Failed)
        );
    }

    #[test]
    fn an_export_or_import_line_parses_back_and_nothing_else_does() {
        for f in Failure::CODES {
            let line = format!(r#"{{"failed":"{f}"}}"#);
            assert_eq!(parse_export(&line), Some(Err(f)));
            assert_eq!(parse_import(&line), Some(Err(f)));
        }
        let sha = "c".repeat(40);
        let ok = json!({"done": {"branch": "storm/ags_C1", "commit": sha, "already": false}});
        assert!(parse_import(&ok.to_string()).is_some());
        for bad in [
            String::new(),
            r#"{"failed":"rm -rf"}"#.into(),
            json!({"done": {"branch": "--upload-pack=x", "commit": sha, "already": false}}).to_string(),
            json!({"done": {"branch": "storm/ags_C1", "commit": "HEAD", "already": false}}).to_string(),
            json!({"done": {"directory": "relative", "files": 1, "dirs": 0, "bytes": 1, "skipped": 0}}).to_string(),
        ] {
            assert_eq!(parse_import(&bad), None, "{bad}");
        }
        let clone = |patch: Value, commit: &str| {
            json!({"done": {"mode": "clone", "tip": sha, "base": "-", "base_found": false,
                "commits": [{"id": commit, "subject": "s"}], "more_commits": false,
                "files": [], "more_files": false, "patch": patch, "patch_truncated": false}})
            .to_string()
        };
        assert!(parse_export(&clone(Value::Null, &sha)).is_some());
        assert_eq!(parse_export(&clone(Value::Null, "nope")), None);
        assert_eq!(
            parse_export(&clone(json!("x".repeat(MAX_PATCH + 1)), &sha)),
            None
        );
        assert_eq!(
            parse_export(&json!({"done": {"mode": "other"}}).to_string()),
            None
        );
        // Only the fields an export has come through.
        let extra = clone(Value::Null, &sha).replace("\"mode\"", "\"extra\":1,\"mode\"");
        assert!(
            parse_export(&extra)
                .unwrap()
                .unwrap()
                .get("extra")
                .is_none()
        );
    }

    fn wire(answer: &Map<String, Value>) -> usize {
        crate::protocol::ok_with(crate::protocol::Op::ChildExport, answer.clone()).len()
    }

    /// What a child's review may print at every one of its limits, with
    /// every string made of control bytes, which serialize as six bytes
    /// each. `cut`: the export itself cut its commits and patch, as one at
    /// those limits does.
    pub fn export_at_every_limit(cut: bool) -> String {
        let control = |n: usize| "\u{1}".repeat(n);
        let id = "a".repeat(40);
        let files: Vec<Value> = (0..MAX_FILES)
            .map(|_| json!({"path": control(MAX_PATH), "added": 1, "removed": 1}))
            .collect();
        let commits: Vec<Value> = (0..MAX_COMMITS)
            .map(|_| json!({"id": id, "subject": control(MAX_SUBJECT)}))
            .collect();
        json!({"done": {"mode": "clone", "tip": id, "base": id, "base_found": true,
            "commits": commits, "more_commits": cut, "files": files, "more_files": false,
            "patch": control(MAX_PATCH), "patch_truncated": cut}})
        .to_string()
    }

    fn answer_of(line: &str) -> Map<String, Value> {
        let Some(Ok(Value::Object(mut v))) = parse_export(line) else {
            panic!("no export");
        };
        v.insert("child".into(), "ags_C".into());
        v.insert("parent".into(), "ags_P".into());
        v.insert(
            "parent_workspace".into(),
            format!("/{}", "\u{1}".repeat(4095)).into(),
        );
        v
    }

    /// 139-D2: the host reads 2 MiB of a child's answer; an export at every
    /// limit is several times that once serialized, so the helper trims it,
    /// files first, then the patch, then commits, and says so. What names
    /// the child and its work is never trimmed.
    #[test]
    fn an_export_at_every_limit_fits_the_hosts_read() {
        let mut v = answer_of(&export_at_every_limit(true));
        assert!(wire(&v) > 4 * CHILD_ANSWER_BUDGET);
        assert!(fit(
            &mut v,
            crate::protocol::Op::ChildExport,
            CHILD_ANSWER_BUDGET
        ));
        let sent = crate::protocol::ok_with(crate::protocol::Op::ChildExport, v.clone());
        assert!(sent.len() <= CHILD_ANSWER_BUDGET, "{} bytes", sent.len());
        assert!(sent.ends_with('\n'));
        for flag in ["more_files", "patch_truncated", "more_commits"] {
            assert_eq!(v[flag], true, "{flag}");
        }
        for (k, want) in [
            ("mode", json!("clone")),
            ("tip", json!("a".repeat(40))),
            ("base", json!("a".repeat(40))),
            ("base_found", json!(true)),
            ("child", json!("ags_C")),
            ("parent", json!("ags_P")),
        ] {
            assert_eq!(v[k], want, "{k}");
        }
        assert_eq!(v["parent_workspace"].as_str().unwrap().len(), 4096);
        let kept = v["files"].as_array().unwrap().len();
        assert!(kept < MAX_FILES);
        assert_eq!(v["omitted"]["files"], MAX_FILES - kept);
        // The rest fits once the files are cut.
        assert_eq!(v["patch"].as_str().unwrap().len(), MAX_PATCH);
        assert_eq!(v["commits"].as_array().unwrap().len(), MAX_COMMITS);
    }

    /// An ordinary export is sent as it is.
    #[test]
    fn an_ordinary_export_is_untouched() {
        let id = "b".repeat(40);
        let line = json!({"done": {"mode": "clone", "tip": id, "base": id, "base_found": true,
            "commits": [{"id": id, "subject": "work"}], "more_commits": false,
            "files": [{"path": "a.txt", "added": 1, "removed": 0}], "more_files": false,
            "patch": "+a\n", "patch_truncated": false}})
        .to_string();
        let mut v = answer_of(&line);
        let before = v.clone();
        assert!(fit(
            &mut v,
            crate::protocol::Op::ChildExport,
            CHILD_ANSWER_BUDGET
        ));
        assert_eq!(v, before);
        for flag in ["more_files", "patch_truncated", "more_commits"] {
            assert_eq!(v[flag], false, "{flag}");
        }
        assert!(v.get("omitted").is_none());
    }

    /// Files go first, then the patch, then commits, each only as far as
    /// the budget needs.
    #[test]
    fn an_answer_is_trimmed_files_then_patch_then_commits() {
        let full = answer_of(&export_at_every_limit(false));
        let essential = {
            let mut v = full.clone();
            v.insert("files".into(), json!([]));
            v.insert("commits".into(), json!([]));
            v.insert("patch".into(), json!(""));
            wire(&v)
        };
        let op = crate::protocol::Op::ChildExport;
        let mut v = full.clone();
        assert!(fit(&mut v, op, wire(&full) - 1));
        assert!(wire(&v) < wire(&full));
        assert_eq!(v["files"].as_array().unwrap().len(), MAX_FILES - 1);
        assert_eq!(
            (v["more_files"].clone(), v["patch_truncated"].clone()),
            (json!(true), json!(false))
        );
        let mut v = full.clone();
        assert!(fit(&mut v, op, essential + 300_000));
        assert!(v["files"].as_array().unwrap().is_empty());
        assert_eq!(v["patch_truncated"], true);
        assert_eq!(v["commits"].as_array().unwrap().len(), MAX_COMMITS);
        assert_eq!(v["more_commits"], false);
        let mut v = full.clone();
        assert!(fit(&mut v, op, essential + 100_000));
        assert_eq!(v["patch"], "");
        let kept = v["commits"].as_array().unwrap().len();
        assert!(0 < kept && kept < MAX_COMMITS, "{kept}");
        assert_eq!(v["more_commits"], true);
        assert_eq!(v["omitted"]["commits"], MAX_COMMITS - kept);
        assert!(wire(&v) <= essential + 100_000);
        let mut v = full;
        assert!(!fit(&mut v, op, essential / 2));
    }

    /// The host's read is storm-runtime's `MAX_CHILD_RESPONSE`, pinned here
    /// because the two crates share no code (136); its suite pins the
    /// other side.
    #[test]
    fn the_answer_budget_fits_the_hosts_read() {
        const HOSTS_READ: usize = 2 << 20;
        const { assert!(CHILD_ANSWER_BUDGET + (64 << 10) <= HOSTS_READ) };
        assert_eq!(CHILD_ANSWER_BUDGET, 2_031_616);
    }

    #[test]
    fn the_numstat_reader_takes_renames_and_binaries() {
        let listing = b"1\t0\ta.txt\0-\t-\tbin.dat\x000\t0\t\0old.txt\0new.txt\0";
        let files = numstat(listing);
        assert_eq!(
            files,
            [
                json!({"path": "a.txt", "added": 1, "removed": 0}),
                json!({"path": "bin.dat", "added": null, "removed": null}),
                json!({"path": "new.txt", "added": 0, "removed": 0}),
            ]
        );
        assert_eq!(HOOKS.len(), crate::clone::HOOK_EVENTS.len());
    }
}
