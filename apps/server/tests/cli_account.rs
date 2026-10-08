//! The account CLI (`passwd`, `single-user`, `pair`), run as a real process.

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_storm-server");

const PASSWORD: &str = "correct horse battery staple";

struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Output {
    fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn run(state: &std::path::Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .arg("--state")
        .arg(state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("running storm-server");

    {
        let mut pipe = child.stdin.take().expect("stdin");
        if let Some(text) = stdin {
            pipe.write_all(text.as_bytes()).expect("writing stdin");
        }
    }

    let out = child.wait_with_output().expect("waiting for storm-server");
    Output {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn passwd(state: &std::path::Path, password: &str) -> Output {
    run(state, &["passwd", "--password-stdin"], Some(password))
}

#[test]
fn passwd_sets_up_a_fresh_storm_then_replaces_the_password() {
    let dir = tempdir::TempDir::new("storm-cli-passwd").unwrap();
    let state = dir.path();

    let first = passwd(state, PASSWORD);
    assert!(first.ok, "{}", first.combined());
    assert!(first.stdout.contains("password set"), "{}", first.stdout);

    let second = passwd(state, "a completely different passphrase");
    assert!(second.ok, "{}", second.combined());
    let id = |o: &Output| o.stdout.split('(').nth(1).map(str::to_string);
    assert_eq!(
        id(&first),
        id(&second),
        "{} / {}",
        first.stdout,
        second.stdout
    );
}

#[test]
fn there_is_no_user_command_any_more() {
    let dir = tempdir::TempDir::new("storm-cli-no-user").unwrap();
    let out = run(dir.path(), &["user", "list"], None);
    assert!(
        !out.ok,
        "`storm-server user` must not parse: {}",
        out.combined()
    );
}

#[test]
fn a_password_piped_with_a_trailing_newline_still_works() {
    let dir = tempdir::TempDir::new("storm-cli-newline").unwrap();
    let out = passwd(dir.path(), "correct horse battery staple\n");
    assert!(out.ok, "{}", out.combined());
}

#[test]
fn a_refused_password_exits_non_zero_and_writes_nothing() {
    let dir = tempdir::TempDir::new("storm-cli-short").unwrap();
    let state = dir.path();
    let out = passwd(state, "short");
    assert!(!out.ok, "{}", out.combined());
    let after = passwd(state, PASSWORD);
    assert!(after.ok, "{}", after.combined());
}

#[test]
fn single_user_keep_needs_the_name_typed_back() {
    let dir = tempdir::TempDir::new("storm-cli-keep").unwrap();
    let state = dir.path();
    let refused = run(
        state,
        &["single-user", "--keep", "dewansh"],
        Some("someone else\n"),
    );
    assert!(!refused.ok);
    assert!(
        refused.combined().contains("not confirmed"),
        "{}",
        refused.combined()
    );

    assert!(passwd(state, PASSWORD).ok);
    let ok = run(state, &["single-user", "--keep", "dewansh", "--yes"], None);
    assert!(ok.ok, "{}", ok.combined());
    assert!(
        ok.stdout.contains("this Storm's account is"),
        "{}",
        ok.stdout
    );
}

#[test]
fn pair_refuses_once_the_storm_is_set_up() {
    let dir = tempdir::TempDir::new("storm-cli-pair").unwrap();
    let state = dir.path();
    assert!(passwd(state, PASSWORD).ok);
    let out = run(state, &["pair", "--addr", "127.0.0.1:1"], None);
    assert!(!out.ok);
    assert!(
        out.combined().contains("already set up"),
        "{}",
        out.combined()
    );
}
