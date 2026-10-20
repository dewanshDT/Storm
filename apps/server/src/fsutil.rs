//! One atomic write for the server's files (decision 128i).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Writes `bytes` to `path` so a reader sees the old file or the new one, never
/// a torn one, and the new one survives a power cut: a temp file beside it,
/// fsynced, renamed over `path`, then the directory fsynced — a rename is only
/// durable once its directory entry is. The parent must exist.
///
/// The temp name ends `.storm-tmp`, which the watcher ignores, so a note write
/// is never seen half-made.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = temp_path(path);
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("renaming {} to {}", tmp.display(), path.display()));
    }
    sync_parent(path)
}

fn temp_path(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}.storm-tmp",
        path.extension()
            .map(|e| e.to_string_lossy())
            .unwrap_or_default()
    ))
}

fn sync_parent(path: &Path) -> Result<()> {
    // Windows cannot open a directory as a file, and NTFS journals the rename.
    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)
            .and_then(|d| d.sync_all())
            .with_context(|| format!("syncing {}", dir.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_replaces_the_file_and_leaves_no_temp_behind() {
        let dir = tempdir::TempDir::new("storm-fsutil").unwrap();
        let path = dir.path().join("vaults.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["vaults.json".to_string()]);
    }

    #[test]
    fn the_temp_name_is_one_the_watcher_ignores() {
        assert_eq!(
            temp_path(Path::new("Notes/A.md")),
            Path::new("Notes/A.md.storm-tmp")
        );
        assert_eq!(
            temp_path(Path::new("README")),
            Path::new("README..storm-tmp")
        );
    }

    #[test]
    fn a_missing_parent_is_an_error_and_writes_nothing() {
        let dir = tempdir::TempDir::new("storm-fsutil").unwrap();
        let path = dir.path().join("gone/vaults.json");
        assert!(write_atomic(&path, b"x").is_err());
        assert!(!dir.path().join("gone").exists());
    }
}
