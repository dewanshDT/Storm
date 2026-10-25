//! A session's home (136e, 136f): what goes into it at start, and what of
//! it is kept for Resume at lease end.
//!
//! - **The snapshot.** A per-profile allowlist of the host account's agent
//!   configuration is copied in at start: settings, instructions, MCP server
//!   definitions, plugins, skills, the shell's start-up files. Most of it is
//!   root's and only readable by the session ([`Access::ReadOnly`]); the few
//!   files an agent rewrites as it runs (`.claude.json`, Codex's
//!   `config.toml`, Cursor's `cli-config.json`) are the session's own copy
//!   ([`Access::Private`]). Either way nothing goes back: the next session
//!   gets the account's version again.
//! - **Credentials are never part of it.** No credential file is on the
//!   list, and [`DENIED`] names are skipped inside every copied tree. The
//!   one exception 136f allows, a static API key from a provider's own
//!   store, is filtered out of that store ([`Store`]) and copied read-only;
//!   a refreshable login (OAuth tokens, a ChatGPT login) never is.
//! - **Resume.** At lease end the conversations a session's agent wrote are
//!   moved out of its home into the helper's own `kept/<session>/`, root's.
//!   A resumed session names its predecessor; only once that one has ended,
//!   and only the resumed conversation's files, are copied into the new
//!   home. Keeping them by Storm session, not by the agent's id, means one
//!   session can never plant a conversation another resumes.
//!
//! Every source is opened one component at a time with `O_NOFOLLOW` and
//! must be the expected account's; links are copied as links, never
//! followed. The helper keeps no `CAP_DAC_OVERRIDE`, so the home is root's
//! while it is filled and each entry gets its owner and mode once it is
//! complete. Owners and modes go through [`Ownership`], so the tests read
//! them as data.

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
use serde_json::Value;

use crate::convert::{Entry, Key, Kind, Meta, Node, Ownership};

/// How much one session's snapshot may copy.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 64;
/// How many ended sessions' conversations are kept for Resume.
pub const MAX_KEPT: usize = 256;

/// Never copied from inside a snapshot tree: credential stores and files
/// that commonly hold a token (136e: "credential files are never part of
/// it").
pub const DENIED: &[&str] = &[
    ".credentials.json",
    "credentials.json",
    "auth.json",
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
];

/// Variables a launch may not carry on an isolated host: a refreshable
/// login (136f), even from the operator's env file.
pub const REFRESHABLE_ENV: &[&str] = &["CLAUDE_CODE_OAUTH_REFRESH_TOKEN"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Root's; the session's own group may read, search and run it.
    ReadOnly,
    /// The session user's own copy.
    Private,
}

/// Where an item is copied from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Src {
    /// The host account's home, `<host state>/home`.
    Home,
    /// Storm's Codex home, `<host state>/agents/codex` (decision 130b).
    Codex,
}

/// A provider's own credential store, of which only static API keys are
/// copied (136f's third row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    /// OpenCode's `auth.json`: `{provider: {"type": "api", "key": …}}`;
    /// `oauth` and `wellknown` entries are refreshable or derived.
    OpenCode,
    /// Codex's `auth.json`: an `OPENAI_API_KEY` with no ChatGPT `tokens`.
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    None,
    /// `.claude.json`, without a key Claude Code stored itself.
    ClaudeJson,
    /// Claude Code's plugin registry names install paths in the account's
    /// home; in the copy they name the session's.
    PluginPaths,
    /// Codex keys a hook's trust on its config's path; the copy's is the
    /// session's.
    CodexConfig,
    Store(Store),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub src: Src,
    /// Relative to its source.
    pub from: String,
    /// Relative to the session's home.
    pub to: String,
    pub access: Access,
    pub filter: Filter,
}

fn item(src: Src, from: &str, to: &str, access: Access, filter: Filter) -> Item {
    Item {
        src,
        from: from.into(),
        to: to.into(),
        access,
        filter,
    }
}

/// The shell's start-up files, for every profile: every agent runs a shell.
pub const SHELL_FILES: &[&str] = &[
    ".profile",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".bash_aliases",
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".zlogin",
    ".zlogout",
    ".inputrc",
];

/// 136e's per-profile allowlist. A provider the operator configured under
/// another id gets the shell's files only.
pub fn items(provider: &str, session: &str) -> Vec<Item> {
    use Access::{Private, ReadOnly};
    use Src::{Codex, Home};
    let mut out: Vec<Item> = SHELL_FILES
        .iter()
        .map(|f| item(Home, f, f, ReadOnly, Filter::None))
        .collect();
    let ro = |src, path: &str| item(src, path, path, ReadOnly, Filter::None);
    match provider {
        "claude-code" => {
            for path in [
                ".claude/settings.json",
                ".claude/CLAUDE.md",
                ".claude/agents",
                ".claude/commands",
                ".claude/skills",
                ".claude/hooks",
                ".claude/output-styles",
            ] {
                out.push(ro(Home, path));
            }
            out.push(item(
                Home,
                ".claude/plugins",
                ".claude/plugins",
                ReadOnly,
                Filter::PluginPaths,
            ));
            out.push(item(
                Home,
                ".claude.json",
                ".claude.json",
                Private,
                Filter::ClaudeJson,
            ));
        }
        "opencode" => {
            out.push(ro(Home, ".config/opencode"));
            out.push(item(
                Home,
                ".local/share/opencode/auth.json",
                ".local/share/opencode/auth.json",
                ReadOnly,
                Filter::Store(Store::OpenCode),
            ));
        }
        "codex" => {
            out.push(item(
                Codex,
                "config.toml",
                ".codex/config.toml",
                Private,
                Filter::CodexConfig,
            ));
            let profile = format!("storm-{session}.config.toml");
            out.push(item(
                Codex,
                &profile,
                &format!(".codex/{profile}"),
                ReadOnly,
                Filter::None,
            ));
            for path in ["AGENTS.md", "prompts", "skills", "rules", "plugins"] {
                out.push(item(
                    Codex,
                    path,
                    &format!(".codex/{path}"),
                    ReadOnly,
                    Filter::None,
                ));
            }
            out.push(item(
                Codex,
                "auth.json",
                ".codex/auth.json",
                ReadOnly,
                Filter::Store(Store::Codex),
            ));
        }
        "cursor" => {
            out.push(item(
                Home,
                ".cursor/cli-config.json",
                ".cursor/cli-config.json",
                Private,
                Filter::None,
            ));
            out.push(ro(Home, ".cursor/mcp.json"));
            out.push(ro(Home, ".cursor/rules"));
        }
        _ => {}
    }
    out
}

/// The static keys a store may give a session, or `None` when it has none
/// (a refreshable login is never one).
pub fn static_keys(store: Store, bytes: &[u8]) -> Option<Vec<u8>> {
    let Ok(Value::Object(all)) = serde_json::from_slice::<Value>(bytes) else {
        return None;
    };
    let out = match store {
        Store::OpenCode => {
            let mut keep = serde_json::Map::new();
            for (name, entry) in all {
                if entry["type"] == "api"
                    && let Some(key) = entry["key"].as_str().filter(|k| !k.is_empty())
                {
                    keep.insert(name, serde_json::json!({"type": "api", "key": key}));
                }
            }
            if keep.is_empty() {
                return None;
            }
            Value::Object(keep)
        }
        Store::Codex => {
            let tokens = all.get("tokens").is_some_and(|t| !t.is_null());
            let key = all
                .get("OPENAI_API_KEY")
                .and_then(Value::as_str)
                .filter(|k| !k.is_empty());
            match key {
                Some(key) if !tokens => serde_json::json!({"OPENAI_API_KEY": key}),
                _ => return None,
            }
        }
    };
    serde_json::to_vec_pretty(&out).ok()
}

/// What a filter makes of a file's bytes; `None` leaves it out.
fn filtered(filter: Filter, rel: &[u8], bytes: Vec<u8>, cx: &Paths) -> Option<Vec<u8>> {
    match filter {
        Filter::None => Some(bytes),
        Filter::Store(store) => static_keys(store, &bytes),
        Filter::ClaudeJson => {
            let Ok(Value::Object(mut all)) = serde_json::from_slice::<Value>(&bytes) else {
                return None;
            };
            all.remove("primaryApiKey");
            serde_json::to_vec_pretty(&Value::Object(all)).ok()
        }
        Filter::PluginPaths => {
            if rel != b"installed_plugins.json" && rel != b"known_marketplaces.json" {
                return Some(bytes);
            }
            let Ok(mut v) = serde_json::from_slice::<Value>(&bytes) else {
                return None;
            };
            rewrite_prefix(&mut v, &cx.host_home, &cx.home);
            serde_json::to_vec_pretty(&v).ok()
        }
        Filter::CodexConfig => {
            let text = String::from_utf8(bytes).ok()?;
            let mut table: toml::Table = text.parse().ok()?;
            let from = format!("{}/", cx.codex_home.display());
            let to = format!("{}/", cx.home.join(".codex").display());
            if let Some(toml::Value::Table(hooks)) = table.get_mut("hooks")
                && let Some(toml::Value::Table(state)) = hooks.get_mut("state")
            {
                let old = std::mem::take(state);
                for (key, value) in old {
                    let key = match key.strip_prefix(&from) {
                        Some(rest) => format!("{to}{rest}"),
                        None => key,
                    };
                    state.insert(key, value);
                }
            }
            toml::to_string(&table).ok().map(String::into_bytes)
        }
    }
}

fn rewrite_prefix(v: &mut Value, from: &Path, to: &Path) {
    match v {
        Value::String(s) => {
            let from = from.to_string_lossy();
            if let Some(rest) = s.strip_prefix(from.as_ref())
                && (rest.is_empty() || rest.starts_with('/'))
            {
                *s = format!("{}{rest}", to.display());
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|v| rewrite_prefix(v, from, to)),
        Value::Object(o) => o.values_mut().for_each(|v| rewrite_prefix(v, from, to)),
        _ => {}
    }
}

/// The paths one session's home is built from.
#[derive(Debug, Clone)]
pub struct Paths {
    pub host_state: PathBuf,
    pub host_home: PathBuf,
    pub codex_home: PathBuf,
    /// The session's home.
    pub home: PathBuf,
}

impl Paths {
    pub fn new(host_state: &Path, home: &Path) -> Self {
        Self {
            host_state: host_state.to_path_buf(),
            host_home: host_state.join("home"),
            codex_home: host_state.join("agents").join("codex"),
            home: home.to_path_buf(),
        }
    }
}

// `Stat`'s field types differ between architectures.
#[allow(clippy::unnecessary_cast)]
fn stat_meta(st: &rustix::fs::Stat) -> (Key, Meta) {
    (
        Key {
            dev: st.st_dev as u64,
            ino: st.st_ino as u64,
        },
        Meta {
            uid: st.st_uid,
            gid: st.st_gid,
            mode: st.st_mode as u32 & 0o7777,
        },
    )
}

fn kind_of(st: &rustix::fs::Stat) -> Kind {
    match FileType::from_raw_mode(st.st_mode as _) {
        FileType::Directory => Kind::Dir,
        FileType::RegularFile => Kind::File,
        FileType::Symlink => Kind::Link,
        _ => Kind::Other,
    }
}

fn cname(name: &[u8]) -> Result<CString> {
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        bail!("{} is not a file name", String::from_utf8_lossy(name));
    }
    Ok(CString::new(name)?)
}

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Who owns an entry now, as [`Ownership`] sees it.
fn owner_of(own: &dyn Ownership, st: &rustix::fs::Stat) -> u32 {
    let (key, seen) = stat_meta(st);
    own.meta(key, seen).uid
}

/// A directory under `base`, by a relative path, opened one component at a
/// time without following a link, each the account `owner`'s (when one is
/// named). `None` if a component is missing or is anything but such a
/// directory.
fn open_dir_under(
    base: BorrowedFd<'_>,
    rel: &str,
    owner: Option<u32>,
    own: &dyn Ownership,
) -> Result<Option<OwnedFd>> {
    let mut fd = rustix::fs::openat(base, c".", DIR_FLAGS, Mode::empty())?;
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let name = cname(part.as_bytes())?;
        let next = match rustix::fs::openat(&fd, name.as_c_str(), DIR_FLAGS, Mode::empty()) {
            Ok(next) => next,
            Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP) => {
                return Ok(None);
            }
            Err(e) => return Err(e).with_context(|| format!("opening {rel}")),
        };
        if let Some(o) = owner
            && owner_of(own, &rustix::fs::fstat(&next)?) != o
        {
            return Ok(None);
        }
        fd = next;
    }
    Ok(Some(fd))
}

/// Copies into a session's home, within one budget.
pub struct Copier<'a> {
    own: &'a dyn Ownership,
    /// The session user's uid and primary gid.
    uid: u32,
    gid: u32,
    bytes: u64,
    entries: usize,
}

impl<'a> Copier<'a> {
    pub fn new(own: &'a dyn Ownership, uid: u32, gid: u32) -> Self {
        Self {
            own,
            uid,
            gid,
            bytes: 0,
            entries: 0,
        }
    }

    fn target(&self, access: Access, kind: Kind, src_mode: u32) -> Meta {
        let x = src_mode & 0o100 != 0;
        let (uid, mode) = match (access, kind) {
            (Access::ReadOnly, Kind::Dir) => (0, 0o550),
            (Access::ReadOnly, _) => (0, if x { 0o550 } else { 0o440 }),
            (Access::Private, Kind::Dir) => (self.uid, 0o700),
            (Access::Private, _) => (self.uid, if x { 0o700 } else { 0o600 }),
        };
        Meta {
            uid,
            gid: self.gid,
            mode,
        }
    }

    fn count(&mut self, bytes: u64) -> Result<()> {
        self.entries += 1;
        self.bytes += bytes;
        if self.entries > MAX_ENTRIES || self.bytes > MAX_BYTES {
            bail!("the account's configuration is larger than a session's snapshot may be");
        }
        Ok(())
    }

    /// Gives an entry its final owner and mode, through its descriptor (a
    /// link by name).
    fn settle(
        &self,
        fd: Option<BorrowedFd<'_>>,
        at: Option<(BorrowedFd<'_>, &CStr)>,
        kind: Kind,
        meta: Meta,
    ) -> Result<()> {
        let st = match (fd, at) {
            (Some(fd), _) => rustix::fs::fstat(fd)?,
            (None, Some((dir, name))) => rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)?,
            _ => bail!("nothing to settle"),
        };
        let (key, seen) = stat_meta(&st);
        let node = match (fd, at) {
            (Some(fd), _) => Node::Fd(fd),
            (None, Some((dir, name))) => Node::At(dir, name),
            _ => unreachable!(),
        };
        let e = Entry {
            rel: b"",
            kind,
            key,
            seen,
            node,
        };
        self.own.chown(&e, meta.uid, meta.gid)?;
        if matches!(kind, Kind::Dir | Kind::File) {
            self.own.chmod(&e, meta.mode)?;
        }
        Ok(())
    }

    /// Makes `rel`'s parent directories in the home, root's for now; each
    /// one made is listed so [`Copier::finish`] gives it to the session.
    fn parents(
        &self,
        home: BorrowedFd<'_>,
        rel: &str,
        made: &mut Vec<String>,
    ) -> Result<(OwnedFd, CString)> {
        let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
        let Some((last, dirs)) = parts.split_last() else {
            bail!("an empty destination");
        };
        let mut fd = rustix::fs::openat(home, c".", DIR_FLAGS, Mode::empty())?;
        let mut so_far = String::new();
        for part in dirs {
            let name = cname(part.as_bytes())?;
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(part);
            match rustix::fs::mkdirat(&fd, name.as_c_str(), Mode::from_raw_mode(0o700)) {
                Ok(()) => made.push(so_far.clone()),
                Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(e).with_context(|| format!("making {so_far}")),
            }
            fd = rustix::fs::openat(&fd, name.as_c_str(), DIR_FLAGS, Mode::empty())
                .with_context(|| format!("opening {so_far}"))?;
        }
        Ok((fd, cname(last.as_bytes())?))
    }

    fn write_file(
        &mut self,
        dir: BorrowedFd<'_>,
        name: &CStr,
        bytes: &[u8],
        meta: Meta,
    ) -> Result<()> {
        self.count(bytes.len() as u64)?;
        let fd = rustix::fs::openat(
            dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .with_context(|| format!("creating {}", name.to_string_lossy()))?;
        let mut file = std::fs::File::from(fd);
        file.write_all(bytes)?;
        file.sync_all()?;
        self.settle(Some(file.as_fd()), None, Kind::File, meta)
    }

    fn read_file(fd: OwnedFd, limit: u64) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::from(fd)
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            bail!("a file in the snapshot is too large");
        }
        Ok(bytes)
    }

    /// Copies one entry `name` of `src` (a directory or file of `owner`'s)
    /// to `name` in `dst`, recursively. `rel` is its path inside the item,
    /// for the filter.
    #[allow(clippy::too_many_arguments)]
    fn copy_entry(
        &mut self,
        src: BorrowedFd<'_>,
        name: &CStr,
        dst: BorrowedFd<'_>,
        dst_name: &CStr,
        owner: u32,
        access: Access,
        filter: Filter,
        rel: &mut Vec<u8>,
        depth: usize,
        cx: &Paths,
        skip_denied: bool,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            bail!("the snapshot is nested deeper than {MAX_DEPTH} directories");
        }
        if skip_denied && DENIED.iter().any(|d| d.as_bytes() == name.to_bytes()) {
            return Ok(());
        }
        let st = match rustix::fs::statat(src, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) => st,
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if owner_of(self.own, &st) != owner {
            return Ok(());
        }
        let kind = kind_of(&st);
        let (_, seen) = stat_meta(&st);
        let meta = self.target(access, kind, seen.mode);
        match kind {
            Kind::File => {
                let fd = rustix::fs::openat(
                    src,
                    name,
                    OFlags::RDONLY
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC
                        | OFlags::NONBLOCK
                        | OFlags::NOCTTY,
                    Mode::empty(),
                )?;
                if stat_meta(&rustix::fs::fstat(&fd)?).0 != stat_meta(&st).0 {
                    bail!("a file changed while it was being copied");
                }
                let bytes = Self::read_file(fd, MAX_BYTES - self.bytes.min(MAX_BYTES))?;
                if let Some(bytes) = filtered(filter, rel, bytes, cx) {
                    self.write_file(dst, dst_name, &bytes, meta)?;
                }
            }
            Kind::Dir => {
                let from = rustix::fs::openat(src, name, DIR_FLAGS, Mode::empty())?;
                if stat_meta(&rustix::fs::fstat(&from)?).0 != stat_meta(&st).0 {
                    bail!("a directory changed while it was being copied");
                }
                self.count(0)?;
                rustix::fs::mkdirat(dst, dst_name, Mode::from_raw_mode(0o700))
                    .with_context(|| format!("making {}", dst_name.to_string_lossy()))?;
                let to = rustix::fs::openat(dst, dst_name, DIR_FLAGS, Mode::empty())?;
                let mut names: Vec<CString> = Vec::new();
                for entry in Dir::read_from(&from)? {
                    let entry = entry?;
                    let n = entry.file_name();
                    if n.to_bytes() != b"." && n.to_bytes() != b".." {
                        names.push(n.to_owned());
                    }
                }
                names.sort();
                for n in names {
                    let base = rel.len();
                    if !rel.is_empty() {
                        rel.push(b'/');
                    }
                    rel.extend_from_slice(n.to_bytes());
                    self.copy_entry(
                        from.as_fd(),
                        &n,
                        to.as_fd(),
                        &n,
                        owner,
                        access,
                        filter,
                        rel,
                        depth + 1,
                        cx,
                        skip_denied,
                    )?;
                    rel.truncate(base);
                }
                // Its contents are in: now it can stop being writable.
                self.settle(Some(to.as_fd()), None, Kind::Dir, meta)?;
            }
            Kind::Link => {
                self.count(0)?;
                let target = rustix::fs::readlinkat(src, name, Vec::new())?;
                rustix::fs::symlinkat(target.as_c_str(), dst, dst_name)?;
                self.settle(None, Some((dst, dst_name)), Kind::Link, meta)?;
            }
            // Sockets, FIFOs and devices are not configuration.
            Kind::Other => {}
        }
        Ok(())
    }

    /// Gives the directories [`Copier::parents`] made, and the home itself,
    /// to the session.
    fn finish(&self, home: BorrowedFd<'_>, made: &[String]) -> Result<()> {
        let dir = self.target(Access::Private, Kind::Dir, 0o700);
        for rel in made.iter().rev() {
            if let Some(fd) = open_dir_under(home, rel, None, self.own)? {
                self.settle(Some(fd.as_fd()), None, Kind::Dir, dir)?;
            }
        }
        self.settle(Some(home), None, Kind::Dir, dir)
    }
}

fn open_root(path: &Path) -> Result<OwnedFd> {
    rustix::fs::open(path, DIR_FLAGS, Mode::empty())
        .with_context(|| format!("opening {}", path.display()))
}

/// A resumed conversation: its predecessor and the agent's own id for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    pub from: String,
    pub conversation: String,
}

/// The agent's id for a conversation, as it may name a file: a lowercase
/// UUID for Claude Code and Codex, `ses_` and alphanumerics for OpenCode
/// (decision 89).
pub fn conversation_ok(provider: &str, id: &str) -> bool {
    match provider {
        "claude-code" | "codex" => {
            id.len() == 36
                && id.bytes().enumerate().all(|(i, c)| match i {
                    8 | 13 | 18 | 23 => c == b'-',
                    _ => c.is_ascii_digit() || (b'a'..=b'f').contains(&c),
                })
        }
        "opencode" => id.strip_prefix("ses_").is_some_and(|rest| {
            (1..=64).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        }),
        _ => false,
    }
}

/// Where each resumable agent keeps its conversations in a home.
pub fn conversation_paths(provider: &str) -> &'static [&'static str] {
    match provider {
        "claude-code" => &[".claude/projects"],
        "codex" => &[".codex/sessions"],
        "opencode" => &[
            ".local/share/opencode/opencode.db",
            ".local/share/opencode/opencode.db-wal",
            ".local/share/opencode/storage",
        ],
        _ => &[],
    }
}

/// Builds a session's home at start: the snapshot of the account's config
/// for `provider`, then, for a resume, its conversation. `home` is the
/// session's, empty, made by `prepare`.
#[allow(clippy::too_many_arguments)]
pub fn populate(
    paths: &Paths,
    provider: &str,
    session: &str,
    host_uid: u32,
    uid: u32,
    gid: u32,
    resume: Option<(&Resume, &Path)>,
    own: &dyn Ownership,
) -> Result<()> {
    let mut c = Copier::new(own, uid, gid);
    let home = open_root(&paths.home)?;
    // Root's while it is filled: the helper has no CAP_DAC_OVERRIDE.
    c.settle(
        Some(home.as_fd()),
        None,
        Kind::Dir,
        Meta {
            uid: 0,
            gid: 0,
            mode: 0o700,
        },
    )?;
    let mut made = Vec::new();
    let host_depth = paths.host_state.components().count();
    let state = crate::files::open_owned_dir(&paths.host_state, host_depth, host_uid)?;
    for it in items(provider, session) {
        let base = match it.src {
            Src::Home => "home",
            Src::Codex => "agents/codex",
        };
        let rel = format!("{base}/{}", it.from);
        let (dir_rel, name) = rel.rsplit_once('/').expect("a base");
        let Some(dir) = open_dir_under(state.as_fd(), dir_rel, Some(host_uid), own)? else {
            continue;
        };
        let name = cname(name.as_bytes())?;
        if rustix::fs::statat(&dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW).is_err() {
            continue;
        }
        let (to_dir, to_name) = c.parents(home.as_fd(), &it.to, &mut made)?;
        // A store is named on purpose; everything else skips them.
        let skip_denied = !matches!(it.filter, Filter::Store(_));
        c.copy_entry(
            dir.as_fd(),
            &name,
            to_dir.as_fd(),
            &to_name,
            host_uid,
            it.access,
            it.filter,
            &mut Vec::new(),
            0,
            paths,
            skip_denied,
        )?;
    }
    if let Some((resume, kept)) = resume {
        copy_conversation(
            &mut c,
            home.as_fd(),
            &mut made,
            provider,
            resume,
            kept,
            paths,
        )?;
    }
    c.finish(home.as_fd(), &made)
}

/// Copies the one conversation from the predecessor's kept files.
fn copy_conversation(
    c: &mut Copier,
    home: BorrowedFd<'_>,
    made: &mut Vec<String>,
    provider: &str,
    resume: &Resume,
    kept: &Path,
    cx: &Paths,
) -> Result<()> {
    let recorded = std::fs::read_to_string(kept.join("provider")).unwrap_or_default();
    if recorded != provider {
        bail!("the predecessor's conversations are not {provider}'s");
    }
    let root = open_root(&kept.join("home"))?;
    let own = c.own;
    let id = resume.conversation.as_str();
    let mut found = false;
    let copy = |c: &mut Copier, made: &mut Vec<String>, rel: &str| -> Result<()> {
        let (dir_rel, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let Some(dir) = open_dir_under(root.as_fd(), dir_rel, Some(0), own)? else {
            return Ok(());
        };
        let name = cname(name.as_bytes())?;
        let (to_dir, to_name) = c.parents(home, rel, made)?;
        c.copy_entry(
            dir.as_fd(),
            &name,
            to_dir.as_fd(),
            &to_name,
            0,
            Access::Private,
            Filter::None,
            &mut Vec::new(),
            0,
            cx,
            false,
        )
    };
    match provider {
        "claude-code" => {
            let projects = ".claude/projects";
            for slug in list(root.as_fd(), projects, own)? {
                let slug = String::from_utf8_lossy(&slug).into_owned();
                let file = format!("{projects}/{slug}/{id}.jsonl");
                if is_file(root.as_fd(), &file, own)? {
                    found = true;
                    copy(c, made, &file)?;
                    let dir = format!("{projects}/{slug}/{id}");
                    if open_dir_under(root.as_fd(), &dir, Some(0), own)?.is_some() {
                        copy(c, made, &dir)?;
                    }
                }
            }
        }
        "codex" => {
            let suffix = format!("-{id}.jsonl");
            let mut files = Vec::new();
            find_files(
                root.as_fd(),
                ".codex/sessions",
                own,
                &mut |rel, name| {
                    if name.ends_with(suffix.as_bytes()) {
                        files.push(rel.to_string());
                    }
                },
                0,
            )?;
            for rel in files {
                found = true;
                copy(c, made, &rel)?;
            }
        }
        "opencode" => {
            let db = ".local/share/opencode/opencode.db";
            let wal = ".local/share/opencode/opencode.db-wal";
            if is_file(root.as_fd(), db, own)? {
                let named = |rel: &str| -> Result<bool> {
                    let path = kept.join("home").join(rel);
                    Ok(path.is_file() && contains(&std::fs::read(&path)?, id.as_bytes()))
                };
                found = named(db)? || (is_file(root.as_fd(), wal, own)? && named(wal)?);
                if found {
                    copy(c, made, db)?;
                    if is_file(root.as_fd(), wal, own)? {
                        copy(c, made, wal)?;
                    }
                    let diff = format!(".local/share/opencode/storage/session_diff/{id}.json");
                    if is_file(root.as_fd(), &diff, own)? {
                        copy(c, made, &diff)?;
                    }
                }
            }
        }
        _ => bail!("{provider} has no conversations to resume"),
    }
    if !found {
        bail!("the predecessor kept no conversation {id}");
    }
    Ok(())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// A regular file of root's under a kept root.
fn is_file(root: BorrowedFd<'_>, rel: &str, own: &dyn Ownership) -> Result<bool> {
    let (dir_rel, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let Some(dir) = open_dir_under(root, dir_rel, Some(0), own)? else {
        return Ok(false);
    };
    let name = cname(name.as_bytes())?;
    Ok(
        rustix::fs::statat(&dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|st| kind_of(&st) == Kind::File && owner_of(own, &st) == 0),
    )
}

fn list(root: BorrowedFd<'_>, rel: &str, own: &dyn Ownership) -> Result<Vec<Vec<u8>>> {
    let Some(dir) = open_dir_under(root, rel, Some(0), own)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in Dir::read_from(&dir)? {
        let entry = entry?;
        let n = entry.file_name().to_bytes();
        if n != b"." && n != b".." && !n.contains(&b'/') {
            out.push(n.to_vec());
        }
    }
    out.sort();
    Ok(out)
}

/// Every regular file under `rel`, by its path and name.
fn find_files(
    root: BorrowedFd<'_>,
    rel: &str,
    own: &dyn Ownership,
    f: &mut dyn FnMut(&str, &[u8]),
    depth: usize,
) -> Result<()> {
    if depth > MAX_DEPTH {
        bail!("the kept conversations are nested too deep");
    }
    let Some(dir) = open_dir_under(root, rel, Some(0), own)? else {
        return Ok(());
    };
    for name in list(root, rel, own)? {
        let Ok(text) = std::str::from_utf8(&name) else {
            continue;
        };
        let c = cname(&name)?;
        let Ok(st) = rustix::fs::statat(&dir, c.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) else {
            continue;
        };
        let child = format!("{rel}/{text}");
        match kind_of(&st) {
            Kind::File => f(&child, &name),
            Kind::Dir => find_files(root, &child, own, f, depth + 1)?,
            _ => {}
        }
    }
    Ok(())
}

/// At lease end, with every directory of `home` already root's (the first
/// half of `files::remove_tree`): moves the conversations `provider`'s
/// agent wrote to `kept`, and makes all of it root's alone.
pub fn keep(home: &Path, kept: &Path, provider: &str, own: &dyn Ownership) -> Result<bool> {
    let wanted = conversation_paths(provider);
    if wanted.is_empty() || !home.is_dir() {
        return Ok(false);
    }
    let from = open_root(home)?;
    let mut moved = false;
    if kept.symlink_metadata().is_ok() {
        crate::files::remove_tree(kept, own)?;
    }
    for rel in wanted {
        let (dir_rel, name) = rel.rsplit_once('/').expect("nested");
        // Root's by now; a link the session left anywhere on the way is not
        // a directory, so it is never followed.
        let Some(dir) = open_dir_under(from.as_fd(), dir_rel, Some(0), own)? else {
            continue;
        };
        let name = cname(name.as_bytes())?;
        let Ok(st) = rustix::fs::statat(&dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) else {
            continue;
        };
        if !matches!(kind_of(&st), Kind::Dir | Kind::File) {
            continue;
        }
        let target = kept.join("home").join(dir_rel);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&target)?;
        let to = open_root(&target)?;
        rustix::fs::renameat(&dir, name.as_c_str(), &to, name.as_c_str())
            .with_context(|| format!("keeping {rel}"))?;
        moved = true;
    }
    if !moved {
        return Ok(false);
    }
    crate::convert::walk(kept, &mut |e| {
        own.chown(&e, 0, 0)?;
        match e.kind {
            Kind::Dir => own.chmod(&e, 0o700),
            Kind::File => own.chmod(&e, 0o600),
            _ => Ok(()),
        }
    })?;
    crate::files::write_root_private(&kept.join("provider"), provider.as_bytes())?;
    Ok(true)
}

/// Drops the oldest kept sessions beyond [`MAX_KEPT`].
pub fn prune(kept_root: &Path, own: &dyn Ownership) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(kept_root) else {
        return Ok(());
    };
    let mut all: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| crate::policy::session_ok(&e.file_name().to_string_lossy()))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if all.len() <= MAX_KEPT {
        return Ok(());
    }
    all.sort();
    let excess = all.len() - MAX_KEPT;
    for (_, path) in all.into_iter().take(excess) {
        crate::files::remove_tree(&path, own)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::Recorded;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    const UID: u32 = 2007;
    const GID: u32 = 2007;
    const A: &str = "0f6c1e2a-1111-4c3b-9a9a-000000000001";
    const B: &str = "0f6c1e2a-1111-4c3b-9a9a-000000000002";

    fn me() -> u32 {
        rustix::process::getuid().as_raw()
    }

    struct Fx {
        _t: tempfile::TempDir,
        host_state: PathBuf,
        homes: PathBuf,
        own: Recorded,
    }

    impl Fx {
        fn account(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.host_state.join("home").join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
        fn codex(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.host_state.join("agents/codex").join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
        fn home(&self, session: &str) -> PathBuf {
            let h = self.homes.join(session);
            std::fs::create_dir_all(&h).unwrap();
            h
        }
        fn populate(&self, provider: &str, session: &str) -> Result<PathBuf> {
            let home = self.home(session);
            populate(
                &Paths::new(&self.host_state, &home),
                provider,
                session,
                me(),
                UID,
                GID,
                None,
                &self.own,
            )?;
            Ok(home)
        }
        fn meta(&self, p: &Path) -> Meta {
            let md = std::fs::symlink_metadata(p).unwrap();
            self.own.meta(
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
    }

    fn fx() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let host_state = base.join("host");
        std::fs::create_dir_all(host_state.join("home")).unwrap();
        Fx {
            host_state,
            homes: base.join("homes"),
            own: Recorded::default(),
            _t: t,
        }
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    /// IT-6: the session gets the account's config; what it does to its
    /// copy never reaches the account, and the next session gets the
    /// account's version again.
    #[test]
    fn it_6_edits_to_the_snapshot_never_reach_the_account() {
        let f = fx();
        let settings = f.account(".claude/settings.json", br#"{"theme":"dark"}"#);
        f.account(".claude/skills/review/SKILL.md", b"# review");
        f.account(".bashrc", b"export A=1\n");
        f.account(
            ".claude.json",
            br#"{"mcpServers":{"x":{}},"numStartups":3}"#,
        );
        let home = f.populate("claude-code", "ags_A").unwrap();
        assert_eq!(
            read(&home.join(".claude/settings.json")),
            r#"{"theme":"dark"}"#
        );
        assert_eq!(
            read(&home.join(".claude/skills/review/SKILL.md")),
            "# review"
        );
        assert_eq!(read(&home.join(".bashrc")), "export A=1\n");
        // The session edits its copy (here the test owns every file).
        std::fs::write(home.join(".claude/settings.json"), b"{\"theme\":\"evil\"}").unwrap();
        std::fs::write(home.join(".claude.json"), b"{}").unwrap();
        assert_eq!(read(&settings), r#"{"theme":"dark"}"#);
        let next = f.populate("claude-code", "ags_B").unwrap();
        assert_eq!(
            read(&next.join(".claude/settings.json")),
            r#"{"theme":"dark"}"#
        );
        let json: Value = serde_json::from_str(&read(&next.join(".claude.json"))).unwrap();
        assert_eq!(json["numStartups"], 3);
    }

    /// AT-3 and IG-3 as data: what a session loads is root's and read-only
    /// to it, or its own private copy; its home is its own.
    #[test]
    fn at_3_the_snapshot_is_read_only_or_private_to_the_session() {
        let f = fx();
        f.account(".claude/settings.json", b"{}");
        let hook = f.account(".claude/hooks/check.sh", b"#!/bin/sh\n");
        std::fs::set_permissions(&hook, PermissionsExt::from_mode(0o755)).unwrap();
        f.account(".claude.json", b"{}");
        f.account(".profile", b"PATH=$PATH\n");
        let home = f.populate("claude-code", "ags_A").unwrap();
        let ro = |rel: &str, mode: u32| {
            let m = f.meta(&home.join(rel));
            assert_eq!((m.uid, m.gid, m.mode), (0, GID, mode), "{rel}");
        };
        ro(".claude/settings.json", 0o440);
        ro(".claude/hooks", 0o550);
        ro(".claude/hooks/check.sh", 0o550);
        ro(".profile", 0o440);
        let own = |rel: &str, mode: u32| {
            let m = f.meta(&home.join(rel));
            assert_eq!((m.uid, m.gid, m.mode), (UID, GID, mode), "{rel}");
        };
        own("", 0o700);
        own(".claude", 0o700);
        own(".claude.json", 0o600);
        // Nothing the session reads is writable by any other account.
        for rel in [
            ".claude/settings.json",
            ".claude/hooks/check.sh",
            ".claude.json",
        ] {
            assert_eq!(f.meta(&home.join(rel)).mode & 0o022, 0, "{rel}");
        }
    }

    /// IT-5: no refreshable login is copied, the static keys 136f allows
    /// are read-only, and the account's credential files are untouched.
    #[test]
    fn it_5_no_refreshable_login_reaches_a_home() {
        let f = fx();
        let creds = f.account(
            ".claude/.credentials.json",
            br#"{"claudeAiOauth":{"refreshToken":"sk-ant-ort01-REFRESH"}}"#,
        );
        f.account(".claude/settings.json", b"{}");
        f.account(
            ".claude.json",
            br#"{"primaryApiKey":"sk-ant-api-STORED","oauthAccount":{"emailAddress":"a@b"}}"#,
        );
        let opencode = f.account(
            ".local/share/opencode/auth.json",
            br#"{"anthropic":{"type":"oauth","refresh":"R-OC","access":"A-OC","expires":1},
                "openai":{"type":"api","key":"sk-STATIC-OC"},
                "zen":{"type":"wellknown","key":"K","token":"T-OC"}}"#,
        );
        let codex = f.codex(
            "auth.json",
            br#"{"OPENAI_API_KEY":null,"tokens":{"refresh_token":"R-CX","access_token":"A-CX"}}"#,
        );
        let before: Vec<(PathBuf, Vec<u8>, u32)> = [&creds, &opencode, &codex]
            .iter()
            .map(|p| {
                let md = std::fs::metadata(p).unwrap();
                ((*p).clone(), std::fs::read(p).unwrap(), md.mode())
            })
            .collect();
        let mut all = String::new();
        for (provider, session) in [
            ("claude-code", "ags_C"),
            ("opencode", "ags_O"),
            ("codex", "ags_X"),
        ] {
            let home = f.populate(provider, session).unwrap();
            crate::convert::walk(&home, &mut |e| {
                if e.kind == Kind::File {
                    all.push_str(&read(&home.join(OsStr::from_bytes(e.rel))));
                }
                Ok(())
            })
            .unwrap();
            if provider == "opencode" {
                let auth = home.join(".local/share/opencode/auth.json");
                let v: Value = serde_json::from_str(&read(&auth)).unwrap();
                assert_eq!(
                    v,
                    serde_json::json!({"openai": {"type": "api", "key": "sk-STATIC-OC"}})
                );
                let m = f.meta(&auth);
                assert_eq!((m.uid, m.mode), (0, 0o440), "a static key is read-only");
            }
            if provider == "codex" {
                assert!(!home.join(".codex/auth.json").exists(), "a ChatGPT login");
            }
            assert!(!home.join(".claude/.credentials.json").exists());
        }
        for secret in ["REFRESH", "STORED", "R-OC", "A-OC", "T-OC", "R-CX", "A-CX"] {
            assert!(!all.contains(secret), "{secret} reached a home");
        }
        assert!(all.contains("a@b"), "the rest of .claude.json is kept");
        for (p, bytes, mode) in before {
            let md = std::fs::metadata(&p).unwrap();
            assert_eq!(
                (std::fs::read(&p).unwrap(), md.mode()),
                (bytes, mode),
                "{}",
                p.display()
            );
        }
        assert!(
            f.own.overlay.borrow().keys().all(|k| {
                [&creds, &opencode, &codex].iter().all(|p| {
                    let md = std::fs::metadata(p).unwrap();
                    *k != Key {
                        dev: md.dev(),
                        ino: md.ino(),
                    }
                })
            }),
            "nothing of the account's was chowned or chmodded"
        );
    }

    #[test]
    fn a_static_codex_key_is_copied_and_nothing_else_of_its_store() {
        assert_eq!(
            static_keys(
                Store::Codex,
                br#"{"OPENAI_API_KEY":"sk-1","auth_mode":"apikey","last_refresh":null}"#
            )
            .map(|b| serde_json::from_slice::<Value>(&b).unwrap()),
            Some(serde_json::json!({"OPENAI_API_KEY": "sk-1"}))
        );
        for refused in [
            &br#"{"OPENAI_API_KEY":"sk-1","tokens":{"refresh_token":"r"}}"#[..],
            br#"{"OPENAI_API_KEY":"","tokens":null}"#,
            br#"{"tokens":{"refresh_token":"r"}}"#,
            b"not json",
            b"[]",
        ] {
            assert_eq!(
                static_keys(Store::Codex, refused),
                None,
                "{}",
                String::from_utf8_lossy(refused)
            );
        }
        assert_eq!(
            static_keys(Store::OpenCode, br#"{"a":{"type":"oauth","refresh":"r"}}"#),
            None
        );
    }

    #[test]
    fn credential_files_inside_a_copied_tree_are_left_out() {
        let f = fx();
        f.account(".config/opencode/opencode.json", b"{}");
        for name in DENIED {
            f.account(&format!(".config/opencode/{name}"), b"TOKEN");
            f.account(&format!(".config/opencode/plugin/{name}"), b"TOKEN");
        }
        let home = f.populate("opencode", "ags_A").unwrap();
        assert!(home.join(".config/opencode/opencode.json").is_file());
        for name in DENIED {
            assert!(!home.join(".config/opencode").join(name).exists(), "{name}");
            assert!(
                !home.join(".config/opencode/plugin").join(name).exists(),
                "{name}"
            );
        }
    }

    /// Root never follows a link the account left: a link is copied as a
    /// link, and a linked directory on the way to an item is not entered.
    #[test]
    fn links_are_copied_as_links_never_followed() {
        let f = fx();
        let home_dir = f.host_state.join("home");
        symlink("/etc/shadow", home_dir.join(".bashrc")).unwrap();
        let elsewhere = f.host_state.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("settings.json"), b"SECRET").unwrap();
        symlink(&elsewhere, home_dir.join(".claude")).unwrap();
        let home = f.populate("claude-code", "ags_A").unwrap();
        assert_eq!(
            std::fs::read_link(home.join(".bashrc")).unwrap(),
            Path::new("/etc/shadow")
        );
        assert_eq!(f.meta(&home.join(".bashrc")).uid, 0);
        assert!(!home.join(".claude/settings.json").exists());
    }

    #[test]
    fn only_the_host_accounts_own_files_are_copied() {
        let f = fx();
        let planted = f.account(".zshrc", b"not the host's");
        f.account(".bashrc", b"the host's");
        let md = std::fs::metadata(&planted).unwrap();
        f.own.overlay.borrow_mut().insert(
            Key {
                dev: md.dev(),
                ino: md.ino(),
            },
            Meta {
                uid: 0,
                gid: 0,
                mode: 0o644,
            },
        );
        let home = f.populate("shell", "ags_A").unwrap();
        assert!(!home.join(".zshrc").exists());
        assert!(home.join(".bashrc").exists());
    }

    #[test]
    fn paths_into_the_accounts_home_name_the_sessions() {
        let f = fx();
        let host_home = f.host_state.join("home");
        f.account(
            ".claude/plugins/installed_plugins.json",
            serde_json::json!({"plugins": {"p@m": [{"installPath": format!("{}/.claude/plugins/cache/p", host_home.display())}]}})
                .to_string()
                .as_bytes(),
        );
        let codex_home = f.host_state.join("agents/codex");
        f.codex(
            "config.toml",
            format!(
                "model = \"o\"\n[hooks.state.\"{}/config.toml:session_start:0:0\"]\ntrusted_hash = \"abc\"\n",
                codex_home.display()
            )
            .as_bytes(),
        );
        f.codex(
            "storm-ags_X.config.toml",
            b"[mcp_servers.storm]\ncommand = \"x\"\n",
        );
        f.codex("storm-ags_OTHER.config.toml", b"[mcp_servers.other]\n");
        let home = f.populate("claude-code", "ags_A").unwrap();
        let v: Value =
            serde_json::from_str(&read(&home.join(".claude/plugins/installed_plugins.json")))
                .unwrap();
        assert_eq!(
            v["plugins"]["p@m"][0]["installPath"],
            format!("{}/.claude/plugins/cache/p", home.display())
        );
        let home = f.populate("codex", "ags_X").unwrap();
        let config: toml::Table = read(&home.join(".codex/config.toml")).parse().unwrap();
        let key = format!("{}/.codex/config.toml:session_start:0:0", home.display());
        assert_eq!(
            config["hooks"]["state"][&key]["trusted_hash"].as_str(),
            Some("abc")
        );
        assert_eq!(config["model"].as_str(), Some("o"));
        assert!(home.join(".codex/storm-ags_X.config.toml").is_file());
        assert!(
            !home.join(".codex/storm-ags_OTHER.config.toml").exists(),
            "another session's profile"
        );
        assert_eq!(
            f.meta(&home.join(".codex/config.toml")).uid,
            UID,
            "Codex rewrites its config"
        );
    }

    /// A Claude Code home after a session: two conversations and the rest.
    fn claude_home(f: &Fx, session: &str) -> PathBuf {
        let home = f.home(session);
        let project = home.join(".claude/projects/-w-app");
        std::fs::create_dir_all(project.join(A)).unwrap();
        std::fs::write(project.join(format!("{A}.jsonl")), format!("{session}:{A}")).unwrap();
        std::fs::write(project.join(A).join("tool.txt"), b"a's tool output").unwrap();
        std::fs::write(project.join(format!("{B}.jsonl")), format!("{session}:{B}")).unwrap();
        std::fs::write(home.join(".claude/history.jsonl"), b"not a conversation").unwrap();
        home
    }

    fn end(f: &Fx, home: &Path, kept: &Path, provider: &str) -> bool {
        crate::files::rootify(home, &f.own).unwrap();
        keep(home, kept, provider, &f.own).unwrap()
    }

    /// Resume copies only the resumed conversation, from its own
    /// predecessor's kept files, into a home that is the new session's.
    #[test]
    fn a_resume_gets_only_its_conversation_from_its_own_predecessor() {
        let f = fx();
        let kept_root = f.homes.parent().unwrap().join("kept");
        let a = claude_home(&f, "ags_A");
        assert!(end(&f, &a, &kept_root.join("ags_A"), "claude-code"));
        assert!(
            !a.join(".claude/projects").exists(),
            "moved out of the home"
        );
        let kept = kept_root.join("ags_A");
        let m = f.meta(&kept.join(format!("home/.claude/projects/-w-app/{A}.jsonl")));
        assert_eq!((m.uid, m.mode), (0, 0o600), "kept as root's alone");
        // Another session planted a conversation under A's id in its own home.
        let b = claude_home(&f, "ags_B");
        std::fs::write(
            b.join(format!(".claude/projects/-w-app/{A}.jsonl")),
            b"PLANTED",
        )
        .unwrap();
        assert!(end(&f, &b, &kept_root.join("ags_B"), "claude-code"));

        let home = f.home("ags_C");
        let resume = Resume {
            from: "ags_A".into(),
            conversation: A.into(),
        };
        populate(
            &Paths::new(&f.host_state, &home),
            "claude-code",
            "ags_C",
            me(),
            UID,
            GID,
            Some((&resume, &kept)),
            &f.own,
        )
        .unwrap();
        let project = home.join(".claude/projects/-w-app");
        assert_eq!(
            read(&project.join(format!("{A}.jsonl"))),
            format!("ags_A:{A}")
        );
        assert_eq!(read(&project.join(A).join("tool.txt")), "a's tool output");
        assert!(
            !project.join(format!("{B}.jsonl")).exists(),
            "only the resumed conversation"
        );
        assert!(!home.join(".claude/history.jsonl").exists());
        let m = f.meta(&project.join(format!("{A}.jsonl")));
        assert_eq!((m.uid, m.mode), (UID, 0o600), "the new session's own");

        let wrong = Resume {
            from: "ags_A".into(),
            conversation: "0f6c1e2a-1111-4c3b-9a9a-00000000000f".into(),
        };
        let home = f.home("ags_D");
        assert!(
            populate(
                &Paths::new(&f.host_state, &home),
                "claude-code",
                "ags_D",
                me(),
                UID,
                GID,
                Some((&wrong, &kept)),
                &f.own
            )
            .is_err()
        );
        let home = f.home("ags_E");
        assert!(
            populate(
                &Paths::new(&f.host_state, &home),
                "codex",
                "ags_E",
                me(),
                UID,
                GID,
                Some((&resume, &kept)),
                &f.own
            )
            .is_err(),
            "another agent's conversations"
        );
        // Kept as another agent's, it is not Claude Code's to resume.
        std::fs::write(kept.join("provider"), b"codex").unwrap();
        let home = f.home("ags_F");
        assert!(
            populate(
                &Paths::new(&f.host_state, &home),
                "claude-code",
                "ags_F",
                me(),
                UID,
                GID,
                Some((&resume, &kept)),
                &f.own
            )
            .is_err(),
            "the provider it was kept for"
        );
    }

    #[test]
    fn codex_and_opencode_conversations_are_kept_and_resumed() {
        let f = fx();
        let kept_root = f.homes.parent().unwrap().join("kept");
        let x = f.home("ags_X");
        let day = x.join(".codex/sessions/2026/10/25");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join(format!("rollout-2026-10-25T01-00-00-{A}.jsonl")),
            b"codex a",
        )
        .unwrap();
        std::fs::write(
            day.join(format!("rollout-2026-10-25T02-00-00-{B}.jsonl")),
            b"codex b",
        )
        .unwrap();
        assert!(end(&f, &x, &kept_root.join("ags_X"), "codex"));
        let home = f.home("ags_Y");
        let resume = Resume {
            from: "ags_X".into(),
            conversation: A.into(),
        };
        populate(
            &Paths::new(&f.host_state, &home),
            "codex",
            "ags_Y",
            me(),
            UID,
            GID,
            Some((&resume, &kept_root.join("ags_X"))),
            &f.own,
        )
        .unwrap();
        let day = home.join(".codex/sessions/2026/10/25");
        assert_eq!(
            read(&day.join(format!("rollout-2026-10-25T01-00-00-{A}.jsonl"))),
            "codex a"
        );
        assert!(
            !day.join(format!("rollout-2026-10-25T02-00-00-{B}.jsonl"))
                .exists()
        );

        let o = f.home("ags_O");
        let data = o.join(".local/share/opencode");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(
            data.join("opencode.db"),
            b"SQLite format 3\0...ses_abc123...",
        )
        .unwrap();
        assert!(end(&f, &o, &kept_root.join("ags_O"), "opencode"));
        let home = f.home("ags_P");
        let resume = Resume {
            from: "ags_O".into(),
            conversation: "ses_abc123".into(),
        };
        populate(
            &Paths::new(&f.host_state, &home),
            "opencode",
            "ags_P",
            me(),
            UID,
            GID,
            Some((&resume, &kept_root.join("ags_O"))),
            &f.own,
        )
        .unwrap();
        assert!(home.join(".local/share/opencode/opencode.db").is_file());
        let home = f.home("ags_Q");
        let other = Resume {
            from: "ags_O".into(),
            conversation: "ses_zzz".into(),
        };
        assert!(
            populate(
                &Paths::new(&f.host_state, &home),
                "opencode",
                "ags_Q",
                me(),
                UID,
                GID,
                Some((&other, &kept_root.join("ags_O"))),
                &f.own
            )
            .is_err(),
            "a database without the conversation"
        );
    }

    /// A session that left a link where its conversations go keeps nothing
    /// through it.
    #[test]
    fn keeping_never_follows_a_link_the_session_left() {
        let f = fx();
        let kept_root = f.homes.parent().unwrap().join("kept");
        let victim = f.home("ags_V");
        std::fs::create_dir_all(victim.join(".claude/projects/-w")).unwrap();
        std::fs::write(victim.join(".claude/projects/-w/x.jsonl"), b"victim's").unwrap();
        let a = f.home("ags_A");
        symlink(victim.join(".claude"), a.join(".claude")).unwrap();
        assert!(!end(&f, &a, &kept_root.join("ags_A"), "claude-code"));
        assert!(
            victim.join(".claude/projects/-w/x.jsonl").is_file(),
            "not moved"
        );
        let b = f.home("ags_B");
        std::fs::create_dir_all(b.join(".claude")).unwrap();
        symlink(victim.join(".claude/projects"), b.join(".claude/projects")).unwrap();
        assert!(!end(&f, &b, &kept_root.join("ags_B"), "claude-code"));
        assert!(
            victim.join(".claude/projects/-w/x.jsonl").is_file(),
            "not moved"
        );
    }

    #[test]
    fn only_the_newest_kept_sessions_stay() {
        let f = fx();
        let kept_root = f.homes.parent().unwrap().join("kept");
        for n in 0..MAX_KEPT + 3 {
            std::fs::create_dir_all(kept_root.join(format!("ags_K{n}"))).unwrap();
        }
        std::fs::create_dir_all(kept_root.join("not-a-session")).unwrap();
        prune(&kept_root, &f.own).unwrap();
        assert_eq!(f.own.removed.borrow().len(), 3);
    }

    #[test]
    fn conversation_ids_are_their_agents_shapes() {
        assert!(conversation_ok("claude-code", A));
        assert!(conversation_ok("codex", A));
        assert!(conversation_ok("opencode", "ses_abc"));
        for (p, bad) in [
            ("claude-code", "../../etc/passwd"),
            ("claude-code", "0F6C1E2A-1111-4C3B-9A9A-000000000001"),
            ("opencode", "ses_a/b"),
            ("opencode", "ses_"),
            ("shell", A),
        ] {
            assert!(!conversation_ok(p, bad), "{p} {bad}");
        }
    }
}
