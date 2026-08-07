//! Watches the storage root for edits made outside Storm.
//!
//! The PRD wants vaults to stay greppable and possibly exported read-only over
//! the NAS share, so the server cannot assume it is the only writer. Someone
//! editing a note with `nvim` on the server, or a script appending to a daily
//! note, must show up on every client.
//!
//! The server's *own* writes also land here. They are filtered by content hash
//! rather than by bookkeeping: [`crate::index::Indexer::refresh_path`] returns
//! `None` when a file's hash already matches the index, which is inherently
//! correct even if events arrive late, out of order, or coalesced.
//!
//! **One watcher covers every vault.** Watching the root and attributing each
//! event to a vault by directory prefix means adding or removing a vault needs
//! no watcher work at all — the alternative was one watcher per vault plus a
//! shutdown path for each. It does *not* register new vaults: a directory
//! dropped into the root is adopted at the next restart, not on sight.

use std::path::{Path, PathBuf};

use std::time::Duration;

use anyhow::Result;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::api::Shared;

/// Events are coalesced over this window. Editors routinely produce several
/// events per save (truncate, write, chmod, rename-into-place).
const DEBOUNCE: Duration = Duration::from_millis(500);

/// A path attributed to a vault: which vault directory it fell under, and the
/// note path relative to that vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributed {
    pub dir: String,
    pub rel: String,
}

/// Starts watching `root`, reindexing and broadcasting as files change.
///
/// Respawns itself when the storage root moves, so a root change through
/// `PUT /v1/config` does not leave the watcher looking at the old location.
pub fn spawn(root: PathBuf, state: Shared) -> Result<()> {
    let mut root_changed = state.root_changed.subscribe();
    let mut current = root;

    tokio::spawn(async move {
        loop {
            let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
            if let Err(e) = watch_once(current.clone(), state.clone(), stop_rx) {
                tracing::error!(root = %current.display(), error = %e, "could not watch the storage root");
            }

            match root_changed.recv().await {
                Ok(next) => {
                    tracing::info!(from = %current.display(), to = %next.display(), "storage root moved; restarting the watcher");
                    let _ = stop_tx.send(());
                    current = next;
                }
                Err(_) => break,
            }
        }
    });

    Ok(())
}

/// One watch session over `root`, ending when `stop` fires.
fn watch_once(
    root: PathBuf,
    state: Shared,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    tokio::spawn(async move {
        // Keep the watcher alive for the lifetime of the task.
        let _watcher = watcher;
        let mut pending: Vec<Attributed> = Vec::new();

        loop {
            if stop.try_recv().is_ok() {
                break;
            }

            // Collect events until the stream goes quiet for DEBOUNCE.
            let event = match tokio::time::timeout(DEBOUNCE, rx.recv()).await {
                Ok(Some(event)) => Some(event),
                Ok(None) => break,
                Err(_) => None, // debounce window elapsed
            };

            if let Some(event) = event {
                if !matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                ) {
                    continue;
                }
                for path in event.paths {
                    if let Some(hit) = attribute(&root, &path).filter(|h| !pending.contains(h)) {
                        pending.push(hit);
                    }
                }
                continue;
            }

            if pending.is_empty() {
                continue;
            }
            for hit in std::mem::take(&mut pending) {
                apply(&state, &hit).await;
            }
        }
    });

    Ok(())
}

/// Reindexes one path and pushes the resulting change, if any.
async fn apply(state: &Shared, hit: &Attributed) {
    let handle = {
        let vaults = state.vaults.read().await;
        // An event under a directory that is not a registered vault is dropped
        // rather than treated as an error — the root can hold anything.
        match vaults.registry.by_dir(&hit.dir) {
            Some(entry) => vaults.get(&entry.id),
            None => None,
        }
    };
    let Some(handle) = handle else {
        return;
    };

    let mut ix = handle.indexer.lock().await;
    match ix.refresh_path(&hit.rel) {
        // `None` means the content hash was unchanged — almost always our own
        // write coming back to us.
        Ok(None) => {}
        Ok(Some(seq)) => {
            tracing::info!(vault = %hit.dir, path = %hit.rel, seq, "external change indexed");
            if let Ok(Some(change)) = ix
                .db
                .changes_since(seq - 1, 1)
                .map(|c| c.into_iter().next())
            {
                let _ = state.events.send(change);
            }
        }
        Err(e) => {
            tracing::warn!(vault = %hit.dir, path = %hit.rel, error = %e, "failed to index external change")
        }
    }
}

/// Splits an absolute path under the storage root into its vault directory and
/// the note path inside it.
///
/// Filters out non-markdown files, dotted directories, the temp files our own
/// atomic writes create, and anything sitting directly under the root — a
/// loose `README.md` beside the vault directories belongs to no vault.
fn attribute(root: &Path, path: &Path) -> Option<Attributed> {
    let rel = path.strip_prefix(root).ok()?;
    let rel_str = rel.to_string_lossy().replace('\\', "/");

    if rel_str.is_empty() || rel_str.contains("storm-tmp") {
        return None;
    }
    if rel_str.split('/').any(|seg| seg.starts_with('.')) {
        return None;
    }
    if !rel_str.ends_with(".md") {
        return None;
    }

    let (dir, inner) = rel_str.split_once('/')?;
    if inner.is_empty() {
        return None;
    }
    Some(Attributed {
        dir: dir.to_string(),
        rel: inner.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/vaults")
    }

    fn hit(dir: &str, rel: &str) -> Option<Attributed> {
        Some(Attributed {
            dir: dir.to_string(),
            rel: rel.to_string(),
        })
    }

    #[test]
    fn attributes_markdown_to_its_vault() {
        assert_eq!(
            attribute(&root(), Path::new("/vaults/personal/Daily/A.md")),
            hit("personal", "Daily/A.md")
        );
        assert_eq!(
            attribute(&root(), Path::new("/vaults/work/Note.md")),
            hit("work", "Note.md")
        );
    }

    #[test]
    fn two_vaults_can_hold_the_same_note_path() {
        // The collision that one shared index could not have represented.
        let a = attribute(&root(), Path::new("/vaults/personal/Daily/2026-08-07.md")).unwrap();
        let b = attribute(&root(), Path::new("/vaults/work/Daily/2026-08-07.md")).unwrap();
        assert_eq!(a.rel, b.rel);
        assert_ne!(a.dir, b.dir);
    }

    #[test]
    fn ignores_files_directly_under_the_root() {
        // A loose README beside the vault directories belongs to no vault.
        assert_eq!(attribute(&root(), Path::new("/vaults/README.md")), None);
    }

    #[test]
    fn ignores_our_own_temp_files() {
        // Atomic writes create these; reacting to them would be a feedback loop.
        assert_eq!(
            attribute(&root(), Path::new("/vaults/personal/A.md.storm-tmp")),
            None
        );
    }

    #[test]
    fn ignores_dotted_directories() {
        assert_eq!(
            attribute(
                &root(),
                Path::new("/vaults/personal/.obsidian/workspace.md")
            ),
            None
        );
        assert_eq!(attribute(&root(), Path::new("/vaults/.hidden/A.md")), None);
    }

    #[test]
    fn ignores_non_markdown() {
        assert_eq!(
            attribute(&root(), Path::new("/vaults/personal/attachments/img.png")),
            None
        );
    }

    #[test]
    fn ignores_paths_outside_the_root() {
        assert_eq!(attribute(&root(), Path::new("/etc/passwd.md")), None);
    }
}
