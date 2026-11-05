//! What the helper reads from the operator's `runtime.toml`: the workspace
//! roots, the mode, and the provider ids. Only the operator's file, never a
//! request, says which program a session runs (136c).

use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
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
    /// The host's loopback ports an isolated session may reach (136d).
    #[serde(default)]
    pub isolation_host_ports: Vec<u16>,
    /// The most one child workspace may take, in bytes (137f).
    #[serde(default)]
    pub child_workspace_max: Option<u64>,
    /// The most every child workspace on the host may take together.
    #[serde(default)]
    pub children_max: Option<u64>,
}

pub const DEFAULT_CHILD_WORKSPACE_MAX: u64 = 2 << 30;
pub const DEFAULT_CHILDREN_MAX: u64 = 20 << 30;

/// 137f's two caps, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub child: u64,
    pub children: u64,
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

    /// A cap of 0 would refuse every child, which is never what an
    /// operator means: it is refused here rather than at every start.
    pub fn caps(&self) -> Result<Caps> {
        let caps = Caps {
            child: self
                .child_workspace_max
                .unwrap_or(DEFAULT_CHILD_WORKSPACE_MAX),
            children: self.children_max.unwrap_or(DEFAULT_CHILDREN_MAX),
        };
        if caps.child == 0 || caps.children == 0 {
            anyhow::bail!("child_workspace_max and children_max must be above 0");
        }
        Ok(caps)
    }

    /// The configured roots, each refused unless it is absolute, in its
    /// normal form, clear of the system's and Storm's own directories, and
    /// opened from `/` without following a link, a directory at every
    /// component (G-136-2). This runs as root over a path whose parent the
    /// host account may own, so a resolved link would be the operator's
    /// trust handed to that account. A root that does not exist is left
    /// out, as the host leaves it out.
    pub fn roots(&self, state: &Path, host_state: &Path) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for root in &self.workspace_roots {
            check_root_place(root, state, host_state)?;
            match crate::files::open_dir(root) {
                Ok(_) => out.push(root.components().collect()),
                Err(e) if crate::files::errno_of(&e) == Some(rustix::io::Errno::NOENT) => {}
                Err(e) => {
                    return Err(e.context(format!(
                        "the workspace root {} is refused: it must be a real directory, \
                         with no symbolic link anywhere in its path",
                        root.display()
                    )));
                }
            }
        }
        Ok(out)
    }
}

/// Never a workspace root, nor inside one, nor holding one.
const SYSTEM_DIRS: &[&str] = &[
    "/etc", "/usr", "/boot", "/root", "/home", "/proc", "/sys", "/dev",
];

/// The host's own state that is not workspaces.
const HOST_STATE_DIRS: &[&str] = &["agents", "sessions", "launch", "home"];

fn check_root_place(root: &Path, state: &Path, host_state: &Path) -> Result<()> {
    let normal: PathBuf = root.components().collect();
    if !root.is_absolute()
        || root
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        bail!(
            "the workspace root {} is refused: it must be an absolute path with no . or ..",
            root.display()
        );
    }
    let overlaps = |dir: &Path| normal.starts_with(dir) || dir.starts_with(&normal);
    let mut clash = SYSTEM_DIRS
        .iter()
        .map(PathBuf::from)
        .chain([state.to_path_buf()])
        .chain(HOST_STATE_DIRS.iter().map(|d| host_state.join(d)))
        .find(|d| overlaps(d));
    if clash.is_none() && host_state.starts_with(&normal) {
        clash = Some(host_state.to_path_buf());
    }
    if normal == Path::new("/") {
        clash = Some("/".into());
    }
    if let Some(dir) = clash {
        bail!(
            "the workspace root {} is refused: it overlaps {}; move it to its own directory, \
             such as /var/lib/storm-runtime/workspaces or /srv/work",
            root.display(),
            dir.display()
        );
    }
    Ok(())
}

/// A workspace is a directory directly under a root whose name does not
/// start with a dot (the host's `resolve_workspace`). Symlinks are not
/// workspaces here: the conversion never follows one.
pub fn workspaces(roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for root in roots {
        let dir = crate::files::open_dir(root)?;
        let names =
            crate::files::dir_names(&dir).with_context(|| format!("listing {}", root.display()))?;
        for name in names {
            if name.to_bytes().first() == Some(&b'.') {
                continue;
            }
            let st = match rustix::fs::statat(&dir, &name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) => st,
                Err(rustix::io::Errno::NOENT) => continue,
                Err(e) => return Err(e).with_context(|| format!("stat {}", root.display())),
            };
            if rustix::fs::FileType::from_raw_mode(st.st_mode as _)
                == rustix::fs::FileType::Directory
            {
                out.push(root.join(std::ffi::OsStr::from_bytes(name.to_bytes())));
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
    fn the_host_port_list_is_the_operators_and_empty_by_default() {
        assert!(
            HelperConfig::parse("")
                .unwrap()
                .isolation_host_ports
                .is_empty()
        );
        let c = HelperConfig::parse("isolation_host_ports = [11434, 5432]").unwrap();
        assert_eq!(c.isolation_host_ports, [11434, 5432]);
        assert!(HelperConfig::parse("isolation_host_ports = [70000]").is_err());
        assert!(HelperConfig::parse("isolation_host_ports = [\"80\"]").is_err());
    }

    #[test]
    fn the_child_caps_default_to_2_and_20_gib() {
        assert_eq!(
            HelperConfig::parse("").unwrap().caps().unwrap(),
            Caps {
                child: 2 * 1024 * 1024 * 1024,
                children: 20 * 1024 * 1024 * 1024,
            }
        );
        let c = HelperConfig::parse("child_workspace_max = 1000\nchildren_max = 5000").unwrap();
        assert_eq!(
            c.caps().unwrap(),
            Caps {
                child: 1000,
                children: 5000
            }
        );
        assert!(
            HelperConfig::parse("children_max = 0")
                .unwrap()
                .caps()
                .is_err()
        );
        assert!(HelperConfig::parse("child_workspace_max = -1").is_err());
        assert!(HelperConfig::parse("child_workspace_max = \"2G\"").is_err());
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

    struct Roots {
        _t: tempfile::TempDir,
        base: PathBuf,
        state: PathBuf,
        host_state: PathBuf,
    }

    fn roots_fixture() -> Roots {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let (state, host_state) = (base.join("isolation"), base.join("host"));
        std::fs::create_dir_all(host_state.join("workspaces/app")).unwrap();
        std::fs::create_dir_all(host_state.join("agents")).unwrap();
        std::fs::create_dir_all(state.join("children")).unwrap();
        Roots {
            _t: t,
            base,
            state,
            host_state,
        }
    }

    impl Roots {
        fn resolve(&self, roots: &[&Path]) -> Result<Vec<PathBuf>> {
            HelperConfig {
                workspace_roots: roots.iter().map(|r| r.to_path_buf()).collect(),
                ..HelperConfig::default()
            }
            .roots(&self.state, &self.host_state)
        }

        fn refused(&self, root: &Path, why: &str) {
            let err = format!("{:#}", self.resolve(&[root]).unwrap_err());
            assert!(err.contains(&root.display().to_string()), "{err}");
            assert!(err.contains(why), "{root:?}: {err}");
        }
    }

    #[test]
    fn a_real_root_resolves_and_a_missing_one_is_left_out() {
        let r = roots_fixture();
        let ws = r.host_state.join("workspaces");
        let missing = r.base.join("missing/ws");
        let trailing = PathBuf::from(format!("{}/", ws.display()));
        assert_eq!(
            r.resolve(&[&ws, &missing]).unwrap(),
            std::slice::from_ref(&ws)
        );
        assert_eq!(r.resolve(&[&trailing]).unwrap(), std::slice::from_ref(&ws));
        assert_eq!(
            workspaces(std::slice::from_ref(&ws)).unwrap(),
            [ws.join("app")]
        );
    }

    /// G-136-2 H1: the host account can replace a root it owns the parent
    /// of with a link; the root's root must never follow it.
    #[test]
    fn a_symlinked_root_is_refused() {
        let r = roots_fixture();
        let link = r.base.join("ws-link");
        std::os::unix::fs::symlink(r.host_state.join("workspaces"), &link).unwrap();
        r.refused(&link, "is a symbolic link");
        let to_etc = r.base.join("to-etc");
        std::os::unix::fs::symlink("/etc", &to_etc).unwrap();
        r.refused(&to_etc, "is a symbolic link");
        assert!(workspaces(&[link]).is_err(), "nor listed through one");
    }

    #[test]
    fn a_symlink_in_a_middle_component_is_refused() {
        let r = roots_fixture();
        let link = r.base.join("host-link");
        std::os::unix::fs::symlink(&r.host_state, &link).unwrap();
        let root = link.join("workspaces");
        r.refused(&root, "is a symbolic link");
        let err = format!("{:#}", r.resolve(&[&root]).unwrap_err());
        assert!(
            err.contains(&link.display().to_string()),
            "names the link: {err}"
        );
    }

    #[test]
    fn a_root_that_is_not_a_directory_is_refused() {
        let r = roots_fixture();
        let file = r.base.join("file");
        std::fs::write(&file, b"").unwrap();
        r.refused(&file, "is not a directory");
        r.refused(&file.join("below"), "is not a directory");
        r.refused(Path::new("relative/ws"), "absolute");
        r.refused(&r.host_state.join("workspaces/../agents"), "absolute");
    }

    /// G-136-2 L3.
    #[test]
    fn a_root_overlapping_a_system_path_is_refused() {
        let r = roots_fixture();
        for root in [
            "/",
            "/etc",
            "/etc/x",
            "/usr",
            "/usr/local/work",
            "/boot",
            "/root",
            "/home",
            "/home/me/work",
            "/proc",
            "/sys",
            "/dev/shm",
        ] {
            r.refused(Path::new(root), "overlaps");
        }
        // Holding one is overlapping it.
        r.refused(&r.base, "overlaps");
    }

    #[test]
    fn a_root_inside_the_helpers_or_the_hosts_own_state_is_refused() {
        let r = roots_fixture();
        r.refused(&r.state, "overlaps");
        r.refused(&r.state.join("children"), "overlaps");
        r.refused(&r.host_state, "overlaps");
        for dir in ["agents", "agents/managed", "sessions", "launch", "home"] {
            r.refused(&r.host_state.join(dir), "overlaps");
        }
        let layout = crate::session::Layout::installed();
        let config = HelperConfig::parse("workspace_roots = [\"/var/lib/storm-runtime\"]").unwrap();
        assert!(config.roots(&layout.state, &layout.host_state).is_err());
        let config = HelperConfig::parse("workspace_roots = [\"/var/lib\"]").unwrap();
        assert!(config.roots(&layout.state, &layout.host_state).is_err());
    }
}
