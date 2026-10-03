//! The package's files keep the promises the unit makes (decision 77e).
//!
//! These read the real files the `.deb` ships. A unit and a package that
//! disagree about a path fail only on a real box (the 203/EXEC of decision 75),
//! and a sandbox setting someone "tidies up" fails only when OpenCode starts.

use std::path::Path;

fn repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn code_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|l| !l.starts_with('#'))
}

fn unit_value(unit: &str, key: &str) -> Option<String> {
    code_lines(unit).find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
}

const UNIT: &str = "deploy/storm-runtime.service";

#[test]
fn memory_deny_write_execute_is_never_set() {
    // Gate Q6: OpenCode's TUI dies under it ("tcc: error: mprotect failed").
    let unit = repo_file(UNIT);
    assert!(
        unit_value(&unit, "MemoryDenyWriteExecute").is_none(),
        "the runtime unit must not set MemoryDenyWriteExecute (freeze §5.8)"
    );
}

#[test]
fn the_unit_keeps_the_frozen_hardening() {
    let unit = repo_file(UNIT);
    for (key, value) in [
        ("User", "storm-runtime"),
        ("Group", "storm-runtime"),
        ("NoNewPrivileges", "true"),
        ("PrivateTmp", "true"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "true"),
        ("ProtectKernelTunables", "true"),
        ("ProtectKernelModules", "true"),
        ("ProtectControlGroups", "true"),
        ("LockPersonality", "true"),
        ("RestrictNamespaces", "true"),
        ("UMask", "0027"),
        ("StateDirectoryMode", "0700"),
    ] {
        assert_eq!(unit_value(&unit, key).as_deref(), Some(value), "{key}");
    }
    let families = unit_value(&unit, "RestrictAddressFamilies").unwrap();
    assert!(families.contains("AF_NETLINK"), "{families}");
}

#[test]
fn a_revoked_host_is_not_restarted() {
    // main.rs exits 3 when the server refuses the key (freeze §5.6).
    let unit = repo_file(UNIT);
    assert_eq!(
        unit_value(&unit, "RestartPreventExitStatus").as_deref(),
        Some("3")
    );
    let main = repo_file("apps/runtime/src/main.rs");
    assert!(
        main.contains("std::process::exit(3)"),
        "main must exit 3 on refusal"
    );
}

#[test]
fn the_unit_runs_what_the_package_installs_where_it_installs_it() {
    let unit = repo_file(UNIT);
    let exec = unit_value(&unit, "ExecStart").expect("ExecStart");
    assert!(exec.starts_with("/usr/bin/storm-runtime serve"), "{exec}");
    let cargo = repo_file("apps/runtime/Cargo.toml");
    assert!(cargo.contains(r#"["target/release/storm-runtime", "usr/bin/", "755"]"#));
    assert!(cargo.contains(r#""../../deploy/storm-runtime.service", "lib/systemd/system/""#));
    assert!(exec.contains("--config /etc/storm-runtime/runtime.toml"));
    assert!(cargo.contains(r#""etc/storm-runtime/runtime.toml""#));
    // HOME is the account's home, where an agent CLI was logged in.
    assert_eq!(
        unit_value(&unit, "Environment").as_deref(),
        Some("HOME=/var/lib/storm-runtime/home")
    );
    let postinst = repo_file("apps/runtime/debian/postinst");
    assert!(postinst.contains("--home-dir /var/lib/storm-runtime/home"));
}

#[test]
fn nothing_puts_the_runtime_in_another_group() {
    // P3 / AC-S3: the server's group mode is what keeps the runtime out of
    // the vaults and auth.db. No script may add it to a group.
    for script in ["postinst", "prerm", "postrm"] {
        let text = repo_file(&format!("apps/runtime/debian/{script}"));
        for line in code_lines(&text) {
            assert!(
                !(line.contains("usermod") || line.contains("gpasswd") || line.contains("adduser")),
                "{script}: {line}"
            );
            assert!(!line.contains(" -G "), "{script}: {line}");
        }
    }
}

#[test]
fn an_upgrade_never_stops_the_host() {
    // dpkg runs the OLD package's prerm with `upgrade` (#46).
    let prerm = repo_file("apps/runtime/debian/prerm");
    assert!(
        prerm.contains("remove|deconfigure)"),
        "prerm must act on removal only"
    );
}

#[test]
fn the_example_config_parses_and_points_inside_the_sandbox() {
    let text = repo_file("deploy/runtime.toml.example");
    let config: storm_runtime::config::RuntimeConfig = toml::from_str(&text).unwrap();
    assert_eq!(
        config.workspace_roots,
        [std::path::PathBuf::from(
            "/var/lib/storm-runtime/workspaces"
        )]
    );
    assert_eq!(
        config.providers().unwrap().len(),
        3,
        "the built-ins by default"
    );
}
