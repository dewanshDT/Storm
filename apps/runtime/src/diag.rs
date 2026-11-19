//! storm-runtime's half of decision 162c: installs the shared log ring
//! (`storm_diag::Ring`) and builds this host's `Diag` bundle — its
//! execution profile and whether it is enrolled, never a server address
//! or a workspace path. `storm-runtime diag` builds the same bundle from
//! a separate process: no running `serve` loop to ask, so its facts come
//! from `host.json` and `runtime.toml` on disk and its logs from
//! `--log-file`, never the ring.

use std::path::Path;

use anyhow::Result;
use storm_diag::{Diag, Ring, RuntimeDiag};

/// Builds the process-wide ring. Called once from `main`, alongside
/// (never instead of) the stdout/`--log-file` layer; the handle it
/// returns is `main`'s to keep — a later slice (162-2) is what threads it
/// into the host link so a live `Diag` can read from it.
pub fn install_ring() -> Ring {
    Ring::default()
}

/// The default tail length for a `Diag` bundle (162c: "the last ~300 log
/// lines").
pub const DEFAULT_LOG_LINES: usize = 300;

/// `storm-runtime diag`: a fresh process, with no live link to ask
/// whether it is actually connected right now — `online` is always
/// `false` here, which is also the honest answer while the service isn't
/// running at all.
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

/// The last `n` lines of `path`. Simple over efficient: a `diag` CLI
/// invocation is a one-off, and a log file big enough for a full read to
/// matter is already past what `--lines 300` would show anyway.
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
