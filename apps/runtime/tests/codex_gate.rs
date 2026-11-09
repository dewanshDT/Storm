//! The Codex adapter against a real `codex` binary (decisions 130b, 132c).
//!
//! Runs only when `STORM_CODEX_BIN` names one (`npm i @openai/codex` puts it
//! in `node_modules/.bin/codex`); CI has none, so it passes as a no-op there.
//! No account is needed: Codex is logged in with a fake key, and everything
//! checked here happens before the model is reached. What it proves, with
//! the config Storm's own code writes:
//!
//! - a session lists its own servers **and** the account's (132c), and a
//!   plugin's, local and remote; never an untrusted repository's;
//! - Storm's `SessionStart` hook runs **without
//!   `--dangerously-bypass-hook-trust`**, because Storm wrote its trust
//!   record, and reports the conversation id through the real
//!   `storm-runtime harness-session`; a repository's hook and a plugin's,
//!   unreviewed, never run;
//! - under `storm-only`, the account's server and a plugin's remote one are
//!   listed but disabled;
//! - in the account's own home (decision 151 N3), Storm's servers and its
//!   trusted hook arrive as `-c` overrides alone, and the home's files are
//!   byte for byte what they were.
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

    let home = codex::home(state.path());
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[mcp_servers.account_server]\ncommand = \"/bin/echo\"\n",
    )
    .unwrap();
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

    // Plugins installed the way the account would install them, from a
    // marketplace: one with a hook, one with a local and a remote server.
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
    codex::baseline(&home, ws.path(), Path::new(RUNTIME), true, false).unwrap();

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
        &codex::own_servers(&home),
        true,
    )
    .unwrap();

    let list = |extra: &[String]| -> String {
        let mut c = codex_cmd(&bin, &home, user_home.path(), ws.path());
        c.args(["-p", &codex::profile_name("ags_GATE")]).args(extra);
        let out = c.args(["mcp", "list"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let line = |listing: &str, name: &str| -> String {
        listing
            .lines()
            .find(|l| l.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("no {name} in {listing}"))
            .to_string()
    };
    let own = list(&[]);
    for name in ["storm", "account_server", "plain_server", "plain_remote"] {
        assert!(line(&own, name).contains("enabled"), "{name}: {own}");
    }
    assert!(
        !own.contains("repo_server"),
        "an untrusted repository's server loaded: {own}"
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
    assert!(
        !profile.args.iter().any(|a| a.contains("bypass")),
        "{:?}",
        profile.args
    );
    let mut exec = codex_cmd(&bin, &home, user_home.path(), ws.path());
    exec.arg("exec");
    // `exec` takes the session flags but not the TUI's daemon or approval
    // ones.
    let mut args = profile
        .args
        .iter()
        .chain(profile.service_args)
        .copied()
        .peekable();
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
    let reported = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("Storm's hook never ran: its trust record was not accepted");
    let reported: storm_runtime::harness::HarnessLine = serde_json::from_str(&reported).unwrap();
    assert_eq!(reported.handle, "gate-handle");
    assert!(
        storm_runtime::harness::valid_id("codex", &reported.harness_session),
        "{reported:?}"
    );
    assert!(!canary.exists(), "a repository's hook ran");
    assert!(!plugin_canary.exists(), "a plugin's unreviewed hook ran");

    // storm-only: the account's server and a plugin's remote one are off.
    let (extras, _) = codex::baseline(&home, ws.path(), Path::new(RUNTIME), true, true).unwrap();
    let extra: Vec<String> = extras
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let only = list(&extra);
    assert!(line(&only, "storm").contains("enabled"), "{only}");
    assert!(line(&only, "account_server").contains("disabled"), "{only}");
    assert!(line(&only, "plain_remote").contains("disabled"), "{only}");
}

/// The account's own files in its Codex home, with their bytes; Codex's
/// databases, logs and sessions beside them are its own to change.
fn account_files(dir: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for name in ["config.toml", "auth.json"] {
        let path = dir.join(name);
        out.push((path.clone(), std::fs::read(&path).unwrap()));
    }
    out
}

#[test]
fn the_accounts_own_home_holds_against_the_real_binary() {
    let Some(bin) = std::env::var("STORM_CODEX_BIN").ok() else {
        eprintln!("STORM_CODEX_BIN is not set; skipping the Codex gate");
        return;
    };
    let user_home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    std::fs::create_dir(ws.path().join(".git")).unwrap();
    let home = user_home.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let own_canary = ws.path().join("OWN-HOOK-RAN");
    std::fs::write(
        home.join("config.toml"),
        format!(
            "[mcp_servers.account_server]\ncommand = \"/bin/echo\"\n\
             [[hooks.SessionStart]]\n[[hooks.SessionStart.hooks]]\n\
             type = \"command\"\ncommand = \"touch {}\"\n",
            own_canary.display()
        ),
    )
    .unwrap();
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
    let before = account_files(&home);

    let (baseline, _) =
        codex::baseline_own(&home, ws.path(), Path::new(RUNTIME), true, false).unwrap();
    let socket_dir = tempfile::tempdir().unwrap();
    let socket = socket_dir.path().join("mcp.sock");
    let grant = McpGrant {
        id: "mcc_storm".into(),
        slug: "storm".into(),
        plugin_origin: None,
    };
    let servers = codex::session_servers(
        &[&grant],
        "gate-handle",
        &socket,
        Path::new(RUNTIME),
        &codex::own_servers(&home),
        false,
    );
    let flags: Vec<String> = baseline
        .args
        .iter()
        .chain(&servers.args)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    let out = codex_cmd(&bin, &home, user_home.path(), ws.path())
        .args(&flags)
        .args(["mcp", "list"])
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&out.stdout).into_owned();
    for name in ["storm", "account_server"] {
        assert!(
            listing
                .lines()
                .any(|l| l.split_whitespace().next() == Some(name) && l.contains("enabled")),
            "{name}: {listing}"
        );
    }

    let listener = UnixListener::bind(&socket).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
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
    let mut exec = codex_cmd(&bin, &home, user_home.path(), ws.path());
    exec.arg("exec").args(&flags);
    for (k, v) in &servers.env {
        exec.env(k, v);
    }
    let _ = exec
        .args(["--skip-git-repo-check", "hi"])
        .env(storm_runtime::harness::HANDLE_ENV, "gate-handle")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let reported = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("Storm's -c hook never ran: its -c trust record was not accepted");
    let reported: storm_runtime::harness::HarnessLine = serde_json::from_str(&reported).unwrap();
    assert_eq!(reported.handle, "gate-handle");
    assert!(
        !own_canary.exists(),
        "the account's unreviewed hook ran beside Storm's"
    );
    assert_eq!(
        account_files(&home),
        before,
        "Storm wrote to the account's home"
    );
}
