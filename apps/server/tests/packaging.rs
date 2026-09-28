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

/// The value of `key=` in a unit file, ignoring comments.
fn unit_value(unit: &str, key: &str) -> Option<String> {
    unit.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
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
