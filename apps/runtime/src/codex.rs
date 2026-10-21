//! The Codex adapter (decision 130b).
//!
//! Codex keeps its login and every conversation under `CODEX_HOME`, and
//! Resume needs a conversation to outlive the session that had it, so a host
//! has **one Storm-owned home**, `<state>/agents/codex`, not one per session.
//! What is per session lives beside it:
//!
//! - **MCP servers** in a profile, `storm-<session>.config.toml` (`0600`,
//!   removed when the session ends), loaded with `-p`. Every server asks
//!   before each call (`default_tools_approval_mode = "prompt"`, which also
//!   turns "approve for this session" into a single approval): G2.
//! - Nothing else: the conversation id arrives through Storm's
//!   `SessionStart` hook, with the session's handle in its environment.
//!
//! **What Storm owns in `config.toml`** is rewritten before every launch, and
//! everything Codex writes there itself is kept:
//!
//! - no `mcp_servers` and no `profiles`: a session's servers are its own
//!   profile's and nothing else (AM32's rule);
//! - `hooks` is Storm's `SessionStart` hook and nothing else, because Codex
//!   runs with `--dangerously-bypass-hook-trust`;
//! - every workspace Codex has met is `trust_level = "untrusted"`, and the
//!   launching one is added: a repository's own `.codex/` config, hooks and
//!   MCP servers never load, and Codex's folder screen offers only "Open
//!   restricted" (found on 0.163; a workspace with no entry offers Trust);
//! - `check_for_update_on_startup = false`, `features.daemon_auto_start =
//!   false` (a session owns its process; a shared daemon would hold a
//!   conversation's writer lock), and a title of the activity and the
//!   conversation's name.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use toml::{Table, Value};

use crate::mcp::{McpGrant, private_dir};
use crate::provider::LaunchExtras;

pub const HOME_ENV: &str = "CODEX_HOME";
const PROFILE_PREFIX: &str = "storm-";
const PROFILE_SUFFIX: &str = ".config.toml";

/// One launch at a time rewrites `config.toml`; Codex itself may write it
/// too, which a rename-into-place keeps from tearing.
static CONFIG: Mutex<()> = Mutex::new(());

/// The host's Codex home.
pub fn home(state_dir: &Path) -> PathBuf {
    state_dir.join("agents").join("codex")
}

pub fn profile_name(session: &str) -> String {
    format!("{PROFILE_PREFIX}{session}")
}

fn profile_path(home: &Path, session: &str) -> PathBuf {
    home.join(format!("{}{PROFILE_SUFFIX}", profile_name(session)))
}

/// What every Codex session gains, grants or not: its home, with Storm's
/// keys written for `workspace`, and the hook that reports the conversation
/// id by running `exe harness-session`.
pub fn baseline(home: &Path, workspace: &Path, exe: &Path) -> Result<LaunchExtras> {
    private_dir(home.parent().context("the Codex home has a parent")?)?;
    private_dir(home)?;
    let path = home.join("config.toml");
    let _guard = CONFIG.lock().unwrap();
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    // A file Codex cannot have written is Storm's to replace, not to merge.
    let table: Table = existing.parse().unwrap_or_else(|e| {
        tracing::warn!(path = %path.display(), error = %e, "replacing an unreadable Codex config");
        Table::new()
    });
    let table = owned_config(table, workspace, exe);
    crate::fsutil::write_atomic(
        &path,
        toml::to_string(&table)?.as_bytes(),
        crate::fsutil::Access::Private,
    )
    .with_context(|| format!("writing {}", path.display()))?;
    Ok(LaunchExtras {
        env: vec![(HOME_ENV.into(), home.as_os_str().to_owned())],
        ..Default::default()
    })
}

/// `table` with Storm's keys set (see the module docs).
pub fn owned_config(mut table: Table, workspace: &Path, exe: &Path) -> Table {
    table.remove("mcp_servers");
    table.remove("profiles");
    table.remove("profile");
    table.insert("check_for_update_on_startup".into(), Value::Boolean(false));

    let features = sub_table(&mut table, "features");
    features.insert("daemon_auto_start".into(), Value::Boolean(false));

    let tui = sub_table(&mut table, "tui");
    tui.insert(
        "terminal_title".into(),
        Value::Array(vec!["activity".into(), "thread-name".into()]),
    );

    let projects = sub_table(&mut table, "projects");
    let mut keys: Vec<String> = projects.keys().cloned().collect();
    keys.push(workspace.display().to_string());
    if let Ok(canonical) = workspace.canonicalize() {
        keys.push(canonical.display().to_string());
    }
    for key in keys {
        let mut project = Table::new();
        project.insert("trust_level".into(), "untrusted".into());
        projects.insert(key, Value::Table(project));
    }

    table.insert("hooks".into(), Value::Table(hooks(exe)));
    table
}

/// Storm's `SessionStart` hook and nothing else. It fires on a fresh start
/// and on `/clear` (a new conversation id), not on a resume: Storm already
/// knows that id, and Codex 0.163 starts a resume's hook without ever
/// feeding it, so the hook would hold the session up to its timeout.
fn hooks(exe: &Path) -> Table {
    let mut handler = Table::new();
    handler.insert("type".into(), "command".into());
    handler.insert(
        "command".into(),
        format!("{} harness-session", sh_quote(&exe.display().to_string())).into(),
    );
    handler.insert(
        "commandWindows".into(),
        format!("\"{}\" harness-session", exe.display()).into(),
    );
    handler.insert("timeout".into(), Value::Integer(10));
    let mut group = Table::new();
    group.insert("matcher".into(), "startup|clear".into());
    group.insert("hooks".into(), Value::Array(vec![Value::Table(handler)]));
    let mut hooks = Table::new();
    hooks.insert(
        "SessionStart".into(),
        Value::Array(vec![Value::Table(group)]),
    );
    hooks
}

fn sub_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !matches!(table.get(key), Some(Value::Table(_))) {
        table.insert(key.into(), Value::Table(Table::new()));
    }
    match table.get_mut(key) {
        Some(Value::Table(t)) => t,
        _ => unreachable!("inserted above"),
    }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Writes `session`'s MCP servers into its own profile and returns `-p` for
/// it. The profile holds the bridge handle and the socket path, never a
/// credential.
pub fn write_profile(
    home: &Path,
    session: &str,
    grants: &[&McpGrant],
    handle: &str,
    socket: &Path,
    exe: &Path,
) -> Result<LaunchExtras> {
    let mut servers = Table::new();
    for g in grants {
        let mut env = Table::new();
        env.insert(crate::mcp::HANDLE_ENV.into(), handle.into());
        env.insert(
            crate::mcp::SOCKET_ENV.into(),
            socket.display().to_string().into(),
        );
        let mut server = Table::new();
        server.insert("command".into(), exe.display().to_string().into());
        server.insert(
            "args".into(),
            Value::Array(vec!["mcp-bridge".into(), g.slug.clone().into()]),
        );
        server.insert("env".into(), Value::Table(env));
        server.insert("default_tools_approval_mode".into(), "prompt".into());
        servers.insert(g.slug.clone(), Value::Table(server));
    }
    let mut profile = Table::new();
    profile.insert("mcp_servers".into(), Value::Table(servers));
    private_dir(home)?;
    let path = profile_path(home, session);
    crate::fsutil::write_atomic(
        &path,
        toml::to_string(&profile)?.as_bytes(),
        crate::fsutil::Access::Private,
    )
    .with_context(|| format!("writing {}", path.display()))?;
    Ok(LaunchExtras {
        args: vec!["-p".into(), profile_name(session).into()],
        ..Default::default()
    })
}

/// A session ended: its profile, and with it its handle, goes.
pub fn forget(home: &Path, session: &str) {
    let _ = std::fs::remove_file(profile_path(home, session));
}

/// Every session profile belongs to a session that died with the previous
/// process.
pub fn forget_all(home: &Path) {
    let Ok(entries) = std::fs::read_dir(home) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(PROFILE_PREFIX) && name.ends_with(PROFILE_SUFFIX) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(slug: &str) -> McpGrant {
        McpGrant {
            id: format!("mcc_{slug}"),
            slug: slug.into(),
        }
    }

    fn read(path: &Path) -> Table {
        std::fs::read_to_string(path).unwrap().parse().unwrap()
    }

    /// Storm's keys win; Codex's own are kept; a repository can never be
    /// trusted, and no server or hook but Storm's survives a launch.
    #[test]
    fn the_config_is_storms_where_it_matters_and_codexs_elsewhere() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("config.toml"),
            r#"
model = "gpt-6"
check_for_update_on_startup = true
profile = "evil"
[mcp_servers.evil]
command = "/bin/evil"
[profiles.evil]
model = "x"
[projects."/somewhere/else"]
trust_level = "trusted"
[[hooks.SessionStart]]
[[hooks.SessionStart.hooks]]
type = "command"
command = "curl evil"
[tui]
screen_reader_detection_done = true
"#,
        )
        .unwrap();
        let extras = baseline(&home, ws.path(), Path::new("/opt/storm/storm-runtime")).unwrap();
        assert_eq!(extras.env, [(HOME_ENV.into(), home.as_os_str().to_owned())]);
        let t = read(&home.join("config.toml"));
        assert_eq!(t["model"].as_str(), Some("gpt-6"), "Codex's own key kept");
        assert_eq!(
            t["tui"]["screen_reader_detection_done"].as_bool(),
            Some(true)
        );
        assert!(t.get("mcp_servers").is_none());
        assert!(t.get("profiles").is_none());
        assert!(t.get("profile").is_none());
        assert_eq!(t["check_for_update_on_startup"].as_bool(), Some(false));
        assert_eq!(t["features"]["daemon_auto_start"].as_bool(), Some(false));
        let projects = t["projects"].as_table().unwrap();
        assert_eq!(
            projects["/somewhere/else"]["trust_level"].as_str(),
            Some("untrusted")
        );
        assert_eq!(
            projects[&ws.path().display().to_string()]["trust_level"].as_str(),
            Some("untrusted")
        );
        let starts = t["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 1, "only Storm's hook");
        assert_eq!(starts[0]["matcher"].as_str(), Some("startup|clear"));
        let handler = &starts[0]["hooks"][0];
        assert_eq!(
            handler["command"].as_str(),
            Some("'/opt/storm/storm-runtime' harness-session")
        );
        assert!(t["hooks"].as_table().unwrap().len() == 1);
        assert!(crate::platform::is_private(&home.join("config.toml")).unwrap());
    }

    #[test]
    fn an_unreadable_config_is_replaced() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "[[[ not toml").unwrap();
        baseline(&home, ws.path(), Path::new("/x")).unwrap();
        let t = read(&home.join("config.toml"));
        assert_eq!(t["check_for_update_on_startup"].as_bool(), Some(false));
    }

    #[test]
    fn a_profile_holds_only_the_sessions_servers_and_asks_for_each() {
        let state = tempfile::tempdir().unwrap();
        let home = home(state.path());
        let (a, b) = (grant("storm"), grant("github"));
        let extras = write_profile(
            &home,
            "ags_ONE",
            &[&a, &b],
            "the-handle",
            Path::new("/run/mcp.sock"),
            Path::new("/opt/storm-runtime"),
        )
        .unwrap();
        assert_eq!(extras.args, ["-p", "storm-ags_ONE"]);
        let path = home.join("storm-ags_ONE.config.toml");
        assert!(crate::platform::is_private(&path).unwrap());
        let t = read(&path);
        assert_eq!(t.len(), 1, "only mcp_servers");
        let servers = t["mcp_servers"].as_table().unwrap();
        assert_eq!(servers.len(), 2);
        let storm = &servers["storm"];
        assert_eq!(storm["command"].as_str(), Some("/opt/storm-runtime"));
        assert_eq!(storm["args"][0].as_str(), Some("mcp-bridge"));
        assert_eq!(storm["args"][1].as_str(), Some("storm"));
        assert_eq!(
            storm["env"][crate::mcp::HANDLE_ENV].as_str(),
            Some("the-handle")
        );
        for s in servers.values() {
            assert_eq!(s["default_tools_approval_mode"].as_str(), Some("prompt"));
        }

        forget(&home, "ags_ONE");
        assert!(!path.exists());
        write_profile(
            &home,
            "ags_TWO",
            &[&a],
            "h",
            Path::new("/s"),
            Path::new("/x"),
        )
        .unwrap();
        std::fs::write(home.join("config.toml"), "").unwrap();
        forget_all(&home);
        assert!(!home.join("storm-ags_TWO.config.toml").exists());
        assert!(
            home.join("config.toml").exists(),
            "only session profiles go"
        );
    }
}
