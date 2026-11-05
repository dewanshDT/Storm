//! The command an export or an Apply unit runs (137e), as the unit runs
//! it: the helper's own binary, `child-review`, unprivileged. It prints
//! one line, which is all the helper reads.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("/usr/bin/git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn review(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_storm-runtime-isolation"))
        .arg("child-review")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn an_export_then_an_apply_through_the_command() {
    if rustix::process::geteuid().is_root() || !Path::new("/usr/bin/git").is_file() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let child = t.path().join("child");
    std::fs::create_dir(&child).unwrap();
    git(&child, &["init", "-q", "--template="]);
    std::fs::write(child.join("a"), "1").unwrap();
    git(&child, &["add", "a"]);
    git(&child, &["commit", "-q", "-m", "one"]);
    let base = git(&child, &["rev-parse", "HEAD"]);
    std::fs::write(child.join("a"), "2").unwrap();
    git(&child, &["commit", "-q", "-am", "two"]);
    let dir = t.path().join("review");
    std::fs::create_dir(&dir).unwrap();
    let (c, d) = (child.to_str().unwrap(), dir.to_str().unwrap());
    let (ok, out) = review(&["export", "clone", c, d, &base, "1073741824"]);
    assert!(ok, "{out}");
    let v = storm_runtime_isolation::review::parse_export(&out)
        .unwrap()
        .unwrap();
    let tip = v["tip"].as_str().unwrap().to_string();
    assert_eq!(v["commits"][0]["subject"], "two");
    let parent = t.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    git(&parent, &["init", "-q", "--template="]);
    git(&parent, &["commit", "-q", "--allow-empty", "-m", "p"]);
    let bundle = dir.join(storm_runtime_isolation::review::BUNDLE);
    let (ok, out) = review(&[
        "import",
        "clone",
        parent.to_str().unwrap(),
        bundle.to_str().unwrap(),
        "storm/ags_C1",
        &tip,
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        storm_runtime_isolation::review::parse_import(&out)
            .unwrap()
            .unwrap()["branch"],
        "storm/ags_C1"
    );
    assert_eq!(git(&parent, &["rev-parse", "storm/ags_C1"]), tip);
    // A scratch Apply through the command.
    let (ok, out) = review(&[
        "import",
        "scratch",
        c,
        parent.to_str().unwrap(),
        "ags_C1",
        "copied",
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::read_to_string(parent.join("copied/a")).unwrap(),
        "2"
    );
    // Anything but its own arguments is refused.
    assert!(!review(&["export"]).0);
    assert!(!review(&["import", "clone", "/x"]).0);
}
