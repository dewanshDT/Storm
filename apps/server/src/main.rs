//! Storm sync server.
//!
//! A single binary that owns the canonical vault: a plain directory of
//! markdown files, plus a sibling `state/` directory holding the derived
//! SQLite index and version history.
//!
//! v1 binds to the LAN with a single shared bearer token. That is only
//! defensible while it stays on the LAN — exposing this beyond it needs TLS
//! and per-device tokens first.

mod api;
mod db;
mod frontmatter;
mod index;
mod merge;
mod parse;
mod vault;
mod watcher;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::{Mutex, broadcast};

use crate::api::AppState;
use crate::db::Db;
use crate::index::Indexer;
use crate::vault::Vault;

#[derive(Parser, Debug)]
#[command(name = "storm-server", about = "Storm sync server")]
struct Args {
    /// Vault directory — the canonical markdown files.
    #[arg(long, default_value = "./vault")]
    vault: PathBuf,

    /// State directory for the SQLite index. Kept out of the vault so the
    /// vault stays pure markdown.
    #[arg(long, default_value = "./state")]
    state: PathBuf,

    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    #[arg(long, default_value_t = 8484)]
    port: u16,

    /// Shared bearer token. Generated and printed if omitted.
    #[arg(long, env = "STORM_TOKEN")]
    token: Option<String>,

    /// Report what an import would change, then exit without writing anything.
    ///
    /// Run this first when pointing Storm at an existing Obsidian vault.
    #[arg(long)]
    dry_run: bool,

    /// Directory holding the built Flutter web client, served at `/`.
    #[arg(long)]
    web: Option<PathBuf>,

    /// Write a consistent snapshot of the index here and exit.
    ///
    /// Safe to run against a live server: SQLite does the right thing, which
    /// a plain file copy of a WAL-mode database does not.
    #[arg(long, value_name = "FILE")]
    backup_db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "storm_server=info,tower_http=warn".into()),
        )
        .init();

    let args = Args::parse();

    // Backup mode exits before touching the vault or starting anything.
    if let Some(dest) = &args.backup_db {
        let db = Db::open(&args.state.join("index.db")).context("opening index")?;
        db.snapshot_to(dest)
            .with_context(|| format!("writing snapshot to {}", dest.display()))?;
        println!("index snapshot written to {}", dest.display());
        return Ok(());
    }

    let vault = Vault::new(&args.vault).context("opening vault")?;
    let db = Db::open(&args.state.join("index.db")).context("opening index")?;
    let mut indexer = Indexer::new(vault, db);

    tracing::info!(vault = %args.vault.display(), "scanning vault");
    let report = indexer
        .reconcile(args.dry_run)
        .context("reconciling vault with index")?;

    if args.dry_run {
        println!("Dry run — nothing was written.\n");
        println!("  markdown files found : {}", report.scanned);
        println!("  would add `id` to    : {}", report.ids_assigned);
        for path in report.would_assign.iter().take(20) {
            println!("      {path}");
        }
        if report.ids_assigned > 20 {
            println!("      … and {} more", report.ids_assigned - 20);
        }
        println!("\nRun again without --dry-run to apply.");
        return Ok(());
    }

    tracing::info!(
        scanned = report.scanned,
        ids_assigned = report.ids_assigned,
        indexed = report.indexed,
        updated = report.updated,
        removed = report.removed,
        "vault reconciled"
    );

    let token = args.token.unwrap_or_else(|| {
        let generated = uuid::Uuid::new_v4().to_string();
        println!("\n  No --token given. Using a generated one for this run:\n");
        println!("      {generated}\n");
        println!("  Pass --token or set STORM_TOKEN to keep it stable across restarts.\n");
        generated
    });

    let (events, _) = broadcast::channel(1024);
    let vault_root = indexer.vault.root().to_path_buf();
    let state = Arc::new(AppState {
        indexer: Mutex::new(indexer),
        events,
        token,
    });

    watcher::spawn(&vault_root, state.clone()).context("starting file watcher")?;
    tracing::info!(path = %vault_root.display(), "watching vault for external edits");

    let mut app = api::router(state);

    // The Flutter web client is served by this same binary, so there is one
    // thing to run in the homelab rather than two.
    if let Some(web_dir) = &args.web {
        use axum::http::HeaderValue;
        use tower_http::services::{ServeDir, ServeFile};
        use tower_http::set_header::SetResponseHeaderLayer;

        let index = web_dir.join("index.html");
        // `fallback` rather than `not_found_service`: the latter serves
        // index.html's body but keeps ServeDir's 404 status, which breaks
        // caching and the Flutter service worker on any deep link.
        app = app
            .fallback_service(ServeDir::new(web_dir).fallback(ServeFile::new(index)))
            // Cross-origin isolation. Without these the browser withholds
            // `SharedArrayBuffer`, and drift's web backend silently falls back
            // from OPFS to IndexedDB — slower, and on Chrome for Android it
            // loses cross-tab safety entirely. Everything Storm serves is
            // same-origin, so isolating costs nothing.
            .layer(SetResponseHeaderLayer::overriding(
                axum::http::header::HeaderName::from_static("cross-origin-opener-policy"),
                HeaderValue::from_static("same-origin"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                axum::http::header::HeaderName::from_static("cross-origin-embedder-policy"),
                HeaderValue::from_static("require-corp"),
            ));
        tracing::info!(path = %web_dir.display(), "serving web client");
    }

    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;

    tracing::info!("storm-server listening on http://{addr}");
    axum::serve(listener, app).await.context("serving")?;
    Ok(())
}
