//! The agent's own session id, so Storm can resume the conversation rather
//! than start a new one (decision 89).
//!
//! Claude Code and OpenCode each keep their conversations under an id of
//! their own, per host and per workspace. The host learns it while the
//! session runs, never by reading the terminal:
//!
//! - **`claude-code`**: a fresh session is started with `--session-id <uuid>`,
//!   and every session gets a `SessionStart` hook (through `--settings`, a
//!   file in the session's directory) that
//!   runs `storm-runtime harness-session`. The hook fires on startup, resume,
//!   `/clear` and compaction, with the id the conversation now has.
//! - **`opencode`**: no id can be chosen up front. `OPENCODE_CONFIG_DIR`
//!   points at the session's own directory, which holds a plugin that runs
//!   `storm-runtime harness-session <id>` for the top-level session a message
//!   lands in. The variable adds to the operator's config, never replaces it.
//!
//! `harness-session` sends the id to the daemon's socket with the session's
//! handle, which only that session was given. A resume starts the CLI with
//! `--resume <id>` or `-s <id>`.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::Result;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::mcp::{private_dir, write_private};
use crate::provider::LaunchExtras;

pub const HANDLE_ENV: &str = "STORM_HARNESS_HANDLE";

/// One line from `harness-session` on the daemon socket.
#[derive(Debug, Deserialize, Serialize)]
pub struct HarnessLine {
    pub handle: String,
    pub harness_session: String,
}

/// Whether `id` is a session id `provider` could have made. It becomes an
/// argument, so nothing else is ever accepted. The shapes are the profiles'
/// (decision 130).
pub fn valid_id(provider: &str, id: &str) -> bool {
    crate::profile::valid_id(provider, id)
}

fn new_uuid() -> String {
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = data_encoding::HEXLOWER.encode(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// What a session's launch gains so its id reaches the host, and the id when
/// it is known before the agent starts. `resume` must already be valid for
/// `provider`. Nothing for any other provider. `synced` is merged into Claude
/// Code's settings file and checked there (token sync, AM71).
pub fn launch(
    provider: &str,
    dir: &Path,
    handle: &str,
    socket: &Path,
    exe: &Path,
    resume: Option<&str>,
    synced: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<(LaunchExtras, Option<String>)> {
    let env = vec![
        (HANDLE_ENV.into(), handle.into()),
        (crate::mcp::SOCKET_ENV.into(), socket.as_os_str().to_owned()),
    ];
    match provider {
        "claude-code" => {
            let (mut args, id): (Vec<std::ffi::OsString>, String) = match resume {
                Some(id) => (vec!["--resume".into(), id.into()], id.to_string()),
                None => {
                    let id = new_uuid();
                    (vec!["--session-id".into(), id.clone().into()], id)
                }
            };
            let hook = format!(
                "{} harness-session",
                shell_quote(&exe.display().to_string())
            );
            let mut settings = json!({
                "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": hook}]}]}
            });
            if let Some(pins) = synced {
                settings
                    .as_object_mut()
                    .expect("an object")
                    .extend(pins.clone());
            }
            // A file, not inline JSON: npm installs Claude Code on Windows as
            // `claude.cmd`, and a batch file's arguments may hold no `"`
            // (AM53; found by the Windows acceptance run).
            private_dir(dir)?;
            let path = dir.join("claude-settings.json");
            write_private(&path, settings.to_string().as_bytes())?;
            if let Some(pins) = synced {
                crate::credential::verify_settings(&std::fs::read(&path)?, pins)?;
            }
            args.extend(["--settings".into(), path.into_os_string()]);
            Ok((
                LaunchExtras {
                    args,
                    env,
                    ..Default::default()
                },
                Some(id),
            ))
        }
        "opencode" => {
            let config = dir.join("opencode");
            let plugins = config.join("plugin");
            private_dir(dir)?;
            private_dir(&config)?;
            private_dir(&plugins)?;
            write_private(&plugins.join("storm.js"), plugin(exe).as_bytes())?;
            let mut env = env;
            env.push(("OPENCODE_CONFIG_DIR".into(), config.into_os_string()));
            let args = match resume {
                Some(id) => vec!["-s".into(), id.into()],
                None => Vec::new(),
            };
            Ok((
                LaunchExtras {
                    args,
                    env,
                    ..Default::default()
                },
                resume.map(str::to_string),
            ))
        }
        _ => Ok((LaunchExtras::default(), None)),
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The OpenCode plugin. A subagent's session has a parent and is never the
/// one to resume; an id is reported once per change.
fn plugin(exe: &Path) -> String {
    let exe = serde_json::to_string(&exe.display().to_string()).expect("a string serializes");
    format!(
        r#"import {{ spawn }} from "node:child_process";

export const StormHarness = async () => {{
  const children = new Set();
  let last;
  return {{
    event: async ({{ event }}) => {{
      const info = event.properties?.info;
      if (event.type === "session.created" || event.type === "session.updated") {{
        if (info?.parentID) children.add(info.id);
        return;
      }}
      if (event.type !== "message.updated") return;
      const id = info?.sessionID;
      if (typeof id !== "string" || id === last || children.has(id)) return;
      last = id;
      try {{
        spawn({exe}, ["harness-session", id], {{ stdio: "ignore", detached: true }}).unref();
      }} catch {{}}
    }},
  }};
}};
"#
    )
}

/// `storm-runtime harness-session [ID]`: sends the id, or the `session_id`
/// of the hook input on stdin, to the daemon. Silent and always successful:
/// a hook's output reaches the agent, and a failure here must never get in
/// its way.
pub fn report(id: Option<String>) {
    let _ = try_report(id);
}

fn try_report(id: Option<String>) -> Result<()> {
    let handle = std::env::var(HANDLE_ENV)?;
    let socket = std::env::var_os(crate::mcp::SOCKET_ENV).ok_or(anyhow::anyhow!("no socket"))?;
    let id = match id {
        Some(id) => id,
        None => {
            let mut input = String::new();
            std::io::stdin().take(1 << 20).read_to_string(&mut input)?;
            let hook: serde_json::Value = serde_json::from_str(&input)?;
            hook["session_id"]
                .as_str()
                .ok_or(anyhow::anyhow!("no session_id"))?
                .to_string()
        }
    };
    send(&socket, handle, id)
}

pub(crate) fn send(socket: impl AsRef<Path>, handle: String, id: String) -> Result<()> {
    let mut line = serde_json::to_vec(&HarnessLine {
        handle,
        harness_session: id,
    })?;
    line.push(b'\n');
    let mut stream = crate::platform::ipc::connect_blocking(socket.as_ref().as_os_str())?;
    stream.write_all(&line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn only_ids_the_agent_could_have_made_are_valid() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../docs/harness-session-vectors.json"))
                .unwrap();
        for (set, want) in [("valid", true), ("invalid", false)] {
            for case in v[set].as_array().unwrap() {
                let (provider, id) = (
                    case["provider"].as_str().unwrap(),
                    case["id"].as_str().unwrap(),
                );
                assert_eq!(valid_id(provider, id), want, "{provider} {id:?}");
            }
        }
    }

    #[test]
    fn a_new_uuid_is_a_valid_v4() {
        let id = new_uuid();
        assert!(valid_id("claude-code", &id), "{id}");
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(new_uuid(), id);
    }

    #[test]
    fn claude_code_starts_with_its_id_and_a_session_start_hook() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/opt/it's/storm-runtime");
        let (extras, id) = launch(
            "claude-code",
            dir.path(),
            "h",
            Path::new("/s"),
            exe,
            None,
            None,
        )
        .unwrap();
        let id = id.unwrap();
        let args: Vec<String> = extras
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[..2], ["--session-id".to_string(), id]);
        assert_eq!(args[2], "--settings");
        assert_eq!(Path::new(&args[3]), dir.path().join("claude-settings.json"));
        crate::platform::assert_private(Path::new(&args[3]), 0o600);
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&args[3]).unwrap()).unwrap();
        assert_eq!(
            settings["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            r"'/opt/it'\''s/storm-runtime' harness-session"
        );
        assert!(settings.get("env").is_none(), "unsynced: unchanged");
        assert!(settings.get("apiKeyHelper").is_none());
        assert!(extras.env.contains(&(HANDLE_ENV.into(), "h".into())));
        assert!(
            extras
                .env
                .contains(&(crate::mcp::SOCKET_ENV.into(), "/s".into()))
        );
    }

    #[test]
    fn a_synced_session_has_one_settings_file_with_every_pin_and_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let odd = r#"http://u:p"@proxy\lan:3128/ü"#;
        let pins = crate::credential::settings(|name| {
            (name == "HTTPS_PROXY" || name == "SSL_CERT_DIR").then(|| odd.into())
        })
        .unwrap();
        let (extras, _) = launch(
            "claude-code",
            dir.path(),
            "h",
            Path::new("/s"),
            Path::new("/x"),
            None,
            Some(&pins),
        )
        .unwrap();
        let at = extras.args.iter().position(|a| a == "--settings").unwrap();
        assert_eq!(extras.args.iter().filter(|a| *a == "--settings").count(), 1);
        let path = Path::new(&extras.args[at + 1]);
        crate::platform::assert_private(path, 0o600);
        let bytes = std::fs::read(path).unwrap();
        let settings: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(settings["apiKeyHelper"], "");
        assert_eq!(
            settings["env"]["ANTHROPIC_BASE_URL"],
            "https://api.anthropic.com"
        );
        assert_eq!(settings["env"]["HTTPS_PROXY"], odd);
        assert_eq!(settings["env"]["SSL_CERT_DIR"], odd);
        for name in crate::credential::EMPTY_PINS {
            assert_eq!(settings["env"][name], "", "{name}");
        }
        for name in crate::credential::NETWORK_PINS {
            assert!(settings["env"][name].is_string(), "{name}");
        }
        assert!(
            settings["env"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v.is_string())
        );
        assert!(
            settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .ends_with("harness-session")
        );
        crate::credential::verify_settings(&bytes, &pins).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sk-ant-"));
        assert!(!String::from_utf8_lossy(&bytes).contains(crate::credential::TOKEN_ENV));
    }

    #[test]
    fn a_resume_names_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/x");
        let uuid = "80d0cf8c-8176-43b4-8206-7908d74d1387";
        let (claude, id) = launch(
            "claude-code",
            dir.path(),
            "h",
            Path::new("/s"),
            exe,
            Some(uuid),
            None,
        )
        .unwrap();
        assert_eq!(
            claude.args[..2],
            [OsString::from("--resume"), OsString::from(uuid)]
        );
        assert_eq!(id.as_deref(), Some(uuid));

        let ses = "ses_ed5303b0dffeRnwXZdkDih2uxK";
        let (oc, id) = launch(
            "opencode",
            dir.path(),
            "h",
            Path::new("/s"),
            exe,
            Some(ses),
            None,
        )
        .unwrap();
        assert_eq!(oc.args, ["-s", ses]);
        assert_eq!(id.as_deref(), Some(ses));
    }

    #[test]
    fn opencode_gets_a_config_dir_with_the_plugin_and_no_id_yet() {
        let dir = tempfile::tempdir().unwrap();
        let (extras, id) = launch(
            "opencode",
            dir.path(),
            "h",
            Path::new("/s"),
            Path::new("/usr/bin/storm-runtime"),
            None,
            None,
        )
        .unwrap();
        assert!(extras.args.is_empty());
        assert_eq!(id, None);
        let config = dir.path().join("opencode");
        assert!(extras.env.contains(&(
            "OPENCODE_CONFIG_DIR".into(),
            config.clone().into_os_string()
        )));
        let plugin = std::fs::read_to_string(config.join("plugin/storm.js")).unwrap();
        assert!(plugin.contains(r#"spawn("/usr/bin/storm-runtime", ["harness-session", id]"#));
        crate::platform::assert_private(&config.join("plugin/storm.js"), 0o600);
    }

    #[test]
    fn other_providers_gain_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for provider in ["shell", "fake", "my-agent"] {
            let (extras, id) = launch(
                provider,
                dir.path(),
                "h",
                Path::new("/s"),
                Path::new("/x"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(extras, LaunchExtras::default());
            assert_eq!(id, None);
        }
    }
}
