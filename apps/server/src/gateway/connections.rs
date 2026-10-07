//! Connections: the rows, their vocabulary and their input rules (spec §5,
//! decision 81c). No policy here — who may do what is `ops.rs`'s.

use serde::{Deserialize, Serialize};

/// The built-in connection's id and slug (spec §5). It is not a row: it
/// exists for every owner, cannot be deleted, and has no credential. Its id is
/// its slug so the host's `start.mcp` and the gateway route can name it the
/// same way as any `mcc_` connection.
pub const BUILTIN_ID: &str = "storm";
pub const BUILTIN_SLUG: &str = "storm";

pub mod status {
    /// An `oauth` connection before its first authorization (81g).
    pub const PENDING_AUTH: &str = "pending_auth";
    pub const CONNECTED: &str = "connected";
    pub const NEEDS_REAUTH: &str = "needs_reauth";
    pub const ERROR: &str = "error";
    pub const DISABLED: &str = "disabled";
    /// The tombstone a disconnect leaves (§13). Never listed, never callable.
    pub const REVOKED: &str = "revoked";
}

pub mod auth_kind {
    pub const OAUTH: &str = "oauth";
    pub const STATIC: &str = "static";
    pub const NONE: &str = "none";
}

/// The credential kinds a `credentials` row can hold; each is also the AAD's
/// second half.
pub mod credential_kind {
    pub const STATIC: &str = "static";
    pub const OAUTH_TOKENS: &str = "oauth_tokens";
}

/// A connection row. Carries no credential: those are only ever in
/// `credentials`, sealed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Connection {
    pub id: String,
    pub owner_user_id: String,
    pub slug: String,
    pub display_name: String,
    pub url: String,
    pub auth_kind: String,
    pub status: String,
    pub tool_allowlist: Vec<String>,
    /// Every tool name seen the last time the upstream was listed; `None`
    /// until it has been. A tool not in here when it first appears defaults
    /// off (G-D16).
    pub known_tools: Option<Vec<String>>,
    pub expose_resources: bool,
    pub expose_prompts: bool,
    pub upstream_account_label: Option<String>,
    pub last_ok: Option<String>,
    pub last_error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A static-header credential (G-D24: a GitHub PAT is one). Sealed as JSON
/// under kind `static`. Its `Debug` never prints the value.
#[derive(Clone, Serialize, Deserialize)]
pub struct StaticCredential {
    #[serde(default = "default_header")]
    pub header: String,
    pub value: String,
}

fn default_header() -> String {
    "Authorization".into()
}

impl std::fmt::Debug for StaticCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticCredential")
            .field("header", &self.header)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// A slug is the server name an agent sees and the prefix of OpenCode's
/// `"<slug>_*": "ask"` rule (AM32), so it is `[a-z0-9-]`, 1–32 characters,
/// not starting or ending with `-`. `_` would let one slug's glob match
/// another's tools. `storm` belongs to the built-in connection.
pub fn validate_slug(slug: &str) -> Result<(), &'static str> {
    let ok = (1..=32).contains(&slug.len())
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err("a slug is 1–32 lowercase letters, digits and inner '-'");
    }
    if slug == BUILTIN_SLUG {
        return Err("'storm' is the built-in connection's slug");
    }
    Ok(())
}

/// A slug from a display name: lowercase, runs of anything else become one
/// `-`, trimmed, cut to 32. May still fail [`validate_slug`] (an empty or
/// reserved result), which the caller reports.
pub fn slug_from(display_name: &str) -> String {
    let mut out = String::new();
    for c in display_name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut out: String = out.chars().take(32).collect();
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn validate_display_name(name: &str) -> Result<(), &'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("a display name is 1–80 characters, without control characters");
    }
    Ok(())
}

/// An upstream URL (spec §5, G-D15): absolute `https`, with a host, no
/// userinfo (a credential in a URL is a credential in every log and every
/// list), and no fragment.
///
/// **Not here: the private-range rule.** §10 applies it to OAuth discovery,
/// where the URLs come from an upstream's metadata and are not the owner's.
/// A static connection's URL is typed by the owner, who may run their own MCP
/// server on the LAN; refusing a private address there would protect the
/// owner from themselves and break a homelab's main use. (decision 81c)
///
/// `allow_http` exists for the test suites only (`serve
/// --gateway-allow-http-upstreams`, hidden): a mock upstream on loopback has
/// no certificate. Nothing in a real deployment sets it.
pub fn validate_url(url: &str, allow_http: bool) -> Result<(), &'static str> {
    if url.len() > 2048 || url.chars().any(|c| c.is_control() || c == ' ') {
        return Err("the URL is too long or contains spaces or control characters");
    }
    let uri: axum::http::Uri = url.parse().map_err(|_| "the URL does not parse")?;
    let scheme_ok = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => allow_http,
        _ => false,
    };
    if !scheme_ok {
        return Err("an integration URL must be https");
    }
    let Some(authority) = uri.authority() else {
        return Err("the URL has no host");
    };
    if authority.as_str().contains('@') {
        return Err("the URL must not carry a username or password");
    }
    if authority.host().is_empty() {
        return Err("the URL has no host");
    }
    if url.contains('#') {
        return Err("the URL must not have a fragment");
    }
    Ok(())
}

/// Headers the HTTP client or the MCP transport owns. A static credential
/// naming one would either be overwritten or break the session.
const RESERVED_HEADERS: &[&str] = &[
    "accept",
    "connection",
    "content-length",
    "content-type",
    "host",
    "last-event-id",
    "mcp-protocol-version",
    "mcp-session-id",
    "te",
    "transfer-encoding",
    "upgrade",
];

/// A static credential: a header name (`Authorization` by default) and a
/// value of printable ASCII, no CR or LF (no header injection), 1–8192 bytes.
pub fn validate_static(header: &str, value: &str) -> Result<(), &'static str> {
    let name_ok = (1..=64).contains(&header.len())
        && header
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !name_ok {
        return Err("a header name is 1–64 letters, digits and '-'");
    }
    if RESERVED_HEADERS.contains(&header.to_ascii_lowercase().as_str()) {
        return Err("that header is set by the MCP transport itself");
    }
    let value_ok = (1..=8192).contains(&value.len())
        && value
            .bytes()
            .all(|b| (0x20..0x7f).contains(&b) || b == b'\t')
        && !value.trim().is_empty();
    if !value_ok {
        return Err("a credential value is 1–8192 printable ASCII characters");
    }
    Ok(())
}

/// A tool allowlist: names as the upstream gives them, bounded so a client
/// cannot grow a row without limit.
pub fn validate_allowlist(tools: &[String]) -> Result<(), &'static str> {
    if tools.len() > 1000 {
        return Err("an allowlist names at most 1000 tools");
    }
    if tools
        .iter()
        .any(|t| t.is_empty() || t.len() > 128 || t.chars().any(char::is_control))
    {
        return Err("a tool name is 1–128 characters, without control characters");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A plausible row, for tests elsewhere in the gateway.
    pub(crate) fn connection() -> Connection {
        Connection {
            id: "mcc_TEST".into(),
            owner_user_id: "usr_TEST".into(),
            slug: "test".into(),
            display_name: "Test".into(),
            url: "https://x.example/mcp".into(),
            auth_kind: auth_kind::NONE.into(),
            status: status::CONNECTED.into(),
            tool_allowlist: Vec::new(),
            known_tools: None,
            expose_resources: true,
            expose_prompts: true,
            upstream_account_label: None,
            last_ok: None,
            last_error_code: None,
            created_at: "t".into(),
            updated_at: "t".into(),
        }
    }

    #[test]
    fn slugs_fit_every_place_an_agent_sees_them() {
        for good in ["notion", "linear-2", "gh", "a"] {
            assert!(validate_slug(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "Notion",
            "no_tion",
            "-notion",
            "notion-",
            "n.otion",
            "storm",
            &"x".repeat(33),
        ] {
            assert!(validate_slug(bad).is_err(), "{bad}");
        }
        assert_eq!(slug_from("GitHub (work)"), "github-work");
        assert_eq!(slug_from("  Linear  "), "linear");
        assert_eq!(slug_from("!!!"), "");
        assert!(validate_slug(&slug_from("Storm")).is_err());
    }

    #[test]
    fn an_upstream_url_is_https_with_a_host_and_no_userinfo() {
        assert!(validate_url("https://mcp.notion.com/mcp", false).is_ok());
        assert!(validate_url("http://127.0.0.1:9/mcp", true).is_ok());
        assert!(validate_url("https://api.example.com:8443/mcp/?x=1", false).is_ok());
        // The owner's own LAN MCP server is allowed (decision 81c).
        assert!(validate_url("https://192.168.1.20/mcp", false).is_ok());
        for bad in [
            "http://mcp.notion.com/mcp",
            "mcp.notion.com/mcp",
            "https://user:pass@mcp.example.com/",
            "https://token@mcp.example.com/",
            "https:///mcp",
            "https://mcp.example.com/#frag",
            "https://mcp.example.com/a b",
            "ftp://x/",
        ] {
            assert!(validate_url(bad, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_static_credential_cannot_inject_a_header() {
        assert!(validate_static("Authorization", "Bearer ghp_abc").is_ok());
        assert!(validate_static("X-Api-Key", "k").is_ok());
        assert!(validate_static("Authorization", "Bearer x\r\nHost: evil").is_err());
        assert!(validate_static("Authorization", "").is_err());
        assert!(validate_static("Authorization", "   ").is_err());
        assert!(validate_static("Mcp-Session-Id", "x").is_err());
        assert!(validate_static("HOST", "x").is_err());
        assert!(validate_static("X Api", "x").is_err());
    }

    #[test]
    fn a_static_credential_debug_prints_no_value() {
        let c = StaticCredential {
            header: "Authorization".into(),
            value: "Bearer ghp_secret".into(),
        };
        let printed = format!("{c:?}");
        assert!(!printed.contains("ghp_secret"), "{printed}");
        assert!(printed.contains("Authorization"));
    }
}
