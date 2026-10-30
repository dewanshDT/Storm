//! `storm-runtime` — a Storm Runtime Host (Agent Runtime V1; decision 77).

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

/// The host's state: its key, `host.json` and its session table. Linux: the
/// systemd unit's `StateDirectory` (freeze §5.8). macOS: AM33. Windows: AM48.
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
    /// An external MCP client's stdio server for a Storm Server's `/mcp`
    /// (Claude Code, Cursor, Claude Desktop start it). Verifies the server's
    /// identity before sending anything.
    ///
    /// The key is read from STORM_MCP_KEY or --key-file, **never an
    /// argument**: arguments are visible to every user in `ps`.
    McpConnect {
        /// The server's id (`srv_…`), as the app shows it.
        #[arg(long)]
        server: String,
        /// The server's public key, unpadded base64url, as the app shows it.
        // base64url starts with `-` one key in 64; clap would read it as a flag.
        #[arg(long, allow_hyphen_values = true)]
        pk: String,
        /// The server's own address, e.g. `http://192.168.1.10:8484`.
        /// Optional when a --relay is given.
        #[arg(long)]
        addr: Option<String>,
        /// A relay that reaches the server (`wss://…`). Repeatable. Only a
        /// seed: the server's current relays are learned on each connect
        /// and cached.
        #[arg(long = "relay")]
        relays: Vec<String>,
        /// A file holding the `stk_` key, instead of STORM_MCP_KEY.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Tells the host which conversation the agent is on, so Storm can
    /// resume it. Run by the agent's own hook or plugin, never by hand; reads
    /// the id from stdin (a Claude Code hook's input) when none is given.
    HarnessSession { id: Option<String> },
    /// Tells the host the agent's permission prompt is waiting for a person
    /// (`permission_prompt`) or answered (`none`). Run by the agent's own hook
    /// or plugin, never by hand.
    // Hidden while no adapter's hooks are on (134-9): the site lists commands.
    #[command(hide = true)]
    Attention { kind: String },
    /// The agent's turn began, ended or hit a usage limit (decision 144b).
    /// Run by the agent's own hook or plugin, never by hand.
    #[command(hide = true)]
    HarnessSignal { kind: String },
    /// The agent's final message for its turn (decision 145c), from its
    /// hook's input on stdin. Run by the agent's own hook or plugin.
    #[command(hide = true)]
    HarnessResult,
    /// A child's tool call its own permissions refused (decision 148b),
    /// from its hook's input on stdin. Run by the agent's own hook or
    /// plugin, never by hand.
    #[command(hide = true)]
    HarnessDenied { kind: String },
    /// Waits until the host says something is unread, then wakes the agent
    /// the way it documents (decisions 144c, 145d). Run by the agent's own
    /// hook or plugin, never by hand.
    #[command(hide = true)]
    HarnessWake {
        #[arg(long)]
        plugin: bool,
    },
    /// Install this host as a system service, as root. macOS: creates the
    /// `_stormruntime` account and `/Library/StormRuntime`, and loads the
    /// LaunchDaemon, which runs the host once it is enrolled. Windows (in an
    /// elevated shell): creates `C:\ProgramData\StormRuntime` and the
    /// `StormRuntime` service, which enrolling starts. Re-run it to upgrade.
    /// On Linux the package does this.
    Install {
        /// Who shares the workspace root with the host. Defaults to the user
        /// who ran sudo.
        #[arg(long)]
        operator: Option<String>,
    },
    /// Remove the system service `install` set up, as root (elevated on
    /// Windows). Workspaces are always kept.
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
    /// The Windows service's entry point (AM50). Run by the Service Control
    /// Manager, never by hand.
    #[cfg(windows)]
    #[command(hide = true)]
    Service,
    /// One agent install on an isolated host (decision 136g), run by the
    /// isolation helper as the install identity, never by hand. Prints one
    /// JSON line: the version, or the failure's code.
    #[command(hide = true)]
    AgentJob { agent: String, action: String },
    /// One of the two probe sessions of the isolation self-test (decision
    /// 136j), run by the isolation helper as a session, never by hand.
    #[command(hide = true)]
    IsolationProbe {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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
        Command::McpConnect {
            server,
            pk,
            addr,
            relays,
            key_file,
        } => {
            // stderr only: stdout is the MCP stream.
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .init();
            storm_runtime::mcp_connect::run(storm_runtime::mcp_connect::Options {
                server,
                pk,
                addr,
                relays,
                key_file,
            })
            .await
        }
        Command::HarnessSession { id } => {
            storm_runtime::harness::report(id);
            Ok(())
        }
        Command::Attention { kind } => {
            storm_runtime::harness::report_attention(&kind);
            Ok(())
        }
        Command::HarnessSignal { kind } => {
            storm_runtime::harness::report_signal(&kind);
            Ok(())
        }
        Command::HarnessResult => {
            storm_runtime::harness::report_result();
            Ok(())
        }
        Command::HarnessDenied { kind } => {
            storm_runtime::harness::report_denied(&kind);
            Ok(())
        }
        Command::HarnessWake { plugin } => std::process::exit(storm_runtime::harness::wake(plugin)),
        Command::AgentJob { agent, action } => {
            let line = storm_runtime::install::isolated_job(
                std::path::Path::new(DEFAULT_STATE),
                &agent,
                &action,
            )
            .await;
            println!("{line}");
            Ok(())
        }
        Command::IsolationProbe { args } => {
            std::process::exit(storm_runtime::isolation_probe::main(&args))
        }
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
            if cfg!(windows) && state == std::path::Path::new(DEFAULT_STATE) {
                start_the_service();
            }
            Ok(())
        }
        #[cfg(windows)]
        Command::Service => storm_runtime::platform::service_main(),
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
            let stop = stop_signal()?;
            match storm_runtime::serve::run(&state, &config, exclusive_account, stop).await? {
                storm_runtime::serve::Outcome::Stopped => Ok(()),
                // A refused key is final: exit with a status the unit is told
                // not to restart on, rather than looping against a server
                // that has revoked this host (freeze §5.6, AM36).
                storm_runtime::serve::Outcome::Revoked
                | storm_runtime::serve::Outcome::Failed(_) => std::process::exit(3),
            }
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

/// What ends a host run by hand or by systemd and launchd: SIGTERM or
/// SIGINT.
#[cfg(unix)]
fn stop_signal() -> Result<impl std::future::Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

/// What ends a host run by hand in a console: Ctrl+C, Ctrl+Break, closing
/// the window, or the machine shutting down. The service has its own.
#[cfg(windows)]
fn stop_signal() -> Result<impl std::future::Future<Output = ()>> {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown};
    let (mut c, mut b, mut close, mut shutdown) =
        (ctrl_c()?, ctrl_break()?, ctrl_close()?, ctrl_shutdown()?);
    Ok(async move {
        tokio::select! {
            _ = c.recv() => {}
            _ = b.recv() => {}
            _ = close.recv() => {}
            _ = shutdown.recv() => {}
        }
    })
}

/// After enrolling the installed host: the service runs from now on (AM50).
fn start_the_service() {
    #[cfg(windows)]
    match storm_runtime::platform::enable_and_start() {
        Ok(()) => println!("Started the StormRuntime service; it starts with Windows from now on."),
        Err(e) => println!(
            "Enrolled, but the service did not start: {e:#}\n\
             Start it from an elevated shell: Start-Service StormRuntime"
        ),
    }
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
