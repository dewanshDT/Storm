//! Decision 161, slice 161-9: the ntfy sink. Every non-silent notice is
//! also published to the account's ntfy topic — independent of whether any
//! device is connected, since the point is reaching a closed phone — except
//! an active device already showing the notice's session (the "nothing to
//! add" case [`super::AgentManager::active_device_has_session_open`]
//! shares with the in-app toast/OS choice), and a 15s grace that lets a
//! device opening the session in the meantime cancel it, same as the
//! planned Web Push/UnifiedPush path's "Duplicates with the app in front".
//!
//! This module holds the parts with no `AgentManager` state: the token's
//! secret-at-rest box, the URL/topic rules, the preview-level renderer and
//! the HTTP publish call. The due-queue and the forwarder loop that drives
//! it live in `notifications.rs` (schema, `insert_notice`) and
//! `crate::ntfy_forwarder` (the async task) respectively.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::Rng;
use rusqlite::OptionalExtension;

use super::notifications::{
    AGENT_MESSAGE, CHILD_FINISHED, DELETE_REQUEST, ERROR, FINISHED, HOST_OFFLINE, NEEDS_YOU,
    Notification, STOPPED_WORKING, SUMMARY, TERMINAL_NOTICE, TEST, USAGE_LIMIT,
};

// ---------------------------------------------------------------------
// The token's secret-at-rest box
// ---------------------------------------------------------------------
//
// Independent of the gateway's connection-scoped keyring
// (`gateway::crypto`), since this lives under `state/agent/`, not
// `state/gateway/`, and seals exactly one secret. Same primitive and the
// same limit: XChaCha20-Poly1305 (`chacha20poly1305`, pure Rust) protects a
// leak of `agent.db` alone, never root or a full backup — the key is a
// file for the same `ls -l`-auditable reason as the server identity key
// (A2).

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
/// Domain separation: a ciphertext sealed for anything else never opens
/// here, and vice versa.
const AAD: &[u8] = b"agent.notification_prefs.ntfy_token.v1";

pub struct SecretKey(XChaCha20Poly1305);

pub struct Sealed {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl SecretKey {
    pub fn seal(&self, plaintext: &str) -> Result<Sealed> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = self
            .0
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: AAD,
                },
            )
            .map_err(|_| anyhow::anyhow!("sealing the ntfy token failed"))?;
        Ok(Sealed {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    /// Never logged and never returned over the wire — the caller only
    /// ever uses the string to build one request's `Authorization` header.
    pub fn open(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<String> {
        if nonce.len() != NONCE_LEN {
            bail!("a sealed ntfy token has a {}-byte nonce", nonce.len());
        }
        let plaintext = self
            .0
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: AAD,
                },
            )
            .map_err(|_| anyhow::anyhow!("the stored ntfy token did not authenticate"))?;
        String::from_utf8(plaintext).context("the stored ntfy token was not valid UTF-8")
    }
}

/// Loads `state/agent/keys/ntfy.key` (32 random bytes, `0600` in a `0700`
/// directory, created *with* that mode — no window at a wider one),
/// generating one on first use.
pub fn load_or_create_key(state_dir: &Path) -> Result<SecretKey> {
    let dir = state_dir.join("agent").join("keys");
    crate::auth::identity::create_private_dir(&dir)?;
    let path = dir.join("ntfy.key");
    let bytes = if path.exists() {
        std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?
    } else {
        let mut key = vec![0u8; KEY_LEN];
        rand::rng().fill_bytes(&mut key);
        write_private_file(&path, &key)?;
        key
    };
    if bytes.len() != KEY_LEN {
        bail!("{} is not a {KEY_LEN}-byte key", path.display());
    }
    Ok(SecretKey(
        XChaCha20Poly1305::new_from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("the ntfy key is the wrong length"))?,
    ))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        f.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Settings: read fresh for every delivery, never cached on AgentManager
// ---------------------------------------------------------------------

/// What a notice is published with. The token is opened only for as long
/// as building one request needs it, and `Debug` never shows it.
pub struct NtfySettings {
    pub url: String,
    pub token: Option<String>,
    pub preview: String,
}

impl std::fmt::Debug for NtfySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NtfySettings")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("preview", &self.preview)
            .finish()
    }
}

/// One raw row from `notification_prefs`, before the token is opened.
type SinkRow = (Option<String>, String, Option<Vec<u8>>, Option<Vec<u8>>);

/// `None` when the sink is off ("enabled iff url set").
pub fn load_settings(conn: &rusqlite::Connection, key: &SecretKey) -> Result<Option<NtfySettings>> {
    let row: Option<SinkRow> = conn
        .query_row(
            "SELECT ntfy_url, ntfy_preview, ntfy_token_sealed, ntfy_token_nonce
             FROM notification_prefs WHERE only_row = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((url, preview, sealed, nonce)) = row else {
        return Ok(None);
    };
    let Some(url) = url.filter(|u| !u.is_empty()) else {
        return Ok(None);
    };
    let token = match (sealed, nonce) {
        (Some(ciphertext), Some(nonce)) => Some(key.open(&nonce, &ciphertext)?),
        _ => None,
    };
    Ok(Some(NtfySettings {
        url,
        token,
        preview,
    }))
}

// ---------------------------------------------------------------------
// URL and topic
// ---------------------------------------------------------------------

/// Splits a topic URL (what the person pastes, or "Generate private
/// topic" fills in — e.g. `https://ntfy.sh/storm-ab12…`) into the server
/// root ntfy's JSON publish API posts to (`https://ntfy.sh/`) and the
/// bare topic name. JSON, not the header API: a session or agent title is
/// arbitrary UTF-8, and ntfy's own docs warn headers may mangle that
/// (docs.ntfy.sh/publish, "UTF-8 in HTTP headers") while the JSON body is
/// native UTF-8.
fn split_topic_url(url: &str) -> std::result::Result<(String, String), String> {
    let parsed = url::Url::parse(url).map_err(|_| "ntfy_url does not parse".to_string())?;
    let topic = parsed.path().trim_start_matches('/').to_string();
    if topic.is_empty() || topic.contains('/') {
        return Err(
            "ntfy_url must point at exactly one topic, e.g. https://ntfy.sh/storm-…".into(),
        );
    }
    if !topic
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("a topic is letters, digits, '_' and '-' only".into());
    }
    let mut root = parsed.clone();
    root.set_path("/");
    root.set_query(None);
    root.set_fragment(None);
    Ok((root.to_string(), topic))
}

/// A loopback, RFC 1918, link-local or unique-local IP literal, or
/// `localhost` — the same rule a gateway connection's URL applies to an
/// http upstream (`gateway::connections::is_lan_host`), copied rather than
/// shared so this module does not reach into the gateway for one helper.
fn is_lan_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(crate::discovery::is_lan_scoped)
}

/// `ntfy_url`: absolute `https`, one topic, no userinfo and no fragment;
/// plain `http` only to a LAN-scoped host, since a self-hosted ntfy
/// instance rarely has a certificate (the same shape as a gateway
/// connection's URL, decision 126, AM-G13).
pub fn validate_url(raw: &str) -> std::result::Result<(), String> {
    if raw.len() > 2048 || raw.chars().any(|c| c.is_control() || c == ' ') {
        return Err("ntfy_url is too long or contains spaces or control characters".into());
    }
    let parsed = url::Url::parse(raw).map_err(|_| "ntfy_url does not parse".to_string())?;
    match parsed.scheme() {
        "https" => {}
        "http" if is_lan_host(parsed.host_str().unwrap_or_default()) => {}
        "http" => {
            return Err(
                "ntfy_url must be https, or http to an address on the local network".into(),
            );
        }
        _ => return Err("ntfy_url must be https".into()),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("ntfy_url must not carry a username or password".into());
    }
    if parsed.fragment().is_some() {
        return Err("ntfy_url must not have a fragment".into());
    }
    split_topic_url(raw)?;
    Ok(())
}

// ---------------------------------------------------------------------
// Preview text, priority and tags
// ---------------------------------------------------------------------

/// Builds ntfy's Title + Message for one notice, by the account's own
/// `ntfy_preview` level ("Payload text by channel"): plain text, which
/// ntfy — or a self-hosted instance — stores, so agent and terminal text
/// (the only kinds carrying a person's own words, `agent_message` and
/// `terminal_notice`) is shown only at "full", with the warning the
/// settings page carries. Every other kind's `title` is already Storm's
/// own phrase ("tidier needs you") and safe at any level.
pub fn render(preview: &str, n: &Notification) -> (String, String) {
    let is_agent_text = matches!(n.kind.as_str(), AGENT_MESSAGE | TERMINAL_NOTICE);
    match preview {
        "full" => {
            let title = match (&n.session_title, &n.provider) {
                (Some(session), Some(provider)) => {
                    format!("{session} · {}", super::permission::label(provider))
                }
                (Some(session), None) => session.clone(),
                (None, _) => "Storm".to_string(),
            };
            let message = if is_agent_text {
                n.body
                    .clone()
                    .filter(|b| !b.is_empty())
                    .unwrap_or_else(|| n.title.clone())
            } else {
                n.title.clone()
            };
            (title, message)
        }
        "storm" => (
            "Storm".to_string(),
            format!("Storm · {}", kind_label(&n.kind)),
        ),
        // "session" (default), and anything unrecognised: fail toward the
        // least revealing level, never toward "full".
        _ => {
            let message = if is_agent_text {
                match &n.session_title {
                    Some(s) => format!("{s} sent a message"),
                    None => "Storm sent a message".to_string(),
                }
            } else {
                n.title.clone()
            };
            ("Storm".to_string(), message)
        }
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        NEEDS_YOU => "needs you",
        DELETE_REQUEST => "wants to delete something",
        FINISHED => "finished its turn",
        ERROR => "hit an error",
        USAGE_LIMIT => "hit its usage limit",
        HOST_OFFLINE => "a host went offline",
        CHILD_FINISHED => "a child finished",
        STOPPED_WORKING => "stopped working",
        AGENT_MESSAGE | TERMINAL_NOTICE => "sent a message",
        SUMMARY => "a summary",
        TEST => "a test notification",
        _ => "a notice",
    }
}

/// Priority from urgency: high needs/delete requests sit at ntfy's "max"
/// (5) since a delete request expires in 10 minutes; low (child finished,
/// stopped working) stays below the default so it never vibrates.
pub fn priority_for(urgency: &str) -> u8 {
    match urgency {
        "high" => 5,
        "low" => 2,
        _ => 3,
    }
}

/// ntfy emoji tags per kind, best effort (an unrecognised tag is shown as
/// plain text by ntfy's clients, never an error).
pub fn tags_for(kind: &str) -> Vec<&'static str> {
    match kind {
        NEEDS_YOU => vec!["warning"],
        DELETE_REQUEST => vec!["wastebasket"],
        FINISHED => vec!["white_check_mark"],
        ERROR => vec!["x"],
        USAGE_LIMIT => vec!["hourglass"],
        HOST_OFFLINE => vec!["satellite"],
        CHILD_FINISHED => vec!["baby"],
        STOPPED_WORKING => vec!["double_vertical_bar"],
        AGENT_MESSAGE | TERMINAL_NOTICE => vec!["speech_balloon"],
        SUMMARY => vec!["bell"],
        TEST => vec!["test_tube"],
        _ => vec![],
    }
}

// ---------------------------------------------------------------------
// Grace and delivery
// ---------------------------------------------------------------------

/// The independent 15s grace before a pending notice is actually sent —
/// scaled down under test so the grace-cancellation and retry tests cost
/// milliseconds, not real seconds.
pub const GRACE: Duration = if cfg!(test) {
    Duration::from_millis(60)
} else {
    Duration::from_secs(15)
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(10)
};
/// Bounded retries (never the feedback queue's week-long requeue): this
/// already runs well after the notice's own grace, off the Notifier's own
/// thread, and must not pile up retries across restarts.
const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(1)
};

/// `created` (RFC3339) plus [`GRACE`], for `notifications.ntfy_due_at` —
/// computed once at insert time rather than re-derived from `created` on
/// every poll.
pub fn due_at(created: &str) -> String {
    use time::format_description::well_known::Rfc3339;
    match time::OffsetDateTime::parse(created, &Rfc3339) {
        Ok(t) => (t + time::Duration::milliseconds(GRACE.as_millis() as i64))
            .format(&Rfc3339)
            .unwrap_or_else(|_| created.to_string()),
        Err(_) => created.to_string(),
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(crate::gateway::upstream::tls_config())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("the ntfy client builds")
}

enum PublishOutcome {
    Permanent(String),
    Retry(String),
}

async fn publish_once(
    client: &reqwest::Client,
    root: &str,
    body: &str,
    token: Option<&str>,
) -> std::result::Result<(), PublishOutcome> {
    let mut req = client.post(root).header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let resp = match req.body(body.to_string()).send().await {
        Ok(r) => r,
        Err(e) => {
            return Err(PublishOutcome::Retry(if e.is_timeout() {
                "timeout".into()
            } else {
                "network error".into()
            }));
        }
    };
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    match status.as_u16() {
        // The request itself is wrong, or the server rejects it on
        // principle (bad topic, bad auth, payload too large): retrying
        // would only repeat the same failure.
        400 | 401 | 403 | 404 | 413 => Err(PublishOutcome::Permanent(format!("http {status}"))),
        _ => Err(PublishOutcome::Retry(format!("http {status}"))),
    }
}

/// Publishes one notice through ntfy's JSON API (`POST` to the server
/// root with `topic` in the body — see [`split_topic_url`]), up to
/// [`MAX_ATTEMPTS`] tries with a short backoff on a network error or a
/// retryable status (429, 5xx). The token, if any, rides as
/// `Authorization: Bearer`, never logged. There is deliberately no
/// `X-Id`/sequence header for replace-by-key: ntfy's publish API
/// (docs.ntfy.sh/publish) has no header that replaces an already
/// delivered message, only `X-Title`/`X-Priority`/`X-Tags`/`X-Click`/
/// `X-Actions`/`X-Delay`/`X-Markdown`/`X-Email`/`X-Template`/`X-Timezone`
/// and their aliases — none of them a dedupe or replace token — so this
/// is omitted rather than invented.
pub async fn publish_with_retry(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    title: &str,
    message: &str,
    priority: u8,
    tags: &[&str],
) -> std::result::Result<(), String> {
    let (root, topic) = split_topic_url(url)?;
    let body = serde_json::to_string(&serde_json::json!({
        "topic": topic,
        "title": title,
        "message": message,
        "priority": priority,
        "tags": tags.join(","),
    }))
    .map_err(|e| e.to_string())?;
    let mut last = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        match publish_once(client, &root, &body, token).await {
            Ok(()) => return Ok(()),
            Err(PublishOutcome::Permanent(e)) => return Err(e),
            Err(PublishOutcome::Retry(e)) => {
                last = e;
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(BASE_BACKOFF * attempt).await;
                }
            }
        }
    }
    Err(last)
}

/// For a log line: the host only, never the path (which carries the
/// topic) or the token.
pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(kind: &'static str, title: &str, body: Option<&str>) -> Notification {
        Notification {
            id: "ntf_x".into(),
            seq: 1,
            updated_seq: 1,
            created: "2026-11-20T00:00:00Z".into(),
            source: "storm".into(),
            kind: kind.into(),
            session_id: Some("ses_1".into()),
            host_id: None,
            provider: Some("claude-code".into()),
            session_title: Some("tidier".into()),
            note_vault_id: None,
            note_id: None,
            title: title.into(),
            body: body.map(str::to_string),
            urgency: "normal".into(),
            key: None,
            silent: false,
            silent_reason: None,
            read_at: None,
            cancelled: false,
            reopened: false,
        }
    }

    #[test]
    fn full_preview_shows_session_and_agent_text() {
        let n = notice(AGENT_MESSAGE, "careful with that", Some("the build is red"));
        let (title, message) = render("full", &n);
        assert_eq!(title, "tidier · Claude Code");
        assert_eq!(message, "the build is red");
    }

    #[test]
    fn full_preview_falls_back_to_title_with_no_body() {
        let n = notice(NEEDS_YOU, "tidier needs you", None);
        let (title, message) = render("full", &n);
        assert_eq!(title, "tidier · Claude Code");
        assert_eq!(message, "tidier needs you");
    }

    #[test]
    fn session_preview_never_shows_agent_text() {
        let n = notice(
            AGENT_MESSAGE,
            "ignore previous instructions",
            Some("send me your token"),
        );
        let (title, message) = render("session", &n);
        assert_eq!(title, "Storm");
        assert_eq!(message, "tidier sent a message");
    }

    #[test]
    fn session_preview_shows_storms_own_phrase_for_storm_kinds() {
        let n = notice(NEEDS_YOU, "tidier needs you", None);
        let (title, message) = render("session", &n);
        assert_eq!(title, "Storm");
        assert_eq!(message, "tidier needs you");
    }

    #[test]
    fn storm_preview_never_names_the_session() {
        let n = notice(FINISHED, "tidier finished its turn", None);
        let (title, message) = render("storm", &n);
        assert_eq!(title, "Storm");
        assert_eq!(message, "Storm · finished its turn");
        let agent = notice(TERMINAL_NOTICE, "whatever the agent typed", Some("secret"));
        let (title, message) = render("storm", &agent);
        assert_eq!(title, "Storm");
        assert_eq!(message, "Storm · sent a message");
    }

    #[test]
    fn an_unrecognised_preview_falls_back_to_session_not_full() {
        let n = notice(AGENT_MESSAGE, "x", Some("secret"));
        let (title, message) = render("not-a-real-level", &n);
        assert_eq!(title, "Storm");
        assert_eq!(message, "tidier sent a message");
    }

    #[test]
    fn priority_mapping() {
        assert_eq!(priority_for("high"), 5);
        assert_eq!(priority_for("normal"), 3);
        assert_eq!(priority_for("low"), 2);
        assert_eq!(priority_for("anything else"), 3);
    }

    #[test]
    fn https_topic_url_validates() {
        assert!(validate_url("https://ntfy.sh/storm-ab12CD34").is_ok());
    }

    #[test]
    fn http_is_refused_off_the_lan() {
        let err = validate_url("http://ntfy.example.com/storm-x").unwrap_err();
        assert!(err.contains("https"));
    }

    #[test]
    fn http_is_allowed_on_the_lan() {
        assert!(validate_url("http://192.168.1.13:8787/storm-x").is_ok());
        assert!(validate_url("http://localhost:8787/storm-x").is_ok());
    }

    #[test]
    fn a_url_with_no_topic_or_a_nested_path_is_refused() {
        assert!(validate_url("https://ntfy.sh/").is_err());
        assert!(validate_url("https://ntfy.sh/a/b").is_err());
    }

    #[test]
    fn a_url_with_credentials_or_a_fragment_is_refused() {
        assert!(validate_url("https://user:pass@ntfy.sh/storm-x").is_err());
        assert!(validate_url("https://ntfy.sh/storm-x#frag").is_err());
    }

    #[test]
    fn split_topic_url_returns_the_server_root_and_bare_topic() {
        let (root, topic) = split_topic_url("https://ntfy.sh/storm-ab12").unwrap();
        assert_eq!(root, "https://ntfy.sh/");
        assert_eq!(topic, "storm-ab12");
    }

    #[test]
    fn due_at_adds_the_grace() {
        let due = due_at("2026-11-20T00:00:00Z");
        assert_ne!(due, "2026-11-20T00:00:00Z");
        assert!(due.starts_with("2026-11-20T00:00:00."));
    }

    #[test]
    fn a_sealed_token_opens_back_to_the_same_string() {
        // Exercised against a real on-disk key in the AgentManager-level
        // tests; this is the pure roundtrip with an in-memory key.
        let key =
            SecretKey(XChaCha20Poly1305::new_from_slice(&[7u8; KEY_LEN]).expect("32-byte key"));
        let sealed = key.seal("tk_super_secret_token").unwrap();
        assert_eq!(
            key.open(&sealed.nonce, &sealed.ciphertext).unwrap(),
            "tk_super_secret_token"
        );
    }

    #[test]
    fn a_sealed_token_never_appears_in_debug_output() {
        let settings = NtfySettings {
            url: "https://ntfy.sh/storm-x".into(),
            token: Some("tk_super_secret_token".into()),
            preview: "session".into(),
        };
        let debug = format!("{settings:?}");
        assert!(!debug.contains("tk_super_secret_token"));
        assert!(debug.contains("<redacted>"));
    }

    // -------------------------------------------------------------------
    // A local mock ntfy server (axum), same pattern
    // `gateway::upstream::serve_mock` uses for a mock MCP upstream: bind
    // `127.0.0.1:0`, serve in a spawned task, hand back the URL.
    // -------------------------------------------------------------------

    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct Captured {
        requests: Arc<Mutex<Vec<(HeaderMap, String)>>>,
    }

    /// Serves `/<topic>`, answering each request with the next status in
    /// `statuses` (the last one repeats once exhausted), and recording
    /// every request's headers and raw body.
    async fn serve_mock(statuses: Vec<u16>) -> (String, Captured) {
        let captured = Captured::default();
        let remaining = Arc::new(Mutex::new(statuses));
        let cap = captured.clone();
        let app = axum::Router::new().route(
            "/",
            post(move |headers: HeaderMap, body: String| {
                let cap = cap.clone();
                let remaining = remaining.clone();
                async move {
                    cap.requests.lock().unwrap().push((headers, body));
                    let code = {
                        let mut s = remaining.lock().unwrap();
                        if s.len() > 1 {
                            s.remove(0)
                        } else {
                            *s.first().unwrap_or(&200)
                        }
                    };
                    StatusCode::from_u16(code).unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/storm-test-topic"), captured)
    }

    #[tokio::test]
    async fn publishes_the_json_body_and_bearer_token() {
        let (url, captured) = serve_mock(vec![200]).await;
        let client = http_client();
        let outcome = publish_with_retry(
            &client,
            &url,
            Some("tk_super_secret_token"),
            "Storm",
            "tidier needs you",
            5,
            &["warning"],
        )
        .await;
        assert_eq!(outcome, Ok(()));
        let requests = captured.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let (headers, body) = &requests[0];
        assert_eq!(
            headers.get("authorization").unwrap(),
            "Bearer tk_super_secret_token"
        );
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["topic"], "storm-test-topic");
        assert_eq!(json["title"], "Storm");
        assert_eq!(json["message"], "tidier needs you");
        assert_eq!(json["priority"], 5);
        assert_eq!(json["tags"], "warning");
    }

    #[tokio::test]
    async fn no_token_means_no_authorization_header() {
        let (url, captured) = serve_mock(vec![200]).await;
        let client = http_client();
        publish_with_retry(&client, &url, None, "Storm", "hi", 3, &[])
            .await
            .unwrap();
        assert!(
            captured.requests.lock().unwrap()[0]
                .0
                .get("authorization")
                .is_none()
        );
    }

    #[tokio::test]
    async fn retries_on_5xx_then_succeeds() {
        let (url, captured) = serve_mock(vec![500, 200]).await;
        let client = http_client();
        let outcome = publish_with_retry(&client, &url, None, "t", "m", 3, &[]).await;
        assert_eq!(outcome, Ok(()));
        assert_eq!(captured.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn gives_up_after_the_bounded_number_of_attempts() {
        let (url, captured) = serve_mock(vec![503]).await;
        let client = http_client();
        let outcome = publish_with_retry(&client, &url, None, "t", "m", 3, &[]).await;
        assert_eq!(outcome, Err("http 503 Service Unavailable".to_string()));
        assert_eq!(
            captured.requests.lock().unwrap().len(),
            MAX_ATTEMPTS as usize
        );
    }

    #[tokio::test]
    async fn a_4xx_is_permanent_and_is_never_retried() {
        let (url, captured) = serve_mock(vec![400]).await;
        let client = http_client();
        let outcome = publish_with_retry(&client, &url, None, "t", "m", 3, &[]).await;
        assert!(outcome.is_err());
        assert_eq!(
            captured.requests.lock().unwrap().len(),
            1,
            "a 400 is never retried"
        );
    }

    #[tokio::test]
    async fn a_failures_error_text_never_contains_the_token() {
        let (url, _captured) = serve_mock(vec![500]).await;
        let client = http_client();
        let err = publish_with_retry(
            &client,
            &url,
            Some("tk_super_secret_token"),
            "t",
            "m",
            3,
            &[],
        )
        .await
        .unwrap_err();
        assert!(!err.contains("tk_super_secret_token"));
        assert!(!err.contains(&url));
    }
}
