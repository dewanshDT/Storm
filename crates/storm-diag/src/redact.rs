//! Redaction for log lines and diagnostics (decision 162c): one function,
//! shared by `storm-server` and `storm-runtime`.

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

regex_cell!(ENROLL, r"storm-enroll:v1:[^\s\x22']+");
regex_cell!(PAIR_URI, r"storm://pair\?[^\s\x22']+");
regex_cell!(BEARER, r"(?i)\bBearer\s+\S+");
regex_cell!(STORM_DEVICE, r"(?i)\bStormDevice\s+\S+");
regex_cell!(KEY_PREFIXED, r"\b(?:stk|sht)_[A-Za-z0-9_-]+");
regex_cell!(
    JWT_LIKE,
    r"\b[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b"
);

// No `/`: that's also in every path, and this runs before path masking.
regex_cell!(LONG_SECRET, r"\b[A-Za-z0-9+_=-]{24,}\b");

regex_cell!(EMAIL, r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b");

regex_cell!(
    IPV4,
    r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b"
);

// No leading `\b`: `:` isn't a word char, so it can't anchor before `::1`.
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

// URL form only — a bare dotted word (`runtime.toml`) must not match.
regex_cell!(
    URL_HOST,
    r"(?i)\b((?:https?|wss?|storm)://)([a-zA-Z0-9.-]+)"
);
regex_cell!(
    MDNS_HOST,
    r"\b[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.local\b"
);

regex_cell!(HOME_UNIX, r"/(?:home|Users)/[^/\s\x22']+/");
regex_cell!(HOME_WINDOWS, r"C:\\Users\\[^\\\s\x22']+\\");

regex_cell!(
    NOTE_OR_FILE_KV,
    r"(?i)\b(path|note|file)=([^\s\x22']+\.(?:md|png|jpe?g|gif|webp|pdf|svg))\b"
);
regex_cell!(
    NOTE_OR_FILE_SHAPE,
    r"[^\s\x22']*/[^\s\x22']*\.(?:md|png|jpe?g|gif|webp|pdf|svg)\b"
);

regex_cell!(
    PROJECT_DIR,
    r"[^\s\x22']*[/\\](workspaces|worktrees)[/\\]([^/\\\s\x22']+)"
);

fn mask_for_ext(whole: &str) -> &'static str {
    if whole.to_ascii_lowercase().ends_with(".md") {
        "<note>"
    } else {
        "<file>"
    }
}

fn ip_is_lan(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            let seg0 = v6.segments()[0];
            (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
}

fn redact_addresses(input: &str) -> String {
    let mask = |caps: &regex::Captures| {
        let m = &caps[0];
        match m.parse::<IpAddr>() {
            Ok(ip) if ip_is_lan(&ip) => "<lan>".to_string(),
            Ok(_) => "<ip>".to_string(),
            Err(_) => m.to_string(),
        }
    };
    let masked = IPV6.replace_all(input, mask);
    IPV4.replace_all(&masked, mask).into_owned()
}

fn redact_hosts(input: &str) -> String {
    let with_urls = URL_HOST.replace_all(input, |caps: &regex::Captures| {
        let scheme = &caps[1];
        let host = &caps[2];
        if host.parse::<IpAddr>().is_ok() || host.eq_ignore_ascii_case("localhost") {
            format!("{scheme}{host}")
        } else {
            format!("{scheme}<host>")
        }
    });
    MDNS_HOST.replace_all(&with_urls, "<host>").into_owned()
}

fn redact_paths(input: &str) -> String {
    let with_kv = NOTE_OR_FILE_KV.replace_all(input, |caps: &regex::Captures| {
        format!("{}={}", &caps[1], mask_for_ext(&caps[2]))
    });
    NOTE_OR_FILE_SHAPE
        .replace_all(&with_kv, |caps: &regex::Captures| {
            mask_for_ext(&caps[0]).to_string()
        })
        .into_owned()
}

fn redact_project_dirs(input: &str) -> String {
    PROJECT_DIR
        .replace_all(input, |caps: &regex::Captures| {
            format!("/{}/<ws>", &caps[1])
        })
        .into_owned()
}

/// Scrubs `input` of every shape decision 162c says a diagnostics bundle
/// must never carry. `sensitive` is this call site's own list of exact
/// strings it knows are secret — vault names, note titles and paths.
pub fn redact(input: &str, sensitive: &[Sensitive]) -> String {
    let mut out = input.to_string();

    // Longest first: a shorter registered name must not leave a partial
    // mask inside a longer one that contains it.
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
    out = redact_paths(&out);
    out = LONG_SECRET.replace_all(&out, "<redacted>").into_owned();
    out = redact_addresses(&out);
    out = redact_hosts(&out);
    out = EMAIL.replace_all(&out, "<email>").into_owned();
    out = redact_project_dirs(&out);
    out = HOME_UNIX.replace_all(&out, "~/").into_owned();
    out = HOME_WINDOWS.replace_all(&out, r"~\").into_owned();

    out
}

/// [`redact`] with no caller-supplied names — the ring buffer's default.
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
        let out = r("reply_to: alice@example.com");
        assert!(!out.contains("alice@example.com"));
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
        assert_eq!(r("[14:23:05] started"), "[14:23:05] started");
    }

    #[test]
    fn url_hostname_is_masked_but_scheme_and_port_survive() {
        let out = r("dialing http://my-server.example.com:8484/v1/health");
        assert_eq!(out, "dialing http://<host>:8484/v1/health");
    }

    #[test]
    fn mdns_hostname_is_masked() {
        let out = r("advertising as Example-Laptop.local");
        assert_eq!(out, "advertising as <host>");
    }

    #[test]
    fn config_filenames_are_not_mistaken_for_hostnames() {
        assert_eq!(r("loaded runtime.toml"), "loaded runtime.toml");
        assert_eq!(r("state/auth.db opened"), "state/auth.db opened");
    }

    #[test]
    fn home_paths_are_masked_on_every_platform() {
        assert_eq!(
            r("workspace /home/alice/projects/storm"),
            "workspace ~/projects/storm"
        );
        assert_eq!(
            r("workspace /Users/alice/projects/storm"),
            "workspace ~/projects/storm"
        );
        assert_eq!(
            r(r"workspace C:\Users\alice\projects\storm"),
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

    #[test]
    fn note_path_after_key_is_masked() {
        assert_eq!(r("path=Projects/Ideas.md"), "path=<note>");
        assert_eq!(r("note=Ideas.md saved"), "note=<note> saved");
    }

    #[test]
    fn attachment_path_after_key_is_masked_as_file() {
        assert_eq!(r("file=Storm/Feedback/x.png"), "file=<file>");
    }

    #[test]
    fn bare_note_or_attachment_shaped_token_is_masked() {
        assert_eq!(r("wrote to Storm/Feedback/x.png"), "wrote to <file>");
        assert_eq!(r("opened Projects/Ideas.md"), "opened <note>");
    }

    #[test]
    fn config_filenames_with_no_slash_are_not_masked() {
        assert_eq!(r("loaded runtime.toml"), "loaded runtime.toml");
        assert_eq!(r("note.md.bak ignored"), "note.md.bak ignored");
    }

    #[test]
    fn workspace_segment_is_masked_regardless_of_the_prefix() {
        assert_eq!(
            r("in /var/lib/storm-runtime/workspaces/acme-app/src/main.rs"),
            "in /workspaces/<ws>/src/main.rs"
        );
        assert_eq!(
            r("in /home/alice/workspaces/acme-app"),
            "in /workspaces/<ws>"
        );
    }

    #[test]
    fn worktree_segment_is_masked_regardless_of_the_prefix() {
        assert_eq!(
            r(".claude/worktrees/162-1/apps/server"),
            "/worktrees/<ws>/apps/server"
        );
    }
}
