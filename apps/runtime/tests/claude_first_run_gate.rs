//! An isolated Claude Code session starts past its first run (decision
//! 142), against a real `claude` binary.
//!
//! Runs only when `STORM_CLAUDE_BIN` names one; CI has none, so it passes as
//! a no-op there. No account or network is needed: both screens come before
//! the model is reached. Each start gets a fresh temporary home and folder,
//! and no credential; it runs `claude` in a terminal through `script` for a
//! few seconds and reads what it drew.
//!
//! - With the `.claude.json` the helper's filter writes for that folder,
//!   neither the theme picker nor the trust prompt appears, and the prompt
//!   does.
//! - The control: with the `{}` of an account that never ran Claude Code,
//!   the theme picker does, and once onboarded, the trust prompt does.
#![cfg(target_os = "linux")]

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use storm_runtime_isolation::home::claude_json;

const SECS: u32 = 8;
const THEME: &str = "Choosethetextstyle";
const TRUST: &str = "trustthisfolder";
const PROMPT: &str = "shift+tab";

/// What `claude` drew in `ws` with `config` as its `.claude.json`, with
/// escape sequences and whitespace removed (it places words by cursor).
fn first_screens(bin: &str, config: &[u8], ws: &Path) -> String {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".claude.json"), config).unwrap();
    let mut child = Command::new("/usr/bin/script")
        .args(["-qfec"])
        .arg(format!(
            "stty cols 100 rows 40 && exec timeout -s KILL {SECS} '{bin}'"
        ))
        .arg("/dev/null")
        .current_dir(ws)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.path())
        .env("SHELL", "/bin/sh")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    let _ = child.wait();
    let text = String::from_utf8_lossy(&out);
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    for c in chars.by_ref() {
                        if c == '\x07' {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_whitespace() {
            plain.push(c);
        }
    }
    plain
}

#[test]
fn g_142_an_isolated_claude_code_starts_past_its_first_run() {
    let Some(bin) = std::env::var("STORM_CLAUDE_BIN").ok() else {
        eprintln!("STORM_CLAUDE_BIN is not set; skipping the Claude Code first-run gate");
        return;
    };
    let fresh = |config: &dyn Fn(&Path) -> Vec<u8>| {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().canonicalize().unwrap();
        first_screens(&bin, &config(&ws), &ws)
    };

    let past = fresh(&|ws| claude_json(b"{}", ws).unwrap());
    assert!(!past.contains(THEME), "the theme picker appeared: {past}");
    assert!(!past.contains(TRUST), "the trust prompt appeared: {past}");
    assert!(past.contains(PROMPT), "no prompt: {past}");

    let never_ran = fresh(&|_| b"{}".to_vec());
    assert!(never_ran.contains(THEME), "no theme picker: {never_ran}");

    let onboarded = fresh(&|_| br#"{"hasCompletedOnboarding":true}"#.to_vec());
    assert!(onboarded.contains(TRUST), "no trust prompt: {onboarded}");
}
