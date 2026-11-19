//! storm-runtime's half of decision 162c.

use std::path::Path;

use anyhow::Result;
use storm_diag::{Diag, Ring, RuntimeDiag};

pub fn install_ring() -> Ring {
    Ring::default()
}

pub const DEFAULT_LOG_LINES: usize = 300;

/// A fresh process, with no live link to ask — `online` is always
/// `false`.
pub fn cli(
    state: &Path,
    config: Option<&Path>,
    log_file: Option<&Path>,
    lines: usize,
) -> Result<Diag> {
    let version = env!("CARGO_PKG_VERSION");
    let mut diag = Diag::new("storm-runtime", version);

    let enrolled = crate::identity::HostConfig::exists(state).unwrap_or(false);
    let execution_profile = config
        .map(crate::config::RuntimeConfig::load)
        .and_then(|c| c.ok())
        .map(|c| c.isolation.as_str().to_string())
        .unwrap_or_else(|| crate::config::Isolation::default().as_str().to_string());

    diag = diag.with_runtime(RuntimeDiag {
        execution_profile,
        enrolled,
        online: false,
    });

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
    fn cli_with_no_state_reports_not_enrolled() {
        let dir = tempfile::tempdir().unwrap();
        let diag = cli(dir.path(), None, None, 300).unwrap();
        let runtime = diag.runtime.expect("runtime diag");
        assert!(!runtime.enrolled);
        assert!(!runtime.online);
        assert_eq!(runtime.execution_profile, "off");
        assert_eq!(
            diag.logs_note.as_deref(),
            Some("logs: not available from CLI; use the app, or pass --log-file")
        );
    }

    #[test]
    fn cli_log_tail_is_redacted_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("storm-runtime.log");
        let mut lines = Vec::new();
        for i in 0..10 {
            lines.push(format!("line {i}"));
        }
        lines.push("storm-enroll:v1:http://10.0.0.5:8484:srv_x:pk_y:tok_SUPERSECRET".to_string());
        std::fs::write(&log, lines.join("\n")).unwrap();

        let diag = cli(dir.path(), None, Some(&log), 3).unwrap();
        assert_eq!(diag.logs.len(), 3);
        for line in &diag.logs {
            assert!(!line.contains("tok_SUPERSECRET"));
            assert!(!line.contains("10.0.0.5"));
        }
    }
}
