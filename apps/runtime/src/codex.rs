//! The Codex adapter (decision 130b, as amended by 132c).
//!
//! Codex keeps its login and every conversation under `CODEX_HOME`, and
//! Resume needs a conversation to outlive the session that had it, so a host
//! has **one Storm-owned home**, `<state>/agents/codex`, not one per session.
//! What is per session lives beside it:
//!
//! - **MCP servers** in a profile, `storm-<session>.config.toml` (`0600`,
//!   removed when the session ends), loaded with `-p`. Every server asks
//!   before each call (`default_tools_approval_mode = "prompt"`, which also
//!   turns "approve for this session" into a single approval): G2. A profile
//!   merges into `config.toml` key by key, so a server named like one of the
//!   account's is written as `storm_<slug>`.
//! - Nothing else: the conversation id arrives through Storm's
//!   `SessionStart` hook, with the session's handle in its environment.
//!
//! **The account's config is its own** (decision 132): its `mcp_servers`,
//! profiles, plugins, plugin servers, apps, hooks and folder trust load as
//! they would without Storm, and Codex asks its own questions about them.
//! Storm writes only:
//!
//! - its `SessionStart` hook, appended to the account's, **with its
//!   `trusted_hash`** under `hooks.state` (132c-1), so Codex runs it without
//!   `--dangerously-bypass-hook-trust` and reviews every other hook itself;
//! - `check_for_update_on_startup = false`, `features.daemon_auto_start =
//!   false` (a session owns its process; a shared daemon would hold a
//!   conversation's writer lock), and a title of the activity and the
//!   conversation's name.
//!
//! Before 132, Storm deleted the account's servers, pinned every workspace
//! untrusted and switched plugins, plugin servers and apps off; the first
//! launch after it undoes what it can (`storm-plugins-off.json` marks a home
//! that had it).
//!
//! **Under `agent_mcp = "storm-only"`** a session gets Storm's servers alone:
//! the account's are withheld for the session (`-c mcp_servers.<name>.enabled
//! = false`, never deleted), every workspace is `untrusted` (a repository's
//! `.codex/` config never loads), a plugin's remote servers are off and its
//! stdio ones follow `plugin_mcp` (131i), and ChatGPT apps are off (131l).
//! What that leaves out is named at launch (131a).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use toml::{Table, Value};

use crate::mcp::{McpGrant, private_dir};
use crate::plugins::{Plan, Skipped};
use crate::provider::LaunchExtras;

pub const HOME_ENV: &str = "CODEX_HOME";
const PROFILE_PREFIX: &str = "storm-";
const PROFILE_SUFFIX: &str = ".config.toml";
/// Written by Storm before decision 132: its presence says the home still
/// carries 130f's and 131's switches.
const PLUGINS_OFF: &str = "storm-plugins-off.json";
const HOOK_MATCHER: &str = "startup|clear";
const HOOK_TIMEOUT: i64 = 10;

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

/// The names of the account's own servers in the home's `config.toml`.
pub fn own_servers(home: &Path) -> HashSet<String> {
    std::fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|t| t.parse::<Table>().ok())
        .and_then(|t| t.get("mcp_servers").and_then(Value::as_table).cloned())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// What every Codex session gains, grants or not: its home, with Storm's
/// keys written for `workspace`, and the hook that reports the conversation
/// id by running `exe harness-session`; under `storm_only`, the overrides
/// that keep the account's servers out and the plan of what the session
/// will not have (decision 131).
pub fn baseline(
    home: &Path,
    workspace: &Path,
    exe: &Path,
    allow_local: bool,
    storm_only: bool,
) -> Result<(LaunchExtras, Plan)> {
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
    let mut table: Table = existing.parse().unwrap_or_else(|e| {
        tracing::warn!(path = %path.display(), error = %e, "replacing an unreadable Codex config");
        Table::new()
    });
    let off_path = home.join(PLUGINS_OFF);
    let was_off: Option<Vec<String>> = std::fs::read(&off_path)
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap_or_default());
    if let Some(was_off) = &was_off {
        undo_isolation(&mut table, was_off);
    }
    common_keys(&mut table);
    set_hook(&mut table, exe, &path);
    let mut extras = LaunchExtras {
        env: vec![(HOME_ENV.into(), home.as_os_str().to_owned())],
        ..Default::default()
    };
    let mut plan = Plan::default();
    if storm_only {
        let own: Vec<String> = table
            .get("mcp_servers")
            .and_then(Value::as_table)
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        pin_untrusted(&mut table, workspace);
        sub_table(&mut table, "features").insert("apps".into(), Value::Boolean(false));
        switch_plugin_servers(&mut table, home, allow_local, &mut plan);
        for name in &own {
            match withhold(name) {
                Some(arg) => extras.args.extend(["-c".into(), arg.into()]),
                None => tracing::warn!(server = %name, "cannot withhold a server with this name"),
            }
        }
        name_the_rest(&mut plan, &own, home, workspace);
    }
    crate::fsutil::write_atomic(
        &path,
        toml::to_string(&table)?.as_bytes(),
        crate::fsutil::Access::Private,
    )
    .with_context(|| format!("writing {}", path.display()))?;
    if was_off.is_some() {
        let _ = std::fs::remove_file(&off_path);
    }
    Ok((extras, plan))
}

/// `mcp_servers.<name>.enabled=false` for `-c`, or `None` for a name the
/// dotted path cannot spell.
fn withhold(name: &str) -> Option<String> {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    plain.then(|| format!("mcp_servers.{name}.enabled=false"))
}

/// Undoes what Storm wrote before decision 132: the plugins it turned off
/// for their hooks (130f) and `plugins._default`, its plugin-server switches
/// (131i), `features.apps = false` (131l) and the `untrusted` pins. The
/// account's servers it deleted then are gone.
fn undo_isolation(table: &mut Table, was_off: &[String]) {
    if let Some(Value::Table(plugins)) = table.get_mut("plugins") {
        if matches!(plugins.get("_default"), Some(Value::Table(d))
            if d.len() == 1 && d.get("enabled") == Some(&Value::Boolean(false)))
        {
            plugins.remove("_default");
        }
        for (id, entry) in plugins.iter_mut() {
            let Value::Table(entry) = entry else { continue };
            if was_off.contains(id) {
                entry.insert("enabled".into(), Value::Boolean(true));
            }
            if let Some(Value::Table(servers)) = entry.get_mut("mcp_servers") {
                for (_, server) in servers.iter_mut() {
                    if let Value::Table(server) = server {
                        server.remove("enabled");
                    }
                }
                servers.retain(|_, s| !matches!(s, Value::Table(t) if t.is_empty()));
                if servers.is_empty() {
                    entry.remove("mcp_servers");
                }
            }
        }
    }
    if let Some(Value::Table(features)) = table.get_mut("features")
        && features.get("apps") == Some(&Value::Boolean(false))
    {
        features.remove("apps");
    }
    if let Some(Value::Table(projects)) = table.get_mut("projects") {
        for (_, project) in projects.iter_mut() {
            if let Value::Table(project) = project
                && project.get("trust_level").and_then(Value::as_str) == Some("untrusted")
            {
                project.remove("trust_level");
            }
        }
        projects.retain(|_, p| !matches!(p, Value::Table(t) if t.is_empty()));
    }
}

fn common_keys(table: &mut Table) {
    table.insert("check_for_update_on_startup".into(), Value::Boolean(false));
    sub_table(table, "features").insert("daemon_auto_start".into(), Value::Boolean(false));
    sub_table(table, "tui").insert(
        "terminal_title".into(),
        Value::Array(vec!["activity".into(), "thread-name".into()]),
    );
}

/// Every workspace Codex has met, and the launching one, `untrusted`: a
/// repository's own `.codex/` config, hooks and MCP servers never load, and
/// Codex's folder screen offers only "Open restricted".
fn pin_untrusted(table: &mut Table, workspace: &Path) {
    let projects = sub_table(table, "projects");
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
}

/// 131i: every remote server of an enabled plugin is off; its stdio servers
/// are on unless `allow_local` is false. Storm owns each plugin server's
/// `enabled`; a server's other policy keys are kept.
fn switch_plugin_servers(table: &mut Table, home: &Path, allow_local: bool, plan: &mut Plan) {
    let plugins = sub_table(table, "plugins");
    for (id, entry) in plugins.iter_mut() {
        let Value::Table(entry) = entry else { continue };
        if id == "_default" || entry.get("enabled").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let name = id.rsplit_once('@').map_or(id.as_str(), |(n, _)| n);
        let source = format!("Codex plugin {name}");
        let Some(servers) = installed_servers(home, id) else {
            plan.skipped.push(Skipped {
                name: name.into(),
                source,
                reason: crate::plugins::UNREADABLE,
            });
            entry.insert("enabled".into(), Value::Boolean(false));
            continue;
        };
        let overlay = sub_table(entry, "mcp_servers");
        for (server, declared) in servers {
            let remote = declared.as_object().is_none_or(crate::plugins::is_remote);
            let on = !remote && allow_local;
            sub_table(overlay, &server).insert("enabled".into(), Value::Boolean(on));
            if remote && let Some(declared) = declared.as_object() {
                plan.remote.extend(crate::plugins::remote_entry(
                    "codex", name, None, &server, declared,
                ));
            }
            let reason = if remote {
                crate::plugins::REMOTE
            } else if !allow_local {
                crate::plugins::OFF
            } else {
                plan.native.push(server);
                continue;
            };
            plan.skipped.push(Skipped {
                name: server,
                source: source.clone(),
                reason,
            });
        }
    }
}

/// The remote servers of the plugins enabled in Storm's Codex home, for the
/// host's inventory (131g).
pub fn remote_inventory(home: &Path) -> Vec<serde_json::Value> {
    let table: Table = std::fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (id, entry) in table
        .get("plugins")
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
    {
        if id == "_default" || entry.get("enabled").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let name = id.rsplit_once('@').map_or(id.as_str(), |(n, _)| n);
        for (server, declared) in installed_servers(home, id).into_iter().flatten() {
            if let Some(declared) = declared.as_object()
                && crate::plugins::is_remote(declared)
            {
                out.extend(crate::plugins::remote_entry(
                    "codex", name, None, &server, declared,
                ));
            }
        }
    }
    out
}

/// The servers every installed version of plugin `id` declares, the first
/// declaration of a name winning; `None` when one cannot be read.
fn installed_servers(home: &Path, id: &str) -> Option<Vec<(String, serde_json::Value)>> {
    let base = plugin_base(home, id)?;
    let mut out: Vec<(String, serde_json::Value)> = Vec::new();
    for version in std::fs::read_dir(base).ok()? {
        for (name, declared) in crate::plugins::declared(&version.ok()?.path())? {
            if !out.iter().any(|(n, _)| *n == name) {
                out.push((name, declared));
            }
        }
    }
    Some(out)
}

/// 131a, under `storm-only`: the account's own servers this launch withheld,
/// the workspace's `.codex/` servers (never loaded: it is pinned untrusted)
/// and ChatGPT apps when the home has a ChatGPT login.
fn name_the_rest(plan: &mut Plan, own: &[String], home: &Path, workspace: &Path) {
    for name in own {
        plan.skipped.push(Skipped {
            name: name.clone(),
            source: "your own".into(),
            reason: crate::plugins::OWN,
        });
    }
    let repo = std::fs::read_to_string(workspace.join(".codex").join("config.toml"))
        .ok()
        .and_then(|t| t.parse::<Table>().ok());
    if let Some(Value::Table(servers)) = repo.as_ref().and_then(|t| t.get("mcp_servers")) {
        for name in servers.keys() {
            plan.skipped.push(Skipped {
                name: name.clone(),
                source: "this workspace's .codex/config.toml".into(),
                reason: crate::plugins::PROJECT,
            });
        }
    }
    let auth = crate::plugins::read_json(&home.join("auth.json"));
    if !auth["tokens"].is_null() {
        plan.skipped.push(Skipped {
            name: "ChatGPT apps".into(),
            source: "your ChatGPT login".into(),
            reason: crate::plugins::APPS,
        });
    }
}

/// `plugins/cache/<marketplace>/<name>` for `<name>@<marketplace>`, split on
/// the last `@` as Codex's `PluginId::parse` does; `None` for an id whose
/// parts could leave the cache.
fn plugin_base(home: &Path, id: &str) -> Option<PathBuf> {
    let (name, marketplace) = id.rsplit_once('@')?;
    let segment_ok =
        |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', '\0']);
    if !segment_ok(name) || !segment_ok(marketplace) {
        return None;
    }
    Some(
        home.join("plugins")
            .join("cache")
            .join(marketplace)
            .join(name),
    )
}

/// Storm's `SessionStart` hook, appended to the account's, and its trust
/// record (132c-1). It fires on a fresh start and on `/clear` (a new
/// conversation id), not on a resume: Storm already knows that id, and Codex
/// 0.163 starts a resume's hook without ever feeding it, so the hook would
/// hold the session up to its timeout.
fn set_hook(table: &mut Table, exe: &Path, config: &Path) {
    let command = format!("{} harness-session", sh_quote(&exe.display().to_string()));
    let command_windows = format!("\"{}\" harness-session", exe.display());
    let mut handler = Table::new();
    handler.insert("type".into(), "command".into());
    handler.insert("command".into(), command.clone().into());
    handler.insert("commandWindows".into(), command_windows.clone().into());
    handler.insert("timeout".into(), Value::Integer(HOOK_TIMEOUT));
    let mut group = Table::new();
    group.insert("matcher".into(), HOOK_MATCHER.into());
    group.insert("hooks".into(), Value::Array(vec![Value::Table(handler)]));

    let hooks = sub_table(table, "hooks");
    if !matches!(hooks.get("SessionStart"), Some(Value::Array(_))) {
        hooks.insert("SessionStart".into(), Value::Array(Vec::new()));
    }
    let Some(Value::Array(starts)) = hooks.get_mut("SessionStart") else {
        unreachable!("inserted above")
    };
    // Storm's earlier hook, whatever binary it named.
    starts.retain(|g| !is_storms(g));
    starts.push(Value::Table(group));
    let index = starts.len() - 1;

    let hash = trusted_hash(if cfg!(windows) {
        &command_windows
    } else {
        &command
    });
    // Codex keys a hook on the path it loaded the file from: as Storm wrote
    // it, or with the home resolved.
    let mut paths = vec![config.to_path_buf()];
    if let (Some(dir), Some(name)) = (config.parent(), config.file_name())
        && let Ok(dir) = dir.canonicalize()
        && dir.join(name) != config
    {
        paths.push(dir.join(name));
    }
    let state = sub_table(sub_table(table, "hooks"), "state");
    for path in paths {
        let key = format!("{}:session_start:{index}:0", path.display());
        sub_table(state, &key).insert("trusted_hash".into(), hash.clone().into());
    }
}

fn is_storms(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|h| {
            h.get("command")
                .and_then(Value::as_str)
                .is_some_and(|c| c.ends_with("' harness-session"))
        })
}

/// Codex's hash of a `SessionStart` hook (0.163, found by gate G-132c): the
/// SHA-256 of the compact, key-sorted JSON of its event, matcher and
/// handler, with the command the platform runs.
fn trusted_hash(command: &str) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::json!({
        "event_name": "session_start",
        "hooks": [{"async": false, "command": command, "timeout": HOOK_TIMEOUT, "type": "command"}],
        "matcher": HOOK_MATCHER,
    });
    let digest = Sha256::digest(serde_json::to_vec(&canonical).expect("plain JSON"));
    format!("sha256:{}", data_encoding::HEXLOWER.encode(&digest[..]))
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
    taken: &HashSet<String>,
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
        // A profile merges into the account's config key by key, so a
        // same-named server would mix the two.
        let key = if taken.contains(&g.slug) {
            format!("storm_{}", g.slug)
        } else {
            g.slug.clone()
        };
        servers.insert(key, Value::Table(server));
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
            plugin_origin: None,
        }
    }

    fn read(path: &Path) -> Table {
        std::fs::read_to_string(path).unwrap().parse().unwrap()
    }

    const ACCOUNT: &str = r#"
model = "gpt-6"
check_for_update_on_startup = true
profile = "mine"
[mcp_servers.mine]
command = "/bin/mine"
[profiles.mine]
model = "x"
[projects."/somewhere/else"]
trust_level = "trusted"
[[hooks.SessionStart]]
[[hooks.SessionStart.hooks]]
type = "command"
command = "my-own-hook"
[tui]
screen_reader_detection_done = true
"#;

    /// Decision 132c: the account's config is its own; Storm adds its hook,
    /// trusted, and its few keys.
    #[test]
    fn the_accounts_config_is_kept_and_storms_hook_added_trusted() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), ACCOUNT).unwrap();
        let exe = Path::new("/opt/storm/storm-runtime");
        let (extras, plan) = baseline(&home, ws.path(), exe, true, false).unwrap();
        assert_eq!(extras.env, [(HOME_ENV.into(), home.as_os_str().to_owned())]);
        assert!(extras.args.is_empty());
        assert_eq!(plan, Plan::default(), "nothing is left out");
        let t = read(&home.join("config.toml"));
        assert_eq!(t["model"].as_str(), Some("gpt-6"));
        assert_eq!(
            t["mcp_servers"]["mine"]["command"].as_str(),
            Some("/bin/mine")
        );
        assert_eq!(t["profile"].as_str(), Some("mine"));
        assert!(t["profiles"].get("mine").is_some());
        assert_eq!(
            t["projects"]["/somewhere/else"]["trust_level"].as_str(),
            Some("trusted")
        );
        assert!(t.get("features").unwrap().get("apps").is_none());
        assert_eq!(t["check_for_update_on_startup"].as_bool(), Some(false));
        assert_eq!(t["features"]["daemon_auto_start"].as_bool(), Some(false));

        let starts = t["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 2, "the account's hook, then Storm's");
        assert_eq!(
            starts[0]["hooks"][0]["command"].as_str(),
            Some("my-own-hook")
        );
        assert_eq!(starts[1]["matcher"].as_str(), Some("startup|clear"));
        let command = "'/opt/storm/storm-runtime' harness-session";
        assert_eq!(starts[1]["hooks"][0]["command"].as_str(), Some(command));
        let key = format!("{}:session_start:1:0", home.join("config.toml").display());
        // The command Codex runs, and hashes, on this platform.
        let run = if cfg!(windows) {
            "\"/opt/storm/storm-runtime\" harness-session"
        } else {
            command
        };
        assert_eq!(
            t["hooks"]["state"][&key]["trusted_hash"].as_str(),
            Some(trusted_hash(run).as_str())
        );
        assert!(crate::platform::is_private(&home.join("config.toml")).unwrap());

        // A later launch, from another binary, still has one Storm hook.
        baseline(
            &home,
            ws.path(),
            Path::new("/usr/bin/storm-runtime"),
            true,
            false,
        )
        .unwrap();
        let t = read(&home.join("config.toml"));
        let starts = t["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 2);
        assert_eq!(
            starts[1]["hooks"][0]["command"].as_str(),
            Some("'/usr/bin/storm-runtime' harness-session")
        );
    }

    /// The hash Codex 0.163's `hooks/list` reported for this hook (gate
    /// G-132c), so Storm's agrees with the binary's.
    #[test]
    fn the_trust_hash_is_codexs() {
        assert_eq!(
            trusted_hash(
                "'/tmp/claude-960/-var-lib-storm-runtime-workspaces-scratch/5d78556f-d565-4993-9929-5161984f2122/scratchpad/codexbin/hook.sh'"
            ),
            "sha256:8be8cd0aa5ea017909a131747d524d248a47ae2d211ff27e845d64e96c641433"
        );
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

    /// The first launch after 132 undoes 130f's and 131's switches, once.
    #[test]
    fn storms_earlier_switches_are_undone_once() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("config.toml"),
            r#"
[plugins."hooked@m"]
enabled = false
[plugins."mine@m"]
enabled = false
[plugins."slack@m"]
enabled = true
[plugins."slack@m".mcp_servers.remote]
enabled = false
[plugins."slack@m".mcp_servers.local]
enabled = true
enabled_tools = ["a"]
[plugins._default]
enabled = false
[features]
apps = false
[projects."/a"]
trust_level = "untrusted"
"#,
        )
        .unwrap();
        std::fs::write(home.join(PLUGINS_OFF), r#"["hooked@m"]"#).unwrap();
        baseline(&home, ws.path(), Path::new("/x"), true, false).unwrap();
        let t = read(&home.join("config.toml"));
        let p = t["plugins"].as_table().unwrap();
        assert_eq!(p["hooked@m"]["enabled"].as_bool(), Some(true));
        assert_eq!(
            p["mine@m"]["enabled"].as_bool(),
            Some(false),
            "the account's own"
        );
        assert!(p.get("_default").is_none());
        let servers = p["slack@m"]["mcp_servers"].as_table().unwrap();
        assert!(servers.get("remote").is_none());
        assert!(servers["local"].get("enabled").is_none());
        assert_eq!(servers["local"]["enabled_tools"][0].as_str(), Some("a"));
        assert!(t["features"].get("apps").is_none());
        assert!(t.get("projects").is_none_or(|p| p.get("/a").is_none()));
        assert!(!home.join(PLUGINS_OFF).exists());

        // Once: an `untrusted` the account chose later stays.
        let mut t = read(&home.join("config.toml"));
        let mut project = Table::new();
        project.insert("trust_level".into(), "untrusted".into());
        sub_table(&mut t, "projects").insert("/b".into(), Value::Table(project));
        std::fs::write(home.join("config.toml"), toml::to_string(&t).unwrap()).unwrap();
        baseline(&home, ws.path(), Path::new("/x"), true, false).unwrap();
        let t = read(&home.join("config.toml"));
        assert_eq!(
            t["projects"]["/b"]["trust_level"].as_str(),
            Some("untrusted")
        );
    }

    /// `storm-only`: the account's servers are withheld, never deleted;
    /// workspaces are untrusted; a plugin's remote server and ChatGPT apps
    /// are off; everything left out is named (131i, 131l, 131a).
    #[test]
    fn storm_only_withholds_without_deleting_and_names_the_rest() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        install(
            &home,
            "slack@m",
            &[(
                ".mcp.json",
                r#"{"mcpServers":{"local":{"command":"x"},"remote":{"type":"http","url":"https://mcp.slack.com/mcp"}}}"#,
            )],
        );
        std::fs::create_dir_all(ws.path().join(".codex")).unwrap();
        std::fs::write(
            ws.path().join(".codex/config.toml"),
            "[mcp_servers.repo_srv]\ncommand = \"evil\"\n",
        )
        .unwrap();
        std::fs::write(home.join("auth.json"), r#"{"tokens":{"id_token":"x"}}"#).unwrap();
        std::fs::write(
            home.join("config.toml"),
            "[plugins.\"slack@m\"]\nenabled = true\n[mcp_servers.mine]\ncommand = \"/bin/mine\"\n[mcp_servers.\"odd.name\"]\ncommand = \"/bin/odd\"\n",
        )
        .unwrap();
        let (extras, plan) = baseline(&home, ws.path(), Path::new("/x"), true, true).unwrap();
        assert_eq!(extras.args, ["-c", "mcp_servers.mine.enabled=false"]);
        let t = read(&home.join("config.toml"));
        assert_eq!(
            t["mcp_servers"]["mine"]["command"].as_str(),
            Some("/bin/mine")
        );
        assert_eq!(
            t["projects"][&ws.path().display().to_string()]["trust_level"].as_str(),
            Some("untrusted")
        );
        assert_eq!(t["features"]["apps"].as_bool(), Some(false));
        let servers = &t["plugins"]["slack@m"]["mcp_servers"];
        assert_eq!(servers["remote"]["enabled"].as_bool(), Some(false));
        assert_eq!(servers["local"]["enabled"].as_bool(), Some(true));
        assert_eq!(plan.native, ["local"]);
        assert_eq!(remote_inventory(&home), plan.remote);
        let named: Vec<_> = plan
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason))
            .collect();
        for want in [
            ("remote", crate::plugins::REMOTE),
            ("mine", crate::plugins::OWN),
            ("repo_srv", crate::plugins::PROJECT),
            ("ChatGPT apps", crate::plugins::APPS),
        ] {
            assert!(named.contains(&want), "{want:?} in {named:?}");
        }

        let (_, off) = baseline(&home, ws.path(), Path::new("/x"), false, true).unwrap();
        assert!(off.native.is_empty());
        assert!(
            off.skipped
                .iter()
                .any(|s| s.name == "local" && s.reason == crate::plugins::OFF)
        );
    }

    #[test]
    fn an_unreadable_config_is_replaced() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let home = home(state.path());
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "[[[ not toml").unwrap();
        baseline(&home, ws.path(), Path::new("/x"), true, false).unwrap();
        let t = read(&home.join("config.toml"));
        assert_eq!(t["check_for_update_on_startup"].as_bool(), Some(false));
    }

    #[test]
    fn a_profile_holds_only_the_sessions_servers_and_asks_for_each() {
        let state = tempfile::tempdir().unwrap();
        let home = home(state.path());
        let (a, b) = (grant("storm"), grant("github"));
        let taken: HashSet<String> = ["github".to_string()].into();
        let extras = write_profile(
            &home,
            "ags_ONE",
            &[&a, &b],
            "the-handle",
            Path::new("/run/mcp.sock"),
            Path::new("/opt/storm-runtime"),
            &taken,
        )
        .unwrap();
        assert_eq!(extras.args, ["-p", "storm-ags_ONE"]);
        let path = home.join("storm-ags_ONE.config.toml");
        assert!(crate::platform::is_private(&path).unwrap());
        let t = read(&path);
        assert_eq!(t.len(), 1, "only mcp_servers");
        let servers = t["mcp_servers"].as_table().unwrap();
        assert_eq!(
            servers.keys().collect::<Vec<_>>(),
            ["storm", "storm_github"],
            "a name the account uses steps aside"
        );
        let storm = &servers["storm"];
        assert_eq!(storm["command"].as_str(), Some("/opt/storm-runtime"));
        assert_eq!(storm["args"][0].as_str(), Some("mcp-bridge"));
        assert_eq!(storm["args"][1].as_str(), Some("storm"));
        assert_eq!(servers["storm_github"]["args"][1].as_str(), Some("github"));
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
            &HashSet::new(),
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
