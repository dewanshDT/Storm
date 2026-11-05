//! The command a child's clone unit runs (137b), as the unit runs it: the
//! helper's own binary, `child-clone`, as an unprivileged user. It prints
//! one line, which is all the helper reads.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("/usr/bin/git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn child_clone(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_storm-runtime-isolation"))
        .arg("child-clone")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn the_clone_command_prints_one_checked_line() {
    if rustix::process::geteuid().is_root() || !Path::new("/usr/bin/git").is_file() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let parent = t.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    git(&parent, &["init", "-q", "--template="]);
    std::fs::write(parent.join("a"), "1").unwrap();
    git(&parent, &["add", "a"]);
    git(&parent, &["commit", "-q", "-m", "one"]);
    let dest = t.path().join("dest");
    std::fs::create_dir(&dest).unwrap();
    let (ok, out) = child_clone(&[
        parent.to_str().unwrap(),
        dest.to_str().unwrap(),
        "1073741824",
    ]);
    assert!(ok, "{out}");
    let report = storm_runtime_isolation::clone::parse_line(&out)
        .unwrap()
        .unwrap();
    assert!(storm_runtime_isolation::clone::commit_ok(&report.commit));
    assert_eq!(std::fs::read_to_string(dest.join("a")).unwrap(), "1");
    // Too small a budget is a code, not a failure of the command.
    let small = t.path().join("small");
    std::fs::create_dir(&small).unwrap();
    let (ok, out) = child_clone(&[parent.to_str().unwrap(), small.to_str().unwrap(), "1"]);
    assert!(ok);
    assert_eq!(
        storm_runtime_isolation::clone::parse_line(&out).unwrap(),
        Err("workspace_cap")
    );
    // Anything but its three arguments is refused.
    let (ok, _) = child_clone(&["/x"]);
    assert!(!ok);
}
