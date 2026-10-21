//! One atomic write for the host's state files (decision 128i).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Who may read the file once it exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The host's account only, created that way (A2): through
    /// `platform::open_private`, never chmod-ed afterwards.
    Private,
    /// Whatever the process's umask or the directory grants.
    Default,
}

/// Writes `bytes` to `path` so a reader sees the old file or the new one, never
/// a torn one, and the new one survives a power cut: a temp file beside it,
/// fsynced, renamed over `path`, then the directory fsynced — a rename is only
/// durable once its directory entry is. The parent must exist.
///
/// The temp name carries the process id, so two processes writing one file
/// (the race cache) never share a temp.
pub fn write_atomic(path: &Path, bytes: &[u8], access: Access) -> io::Result<()> {
    let tmp = temp_path(path);
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        let mut file = match access {
            Access::Private => crate::platform::open_private(&tmp, &mut options)?,
            Access::Default => options.open(&tmp)?,
        };
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    sync_parent(path)
}

fn temp_path(path: &Path) -> PathBuf {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_extension(format!("{ext}.tmp{}", std::process::id()))
}

fn sync_parent(path: &Path) -> io::Result<()> {
    // Windows cannot open a directory as a file, and NTFS journals the rename.
    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn it_replaces_the_file_and_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.json");
        write_atomic(&path, b"one", Access::Default).unwrap();
        write_atomic(&path, b"two", Access::Default).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(names(dir.path()), vec!["host.json".to_string()]);
    }

    #[test]
    fn a_private_file_is_private_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.json");
        write_atomic(&path, b"secret-free but private", Access::Private).unwrap();
        assert!(crate::platform::is_private(&path).unwrap());
    }

    #[test]
    fn a_missing_parent_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_atomic(&dir.path().join("gone/x.json"), b"x", Access::Default).is_err());
        assert!(names(dir.path()).is_empty());
    }
}
