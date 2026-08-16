//! The operator CLI, driven as a real process.
//!
//! The unit tests in `auth::users` cover the rules; this covers the thing a
//! person actually types. It runs the built binary rather than calling into the
//! library, because the failures worth catching here — a subcommand that does
//! not parse, a password read that eats the wrong stream, an exit status of 0 on
//! a refusal — are all invisible from inside the crate.
//!
//! Every account here is created through `--password-stdin`. That is also the
//! path the VM runbook uses, so it is the one that has to keep working.

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_storm-server");

/// A password that satisfies the 12-character minimum.
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

    // The pipe is closed either way: a command that prompts would otherwise
    // block forever on a terminal that is never going to answer.
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

fn add(state: &std::path::Path, args: &[&str]) -> Output {
    let mut full = vec!["user", "add"];
    full.extend_from_slice(args);
    full.push("--password-stdin");
    run(state, &full, Some(PASSWORD))
}

#[test]
fn the_first_account_is_an_owner_and_later_ones_are_not() {
    let dir = tempdir::TempDir::new("storm-cli-roles").unwrap();
    let state = dir.path();

    let out = add(state, &["dewansh"]);
    assert!(out.ok, "{}", out.combined());
    assert!(out.stdout.contains("as owner"), "{}", out.stdout);
    assert!(state.join("auth.db").exists(), "auth.db was not created");

    // Defaulting the second account to owner would quietly hand out
    // administrative rights on every `user add`.
    let out = add(state, &["helper"]);
    assert!(out.ok, "{}", out.combined());
    assert!(out.stdout.contains("as member"), "{}", out.stdout);

    let list = run(state, &["user", "list"], None);
    assert!(list.ok, "{}", list.combined());
    assert!(list.stdout.contains("dewansh"), "{}", list.stdout);
    assert!(list.stdout.contains("helper"), "{}", list.stdout);
    assert!(list.stdout.contains("1 active owner"), "{}", list.stdout);
    // Freshly written hashes use the measured parameters, so nothing is stale.
    assert!(!list.stdout.contains("outdated"), "{}", list.stdout);
}

#[test]
fn the_first_account_cannot_be_created_as_a_member() {
    let dir = tempdir::TempDir::new("storm-cli-first").unwrap();
    let state = dir.path();

    let out = add(state, &["helper", "--role", "member"]);
    assert!(!out.ok, "a member-only server must be refused");
    assert!(
        out.combined().contains("first user must be an owner"),
        "{}",
        out.combined()
    );

    let list = run(state, &["user", "list"], None);
    assert!(list.stdout.contains("no users yet"), "{}", list.stdout);
}

#[test]
fn the_last_owner_is_protected_from_the_command_line_too() {
    let dir = tempdir::TempDir::new("storm-cli-owner").unwrap();
    let state = dir.path();
    assert!(add(state, &["dewansh"]).ok);
    assert!(add(state, &["helper"]).ok);

    for args in [
        vec!["user", "delete", "dewansh", "--yes"],
        vec!["user", "disable", "dewansh"],
        vec!["user", "role", "dewansh", "member"],
    ] {
        let out = run(state, &args, None);
        assert!(!out.ok, "{args:?} should have failed: {}", out.combined());
        assert!(
            out.combined().contains("only active owner"),
            "{args:?}: {}",
            out.combined()
        );
    }

    // Promoting the other account first is the documented way through.
    assert!(run(state, &["user", "role", "helper", "owner"], None).ok);
    let out = run(state, &["user", "role", "dewansh", "member"], None);
    assert!(out.ok, "{}", out.combined());
}

#[test]
fn a_refused_password_exits_non_zero_and_writes_nothing() {
    let dir = tempdir::TempDir::new("storm-cli-weak").unwrap();
    let state = dir.path();

    let out = run(
        state,
        &["user", "add", "dewansh", "--password-stdin"],
        Some("short"),
    );
    assert!(!out.ok, "a short password must fail");
    assert!(
        out.combined().contains("minimum is 12"),
        "{}",
        out.combined()
    );

    let list = run(state, &["user", "list"], None);
    assert!(
        list.stdout.contains("no users yet"),
        "a refused password must not leave an account: {}",
        list.stdout
    );
}

#[test]
fn usernames_collide_case_insensitively() {
    let dir = tempdir::TempDir::new("storm-cli-fold").unwrap();
    let state = dir.path();
    assert!(add(state, &["Dewansh"]).ok);

    let out = add(state, &["dewansh"]);
    assert!(!out.ok, "a case variant is the same account");
    assert!(
        out.combined().contains("already exists"),
        "{}",
        out.combined()
    );
}

#[test]
fn passwd_replaces_the_password_and_says_so() {
    let dir = tempdir::TempDir::new("storm-cli-passwd").unwrap();
    let state = dir.path();
    assert!(add(state, &["dewansh"]).ok);

    // The command verifies the stored hash against the new password before it
    // reports success, so a zero exit here is evidence the account can be
    // logged into with it — not merely that a row was written.
    let out = run(
        state,
        &["passwd", "dewansh", "--password-stdin"],
        Some("a completely different passphrase"),
    );
    assert!(out.ok, "{}", out.combined());
    assert!(out.stdout.contains("password updated"), "{}", out.stdout);

    let missing = run(
        state,
        &["passwd", "nobody", "--password-stdin"],
        Some(PASSWORD),
    );
    assert!(!missing.ok);
    assert!(
        missing.combined().contains("no user named `nobody`"),
        "{}",
        missing.combined()
    );
}

#[test]
fn disable_enable_and_delete_round_trip() {
    let dir = tempdir::TempDir::new("storm-cli-lifecycle").unwrap();
    let state = dir.path();
    assert!(add(state, &["dewansh"]).ok);
    assert!(add(state, &["helper"]).ok);

    assert!(run(state, &["user", "disable", "helper"], None).ok);
    let list = run(state, &["user", "list"], None);
    assert!(list.stdout.contains("disabled"), "{}", list.stdout);

    assert!(run(state, &["user", "enable", "helper"], None).ok);
    let list = run(state, &["user", "list"], None);
    assert!(!list.stdout.contains("disabled"), "{}", list.stdout);

    // Unconfirmed deletes are refused: the prompt wants the username back.
    let out = run(
        state,
        &["user", "delete", "helper"],
        Some("something else\n"),
    );
    assert!(!out.ok, "{}", out.combined());
    assert!(
        out.combined().contains("not confirmed"),
        "{}",
        out.combined()
    );
    assert!(
        run(state, &["user", "list"], None)
            .stdout
            .contains("helper")
    );

    // Typing the name through is the confirmation.
    let out = run(state, &["user", "delete", "helper"], Some("helper\n"));
    assert!(out.ok, "{}", out.combined());
    let list = run(state, &["user", "list"], None);
    assert!(!list.stdout.contains("helper"), "{}", list.stdout);
}

#[test]
fn a_password_piped_with_a_trailing_newline_still_works() {
    // `echo 'secret' | storm-server user add …` is how a runbook writes this,
    // and echo adds a newline. Storing it would create an account whose
    // password cannot be typed at any prompt.
    let dir = tempdir::TempDir::new("storm-cli-newline").unwrap();
    let state = dir.path();

    let out = run(
        state,
        &["user", "add", "dewansh", "--password-stdin"],
        Some("correct horse battery staple\n"),
    );
    assert!(out.ok, "{}", out.combined());
}

#[test]
fn a_malformed_username_is_refused_before_a_password_is_asked_for() {
    let dir = tempdir::TempDir::new("storm-cli-name").unwrap();
    let state = dir.path();

    // No stdin at all: if the command asked for a password before checking the
    // name, this would fail on an empty read instead of on the username.
    let out = run(state, &["user", "add", "has space"], None);
    assert!(!out.ok);
    assert!(
        out.combined().contains("username may contain"),
        "{}",
        out.combined()
    );
}
