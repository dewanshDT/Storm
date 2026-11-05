//! Decision 133's recipes against the vendors' real channels (gates
//! G-133-1…5).
//!
//! Runs only with `STORM_AGENT_INSTALL_GATE=1`: it downloads hundreds of
//! megabytes. Run it as the host's account, under the host's unit, to prove
//! the recipes work inside its sandbox. Each agent is installed into a fresh
//! state directory, its binary answers `--version` through `bin/`, an update
//! keeps one previous release, and a remove leaves nothing behind.
#![cfg(unix)]

use std::path::Path;

use storm_runtime::install::{self, Op};

fn path_with(state: &Path) -> std::ffi::OsString {
    let mut dirs = vec![storm_runtime::agents::managed_bin(state)];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(dirs).unwrap()
}

async fn gate(agent: &str) {
    let state =
        tempfile::tempdir_in(std::env::var_os("STORM_GATE_DIR").unwrap_or(".".into())).unwrap();
    let path = path_with(state.path());
    let steps = std::sync::Mutex::new(Vec::new());
    let step = |s: &'static str| steps.lock().unwrap().push(s);

    let version = install::run(state.path(), agent, Op::Install, Some(path.clone()), &step)
        .await
        .unwrap_or_else(|e| panic!("{agent}: install failed: {}", e.code()));
    let version = version.unwrap();
    eprintln!(
        "{agent}: installed {version}, steps {:?}",
        steps.lock().unwrap()
    );
    let target = install::managed_target(state.path(), agent).expect("bin/ points at it");
    assert!(target.starts_with(storm_runtime::agents::managed_dir(state.path())));
    assert_eq!(
        storm_runtime::agents::source(&target, state.path()),
        "storm",
        "{agent}"
    );

    let updated = install::run(state.path(), agent, Op::Update, Some(path.clone()), &step)
        .await
        .unwrap_or_else(|e| panic!("{agent}: update failed: {}", e.code()));
    assert_eq!(
        updated.as_deref(),
        Some(version.as_str()),
        "{agent}: same latest"
    );

    install::run(state.path(), agent, Op::Remove, Some(path.clone()), &step)
        .await
        .unwrap_or_else(|e| panic!("{agent}: remove failed: {}", e.code()));
    assert!(
        install::managed_target(state.path(), agent).is_none(),
        "{agent}"
    );
    assert_eq!(
        install::run(state.path(), agent, Op::Remove, Some(path), &step).await,
        Err(install::Failure::NotManaged),
        "{agent}: a second remove has nothing of Storm's to remove"
    );
}

macro_rules! gate {
    ($name:ident, $agent:literal) => {
        #[tokio::test]
        async fn $name() {
            if std::env::var("STORM_AGENT_INSTALL_GATE").as_deref() != Ok("1") {
                eprintln!("STORM_AGENT_INSTALL_GATE is not set; skipping");
                return;
            }
            gate($agent).await;
        }
    };
}

gate!(g_133_1_claude_code, "claude-code");
gate!(g_133_2_opencode, "opencode");
gate!(g_133_3_cursor, "cursor");
gate!(g_133_4_codex, "codex");
gate!(g_133_4_command_code, "command-code");
