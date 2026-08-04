//! Watches the vault for edits made outside Storm.
//!
//! The PRD wants the vault to stay greppable and possibly exported read-only
//! over the NAS share, so the server cannot assume it is the only writer.
//! Someone editing a note with `nvim` on the server, or a script appending to
//! a daily note, must show up on every client.
//!
//! The server's *own* writes also land here. They are filtered by content
//! hash rather than by bookkeeping: [`Indexer::refresh_path`] returns `None`
//! when a file's hash already matches the index, which is inherently correct
//! even if events arrive late, out of order, or coalesced.

use std::path::Path;

use std::time::Duration;

use anyhow::Result;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::api::Shared;

/// Events are coalesced over this window. Editors routinely produce several
/// events per save (truncate, write, chmod, rename-into-place).
const DEBOUNCE: Duration = Duration::from_millis(500);

/// Starts watching `root`, reindexing and broadcasting as files change.
pub fn spawn(root: &Path, state: Shared) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;

    let root = root.to_path_buf();
    tokio::spawn(async move {
        // Keep the watcher alive for the lifetime of the task.
        let _watcher = watcher;
        let mut pending: Vec<String> = Vec::new();

        loop {
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
                    if let Some(rel) = relative_markdown_path(&root, &path)
                        .filter(|r| !pending.contains(r))
                    {
                        pending.push(rel);
                    }
                }
                continue;
            }

            if pending.is_empty() {
                continue;
            }
            for rel in std::mem::take(&mut pending) {
                apply(&state, &rel).await;
            }
        }
    });

    Ok(())
}

/// Reindexes one path and pushes the resulting change, if any.
async fn apply(state: &Shared, rel: &str) {
    let mut ix = state.indexer.lock().await;
    match ix.refresh_path(rel) {
        // `None` means the content hash was unchanged — almost always our own
        // write coming back to us.
        Ok(None) => {}
        Ok(Some(seq)) => {
            tracing::info!(path = %rel, seq, "external change indexed");
            if let Ok(Some(change)) = ix.db.changes_since(seq - 1, 1).map(|c| c.into_iter().next())
            {
                let _ = state.events.send(change);
            }
        }
        Err(e) => tracing::warn!(path = %rel, error = %e, "failed to index external change"),
    }
}

/// Maps an absolute path to a vault-relative markdown path, or `None`.
///
/// Filters out non-markdown files, dotted directories, and the temp files our
/// own atomic writes create.
fn relative_markdown_path(root: &Path, path: &Path) -> Option<String> {
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
    Some(rel_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from("/vault")
    }

    #[test]
    fn accepts_markdown_under_the_root() {
        assert_eq!(
            relative_markdown_path(&root(), Path::new("/vault/Daily/A.md")),
            Some("Daily/A.md".to_string())
        );
    }

    #[test]
    fn ignores_our_own_temp_files() {
        // Atomic writes create these; reacting to them would be a feedback loop.
        assert_eq!(
            relative_markdown_path(&root(), Path::new("/vault/A.md.storm-tmp")),
            None
        );
    }

    #[test]
    fn ignores_dotted_directories() {
        assert_eq!(
            relative_markdown_path(&root(), Path::new("/vault/.obsidian/workspace.md")),
            None
        );
        assert_eq!(
            relative_markdown_path(&root(), Path::new("/vault/.git/COMMIT_EDITMSG.md")),
            None
        );
    }

    #[test]
    fn ignores_non_markdown() {
        assert_eq!(
            relative_markdown_path(&root(), Path::new("/vault/attachments/img.png")),
            None
        );
    }

    #[test]
    fn ignores_paths_outside_the_root() {
        assert_eq!(relative_markdown_path(&root(), Path::new("/etc/passwd.md")), None);
    }
}
