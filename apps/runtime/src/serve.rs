//! `serve`: run the host until it is told to stop or is revoked (AM36,
//! AM37). One routine for every way a host is started — by hand, by systemd
//! or launchd (SIGTERM, SIGINT), or by the Windows Service Control Manager
//! (Stop, PreShutdown) — which differ only in what `stop` is.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

/// How a host's run ended.
#[derive(Debug)]
pub enum Outcome {
    /// Told to stop: its sessions were ended and nothing was posted, so the
    /// next `hello` reports them `failed (host_restart)` (AM37).
    Stopped,
    /// The server refused its key. `host.json` has been renamed (AM36); the
    /// service definition must not restart it.
    Revoked,
    /// The run itself failed after starting (its socket could not be bound,
    /// say). Unix units treat it as they always have: exit 3, no restart.
    Failed(anyhow::Error),
}

/// Runs the host from `state` and `config` until `stop` resolves or the
/// server revokes it. `stop` is armed before anything else can fail, so a
/// stop never takes a default action that leaves sessions behind.
pub async fn run(
    state: &Path,
    config: &Path,
    exclusive_account: bool,
    stop: impl Future<Output = ()>,
) -> Result<Outcome> {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        platform = crate::platform::NAME,
        state = %state.display(),
        config = %config.display(),
        exclusive_account,
        "starting"
    );
    crate::platform::prepare_home();
    let config = crate::config::RuntimeConfig::load(config)?;
    let host = crate::host::Host::new(state, config)?;
    host.log_inventory();
    if exclusive_account {
        // Whatever a previous host left behind — after a SIGKILL or a crash
        // nothing ended its sessions (AM37).
        sweep("start").await;
    }
    let outcome = tokio::select! {
        ended = host.clone().run() => ended,
        _ = stop => Ok(()),
    };
    if let Err(e) = outcome {
        eprintln!("storm-runtime: {e:#}");
        if exclusive_account {
            sweep("revoked").await;
        }
        if e.downcast_ref::<crate::client::Refused>().is_some() {
            tracing::error!(error = %format!("{e:#}"), "the server revoked this host");
            return Ok(Outcome::Revoked);
        }
        return Ok(Outcome::Failed(e));
    }
    tracing::info!("stopping");
    host.shutdown(crate::cli::DEFAULT_GRACE).await;
    if exclusive_account {
        sweep("stop").await;
    }
    Ok(Outcome::Stopped)
}

/// AM37's sweep, off the async threads: it sleeps through its grace.
async fn sweep(when: &str) {
    let swept =
        tokio::task::spawn_blocking(|| crate::platform::sweep_account(Duration::from_millis(500)))
            .await;
    match swept {
        Ok((0, _)) => tracing::info!(when, "account sweep: nothing left over"),
        Ok((hung_up, killed)) => tracing::info!(
            when,
            hung_up,
            killed,
            "account sweep: ended this account's other processes"
        ),
        Err(e) => tracing::warn!(when, error = %e, "account sweep did not finish"),
    }
}
