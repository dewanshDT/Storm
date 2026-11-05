//! Which credential an isolated session may start on (decision 136f).
//!
//! On a host with `isolation = "per-session"` a provider's credential
//! reaches a session only by a path that cannot rotate the account's
//! tokens out from under another session:
//!
//! - Claude Code's synced token (decision 124), in that session's
//!   environment only;
//! - a provider's API key from the host's env file, in the environment
//!   only;
//! - a static API-key entry in a provider's own store, which the helper
//!   copies read-only into the session's home (`isolation/src/home.rs`).
//!
//! A refreshable login — Claude Code's `/login`, Codex's ChatGPT login,
//! OpenCode's OAuth entries — is never one of them. A profile with no
//! permitted path for this host's setup is refused with
//! `credential_unavailable` and the reason, never started on the account's
//! own login.

use std::path::Path;

use serde_json::Value;

/// Taken out of every isolated session's environment, even from the env
/// file: a refreshable login (136f). The helper drops them too.
pub const REFRESHABLE_ENV: [&str; 1] = ["CLAUDE_CODE_OAUTH_REFRESH_TOKEN"];

/// A session id as the server mints it (`ags_` and up to 64
/// alphanumerics), as the helper checks it before it names a directory.
pub fn session_id_ok(id: &str) -> bool {
    id.strip_prefix("ags_").is_some_and(|rest| {
        (1..=64).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// The variables a provider reads its key from, per profile.
pub fn env_keys(provider: &str) -> &'static [&'static str] {
    match provider {
        // A setup-token in the env file is long-lived, like a synced one.
        "claude-code" => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
        ],
        "codex" => &["OPENAI_API_KEY", "CODEX_API_KEY"],
        "cursor" => &["CURSOR_API_KEY"],
        _ => &[],
    }
}

/// What a store holds, as far as 136f cares.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StoreKinds {
    pub static_key: bool,
    pub refreshable: bool,
}

/// OpenCode's `auth.json`: `{"type": "api"}` entries are static keys;
/// `oauth` and `wellknown` ones are logins.
pub fn opencode_store(bytes: &[u8]) -> StoreKinds {
    let mut out = StoreKinds::default();
    if let Ok(Value::Object(all)) = serde_json::from_slice::<Value>(bytes) {
        for entry in all.values() {
            if entry["type"] == "api" && entry["key"].as_str().is_some_and(|k| !k.is_empty()) {
                out.static_key = true;
            } else {
                out.refreshable = true;
            }
        }
    }
    out
}

/// Codex's `auth.json`: an `OPENAI_API_KEY` with no ChatGPT `tokens` is a
/// static key; `tokens` is a login.
pub fn codex_store(bytes: &[u8]) -> StoreKinds {
    let Ok(Value::Object(all)) = serde_json::from_slice::<Value>(bytes) else {
        return StoreKinds::default();
    };
    let refreshable = all.get("tokens").is_some_and(|t| !t.is_null());
    let key = all
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|k| !k.is_empty());
    StoreKinds {
        static_key: key && !refreshable,
        refreshable,
    }
}

/// What the check reads.
pub struct Setup<'a> {
    pub provider: &'a str,
    /// The launch pulls a synced token (decision 124).
    pub synced: bool,
    /// Whether the provider's env file sets a variable, non-empty.
    pub env_set: &'a dyn Fn(&str) -> bool,
    /// Whether it sets any `*_API_KEY` (OpenCode reads many).
    pub any_api_key: bool,
    /// The host account's home.
    pub account_home: &'a Path,
    /// Storm's Codex home (`codex::home`).
    pub codex_home: &'a Path,
}

/// `Ok` when the session has a permitted path; the reason otherwise.
pub fn check(s: &Setup) -> Result<(), &'static str> {
    let env = env_keys(s.provider).iter().any(|k| (s.env_set)(k));
    match s.provider {
        "claude-code" => {
            if s.synced || env {
                Ok(())
            } else {
                Err(
                    "Claude Code's own login is refreshable and is not used on an isolated host; \
                     sync a token or set an API key in its env file",
                )
            }
        }
        "codex" => {
            let store = std::fs::read(s.codex_home.join("auth.json"))
                .map(|b| codex_store(&b))
                .unwrap_or_default();
            if env || store.static_key {
                Ok(())
            } else {
                Err(
                    "Codex's ChatGPT login is refreshable and is not used on an isolated host; \
                     set OPENAI_API_KEY in its env file or log Codex in with an API key",
                )
            }
        }
        "opencode" => {
            let store = std::fs::read(s.account_home.join(".local/share/opencode/auth.json"))
                .map(|b| opencode_store(&b))
                .unwrap_or_default();
            let any_env = s.any_api_key;
            // OpenCode's free models need no credential at all; only a host
            // whose one login is OAuth has nothing to start on.
            if store.refreshable && !store.static_key && !any_env {
                Err(
                    "OpenCode's OAuth logins are refreshable and are not used on an isolated host; \
                     add an API key to its env file or to `opencode auth login`",
                )
            } else {
                Ok(())
            }
        }
        "cursor" => {
            if env {
                Ok(())
            } else {
                Err(
                    "Cursor's login is refreshable and is not used on an isolated host; \
                     set CURSOR_API_KEY in its env file",
                )
            }
        }
        "command-code" => {
            Err("Command Code has no credential path an isolated session may use (decision 136f)")
        }
        _ => Ok(()),
    }
}

/// Whether `env` has a variable ending in `_API_KEY` with a value, for
/// OpenCode's many providers.
pub fn any_api_key(env: &[(std::ffi::OsString, std::ffi::OsString)]) -> bool {
    env.iter()
        .any(|(k, v)| k.to_str().is_some_and(|k| k.ends_with("_API_KEY")) && !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup<'a>(
        provider: &'a str,
        synced: bool,
        env: &'a dyn Fn(&str) -> bool,
        home: &'a Path,
    ) -> Setup<'a> {
        Setup {
            provider,
            synced,
            env_set: env,
            any_api_key: env("*_API_KEY"),
            account_home: home,
            codex_home: home,
        }
    }

    /// IT-5's half the host decides: a refreshable login is never a reason
    /// to start, and what 136f permits always is.
    #[test]
    fn it_5_only_136fs_paths_start_an_isolated_session() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let none = |_: &str| false;
        let key = |k: &str| k == "ANTHROPIC_API_KEY";
        assert!(check(&setup("claude-code", true, &none, home)).is_ok());
        assert!(check(&setup("claude-code", false, &key, home)).is_ok());
        assert!(check(&setup("claude-code", false, &none, home)).is_err());
        // The account's own /login is there and changes nothing.
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/.credentials.json"),
            b"{\"claudeAiOauth\":{}}",
        )
        .unwrap();
        assert!(check(&setup("claude-code", false, &none, home)).is_err());

        std::fs::write(
            home.join("auth.json"),
            br#"{"OPENAI_API_KEY":null,"tokens":{"refresh_token":"r"}}"#,
        )
        .unwrap();
        assert!(
            check(&setup("codex", false, &none, home)).is_err(),
            "a ChatGPT login"
        );
        let openai = |k: &str| k == "OPENAI_API_KEY";
        assert!(check(&setup("codex", false, &openai, home)).is_ok());
        std::fs::write(home.join("auth.json"), br#"{"OPENAI_API_KEY":"sk-1"}"#).unwrap();
        assert!(
            check(&setup("codex", false, &none, home)).is_ok(),
            "a static key"
        );

        assert!(
            check(&setup("opencode", false, &none, home)).is_ok(),
            "free models"
        );
        let store = home.join(".local/share/opencode");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(
            store.join("auth.json"),
            br#"{"anthropic":{"type":"oauth","refresh":"r"}}"#,
        )
        .unwrap();
        assert!(
            check(&setup("opencode", false, &none, home)).is_err(),
            "OAuth only"
        );
        let any = |k: &str| k == "*_API_KEY";
        assert!(check(&setup("opencode", false, &any, home)).is_ok());
        std::fs::write(
            store.join("auth.json"),
            br#"{"anthropic":{"type":"oauth","refresh":"r"},"openai":{"type":"api","key":"k"}}"#,
        )
        .unwrap();
        assert!(check(&setup("opencode", false, &none, home)).is_ok());

        assert!(check(&setup("cursor", false, &none, home)).is_err());
        let cursor = |k: &str| k == "CURSOR_API_KEY";
        assert!(check(&setup("cursor", false, &cursor, home)).is_ok());
        assert!(check(&setup("command-code", false, &none, home)).is_err());
        assert!(check(&setup("shell", false, &none, home)).is_ok());
    }

    #[test]
    fn stores_are_read_as_136f_counts_them() {
        assert_eq!(
            opencode_store(
                br#"{"a":{"type":"api","key":"k"},"b":{"type":"wellknown","key":"k","token":"t"}}"#
            ),
            StoreKinds {
                static_key: true,
                refreshable: true
            }
        );
        assert_eq!(opencode_store(b"nonsense"), StoreKinds::default());
        assert_eq!(
            codex_store(br#"{"OPENAI_API_KEY":"sk","tokens":{"id_token":"x"}}"#),
            StoreKinds {
                static_key: false,
                refreshable: true
            }
        );
        assert!(any_api_key(&[("GROQ_API_KEY".into(), "g".into())]));
        assert!(!any_api_key(&[
            ("GROQ_API_KEY".into(), "".into()),
            ("TOKEN".into(), "t".into())
        ]));
    }
}
