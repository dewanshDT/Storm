//! The Codex adapter against a real `codex` binary (decision 130b).
//!
//! Runs only when `STORM_CODEX_BIN` names one (`npm i @openai/codex` puts it
//! in `node_modules/.bin/codex`); CI has none, so it passes as a no-op there.
//! No account is needed: Codex is logged in with a fake key, and everything
//! checked here happens before the model is reached. What it proves, with
//! the config Storm's own code writes:
//!
//! - a session lists exactly its own servers, never the account's home
//!   config's nor a hostile repository's `.codex/config.toml` (AM32);
//! - a session with no profile lists none;
//! - Storm's `SessionStart` hook reports the conversation id through the
//!   real `storm-runtime harness-session`, with the session's handle, and a
//!   repository's own hooks never run;
//! - a plugin installed from a marketplace runs no hook, and one without
//!   hooks keeps its server (130f) but not its remote one (131i).
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use storm_runtime::codex;
use storm_runtime::mcp::McpGrant;

const RUNTIME: &str = env!("CARGO_BIN_EXE_storm-runtime");

fn codex_cmd(bin: &str, home: &Path, user_home: &Path, workspace: &Path) -> Command {
    let mut c = Command::new(bin);
    c.current_dir(workspace)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", user_home)
        .env(codex::HOME_ENV, home)
        .stdin(Stdio::null());
    c
}

#[test]
fn storms_codex_config_holds_against_the_real_binary() {
    let Some(bin) = std::env::var("STORM_CODEX_BIN").ok() else {
        eprintln!("STORM_CODEX_BIN is not set; skipping the Codex gate");
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let canary = ws.path().join("REPO-HOOK-RAN");
    std::fs::create_dir(ws.path().join(".git")).unwrap();
    std::fs::create_dir(ws.path().join(".codex")).unwrap();
    std::fs::write(
        ws.path().join(".codex/config.toml"),
        format!(
            "[mcp_servers.repo_server]\ncommand = \"/bin/echo\"\n\
             [[hooks.SessionStart]]\n[[hooks.SessionStart.hooks]]\n\
             type = \"command\"\ncommand = \"touch {}\"\n",
            canary.display()
        ),
    )
    .unwrap();
    std::fs::create_dir_all(user_home.path().join(".codex")).unwrap();
    std::fs::write(
        user_home.path().join(".codex/config.toml"),
        "[mcp_servers.account_server]\ncommand = \"/bin/echo\"\n",
    )
    .unwrap();

    let home = codex::home(state.path());
    codex::baseline(&home, ws.path(), Path::new(RUNTIME), true).unwrap();
    let login = codex_cmd(&bin, &home, user_home.path(), ws.path())
        .args(["login", "--with-api-key"])
        .stdin(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"sk-not-a-real-key")?;
            child.wait()
        })
        .unwrap();
    assert!(login.success(), "codex login");

    // 130f: plugins installed the way the account would install them, from a
    // marketplace. Codex runs a plugin's hooks under Storm's trust bypass, so
    // the one with a hook must be off; the one without keeps its server.
    let market = tempfile::tempdir().unwrap();
    let plugin_canary = ws.path().join("PLUGIN-HOOK-RAN");
    let write = |path: &str, body: String| {
        let path = market.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    write(
        ".claude-plugin/marketplace.json",
        r#"{"name":"gate","owner":{"name":"gate"},"plugins":[
            {"name":"hooked","source":"./hooked"},{"name":"plain","source":"./plain"}]}"#
            .into(),
    );
    for name in ["hooked", "plain"] {
        write(
            &format!("{name}/.claude-plugin/plugin.json"),
            format!(r#"{{"name":"{name}","version":"1.0.0"}}"#),
        );
    }
    write(
        "hooked/hooks/hooks.json",
        format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"touch {}"}}]}}]}}}}"#,
            plugin_canary.display()
        ),
    );
    write(
        "plain/.mcp.json",
        r#"{"mcpServers":{"plain_server":{"command":"/bin/cat"},"plain_remote":{"type":"http","url":"https://mcp.slack.com/mcp"}}}"#.into(),
    );
    for args in [
        vec![
            "plugin",
            "marketplace",
            "add",
            &market.path().to_string_lossy(),
        ],
        vec!["plugin", "add", "hooked@gate"],
        vec!["plugin", "add", "plain@gate"],
    ] {
        let out = codex_cmd(&bin, &home, user_home.path(), ws.path())
            .args(&args)
            .output()
            .unwrap();
        assert!(out.status.success(), "codex {args:?}: {out:?}");
    }
    codex::baseline(&home, ws.path(), Path::new(RUNTIME), true).unwrap();

    let socket_dir = tempfile::tempdir().unwrap();
    let socket = socket_dir.path().join("mcp.sock");
    let grant = McpGrant {
        id: "mcc_storm".into(),
        slug: "storm".into(),
        plugin_origin: None,
    };
    codex::write_profile(
        &home,
        "ags_GATE",
        &[&grant],
        "gate-handle",
        &socket,
        Path::new(RUNTIME),
    )
    .unwrap();

    let list = |profile: Option<&str>| -> String {
        let mut c = codex_cmd(&bin, &home, user_home.path(), ws.path());
        if let Some(p) = profile {
            c.args(["-p", p]);
        }
        let out = c.args(["mcp", "list"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let with = list(Some(&codex::profile_name("ags_GATE")));
    assert!(with.contains("storm"), "{with}");
    assert!(
        with.contains("plain_server"),
        "a plugin without hooks lost its server: {with}"
    );
    // 131i: its remote server is listed, and off.
    let remote = with
        .lines()
        .find(|l| l.contains("plain_remote"))
        .unwrap_or_else(|| panic!("no plain_remote in {with}"));
    assert!(remote.contains("disabled"), "{remote}");
    assert!(
        !with.contains("repo_server"),
        "a repository's server loaded: {with}"
    );
    assert!(
        !with.contains("account_server"),
        "the account's server loaded: {with}"
    );
    let without = list(None);
    assert!(
        !without.contains("storm") && !without.contains("repo_server"),
        "{without}"
    );

    // The hook: `codex exec` starts a conversation (and fails at the model,
    // which a fake key cannot reach) after running SessionStart.
    let listener = UnixListener::bind(&socket).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    // The session's MCP bridge dials the same socket as the hook (Codex
    // starts its servers too); only the hook's line names a conversation.
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                let _ = BufReader::new(stream).read_line(&mut line);
                if line.contains("harness_session") {
                    let _ = tx.send(line);
                }
            });
        }
    });
    let profile = storm_runtime::profile::get("codex").unwrap();
    let mut exec = codex_cmd(&bin, &home, user_home.path(), ws.path());
    exec.arg("exec");
    // `exec` takes the session flags but not the TUI's daemon or approval
    // ones.
    let mut args = profile.args.iter().copied().peekable();
    while let Some(a) = args.next() {
        match a {
            "--no-daemon" => {}
            "--ask-for-approval" => {
                args.next();
            }
            _ => {
                exec.arg(a);
            }
        }
    }
    let status = exec
        .args([
            "-p",
            &codex::profile_name("ags_GATE"),
            "--skip-git-repo-check",
            "hi",
        ])
        .env(storm_runtime::harness::HANDLE_ENV, "gate-handle")
        .env(storm_runtime::mcp::SOCKET_ENV, &socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let _ = status; // the model is unreachable; the hook ran before it
    let line = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("Storm's hook never reported");
    let reported: storm_runtime::harness::HarnessLine = serde_json::from_str(&line).unwrap();
    assert_eq!(reported.handle, "gate-handle");
    assert!(
        storm_runtime::harness::valid_id("codex", &reported.harness_session),
        "{line}"
    );
    assert!(!canary.exists(), "a repository's hook ran");
    assert!(
        !plugin_canary.exists(),
        "a plugin's hook ran under the trust bypass (130f)"
    );
}
