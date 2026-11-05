//! `runtime.toml` and the workspaces it names (freeze §5.7, §8; decision 77c).
//!
//! The operator owns this file. Every value in it is an operational default,
//! not an architectural limit. Provider secrets never live here: a provider
//! names an env file, which must be `0600`, and Storm never stores or transmits
//! what is in it.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::CliProvider;
use crate::fake::FakeProvider;
use crate::provider::{Provider, ProviderId};

pub const DEFAULT_CONFIG: &str = crate::platform::DEFAULT_CONFIG;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Directories whose subdirectories are this host's workspaces.
    #[serde(default)]
    pub workspace_roots: Vec<PathBuf>,
    /// The providers this host offers. Omitted: the three built-ins.
    #[serde(default)]
    pub providers: Option<Vec<ProviderConfig>>,
    #[serde(default = "default_max_sessions")]
    pub max_sessions: u32,
    #[serde(default = "default_scrollback")]
    pub scrollback_bytes: usize,
    /// Where agent CLIs are looked for, and the `PATH` every session gets
    /// (AM38). Omitted: the platform's default.
    #[serde(default)]
    pub path: Option<Vec<PathBuf>>,
    /// Storm data roots this host must never put a workspace in or around
    /// (D3). The default is the packaged one.
    #[serde(default = "default_forbidden")]
    pub forbidden_roots: Vec<PathBuf>,
    /// Whether a Claude Code or Codex session's enabled plugins' local
    /// (stdio) MCP servers load under `agent_mcp = "storm-only"` (decision
    /// 131b); otherwise the agent loads its plugins itself (decision 132).
    #[serde(default)]
    pub plugin_mcp: PluginMcp,
    /// Whether a session keeps the MCP servers its agent loads by itself —
    /// the account's, its plugins', a project's — beside Storm's
    /// integrations (decision 132), or gets Storm's alone.
    #[serde(default)]
    pub agent_mcp: AgentMcp,
    /// Whether the app may install, update and remove agent CLIs on this
    /// host (decision 133). `"off"` is for an operator who pins CLIs by hand.
    #[serde(default)]
    pub agent_install: AgentInstall,
    /// Whether Storm updates the agents it installed by itself, once a day
    /// and only while none of that agent's sessions runs (133-5).
    #[serde(default)]
    pub agent_updates: AgentUpdates,
    /// Per-session isolation (decision 136a), set by `storm-runtime-isolation
    /// on` and `off`, which also convert the workspaces. On, no session ever
    /// runs as this host's account: one that cannot be isolated is refused.
    #[serde(default)]
    pub isolation: Isolation,
    /// With isolation on, the TCP ports on this host's loopback a session
    /// may reach from its own network (136d); the helper reads them.
    #[serde(default)]
    pub isolation_host_ports: Vec<u16>,
    /// With isolation on, the most one spawned child's workspace may take,
    /// in bytes (decision 137f; default 2 GiB); the helper reads it.
    #[serde(default)]
    pub child_workspace_max: Option<u64>,
    /// And every child's on this host together (default 20 GiB).
    #[serde(default)]
    pub children_max: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Isolation {
    #[default]
    Off,
    PerSession,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginMcp {
    #[default]
    Allow,
    Off,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMcp {
    #[default]
    Own,
    StormOnly,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentInstall {
    #[default]
    On,
    Off,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentUpdates {
    #[default]
    Auto,
    Manual,
}

fn default_max_sessions() -> u32 {
    8
}

fn default_scrollback() -> usize {
    crate::scrollback::DEFAULT_CAPACITY
}

fn default_forbidden() -> Vec<PathBuf> {
    if cfg!(windows) {
        // A Windows server's data lives in a profile, which AM54 refuses.
        Vec::new()
    } else {
        vec![PathBuf::from("/srv/storm")]
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            workspace_roots: Vec::new(),
            providers: None,
            max_sessions: default_max_sessions(),
            scrollback_bytes: default_scrollback(),
            path: None,
            forbidden_roots: default_forbidden(),
            plugin_mcp: PluginMcp::default(),
            agent_mcp: AgentMcp::default(),
            agent_install: AgentInstall::default(),
            agent_updates: AgentUpdates::default(),
            isolation: Isolation::default(),
            isolation_host_ports: Vec::new(),
            child_workspace_max: None,
            children_max: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub id: String,
    /// `cli` (default) or `fake`.
    #[serde(default = "cli_kind")]
    pub kind: String,
    /// For `cli`. Omitted on a built-in id, the built-in launch is used.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// `KEY=VALUE` lines, mode `0600`.
    #[serde(default)]
    pub env_file: Option<PathBuf>,
    /// Provider settings merged into a session's own config, for `opencode`
    /// (G-D22): a session runs with `XDG_CONFIG_HOME` of its own, so the
    /// host's global OpenCode settings (model, agents) are not loaded, and
    /// this is where they come from instead. **Never an MCP server**: an `mcp`
    /// key here is dropped by the writer.
    #[serde(default)]
    pub settings: Option<toml::Value>,
}

fn cli_kind() -> String {
    "cli".into()
}

impl RuntimeConfig {
    pub fn isolated(&self) -> bool {
        self.isolation == Isolation::PerSession
    }

    /// Loads `path`, or the defaults when it does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        config.check_roots()?;
        config.check_host_ports()?;
        config.check_child_caps()?;
        Ok(config)
    }

    /// The helper's rule (`net::host_ports`), so a list it would refuse
    /// stops the host here with the reason, not every session later.
    pub fn check_host_ports(&self) -> Result<()> {
        if self.isolation_host_ports.contains(&0) {
            bail!("isolation_host_ports: 0 is not a port");
        }
        let mut ports = self.isolation_host_ports.clone();
        ports.sort_unstable();
        ports.dedup();
        if ports.len() > 64 {
            bail!("isolation_host_ports: at most 64 ports");
        }
        Ok(())
    }

    /// The helper's rule (`HelperConfig::caps`): a cap of 0 refuses every
    /// child, which no operator means.
    pub fn check_child_caps(&self) -> Result<()> {
        if self.child_workspace_max == Some(0) || self.children_max == Some(0) {
            bail!("child_workspace_max and children_max must be above 0");
        }
        Ok(())
    }

    /// A workspace root may not be inside, or contain, a Storm data root this
    /// host can see (D3, freeze §8). The permissions of P3 keep
    /// `storm-runtime` out of one anyway; this keeps an operator from pointing
    /// agents at the vault by mistake. Nor may it be in, or contain, one of
    /// the platform's protected locations (AM39): on macOS, people's homes
    /// and mounted volumes, which a LaunchDaemon cannot be granted without
    /// Full Disk Access.
    pub fn check_roots(&self) -> Result<()> {
        self.check_roots_against(crate::platform::PROTECTED_ROOTS)
    }

    fn check_roots_against(&self, protected: &[&str]) -> Result<()> {
        for root in &self.workspace_roots {
            let root = canonical(root);
            for forbidden in &self.forbidden_roots {
                let forbidden = canonical(forbidden);
                if root.starts_with(&forbidden) || forbidden.starts_with(&root) {
                    bail!(
                        "workspace root {} overlaps the Storm data root {} — agents must \
                         never work inside the vault store (D3)",
                        root.display(),
                        forbidden.display()
                    );
                }
            }
            for protected in protected.iter().map(Path::new) {
                if root.starts_with(protected) || protected.starts_with(&root) {
                    bail!(
                        "workspace root {} overlaps {} — a Runtime Host never works in \
                         people's homes or on mounted volumes, and needs no Full Disk \
                         Access (AM39). Put workspaces in the host's own root, and clone \
                         or move a checkout there",
                        root.display(),
                        protected.display()
                    );
                }
            }
            if let Some(place) = crate::platform::refused_root(&root) {
                bail!(
                    "workspace root {} is in or contains {place} — a Runtime Host never \
                     works in people's profiles, on network shares or on removable \
                     drives (AM54). Put workspaces in the host's own root, and clone or \
                     move a checkout there",
                    root.display()
                );
            }
        }
        Ok(())
    }

    /// The one `PATH` availability resolves against and sessions run with
    /// (AM38): `path` when set, else the platform's default. `None` keeps
    /// the inherited one.
    pub fn search_path(&self) -> Result<Option<OsString>> {
        match &self.path {
            Some(dirs) => Ok(Some(
                std::env::join_paths(dirs).context("`path` in runtime.toml")?,
            )),
            None => Ok(crate::platform::default_path()),
        }
    }

    /// [`Self::search_path`] with `managed` (the folder of the agents Storm
    /// installed, decision 133-2) first, so a session finds Storm's copy
    /// before any other.
    pub fn search_path_with(&self, managed: &Path) -> Result<OsString> {
        let rest = match self.search_path()? {
            Some(path) => Some(path),
            None => std::env::var_os("PATH"),
        };
        let mut dirs = vec![managed.to_path_buf()];
        dirs.extend(rest.iter().flat_map(std::env::split_paths));
        std::env::join_paths(dirs).context("the host's search path")
    }

    /// The `settings` of the `opencode` provider entry, as JSON.
    pub fn opencode_settings(&self) -> Option<serde_json::Value> {
        self.providers
            .as_ref()?
            .iter()
            .find(|p| p.id == "opencode")?
            .settings
            .as_ref()
            .and_then(|v| serde_json::to_value(v).ok())
    }

    /// The providers this host offers, by id.
    pub fn providers(&self) -> Result<Vec<Arc<dyn Provider>>> {
        self.providers_on(self.search_path()?, None)
    }

    /// [`Self::providers`] on [`Self::search_path_with`] `managed`.
    pub fn providers_with(&self, managed: &Path) -> Result<Vec<Arc<dyn Provider>>> {
        self.providers_on(Some(self.search_path_with(managed)?), None)
    }

    /// [`Self::providers_with`], each `cli` provider's sessions started by
    /// the isolation helper (decision 136).
    pub fn providers_isolated(
        &self,
        managed: &Path,
        link: crate::isolation::Link,
    ) -> Result<Vec<Arc<dyn Provider>>> {
        self.providers_on(Some(self.search_path_with(managed)?), Some(link))
    }

    fn providers_on(
        &self,
        path: Option<OsString>,
        isolation: Option<crate::isolation::Link>,
    ) -> Result<Vec<Arc<dyn Provider>>> {
        let on_path = |p: CliProvider| {
            let p = match &path {
                Some(path) => p.with_path(path.clone()),
                None => p,
            };
            match &isolation {
                Some(link) => p.isolated(link.clone()),
                None => p,
            }
        };
        // With no list, every built-in (decision 130): one that is not
        // installed is offered as such, and the launcher says so.
        let Some(entries) = &self.providers else {
            return Ok(crate::profile::BUILT_IN
                .iter()
                .map(|p| Arc::new(on_path(CliProvider::built_in(p))) as Arc<dyn Provider>)
                .collect());
        };
        let mut out: Vec<Arc<dyn Provider>> = Vec::new();
        for entry in entries {
            let id = ProviderId::new(&entry.id)
                .with_context(|| format!("provider id `{}` is not valid", entry.id))?;
            let provider: Arc<dyn Provider> = match entry.kind.as_str() {
                "fake" => Arc::new(FakeProvider::new()),
                "cli" => {
                    let base = match (entry.command.as_deref(), id.as_str()) {
                        (Some(cmd), _) => CliProvider::new(id.clone(), cmd, &entry.args),
                        (None, built_in) => match crate::profile::get(built_in) {
                            Some(p) => CliProvider::built_in(p),
                            None => bail!("provider `{built_in}` needs a command"),
                        },
                    };
                    let base = on_path(base);
                    Arc::new(match &entry.env_file {
                        Some(path) => base.with_env(read_env_file(path)?),
                        None => base,
                    })
                }
                other => bail!("provider `{}` has unknown kind `{other}`", entry.id),
            };
            out.push(provider);
        }
        Ok(out)
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Reads a provider env file. **Refused unless it is `0600`-or-stricter**
/// (on Windows: unless no broad group may read it, AM49): it holds API
/// keys, and a world-readable key file is the leak.
pub fn read_env_file(path: &Path) -> Result<Vec<(OsString, OsString)>> {
    let private =
        crate::platform::is_private(path).with_context(|| format!("reading {}", path.display()))?;
    if !private {
        bail!(
            "{} is readable beyond its owner; provider env files must be 0600 \
             (on Windows: readable by no group such as Users or Everyone)",
            path.display()
        );
    }
    let text = std::fs::read_to_string(path)?;
    let mut vars = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .with_context(|| format!("{}:{}: expected KEY=VALUE", path.display(), n + 1))?;
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(v);
        vars.push((k.trim().into(), v.into()));
    }
    Ok(vars)
}

/// The workspaces: the non-hidden subdirectories of each root. The first root
/// to name a workspace wins; a later duplicate is skipped, since a workspace's
/// identity is `(host_id, name)`.
pub fn list_workspaces(roots: &[PathBuf]) -> Vec<String> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut names = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut here: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        here.sort();
        for name in here {
            if seen.insert(name.clone(), ()).is_none() {
                names.push(name);
            }
        }
    }
    names
}

/// Each of `names`' canonical directory, as a session would start in it, so
/// the server's WS-9 check (137-2) sees two names for one directory
/// (overlapping roots) as one. A name that no longer resolves is left out,
/// which the server reads as possibly any workspace.
pub fn workspace_paths(roots: &[PathBuf], names: &[String]) -> HashMap<String, String> {
    let roots = canonical_roots(roots);
    names
        .iter()
        .filter_map(|name| {
            let dir = resolve_in(&roots, name)?;
            Some((name.clone(), dir.display().to_string()))
        })
        .collect()
}

/// Resolves a workspace name to its directory, **inside a root only**: a name
/// is never a path, and a symlink that leads out of the root is refused.
pub fn resolve_workspace(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || name.starts_with('.')
        || name.chars().any(std::path::is_separator)
        || (cfg!(windows) && name.contains(':'))
        || name.contains('\0')
        || Path::new(name).is_absolute()
    {
        return None;
    }
    resolve_in(&canonical_roots(roots), name)
}

/// Whether `name` may name a new workspace (decision 141c): what
/// [`resolve_workspace`] accepts, and no control character, no `\` or `:`
/// on any platform, and nothing a reader could mistake for another name.
pub fn new_workspace_name_ok(name: &str) -> bool {
    (1..=128).contains(&name.chars().count())
        && !name.starts_with('.')
        && name.trim() == name
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
}

/// Makes the workspace `name` in the first root (decision 141c), with
/// `create_dir`, never through a link, and `git init` it if asked. Never
/// replaces anything: a name in any root is `exists`. The error is the
/// stable code the server is told.
pub fn create_workspace(roots: &[PathBuf], name: &str, git: bool) -> Result<String, &'static str> {
    if !new_workspace_name_ok(name) {
        return Err("bad_name");
    }
    let root = roots
        .first()
        .and_then(|r| r.canonicalize().ok())
        .filter(|r| r.is_dir())
        .ok_or("no_root")?;
    // A later root's workspace of that name would be hidden by this one.
    if roots
        .iter()
        .any(|r| std::fs::symlink_metadata(r.join(name)).is_ok())
    {
        return Err("exists");
    }
    let dir = root.join(name);
    if let Err(e) = std::fs::create_dir(&dir) {
        return Err(match e.kind() {
            std::io::ErrorKind::AlreadyExists => "exists",
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
                "not_writable"
            }
            _ => "failed",
        });
    }
    if git && let Err(e) = git_init(&dir) {
        tracing::warn!(error = %e, "new workspace: git init failed; removed it");
        let _ = std::fs::remove_dir_all(&dir);
        return Err("git_failed");
    }
    Ok(name.to_string())
}

fn git_init(dir: &Path) -> Result<()> {
    let mut git = std::process::Command::new("git");
    git.args(["init", "-q", "-b", "main", "--"])
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    // The account's own template directory or hooks are not the folder's.
    #[cfg(unix)]
    git.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    let out = git.output().context("running git")?;
    if !out.status.success() {
        bail!("git init: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn canonical_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots.iter().filter_map(|r| r.canonicalize().ok()).collect()
}

fn resolve_in(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    roots.iter().find_map(|root| {
        let real = root.join(name).canonicalize().ok()?;
        (real.starts_with(root) && real.is_dir()).then_some(real)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// 141c: a new workspace is one plain name, made in the first root
    /// only, never over anything that exists in any root.
    #[test]
    fn a_new_workspace_is_made_in_the_first_root_and_never_over_one() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::create_dir(second.path().join("taken")).unwrap();
        let roots = vec![first.path().to_path_buf(), second.path().to_path_buf()];

        assert_eq!(
            create_workspace(&roots, "demo 2", false),
            Ok("demo 2".into())
        );
        assert!(first.path().join("demo 2").is_dir());
        assert!(!first.path().join("demo 2/.git").exists());
        assert!(list_workspaces(&roots).contains(&"demo 2".to_string()));
        for taken in ["demo 2", "taken"] {
            assert_eq!(
                create_workspace(&roots, taken, false),
                Err("exists"),
                "{taken}"
            );
        }
        let long = "x".repeat(129);
        for bad in [
            "",
            ".",
            "..",
            ".hidden",
            "a/b",
            "../x",
            "/etc",
            "a\\b",
            "c:",
            "bell\u{7}",
            "new\nline",
            " pad",
            "pad ",
            &long,
        ] {
            assert_eq!(
                create_workspace(&roots, bad, false),
                Err("bad_name"),
                "{bad:?}"
            );
        }
        assert!(!second.path().join("demo 2").exists());

        let gone = vec![first.path().join("missing"), second.path().to_path_buf()];
        assert_eq!(create_workspace(&gone, "x", false), Err("no_root"));
        assert!(!second.path().join("x").exists(), "never a later root");
    }

    #[cfg(unix)]
    #[test]
    fn a_new_workspace_never_goes_through_a_link() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path().join("nowhere"), root.path().join("dangling"))
            .unwrap();
        let roots = vec![root.path().to_path_buf()];
        assert_eq!(create_workspace(&roots, "dangling", false), Err("exists"));
        assert!(!outside.path().join("nowhere").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_new_workspace_can_start_as_a_git_repository_on_main() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let roots = vec![root.path().to_path_buf()];
        assert_eq!(create_workspace(&roots, "repo", true), Ok("repo".into()));
        let head = std::fs::read_to_string(root.path().join("repo/.git/HEAD")).unwrap();
        assert_eq!(head.trim(), "ref: refs/heads/main");
    }

    #[cfg(unix)]
    #[test]
    fn a_name_is_never_a_path_and_a_symlink_cannot_leave_the_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("storm")).unwrap();
        std::fs::create_dir(root.path().join(".hidden")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        let roots = vec![root.path().to_path_buf()];

        assert!(resolve_workspace(&roots, "storm").is_some());
        for bad in ["..", "../x", "/etc", "storm/../..", ".hidden", "", "escape"] {
            assert!(resolve_workspace(&roots, bad).is_none(), "{bad:?}");
        }
        let listed = list_workspaces(&roots);
        assert!(listed.contains(&"storm".to_string()));
        assert!(!listed.contains(&".hidden".to_string()));
    }

    /// 137-2: a root inside another lists one directory under two names;
    /// their reported directories are what tells the server so.
    #[cfg(unix)]
    #[test]
    fn overlapping_roots_report_one_directory_for_both_names() {
        let outer = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outer.path().join("storm/site")).unwrap();
        std::fs::create_dir(outer.path().join("other")).unwrap();
        let linked = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outer.path(), linked.path().join("root")).unwrap();
        // The same root through a symlink first, then a root inside it.
        let roots = vec![linked.path().join("root"), outer.path().join("storm")];
        let paths = workspace_paths(&roots, &list_workspaces(&roots));
        let real = outer.path().canonicalize().unwrap();
        assert_eq!(paths["storm"], real.join("storm").display().to_string());
        assert_eq!(paths["site"], real.join("storm/site").display().to_string());
        assert_eq!(paths["other"], real.join("other").display().to_string());
        assert_eq!(paths.len(), list_workspaces(&roots).len());
    }

    /// A scratch directory a workspace root may be in: on Windows, the
    /// runner's temp is under `C:\Users`, which AM54 refuses.
    fn root_dir() -> tempfile::TempDir {
        match std::env::var_os("ProgramData").filter(|_| cfg!(windows)) {
            Some(dir) => tempfile::tempdir_in(dir).unwrap(),
            None => tempfile::tempdir().unwrap(),
        }
    }

    #[test]
    fn a_root_overlapping_a_storm_data_root_is_refused() {
        let data = root_dir();
        let inside = data.path().join("vaults");
        std::fs::create_dir(&inside).unwrap();
        for root in [inside.clone(), data.path().to_path_buf()] {
            let config = RuntimeConfig {
                workspace_roots: vec![root],
                forbidden_roots: vec![data.path().to_path_buf()],
                ..RuntimeConfig::default()
            };
            assert!(config.check_roots().is_err());
        }
        let elsewhere = root_dir();
        let config = RuntimeConfig {
            workspace_roots: vec![elsewhere.path().to_path_buf()],
            forbidden_roots: vec![data.path().to_path_buf()],
            ..RuntimeConfig::default()
        };
        config.check_roots().unwrap();
    }

    #[cfg(unix)]
    /// An executable named `name` in `dir` that prints its PATH.
    fn agent(dir: &Path, name: &str) {
        let bin = dir.join(name);
        std::fs::write(&bin, "#!/bin/sh\necho PATH=$PATH\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn agent_clis_are_found_on_the_configured_path_not_only_the_hosts() {
        // AM38: under launchd the host's own PATH is bare, so `claude` in
        // /opt/homebrew/bin was reported not installed. A configured `path`
        // is where availability looks, for built-ins and custom providers.
        let brew = tempfile::tempdir().unwrap();
        agent(brew.path(), "claude");
        agent(brew.path(), "storm-test-agent");
        let text = format!(
            "path = [{:?}, \"/usr/bin\", \"/bin\"]\n\
             [[providers]]\nid = \"claude-code\"\n\
             [[providers]]\nid = \"custom\"\ncommand = \"storm-test-agent\"\n",
            brew.path()
        );
        let config: RuntimeConfig = toml::from_str(&text).unwrap();
        let path = config.search_path().unwrap().unwrap();
        assert_eq!(
            path,
            OsString::from(format!("{}:/usr/bin:/bin", brew.path().display()))
        );
        for p in config.providers().unwrap() {
            assert_eq!(
                p.available(),
                crate::provider::Availability::Available,
                "{} not found on {path:?}",
                p.id().as_str()
            );
        }
        // Without it, the same CLIs are not on the host's PATH.
        let bare: RuntimeConfig =
            toml::from_str("[[providers]]\nid = \"custom\"\ncommand = \"storm-test-agent\"\n")
                .unwrap();
        assert_eq!(
            bare.providers().unwrap()[0].available(),
            crate::provider::Availability::NotInstalled
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_root_in_or_around_a_protected_location_is_refused() {
        // AM39, with macOS's list: a home, a volume, the data volume's
        // spelling of a home, and any root that contains them.
        let protected = crate::platform::launchd::PROTECTED_ROOTS;
        for root in [
            "/Users/alice/Developer",
            "/Users",
            "/Volumes/NAS/storm",
            "/Network/Servers/x",
            "/System/Volumes/Data/Users/alice/Developer",
            "/",
            "/System",
        ] {
            let config = RuntimeConfig {
                workspace_roots: vec![PathBuf::from(root)],
                forbidden_roots: vec![],
                ..RuntimeConfig::default()
            };
            let err = config
                .check_roots_against(protected)
                .unwrap_err()
                .to_string();
            assert!(err.contains("AM39"), "{root}: {err}");
        }
        for root in [
            "/Library/StormRuntime/workspaces",
            "/opt/work",
            "/Users-shared",
        ] {
            let config = RuntimeConfig {
                workspace_roots: vec![PathBuf::from(root)],
                forbidden_roots: vec![],
                ..RuntimeConfig::default()
            };
            config
                .check_roots_against(protected)
                .unwrap_or_else(|e| panic!("{root}: {e}"));
        }
    }

    #[test]
    fn the_platforms_own_list_is_what_check_roots_enforces() {
        if let Some(first) = crate::platform::PROTECTED_ROOTS.first() {
            let config = RuntimeConfig {
                workspace_roots: vec![Path::new(first).join("x")],
                ..RuntimeConfig::default()
            };
            assert!(config.check_roots().is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_env_file_must_be_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude.env");
        std::fs::write(&path, "# comment\nANTHROPIC_API_KEY=\"sk-test\"\nX = y\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_env_file(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let vars = read_env_file(&path).unwrap();
        assert_eq!(vars[0], ("ANTHROPIC_API_KEY".into(), "sk-test".into()));
        assert_eq!(vars[1], ("X".into(), "y".into()));
    }

    #[test]
    fn the_config_parses_and_defaults() {
        let config: RuntimeConfig = toml::from_str(
            r#"
            workspace_roots = ["/work"]
            max_sessions = 3
            [[providers]]
            id = "fake"
            kind = "fake"
            [[providers]]
            id = "claude-code"
            "#,
        )
        .unwrap();
        assert_eq!(config.max_sessions, 3);
        let ids: Vec<String> = config
            .providers()
            .unwrap()
            .iter()
            .map(|p| p.id().as_str().to_string())
            .collect();
        assert_eq!(ids, ["fake", "claude-code"]);
        // With no list, every built-in, in the profile table's order
        // (decision 130).
        let defaults: Vec<String> = RuntimeConfig::default()
            .providers()
            .unwrap()
            .iter()
            .map(|p| p.id().as_str().to_string())
            .collect();
        assert_eq!(
            defaults,
            [
                "claude-code",
                "opencode",
                "codex",
                "cursor",
                "command-code",
                "shell"
            ]
        );
        assert!(
            toml::from_str::<RuntimeConfig>("bogus = 1").is_err(),
            "unknown keys refused"
        );
    }

    /// Decision 131b: plugin servers are allowed unless the host says off.
    #[test]
    fn agent_install_and_updates_default_on_and_auto() {
        let c = RuntimeConfig::default();
        assert_eq!(c.agent_install, AgentInstall::On);
        assert_eq!(c.agent_updates, AgentUpdates::Auto);
        let off: RuntimeConfig =
            toml::from_str("agent_install = \"off\"\nagent_updates = \"manual\"").unwrap();
        assert_eq!(off.agent_install, AgentInstall::Off);
        assert_eq!(off.agent_updates, AgentUpdates::Manual);
    }

    /// Decision 136a: off unless the operator's command turned it on, and
    /// only its two values parse.
    #[test]
    fn isolation_defaults_off() {
        assert_eq!(RuntimeConfig::default().isolation, Isolation::Off);
        let on: RuntimeConfig = toml::from_str("isolation = \"per-session\"").unwrap();
        assert_eq!(on.isolation, Isolation::PerSession);
        for bad in [
            "isolation = \"on\"",
            "isolation = true",
            "isolation = \"per_session\"",
        ] {
            assert!(toml::from_str::<RuntimeConfig>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_child_caps_are_bytes_above_zero() {
        let c: RuntimeConfig =
            toml::from_str("child_workspace_max = 1073741824\nchildren_max = 10737418240").unwrap();
        assert_eq!(
            (c.child_workspace_max, c.children_max),
            (Some(1 << 30), Some(10 << 30))
        );
        assert!(c.check_child_caps().is_ok());
        assert!(RuntimeConfig::default().check_child_caps().is_ok());
        let zero: RuntimeConfig = toml::from_str("children_max = 0").unwrap();
        assert!(zero.check_child_caps().is_err());
        assert!(toml::from_str::<RuntimeConfig>("child_workspace_max = \"2 GiB\"").is_err());
    }

    #[test]
    fn isolation_host_ports_default_empty_and_are_checked() {
        assert!(RuntimeConfig::default().isolation_host_ports.is_empty());
        let c: RuntimeConfig = toml::from_str("isolation_host_ports = [11434, 5432]").unwrap();
        assert_eq!(c.isolation_host_ports, [11434, 5432]);
        c.check_host_ports().unwrap();
        let zero: RuntimeConfig = toml::from_str("isolation_host_ports = [0]").unwrap();
        assert!(zero.check_host_ports().is_err());
        let many = RuntimeConfig {
            isolation_host_ports: (1..=65).collect(),
            ..RuntimeConfig::default()
        };
        assert!(many.check_host_ports().is_err());
        assert!(toml::from_str::<RuntimeConfig>("isolation_host_ports = [70000]").is_err());
    }

    #[test]
    fn the_managed_folder_comes_first_on_the_search_path() {
        let c: RuntimeConfig = toml::from_str("path = [\"/usr/bin\", \"/bin\"]").unwrap();
        let path = c
            .search_path_with(Path::new("/state/agents/managed/bin"))
            .unwrap();
        let dirs: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(
            dirs,
            [
                PathBuf::from("/state/agents/managed/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin")
            ]
        );
    }

    #[test]
    fn agent_mcp_defaults_to_the_agents_own() {
        assert_eq!(RuntimeConfig::default().agent_mcp, AgentMcp::Own);
        let only: RuntimeConfig = toml::from_str("agent_mcp = \"storm-only\"").unwrap();
        assert_eq!(only.agent_mcp, AgentMcp::StormOnly);
        assert!(toml::from_str::<RuntimeConfig>("agent_mcp = \"strict\"").is_err());
    }

    #[test]
    fn plugin_mcp_defaults_to_allow() {
        assert_eq!(RuntimeConfig::default().plugin_mcp, PluginMcp::Allow);
        let off: RuntimeConfig = toml::from_str("plugin_mcp = \"off\"").unwrap();
        assert_eq!(off.plugin_mcp, PluginMcp::Off);
        assert!(toml::from_str::<RuntimeConfig>("plugin_mcp = \"maybe\"").is_err());
    }

    /// AM54: a workspace name is never a path on Windows either, whichever
    /// separator or drive spelling it tries.
    #[test]
    fn a_name_with_any_separator_is_refused() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("storm")).unwrap();
        let roots = vec![root.path().to_path_buf()];
        assert!(resolve_workspace(&roots, "storm").is_some());
        for bad in ["storm/..", "../storm", "a/b"] {
            assert!(resolve_workspace(&roots, bad).is_none(), "{bad:?}");
        }
        if cfg!(windows) {
            for bad in [r"..\storm", r"storm\..", "C:storm", r"C:\Windows"] {
                assert!(resolve_workspace(&roots, bad).is_none(), "{bad:?}");
            }
        }
    }

    /// AM54: on Windows the machine decides; a root under the profiles
    /// directory is refused, Storm's own and a temp directory are not.
    #[cfg(windows)]
    #[test]
    fn a_root_in_a_profile_is_refused_on_windows() {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let profile = PathBuf::from(format!(r"{drive}\Users"));
        let config = RuntimeConfig {
            workspace_roots: vec![profile.join("someone").join("src")],
            forbidden_roots: vec![],
            ..RuntimeConfig::default()
        };
        let err = config.check_roots().unwrap_err().to_string();
        assert!(err.contains("AM54"), "{err}");
        let own = root_dir();
        let config = RuntimeConfig {
            workspace_roots: vec![own.path().to_path_buf()],
            forbidden_roots: vec![],
            ..RuntimeConfig::default()
        };
        config.check_roots().unwrap();
    }
}
