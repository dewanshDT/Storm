//! What the helper reads from the operator's `runtime.toml`: the workspace
//! roots, the mode, and the provider ids. Only the operator's file, never a
//! request, says which program a session runs (136c).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// The providers a host offers when `runtime.toml` lists none (decision
/// 130). Pinned against storm-runtime's `profile::BUILT_IN` by its
/// packaging tests.
pub const BUILT_IN_PROVIDERS: &[&str] = &[
    "claude-code",
    "opencode",
    "codex",
    "cursor",
    "command-code",
    "shell",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Isolation {
    #[default]
    Off,
    PerSession,
}

/// What each built-in runs (storm-runtime's `profile::BUILT_IN`; pinned by
/// its packaging tests). `shell` is the host account's login shell.
pub const BUILT_IN_COMMANDS: &[(&str, &str)] = &[
    ("claude-code", "claude"),
    ("opencode", "opencode"),
    ("codex", "codex"),
    ("cursor", "cursor-agent"),
    ("command-code", "command-code"),
    ("shell", ""),
];

#[derive(Debug, Clone, Deserialize)]
struct Provider {
    id: String,
    #[serde(default)]
    command: Option<String>,
}

/// The fields the helper needs; the rest of the file is the host's.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HelperConfig {
    #[serde(default)]
    pub workspace_roots: Vec<PathBuf>,
    #[serde(default)]
    pub isolation: Isolation,
    #[serde(default)]
    providers: Option<Vec<Provider>>,
    /// Where agent CLIs are looked for (the host's `path`).
    #[serde(default)]
    pub path: Option<Vec<PathBuf>>,
}

impl HelperConfig {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn provider_ids(&self) -> Vec<String> {
        match &self.providers {
            Some(list) => list.iter().map(|p| p.id.clone()).collect(),
            None => BUILT_IN_PROVIDERS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Each provider id with the command it runs: the operator's, or the
    /// built-in's. An empty command is the login shell.
    pub fn provider_commands(&self) -> Vec<(String, String)> {
        let built_in = |id: &str| {
            BUILT_IN_COMMANDS
                .iter()
                .find(|(b, _)| *b == id)
                .map(|(_, c)| c.to_string())
        };
        match &self.providers {
            Some(list) => list
                .iter()
                .filter_map(|p| {
                    let command = p.command.clone().or_else(|| built_in(&p.id))?;
                    Some((p.id.clone(), command))
                })
                .collect(),
            None => BUILT_IN_COMMANDS
                .iter()
                .map(|(id, c)| (id.to_string(), c.to_string()))
                .collect(),
        }
    }

    /// The roots as the system resolves them; one that does not exist is
    /// left out, as the host leaves it out.
    pub fn canonical_roots(&self) -> Vec<PathBuf> {
        self.workspace_roots
            .iter()
            .filter_map(|r| r.canonicalize().ok())
            .filter(|r| r.is_absolute() && r != Path::new("/"))
            .collect()
    }
}

/// A workspace is a directory directly under a root whose name does not
/// start with a dot (the host's `resolve_workspace`). Symlinks are not
/// workspaces here: the conversion never follows one.
pub fn workspaces(roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for root in roots {
        let entries =
            std::fs::read_dir(root).with_context(|| format!("listing {}", root.display()))?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            if name.as_encoded_bytes().first() == Some(&b'.') {
                continue;
            }
            if entry.file_type()?.is_dir() {
                out.push(root.join(name));
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_defaults_off_and_only_two_values_parse() {
        assert_eq!(HelperConfig::parse("").unwrap().isolation, Isolation::Off);
        assert_eq!(
            HelperConfig::parse("isolation = \"per-session\"")
                .unwrap()
                .isolation,
            Isolation::PerSession
        );
        assert!(HelperConfig::parse("isolation = \"on\"").is_err());
        assert!(HelperConfig::parse("isolation = true").is_err());
    }

    #[test]
    fn providers_come_from_the_operator_or_the_built_ins() {
        let c = HelperConfig::parse(
            "max_sessions = 3\n[[providers]]\nid = \"shell\"\ncommand = \"/bin/bash\"\n",
        )
        .unwrap();
        assert_eq!(c.provider_ids(), ["shell"]);
        assert_eq!(
            c.provider_commands(),
            [("shell".to_string(), "/bin/bash".to_string())]
        );
        let built = HelperConfig::parse("[[providers]]\nid = \"claude-code\"\n").unwrap();
        assert_eq!(
            built.provider_commands(),
            [("claude-code".to_string(), "claude".to_string())]
        );
        assert_eq!(
            BUILT_IN_COMMANDS
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            BUILT_IN_PROVIDERS
        );
        assert_eq!(
            HelperConfig::parse("").unwrap().provider_ids().len(),
            BUILT_IN_PROVIDERS.len()
        );
    }

    #[test]
    fn workspaces_are_the_undotted_directories_under_each_root() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("ws");
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::write(root.join("file"), b"").unwrap();
        std::os::unix::fs::symlink(root.join("a"), root.join("link")).unwrap();
        let found = workspaces(std::slice::from_ref(&root)).unwrap();
        assert_eq!(found, [root.join("a"), root.join("b")]);
    }
}
