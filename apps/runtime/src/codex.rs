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
//!   conversation's name;
//! - **no plugin with hooks** (130f): Codex applies
//!   `--dangerously-bypass-hook-trust` to a plugin's hooks as well as to
//!   Storm's, so a plugin that declares any is off. `plugins._default` is off,
//!   so a plugin Storm cannot inspect (an account-synced one) never loads;
//!   every listed plugin gets an explicit `enabled`. Which plugins Storm
//!   turned off is remembered beside the config, so one that drops its hooks
//!   comes back, and one the account turned off stays off.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use toml::{Table, Value};

use crate::mcp::{McpGrant, private_dir};
use crate::provider::LaunchExtras;

pub const HOME_ENV: &str = "CODEX_HOME";
const PROFILE_PREFIX: &str = "storm-";
const PROFILE_SUFFIX: &str = ".config.toml";
/// The plugins Storm turned off for their hooks, as a JSON list of ids.
const PLUGINS_OFF: &str = "storm-plugins-off.json";
/// Where Codex may find a plugin's manifest (0.163's
/// `DISCOVERABLE_PLUGIN_MANIFEST_PATHS`, and the root `plugin.json`).
const MANIFESTS: [&str; 4] = [
    "plugin.json",
    ".codex-plugin/plugin.json",
    ".claude-plugin/plugin.json",
    ".cursor-plugin/plugin.json",
];

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
    let mut table = owned_config(table, workspace, exe);
    let off_path = home.join(PLUGINS_OFF);
    let was_off: Vec<String> = std::fs::read(&off_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let off = switch_plugins(&mut table, home, &was_off);
    if !off.is_empty() {
        tracing::warn!(plugins = ?off, "Codex plugins with hooks are off in Storm sessions (130f)");
    }
    crate::fsutil::write_atomic(
        &path,
        toml::to_string(&table)?.as_bytes(),
        crate::fsutil::Access::Private,
    )
    .with_context(|| format!("writing {}", path.display()))?;
    crate::fsutil::write_atomic(
        &off_path,
        &serde_json::to_vec(&off)?,
        crate::fsutil::Access::Private,
    )
    .with_context(|| format!("writing {}", off_path.display()))?;
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

/// Turns off every plugin in `table` that has hooks, or that Storm cannot
/// inspect, and returns their ids. `was_off` is what Storm turned off last
/// time: those count as wanted, so a plugin that no longer has hooks comes
/// back. A plugin the account itself turned off stays off.
pub fn switch_plugins(table: &mut Table, home: &Path, was_off: &[String]) -> Vec<String> {
    let plugins = sub_table(table, "plugins");
    let mut off = Vec::new();
    for (id, entry) in plugins.iter_mut() {
        let Value::Table(entry) = entry else { continue };
        if id == "_default" {
            continue;
        }
        let wanted = was_off.contains(id)
            || entry
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
        if !wanted {
            continue;
        }
        let hooked = plugin_may_have_hooks(home, id);
        entry.insert("enabled".into(), Value::Boolean(!hooked));
        if hooked {
            off.push(id.clone());
        }
    }
    let mut default = Table::new();
    default.insert("enabled".into(), Value::Boolean(false));
    plugins.insert("_default".into(), Value::Table(default));
    off
}

/// Whether any installed version of plugin `id` (`<name>@<marketplace>`)
/// declares hooks: a `hooks` entry beside its manifest, or a `hooks` key in
/// any manifest Codex might read. Anything Storm cannot read counts as yes.
fn plugin_may_have_hooks(home: &Path, id: &str) -> bool {
    let Some((name, marketplace)) = id.rsplit_once('@') else {
        return true;
    };
    let segment_ok =
        |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', '\0']);
    if !segment_ok(name) || !segment_ok(marketplace) {
        return true;
    }
    let base = home
        .join("plugins")
        .join("cache")
        .join(marketplace)
        .join(name);
    let Ok(versions) = std::fs::read_dir(&base) else {
        return true;
    };
    let mut any = false;
    for version in versions {
        let Ok(version) = version else { return true };
        any = true;
        if version_may_have_hooks(&version.path()) {
            return true;
        }
    }
    !any
}

fn version_may_have_hooks(root: &Path) -> bool {
    if std::fs::symlink_metadata(root.join("hooks")).is_ok() {
        return true;
    }
    MANIFESTS.iter().any(|m| match std::fs::read(root.join(m)) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .map(|v| v.get("hooks").is_some())
            .unwrap_or(true),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    })
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

    fn install(home: &Path, id: &str, files: &[(&str, &str)]) {
        let (name, marketplace) = id.rsplit_once('@').unwrap();
        let root = home
            .join("plugins/cache")
            .join(marketplace)
            .join(name)
            .join("1.0.0");
        for (path, body) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
    }

    /// 130f: a plugin whose hooks Codex would run under Storm's trust bypass
    /// is off, and so is anything Storm cannot inspect; a plain plugin and
    /// the account's own choice are kept.
    #[test]
    fn a_plugin_with_hooks_or_unknown_is_off() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        let manifest = r#"{"name":"x","version":"1.0.0"}"#;
        install(
            &home,
            "plain@m",
            &[(".codex-plugin/plugin.json", manifest), (".mcp.json", "{}")],
        );
        install(
            &home,
            "hookfile@m",
            &[
                (".claude-plugin/plugin.json", manifest),
                ("hooks/hooks.json", "{}"),
            ],
        );
        install(
            &home,
            "hookkey@m",
            &[(
                ".claude-plugin/plugin.json",
                r#"{"name":"x","hooks":"./h.json"}"#,
            )],
        );
        install(
            &home,
            "garbled@m",
            &[(".codex-plugin/plugin.json", "{ not json")],
        );
        install(&home, "mine@m", &[("hooks/hooks.json", "{}")]);
        std::fs::write(
            home.join("config.toml"),
            r#"
[plugins."plain@m"]
enabled = true
[plugins."hookfile@m"]
enabled = true
[plugins."hookkey@m"]
[plugins."garbled@m"]
enabled = true
[plugins."missing@m"]
enabled = true
[plugins."../../etc@m"]
enabled = true
[plugins."mine@m"]
enabled = false
[plugins._default]
enabled = true
"#,
        )
        .unwrap();
        baseline(&home, ws.path(), Path::new("/x")).unwrap();
        let t = read(&home.join("config.toml"));
        let p = t["plugins"].as_table().unwrap();
        let on = |id: &str| p[id]["enabled"].as_bool().unwrap();
        assert!(on("plain@m"), "a plugin without hooks is kept");
        for id in [
            "hookfile@m",
            "hookkey@m",
            "garbled@m",
            "missing@m",
            "../../etc@m",
        ] {
            assert!(!on(id), "{id} should be off");
        }
        assert!(!on("mine@m"), "the account's own choice is kept");
        assert!(
            !p["_default"]["enabled"].as_bool().unwrap(),
            "unlisted plugins never load"
        );
        let mut off: Vec<String> =
            serde_json::from_slice(&std::fs::read(home.join(PLUGINS_OFF)).unwrap()).unwrap();
        off.sort();
        assert_eq!(
            off,
            [
                "../../etc@m",
                "garbled@m",
                "hookfile@m",
                "hookkey@m",
                "missing@m"
            ]
        );
        assert!(crate::platform::is_private(&home.join(PLUGINS_OFF)).unwrap());
    }

    /// A plugin Storm turned off comes back once it has no hooks; one the
    /// account turned off does not.
    #[test]
    fn a_plugin_storm_turned_off_comes_back_without_hooks() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        install(&home, "p@m", &[("hooks/hooks.json", "{}")]);
        std::fs::write(
            home.join("config.toml"),
            "[plugins.\"p@m\"]\nenabled = true\n",
        )
        .unwrap();
        baseline(&home, ws.path(), Path::new("/x")).unwrap();
        assert_eq!(
            read(&home.join("config.toml"))["plugins"]["p@m"]["enabled"].as_bool(),
            Some(false)
        );

        std::fs::remove_dir_all(home.join("plugins/cache/m/p/1.0.0/hooks")).unwrap();
        baseline(&home, ws.path(), Path::new("/x")).unwrap();
        assert_eq!(
            read(&home.join("config.toml"))["plugins"]["p@m"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            std::fs::read_to_string(home.join(PLUGINS_OFF)).unwrap(),
            "[]"
        );
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
