//! Storm sync server.
//!
//! A single binary that owns the canonical vaults: a storage root holding one
//! plain directory of markdown files per vault, plus a sibling `state/`
//! directory holding the registry and one derived SQLite index per vault.
//!
//! Operator commands (`up` / `down` / `status`) own the systemd install; the
//! long-running process is `serve`.
//!
//! v1 binds to the LAN. Authentication is per-device pairing plus sessions
//! defensible while it stays on the LAN — exposing this beyond it needs TLS
//! and per-device tokens first.

// An axum handler that can fail returns `Response` as its error type — that is
// the framework's shape, not a choice this crate makes, and it is how a handler
// answers with a status and a body instead of a 500. clippy counts those bytes
// and asks for `Box<Response>`, which would move an already-heap-backed body
// behind a second allocation in every handler to shrink a value that never
// outlives one request.
//
// **It fires on x86_64-linux and not on aarch64-darwin**, on the same 1.98.0.
// So it arrived through `dtolnay/rust-toolchain@stable` moving under CI — the
// `server (rust)` job went red on 2026-08-31 and PR #34 was merged past it —
// and it cannot be reproduced, or a fix verified, on a Mac. A lint nobody can
// run locally is not a gate; it is a tax on whoever pushes next.
#![allow(clippy::result_large_err)]

mod api;
mod auth;
mod db;
mod frontmatter;
mod index;
mod install;
mod kit;
mod mcp;
mod merge;
mod ops;
mod parse;
mod registry;
mod relay;
mod vault;
mod watcher;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tokio::sync::{RwLock, broadcast};

use crate::api::AppState;
use crate::db::Db;
use crate::index::Indexer;
use crate::registry::Registry;

#[derive(Parser, Debug)]
#[command(name = "storm-server", about = "Storm sync server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the sync server (what systemd starts).
    Serve(ServeArgs),
    /// Write config, enable the systemd unit, and start it.
    Up(UpArgs),
    /// Stop and disable the systemd unit.
    Down,
    /// Show unit state and a local health probe.
    Status,
    /// Report what an import would change, then exit without writing.
    DryRun(VaultArgs),
    /// Create and manage local user accounts.
    User(UserArgs),
    /// Set a user's password. The recovery path when one is forgotten (A11).
    Passwd(PasswdArgs),
    /// Print a bootstrap pairing QR code for a fresh server (no users yet).
    Pair(PairArgs),
    /// Snapshot every vault index into DIR, then exit.
    BackupDb {
        /// State directory holding the registry and indexes.
        #[arg(long, default_value = "./state")]
        state: PathBuf,
        /// Destination directory for the snapshots.
        #[arg(value_name = "DIR")]
        dest: PathBuf,
    },
}

#[derive(clap::Args, Debug)]
struct UpArgs {
    /// Directory for vaults/, state/, and backups/ (default /srv/storm).
    ///
    /// Ignored for a path that `--vault-root` / `--state` override individually —
    /// needed when vaults and state already live in different places.
    #[arg(long, default_value = "/srv/storm")]
    data_root: PathBuf,

    /// Override the storage root (default: `<data-root>/vaults`).
    #[arg(long)]
    vault_root: Option<PathBuf>,

    /// Override the state directory (default: `<data-root>/state`).
    #[arg(long)]
    state: Option<PathBuf>,

    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    #[arg(long, default_value_t = 8484)]
    port: u16,

    /// Built Flutter web client directory (package default).
    #[arg(long, default_value = "/usr/share/storm/web")]
    web: PathBuf,
}

/// Account management, on the host.
///
/// These run on the box rather than over the network, and that is the design
/// rather than a limitation for now: creating a user remotely needs device auth
/// (A8), which arrives with pairing. Until then the only way to make an account
/// is to have shell access, which is the same trust level A11 already accepts
/// for `passwd`.
///
/// **There is deliberately no `--password` flag anywhere in here.** A password
/// in an argument is in the shell history and, while the process runs, in `ps`
/// for every other user on the box. The password is prompted for without echo,
/// or read from stdin with `--password-stdin` for scripts.
#[derive(clap::Args, Debug)]
struct UserArgs {
    /// State directory holding auth.db.
    ///
    /// `global` so it reads naturally in either position — `user --state X add
    /// name` and `user add name --state X` are the same command. Without it
    /// clap accepts only the first, which is not where a hand reaches for it.
    #[arg(long, default_value = "./state", global = true)]
    state: PathBuf,

    #[command(subcommand)]
    command: UserCommand,
}

#[derive(Subcommand, Debug)]
enum UserCommand {
    /// Create an account. The first one on a server is always an owner.
    Add {
        username: String,

        /// owner, admin or member. Defaults to owner for the first account on a
        /// server and member for every one after it.
        #[arg(long)]
        role: Option<String>,

        /// Name to show instead of the username. Unrestricted, unlike the username.
        #[arg(long)]
        display_name: Option<String>,

        /// Read the password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
    /// List accounts.
    List,
    /// Keep the account but refuse its logins.
    Disable { username: String },
    /// Re-enable a disabled account.
    Enable { username: String },
    /// Change an account's role.
    Role { username: String, role: String },
    /// Delete an account, its sessions and its vault grants.
    Delete {
        username: String,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(clap::Args, Debug)]
struct PasswdArgs {
    /// State directory holding auth.db.
    #[arg(long, default_value = "./state")]
    state: PathBuf,

    username: String,

    /// Read the password from stdin instead of prompting.
    #[arg(long)]
    password_stdin: bool,
}

#[derive(clap::Args, Debug)]
struct PairArgs {
    /// State directory holding auth.db and the server identity.
    #[arg(long, default_value = "./state")]
    state: PathBuf,

    /// Address the QR code tells the client to connect to.
    #[arg(long)]
    addr: Option<String>,

    /// Also draw the URI as a scannable QR block.
    ///
    /// Opt-in rather than the default: the block is ~45 columns wide and is
    /// noise when the output is being read by a script, which is how the
    /// deploy scripts and `auth_e2e.py` consume this command.
    #[arg(long)]
    qr: bool,
}

#[derive(clap::Args, Debug)]
struct VaultArgs {
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
}

#[derive(clap::Args, Debug)]
struct ServeArgs {
    #[command(flatten)]
    vault: VaultArgs,

    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    #[arg(long, default_value_t = 8484)]
    port: u16,

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
    // Deliberately no `--relay` here, however much it looks like `--mcp`'s
    // sibling. `Registry::relays` is a setting the app can also change at
    // runtime, and a flag would seed it before `state/vaults.json` exists —
    // exactly the two-authorities bug `--vault-root` already taught us:
    // a root chosen in the app was recorded, then ignored on the next boot,
    // then erased by the next save. A setting that does not survive a
    // restart is not a setting. If relays ever need a boot-time override,
    // it needs the same reconcile-with-the-stored-value treatment `--mcp`
    // gets above, not a bare flag.
}

/// Rewrite a legacy flat-flag invocation into a subcommand.
///
/// For one release, `storm-server --vault-root …` still works by inserting
/// `serve` (or mapping `--dry-run` / `--backup-db`). New scripts should use
/// the subcommands.
fn normalize_argv(args: Vec<OsString>) -> Vec<OsString> {
    if args.len() <= 1 {
        return args;
    }
    let first = args[1].to_string_lossy();
    const KNOWN: &[&str] = &[
        "serve",
        "up",
        "down",
        "status",
        "dry-run",
        "backup-db",
        "user",
        "passwd",
        "pair",
        "help",
        "-h",
        "--help",
        "-V",
        "--version",
    ];
    if KNOWN.iter().any(|k| first == *k) {
        return args;
    }
    if !first.starts_with('-') {
        return args;
    }

    let as_str: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    eprintln!(
        "warning: invoking storm-server without a subcommand is deprecated; \
         use `storm-server serve` (or dry-run / backup-db)"
    );

    if let Some(i) = as_str.iter().position(|a| a == "--backup-db") {
        let dest = as_str.get(i + 1).cloned().unwrap_or_default();
        let mut out: Vec<OsString> = vec![args[0].clone(), "backup-db".into()];
        if !dest.is_empty() && !dest.starts_with('-') {
            out.push(dest.into());
        }
        let mut idx = 1;
        while idx < as_str.len() {
            if as_str[idx] == "--backup-db" {
                idx += 2;
                continue;
            }
            out.push(args[idx].clone());
            idx += 1;
        }
        return out;
    }

    if as_str.iter().any(|a| a == "--dry-run") {
        let mut out: Vec<OsString> = vec![args[0].clone(), "dry-run".into()];
        for (i, s) in as_str.iter().enumerate().skip(1) {
            if s == "--dry-run" {
                continue;
            }
            out.push(args[i].clone());
        }
        return out;
    }

    let mut out = args;
    out.insert(1, "serve".into());
    out
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
fn resolve_root(args: &VaultArgs) -> Result<PathBuf> {
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

/// Snapshots the auth database, the server's keys and every vault's index into
/// `dest`.
fn backup_all(state_dir: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;

    let registry_src = state_dir.join(registry::REGISTRY_FILE);
    if registry_src.exists() {
        std::fs::copy(&registry_src, dest.join(registry::REGISTRY_FILE))
            .with_context(|| format!("copying {}", registry_src.display()))?;
    }

    // Before the vault loop, and before the "no vaults" early return below:
    // this is the one thing in state/ that cannot be rebuilt by rescanning
    // markdown, so a backup that skipped it would restore into a server holding
    // every note with nobody able to log in. A server with no vaults yet still
    // has an identity worth keeping.
    backup_auth(state_dir, dest)?;

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

/// Copies `auth.db` and the private keys it describes into `dest`.
///
/// The database goes through `VACUUM INTO`, like an index, because it runs in
/// WAL mode with the server holding it open — a plain copy can catch committed
/// pages still sitting in the -wal and produce a file that opens and is missing
/// rows.
///
/// The key files are copied as files, which is right for them: they are written
/// once and never modified, so there is no torn state to catch. They have to
/// travel with the database, and this is an extension of the design's backup
/// rule rather than a restatement of it — `auth.db` alone restores a server that
/// knows which key is active and cannot sign with it, which is an identity loss
/// wearing a healthy-looking database.
fn backup_auth(state_dir: &Path, dest: &Path) -> Result<()> {
    let src = auth::AuthDb::path_in(state_dir);
    if !src.exists() {
        return Ok(());
    }
    let db = auth::AuthDb::open_at(&src).context("opening the auth database")?;
    let out = dest.join(auth::AUTH_DB_FILE);
    db.snapshot_to(&out)
        .with_context(|| format!("writing snapshot to {}", out.display()))?;
    println!("  auth -> {}", out.display());

    let keys = state_dir.join(auth::IDENTITY_DIR);
    if !keys.is_dir() {
        return Ok(());
    }
    let keys_out = dest.join(auth::IDENTITY_DIR);
    std::fs::create_dir_all(&keys_out)
        .with_context(|| format!("creating {}", keys_out.display()))?;
    restrict_to_owner(&keys_out)?;

    let mut copied = 0;
    for entry in std::fs::read_dir(&keys).with_context(|| format!("reading {}", keys.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let to = keys_out.join(entry.file_name());
        std::fs::copy(entry.path(), &to)
            .with_context(|| format!("copying {}", entry.path().display()))?;
        // Set explicitly rather than trusting the copy to carry the mode: this
        // is the whole reason the key is a file instead of a row.
        restrict_key_file(&to)?;
        copied += 1;
    }
    println!("  keys -> {} ({copied})", keys_out.display());
    Ok(())
}

fn restrict_to_owner(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("tightening {}", dir.display()))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn restrict_key_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("tightening {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

struct PreparedVaults {
    root: PathBuf,
    state_dir: PathBuf,
    registry: Registry,
    /// No registry file existed when this ran. Recorded here because it can
    /// only be observed *before* the registry loads, and the serve path needs
    /// it afterwards to decide whether to seed the `kit` vault.
    first_run: bool,
}

fn prepare_vaults(args: &VaultArgs) -> Result<PreparedVaults> {
    let root = resolve_root(args)?;
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

    // Observed before the load, because loading is what creates the file this
    // asks about.
    let first_run = kit::is_first_run(&state_dir);

    let mut registry = Registry::load(&state_dir, &root).context("reading the vault registry")?;

    // The registry's root wins, because it is a setting the app can change and
    // one that does not survive a restart is not a setting. The flag seeds the
    // first run. When both are given and disagree, say so rather than silently
    // picking one: that mismatch used to end with the app's choice erased and
    // every vault adopted under it left registered as `missing`.
    let asked_for_root = args.vault_root.is_some() || args.vault.is_some();
    if asked_for_root && registry.root != root {
        tracing::warn!(
            stored = %registry.root.display(),
            requested = %root.display(),
            "the stored storage root differs from the one on the command line; \
             using the stored one. Change it in Storm's server settings, or edit \
             state/vaults.json — Storm never moves vault directories."
        );
    }
    let root = registry.root.clone();

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

    Ok(PreparedVaults {
        root,
        state_dir,
        registry,
        first_run,
    })
}

fn run_dry_run(args: VaultArgs) -> Result<()> {
    let prepared = prepare_vaults(&args)?;
    println!("Dry run — nothing was written.\n");
    println!("  storage root : {}", prepared.root.display());
    println!("  vaults       : {}\n", prepared.registry.vaults.len());
    for entry in &prepared.registry.vaults {
        let vault = vault::Vault::new(prepared.registry.path_of(entry))?;
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
    println!("\nRun again without dry-run to apply.");
    Ok(())
}

/// Opens `auth.db` for a CLI command, warning about the ownership trap.
///
/// The database is created on first use, so `sudo storm-server user add` on a
/// server that runs as `storm` leaves a root-owned `auth.db` the service cannot
/// write. That surfaces much later as a login that fails for no visible reason,
/// so it is worth a line here.
fn open_auth_db(state_dir: &Path) -> Result<auth::AuthDb> {
    let existed = auth::AuthDb::path_in(state_dir).exists();
    let db = auth::AuthDb::open(state_dir)
        .with_context(|| format!("opening the auth database in {}", state_dir.display()))?;
    if !existed {
        println!("created {}", auth::AuthDb::path_in(state_dir).display());
    }
    warn_if_owner_mismatch(state_dir);
    Ok(db)
}

#[cfg(unix)]
fn warn_if_owner_mismatch(state_dir: &Path) {
    use std::os::unix::fs::MetadataExt;
    let path = auth::AuthDb::path_in(state_dir);
    let (Ok(dir), Ok(file)) = (std::fs::metadata(state_dir), std::fs::metadata(&path)) else {
        return;
    };
    if dir.uid() != file.uid() {
        eprintln!(
            "warning: {} is owned by uid {} but its directory by uid {}. The server \
             may not be able to write it — run this as the service user \
             (`sudo -u storm storm-server …`) or chown it back.",
            path.display(),
            file.uid(),
            dir.uid()
        );
    }
}

#[cfg(not(unix))]
fn warn_if_owner_mismatch(_state_dir: &Path) {}

/// Reads a new password, without echoing it.
///
/// Prompted twice, because there is no "forgot password" flow behind this — a
/// typo in the only copy of a password is an account nobody can reach until an
/// operator runs `passwd`. `--password-stdin` is the scriptable path.
fn read_new_password(from_stdin: bool, prompt: &str) -> Result<String> {
    let password = if from_stdin {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading the password from stdin")?;
        // Only the line ending is stripped. Trimming whitespace generally would
        // silently change a password that legitimately ends in a space, and the
        // account would then refuse the password its owner thinks they set.
        buf.trim_end_matches(['\n', '\r']).to_string()
    } else {
        let first = rpassword::prompt_password(prompt).context("reading the password")?;
        let again = rpassword::prompt_password("Repeat: ").context("reading the password")?;
        if first != again {
            bail!("the two passwords do not match");
        }
        first
    };

    if let Err(why) = auth::password::validate_password(&password) {
        bail!(why);
    }
    Ok(password)
}

/// Reads the stored hash back and checks the password against it.
///
/// A password that was written but does not verify is an account nobody can
/// reach, and the person who discovers it is the user at a login prompt some
/// weeks later. This costs one extra verify on a rare interactive command, and
/// it exercises the same hash-then-verify path login will use.
async fn confirm_stored_password(
    db: &auth::AuthDb,
    hasher: &auth::Hasher,
    user_id: &str,
    password: String,
) -> Result<()> {
    let stored = db
        .password_hash_of(user_id)?
        .context("the account has no password hash immediately after one was written")?;
    if !hasher.verify(password, stored).await? {
        bail!(
            "the password was stored but does not verify against what was written. \
             Do not rely on this account; auth.db may be damaged."
        );
    }
    Ok(())
}

async fn run_user(args: UserArgs) -> Result<()> {
    let mut db = open_auth_db(&args.state)?;
    let now = index::now_rfc3339();

    match args.command {
        UserCommand::Add {
            username,
            role,
            display_name,
            password_stdin,
        } => {
            let role = match role.as_deref() {
                Some(name) => auth::users::Role::parse(name)?,
                // The first account must be an owner and `create_user` enforces
                // it; defaulting the rest to member keeps "add a user" from
                // quietly minting another administrator.
                None if db.user_count()? == 0 => auth::users::Role::Owner,
                None => auth::users::Role::Member,
            };
            // Validate the name before asking for a password: being told the
            // username is malformed after typing a password twice is a small
            // cruelty, and `create_user` checks it again anyway.
            if let Err(why) = auth::users::validate_username(&username) {
                bail!(why);
            }

            let password = read_new_password(password_stdin, "New password: ")?;
            let hasher = auth::Hasher::new();
            let hash = hasher.hash(password.clone()).await?;
            let user = auth::users::create_user(
                &mut db,
                auth::users::NewUser {
                    username: &username,
                    display_name: display_name.as_deref(),
                    password_hash: &hash,
                    role,
                },
                &now,
            )?;
            confirm_stored_password(&db, &hasher, &user.id, password).await?;
            println!(
                "created {} ({}) as {}",
                user.username,
                user.id,
                role.as_str()
            );
        }

        UserCommand::List => {
            let users = db.list_users()?;
            if users.is_empty() {
                println!(
                    "no users yet — `storm-server user add <name>` creates the first, \
                     which is always an owner"
                );
                return Ok(());
            }
            println!(
                "{:<20} {:<7} {:<9} {:<22} {:<22} PASSWORD",
                "USERNAME", "ROLE", "STATUS", "CREATED", "LAST LOGIN"
            );
            let mut outdated = 0;
            for user in &users {
                let password = match db.password_hash_of(&user.id)? {
                    Some(phc) if auth::password::needs_rehash(&phc) => {
                        outdated += 1;
                        "outdated"
                    }
                    Some(_) => "current",
                    None => "missing",
                };
                println!(
                    "{:<20} {:<7} {:<9} {:<22} {:<22} {}",
                    user.username,
                    user.role.as_str(),
                    user.status.as_str(),
                    user.created,
                    user.last_login.as_deref().unwrap_or("never"),
                    password
                );
            }
            println!(
                "\n{} user(s), {} active owner(s)",
                users.len(),
                db.active_owner_count()?
            );
            if outdated > 0 {
                println!(
                    "{outdated} password(s) hashed with weaker parameters than this build uses; \
                     they are upgraded on next login."
                );
            }
        }

        UserCommand::Disable { username } => {
            let user =
                auth::users::set_status(&mut db, &username, auth::users::Status::Disabled, &now)?;
            println!("disabled {}", user.username);
        }

        UserCommand::Enable { username } => {
            let user =
                auth::users::set_status(&mut db, &username, auth::users::Status::Active, &now)?;
            println!("enabled {}", user.username);
        }

        UserCommand::Role { username, role } => {
            let role = auth::users::Role::parse(&role)?;
            let user = auth::users::set_role(&mut db, &username, role, &now)?;
            println!("{} is now {}", user.username, role.as_str());
        }

        UserCommand::Delete { username, yes } => {
            if !yes {
                // There is no undo, and the delete takes sessions and vault
                // grants with it. Typing the name is cheap insurance against a
                // mistyped argument.
                print!(
                    "Delete `{username}`, its sessions and its vault grants? Type the username to confirm: "
                );
                use std::io::Write;
                std::io::stdout().flush().ok();
                let mut line = String::new();
                std::io::stdin()
                    .read_line(&mut line)
                    .context("reading the confirmation")?;
                if line.trim() != username {
                    bail!("not confirmed; nothing was deleted");
                }
            }
            let user = auth::users::delete_user(&mut db, &username, &now)?;
            println!("deleted {} ({})", user.username, user.id);
        }
    }
    Ok(())
}

/// `storm-server passwd` — the recovery path (A11).
///
/// Deliberately a host-side bypass: root on the box can already read `auth.db`
/// and every vault, so pretending a password reset needs more than shell access
/// would be theatre. It writes a security event so the reset is visible.
async fn run_passwd(args: PasswdArgs) -> Result<()> {
    let mut db = open_auth_db(&args.state)?;
    let now = index::now_rfc3339();

    if db.find_user(&args.username)?.is_none() {
        bail!(
            "no user named `{}` — `storm-server user list` shows the accounts on this server",
            args.username
        );
    }

    let password = read_new_password(
        args.password_stdin,
        &format!("New password for {}: ", args.username),
    )?;
    let hasher = auth::Hasher::new();
    let hash = hasher.hash(password.clone()).await?;
    let user = auth::users::set_password(&mut db, &args.username, &hash, &now)?;
    confirm_stored_password(&db, &hasher, &user.id, password).await?;
    println!("password updated for {} ({})", user.username, user.id);
    Ok(())
}

/// `storm-server pair` — print a bootstrap pairing QR code.
///
/// Only valid when the user table is empty (fresh server). The QR encodes the
/// server's public key, a short-lived nonce, and the address the client should
/// connect to. Scanning it with Storm Client triggers device registration and
/// first-user creation.
fn run_pair(args: PairArgs) -> Result<()> {
    let mut db = open_auth_db(&args.state)?;
    let now = index::now_rfc3339();

    let user_count = db.count_users()?;
    if user_count > 0 {
        bail!(
            "users already exist — use `POST /v1/pairings` from an authenticated client \
             to add a new device"
        );
    }

    // Load the server identity. It must exist by the time `pair` is called —
    // `serve` creates it, and `pair` only makes sense on a server that has
    // booted at least once.
    let mut auth_db_check = auth::AuthDb::open(&args.state)
        .with_context(|| format!("opening the auth database in {}", args.state.display()))?;
    let identity = auth::identity::load_or_create(&mut auth_db_check, &args.state, &now)
        .context("loading server identity — has the server booted at least once?")?;

    let addr = args.addr.unwrap_or_else(|| {
        // The client dials whatever lands in this field, so a loopback guess
        // is unusable from the one device that matters — a phone, where
        // 127.0.0.1 is the phone itself. Default to an address on this box
        // that something else can actually reach, and say plainly that the
        // port is still a guess.
        let host = advertised_host("0.0.0.0");
        eprintln!(
            "no --addr given; using {host}:8484. If the server listens on \
             another port or interface, re-run with --addr host:port — the \
             client dials this exactly as written."
        );
        format!("{host}:8484")
    });

    let (nonce, session) = auth::pairing::create(
        &mut db,
        auth::pairing::PairingPurpose::FirstUser,
        None,
        // Unbound: a QR is carried across the room to another device, which
        // is the opposite of the web nonce's one-peer rule.
        None,
        &now,
    )
    .context("creating pairing session")?;

    let qr = auth::pairing::encode_qr(
        &identity.server_id,
        &identity.public_key_b64(),
        &nonce,
        &session.expires,
        &addr,
    );

    if args.qr {
        // The URI is still printed below. A terminal can be too narrow for the
        // block, the colours can be inverted, and pasting has to keep working
        // when it is — so the QR is an addition, never a replacement.
        match render_qr(&qr.to_uri()) {
            Ok(block) => println!("\n{block}"),
            Err(e) => eprintln!("could not render the QR ({e}); the URI follows"),
        }
    }

    println!("\n  Pairing URI — scan the code above, or paste this into Storm:\n");
    println!("    {}\n", qr.to_uri());
    println!("  Expires: {}", session.expires);
    println!("  Session: {}\n", session.id);
    Ok(())
}

async fn run_serve(args: ServeArgs) -> Result<()> {
    let prepared = prepare_vaults(&args.vault)?;
    let mut registry = prepared.registry;
    let state_dir = prepared.state_dir;
    let root = prepared.root;

    // The `kit` vault carries the agent tooling every Storm server is expected
    // to have, so a first boot creates it rather than leaving it as a setup
    // step. Only on a first boot: deleting it afterwards is a decision, and
    // restoring it on every start would override that decision silently.
    if prepared.first_run {
        match kit::seed(&mut registry, &index::now_rfc3339()) {
            Ok(true) => tracing::info!(vault = kit::VAULT_NAME, "seeded the agent kit vault"),
            Ok(false) => {}
            // A server that cannot write one vault should still serve the
            // others. Loud, not fatal.
            Err(e) => tracing::warn!(error = %e, "could not seed the kit vault"),
        }
    }

    registry.save(&state_dir).context("saving the registry")?;
    let mut vault_set = api::open_vaults(&registry, &state_dir).context("opening vaults")?;

    // Identity before anything is served. A first boot mints it; every boot
    // after reads it back, and a mismatch between the recorded key and the file
    // is a hard failure rather than a quietly regenerated keypair.
    //
    // The database handle stays open in `AppState` so request handlers can
    // authenticate sessions and mint WS tickets without racing on open/close.
    let mut auth_db = auth::AuthDb::open(&state_dir).context("opening the auth database")?;
    let identity = Arc::new(auth::identity::load_or_create(
        &mut auth_db,
        &state_dir,
        &index::now_rfc3339(),
    )?);
    tracing::info!(
        server_id = %identity.server_id,
        name = %identity.name,
        key_id = %identity.key_id,
        "server identity"
    );

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
    // Observability only — no tunnel client exists yet, so `registered` is
    // always 0 today. Not a second authority: nothing here is read back.
    tracing::info!(
        configured = vault_set.registry.relays.len(),
        registered = vault_set.registry.registered_relays.snapshot().len(),
        "relay configuration"
    );

    // The A10 migration switch, read from the registry rather than assumed.
    // Absent from an older registry it loads as `true`, because that registry
    // belongs to a server whose clients all hold the shared token.

    // **The QR's `addr` is where a client should dial, not where we bind.**
    //
    // A server told `--host 0.0.0.0` (which is every real deployment, and what
    // the systemd unit passes) would otherwise advertise `0.0.0.0:8484` in the
    // pairing URI. The client takes that field as the base URL it will use for
    // every subsequent call, so pairing from a phone failed at the first
    // request: a wildcard bind is not an address anything can connect to.
    //
    // Found by pairing a real phone against a real server — the local suites
    // and the client's own live tests all dial an address they were handed by
    // the harness, so none of them ever read this field.
    let listen_addr = format!("{}:{}", advertised_host(&args.host), args.port);

    // Mirrored out of the registry so a change applies to the next request
    // rather than the next restart (A13).
    let vault_set_allow_registration = vault_set.registry.allow_registration;

    // Taken before the registry moves into `AppState`. The configured list is
    // a snapshot: relays added through `PUT /v1/config/relays` are picked up on
    // the next restart, not live. `registered_relays` is a shared handle, so
    // what the supervisors record below is what `/v1/server` reports.
    let configured_relays = vault_set.registry.relays.clone();
    let registered_relays = vault_set.registry.registered_relays.clone();
    let tunnel_identity = identity.clone();

    // Bootstrap pairing: when no users exist, create a pairing session and log
    // the QR URI so the operator can scan it with a Storm Client.
    let bootstrap_nonce = {
        let now = crate::index::now_rfc3339();
        let user_count = auth_db.count_users()?;
        if user_count == 0 {
            let (nonce, session) = crate::auth::pairing::create(
                &mut auth_db,
                crate::auth::pairing::PairingPurpose::FirstUser,
                None,
                None,
                &now,
            )
            .context("creating bootstrap pairing session")?;
            let qr = crate::auth::pairing::encode_qr(
                &identity.server_id,
                &identity.public_key_b64(),
                &nonce,
                &session.expires,
                &listen_addr,
            );
            tracing::info!(
                uri = %qr.to_uri(),
                "bootstrap pairing QR — scan with Storm Client to create the first user"
            );
            Some(nonce)
        } else {
            None
        }
    };

    let state = Arc::new(AppState {
        vaults: RwLock::new(vault_set),
        events,
        state_dir: state_dir.clone(),
        identity,
        root_changed,
        mcp_enabled: std::sync::atomic::AtomicBool::new(mcp_enabled),
        mcp_writable: std::sync::atomic::AtomicBool::new(mcp_writable),
        auth_db: Arc::new(tokio::sync::Mutex::new(auth_db)),
        allow_registration: std::sync::atomic::AtomicBool::new(vault_set_allow_registration),
        bootstrap_nonce,
        listen_addr,
        // The policy Storm ships: every authenticated caller reaches every
        // vault, which is what the server already did. The boundary is what
        // is new — see `auth/authz.rs`.
        vault_policy: Arc::new(crate::auth::authz::AllowAuthenticated),
        // One hasher for the process, so the semaphore actually bounds
        // anything. See the field's documentation in `api.rs`.
        hasher: auth::Hasher::new(),
        // Same: one limiter for the process, or the limits do not exist.
        login_limiter: auth::ratelimit::LoginLimiter::new(),
    });

    // One watcher over the whole root, attributing each event to a vault by
    // directory prefix. Adding or removing a vault then needs no watcher work
    // at all, and a root change respawns this one.
    watcher::spawn(root.clone(), state.clone()).context("starting file watcher")?;
    tracing::info!(path = %root.display(), "watching the storage root for external edits");

    // Kept for the web fallback below, which mints bootstrap nonces and so
    // needs the same state the API has.
    let web_state = state.clone();

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
        let assets = ServeDir::new(web_dir).fallback(ServeFile::new(index.clone()));

        // **A path that looks like a file is an asset; everything else is a
        // client route and gets the document.** The document is the only thing
        // that carries a bootstrap nonce, and it has to carry one on a deep
        // link (`/login`) as well as on `/`. Deciding by shape up front beats
        // asking `ServeDir` afterwards whether what it returned happened to be
        // its index fallback — a question its response cannot answer.
        //
        // `fallback` rather than `not_found_service` for the asset service:
        // the latter serves index.html's body but keeps ServeDir's 404 status,
        // which breaks caching and the Flutter service worker on any deep link.
        app = app
            .fallback(move |req: axum::extract::Request| {
                let state = web_state.clone();
                let index = index.clone();
                let assets = assets.clone();
                async move {
                    let looks_like_a_file = req
                        .uri()
                        .path()
                        .rsplit('/')
                        .next()
                        .is_some_and(|last| last.contains('.'));
                    if looks_like_a_file {
                        use tower::ServiceExt;
                        let mut response = assets
                            .oneshot(req)
                            .await
                            .map(axum::response::IntoResponse::into_response)
                            .unwrap_or_else(|e| match e {});
                        // **`no-cache`, meaning revalidate — not "do not
                        // store".** Flutter's build output keeps stable
                        // filenames (`main.dart.js`, `flutter_bootstrap.js`)
                        // across builds, so there is no content hash to make a
                        // long `max-age` safe. With no header at all, browsers
                        // fall back to *heuristic* caching and reuse the old
                        // bundle without even asking — which is how a deployed
                        // release quietly failed to reach a returning browser,
                        // twice in one afternoon.
                        //
                        // `ServeDir` already sends an ETag, so revalidating
                        // costs a conditional request and a bodyless `304`.
                        response.headers_mut().insert(
                            axum::http::header::CACHE_CONTROL,
                            HeaderValue::from_static("no-cache"),
                        );
                        return response;
                    }
                    let peer = req
                        .extensions()
                        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                        .map(|c| c.0);
                    let headers = req.headers().clone();
                    match peer {
                        Some(peer) => api::serve_web_index(state, index, peer, headers).await,
                        // No peer means no binding is possible, and an unbound
                        // web nonce is the thing this design refuses to mint.
                        None => api::serve_web_index_without_bootstrap(index).await,
                    }
                }
            })
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

    // **The tunnel serves the same `app` the listener does**, cloned rather
    // than rebuilt: same routes, same auth layers, same web fallback. A second
    // router assembled for relayed traffic is how the two would drift, and R13
    // is precisely the claim that they cannot.
    //
    // Spawned before `serve` and never awaited on the request path — a relay
    // that is unreachable, slow or hung costs a background task and nothing
    // else. `bind_host` is `args.host`, matching what `mcp::allowed_hosts` was
    // built from above.
    let tunnels = relay::Tunnels::spawn(
        &configured_relays,
        tunnel_identity,
        app.clone(),
        registered_relays,
        &addr,
    );
    if !configured_relays.is_empty() {
        tracing::info!(relays = configured_relays.len(), "connecting to relays");
    }

    tracing::info!("storm-server listening on http://{addr}");
    // `into_make_service_with_connect_info` is what makes the peer address
    // available to handlers. `/v1/pair/local` refuses anything that is not
    // loopback, and without this it would have no way to tell.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    // Graceful shutdown exists so the tunnels below get a chance to run.
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
    })
    .await
    .context("serving")?;

    // `DEREGISTER`, and wait for it to go out. Without this the relay holds the
    // `server_id` until a heartbeat timeout and every client trying to reach
    // this server waits that out — a restart would look like an outage for as
    // long as the timeout lasts.
    tunnels.shutdown().await;
    Ok(())
}

/// Draws a string as a QR block for a terminal.
///
/// Two rows of modules per line of text via the half-block character, because
/// a terminal cell is about twice as tall as it is wide — one module per line
/// produces a stretched code that phones read poorly, and takes twice the
/// screen. **Dark modules are drawn light and light modules dark**: this is
/// printed on a dark terminal, and a scanner needs the *quiet zone and light
/// modules* to be the bright ones. On a light terminal it reads inverted,
/// which most phone cameras handle.
fn render_qr(data: &str) -> Result<String> {
    use qrcode::{EcLevel, QrCode};

    // Low correction on purpose: the URI is ~200 bytes, and a higher level
    // makes the code denser rather than more readable at this size.
    let code = QrCode::with_error_correction_level(data, EcLevel::L)
        .context("encoding the pairing URI as a QR code")?;
    let modules = code.to_colors();
    let width = code.width();

    // Four modules of quiet zone on every side; a code flush against other
    // output is one a scanner will not find.
    const QUIET: usize = 4;
    let dark = |x: usize, y: usize| -> bool {
        if x < QUIET || y < QUIET || x >= width + QUIET || y >= width + QUIET {
            return false;
        }
        modules[(y - QUIET) * width + (x - QUIET)] == qrcode::Color::Dark
    };

    let total = width + QUIET * 2;
    let mut out = String::new();
    for row in (0..total).step_by(2) {
        for x in 0..total {
            let top = dark(x, row);
            let bottom = if row + 1 < total {
                dark(x, row + 1)
            } else {
                false
            };
            // Inverted, per the note above: a "dark" module prints as an unlit
            // half so the light ones carry the brightness.
            out.push(match (top, bottom) {
                (true, true) => ' ',
                (true, false) => '▄',
                (false, true) => '▀',
                (false, false) => '█',
            });
        }
        out.push('\n');
    }
    Ok(out)
}

/// The host a *client* should dial, given the host we were told to bind.
///
/// A wildcard bind (`0.0.0.0`, `::`, or empty) means "every interface", which
/// is not somewhere anything can connect to. Anything else was chosen
/// deliberately by the operator and is passed through untouched — including
/// `127.0.0.1`, which is correct for a client on the same machine and is what
/// the test harnesses use.
///
/// Resolved by asking the OS which local address it would use to reach a
/// public one. No packet is sent: a UDP socket's `connect` only sets the peer
/// and picks the route, so this works with no network and no DNS.
fn advertised_host(bind_host: &str) -> String {
    if !matches!(bind_host, "0.0.0.0" | "::" | "[::]" | "") {
        return bind_host.to_string();
    }
    let routable = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("203.0.113.1:80")?; // TEST-NET-3; never actually reached
            s.local_addr()
        })
        .map(|a| a.ip().to_string());

    match routable {
        Ok(ip) => ip,
        Err(e) => {
            // Better a hostname the operator can fix than a wildcard that
            // silently breaks every pairing.
            tracing::warn!(
                error = %e,
                "could not determine a routable address for the pairing QR; \
                 falling back to the bind host"
            );
            bind_host.to_string()
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "storm_server=info,tower_http=warn".into()),
        )
        .init();

    let cli = Cli::parse_from(normalize_argv(std::env::args_os().collect()));

    match cli.command {
        Commands::Serve(args) => run_serve(args).await,
        Commands::Up(args) => {
            let vault_root = args
                .vault_root
                .unwrap_or_else(|| args.data_root.join("vaults"));
            let state = args.state.unwrap_or_else(|| args.data_root.join("state"));
            let backups = args.data_root.join("backups");
            // ReadWritePaths must cover every path the service writes. When
            // vaults and state are split (NAS vaults + local state), widen to
            // both parents via the data-root drop-in's primary path plus an
            // extra line — `up` passes the data_root for the drop-in and the
            // concrete vault/state paths for the env file.
            install::up(install::UpOptions {
                data_root: args.data_root,
                vault_root,
                state,
                backups,
                host: args.host,
                port: args.port,
                web: args.web,
            })
        }
        Commands::Down => install::down(),
        Commands::Status => install::status(),
        Commands::DryRun(args) => run_dry_run(args),
        Commands::User(args) => run_user(args).await,
        Commands::Passwd(args) => run_passwd(args).await,
        Commands::Pair(args) => run_pair(args),
        Commands::BackupDb { state, dest } => {
            backup_all(&state, &dest)?;
            println!("index snapshots written to {}", dest.display());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_stored_password_that_does_not_verify_is_caught() {
        // The guard on `user add` and `passwd`: it reads the hash back out of
        // the database and checks it, so a row that was written but cannot be
        // logged into is reported now rather than at a login prompt weeks later.
        let dir = tempdir::TempDir::new("storm-confirm").unwrap();
        let mut db = auth::AuthDb::open(dir.path()).unwrap();
        let hasher = auth::Hasher::new();
        let hash = hasher.hash("correct horse battery".into()).await.unwrap();
        let user = auth::users::create_user(
            &mut db,
            auth::users::NewUser {
                username: "dewansh",
                display_name: None,
                password_hash: &hash,
                role: auth::users::Role::Owner,
            },
            "2026-08-16T00:00:00Z",
        )
        .unwrap();

        confirm_stored_password(&db, &hasher, &user.id, "correct horse battery".into())
            .await
            .expect("the password just written must verify");

        let err = confirm_stored_password(&db, &hasher, &user.id, "a different password".into())
            .await
            .expect_err("a password that does not match must be reported");
        assert!(err.to_string().contains("does not verify"), "{err}");
    }

    fn vault_args(vault_root: Option<PathBuf>, vault: Option<PathBuf>) -> VaultArgs {
        VaultArgs {
            vault_root,
            vault,
            state: PathBuf::from("./state"),
        }
    }

    #[test]
    fn vault_root_is_used_as_given() {
        let dir = tempdir::TempDir::new("storm-root").unwrap();
        let root = dir.path().join("vaults");
        std::fs::create_dir_all(&root).unwrap();
        let resolved = resolve_root(&vault_args(Some(root.clone()), None)).unwrap();
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

        let resolved = resolve_root(&vault_args(None, Some(vault.clone()))).unwrap();
        assert_eq!(resolved, dir.path().canonicalize().unwrap());
        assert_ne!(resolved, vault, "the vault itself is never the root");
    }

    #[test]
    fn a_missing_vault_path_fails_loudly_with_both_paths() {
        // A skipped `mv` in the runbook must read as a clear failure, not as
        // a healthy server with zero vaults.
        let dir = tempdir::TempDir::new("storm-missing").unwrap();
        let vault = dir.path().join("not-here");
        let err = resolve_root(&vault_args(None, Some(vault.clone())))
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
        assert!(resolve_root(&vault_args(None, Some(file))).is_err());
    }

    #[test]
    fn both_flags_together_are_an_error_not_a_precedence_rule() {
        let dir = tempdir::TempDir::new("storm-both").unwrap();
        std::fs::create_dir_all(dir.path().join("vault")).unwrap();
        assert!(
            resolve_root(&vault_args(
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

    /// A state directory with an identity and one indexed vault.
    fn seeded_state(dir: &Path) -> (PathBuf, auth::ServerIdentity) {
        let root = dir.join("vaults");
        let state = dir.join("state");
        std::fs::create_dir_all(root.join("personal")).unwrap();
        std::fs::create_dir_all(&state).unwrap();

        let mut registry = Registry::load(&state, &root).unwrap();
        registry.scan_root(&state, "2026-08-13T00:00:00Z").unwrap();
        registry.save(&state).unwrap();
        let id = &registry.vaults[0].id;
        Db::open(&state.join(id).join("index.db"), id).unwrap();

        let mut auth_db = auth::AuthDb::open(&state).unwrap();
        let identity =
            auth::identity::load_or_create(&mut auth_db, &state, "2026-08-13T00:00:00Z").unwrap();
        (state, identity)
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &dest);
            } else {
                std::fs::copy(entry.path(), &dest).unwrap();
            }
        }
    }

    #[test]
    fn a_backup_carries_the_auth_database_and_its_keys() {
        let dir = tempdir::TempDir::new("storm-backup-auth").unwrap();
        let (state, identity) = seeded_state(dir.path());
        let dest = dir.path().join("snapshot");
        backup_all(&state, &dest).unwrap();

        assert!(
            dest.join(auth::AUTH_DB_FILE).exists(),
            "auth.db is not in the snapshot — a restore would have every note \
             and nobody able to log in"
        );
        let key = auth::identity::key_path(&dest, &identity.key_id);
        assert!(
            key.exists(),
            "the private key is not in the snapshot; auth.db alone restores a \
             server that knows its key and cannot sign with it"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the backed-up key is {mode:o}");
        }
    }

    #[test]
    fn a_server_with_no_vaults_still_backs_up_its_identity() {
        // `backup_all` returns early when the registry is empty. The identity
        // is not a vault's, so that return must not skip it.
        let dir = tempdir::TempDir::new("storm-backup-empty").unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let mut auth_db = auth::AuthDb::open(&state).unwrap();
        let identity =
            auth::identity::load_or_create(&mut auth_db, &state, "2026-08-13T00:00:00Z").unwrap();

        let dest = dir.path().join("snapshot");
        backup_all(&state, &dest).unwrap();
        assert!(dest.join(auth::AUTH_DB_FILE).exists());
        assert!(auth::identity::key_path(&dest, &identity.key_id).exists());
    }

    #[test]
    fn backup_wipe_restore_gives_back_a_server_that_can_still_sign() {
        // The cycle that matters, end to end: file existence proves nothing —
        // what has to survive is the ability to answer a challenge with the
        // same key clients pinned.
        let dir = tempdir::TempDir::new("storm-restore").unwrap();
        let (state, before) = seeded_state(dir.path());
        let dest = dir.path().join("snapshot");
        backup_all(&state, &dest).unwrap();

        let nonce = "0123456789abcdef0123";
        let signature = before.sign_challenge(nonce);

        std::fs::remove_dir_all(&state).unwrap();
        assert!(!state.exists());
        copy_tree(&dest, &state);

        let mut auth_db = auth::AuthDb::open(&state).unwrap();
        let after =
            auth::identity::load_or_create(&mut auth_db, &state, "2026-09-01T00:00:00Z").unwrap();

        assert_eq!(before.server_id, after.server_id, "the identity changed");
        assert_eq!(before.key_id, after.key_id);
        assert_eq!(before.public_key_b64(), after.public_key_b64());
        assert_eq!(
            after.sign_challenge(nonce),
            signature,
            "the restored server signs differently — every pinned client would \
             have to re-pair"
        );
    }

    #[tokio::test]
    async fn a_restore_gives_back_users_who_can_still_log_in() {
        // The failure the data model names: a restore that carries the notes and
        // not `auth.db` is a server holding every note that nobody can log into.
        // Now that accounts exist, "the users came back" means their passwords
        // still verify — not that a row is present.
        let dir = tempdir::TempDir::new("storm-restore-users").unwrap();
        let (state, _identity) = seeded_state(dir.path());

        let hasher = auth::Hasher::new();
        let hash = hasher.hash("correct horse battery".into()).await.unwrap();
        {
            let mut db = auth::AuthDb::open(&state).unwrap();
            auth::users::create_user(
                &mut db,
                auth::users::NewUser {
                    username: "dewansh",
                    display_name: None,
                    password_hash: &hash,
                    role: auth::users::Role::Owner,
                },
                "2026-08-16T00:00:00Z",
            )
            .unwrap();
        }

        let dest = dir.path().join("snapshot");
        backup_all(&state, &dest).unwrap();
        std::fs::remove_dir_all(&state).unwrap();
        copy_tree(&dest, &state);

        let db = auth::AuthDb::open(&state).unwrap();
        let user = db
            .find_user("dewansh")
            .unwrap()
            .expect("the restored server has no users");
        assert_eq!(user.role, auth::users::Role::Owner);
        let stored = db.password_hash_of(&user.id).unwrap().unwrap();
        assert!(
            hasher
                .verify("correct horse battery".into(), stored)
                .await
                .unwrap(),
            "the restored account exists but its password does not verify"
        );
    }

    #[test]
    fn normalize_argv_inserts_serve_for_legacy_flags() {
        let out = normalize_argv(vec![
            "storm-server".into(),
            "--vault-root".into(),
            "/tmp/v".into(),
        ]);
        assert_eq!(out[1], "serve");
        assert_eq!(out[2], "--vault-root");
    }

    #[test]
    fn normalize_argv_rewrites_dry_run() {
        let out = normalize_argv(vec![
            "storm-server".into(),
            "--vault-root".into(),
            "/tmp/v".into(),
            "--dry-run".into(),
        ]);
        assert_eq!(out[1], "dry-run");
        assert!(!out.iter().any(|a| a == "--dry-run"));
    }

    #[test]
    fn normalize_argv_rewrites_backup_db() {
        let out = normalize_argv(vec![
            "storm-server".into(),
            "--state".into(),
            "/tmp/s".into(),
            "--backup-db".into(),
            "/tmp/out".into(),
        ]);
        assert_eq!(out[1], "backup-db");
        assert_eq!(out[2], "/tmp/out");
        assert!(!out.iter().any(|a| a == "--backup-db"));
    }

    #[test]
    fn a_wildcard_bind_is_never_advertised_to_a_client() {
        // The pairing QR's `addr` is the base URL the client will use for
        // every call after pairing. `0.0.0.0` is where we listen, not
        // somewhere anything can connect to — a phone handed that fails on its
        // first request, which is exactly how this was found.
        for wildcard in ["0.0.0.0", "::", "[::]", ""] {
            let advertised = advertised_host(wildcard);
            assert_ne!(
                advertised, wildcard,
                "a client cannot dial the wildcard {wildcard:?}"
            );
            assert!(!advertised.is_empty());
        }
    }

    #[test]
    fn a_deliberate_bind_host_is_passed_through() {
        // An operator who says 127.0.0.1 means it — that is right for a client
        // on the same box, and it is what the test harnesses dial. Substituting
        // a LAN address here would break every local suite.
        for chosen in ["127.0.0.1", "192.168.1.10", "storm.example.com"] {
            assert_eq!(advertised_host(chosen), chosen);
        }
    }
}
