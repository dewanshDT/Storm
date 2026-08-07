//! The canonical vault on disk: a plain directory of `.md` files.
//!
//! Everything here is written so the vault stays exactly what an Obsidian
//! vault is — a greppable, rsync-able tree of markdown with no hidden
//! indirection. Storm's own state lives in a sibling `state/` directory, never
//! inside the vault.
//!
//! Two invariants this module enforces:
//!
//!  * **Writes are atomic.** Write to a temp file in the same directory,
//!    fsync, then rename. A crash mid-write can leave a stray temp file but
//!    never a truncated note.
//!  * **Paths cannot escape the vault.** Every path arriving from the API is
//!    resolved against the root and rejected if it climbs out. This holds even
//!    though v1 is LAN-only.

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

pub struct Vault {
    root: PathBuf,
}

/// One markdown file found on disk.
pub struct ScannedFile {
    /// Vault-relative, forward-slashed, e.g. `Daily/2026-08-05.md`.
    pub rel_path: String,
    pub content: String,
}

impl Vault {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("creating vault root {}", root.display()))?;
        let root = root
            .canonicalize()
            .with_context(|| format!("resolving vault root {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a vault-relative path to an absolute one, refusing escapes.
    ///
    /// Rejects absolute paths, `..` components, Windows prefixes, and anything
    /// that would land outside the root. Note this is a *lexical* check made
    /// before the file exists, so it also covers paths for files we're about
    /// to create.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        if rel.is_empty() {
            bail!("empty path");
        }
        let candidate = Path::new(rel);
        if candidate.is_absolute() {
            bail!("absolute paths are not allowed: {rel}");
        }

        let mut out = self.root.clone();
        for component in candidate.components() {
            match component {
                Component::Normal(part) => {
                    let s = part.to_string_lossy();
                    if s.starts_with('.') {
                        bail!("hidden path segments are not allowed: {rel}");
                    }
                    out.push(part);
                }
                Component::CurDir => {}
                Component::ParentDir => bail!("path escapes the vault: {rel}"),
                Component::RootDir | Component::Prefix(_) => {
                    bail!("absolute paths are not allowed: {rel}")
                }
            }
        }

        // Defence in depth: if any parent is a symlink out of the vault, the
        // canonical form of the deepest existing ancestor must still be inside.
        let mut probe = out.as_path();
        loop {
            if probe.exists() {
                let real = probe
                    .canonicalize()
                    .with_context(|| format!("resolving {}", probe.display()))?;
                if !real.starts_with(&self.root) {
                    bail!("path escapes the vault via a symlink: {rel}");
                }
                break;
            }
            match probe.parent() {
                Some(p) => probe = p,
                None => break,
            }
        }

        Ok(out)
    }

    /// Reads a file as raw bytes — attachments are opaque blobs.
    pub fn read_bytes(&self, rel: &str) -> Result<Vec<u8>> {
        let path = self.resolve(rel)?;
        fs::read(&path).with_context(|| format!("reading {}", path.display()))
    }

    /// Writes raw bytes atomically, same as [`Vault::write`].
    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = path.with_extension(format!(
            "{}.storm-tmp",
            path.extension()
                .map(|e| e.to_string_lossy())
                .unwrap_or_default()
        ));
        {
            use std::io::Write;
            let mut f = fs::File::create(&tmp)
                .with_context(|| format!("creating temp file {}", tmp.display()))?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }

    /// Every non-markdown file in the vault — the attachments.
    ///
    /// Same walk as [`Vault::scan`] but the inverse extension filter, so a
    /// vault stays one tree of files with no separate blob store.
    pub fn scan_attachments(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name()
                    .to_str()
                    .map(|n| n.starts_with('.') && e.depth() > 0)
                    .unwrap_or(false)
            })
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|e| e.to_str()) == Some("md") {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&self.root)?
                .to_string_lossy()
                .replace('\\', "/");
            if rel.contains("storm-tmp") {
                continue;
            }
            out.push(rel);
        }
        out.sort();
        Ok(out)
    }

    pub fn read(&self, rel: &str) -> Result<String> {
        let path = self.resolve(rel)?;
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.resolve(rel).map(|p| p.is_file()).unwrap_or(false)
    }

    /// Writes atomically: temp file in the same directory, fsync, rename.
    pub fn write(&self, rel: &str, content: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }

        // Same directory as the target, so the rename stays within one
        // filesystem and is therefore atomic.
        let tmp = path.with_extension(format!(
            "{}.storm-tmp",
            path.extension()
                .map(|e| e.to_string_lossy())
                .unwrap_or_default()
        ));

        {
            use std::io::Write;
            let mut f = fs::File::create(&tmp)
                .with_context(|| format!("creating temp file {}", tmp.display()))?;
            f.write_all(content.as_bytes())?;
            f.sync_all()?;
        }

        fs::rename(&tmp, &path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }

    /// Deletes a note, pruning any directories it leaves empty.
    ///
    /// `keep` holds vault-relative paths of folders the user created on
    /// purpose; see [`Vault::prune_empty_parents`].
    pub fn delete(&self, rel: &str, keep: &HashSet<String>) -> Result<()> {
        let path = self.resolve(rel)?;
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
            self.prune_empty_parents(&path, keep);
        }
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str, keep: &HashSet<String>) -> Result<()> {
        let src = self.resolve(from)?;
        let dst = self.resolve(to)?;
        if dst.exists() {
            bail!("destination already exists: {to}");
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&src, &dst)
            .with_context(|| format!("moving {} to {}", src.display(), dst.display()))?;
        self.prune_empty_parents(&src, keep);
        Ok(())
    }

    /// Creates a directory. Used by the explicit "new folder" operation; note
    /// writes still create their own parents implicitly.
    pub fn create_dir(&self, rel: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        if path.is_file() {
            bail!("a file already exists at {rel}");
        }
        fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok(())
    }

    /// Removes an empty directory. Refuses if anything is still inside.
    pub fn remove_dir(&self, rel: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        if !path.is_dir() {
            bail!("no such folder: {rel}");
        }
        let empty = fs::read_dir(&path)?.next().is_none();
        if !empty {
            bail!("folder is not empty: {rel}");
        }
        fs::remove_dir(&path).with_context(|| format!("removing {}", path.display()))?;
        Ok(())
    }

    /// Renames a directory and everything under it.
    pub fn rename_dir(&self, from: &str, to: &str) -> Result<()> {
        let src = self.resolve(from)?;
        let dst = self.resolve(to)?;
        if !src.is_dir() {
            bail!("no such folder: {from}");
        }
        if dst.exists() {
            bail!("destination already exists: {to}");
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&src, &dst)
            .with_context(|| format!("moving {} to {}", src.display(), dst.display()))?;
        Ok(())
    }

    /// Vault-relative, forward-slashed path for an absolute path inside the
    /// vault. `None` for the root itself or anything outside.
    fn relative(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() { None } else { Some(s) }
    }

    /// Removes directories left empty by a delete or move, so the tree doesn't
    /// accumulate empty folders. Stops at the vault root.
    ///
    /// **Stops at any folder in `keep`.** Those are folders the user created
    /// explicitly, recorded in the `folders` table. Without the exemption a
    /// folder made on purpose would vanish the moment its last note was moved
    /// or deleted, which is the entire reason that table exists.
    fn prune_empty_parents(&self, from: &Path, keep: &HashSet<String>) {
        let mut dir = from.parent();
        while let Some(d) = dir {
            if d == self.root || !d.starts_with(&self.root) {
                break;
            }
            if self.relative(d).is_some_and(|rel| keep.contains(&rel)) {
                break;
            }
            let empty = fs::read_dir(d)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false);
            if !empty || fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }

    /// Walks the vault for markdown files.
    ///
    /// Skips dotted directories so `.obsidian/`, `.git/` and `.trash/` never
    /// enter the index — Obsidian vaults routinely contain all three.
    pub fn scan(&self) -> Result<Vec<ScannedFile>> {
        let mut out = Vec::new();

        for entry in WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name()
                    .to_str()
                    .map(|n| n.starts_with('.') && e.depth() > 0)
                    .unwrap_or(false)
            })
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let rel = entry
                .path()
                .strip_prefix(&self.root)?
                .to_string_lossy()
                .replace('\\', "/");

            match fs::read_to_string(entry.path()) {
                Ok(content) => out.push(ScannedFile {
                    rel_path: rel,
                    content,
                }),
                // A note we can't decode as UTF-8 is skipped rather than
                // aborting the whole scan.
                Err(e) => tracing::warn!(path = %rel, error = %e, "skipping unreadable file"),
            }
        }

        out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(out)
    }
}

/// Content hash used to detect external edits. Blake3 is fast enough to run on
/// every file of a large vault at startup.
pub fn hash(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> (tempdir::TempDir, Vault) {
        let dir = tempdir::TempDir::new("storm-vault").unwrap();
        let vault = Vault::new(dir.path()).unwrap();
        (dir, vault)
    }

    /// No folder is exempt from pruning — the pre-`folders` behaviour.
    fn nothing() -> HashSet<String> {
        HashSet::new()
    }

    fn keeping(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn writes_and_reads() {
        let (_d, v) = temp_vault();
        v.write("Notes/A.md", "# A\n").unwrap();
        assert_eq!(v.read("Notes/A.md").unwrap(), "# A\n");
        assert!(v.exists("Notes/A.md"));
    }

    #[test]
    fn write_leaves_no_temp_files_behind() {
        let (_d, v) = temp_vault();
        v.write("A.md", "content\n").unwrap();
        let files = v.scan().unwrap();
        assert_eq!(files.len(), 1);
        let stray: Vec<_> = fs::read_dir(v.root())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("storm-tmp"))
            .collect();
        assert!(stray.is_empty(), "temp file left behind");
    }

    #[test]
    fn overwrites_atomically() {
        let (_d, v) = temp_vault();
        v.write("A.md", "first\n").unwrap();
        v.write("A.md", "second\n").unwrap();
        assert_eq!(v.read("A.md").unwrap(), "second\n");
    }

    #[test]
    fn rejects_path_traversal() {
        let (_d, v) = temp_vault();
        for bad in [
            "../escape.md",
            "Notes/../../escape.md",
            "/etc/passwd",
            "Notes/./../../x.md",
        ] {
            assert!(v.resolve(bad).is_err(), "should have rejected {bad}");
        }
    }

    #[test]
    fn rejects_hidden_segments() {
        let (_d, v) = temp_vault();
        assert!(v.resolve(".obsidian/config").is_err());
        assert!(v.resolve("Notes/.hidden/x.md").is_err());
    }

    #[test]
    fn accepts_ordinary_nested_paths() {
        let (_d, v) = temp_vault();
        assert!(v.resolve("Daily/2026/08/05.md").is_ok());
        assert!(v.resolve("A note with spaces.md").is_ok());
        assert!(v.resolve("日本語/ノート.md").is_ok());
    }

    #[test]
    fn scan_skips_dotted_directories() {
        let (_d, v) = temp_vault();
        v.write("Real.md", "real\n").unwrap();
        fs::create_dir_all(v.root().join(".obsidian")).unwrap();
        fs::write(v.root().join(".obsidian/notes.md"), "config").unwrap();
        fs::create_dir_all(v.root().join(".trash")).unwrap();
        fs::write(v.root().join(".trash/deleted.md"), "old").unwrap();

        let found = v.scan().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rel_path, "Real.md");
    }

    #[test]
    fn scan_ignores_non_markdown() {
        let (_d, v) = temp_vault();
        v.write("A.md", "a\n").unwrap();
        fs::write(v.root().join("image.png"), [0u8, 1, 2]).unwrap();
        fs::write(v.root().join("notes.txt"), "text").unwrap();

        let found = v.scan().unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn scan_returns_forward_slashed_relative_paths() {
        let (_d, v) = temp_vault();
        v.write("Folder/Sub/Note.md", "x\n").unwrap();
        let found = v.scan().unwrap();
        assert_eq!(found[0].rel_path, "Folder/Sub/Note.md");
    }

    #[test]
    fn rename_moves_and_prunes() {
        let (_d, v) = temp_vault();
        v.write("Old/A.md", "body\n").unwrap();
        v.rename("Old/A.md", "New/B.md", &nothing()).unwrap();

        assert!(!v.exists("Old/A.md"));
        assert_eq!(v.read("New/B.md").unwrap(), "body\n");
        // The emptied `Old/` directory should be gone.
        assert!(!v.root().join("Old").exists());
    }

    #[test]
    fn rename_refuses_to_clobber() {
        let (_d, v) = temp_vault();
        v.write("A.md", "a\n").unwrap();
        v.write("B.md", "b\n").unwrap();
        assert!(v.rename("A.md", "B.md", &nothing()).is_err());
        assert_eq!(v.read("B.md").unwrap(), "b\n");
    }

    #[test]
    fn delete_prunes_empty_parents() {
        let (_d, v) = temp_vault();
        v.write("Deep/Nested/A.md", "x\n").unwrap();
        v.delete("Deep/Nested/A.md", &nothing()).unwrap();
        assert!(!v.root().join("Deep").exists());
        assert!(v.root().exists());
    }

    #[test]
    fn delete_keeps_non_empty_parents() {
        let (_d, v) = temp_vault();
        v.write("Folder/A.md", "a\n").unwrap();
        v.write("Folder/B.md", "b\n").unwrap();
        v.delete("Folder/A.md", &nothing()).unwrap();
        assert!(v.exists("Folder/B.md"));
    }

    #[test]
    fn a_recorded_folder_survives_the_delete_that_empties_it() {
        // The whole reason the `folders` table exists. Without the exemption,
        // creating a folder and then removing its only note would silently
        // delete the folder too.
        let (_d, v) = temp_vault();
        v.write("Keep/A.md", "x\n").unwrap();
        v.delete("Keep/A.md", &keeping(&["Keep"])).unwrap();
        assert!(v.root().join("Keep").is_dir());
    }

    #[test]
    fn pruning_stops_at_the_recorded_folder_not_above_it() {
        let (_d, v) = temp_vault();
        v.write("Keep/Inner/Deeper/A.md", "x\n").unwrap();
        v.delete("Keep/Inner/Deeper/A.md", &keeping(&["Keep"]))
            .unwrap();

        // Everything below the recorded folder goes; the folder itself stays.
        assert!(v.root().join("Keep").is_dir());
        assert!(!v.root().join("Keep/Inner").exists());
    }

    #[test]
    fn an_unrecorded_folder_is_still_pruned() {
        let (_d, v) = temp_vault();
        v.write("Derived/A.md", "x\n").unwrap();
        v.delete("Derived/A.md", &keeping(&["SomethingElse"]))
            .unwrap();
        assert!(!v.root().join("Derived").exists());
    }

    #[test]
    fn remove_dir_refuses_a_non_empty_folder() {
        let (_d, v) = temp_vault();
        v.write("Folder/A.md", "a\n").unwrap();
        assert!(v.remove_dir("Folder").is_err());
        assert!(v.exists("Folder/A.md"));

        v.delete("Folder/A.md", &keeping(&["Folder"])).unwrap();
        v.remove_dir("Folder").unwrap();
        assert!(!v.root().join("Folder").exists());
    }

    #[test]
    fn rename_dir_moves_the_whole_subtree() {
        let (_d, v) = temp_vault();
        v.write("Old/A.md", "a\n").unwrap();
        v.write("Old/Sub/B.md", "b\n").unwrap();
        v.rename_dir("Old", "New").unwrap();

        assert_eq!(v.read("New/A.md").unwrap(), "a\n");
        assert_eq!(v.read("New/Sub/B.md").unwrap(), "b\n");
        assert!(!v.root().join("Old").exists());
    }

    #[test]
    fn rename_dir_refuses_to_clobber() {
        let (_d, v) = temp_vault();
        v.write("A/x.md", "x\n").unwrap();
        v.write("B/y.md", "y\n").unwrap();
        assert!(v.rename_dir("A", "B").is_err());
        assert!(v.exists("B/y.md"));
    }

    #[test]
    fn create_dir_is_idempotent_and_refuses_over_a_file() {
        let (_d, v) = temp_vault();
        v.create_dir("New/Nested").unwrap();
        v.create_dir("New/Nested").unwrap();
        assert!(v.root().join("New/Nested").is_dir());

        v.write("Taken.md", "x\n").unwrap();
        assert!(v.create_dir("Taken.md").is_err());
    }

    #[test]
    fn hashing_is_stable_and_distinguishing() {
        assert_eq!(hash("same"), hash("same"));
        assert_ne!(hash("a"), hash("b"));
    }
}
