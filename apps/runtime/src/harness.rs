//! The agent's own session id, so Storm can resume the conversation rather
//! than start a new one (decision 89).
//!
//! Claude Code and OpenCode each keep their conversations under an id of
//! their own, per host and per workspace. The host learns it while the
//! session runs, never by reading the terminal:
//!
//! - **`claude-code`**: a fresh session is started with `--session-id <uuid>`,
//!   and every session gets a `SessionStart` hook (through `--settings`) that
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

/// The providers whose conversations can be resumed.
pub const PROVIDERS: [&str; 2] = ["claude-code", "opencode"];

/// One line from `harness-session` on the daemon socket.
#[derive(Debug, Deserialize, Serialize)]
pub struct HarnessLine {
    pub handle: String,
    pub harness_session: String,
}

/// Whether `id` is a session id `provider` could have made. It becomes an
/// argument, so nothing else is ever accepted.
pub fn valid_id(provider: &str, id: &str) -> bool {
    match provider {
        "claude-code" => is_uuid(id),
        "opencode" => id.strip_prefix("ses_").is_some_and(|rest| {
            (1..=64).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        }),
        _ => false,
    }
}

fn is_uuid(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
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
/// `provider`. Nothing for any other provider.
pub fn launch(
    provider: &str,
    dir: &Path,
    handle: &str,
    socket: &Path,
    exe: &Path,
    resume: Option<&str>,
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
            let settings = json!({
                "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": hook}]}]}
            });
            args.extend(["--settings".into(), settings.to_string().into()]);
            Ok((LaunchExtras { args, env }, Some(id)))
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
            Ok((LaunchExtras { args, env }, resume.map(str::to_string)))
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
    let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
    stream.write_all(&line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn only_ids_the_agent_could_have_made_are_valid() {
        assert!(valid_id(
            "claude-code",
            "80d0cf8c-8176-43b4-8206-7908d74d1387"
        ));
        assert!(valid_id("opencode", "ses_ed5303b0dffeRnwXZdkDih2uxK"));
        for (provider, bad) in [
            ("claude-code", "80D0CF8C-8176-43B4-8206-7908D74D1387"),
            ("claude-code", "80d0cf8c-8176-43b4-8206-7908d74d138"),
            ("claude-code", "--dangerously-skip-permissions"),
            ("opencode", "ses_"),
            ("opencode", "ses_abc;rm"),
            ("opencode", "-s"),
            ("shell", "ses_abc"),
            ("claude-code", "ses_abc"),
        ] {
            assert!(!valid_id(provider, bad), "{provider} {bad}");
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
        let (extras, id) =
            launch("claude-code", dir.path(), "h", Path::new("/s"), exe, None).unwrap();
        let id = id.unwrap();
        let args: Vec<String> = extras
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[..2], ["--session-id".to_string(), id]);
        assert_eq!(args[2], "--settings");
        let settings: serde_json::Value = serde_json::from_str(&args[3]).unwrap();
        assert_eq!(
            settings["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            r"'/opt/it'\''s/storm-runtime' harness-session"
        );
        assert!(extras.env.contains(&(HANDLE_ENV.into(), "h".into())));
        assert!(
            extras
                .env
                .contains(&(crate::mcp::SOCKET_ENV.into(), "/s".into()))
        );
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
        )
        .unwrap();
        assert_eq!(
            claude.args[..2],
            [OsString::from("--resume"), OsString::from(uuid)]
        );
        assert_eq!(id.as_deref(), Some(uuid));

        let ses = "ses_ed5303b0dffeRnwXZdkDih2uxK";
        let (oc, id) =
            launch("opencode", dir.path(), "h", Path::new("/s"), exe, Some(ses)).unwrap();
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
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(config.join("plugin/storm.js"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
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
            )
            .unwrap();
            assert_eq!(extras, LaunchExtras::default());
            assert_eq!(id, None);
        }
    }
}
