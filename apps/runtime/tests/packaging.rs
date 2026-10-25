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
    // AM36: and the refused identity is moved aside, which is what stops
    // launchd, which has no RestartPreventExitStatus.
    let host = repo_file("apps/runtime/src/host.rs");
    assert!(host.contains("HostConfig::mark_revoked(&self.state_dir)"));
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
    // AM37: the account is the host's alone, so the host sweeps it.
    assert!(exec.contains("--exclusive-account"), "{exec}");
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
        storm_runtime::profile::BUILT_IN.len(),
        "the built-ins by default"
    );
}

// ---- per-session isolation (decision 136) -------------------------------

const HELPER_UNIT: &str = "deploy/storm-runtime-isolation.service";
const HELPER_SOCKET: &str = "deploy/storm-runtime-isolation.socket";

/// G-136-6: isolation adds a separate helper and leaves the host's own unit
/// exactly as it was. Every directive line of storm-runtime.service is
/// pinned; changing one needs a decision, not a tidy-up.
#[test]
fn g_136_6_the_host_unit_is_unchanged_by_isolation() {
    let unit = repo_file(UNIT);
    let directives: Vec<&str> = code_lines(&unit).filter(|l| !l.is_empty()).collect();
    assert_eq!(
        directives,
        [
            "[Unit]",
            "Description=Storm Runtime Host",
            "Documentation=https://github.com/dewanshDT/Storm/blob/main/deploy/README.md",
            "After=network-online.target",
            "Wants=network-online.target",
            "StartLimitIntervalSec=60",
            "StartLimitBurst=5",
            "[Service]",
            "Type=exec",
            "User=storm-runtime",
            "Group=storm-runtime",
            "Environment=HOME=/var/lib/storm-runtime/home",
            "ExecStart=/usr/bin/storm-runtime serve --state /var/lib/storm-runtime --config /etc/storm-runtime/runtime.toml --exclusive-account",
            "StateDirectory=storm-runtime",
            "StateDirectoryMode=0700",
            "Restart=on-failure",
            "RestartSec=3",
            "RestartPreventExitStatus=3",
            "UMask=0027",
            "NoNewPrivileges=true",
            "PrivateTmp=true",
            "PrivateDevices=true",
            "ProtectSystem=strict",
            "ProtectHome=true",
            "ProtectKernelTunables=true",
            "ProtectKernelModules=true",
            "ProtectControlGroups=true",
            "LockPersonality=true",
            "RestrictNamespaces=true",
            "RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX AF_NETLINK",
            "[Install]",
            "WantedBy=multi-user.target",
        ]
    );
}

fn unit_values<'a>(unit: &'a str, key: &str) -> Vec<&'a str> {
    let prefix = format!("{key}=");
    code_lines(unit)
        .filter_map(|l| l.strip_prefix(prefix.as_str()))
        .collect()
}

/// G-136-2's half that a file can hold: the helper keeps three
/// capabilities, each with its reason in the unit, runs nothing but its
/// own binary and systemd's tools, and is sandboxed in every way that does
/// not stop it doing its job.
#[test]
fn the_helper_unit_keeps_only_what_it_needs() {
    let unit = repo_file(HELPER_UNIT);
    for (key, value) in [
        ("ExecStart", "/usr/sbin/storm-runtime-isolation serve"),
        ("User", "root"),
        (
            "CapabilityBoundingSet",
            "CAP_DAC_READ_SEARCH CAP_CHOWN CAP_FOWNER",
        ),
        ("ReadWritePaths", "/var/lib/storm-runtime"),
        ("AmbientCapabilities", ""),
        ("NoNewPrivileges", "true"),
        ("PrivateNetwork", "true"),
        ("RestrictAddressFamilies", "AF_UNIX"),
        ("IPAddressDeny", "any"),
        ("PrivateTmp", "true"),
        ("PrivateDevices", "true"),
        ("PrivateIPC", "true"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "true"),
        ("ProtectKernelTunables", "true"),
        ("ProtectKernelModules", "true"),
        ("ProtectKernelLogs", "true"),
        ("ProtectControlGroups", "true"),
        ("ProtectProc", "invisible"),
        ("RestrictNamespaces", "true"),
        ("LockPersonality", "true"),
        ("MemoryDenyWriteExecute", "true"),
        ("SystemCallArchitectures", "native"),
        ("UMask", "0077"),
        ("StateDirectoryMode", "0700"),
        ("Requires", "storm-runtime-isolation.socket"),
    ] {
        assert_eq!(unit_values(&unit, key), [value], "{key}");
    }
    assert_eq!(
        unit_values(&unit, "SystemCallFilter"),
        [
            "@system-service",
            "~@privileged @resources @mount",
            "@chown"
        ]
    );
    // Each capability is named in a comment that says what it is for.
    let comments: String = unit.lines().filter(|l| l.starts_with('#')).collect();
    for cap in ["CAP_DAC_READ_SEARCH", "CAP_CHOWN", "CAP_FOWNER"] {
        assert!(
            comments.contains(&format!("# - {cap}:")),
            "{cap} has no reason"
        );
    }
    assert!(comments.contains("Not RestrictSUIDSGID"));
    // Socket-activated only: never started at boot on its own, and never
    // given a writable path beyond its own state and the host's.
    assert!(!unit.contains("[Install]"));
    for key in [
        "RestrictSUIDSGID",
        "ExecStartPre",
        "ExecStartPost",
        "Environment",
        "SupplementaryGroups",
    ] {
        assert!(unit_values(&unit, key).is_empty(), "{key}");
    }
}

/// 136c: a socket only the host account and root can reach.
#[test]
fn the_helper_socket_is_the_host_accounts_alone() {
    let unit = repo_file(HELPER_SOCKET);
    for (key, value) in [
        (
            "ExecStartPre",
            "/usr/bin/install -d -m 0710 -o root -g storm-runtime /run/storm-runtime-isolation",
        ),
        ("ListenStream", "/run/storm-runtime-isolation/helper.sock"),
        ("SocketUser", "root"),
        ("SocketGroup", "storm-runtime"),
        ("SocketMode", "0660"),
        ("DirectoryMode", "0710"),
        ("Accept", "false"),
    ] {
        assert_eq!(unit_values(&unit, key), [value], "{key}");
    }
}

#[test]
fn the_package_ships_the_helper_disabled() {
    let cargo = repo_file("apps/runtime/Cargo.toml");
    for asset in [
        r#"["isolation/target/release/storm-runtime-isolation", "usr/sbin/", "755"]"#,
        r#"["../../deploy/storm-runtime-isolation.socket", "lib/systemd/system/", "644"]"#,
        r#"["../../deploy/storm-runtime-isolation.service", "lib/systemd/system/", "644"]"#,
        r#"["../../deploy/storm-runtime-isolation.sysusers", "usr/share/storm-runtime-isolation/sysusers.conf", "644"]"#,
    ] {
        assert!(cargo.contains(asset), "{asset}");
    }
    assert!(cargo.contains(r#"recommends = "passt""#));
    // A host with isolation off has no pool accounts and no helper running:
    // nothing at install creates the one or enables the other.
    for script in ["postinst", "prerm", "postrm"] {
        let text = repo_file(&format!("apps/runtime/debian/{script}"));
        for line in code_lines(&text) {
            assert!(!line.contains("sysusers"), "{script}: {line}");
            assert!(
                !line.contains("storm-s0") && !line.contains("storm-w0"),
                "{script}: {line}"
            );
            assert!(
                !line.contains("useradd") || line.contains("storm-runtime"),
                "{script}: {line}"
            );
            if line.contains("storm-runtime-isolation") {
                assert!(
                    !line.contains("enable") || line.contains("disable"),
                    "{script}: {line}"
                );
                assert!(!line.contains(" start "), "{script}: {line}");
            }
        }
    }
}

/// The helper accepts a provider only by an id the operator's configuration
/// has; with none configured, the built-ins. Its list is this crate's.
#[test]
fn the_helper_knows_the_same_built_in_providers() {
    let helper = repo_file("apps/runtime/isolation/src/config.rs");
    let start = helper
        .find("pub const BUILT_IN_PROVIDERS")
        .expect("BUILT_IN_PROVIDERS");
    let list = &helper[start..start + helper[start..].find("];").unwrap()];
    let list = &list[list.find('[').unwrap()..];
    let ids: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
    let ours: Vec<&str> = storm_runtime::profile::BUILT_IN
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, ours);
    // And what each runs: the helper resolves the program itself (136c).
    let start = helper
        .find("pub const BUILT_IN_COMMANDS")
        .expect("BUILT_IN_COMMANDS");
    let list = &helper[start..start + helper[start..].find("];").unwrap()];
    let list = &list[list.find('[').unwrap()..];
    let quoted: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
    let commands: Vec<(&str, &str)> = quoted.chunks(2).map(|c| (c[0], c[1])).collect();
    let ours: Vec<(&str, &str)> = storm_runtime::profile::BUILT_IN
        .iter()
        .map(|p| (p.id, p.command))
        .collect();
    assert_eq!(commands, ours);
}

#[test]
fn the_example_config_leaves_isolation_off() {
    let text = repo_file("deploy/runtime.toml.example");
    let config: storm_runtime::config::RuntimeConfig = toml::from_str(&text).unwrap();
    assert_eq!(config.isolation, storm_runtime::config::Isolation::Off);
    assert!(text.contains("# isolation = \"off\""));
}
