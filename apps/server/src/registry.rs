//! Which vaults exist, and where.
//!
//! A vault is a directory under the storage root. The registry maps a stable
//! UUID to that directory and to a display name, so renaming either is an edit
//! here rather than a migration — the same reason notes are tracked by UUID and
//! not by path. Without it, renaming a directory would orphan its index and
//! every client's cached notes.
//!
//! Stored as plain JSON at `state/vaults.json` rather than in a database. It is
//! the one piece of Storm's state a human might need to repair by hand when
//! something has gone wrong, and the vault-is-greppable principle should reach
//! it too.
//!
//! **The registry never deletes or moves files.** Removing a vault forgets it;
//! the directory stays. Pointing the root somewhere else does not relocate
//! anything.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const REGISTRY_FILE: &str = "vaults.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    pub id: String,
    /// What the user calls it.
    pub name: String,
    /// Directory name under the root — not a path, so the whole registry moves
    /// when the root does.
    pub dir: String,
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    pub root: PathBuf,
    #[serde(default)]
    pub vaults: Vec<VaultEntry>,
    /// Whether the MCP endpoint serves requests.
    ///
    /// Here rather than in a file of its own because this is already where the
    /// server's own persisted settings live — `root` is one — and a second
    /// settings file would be a second thing to back up and keep in step.
    ///
    /// `#[serde(default)]` so a registry written before this existed loads with
    /// MCP off, which is the safe direction: an upgrade never switches on a way
    /// to read the vault.
    #[serde(default)]
    pub mcp_enabled: bool,
    /// Whether MCP may *change* the vault, not just read it.
    ///
    /// A second flag rather than a three-state mode so an older registry loads
    /// as read-only rather than failing to parse — and because "off" and
    /// "read-only" are the two safe states, and both should be reachable by a
    /// field simply being absent.
    #[serde(default)]
    pub mcp_writable: bool,
}

/// What a scan of a candidate root would do, without doing it.
///
/// `PUT /v1/config` reports this before committing, so "point the root at an
/// empty directory" cannot silently orphan every vault.
#[derive(Debug, Clone, Serialize, Default)]
pub struct RootPreview {
    /// Registered vaults whose directory exists under the candidate root.
    pub found: Vec<String>,
    /// Registered vaults whose directory does not. These keep their registry
    /// entry and are served as `missing`.
    pub orphaned: Vec<String>,
    /// Unregistered directories that would be adopted as new vaults.
    pub adopted: Vec<String>,
}

impl Registry {
    /// Loads the registry, or returns an empty one rooted at `first_run_root`.
    ///
    /// A missing file is the first-run case, not an error. A corrupt one *is*
    /// an error: silently starting with zero vaults is the failure mode this
    /// whole module is written to avoid.
    ///
    /// **The stored root wins.** `first_run_root` seeds a registry that does
    /// not exist yet and is otherwise ignored, because the root is a setting
    /// the app can change and a setting that does not survive a restart is not
    /// a setting. This used to overwrite the parsed value with the argument,
    /// which meant a root chosen in the app was recorded, ignored on the next
    /// boot, and then *erased* by the next save — with any vault adopted under
    /// it left behind as a permanently `missing` entry.
    pub fn load(state_dir: &Path, first_run_root: &Path) -> Result<Self> {
        let path = state_dir.join(REGISTRY_FILE);
        if !path.exists() {
            return Ok(Self {
                root: first_run_root.to_path_buf(),
                vaults: Vec::new(),
                mcp_enabled: false,
                mcp_writable: false,
            });
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("reading registry {}", path.display()))?;
        let mut registry: Registry = serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing registry {} — fix or remove it; \
                 Storm will not start with an unreadable vault list",
                path.display()
            )
        })?;
        // Only when the file carries no root at all — a hand-edited or
        // truncated registry — does the argument stand in for it.
        if registry.root.as_os_str().is_empty() {
            registry.root = first_run_root.to_path_buf();
        }
        Ok(registry)
    }

    /// Writes the registry atomically: temp file in the same directory, fsync,
    /// rename. Same discipline as note writes — a crash must never leave a
    /// half-written vault list.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        fs::create_dir_all(state_dir)
            .with_context(|| format!("creating state dir {}", state_dir.display()))?;
        let path = state_dir.join(REGISTRY_FILE);
        let tmp = path.with_extension("json.storm-tmp");
        let json = serde_json::to_string_pretty(self)?;
        {
            use std::io::Write;
            let mut f =
                fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
            f.write_all(json.as_bytes())?;
            f.write_all(b"\n")?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&VaultEntry> {
        self.vaults.iter().find(|v| v.id == id)
    }

    pub fn by_dir(&self, dir: &str) -> Option<&VaultEntry> {
        self.vaults.iter().find(|v| v.dir == dir)
    }

    pub fn path_of(&self, entry: &VaultEntry) -> PathBuf {
        self.root.join(&entry.dir)
    }

    /// Whether the vault's directory is actually there.
    pub fn is_missing(&self, entry: &VaultEntry) -> bool {
        !self.path_of(entry).is_dir()
    }

    /// Directories under `root` that could be vaults.
    ///
    /// Directories only, and never `state_dir` or anything dot-prefixed. That
    /// exclusion is load-bearing rather than tidy: the `--vault` compatibility
    /// shim makes the root the *parent* of the old vault directory, which is
    /// typically `state/`'s parent too. Without this, `state` would be adopted
    /// as a vault and its SQLite files indexed as notes.
    ///
    /// Loose files sitting directly under the root — a stray `README.md`, a
    /// `.DS_Store` — are ignored rather than half-registered.
    pub fn candidate_dirs(root: &Path, state_dir: &Path) -> Result<Vec<String>> {
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let state_real = state_dir.canonicalize().ok();
        let mut out = BTreeSet::new();

        for entry in
            fs::read_dir(root).with_context(|| format!("reading root {}", root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if let (Some(real), Ok(candidate)) = (&state_real, entry.path().canonicalize())
                && &candidate == real
            {
                continue;
            }
            out.insert(name);
        }
        Ok(out.into_iter().collect())
    }

    /// What [`Registry::scan_root`] would do against `root`, without doing it.
    pub fn preview(&self, root: &Path, state_dir: &Path) -> Result<RootPreview> {
        let dirs = Self::candidate_dirs(root, state_dir)?;
        let mut preview = RootPreview::default();

        for vault in &self.vaults {
            if dirs.contains(&vault.dir) {
                preview.found.push(vault.name.clone());
            } else {
                preview.orphaned.push(vault.name.clone());
            }
        }
        for dir in dirs {
            if self.by_dir(&dir).is_none() {
                preview.adopted.push(dir);
            }
        }
        Ok(preview)
    }

    /// Adopts unregistered directories under the root.
    ///
    /// Entries whose directory has vanished are **kept and reported as
    /// `missing`**, never dropped — a vault that disappears from the registry
    /// looks identical to one that never existed.
    ///
    /// Not reactive. This runs at startup, on a successful root change, and on
    /// vault create/delete. A directory dropped into the root from outside the
    /// app does not appear until the server restarts; the file watcher covers
    /// note edits inside registered vaults, not vault registration itself.
    pub fn scan_root(&mut self, state_dir: &Path, now: &str) -> Result<Vec<String>> {
        let dirs = Self::candidate_dirs(&self.root, state_dir)?;
        let mut adopted = Vec::new();

        for dir in dirs {
            if self.by_dir(&dir).is_some() {
                continue;
            }
            self.vaults.push(VaultEntry {
                id: uuid::Uuid::new_v4().to_string(),
                name: dir.clone(),
                dir: dir.clone(),
                created: now.to_string(),
            });
            adopted.push(dir);
        }

        self.vaults.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(adopted)
    }

    /// Creates a new vault directory under the root and registers it.
    pub fn create(&mut self, name: &str, now: &str) -> Result<VaultEntry> {
        let dir = sanitize_dir_name(name)?;
        if self.by_dir(&dir).is_some() {
            bail!("a vault directory called “{dir}” already exists");
        }
        let path = self.root.join(&dir);
        if path.exists() {
            bail!("{} already exists on disk", path.display());
        }
        fs::create_dir_all(&path)
            .with_context(|| format!("creating vault directory {}", path.display()))?;

        let entry = VaultEntry {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.trim().to_string(),
            dir,
            created: now.to_string(),
        };
        self.vaults.push(entry.clone());
        self.vaults.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entry)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<VaultEntry> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            bail!("a vault needs a name");
        }
        let entry = self
            .vaults
            .iter_mut()
            .find(|v| v.id == id)
            .with_context(|| format!("no vault {id}"))?;
        // Display name only. The directory keeps its name so nothing on disk
        // moves and no client's cached paths are invalidated.
        entry.name = trimmed.to_string();
        let updated = entry.clone();
        self.vaults.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(updated)
    }

    /// Forgets a vault. **Leaves every file on disk**, by design.
    pub fn remove(&mut self, id: &str) -> Result<VaultEntry> {
        let at = self
            .vaults
            .iter()
            .position(|v| v.id == id)
            .with_context(|| format!("no vault {id}"))?;
        Ok(self.vaults.remove(at))
    }
}

/// Turns a display name into a safe directory name.
///
/// The same rules `Vault::resolve` enforces on note paths: no separators, no
/// `..`, no leading dot. A name that survives this is one the vault scanner
/// will also accept.
pub fn sanitize_dir_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("a vault needs a name");
    }
    if trimmed.starts_with('.') {
        bail!("a vault name cannot start with a dot");
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        bail!("a vault name cannot contain a path separator");
    }
    if trimmed == ".." || trimmed == "." {
        bail!("invalid vault name");
    }
    if trimmed.len() > 100 {
        bail!("that vault name is too long");
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> String {
        "2026-08-07T10:00:00Z".to_string()
    }

    #[test]
    fn the_stored_root_survives_a_different_argument() {
        // The bug this replaces: `load` overwrote the parsed root with its
        // argument, so a root chosen in the app was written to vaults.json,
        // ignored on the next boot, and then erased by the next save — leaving
        // any vault adopted under it registered but permanently `missing`.
        let dir = tempdir::TempDir::new("storm-root").unwrap();
        let state = dir.path().join("state");
        let chosen = dir.path().join("chosen");
        let on_the_command_line = dir.path().join("flag");

        let mut registry = Registry::default();
        registry.root = chosen.clone();
        registry.save(&state).unwrap();

        let loaded = Registry::load(&state, &on_the_command_line).unwrap();
        assert_eq!(loaded.root, chosen, "the stored root is the setting");
    }

    #[test]
    fn the_argument_seeds_a_registry_that_does_not_exist_yet() {
        let dir = tempdir::TempDir::new("storm-root2").unwrap();
        let state = dir.path().join("state");
        let first_run = dir.path().join("vaults");

        let loaded = Registry::load(&state, &first_run).unwrap();
        assert_eq!(loaded.root, first_run);
        assert!(loaded.vaults.is_empty());
    }

    #[test]
    fn a_registry_with_no_root_at_all_falls_back_to_the_argument() {
        // A hand-repaired file, which the module docstring invites.
        let dir = tempdir::TempDir::new("storm-root3").unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join(REGISTRY_FILE), r#"{"root":"","vaults":[]}"#).unwrap();

        let fallback = dir.path().join("vaults");
        assert_eq!(Registry::load(&state, &fallback).unwrap().root, fallback);
    }

    struct Fixture {
        _dir: tempdir::TempDir,
        root: PathBuf,
        state: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempdir::TempDir::new("storm-registry").unwrap();
        let root = dir.path().join("vaults");
        let state = dir.path().join("state");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&state).unwrap();
        Fixture {
            _dir: dir,
            root,
            state,
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let f = fixture();
        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        fs::create_dir_all(f.root.join("personal")).unwrap();
        reg.scan_root(&f.state, &now()).unwrap();
        reg.save(&f.state).unwrap();

        let again = Registry::load(&f.state, &f.root).unwrap();
        assert_eq!(again.vaults.len(), 1);
        assert_eq!(again.vaults[0].dir, "personal");
        assert_eq!(again.vaults[0].id, reg.vaults[0].id);
    }

    #[test]
    fn adopts_unregistered_directories() {
        let f = fixture();
        fs::create_dir_all(f.root.join("work")).unwrap();
        fs::create_dir_all(f.root.join("recipes")).unwrap();

        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        let adopted = reg.scan_root(&f.state, &now()).unwrap();
        assert_eq!(adopted.len(), 2);

        // Idempotent: a second scan adopts nothing and mints no new ids.
        let ids: Vec<_> = reg.vaults.iter().map(|v| v.id.clone()).collect();
        assert!(reg.scan_root(&f.state, &now()).unwrap().is_empty());
        let after: Vec<_> = reg.vaults.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids, after);
    }

    #[test]
    fn a_vanished_directory_is_missing_not_dropped() {
        let f = fixture();
        fs::create_dir_all(f.root.join("personal")).unwrap();
        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        reg.scan_root(&f.state, &now()).unwrap();

        fs::remove_dir_all(f.root.join("personal")).unwrap();
        reg.scan_root(&f.state, &now()).unwrap();

        // Still registered — a vault that quietly disappears from the list is
        // indistinguishable from one that never existed.
        assert_eq!(reg.vaults.len(), 1);
        assert!(reg.is_missing(&reg.vaults[0]));
    }

    #[test]
    fn scan_skips_state_dir_and_dotfiles() {
        // The exact layout the `--vault` compatibility shim produces: the root
        // is the parent of both the vault directory and `state/`.
        let dir = tempdir::TempDir::new("storm-shim").unwrap();
        let root = dir.path().to_path_buf();
        let state = root.join("state");
        fs::create_dir_all(root.join("vault")).unwrap();
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("README.md"), "loose file").unwrap();

        let mut reg = Registry::load(&state, &root).unwrap();
        reg.scan_root(&state, &now()).unwrap();

        assert_eq!(reg.vaults.len(), 1, "only `vault/` is a vault");
        assert_eq!(reg.vaults[0].dir, "vault");
    }

    #[test]
    fn preview_reports_found_orphaned_and_adopted() {
        let f = fixture();
        fs::create_dir_all(f.root.join("personal")).unwrap();
        fs::create_dir_all(f.root.join("work")).unwrap();
        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        reg.scan_root(&f.state, &now()).unwrap();

        // A different root holding one of the two, plus a stranger.
        let other = f.root.parent().unwrap().join("elsewhere");
        fs::create_dir_all(other.join("work")).unwrap();
        fs::create_dir_all(other.join("archive")).unwrap();

        let p = reg.preview(&other, &f.state).unwrap();
        assert_eq!(p.found, vec!["work"]);
        assert_eq!(p.orphaned, vec!["personal"]);
        assert_eq!(p.adopted, vec!["archive"]);
    }

    #[test]
    fn preview_of_an_empty_root_orphans_everything() {
        let f = fixture();
        fs::create_dir_all(f.root.join("personal")).unwrap();
        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        reg.scan_root(&f.state, &now()).unwrap();

        let empty = f.root.parent().unwrap().join("empty");
        fs::create_dir_all(&empty).unwrap();

        let p = reg.preview(&empty, &f.state).unwrap();
        assert!(p.found.is_empty());
        assert_eq!(p.orphaned, vec!["personal"]);
    }

    #[test]
    fn create_makes_the_directory_and_registers_it() {
        let f = fixture();
        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        let entry = reg.create("My Notes", &now()).unwrap();

        assert!(f.root.join(&entry.dir).is_dir());
        assert_eq!(entry.name, "My Notes");
        assert!(reg.create("My Notes", &now()).is_err(), "no duplicates");
    }

    #[test]
    fn remove_forgets_but_never_deletes() {
        let f = fixture();
        fs::create_dir_all(f.root.join("personal")).unwrap();
        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        reg.scan_root(&f.state, &now()).unwrap();
        let id = reg.vaults[0].id.clone();

        reg.remove(&id).unwrap();
        assert!(reg.vaults.is_empty());
        assert!(
            f.root.join("personal").is_dir(),
            "removing a vault must leave its files alone"
        );
    }

    #[test]
    fn rename_changes_the_display_name_only() {
        let f = fixture();
        fs::create_dir_all(f.root.join("personal")).unwrap();
        let mut reg = Registry::load(&f.state, &f.root).unwrap();
        reg.scan_root(&f.state, &now()).unwrap();
        let id = reg.vaults[0].id.clone();

        let updated = reg.rename(&id, "Personal Notes").unwrap();
        assert_eq!(updated.name, "Personal Notes");
        assert_eq!(updated.dir, "personal", "nothing on disk moves");
        assert_eq!(updated.id, id);
    }

    #[test]
    fn rejects_unsafe_vault_names() {
        for bad in ["", "   ", ".hidden", "a/b", "a\\b", "..", "."] {
            assert!(sanitize_dir_name(bad).is_err(), "should reject {bad:?}");
        }
        assert_eq!(sanitize_dir_name("  Work  ").unwrap(), "Work");
    }

    #[test]
    fn a_corrupt_registry_is_an_error_not_an_empty_list() {
        let f = fixture();
        fs::write(f.state.join(REGISTRY_FILE), "{ not json").unwrap();
        assert!(Registry::load(&f.state, &f.root).is_err());
    }
}
