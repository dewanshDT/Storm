//! `runtime.toml` and the workspaces it names (freeze §5.7, §8; decision 77c).
//!
//! The operator owns this file. Every value in it is an operational default,
//! not an architectural limit. Provider secrets never live here: a provider
//! names an env file, which must be `0600`, and Storm never stores or transmits
//! what is in it.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
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
}

fn default_max_sessions() -> u32 {
    8
}

fn default_scrollback() -> usize {
    crate::scrollback::DEFAULT_CAPACITY
}

fn default_forbidden() -> Vec<PathBuf> {
    vec![PathBuf::from("/srv/storm")]
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
        Ok(config)
    }

    /// A workspace root may not be inside, or contain, a Storm data root this
    /// host can see (D3, freeze §8). The permissions of P3 keep
    /// `storm-runtime` out of one anyway; this keeps an operator from pointing
    /// agents at the vault by mistake.
    pub fn check_roots(&self) -> Result<()> {
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
        let path = self.search_path()?;
        let on_path = |p: CliProvider| match &path {
            Some(path) => p.with_path(path.clone()),
            None => p,
        };
        let Some(entries) = &self.providers else {
            return Ok(vec![
                Arc::new(on_path(CliProvider::claude_code())),
                Arc::new(on_path(CliProvider::opencode())),
                Arc::new(on_path(CliProvider::shell())),
            ]);
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
                        (None, "claude-code") => CliProvider::claude_code(),
                        (None, "opencode") => CliProvider::opencode(),
                        (None, "shell") => CliProvider::shell(),
                        (None, other) => bail!("provider `{other}` needs a command"),
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

/// Reads a provider env file. **Refused unless it is `0600`-or-stricter**:
/// it holds API keys, and a world-readable key file is the leak.
pub fn read_env_file(path: &Path) -> Result<Vec<(OsString, OsString)>> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if meta.permissions().mode() & 0o077 != 0 {
        bail!(
            "{} is readable beyond its owner; provider env files must be 0600",
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

/// Resolves a workspace name to its directory, **inside a root only**: a name
/// is never a path, and a symlink that leads out of the root is refused.
pub fn resolve_workspace(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || name.starts_with('.')
        || name.contains('/')
        || name.contains('\0')
        || Path::new(name).is_absolute()
    {
        return None;
    }
    for root in roots {
        let Ok(root) = root.canonicalize() else {
            continue;
        };
        let candidate = root.join(name);
        if let Ok(real) = candidate.canonicalize()
            && real.starts_with(&root)
            && real.is_dir()
        {
            return Some(real);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_root_overlapping_a_storm_data_root_is_refused() {
        let data = tempfile::tempdir().unwrap();
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
        let elsewhere = tempfile::tempdir().unwrap();
        let config = RuntimeConfig {
            workspace_roots: vec![elsewhere.path().to_path_buf()],
            forbidden_roots: vec![data.path().to_path_buf()],
            ..RuntimeConfig::default()
        };
        config.check_roots().unwrap();
    }

    /// An executable named `name` in `dir` that prints its PATH.
    fn agent(dir: &Path, name: &str) {
        let bin = dir.join(name);
        std::fs::write(&bin, "#!/bin/sh\necho PATH=$PATH\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

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
        let defaults = RuntimeConfig::default().providers().unwrap();
        assert_eq!(defaults.len(), 3);
        assert!(
            toml::from_str::<RuntimeConfig>("bogus = 1").is_err(),
            "unknown keys refused"
        );
    }
}
