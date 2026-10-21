//! The Cursor and Command Code writers against the real binaries (131j, 131k).
//!
//! Each runs only when its variable names a binary — `STORM_CURSOR_BIN`
//! (`cursor-agent`) and `STORM_COMMAND_CODE_BIN` (`command-code`); CI has
//! neither, so both pass as no-ops there. No account is needed: each CLI's
//! own `mcp list` reads the config Storm wrote. What it proves:
//!
//! - Cursor reads the disabled list where Storm writes it, so a hostile
//!   repository's and the account's servers are `disabled`;
//! - Command Code takes Storm's local file over the repository's `.mcp.json`
//!   and the account's own servers: each is `disabled`, the bridge `enabled`.
//!
//! Neither shows a session's servers actually starting, which needs a
//! signed-in account (G-C1, G-CC1); until then both profiles keep
//! `mcp: None`.
#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Stdio};

use storm_runtime::mcp::McpGrant;

fn listing(bin: &str, home: &Path, workspace: &Path) -> String {
    let out = Command::new(bin)
        .args(["mcp", "list"])
        .current_dir(workspace)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

fn grant() -> McpGrant {
    McpGrant {
        id: "mcc_storm".into(),
        slug: "storm".into(),
        plugin_origin: None,
    }
}

#[test]
fn cursor_reads_storms_disabled_list() {
    let Some(bin) = std::env::var("STORM_CURSOR_BIN").ok() else {
        eprintln!("STORM_CURSOR_BIN is not set; skipping the Cursor gate");
        return;
    };
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("Repo_X");
    let ws = repo.join("sub");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join(".cursor")).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        repo.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"evil":{"command":"/bin/echo"}}}"#,
    )
    .unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    std::fs::write(
        home.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"mine":{"command":"/bin/echo"}}}"#,
    )
    .unwrap();
    let before = listing(&bin, &home, &ws);
    assert!(before.contains("evil: not loaded"), "{before}");
    let storm = grant();
    storm_runtime::cursor::write_session(
        &t.path().join("ags_C"),
        &ws,
        &home,
        &home.join(".cursor"),
        &[&storm],
        "h",
        Path::new("/s"),
        Path::new("/x"),
    )
    .unwrap();
    let after = listing(&bin, &home, &ws);
    assert!(after.contains("evil: disabled"), "{after}");
    assert!(after.contains("mine: disabled"), "{after}");
}

#[test]
fn command_code_takes_storms_local_file() {
    let Some(bin) = std::env::var("STORM_COMMAND_CODE_BIN").ok() else {
        eprintln!("STORM_COMMAND_CODE_BIN is not set; skipping the Command Code gate");
        return;
    };
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("My Repo.v2");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        ws.join(".mcp.json"),
        r#"{"mcpServers":{"evil":{"command":"/bin/echo"}}}"#,
    )
    .unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(home.join(".commandcode")).unwrap();
    std::fs::write(
        home.join(".commandcode/mcp.json"),
        r#"{"mcpServers":{"mine":{"command":"/bin/echo"}}}"#,
    )
    .unwrap();
    let row = |text: &str, name: &str| {
        text.lines()
            .find(|l| l.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("no {name} in {text}"))
            .to_string()
    };
    let before = listing(&bin, &home, &ws);
    assert!(row(&before, "evil").ends_with("enabled"), "{before}");
    let storm = grant();
    storm_runtime::command_code::write_session(
        &ws,
        &home,
        &[&storm],
        "h",
        Path::new("/s"),
        Path::new("/bin/cat"),
    )
    .unwrap();
    let after = listing(&bin, &home, &ws);
    assert!(row(&after, "evil").ends_with("disabled"), "{after}");
    assert!(row(&after, "mine").ends_with("disabled"), "{after}");
    let bridge = row(&after, "storm");
    assert!(
        bridge.ends_with("enabled") && !bridge.ends_with("disabled"),
        "{after}"
    );
}
