//! `storm-runtime` — a Storm Runtime Host (Agent Runtime V1; decision 77).

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::signal::unix::{SignalKind, signal};

/// The host's state: its key, `host.json` and its session table. Linux: the
/// systemd unit's `StateDirectory` (freeze §5.8). macOS: AM33.
const DEFAULT_STATE: &str = storm_runtime::platform::DEFAULT_STATE;

#[derive(Parser)]
#[command(name = "storm-runtime", version, about = "Storm Runtime Host")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enroll this host with a Storm Server.
    ///
    /// Reads the enrollment string the app showed you from stdin, or prompts
    /// for it without echo. **Never pass it as an argument**: it carries a
    /// single-use secret, and arguments are visible to every user in `ps`.
    Enroll {
        #[arg(long, env = "STORM_RUNTIME_STATE", default_value = DEFAULT_STATE)]
        state: PathBuf,
        /// How this host appears in the app. Defaults to the hostname.
        #[arg(long)]
        name: Option<String>,
        /// Replace an existing enrollment.
        #[arg(long)]
        force: bool,
    },
    /// Run the host: connect to the server and run the agents it asks for.
    Serve {
        #[arg(long, env = "STORM_RUNTIME_STATE", default_value = DEFAULT_STATE)]
        state: PathBuf,
        #[arg(long, env = "STORM_RUNTIME_CONFIG", default_value = storm_runtime::config::DEFAULT_CONFIG)]
        config: PathBuf,
        /// Declare that this account runs nothing but this host (AM37): at
        /// startup and after shutdown, end every other process of it. Passed
        /// by the packaged service definitions; refused unless running as the
        /// service account, and never as root.
        #[arg(long)]
        exclusive_account: bool,
    },
    /// The agent's stdio MCP server for one gateway connection. Started by
    /// the agent CLI from the session's MCP config, never by hand: it reads
    /// its session handle and the daemon's socket from the environment, and
    /// holds no credential.
    McpBridge {
        /// The connection's slug.
        slug: String,
    },
    /// Install this host as a system service, as root. macOS: creates the
    /// `_stormruntime` account and `/Library/StormRuntime`, and loads the
    /// LaunchDaemon, which runs the host once it is enrolled. Re-run it to
    /// upgrade. On Linux the package does this.
    Install {
        /// Who shares the workspace root with the host. Defaults to the user
        /// who ran sudo.
        #[arg(long)]
        operator: Option<String>,
    },
    /// Remove the system service `install` set up, as root. Workspaces are
    /// always kept.
    Uninstall {
        /// Also remove the host's identity, state, logs, config and account.
        #[arg(long)]
        purge: bool,
    },
    /// Check this host's enrollment: verify the server, prove the key, and
    /// ask the server who it thinks this host is.
    Check {
        #[arg(long, env = "STORM_RUNTIME_STATE", default_value = DEFAULT_STATE)]
        state: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // **Before anything opens a TLS connection.** rustls panics on the first
    // handshake if it cannot tell which provider to use, and `ring` is the one
    // the musl/zig release build can compile — never aws-lc-rs (decision 77b,
    // the storm-server rule).
    let _ = rustls::crypto::ring::default_provider().install_default();

    match Cli::parse().command {
        // Logs nothing: stdout is the MCP stream, and stderr is the agent's.
        Command::McpBridge { slug } => storm_runtime::bridge::run(slug).await,
        Command::Enroll { state, name, force } => {
            let enrollment = read_enrollment()?;
            let name = name.unwrap_or_else(storm_runtime::platform::host_name);
            // Said before host.json exists: writing it may start the host,
            // whose account sweep would end this process (AM35, AM37).
            storm_runtime::client::enroll(&state, &enrollment, &name, force, |config| {
                println!("Enrolled as {} ({}).", name, config.host_id);
                println!("  server : {} ({})", config.server_url, config.server_id);
                println!("  state  : {}", state.display());
                let _ = std::io::Write::flush(&mut std::io::stdout());
            })
            .await?;
            Ok(())
        }
        Command::Serve {
            state,
            config,
            exclusive_account,
        } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .init();
            if exclusive_account {
                storm_runtime::platform::check_exclusive_account().map_err(anyhow::Error::msg)?;
            }
            // Before anything else can fail: SIGTERM must never take the
            // default action and leave sessions behind (AM37).
            let mut terminate = signal(SignalKind::terminate())?;
            let mut interrupt = signal(SignalKind::interrupt())?;
            let config = storm_runtime::config::RuntimeConfig::load(&config)?;
            let host = storm_runtime::host::Host::new(&state, config)?;
            if exclusive_account {
                // Whatever a previous host left behind — after a SIGKILL or a
                // crash nothing ended its sessions (AM37).
                sweep().await;
            }
            let outcome = tokio::select! {
                ended = host.clone().run() => ended,
                _ = terminate.recv() => Ok(()),
                _ = interrupt.recv() => Ok(()),
            };
            if let Err(e) = outcome {
                eprintln!("storm-runtime: {e:#}");
                // A refused key is final: exit with a status the unit is told
                // not to restart on, rather than looping against a server that
                // has revoked this host (freeze §5.6, AM36).
                if exclusive_account {
                    sweep().await;
                }
                std::process::exit(3);
            }
            tracing::info!("stopping");
            host.shutdown(storm_runtime::cli::DEFAULT_GRACE).await;
            if exclusive_account {
                sweep().await;
            }
            Ok(())
        }
        Command::Install { operator } => {
            storm_runtime::platform::install(&storm_runtime::platform::InstallOptions { operator })
        }
        Command::Uninstall { purge } => {
            storm_runtime::platform::uninstall(&storm_runtime::platform::UninstallOptions { purge })
        }
        Command::Check { state } => {
            let config = storm_runtime::identity::HostConfig::load(&state)?;
            let key = storm_runtime::identity::HostKey::load(&state, &config.key_id)?;
            let client = storm_runtime::client::ServerClient::for_host(&config)?;
            client.verify_server().await?;
            let token = client.authenticate(&config.host_id, &key).await?;
            let me: serde_json::Value = client
                .http()
                .get(format!("{}/v1/runtime/whoami", client.base()))
                .bearer_auth(&token.token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!(
                "OK: {} is enrolled at {} as {} (token valid until {}).",
                me["name"].as_str().unwrap_or("?"),
                config.server_url,
                config.host_id,
                token.expires
            );
            Ok(())
        }
    }
}

/// AM37's sweep, off the async threads: it sleeps through its grace.
async fn sweep() {
    let _ = tokio::task::spawn_blocking(|| {
        storm_runtime::platform::sweep_account(std::time::Duration::from_millis(500))
    })
    .await;
}

fn read_enrollment() -> Result<String> {
    let line = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Paste the enrollment string from the Storm app: ")
            .context("reading the enrollment string")?
    } else {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .context("reading the enrollment string from stdin")?;
        line
    };
    Ok(line.trim().to_string())
}
