//! A git child's workspace (137b, W2): an independent clone of the parent's
//! `HEAD` commit, made by `storm-runtime-isolation child-clone` under the
//! child's own identity, in a unit of its own (`children::CloneSpec`). This
//! never runs as root: it runs git on the two paths it is given and prints
//! one line.
//!
//! Nothing the parent's repository or an account's git configuration says
//! may run. Every git call starts from an empty environment with no system
//! or global configuration, hooks pointed at `/dev/null`, no template, no
//! protocol but `file`, no lazy fetch, and objects checked as they arrive.
//! The parent is read only through git's transport, never by copying its
//! `.git`, so its hooks, config and alternates stay behind. Only committed
//! history comes across (137b, Q-137-3): a clone has no untracked, ignored
//! or uncommitted file. The remote is removed once the clone is made, so
//! nothing in the child's repository points back at the parent.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Map, Value, json};

/// The only git a child's clone runs: the system's, never one on a path a
/// repository or an account could choose.
pub const GIT: &str = "/usr/bin/git";
/// What a pool user's `HOME` is (`/nonexistent` in the sysusers file).
pub const HOME: &str = "/nonexistent";
/// Where a unit's trust file is written: its own `/tmp` (`PrivateTmp=yes`
/// in `children::sandbox`).
pub const TMP: &str = "/tmp";
/// The parent while the clone is made; removed after.
const REMOTE: &str = "storm-parent";
/// Through git's transport, never a copy of the parent's `.git`; one
/// branch, no template.
pub const CLONE: &[&str] = &[
    "clone",
    "--quiet",
    "--no-local",
    "--single-branch",
    "--no-tags",
    "--no-checkout",
    "--template=",
    "--origin",
    REMOTE,
];
/// Attributes files read to find large-file pointers, and their size.
const MAX_ATTRIBUTES: usize = 64;
const MAX_ATTRIBUTES_BYTES: usize = 1 << 20;
/// What a file costs on disk at least, for the estimate before checkout.
const BLOCK: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The commit the child starts from (the approval card's).
    pub commit: String,
    /// The branch it is on, as the parent's `HEAD` named it; none when the
    /// parent's `HEAD` was detached.
    pub branch: Option<String>,
    /// Submodules are not materialized (137b's known limit).
    pub submodules: bool,
    /// Nor are large-file pointers.
    pub large_files: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// What the clone would take is above its budget (137f).
    WorkspaceCap,
    /// git could not read or clone the parent's repository.
    Unreadable,
    /// The parent's `HEAD` has no commit to start from.
    NoCommit,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Failure::WorkspaceCap => "workspace_cap",
            Failure::Unreadable => "unreadable_repository",
            Failure::NoCommit => "no_commit",
        }
    }

    pub const CODES: [&'static str; 3] = ["workspace_cap", "unreadable_repository", "no_commit"];
}

/// The whole environment of every git call. `global` is a `Trust` file.
pub fn environment(home: &Path, global: &Path) -> Vec<(&'static str, String)> {
    let s = |v: &str| v.to_string();
    vec![
        ("PATH", s("/usr/bin:/bin")),
        ("HOME", home.display().to_string()),
        ("LC_ALL", s("C")),
        // No system configuration and no account's: its filters, hooks,
        // template directory, fsmonitor and pack-objects hook stay out. The
        // global file is Storm's own, and holds only `safe.directory`.
        ("GIT_CONFIG_NOSYSTEM", s("1")),
        ("GIT_CONFIG_SYSTEM", s("/dev/null")),
        ("GIT_CONFIG_GLOBAL", global.display().to_string()),
        ("GIT_ATTR_NOSYSTEM", s("1")),
        // A partial clone's promisor remote is the parent's config: the
        // upload-pack reading it must never fetch through it (CVE-2024-32004).
        ("GIT_NO_LAZY_FETCH", s("1")),
        ("GIT_NO_REPLACE_OBJECTS", s("1")),
        ("GIT_PROTOCOL_FROM_USER", s("0")),
        ("GIT_ALLOW_PROTOCOL", s("file")),
        ("GIT_TERMINAL_PROMPT", s("0")),
        ("GIT_PAGER", s("cat")),
        ("GIT_EDITOR", s("false")),
    ]
}

/// Every hook event git has (githooks(5)). A repository's configuration
/// can name a command for any of them (`hook.<name>.command` and
/// `.event`) under a name of its choosing, which `core.hooksPath` does not
/// reach; only switching each event off does.
pub const HOOK_EVENTS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
];

/// Given on every git call's command line, as protected configuration.
/// git hands it to the git processes it starts, but not to a `file://`
/// transport's upload-pack, which is why `Trust` exists.
pub fn protections(parent: &Path) -> Vec<String> {
    let mut out: Vec<String> = vec![
        "core.hooksPath=/dev/null".into(),
        "core.fsmonitor=false".into(),
        "core.attributesFile=/dev/null".into(),
        "init.templateDir=".into(),
        "credential.helper=".into(),
        "protocol.allow=never".into(),
        "protocol.file.allow=always".into(),
        "transfer.fsckObjects=true".into(),
        "submodule.recurse=false".into(),
        "fetch.recurseSubmodules=false".into(),
        "gc.auto=0".into(),
        "maintenance.auto=false".into(),
        "advice.detachedHead=false".into(),
        // A repository with alternates runs this to list their refs.
        "core.alternateRefsCommand=".into(),
        "transfer.bundleURI=false".into(),
        "fetch.bundleURI=".into(),
        "log.showSignature=false".into(),
    ];
    // The parent's repository is another user's; it is read, never
    // trusted, and this is the only one git is told to read.
    out.extend(safe_directories(parent).map(|d| format!("safe.directory={d}")));
    out.extend(
        HOOK_EVENTS
            .iter()
            .map(|e| format!("hook.{e}.enabled=false")),
    );
    out
}

fn safe_directories(repo: &Path) -> [String; 2] {
    let repo = repo.display();
    [format!("{repo}"), format!("{repo}/.git")]
}

/// The repository a unit's git must read, as `safe.directory` in a file
/// `GIT_CONFIG_GLOBAL` names, and nothing else (decision 140h). git hands a
/// `file://` transport's upload-pack none of its `-c` configuration, so
/// there the command line's `safe.directory` is lost and upload-pack
/// refuses a repository another user owns. Removed when dropped.
pub struct Trust {
    path: PathBuf,
}

impl Trust {
    pub fn write(dir: &Path, repo: &Path) -> std::io::Result<Trust> {
        use rustix::fs::{Mode, OFlags};
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut text = String::from("[safe]\n");
        for d in safe_directories(repo) {
            if d.chars().any(char::is_control) {
                return Err(std::io::ErrorKind::InvalidInput.into());
            }
            let quoted = d.replace('\\', "\\\\").replace('"', "\\\"");
            text.push_str(&format!("\tdirectory = \"{quoted}\"\n"));
        }
        let pid = std::process::id();
        loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!("storm-git-trust-{pid}-{n}"));
            let fd = match rustix::fs::open(
                &path,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::EXIST) if n < 1 << 16 => continue,
                Err(e) => return Err(e.into()),
            };
            let trust = Trust { path };
            std::fs::File::from(fd).write_all(text.as_bytes())?;
            return Ok(trust);
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Trust {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) struct Git<'a> {
    pub git: &'a Path,
    pub env: Vec<(&'static str, String)>,
    pub config: Vec<String>,
}

impl Git<'_> {
    fn command(&self, dir: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(self.git);
        command
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for c in &self.config {
            command.arg("-c").arg(c);
        }
        command.args(args);
        command
    }

    pub(crate) fn run(&self, dir: &Path, args: &[&str]) -> Result<Vec<u8>, ()> {
        match self.command(dir, args).output() {
            Ok(out) if out.status.success() => Ok(out.stdout),
            Ok(out) => {
                eprintln!(
                    "git {} failed: {}",
                    args.first().unwrap_or(&""),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                Err(())
            }
            Err(e) => {
                eprintln!("running git: {e}");
                Err(())
            }
        }
    }

    /// At most `limit` bytes of what it prints, and whether there was more:
    /// a diff of a large workspace never has to fit in memory. A git that
    /// was cut short is killed, never waited out.
    pub(crate) fn run_limited(
        &self,
        dir: &Path,
        args: &[&str],
        limit: usize,
    ) -> Result<(Vec<u8>, bool), ()> {
        use std::io::Read;
        let mut child = self
            .command(dir, args)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| eprintln!("running git: {e}"))?;
        let mut out = Vec::new();
        let stdout = child.stdout.take().ok_or(())?;
        let got = stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|_| ())?;
        let truncated = got > limit;
        if truncated {
            out.truncate(limit);
            let _ = child.kill();
            let _ = child.wait();
            return Ok((out, true));
        }
        match child.wait() {
            Ok(status) if status.success() => Ok((out, false)),
            _ => {
                eprintln!("git {} failed", args.first().unwrap_or(&""));
                Err(())
            }
        }
    }
}

/// A branch name as it may appear on the approval card: anything else is
/// left unnamed rather than passed on.
pub fn branch_ok(name: &str) -> bool {
    (1..=200).contains(&name.len())
        && !name.starts_with(['-', '/', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/-+@".contains(&b))
}

pub fn commit_ok(id: &str) -> bool {
    matches!(id.len(), 40 | 64)
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

struct Tree {
    /// What checking it out takes at least.
    bytes: u64,
    submodules: bool,
    /// The attributes files' blobs.
    attributes: Vec<String>,
}

fn read_tree(listing: &[u8]) -> Tree {
    let mut tree = Tree {
        bytes: 0,
        submodules: false,
        attributes: Vec::new(),
    };
    for record in listing.split(|&b| b == 0).filter(|r| !r.is_empty()) {
        let Some(tab) = record.iter().position(|&b| b == b'\t') else {
            continue;
        };
        let (meta, path) = (&record[..tab], &record[tab + 1..]);
        let fields: Vec<&[u8]> = meta
            .split(|&b| b == b' ')
            .filter(|f| !f.is_empty())
            .collect();
        let [_mode, kind, object, size] = fields[..] else {
            continue;
        };
        match kind {
            b"commit" => tree.submodules = true,
            b"blob" => {
                let size: u64 = std::str::from_utf8(size)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                tree.bytes += size.div_ceil(BLOCK).max(1) * BLOCK;
                let name = path.rsplit(|&b| b == b'/').next().unwrap_or(path);
                if name == b".gitattributes" && tree.attributes.len() < MAX_ATTRIBUTES {
                    tree.attributes
                        .push(String::from_utf8_lossy(object).into_owned());
                }
            }
            _ => {}
        }
    }
    tree
}

/// Clones `parent`'s `HEAD` into the empty directory `dest`, in at most
/// `budget` bytes. `home` is the user's `HOME`, which git is never let read;
/// `tmp` is where its `Trust` is written.
pub fn clone(
    git: &Path,
    parent: &Path,
    dest: &Path,
    budget: u64,
    home: &Path,
    tmp: &Path,
) -> Result<Report, Failure> {
    let trust = Trust::write(tmp, parent).map_err(|e| {
        eprintln!("writing git's trust file: {e}");
        Failure::Unreadable
    })?;
    let git = Git {
        git,
        env: environment(home, trust.path()),
        config: protections(parent),
    };
    let url = format!("file://{}", parent.display());
    let dest_arg = dest.to_str().ok_or(Failure::Unreadable)?;
    let mut args = CLONE.to_vec();
    args.extend(["--", &url, dest_arg]);
    git.run(dest, &args).map_err(|()| Failure::Unreadable)?;
    // `remote remove` leaves the remote's HEAD behind, pointing at nothing.
    let _ = git.run(
        dest,
        &[
            "symbolic-ref",
            "--delete",
            &format!("refs/remotes/{REMOTE}/HEAD"),
        ],
    );
    git.run(dest, &["remote", "remove", REMOTE])
        .map_err(|()| Failure::Unreadable)?;
    let commit = git
        .run(dest, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .map_err(|()| Failure::NoCommit)?;
    let commit = String::from_utf8_lossy(&commit).trim().to_string();
    if !commit_ok(&commit) {
        return Err(Failure::NoCommit);
    }
    let branch = git
        .run(dest, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .filter(|b| branch_ok(b));
    let listing = git
        .run(dest, &["ls-tree", "-r", "-l", "-z", "--full-tree", "HEAD"])
        .map_err(|()| Failure::Unreadable)?;
    let tree = read_tree(&listing);
    let objects = crate::children::disk_usage(dest).map_err(|_| Failure::Unreadable)?;
    // Checked before the checkout writes anything: a small repository can
    // expand to far more than its objects.
    if objects.saturating_add(tree.bytes) > budget {
        return Err(Failure::WorkspaceCap);
    }
    let mut large_files = false;
    for blob in &tree.attributes {
        let text = git
            .run(dest, &["cat-file", "blob", blob])
            .map_err(|()| Failure::Unreadable)?;
        let text = &text[..text.len().min(MAX_ATTRIBUTES_BYTES)];
        if text.windows(10).any(|w| w == b"filter=lfs") {
            large_files = true;
        }
    }
    git.run(dest, &["reset", "--hard", "--quiet", "HEAD"])
        .map_err(|()| Failure::Unreadable)?;
    // The clone's own history of how it was made names the parent's path.
    let dot = dest.join(".git");
    let _ = std::fs::remove_dir_all(dot.join("logs"));
    for file in ["FETCH_HEAD", "ORIG_HEAD"] {
        let _ = std::fs::remove_file(dot.join(file));
    }
    if crate::children::disk_usage(dest).map_err(|_| Failure::Unreadable)? > budget {
        return Err(Failure::WorkspaceCap);
    }
    Ok(Report {
        commit,
        branch,
        submodules: tree.submodules,
        large_files,
    })
}

/// The one line the clone prints, which the helper checks.
pub fn line(result: &Result<Report, Failure>) -> String {
    match result {
        Ok(r) => json!({"done": {
            "commit": r.commit,
            "branch": r.branch,
            "submodules": r.submodules,
            "large_files": r.large_files,
        }})
        .to_string(),
        Err(f) => json!({"failed": f.code()}).to_string(),
    }
}

/// The clone's last line, checked before any of it reaches the host: a
/// commit id, a branch name of a safe shape, two flags, or a known code.
pub fn parse_line(stdout: &str) -> Option<Result<Report, &'static str>> {
    let line = stdout.lines().rev().find(|l| !l.trim().is_empty())?;
    let v: Map<String, Value> = serde_json::from_str(line).ok()?;
    if let Some(code) = v.get("failed") {
        let code = code.as_str()?;
        return Failure::CODES.into_iter().find(|c| *c == code).map(Err);
    }
    let done = v.get("done").and_then(Value::as_object)?;
    let commit = done.get("commit").and_then(Value::as_str)?;
    if !commit_ok(commit) {
        return None;
    }
    let branch = match done.get("branch") {
        None | Some(Value::Null) => None,
        Some(Value::String(b)) if branch_ok(b) => Some(b.clone()),
        Some(_) => return None,
    };
    let flag = |k: &str| done.get(k).and_then(Value::as_bool);
    Some(Ok(Report {
        commit: commit.to_string(),
        branch,
        submodules: flag("submodules")?,
        large_files: flag("large_files")?,
    }))
}

/// `storm-runtime-isolation child-clone <parent> <dest> <budget>`, which
/// only a child's clone unit runs.
pub fn main(args: &[String]) -> std::process::ExitCode {
    let [parent, dest, budget] = args else {
        eprintln!("usage: storm-runtime-isolation child-clone <parent> <dest> <budget>");
        return std::process::ExitCode::from(2);
    };
    if rustix::process::geteuid().is_root() {
        eprintln!("storm-runtime-isolation: a child's clone never runs as root");
        return std::process::ExitCode::FAILURE;
    }
    let Ok(budget) = budget.parse::<u64>() else {
        return std::process::ExitCode::from(2);
    };
    let result = clone(
        Path::new(GIT),
        Path::new(parent),
        Path::new(dest),
        budget,
        Path::new(HOME),
        Path::new(TMP),
    );
    println!("{}", line(&result));
    std::process::ExitCode::SUCCESS
}

/// These run real git as this test's user: everything a child's clone
/// does but the identity switch, which needs root (the codebox steps).
#[cfg(test)]
pub mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// Every hook git runs (githooks(5)).
    pub const HOOKS: &[&str] = HOOK_EVENTS;

    /// git for building fixtures: nothing of this machine's configuration,
    /// and no hook of the fixture's own.
    pub fn git(dir: &Path, args: &[&str]) -> String {
        git_with(dir, args, &[])
    }

    pub fn git_with(dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> String {
        let out = Command::new(GIT)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", HOME)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .envs(env.iter().map(|(k, v)| (k, v)))
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repository with `files` committed on `main`.
    pub fn repo(dir: &Path, files: &[(&str, &str)]) -> String {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "--template="]);
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "first"]);
        git(dir, &["rev-parse", "HEAD"])
    }

    /// Something a repository or an account can make git run. It writes
    /// its name to `marks/` when it runs, then does its job: a filter
    /// passes its input through, the pack-objects hook runs pack-objects.
    pub struct Plants {
        pub base: PathBuf,
    }

    impl Plants {
        pub fn new(base: &Path) -> Self {
            std::fs::create_dir_all(base.join("marks")).unwrap();
            Self {
                base: base.to_path_buf(),
            }
        }

        pub fn evil(&self, name: &str) -> PathBuf {
            let path = self.base.join("bin").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let marks = self.base.join("marks");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho \"$@\" >> {}/{name}\ncase \"$1\" in\n  smudge|clean) exec cat ;;\n  git) exec \"$@\" ;;\nesac\nexit 0\n",
                    marks.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        /// A hooks directory with every hook, each marking `<prefix>-<hook>`.
        pub fn hooks(&self, dir: &Path, prefix: &str) {
            std::fs::create_dir_all(dir).unwrap();
            for hook in HOOKS {
                let evil = self.evil(&format!("{prefix}-{hook}"));
                std::fs::copy(evil, dir.join(hook)).unwrap();
                std::fs::set_permissions(dir.join(hook), std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
        }

        pub fn ran(&self) -> Vec<String> {
            let mut out: Vec<String> = std::fs::read_dir(self.base.join("marks"))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            out.sort();
            out
        }

        pub fn clear(&self) {
            std::fs::remove_dir_all(self.base.join("marks")).unwrap();
            std::fs::create_dir_all(self.base.join("marks")).unwrap();
        }
    }

    pub const POINTER: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";

    /// A parent repository with everything 137b and WT-7 name planted in
    /// it and in an account's git configuration (`home`): every hook, in
    /// `.git/hooks` and in a `core.hooksPath`; smudge and clean filters,
    /// in the repository's config, an included file and the account's;
    /// `.gitattributes` naming them and large-file storage; `core.fsmonitor`;
    /// `include.path` and `includeIf`; a template directory; the
    /// pack-objects hook, the alternate-refs command, an upload-pack, an
    /// SSH command, an askpass and a credential helper; and a submodule.
    pub struct Hostile {
        pub plants: Plants,
        pub parent: PathBuf,
        pub home: PathBuf,
        pub commit: String,
    }

    /// A command for every hook event under a name of the repository's
    /// own (`hook.<name>.command`), which `core.hooksPath` does not reach.
    pub fn config_hooks(plants: &Plants, prefix: &str) -> String {
        let mut out = String::new();
        for event in HOOKS {
            let name = format!("{prefix}-{event}");
            out.push_str(&format!(
                "[hook \"{name}\"]\n\tcommand = {}\n\tevent = {event}\n",
                plants.evil(&name).display()
            ));
        }
        out
    }

    pub fn hostile(base: &Path) -> Hostile {
        hostile_at(base, &base.join("parent"))
    }

    pub fn hostile_at(base: &Path, parent: &Path) -> Hostile {
        let plants = Plants::new(base);
        let parent = parent.to_path_buf();
        repo(
            &parent,
            &[
                ("a.txt", "committed\n"),
                ("inc.txt", "included\n"),
                ("big.bin", POINTER),
                (
                    ".gitattributes",
                    "* filter=evil diff=evil merge=evil\ninc.txt filter=inc\n*.bin filter=lfs diff=lfs merge=lfs -text\n",
                ),
                (
                    ".gitmodules",
                    "[submodule \"sub\"]\n\tpath = sub\n\turl = file:///nonexistent/sub\n",
                ),
            ],
        );
        // A gitlink, as a submodule is recorded.
        let sub = git(&parent, &["rev-parse", "HEAD"]);
        git(
            &parent,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{sub},sub"),
            ],
        );
        git(&parent, &["commit", "-q", "-m", "submodule"]);
        let commit = git(&parent, &["rev-parse", "HEAD"]);
        let dot = parent.join(".git");
        plants.hooks(&dot.join("hooks"), "repo-hook");
        plants.hooks(&base.join("repo-hooks-path"), "repo-hooks-path");
        let e = |n: &str| plants.evil(n).display().to_string();
        let included = base.join("included.config");
        std::fs::write(
            &included,
            format!(
                "[filter \"inc\"]\n\tsmudge = {} smudge\n\tclean = {} clean\n[core]\n\tsshCommand = {}\n",
                e("included-smudge"),
                e("included-clean"),
                e("included-ssh")
            ),
        )
        .unwrap();
        let config = format!(
            "[core]\n\thooksPath = {hooks}\n\tfsmonitor = {fsmonitor}\n\talternateRefsCommand = {alt}\n\tsshCommand = {ssh}\n\taskPass = {askpass}\n\tpager = {pager}\n\teditor = {editor}\n\
             [include]\n\tpath = {included}\n\
             [includeIf \"onbranch:main\"]\n\tpath = {included}\n\
             [filter \"evil\"]\n\tsmudge = {smudge} smudge\n\tclean = {clean} clean\n\
             [filter \"lfs\"]\n\tsmudge = {lfs} smudge\n\tclean = {lfs} clean\n\
             [uploadpack]\n\tpackObjectsHook = {packhook}\n\
             [credential]\n\thelper = !{cred}\n\
             [diff \"evil\"]\n\tcommand = {diff}\n\
             [merge \"evil\"]\n\tdriver = {merge}\n\
             [remote \"origin\"]\n\turl = file:///nonexistent\n\tuploadpack = {uploadpack}\n\
             [gpg]\n\tprogram = {gpg}\n",
            hooks = base.join("repo-hooks-path").display(),
            fsmonitor = e("repo-fsmonitor"),
            alt = e("repo-alternate-refs"),
            ssh = e("repo-ssh"),
            askpass = e("repo-askpass"),
            pager = e("repo-pager"),
            editor = e("repo-editor"),
            included = included.display(),
            smudge = e("repo-smudge"),
            clean = e("repo-clean"),
            lfs = e("repo-lfs"),
            packhook = e("repo-pack-objects-hook"),
            cred = e("repo-credential"),
            diff = e("repo-diff"),
            merge = e("repo-merge"),
            uploadpack = e("repo-upload-pack"),
            gpg = e("repo-gpg"),
        );
        let mut text = std::fs::read_to_string(dot.join("config")).unwrap();
        text.push_str(&config);
        text.push_str(&config_hooks(&plants, "repo-config-hook"));
        std::fs::write(dot.join("config"), text).unwrap();
        // The account's own configuration, which a child must not inherit.
        let home = base.join("home");
        plants.hooks(&base.join("account-hooks"), "account-hook");
        plants.hooks(&base.join("template/hooks"), "template");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(base.join("account-attributes"), "* filter=evil\n").unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[core]\n\thooksPath = {hooks}\n\tfsmonitor = {fsmonitor}\n\tattributesFile = {attributes}\n\
                 [init]\n\ttemplateDir = {template}\n\
                 [filter \"evil\"]\n\tsmudge = {smudge} smudge\n\tclean = {smudge} clean\n\
                 [filter \"lfs\"]\n\tsmudge = {lfs} smudge\n\tclean = {lfs} clean\n\
                 [uploadpack]\n\tpackObjectsHook = {packhook}\n\
                 [credential]\n\thelper = !{cred}\n",
                hooks = base.join("account-hooks").display(),
                fsmonitor = e("account-fsmonitor"),
                attributes = base.join("account-attributes").display(),
                template = base.join("template").display(),
                smudge = e("account-smudge"),
                lfs = e("account-lfs"),
                packhook = e("account-pack-objects-hook"),
                cred = e("account-credential"),
            ),
        )
        .unwrap();
        Hostile {
            plants,
            parent,
            home,
            commit,
        }
    }

    fn empty_dir(base: &Path, name: &str) -> PathBuf {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const BIG: u64 = 1 << 30;

    /// The positive control: each plant is live, so a test that finds
    /// none of them ran says something.
    #[test]
    fn the_planted_hooks_filters_and_commands_are_live() {
        let t = tempfile::tempdir().unwrap();
        let h = hostile(t.path());
        let home = [("HOME", h.home.as_path())];
        // A plain clone under the account's configuration.
        let plain = t.path().join("plain");
        let url = format!("file://{}", h.parent.display());
        Command::new(GIT)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &h.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["clone", "-q", &url])
            .arg(&plain)
            .status()
            .unwrap();
        let ran = h.plants.ran();
        for want in [
            "account-pack-objects-hook",
            "account-hook-post-checkout",
            "account-hook-reference-transaction",
            "account-smudge",
            "account-lfs",
        ] {
            assert!(ran.contains(&want.to_string()), "{want} in {ran:?}");
        }
        h.plants.clear();
        // A template's hooks, where no hooksPath overrides them.
        let templated = t.path().join("templated");
        git(
            t.path(),
            &[
                "clone",
                "-q",
                &format!("--template={}", t.path().join("template").display()),
                &url,
                templated.to_str().unwrap(),
            ],
        );
        assert!(
            h.plants
                .ran()
                .contains(&"template-post-checkout".to_string())
        );
        h.plants.clear();
        // The repository's own: its hooksPath, its .git/hooks, its
        // filters, the included file's and its fsmonitor.
        std::fs::remove_file(h.parent.join("a.txt")).unwrap();
        std::fs::remove_file(h.parent.join("inc.txt")).unwrap();
        git_with(&h.parent, &["checkout", "--", "a.txt", "inc.txt"], &home);
        git(&h.parent, &["commit", "-q", "--allow-empty", "-m", "x"]);
        git(
            &h.parent,
            &[
                "-c",
                "core.hooksPath=.git/hooks",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "y",
            ],
        );
        git(&h.parent, &["status", "--porcelain"]);
        let ran = h.plants.ran();
        for want in [
            "repo-smudge",
            "included-smudge",
            "repo-hooks-path-pre-commit",
            "repo-hooks-path-reference-transaction",
            "repo-hook-pre-commit",
            "repo-hook-post-commit",
            "repo-fsmonitor",
            "repo-config-hook-pre-commit",
            "repo-config-hook-reference-transaction",
        ] {
            assert!(ran.contains(&want.to_string()), "{want} in {ran:?}");
        }
    }

    /// WT-7, the half that is git's: none of it runs while a child's
    /// workspace is made, under an account whose configuration has its
    /// own.
    #[test]
    fn wt_7_a_childs_clone_runs_nothing_from_the_repository_or_the_account() {
        let t = tempfile::tempdir().unwrap();
        let h = hostile(t.path());
        let dest = empty_dir(t.path(), "child");
        let report = clone(Path::new(GIT), &h.parent, &dest, BIG, &h.home, t.path()).unwrap();
        assert_eq!(h.plants.ran(), Vec::<String>::new());
        assert_eq!(report.commit, h.commit);
        assert_eq!(report.branch.as_deref(), Some("main"));
        assert!(report.submodules && report.large_files, "{report:?}");
        // Checked out exactly as committed: no filter touched a byte.
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "committed\n"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("big.bin")).unwrap(),
            POINTER
        );
        // The submodule is not materialized.
        assert!(
            std::fs::read_dir(dest.join("sub"))
                .map(|mut d| d.next().is_none())
                .unwrap_or(true)
        );
    }

    /// WT-7: only committed history; untracked, ignored, staged and
    /// modified files stay with the parent (Q-137-3).
    #[test]
    fn wt_7_only_committed_files_reach_the_child() {
        let t = tempfile::tempdir().unwrap();
        let parent = t.path().join("parent");
        repo(
            &parent,
            &[("a.txt", "committed\n"), (".gitignore", "secret.env\n")],
        );
        std::fs::write(parent.join("a.txt"), "uncommitted\n").unwrap();
        std::fs::write(parent.join("untracked.txt"), "x").unwrap();
        std::fs::write(parent.join("secret.env"), "TOKEN=1").unwrap();
        std::fs::write(parent.join("staged.txt"), "x").unwrap();
        git(&parent, &["add", "staged.txt"]);
        let dest = empty_dir(t.path(), "child");
        clone(
            Path::new(GIT),
            &parent,
            &dest,
            BIG,
            Path::new(HOME),
            t.path(),
        )
        .unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [".git", ".gitignore", "a.txt"]);
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "committed\n"
        );
        assert_eq!(git(&dest, &["status", "--porcelain"]), "");
    }

    /// 137b: the child's repository is its own: its objects copied, no
    /// remote, no hooks, a fresh configuration, nothing that names the
    /// parent.
    #[test]
    fn the_clone_is_independent_of_its_parent() {
        let t = tempfile::tempdir().unwrap();
        let h = hostile(t.path());
        let dest = empty_dir(t.path(), "child");
        clone(Path::new(GIT), &h.parent, &dest, BIG, &h.home, t.path()).unwrap();
        let dot = dest.join(".git");
        assert!(!dot.join("hooks").exists(), "no template was copied");
        assert!(!dot.join("objects/info/alternates").exists());
        assert_eq!(git(&dest, &["remote"]), "");
        let config = std::fs::read_to_string(dot.join("config")).unwrap();
        for absent in [
            "hooksPath",
            "fsmonitor",
            "filter",
            "include",
            "remote",
            "url",
        ] {
            assert!(!config.contains(absent), "{absent} in {config}");
        }
        // Nothing under .git names the parent's path.
        let needle = h.parent.display().to_string();
        let mut stack = vec![dot.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let bytes = std::fs::read(&p).unwrap();
                    assert!(
                        !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                        "{} names the parent",
                        p.display()
                    );
                }
            }
        }
        // Its objects are its own: removing the parent loses nothing.
        std::fs::remove_dir_all(&h.parent).unwrap();
        git(&dest, &["fsck", "--full", "--no-progress"]);
        // And its commits are the child's to make.
        git(&dest, &["commit", "-q", "--allow-empty", "-m", "child"]);
    }

    #[test]
    fn a_detached_head_an_empty_repository_and_no_repository() {
        let t = tempfile::tempdir().unwrap();
        let parent = t.path().join("parent");
        let first = repo(&parent, &[("a", "1")]);
        std::fs::write(parent.join("a"), "2").unwrap();
        git(&parent, &["commit", "-q", "-am", "second"]);
        git(&parent, &["checkout", "-q", "--detach", &first]);
        let dest = empty_dir(t.path(), "detached");
        let r = clone(
            Path::new(GIT),
            &parent,
            &dest,
            BIG,
            Path::new(HOME),
            t.path(),
        )
        .unwrap();
        assert_eq!((r.commit.as_str(), r.branch), (first.as_str(), None));
        assert_eq!(std::fs::read_to_string(dest.join("a")).unwrap(), "1");
        let empty = t.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        git(&empty, &["init", "-q", "--template="]);
        let dest = empty_dir(t.path(), "of-empty");
        assert_eq!(
            clone(
                Path::new(GIT),
                &empty,
                &dest,
                BIG,
                Path::new(HOME),
                t.path()
            ),
            Err(Failure::NoCommit)
        );
        let dest = empty_dir(t.path(), "of-nothing");
        assert_eq!(
            clone(
                Path::new(GIT),
                &t.path().join("nothing"),
                &dest,
                BIG,
                Path::new(HOME),
                t.path()
            ),
            Err(Failure::Unreadable)
        );
    }

    /// WT-9's mid-way half: a repository whose objects are small but whose
    /// checkout is not stops before the checkout writes anything.
    #[test]
    fn wt_9_a_clone_above_its_budget_stops_before_its_checkout() {
        let t = tempfile::tempdir().unwrap();
        let parent = t.path().join("parent");
        let zeros = "0".repeat(4 << 20);
        repo(&parent, &[("zeros.txt", &zeros)]);
        let objects = crate::children::disk_usage(&parent.join(".git")).unwrap();
        assert!(objects < 1 << 20, "{objects}");
        let dest = empty_dir(t.path(), "child");
        assert_eq!(
            clone(
                Path::new(GIT),
                &parent,
                &dest,
                2 << 20,
                Path::new(HOME),
                t.path()
            ),
            Err(Failure::WorkspaceCap)
        );
        assert!(!dest.join("zeros.txt").exists());
        let dest = empty_dir(t.path(), "roomy");
        clone(
            Path::new(GIT),
            &parent,
            &dest,
            8 << 20,
            Path::new(HOME),
            t.path(),
        )
        .unwrap();
        assert!(dest.join("zeros.txt").exists());
    }

    /// CVE-2024-32004's shape: the parent is a partial clone whose promisor
    /// remote's upload-pack is a command. Reading it never fetches.
    #[test]
    fn a_partial_clones_promisor_remote_never_runs() {
        let t = tempfile::tempdir().unwrap();
        let plants = Plants::new(t.path());
        let src = t.path().join("src");
        repo(&src, &[("b.txt", "big\n")]);
        git(&src, &["config", "uploadpack.allowFilter", "true"]);
        let lazy = t.path().join("lazy");
        git(
            t.path(),
            &[
                "clone",
                "-q",
                "--no-local",
                "--filter=blob:none",
                "--no-checkout",
                &format!("file://{}", src.display()),
                lazy.to_str().unwrap(),
            ],
        );
        let evil = plants.evil("promisor-upload-pack");
        git(
            &lazy,
            &["config", "remote.origin.uploadpack", evil.to_str().unwrap()],
        );
        let dest = empty_dir(t.path(), "child");
        assert_eq!(
            clone(Path::new(GIT), &lazy, &dest, BIG, Path::new(HOME), t.path()),
            Err(Failure::Unreadable)
        );
        assert_eq!(plants.ran(), Vec::<String>::new());
    }

    /// The layers that do not show on this machine, held as data: a
    /// system configuration, a template, a lazy fetch, a network
    /// protocol.
    #[test]
    fn every_git_call_has_the_whole_protection() {
        let env = environment(Path::new("/h"), Path::new("/tmp/trust"));
        for (k, v) in [
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ("GIT_CONFIG_GLOBAL", "/tmp/trust"),
            ("GIT_ATTR_NOSYSTEM", "1"),
            ("GIT_NO_LAZY_FETCH", "1"),
            ("GIT_ALLOW_PROTOCOL", "file"),
            ("GIT_PROTOCOL_FROM_USER", "0"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("HOME", "/h"),
        ] {
            assert!(env.contains(&(k, v.to_string())), "{k}");
        }
        let config = protections(Path::new("/w/app"));
        for want in [
            "core.hooksPath=/dev/null",
            "core.fsmonitor=false",
            "core.attributesFile=/dev/null",
            "init.templateDir=",
            "credential.helper=",
            "protocol.allow=never",
            "protocol.file.allow=always",
            "transfer.fsckObjects=true",
            "submodule.recurse=false",
            "safe.directory=/w/app",
        ] {
            assert!(config.contains(&want.to_string()), "{want}");
        }
        assert!(
            !config.iter().any(|c| c == "safe.directory=*"),
            "only the parent is trusted"
        );
        for flag in ["--no-local", "--template=", "--single-branch", "--no-tags"] {
            assert!(CLONE.contains(&flag), "{flag}");
        }
    }

    /// What `git config` reads back from a file.
    fn config_list(file: &Path) -> Vec<String> {
        git(
            Path::new("/"),
            &["config", "--file", file.to_str().unwrap(), "--list"],
        )
        .lines()
        .map(String::from)
        .collect()
    }

    /// 140h: the trust file holds the one repository and its `.git`,
    /// nothing else, private and new; and it is gone once dropped.
    #[test]
    fn the_trust_file_names_only_the_one_repository_and_goes_away() {
        use std::os::unix::fs::MetadataExt;
        let t = tempfile::tempdir().unwrap();
        let trust = Trust::write(t.path(), Path::new("/w/app")).unwrap();
        let path = trust.path().to_path_buf();
        assert_eq!(path.parent(), Some(t.path()));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[safe]\n\tdirectory = \"/w/app\"\n\tdirectory = \"/w/app/.git\"\n"
        );
        assert_eq!(
            config_list(&path),
            ["safe.directory=/w/app", "safe.directory=/w/app/.git"]
        );
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
        let other = Trust::write(t.path(), Path::new("/w/app")).unwrap();
        assert_ne!(other.path(), path, "each its own");
        drop(other);
        drop(trust);
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 0);
        // A name git would read differently is quoted; one it cannot hold
        // is refused.
        let odd = Trust::write(t.path(), Path::new(r#"/w/a "b" \c; #d "#)).unwrap();
        assert_eq!(
            config_list(odd.path()),
            [
                r#"safe.directory=/w/a "b" \c; #d "#,
                r#"safe.directory=/w/a "b" \c; #d /.git"#
            ]
        );
        drop(odd);
        assert!(Trust::write(t.path(), Path::new("/w/a\n[core]\n")).is_err());
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 0);
    }

    /// 140h's premise, on this machine's git: a `file://` transport's
    /// upload-pack runs with no `-c` configuration (git clears
    /// `GIT_CONFIG_PARAMETERS` for it), but with the global file.
    #[test]
    fn the_transport_drops_the_command_line_but_keeps_the_global_file() {
        let t = tempfile::tempdir().unwrap();
        let plants = Plants::new(t.path());
        let parent = t.path().join("parent");
        repo(&parent, &[("a", "1")]);
        let hook = format!(
            "uploadpack.packObjectsHook={}",
            plants.evil("pack-objects-hook").display()
        );
        let clone_with = |env: Vec<(&'static str, String)>, config: Vec<String>, name: &str| {
            let dest = t.path().join(name);
            let url = format!("file://{}", parent.display());
            let mut args = CLONE.to_vec();
            args.extend(["--", &url, dest.to_str().unwrap()]);
            let g = Git {
                git: Path::new(GIT),
                env,
                config,
            };
            g.run(t.path(), &args).unwrap();
        };
        let null = Path::new("/dev/null");
        clone_with(
            environment(Path::new(HOME), null),
            vec![hook.clone()],
            "cli",
        );
        assert_eq!(plants.ran(), Vec::<String>::new(), "-c never reached it");
        let global = t.path().join("global");
        std::fs::write(
            &global,
            format!(
                "[uploadpack]\n\tpackObjectsHook = {}\n",
                plants.evil("pack-objects-hook").display()
            ),
        )
        .unwrap();
        clone_with(environment(Path::new(HOME), &global), vec![], "from-global");
        assert_eq!(plants.ran(), ["pack-objects-hook"], "the global file did");
    }

    /// Whether this git's upload-pack refuses a repository it takes to be
    /// another user's (it does on Debian 13's 2.47; 2.56 no longer checks).
    pub fn upload_pack_checks_ownership(repo: &Path) -> bool {
        !Command::new(GIT)
            .env_clear()
            .envs(environment(Path::new(HOME), Path::new("/dev/null")))
            .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
            .args(["upload-pack", "--advertise-refs"])
            .arg(repo)
            .stdin(Stdio::null())
            .output()
            .unwrap()
            .status
            .success()
    }

    /// 140h, the defect: the parent is the host account's, not the clone's
    /// user's. git is made to take every repository for another user's
    /// (`GIT_TEST_ASSUME_DIFFERENT_OWNER`), and the clone's own transport
    /// call runs as `clone` runs it: with only `-c safe.directory` the
    /// parent is refused where upload-pack checks; with the trust file it
    /// is read.
    #[test]
    fn a_clone_reads_a_parent_another_user_owns() {
        let t = tempfile::tempdir().unwrap();
        let parent = t.path().join("parent");
        let commit = repo(&parent, &[("a", "1")]);
        let tmp = empty_dir(t.path(), "tmp");
        let foreign = |global: &Path, name: &str| {
            let mut env = environment(Path::new(HOME), global);
            env.push(("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1".into()));
            let g = Git {
                git: Path::new(GIT),
                env,
                config: protections(&parent),
            };
            let dest = t.path().join(name);
            let url = format!("file://{}", parent.display());
            let mut args = CLONE.to_vec();
            args.extend(["--", &url, dest.to_str().unwrap()]);
            (g.run(t.path(), &args), dest)
        };
        let (before, _) = foreign(Path::new("/dev/null"), "before");
        if upload_pack_checks_ownership(&parent) {
            assert!(before.is_err(), "-c safe.directory alone is lost");
        } else {
            eprintln!("this git's upload-pack does not check ownership");
        }
        let trust = Trust::write(&tmp, &parent).unwrap();
        let (after, dest) = foreign(trust.path(), "after");
        assert!(after.is_ok());
        assert_eq!(
            git(&dest, &["rev-parse", "refs/remotes/storm-parent/main"]),
            commit
        );
        drop(trust);
        // The whole clone leaves no trust file behind, made or failed.
        let dest = empty_dir(t.path(), "whole");
        let r = clone(Path::new(GIT), &parent, &dest, BIG, Path::new(HOME), &tmp).unwrap();
        assert_eq!(r.commit, commit);
        let dest = empty_dir(t.path(), "capped");
        assert_eq!(
            clone(Path::new(GIT), &parent, &dest, 1, Path::new(HOME), &tmp),
            Err(Failure::WorkspaceCap)
        );
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
    }

    #[test]
    fn the_line_parses_back_and_nothing_else_does() {
        let r = Report {
            commit: "a".repeat(40),
            branch: Some("feat/x".into()),
            submodules: true,
            large_files: false,
        };
        assert_eq!(parse_line(&line(&Ok(r.clone()))), Some(Ok(r)));
        for f in [
            Failure::WorkspaceCap,
            Failure::Unreadable,
            Failure::NoCommit,
        ] {
            assert_eq!(parse_line(&line(&Err(f))), Some(Err(f.code())));
        }
        let sha = "b".repeat(64);
        assert!(parse_line(&format!("noise\n{{\"done\":{{\"commit\":\"{sha}\",\"branch\":null,\"submodules\":false,\"large_files\":false}}}}\n")).is_some());
        for bad in [
            "",
            "{}",
            r#"{"failed":"rm -rf"}"#,
            r#"{"done":{"commit":"xyz","branch":null,"submodules":false,"large_files":false}}"#,
            r#"{"done":{"commit":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","branch":null,"submodules":false,"large_files":false}}"#,
            &format!(
                r#"{{"done":{{"commit":"{sha}","branch":"--upload-pack=x","submodules":false,"large_files":false}}}}"#
            ),
            &format!(
                r#"{{"done":{{"commit":"{sha}","branch":null,"submodules":"yes","large_files":false}}}}"#
            ),
        ] {
            assert_eq!(parse_line(bad), None, "{bad}");
        }
    }
}
