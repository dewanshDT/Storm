//! `storm-server backup` (decision 99, W-S3): `deploy/storm-backup.sh`'s steps,
//! natively, for a machine with no bash, rsync or find — Windows' nightly
//! task. Linux and macOS keep the script.
//!
//! Into `<dest>/<YYYY-MM-DD>/`: a mirror of the vaults (from the stored
//! storage root, which wins over the flag), the index and auth snapshots
//! (`backup-db`), a check that the snapshots open, then pruning of dated
//! folders past `keep_days`. Unlike the script it copies every file in full:
//! NTFS hard links are possible but the script's `--link-dest` is not ported.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use time::Date;
use time::macros::format_description;

const STAMP: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]");

pub fn run(vault_root: &Path, state: &Path, dest_root: &Path, keep_days: u32) -> Result<()> {
    let today = time::OffsetDateTime::now_utc().date();
    let stamp = today.format(STAMP)?;
    let dest = dest_root.join(&stamp);
    tracing::info!(dest = %dest.display(), "backup: {stamp}");

    // The stored root wins, as in the script: a root chosen in the app must
    // be what is backed up.
    let registry = crate::registry::Registry::load(state, Path::new(""))?;
    let root = if registry.root.as_os_str().is_empty() {
        vault_root.to_path_buf()
    } else {
        registry.root
    };
    // As in the script (decision 128j): an empty root while the registry
    // lists vaults is an unmounted share, and backing it up would age out
    // every good night. Refused before anything is written.
    if !registry.vaults.is_empty() && !holds_a_file(&root) {
        anyhow::bail!(
            "{} holds no files, but the registry lists vaults — is it mounted?",
            root.display()
        );
    }
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    let vaults = dest.join("vaults");
    if vaults.exists() {
        std::fs::remove_dir_all(&vaults)
            .with_context(|| format!("clearing {}", vaults.display()))?;
    }
    let files = copy_tree(&root, &vaults)?;
    tracing::info!(root = %root.display(), files, "backup: vaults");

    let index = dest.join("index");
    crate::backup_all(state, &index)?;
    let verify = dest_root.join(format!(".verify-{stamp}"));
    let opened = crate::backup_all(&index, &verify);
    let _ = std::fs::remove_dir_all(&verify);
    opened.context("the snapshots did not open — keeping them, but investigate")?;
    tracing::info!("backup: verified, the snapshots open");

    for old in expired(dest_root, today, keep_days)? {
        tracing::info!(pruned = %old.display(), "backup: pruned");
        std::fs::remove_dir_all(&old).with_context(|| format!("removing {}", old.display()))?;
    }
    tracing::info!("backup: done");
    Ok(())
}

/// The dated folders in `dest_root` older than `keep_days` before `today`.
/// Anything not named as a date is not the backup's, and is left alone.
pub fn expired(dest_root: &Path, today: Date, keep_days: u32) -> Result<Vec<PathBuf>> {
    let cutoff = today - time::Duration::days(i64::from(keep_days));
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dest_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if let Some(date) = name.to_str().and_then(|n| Date::parse(n, STAMP).ok())
            && date < cutoff
        {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Whether any regular file is under `dir`. Unreadable counts as empty.
fn holds_a_file(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| match entry.file_type() {
        Ok(kind) if kind.is_file() => true,
        Ok(kind) if kind.is_dir() => holds_a_file(&entry.path()),
        _ => false,
    })
}

/// Copies the tree at `from` to `to` (which must not exist), files and
/// directories only: a symlink is skipped rather than followed out of the
/// tree. Returns how many files it copied.
pub fn copy_tree(from: &Path, to: &Path) -> Result<usize> {
    std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    let mut files = 0;
    for entry in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            files += copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", entry.path().display()))?;
            files += 1;
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dated_folders_past_the_window_are_pruned() {
        let root = tempdir::TempDir::new("backups").unwrap();
        for name in [
            "2026-01-01",
            "2026-09-30",
            "2026-10-11",
            "keep-me",
            "2026-13-40",
        ] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        std::fs::write(root.path().join("2025-01-01"), "a file, not a backup").unwrap();
        let today = Date::parse("2026-10-12", STAMP).unwrap();
        let old = expired(root.path(), today, 30).unwrap();
        assert_eq!(old, vec![root.path().join("2026-01-01")]);
    }

    #[test]
    fn the_tree_is_copied_whole() {
        let from = tempdir::TempDir::new("from").unwrap();
        std::fs::create_dir_all(from.path().join("personal/Projects")).unwrap();
        std::fs::write(from.path().join("personal/a.md"), "a").unwrap();
        std::fs::write(from.path().join("personal/Projects/b.md"), "b").unwrap();
        std::fs::write(from.path().join("personal/.storm.yml"), "c").unwrap();
        let to = tempdir::TempDir::new("to").unwrap();
        let dest = to.path().join("vaults");
        assert_eq!(copy_tree(from.path(), &dest).unwrap(), 3);
        assert_eq!(
            std::fs::read_to_string(dest.join("personal/Projects/b.md")).unwrap(),
            "b"
        );
        assert!(dest.join("personal/.storm.yml").exists());
    }
}
