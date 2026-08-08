//! Storm sync server.
//!
//! A single binary that owns the canonical vaults: a storage root holding one
//! plain directory of markdown files per vault, plus a sibling `state/`
//! directory holding the registry and one derived SQLite index per vault.
//!
//! v1 binds to the LAN with a single shared bearer token. That is only
//! defensible while it stays on the LAN — exposing this beyond it needs TLS
//! and per-device tokens first.

mod api;
mod db;
mod frontmatter;
mod index;
mod mcp;
mod merge;
mod ops;
mod parse;
mod registry;
mod vault;
mod watcher;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::sync::{RwLock, broadcast};

use crate::api::AppState;
use crate::db::Db;
use crate::index::Indexer;
use crate::registry::Registry;

#[derive(Parser, Debug)]
#[command(name = "storm-server", about = "Storm sync server")]
struct Args {
    /// Storage root — a directory holding one directory per vault.
    #[arg(long, env = "STORM_VAULT_ROOT")]
    vault_root: Option<PathBuf>,

    /// Deprecated: a single vault directory.
    ///
    /// Accepted for one release so an existing single-vault deployment starts
    /// without edits. The storage root becomes this directory's *parent* and
    /// this one directory is registered as a vault.
    #[arg(long)]
    vault: Option<PathBuf>,

    /// State directory for the registry and the per-vault SQLite indexes. Kept
    /// out of the vaults so they stay pure markdown.
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

    /// Serve the Model Context Protocol endpoint at `/mcp`.
    ///
    /// Read-only tools over the same vaults and the same bearer token. Off by
    /// default: it is a new surface on the notes, and turning one on should be
    /// a decision rather than a side effect of upgrading.
    #[arg(long)]
    mcp: bool,

    /// Write consistent snapshots of every vault index into this directory,
    /// then exit.
    ///
    /// Safe to run against a live server: SQLite does the right thing, which
    /// a plain file copy of a WAL-mode database does not. Writes one
    /// `<vault-id>.db` per vault plus a copy of `vaults.json`.
    #[arg(long, value_name = "DIR")]
    backup_db: Option<PathBuf>,
}

/// Works out the storage root from the two mutually exclusive flags.
///
/// The compatibility shim is deliberately explicit rather than a
/// reinterpretation. `--vault` points *at* a vault's markdown directory, so
/// treating that same path as a root would make the server scan inside the
/// vault for sub-vaults — registering `Daily/` and `Projects/` as vaults and
/// finding no notes at the top level.
///
/// Every failure here exits non-zero with the actual paths. Starting with zero
/// vaults instead would read as "my notes disappeared" rather than as a skipped
/// step in the runbook.
fn resolve_root(args: &Args) -> Result<PathBuf> {
    match (&args.vault_root, &args.vault) {
        (Some(_), Some(_)) => bail!(
            "--vault-root and --vault are mutually exclusive. \
             Use --vault-root; --vault is deprecated."
        ),
        (Some(root), None) => Ok(root.clone()),
        (None, Some(vault)) => {
            if !vault.exists() {
                bail!(
                    "--vault {} does not exist.\n\
                     If you are migrating, move the vault under a storage root \
                     and pass --vault-root instead.",
                    vault.display()
                );
            }
            if !vault.is_dir() {
                bail!("--vault {} is not a directory", vault.display());
            }
            let absolute = vault
                .canonicalize()
                .with_context(|| format!("resolving --vault {}", vault.display()))?;
            let parent = absolute
                .parent()
                .filter(|p| p.parent().is_some())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "--vault {} has no usable parent to use as a storage root",
                        absolute.display()
                    )
                })?
                .to_path_buf();

            tracing::warn!(
                vault = %absolute.display(),
                root = %parent.display(),
                "--vault is deprecated; using its parent as the storage root. \
                 Move to --vault-root before the next release."
            );
            Ok(parent)
        }
        (None, None) => Ok(PathBuf::from("./vaults")),
    }
}

/// Moves a pre-multi-vault `state/index.db` into its vault's own state
/// directory.
///
/// Runs once, and only when there is exactly one vault — with more than one
/// there is no way to know which the old index belonged to, and guessing would
/// attach one vault's history to another.
///
/// The index must be *moved*, not left to be rebuilt: `note_versions` is the
/// 3-way merge's base and is the one thing in `state/` that cannot be
/// regenerated from the markdown.
fn migrate_legacy_index(state_dir: &Path, registry: &Registry) -> Result<()> {
    let legacy = state_dir.join("index.db");
    if !legacy.exists() {
        return Ok(());
    }
    if registry.vaults.len() != 1 {
        tracing::warn!(
            path = %legacy.display(),
            vaults = registry.vaults.len(),
            "a legacy index is present but there is not exactly one vault — \
             leaving it alone. Move it to state/<vault-id>/index.db by hand."
        );
        return Ok(());
    }

    let entry = &registry.vaults[0];
    let dest_dir = state_dir.join(&entry.id);
    let dest = dest_dir.join("index.db");
    if dest.exists() {
        tracing::warn!(
            legacy = %legacy.display(),
            dest = %dest.display(),
            "both a legacy and a per-vault index exist — keeping the per-vault one"
        );
        return Ok(());
    }

    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("creating {}", dest_dir.display()))?;
    std::fs::rename(&legacy, &dest)
        .with_context(|| format!("moving {} to {}", legacy.display(), dest.display()))?;

    // WAL sidecars belong to the database and must travel with it; leaving
    // them behind can strand committed pages.
    for suffix in ["-wal", "-shm"] {
        let from = state_dir.join(format!("index.db{suffix}"));
        if from.exists() {
            let to = dest_dir.join(format!("index.db{suffix}"));
            let _ = std::fs::rename(&from, &to);
        }
    }

    tracing::info!(
        vault = %entry.name,
        from = %legacy.display(),
        to = %dest.display(),
        "migrated the single-vault index, preserving version history"
    );
    Ok(())
}

/// Snapshots every vault's index into `dest`.
fn backup_all(state_dir: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;

    let registry_src = state_dir.join(registry::REGISTRY_FILE);
    if registry_src.exists() {
        std::fs::copy(&registry_src, dest.join(registry::REGISTRY_FILE))
            .with_context(|| format!("copying {}", registry_src.display()))?;
    }

    let registry = Registry::load(state_dir, Path::new("/"))?;
    if registry.vaults.is_empty() {
        println!("no vaults registered — nothing to snapshot");
        return Ok(());
    }
    for entry in &registry.vaults {
        let src = state_dir.join(&entry.id).join("index.db");
        if !src.exists() {
            continue;
        }
        let db = Db::open(&src, &entry.id)
            .with_context(|| format!("opening index for {}", entry.name))?;
        // The snapshot mirrors the state directory's own layout, so restoring
        // is a plain copy and the backup can be verified by simply reopening
        // it as a state directory.
        let out_dir = dest.join(&entry.id);
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("creating {}", out_dir.display()))?;
        let out = out_dir.join("index.db");
        db.snapshot_to(&out)
            .with_context(|| format!("writing snapshot to {}", out.display()))?;
        println!("  {} -> {}", entry.name, out.display());
    }
    Ok(())
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

    // Backup mode exits before touching a vault or starting anything.
    if let Some(dest) = &args.backup_db {
        backup_all(&args.state, dest)?;
        println!("index snapshots written to {}", dest.display());
        return Ok(());
    }

    let root = resolve_root(&args)?;
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating storage root {}", root.display()))?;
    std::fs::create_dir_all(&args.state)
        .with_context(|| format!("creating state directory {}", args.state.display()))?;
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving storage root {}", root.display()))?;
    let state_dir = args
        .state
        .canonicalize()
        .with_context(|| format!("resolving state directory {}", args.state.display()))?;

    let mut registry = Registry::load(&state_dir, &root).context("reading the vault registry")?;
    let adopted = registry.scan_root(&state_dir, &index::now_rfc3339())?;
    for dir in &adopted {
        tracing::info!(dir = %dir, "adopted a new vault");
    }
    migrate_legacy_index(&state_dir, &registry)?;

    tracing::info!(
        root = %root.display(),
        vaults = registry.vaults.len(),
        "scanning vaults"
    );

    if args.dry_run {
        println!("Dry run — nothing was written.\n");
        println!("  storage root : {}", root.display());
        println!("  vaults       : {}\n", registry.vaults.len());
        for entry in &registry.vaults {
            let vault = vault::Vault::new(registry.path_of(entry))?;
            let db = Db::open_in_memory()?;
            let mut ix = Indexer::new(vault, db);
            let report = ix.reconcile(true)?;
            println!("  {}", entry.name);
            println!("      markdown files found : {}", report.scanned);
            println!("      would add `id` to    : {}", report.ids_assigned);
            for path in report.would_assign.iter().take(10) {
                println!("          {path}");
            }
            if report.ids_assigned > 10 {
                println!("          … and {} more", report.ids_assigned - 10);
            }
        }
        println!("\nRun again without --dry-run to apply.");
        return Ok(());
    }

    registry.save(&state_dir).context("saving the registry")?;
    let mut vault_set = api::open_vaults(&registry, &state_dir).context("opening vaults")?;

    let token = args.token.unwrap_or_else(|| {
        let generated = uuid::Uuid::new_v4().to_string();
        println!("\n  No --token given. Using a generated one for this run:\n");
        println!("      {generated}\n");
        println!("  Pass --token or set STORM_TOKEN to keep it stable across restarts.\n");
        generated
    });

    let (events, _) = broadcast::channel(1024);
    let (root_changed, _) = broadcast::channel(4);

    // `--mcp` turns it on at boot; otherwise the persisted setting decides, so
    // a toggle made from the app survives a restart. The flag is an override
    // rather than the source of truth, which is why it also persists — a server
    // started with it and then switched off in the app stays off.
    let mcp_enabled = args.mcp || vault_set.registry.mcp_enabled;
    if mcp_enabled != vault_set.registry.mcp_enabled {
        vault_set.registry.mcp_enabled = mcp_enabled;
        vault_set.registry.save(&state_dir)?;
    }
    let mcp_writable = mcp_enabled && vault_set.registry.mcp_writable;
    tracing::info!(
        enabled = mcp_enabled,
        writable = mcp_writable,
        allowed_hosts = ?mcp::allowed_hosts(&args.host, args.port),
        "MCP endpoint at /mcp (read-only tools, same bearer token)"
    );

    let state = Arc::new(AppState {
        vaults: RwLock::new(vault_set),
        events,
        token,
        state_dir: state_dir.clone(),
        root_changed,
        mcp_enabled: std::sync::atomic::AtomicBool::new(mcp_enabled),
        mcp_writable: std::sync::atomic::AtomicBool::new(mcp_writable),
    });

    // One watcher over the whole root, attributing each event to a vault by
    // directory prefix. Adding or removing a vault then needs no watcher work
    // at all, and a root change respawns this one.
    watcher::spawn(root.clone(), state.clone()).context("starting file watcher")?;
    tracing::info!(path = %root.display(), "watching the storage root for external edits");

    let mut app = api::router(
        state,
        mcp::McpOptions {
            allowed_hosts: mcp::allowed_hosts(&args.host, args.port),
        },
    );

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

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with(vault_root: Option<PathBuf>, vault: Option<PathBuf>) -> Args {
        Args {
            vault_root,
            vault,
            state: PathBuf::from("./state"),
            host: "127.0.0.1".into(),
            port: 8484,
            token: None,
            mcp: false,
            dry_run: false,
            web: None,
            backup_db: None,
        }
    }

    #[test]
    fn vault_root_is_used_as_given() {
        let dir = tempdir::TempDir::new("storm-root").unwrap();
        let root = dir.path().join("vaults");
        std::fs::create_dir_all(&root).unwrap();
        let resolved = resolve_root(&args_with(Some(root.clone()), None)).unwrap();
        assert_eq!(resolved, root);
    }

    #[test]
    fn the_vault_shim_uses_the_parent_as_the_root() {
        // `--vault` points AT a vault, not at something containing vaults.
        // Reinterpreting the same path as a root would scan inside the vault
        // and register `Daily/` and `Projects/` as vaults.
        let dir = tempdir::TempDir::new("storm-shim").unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(vault.join("Daily")).unwrap();

        let resolved = resolve_root(&args_with(None, Some(vault.clone()))).unwrap();
        assert_eq!(resolved, dir.path().canonicalize().unwrap());
        assert_ne!(resolved, vault, "the vault itself is never the root");
    }

    #[test]
    fn a_missing_vault_path_fails_loudly_with_both_paths() {
        // A skipped `mv` in the runbook must read as a clear failure, not as
        // a healthy server with zero vaults.
        let dir = tempdir::TempDir::new("storm-missing").unwrap();
        let vault = dir.path().join("not-here");
        let err = resolve_root(&args_with(None, Some(vault.clone())))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not-here"), "message lost the path: {err}");
        assert!(err.contains("--vault-root"), "no way forward given: {err}");
    }

    #[test]
    fn a_vault_path_that_is_a_file_is_refused() {
        let dir = tempdir::TempDir::new("storm-file").unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "x").unwrap();
        assert!(resolve_root(&args_with(None, Some(file))).is_err());
    }

    #[test]
    fn both_flags_together_are_an_error_not_a_precedence_rule() {
        let dir = tempdir::TempDir::new("storm-both").unwrap();
        std::fs::create_dir_all(dir.path().join("vault")).unwrap();
        assert!(
            resolve_root(&args_with(
                Some(dir.path().to_path_buf()),
                Some(dir.path().join("vault")),
            ))
            .is_err()
        );
    }

    #[test]
    fn the_legacy_index_moves_into_the_single_vault() {
        // `note_versions` is the merge base and cannot be rebuilt from the
        // markdown, so this has to be a move rather than a regeneration.
        let dir = tempdir::TempDir::new("storm-legacy").unwrap();
        let root = dir.path().join("vaults");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("personal")).unwrap();
        std::fs::create_dir_all(&state).unwrap();

        // A recognisable legacy index with one note's history in it.
        {
            let mut db = Db::open(&state.join("index.db"), "legacy").unwrap();
            let row = db::NoteRow {
                id: "n1".into(),
                path: "A.md".into(),
                title: "A".into(),
                version: 1,
                content_hash: "h".into(),
                created: "2026-08-07T10:00:00Z".into(),
                modified: "2026-08-07T10:00:00Z".into(),
                size: 3,
            };
            db.record_note(&row, "# A", &parse::extract("# A"), db::KIND_CREATED, None)
                .unwrap();
        }

        let mut registry = Registry::load(&state, &root).unwrap();
        registry.scan_root(&state, "2026-08-07T10:00:00Z").unwrap();
        migrate_legacy_index(&state, &registry).unwrap();

        let id = &registry.vaults[0].id;
        assert!(!state.join("index.db").exists(), "the legacy file moved");
        let moved = state.join(id).join("index.db");
        assert!(moved.exists());

        let db = Db::open(&moved, id).unwrap();
        assert_eq!(db.list_notes().unwrap().len(), 1);
        assert_eq!(
            db.version_content("n1", 1).unwrap().as_deref(),
            Some("# A"),
            "version history must survive the move"
        );
    }

    #[test]
    fn the_legacy_index_is_left_alone_when_the_vault_is_ambiguous() {
        // Two vaults means no way to know which one the old index belonged to.
        // Guessing would attach one vault's history to another.
        let dir = tempdir::TempDir::new("storm-ambiguous").unwrap();
        let root = dir.path().join("vaults");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("personal")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        Db::open(&state.join("index.db"), "legacy").unwrap();

        let mut registry = Registry::load(&state, &root).unwrap();
        registry.scan_root(&state, "2026-08-07T10:00:00Z").unwrap();
        migrate_legacy_index(&state, &registry).unwrap();

        assert!(state.join("index.db").exists(), "left for a human to place");
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let dir = tempdir::TempDir::new("storm-twice").unwrap();
        let root = dir.path().join("vaults");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("personal")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        Db::open(&state.join("index.db"), "legacy").unwrap();

        let mut registry = Registry::load(&state, &root).unwrap();
        registry.scan_root(&state, "2026-08-07T10:00:00Z").unwrap();
        migrate_legacy_index(&state, &registry).unwrap();
        migrate_legacy_index(&state, &registry).unwrap();

        let id = &registry.vaults[0].id;
        assert!(state.join(id).join("index.db").exists());
    }
}
