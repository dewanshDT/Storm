//! The package's files agree with each other (decision 75).
//!
//! `storm-backup.service` pointed at `/usr/local/bin/storm-backup.sh`, a path
//! from M6's hand install, while the `.deb` has installed the script to
//! `/usr/bin` since M15. Every packaged backup since failed with `203/EXEC`, and
//! nothing noticed for more than a month. These read the real files.

use std::path::Path;

fn repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// A unit file's or shell script's lines, trimmed, without comments.
fn code_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|l| !l.starts_with('#'))
}

/// The value of `key=` in a unit file, ignoring comments.
fn unit_value(unit: &str, key: &str) -> Option<String> {
    code_lines(unit).find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
}

#[test]
fn the_backup_unit_runs_the_script_where_the_package_installs_it() {
    let unit = repo_file("deploy/storm-backup.service");
    let exec = unit_value(&unit, "ExecStart").expect("ExecStart");
    let installed_dir = {
        let cargo = repo_file("apps/server/Cargo.toml");
        let line = cargo
            .lines()
            .find(|l| l.contains("\"../../deploy/storm-backup.sh\""))
            .expect("storm-backup.sh in the .deb's assets");
        // ["../../deploy/storm-backup.sh", "usr/bin/", "755"]
        let dest = line.split('"').nth(3).expect("an asset destination");
        format!("/{dest}")
    };
    assert_eq!(
        exec,
        format!("{installed_dir}storm-backup.sh"),
        "the unit runs a path the package does not install"
    );
}

#[test]
fn the_backup_unit_gets_its_environment_from_systemd() {
    // The env file is 0600 root; the script runs as the service user and
    // cannot read it. systemd, reading it as root, can.
    let unit = repo_file("deploy/storm-backup.service");
    assert_eq!(
        unit_value(&unit, "EnvironmentFile").as_deref(),
        Some("/etc/storm/storm.env")
    );
    let env_mode = repo_file("apps/server/Cargo.toml");
    assert!(
        env_mode.contains("\"etc/storm/storm.env\", \"600\""),
        "if the env file stops being 0600 this test's premise changed; revisit"
    );
}

/// Both units create files private to their user and group (decision 76).
///
/// Without it systemd's default umask, 0022, left every index and `auth.db`
/// readable by any local account, and the backup's archives — which carry
/// `auth.db` and the identity keys — the same.
#[test]
fn both_units_keep_what_they_create_from_other_accounts() {
    for unit in ["deploy/storm-server.service", "deploy/storm-backup.service"] {
        let text = repo_file(unit);
        assert_eq!(
            unit_value(&text, "UMask").as_deref(),
            Some("0027"),
            "{unit} must set UMask=0027"
        );
    }
}

/// The package closes the default root to other accounts, on upgrades too
/// (decision 76). A fresh install also gets 0750 subdirectories.
#[test]
fn the_package_closes_the_default_root_to_other_accounts() {
    let postinst = repo_file("apps/server/debian/postinst");
    let code: Vec<&str> = code_lines(&postinst).collect();
    assert!(
        code.iter().any(|l| l.starts_with("chmod o-rwx /srv/storm")),
        "postinst must remove other-access from /srv/storm on every install"
    );
    assert!(
        code.iter()
            .any(|l| l.starts_with("chmod 0750 /srv/storm/vaults ")),
        "a fresh install must create the default subdirectories 0750"
    );
    assert!(
        code.iter().any(|l| l.contains("! -L /srv/storm")),
        "never chmod through a symlink, which could point at a shared mount"
    );
}

#[test]
fn the_server_unit_can_list_interfaces_for_lan_discovery() {
    let unit = repo_file("deploy/storm-server.service");
    let families = unit_value(&unit, "RestrictAddressFamilies").unwrap();
    assert!(families.contains("AF_NETLINK"), "{families}");
}

/// One `[package.metadata.deb]` asset line: source and mode, by destination.
fn deb_asset(dest: &str) -> Option<(String, String)> {
    let manifest = repo_file("apps/server/Cargo.toml");
    manifest.lines().map(str::trim).find_map(|l| {
        let parts: Vec<&str> = l
            .trim_end_matches(',')
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|p| p.trim().trim_matches('"'))
            .collect();
        (parts.len() == 3 && parts[1] == dest).then(|| (parts[0].to_string(), parts[2].to_string()))
    })
}

#[test]
fn the_package_ships_a_valid_ufw_profile() {
    let (src, mode) = deb_asset("etc/ufw/applications.d/storm-server").expect("shipped");
    assert_eq!(src, "../../deploy/firewall/storm-server.ufw");
    assert_eq!(mode, "644");
    assert!(
        repo_file("apps/server/Cargo.toml").contains("\"/etc/ufw/applications.d/storm-server\""),
        "a conffile, so an operator's edit survives an upgrade"
    );

    // ufw's rules for a profile: one section named like the app, then title,
    // description and ports, each `port/proto`.
    let profile = repo_file("deploy/firewall/storm-server.ufw");
    let lines: Vec<&str> = code_lines(&profile).filter(|l| !l.is_empty()).collect();
    assert_eq!(lines[0], "[storm-server]");
    let value = |k: &str| {
        lines
            .iter()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("{k}= missing"))
    };
    assert!(!value("title").is_empty());
    assert!(!value("description").is_empty());
    let (port, proto) = value("ports").split_once('/').expect("port/proto");
    assert_eq!(proto, "tcp");
    assert_eq!(port.parse::<u16>().unwrap(), 8484, "the packaged default");
    assert_eq!(lines.len(), 4, "{lines:?}");
}

#[test]
fn the_package_ships_a_valid_firewalld_service() {
    let (src, mode) = deb_asset("usr/lib/firewalld/services/storm-server.xml").expect("shipped");
    assert_eq!(src, "../../deploy/firewall/storm-server.xml");
    assert_eq!(mode, "644");

    let xml = repo_file("deploy/firewall/storm-server.xml");
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n"));
    // Well formed: every element opened is closed, in order.
    let mut open: Vec<String> = Vec::new();
    for tag in xml.split('<').skip(1).map(|t| t.split('>').next().unwrap()) {
        if tag.starts_with('?') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            assert_eq!(open.pop().as_deref(), Some(name), "{xml}");
        } else if !tag.ends_with('/') {
            open.push(tag.split_whitespace().next().unwrap().to_string());
        }
    }
    assert!(open.is_empty(), "unclosed: {open:?}");
    assert!(xml.contains("<service>") && xml.contains("<short>Storm server</short>"));
    assert_eq!(xml.matches("<port ").count(), 1);
    assert!(xml.contains("<port protocol=\"tcp\" port=\"8484\"/>"));
}

#[test]
fn the_package_never_touches_a_firewall_rule() {
    let postrm = repo_file("apps/server/debian/postrm");
    assert!(
        code_lines(&postrm).any(|l| l == "rm -f /etc/firewalld/services/storm-server.xml"),
        "purge removes the port override `up` wrote"
    );
    // Adding or removing a rule is the installer's, with consent (decision 119).
    for script in ["postinst", "postrm", "prerm"] {
        let text = repo_file(&format!("apps/server/debian/{script}"));
        for tool in ["ufw ", "firewall-cmd"] {
            assert!(
                !code_lines(&text).any(|l| l.contains(tool)),
                "{script} runs {tool}"
            );
        }
    }
}
