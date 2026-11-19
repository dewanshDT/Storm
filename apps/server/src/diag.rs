//! storm-server's half of decision 162c.

use std::path::Path;

use anyhow::Result;
use storm_diag::{Diag, Ring, ServerDiag};

pub fn install_ring() -> Ring {
    Ring::default()
}

pub const DEFAULT_LOG_LINES: usize = 300;

/// A fresh process, so counts come from disk rather than a live
/// `Registry`, and logs are `log_file`'s tail rather than the ring.
pub fn cli(state: &Path, log_file: Option<&Path>, lines: usize) -> Result<Diag> {
    let version = env!("CARGO_PKG_VERSION");
    let mut diag = Diag::new("storm-server", version);

    if let Ok(registry) = crate::registry::Registry::load(state, Path::new("")) {
        let host_count = if crate::auth::AuthDb::path_in(state).exists() {
            crate::auth::AuthDb::open(state)
                .and_then(|db| db.list_hosts())
                .map(|hosts| hosts.iter().filter(|h| !h.is_revoked()).count())
                .unwrap_or(0)
        } else {
            0
        };
        diag = diag.with_server(ServerDiag {
            vault_count: registry.vaults.len(),
            relay_count: registry.relays.len(),
            relay_registered: None,
            host_count,
            hosts_online: None,
        });
    }

    match log_file {
        Some(path) => match tail_lines(path, lines) {
            Ok(tail) => {
                diag = diag.with_logs(tail.iter().map(|l| storm_diag::redact_default(l)).collect());
            }
            Err(e) => {
                diag = diag.with_logs_note(format!("could not read {}: {e}", path.display()));
            }
        },
        None => {
            diag = diag
                .with_logs_note("logs: not available from CLI; use the app, or pass --log-file");
        }
    }

    Ok(diag)
}

fn tail_lines(path: &Path, n: usize) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)?;
    let all: Vec<&str> = text.lines().collect();
    let skip = all.len().saturating_sub(n);
    Ok(all[skip..].iter().map(|l| l.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_with_no_state_and_no_log_file_still_returns_a_bundle() {
        let dir = tempdir::TempDir::new("storm-diag-cli").unwrap();
        let diag = cli(dir.path(), None, 300).unwrap();
        assert_eq!(diag.component, "storm-server");
        // No `vaults.json` yet: `Registry::load` hands back an empty
        // registry rather than an error, so the bundle still has a
        // `server` section, with zero counts.
        let server = diag.server.expect("server diag");
        assert_eq!(server.vault_count, 0);
        assert_eq!(server.host_count, 0);
        assert_eq!(
            diag.logs_note.as_deref(),
            Some("logs: not available from CLI; use the app, or pass --log-file")
        );
    }

    #[test]
    fn cli_reports_vault_and_relay_counts_from_a_registry_on_disk() {
        let dir = tempdir::TempDir::new("storm-diag-cli-registry").unwrap();
        let root = dir.path().join("vaults");
        std::fs::create_dir_all(&root).unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let mut registry = crate::registry::Registry::load(&state, &root).unwrap();
        registry.create("My Notes", "2026-11-19T00:00:00Z").unwrap();
        registry.save(&state).unwrap();

        let diag = cli(&state, None, 300).unwrap();
        let server = diag.server.expect("server diag");
        assert_eq!(server.vault_count, 1);
        assert_eq!(server.relay_count, 0);
        assert_eq!(server.host_count, 0);
        assert!(server.relay_registered.is_none());
        assert!(server.hosts_online.is_none());
    }

    #[test]
    fn cli_log_tail_is_redacted_and_bounded() {
        let dir = tempdir::TempDir::new("storm-diag-cli-log").unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let log = dir.path().join("storm-server.log");
        let mut lines = Vec::new();
        for i in 0..10 {
            lines.push(format!("line {i}"));
        }
        lines.push("Authorization: Bearer stk_SUPERSECRETVALUE0123456789".to_string());
        std::fs::write(&log, lines.join("\n")).unwrap();

        let diag = cli(&state, Some(&log), 3).unwrap();
        assert_eq!(diag.logs.len(), 3);
        for line in &diag.logs {
            assert!(!line.contains("stk_SUPERSECRETVALUE0123456789"));
        }
        assert!(diag.logs.last().unwrap().contains("Bearer <redacted>"));
    }
}
