//! Installing, updating and removing agent CLIs (decision 133-1, 133-2).
//!
//! **A recipe is compiled in.** The server names an agent and an operation;
//! where the CLI comes from, how its download is checked and where it lands
//! is decided here, so nothing on the wire can make a host fetch software it
//! was not built to fetch. Storm never pipes a vendor's install script into a
//! shell: each writes outside any folder Storm owns, and changes without
//! notice.
//!
//! - **Claude Code**: the release channel's `manifest.json` gives a SHA-256
//!   and a size per platform.
//! - **OpenCode**: the version from npm's `opencode-ai`, the archive from its
//!   GitHub release, checked against the digest GitHub's API reports.
//! - **Cursor**: the version its own installer names; Cursor publishes no
//!   checksum, which the app says on its row.
//! - **Codex, Command Code**: `npm install` into Storm's own prefix; npm
//!   checks each package's `integrity`.
//!
//! **A checksum mismatch never installs.** Everything lands under
//! `<state>/agents/managed` ([`crate::agents::managed_dir`]); a release is
//! activated by swapping `bin/<command>` atomically, so a running session
//! keeps the binary it started and the next one gets the new one, and the
//! previous release is kept for one rollback.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What the server may ask for (133-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Install,
    Update,
    Remove,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Remove => "remove",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Install, Self::Update, Self::Remove]
            .into_iter()
            .find(|op| op.name() == name)
    }
}

/// Why a job failed: the stable codes the app shows (133-4). npm's output
/// never crosses the wire (it can hold registry URLs with tokens), and
/// neither does a URL; the host's own log keeps the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Unknown,
    Blocked(&'static str),
    DownloadFailed,
    ChecksumMismatch,
    UnpackFailed,
    NpmFailed,
    VersionProbeFailed,
    DiskFull,
    NotManaged,
    InUse,
    Busy,
    Timeout,
}

impl Failure {
    /// Back from a code an install job printed (136g): a known one, or
    /// `job_failed`.
    pub fn from_code(code: &str) -> Self {
        const BLOCKED: [&str; 4] = [
            "disabled",
            "unsupported_platform",
            "no_node",
            "node_too_old",
        ];
        if let Some(reason) = BLOCKED.iter().find(|r| **r == code) {
            return Self::Blocked(reason);
        }
        [
            Self::Unknown,
            Self::DownloadFailed,
            Self::ChecksumMismatch,
            Self::UnpackFailed,
            Self::NpmFailed,
            Self::VersionProbeFailed,
            Self::DiskFull,
            Self::NotManaged,
            Self::InUse,
            Self::Busy,
            Self::Timeout,
        ]
        .into_iter()
        .find(|f| f.code() == code)
        .unwrap_or(Self::Blocked("job_failed"))
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Unknown => "unknown_agent",
            Self::Blocked(reason) => reason,
            Self::DownloadFailed => "download_failed",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::UnpackFailed => "unpack_failed",
            Self::NpmFailed => "npm_failed",
            Self::VersionProbeFailed => "version_probe_failed",
            Self::DiskFull => "disk_full",
            Self::NotManaged => "not_managed",
            Self::InUse => "in_use",
            Self::Busy => "busy",
            Self::Timeout => "timeout",
        }
    }
}

fn io_failure(e: &std::io::Error, otherwise: Failure) -> Failure {
    // ENOSPC: say so rather than "unpack failed".
    if e.raw_os_error() == Some(28) {
        Failure::DiskFull
    } else {
        otherwise
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Claude,
    OpenCode,
    Cursor,
    Npm(&'static str),
}

/// One agent's recipe.
#[derive(Debug, Clone, Copy)]
pub struct Recipe {
    pub agent: &'static str,
    /// The command its profile runs (`profile.rs`), and `bin/<command>`.
    pub command: &'static str,
    kind: Kind,
    /// The largest download accepted.
    max_bytes: u64,
}

const MIB: u64 = 1024 * 1024;

pub const RECIPES: [Recipe; 5] = [
    Recipe {
        agent: "claude-code",
        command: "claude",
        kind: Kind::Claude,
        max_bytes: 800 * MIB,
    },
    Recipe {
        agent: "opencode",
        command: "opencode",
        kind: Kind::OpenCode,
        max_bytes: 400 * MIB,
    },
    Recipe {
        agent: "codex",
        command: "codex",
        kind: Kind::Npm("@openai/codex"),
        max_bytes: 0,
    },
    Recipe {
        agent: "cursor",
        command: "cursor-agent",
        kind: Kind::Cursor,
        max_bytes: 400 * MIB,
    },
    Recipe {
        agent: "command-code",
        command: "command-code",
        kind: Kind::Npm("command-code"),
        max_bytes: 0,
    },
];

pub fn recipe(agent: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.agent == agent)
}

/// Whether Cursor's row must say its download is not checksum-verified.
pub fn verified(agent: &str) -> bool {
    !matches!(recipe(agent).map(|r| r.kind), Some(Kind::Cursor))
}

/// The hosts a recipe may reach, every hop of every redirect included.
const ALLOWED_HOSTS: [&str; 8] = [
    "downloads.claude.ai",
    "registry.npmjs.org",
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
    "cursor.com",
    "downloads.cursor.com",
];

fn allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h))
}

fn http() -> Result<reqwest::Client, Failure> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .user_agent(concat!("storm-runtime/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 5 || !allowed(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| Failure::DownloadFailed)
}

/// A version string from the network, before it names a folder: at most 64
/// of `[A-Za-z0-9._-]`, starting with a digit.
fn version_ok(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !v.contains("..")
}

async fn get_text(http: &reqwest::Client, url: &str) -> Result<String, Failure> {
    let url = reqwest::Url::parse(url).map_err(|_| Failure::DownloadFailed)?;
    if !allowed(&url) {
        return Err(Failure::DownloadFailed);
    }
    let response = http
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            tracing::warn!(error = %e, "agent install: a request failed");
            Failure::DownloadFailed
        })?;
    if !allowed(response.url()) {
        return Err(Failure::DownloadFailed);
    }
    let text = response.text().await.map_err(|_| Failure::DownloadFailed)?;
    if text.len() > 4 * MIB as usize {
        return Err(Failure::DownloadFailed);
    }
    Ok(text)
}

async fn get_json(http: &reqwest::Client, url: &str) -> Result<serde_json::Value, Failure> {
    serde_json::from_str(&get_text(http, url).await?).map_err(|_| Failure::DownloadFailed)
}

/// Streams `url` into `to`, at most `max` bytes, and returns its SHA-256.
async fn download(
    http: &reqwest::Client,
    url: &str,
    to: &Path,
    max: u64,
) -> Result<[u8; 32], Failure> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let url = reqwest::Url::parse(url).map_err(|_| Failure::DownloadFailed)?;
    if !allowed(&url) {
        return Err(Failure::DownloadFailed);
    }
    let response = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            tracing::warn!(error = %e, "agent install: a download failed");
            Failure::DownloadFailed
        })?;
    if !allowed(response.url()) || response.content_length().is_some_and(|n| n > max) {
        return Err(Failure::DownloadFailed);
    }
    let mut file = tokio::fs::File::create(to)
        .await
        .map_err(|e| io_failure(&e, Failure::DownloadFailed))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|_| Failure::DownloadFailed)?;
        total += chunk.len() as u64;
        if total > max {
            return Err(Failure::DownloadFailed);
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| io_failure(&e, Failure::DownloadFailed))?;
    }
    file.flush()
        .await
        .map_err(|e| io_failure(&e, Failure::DownloadFailed))?;
    Ok(hasher.finalize().into())
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// A download whose SHA-256 is not the published one never installs.
fn check(got: &[u8; 32], published: &str) -> Result<(), Failure> {
    if hex(got).eq_ignore_ascii_case(published.trim()) {
        Ok(())
    } else {
        Err(Failure::ChecksumMismatch)
    }
}

/// `(os, arch)` as the vendors spell them, or `None` on a platform no recipe
/// supports yet (133-Q1).
fn platform() -> Option<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some((os, arch))
}

/// A Linux whose libc is musl, as the vendors' installers check for it.
fn musl() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_dir("/lib")
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-"))
}

/// An x86-64 CPU without AVX2 gets OpenCode's `-baseline` build.
fn needs_baseline() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        !std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// The newest version the recipe would install (133-Q2): every agent's own
/// "latest", and for Cursor whatever its installer names today.
pub async fn latest(agent: &str) -> Result<String, Failure> {
    let recipe = recipe(agent).ok_or(Failure::Unknown)?;
    let http = http()?;
    let v = match recipe.kind {
        Kind::Claude => get_text(
            &http,
            "https://downloads.claude.ai/claude-code-releases/latest",
        )
        .await?
        .trim()
        .to_string(),
        Kind::OpenCode => {
            get_json(&http, "https://registry.npmjs.org/opencode-ai/latest").await?["version"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        }
        Kind::Npm(package) => get_json(
            &http,
            &format!("https://registry.npmjs.org/{package}/latest"),
        )
        .await?["version"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        Kind::Cursor => cursor_version(&get_text(&http, "https://cursor.com/install").await?)
            .unwrap_or_default(),
    };
    if version_ok(&v) {
        Ok(v)
    } else {
        Err(Failure::DownloadFailed)
    }
}

/// The version Cursor's installer downloads: the path segment after
/// `downloads.cursor.com/lab/`.
pub fn cursor_version(script: &str) -> Option<String> {
    let rest = script.split("downloads.cursor.com/lab/").nth(1)?;
    let v: String = rest.chars().take_while(|c| *c != '/').collect();
    version_ok(&v).then_some(v)
}

fn agent_dir(state_dir: &Path, recipe: &Recipe) -> PathBuf {
    crate::agents::managed_dir(state_dir).join(recipe.agent)
}

fn npm_prefix(state_dir: &Path) -> PathBuf {
    crate::agents::managed_dir(state_dir).join("npm")
}

/// The managed binary `bin/<command>` points at, if Storm installed one.
pub fn managed_target(state_dir: &Path, agent: &str) -> Option<PathBuf> {
    let recipe = recipe(agent)?;
    let link = crate::agents::managed_bin(state_dir).join(recipe.command);
    std::fs::read_link(link).ok()
}

/// Runs one job. Blocking work goes to the blocking pool; `step` is told
/// each phase (`Downloading`, `Verifying`, `Installing`) for the app.
/// Returns the version now installed, or `None` after a remove.
pub async fn run(
    state_dir: &Path,
    agent: &str,
    op: Op,
    path: Option<OsString>,
    step: &(dyn Fn(&'static str) + Sync),
) -> Result<Option<String>, Failure> {
    run_as(state_dir, agent, op, path, step, false).await
}

/// The isolation helper's job (136g): `run_as` the install identity, with
/// the `PATH` its unit gives, answered as one line for the helper to check.
pub async fn isolated_job(state_dir: &Path, agent: &str, action: &str) -> serde_json::Value {
    let Some(op) = Op::from_name(action) else {
        return serde_json::json!({"failed": Failure::Unknown.code()});
    };
    if let Some(home) = std::env::var_os("HOME") {
        let _ = crate::platform::create_private_dir(Path::new(&home));
    }
    let path = std::env::var_os("PATH");
    match run_as(state_dir, agent, op, path, &|_| {}, true).await {
        Ok(version) => serde_json::json!({"done": {"version": version}}),
        Err(f) => serde_json::json!({"failed": f.code()}),
    }
}

/// [`run`], and with `shared` as the install identity on an isolated host
/// (136g): what it writes is for every session to read.
pub async fn run_as(
    state_dir: &Path,
    agent: &str,
    op: Op,
    path: Option<OsString>,
    step: &(dyn Fn(&'static str) + Sync),
    shared: bool,
) -> Result<Option<String>, Failure> {
    let recipe = recipe(agent).ok_or(Failure::Unknown)?;
    if op == Op::Remove {
        return remove(state_dir, recipe, path).await.map(|()| None);
    }
    if op == Op::Update && managed_target(state_dir, agent).is_none() {
        return Err(Failure::NotManaged);
    }
    let version = latest(agent).await?;
    if op == Op::Update
        && std::fs::read_to_string(agent_dir(state_dir, recipe).join("current"))
            .is_ok_and(|c| c.trim() == version)
        && managed_target(state_dir, agent).is_some_and(|t| t.exists())
    {
        return Ok(Some(version));
    }
    let target = match recipe.kind {
        Kind::Npm(package) => {
            step("Installing");
            npm_install(state_dir, recipe, package, &version, path.clone(), shared).await?
        }
        _ => fetch_release(state_dir, recipe, &version, step, shared).await?,
    };
    activate_as(state_dir, recipe, &target, &version, shared)
        .map_err(|e| io_failure(&e, Failure::UnpackFailed))?;
    let probed = tokio::task::spawn_blocking({
        let target = target.clone();
        move || probe(&target, path.as_deref())
    })
    .await
    .ok()
    .flatten();
    match probed {
        Some(v) => Ok(Some(v)),
        None => Err(Failure::VersionProbeFailed),
    }
}

fn probe(binary: &Path, path: Option<&OsStr>) -> Option<String> {
    crate::agents::parse_version(&crate::agents::run_version(binary, path)?)
}

/// Claude Code, OpenCode and Cursor: download, check, unpack into
/// `<agent>/<version>/`, and return the binary.
async fn fetch_release(
    state_dir: &Path,
    recipe: &Recipe,
    version: &str,
    step: &(dyn Fn(&'static str) + Sync),
    shared: bool,
) -> Result<PathBuf, Failure> {
    let (os, arch) = platform().ok_or(Failure::Blocked("unsupported_platform"))?;
    let http = http()?;
    let dir = agent_dir(state_dir, recipe);
    let managed = crate::agents::managed_dir(state_dir);
    if shared {
        std::fs::create_dir_all(&managed)
    } else {
        crate::platform::create_private_dir(&managed).or_else(|_| std::fs::create_dir_all(&managed))
    }
    .map_err(|e| io_failure(&e, Failure::UnpackFailed))?;
    std::fs::create_dir_all(&dir).map_err(|e| io_failure(&e, Failure::UnpackFailed))?;
    let final_dir = dir.join(version);
    let staging = dir.join(format!(".tmp-{version}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| io_failure(&e, Failure::UnpackFailed))?;
    let result = async {
        let binary = match recipe.kind {
            Kind::Claude => {
                let platform = format!("{os}-{arch}{}", if musl() { "-musl" } else { "" });
                let base = format!("https://downloads.claude.ai/claude-code-releases/{version}");
                let manifest = get_json(&http, &format!("{base}/manifest.json")).await?;
                let entry = &manifest["platforms"][&platform];
                let want = entry["checksum"].as_str().ok_or(Failure::DownloadFailed)?;
                let size = entry["size"].as_u64().unwrap_or(recipe.max_bytes);
                let file = staging.join("claude");
                step("Downloading");
                let got = download(&http, &format!("{base}/{platform}/claude"), &file, size.min(recipe.max_bytes)).await?;
                step("Verifying");
                check(&got, want)?;
                make_executable(&file).map_err(|e| io_failure(&e, Failure::UnpackFailed))?;
                PathBuf::from("claude")
            }
            Kind::OpenCode => {
                let mut target = format!("{os}-{arch}");
                if arch == "x64" && needs_baseline() {
                    target.push_str("-baseline");
                }
                if musl() {
                    target.push_str("-musl");
                }
                let ext = if os == "linux" { "tar.gz" } else { "zip" };
                let name = format!("opencode-{target}.{ext}");
                let release = get_json(
                    &http,
                    &format!("https://api.github.com/repos/anomalyco/opencode/releases/tags/v{version}"),
                )
                .await?;
                let want = release["assets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|a| a["name"] == name.as_str())
                    .and_then(|a| a["digest"].as_str())
                    .and_then(|d| d.strip_prefix("sha256:"))
                    .ok_or(Failure::DownloadFailed)?
                    .to_string();
                let archive = staging.join(&name);
                step("Downloading");
                let got = download(
                    &http,
                    &format!("https://github.com/anomalyco/opencode/releases/download/v{version}/{name}"),
                    &archive,
                    recipe.max_bytes,
                )
                .await?;
                step("Verifying");
                check(&got, &want)?;
                step("Installing");
                unpack(&archive, &staging, false).await?;
                let _ = std::fs::remove_file(&archive);
                PathBuf::from("opencode")
            }
            Kind::Cursor => {
                let archive = staging.join("agent-cli-package.tar.gz");
                step("Downloading");
                download(
                    &http,
                    &format!("https://downloads.cursor.com/lab/{version}/{os}/{arch}/agent-cli-package.tar.gz"),
                    &archive,
                    recipe.max_bytes,
                )
                .await?;
                // Cursor publishes no checksum (133-1): TLS to its own host is
                // all there is, and the app says so.
                step("Installing");
                unpack(&archive, &staging, true).await?;
                let _ = std::fs::remove_file(&archive);
                PathBuf::from("cursor-agent")
            }
            Kind::Npm(_) => unreachable!("npm agents are not fetched"),
        };
        if !staging.join(&binary).is_file() {
            return Err(Failure::UnpackFailed);
        }
        Ok(binary)
    }
    .await;
    let binary = match result {
        Ok(binary) => binary,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let _ = std::fs::remove_dir_all(&final_dir);
    std::fs::rename(&staging, &final_dir).map_err(|e| io_failure(&e, Failure::UnpackFailed))?;
    Ok(final_dir.join(binary))
}

fn make_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Unpacks with the system's `tar` (bsdtar on macOS also reads a zip), which
/// refuses absolute and `..` member names by default.
async fn unpack(archive: &Path, into: &Path, strip_one: bool) -> Result<(), Failure> {
    let mut command = tokio::process::Command::new("tar");
    command.arg("-xf").arg(archive).arg("-C").arg(into);
    if strip_one {
        command.arg("--strip-components=1");
    }
    let status = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|_| Failure::UnpackFailed)?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::UnpackFailed)
    }
}

/// Codex and Command Code: `npm install` into Storm's prefix. Codex runs no
/// install script and gets `--ignore-scripts`; Command Code's ripgrep
/// dependency fetches its binary in one (gate G-133-4).
async fn npm_install(
    state_dir: &Path,
    recipe: &Recipe,
    package: &str,
    version: &str,
    path: Option<OsString>,
    shared: bool,
) -> Result<PathBuf, Failure> {
    let npm = crate::pty::resolve(OsStr::new("npm"), path.as_deref())
        .ok_or(Failure::Blocked("no_node"))?;
    if shared {
        // Under the install identity's UMask=0022 npm would make its cache,
        // and the logs in it, readable by every session.
        crate::platform::create_private_dir(
            &crate::agents::managed_dir(state_dir).join("npm-cache"),
        )
        .map_err(|e| io_failure(&e, Failure::NpmFailed))?;
    }
    let prefix = npm_prefix(state_dir);
    std::fs::create_dir_all(&prefix).map_err(|e| io_failure(&e, Failure::NpmFailed))?;
    let mut command = tokio::process::Command::new(npm);
    command.arg("install").arg("--prefix").arg(&prefix).args([
        "--no-fund",
        "--no-audit",
        "--save-exact",
        "--loglevel=error",
    ]);
    if recipe.agent == "codex" {
        command.arg("--ignore-scripts");
    }
    command
        .arg(format!("{package}@{version}"))
        .env(
            "npm_config_cache",
            crate::agents::managed_dir(state_dir).join("npm-cache"),
        )
        .env("npm_config_update_notifier", "false")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if let Some(path) = &path {
        command.env("PATH", path);
    }
    let out = command.output().await.map_err(|_| Failure::NpmFailed)?;
    if !out.status.success() {
        let tail: String = String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .join(" | ");
        tracing::warn!(agent = recipe.agent, npm = %tail, "agent install: npm failed");
        if tail.contains("ENOSPC") {
            return Err(Failure::DiskFull);
        }
        return Err(Failure::NpmFailed);
    }
    let binary = prefix
        .join("node_modules")
        .join(".bin")
        .join(recipe.command);
    if binary.exists() {
        Ok(binary)
    } else {
        Err(Failure::NpmFailed)
    }
}

/// Points `bin/<command>` at `target`, atomically, records the version, and
/// keeps only the current and previous releases.
#[cfg(test)]
fn activate(
    state_dir: &Path,
    recipe: &Recipe,
    target: &Path,
    version: &str,
) -> std::io::Result<()> {
    activate_as(state_dir, recipe, target, version, false)
}

fn activate_as(
    state_dir: &Path,
    recipe: &Recipe,
    target: &Path,
    version: &str,
    shared: bool,
) -> std::io::Result<()> {
    let bin = crate::agents::managed_bin(state_dir);
    std::fs::create_dir_all(&bin)?;
    let link = bin.join(recipe.command);
    let staged = bin.join(format!(".{}.new", recipe.command));
    let _ = std::fs::remove_file(&staged);
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &staged)?;
    #[cfg(not(unix))]
    std::fs::write(&staged, format!("@\"{}\" %*\r\n", target.display()))?;
    std::fs::rename(&staged, &link)?;
    let dir = agent_dir(state_dir, recipe);
    std::fs::create_dir_all(&dir)?;
    let previous = std::fs::read_to_string(dir.join("current")).ok();
    crate::fsutil::write_atomic(
        &dir.join("current"),
        version.as_bytes(),
        if shared {
            crate::fsutil::Access::Default
        } else {
            crate::fsutil::Access::Private
        },
    )
    .map_err(std::io::Error::other)?;
    if !matches!(recipe.kind, Kind::Npm(_)) {
        let keep = [
            Some(version.to_string()),
            previous.map(|p| p.trim().to_string()),
        ];
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let release = entry.path().is_dir() && !name.starts_with('.');
            if release && !keep.iter().flatten().any(|k| *k == name) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(())
}

/// Removes what Storm installed for `recipe`, and nothing else.
async fn remove(state_dir: &Path, recipe: &Recipe, path: Option<OsString>) -> Result<(), Failure> {
    let link = crate::agents::managed_bin(state_dir).join(recipe.command);
    if std::fs::symlink_metadata(&link).is_err() {
        return Err(Failure::NotManaged);
    }
    std::fs::remove_file(&link).map_err(|e| io_failure(&e, Failure::UnpackFailed))?;
    if let Kind::Npm(package) = recipe.kind
        && let Some(npm) = crate::pty::resolve(OsStr::new("npm"), path.as_deref())
    {
        let mut command = tokio::process::Command::new(npm);
        command
            .arg("uninstall")
            .arg("--prefix")
            .arg(npm_prefix(state_dir))
            .args(["--no-fund", "--no-audit", "--loglevel=error", package])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Some(path) = &path {
            command.env("PATH", path);
        }
        let _ = command.status().await;
    }
    let _ = std::fs::remove_dir_all(agent_dir(state_dir, recipe));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_installable_agent_has_a_recipe_for_its_profiles_command() {
        for agent in crate::agents::MANAGEABLE {
            let r = recipe(agent).unwrap_or_else(|| panic!("{agent}"));
            assert_eq!(
                Some(r.command),
                crate::profile::get(agent).map(|p| p.command),
                "{agent}"
            );
        }
        assert!(recipe("shell").is_none() && recipe("my-agent").is_none());
        assert!(!verified("cursor") && verified("claude-code"));
    }

    #[test]
    fn a_version_from_the_network_cannot_name_another_folder() {
        for good in ["2.1.310", "1.19.4", "2026.10.22-27dcf5f", "0.164.0"] {
            assert!(version_ok(good), "{good}");
        }
        for bad in [
            "",
            "../x",
            "1.0/../..",
            "v1.0",
            "1 0",
            "1..2",
            &"9".repeat(65),
        ] {
            assert!(!version_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn cursor_s_version_is_read_from_its_installer() {
        let script = r#"DOWNLOAD_URL="https://downloads.cursor.com/lab/2026.10.22-27dcf5f/${OS}/${ARCH}/agent-cli-package.tar.gz""#;
        assert_eq!(
            cursor_version(script).as_deref(),
            Some("2026.10.22-27dcf5f")
        );
        assert_eq!(cursor_version("no url"), None);
        assert_eq!(cursor_version("downloads.cursor.com/lab/../evil/x"), None);
    }

    #[test]
    fn only_the_vendors_hosts_over_https() {
        for ok in [
            "https://downloads.claude.ai/claude-code-releases/latest",
            "https://release-assets.githubusercontent.com/x",
        ] {
            assert!(allowed(&reqwest::Url::parse(ok).unwrap()), "{ok}");
        }
        for bad in [
            "http://downloads.claude.ai/x",
            "https://evil.example/x",
            "https://downloads.claude.ai.evil.example/x",
        ] {
            assert!(!allowed(&reqwest::Url::parse(bad).unwrap()), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn activation_swaps_the_link_and_keeps_one_previous_release() {
        let t = tempfile::tempdir().unwrap();
        let r = recipe("claude-code").unwrap();
        let dir = agent_dir(t.path(), r);
        for v in ["1.0.0", "1.1.0", "1.2.0"] {
            std::fs::create_dir_all(dir.join(v)).unwrap();
            std::fs::write(dir.join(v).join("claude"), v).unwrap();
            activate(t.path(), r, &dir.join(v).join("claude"), v).unwrap();
        }
        assert_eq!(
            managed_target(t.path(), "claude-code"),
            Some(dir.join("1.2.0").join("claude"))
        );
        assert!(dir.join("1.1.0").exists(), "one rollback kept");
        assert!(!dir.join("1.0.0").exists(), "older releases pruned");
        assert_eq!(
            std::fs::read_to_string(dir.join("current")).unwrap(),
            "1.2.0"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_touches_only_what_storm_installed() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(
            remove(t.path(), recipe("cursor").unwrap(), None).await,
            Err(Failure::NotManaged)
        );
        let r = recipe("cursor").unwrap();
        let dir = agent_dir(t.path(), r).join("1.0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cursor-agent"), "x").unwrap();
        activate(t.path(), r, &dir.join("cursor-agent"), "1.0").unwrap();
        remove(t.path(), r, None).await.unwrap();
        assert!(managed_target(t.path(), "cursor").is_none());
        assert!(!agent_dir(t.path(), r).exists());
    }

    #[test]
    fn one_changed_byte_fails_the_checksum() {
        let body = b"a release".to_vec();
        let published = hex(&Sha256::digest(&body)[..]);
        let mut corrupted = body.clone();
        corrupted[0] ^= 1;
        let digest = |b: &[u8]| -> [u8; 32] { Sha256::digest(b).into() };
        assert_eq!(check(&digest(&body), &published), Ok(()));
        assert_eq!(
            check(&digest(&corrupted), &published),
            Err(Failure::ChecksumMismatch)
        );
        assert_eq!(check(&digest(&body), ""), Err(Failure::ChecksumMismatch));
    }
}
