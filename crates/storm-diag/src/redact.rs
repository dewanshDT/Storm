//! One shared redaction function for log lines and diagnostics (decision
//! 162c). Both `storm-server` and `storm-runtime` call this — never their
//! own copy — so a secret shape learned once is scrubbed everywhere.
//!
//! The rules are deliberately generous: a line redacted that held nothing
//! sensitive costs a little readability; a secret that slips through is
//! exactly what AC-162-2 exists to catch. Prefer the false positive.

use std::net::IpAddr;
use std::sync::LazyLock;

use regex::Regex;

/// A caller-known sensitive string with no fixed shape a regex could find
/// on its own — a vault's display name, or a note's title or path.
#[derive(Debug, Clone)]
pub struct Sensitive {
    pub value: String,
    pub kind: SensitiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensitiveKind {
    Vault,
    Note,
}

impl SensitiveKind {
    fn mask(self) -> &'static str {
        match self {
            SensitiveKind::Vault => "<vault>",
            SensitiveKind::Note => "<note>",
        }
    }
}

impl Sensitive {
    pub fn vault(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            kind: SensitiveKind::Vault,
        }
    }

    pub fn note(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            kind: SensitiveKind::Note,
        }
    }
}

macro_rules! regex_cell {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("static regex"));
    };
}

// `storm-enroll:v1:<server_url>:<server_id>:<server_pubkey>:<token>`
// (`apps/server/src/auth/hosts.rs::enrollment_string`, `apps/runtime/src/
// identity.rs::ENROLL_PREFIX`) is a single-use secret end to end.
regex_cell!(ENROLL, r"storm-enroll:v1:[^\s\x22']+");

// The bootstrap pairing QR payload (`apps/server/src/auth/pairing.rs`):
// `storm://pair?v=1&sid=...&pk=...&n=...&exp=...&addr=...`. The nonce and
// key in it are exactly as sensitive as the token they mint.
regex_cell!(PAIR_URI, r"storm://pair\?[^\s\x22']+");

// `Authorization: Bearer <stk_/sht_/opaque>` and the bare `Bearer ...`
// form error messages and client logs also print.
regex_cell!(BEARER, r"(?i)\bBearer\s+\S+");

// `Authorization: StormDevice <id>:<secret>` (`apps/server/src/api.rs`).
regex_cell!(STORM_DEVICE, r"(?i)\bStormDevice\s+\S+");

// A bare `stk_`/`sht_` key or token, outside a `Bearer`/`StormDevice`
// header — e.g. printed by itself, or sitting in a URL or a config dump.
regex_cell!(KEY_PREFIXED, r"\b(?:stk|sht)_[A-Za-z0-9_-]+");

// JWT-like: three base64url segments joined by dots.
regex_cell!(
    JWT_LIKE,
    r"\b[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b"
);

// A long run of base64url/hex-alphabet characters: most keys, hashes and
// tokens that reach a log are shaped like this even without a known
// prefix. 24 is short enough to catch a blake3 hex digest (64) and a
// 256-bit key's base64url (43), long enough that ordinary words don't
// trip it. **No `/`**, deliberately: standard base64's padding alphabet
// has one, but so does every path, and this runs before path masking —
// `/home/dewansh/projects/storm` must stay a path, not become a secret.
regex_cell!(LONG_SECRET, r"\b[A-Za-z0-9+_=-]{24,}\b");

// RFC 5322-ish, good enough for log text rather than for validating input.
regex_cell!(EMAIL, r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b");

regex_cell!(
    IPV4,
    r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b"
);

// The common long form, covering both the full 8-group and every
// "::"-compressed shape. It requires either 7 colons or a double colon, so
// it does not fire on an ordinary `HH:MM:SS` timestamp.
//
// Deliberately no `\b` at the edges: `:` is not a word character, so a
// leading `\b` can never match before a bare `::1` that follows a space —
// there is no word/non-word transition there for it to anchor on. Every
// alternative already requires its own colons or hex run, which is enough
// to stop it bleeding into surrounding punctuation.
regex_cell!(
    IPV6,
    concat!(
        r"(?:",
        r"(?:[A-Fa-f0-9]{1,4}:){7}[A-Fa-f0-9]{1,4}",
        r"|(?:[A-Fa-f0-9]{1,4}:){1,7}:",
        r"|(?:[A-Fa-f0-9]{1,4}:){1,6}:[A-Fa-f0-9]{1,4}",
        r"|(?:[A-Fa-f0-9]{1,4}:){1,5}(?::[A-Fa-f0-9]{1,4}){1,2}",
        r"|(?:[A-Fa-f0-9]{1,4}:){1,4}(?::[A-Fa-f0-9]{1,4}){1,3}",
        r"|(?:[A-Fa-f0-9]{1,4}:){1,3}(?::[A-Fa-f0-9]{1,4}){1,4}",
        r"|(?:[A-Fa-f0-9]{1,4}:){1,2}(?::[A-Fa-f0-9]{1,4}){1,5}",
        r"|[A-Fa-f0-9]{1,4}:(?:(?::[A-Fa-f0-9]{1,4}){1,6})",
        r"|:(?:(?::[A-Fa-f0-9]{1,4}){1,7}|:)",
        r")"
    )
);

// A DNS-shaped host named in a URL's authority: `scheme://host[...]`. Kept
// to the URL form on purpose — a bare dotted word (`runtime.toml`,
// `0.18.0`) is not masked, or every config filename and version number in
// the log would be, which is the kind of false positive that makes a
// bundle useless rather than merely cautious.
regex_cell!(
    URL_HOST,
    r"(?i)\b((?:https?|wss?|storm)://)([a-zA-Z0-9.-]+)"
);

// An mDNS name (decision 118's `_storm._tcp`), wherever it appears.
regex_cell!(
    MDNS_HOST,
    r"\b[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.local\b"
);

regex_cell!(HOME_UNIX, r"/(?:home|Users)/[^/\s\x22']+/");
regex_cell!(HOME_WINDOWS, r"C:\\Users\\[^\\\s\x22']+\\");

fn ip_is_lan(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            let seg0 = v6.segments()[0];
            // fc00::/7 (unique local) and fe80::/10 (link-local).
            (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
}

fn redact_addresses(input: &str) -> String {
    let masked = IPV6.replace_all(input, |caps: &regex::Captures| {
        let m = &caps[0];
        match m.parse::<IpAddr>() {
            Ok(ip) if ip_is_lan(&ip) => "<lan>".to_string(),
            Ok(_) => "<ip>".to_string(),
            // Not actually a valid address (the pattern is permissive) —
            // leave it alone rather than mask ordinary text.
            Err(_) => m.to_string(),
        }
    });
    IPV4.replace_all(&masked, |caps: &regex::Captures| {
        let m = &caps[0];
        match m.parse::<IpAddr>() {
            Ok(ip) if ip_is_lan(&ip) => "<lan>".to_string(),
            Ok(_) => "<ip>".to_string(),
            Err(_) => m.to_string(),
        }
    })
    .into_owned()
}

fn redact_hosts(input: &str) -> String {
    let with_urls = URL_HOST.replace_all(input, |caps: &regex::Captures| {
        let scheme = &caps[1];
        let host = &caps[2];
        if host.parse::<IpAddr>().is_ok() || host.eq_ignore_ascii_case("localhost") {
            // Already masked by `redact_addresses`, or loopback by name.
            format!("{scheme}{host}")
        } else {
            format!("{scheme}<host>")
        }
    });
    MDNS_HOST.replace_all(&with_urls, "<host>").into_owned()
}

/// Scrubs `input` of every shape decision 162c says a diagnostics bundle
/// must never carry. `sensitive` is this call site's own list of exact
/// strings it knows are secret — vault names, note titles and paths —
/// which have no shape a generic rule could find.
pub fn redact(input: &str, sensitive: &[Sensitive]) -> String {
    let mut out = input.to_string();

    // Longest value first: "My Notes" must not leave "My <vault>" behind
    // because "Notes" (a shorter, also-registered name) matched first.
    let mut ordered: Vec<&Sensitive> = sensitive.iter().collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(s.value.chars().count()));
    for s in ordered {
        if s.value.is_empty() {
            continue;
        }
        out = out.replace(s.value.as_str(), s.kind.mask());
    }

    out = ENROLL
        .replace_all(&out, "storm-enroll:v1:<redacted>")
        .into_owned();
    out = PAIR_URI
        .replace_all(&out, "storm://pair?<redacted>")
        .into_owned();
    out = BEARER.replace_all(&out, "Bearer <redacted>").into_owned();
    out = STORM_DEVICE
        .replace_all(&out, "StormDevice <redacted>")
        .into_owned();
    out = KEY_PREFIXED.replace_all(&out, "<redacted>").into_owned();
    out = JWT_LIKE.replace_all(&out, "<redacted>").into_owned();
    out = LONG_SECRET.replace_all(&out, "<redacted>").into_owned();
    out = redact_addresses(&out);
    out = redact_hosts(&out);
    out = EMAIL.replace_all(&out, "<email>").into_owned();
    out = HOME_UNIX.replace_all(&out, "~/").into_owned();
    out = HOME_WINDOWS.replace_all(&out, r"~\").into_owned();

    out
}

/// [`redact`] with no caller-supplied names — the ring buffer's default,
/// since a log line is redacted before anyone holding a vault or note name
/// is in scope to supply one.
pub fn redact_default(input: &str) -> String {
    redact(input, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> String {
        redact_default(s)
    }

    fn clean(s: &str) -> bool {
        !s.contains("secret") && !s.contains("SECRET")
    }

    #[test]
    fn bearer_token_is_redacted() {
        let out = r("Authorization: Bearer stk_abcdef0123456789ABCDEF");
        assert!(!out.contains("stk_abcdef0123456789ABCDEF"));
        assert!(out.contains("Bearer <redacted>"));
    }

    #[test]
    fn storm_device_header_is_redacted() {
        let out = r("Authorization: StormDevice dev_survivor1:supersecretdevicesecret123");
        assert!(!out.contains("supersecretdevicesecret123"));
        assert!(out.contains("StormDevice <redacted>"));
    }

    #[test]
    fn bare_stk_key_is_redacted() {
        let out = r("mcp key=stk_fromfileSUPERSECRETVALUE in config");
        assert!(!out.contains("stk_fromfileSUPERSECRETVALUE"));
    }

    #[test]
    fn bare_sht_token_is_redacted() {
        let out = r("host token sht_0123456789abcdefSECRET presented");
        assert!(!out.contains("sht_0123456789abcdefSECRET"));
    }

    #[test]
    fn enrollment_string_is_redacted() {
        let out =
            r("storm-enroll:v1:http://192.168.1.10:8484:srv_abc123:pk_deadbeef:tok_SUPERSECRET01");
        assert!(!out.contains("tok_SUPERSECRET01"));
        assert!(!out.contains("192.168.1.10"));
        assert!(out.starts_with("storm-enroll:v1:<redacted>"));
    }

    #[test]
    fn pairing_uri_is_redacted() {
        let out = r(
            "storm://pair?v=1&sid=srv_x&pk=verysecretkeyvalue&n=noncevalue123&exp=e&addr=192.168.1.5:8484",
        );
        assert!(!out.contains("verysecretkeyvalue"));
        assert!(!out.contains("192.168.1.5"));
    }

    #[test]
    fn jwt_like_value_is_redacted() {
        let out = r(
            "token=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJzZWNyZXQtcGF5bG9hZCJ9.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        );
        assert!(clean(&out));
        assert!(!out.contains("eyJzdWIiOiJzZWNyZXQtcGF5bG9hZCJ9"));
    }

    #[test]
    fn long_base64_secret_is_redacted() {
        let out = r("session key: 8f3a9c2e7b1d4f6a0e5c8b2d7a4f1e9c6b3a0d8f");
        assert!(!out.contains("8f3a9c2e7b1d4f6a0e5c8b2d7a4f1e9c6b3a0d8f"));
    }

    #[test]
    fn email_is_redacted() {
        let out = r("reply_to: dewansh.dt@gmail.com");
        assert!(!out.contains("dewansh.dt@gmail.com"));
        assert!(out.contains("<email>"));
    }

    #[test]
    fn loopback_and_lan_ipv4_keep_shape_as_lan() {
        assert_eq!(r("connect 127.0.0.1:8484"), "connect <lan>:8484");
        assert_eq!(r("peer 192.168.1.42 joined"), "peer <lan> joined");
        assert_eq!(r("peer 10.0.0.5 joined"), "peer <lan> joined");
        assert_eq!(r("peer 172.16.3.9 joined"), "peer <lan> joined");
    }

    #[test]
    fn public_ipv4_is_fully_redacted() {
        let out = r("peer 203.0.113.9 joined");
        assert!(!out.contains("203.0.113.9"));
        assert!(out.contains("<ip>"));
    }

    #[test]
    fn loopback_and_lan_ipv6_keep_shape_as_lan() {
        assert_eq!(r("connect ::1"), "connect <lan>");
        assert!(r("peer fe80::1ff:fe23:4567:890a seen").contains("<lan>"));
        assert!(r("peer fd12:3456:789a:1::1 seen").contains("<lan>"));
    }

    #[test]
    fn public_ipv6_is_fully_redacted() {
        let out = r("peer 2001:db8::ff00:42:8329 joined");
        assert!(!out.contains("2001:db8"));
        assert!(out.contains("<ip>"));
    }

    #[test]
    fn plain_timestamp_is_not_treated_as_an_address() {
        // Two colons, no "::": must survive untouched.
        assert_eq!(r("[14:23:05] started"), "[14:23:05] started");
    }

    #[test]
    fn url_hostname_is_masked_but_scheme_and_port_survive() {
        let out = r("dialing http://my-server.example.com:8484/v1/health");
        assert_eq!(out, "dialing http://<host>:8484/v1/health");
    }

    #[test]
    fn mdns_hostname_is_masked() {
        let out = r("advertising as Dewansh-Laptop.local");
        assert_eq!(out, "advertising as <host>");
    }

    #[test]
    fn config_filenames_are_not_mistaken_for_hostnames() {
        // The URL/mDNS-only host rule must not eat ordinary dotted words.
        assert_eq!(r("loaded runtime.toml"), "loaded runtime.toml");
        assert_eq!(r("state/auth.db opened"), "state/auth.db opened");
    }

    #[test]
    fn home_paths_are_masked_on_every_platform() {
        assert_eq!(
            r("workspace /home/dewansh/projects/storm"),
            "workspace ~/projects/storm"
        );
        assert_eq!(
            r("workspace /Users/dewansh/projects/storm"),
            "workspace ~/projects/storm"
        );
        assert_eq!(
            r(r"workspace C:\Users\dewansh\projects\storm"),
            r"workspace ~\projects\storm"
        );
    }

    #[test]
    fn vault_and_note_names_are_masked_when_supplied() {
        let sensitive = vec![
            Sensitive::vault("My Notes"),
            Sensitive::note("Secret Plan.md"),
        ];
        let out = redact("opened My Notes/Secret Plan.md for editing", &sensitive);
        assert_eq!(out, "opened <vault>/<note> for editing");
    }

    #[test]
    fn longer_sensitive_name_wins_over_a_shorter_overlapping_one() {
        let sensitive = vec![Sensitive::vault("Notes"), Sensitive::vault("My Notes")];
        let out = redact("in My Notes today", &sensitive);
        assert_eq!(out, "in <vault> today");
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        let out = r("session started, 3 agents running, all idle");
        assert_eq!(out, "session started, 3 agents running, all idle");
    }
}
