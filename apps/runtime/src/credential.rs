//! Token sync (decision 124, AM68–AM71): a `claude-code` session whose
//! `start` names `provider_credential` runs on the operator's synced Claude
//! subscription token instead of this host's own login.
//!
//! - The token is pulled per launch, during `starting`, from
//!   `GET /v1/runtime/sessions/{id}/provider-credential`. It is held only in
//!   a [`SecretOsString`] until the CLI is spawned, and reaches the CLI only
//!   as `CLAUDE_CODE_OAUTH_TOKEN` in its process environment: never argv, a
//!   file, a log line or an error.
//! - Competing credentials and routing switches are removed from the
//!   process environment ([`ENV_REMOVE`]) and pinned in the session's one
//!   `--settings` file ([`settings`]), which beats workspace, local and user
//!   settings and survives their mid-session reload (gate G-TS, CLI
//!   2.1.301). An invalid `--settings` file is ignored by the CLI with only a
//!   warning, so every value written is a string and the file is re-read
//!   before use ([`verify_settings`]).
//! - Any failure ends the launch. A synced session never falls back to the
//!   host's own login.
//!
//! **Known gaps** (G-TS): a workspace `env.CLAUDE_CODE_OAUTH_TOKEN` replaces
//! the synced token (a substitution, not a leak: the base URL stays pinned),
//! and it cannot be pinned without writing the token into `--settings`. A
//! managed `apiKeyHelper` outranks the flag settings: one in a readable
//! managed-settings file refuses the launch ([`managed_helper`]); one set by
//! MDM (a macOS profile, the Windows registry) or in an unreadable file is
//! not detected.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};

pub const KIND: &str = "claude_subscription_token";
pub const PROVIDER: &str = "claude-code";
pub const TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
pub const BASE_URL: &str = "https://api.anthropic.com";

/// Removed from a synced session's process environment, whatever the
/// daemon's environment or the provider's env file said. §0.5's list, plus
/// the routing switches 2.1.301 also reads.
pub const ENV_REMOVE: [&str; 16] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_SCOPES",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_GATEWAY",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "ANTHROPIC_IDENTITY_TOKEN_FILE",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
];

/// Pinned in the settings `env` to `""`, which the CLI treats as unset.
pub const EMPTY_PINS: [&str; 9] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_GATEWAY",
];

/// Pinned in the settings `env` to the daemon's own value, or `""`. The
/// value must be copied: a flag `""` cancels a process-env proxy or CA too.
pub const NETWORK_PINS: [&str; 12] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "NODE_EXTRA_CA_CERTS",
    "NODE_TLS_REJECT_UNAUTHORIZED",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

const PULL_ATTEMPTS: u32 = 4;
const PULL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BODY: usize = 8 * 1024;

/// An environment value that is never printed. `Debug` shows nothing of it,
/// it has no `Display` and no `Clone`, and its bytes are overwritten on drop.
pub struct SecretOsString(OsString);

impl SecretOsString {
    pub fn new(value: impl Into<OsString>) -> Self {
        Self(value.into())
    }

    pub(crate) fn expose(&self) -> &OsStr {
        &self.0
    }
}

impl fmt::Debug for SecretOsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretOsString(<redacted>)")
    }
}

impl PartialEq for SecretOsString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for SecretOsString {}

impl Drop for SecretOsString {
    fn drop(&mut self) {
        wipe(&mut std::mem::take(&mut self.0).into_encoded_bytes());
    }
}

fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Why a synced launch cannot go ahead. Its `Display` is the host log's
/// reason, and never holds any part of the token.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncError {
    UnknownKind,
    WrongProvider,
    ManagedHelper(PathBuf),
    UnpinnableValue(&'static str),
    Refused(u16, Option<String>),
    Unreachable(String),
    BadResponse(&'static str),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("could not fetch the synced Claude login: ")?;
        match self {
            Self::UnknownKind => f.write_str("a credential kind this host does not know"),
            Self::WrongProvider => f.write_str("only a claude-code session can be synced"),
            Self::ManagedHelper(path) => write!(
                f,
                "a settings file on this host sets apiKeyHelper ({})",
                path.display()
            ),
            Self::UnpinnableValue(name) => {
                write!(
                    f,
                    "this host's {name} is not valid UTF-8, so it cannot be pinned"
                )
            }
            Self::Refused(status, Some(code)) => write!(f, "the server refused ({status} {code})"),
            Self::Refused(status, None) => write!(f, "the server refused ({status})"),
            Self::Unreachable(why) => write!(f, "the server could not be reached: {why}"),
            Self::BadResponse(why) => write!(f, "the server's answer was not usable: {why}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// The checks made before anything is pulled: the kind, the provider, and no
/// managed helper the flag settings cannot neutralise.
pub fn check_launch(kind: &str, provider: &str, managed_dir: &Path) -> Result<(), SyncError> {
    if kind != KIND {
        return Err(SyncError::UnknownKind);
    }
    if provider != PROVIDER {
        return Err(SyncError::WrongProvider);
    }
    match managed_helper(managed_dir) {
        Some(path) => Err(SyncError::ManagedHelper(path)),
        None => Ok(()),
    }
}

/// Where Claude Code reads its managed settings on this platform.
pub fn managed_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        "/Library/Application Support/ClaudeCode".into()
    } else if cfg!(windows) {
        r"C:\Program Files\ClaudeCode".into()
    } else {
        "/etc/claude-code".into()
    }
}

/// The readable managed-settings file, if any, that names an `apiKeyHelper`.
pub fn managed_helper(dir: &Path) -> Option<PathBuf> {
    let mut files = vec![dir.join("managed-settings.json")];
    if let Ok(entries) = std::fs::read_dir(dir.join("managed-settings.d")) {
        let mut more: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        more.sort();
        files.extend(more);
    }
    files.into_iter().find(|path| {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|v| v["apiKeyHelper"].as_str().is_some_and(|h| !h.is_empty()))
    })
}

/// What a synced session's `--settings` file gains: `"apiKeyHelper": ""` and
/// the `env` pins. `daemon` reads this daemon's own environment.
pub fn settings(
    daemon: impl Fn(&str) -> Option<OsString>,
) -> Result<Map<String, Value>, SyncError> {
    let mut env = Map::new();
    env.insert("ANTHROPIC_BASE_URL".into(), BASE_URL.into());
    for name in EMPTY_PINS {
        env.insert(name.into(), "".into());
    }
    for name in NETWORK_PINS {
        let value = match daemon(name) {
            Some(v) => v
                .into_string()
                .map_err(|_| SyncError::UnpinnableValue(name))?,
            None => String::new(),
        };
        env.insert(name.into(), value.into());
    }
    let mut out = Map::new();
    out.insert("apiKeyHelper".into(), "".into());
    out.insert("env".into(), Value::Object(env));
    Ok(out)
}

/// Re-reads written settings bytes and checks they hold every pin as a
/// string: a file the CLI would reject runs without its pins.
pub fn verify_settings(bytes: &[u8], pins: &Map<String, Value>) -> anyhow::Result<()> {
    let parsed: Value = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        parsed["apiKeyHelper"].as_str() == Some(""),
        "the settings lost apiKeyHelper"
    );
    let env = parsed["env"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("the settings have no env"))?;
    anyhow::ensure!(
        env.values().all(Value::is_string),
        "a settings env value is not a string"
    );
    for (name, want) in pins["env"].as_object().into_iter().flatten() {
        anyhow::ensure!(
            env.get(name) == Some(want),
            "the settings lost the {name} pin"
        );
    }
    Ok(())
}

#[derive(Deserialize)]
struct Pulled {
    kind: String,
    value: String,
}

#[derive(Deserialize)]
struct Refusal {
    error: String,
}

/// Pulls the session's synced token, retrying a network failure or a `5xx`
/// while the session is `starting`. A refusal is final. `on_unauthorized`
/// runs on a `401`, so the host re-authenticates.
pub async fn pull(
    http: &reqwest::Client,
    base: &str,
    host_token: &str,
    session: &str,
    on_unauthorized: impl Fn(),
) -> Result<SecretOsString, SyncError> {
    let url = format!("{base}/v1/runtime/sessions/{session}/provider-credential");
    let mut wait = Duration::from_millis(250);
    let mut attempt = 1;
    loop {
        let err = match http
            .get(&url)
            .bearer_auth(host_token)
            .timeout(PULL_TIMEOUT)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return read(response).await,
            Ok(response) if response.status().is_server_error() => refusal(response).await,
            Ok(response) => {
                if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                    on_unauthorized();
                }
                return Err(refusal(response).await);
            }
            Err(e) => SyncError::Unreachable(without_url(&e)),
        };
        if attempt >= PULL_ATTEMPTS {
            return Err(err);
        }
        tracing::debug!(session, attempt, "retrying the synced login pull");
        tokio::time::sleep(wait).await;
        wait *= 2;
        attempt += 1;
    }
}

fn without_url(e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "connection failed"
    } else {
        "request failed"
    };
    kind.to_string()
}

async fn refusal(response: reqwest::Response) -> SyncError {
    let status = response.status().as_u16();
    let code = response
        .bytes()
        .await
        .ok()
        .filter(|b| b.len() <= MAX_BODY)
        .and_then(|b| serde_json::from_slice::<Refusal>(&b).ok())
        .map(|r| r.error)
        .filter(|c| {
            (1..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        });
    SyncError::Refused(status, code)
}

async fn read(response: reqwest::Response) -> Result<SecretOsString, SyncError> {
    let mut body = Vec::from(
        response
            .bytes()
            .await
            .map_err(|e| SyncError::Unreachable(without_url(&e)))?,
    );
    let parsed = if body.len() > MAX_BODY {
        Err(SyncError::BadResponse("too large"))
    } else {
        serde_json::from_slice::<Pulled>(&body)
            .map_err(|_| SyncError::BadResponse("not the pinned shape"))
    };
    wipe(&mut body);
    let pulled = parsed?;
    let value = SecretOsString::new(pulled.value);
    if pulled.kind != KIND {
        return Err(SyncError::BadResponse("another credential kind"));
    }
    if !token_shape(value.expose()) {
        return Err(SyncError::BadResponse("not a Claude subscription token"));
    }
    Ok(value)
}

/// `^sk-ant-oat01-[A-Za-z0-9_-]{20,400}$` (§0.1).
fn token_shape(value: &OsStr) -> bool {
    value
        .to_str()
        .and_then(|v| v.strip_prefix("sk-ant-oat01-"))
        .is_some_and(|rest| {
            (20..=400).contains(&rest.len())
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    pub(crate) const CANARY: &str =
        "sk-ant-oat01-STORMCANARY-RUNTIME-0000000000000000000000000000000000";

    /// What the stub answers the pull route with: a queue of `(status,
    /// body)`, the last one repeating. Every other request gets `200 {}`.
    pub(crate) type Answers = Arc<Mutex<Vec<(u16, String)>>>;
    /// `(method, path, authorization, body)` of every request.
    pub(crate) type Seen = Arc<Mutex<Vec<(String, String, String, String)>>>;

    pub(crate) fn ok_body(value: &str) -> String {
        serde_json::json!({"kind": KIND, "value": value, "expires_at": "2027-10-16T09:00:00Z"})
            .to_string()
    }

    pub(crate) async fn stub_server(answers: Answers) -> (String, Seen) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let record = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut conn, _)) = listener.accept().await else {
                    return;
                };
                let (record, answers) = (record.clone(), answers.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    loop {
                        let head_end = loop {
                            if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break at + 4;
                            }
                            let mut chunk = [0u8; 4096];
                            match conn.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            }
                        };
                        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                        let mut words = head.split_whitespace();
                        let method = words.next().unwrap_or("").to_string();
                        let path = words.next().unwrap_or("").to_string();
                        let lower = head.to_lowercase();
                        let auth = lower
                            .lines()
                            .find_map(|l| l.strip_prefix("authorization:"))
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        let len: usize = lower
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        while buf.len() < head_end + len {
                            let mut chunk = [0u8; 4096];
                            match conn.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            }
                        }
                        let body =
                            String::from_utf8_lossy(&buf[head_end..head_end + len]).into_owned();
                        buf.drain(..head_end + len);
                        let pull = method == "GET" && path.ends_with("/provider-credential");
                        record.lock().unwrap().push((method, path, auth, body));
                        let (status, body) = if pull {
                            let mut a = answers.lock().unwrap();
                            if a.len() > 1 {
                                a.remove(0)
                            } else {
                                a[0].clone()
                            }
                        } else {
                            (200, "{}".to_string())
                        };
                        let reply = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                            body.len()
                        );
                        if conn.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (url, seen)
    }

    fn answers(list: &[(u16, String)]) -> Answers {
        Arc::new(Mutex::new(list.to_vec()))
    }

    async fn pull_from(list: &[(u16, String)]) -> (Result<SecretOsString, SyncError>, Seen, u32) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (url, seen) = stub_server(answers(list)).await;
        let http = reqwest::Client::new();
        let reauth = std::sync::atomic::AtomicU32::new(0);
        let got = pull(&http, &url, "sht_test", "ags_X", || {
            reauth.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .await;
        (got, seen, reauth.into_inner())
    }

    #[tokio::test]
    async fn a_pull_gets_the_token_on_the_host_token() {
        let (got, seen, _) = pull_from(&[(200, ok_body(CANARY))]).await;
        assert_eq!(got.unwrap().expose(), CANARY);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "GET");
        assert_eq!(seen[0].1, "/v1/runtime/sessions/ags_X/provider-credential");
        assert_eq!(seen[0].2, "bearer sht_test");
    }

    #[tokio::test]
    async fn a_refusal_is_final_and_names_its_code_only() {
        for (status, code) in [
            (403, "host_not_in_scope"),
            (404, "no_credential"),
            (404, "not_your_session"),
            (409, "session_not_starting"),
            (409, "wrong_provider"),
        ] {
            let body = serde_json::json!({"error": code}).to_string();
            let (got, seen, _) = pull_from(&[(status, body)]).await;
            assert_eq!(
                got.unwrap_err(),
                SyncError::Refused(status, Some(code.into()))
            );
            assert_eq!(seen.lock().unwrap().len(), 1, "{code} is not retried");
        }
        let (got, _, reauth) = pull_from(&[(401, "{}".into())]).await;
        assert_eq!(got.unwrap_err(), SyncError::Refused(401, None));
        assert_eq!(reauth, 1);
    }

    #[tokio::test]
    async fn a_server_error_is_retried_then_fails() {
        let (got, seen, _) = pull_from(&[(503, "{}".into()), (200, ok_body(CANARY))]).await;
        assert_eq!(got.unwrap().expose(), CANARY);
        assert_eq!(seen.lock().unwrap().len(), 2);

        let (got, seen, _) = pull_from(&[(500, "{}".into())]).await;
        assert_eq!(got.unwrap_err(), SyncError::Refused(500, None));
        assert_eq!(seen.lock().unwrap().len(), PULL_ATTEMPTS as usize);
    }

    #[tokio::test]
    async fn an_unreachable_server_fails_after_its_retries() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let got = pull(
            &reqwest::Client::new(),
            &format!("http://127.0.0.1:{port}"),
            "sht_test",
            "ags_X",
            || {},
        )
        .await;
        assert!(matches!(got, Err(SyncError::Unreachable(_))), "{got:?}");
    }

    #[tokio::test]
    async fn an_unusable_answer_fails_without_echoing_it() {
        let wrong_kind = serde_json::json!({"kind": "other", "value": CANARY}).to_string();
        let short = ok_body("sk-ant-oat01-short");
        let api_key = ok_body("sk-ant-api03-STORMCANARY-00000000000000000000000000");
        let newline = ok_body(&format!("{CANARY}\nX=1"));
        for body in [
            wrong_kind,
            short,
            api_key,
            newline,
            "{}".into(),
            "nope".into(),
        ] {
            let (got, _, _) = pull_from(&[(200, body)]).await;
            let err = got.unwrap_err();
            assert!(matches!(err, SyncError::BadResponse(_)), "{err:?}");
            assert!(!format!("{err} {err:?}").contains("STORMCANARY"));
        }
    }

    #[test]
    fn the_token_shape_is_the_specs() {
        assert!(token_shape(OsStr::new(CANARY)));
        assert!(token_shape(OsStr::new(&format!(
            "sk-ant-oat01-{}",
            "a".repeat(20)
        ))));
        assert!(!token_shape(OsStr::new(&format!(
            "sk-ant-oat01-{}",
            "a".repeat(19)
        ))));
        assert!(!token_shape(OsStr::new(&format!(
            "sk-ant-oat01-{}",
            "a".repeat(401)
        ))));
        assert!(!token_shape(OsStr::new(&format!("{CANARY} "))));
    }

    #[test]
    fn a_secret_is_never_debug_printed() {
        let secret = SecretOsString::new(CANARY);
        let extras = crate::provider::LaunchExtras {
            secret_env: vec![(TOKEN_ENV.into(), SecretOsString::new(CANARY))],
            ..Default::default()
        };
        for printed in [
            format!("{secret:?}"),
            format!("{extras:?}"),
            format!("{extras:#?}"),
        ] {
            assert!(!printed.contains("STORMCANARY"), "{printed}");
            assert!(!printed.contains("oat01"), "{printed}");
        }
        assert!(format!("{extras:?}").contains(TOKEN_ENV), "names are shown");
        for err in [
            SyncError::Refused(403, Some("host_not_in_scope".into())),
            SyncError::Unreachable("timed out".into()),
            SyncError::BadResponse("x"),
        ] {
            assert!(format!("{err}").starts_with("could not fetch the synced Claude login"));
        }
    }

    #[test]
    fn only_a_claude_code_session_of_the_known_kind_is_synced() {
        let none = tempfile::tempdir().unwrap();
        assert_eq!(check_launch(KIND, PROVIDER, none.path()), Ok(()));
        assert_eq!(
            check_launch("openai_token", PROVIDER, none.path()),
            Err(SyncError::UnknownKind)
        );
        for provider in ["opencode", "shell", "fake", "my-agent"] {
            assert_eq!(
                check_launch(KIND, provider, none.path()),
                Err(SyncError::WrongProvider)
            );
        }
    }

    #[test]
    fn a_managed_helper_refuses_the_launch() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(managed_helper(dir.path()), None);
        std::fs::write(
            dir.path().join("managed-settings.json"),
            r#"{"apiKeyHelper":"","env":{"X":"1"}}"#,
        )
        .unwrap();
        assert_eq!(managed_helper(dir.path()), None, "an empty helper is none");
        std::fs::create_dir(dir.path().join("managed-settings.d")).unwrap();
        let drop_in = dir.path().join("managed-settings.d/10-key.json");
        std::fs::write(&drop_in, r#"{"apiKeyHelper":"/usr/bin/get-key"}"#).unwrap();
        assert_eq!(managed_helper(dir.path()), Some(drop_in.clone()));
        let err = check_launch(KIND, PROVIDER, dir.path()).unwrap_err();
        assert_eq!(err, SyncError::ManagedHelper(drop_in));
        assert!(
            err.to_string()
                .contains("a settings file on this host sets apiKeyHelper")
        );
        std::fs::write(
            dir.path().join("managed-settings.json"),
            r#"{"apiKeyHelper":"/x"}"#,
        )
        .unwrap();
        assert_eq!(
            managed_helper(dir.path()),
            Some(dir.path().join("managed-settings.json"))
        );
    }

    fn daemon_with(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.into())
    }

    #[test]
    fn the_pins_are_strings_and_copy_the_daemons_network_values() {
        let pins = settings(daemon_with(&[
            ("HTTPS_PROXY", "http://proxy.lan:3128"),
            ("NODE_EXTRA_CA_CERTS", "/etc/ssl/lan-ca.pem"),
            ("ANTHROPIC_API_KEY", "sk-ant-api03-daemon"),
        ]))
        .unwrap();
        assert_eq!(pins["apiKeyHelper"], "");
        let env = pins["env"].as_object().unwrap();
        assert_eq!(env["ANTHROPIC_BASE_URL"], BASE_URL);
        assert_eq!(env["HTTPS_PROXY"], "http://proxy.lan:3128");
        assert_eq!(env["NODE_EXTRA_CA_CERTS"], "/etc/ssl/lan-ca.pem");
        assert_eq!(env["https_proxy"], "");
        assert_eq!(env["NODE_TLS_REJECT_UNAUTHORIZED"], "");
        assert_eq!(
            env["ANTHROPIC_API_KEY"], "",
            "never the daemon's credential"
        );
        for name in EMPTY_PINS.iter().chain(&NETWORK_PINS) {
            assert!(env[*name].is_string(), "{name}");
        }
        assert_eq!(env.len(), 1 + EMPTY_PINS.len() + NETWORK_PINS.len());
        assert!(!env.contains_key(TOKEN_ENV), "the token is never pinned");
        let bytes = serde_json::to_vec(&pins).unwrap();
        verify_settings(&bytes, &pins).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_value_that_cannot_be_a_json_string_fails_closed() {
        use std::os::unix::ffi::OsStringExt;
        let got = settings(|name| (name == "SSL_CERT_DIR").then(|| OsString::from_vec(vec![0xff])));
        assert_eq!(got.unwrap_err(), SyncError::UnpinnableValue("SSL_CERT_DIR"));
    }

    #[test]
    fn a_settings_file_missing_a_pin_is_refused() {
        let pins = settings(|_| None).unwrap();
        let mut broken = Value::Object(pins.clone());
        broken["env"]["NO_PROXY"] = Value::Null;
        assert!(verify_settings(broken.to_string().as_bytes(), &pins).is_err());
        let mut broken = Value::Object(pins.clone());
        broken["apiKeyHelper"] = Value::Null;
        assert!(verify_settings(broken.to_string().as_bytes(), &pins).is_err());
        let mut broken = Value::Object(pins.clone());
        broken["env"].as_object_mut().unwrap().remove("all_proxy");
        assert!(verify_settings(broken.to_string().as_bytes(), &pins).is_err());
        assert!(verify_settings(b"{", &pins).is_err());
    }
}
